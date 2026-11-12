//! The Infinite route inside a run: the live 3x3 chunk window that recentres as the runner
//! walks, stairwells between levels, chunk state that survives unloading (relays, batteries,
//! coins and pickups taken, cells discovered), and the hunter population that grows with
//! recovered relays (GameConfig.streamed_hunter_population).

use super::resident_hunters::Slot;
use super::{pdist, FloorPickup, Game, Item};
use crate::config::*;
use crate::content::Content;
use crate::hunters::brain::{Brain, State};
use crate::hunters::gimmicks::Gimmick;
use crate::hunters::residency::{self, Cand, Record};
use crate::shared::safe_room::{SafeRoom, SafeRooms, Status};
use crate::world::catalog::Catalog;
use crate::world::facility::{Facility, Pickup, Stair};
use crate::world::fixtures::Fixtures;
use crate::world::infinite::{self, ChunkKey, CHUNK};
use crate::world::route::hunter_spawns::HunterSpawn;
use crate::world::route::live;
use crate::world::rng::Rng;
use crate::world::Cell;
use super::level_stack::{floor_y_of, level_at, levels_to_keep, LevelStack, Resident};
use super::player::stair_height_at;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

thread_local! {
    /// `keep_stack`'s list of pinned levels, reused every frame so it allocates nothing once it has grown.
    static KEEP_PINNED: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The resident levels walked in `keep_stack`, reused every frame (`ResidentHunters::levels_into`).
    static KEEP_RESIDENT: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What the full map draws of one discovered level-0 chunk (Godot's `_discovered_chunks` record for the map). It is taken
/// when the chunk first loads and kept for the run, so the chunk stays on the map after the window moves on. `walls` and
/// `holes` are chunk-local (row-major, `z * CHUNK + x`); the stairs, relays, batteries and props are absolute cells.
#[derive(Clone, Debug)]
pub struct MapChunk {
    /// Solid cells as the window draws them (the carved grid, so a stair flight is open).
    pub walls: Vec<bool>,
    /// Stairwell cells: the wells of each flight, open and drawn dark.
    pub holes: Vec<bool>,
    /// The chunk's stair landings, absolute.
    pub stairs: Vec<Stair>,
    /// Relays and batteries with their entity ids, absolute.
    pub relays: Vec<(String, Cell)>,
    pub batteries: Vec<(String, Cell)>,
    /// The solid props, drawn as their collision boxes once discovered.
    pub props: Vec<MapProp>,
}

/// A solid prop's collision box for the full map: its entity id, its absolute cell and its rotated half extents in cells.
#[derive(Clone, Debug)]
pub struct MapProp {
    pub id: String,
    pub cell: Cell,
    pub half: (f32, f32),
}

impl MapChunk {
    /// The record of level-0 chunk `k` from its own facility (the chunk as the window merged it, from the window cache) and
    /// its entity ids: `relay_ids` and `battery_ids` line up with `fac.relays` and `fac.batteries`.
    fn of(k: ChunkKey, fac: &Facility, relay_ids: &[String], battery_ids: &[String]) -> MapChunk {
        let o = (k.cx * CHUNK, k.cy * CHUNK);
        let abs = |c: Cell| Cell::new(c.x + o.0, c.y + o.1);
        let mut holes = vec![false; (CHUNK * CHUNK) as usize];
        for s in &fac.stairs {
            for c in s.well_cells() {
                if (0..CHUNK).contains(&c.x) && (0..CHUNK).contains(&c.y) {
                    holes[(c.y * CHUNK + c.x) as usize] = true;
                }
            }
        }
        MapChunk {
            walls: (0..CHUNK)
                .flat_map(|z| (0..CHUNK).map(move |x| fac.is_wall_cell(x, z)))
                .collect(),
            holes,
            stairs: fac
                .stairs
                .iter()
                .map(|s| Stair {
                    cell: abs(s.cell),
                    ..*s
                })
                .collect(),
            relays: fac
                .relays
                .iter()
                .zip(relay_ids)
                .map(|(c, id)| (id.clone(), abs(*c)))
                .collect(),
            batteries: fac
                .batteries
                .iter()
                .zip(battery_ids)
                .map(|(c, id)| (id.clone(), abs(*c)))
                .collect(),
            props: fac
                .props
                .iter()
                .filter(|p| p.solid)
                .map(|p| MapProp {
                    id: p.id.clone(),
                    cell: abs(p.cell),
                    half: p.half,
                })
                .collect(),
        }
    }
}

/// The level-0 full map's feed (ui/map.rs): the record of every chunk the runner has seen a cell of, in key order, the cells it
/// has seen, and the frame they are drawn in.
pub struct FullMapFeed<'a> {
    /// Every level-0 chunk with a seen cell, by key, in (cx, cy) order.
    pub chunks: Vec<(ChunkKey, &'a MapChunk)>,
    /// The seen cells (a cell outside them is not drawn, nor are the marks on it).
    pub seen: &'a super::discovery::SeenCells,
    /// The window's origin: the absolute cell of its (0, 0). Window-local positions (the runner, the hunters) add it.
    pub origin: (i32, i32),
    /// The waypoint route (`WaypointRoutes::full`) in absolute cell centres.
    pub route: &'a [(f32, f32)],
    /// The waypoint's pin, absolute, when it is on level 0.
    pub waypoint: Option<(f32, f32)>,
}

/// The map state the admin MAP REVEAL overwrites, kept so switching it off can undo it.
pub struct MapRevealUndo {
    discovered: Vec<Vec<bool>>,
    level0_seen: Option<super::discovery::SeenCells>,
}

pub struct InfState {
    /// The world seed of the route: str(run_seed) of the room seed text (route::world_seed).
    pub seed: String,
    pub level: i32,
    /// Chunk at the middle of the window.
    pub center: (i32, i32),
    cache: HashMap<ChunkKey, (Facility, Fixtures)>,
    /// Frames to wait before the next chunk is generated ahead of a window crossing (`prewarm_chunks`).
    prewarm_wait: u32,
    /// Claimed pickups by entity id (Godot's claimed_pickups, streamed_world_state.gd:11): the Godot
    /// id on level 0, `L<level>:<KIND>:<x>:<y>` (absolute cell) on the other levels.
    claimed: HashSet<String>,
    /// The entity ids of the window now built, aligned with its pickup lists.
    ids: live::WindowIds,
    /// Each level-0 chunk's layout pickups, read once (`live::window_ids`).
    layout_pickups: HashMap<ChunkKey, Vec<live::LayoutPickup>>,
    /// Level-0 cells: the discovered chunks. A level-0 chunk's walls are on the map from the moment it
    /// loads (Godot `_discover_loaded_chunks`, `_discovered_chunks`), so the grid is read from this set.
    discovered_chunks: HashSet<ChunkKey>,
    /// Level-0 rooms the runner has stood in (Godot `_discovered_rooms`): the layout room id and the
    /// room's absolute origin, `id@x,y` (see `room_key`).
    discovered_rooms: HashSet<String>,
    /// Level-0 map markers discovered (Godot `_discovered_map_markers`): the entity ids of the relays,
    /// batteries, pickups, coins and props that the map draws once found (see `discover_map`).
    markers: HashSet<String>,
    /// Level-0 map records of the discovered chunks (see `MapChunk`), taken when each chunk is discovered and never dropped,
    /// so the full map draws every discovered chunk after it has left the window.
    map_chunks: HashMap<ChunkKey, MapChunk>,
    /// Level-0 cells the runner has seen (mod.rs `reveal`; the full map and the minimap draw only these): a bitmap per chunk
    /// keyed by absolute cell (`discovery::SeenCells`), so it outlives the window and a chunk unloading.
    pub(super) level0_seen: super::discovery::SeenCells,
    /// Off level 0, the cells the line-of-sight reveal (mod.rs `reveal`) has found, by (level, x, y).
    seen: HashSet<(i32, i32, i32)>,
    pub chunks_visited: HashSet<ChunkKey>,
    /// The run's safe rooms: integrity by room, the active checkpoint and the breach damage carried
    /// between steps (Godot world_state.safe_rooms, active_checkpoint, _safe_damage_accumulators).
    pub safe: SafeRooms,
    /// The middle chunk's safe room on the level-0 layout, `None` when that chunk is not safe.
    room: Option<SafeRoom>,
    /// Whether the window is the level-0 layout (`live::window_ids` succeeded). The safe-room rules
    /// run only then; the old builder keeps its own safe zone.
    live: bool,
    /// `Game::time` at the last safe-room step, so the step knows its dt (-1 before the first).
    last_time: f32,
    /// `Game::time` at the last prune of drained pipe flows (`pipes_tick`).
    pipes_pruned_at: f32,
    /// Seconds interact has been held on a valve wheel (game/interact.rs `update_valve_hold`), the seconds the wheel
    /// still spins after a turn, and the turns made here that the net layer has not sent yet.
    pub(super) valve_hold: f32,
    pub(super) valve_spin: f32,
    pub(super) valve_outbox: Vec<super::pipes::TurnEvent>,
    /// The valve turns a guest asked the host for (`Game::take_valve_requests`).
    pub(super) valve_requests: Vec<super::interact::ValveAsk>,
    /// The burst flow ids a guest's Axe opened, for the host to confirm (`Game::take_burst_requests`).
    pub(super) burst_requests: Vec<String>,
    /// Checkpoints the runner has activated this run (Godot `_run_statistics.checkpoints_activated`): one for every
    /// change of the active checkpoint, so walking back into the room already active counts nothing.
    pub checkpoints_activated: u32,
    /// Level-0 hunters the live window does not hold: dormant, or outside the 3x3 window. Their positions are absolute
    /// cells (`Brain.pos` and the target fields), so they keep their place as the window moves (Godot world_state.hunters).
    stored: HashMap<String, Brain>,
    /// The entity ids of `Game.hunters` on level 0, in the same order (`""` for a hunter the route did not admit, such as
    /// an admin summon, which is never stored).
    loaded: Vec<String>,
    /// The hunters of each level off level 0 that the runner has left by a stairwell, at absolute cells (as `stored`), so the
    /// level keeps them for the way back (Godot keeps every hunter in world_state on its own level).
    floor_hunters: HashMap<i32, Vec<Brain>>,
    /// Seconds gathered toward the next coarse offscreen step (`Game::simulate_offscreen`, data/hunters/offscreen.json).
    pub(super) offscreen_clock: f32,
    /// Coarse steps taken, which seeds each step's draw.
    pub(super) offscreen_tick: u64,
    /// Each level-0 chunk's hunter spawns, read once (`live::hunter_spawns`).
    spawns: HashMap<ChunkKey, Vec<HunterSpawn>>,
    /// Level-0 walls of the discovered chunks, row-major (`live::walls`), read when a chunk is first loaded: Godot keeps
    /// them on `_discovered_chunks[coord].walls` for the waypoint grid.
    walls: HashMap<ChunkKey, Vec<bool>>,
    /// The discovered floor the waypoint routes read, rebuilt when a chunk is discovered (`floor_chunks` is the count it
    /// was built from).
    floor: DiscoveredFloor,
    floor_chunks: usize,
    /// The runner's one waypoint (Godot `WaypointState`), `None` when there is none.
    pub waypoint: Option<Waypoint>,
    /// The routes to the waypoint, planned when `route_key` changes (`refresh_waypoint`).
    routes: WaypointRoutes,
    route_key: Option<RouteKey>,
    /// The levels resident beside the active one (`keep_stack`): built at the window's centre, swapped in by `relabel`.
    pub stack: LevelStack,
    /// The windows built so far (`infinite::window` calls), so a test can see that a change builds none.
    pub(super) builds: u32,
    /// Set by `relabel`: the keep set of the new level is topped up from the next frame, so the frame of the change loads
    /// nothing (`keep_stack` clears it).
    relabelled: bool,
}

/// An estimate of the bytes a built window holds: the per-cell grids and the entries of its lists.
pub(super) fn window_bytes(fac: &Facility, fix: &Fixtures) -> usize {
    use std::mem::size_of;
    let cells = (fac.cols.max(0) as usize) * (fac.rows.max(0) as usize);
    // wall, region, hole and hunter-open flags, and the solid-prop index per cell
    let grids = cells * (4 + size_of::<usize>());
    grids
        + fac.props.len() * size_of::<crate::world::facility::Prop>()
        + fac.rooms.len() * size_of::<crate::world::facility::Room>()
        + fac.pickups.len() * size_of::<(Cell, Pickup)>()
        + fac.stairs.len() * size_of::<Stair>()
        + (fac.relays.len() + fac.batteries.len() + fix.vents.len() + fix.coins.len()) * size_of::<Cell>()
}

/// Godot `WaypointState.ARRIVAL_RADIUS`: the runner within this many cells (XZ) of the waypoint clears it.
pub const WAYPOINT_ARRIVAL_RADIUS: f32 = 2.0;
/// Godot `WaypointState.MAX_PLACE_RANGE`: the longest reach of the crosshair placement, in cells.
pub const WAYPOINT_PLACE_RANGE: f32 = 90.0;
/// Godot `WAYPOINT_TRAIL_METRES`: the floor chevrons (PATH guidance) cover only this much of the route.
pub const WAYPOINT_TRAIL_METRES: f32 = 28.0;
/// Godot `WaypointLine.STRAIGHT_FALLBACK`: a route that stops short runs straight on to the waypoint.
const STRAIGHT_FALLBACK: bool = true;

/// The runner's one waypoint (Godot `WaypointState`, one per player): an absolute cell centre and the level it is on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    pub at: (f32, f32),
    pub level: i32,
}

/// The waypoint's two routes, as absolute cell centres. `full` is the route the minimap and the full map draw and that the
/// floor chevrons follow (Godot `_waypoint_route`); `line` is the floor line (Godot `_waypoint_line_route`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WaypointRoutes {
    pub full: Vec<(f32, f32)>,
    pub line: Vec<(f32, f32)>,
}

/// A PATH-guidance floor chevron (Godot `_update_waypoint_ground_trail`): its centre and the unit direction of travel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chevron {
    pub at: (f32, f32),
    pub dir: (f32, f32),
}

/// What a route was planned for (Godot `_refresh_waypoint_route`'s key): a change in any field plans it again.
#[derive(Clone, Copy, Debug, PartialEq)]
struct RouteKey {
    level: i32,
    center: (i32, i32),
    from: (i32, i32),
    to: Option<(i32, i32)>,
    discovered: usize,
}

/// Godot `WaypointGrid` over the level-0 discovered chunks: every open cell of a discovered chunk (absolute cells), with the
/// connected part (4 neighbours) it belongs to. Built from the chunks' walls, so a cell reads open once its chunk is discovered.
#[derive(Clone, Debug, Default)]
pub struct DiscoveredFloor {
    comp: HashMap<(i32, i32), u32>,
}

impl DiscoveredFloor {
    /// The floor of `chunks`, each chunk's walls from `walls` (row-major, `z * CHUNK + x`, true = solid). A chunk with no
    /// walls on record is closed.
    pub fn build(chunks: &HashSet<ChunkKey>, walls: &HashMap<ChunkKey, Vec<bool>>) -> DiscoveredFloor {
        let n = CHUNK as usize;
        let mut open: Vec<(i32, i32)> = Vec::new();
        for k in chunks {
            let Some(w) = walls.get(k) else { continue };
            if w.len() != n * n {
                continue;
            }
            for z in 0..CHUNK {
                for x in 0..CHUNK {
                    if !w[z as usize * n + x as usize] {
                        open.push((k.cx * CHUNK + x, k.cy * CHUNK + z));
                    }
                }
            }
        }
        let open_set: HashSet<(i32, i32)> = open.iter().copied().collect();
        open.sort();
        let mut comp: HashMap<(i32, i32), u32> = HashMap::with_capacity(open.len());
        let mut next = 0u32;
        for start in open {
            if comp.contains_key(&start) {
                continue;
            }
            comp.insert(start, next);
            let mut stack = vec![start];
            while let Some(c) = stack.pop() {
                for nb in neighbours4(c) {
                    if open_set.contains(&nb) && !comp.contains_key(&nb) {
                        comp.insert(nb, next);
                        stack.push(nb);
                    }
                }
            }
            next += 1;
        }
        DiscoveredFloor { comp }
    }

    pub fn is_open(&self, c: (i32, i32)) -> bool {
        self.comp.contains_key(&c)
    }

    /// Godot `WaypointGrid.reachable_target`: `to` when it is open on `from`'s side of the walls, else the open cell of that
    /// side nearest `to` (the frontier). `to` when `from` is closed.
    pub fn reachable_target(&self, from: (i32, i32), to: (i32, i32)) -> (i32, i32) {
        let Some(&id) = self.comp.get(&from) else { return to };
        if self.comp.get(&to) == Some(&id) {
            return to;
        }
        let d2 = |c: (i32, i32)| {
            let (dx, dy) = ((c.0 - to.0) as i64, (c.1 - to.1) as i64);
            dx * dx + dy * dy
        };
        let mut best = from;
        for (&c, &cid) in &self.comp {
            if cid == id && (d2(c), c) < (d2(best), best) {
                best = c;
            }
        }
        best
    }

    /// Godot `WaypointGrid.path_to` (4 neighbours, Manhattan heuristic): the cells from `from` to `to`, both open. Empty when
    /// either is closed or no open route joins them.
    pub fn path(&self, from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
        if !self.is_open(from) || !self.is_open(to) {
            return vec![];
        }
        let h = |c: (i32, i32)| (c.0 - to.0).abs() + (c.1 - to.1).abs();
        let mut g: HashMap<(i32, i32), i32> = HashMap::new();
        let mut came: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
        let mut open = BinaryHeap::new();
        g.insert(from, 0);
        open.push(Reverse((h(from), from.0, from.1)));
        while let Some(Reverse((f, x, y))) = open.pop() {
            let c = (x, y);
            let gc = g[&c];
            if f > gc + h(c) {
                continue; // a stale entry: the cell was reached more cheaply since
            }
            if c == to {
                let mut path = vec![c];
                let mut cur = c;
                while let Some(&p) = came.get(&cur) {
                    path.push(p);
                    cur = p;
                }
                path.reverse();
                return path;
            }
            for nb in neighbours4(c) {
                if !self.is_open(nb) {
                    continue;
                }
                let ng = gc + 1;
                if g.get(&nb).is_some_and(|&old| old <= ng) {
                    continue;
                }
                g.insert(nb, ng);
                came.insert(nb, c);
                open.push(Reverse((ng + h(nb), nb.0, nb.1)));
            }
        }
        vec![]
    }
}

fn neighbours4(c: (i32, i32)) -> [(i32, i32); 4] {
    [(c.0 + 1, c.1), (c.0 - 1, c.1), (c.0, c.1 + 1), (c.0, c.1 - 1)]
}

/// A cell's centre (the point the routes run through).
fn centre(c: (i32, i32)) -> (f32, f32) {
    (c.0 as f32 + 0.5, c.1 as f32 + 0.5)
}

/// Drops the route's points before the cell `from` when the route runs through it; false (route untouched) when it
/// does not, so the caller plans again.
fn trim_route(route: &mut Vec<(f32, f32)>, from: (i32, i32)) -> bool {
    let c = centre(from);
    match route.iter().position(|p| *p == c) {
        Some(i) => {
            route.drain(..i);
            true
        }
        None => false,
    }
}

/// Godot `WaypointLine.choose_route` (waypoint_line.gd:21): the floor line's points, as cell centres. `discovered` is the
/// discovered cell path, possibly empty. A discovered route that stops short of `to` runs straight on to it; without one, a
/// straight segment runs from `from`.
pub fn choose_line(discovered: &[(i32, i32)], from: (i32, i32), to: (i32, i32)) -> Vec<(f32, f32)> {
    if discovered.len() >= 2 {
        let mut points: Vec<(f32, f32)> = discovered.iter().map(|c| centre(*c)).collect();
        if discovered.last() != Some(&to) && STRAIGHT_FALLBACK {
            points.push(centre(to));
        }
        return points;
    }
    if !STRAIGHT_FALLBACK || from == to {
        return vec![];
    }
    vec![centre(from), centre(to)]
}

/// Godot `_update_waypoint_ground_trail` (PATH guidance): chevrons spaced 1.6 cells apart along the first
/// `WAYPOINT_TRAIL_METRES` of `route` (window cell-centre coordinates), the first one 1 cell in.
pub fn chevrons(route: &[(f32, f32)]) -> Vec<Chevron> {
    let mut out = Vec::new();
    if route.len() < 2 {
        return out;
    }
    let mut travelled = 0.0f32;
    let mut next = 1.0f32;
    for pair in route.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let length = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if length <= 0.0001 {
            continue;
        }
        let dir = ((b.0 - a.0) / length, (b.1 - a.1) / length);
        while next <= travelled + length {
            if next > WAYPOINT_TRAIL_METRES {
                break;
            }
            let d = next - travelled;
            out.push(Chevron {
                at: (a.0 + dir.0 * d, a.1 + dir.1 * d),
                dir,
            });
            next += 1.6;
        }
        travelled += length;
        if travelled > WAYPOINT_TRAIL_METRES {
            break;
        }
    }
    out
}

/// Absolute cell centres as window cell-centre coordinates.
fn to_window(cells: &[(f32, f32)], origin: (i32, i32)) -> Vec<(f32, f32)> {
    cells
        .iter()
        .map(|c| (c.0 - origin.0 as f32, c.1 - origin.1 as f32))
        .collect()
}

/// Godot `_safe_context` (infinite_route_test.gd:4100): the runner is inside a safe room within this many cells of its centre.
pub const SAFE_ROOM_REACH: f32 = 3.25;
/// Godot `_on_tool_used` / `_safe_room_repair_in_reach`: a crowbar or rescue axe repairs a room from this far.
pub const SAFE_ROOM_REPAIR_REACH: f32 = 4.25;
/// Godot `_hunter_step_group`: a hunter this close to the centre of a safe room batters it while the runner is inside.
pub const SAFE_ROOM_BREACH_REACH: f32 = 4.25;
/// Godot `_hunter_step_group`: a hunter within this many cells of the centre of a safe room the runner is inside
/// is sent to investigate its centre.
pub const SAFE_ROOM_LURE_REACH: f32 = 18.0;
/// Largest step the safe-room rules take in one go (a hitch must not do a second's breach damage at once).
const SAFE_ROOM_MAX_DT: f32 = 0.25;

/// Where a runner entering the window stands: the window's start cell, or (on a sewer level, where the centre is the
/// channel crossing) the nearest dry cell the walking grid keeps open. The channel's water is fenced by railings, so a
/// runner put in it could never climb out to the walkways.
/// On a level with a flight the cell must also reach one by a dry route (no water, railing or well), so it is never a
/// pocket the railings cut off.
pub(crate) fn dry_start(f: &Facility) -> (f32, f32) {
    let s = f.start;
    let ok = |c: Cell| !f.is_water(c) && f.hunter_open(c) && f.body_fits(c.x as f32 + 0.5, c.y as f32 + 0.5, PLAYER_RADIUS);
    // A dry cell the railings or a well have cut off from every flight is a pocket the runner could never leave (the walk to
    // a landing found no route or stalled at its fence). `joined` says whether the dry region of a cell holds a flight's
    // cell; the regions already seen to hold none are remembered so each is flooded once. A level with no flight takes any.
    let anchors: HashSet<Cell> = f.stairs.iter().map(|st| st.cell).filter(|c| f.hunter_open(*c)).collect();
    let mut pockets: HashSet<Cell> = HashSet::new();
    let mut joined = |c: Cell| -> bool {
        if anchors.is_empty() {
            return true;
        }
        if pockets.contains(&c) {
            return false;
        }
        let mut seen: HashSet<Cell> = HashSet::from([c]);
        let mut q = vec![c];
        while let Some(at) = q.pop() {
            if anchors.contains(&at) {
                return true;
            }
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = Cell::new(at.x + dx, at.y + dy);
                if !seen.contains(&n) && f.hunter_open(n) && !f.is_water(n) && !f.rail_between(at, n) {
                    seen.insert(n);
                    q.push(n);
                }
            }
        }
        pockets.extend(seen);
        false
    };
    if ok(s) && joined(s) {
        return s.center();
    }
    for r in 1..=12i32 {
        let mut best: Option<(f32, Cell)> = None;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dy.abs() != r {
                    continue;
                }
                let c = Cell::new(s.x + dx, s.y + dy);
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if best.map_or(true, |b| d < b.0) && ok(c) && joined(c) {
                    best = Some((d, c));
                }
            }
        }
        if let Some((_, c)) = best {
            return c.center();
        }
    }
    s.center()
}

/// The centre of the window's middle chunk, where the runner's safe room is: the chunk's centre cell (22) in
/// window cells, plus the half cell that `Cell::center` adds (Godot `(coord + .5) * CHUNK_SIZE`).
fn middle_safe_centre() -> (f32, f32) {
    let c = (CHUNK + live::CENTRE) as f32 + 0.5;
    (c, c)
}

/// Godot `MAP_DISCOVERY_NEARBY_RADIUS` (infinite_route_test.gd:25): a marker this close to the runner is
/// discovered even outside its room.
pub const MAP_DISCOVERY_NEARBY_RADIUS: f32 = 4.5;

/// The ids of a window the layout does not supply (levels -1 to -3, or the legacy level-0 builder):
/// the level, the list and the absolute cell, as the ledger keyed them before entity ids.
fn cell_ids(level: i32, origin: (i32, i32), fac: &Facility, fix: &Fixtures) -> live::WindowIds {
    let id = |kind: &str, c: &Cell| format!("L{level}:{kind}:{}:{}", c.x + origin.0, c.y + origin.1);
    live::WindowIds {
        relays: fac.relays.iter().map(|c| id("RELAY", c)).collect(),
        batteries: fac.batteries.iter().map(|c| id("BATTERY", c)).collect(),
        pickups: fac.pickups.iter().map(|(c, _)| id("PICKUP", c)).collect(),
        coins: fix.coins.iter().map(|c| id("COIN", c)).collect(),
    }
}

/// The entity ids of one level-0 chunk's relays and batteries, aligned with `fac.relays` and `fac.batteries`. On the layout
/// (`live`), the chunk's layout ids in the order `window_ids` keys them; otherwise the absolute-cell keys `cell_ids` falls
/// back to, so a chunk keeps the same ids the window gives it.
fn chunk_ids(layout: Option<&Vec<live::LayoutPickup>>, live: bool, k: ChunkKey, fac: &Facility) -> (Vec<String>, Vec<String>) {
    if live {
        if let Some(list) = layout {
            let of_kind = |kind: &str| -> Vec<String> {
                list.iter()
                    .filter(|p| p.kind == kind)
                    .map(|p| p.id.clone())
                    .collect()
            };
            let (relays, batteries) = (of_kind("relay"), of_kind("battery"));
            if relays.len() == fac.relays.len() && batteries.len() == fac.batteries.len() {
                return (relays, batteries);
            }
        }
    }
    let o = (k.cx * CHUNK, k.cy * CHUNK);
    let id = |kind: &str, c: &Cell| format!("L0:{kind}:{}:{}", c.x + o.0, c.y + o.1);
    (
        fac.relays.iter().map(|c| id("RELAY", c)).collect(),
        fac.batteries.iter().map(|c| id("BATTERY", c)).collect(),
    )
}

/// The floor item a window pickup becomes, with its count. The live `Item` enum holds the flashlight
/// tools, the lockpick, the battery, the stim pack and the revive kit; a backpack has no `Item` yet, so it
/// is not on the floor (see docs/PARITY.md, "Live level 0"). Taking one goes through `grant` (game/interact.rs).
fn floor_item(k: &Pickup) -> Option<(Item, i32)> {
    Some(match k {
        Pickup::Taser => (Item::Taser, 1),
        Pickup::Crowbar => (Item::Crowbar, CROWBAR_DURABILITY),
        Pickup::Axe => (Item::Axe, AXE_DURABILITY),
        Pickup::Battery => (Item::Battery, 1),
        Pickup::Stim => (Item::Stim, 1),
        Pickup::Lockpick => (Item::Lockpick, 1),
        Pickup::ReviveKit => (Item::ReviveKit, 1),
        Pickup::MusicBox => (Item::MusicBox, 1),
        _ => return None,
    })
}

/// The indices into `fac.pickups` that `floor_pickups` keeps, in order: floor pickup i is
/// `fac.pickups[floor_source(fac)[i]]`.
fn floor_source(fac: &Facility) -> Vec<usize> {
    fac.pickups
        .iter()
        .enumerate()
        .filter(|(_, e)| floor_item(&e.1).is_some())
        .map(|(i, _)| i)
        .collect()
}

/// streamed_hunter_population: none before the activation relay, then the base lineup and one
/// more every STREAMED_HUNTER_RELAY_STEP relays (relaxed 3, standard 2, hard 1).
pub fn population(diff: Difficulty, relays: i32) -> i32 {
    // streamed_base / streamed_step per difficulty come from data/progression.json (B9)
    crate::game::progression::Progression::load().streamed_hunters(diff, relays)
}

