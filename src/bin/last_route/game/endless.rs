//! The Infinite route inside a run: the live 3x3 chunk window that recentres as the runner
//! walks, stairwells between levels, chunk state that survives unloading (relays, batteries,
//! coins and pickups taken, cells discovered), and the hunter population that grows with
//! recovered relays (GameConfig.streamed_hunter_population).

use super::{pdist, FloorPickup, Game, Item};
use crate::config::*;
use crate::content::Content;
use crate::hunters::brain::{Brain, State};
use crate::hunters::gimmicks::Gimmick;
use crate::hunters::residency::{self, Cand, Record};
use crate::shared::safe_room::{SafeRoom, SafeRooms, Status};
use crate::world::catalog::Catalog;
use crate::world::facility::{stair_well_cells, Facility, Pickup, Stair, STAIR_FLIGHT};
use crate::world::fixtures::Fixtures;
use crate::world::infinite::{self, ChunkKey, CHUNK};
use crate::world::route::hunter_spawns::HunterSpawn;
use crate::world::route::live;
use crate::world::rng::Rng;
use crate::world::Cell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

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
            for step in 1..=STAIR_FLIGHT {
                for c in stair_well_cells(*s, step) {
                    if (0..CHUNK).contains(&c.x) && (0..CHUNK).contains(&c.y) {
                        holes[(c.y * CHUNK + c.x) as usize] = true;
                    }
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

/// The level-0 full map's feed (ui/map.rs): every discovered chunk's record in key order, and the frame they are drawn in.
pub struct FullMapFeed<'a> {
    /// Every discovered level-0 chunk, by key, in (cx, cy) order.
    pub chunks: Vec<(ChunkKey, &'a MapChunk)>,
    /// The window's origin: the absolute cell of its (0, 0). Window-local positions (the runner, the hunters) add it.
    pub origin: (i32, i32),
    /// The waypoint route (`WaypointRoutes::full`) in absolute cell centres.
    pub route: &'a [(f32, f32)],
    /// The waypoint's pin, absolute, when it is on level 0.
    pub waypoint: Option<(f32, f32)>,
}

pub struct InfState {
    /// The world seed of the route: str(run_seed) of the room seed text (route::world_seed).
    pub seed: String,
    pub level: i32,
    /// Chunk at the middle of the window.
    pub center: (i32, i32),
    cache: HashMap<ChunkKey, (Facility, Fixtures)>,
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
/// tools, the battery and the stim pack; a lockpick, revive kit or backpack has no `Item` yet, so it
/// is not on the floor (see docs/PARITY.md, "Live level 0").
fn floor_item(k: &Pickup) -> Option<(Item, i32)> {
    Some(match k {
        Pickup::Taser => (Item::Taser, 1),
        Pickup::Crowbar => (Item::Crowbar, CROWBAR_DURABILITY),
        Pickup::Axe => (Item::Axe, AXE_DURABILITY),
        Pickup::Battery => (Item::Battery, 1),
        Pickup::Stim => (Item::Stim, 1),
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
    let activation = diff.profile().activation_relays;
    if relays < activation {
        return 0;
    }
    let step = match diff {
        Difficulty::Relaxed => 3,
        Difficulty::Standard => 2,
        Difficulty::Hard => 1,
    };
    1 + (relays - activation) / step
}

impl InfState {
    pub fn new(seed: &str, level: i32) -> InfState {
        InfState {
            seed: seed.to_string(),
            level,
            center: (0, 0),
            cache: HashMap::new(),
            claimed: HashSet::new(),
            ids: live::WindowIds::default(),
            layout_pickups: HashMap::new(),
            discovered_chunks: HashSet::new(),
            discovered_rooms: HashSet::new(),
            markers: HashSet::new(),
            map_chunks: HashMap::new(),
            seen: HashSet::new(),
            chunks_visited: HashSet::new(),
            safe: SafeRooms::default(),
            room: None,
            live: false,
            last_time: -1.0,
            checkpoints_activated: 0,
            stored: HashMap::new(),
            loaded: Vec::new(),
            floor_hunters: HashMap::new(),
            spawns: HashMap::new(),
            walls: HashMap::new(),
            floor: DiscoveredFloor::default(),
            floor_chunks: 0,
            waypoint: None,
            routes: WaypointRoutes::default(),
            route_key: None,
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
        // level 0 keeps its discovery as chunks (discovered_chunks); the cell list is for the other levels
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
        for (y, row) in self.discovered.iter_mut().enumerate() {
            for (x, d) in row.iter_mut().enumerate() {
                let (ax, ay) = (x as i32 + ox, y as i32 + oy);
                *d = if l == 0 {
                    // a level-0 cell is discovered when its chunk is (Godot `_remember_discovered_chunk`)
                    let key = ChunkKey {
                        cx: ax.div_euclid(CHUNK),
                        cy: ay.div_euclid(CHUNK),
                        level: 0,
                    };
                    inf.discovered_chunks.contains(&key)
                } else {
                    inf.seen.contains(&(l, ax, ay))
                };
            }
        }
    }

    /// Builds the window around the current centre chunk and level.
    fn load_window(&mut self, content: &Content, catalog: &Catalog) {
        let Some(inf) = self.inf.as_mut() else { return };
        let (fac, fix) = infinite::window(
            &inf.seed,
            inf.center,
            inf.level,
            catalog,
            &content.loot,
            &mut inf.cache,
        );
        // claim ids for the new window: the layout's on level 0, else by absolute cell
        let (level, origin) = (inf.level, inf.origin());
        let lens = [fac.relays.len(), fac.batteries.len(), fac.pickups.len(), fix.coins.len()];
        let live_ids = live::window_ids(&mut inf.layout_pickups, &inf.seed, inf.center, level, lens);
        inf.live = live_ids.is_some();
        inf.ids = live_ids.unwrap_or_else(|| cell_ids(level, origin, &fac, &fix));
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
        self.fac = fac;
        self.fix = fix;
        self.adopt_floor(content);
        self.sync_in();
        self.sync_safe_zone();
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
        let start = self.fac.start.center();
        self.p.place(start);
        self.p.pos = start;
        self.reset_hunters(content);
        self.fade = 1.0;
        self.reveal();
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
        self.recentre_window(content, catalog);
        self.discover_map();
        self.clear_reached_waypoint();
        self.refresh_waypoint();
        let dt = self.safe_room_dt();
        let before = self.active_checkpoint().to_string();
        self.safe_room_step(dt);
        // a checkpoint newly made active saves the run (Godot `_accept_checkpoint`: autosave("checkpoint")); standing on in
        // the room does not save it again every frame (`_update_checkpoint` skips the `_last_checkpoint`)
        if !self.active_checkpoint().is_empty() && self.active_checkpoint() != before {
            self.autosave("checkpoint");
        }
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
            for h in self.hunters.iter_mut() {
                if h.active && pdist(h.pos, centre) <= SAFE_ROOM_LURE_REACH {
                    h.state = State::Investigate;
                    h.last_known = Some(centre);
                }
            }
        }
        let breachers = if inside {
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
        if status.breached && !was {
            self.say("SAFE ROOM BREACHED");
        }
        status
    }

    /// Godot `_admin_edit_checkpoint` (infinite_route_test.gd): DAMAGE CHECKPOINT takes 25 off the runner's safe room,
    /// REPAIR CHECKPOINT puts 25 back. `None` when the runner's chunk has no safe room (or off the level-0 layout).
    pub fn admin_edit_checkpoint(&mut self, amount: i32) -> Option<Status> {
        let room = self.live_room()?;
        if amount >= 0 {
            let status = self.inf.as_mut()?.safe.repair(&room, amount);
            self.sync_safe_zone();
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
        self.say(&format!("SAFE ROOM REPAIRED // {}%", status.integrity));
        true
    }

    /// The checkpoint the runner last activated on this route ("" before any). Godot respawns the runner there
    /// (`_checkpoint_world_position`); nothing reads it yet, since a death ends a run in this build.
    pub fn active_checkpoint(&self) -> &str {
        self.inf.as_ref().map_or("", |i| i.safe.active_checkpoint())
    }

    fn recentre_window(&mut self, content: &Content, catalog: &Catalog) {
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
        for h in self.hunters.iter_mut() {
            h.pos = (h.pos.0 - sx, h.pos.1 - sy);
            h.last_known = h.last_known.map(|k| (k.0 - sx, k.1 - sy));
            h.wander_target = h.wander_target.map(|k| (k.0 - sx, k.1 - sy));
            h.search_target = h.search_target.map(|k| (k.0 - sx, k.1 - sy));
            h.clear_path();
        }
        // NPC teammates share the window frame, so they shift with it (game/npc.rs)
        for n in self.npcs.iter_mut() {
            n.shift((-sx, -sy));
        }
        self.load_window(content, catalog);
        if self.inf.as_ref().is_some_and(|i| i.level == 0) {
            // level 0: residency decides who is in the window; a hunter that leaves it is stored, not respawned
            self.residency(content);
            return;
        }
        self.respawn_outside_window();
    }

    /// Off level 0: the hunters that stand outside the window (or in a wall) come back in out of sight.
    fn respawn_outside_window(&mut self) {
        let size = (CHUNK * 3) as f32;
        let mut rng = Rng::new(&format!(
            "{}:RESPAWN:{}",
            self.level_seed(),
            self.time as i32
        ));
        for i in 0..self.hunters.len() {
            let p = self.hunters[i].pos;
            if p.0 < 0.0
                || p.1 < 0.0
                || p.0 >= size
                || p.1 >= size
                || !self.fac.body_fits(p.0, p.1, HUNTER_RADIUS)
            {
                if let Some(spot) = self.spawn_spot(&mut rng) {
                    self.hunters[i].pos = spot;
                    self.hunters[i].clear_path();
                }
            }
        }
    }

    /// Takes a stairwell to the level above or below.
    pub(super) fn take_stairs(&mut self, i: usize, content: &Content, catalog: &Catalog) {
        let s = self.fac.stairs[i];
        // the level stack ends at 0 and at the Inferno (-3): a flight past either end is not taken
        let to = self.inf.as_ref().map_or(0, |inf| inf.level) + s.dir;
        if !infinite::level_allowed(to) {
            return;
        }
        let other = Cell::new(
            s.cell.x + s.run.0 * (STAIR_FLIGHT + 1),
            s.cell.y + s.run.1 * (STAIR_FLIGHT + 1),
        );
        self.sync_out();
        // the level being left keeps its hunters at their absolute cells, to be found again on the way back
        self.store_level0_hunters();
        self.store_floor_hunters();
        if let Some(inf) = self.inf.as_mut() {
            inf.level += s.dir;
        }
        self.load_window(content, catalog);
        let landing = self.fac.stairs.iter().find(|t| t.cell == other).copied();
        match landing {
            Some(t) => {
                let (cx, cy) = t.cell.center();
                self.p.pos = self.fac.nearest_open_or(
                    cx - t.run.0 as f32 * 0.6,
                    cy - t.run.1 as f32 * 0.6,
                    PLAYER_RADIUS,
                );
                self.p.yaw = (t.run.0 as f32).atan2(t.run.1 as f32);
            }
            None => self.p.pos = self.fac.start.center(),
        }
        // the level's own hunters come back where they were; a level not visited before gets its lineup
        self.restore_level_hunters(content);
        let level = self.inf.as_ref().map(|i| i.level).unwrap_or(0);
        self.announce(&format!("LEVEL {level}"));
        self.fade = 1.0;
        // the save is the caller's: interact's `autosave("level")` on a flight taken (no double write)
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
                    && self.fac.hunter_open(*c)
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
                self.hunters.push(b);
            }
        }
        self.ensure_hunters(content);
    }

    /// Adds route hunters until the population matches the relays recovered so far. On level 0 this is the
    /// residency pass (admission, then loading and storing); the old builder's levels keep the spawn-spot lineup.
    pub(super) fn ensure_hunters(&mut self, content: &Content) {
        if self.inf.as_ref().is_some_and(|i| i.level == 0) {
            self.residency(content);
            return;
        }
        let want = population(self.difficulty, self.relays_collected);
        let have = self
            .hunters
            .iter()
            .filter(|h| !matches!(h.def.id.as_str(), "nyxie" | "solyth"))
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
            let at = self.p.pos;
            b.activate_at(at);
            self.hunters.push(b);
        }
    }

    /// Level 0: the window's hunter residency (Godot `_refresh_hunter_stream_work`, see residency.rs). The hunters that exist
    /// are the stored ones, and the loaded ones at their absolute cells. Admission takes the layout spawns of the active
    /// chunks up to the population. A wanted hunter that stands in the runner's 3x3 window is loaded and the rest stay
    /// stored. A new hunter is created dormant at its spawn cell (Godot `ensure_hunter`), and wakes at the runner on its
    /// first load, where the relay activation used to wake a spawn. A change in who is loaded resets the director, whose
    /// ids are indices into `hunters`.
    fn residency(&mut self, content: &Content) {
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
        let runner_pos = self.p.pos;
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
    }

    /// Leaving level 0 by a stairwell: the loaded hunters go to the store at their absolute cells, so the level
    /// they were on keeps them (Godot keeps every hunter in world_state).
    fn store_level0_hunters(&mut self) {
        let Some(inf) = self.inf.as_mut() else { return };
        if inf.level != 0 {
            return;
        }
        inf.loaded.resize(self.hunters.len(), String::new());
        let o = inf.origin();
        let o = (o.0 as f32, o.1 as f32);
        let old_hunters = std::mem::take(&mut self.hunters);
        let old_ids = std::mem::take(&mut inf.loaded);
        for (mut b, id) in old_hunters.into_iter().zip(old_ids) {
            if !id.is_empty() {
                shift_brain(&mut b, o);
                inf.stored.insert(id, b);
            }
        }
    }

    /// Leaving a level off level 0 by a stairwell: its hunters are kept at their absolute cells under the level (the
    /// level-0 hunters go to `stored`, see `store_level0_hunters`).
    fn store_floor_hunters(&mut self) {
        let Some(inf) = self.inf.as_mut() else { return };
        if inf.level == 0 {
            return;
        }
        let o = inf.origin();
        let o = (o.0 as f32, o.1 as f32);
        let mut kept = std::mem::take(&mut self.hunters);
        for b in kept.iter_mut() {
            shift_brain(b, o);
        }
        inf.floor_hunters.insert(inf.level, kept);
    }

    /// Arriving on a level by a stairwell. Level 0 loads its stored hunters by residency. Another level the runner has
    /// been on gets back the hunters it kept, in the new window's frame (one now outside the window comes back in out of
    /// sight, as on a window move), topped up to the population. A level not visited before gets its lineup.
    fn restore_level_hunters(&mut self, content: &Content) {
        let Some(inf) = self.inf.as_mut() else { return };
        if inf.level == 0 {
            // the loaded set was emptied into `stored` on the way out; residency loads the window's share again
            self.hunters.clear();
            inf.loaded.clear();
            self.reset_director();
            self.residency(content);
            return;
        }
        let Some(mut kept) = inf.floor_hunters.remove(&inf.level) else {
            self.reset_hunters(content);
            return;
        };
        let o = inf.origin();
        let o = (-(o.0 as f32), -(o.1 as f32));
        for b in kept.iter_mut() {
            shift_brain(b, o);
        }
        self.hunters = kept;
        self.reset_director();
        self.respawn_outside_window();
        self.ensure_hunters(content);
    }

    pub fn save_checkpoint(&self) {
        crate::shared::checkpoint::save(&crate::shared::checkpoint::Checkpoint {
            run_seed: self.run_seed,
            room_text: self.room_text.clone(),
            depth: self.inf.as_ref().map(|i| i.level).unwrap_or(self.depth),
            score: self.score,
            coins: self.coins,
            difficulty: self.difficulty,
            infinite: self.inf.is_some(),
        });
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

    /// The level-0 full map's feed: every discovered chunk, in key order. `None` off level 0.
    pub fn full_map_feed(&self) -> Option<FullMapFeed<'_>> {
        let inf = self.inf.as_ref()?;
        if inf.level != 0 || inf.map_chunks.is_empty() {
            return None;
        }
        let mut chunks: Vec<(ChunkKey, &MapChunk)> =
            inf.map_chunks.iter().map(|(k, c)| (*k, c)).collect();
        chunks.sort_by_key(|(k, _)| (k.cx, k.cy));
        Some(FullMapFeed {
            chunks,
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
fn shift_brain(b: &mut Brain, d: (f32, f32)) {
    let s = move |p: (f32, f32)| (p.0 + d.0, p.1 + d.1);
    b.pos = s(b.pos);
    b.last_known = b.last_known.map(s);
    b.wander_target = b.wander_target.map(s);
    b.search_target = b.search_target.map(s);
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

    /// Godot `_crosshair_waypoint_target` (infinite_route_test.gd:1981): where a view ray from `eye` along `dir` lands on the
    /// floor, marched over the live window's floor in steps of a quarter cell out to `WAYPOINT_PLACE_RANGE`. `floor_y` is the
    /// floor's height. Returns a window position (XZ), or `None` when nothing in range is open.
    pub fn crosshair_waypoint(
        &self,
        eye: (f32, f32, f32),
        dir: (f32, f32, f32),
        floor_y: f32,
    ) -> Option<(f32, f32)> {
        let mut last_open = None;
        let mut d = 0.0f32;
        while d <= WAYPOINT_PLACE_RANGE {
            let p = (eye.0 + dir.0 * d, eye.1 + dir.1 * d, eye.2 + dir.2 * d);
            let open = self
                .fac
                .hunter_open(Cell::new(p.0.floor() as i32, p.2.floor() as i32));
            if p.1 <= floor_y + 0.05 {
                if open {
                    return Some((p.0, p.2));
                }
                break;
            }
            if !open {
                break;
            }
            last_open = Some((p.0, p.2));
            d += 0.25;
        }
        last_open
    }
}

#[cfg(test)]
mod tests {
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
    fn level_zero_chunks_are_on_the_map_as_they_load() {
        let (mut g, content, catalog) = run();
        // Godot _discover_loaded_chunks: the nine loaded chunks are discovered at once
        assert_eq!(g.inf.as_ref().unwrap().discovered_chunks.len(), 9);
        assert!(g.discovered.iter().all(|r| r.iter().all(|d| *d)));
        // a window move loads a column of new chunks; the chunks it leaves stay discovered
        g.p.pos.0 += CHUNK as f32;
        g.stream_window(&content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().discovered_chunks.len(), 12);
        assert!(g.discovered.iter().all(|r| r.iter().all(|d| *d)));
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
        assert_eq!(g.full_map_feed().expect("level 0 has a feed").chunks.len(), 9);
        // one chunk west: the window recentres on (-1, 0) and the chunks at cx -2 come in
        g.p.pos.0 -= CHUNK as f32;
        g.stream_window(&content, &catalog);
        let origin = g.inf.as_ref().unwrap().origin();
        let feed = g.full_map_feed().expect("level 0 has a feed");
        assert_eq!(feed.chunks.len(), 12);
        assert!(feed.chunks.iter().any(|(k, _)| k.cx == -2), "the chunk west of the old window is discovered");
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
        g.end_run(0, "caught");
        assert!(g.report.is_some(), "the catch ends the run");
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

    #[test]
    fn leaving_level_zero_stores_the_loaded_hunters_at_their_cells() {
        let (mut g, _, _) = populated();
        let before = absolute(&g);
        assert!(!g.hunters.is_empty());
        g.store_level0_hunters();
        assert!(g.hunters.is_empty());
        let after = absolute(&g);
        assert_eq!(after.len(), before.len());
        for (id, pos) in &before {
            assert_eq!(after.get(id), Some(pos), "{id} is stored where it stood");
        }
    }

    /// The stairwell of the window going `dir` (-1 down, +1 up), if it has one.
    fn stair(g: &Game, dir: i32) -> Option<usize> {
        g.fac.stairs.iter().position(|s| s.dir == dir)
    }

    /// The stairwell back: the landing the last `take_stairs` arrived on.
    fn stair_back(g: &Game, dir: i32) -> usize {
        let at = g.p.pos;
        let mut best = None;
        for (i, s) in g.fac.stairs.iter().enumerate() {
            if s.dir != dir {
                continue;
            }
            let d = pdist(s.cell.center(), at);
            if best.is_none_or(|(_, b)| d < b) {
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
        g.take_stairs(down, &content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, dir);
        let back = stair_back(&g, -dir);
        g.take_stairs(back, &content, &catalog);
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
        g.take_stairs(i, &content, &catalog);
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
        g.take_stairs(back, &content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, 0);
        let again = stair_back(&g, dir);
        g.take_stairs(again, &content, &catalog);
        assert_eq!(g.inf.as_ref().unwrap().level, dir);
        assert_eq!(g.inf.as_ref().unwrap().origin(), o, "the same window");
        assert_eq!(mark(&g), before, "the floor's hunters are the ones it kept");
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
    fn a_crosshair_at_the_floor_lands_on_the_cell_below() {
        let (g, _, _) = run();
        let s = (CHUNK + live::CENTRE) as f32 + 0.5;
        let hit = g.crosshair_waypoint((s, 1.0, s), (0.0, -1.0, 0.0), 0.0);
        assert_eq!(hit, Some((s, s)));
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