/// A wander target for a hunter of a resident floor, drawn on that floor's own facility `fac` (G20) rather than the
/// active floor: a random spot within `reach` cells of `from`, settled onto the nearest cell a hunter may stand on.
pub fn resident_wander_target(fac: &Facility, from: (f32, f32), reach: f32, rng: &mut Rng) -> Option<(f32, f32)> {
    let a = rng.next_f32() * std::f32::consts::TAU;
    let d = rng.next_f32() * reach.max(0.0);
    let want = (from.0 + a.cos() * d, from.1 + a.sin() * d);
    super::resident_hunters::settle_cell(fac, want, reach.ceil() as i32 + 2)
}

impl InfState {
    /// Every hunter the shelves hold: the level-0 ones the window does not load (`stored`) and those of the levels the
    /// runner left (`floor_hunters`). They exist in the run though no list shows them (the specialty bar counts them).
    pub(super) fn shelved_hunters(&self) -> impl Iterator<Item = &Brain> {
        self.stored.values().chain(self.floor_hunters.values().flatten())
    }
    pub(super) fn floor_hunters_mut(&mut self) -> &mut HashMap<i32, Vec<Brain>> {
        &mut self.floor_hunters
    }
    /// The identities of the level-0 hunters residency holds in the stores (outside the 3x3 window, not simulated). They
    /// exist on level 0 all the same, so Glyra counts as on the floor there (`Game::glyra_on_floor`, the tune gate). Empty
    /// off level 0, where a hunter that is not in the active list is on another floor's list or shelf, and for a guest,
    /// which stores nothing.
    pub(super) fn stored_hunter_ids(&self) -> Vec<&str> {
        if self.level != 0 {
            return Vec::new();
        }
        self.stored.values().map(|b| b.def.id.as_str()).collect()
    }
    pub(super) fn stored_mut(&mut self) -> &mut HashMap<String, Brain> {
        &mut self.stored
    }
    pub(super) fn loaded_mut(&mut self) -> &mut Vec<String> {
        &mut self.loaded
    }
    pub fn new(seed: &str, level: i32) -> InfState {
        InfState {
            seed: seed.to_string(),
            level,
            center: (0, 0),
            cache: HashMap::new(),
            prewarm_wait: 0,
            claimed: HashSet::new(),
            ids: live::WindowIds::default(),
            layout_pickups: HashMap::new(),
            discovered_chunks: HashSet::new(),
            discovered_rooms: HashSet::new(),
            markers: HashSet::new(),
            map_chunks: HashMap::new(),
            level0_seen: super::discovery::SeenCells::default(),
            seen: HashSet::new(),
            chunks_visited: HashSet::new(),
            safe: SafeRooms::default(),
            room: None,
            live: false,
            last_time: -1.0,
            pipes_pruned_at: 0.0,
            valve_hold: 0.0,
            valve_spin: 0.0,
            valve_outbox: Vec::new(),
            valve_requests: Vec::new(),
            burst_requests: Vec::new(),
            checkpoints_activated: 0,
            stored: HashMap::new(),
            loaded: Vec::new(),
            floor_hunters: HashMap::new(),
            offscreen_clock: 0.0,
            offscreen_tick: 0,
            spawns: HashMap::new(),
            walls: HashMap::new(),
            floor: DiscoveredFloor::default(),
            floor_chunks: 0,
            waypoint: None,
            routes: WaypointRoutes::default(),
            route_key: None,
            stack: LevelStack::default(),
            builds: 0,
            relabelled: false,
        }
    }

    pub fn origin(&self) -> (i32, i32) {
        ((self.center.0 - 1) * CHUNK, (self.center.1 - 1) * CHUNK)
    }
}

impl Game {
    pub fn is_infinite(&self) -> bool {
        self.inf.is_some()
    }

    /// Writes this window's live state into the persistent per-chunk sets: the claims by entity id
    /// and the discovered cells.
    fn sync_out(&mut self) {
        let Some(inf) = self.inf.as_mut() else { return };
        for (i, id) in inf.ids.relays.iter().enumerate() {
            if self.relays_taken.get(i) == Some(&true) {
                inf.claimed.insert(id.clone());
            }
        }
        for (i, id) in inf.ids.batteries.iter().enumerate() {
            if self.batteries_taken.get(i) == Some(&true) {
                inf.claimed.insert(id.clone());
            }
        }
        for (i, id) in inf.ids.coins.iter().enumerate() {
            if self.coins_taken.get(i) == Some(&true) {
                inf.claimed.insert(id.clone());
            }
        }
        // floor pickup gi is fac.pickups[src[gi]]; dropped ones (past the floor list) are not layout pickups
        for (gi, fi) in floor_source(&self.fac).into_iter().enumerate() {
            if self.pickups.get(gi).is_some_and(|p| p.taken) {
                inf.claimed.insert(inf.ids.pickups[fi].clone());
            }
        }
        // level 0 keeps its discovery as per-chunk bitmaps (level0_seen), written as the runner sees; the cell set is for the other levels
        if inf.level != 0 {
            let (ox, oy) = inf.origin();
            let l = inf.level;
            for (y, row) in self.discovered.iter().enumerate() {
                for (x, d) in row.iter().enumerate() {
                    if *d {
                        inf.seen.insert((l, x as i32 + ox, y as i32 + oy));
                    }
                }
            }
        }
    }

    /// Restores the claims and the discovered cells of the chunks now in the window.
    fn sync_in(&mut self) {
        let Some(inf) = self.inf.as_ref() else { return };
        for (i, id) in inf.ids.relays.iter().enumerate() {
            if let Some(t) = self.relays_taken.get_mut(i) {
                *t = inf.claimed.contains(id);
            }
        }
        for (i, id) in inf.ids.batteries.iter().enumerate() {
            if let Some(t) = self.batteries_taken.get_mut(i) {
                *t = inf.claimed.contains(id);
            }
        }
        for (i, id) in inf.ids.coins.iter().enumerate() {
            if let Some(t) = self.coins_taken.get_mut(i) {
                *t = inf.claimed.contains(id);
            }
        }
        for (gi, fi) in floor_source(&self.fac).into_iter().enumerate() {
            if let Some(p) = self.pickups.get_mut(gi) {
                p.taken = inf.claimed.contains(&inf.ids.pickups[fi]);
            }
        }
        let (ox, oy) = inf.origin();
        let l = inf.level;
        if l == 0 {
            // level 0: the bitmaps of the chunks the window covers, by absolute cell (`discovery::SeenCells`)
            inf.level0_seen.fill_window((ox, oy), &mut self.discovered);
            return;
        }
        for (y, row) in self.discovered.iter_mut().enumerate() {
            for (x, d) in row.iter_mut().enumerate() {
                *d = inf.seen.contains(&(l, x as i32 + ox, y as i32 + oy));
            }
        }
    }

    /// Builds the window around the current centre chunk and level.
    fn load_window(&mut self, content: &Content, catalog: &Catalog) {
        let Some(inf) = self.inf.as_mut() else { return };
        // the storey's window is the resident one (`keep_stack`) when it was built for this centre, so the change to
        // that storey builds nothing on its frame; otherwise it is built here
        let ahead = inf.stack.take(inf.level).filter(|r| r.center == inf.center);
        let (fac, fix) = match ahead {
            Some(r) => (r.fac, r.fix),
            None => {
                inf.builds += 1;
                let _window_build_perf = crate::app::perf::section("window build");
                infinite::window(&inf.seed, inf.center, inf.level, catalog, &content.loot, &mut inf.cache)
            }
        };
        let _window_adopt_perf = crate::app::perf::section("window adopt");
        self.adopt_window(fac, fix, content);
    }

    /// Everything after a window is obtained: the window becomes the live one (`self.fac`, `self.fix`) and the runner's
    /// claims, discovery, depth, focus, floor and safe zone follow it.
    fn adopt_window(&mut self, fac: Facility, fix: Fixtures, content: &Content) {
        if self.inf.is_none() {
            return;
        }
        self.fac = fac;
        self.fix = fix;
        let level = self.inf.as_ref().map_or(0, |i| i.level);
        self.fac.level = level;
        self.claim_window();
        // the focus is a prop index of the window just replaced: it would name another door in this one
        self.focus = None;
        self.adopt_floor(content);
        self.sync_in();
        self.sync_safe_zone();
        self.sync_sewer_gates();
    }

    /// Claims the ids of the live window (`self.fac`, `self.fix`), records the chunks it covers and reads the map
    /// discovery: shared by a loaded window and a relabelled one.
    fn claim_window(&mut self) {
        let Some(inf) = self.inf.as_mut() else { return };
        let fac = &self.fac;
        let fix = &self.fix;
        // claim ids for the new window: the layout's on level 0, else by absolute cell
        let (level, origin) = (inf.level, inf.origin());
        let lens = [fac.relays.len(), fac.batteries.len(), fac.pickups.len(), fix.coins.len()];
        let live_ids = live::window_ids(&mut inf.layout_pickups, &inf.seed, inf.center, level, lens);
        inf.live = live_ids.is_some();
        inf.ids = live_ids.unwrap_or_else(|| cell_ids(level, origin, fac, fix));
        // the middle chunk is the runner's, so its safe room is the only one the runner's rules read (Godot
        // `_safe_context` reads the runner's chunk); the other rooms of the window are only held in `inf.safe`
        inf.room = if inf.live {
            live::safe_room(&inf.seed, ChunkKey { cx: inf.center.0, cy: inf.center.1, level: 0 })
        } else {
            None
        };
        for dy in -1..=1 {
            for dx in -1..=1 {
                let k = ChunkKey {
                    cx: inf.center.0 + dx,
                    cy: inf.center.1 + dy,
                    level: inf.level,
                };
                inf.chunks_visited.insert(k);
                // Godot _discover_loaded_chunks: every chunk the window loads goes on the map, with its walls
                if inf.level == 0 {
                    inf.discovered_chunks.insert(k);
                    if !inf.walls.contains_key(&k) {
                        let walls = live::walls(&inf.seed, k).unwrap_or_default();
                        inf.walls.insert(k, walls);
                    }
                    // the full map's record, read from the chunk's cached facility now: the cache may drop it once the window moves on
                    if !inf.map_chunks.contains_key(&k) {
                        if let Some((cfac, _)) = inf.cache.get(&k) {
                            let (relay_ids, battery_ids) =
                                chunk_ids(inf.layout_pickups.get(&k), inf.live, k, cfac);
                            let record = MapChunk::of(k, cfac, &relay_ids, &battery_ids);
                            inf.map_chunks.insert(k, record);
                        }
                    }
                }
            }
        }
        if inf.level == 0 && inf.floor_chunks != inf.discovered_chunks.len() {
            inf.floor = DiscoveredFloor::build(&inf.discovered_chunks, &inf.walls);
            inf.floor_chunks = inf.discovered_chunks.len();
        }
        inf.route_key = None;
        self.depth = 1 - inf.level;
    }

    /// Puts the runner's safe room on the window's safe zone (`Facility.safe_zone`) while it is intact.
    /// The zone is what `in_safe_zone`, the hunters' target flag, the meters, the pad and the map read, so
    /// they follow the room without a change of their own. Off the level-0 layout the window keeps what
    /// its builder gave it.
    fn sync_safe_zone(&mut self) {
        let Some(inf) = self.inf.as_ref() else { return };
        if !inf.live {
            return;
        }
        let intact = inf
            .room
            .as_ref()
            .is_some_and(|r| !inf.safe.status(r).breached);
        let (x, y) = middle_safe_centre();
        self.fac.safe_zone = intact.then(|| Cell::new(x.floor() as i32, y.floor() as i32));
    }

    /// Enters level `level` with the runner at the window's centre lobby (a new run or a
    /// continued checkpoint).
    pub fn enter_infinite(&mut self, content: &Content, catalog: &Catalog) {
        self.load_window(content, catalog);
        // Godot puts the runner on the spawn cell's exact centre (infinite_route_test.gd:282,
        // streamed_facility_view_3d.gd spawn_position). `place` adds the finite floors' one-cell
        // south offset, so the infinite start sets the position after it.
        let start = dry_start(&self.fac);
        self.p.place(start);
        self.p.pos = start;
        self.reset_hunters(content);
        self.sync_specialty();
        // No NPC is made here: a room holds only its real players. Bots come from the admin panel
        // (`npc_spawn`); `npc_ensure_run` stays for callers that ask for the data/npcs.json run NPCs.
        self.fade = 1.0;
        self.reveal();
        self.reveal_start_room();
        self.discover_map();
        self.refresh_waypoint();
    }

    /// Rebuilds the live window from its seed and level: the runner stays where it stands and the window keeps its
    /// origin. The nine chunks leave the chunk cache and the layout read, so they are built again. The claims, the
    /// discovered cells and the safe rooms carry over, as they do when the window recentres.
    pub fn regen_window(&mut self, content: &Content, catalog: &Catalog) {
        if self.inf.is_none() {
            return;
        }
        self.sync_out();
        if let Some(inf) = self.inf.as_mut() {
            inf.stack.residents.clear();
            let (cx, cy, level) = (inf.center.0, inf.center.1, inf.level);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let k = ChunkKey {
                        cx: cx + dx,
                        cy: cy + dy,
                        level,
                    };
                    inf.cache.remove(&k);
                    inf.layout_pickups.remove(&k);
                }
            }
        }
        self.load_window(content, catalog);
    }

    /// Recentres the window once the runner leaves the middle chunk, then reads the map discovery
    /// (every frame, as Godot's `_update_map_discovery` does), clears the waypoint on arrival and plans its route.
    pub(super) fn stream_window(&mut self, content: &Content, catalog: &Catalog) {
        if self.inf.is_none() {
            return;
        }
        let from = self.inf.as_ref().map(|i| (i.center, i.level));
        self.recentre_window(content, catalog);
        if self.inf.as_ref().map(|i| (i.center, i.level)) != from && self.p.hidden.is_none() {
            // a new window: its doors are registered shut and the crosshair target is read again, so the prompt
            // names the door the runner is at now (the focus found before the recentre indexed the old props)
            let level = self.runner_level();
            let bodies: Vec<(f32, f32)> = std::iter::once(self.p.pos)
                .chain(self.hunters.iter().filter(|h| h.active).map(|h| h.pos))
                .chain(self.npcs.iter().filter(|n| n.level == level).map(|n| n.at))
                .collect();
            self.fac.update_doors(&[], &bodies, 0.0);
            self.focus = self.find_focus();
        }
        self.discover_map();
        self.clear_reached_waypoint();
        self.refresh_waypoint();
        let _window_residency_perf = crate::app::perf::section("window residency");
        self.keep_stack(content, catalog);
        self.prewarm_chunks(content, catalog);
        self.pipes_tick();
        let dt = self.safe_room_dt();
        let before = self.active_checkpoint().to_string();
        self.safe_room_step(dt);
        // a checkpoint newly made active saves the run (Godot `_accept_checkpoint`: autosave("checkpoint")); standing on in
        // the room does not save it again every frame (`_update_checkpoint` skips the `_last_checkpoint`)
        if !self.active_checkpoint().is_empty() && self.active_checkpoint() != before {
            self.autosave("checkpoint");
        }
    }

    /// Generates, one chunk every `gap_frames` frames (data/chunk_prewarm.json), the chunks the window will need when the runner crosses the edge
    /// of the middle chunk it is within `margin_m` metres of, into the window's chunk cache. `window` builds
    /// missing chunks synchronously, so without this the crossing's frame generated three to seven chunks at once (a
    /// 150 to 190 ms hitch measured on the web build, and the world popping in after it); with it the crossing is a
    /// cache hit. A chunk is a pure function of the seed, so the world is the same either way.
    fn prewarm_chunks(&mut self, content: &Content, catalog: &Catalog) {
        let pos = self.p.pos;
        let cfg = super::preload::chunk_prewarm();
        let Some(inf) = self.inf.as_mut() else { return };
        inf.prewarm_wait = inf.prewarm_wait.saturating_sub(1);
        let wanted = infinite::approach_chunks(inf.center, inf.level, pos, cfg.margin_m);
        let step = super::preload::prewarm_step(&cfg, inf.prewarm_wait, &wanted, |k| inf.cache.contains_key(k));
        for k in step {
            let made = infinite::chunk(&inf.seed, k, catalog, &content.loot);
            inf.cache.insert(k, made);
            inf.prewarm_wait = cfg.gap_frames;
        }
    }

    /// Keeps the neighbour levels resident beside the active one (`LevelStack`): the storey a flight or ladder within the
    /// data's `prefetch_distance` of the runner leads to, both sides of the active level while the runner is on a flight
    /// (`levels_to_keep`), and the storeys of the data's preload reach (game/preload.rs), nearest first and at most
    /// `builds_per_frame` a frame. Every resident shares the window's centre, so a walked change of storey is a relabel
    /// (`relabel`), not a load. The connections in reach are built this frame. Residents of another centre, or no longer
    /// kept, are dropped, and the farthest go while the held bytes pass the memory cap; none is dropped while the runner
    /// is on a flight.
    pub(super) fn keep_stack(&mut self, content: &Content, catalog: &Catalog) {
        if let Some(inf) = self.inf.as_mut() {
            if std::mem::take(&mut inf.relabelled) {
                return;
            }
        }
        let Some(inf) = self.inf.as_ref() else { return };
        let (level, center) = (inf.level, inf.center);
        let reach = super::transition::links().prefetch_distance;
        // on a flight, a ladder or in a fall both sides of the active level stay resident and none is evicted
        let on_flight = stair_height_at(&self.fac, self.p.pos).is_some();
        let on_link = on_flight || self.ladder.is_some() || self.p.falling();
        let mut near = levels_to_keep(level, self.p.pos, reach, &self.fac.stairs, on_link, infinite::level_allowed);
        // an open drop within reach (the falls void of level -1) is a connection to the level below, built this frame
        if self.fac.drop_within(self.p.pos.0, self.p.pos.1, reach) && infinite::level_allowed(level - 1) && !near.contains(&(level - 1)) {
            near.push(level - 1);
        }
        let cfg = super::preload::preload();
        // levels a walker is bound for stay built: both ends of every edge walked, each cross-floor goal, the floors of the
        // edge a hunter heads for (hunters of every list, and the bots on an edge)
        // (a scratch list reused every frame: taken here, cleared, put back before the function returns)
        let mut pinned: Vec<i32> = KEEP_PINNED.with(|c| std::mem::take(&mut *c.borrow_mut()));
        pinned.clear();
        for b in &self.hunters {
            b.pin_levels(&mut pinned);
        }
        let mut resident_levels: Vec<i32> = KEEP_RESIDENT.with(|c| std::mem::take(&mut *c.borrow_mut()));
        self.resident_hunters.levels_into(&mut resident_levels);
        for &l in &resident_levels {
            for slot in self.resident_hunters.list(l) {
                slot.brain.pin_levels(&mut pinned);
            }
        }
        KEEP_RESIDENT.with(|c| *c.borrow_mut() = resident_levels);
        for n in &self.npcs {
            if let Some(w) = n.walk.as_ref() {
                pinned.push(w.from.level);
                pinned.push(w.to.level);
            }
        }
        pinned.retain(|l| *l != level && infinite::level_allowed(*l));
        pinned.sort_unstable();
        pinned.dedup();
        let mut keep = super::preload::keep_levels(level, &cfg, infinite::level_allowed, &pinned);
        for t in &near {
            if !keep.contains(t) {
                keep.push(*t);
            }
        }
        let Some(inf) = self.inf.as_mut() else {
            KEEP_PINNED.with(|c| *c.borrow_mut() = pinned);
            return;
        };
        inf.stack.retain_center(center);
        let mut drop: Vec<i32> = Vec::new();
        if on_link {
            // nothing resident is evicted while a flight is walked, a ladder is climbed or a fall runs (only the active level's own label can not be resident)
            let kept: Vec<i32> = inf.stack.levels().into_iter().filter(|l| *l != level).collect();
            inf.stack.retain_levels(&kept);
        } else {
            let kept: Vec<i32> = inf.stack.levels().into_iter().filter(|l| keep.contains(l)).collect();
            inf.stack.retain_levels(&kept);
            let held: Vec<(i32, usize)> = inf.stack.residents.iter().map(|r| (r.level, r.bytes())).collect();
            drop = super::preload::to_evict(&held, &keep, level, &cfg, &pinned);
            let kept: Vec<i32> = inf.stack.levels().into_iter().filter(|l| !drop.contains(l)).collect();
            inf.stack.retain_levels(&kept);
        }
        // the connections in reach are built now; the rest of the keep set at the per-frame budget
        let mut build: Vec<i32> = near.iter().copied().filter(|t| !inf.stack.has(*t)).collect();
        let held_levels: Vec<i32> = inf.stack.levels().into_iter().chain(build.iter().copied()).collect();
        build.extend(super::preload::to_build(
            &keep.iter().copied().filter(|l| *l != level).collect::<Vec<_>>(),
            &held_levels,
            &cfg,
        ));
        for t in build {
            // a level the memory cap just dropped is not built again at once
            if drop.contains(&t) && !near.contains(&t) {
                continue;
            }
            inf.builds += 1;
            let _resident_build_perf = crate::app::perf::section("resident build");
            let (fac, fix) = infinite::window(&inf.seed, center, t, catalog, &content.loot, &mut inf.cache);
            inf.stack.put(Resident { level: t, center, fac, fix });
        }
        KEEP_PINNED.with(|c| *c.borrow_mut() = pinned);
        self.sync_resident_hunters(content);
    }

    /// An Axe breach of sewer pipe number `n` in `chunk` starts its flow (game/pipes.rs). A flow already running is left
    /// as it is. Returns the flow id, which the water mesh and `pipe_depth` read.
    pub(super) fn burst_pipe(&mut self, chunk: super::pipes::Chunk, n: usize) -> String {
        let id = format!("{}{n}", super::pipes::flow_prefix(chunk));
        // the breach is the runner's own Axe swing, so the runner's place is the breach point
        let at = self.p.pos;
        self.open_burst(&id, at, chunk.1);
        // a guest only shows the flow at once: the host confirms it and publishes it to everyone (net/valve_events.rs)
        if self.remote_hunters {
            if let Some(inf) = self.inf.as_mut() {
                inf.burst_requests.push(id.clone());
            }
        }
        id
    }

    /// Starts the burst flow `id` (a flow already running is left as it is), sounds it at `at` and, on the host,
    /// publishes it. The host runs this for its own swing and for a guest's confirmed one.
    pub(super) fn open_burst(&mut self, id: &str, at: (f32, f32), level: i32) {
        let now = self.time;
        self.pipes.start_flow(id, now);
        self.push_noise_at(crate::hunters::noise_events::NoiseKind::PipeBurst, at, level);
        if !self.remote_hunters {
            self.hunter_out.emit(
                crate::net::hunter_events::rules(),
                self.difficulty,
                crate::net::valve_events::BURST,
                crate::net::valve_events::burst_fields(id),
                crate::shared::clock::Instant::now(),
            );
        }
    }

    /// The pipe breaches a guest made since the last call, for the net layer to send to the host; the queue is emptied.
    pub fn take_burst_requests(&mut self) -> Vec<String> {
        self.inf.as_mut().map(|inf| std::mem::take(&mut inf.burst_requests)).unwrap_or_default()
    }

    /// Drops drained pipe flows about once a second of game time.
    fn pipes_tick(&mut self) {
        let now = self.time;
        let Some(inf) = self.inf.as_mut() else { return };
        if now - inf.pipes_pruned_at < 1.0 && now >= inf.pipes_pruned_at {
            return;
        }
        inf.pipes_pruned_at = now;
        self.pipes.prune(now);
    }

    /// The pipe water over the walkway cell `step` steps from the pipe of flow `id`, on outlet `outlet`, now, at this
    /// run's difficulty: the depth the wading reads (the swim state's input).
    pub fn pipe_depth(&self, id: &str, step: i32, outlet: usize) -> f32 {
        let name = format!("{:?}", self.difficulty).to_lowercase();
        self.pipes.depth_at_step(id, step, outlet, self.time, &name)
    }

    /// The seconds since the last safe-room step (`Game::time` advances once a frame). Zero on the first step.
    fn safe_room_dt(&mut self) -> f32 {
        let now = self.time;
        let Some(inf) = self.inf.as_mut() else { return 0.0 };
        let dt = if inf.last_time < 0.0 {
            0.0
        } else {
            (now - inf.last_time).clamp(0.0, SAFE_ROOM_MAX_DT)
        };
        inf.last_time = now;
        dt
    }

    /// The safe-room rules for the runner's room, once a frame (Godot `_update_checkpoint`,
    /// `_hunter_step_group` and `_update_safe_room_breaches`). Standing in the intact room activates its
    /// checkpoint and sends the hunters within SAFE_ROOM_LURE_REACH to investigate its centre. Hunters within
    /// SAFE_ROOM_BREACH_REACH of the centre batter it while the runner is inside, at the breach rate. `dt` is
    /// the time since the last step.
    pub(super) fn safe_room_step(&mut self, dt: f32) {
        let Some(inf) = self.inf.as_ref() else { return };
        if !inf.live {
            return;
        }
        let Some(room) = inf.room.clone() else { return };
        let centre = middle_safe_centre();
        let inside = !inf.safe.status(&room).breached && pdist(self.p.pos, centre) <= SAFE_ROOM_REACH;
        if inside {
            let fresh = self.inf.as_mut().is_some_and(|inf| {
                let before = inf.safe.active_checkpoint().to_string();
                inf.safe.activate(&room) && inf.safe.active_checkpoint() != before
            });
            if fresh {
                self.checkpoint_activated();
            }
            // a guest's hunter states come from the host's frames, so the lure is the host's to make
            let host = !self.remote_hunters;
            for h in self.hunters.iter_mut() {
                if host && h.active && pdist(h.pos, centre) <= SAFE_ROOM_LURE_REACH {
                    h.state = State::Investigate;
                    h.last_known = Some(centre);
                }
            }
        }
        // a guest's hunters are the host's frames: the host batters the room and publishes it (`publish_safe_room`)
        let breachers = if inside && !self.remote_hunters {
            self.hunters
                .iter()
                .filter(|h| pdist(h.pos, centre) <= SAFE_ROOM_BREACH_REACH)
                .count()
        } else {
            0
        };
        let Some(inf) = self.inf.as_mut() else { return };
        if breachers == 0 {
            // nobody is batting the room: the damage carried into the next step is dropped (Godot erases it)
            inf.safe.keep_breaching(&[]);
            return;
        }
        let damage = inf.safe.breach(&room, dt, breachers as u32);
        if damage > 0 {
            self.damage_live_room(&room, damage);
        }
    }

    /// Godot `stream_checkpoint` for the runner who made it: the run statistic, `restore_at_safe_zone` (stamina, lamp,
    /// exhaustion), `Sound.recharge(20)` and the "SAFE CHECKPOINT ACTIVE" line. The autosave Godot runs here
    /// (`autosave("checkpoint")`) is the save-trigger work's to add.
    fn checkpoint_activated(&mut self) {
        if let Some(inf) = self.inf.as_mut() {
            inf.checkpoints_activated += 1;
        }
        self.p.stamina = MAX_STAMINA;
        self.p.lamp = MAX_LAMP_CHARGE;
        self.p.exhausted = false;
        crate::audio::sound::recharge(20.0);
        self.say("SAFE CHECKPOINT ACTIVE");
    }

    /// Integrity off the runner's room (`damage_safe_room`), then the zone follows it. The moment the room falls the
    /// runner hears it (Godot `stream_safe`: "SAFE ROOM BREACHED" in the room the runner stands in).
    fn damage_live_room(&mut self, room: &SafeRoom, amount: i32) -> Status {
        let Some(inf) = self.inf.as_mut() else {
            return Status { integrity: 0, breached: false };
        };
        let was = inf.safe.status(room).breached;
        let status = inf.safe.damage(room, amount);
        self.sync_safe_zone();
        self.publish_safe_room(&room.id, status);
        if status.breached && !was {
            self.say("SAFE ROOM BREACHED");
        }
        status
    }

    /// Host: publishes a room's status to every guest as a `stream_safe_room` world event, so every player sees the
    /// room the same way. A guest's game does nothing here: its room changes only when the host's event arrives.
    fn publish_safe_room(&mut self, room_id: &str, status: Status) {
        if self.remote_hunters {
            return;
        }
        self.hunter_out.emit(
            crate::net::hunter_events::rules(),
            self.difficulty,
            crate::shared::safe_room::EVENT,
            crate::shared::safe_room::event_fields(room_id, status),
            crate::shared::clock::Instant::now(),
        );
    }

    /// Every machine: applies a host's `stream_safe_room` event to the room it names. Returns whether the event was a
    /// safe-room event. A fresh breach of the runner's own room is said here, as the host's breach is.
    pub fn apply_safe_room_event(&mut self, ev: &serde_json::Map<String, serde_json::Value>) -> bool {
        if ev.get("what").and_then(serde_json::Value::as_str) != Some(crate::shared::safe_room::EVENT) {
            return false;
        }
        // a malformed status changes nothing, but it is still this kind of event
        let Some((room_id, status)) = crate::shared::safe_room::parse_event(ev) else { return true };
        let Some(inf) = self.inf.as_mut() else { return true };
        let was = inf.safe.set_by_id(&room_id, status);
        let own = inf.room.as_ref().is_some_and(|r| r.id == room_id);
        self.sync_safe_zone();
        if own && status.breached && !was {
            self.say("SAFE ROOM BREACHED");
        }
        true
    }

    /// Godot `_admin_edit_checkpoint` (infinite_route_test.gd): DAMAGE CHECKPOINT takes 25 off the runner's safe room,
    /// REPAIR CHECKPOINT puts 25 back. `None` when the runner's chunk has no safe room (or off the level-0 layout).
    pub fn admin_edit_checkpoint(&mut self, amount: i32) -> Option<Status> {
        let room = self.live_room()?;
        if amount >= 0 {
            let status = self.inf.as_mut()?.safe.repair(&room, amount);
            self.sync_safe_zone();
            self.publish_safe_room(&room.id, status);
            Some(status)
        } else {
            Some(self.damage_live_room(&room, -amount))
        }
    }

    /// The runner's safe room for the admin status line (Godot `_admin_status`): `None` when the chunk has none.
    pub fn live_room_status(&self) -> Option<Status> {
        let room = self.live_room()?;
        self.inf.as_ref().map(|i| i.safe.status(&room))
    }

    /// Godot `_run_statistics.checkpoints_activated`: 0 off the infinite route.
    pub fn checkpoints_activated(&self) -> u32 {
        self.inf.as_ref().map_or(0, |i| i.checkpoints_activated)
    }

    /// The runner's safe room on the level-0 layout: `None` off level 0 and in a chunk that is not safe.
    fn live_room(&self) -> Option<SafeRoom> {
        self.inf.as_ref().filter(|i| i.live).and_then(|i| i.room.clone())
    }

    /// Godot `_on_tool_used` and `_accept_safe_repair` (infinite_route_test.gd): a crowbar (12) or a rescue axe (18)
    /// swung within SAFE_ROOM_REPAIR_REACH of the runner's safe room repairs it, breached or not.
    /// The caller passes a tool the runner owns. Returns true when a room was repaired.
    pub fn repair_safe_room(&mut self, tool: Item) -> bool {
        let amount = match tool {
            Item::Axe => 18,
            Item::Crowbar => 12,
            _ => return false,
        };
        let Some(room) = self.live_room() else { return false };
        if pdist(self.p.pos, middle_safe_centre()) > SAFE_ROOM_REPAIR_REACH {
            return false;
        }
        let Some(inf) = self.inf.as_mut() else { return false };
        let status = inf.safe.repair(&room, amount);
        self.sync_safe_zone();
        self.publish_safe_room(&room.id, status);
        self.say(&format!("SAFE ROOM REPAIRED // {}%", status.integrity));
        true
    }

    /// The checkpoint the runner last activated on this route ("" before any). Godot respawns the runner there
    /// (`_checkpoint_world_position`); nothing reads it yet, since a death ends a run in this build.
    pub fn active_checkpoint(&self) -> &str {
        self.inf.as_ref().map_or("", |i| i.safe.active_checkpoint())
    }

    pub(super) fn recentre_window(&mut self, content: &Content, catalog: &Catalog) {
        let rx = (self.p.pos.0 / CHUNK as f32).floor() as i32 - 1;
        let ry = (self.p.pos.1 / CHUNK as f32).floor() as i32 - 1;
        if rx == 0 && ry == 0 {
            return;
        }
        self.sync_out();
        if let Some(inf) = self.inf.as_mut() {
            inf.center = (inf.center.0 + rx, inf.center.1 + ry);
        }
        let (sx, sy) = ((rx * CHUNK) as f32, (ry * CHUNK) as f32);
        self.p.pos = (self.p.pos.0 - sx, self.p.pos.1 - sy);
        // a guest's hunters stay where the host's frames put them: the host shifts its own and sends the result
        if !self.remote_hunters {
            for h in self.hunters.iter_mut() {
                shift_brain(h, (-sx, -sy));
            }
            self.shift_resident_hunters((-sx, -sy));
        }
        // NPC teammates share the window frame, so they shift with it (game/npc.rs)
        for n in self.npcs.iter_mut() {
            n.shift((-sx, -sy));
        }
        if let Some(inf) = self.inf.as_mut() {
            // a resident built for the old centre is stale: dropped, and rebuilt below on a flight
            inf.stack.retain_center(inf.center);
        }
        // door swing is kept across the rebuild: the same doors, their cells moved by the window
        let swing = self.fac.door_swing();
        self.load_window(content, catalog);
        self.fac.restore_door_swing(&swing, (-rx * CHUNK, -ry * CHUNK));
        if stair_height_at(&self.fac, self.p.pos).is_some() {
            // on a flight the neighbour is built at the new centre this frame (one hitch; the chunk cache makes it cheap)
            let _window_residency_perf = crate::app::perf::section("window residency");
            self.keep_stack(content, catalog);
        }
        // a resident list is fitted into the window by the facility built for the new centre: the old one is stale (its
        // cells are the old window's), and a spot drawn from it can be a drop or a wall in the new one. A level that left
        // the stack has no facility here and is stored by the next `keep_stack`.
        if !self.remote_hunters {
            self.fit_resident_lists_into_window();
        }
        // bots that travelled coarsely outside the old window are set down on the floor they now stand on
        self.npc_rejoin_window();
        if self.inf.as_ref().is_some_and(|i| i.level == 0) {
            // level 0: residency decides who is in the window; a hunter that leaves it is stored, not respawned
            self.residency(content);
            return;
        }
        self.respawn_outside_window();
    }

    /// Off level 0: the hunters that stand outside the window (or in a wall) come back in out of sight.
    fn respawn_outside_window(&mut self) {
        // a guest's hunters are placed by the host's frames: it does not move them
        if self.remote_hunters {
            return;
        }
        let size = (CHUNK * 3) as f32;
        let mut rng = Rng::new(&format!(
            "{}:RESPAWN:{}",
            self.level_seed(),
            self.time as i32
        ));
        for i in 0..self.hunters.len() {
            let p = self.hunters[i].pos;
            // a hunter on a flight or ladder owns its position (its walk is in window metres and was shifted with it)
            if self.hunters[i].on_edge() {
                continue;
            }
            if p.0 < 0.0
                || p.1 < 0.0
                || p.0 >= size
                || p.1 >= size
                || !self.fac.hunter_fits(p.0, p.1)
            {
                if let Some(spot) = self.spawn_spot(&mut rng) {
                    self.hunters[i].pos = spot;
                    self.hunters[i].clear_path();
                }
            }
        }
    }

    /// Moves the run to storey `to` on the frame it is asked for: the active hunters go to the shelf, the window of `to`
    /// is loaded, `arrive` places the body, and the shelf gives the level's hunters back. The callers are `respawn_on_level`
    /// (admin jump) and the test helper `go_to_level`; ladders, drops and flights move the body in absolute y and the label
    /// follows by `relabel` (`walk_relabel`).
    pub(super) fn change_storey(&mut self, to: i32, content: &Content, catalog: &Catalog, arrive: impl FnOnce(&mut Game)) {
        self.drain_resident_hunters();
        self.sync_out();
        // the level being left keeps its hunters at their absolute cells, to be found again on the way back
        self.shelve_active_hunters();
        if let Some(inf) = self.inf.as_mut() {
            inf.level = to;
        }
        self.p.floor_y = super::level_stack::floor_y_of(to);
        self.load_window(content, catalog);
        arrive(&mut *self);
        // the level's own hunters come back where they were; a level not visited before gets its lineup
        self.unshelve_active_hunters(content);
        self.announce(&format!("LEVEL {to}"));
        // the bar is the new floor's on this frame, not on the next step (the floor the runner left must not be drawn)
        self.sync_specialty();
    }

    /// Respawn semantics on level `to` (admin level jump): the floor is rebuilt, the body stands on the start cell of
    /// that level, one label change, one announce, one autosave. False and no change for a guest, a finite route, a
    /// level that is not allowed or the level already active.
    pub fn respawn_on_level(&mut self, to: i32, content: &Content, catalog: &Catalog) -> bool {
        if self.remote_hunters || !self.inf.as_ref().is_some_and(|i| i.level != to) || !infinite::level_allowed(to) {
            return false;
        }
        self.change_storey(to, content, catalog, |g| {
            let s = g.fac.start.center();
            g.p.pos = g.fac.nearest_open_or(s.0, s.1, PLAYER_RADIUS);
            g.p.set_abs_y(floor_y_of(to));
            g.p.stand_on_ground(&g.fac);
        });
        self.autosave("level");
        true
    }

    /// Test helper: the level change of a ladder, a drop or a respawn, to storey `to`, the body put on the open cell
    /// nearest `at` facing `yaw`, standing on the floor. Through `change_storey`; nothing in the game calls it.
    #[cfg(test)]
    pub(super) fn go_to_level(&mut self, to: i32, at: (f32, f32), yaw: f32, content: &Content, catalog: &Catalog) {
        self.change_storey(to, content, catalog, |g| {
            g.p.pos = g.fac.nearest_open_or(at.0, at.1, PLAYER_RADIUS);
            g.p.yaw = yaw;
            g.p.height = 0.0;
        });
    }

    /// Relabels the active level to the resident neighbour `to` (continuous stairs, docs/STAIRS_CONTINUOUS_DESIGN.md): the
    /// two windows swap roles, the body keeps its x, z, absolute y, velocity and facing, and nothing is loaded. False and
    /// no change when `to` is not resident at the window's centre.
    pub(super) fn relabel(&mut self, to: i32, content: &Content, _catalog: &Catalog) -> bool {
        let Some(inf) = self.inf.as_ref() else { return false };
        if to == inf.level || !inf.stack.get(to).is_some_and(|r| r.center == inf.center) {
            return false;
        }
        self.sync_out();
        // the level being left keeps its hunters at their absolute cells, to be found again on the way back
        let Some(inf) = self.inf.as_mut() else { return false };
        let Some(r) = inf.stack.take(to) else { return false };
        let Some(inf) = self.inf.as_mut() else { return false };
        let old = Resident {
            level: inf.level,
            center: inf.center,
            fac: std::mem::replace(&mut self.fac, r.fac),
            fix: std::mem::replace(&mut self.fix, r.fix),
        };
        inf.stack.put(old);
        inf.relabelled = true;
        let from = inf.level;
        inf.level = to;
        self.fac.level = to;
        self.p.rebase_floor(floor_y_of(to));
        self.claim_window();
        self.focus = None;
        self.adopt_floor(content);
        self.sync_in();
        self.sync_safe_zone();
        let level = self.runner_level();
        let bodies: Vec<(f32, f32)> = std::iter::once(self.p.pos)
            .chain(self.hunters.iter().filter(|h| h.active).map(|h| h.pos))
            .chain(self.npcs.iter().filter(|n| n.level == level).map(|n| n.at))
            .collect();
        self.fac.update_doors(&[], &bodies, 0.0);
        self.focus = self.find_focus();
        self.relabel_hunters(from, to, content);
        self.world_nb.relabel(&mut self.world, to);
        // cheap: the parts are cached, and a level change is not a jump for the render
        self.world_dirty = true;
        self.announce(&format!("LEVEL {to}"));
        // the hunters of the label just taken are the active list now: the bar follows on this frame
        self.sync_specialty();
        self.autosave("level");
        true
    }

    /// A walked flight changes the level label on the frame the body's absolute height crosses into the neighbour
    /// (`level_at`, the hysteresis of world/vertical.rs): once per crossing, with no fade, no hold and no move of the body.
    pub(super) fn walk_relabel(&mut self, content: &Content, catalog: &Catalog) {
        // the label follows the body's absolute height on a flight, a ladder and in a fall alike (no `change_storey`)
        let Some(inf) = self.inf.as_ref() else { return };
        let cur = inf.level;
        let to = level_at(self.p.y(), cur);
        if to != cur && infinite::level_allowed(to) {
            self.relabel(to, content, catalog);
        }
    }

    fn spawn_spot(&self, rng: &mut Rng) -> Option<(f32, f32)> {
        let cands: Vec<Cell> = self
            .fac
            .floor_cells()
            .into_iter()
            .filter(|c| {
                let p = c.center();
                let d = super::pdist(p, self.p.pos);
                d >= 12.0
                    && d <= 34.0
                    && self.fac.hunter_may_spawn(*c)
                    && !self.fac.has_line_of_sight(p, self.p.pos)
            })
            .collect();
        if cands.is_empty() {
            return None;
        }
        Some(cands[rng.next_index(cands.len())].center())
    }

    /// The level's lineup, built from nothing (a new run, or the first visit to a level). On level 0 the hunters are the layout's spawns that residency admits (`residency`), and
    /// Nyxie and Solyth are not added (Godot's streamed lineup has neither). On the old builder's levels: route hunters
    /// for the current relay count, plus Nyxie and Solyth.
    fn reset_hunters(&mut self, content: &Content) {
        self.hunters.clear();
        self.reset_director();
        // a guest only shows the hunters the host's frames send: it builds no lineup of its own
        if super::guest_gate::is_display_only(self) {
            return;
        }
        if self.inf.as_ref().is_some_and(|i| i.level == 0) {
            if let Some(inf) = self.inf.as_mut() {
                inf.loaded.clear();
            }
            self.residency(content);
            return;
        }
        let seed = self.level_seed();
        let mut rng = Rng::new(&format!("{seed}:SPECIALS"));
        for (id, tag) in [("nyxie", "NYXIE"), ("solyth", "SOLYTH")] {
            if let Some(def) = content.hunter(id) {
                let p = self.spawn_spot(&mut rng).unwrap_or(self.fac.start.center());
                let mut b = Brain::new(def.clone(), p, &format!("{seed}:{tag}"), self.difficulty);
                if let Gimmick::Solyth(s) = &mut b.gimmick {
                    s.heat_locked = self.meters.heat_locked;
                }
                self.settle_new_brain(&mut b);
                self.hunters.push(b);
            }
        }
        self.ensure_hunters(content);
    }

    /// Adds route hunters until the population matches the relays recovered so far. On level 0 this is the
    /// residency pass (admission, then loading and storing); the old builder's levels keep the spawn-spot lineup.
    pub(super) fn ensure_hunters(&mut self, content: &Content) {
        self.ensure_hunters_at(content, None);
    }

    /// `ensure_hunters`, with the place a hunter this pass wakes investigating: `wake` (where a bot recovered a relay),
    /// else where the runner stands.
    pub(super) fn ensure_hunters_at(&mut self, content: &Content, wake: Option<(f32, f32)>) {
        if super::guest_gate::is_display_only(self) {
            return;
        }
        if self.inf.as_ref().is_some_and(|i| i.level == 0) {
            self.residency_at(content, wake);
            return;
        }
        // the level's danger adds hunters deeper down (data/danger.json `depth`)
        let want = population(self.difficulty, self.relays_collected) + self.difficulty.danger(self.depth).extra_hunters.max(0);
        let have = self
            .hunters
            .iter()
            .filter(|h| {
                !matches!(h.def.id.as_str(), "nyxie" | "solyth") && h.def.category != "secret" && !h.def.id.starts_with("void_")
            })
            .count() as i32;
        let seed = self.level_seed();
        for n in have..want {
            let mut rng = Rng::new(&format!("{seed}:SPAWN:{n}"));
            let Some(p) = self.spawn_spot(&mut rng) else {
                break;
            };
            let pick = crate::hunters::pick_for_floor(
                content,
                &format!("{seed}:H{n}"),
                self.depth,
                self.difficulty,
            );
            let mut b = Brain::new(
                content.hunters[pick].clone(),
                p,
                &format!("{seed}:HUNTER:{n}"),
                self.difficulty,
            );
            let at = wake.unwrap_or(self.p.pos);
            b.activate_at(at);
            self.settle_new_brain(&mut b);
            self.hunters.push(b);
        }
        self.void_incursion(content);
    }

    /// The Void incursion (hunter_roster.gd `has_void_incursion`, data/hunters/draw_pool.json `incursion`): a floor deep
    /// enough may bring one void hunter beside its lineup, rolled once on the floor's own stream so the same floor always
    /// rolls the same. Host or solo only (`ensure_hunters_at` returns first on a guest).
    fn void_incursion(&mut self, content: &Content) {
        if self.hunters.iter().any(|h| h.def.id.starts_with("void_")) {
            return;
        }
        let seed = self.level_seed();
        let depth = self.depth.max(0) as u32;
        let roll = Rng::new(&format!("{seed}:VOID-INCURSION:D{depth}")).next_float();
        if !crate::content::hunters::incursion().brings_void(depth, roll as f32) {
            return;
        }
        self.spawn_void_hunter(content, &format!("{seed}:VOID-IDENTITY:D{depth}"), None);
    }

    /// Adds one void hunter (an id starting `void_`) picked on the `tag` stream, awake at the runner: at a spawn spot of
    /// the floor, or with `near = Some((at, dist))` at the open point nearest one `dist` cells from `at`. Host or solo
    /// only: a guest's hunters come from the host's frames. Returns whether one was added.
    pub(super) fn spawn_void_hunter(&mut self, content: &Content, tag: &str, near: Option<((f32, f32), f32)>) -> bool {
        if super::guest_gate::is_display_only(self) {
            return false;
        }
        let voids: Vec<usize> = (0..content.hunters.len()).filter(|i| content.hunters[*i].id.starts_with("void_")).collect();
        if voids.is_empty() {
            return false;
        }
        let mut rng = Rng::new(tag);
        let pick = voids[rng.next_index(voids.len())];
        let spot = match near {
            Some((at, dist)) => {
                let a = rng.next_float() as f32 * std::f32::consts::TAU;
                self.fac.nearest_open(at.0 + a.cos() * dist, at.1 + a.sin() * dist, 0.3)
            }
            None => self.spawn_spot(&mut rng),
        };
        let Some(p) = spot else { return false };
        let mut b = Brain::new(content.hunters[pick].clone(), p, &format!("{tag}:HUNTER"), self.difficulty);
        b.activate_at(self.p.pos);
        self.settle_new_brain(&mut b);
        self.hunters.push(b);
        // the residency ids run parallel to the hunter list, and the director's ids index it
        if let Some(inf) = self.inf.as_mut() {
            inf.loaded.resize(self.hunters.len(), String::new());
        }
        self.reset_director();
        true
    }

    /// Anti-AFK void pressure (anti_idle.gd, data/anti_idle.json): a party that stands still past its grace builds
    /// pressure; past `steer_pressure` the void hunters are sent to it, and at `spawn_pressure` one is spawned
    /// `spawn_distance` cells away. The party is the local runner (a hidden runner counts as still). Host or solo only:
    /// a guest passes `is_host = false` and nothing changes; a dedicated room (no local runner) does not run it.
    pub(super) fn step_anti_idle(&mut self, dt: f32, content: &Content) {
        if self.no_local_runner || self.report.is_some() {
            return;
        }
        let is_host = !super::guest_gate::is_display_only(self);
        // a hidden runner is still; a climber is moving only on a frame its climb moved it (the climb leaves `move_vel`
        // as it was at the mount, so the walk speed would read stale); otherwise the walk speed
        let speed = if self.p.hidden.is_some() {
            0.0
        } else if self.ladder.is_some() {
            if self.p.noise > 0.0 { crate::world::vertical::LADDER_CLIMB_SPEED } else { 0.0 }
        } else {
            self.p.move_vel.0.hypot(self.p.move_vel.1)
        };
        let voids = self.hunters.iter().filter(|h| h.active && h.def.id.starts_with("void_")).count();
        let cfg = super::anti_idle::cfg();
        match self.anti_idle.step(dt, is_host, speed, voids, self.difficulty, cfg) {
            super::anti_idle::Action::Spawn => {
                let tag = format!("{}:ANTI-IDLE:{}", self.level_seed(), self.time.to_bits());
                self.spawn_void_hunter(content, &tag, Some((self.p.pos, cfg.spawn_distance)));
            }
            super::anti_idle::Action::Steer => {
                let at = self.p.pos;
                for h in self.hunters.iter_mut().filter(|h| h.active && h.def.id.starts_with("void_")) {
                    if h.state != State::Chase {
                        h.state = State::Investigate;
                        h.last_known = Some(at);
                    }
                }
            }
            super::anti_idle::Action::None => {}
        }
    }

    /// The public route's handover (net/public_reform.rs): this game stops showing the old host's hunters and runs them.
    /// The hunters the old host's frames made here (`shadow`, display only) are taken out of the list, the host's own
    /// lineup is topped up (`ensure_hunters`: on level 0 the residency pass admits the layout's spawns, and a shadow has no
    /// identity in it, so leaving the shadows in would double each one), and every shadow's state is put on the lineup hunter
    /// of its kind that the top-up made: place, facing, state, awake or not, stun, slow, disguise, level and height. A hunter
    /// of a kind the top-up did not make (the lineup already had it, or this runner's relay count admits fewer than the old
    /// host's did) stays as it was shown, as an ordinary hunter. What the old host's brain knew beyond what a frame carries
    /// (its path, its search memory, its timers) is not known here: a hunter starts from a clean brain at the place it was
    /// last seen.
    pub fn become_host(&mut self, content: &Content) {
        if !self.remote_hunters {
            return;
        }
        // the residency ids run parallel to the hunter list: they leave with the shadows
        let old = std::mem::take(&mut self.hunters);
        let mut ids = self.inf.as_mut().map(|i| std::mem::take(&mut i.loaded)).unwrap_or_default();
        ids.resize(old.len(), String::new());
        let (mut shadows, mut kept, mut kept_ids) = (Vec::new(), Vec::new(), Vec::new());
        for (h, id) in old.into_iter().zip(ids) {
            if h.shadow {
                shadows.push(h);
            } else {
                kept.push(h);
                kept_ids.push(id);
            }
        }
        self.hunters = kept;
        if let Some(inf) = self.inf.as_mut() {
            inf.loaded = kept_ids;
        }
        // the other levels' hunters become ordinary residents: they are simulated beside the active level's
        for l in self.resident_hunters.levels() {
            for slot in self.resident_hunters.list_mut(l).iter_mut() {
                slot.brain.shadow = false;
                slot.brain.display_only = false;
            }
        }
        self.remote_hunters = false;
        // the top-up appends what it makes after the hunters already in the list
        let first_new = self.hunters.len();
        self.ensure_hunters(content);
        let mut taken: Vec<usize> = Vec::new();
        let mut extras: Vec<Brain> = Vec::new();
        for mut shown in shadows {
            shown.shadow = false;
            shown.display_only = false;
            let slot = (first_new..self.hunters.len()).find(|i| !taken.contains(i) && self.hunters[*i].def.id == shown.def.id);
            match slot {
                Some(i) => {
                    taken.push(i);
                    let h = &mut self.hunters[i];
                    h.pos = shown.pos;
                    h.facing = shown.facing;
                    h.state = shown.state;
                    h.active = shown.active;
                    h.stun = shown.stun;
                    h.slow_factor = shown.slow_factor;
                    h.disguise = shown.disguise.clone();
                    h.level = shown.level;
                    h.y = shown.y;
                    h.clear_path();
                }
                None => extras.push(shown),
            }
        }
        self.hunters.extend(extras);
        if let Some(inf) = self.inf.as_mut() {
            inf.loaded.resize(self.hunters.len(), String::new());
        }
        self.reset_director();
    }

    /// Level 0: the window's hunter residency (Godot `_refresh_hunter_stream_work`, see residency.rs). The hunters that exist
    /// are the stored ones, and the loaded ones at their absolute cells. Admission takes the layout spawns of the active
    /// chunks up to the population. A wanted hunter that stands in the runner's 3x3 window is loaded and the rest stay
    /// stored. A new hunter is created dormant at its spawn cell (Godot `ensure_hunter`), and wakes at the runner on its
    /// first load, where the relay activation used to wake a spawn. A change in who is loaded resets the director, whose
    /// ids are indices into `hunters`.
    pub(super) fn residency(&mut self, content: &Content) {
        self.residency_at(content, None);
    }

    /// `residency`, with the place a hunter loaded for the first time wakes investigating: `wake`, else the runner.
    pub(super) fn residency_at(&mut self, content: &Content, wake: Option<(f32, f32)>) {
        if super::guest_gate::is_display_only(self) {
            return;
        }
        let (seed, runner, origin, limit) = match self.inf.as_ref() {
            Some(i) if i.level == 0 => (
                i.seed.clone(),
                i.center,
                i.origin(),
                population(self.difficulty, self.relays_collected),
            ),
            _ => return,
        };
        let difficulty = self.difficulty;
        let runner_pos = wake.unwrap_or(self.p.pos);
        let o = (origin.0 as f32, origin.1 as f32);
        let Some(inf) = self.inf.as_mut() else { return };
        // the loaded list follows the hunter list (a caller outside this module may have pushed or removed one)
        inf.loaded.resize(self.hunters.len(), String::new());

        // the hunters that exist: the stored ones, then the loaded ones at their absolute cells
        let mut recs: Vec<Record> = inf
            .stored
            .iter()
            .map(|(id, b)| Record {
                id: id.clone(),
                hunter_id: b.def.id.clone(),
                pos: b.pos,
                active: b.active,
            })
            .collect();
        for (b, id) in self.hunters.iter().zip(&inf.loaded) {
            if !id.is_empty() {
                recs.push(Record {
                    id: id.clone(),
                    hunter_id: b.def.id.clone(),
                    pos: (b.pos.0 + o.0, b.pos.1 + o.1),
                    active: b.active,
                });
            }
        }

        // the candidates: the layout spawns of the active chunks, in chunk order
        let active = residency::active_chunks(runner, &recs);
        let mut keys: Vec<(i32, i32)> = active.iter().copied().collect();
        keys.sort();
        let mut cands: Vec<Cand> = Vec::new();
        for (cx, cy) in keys {
            let k = ChunkKey { cx, cy, level: 0 };
            let spawns = inf
                .spawns
                .entry(k)
                .or_insert_with(|| live::hunter_spawns(&seed, k).unwrap_or_default());
            for s in spawns.iter() {
                cands.push(Cand {
                    id: s.id.clone(),
                    hunter_id: s.hunter_id.to_string(),
                    pos: ((cx * CHUNK + s.cell.x) as f32 + 0.5, (cy * CHUNK + s.cell.y) as f32 + 0.5),
                });
            }
        }

        let adm = residency::admit(&seed, runner, &recs, &active, &cands, limit as i32);
        for c in &adm.created {
            if let Some(def) = content.hunter(&c.hunter_id) {
                let b = Brain::new(def.clone(), c.pos, &format!("{seed}:{}", c.id), difficulty);
                inf.stored.insert(c.id.clone(), b);
            }
        }

        // the loaded set: a hunter no longer wanted, or no longer in the window, is stored; a wanted one in the window is loaded
        let wanted: HashSet<String> = adm.resident.iter().cloned().collect();
        let mut changed = false;
        let mut kept_ids: Vec<String> = Vec::new();
        let old_hunters = std::mem::take(&mut self.hunters);
        let old_ids = std::mem::take(&mut inf.loaded);
        for (mut b, id) in old_hunters.into_iter().zip(old_ids) {
            if !id.is_empty() && !wanted.contains(&id) {
                shift_brain(&mut b, o);
                inf.stored.insert(id, b);
                changed = true;
            } else {
                self.hunters.push(b);
                kept_ids.push(id);
            }
        }
        let mut arrivals: Vec<String> = adm.resident.clone();
        arrivals.sort();
        for id in arrivals {
            if kept_ids.contains(&id) {
                continue;
            }
            let Some(mut b) = inf.stored.remove(&id) else { continue };
            shift_brain(&mut b, (-o.0, -o.1));
            // a hunter that drifted while stored comes in on a walkable, dry cell
            let radius = super::resident_hunters::offscreen_rules().entry_radius;
            if let Some(q) = super::resident_hunters::settle_cell(&self.fac, b.pos, radius) {
                b.pos = q;
            }
            if !b.active {
                b.activate_at(runner_pos);
            }
            self.hunters.push(b);
            kept_ids.push(id);
            changed = true;
        }
        inf.loaded = kept_ids;
        if changed {
            self.reset_director();
        }
        self.park_level0_nyxie();
    }

    /// Whether the level-0 Nyxie exists anywhere: the active list, the shelf of level 0 (the runner is on another level),
    /// or the resident list of level 0 (the runner stands on a flight beside it).
    fn level0_nyxie_exists(&self) -> bool {
        let level0 = |b: &Brain| matches!(&b.gimmick, Gimmick::Nyxie(n) if n.dead_only);
        self.hunters.iter().any(level0)
            || self.resident_hunters.list(0).iter().any(|s| level0(&s.brain))
            || self
                .inf
                .as_ref()
                .is_some_and(|i| i.floor_hunters.get(&0).is_some_and(|v| v.iter().any(level0)))
    }

    /// Level 0 of the Infinite route has no Nyxie in its lineup (Godot's streamed lineup has neither special, the layout
    /// never draws her, and the hunter activation count keeps even the route's own hunters out until the first relays), so
    /// before this a dead flashlight had nobody to bring there. With `ignore_activation_gate` on
    /// (data/hunters/nyxie_wake.json) the first frame the runner's flashlight is dead on level 0 makes one dormant Nyxie,
    /// marked `dead_only`, whatever the difficulty and the relays recovered; it is kept for the run (the shelf and the
    /// stairs carry her like any id-less hunter). Dormant she is unseen and uncatchable, as every sleeping hunter is. Her
    /// dead-lamp wake (and the 10 s lamp kill it grows into) is the ordinary one; she has no 120 s darkness wake, which level
    /// 0 never had. She is not a layout spawn and carries no residency id, so the hunter selection, the draw pool, the
    /// residency admission and the layout are untouched, and she is pushed after the hunters already in the list. A room
    /// with no local runner (a dedicated host) and a guest never make her: the lamp she reads is the runner's.
    pub(super) fn keep_level0_nyxie(&mut self, content: &Content) {
        self.keep_level0_nyxie_with(content, crate::hunters::gimmicks::nyxie::wake_rules());
    }

    /// `keep_level0_nyxie` with the wake rules given (the tests pass edited ones).
    fn keep_level0_nyxie_with(&mut self, content: &Content, rules: &crate::hunters::gimmicks::nyxie::WakeRules) {
        // the cheap tests first: this runs every frame
        if self.p.lamp > 0.0 || !rules.ignore_activation_gate {
            return;
        }
        if super::guest_gate::is_display_only(self) || self.no_local_runner {
            return;
        }
        if !self.inf.as_ref().is_some_and(|i| i.level == 0) || self.level0_nyxie_exists() {
            return;
        }
        let Some(def) = content.hunter("nyxie") else { return };
        let seed = self.level_seed();
        let mut rng = Rng::new(&format!("{seed}:SPECIALS"));
        let p = self.spawn_spot(&mut rng).unwrap_or(self.fac.start.center());
        let mut b = Brain::new(def.clone(), p, &format!("{seed}:NYXIE"), self.difficulty);
        if let Gimmick::Nyxie(n) = &mut b.gimmick {
            n.dead_only = true;
        }
        self.settle_new_brain(&mut b);
        self.push_active(Slot { id: String::new(), brain: b });
    }

    /// A dormant level-0 Nyxie that the window moved away from, or that stands in a wall, comes back in out of sight (the
    /// window moves under her; she is not stored by residency). One that is awake is left alone.
    fn park_level0_nyxie(&mut self) {
        let size = (CHUNK * 3) as f32;
        let seed = self.level_seed();
        let mut rng = Rng::new(&format!("{seed}:PARK:{}", self.time as i32));
        for i in 0..self.hunters.len() {
            let h = &self.hunters[i];
            if h.active || !matches!(&h.gimmick, Gimmick::Nyxie(n) if n.dead_only) {
                continue;
            }
            let p = h.pos;
            let out = p.0 < 0.0 || p.1 < 0.0 || p.0 >= size || p.1 >= size || !self.fac.hunter_fits(p.0, p.1);
            if out {
                if let Some(spot) = self.spawn_spot(&mut rng) {
                    self.hunters[i].pos = spot;
                    self.hunters[i].clear_path();
                }
            }
        }
    }

    pub fn save_checkpoint(&self) {
        crate::shared::checkpoint::save(&self.checkpoint_record());
    }

    /// `save_checkpoint` plus the body's position (`checkpoint::save_with_position`): absolute cells x and z (the
    /// window origin added, so it means the same place after a reload) and the feet height.
    pub fn save_checkpoint_with_position(&self) {
        let (ox, oz) = self.window_origin();
        let pos = [self.p.pos.0 + ox as f32, self.p.y(), self.p.pos.1 + oz as f32];
        crate::shared::checkpoint::save_with_position(&self.checkpoint_record(), pos);
    }

    fn checkpoint_record(&self) -> crate::shared::checkpoint::Checkpoint {
        crate::shared::checkpoint::Checkpoint {
            run_seed: self.run_seed,
            room_text: self.room_text.clone(),
            depth: self.inf.as_ref().map(|i| i.level).unwrap_or(self.depth),
            y: self.inf.as_ref().map(|_| self.p.y()),
            score: self.score,
            coins: self.coins,
            difficulty: self.difficulty,
            infinite: self.inf.is_some(),
        }
    }

    pub fn chunks_visited(&self) -> usize {
        self.inf
            .as_ref()
            .map(|i| i.chunks_visited.len())
            .unwrap_or(0)
    }

    pub(super) fn floor_pickups(&self) -> Vec<FloorPickup> {
        self.fac
            .pickups
            .iter()
            .filter_map(|(c, k)| {
                let (item, n) = floor_item(k)?;
                Some(FloorPickup {
                    pos: c.center(),
                    item,
                    n,
                    taken: false,
                })
            })
            .collect()
    }

    /// Whether the runner has seen level-0 absolute cell (`ax`, `ay`). False off level 0.
    pub fn level0_cell_seen(&self, ax: i32, ay: i32) -> bool {
        self.inf.as_ref().is_some_and(|i| i.level == 0 && i.level0_seen.is_seen(ax, ay))
    }

    /// Marks a level-0 window cell seen in the persistent bitmaps (the grid `discovered` is the window's view of them). A cell
    /// off the window, or any cell off level 0, does nothing. True when the cell was new.
    pub(super) fn mark_level0_seen(&mut self, x: i32, y: i32) -> bool {
        let Some(inf) = self.inf.as_mut() else { return false };
        if inf.level != 0 {
            return false;
        }
        let (ox, oy) = inf.origin();
        inf.level0_seen.mark(x + ox, y + oy)
    }

    /// The start lobby is on the map from the first frame (the safe room, with a cell of wall around it), whatever the line of
    /// sight finds: the runner begins in it, and the map should not open on a blank.
    fn reveal_start_room(&mut self) {
        // level 0 only: the sewer and the Inferno start with what the line of sight finds, as before
        if self.inf.as_ref().map_or(true, |i| i.level != 0) {
            return;
        }
        let Some(sz) = self.fac.safe_zone else { return };
        let r = (SAFE_ZONE_RADIUS + 1.0).ceil() as i32;
        let c = sz.center();
        for y in (sz.y - r).max(0)..=(sz.y + r).min(self.fac.rows - 1) {
            for x in (sz.x - r).max(0)..=(sz.x + r).min(self.fac.cols - 1) {
                if pdist((x as f32 + 0.5, y as f32 + 0.5), c) <= SAFE_ZONE_RADIUS + 1.0 {
                    self.discovered[y as usize][x as usize] = true;
                    self.mark_level0_seen(x, y);
                }
            }
        }
    }

    /// The admin MAP REVEAL: every cell of the window is seen, on level 0 too (all nine chunks' bitmaps are filled, so the full map
    /// and the windows after this one keep it).
    pub fn reveal_whole_window(&mut self) {
        for row in self.discovered.iter_mut() {
            row.iter_mut().for_each(|d| *d = true);
        }
        if let Some(inf) = self.inf.as_mut() {
            if inf.level == 0 {
                for dy in 0..infinite::WINDOW {
                    for dx in 0..infinite::WINDOW {
                        inf.level0_seen.mark_chunk((inf.center.0 - 1 + dx, inf.center.1 - 1 + dy));
                    }
                }
            }
        }
    }

    /// What the admin MAP REVEAL changes: the discovered cells and level 0's seen bitmaps, as they were before it.
    pub fn map_reveal_snapshot(&self) -> MapRevealUndo {
        MapRevealUndo {
            discovered: self.discovered.clone(),
            level0_seen: self.inf.as_ref().map(|i| i.level0_seen.clone()),
        }
    }

    /// Undo the MAP REVEAL: put back the snapshot taken before it was switched on.
    pub fn map_reveal_restore(&mut self, u: MapRevealUndo) {
        self.discovered = u.discovered;
        if let (Some(inf), Some(seen)) = (self.inf.as_mut(), u.level0_seen) {
            inf.level0_seen = seen;
        }
    }

    /// Godot `_update_map_discovery` (infinite_route_test.gd:1676), run every frame on level 0. The
    /// runner's room is discovered with every marker inside it, and each marker within
    /// `MAP_DISCOVERY_NEARBY_RADIUS` of the runner is discovered too. The markers are the window's
    /// entities in the middle chunk, which always holds the runner (`stream_window` keeps it there).
    pub(super) fn discover_map(&mut self) {
        let Some(inf) = self.inf.as_ref() else { return };
        if inf.level != 0 {
            return;
        }
        let at = Cell::new(self.p.pos.0.floor() as i32, self.p.pos.1.floor() as i32);
        let middle = |c: Cell| c.x.div_euclid(CHUNK) == 1 && c.y.div_euclid(CHUNK) == 1;
        if !middle(at) {
            return;
        }
        // the middle chunk's markers: the entity id and the window cell, from the lists the window
        // merged (ids are aligned with each list, see InfState::ids and live::WindowIds)
        let ids = &inf.ids;
        let mut markers: Vec<(&str, Cell)> = Vec::new();
        for (id, c) in ids.relays.iter().zip(&self.fac.relays) {
            markers.push((id.as_str(), *c));
        }
        for (id, c) in ids.batteries.iter().zip(&self.fac.batteries) {
            markers.push((id.as_str(), *c));
        }
        for (id, (c, _)) in ids.pickups.iter().zip(&self.fac.pickups) {
            markers.push((id.as_str(), *c));
        }
        for (id, c) in ids.coins.iter().zip(&self.fix.coins) {
            markers.push((id.as_str(), *c));
        }
        for p in &self.fac.props {
            markers.push((p.id.as_str(), p.cell));
        }
        markers.retain(|(_, c)| middle(*c));

        let origin = inf.origin();
        let mut new_rooms: Vec<String> = Vec::new();
        let mut new_markers: HashSet<String> = HashSet::new();
        for room in &self.fac.rooms {
            let inside = |c: Cell| {
                c.x >= room.x && c.y >= room.y && c.x < room.x + room.w && c.y < room.y + room.h
            };
            if !inside(at) {
                continue;
            }
            let key = room_key(&room.id, room.x + origin.0, room.y + origin.1);
            if !inf.discovered_rooms.contains(&key) {
                new_rooms.push(key);
            }
            for (id, c) in &markers {
                if inside(*c) && !inf.markers.contains(*id) {
                    new_markers.insert(id.to_string());
                }
            }
        }
        for (id, c) in &markers {
            let (dx, dy) = ((c.x - at.x) as f32, (c.y - at.y) as f32);
            if (dx * dx + dy * dy).sqrt() <= MAP_DISCOVERY_NEARBY_RADIUS && !inf.markers.contains(*id) {
                new_markers.insert(id.to_string());
            }
        }

        if let Some(inf) = self.inf.as_mut() {
            inf.discovered_rooms.extend(new_rooms);
            inf.markers.extend(new_markers);
        }
    }

    /// Whether the map draws window relay `i` (`fac.relays[i]`): on level 0 only once its marker is
    /// discovered (Godot `_map_chunk_with_available_pickups`). Off level 0 every relay is drawn.
    pub fn map_relay_shown(&self, i: usize) -> bool {
        self.map_marker_shown(self.inf.as_ref().and_then(|inf| inf.ids.relays.get(i)))
    }

    /// Whether the map draws window battery `i` (`fac.batteries[i]`); see `map_relay_shown`.
    pub fn map_battery_shown(&self, i: usize) -> bool {
        self.map_marker_shown(self.inf.as_ref().and_then(|inf| inf.ids.batteries.get(i)))
    }

    fn map_marker_shown(&self, id: Option<&String>) -> bool {
        match (self.inf.as_ref(), id) {
            (Some(inf), Some(id)) if inf.level == 0 => inf.markers.contains(id.as_str()),
            _ => true,
        }
    }

    /// The level-0 full map's feed: every chunk the runner has seen a cell of, in key order. `None` off level 0, and before the
    /// runner has seen any cell (the map then falls back to the window, which draws nothing undiscovered either).
    pub fn full_map_feed(&self) -> Option<FullMapFeed<'_>> {
        let inf = self.inf.as_ref()?;
        if inf.level != 0 || inf.map_chunks.is_empty() || inf.level0_seen.total() == 0 {
            return None;
        }
        let mut chunks: Vec<(ChunkKey, &MapChunk)> = inf
            .map_chunks
            .iter()
            .filter(|(k, _)| inf.level0_seen.chunk_has_any((k.cx, k.cy)))
            .map(|(k, c)| (*k, c))
            .collect();
        chunks.sort_by_key(|(k, _)| (k.cx, k.cy));
        Some(FullMapFeed {
            chunks,
            seen: &inf.level0_seen,
            origin: inf.origin(),
            route: &inf.routes.full,
            waypoint: inf.waypoint.filter(|w| w.level == 0).map(|w| w.at),
        })
    }

    /// Whether the full map draws level-0 relay or battery `id`: its marker is discovered and it is not claimed. A window
    /// entity claimed since the window last moved is claimed too, from its live flag (`relays_taken`, `batteries_taken`).
    pub fn full_map_pickup_shown(&self, id: &str) -> bool {
        let Some(inf) = self.inf.as_ref() else { return false };
        if !inf.markers.contains(id) {
            return false;
        }
        let claimed = inf.claimed.contains(id)
            || inf
                .ids
                .relays
                .iter()
                .position(|x| x == id)
                .is_some_and(|i| self.relays_taken.get(i) == Some(&true))
            || inf
                .ids
                .batteries
                .iter()
                .position(|x| x == id)
                .is_some_and(|i| self.batteries_taken.get(i) == Some(&true));
        !claimed
    }

    /// Whether the full map draws level-0 prop `id`: props show once their marker is discovered (Godot's prop list grows as
    /// props are found).
    pub fn full_map_prop_shown(&self, id: &str) -> bool {
        self.inf.as_ref().is_some_and(|inf| inf.markers.contains(id))
    }
}

/// Moves a hunter's absolute-cell fields by `d`: its position, its last known cell and its two targets. The path is
/// dropped, since it is in the old frame.
pub(super) fn shift_brain(b: &mut Brain, d: (f32, f32)) {
    let s = move |p: (f32, f32)| (p.0 + d.0, p.1 + d.1);
    b.pos = s(b.pos);
    b.last_known = b.last_known.map(s);
    b.wander_target = b.wander_target.map(s);
    b.search_target = b.search_target.map(s);
    // a walk in progress keeps its own polyline (window metres): it moves with the window so the body stays continuous
    if let Some(w) = b.walk.as_mut() {
        w.shift(d.0.round() as i32, d.1.round() as i32);
    }
    b.shift_cross_goal(d);
    b.gimmick.shift(d);
    b.clear_path();
}

/// A level-0 room's key: its layout id (the `@ox,oy` suffix the window merge adds is dropped, since it
/// moves with the window) and its absolute origin, which does not.
fn room_key(window_id: &str, x: i32, y: i32) -> String {
    let base = window_id.rsplit_once('@').map_or(window_id, |(b, _)| b);
    format!("{base}@{x},{y}")
}

/// The waypoint (Godot `WaypointState` and the route code of infinite_route_test.gd). Positions are window coordinates on the
/// accessors and the placement input; the state keeps absolute cell centres, so the window can move under a waypoint.
impl Game {
    /// Godot `_accept_stream_waypoint` for the runner (infinite_route_test.gd:5286). `at` is a window position, snapped to the
    /// centre of its cell. Placing on the waypoint's own cell clears it; anywhere else replaces it. Returns whether it changed.
    pub fn place_waypoint(&mut self, at: (f32, f32)) -> bool {
        let Some(inf) = self.inf.as_ref() else { return false };
        let o = inf.origin();
        let snapped = (
            (at.0 + o.0 as f32).floor() + 0.5,
            (at.1 + o.1 as f32).floor() + 0.5,
        );
        let level = inf.level;
        if inf
            .waypoint
            .is_some_and(|w| w.level == level && pdist(w.at, snapped) <= 0.1)
        {
            return self.clear_waypoint();
        }
        if let Some(inf) = self.inf.as_mut() {
            inf.waypoint = Some(Waypoint { at: snapped, level });
        }
        self.refresh_waypoint();
        self.say("WAYPOINT SET");
        true
    }

    /// Godot `_clear_local_waypoint` (the clear key, the admin clear): removes the waypoint with "WAYPOINT CLEARED".
    /// False when there is none.
    pub fn clear_waypoint(&mut self) -> bool {
        if !self.drop_waypoint() {
            return false;
        }
        self.say("WAYPOINT CLEARED");
        true
    }

    fn drop_waypoint(&mut self) -> bool {
        let Some(inf) = self.inf.as_mut() else { return false };
        if inf.waypoint.take().is_none() {
            return false;
        }
        self.refresh_waypoint();
        true
    }

    /// Godot `_clear_reached_waypoints` (infinite_route_test.gd:527): the runner within `WAYPOINT_ARRIVAL_RADIUS` of the
    /// waypoint on its level clears it, "WAYPOINT REACHED".
    pub(super) fn clear_reached_waypoint(&mut self) {
        let Some(inf) = self.inf.as_ref() else { return };
        let Some(w) = inf.waypoint else { return };
        let o = inf.origin();
        let here = (self.p.pos.0 + o.0 as f32, self.p.pos.1 + o.1 as f32);
        if w.level != inf.level || pdist(here, w.at) > WAYPOINT_ARRIVAL_RADIUS {
            return;
        }
        if self.drop_waypoint() {
            self.say("WAYPOINT REACHED");
        }
    }

    /// Godot `_refresh_waypoint_route` (infinite_route_test.gd:2098): plans the routes to the waypoint again when the runner,
    /// the waypoint, the window or the discovered floor changed. Off the waypoint's level there is no route.
    pub(super) fn refresh_waypoint(&mut self) {
        let Some(inf) = self.inf.as_ref() else { return };
        let o = inf.origin();
        let from = (
            (self.p.pos.0 + o.0 as f32).floor() as i32,
            (self.p.pos.1 + o.1 as f32).floor() as i32,
        );
        let to = inf
            .waypoint
            .filter(|w| w.level == inf.level)
            .map(|w| (w.at.0.floor() as i32, w.at.1.floor() as i32));
        let key = RouteKey {
            level: inf.level,
            center: inf.center,
            from,
            to,
            discovered: inf.discovered_chunks.len(),
        };
        if inf.route_key == Some(key) {
            return;
        }
        // the runner only stepped along the route it already has: drop the cells behind it instead of searching again
        // (a search over the whole discovered floor per cell step hitches a long run)
        if let (Some(prev), Some(_)) = (inf.route_key, to) {
            if prev.level == key.level && prev.center == key.center && prev.to == key.to && prev.discovered == key.discovered {
                if let Some(inf) = self.inf.as_mut() {
                    let mut routes = inf.routes.clone();
                    if trim_route(&mut routes.full, from) && trim_route(&mut routes.line, from) {
                        inf.routes = routes;
                        inf.route_key = Some(key);
                        return;
                    }
                }
            }
        }
        let routes = match to {
            Some(to) => self.plan_routes(from, to),
            None => WaypointRoutes::default(),
        };
        if let Some(inf) = self.inf.as_mut() {
            inf.routes = routes;
            inf.route_key = Some(key);
        }
    }

    /// The two routes from `from` to `to` (absolute cells). The live window's route comes first (Godot `_waypoint_path`:
    /// everything streamed in); on level 0 the discovered floor is the fallback. The floor line runs over the discovered
    /// floor to its reachable cell nearest the waypoint, then straight on (Godot `_discovered_waypoint_line_path`).
    fn plan_routes(&self, from: (i32, i32), to: (i32, i32)) -> WaypointRoutes {
        let Some(inf) = self.inf.as_ref() else { return WaypointRoutes::default() };
        let o = inf.origin();
        let local = |c: (i32, i32)| Cell::new(c.0 - o.0, c.1 - o.1);
        let live = self.fac.find_path(local(from), local(to));
        let full: Vec<(i32, i32)> = if !live.is_empty() {
            live.iter().map(|c| (c.x + o.0, c.y + o.1)).collect()
        } else if inf.level == 0 {
            inf.floor.path(from, to)
        } else {
            vec![]
        };
        let discovered = if inf.level == 0 {
            let frontier = inf.floor.reachable_target(from, to);
            inf.floor.path(from, frontier)
        } else {
            vec![]
        };
        WaypointRoutes {
            full: full.into_iter().map(centre).collect(),
            line: choose_line(&discovered, from, to),
        }
    }

    /// Feed for the drawing code: the waypoint's pin in window cell coordinates, on the level it was placed on.
    pub fn waypoint_marker(&self) -> Option<(f32, f32)> {
        let inf = self.inf.as_ref()?;
        let w = inf.waypoint?;
        let o = inf.origin();
        (w.level == inf.level).then(|| (w.at.0 - o.0 as f32, w.at.1 - o.1 as f32))
    }

    /// Feed for the drawing code: the route to the waypoint (`full`, Godot `_waypoint_route`) in window cell-centre
    /// coordinates. Empty without a waypoint, off its level, or when no route is known.
    pub fn waypoint_route(&self) -> Vec<(f32, f32)> {
        self.inf
            .as_ref()
            .map_or_else(Vec::new, |inf| to_window(&inf.routes.full, inf.origin()))
    }

    /// Feed for the drawing code: the floor line (`line`, Godot `_waypoint_line_route`) in window cell-centre coordinates.
    pub fn waypoint_line(&self) -> Vec<(f32, f32)> {
        self.inf
            .as_ref()
            .map_or_else(Vec::new, |inf| to_window(&inf.routes.line, inf.origin()))
    }

    /// Feed for the drawing code: the PATH-guidance chevrons along the next `WAYPOINT_TRAIL_METRES` of the route.
    pub fn waypoint_chevrons(&self) -> Vec<Chevron> {
        chevrons(&self.waypoint_route())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    #[test]
    fn stepping_along_a_route_trims_it_and_leaving_it_asks_for_a_new_plan() {
        let mut route: Vec<(f32, f32)> = (0..5).map(|x| centre((x, 3))).collect();
        assert!(trim_route(&mut route, (2, 3)));
        assert_eq!(route, vec![centre((2, 3)), centre((3, 3)), centre((4, 3))]);
        let before = route.clone();
        assert!(!trim_route(&mut route, (9, 9)), "off the route: plan again");
        assert_eq!(route, before);
        assert!(!trim_route(&mut Vec::new(), (0, 0)), "no route: plan again");
    }

    #[test]
    fn a_gimmick_shift_moves_the_wire_ends_with_the_window() {
        use crate::hunters::gimmicks::{ravelle, Gimmick};
        let mut g = Gimmick::Ravelle(Default::default());
        if let Gimmick::Ravelle(r) = &mut g {
            r.wires.push(ravelle::Wire { a: (40.0, 8.0), b: (41.0, 8.0), triggered: false });
        }
        g.shift((-32.0, 0.0));
        let Gimmick::Ravelle(r) = &g else { unreachable!() };
        assert_eq!((r.wires[0].a, r.wires[0].b), ((8.0, 8.0), (9.0, 8.0)));
    }

    fn run() -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_infinite_sim(
            crate::config::Difficulty::Standard,
            7,
            "ROOM-TEST-7",
            0,
            &content,
            &catalog,
        );
        (g, content, catalog)
    }

    #[test]
    fn window_lists_and_ids_line_up() {
        let (g, _, _) = run();
        let inf = g.inf.as_ref().unwrap();
        assert_eq!(inf.ids.relays.len(), g.fac.relays.len());
        assert_eq!(inf.ids.batteries.len(), g.fac.batteries.len());
        assert_eq!(inf.ids.pickups.len(), g.fac.pickups.len());
        assert_eq!(inf.ids.coins.len(), g.fix.coins.len());
        assert_eq!(g.coins_taken.len(), g.fix.coins.len());
        assert_eq!(g.pickups.len(), floor_source(&g.fac).len());
        // the layout ids are the level-0 ids; under KYTTU_LEGACY_CHUNKS=1 the window keys by cell
        if !infinite::legacy_level0() {
            assert!(inf.ids.relays[0].contains(":PICKUP:RELAY:"), "{}", inf.ids.relays[0]);
        }
    }

    #[test]
    fn a_claimed_relay_stays_claimed_when_the_window_moves_away_and_back() {
        let (mut g, content, catalog) = run();
        let id = g.inf.as_ref().unwrap().ids.relays[0].clone();
        g.relays_taken[0] = true;
        // three chunks east: the window's first chunk column leaves, then three back
        for _ in 0..3 {
            g.p.pos.0 += CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        let ids = &g.inf.as_ref().unwrap().ids.relays;
        assert!(!ids.contains(&id), "the relay should have left the window");
        for _ in 0..3 {
            g.p.pos.0 -= CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        let i = g
            .inf
            .as_ref()
            .unwrap()
            .ids
            .relays
            .iter()
            .position(|x| *x == id)
            .expect("the relay is back in the window");
        assert!(g.relays_taken[i], "a claimed relay must stay claimed");
    }

    #[test]
    fn a_window_move_drops_the_door_focus_of_the_old_window() {
        use super::super::interact::Focus;
        let (mut g, content, catalog) = run();
        g.fac.update_doors(&[], &[], 0.0);
        // a focus on a door of this window names a prop index: after the window moves it must not keep naming one
        let (i, _) = g.fac.doors().next().expect("a door in the window");
        g.focus = Some(Focus::Door(i));
        g.p.pos.0 += CHUNK as f32;
        g.stream_window(&content, &catalog);
        match g.focus {
            None => {}
            Some(Focus::Door(j)) => {
                let cell = g.fac.props[j].cell;
                assert!(g.fac.door_centre(cell).is_some(), "the door is registered, so its prompt follows its state");
                let at = g.fac.doors().find(|d| d.0 == j).unwrap().1;
                assert!(pdist(at, g.p.pos) < 4.0, "the door focus is at the runner, not {} m away", pdist(at, g.p.pos));
            }
            Some(_) => {}
        }
    }

    #[test]
    fn no_focus_index_outlives_a_window_move_or_a_rebuild() {
        use super::super::interact::Focus;
        let (mut g, content, catalog) = run();
        let sounds = crate::audio::Sounds::default();
        // the last entry of every list a focus indexes: after the window is rebuilt these are the indices
        // most likely to run off the end of a shorter list
        let foci = |g: &Game| {
            let mut v = vec![];
            if let Some(i) = g.fac.relays.len().checked_sub(1) { v.push(Focus::Relay(i)); }
            if let Some(i) = g.fac.batteries.len().checked_sub(1) { v.push(Focus::Battery(i)); }
            if let Some(i) = g.pickups.len().checked_sub(1) { v.push(Focus::Pickup(i)); }
            if let Some(i) = g.fix.coins.len().checked_sub(1) { v.push(Focus::Coin(i)); }
            if let Some(i) = g.fac.hiding.len().checked_sub(1) { v.push(Focus::Locker(i)); }
            if let Some(i) = g.fac.stairs.iter().rposition(|s| s.is_ladder()) { v.push(Focus::Ladder(i)); }
            if let Some(i) = g.fac.room_lights.len().checked_sub(1) { v.push(Focus::LightSwitch(i)); }
            if let Some(i) = g.fix.vents.len().checked_sub(1) { v.push(Focus::Vent(i)); }
            if let Some((i, _)) = g.fac.doors().last() { v.push(Focus::Door(i)); }
            v
        };
        // far hops (the agent's teleport) and single-chunk steps, each with a focus set before the move
        let hops = [(CHUNK as f32, 0.0), (0.0, -4.0 * CHUNK as f32), (-7.0 * CHUNK as f32, 3.0 * CHUNK as f32), (0.0, CHUNK as f32)];
        for (dx, dy) in hops {
            for f in foci(&g) {
                g.focus = Some(f);
                g.p.pos.0 += dx;
                g.p.pos.1 += dy;
                g.stream_window(&content, &catalog);
                if let Some(f) = g.focus {
                    // every consumer of the focus must still be in range
                    let _ = g.focus_prompt();
                    g.interact(f, &content, &catalog, &sounds);
                }
                g.p.pos = g.fac.start.center();
                g.stream_window(&content, &catalog);
            }
        }
        // a rebuild in place (regen_window) and a fresh floor drop the focus too
        for f in foci(&g) {
            g.focus = Some(f);
            g.regen_window(&content, &catalog);
            assert!(g.focus.is_none(), "{f:?} survived a rebuild of the window");
            g.focus = Some(f);
            g.start_floor_sim(&content, &catalog);
            assert!(g.focus.is_none(), "{f:?} survived a new floor");
        }
    }

    /// The window cells the grid marks discovered.
    fn seen_count(g: &Game) -> usize {
        g.discovered.iter().map(|r| r.iter().filter(|d| **d).count()).sum()
    }

    /// The absolute cells the window grid marks discovered.
    fn seen_cells(g: &Game) -> Vec<(i32, i32)> {
        let (ox, oy) = g.inf.as_ref().unwrap().origin();
        let mut v = vec![];
        for (y, row) in g.discovered.iter().enumerate() {
            for (x, d) in row.iter().enumerate() {
                if *d {
                    v.push((x as i32 + ox, y as i32 + oy));
                }
            }
        }
        v
    }

    /// The first open window cell centre at least `min` cells from `from` that the runner may stand on.
    fn open_spot(g: &Game, from: (f32, f32), min: f32) -> (f32, f32) {
        for y in 0..g.fac.rows {
            for x in 0..g.fac.cols {
                let c = Cell::new(x, y);
                if g.fac.can_occupy(c.center().0 as f64, c.center().1 as f64, 0.3) && pdist(c.center(), from) >= min {
                    return c.center();
                }
            }
        }
        panic!("no open cell {min} cells from {from:?}");
    }

    #[test]
    fn level_zero_starts_with_the_lobby_and_what_the_runner_sees_not_the_whole_window() {
        let (g, _, _) = run();
        // the nine chunks are loaded (the map records and the router read them) but not discovered
        assert_eq!(g.inf.as_ref().unwrap().discovered_chunks.len(), 9);
        let n = seen_count(&g);
        let all = g.discovered.len() * g.discovered[0].len();
        assert!(n > 0, "the start is on the map");
        assert!(n < all / 20, "{n} of {all} cells: a lobby's worth, not the window");
        let (sx, sy) = (g.p.pos.0.floor() as usize, g.p.pos.1.floor() as usize);
        assert!(g.discovered[sy][sx], "the runner's own cell");
        let sz = g.fac.safe_zone.expect("the start chunk is a safe room");
        for dy in -2..=2 {
            for dx in -2..=2 {
                assert!(
                    g.discovered[(sz.y + dy) as usize][(sz.x + dx) as usize],
                    "the safe room cell {dx},{dy} is discovered at the start"
                );
            }
        }
        // the corners of the window are far outside the view
        assert!(!g.discovered[0][0] && !g.discovered[g.fac.rows as usize - 1][g.fac.cols as usize - 1]);
        // the persistent bitmaps hold exactly the window grid
        assert_eq!(g.inf.as_ref().unwrap().level0_seen.total(), n);
    }

    #[test]
    fn level_zero_cells_outside_the_view_are_hidden() {
        let (mut g, _, _) = run();
        g.reveal();
        let reach = crate::ui::map::look::MapLook::load().reveal_radius as f32 + 0.5;
        let lobby = SAFE_ZONE_RADIUS + 2.0;
        let (ox, oy) = g.inf.as_ref().unwrap().origin();
        for (ax, ay) in seen_cells(&g) {
            let d = pdist((ax as f32 + 0.5 - ox as f32, ay as f32 + 0.5 - oy as f32), g.p.pos);
            assert!(d <= reach.max(lobby), "cell {ax},{ay} is {d} from the runner");
        }
        // a cell in a far corner of the window is not seen, by absolute coordinate either
        assert!(!g.level0_cell_seen(ox + 2 * CHUNK + 10, oy + 2 * CHUNK + 10));
        assert!(!g.level0_cell_seen(ox + 1, oy + 1));
    }

    #[test]
    fn level_zero_discovery_grows_with_exploration() {
        let (mut g, _, _) = run();
        let before = seen_count(&g);
        let first = g.inf.as_ref().unwrap().level0_seen.total();
        // stand at an open spot a dozen cells away: the frame's reveal finds the floor around it
        let to = open_spot(&g, g.p.pos, 12.0);
        g.p.pos = to;
        g.reveal();
        let after = seen_count(&g);
        assert!(after > before, "{before} -> {after}");
        assert!(g.inf.as_ref().unwrap().level0_seen.total() > first);
        // seen stays seen: walking back does not take cells off the map
        let start = g.fac.start.center();
        g.p.pos = start;
        g.reveal();
        assert!(seen_count(&g) >= after);
        // and revealing the same view again adds nothing
        let again = seen_count(&g);
        g.reveal();
        assert_eq!(seen_count(&g), again);
    }

    #[test]
    fn level_zero_discovery_persists_across_chunk_reload_and_is_keyed_by_absolute_cell() {
        let (mut g, content, catalog) = run();
        let to = open_spot(&g, g.p.pos, 12.0);
        g.p.pos = to;
        g.reveal();
        let seen = seen_cells(&g);
        assert!(seen.len() > 50);
        // three chunks east: the window has left every chunk it started with, and the new view shows none of the old cells
        for _ in 0..3 {
            g.p.pos.0 += CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        let (ox, _) = g.inf.as_ref().unwrap().origin();
        assert!(seen.iter().all(|(ax, _)| *ax < ox), "the first cells are west of the new window");
        assert!(seen.iter().all(|(ax, ay)| g.level0_cell_seen(*ax, *ay)), "they stay seen off the window");
        assert!(seen_count(&g) < seen.len(), "the new window shows none of the old cells");
        // the chunks are rebuilt from the seed (the cache and layout read are dropped): the cells are the same absolute cells
        g.regen_window(&content, &catalog);
        assert!(seen.iter().all(|(ax, ay)| g.level0_cell_seen(*ax, *ay)));
        // back west: the window grid shows the cells that were seen, at the same absolute cells
        for _ in 0..3 {
            g.p.pos.0 -= CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        let again = seen_cells(&g);
        for c in &seen {
            assert!(again.contains(c), "cell {c:?} is back on the map after returning");
        }
    }

    #[test]
    fn level_zero_discovery_is_per_cell_in_negative_chunks_too() {
        crate::world::route::layout::godot_regions_for_this_test();
        let (mut g, content, catalog) = run();
        g.p.pos.0 -= 2.0 * CHUNK as f32;
        g.stream_window(&content, &catalog);
        g.p.pos = g.fac.start.center();
        g.reveal();
        let c = g.p.pos;
        let (ox, oy) = g.inf.as_ref().unwrap().origin();
        let abs = ((c.0.floor() as i32) + ox, (c.1.floor() as i32) + oy);
        assert!(abs.0 < 0, "the runner is west of the origin chunk: {abs:?}");
        assert!(g.level0_cell_seen(abs.0, abs.1));
        assert!(!g.level0_cell_seen(abs.0 + 20, abs.1), "a cell twenty east is not");
    }

    #[test]
    fn the_admin_map_reveal_fills_the_window_and_keeps_it() {
        let (mut g, content, catalog) = run();
        g.reveal_whole_window();
        assert!(g.discovered.iter().all(|r| r.iter().all(|d| *d)));
        let n = (9 * CHUNK * CHUNK) as usize;
        assert_eq!(g.inf.as_ref().unwrap().level0_seen.total(), n);
        assert_eq!(g.full_map_feed().expect("feed").chunks.len(), 9);
        // one chunk east: the new column is not revealed, the old chunks still are
        g.p.pos.0 += CHUNK as f32;
        g.stream_window(&content, &catalog);
        assert!(!g.discovered.iter().all(|r| r.iter().all(|d| *d)));
        assert_eq!(g.inf.as_ref().unwrap().level0_seen.total(), n);
        assert!(g.discovered[40][10], "an old cell is still on the map");
    }

    #[test]
    fn the_start_room_is_pre_discovered_on_level_zero_only() {
        let content = Content::load();
        let catalog = Catalog::load();
        for level in [crate::world::route::sewer::core::LEVEL, crate::world::infinite::INFERNO_LEVEL] {
            let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-TEST-7", level, &content, &catalog);
            // even a safe room on the storey adds nothing beyond the line of sight
            g.fac.safe_zone = Some(g.fac.start);
            let before = seen_cells(&g);
            g.reveal_start_room();
            assert_eq!(seen_cells(&g), before, "level {level}: the start room is not pre-discovered");
            assert_eq!(g.inf.as_ref().unwrap().level0_seen.total(), 0);
        }
        let (mut g, _, _) = run();
        g.discovered.iter_mut().for_each(|r| r.iter_mut().for_each(|d| *d = false));
        g.reveal_start_room();
        assert!(seen_count(&g) > 0, "level 0 pre-discovers the lobby");
    }

    #[test]
    fn the_sewer_and_the_inferno_keep_their_cell_set_and_level_zero_marks_stay_empty() {
        let content = Content::load();
        let catalog = Catalog::load();
        for level in [crate::world::route::sewer::core::LEVEL, crate::world::infinite::INFERNO_LEVEL] {
            let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-TEST-7", level, &content, &catalog);
            g.reveal();
            let n = seen_count(&g);
            let all = g.discovered.len() * g.discovered[0].len();
            assert!(n > 0 && n < all / 10, "level {level}: {n} cells seen is a view, not the window");
            assert_eq!(g.inf.as_ref().unwrap().level0_seen.total(), 0, "level {level} never writes level 0's bitmaps");
            assert!(g.full_map_feed().is_none(), "the full map feed is level 0's");
            // the cell set carries over a window move, by absolute cell, as before
            let before = seen_cells(&g);
            g.p.pos.0 += CHUNK as f32;
            g.stream_window(&content, &catalog);
            let inf = g.inf.as_ref().unwrap();
            for (ax, ay) in &before {
                assert!(inf.seen.contains(&(level, *ax, *ay)), "level {level}: {ax},{ay} kept");
            }
        }
    }

    #[test]
    fn the_sewer_window_marks_the_water_the_swim_code_reads() {
        let content = Content::load();
        let catalog = Catalog::load();
        for level in [crate::world::route::sewer::core::LEVEL, crate::world::route::sewer::core::LOWER_LEVEL] {
            let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-TEST-7", level, &content, &catalog);
            let wet: Vec<Cell> = g.fac.water_cells().to_vec();
            assert!(!wet.is_empty(), "level {level} has water in its window");
            let (mut with, mut without) = (0, 0);
            for y in 0..g.fac.rows {
                for x in 0..g.fac.cols {
                    let c = Cell::new(x, y);
                    let depth = g.water_depth_on(level, c.center());
                    assert_eq!(g.fac.is_water(c), depth > 0.0, "level {level}: cell {x},{y}: map water and swim water agree");
                    if depth > 0.0 {
                        with += 1;
                    } else {
                        without += 1;
                    }
                }
            }
            assert_eq!(with, wet.len());
            assert!(without > with, "most of the window is dry");
        }
        let (g, _, _) = run();
        assert!(g.fac.water_cells().is_empty(), "level 0 has no channel water");
    }

    #[test]
    fn a_discovered_chunk_keeps_its_map_record_when_it_leaves_the_window() {
        let (mut g, content, catalog) = run();
        let key = ChunkKey { cx: 0, cy: 0, level: 0 };
        let before = g.inf.as_ref().unwrap().map_chunks[&key].clone();
        assert_eq!(before.walls.len(), (CHUNK * CHUNK) as usize);
        // three chunks east: the chunk at (0, 0) leaves the window
        for _ in 0..3 {
            g.p.pos.0 += CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        let inf = g.inf.as_ref().unwrap();
        assert!(!inf.ids.relays.is_empty());
        let after = &inf.map_chunks[&key];
        assert_eq!(after.walls, before.walls, "the walls outlive the window");
        assert_eq!(after.holes, before.holes);
        assert_eq!(after.relays, before.relays);
        assert_eq!(after.batteries, before.batteries);
        assert_eq!(after.props.len(), before.props.len());
        assert_eq!(inf.map_chunks.len(), inf.discovered_chunks.len(), "one record per discovered chunk");
    }

    #[test]
    fn map_records_sit_at_their_absolute_cells_under_the_window_ids() {
        let (g, _, _) = run();
        let inf = g.inf.as_ref().unwrap();
        let o = inf.origin();
        let relays: Vec<(String, Cell)> = inf.map_chunks.values().flat_map(|c| c.relays.clone()).collect();
        let batteries: Vec<(String, Cell)> = inf.map_chunks.values().flat_map(|c| c.batteries.clone()).collect();
        assert_eq!(relays.len(), inf.ids.relays.len(), "every window relay has a record");
        assert_eq!(batteries.len(), inf.ids.batteries.len(), "every window battery has a record");
        for (i, id) in inf.ids.relays.iter().enumerate() {
            let want = Cell::new(g.fac.relays[i].x + o.0, g.fac.relays[i].y + o.1);
            let got: Vec<Cell> = relays.iter().filter(|(x, _)| x == id).map(|(_, c)| *c).collect();
            assert_eq!(got, vec![want], "relay {id}");
        }
        for (i, id) in inf.ids.batteries.iter().enumerate() {
            let want = Cell::new(g.fac.batteries[i].x + o.0, g.fac.batteries[i].y + o.1);
            let got: Vec<Cell> = batteries.iter().filter(|(x, _)| x == id).map(|(_, c)| *c).collect();
            assert_eq!(got, vec![want], "battery {id}");
        }
        let solid = g.fac.props.iter().filter(|p| p.solid).count();
        let props: Vec<&MapProp> = inf.map_chunks.values().flat_map(|c| c.props.iter()).collect();
        assert_eq!(props.len(), solid, "every solid window prop has a box record");
    }

    #[test]
    fn the_full_map_feed_follows_the_discovery_into_negative_chunks() {
        let (mut g, content, catalog) = run();
        // only the start chunk has a seen cell at the start, though nine are loaded
        assert_eq!(g.full_map_feed().expect("level 0 has a feed").chunks.len(), 1);
        // one chunk west: the window recentres on (-1, 0) and the runner stands at its lobby, which it sees
        g.p.pos.0 -= CHUNK as f32;
        g.stream_window(&content, &catalog);
        g.p.pos = g.fac.start.center();
        g.reveal();
        let origin = g.inf.as_ref().unwrap().origin();
        let feed = g.full_map_feed().expect("level 0 has a feed");
        assert_eq!(feed.chunks.len(), 2);
        assert!(feed.chunks.iter().any(|(k, _)| k.cx == -1), "the chunk the runner walked into is discovered");
        assert_eq!(feed.origin, origin);
        // the feed is in key order
        let keys: Vec<(i32, i32)> = feed.chunks.iter().map(|(k, _)| (k.cx, k.cy)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    /// A relay the map may draw, placed in the middle chunk with the room rule switched off.
    fn place_test_relay(g: &mut Game, cell: Cell) -> usize {
        g.fac.relays.push(cell);
        g.relays_taken.push(false);
        let inf = g.inf.as_mut().unwrap();
        inf.ids.relays.push(format!("TEST:RELAY:{}:{}", cell.x, cell.y));
        g.fac.relays.len() - 1
    }

    #[test]
    fn nearby_markers_are_discovered_within_four_and_a_half_cells() {
        let (mut g, _, _) = run();
        g.fac.rooms.clear();
        let i = place_test_relay(&mut g, Cell::new(CHUNK + 25, CHUNK + 25));
        g.p.pos = (CHUNK as f32 + 5.5, CHUNK as f32 + 5.5);
        g.discover_map();
        assert!(!g.map_relay_shown(i), "a relay 20 cells away is not on the map");
        // two cells away: inside MAP_DISCOVERY_NEARBY_RADIUS
        g.p.pos = (CHUNK as f32 + 23.5, CHUNK as f32 + 25.5);
        g.discover_map();
        assert!(g.map_relay_shown(i), "a relay two cells away is discovered");
        // once discovered it stays on the map when the runner walks off
        g.p.pos = (CHUNK as f32 + 5.5, CHUNK as f32 + 5.5);
        g.discover_map();
        assert!(g.map_relay_shown(i));
    }

    #[test]
    fn a_room_discovers_its_markers_and_itself() {
        let (mut g, _, _) = run();
        let mut room = g.fac.rooms[0].clone();
        g.fac.rooms.clear();
        room.id = "ROOM-TEST@0,0".to_string();
        room.x = CHUNK + 15;
        room.y = CHUNK + 15;
        room.w = 10;
        room.h = 10;
        g.fac.rooms.push(room);
        // inside the room but more than 4.5 cells from where the runner will stand
        let i = place_test_relay(&mut g, Cell::new(CHUNK + 24, CHUNK + 24));
        g.p.pos = (CHUNK as f32 + 20.5, CHUNK as f32 + 20.5);
        g.discover_map();
        let key = room_key("ROOM-TEST@0,0", g.fac.rooms[0].x + (g.inf.as_ref().unwrap().center.0 - 1) * CHUNK, g.fac.rooms[0].y + (g.inf.as_ref().unwrap().center.1 - 1) * CHUNK);
        assert!(g.inf.as_ref().unwrap().discovered_rooms.contains(&key), "{key}");
        assert!(g.map_relay_shown(i), "the room's relay is discovered on entry");
    }

    /// A run on the spawn chunk with the runner in the middle of its safe room. The hunters stand in the far
    /// corner (no lure, no breach) until a test places them.
    fn in_room() -> (Game, Content, Catalog) {
        let (mut g, content, catalog) = run();
        // the route's lineup for a population (residency admits the layout spawns; see populated)
        g.relays_collected = 40;
        g.ensure_hunters(&content);
        g.p.pos = middle_safe_centre();
        for h in g.hunters.iter_mut() {
            h.pos = (0.5, 0.5);
        }
        (g, content, catalog)
    }

    fn room_of(g: &Game) -> SafeRoom {
        g.inf.as_ref().unwrap().room.clone().expect("a safe room in the middle chunk")
    }

    fn status_of(g: &Game) -> crate::shared::safe_room::Status {
        g.inf.as_ref().unwrap().safe.status(&room_of(g))
    }

    #[test]
    fn the_spawn_chunk_room_is_the_window_safe_zone() {
        let (g, _, _) = run();
        let inf = g.inf.as_ref().unwrap();
        assert!(inf.live, "level 0 is the layout");
        assert_eq!(inf.center, (0, 0));
        assert_eq!(room_of(&g).id, "0:0:0:SAFE");
        assert_eq!(g.fac.safe_zone, Some(Cell::new(CHUNK + 22, CHUNK + 22)));
        let (g2, _, _) = in_room();
        assert!(g2.in_safe_zone(), "the runner at the room centre is in the safe zone");
    }

    #[test]
    fn a_new_checkpoint_autosaves_once() {
        let (mut g, content, catalog) = in_room();
        assert_eq!(g.last_autosave, "");
        g.stream_window(&content, &catalog);
        assert_eq!(g.active_checkpoint(), "0:0:0:CHECKPOINT");
        assert_eq!(g.last_autosave, "checkpoint", "activating the checkpoint saves the run");
        g.last_autosave = "";
        g.stream_window(&content, &catalog);
        assert_eq!(g.last_autosave, "", "standing on in the room does not save every frame");
    }

    #[test]
    fn recovering_a_relay_autosaves() {
        let (mut g, content, catalog) = run();
        assert!(!g.fac.relays.is_empty(), "the layout has relays");
        g.interact(super::super::Focus::Relay(0), &content, &catalog, &crate::audio::Sounds::default());
        assert_eq!(g.relays_collected, 1);
        assert_eq!(g.last_autosave, "relay", "a recovered relay saves the run");
    }

    #[test]
    fn a_finished_run_does_not_autosave() {
        let (mut g, _, _) = in_room();
        assert!(g.autosave("quit"));
        assert_eq!(g.last_autosave, "quit");
        g.last_autosave = "";
        // a dead runner's run is over (a catch only takes the runner down first: game/revive.rs)
        g.finish_run(0, "caught");
        assert!(g.report.is_some(), "the death ends the run");
        assert!(!g.autosave("quit"), "a caught runner's run has ended");
        assert_eq!(g.last_autosave, "");
    }

    #[test]
    fn standing_in_the_room_activates_its_checkpoint() {
        let (mut g, _, _) = in_room();
        assert_eq!(g.active_checkpoint(), "");
        g.safe_room_step(0.0);
        assert_eq!(g.active_checkpoint(), "0:0:0:CHECKPOINT");
    }

    #[test]
    fn a_fresh_checkpoint_restores_the_runner_once() {
        let (mut g, _, _) = in_room();
        g.p.stamina = 1.0;
        g.p.lamp = 1.0;
        g.p.exhausted = true;
        g.safe_room_step(0.0);
        assert_eq!(g.checkpoints_activated(), 1);
        assert_eq!(g.p.stamina, MAX_STAMINA);
        assert_eq!(g.p.lamp, MAX_LAMP_CHARGE);
        assert!(!g.p.exhausted);
        assert_eq!(g.message, "SAFE CHECKPOINT ACTIVE");
        // standing on in the active room is no new checkpoint
        g.p.stamina = 1.0;
        g.message.clear();
        g.safe_room_step(1.0 / 30.0);
        assert_eq!(g.checkpoints_activated(), 1);
        assert_eq!(g.p.stamina, 1.0);
        assert!(g.message.is_empty());
    }

    #[test]
    fn a_breach_on_the_host_is_published_and_a_guest_applies_it_without_battering_on_its_own() {
        let (mut host, _, _) = in_room();
        host.safe_room_step(0.0);
        host.admin_edit_checkpoint(-100);
        let events = host.hunter_out.take();
        let breach = events
            .iter()
            .find(|e| e.get("what").and_then(|w| w.as_str()) == Some(crate::shared::safe_room::EVENT))
            .expect("the host publishes the room's status");
        assert!(status_of(&host).breached);

        let (mut guest, _, _) = in_room();
        guest.remote_hunters = true;
        let c = middle_safe_centre();
        guest.hunters[0].pos = (c.0 + 2.0, c.1);
        for _ in 0..60 {
            guest.safe_room_step(1.0 / 30.0);
        }
        assert_eq!(status_of(&guest).integrity, 100, "a guest does not batter the room from the host's hunters");
        assert!(guest.apply_safe_room_event(breach));
        assert!(status_of(&guest).breached);
        assert_eq!(guest.fac.safe_zone, None, "the guest sees the room fall");
        assert_eq!(guest.message, "SAFE ROOM BREACHED");
        assert!(!guest.apply_safe_room_event(&serde_json::Map::new()), "other events are not room events");
    }

    #[test]
    fn the_runner_hears_the_room_fall_and_admin_edits_its_integrity() {
        let (mut g, _, _) = in_room();
        g.safe_room_step(0.0);
        assert_eq!(g.admin_edit_checkpoint(-25).map(|s| s.integrity), Some(75));
        assert_eq!(g.live_room_status().map(|s| s.integrity), Some(75));
        for _ in 0..2 {
            g.admin_edit_checkpoint(-25);
        }
        g.message.clear();
        let s = g.admin_edit_checkpoint(-25).unwrap();
        assert!(s.breached && s.integrity == 0);
        assert_eq!(g.message, "SAFE ROOM BREACHED");
        assert_eq!(g.fac.safe_zone, None);
        let s = g.admin_edit_checkpoint(25).unwrap();
        assert!(!s.breached && s.integrity == 25);
        assert!(g.fac.safe_zone.is_some(), "a repaired room is the safe zone again");
        // off the room's chunk there is no checkpoint to edit
        g.inf.as_mut().unwrap().room = None;
        assert!(g.admin_edit_checkpoint(-25).is_none());
    }

    #[test]
    fn a_hunter_at_the_wall_breaches_the_room_until_the_zone_goes() {
        let (mut g, _, _) = in_room();
        assert!(!g.hunters.is_empty());
        let c = middle_safe_centre();
        g.hunters[0].pos = (c.0 + 2.0, c.1);
        for _ in 0..30 {
            g.safe_room_step(1.0 / 30.0);
        }
        // 1.08 * 7 = 7.56 a second for one hunter: seven whole points in the first second
        let s = status_of(&g);
        assert_eq!(s.integrity, 93, "one second of one hunter");
        assert!(!s.breached);
        assert!(g.fac.safe_zone.is_some(), "an intact room is the safe zone");
        for _ in 0..(30 * 20) {
            g.safe_room_step(1.0 / 30.0);
        }
        let s = status_of(&g);
        assert!(s.breached && s.integrity == 0, "{s:?}");
        assert_eq!(g.fac.safe_zone, None, "a breached room is no safe zone");
        // the checkpoint the runner made in the room before the breach stays the active one
        assert_eq!(g.active_checkpoint(), "0:0:0:CHECKPOINT");
    }

    #[test]
    fn hunters_outside_the_breach_reach_do_not_batter_and_the_near_ones_are_lured() {
        let (mut g, _, _) = in_room();
        let c = middle_safe_centre();
        // the first hunter is 10 cells off: lured to the centre, not breaching
        g.hunters[0].active = true;
        g.hunters[0].pos = (c.0 + 10.0, c.1);
        g.hunters[0].state = State::Wander;
        // a dormant hunter 5 cells off: not lured (inactive), and 5 is past the 4.25 breach reach
        if g.hunters.len() > 1 {
            g.hunters[1].active = false;
            g.hunters[1].state = State::Wander;
            g.hunters[1].pos = (c.0 + 5.0, c.1);
        }
        for _ in 0..30 {
            g.safe_room_step(1.0 / 30.0);
        }
        assert_eq!(g.hunters[0].state, State::Investigate);
        assert_eq!(g.hunters[0].last_known, Some(c));
        assert_eq!(status_of(&g).integrity, 100, "nobody is within the breach reach");
        if g.hunters.len() > 1 {
            assert_ne!(g.hunters[1].state, State::Investigate, "an inactive hunter is not lured");
        }
    }

    #[test]
    fn a_guest_is_not_lured_by_the_safe_room_because_the_host_sets_its_hunters() {
        let (mut g, _, _) = in_room();
        g.remote_hunters = true;
        let c = middle_safe_centre();
        g.hunters[0].active = true;
        g.hunters[0].pos = (c.0 + 10.0, c.1);
        g.hunters[0].state = State::Wander;
        g.hunters[0].last_known = None;
        for _ in 0..30 {
            g.safe_room_step(1.0 / 30.0);
        }
        assert_eq!(g.hunters[0].state, State::Wander, "the host's frames set a guest's hunter state");
        assert_eq!(g.hunters[0].last_known, None);
    }

    #[test]
    fn a_guest_keeps_its_hunter_positions_when_the_window_moves() {
        let (mut g, content, catalog) = in_room();
        // off level 0 the move does not run residency, so the only hunter writes are the shift and the respawn
        g.inf.as_mut().unwrap().level = -1;
        g.remote_hunters = true;
        let before: Vec<(f32, f32)> = g.hunters.iter().map(|h| h.pos).collect();
        assert!(!before.is_empty());
        g.p.pos.0 += CHUNK as f32;
        g.recentre_window(&content, &catalog);
        let after: Vec<(f32, f32)> = g.hunters.iter().map(|h| h.pos).collect();
        assert_eq!(after, before, "a guest does not shift or respawn hunters on a window move");
    }

    #[test]
    fn a_guest_is_not_respawned_out_of_the_window() {
        let (mut g, _, _) = in_room();
        g.remote_hunters = true;
        g.hunters[0].pos = (-5.0, 0.5);
        g.respawn_outside_window();
        assert_eq!(g.hunters[0].pos, (-5.0, 0.5), "the host respawns the hunter, not the guest");
    }

    #[test]
    fn a_tool_in_reach_repairs_the_room_even_when_it_is_breached() {
        let (mut g, _, _) = in_room();
        let room = room_of(&g);
        g.inf.as_mut().unwrap().safe.damage(&room, 100);
        g.sync_safe_zone();
        assert!(status_of(&g).breached);
        assert_eq!(g.fac.safe_zone, None);
        // five cells off: out of the repair reach
        g.p.pos = (middle_safe_centre().0 + 5.0, middle_safe_centre().1);
        assert!(!g.repair_safe_room(Item::Axe));
        g.p.pos = middle_safe_centre();
        assert!(g.repair_safe_room(Item::Crowbar), "a crowbar is 12");
        assert_eq!(status_of(&g), crate::shared::safe_room::Status { integrity: 12, breached: false });
        assert!(g.fac.safe_zone.is_some(), "a repaired room is the safe zone again");
        assert!(g.repair_safe_room(Item::Axe), "a rescue axe is 18");
        assert_eq!(status_of(&g).integrity, 30);
        assert!(!g.repair_safe_room(Item::Taser), "only a crowbar or an axe repairs");
    }

    #[test]
    fn a_room_keeps_its_damage_when_the_window_moves_away_and_back() {
        let (mut g, content, catalog) = in_room();
        let c = middle_safe_centre();
        g.hunters[0].pos = (c.0 + 2.0, c.1);
        for _ in 0..30 {
            g.safe_room_step(1.0 / 30.0);
        }
        let hit = status_of(&g).integrity;
        assert!(hit < 100);
        // three chunks east, then three back: the spawn room leaves the window and comes back
        for _ in 0..3 {
            g.p.pos.0 += CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        assert_ne!(g.inf.as_ref().unwrap().center, (0, 0));
        for _ in 0..3 {
            g.p.pos.0 -= CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        assert_eq!(g.inf.as_ref().unwrap().center, (0, 0));
        assert_eq!(status_of(&g).integrity, hit, "the room keeps its damage");
    }

    /// A level-0 run with the relays for a population: the window's hunters are the residency's loaded ones.
    fn populated() -> (Game, Content, Catalog) {
        let (mut g, content, catalog) = run();
        g.relays_collected = 40;
        g.ensure_hunters(&content);
        (g, content, catalog)
    }

    /// Every hunter that exists on level 0, by entity id, at its absolute cell (the loaded ones shifted by the origin).
    fn absolute(g: &Game) -> HashMap<String, (f32, f32)> {
        let inf = g.inf.as_ref().unwrap();
        let o = inf.origin();
        let mut out = HashMap::new();
        for (id, b) in &inf.stored {
            out.insert(id.clone(), b.pos);
        }
        for (b, id) in g.hunters.iter().zip(&inf.loaded) {
            if !id.is_empty() {
                out.insert(id.clone(), (b.pos.0 + o.0 as f32, b.pos.1 + o.1 as f32));
            }
        }
        out
    }

    #[test]
    fn population_reads_the_progression_data() {
        let p = crate::game::progression::Progression::load();
        for d in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
            for r in 0..12 {
                assert_eq!(population(d, r), p.streamed_hunters(d, r), "{d:?} {r}");
            }
            let before = p.activation_relays(d);
            if before > 0 {
                assert_eq!(population(d, before - 1), 0, "{d:?}: none before the activation relay");
            }
            assert_eq!(population(d, before), p.gate(d).streamed_base, "{d:?}: the base lineup at activation");
        }
    }

    #[test]
    fn a_resident_floor_wander_target_lies_on_its_own_floor() {
        let (g, _, _) = populated();
        let mut rng = Rng::new("G20");
        let from = g.hunters.first().map(|b| b.pos).unwrap_or((10.0, 10.0));
        for _ in 0..20 {
            if let Some(q) = resident_wander_target(&g.fac, from, 6.0, &mut rng) {
                let c = crate::world::facility::Cell { x: q.0.floor() as i32, y: q.1.floor() as i32 };
                assert!(g.fac.hunter_may_spawn(c), "the target is standable on the floor given");
            }
        }
    }

    #[test]
    fn the_residency_loads_the_layout_spawns_of_the_window() {
        let (g, _, _) = populated();
        let inf = g.inf.as_ref().unwrap();
        assert!(!g.hunters.is_empty(), "a populated level 0 has hunters in its window");
        assert_eq!(inf.loaded.len(), g.hunters.len());
        assert!(g.hunters.len() as i32 <= population(g.difficulty, 40));
        let window = (CHUNK * 3) as f32;
        for (b, id) in g.hunters.iter().zip(&inf.loaded) {
            assert!(id.contains(":HUNTER:"), "{id} is a layout spawn id");
            assert!(b.pos.0 >= 0.0 && b.pos.0 < window && b.pos.1 >= 0.0 && b.pos.1 < window, "{id} at {:?}", b.pos);
            assert!(b.active, "a hunter wakes on its first load");
        }
        // a wanted hunter in the window is loaded, so nothing stored stands in it
        for (id, b) in &inf.stored {
            assert!(!residency::in_window(residency::chunk_of(b.pos), inf.center), "{id} is stored in the window");
        }
    }

    #[test]
    fn the_residency_is_the_same_on_the_same_seed() {
        let (a, _, _) = populated();
        let (b, _, _) = populated();
        assert_eq!(absolute(&a), absolute(&b));
    }

    #[test]
    fn a_hunters_absolute_cell_survives_the_window_moving_away_and_back() {
        let (mut g, content, catalog) = populated();
        let before = absolute(&g);
        assert!(!before.is_empty());
        for _ in 0..3 {
            g.p.pos.0 += CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        for _ in 0..3 {
            g.p.pos.0 -= CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        let after = absolute(&g);
        for (id, pos) in &before {
            assert_eq!(after.get(id), Some(pos), "{id} keeps its cell");
        }
    }

    #[test]
    fn a_hunter_outside_the_window_is_stored_and_a_loaded_one_is_in_it() {
        let (mut g, content, catalog) = populated();
        for _ in 0..2 {
            g.p.pos.0 += CHUNK as f32;
            g.stream_window(&content, &catalog);
        }
        let inf = g.inf.as_ref().unwrap();
        let o = inf.origin();
        for (b, id) in g.hunters.iter().zip(&inf.loaded) {
            if id.is_empty() {
                continue;
            }
            let abs = (b.pos.0 + o.0 as f32, b.pos.1 + o.1 as f32);
            assert!(residency::in_window(residency::chunk_of(abs), inf.center), "{id} is loaded in the window");
        }
        for (id, b) in &inf.stored {
            assert!(!residency::in_window(residency::chunk_of(b.pos), inf.center), "{id} is stored outside it");
        }
    }

    /// The specialty bar follows the hunters that EXIST on level 0, loaded or stored: a region switch (the window moving
    /// until residency stores a hunter, then back) must not take her slot, her box or the runner's hand away.
    #[test]
    fn a_stored_level_zero_hunter_keeps_her_slot_over_a_region_switch() {
        use crate::game::specialty::SpecItem;
        let (mut g, content, catalog) = populated();
        assert!(!g.hunters.is_empty(), "a populated level 0 has a hunter in its window");
        let id = g.inf.as_ref().unwrap().loaded[0].clone();
        let at = g.hunters[0].pos;
        let mut glyra = Brain::new(content.hunter("glyra").unwrap().clone(), at, "REGION:glyra", g.difficulty);
        glyra.activate_at(g.p.pos);
        g.hunters[0] = glyra;
        g.sync_specialty();
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "Glyra is loaded: slot 6 holds the box");
        g.press_specialty(0);
        g.music_box_left = 31.0;
        // east until residency stores her (she stays where she stands)
        let mut moved = 0;
        while !g.inf.as_ref().unwrap().stored.contains_key(&id) && moved < 8 {
            g.p.pos.0 += CHUNK as f32;
            g.stream_window(&content, &catalog);
            moved += 1;
        }
        assert!(g.inf.as_ref().unwrap().stored.contains_key(&id), "residency stored her after {moved} chunks");
        assert!(!g.hunters.iter().any(|h| h.def.id == "glyra"), "she is not in the window any more");
        g.sync_specialty();
        assert!(g.specialty.visible[0], "she still exists on level 0: the slot stays");
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "and so does the box");
        assert_eq!(g.specialty.selected, Some(0), "and the hand");
        assert_eq!(g.music_box_heard(), 31.0, "and the tune");
        // and back: she is loaded again, still the one box, nothing granted twice
        while g.inf.as_ref().unwrap().stored.contains_key(&id) && moved > 0 {
            g.p.pos.0 -= CHUNK as f32;
            g.stream_window(&content, &catalog);
            moved -= 1;
        }
        g.sync_specialty();
        assert!(g.hunters.iter().any(|h| h.def.id == "glyra"), "loaded again");
        assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
        assert_eq!(g.specialty.selected, Some(0));
        assert_eq!(g.slots.iter().flatten().filter(|s| s.item == Item::MusicBox).count(), 0);
    }

    /// Moves the run across stairwell `i` as the old swap did (to the far end of its flight, one level over), through
    /// `go_to_level`: for the tests that need the hunters of two levels stored and restored.
    fn take(g: &mut Game, i: usize, content: &Content, catalog: &Catalog) {
        let s = g.fac.stairs[i];
        let to = g.inf.as_ref().map_or(0, |inf| inf.level) + s.dir;
        g.go_to_level(to, s.far_end().center(), 0.0, content, catalog);
    }

    /// The stairwell of the window going `dir` (-1 down, +1 up), if it has one.
    fn stair(g: &Game, dir: i32) -> Option<usize> {
        g.fac.stairs.iter().position(|s| s.dir == dir)
    }

    /// The stairwell back: the landing the last `take` arrived on.
    fn stair_back(g: &Game, dir: i32) -> usize {
        let at = g.p.pos;
        let mut best = None;
        for (i, s) in g.fac.stairs.iter().enumerate() {
            if s.dir != dir {
                continue;
            }
            let d = pdist(s.cell.center(), at);
            if best.map_or(true, |(_, b)| d < b) {
                best = Some((i, d));
            }
        }
        best.expect("the landing is a stairwell back").0
    }

    /// Takes a stairwell going `dir` off level 0 and back: the level-0 hunters are where they stood.
    #[test]
    fn the_stairs_keep_the_level_zero_hunters() {
        let (mut g, content, catalog) = populated();
        let down = stair(&g, -1).or_else(|| stair(&g, 1)).expect("the start window has a stairwell");
        let dir = g.fac.stairs[down].dir;
        let before = absolute(&g);
        assert!(!before.is_empty());
        take(&mut g, down, &content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, dir);
        let back = stair_back(&g, -dir);
        take(&mut g, back, &content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, 0);
        let after = absolute(&g);
        for (id, pos) in &before {
            assert_eq!(after.get(id), Some(pos), "{id} keeps its cell over the stairs");
        }
    }

    /// Godot keeps every hunter on her own level: a level off level 0 keeps its hunters (the same brains, where they
    /// stood) while the runner is away, instead of a new lineup on every visit.
    #[test]
    fn a_floor_keeps_its_hunters_over_the_stairs() {
        let (mut g, content, catalog) = populated();
        let i = stair(&g, -1).or_else(|| stair(&g, 1)).expect("the start window has a stairwell");
        let dir = g.fac.stairs[i].dir;
        take(&mut g, i, &content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, dir);
        assert!(!g.hunters.is_empty(), "the floor has its lineup");
        // mark the floor's hunters: one moves to another open cell and one goes to sleep, which a rebuild would undo
        let spot = g
            .fac
            .floor_cells()
            .into_iter()
            .map(|c| c.center())
            .find(|p| g.fac.body_fits(p.0, p.1, HUNTER_RADIUS) && pdist(*p, g.hunters[0].pos) > 2.0)
            .expect("an open cell");
        g.hunters[0].pos = spot;
        g.hunters[0].active = false;
        let o = g.inf.as_ref().unwrap().origin();
        let mark = |g: &Game| -> Vec<(String, (f32, f32), bool)> {
            g.hunters
                .iter()
                .map(|h| (h.def.id.clone(), (h.pos.0 + o.0 as f32, h.pos.1 + o.1 as f32), h.active))
                .collect()
        };
        let before = mark(&g);
        let back = stair_back(&g, -dir);
        take(&mut g, back, &content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, 0);
        let again = stair_back(&g, dir);
        take(&mut g, again, &content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, dir);
        assert_eq!(g.inf.as_ref().unwrap().origin(), o, "the same window");
        assert_eq!(mark(&g), before, "the floor's hunters are the ones it kept");
    }

    fn voids(g: &Game) -> usize {
        g.hunters.iter().filter(|h| h.def.id.starts_with("void_")).count()
    }

    /// Takes every void hunter out of the list and its parallel residency ids, so a test starts with none.
    fn without_voids(g: &mut Game) {
        assert_eq!(g.inf.as_ref().unwrap().loaded.len(), g.hunters.len(), "the residency ids run parallel to the hunters");
        let keep: Vec<bool> = g.hunters.iter().map(|h| !h.def.id.starts_with("void_")).collect();
        let mut k = keep.iter();
        g.hunters.retain(|_| *k.next().unwrap());
        let mut k = keep.iter();
        g.inf.as_mut().unwrap().loaded.retain(|_| *k.next().unwrap());
        g.reset_director();
    }

    /// Seconds of standing still that build the anti-idle pressure up to its spawn mark, plus a margin (all from data).
    fn idle_to_spawn(g: &Game) -> f32 {
        let c = crate::game::anti_idle::cfg();
        c.grace_for(g.difficulty) + c.spawn_pressure / c.rise_per_s.max(1.0e-6) + 5.0
    }

    #[test]
    fn a_guest_idling_past_the_grace_raises_no_pressure_and_spawns_no_void_hunter() {
        let (mut g, content, _) = populated();
        g.remote_hunters = true;
        g.p.move_vel = (0.0, 0.0);
        let before = voids(&g);
        let (count, dt) = (g.hunters.len(), 0.5);
        let mut t = 0.0;
        while t < idle_to_spawn(&g) + crate::game::anti_idle::cfg().spawn_cooldown_s {
            g.time += dt;
            g.step_anti_idle(dt, &content);
            t += dt;
        }
        assert_eq!(g.anti_idle.pressure(), 0.0, "only the host's pressure changes");
        assert_eq!((voids(&g), g.hunters.len()), (before, count), "a guest's hunters come from the host's frames");
        assert!(!g.spawn_void_hunter(&content, "TEST:GUEST", None), "a guest never adds a void hunter");
        assert_eq!(g.hunters.len(), count);
    }

    #[test]
    fn a_solo_runner_idling_on_level_zero_draws_exactly_one_void_hunter_at_the_spawn_distance() {
        let (mut g, content, _) = populated();
        assert_eq!(g.inf.as_ref().unwrap().level, 0);
        without_voids(&mut g);
        g.p.move_vel = (0.0, 0.0);
        let c = crate::game::anti_idle::cfg();
        let dt = 0.5;
        // a spawn spot can miss (the point lands in solid rock): the next try comes after the cooldown, on a new stream
        let limit = idle_to_spawn(&g) + c.spawn_cooldown_s * 6.0;
        let mut t = 0.0;
        while t < limit && voids(&g) == 0 {
            g.time += dt;
            g.step_anti_idle(dt, &content);
            t += dt;
        }
        assert_eq!(voids(&g), 1, "standing still past the grace brings one void hunter");
        assert_eq!(g.inf.as_ref().unwrap().loaded.len(), g.hunters.len(), "the residency ids stay parallel");
        let v = g.hunters.iter().find(|h| h.def.id.starts_with("void_")).unwrap();
        let d = pdist(v.pos, g.p.pos);
        assert!((d - c.spawn_distance).abs() <= c.spawn_distance * 0.5, "spawned {d} cells away, want about {}", c.spawn_distance);
        // standing on, the cap holds: no second void hunter
        let mut t = 0.0;
        while t < c.spawn_cooldown_s * 2.0 {
            g.time += dt;
            g.step_anti_idle(dt, &content);
            t += dt;
        }
        assert_eq!(voids(&g), 1, "max_void_hunters caps the spawns");
        assert_eq!(g.inf.as_ref().unwrap().loaded.len(), g.hunters.len());
    }

    #[test]
    fn a_void_incursion_run_twice_brings_at_most_one_void_hunter() {
        let (mut g, content, _) = populated();
        without_voids(&mut g);
        g.depth = 40;
        g.void_incursion(&content);
        g.void_incursion(&content);
        assert!(voids(&g) <= 1, "the incursion rolls once a floor: {}", voids(&g));
        if voids(&g) == 0 {
            assert!(g.spawn_void_hunter(&content, "TEST:VOID", None));
        }
        g.void_incursion(&content);
        assert_eq!(voids(&g), 1, "a floor that already has a void hunter gets no second one");
        assert_eq!(g.inf.as_ref().unwrap().loaded.len(), g.hunters.len());
    }

    #[test]
    fn a_pickup_burst_saves_once_inside_the_minimum_interval() {
        let (mut g, _, _) = run();
        let a = crate::game::progression::Autosave::load();
        assert!(a.triggers.iter().any(|t| t == "pickup") && !a.always.iter().any(|t| t == "pickup"), "pickup is a rate-limited trigger");
        assert!(a.min_interval_s > 0.0, "the data has a minimum interval to test");
        g.time = 100.0;
        let before = g.autosave_count;
        assert!(g.autosave_on("pickup"), "the first pickup saves");
        g.time += a.min_interval_s * 0.5;
        assert!(!g.autosave_on("pickup"), "a second pickup inside the interval does not");
        assert_eq!(g.autosave_count, before + 1);
        assert_eq!(g.last_autosave, "pickup");
        g.time += a.min_interval_s;
        assert!(g.autosave_on("pickup"), "once the interval has run out it saves again");
        assert_eq!(g.autosave_count, before + 2);
    }

    #[test]
    fn holding_still_on_a_ladder_builds_anti_idle_pressure_and_climbing_drains_it() {
        use crate::game::ladder::Phase;
        let (mut g, content, catalog) = run();
        let i = g.fac.stairs.iter().position(|s| s.is_ladder() && s.dir < 0).expect("the start window has a ladder down");
        // the walk that reached the ladder leaves its speed in move_vel; the climb never clears it
        g.p.move_vel = (3.0, 0.0);
        assert!(g.start_ladder(i), "the runner mounts the ladder");
        let still = crate::game::Controls::default();
        let c = crate::game::anti_idle::cfg();
        let dt = 0.05;
        let mut t = 0.0;
        while t < c.grace_for(g.difficulty) + 10.0 {
            g.advance_ladder(dt, &still, &content, &catalog);
            g.time += dt;
            g.step_anti_idle(dt, &content);
            t += dt;
        }
        assert_eq!(g.ladder.map(|k| k.phase), Some(Phase::Climb), "the runner hangs on the shaft");
        assert!(g.p.move_vel.0.hypot(g.p.move_vel.1) > c.idle_speed, "the stale walk speed is still there");
        let held = g.anti_idle.pressure();
        assert!(held > 0.0, "a climber holding still is idle");
        // climbing down moves the body: the climb speed resets the idle and the pressure eases
        let down = crate::game::Controls { fwd: -1.0, ..crate::game::Controls::default() };
        let mut t = 0.0;
        while t < 0.5 {
            g.advance_ladder(dt, &down, &content, &catalog);
            g.time += dt;
            g.step_anti_idle(dt, &content);
            t += dt;
        }
        assert_eq!(g.ladder.map(|k| k.phase), Some(Phase::Climb));
        assert!(g.anti_idle.pressure() < held, "a climbing runner is not idle");
        assert_eq!(g.anti_idle.idle_seconds(), 0.0);
    }
}

#[cfg(test)]
mod waypoint_tests {
    use super::*;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    fn run() -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_infinite_sim(
            crate::config::Difficulty::Standard,
            7,
            "ROOM-TEST-7",
            0,
            &content,
            &catalog,
        );
        (g, content, catalog)
    }

    /// The window cell at the centre of cell `c`, as a window position.
    fn at(c: Cell) -> (f32, f32) {
        (c.x as f32 + 0.5, c.y as f32 + 0.5)
    }

    /// The first open window cell at least `min` cells from `from` that the live window can route to.
    fn open_far_cell(g: &Game, from: (f32, f32), min: f32) -> Cell {
        let start = Cell::new(from.0.floor() as i32, from.1.floor() as i32);
        for y in 0..g.fac.rows {
            for x in 0..g.fac.cols {
                let c = Cell::new(x, y);
                if g.fac.hunter_open(c) && pdist(from, at(c)) >= min && !g.fac.find_path(start, c).is_empty() {
                    return c;
                }
            }
        }
        panic!("no open cell {min} cells from {from:?}");
    }

    #[test]
    fn placing_snaps_to_the_cell_and_a_second_place_replaces_the_first() {
        let (mut g, _, _) = run();
        // a fresh run's window origin is (-CHUNK, -CHUNK) in absolute cells
        assert!(g.place_waypoint((30.7, 12.2)));
        let first = g.inf.as_ref().unwrap().waypoint.unwrap();
        assert_eq!(first.at, (-14.5, -32.5), "snapped to the cell centre, in absolute cells");
        assert_eq!(first.level, 0);
        assert_eq!(g.message, "WAYPOINT SET");
        assert_eq!(g.waypoint_marker(), Some((30.5, 12.5)), "the same cell in window coordinates");
        // another cell replaces it: one waypoint per player
        assert!(g.place_waypoint((50.2, 50.2)));
        assert_eq!(g.waypoint_marker(), Some((50.5, 50.5)));
    }

    #[test]
    fn placing_on_the_same_cell_clears_it() {
        let (mut g, _, _) = run();
        g.place_waypoint((30.2, 30.2));
        assert!(g.waypoint_marker().is_some());
        assert!(g.place_waypoint((30.9, 30.6)), "the same cell again");
        assert!(g.inf.as_ref().unwrap().waypoint.is_none());
        assert_eq!(g.message, "WAYPOINT CLEARED");
        assert!(!g.clear_waypoint(), "nothing left to clear");
    }

    #[test]
    fn the_route_runs_over_open_window_cells_to_the_waypoint() {
        let (mut g, _, _) = run();
        let target = open_far_cell(&g, g.p.pos, 12.0);
        g.place_waypoint(at(target));
        g.refresh_waypoint();
        let route = g.waypoint_route();
        assert!(route.len() >= 2, "a route of {} points", route.len());
        assert_eq!(route[0], (g.p.pos.0.floor() + 0.5, g.p.pos.1.floor() + 0.5), "starts on the runner's cell");
        assert_eq!(*route.last().unwrap(), at(target), "ends on the waypoint");
        for pair in route.windows(2) {
            let a = Cell::new(pair[0].0.floor() as i32, pair[0].1.floor() as i32);
            let b = Cell::new(pair[1].0.floor() as i32, pair[1].1.floor() as i32);
            assert!((a.x - b.x).abs() <= 1 && (a.y - b.y).abs() <= 1, "steps are neighbours");
            assert!(g.fac.hunter_open(a) && g.fac.hunter_open(b), "every step is open");
        }
        assert_eq!(*g.waypoint_line().last().unwrap(), at(target), "the floor line ends on it too");
    }

    #[test]
    fn a_waypoint_off_the_window_has_no_route_but_a_floor_line_and_a_marker() {
        let (mut g, _, _) = run();
        // far east of the window, on floor nobody has seen
        let far = (g.p.pos.0 + 200.0, g.p.pos.1);
        g.place_waypoint(far);
        g.refresh_waypoint();
        assert!(g.waypoint_route().is_empty(), "no live or discovered route reaches it");
        let line = g.waypoint_line();
        assert!(line.len() >= 2, "the floor line runs on to it");
        let marker = g.waypoint_marker().expect("the pin shows on its level");
        assert_eq!(*line.last().unwrap(), marker);
    }

    #[test]
    fn arriving_within_two_cells_clears_the_waypoint() {
        let (mut g, content, catalog) = run();
        let target = open_far_cell(&g, g.p.pos, 6.0);
        g.place_waypoint(at(target));
        // three cells short of it: not yet. The pin is read again after each step, since the window may recentre.
        let m = g.waypoint_marker().unwrap();
        g.p.pos = (m.0 - 3.0, m.1);
        g.stream_window(&content, &catalog);
        assert!(g.inf.as_ref().unwrap().waypoint.is_some());
        let m = g.waypoint_marker().unwrap();
        g.p.pos = m;
        g.stream_window(&content, &catalog);
        assert!(g.inf.as_ref().unwrap().waypoint.is_none());
        assert_eq!(g.message, "WAYPOINT REACHED");
        assert!(g.waypoint_route().is_empty());
    }

    #[test]
    fn a_waypoint_keeps_its_place_when_the_window_moves() {
        let (mut g, content, catalog) = run();
        let target = open_far_cell(&g, g.p.pos, 6.0);
        g.place_waypoint(at(target));
        let before = g.waypoint_marker().unwrap();
        g.p.pos.0 += CHUNK as f32;
        g.stream_window(&content, &catalog);
        let after = g.waypoint_marker().unwrap();
        assert_eq!(after, (before.0 - CHUNK as f32, before.1), "the window moved one chunk east under the pin");
        assert!(g.waypoint_line().len() >= 2);
    }

    #[test]
    fn a_waypoint_on_another_level_has_no_marker_and_no_route() {
        let (mut g, _, _) = run();
        let target = open_far_cell(&g, g.p.pos, 6.0);
        g.place_waypoint(at(target));
        g.inf.as_mut().unwrap().waypoint.as_mut().unwrap().level = -1;
        g.inf.as_mut().unwrap().route_key = None;
        g.refresh_waypoint();
        assert!(g.waypoint_marker().is_none());
        assert!(g.waypoint_route().is_empty());
        assert!(g.waypoint_line().is_empty());
    }

    #[test]
    fn the_discovered_floor_is_the_open_cells_of_the_discovered_chunks() {
        let (g, _, _) = run();
        let inf = g.inf.as_ref().unwrap();
        assert_eq!(inf.floor_chunks, 9);
        // the spawn cell is open; a cell of a chunk the window never loaded is not
        let spawn = (CHUNK + live::CENTRE, CHUNK + live::CENTRE);
        let origin = inf.origin();
        assert!(inf.floor.is_open((spawn.0 + origin.0, spawn.1 + origin.1)));
        assert!(!inf.floor.is_open((origin.0 + 3 * CHUNK + 5, origin.1 + 5)));
    }

    #[test]
    fn the_discovered_floor_splits_at_a_wall_and_routes_to_its_near_side() {
        let chunk = ChunkKey { cx: 0, cy: 0, level: 0 };
        let n = CHUNK as usize;
        // one chunk, open except a full wall down column x = 10
        let mut walls = vec![false; n * n];
        for z in 0..n {
            walls[z * n + 10] = true;
        }
        let chunks: HashSet<ChunkKey> = [chunk].into_iter().collect();
        let map: HashMap<ChunkKey, Vec<bool>> = [(chunk, walls)].into_iter().collect();
        let floor = DiscoveredFloor::build(&chunks, &map);
        assert!(floor.path((5, 5), (20, 5)).is_empty(), "the wall cuts the chunk in two");
        assert_eq!(floor.reachable_target((5, 5), (20, 5)), (9, 5), "the cell on this side nearest the waypoint");
        assert_eq!(floor.path((5, 5), (9, 5)).len(), 5);
        // a wall-free chunk routes straight across
        let open: HashMap<ChunkKey, Vec<bool>> = [(chunk, vec![false; n * n])].into_iter().collect();
        let floor = DiscoveredFloor::build(&chunks, &open);
        assert_eq!(floor.path((5, 5), (20, 5)).len(), 16);
    }

    #[test]
    fn the_line_follows_the_discovered_route_then_runs_straight_on() {
        let discovered = vec![(0, 0), (1, 0), (2, 0)];
        let line = choose_line(&discovered, (0, 0), (9, 0));
        assert_eq!(line.len(), 4, "three cells, then the straight run to the waypoint");
        assert_eq!(*line.last().unwrap(), (9.5, 0.5));
        // no discovered route: one straight segment from the runner
        assert_eq!(choose_line(&[], (0, 0), (4, 4)), vec![(0.5, 0.5), (4.5, 4.5)]);
        assert!(choose_line(&[], (3, 3), (3, 3)).is_empty());
        // a discovered route that reaches the waypoint is the whole line
        let arrived = vec![(0, 0), (0, 1)];
        assert_eq!(choose_line(&arrived, (0, 0), (0, 1)).len(), 2);
    }

    #[test]
    fn chevrons_are_spaced_one_point_six_cells_in_the_first_28() {
        let route = vec![(0.0, 0.0), (50.0, 0.0)];
        let marks = chevrons(&route);
        // 1.0, 2.6, ... up to 26.6 (the next, 28.2, is past the trail length)
        assert_eq!(marks.len(), 17);
        assert!((marks[0].at.0 - 1.0).abs() < 1e-4);
        assert!((marks.last().unwrap().at.0 - 26.6).abs() < 1e-3);
        assert_eq!(marks[0].dir, (1.0, 0.0));
        assert!(chevrons(&[(1.0, 1.0)]).is_empty());
    }
}

/// A walked flight relabels the level on the frame the body crosses (`walk_relabel`): once per crossing, with the body
/// not moved, no fade, no load and no hold after it.
#[cfg(test)]
mod walked_stair_tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::content::Content;
    use crate::game::Controls;
    use crate::world::catalog::Catalog;
    use crate::headless::steer::{walk_flight, walk_flight_until};
    use crate::world::facility::LinkKind;
    use crate::world::Cell;

    const FRAME: f32 = 1.0 / 60.0;

    /// Opens the whole run of a flight. infinite.rs opens the link box from the landing row only, so a flight along y is
    /// walled after its first cell (a separate bug, outside this change); the test opens the rest of its own flight.
    fn open_run(g: &mut Game, s: Stair) {
        let cells = s.carve_cells();
        let x0 = cells.iter().map(|c| c.x).min().unwrap_or(s.cell.x) - 1;
        let y0 = cells.iter().map(|c| c.y).min().unwrap_or(s.cell.y) - 1;
        let x1 = cells.iter().map(|c| c.x).max().unwrap_or(s.cell.x) + 1;
        let y1 = cells.iter().map(|c| c.y).max().unwrap_or(s.cell.y) + 1;
        g.fac.open_cells(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
    }

    /// A level-0 window with a flight going `dir` (-1 down, +1 up), the body stood on its landing facing up it. The
    /// first frame recentres the window on the body, so the flight is looked up again after it.
    fn on_landing(dir: i32, content: &Content, catalog: &Catalog) -> (Game, Stair) {
        for n in 0..60 {
            let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, &format!("ROOM-WALK-{n}"), 0, content, catalog);
            let Some(first) = g.fac.stairs.iter().copied().find(|s| s.dir == dir && matches!(s.kind, LinkKind::Stairs)) else {
                continue;
            };
            g.p.pos = first.cell.center();
            g.step(FRAME, &Controls::default(), content, catalog, &Sounds::default());
            let pos = g.p.pos;
            let s = g
                .fac
                .stairs
                .iter()
                .copied()
                .filter(|s| s.dir == dir && matches!(s.kind, LinkKind::Stairs))
                .min_by(|a, b| {
                    let (da, db) = (super::pdist(a.cell.center(), pos), super::pdist(b.cell.center(), pos));
                    da.partial_cmp(&db).unwrap()
                })
                .unwrap();
            open_run(&mut g, s);
            g.p.pos = s.cell.center();
            g.p.yaw = (-(s.run.0 as f32)).atan2(-(s.run.1 as f32));
            g.p.pitch = 0.0;
            return (g, s);
        }
        panic!("no test window has a flight going {dir}");
    }

    /// Walks `frames` frames with the forward key at `fwd` through `step` (the game's own frame). Returns the number of
    /// storey changes, and checks that no fade starts: the fade is cleared first and must stay clear on every frame.
    fn walk(g: &mut Game, content: &Content, catalog: &Catalog, frames: usize, fwd: f32) -> usize {
        let c = Controls { fwd, ..Default::default() };
        let mut changes = 0;
        let mut prev = g.inf.as_ref().map_or(0, |i| i.level);
        g.fade = 0.0;
        for _ in 0..frames {
            g.step(FRAME, &c, content, catalog, &Sounds::default());
            let now = g.inf.as_ref().map_or(0, |i| i.level);
            if now != prev {
                changes += 1;
                prev = now;
            }
            assert_eq!(g.fade, 0.0, "a walked flight creates no fade");
        }
        changes
    }

    /// Stands the body a quarter of the way down the flight `s` (on the active level's side of the midpoint), on its tread.
    fn on_flight(g: &mut Game, s: Stair) {
        let path = s.walk_path(g.fac.level, 0.25);
        let p = path[path.len() / 4];
        g.p.pos = p;
        g.p.height = stair_height_at(&g.fac, p).expect("on the flight");
    }

    /// A window with the neighbour of a flight going `dir` resident and the body on the flight: (game, flight, neighbour).
    fn with_neighbour(dir: i32, content: &Content, catalog: &Catalog) -> (Game, Stair, i32) {
        let (mut g, s) = on_landing(dir, content, catalog);
        let to = g.fac.level + s.dir;
        on_flight(&mut g, s);
        g.keep_stack(content, catalog);
        assert!(g.inf.as_ref().unwrap().stack.has(to), "the neighbour of the flight the body is on is resident");
        (g, s, to)
    }

    #[test]
    fn relabel_keeps_x_z_y_velocity_and_facing() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _, to) = with_neighbour(-1, &content, &catalog);
        g.p.move_vel = (0.7, -0.3);
        g.p.yaw = 1.2;
        g.p.pitch = 0.1;
        let (pos, y, eye, vel, yaw, pitch) = (g.p.pos, g.p.y(), g.p.eye_y(), g.p.move_vel, g.p.yaw, g.p.pitch);
        assert!(g.relabel(to, &content, &catalog), "a resident neighbour is relabelled");
        assert_eq!(g.inf.as_ref().unwrap().level, to);
        assert_eq!(g.fac.level, to);
        assert_eq!(g.p.floor_y, floor_y_of(to));
        assert_eq!(g.p.pos, pos, "x and z are untouched");
        assert!((g.p.y() - y).abs() < 1e-5, "the absolute height is untouched: {} -> {}", y, g.p.y());
        assert!((g.p.eye_y() - eye).abs() < 1e-5, "so the eye does not move: a level change is 0 m");
        assert_eq!((g.p.move_vel, g.p.yaw, g.p.pitch), (vel, yaw, pitch), "velocity and facing are untouched");
        assert_eq!(g.depth, 1 - to);
    }

    #[test]
    fn relabel_calls_no_load_window() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _, to) = with_neighbour(-1, &content, &catalog);
        g.fade = 0.0;
        let (builds, pos, hidden) = (g.inf.as_ref().unwrap().builds, g.p.pos, g.p.hidden);
        assert!(g.relabel(to, &content, &catalog));
        assert_eq!(g.inf.as_ref().unwrap().builds, builds, "no window was built");
        assert_eq!(g.fade, 0.0, "no fade");
        assert_eq!(g.p.pos, pos, "the body was not placed");
        assert_eq!(g.p.hidden, hidden);
        let inf = g.inf.as_ref().unwrap();
        assert!(inf.stack.has(0), "the level left is now the resident neighbour");
        assert!(!inf.stack.has(to), "the level entered is the live window");
    }

    #[test]
    fn relabel_refuses_a_level_that_is_not_resident() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _, to) = with_neighbour(-1, &content, &catalog);
        let pos = g.p.pos;
        assert!(!g.relabel(to - 5, &content, &catalog), "no resident, no relabel");
        assert!(!g.relabel(0, &content, &catalog), "the active level is not a neighbour");
        g.inf.as_mut().unwrap().stack.residents.clear();
        assert!(!g.relabel(to, &content, &catalog), "an emptied stack relabels nothing");
        assert_eq!(g.inf.as_ref().unwrap().level, 0);
        assert_eq!(g.p.pos, pos);
        assert_eq!(g.p.floor_y, floor_y_of(0));
    }

    #[test]
    fn a_relabel_swaps_the_two_windows_and_back_restores_them() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _, to) = with_neighbour(-1, &content, &catalog);
        let upper = (g.fac.stairs.len(), g.fac.cols, g.fac.rows);
        let lower = g.inf.as_ref().unwrap().stack.get(to).map(|r| (r.fac.stairs.len(), r.fac.cols, r.fac.rows)).unwrap();
        assert!(g.relabel(to, &content, &catalog));
        assert_eq!((g.fac.stairs.len(), g.fac.cols, g.fac.rows), lower, "the live window is the one that was resident");
        assert!(g.relabel(0, &content, &catalog));
        assert_eq!((g.fac.stairs.len(), g.fac.cols, g.fac.rows), upper, "and back");
    }

    #[test]
    fn a_walked_relabel_happens_on_the_frame_the_height_crosses_and_not_before() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, s, _) = with_neighbour(-1, &content, &catalog);
        // a quarter of the way down is still the upper level: stepping in place relabels nothing
        let before = g.inf.as_ref().unwrap().level;
        g.walk_relabel(&content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, before, "above the crossing the label stays");
        let w = walk_flight(&mut g, s, &content, &catalog, 900);
        assert_eq!(w.level_changes, 1);
    }

    #[test]
    fn stack_residents_all_share_the_window_centre() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, s, _) = with_neighbour(-1, &content, &catalog);
        walk_flight(&mut g, s, &content, &catalog, 900);
        for _ in 0..4 {
            g.keep_stack(&content, &catalog);
        }
        let inf = g.inf.as_ref().unwrap();
        assert!(!inf.stack.residents.is_empty());
        assert!(inf.stack.residents.iter().all(|r| r.center == inf.center), "every resident is at the window's centre");
        assert!(!inf.stack.has(inf.level), "the active level is never also a resident");
    }

    #[test]
    fn recentre_drops_stale_residents_and_rebuilds_the_neighbour_on_a_flight() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, s) = on_landing(-1, &content, &catalog);
        let to = g.fac.level + s.dir;
        on_flight(&mut g, s);
        // the window one chunk west of where it was, the body in its east chunk: the same spot, about to recentre
        {
            let inf = g.inf.as_mut().unwrap();
            inf.center.0 -= 1;
            inf.stack.residents.clear();
        }
        g.p.pos.0 += CHUNK as f32;
        g.load_window(&content, &catalog);
        let on = stair_height_at(&g.fac, g.p.pos);
        assert!(on.is_some(), "the body is still on the flight in the shifted window");
        g.p.height = on.unwrap();
        g.keep_stack(&content, &catalog);
        let stale = g.inf.as_ref().unwrap().center;
        assert!(g.inf.as_ref().unwrap().stack.has(to));
        let builds = g.inf.as_ref().unwrap().builds;
        g.recentre_window(&content, &catalog);
        let inf = g.inf.as_ref().unwrap();
        assert_eq!(inf.center, (stale.0 + 1, stale.1), "the window moved");
        assert!(inf.stack.residents.iter().all(|r| r.center == inf.center), "no resident of the old centre is kept");
        assert!(inf.builds > builds, "the neighbour was built again at the new centre");
        if stair_height_at(&g.fac, g.p.pos).is_some() {
            assert!(inf.stack.has(to), "on a flight the neighbour is back on the frame of the recentre");
        }
    }

    #[test]
    fn keep_stack_keeps_neighbour_while_on_a_flight() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _s, to) = with_neighbour(-1, &content, &catalog);
        // a far-away marker is not evicted while a flight is walked, and the neighbour stays through any number of frames
        let mut marker = g.inf.as_mut().unwrap().stack.take(to).expect("the neighbour");
        marker.level = 40;
        g.inf.as_mut().unwrap().stack.put(marker);
        for _ in 0..6 {
            g.keep_stack(&content, &catalog);
            let st = &g.inf.as_ref().unwrap().stack;
            assert!(st.has(to), "the flight's neighbour is resident on every frame");
            assert!(st.has(40), "nothing is evicted while on a flight");
        }
        // off the flight the same marker goes
        g.p.pos = g.fac.start.center();
        g.p.height = 0.0;
        assert!(stair_height_at(&g.fac, g.p.pos).is_none());
        g.keep_stack(&content, &catalog);
        assert!(!g.inf.as_ref().unwrap().stack.has(40), "a level far away is evicted off the flight");
    }


    /// A level -1 game whose centre chunk holds an open drop (the falls void), with the lip cell to stand on and the
    /// unit step from it into the void: (game, lip cell, step).
    fn falls_game(content: &Content, catalog: &Catalog) -> (Game, Cell, (i32, i32)) {
        for n in 0..120 {
            let g = Game::new_infinite_sim(
                crate::config::Difficulty::Standard,
                7,
                &format!("ROOM-FALL-{n}"),
                crate::world::route::sewer::core::LEVEL,
                content,
                catalog,
            );
            // the centre chunk spans [CHUNK, 2 CHUNK) of the 3x3 window: far enough from its edge that the window holds still
            let inside = |c: Cell| c.x >= CHUNK + 2 && c.x < 2 * CHUNK - 2 && c.y >= CHUNK + 2 && c.y < 2 * CHUNK - 2;
            for &d in g.fac.drop_cells() {
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let lip = Cell::new(d.x - dx, d.y - dz);
                    let (x, z) = lip.center();
                    if inside(lip)
                        && inside(d)
                        && !g.fac.is_drop(lip)
                        && !g.fac.is_hole(lip.x, lip.y)
                        && g.fac.body_fits(x, z, PLAYER_RADIUS)
                    {
                        return (g, lip, (dx, dz));
                    }
                }
            }
        }
        panic!("no level -1 test window has a drop with a standable lip");
    }

    fn idle_frames(g: &mut Game, content: &Content, catalog: &Catalog, frames: usize) {
        for _ in 0..frames {
            g.step(FRAME, &Controls::default(), content, catalog, &Sounds::default());
        }
    }

    #[test]
    fn keep_stack_keeps_the_level_below_near_a_drop() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, lip, _) = falls_game(&content, &catalog);
        let below = crate::world::route::sewer::core::LOWER_LEVEL;
        g.p.pos = lip.center();
        assert!(
            g.fac.drop_within(g.p.pos.0, g.p.pos.1, super::super::transition::links().prefetch_distance),
            "the lip is within reach of the void"
        );
        // the connection in reach is built on the frame (the first call after a relabel only clears its flag)
        g.keep_stack(&content, &catalog);
        g.keep_stack(&content, &catalog);
        assert!(g.inf.as_ref().unwrap().stack.has(below), "the level under the void is resident beside it");
        // and it stays through any number of frames
        for _ in 0..6 {
            g.keep_stack(&content, &catalog);
            assert!(g.inf.as_ref().unwrap().stack.has(below));
        }
    }

    #[test]
    fn keep_stack_never_evicts_during_a_climb_or_a_fall() {
        let (content, catalog) = (Content::load(), Catalog::load());
        // a climb: the ladder is the link, a far marker level survives it and goes once the climb is over
        let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-TEST-7", 0, &content, &catalog);
        let i = g.fac.stairs.iter().position(|s| s.is_ladder() && s.dir < 0).expect("the origin chunk has a ladder down");
        let top = g.fac.stairs[i];
        g.p.pos = top.step_cell().center();
        idle_frames(&mut g, &content, &catalog, 3);
        let i = g.fac.stairs.iter().position(|s| s.is_ladder() && s.dir < 0 && s.cell == top.cell).unwrap_or(i);
        g.keep_stack(&content, &catalog);
        g.keep_stack(&content, &catalog);
        let below = g.inf.as_ref().unwrap().level - 1;
        let mut marker = g.inf.as_mut().unwrap().stack.take(below).expect("the level below the ladder is resident");
        marker.level = 40;
        g.inf.as_mut().unwrap().stack.put(marker);
        assert!(g.start_ladder(i), "the climb starts");
        for _ in 0..6 {
            g.keep_stack(&content, &catalog);
            let st = &g.inf.as_ref().unwrap().stack;
            assert!(st.has(40), "nothing is evicted during a climb");
            assert!(st.has(below), "the ladder's far end stays resident");
        }
        g.ladder = None;
        g.p.pos = g.fac.start.center();
        for _ in 0..2 {
            g.keep_stack(&content, &catalog);
        }
        let reach = super::super::transition::links().prefetch_distance;
        let far = g.fac.stairs.iter().all(|s| pdist(s.cell.center(), g.p.pos) > reach);
        if far {
            assert!(!g.inf.as_ref().unwrap().stack.has(40), "off the climb the far level goes");
        }
    }

    #[test]
    fn a_ladder_climb_down_calls_no_load_window() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-TEST-7", 0, &content, &catalog);
        let top = g.fac.stairs.iter().copied().find(|s| s.is_ladder() && s.dir < 0).expect("a ladder down");
        g.p.pos = top.step_cell().center();
        idle_frames(&mut g, &content, &catalog, 90);
        let here = g.p.pos;
        let i = g
            .fac
            .stairs
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_ladder() && s.dir < 0)
            .min_by(|a, b| {
                let (da, db) = (pdist(a.1.cell.center(), here), pdist(b.1.cell.center(), here));
                da.partial_cmp(&db).unwrap()
            })
            .map(|(i, _)| i)
            .unwrap();
        assert!(g.inf.as_ref().unwrap().stack.has(-1), "the shaft's lower end is resident before the climb");
        let builds = g.inf.as_ref().unwrap().builds;
        assert!(g.start_ladder(i));
        // the mount arms the climb keys only after a release
        g.step(FRAME, &Controls::default(), &content, &catalog, &Sounds::default());
        let down = Controls { fwd: -1.0, ..Default::default() };
        let mut flipped = false;
        for _ in 0..1500 {
            g.step(FRAME, &down, &content, &catalog, &Sounds::default());
            assert_eq!(g.fade, 0.0, "a climb creates no fade");
            if g.inf.as_ref().unwrap().level == -1 {
                flipped = true;
                assert_eq!(g.inf.as_ref().unwrap().builds, builds, "the label flips with no window built");
                break;
            }
        }
        assert!(flipped, "the climb down relabels the level to -1 by itself");
    }

    #[test]
    fn a_walk_off_the_lip_falls_through_the_relabel_with_no_pos_rewrite() {
        use crate::world::route::sewer::core::{LEVEL, LOWER_LEVEL};
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, lip, (dx, dz)) = falls_game(&content, &catalog);
        g.p.pos = lip.center();
        g.p.yaw = (-(dx as f32)).atan2(-(dz as f32)); // forward is (dx, dz)
        g.p.pitch = 0.0;
        g.p.height = 0.0;
        idle_frames(&mut g, &content, &catalog, 90);
        assert!(g.inf.as_ref().unwrap().stack.has(LOWER_LEVEL), "the level under the void is resident");
        assert!(pdist(g.p.pos, lip.center()) < 0.6, "the runner stood on the lip");
        assert_eq!(g.inf.as_ref().unwrap().level, LEVEL);
        let builds = g.inf.as_ref().unwrap().builds;
        let abs = |g: &Game| {
            let o = g.window_origin();
            (o.0 as f32 + g.p.pos.0, o.1 as f32 + g.p.pos.1)
        };
        let walk = Controls { fwd: 1.0, ..Default::default() };
        let (mut prev, start_y) = (abs(&g), g.p.y());
        let mut prev_y = start_y;
        let mut levels = vec![LEVEL];
        let (mut fall_start, mut fall_frames, mut was_falling) = (None, 0usize, false);
        let mut builds_at_flip = None;
        let mut landed = false;
        for frame in 0..900usize {
            g.step(FRAME, &walk, &content, &catalog, &Sounds::default());
            let now = abs(&g);
            assert!(
                pdist(now, prev) <= PLAYER_SPRINT_SPEED * FRAME * 1.2,
                "x and z move only by the controls (frame {frame}): {prev:?} -> {now:?}"
            );
            prev = now;
            let y = g.p.y();
            if fall_start.is_some() {
                assert!(y <= prev_y + 1e-4, "y falls monotonically: {prev_y} -> {y} (frame {frame})");
            }
            prev_y = y;
            if fall_start.is_none() && y < start_y - 1e-3 {
                fall_start = Some(frame);
                // a fall never evicts what is resident, however often the stack is kept
                let before = g.inf.as_ref().unwrap().stack.levels();
                for _ in 0..4 {
                    g.keep_stack(&content, &catalog);
                }
                let st = &g.inf.as_ref().unwrap().stack;
                assert!(before.iter().all(|l| st.has(*l)), "nothing resident is evicted in a fall");
            }
            let lvl = g.inf.as_ref().unwrap().level;
            if lvl != *levels.last().unwrap() {
                levels.push(lvl);
                builds_at_flip = Some(g.inf.as_ref().unwrap().builds);
            }
            if fall_start.is_some() {
                fall_frames += 1;
            }
            if g.p.falling() {
                was_falling = true;
            } else if was_falling {
                landed = true;
                break;
            }
        }
        assert!(landed, "the runner fell and landed");
        let expected = (2.0 * crate::world::vertical::STOREY / PLAYER_GRAVITY).sqrt();
        let took = fall_frames as f32 * FRAME;
        assert!((took - expected).abs() < 0.1, "a storey takes sqrt(2 STOREY / g) = {expected} s, took {took} s");
        assert_eq!(levels, vec![LEVEL, LOWER_LEVEL], "the label flips once, to the level below");
        assert_eq!(builds_at_flip, Some(builds), "the flip builds no window (change_storey and load_window are not called)");
        assert_eq!(g.fade, 0.0);
        assert!((g.p.y() - floor_y_of(LOWER_LEVEL)).abs() < 0.05, "the runner stands on the level below: y = {}", g.p.y());
        // the landing rule: a pool takes the body (splash, soaked, no stun), anything else stuns
        let min_depth = crate::game::swim::fall_rules().pool.min_depth;
        if g.runner_water_depth() > min_depth {
            assert!(g.swim.is_soaked(), "landing in the pool soaks the runner");
            assert_eq!(g.p.stun, 0.0, "a pool landing does not stun");
        } else {
            assert!(g.p.stun > 0.0, "landing outside the pool stuns");
        }
    }

    #[test]
    fn change_storey_takes_the_resident_neighbour_without_a_build() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _, to) = with_neighbour(-1, &content, &catalog);
        let built = g.inf.as_ref().unwrap().builds;
        let at = g.p.pos;
        g.go_to_level(to, at, 0.0, &content, &catalog);
        let inf = g.inf.as_ref().unwrap();
        assert_eq!(inf.level, to, "the change lands on the storey beyond");
        assert_eq!(inf.builds, built, "the change itself builds no window");
        assert!(!inf.stack.has(to), "the window held ready is the one swapped in");
    }

    #[test]
    fn a_walk_down_past_the_midpoint_changes_storey_once() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, s) = on_landing(-1, &content, &catalog);
        let w = walk_flight(&mut g, s, &content, &catalog, 900);
        assert_eq!(w.level_changes, 1);
        assert_eq!(g.inf.as_ref().unwrap().level, -1);
        assert_eq!(g.depth, 2, "the storey below is depth 2");
    }

    #[test]
    fn walking_back_up_changes_storey_once_and_does_not_double_change() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, s) = on_landing(-1, &content, &catalog);
        let flight = s.flight(0).expect("the flight");
        assert_eq!(walk_flight(&mut g, s, &content, &catalog, 900).level_changes, 1);
        // follow the same flight back up from the foot below: one change up, and standing still changes nothing
        let foot = g
            .fac
            .stairs
            .iter()
            .copied()
            .find(|t| t.dir > 0 && t.flight(-1) == Some(flight))
            .expect("the lower end of the flight");
        assert_eq!(walk_flight(&mut g, foot, &content, &catalog, 900).level_changes, 1, "one change back up");
        assert_eq!(g.inf.as_ref().unwrap().level, 0);
        assert_eq!(walk(&mut g, &content, &catalog, 300, 0.0), 0, "standing still changes nothing");
    }

    #[test]
    fn the_cell_is_continuous_across_the_change() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let abs = |g: &Game| {
            let o = g.inf.as_ref().unwrap().origin();
            (g.p.pos.0 + o.0 as f32, g.p.pos.1 + o.1 as f32)
        };
        // the same deterministic walk twice: to the frame of the change, and to one frame short of it
        let (mut g, s) = on_landing(-1, &content, &catalog);
        let w = walk_flight_until(&mut g, s, &content, &catalog, 900, true);
        assert_eq!(g.inf.as_ref().unwrap().level, -1, "the walk changed the storey");
        let after = abs(&g);
        let (mut g2, s2) = on_landing(-1, &content, &catalog);
        walk_flight(&mut g2, s2, &content, &catalog, w.frames - 1);
        let before = abs(&g2);
        // one frame of walking is at most ~0.07 m; a change that moved the cell would be far more
        assert!(
            (before.0 - after.0).abs() < 0.1 && (before.1 - after.1).abs() < 0.1,
            "the cell moved across the change: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn a_change_holds_nothing_the_body_keeps_walking_on_the_next_frame() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, s) = on_landing(-1, &content, &catalog);
        walk_flight_until(&mut g, s, &content, &catalog, 900, true);
        assert_eq!(g.inf.as_ref().unwrap().level, -1, "the walk changed the storey");
        // the walker leaves the body facing the next waypoint, so forward keeps it moving: no halt
        let c = Controls { fwd: 1.0, ..Default::default() };
        let at_change = g.p.pos;
        g.step(FRAME, &c, &content, &catalog, &Sounds::default());
        assert!(pdist(g.p.pos, at_change) > 1e-4, "the body halted after the change");
    }

    fn flight(cell: (i32, i32), dir: i32, ladder: bool) -> Stair {
        let at = Cell::new(cell.0, cell.1);
        if ladder {
            Stair::ladder(at, dir, crate::world::facility::Cover::Manhole)
        } else {
            Stair::stairs(at, dir, (1, 0), crate::world::flight::FlightShape::Switchback, true)
        }
    }

    #[test]
    fn connections_in_reach_name_the_storeys_they_lead_to() {
        let near = (10.5, 10.5);
        let stairs = [flight((10, 10), -1, false), flight((11, 10), 0, true), flight((40, 40), 1, false)];
        let keep = |level: i32, reach: f32, st: &[Stair], on_flight: bool| {
            let mut v = levels_to_keep(level, near, reach, st, on_flight, infinite::level_allowed);
            v.sort();
            v
        };
        // a flight goes where its dir says, a ladder both ways; level 0 has nothing above it; the far flight is out of reach
        assert_eq!(keep(0, 8.0, &stairs, false), vec![-1]);
        assert_eq!(keep(-1, 8.0, &stairs, false), vec![-2, 0]);
        assert!(keep(0, 0.2, &stairs[2..], false).is_empty());
        // a wider reach from data takes the far flight in (up from -1 is 0)
        assert!(keep(-1, 80.0, &stairs, false).contains(&0));
        // on a flight both sides of the active level are kept, whatever is in reach
        assert_eq!(keep(-1, 0.2, &[], true), vec![-2, 0]);
        assert_eq!(keep(0, 0.2, &[], true), vec![-1]);
    }

    #[test]
    fn the_levels_near_the_runner_are_all_kept_within_the_frame_budget() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _) = on_landing(-1, &content, &catalog);
        let level = g.inf.as_ref().unwrap().level;
        let cfg = super::super::preload::preload();
        let keep = super::super::preload::keep_levels(level, &cfg, infinite::level_allowed, &[]);
        assert!(keep.len() > 1, "more than the adjacent storey is kept (the active level is not listed)");
        for _ in 0..=keep.len() {
            g.keep_stack(&content, &catalog);
        }
        let inf = g.inf.as_ref().unwrap();
        for l in keep.iter().filter(|l| **l != level) {
            assert!(inf.stack.has(*l), "level {l} is resident");
        }
        assert!(inf.stack.residents.len() <= cfg.max_windows, "the window count stays within the cap");
        // the first call builds the connection in reach plus at most the per-frame budget
        let (mut g2, _) = on_landing(-1, &content, &catalog);
        let before = g2.inf.as_ref().unwrap().builds;
        g2.keep_stack(&content, &catalog);
        let built = g2.inf.as_ref().unwrap().builds - before;
        assert!(built as usize <= 1 + cfg.builds_per_frame + 1, "a frame builds a bounded number of windows: {built}");
    }

    #[test]
    fn a_window_outside_the_kept_levels_is_dropped_and_counted_in_bytes() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _) = on_landing(-1, &content, &catalog);
        g.keep_stack(&content, &catalog);
        let inf = g.inf.as_mut().unwrap();
        let mut r = inf.stack.residents.pop().expect("a window was kept");
        assert!(r.bytes() > 0, "a built window has a byte size");
        r.level = 40;
        inf.stack.put(r);
        // nothing resident is dropped while the body is on a flight (the landing counts), so step off it first
        g.keep_stack(&content, &catalog);
        assert!(g.inf.as_ref().unwrap().stack.has(40), "no resident is dropped while the body is on a flight");
        g.p.pos = g.fac.start.center();
        assert!(stair_height_at(&g.fac, g.p.pos).is_none(), "the start is off every flight");
        g.keep_stack(&content, &catalog);
        assert!(!g.inf.as_ref().unwrap().stack.has(40), "a level far away is evicted");
    }

    fn level0_nyxies(g: &Game) -> usize {
        g.hunters.iter().filter(|h| matches!(&h.gimmick, Gimmick::Nyxie(n) if n.dead_only)).count()
    }

    #[test]
    fn the_level_zero_nyxie_is_made_by_a_dead_flashlight_and_only_then() {
        let (content, catalog) = (Content::load(), Catalog::load());
        for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
            let mut g = Game::new_infinite_sim(diff, 7, "ROOM-NYXIE-MAKE", 0, &content, &catalog);
            let before = g.hunters.len();
            assert_eq!(level0_nyxies(&g), 0, "{diff:?}: the lineup of a new run has no Nyxie");
            g.keep_level0_nyxie(&content);
            assert_eq!(level0_nyxies(&g), 0, "{diff:?}: a working flashlight makes none");
            g.p.lamp = 0.0;
            g.keep_level0_nyxie(&content);
            assert_eq!(level0_nyxies(&g), 1, "{diff:?}: a dead flashlight makes one, whatever the relays");
            assert_eq!(g.relays_collected, 0);
            assert_eq!(g.hunters.len(), before + 1, "{diff:?}: the route's own hunters are untouched");
            assert!(!g.hunters.last().unwrap().active, "{diff:?}: dormant until she wakes");
            g.keep_level0_nyxie(&content);
            assert_eq!(level0_nyxies(&g), 1, "{diff:?}: never two");
        }
    }

    #[test]
    fn the_level_zero_nyxie_follows_the_wake_file_switch_and_the_room_kind() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let rules = crate::hunters::gimmicks::nyxie::wake_rules();
        let mut off = rules.clone();
        off.ignore_activation_gate = false;
        let mut g = Game::new_infinite_sim(Difficulty::Relaxed, 7, "ROOM-NYXIE-SWITCH", 0, &content, &catalog);
        g.p.lamp = 0.0;
        g.keep_level0_nyxie_with(&content, &off);
        assert_eq!(level0_nyxies(&g), 0, "switch off: level 0 has no Nyxie, as in Godot");
        g.no_local_runner = true;
        g.keep_level0_nyxie_with(&content, rules);
        assert_eq!(level0_nyxies(&g), 0, "a room with no local runner has nobody whose lamp could die");
        g.no_local_runner = false;
        g.remote_hunters = true;
        g.keep_level0_nyxie_with(&content, rules);
        assert_eq!(level0_nyxies(&g), 0, "a guest only shows the hunters the host sends");
        g.remote_hunters = false;
        g.keep_level0_nyxie_with(&content, rules);
        assert_eq!(level0_nyxies(&g), 1, "switch on, a local runner: she is made");
    }

    #[test]
    fn the_level_zero_nyxie_waits_on_the_shelf_and_comes_back_once() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(Difficulty::Relaxed, 7, "ROOM-NYXIE-TRAVEL", 0, &content, &catalog);
        g.p.lamp = 0.0;
        g.keep_level0_nyxie(&content);
        assert_eq!(level0_nyxies(&g), 1, "made by the dead flashlight");
        let at = g.p.pos;
        g.go_to_level(-1, at, 0.0, &content, &catalog);
        assert_eq!(level0_nyxies(&g), 0, "level -1 has its own Nyxie, not the level-0 one");
        assert!(g.level0_nyxie_exists(), "she waits on the level-0 shelf");
        g.keep_level0_nyxie(&content);
        g.go_to_level(0, at, 0.0, &content, &catalog);
        assert_eq!(level0_nyxies(&g), 1, "back on level 0 she is one again, not two");
        g.keep_level0_nyxie(&content);
        for _ in 0..30 {
            g.step(FRAME, &Controls::default(), &content, &catalog, &Sounds::default());
        }
        assert_eq!(level0_nyxies(&g), 1, "and the frames and the residency pass do not make another");
        assert_eq!(g.inf.as_ref().unwrap().loaded.len(), g.hunters.len(), "the residency ids stay aligned with the list");
    }

    #[test]
    fn a_guest_keeps_its_hunters_over_a_stairwell() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let (mut g, _) = on_landing(-1, &content, &catalog);
        g.remote_hunters = true;
        g.hunters = vec![Brain::new(content.hunters[0].clone(), (5.5, 5.5), "GUEST-HUNTER", g.difficulty)];
        let before: Vec<(f32, f32)> = g.hunters.iter().map(|h| h.pos).collect();
        let stored = g.inf.as_ref().unwrap().stored.len();
        let at = g.p.pos;
        g.go_to_level(-1, at, 0.0, &content, &catalog);
        g.go_to_level(0, at, 0.0, &content, &catalog);
        let after: Vec<(f32, f32)> = g.hunters.iter().map(|h| h.pos).collect();
        assert_eq!(before, after, "the host's frames set a guest's hunters, a level change does not");
        assert_eq!(g.inf.as_ref().unwrap().stored.len(), stored, "a guest stores no hunter");
        g.reset_hunters(&content);
        assert!(g.hunters.is_empty(), "a guest builds no lineup of its own");
        g.ensure_hunters(&content);
        assert!(g.hunters.is_empty(), "nor tops one up");
    }

    #[test]
    fn an_axe_breach_starts_the_pipe_flow_and_the_wading_reads_its_depth() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-PIPE-7", 0, &content, &catalog);
        let chunk = (0, 0, 0);
        let id = g.burst_pipe(chunk, 3);
        assert_eq!(id, format!("{}3", super::super::pipes::flow_prefix(chunk)));
        assert!(g.pipes.flows_running(chunk));
        assert_eq!(g.pipe_depth(&id, 0, 0), 0.0, "the water has not risen yet");
        g.time += super::super::pipes::rules().flow.rise_seconds + 5.0;
        assert!(g.pipe_depth(&id, 0, 0) > 0.0, "the flow reaches the cell by the pipe");
        assert_eq!(g.pipe_depth("BRK:9:9:9:1", 0, 0), 0.0, "an unknown flow has no water");
    }

    #[test]
    fn drained_pipe_flows_are_pruned_about_once_a_second() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, "ROOM-PIPE-8", 0, &content, &catalog);
        let chunk = (0, 0, 0);
        let id = g.burst_pipe(chunk, 1);
        let shut = super::super::pipes::TurnEvent {
            valve_id: "SEWER:0:0:0:PROP:1".into(),
            chunk,
            effect: "flood".into(),
            shut: true,
            turns: 0,
            gate: String::new(),
            noise: None,
        };
        g.pipes.apply(&shut, g.time);
        g.pipes_tick();
        assert!(g.pipes.run(&id).is_some(), "a flow still draining stays");
        g.time += super::super::pipes::rules().flow.shut_drain_seconds + 2.0;
        g.pipes_tick();
        assert!(g.pipes.run(&id).is_none(), "the drained flow is dropped");
    }

    /// The hunter specialty bar (game/specialty.rs) across a change of floor, through the real label change (`relabel`,
    /// which a walked flight, a ladder and a fall all end in) and the admin jump (`change_storey`).
    mod specialty_across_floors {
        use super::*;
        use crate::game::resident_hunters::Slot as ShelfSlot;
        use crate::game::specialty::{Mode, SpecItem};

        fn brain_on(g: &Game, content: &Content, id: &str, level: i32) -> Brain {
            let def = content.hunter(id).unwrap_or_else(|| panic!("no hunter {id}")).clone();
            // far from the runner and dormant: nothing in these tests is caught or woken
            let far = g
                .fac
                .floor_cells()
                .into_iter()
                .map(|c| c.center())
                .max_by(|a, b| pdist(*a, g.p.pos).partial_cmp(&pdist(*b, g.p.pos)).unwrap());
            let mut b = Brain::new(def, far.unwrap_or(g.p.pos), &format!("FLOORS:{id}:{level}"), g.difficulty);
            b.level = level;
            b.y = floor_y_of(level);
            b
        }

        /// A runner on a flight with the neighbour floor resident: the active floor holds `here`, the neighbour `there`
        /// (a hunter on a floor that is not the runner's is on that floor's list). Returns the game and the neighbour.
        fn two_floors(here: &[&str], there: &[&str]) -> (Game, i32, Content, Catalog) {
            let (content, catalog) = (Content::load(), Catalog::load());
            let (mut g, _, to) = with_neighbour(-1, &content, &catalog);
            let from = g.inf.as_ref().unwrap().level;
            g.relays_collected = 0;
            g.hunters.clear();
            g.inf.as_mut().unwrap().loaded_mut().clear();
            // the generated world may shelve any of the pair on a floor of its own: the run holds only what the test says
            for id in ["glyra", "wynelle", "solyth"] {
                remove_from_run(&mut g, id);
            }
            g.sync_specialty();
            for id in here {
                let b = brain_on(&g, &content, id, from);
                g.hunters.push(b);
            }
            let other: Vec<ShelfSlot> =
                there.iter().map(|id| ShelfSlot { id: String::new(), brain: brain_on(&g, &content, id, to) }).collect();
            g.resident_hunters.put(to, other);
            g.reset_director();
            g.sync_specialty();
            (g, to, content, catalog)
        }

        fn ids(g: &Game) -> Vec<String> {
            g.hunters.iter().map(|h| h.def.id.clone()).collect()
        }

        /// Every hunter that exists in the run: the drawn floor's list, the other resident floors and the shelves. The
        /// specialty slots follow this (game/specialty.rs `run_hunter_ids`).
        fn run_ids(g: &Game) -> Vec<String> {
            let mut v = ids(g);
            v.extend(g.resident_hunters.iter_all().map(|b| b.def.id.clone()));
            v.extend(g.inf.as_ref().unwrap().shelved_hunters().map(|b| b.def.id.clone()));
            v
        }

        /// Takes a hunter out of the run altogether (every list and shelf): the only way a slot may go.
        fn remove_from_run(g: &mut Game, id: &str) {
            g.hunters.retain(|h| h.def.id != id);
            for l in g.resident_hunters.levels() {
                g.resident_hunters.list_mut(l).retain(|s| s.brain.def.id != id);
            }
            let inf = g.inf.as_mut().unwrap();
            inf.stored_mut().retain(|_, b| b.def.id != id);
            inf.floor_hunters_mut().values_mut().for_each(|v| v.retain(|b| b.def.id != id));
        }

        fn boxes_in_pack(g: &Game) -> usize {
            g.slots.iter().flatten().filter(|s| s.item == Item::MusicBox).count()
        }

        #[test]
        fn the_bar_keeps_what_the_run_still_has_on_the_frame_of_the_change() {
            // floor 0 has Glyra and Wynelle, the floor beyond only Wynelle; slot 6 is in hand. Glyra goes to the list of
            // the floor she stays on: she is still in the run, so nothing is taken from the bar
            let (mut g, to, content, catalog) = two_floors(&["glyra", "wynelle"], &["wynelle"]);
            assert_eq!(g.specialty.visible, [true, true, true, true]);
            g.press_specialty(0);
            assert_eq!(g.specialty.selected, Some(0));
            assert!(g.relabel(to, &content, &catalog));
            assert_eq!(ids(&g), vec!["wynelle"], "the floor beyond has the hunters it kept");
            assert_eq!(g.specialty.visible, [true, true, true, true], "Glyra lives on the floor beside: her slot stays");
            assert!(g.specialty.items[0].is_some(), "and so does her box");
            assert_eq!(g.specialty.selected, Some(0), "and the hand");
            assert_eq!(boxes_in_pack(&g), 0);
        }

        #[test]
        fn a_hunter_only_the_new_floor_brings_is_on_the_bar_on_the_frame_of_the_change() {
            // the floor beyond is loaded with Wynelle after the bar was last read (a floor load puts its lineup on the
            // shelf before the label moves): the bar a frame is drawn with must already hold her slot, not a frame later
            let (mut g, to, content, catalog) = two_floors(&["glyra"], &[]);
            assert_eq!(g.specialty.visible, [true, false, true, true]);
            let wynelle = brain_on(&g, &content, "wynelle", to);
            g.resident_hunters.put(to, vec![ShelfSlot { id: String::new(), brain: wynelle }]);
            assert!(!g.specialty.visible[1], "the bar has not been read since");
            assert!(g.relabel(to, &content, &catalog));
            assert_eq!(ids(&g), vec!["wynelle"]);
            assert_eq!(g.specialty.visible, [true, true, true, true], "slot 7 is there on the frame of the change");
            assert!(g.specialty.items[1].is_some(), "and its item is granted on that frame");
        }

        #[test]
        fn the_tune_is_gated_by_the_new_floor_on_its_first_frame() {
            let (mut g, to, content, catalog) = two_floors(&["glyra"], &["wynelle"]);
            g.music_box_left = 40.0;
            assert_eq!(g.music_box_heard(), 40.0, "Glyra is out: the wound box is heard");
            assert!(g.relabel(to, &content, &catalog));
            assert!(g.specialty.box_slot_visible(), "slot 6 is the run's: Glyra lives on the floor beside");
            assert_eq!(g.music_box_heard(), 0.0, "she is not on this floor: nothing is heard on the first frame");
            assert_eq!(g.music_box_left, 40.0, "the wind itself is untouched");
            // and back: heard again at the remaining time, with no restart of the clock
            g.music_box_left = 25.0;
            assert!(g.relabel(0, &content, &catalog));
            assert_eq!(g.music_box_heard(), 25.0, "Glyra is out again and the box plays on from where it was");
        }

        #[test]
        fn the_tune_stays_silent_for_every_frame_on_a_floor_without_glyra() {
            let (mut g, to, content, catalog) = two_floors(&["glyra"], &["wynelle"]);
            g.music_box_left = 40.0;
            assert!(g.relabel(to, &content, &catalog));
            for _ in 0..5 {
                g.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &Sounds::default());
                let on_glyras_floor = g.inf.as_ref().unwrap().level == 0;
                assert!(g.specialty.box_slot_visible(), "the slot is the run's");
                assert_eq!(g.glyra_on_floor(), on_glyras_floor);
                assert_eq!(g.music_box_heard() > 0.0, on_glyras_floor, "the tune is on her floor only");
            }
        }

        #[test]
        fn state_survives_a_change_when_the_hunter_is_on_both_floors() {
            let (mut g, to, content, catalog) = two_floors(&["glyra", "wynelle", "solyth"], &["glyra", "wynelle", "solyth"]);
            // wind the box (a hold; the box stays), set the temperature item to cold and hold slot 7
            g.press_specialty(0);
            g.use_specialty();
            assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "a click keeps the box");
            g.focus = None;
            g.p.stun = 0.0;
            // from empty: a box held on past 100 % jams (data/music_box.json overwind_seconds), so it starts run down
            g.music_box_left = 0.0;
            g.update_box_wind(crate::game::music_box::rules().wind_seconds, true);
            g.specialty.items[0] = None; // then the slot is cleared by hand so the check below is about floors only
            g.specialty.items[1] = Some(SpecItem::Temperature(Mode::Cold));
            g.press_specialty(1);
            let left = g.music_box_left;
            assert!(left > 0.0);
            assert!(g.relabel(to, &content, &catalog));
            assert_eq!(g.specialty.items[0], None, "a cleared slot is not handed out again by a change of floor");
            assert_eq!(g.specialty.temp_mode(), Some(Mode::Cold), "the type the runner set is kept");
            assert_eq!(g.specialty.selected, Some(1), "and so is the slot in hand");
            assert_eq!(g.music_box_left, left, "the wound clock is the runner's, not the floor's");
            assert_eq!(g.music_box_heard(), left);
            // the same across the whole way back and forth, with frames between
            for _ in 0..4 {
                for lvl in [0, to] {
                    g.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &Sounds::default());
                    if g.inf.as_ref().unwrap().level != lvl {
                        assert!(g.relabel(lvl, &content, &catalog));
                    }
                    assert_eq!(g.specialty.temp_mode(), Some(Mode::Cold));
                    assert_eq!(g.specialty.items[0], None);
                    assert_eq!(g.specialty.selected, Some(1));
                }
            }
        }

        #[test]
        fn a_slot_whose_hunter_is_on_another_floor_stays_and_does_not_double() {
            let (mut g, to, content, catalog) = two_floors(&["glyra", "wynelle"], &["solyth"]);
            assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
            let mode = g.specialty.temp_mode();
            assert!(mode.is_some(), "Wynelle here and Solyth beside: the data default type is granted");
            g.press_specialty(1);
            assert!(g.relabel(to, &content, &catalog));
            // Glyra and Wynelle are on floor 0's list, Solyth is here: the run holds all three, the bar is whole
            assert!(g.specialty.visible[0], "Glyra is in the run");
            assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "her box stays");
            assert_eq!(boxes_in_pack(&g), 0);
            assert!(g.specialty.visible[1]);
            assert_eq!(g.specialty.temp_mode(), mode, "no second grant: the type is not re-picked");
            assert_eq!(g.specialty.selected, Some(1));
            // a floor box is refused, not put in the pack: slot 6 holds one already
            assert!(!g.can_grant(Item::MusicBox));
            assert!(!g.grant(Item::MusicBox, 1));
            assert_eq!(boxes_in_pack(&g), 0);
            // Glyra leaves the run altogether: the slot and the box go, and the box is NOT returned to the pack
            remove_from_run(&mut g, "glyra");
            g.sync_specialty();
            assert!(!g.specialty.visible[0]);
            assert_eq!(g.specialty.items[0], None);
            assert_eq!(boxes_in_pack(&g), 0);
            assert_eq!(g.specialty.selected, Some(1), "the hand on slot 7 is kept");
            // a floor box taken now goes to the pack, since Glyra is nowhere
            assert!(g.give(Item::MusicBox, 1));
            assert_eq!(boxes_in_pack(&g), 1);
            // she is in the run again: the pack box folds into slot 6, so the runner never holds two
            let glyra = brain_on(&g, &content, "glyra", to);
            g.hunters.push(glyra);
            g.sync_specialty();
            assert!(g.specialty.visible[0]);
            assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
            assert_eq!(boxes_in_pack(&g), 0);
        }

        #[test]
        fn hunters_of_the_floor_beside_count_for_the_run() {
            // Glyra and Wynelle are on the neighbour's list only: the runner's floor has none of them, the run does
            let (mut g, _, _, _) = two_floors(&[], &["glyra", "wynelle"]);
            assert_eq!(g.specialty.visible, [true, true, true, true]);
            assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
            assert_eq!(g.specialty.temp_mode(), Some(Mode::Heat));
            assert!(!g.glyra_on_floor(), "she is not on this floor");
            g.music_box_left = 10.0;
            assert_eq!(g.music_box_heard(), 0.0, "so her tune is not heard here");
            // and a run with neither of them has only the reserved slots
            remove_from_run(&mut g, "glyra");
            remove_from_run(&mut g, "wynelle");
            g.sync_specialty();
            assert_eq!(g.specialty.visible, [false, false, true, true]);
            assert_eq!(g.specialty.items, [None; 4]);
        }

        #[test]
        fn the_admin_jump_also_leaves_the_bar_right_on_arrival() {
            let (mut g, _to, content, catalog) = two_floors(&["glyra", "wynelle"], &[]);
            g.press_specialty(0);
            g.music_box_left = 20.0;
            let at = g.p.pos;
            // floor -1 builds its own lineup (Nyxie, Solyth); Glyra and Wynelle stay shelved on floor 0, in the run
            g.go_to_level(-1, at, 0.0, &content, &catalog);
            let run = run_ids(&g);
            let want = |id: &str| run.iter().any(|h| h == id);
            assert!(want("glyra") && want("wynelle"), "the shelved pair is still in the run: {run:?}");
            assert_eq!(g.specialty.visible[0], want("glyra"));
            assert_eq!(g.specialty.visible[1], want("wynelle") || want("solyth"));
            assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "her box is not taken by a jump");
            assert_eq!(g.specialty.selected, Some(0), "nor the hand");
            assert_eq!(boxes_in_pack(&g), 0);
            let here = ids(&g);
            assert_eq!(g.music_box_heard() > 0.0, here.iter().any(|h| h == "glyra"), "the tune only where she is");
            // back up: the shelved Glyra and Wynelle are on floor 0 again, and the tune with them
            g.go_to_level(0, at, 0.0, &content, &catalog);
            assert!(g.specialty.visible[0]);
            assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
            assert_eq!(g.glyra_on_floor(), ids(&g).iter().any(|h| h == "glyra"));
            assert_eq!(g.music_box_heard() > 0.0, g.glyra_on_floor());
        }

        /// The bar's invariants on any floor: a slot shows exactly when one of its hunters exists in the run (the drawn
        /// floor, the floors beside, the shelves), a hidden item slot holds nothing and is not in hand, the runner never
        /// holds a second music box (slot plus pack), and the tune is heard only with Glyra on THIS floor.
        fn bar_is_consistent(g: &Game) {
            let here = run_ids(g);
            let has = |id: &str| here.iter().any(|h| h == id);
            assert_eq!(g.specialty.visible[0], has("glyra"), "slot 6 follows Glyra in the run {:?}", here);
            assert_eq!(g.specialty.visible[1], has("wynelle") || has("solyth"), "slot 7 follows Wynelle and Solyth in the run {:?}", here);
            for i in 0..2 {
                if !g.specialty.visible[i] {
                    assert_eq!(g.specialty.items[i], None, "a hidden slot {i} holds nothing");
                }
            }
            if let Some(s) = g.specialty.selected {
                assert!(g.specialty.visible[s], "the hand is never on a hidden slot");
            }
            let boxes = boxes_in_pack(g) + usize::from(g.specialty.items[0] == Some(SpecItem::MusicBox));
            assert!(boxes <= 1, "never two boxes");
            if !g.glyra_on_floor() {
                assert_eq!(g.music_box_heard(), 0.0, "nothing is heard without Glyra on the floor");
            } else if g.specialty.visible[0] {
                assert_eq!(g.music_box_heard(), g.music_box_left, "with her on the floor it is heard");
            }
        }

        #[test]
        fn rapid_back_and_forth_never_doubles_or_loses_what_stays() {
            // both floors have the whole set: nothing may ever change hands
            let (mut g, to, content, catalog) = two_floors(&["glyra", "wynelle", "solyth"], &["glyra", "wynelle", "solyth"]);
            g.press_specialty(1);
            g.specialty.items[1] = Some(SpecItem::Temperature(Mode::Cold));
            g.music_box_left = 90.0;
            for n in 0..40 {
                let want = if n % 2 == 0 { to } else { 0 };
                assert!(g.relabel(want, &content, &catalog), "hop {n}");
                bar_is_consistent(&g);
                assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "the unspent box stays, hop {n}");
                assert_eq!(g.specialty.temp_mode(), Some(Mode::Cold), "hop {n}");
                assert_eq!(g.specialty.selected, Some(1), "hop {n}");
                assert_eq!(g.music_box_left, 90.0, "hop {n}");
                assert_eq!(boxes_in_pack(&g), 0);
            }
        }

        #[test]
        fn rapid_back_and_forth_with_glyra_on_one_floor_never_mints_a_second_box() {
            let (mut g, to, content, catalog) = two_floors(&["glyra", "wynelle"], &["wynelle"]);
            g.music_box_left = 90.0;
            for n in 0..40 {
                // the step may itself relabel the body back (it stands on the flight of the floor it came from), so hop from
                // wherever the label is: the labels flip every frame, the way a runner dithering on a flight does
                let want = if g.inf.as_ref().unwrap().level == 0 { to } else { 0 };
                assert!(g.relabel(want, &content, &catalog), "hop {n}");
                bar_is_consistent(&g);
                // Glyra is in the run on either floor: the slot and the one box never move; only the tune follows her floor
                assert!(g.specialty.visible[0], "hop {n}");
                assert_eq!(g.music_box_heard() > 0.0, want == 0, "the tune is on her floor only, hop {n}");
                g.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &Sounds::default());
                bar_is_consistent(&g);
                let now = g.inf.as_ref().unwrap().level;
                assert!(g.specialty.visible[0], "hop {n}");
                assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "one box in the slot on either floor, hop {n}");
                assert_eq!(g.music_box_heard() > 0.0, now == 0, "the tune is on her floor only, hop {n}");
                assert_eq!(boxes_in_pack(&g), 0, "hop {n}");
            }
        }

        #[test]
        fn a_run_with_neither_pair_hunter_shows_only_the_reserved_slots() {
            let (mut g, to, content, catalog) = two_floors(&["glyra", "wynelle"], &["nyxie"]);
            g.press_specialty(0);
            g.music_box_left = 12.0;
            assert!(g.relabel(to, &content, &catalog));
            bar_is_consistent(&g);
            // the pair lives on floor 0's list: the bar stays whole on the floor with neither, silent
            assert_eq!(g.specialty.visible, [true, true, true, true]);
            assert_eq!(g.specialty.selected, Some(0));
            assert_eq!(g.music_box_heard(), 0.0);
            // only when the pair is nowhere in the run do the slots go
            remove_from_run(&mut g, "glyra");
            remove_from_run(&mut g, "wynelle");
            g.sync_specialty();
            bar_is_consistent(&g);
            assert_eq!(g.specialty.visible, [false, false, true, true]);
            assert_eq!(g.specialty.items, [None; 4]);
            assert_eq!(g.specialty.selected, None);
            assert_eq!(g.music_box_heard(), 0.0);
            assert_eq!(g.music_box_left, 12.0, "the wind is the runner's and keeps");
            // a key on a hidden or reserved slot does nothing
            for i in 0..2 {
                g.press_specialty(i);
                assert_eq!(g.specialty.selected, None, "slot {i}");
            }
            // frames on the floor change nothing either
            for _ in 0..5 {
                g.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &Sounds::default());
                bar_is_consistent(&g);
                assert_eq!(g.specialty.items, [None; 4]);
            }
            // and a run with Glyra again grants the box once
            let glyra = brain_on(&g, &content, "glyra", g.inf.as_ref().unwrap().level);
            g.hunters.push(glyra);
            g.sync_specialty();
            g.sync_specialty();
            bar_is_consistent(&g);
            assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox));
            assert_eq!(boxes_in_pack(&g), 0);
        }

        #[test]
        fn a_ladder_down_then_a_manhole_chain_keeps_the_bar_consistent_each_hop() {
            use crate::world::route::sewer::core::{LEVEL, LOWER_LEVEL};
            let (mut g, _to, content, catalog) = two_floors(&["glyra", "wynelle"], &[]);
            g.press_specialty(0);
            let at = g.p.pos;
            for lvl in [LEVEL, LOWER_LEVEL, LEVEL, 0, LOWER_LEVEL, 0] {
                g.go_to_level(lvl, at, 0.0, &content, &catalog);
                bar_is_consistent(&g);
                // the pair stays in the run (shelved or on the list) all the way: the hop takes nothing from the runner
                assert_eq!(g.specialty.items[0], Some(SpecItem::MusicBox), "floor {lvl}");
                assert_eq!(g.specialty.selected, Some(0), "floor {lvl}");
                assert_eq!(boxes_in_pack(&g), 0, "floor {lvl}");
                // a frame on the new floor must not move anything the arrival frame decided
                let (vis, items, sel) = (g.specialty.visible, g.specialty.items, g.specialty.selected);
                g.step(1.0 / 60.0, &Controls::default(), &content, &catalog, &Sounds::default());
                bar_is_consistent(&g);
                assert_eq!(g.specialty.visible, vis, "floor {lvl}: the first step keeps the arrival bar");
                assert_eq!(g.specialty.items, items, "floor {lvl}");
                assert_eq!(g.specialty.selected, sel, "floor {lvl}");
            }
        }
    }

}

#[cfg(test)]
mod dry_start_tests {
    use super::*;
    use crate::config::Difficulty;

    /// A run entered on the sewer level stands on a dry cell the walking grid keeps open, not in the fenced channel
    /// at the window's centre (the headless bot's level -1 explore run, seed BRAVO, started there and never got out).
    #[test]
    fn a_sewer_level_entry_stands_the_runner_on_a_dry_open_cell() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let g = Game::new_infinite_sim(Difficulty::Standard, 7, "BRAVO", -1, &content, &catalog);
        let c = Cell::new(g.p.pos.0.floor() as i32, g.p.pos.1.floor() as i32);
        assert!(!g.fac.is_water(c), "the runner starts out of the water");
        assert!(g.fac.hunter_open(c), "the runner starts on the walking grid");
        assert!(pdist(g.p.pos, g.fac.start.center()) <= 12.0 * std::f32::consts::SQRT_2, "near the window's start");
    }

    /// The dry start is never a pocket the railings cut off from every flight: a dry route (no water, no railing, no
    /// well) leads from it to a stair's cell wherever the level has a flight.
    #[test]
    fn the_dry_start_is_joined_to_a_flight_by_a_dry_route() {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut checked = 0;
        for k in 0..6 {
            let g = Game::new_infinite_sim(Difficulty::Standard, 7, &format!("ROOM-SPIRAL-{k}"), -1, &content, &catalog);
            // what dry_start itself picks for this window
            let at = dry_start(&g.fac);
            let start = Cell::new(at.0.floor() as i32, at.1.floor() as i32);
            assert_eq!(at, start.center(), "seed {k}: the start is a cell's centre");
            assert!(!g.fac.is_water(start) && g.fac.hunter_open(start), "seed {k}: a dry cell on the walking grid");
            assert!(g.fac.body_fits(at.0, at.1, PLAYER_RADIUS), "seed {k}: the runner's body fits there");
            let off = (start.x - g.fac.start.x).abs().max((start.y - g.fac.start.y).abs());
            assert!(off <= 12, "seed {k}: within the search ring of the window's start ({off} cells)");
            let anchors: Vec<Cell> = g.fac.stairs.iter().map(|s| s.cell).filter(|c| g.fac.hunter_open(*c)).collect();
            if anchors.is_empty() {
                continue;
            }
            checked += 1;
            let mut seen: HashSet<Cell> = HashSet::from([start]);
            let mut q = vec![start];
            while let Some(at) = q.pop() {
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let n = Cell::new(at.x + dx, at.y + dy);
                    if !seen.contains(&n) && g.fac.hunter_open(n) && !g.fac.is_water(n) && !g.fac.rail_between(at, n) {
                        seen.insert(n);
                        q.push(n);
                    }
                }
            }
            assert!(anchors.iter().any(|a| seen.contains(a)), "seed {k}: the dry start {start:?} is cut off from every flight");
        }
        assert!(checked > 0, "some window had a flight to reach");
    }
}
