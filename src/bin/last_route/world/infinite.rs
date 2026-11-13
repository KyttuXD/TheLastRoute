//! The Infinite route: an endless grid of 45x45 chunks per level (each built by the finite
//! generator, the same footprint as a Standard floor), streamed as a live 3x3 window, with
//! levels going down from level 0 linked by stairwells.
//!
//! From game/world/generation/streamed_route_generator.gd: CHUNK_SIZE 45, REGION_SIZE_CHUNKS 4,
//! the region rules (world::route::regions, SPAWN_MAZE_SHARE 0.6), EDGE_OFFSETS (where chunk
//! seams may open), CORRIDOR_WIDTH 3,
//! STAIR_BAY_ANCHORS and STAIR_BAYS_BY_BOUNDARY_PARITY (a stair link belongs to the boundary
//! between two levels, so both levels agree on it, and up/down links never share a bay), and
//! MEZZANINE_ENABLED false (level 0 is the top). District-specific room banks and shared
//! hall topology are applied by the facility builder, with sewers and the Inferno district
//! selected by level/region while the finite catalogue supplies authored room content.

use super::catalog::Catalog;
use super::facility::{Cell, Facility, Options, Pickup};
use super::flight::Flight;
use super::flight_links::{bay_flight, bay_rect, carve_cells, lower_stair, sewer_well_flights, upper_stair};
use super::fixtures::{self, Fixtures};
use super::rng::Rng;
use super::route::regions;
use crate::content::loot::LootTables;
use std::collections::HashMap;

pub const CHUNK: i32 = 45;
pub const WINDOW: i32 = 3;
pub const REGION_SIZE_CHUNKS: i32 = regions::REGION_SIZE_CHUNKS;
pub const EDGE_OFFSETS: [i32; 7] = [5, 11, 17, 22, 27, 33, 39];
const STAIR_BAY_ANCHORS: [[(i32, i32); 4]; 4] = [
    [(8, 8), (12, 8), (8, 12), (12, 12)],
    [(32, 8), (36, 8), (32, 12), (36, 12)],
    [(8, 32), (12, 32), (8, 36), (12, 36)],
    [(32, 32), (36, 32), (32, 36), (36, 36)],
];
const STAIR_BAYS_BY_PARITY: [[usize; 2]; 2] = [[0, 3], [1, 2]];
/// Share of boundaries (away from the origin column) that carry a stair link (estimated).
const STAIR_CHANCE: f64 = 0.3;
/// Godot `INFERNO_LEVEL` (streamed_route_generator.gd:34): the depth floor. Nothing is built or linked below it.
pub const INFERNO_LEVEL: i32 = -3;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ChunkKey {
    pub cx: i32,
    pub cy: i32,
    pub level: i32,
}

/// The district a chunk belongs to (world::route::regions::region_for, Godot's region_for:
/// `seed` is the world seed, str(run_seed) (route::world_seed), the chunk is (x, z) = (cx, cy)
/// on level k.level).
pub fn region_for(seed: &str, k: ChunkKey) -> &'static str {
    regions::region_for(seed, k.cx, k.cy, k.level)
}

/// The catalogue room region each district prefers to dress its rooms in.
pub fn hall_set(region: &str) -> &'static str {
    match region {
        "maze" => "maze",
        "sewer" => "maze",
        "mansion" => "mansion",
        "hotel" => "hotel",
        "apartment" => "apartment",
        "beach" => "beach",
        "gothic" => "gothic",
        "prison" => "prison",
        "inferno" => "inferno",
        "carnival" => "carnival",
        "hospital" => "hospital",
        "lab" => "lab",
        "greenhouse" => "greenhouse",
        "mall" => "mall",
        _ => "maze",
    }
}

fn boundary_offsets(seed: &str, key: &str) -> Vec<i32> {
    let mut rng = Rng::new(&format!("{seed}:SEAM:{key}"));
    let mut pool = EDGE_OFFSETS.to_vec();
    let count = 1 + (rng.next_float() < 0.5) as usize;
    let mut out = vec![];
    for _ in 0..count {
        out.push(pool.remove(rng.next_index(pool.len())));
    }
    out
}

/// Godot `is_chunk_allowed` (streamed_route_generator.gd:1437-1442) for a level, with MEZZANINE_ENABLED false: the
/// route streams level 0 down to the Inferno (-3) and nothing above or below.
pub fn level_allowed(level: i32) -> bool {
    crate::world::levels::allows(level)
}

/// A stair link on the boundary between `upper` and `upper - 1` in column (cx, cy):
/// Some((anchor, run)) when it exists. Shared by both levels.
pub(crate) fn stair_link(seed: &str, cx: i32, cy: i32, upper: i32) -> Option<((i32, i32), (i32, i32))> {
    // both ends must be levels the route has (Godot `stairwells_at`, 676-685): no flight above 0 or below -3
    if !level_allowed(upper) || !level_allowed(upper - 1) {
        return None;
    }
    let mut rng = Rng::new(&format!("{seed}:STAIRS:{upper}:{cx},{cy}"));
    let roll = rng.next_float();
    if !(cx == 0 && cy == 0) && roll >= STAIR_CHANCE {
        return None;
    }
    let bays = STAIR_BAYS_BY_PARITY[upper.rem_euclid(2) as usize];
    let bay = bays[rng.next_index(2)];
    let anchor = STAIR_BAY_ANCHORS[bay][rng.next_index(4)];
    Some((anchor, bay_run(bay)))
}

/// The unit step a flight runs along in a bay: east in the west bays (0 and 2), west in the east bays.
fn bay_run(bay: usize) -> (i32, i32) {
    if bay == 0 || bay == 2 {
        (1, 0)
    } else {
        (-1, 0)
    }
}

/// Every place the stair link between `upper` and `upper - 1` in column (cx, cy) could go, best first: the link's own
/// anchor, the other three anchors of its bay, then the four anchors of the other bay of the same parity (up and down
/// links never share a bay). Empty when there is no link. A pure function of its arguments, shared by both levels.
pub(crate) fn bay_candidates(seed: &str, cx: i32, cy: i32, upper: i32) -> Vec<((i32, i32), (i32, i32))> {
    let Some((anchor, _)) = stair_link(seed, cx, cy, upper) else {
        return vec![];
    };
    let bays = STAIR_BAYS_BY_PARITY[upper.rem_euclid(2) as usize];
    let first = bays.iter().position(|b| STAIR_BAY_ANCHORS[*b].contains(&anchor)).unwrap_or(0);
    let mut out = vec![];
    for (i, bay) in [bays[first], bays[1 - first]].into_iter().enumerate() {
        let run = bay_run(bay);
        if i == 0 {
            out.push((anchor, run));
        }
        for a in STAIR_BAY_ANCHORS[bay] {
            if !(i == 0 && a == anchor) {
                out.push((a, run));
            }
        }
    }
    out
}

/// `KYTTU_LEGACY_CHUNKS=1` keeps the old level-0 builder (`legacy_chunk`) for comparison. Unset,
/// level 0 is the Godot layout adapter (`route::live`). Levels -1 to -3 always use `legacy_chunk`.
pub fn legacy_level0() -> bool {
    static LEGACY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LEGACY.get_or_init(|| std::env::var("KYTTU_LEGACY_CHUNKS").map(|v| v == "1").unwrap_or(false))
}

/// Builds one chunk: level 0 from the Godot layout (`route::live::chunk`), unless
/// `KYTTU_LEGACY_CHUNKS=1`; every other level from `legacy_chunk`.
pub fn chunk(seed: &str, k: ChunkKey, cat: &Catalog, loot: &LootTables) -> (Facility, Fixtures) {
    if k.level == 0 && !legacy_level0() {
        if let Some(pair) = super::route::live::chunk(seed, k, cat) {
            return pair;
        }
    }
    legacy_chunk(seed, k, cat, loot)
}

/// A legacy chunk's backpack draw from the table's `backpacks` list (`LootTables.pick_backpack`): the rank is
/// `1 + floor(max(0, -level) / BACKPACK_DEPTH_STEP)`, capped at the list length, then one `next_index` over that
/// rank. An empty list draws nothing and places nothing; an unknown name draws but places nothing.
fn backpack_pickup(backpacks: &[String], level: i32, rng: &mut Rng) -> Option<Pickup> {
    let n = backpacks.len();
    if n == 0 {
        return None;
    }
    let depth = (0.0f64.max(-(level as f64)) / super::route::loot::BACKPACK_DEPTH_STEP).floor() as usize;
    let depth_rank = (1 + depth).clamp(1, n);
    match backpacks[rng.next_index(depth_rank).min(n - 1)].as_str() {
        "backpack_small" => Some(Pickup::BackpackSmall),
        "backpack_medium" => Some(Pickup::BackpackMedium),
        "backpack_large" => Some(Pickup::BackpackLarge),
        _ => None,
    }
}

/// The old builder: a `Facility::generate` floor re-dressed as streamed rooms, with Rust's own
/// loot, seams, stair flights and fixtures.
pub fn legacy_chunk(seed: &str, k: ChunkKey, cat: &Catalog, loot: &LootTables) -> (Facility, Fixtures) {
    let text = format!("{seed}:L{}:C{},{}", k.level, k.cx, k.cy);
    // The streamed layout below replaces Facility::generate's generic room
    // graph and population wholesale. Start with its initialized grids without
    // paying to build and then discard that temporary finite floor.
    let mut f = Facility::blank(&text, CHUNK, CHUNK);
    f.level = k.level;
    // CREATE ROOM uses Godot's streamed route, whose room banks and shared halls
    // differ from the finite floor generator. Rebuild the chunk topology before
    // seams, stairs and fixtures are added so every downstream system sees the
    // same bounded rooms and circulation mask.
    let _chunk_layout_perf = crate::app::perf::section("chunk layout");
    let region = region_for(seed, k);
    // level -1: the ported Godot sewer geometry replaces the generic facility floor when the generator gives a chunk
    let sewer_floor = if k.level == super::route::sewer::core::LEVEL || k.level == super::route::sewer::core::LOWER_LEVEL {
        super::route::sewer::chunk(
            super::route::layout::Coord::new(k.cx, k.level, k.cy),
            &super::route::sewer::Inputs { world: seed },
        )
    } else {
        None
    };
    match &sewer_floor {
        Some(sc) => apply_sewer_floor(&mut f, sc, seed, k, cat),
        None => f.apply_godot_streamed_layout(&text, region, cat),
    }
    drop(_chunk_layout_perf);
    // level -3: carve the caverns, halls and crevices (world::inferno) over the maze layout
    if crate::world::levels::layer_name(k.level) == Some("inferno") {
        let inf = super::inferno::InfernoChunk::carve(seed, k);
        for r in inf.clear_rects() {
            f.clear_props_in(r.x, r.y, r.w, r.h);
        }
        for c in inf.open_cells() {
            f.open_cells(c.x, c.y, 1, 1);
        }
        f.rebuild_grids();
    }
    f.has_lift = false;
    f.exit = Cell::new(-10000, -10000);
    // loot (the region's data/loot table): a base battery, then each roll by its chance
    let _chunk_loot_perf = crate::app::perf::section("chunk loot");
    let loot = loot.table_for(region);
    let mut rng = Rng::new(&format!("{text}:LOOT"));
    let mut free: Vec<Cell> = f
        .floor_cells()
        .into_iter()
        .filter(|c| {
            c.len_to(f.start) >= 3.0
                && !f.relays.contains(c)
                && !f.batteries.contains(c)
                && !f.is_water(*c)
                && f.can_occupy(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.23)
        })
        .collect();
    let mut place = |item: Pickup, rng: &mut Rng, out: &mut Vec<(Cell, Pickup)>| {
        if !free.is_empty() {
            out.push((free.remove(rng.next_index(free.len())), item));
        }
    };
    let mut pickups = vec![];
    for b in &loot.base {
        if b == "battery" {
            place(Pickup::Battery, &mut rng, &mut pickups);
        }
    }
    for (kind, chance) in &loot.rolls {
        if (rng.next_float() as f32) >= *chance {
            continue;
        }
        match kind.as_str() {
            "tool" => {
                let total: f32 = loot.tools.iter().map(|t| t.1).sum();
                let mut r = rng.next_float() as f32 * total;
                let mut pick = "taser";
                for (name, w) in &loot.tools {
                    if r < *w {
                        pick = name;
                        break;
                    }
                    r -= w;
                }
                let item = match pick {
                    "crowbar" => Pickup::Crowbar,
                    "rescue_axe" => Pickup::Axe,
                    "lockpick" => Pickup::Lockpick,
                    _ => Pickup::Taser,
                };
                place(item, &mut rng, &mut pickups);
            }
            "battery" => place(Pickup::Battery, &mut rng, &mut pickups),
            "backpack" => {
                if let Some(item) = backpack_pickup(&loot.backpacks, k.level, &mut rng) {
                    place(item, &mut rng, &mut pickups);
                }
            }
            _ => {}
        }
    }
    // one next_float per aid entry in file order (the solo chance), as LootTables.roll_aid
    for (item, chance, _) in &loot.aid {
        let hit = (rng.next_float() as f32) < *chance;
        let pickup = match item.as_str() {
            "stim_pack" => Pickup::Stim,
            "revive_kit" => Pickup::ReviveKit,
            _ => continue,
        };
        if hit {
            place(pickup, &mut rng, &mut pickups);
        }
    }
    f.pickups = pickups;
    drop(_chunk_loot_perf);
    let mut sewer_coins: Vec<Cell> = vec![];
    if let Some(sc) = &sewer_floor {
        sewer_coins = apply_sewer_dressing(&mut f, seed, sc);
    }

    let _chunk_links_perf = crate::app::perf::section("chunk links");
    // seams: open the chunk's border where its boundaries open, then run a 3-wide hall inward
    let sides = [
        (format!("V:{}:{}:{}", k.level, k.cx, k.cy), 0),
        (format!("V:{}:{}:{}", k.level, k.cx + 1, k.cy), 1),
        (format!("H:{}:{}:{}", k.level, k.cx, k.cy), 2),
        (format!("H:{}:{}:{}", k.level, k.cx, k.cy + 1), 3),
    ];
    for (key, side) in sides {
        if sewer_floor.is_some() {
            break; // the sewer's own tunnel cross already reaches every border
        }
        for o in boundary_offsets(seed, &key) {
            for step in 0..CHUNK {
                let (x, y) = match side {
                    0 => (step, o),
                    1 => (CHUNK - 1 - step, o),
                    2 => (o, step),
                    _ => (o, CHUNK - 1 - step),
                };
                let (ax, ay) = if side < 2 { (x, y - 1) } else { (x - 1, y) };
                let (w, h) = if side < 2 { (1, 3) } else { (3, 1) };
                let was_open = !f.is_wall_cell(x, y) && step > 1;
                f.open_cells(ax, ay, w, h);
                if was_open {
                    break;
                }
            }
        }
    }

    // stairwells: level 0 takes the layout's stairwell (route::live::carve_stair_well), walled and dead-ended with
    // no bay. The levels below keep the bay of each boundary's stair link, down to `level - 1` and up to `level + 1`.
    // Every connection is a `Flight` (world::flight_links), the same one on both levels.
    if k.level == 0 {
        for well in super::route::live::layout_stairwells(seed, k) {
            let flight = super::flight_links::level0_well_flight(
                seed,
                k.cx,
                k.cy,
                well.kind,
                (well.door.x, well.door.y, well.door.w),
                well.north,
            );
            let stair = super::route::live::carve_stair_well(&mut f, &well, &flight);
            f.stairs.push(stair);
        }
    } else {
        // this level is the upper end of the link below it
        if let Some(flight) = bay_flight(seed, k.cx, k.cy, k.level) {
            carve_bay(&mut f, &flight, true);
            f.stairs.push(upper_stair(&flight));
        }
        // this level is the lower end of the link above it
        if sewer_floor.is_some() && k.level == super::route::sewer::core::LEVEL {
            // the way up to level 0 is the maze's stairwell: its flights seen from the sewer. The well is already
            // carved by the sewer; open the foot, the margins and the hole.
            for flight in sewer_well_flights(seed, k) {
                carve_bay(&mut f, &flight, false);
                f.stairs.push(lower_stair(&flight));
            }
        } else if let Some(flight) = bay_flight(seed, k.cx, k.cy, k.level + 1) {
            carve_bay(&mut f, &flight, true);
            f.stairs.push(lower_stair(&flight));
        }
    }
    // the sewer's ladder bottoms: the same cells as the maze's ladder tops, so both levels agree. A ladder is the hatch
    // cell and one step cell beside it (`Stair::run`), the same on both levels: the sewer opens the foot, the step cell
    // and their clear margins; the maze (the legacy builder only) opens the hatch and the step cell and clears them.
    for s in crate::world::level_links::ladders(seed, k) {
        let cells = if k.level == 0 { vec![s.cell, s.step_cell()] } else { s.carve_cells() };
        for c in cells {
            f.open_cells(c.x, c.y, 1, 1);
            if k.level == 0 {
                f.clear_props_at(c);
            }
        }
        f.stairs.push(s);
    }
    drop(_chunk_links_perf);
    f.level = k.level;
    f.remove_listed_props(cat);
    let _chunk_grid_perf = crate::app::perf::section("chunk grid");
    f.rebuild_grids();
    drop(_chunk_grid_perf);
    let _chunk_fixtures_perf = crate::app::perf::section("chunk fixtures");
    let (mut fix, box_cell) = fixtures::place_with_box(&f);
    fixtures::adopt_extra_hiding(&mut f, &fix.extra_hiding);
    // the floor's music box is a collectable pickup (game/music_box.rs), not a fixture
    f.pickups.extend(box_cell.map(|c| (c, Pickup::MusicBox)));
    fix.coins.extend(sewer_coins);
    drop(_chunk_fixtures_perf);
    (f, fix)
}

/// Opens the room a flight needs in `f`: the bounding box of its carve cells (hole, landing, foot, both clear margins)
/// grown by one and kept inside the chunk border is cleared of props. With `hall`, it is also opened and a 3-wide hall
/// runs from it to the lobby so the flight is always reachable. Without, only the carve cells themselves are opened
/// (the sewer's wells are already joined to their own tunnels).
fn carve_bay(f: &mut Facility, flight: &Flight, hall: bool) {
    let cells = carve_cells(flight);
    let (x0, y0, w, h) = bay_rect(flight);
    let x1 = x0 + w - 1;
    f.clear_props_reaching(x0, y0, w, h);
    if hall {
        f.open_cells(x0, y0, w, h);
        let (cx, cy) = (f.start.x, f.start.y);
        let mx = (x0 + x1) / 2;
        f.open_cells(mx.min(cx) - 1, y0, (mx - cx).abs() + 3, 3);
        f.open_cells(cx - 1, y0.min(cy), 3, (y0 - cy).abs() + 3);
    } else {
        for c in &cells {
            f.open_cells(c.x, c.y, 1, 1);
        }
    }
}

/// The sewer port's own pickups and hiding spots (`sewer::pickups::chunk_pickups`, `sewer::fixtures::fixtures`)
/// replace the legacy loot rolls on a sewer floor. Relays and batteries fill their lists, tools, aid and backpacks
/// become pickups, and the coins are returned for the chunk's `Fixtures`. A name the `Pickup` enum cannot hold is dropped.
fn apply_sewer_dressing(f: &mut Facility, seed: &str, sc: &super::route::sewer::SewerChunk) -> Vec<Cell> {
    f.relays.clear();
    f.batteries.clear();
    f.pickups.clear();
    let mut coins = vec![];
    for p in super::route::sewer::pickups::chunk_pickups(seed, sc, false) {
        let cell = Cell::new(p.cell.x, p.cell.y);
        if f.is_water(cell) {
            continue;
        }
        match p.kind {
            "relay" => f.relays.push(cell),
            "battery" => f.batteries.push(cell),
            "coin" => coins.push(cell),
            _ => {
                let item = match p.item {
                    "taser" => Pickup::Taser,
                    "lockpick" => Pickup::Lockpick,
                    "crowbar" => Pickup::Crowbar,
                    "rescue_axe" => Pickup::Axe,
                    "stim_pack" => Pickup::Stim,
                    "revive_kit" => Pickup::ReviveKit,
                    "backpack_small" => Pickup::BackpackSmall,
                    "backpack_medium" => Pickup::BackpackMedium,
                    "backpack_large" => Pickup::BackpackLarge,
                    _ => continue,
                };
                f.pickups.push((cell, item));
            }
        }
    }
    let rooms = f.rooms.clone();
    f.hiding = super::route::sewer::fixtures::fixtures(seed, sc)
        .iter()
        .filter(|x| x.kind == "hiding_spot" && x.hide.is_none() && !f.is_water(Cell::new(x.cell.x, x.cell.y)))
        .map(|x| super::facility::Hiding {
            cell: Cell::new(x.cell.x, x.cell.y),
            facing: (x.open_direction.x, x.open_direction.y),
            room: rooms
                .iter()
                .position(|r| x.cell.x >= r.x && x.cell.y >= r.y && x.cell.x < r.x + r.w && x.cell.y < r.y + r.h),
            kind: String::new(),
        })
        .collect();
    coins
}

/// Overwrites `f`'s floor with the sewer chunk: SOLID is wall, every other code (dry, water, bridge, ramp, shaft, and
/// the waterfall void FALL) is open. A FALL cell is not a wall: on level -1 it is a drop (`Facility::set_drop_cells`),
/// open air a body may walk into and fall through, that hunters never enter and nothing is placed on; the spur, the
/// overlook and the lip round it stay floor. On level -2 the same x/z cells are the air above the landing pool
/// (`set_air_above_cells`). The sewer's rooms and props (`sewer_props`) replace the facility's. The start is the junction.
fn apply_sewer_floor(f: &mut Facility, sc: &super::route::sewer::SewerChunk, seed: &str, k: ChunkKey, cat: &Catalog) {
    use super::route::sewer::core::{FALL, LEVEL, LOWER_LEVEL, SOLID};
    f.wall = (0..CHUNK as usize)
        .map(|z| (0..CHUNK as usize).map(|x| sc.sewer_grid[z][x] == SOLID).collect())
        .collect();
    if k.level == LEVEL {
        let mut falls = vec![];
        for z in 0..CHUNK as usize {
            for x in 0..CHUNK as usize {
                if sc.sewer_grid[z][x] == FALL {
                    falls.push(Cell::new(x as i32, z as i32));
                }
            }
        }
        f.set_drop_cells(&falls);
    } else if k.level == LOWER_LEVEL {
        f.set_air_above_cells(&super::flight_links::fall_cells(seed, k.cx, k.cy));
    }
    // the iron railings along the banks are walls (render/sewer_rails.rs draws the same segments): the runner, the bots
    // and the hunters stay on the walkway, and the waterfall jump's gap stays the way in
    let rails = crate::render::sewer_rails::config();
    if rails.collision.solid {
        let gap = sc.falls.as_ref().map(|fl| (fl.jump.cell.x, fl.jump.cell.y));
        let edges: Vec<(Cell, u8)> = crate::render::sewer_rails::channel_rail_segments(&sc.sewer_grid, gap)
            .into_iter()
            .map(|s| (Cell::new(s.x, s.z), super::facility::rail_side_bit(s.step)))
            .collect();
        f.set_rails(&edges, rails.inset_m, rails.collision.thickness_m);
    }
    f.rooms = sc
        .rooms
        .iter()
        .map(|r| super::facility::Room {
            id: r.id.clone(),
            sector: String::new(),
            x: r.rect.x,
            y: r.rect.y,
            w: r.rect.w,
            h: r.rect.h,
            center: Cell::new(r.center.x, r.center.y),
            rtype: r.room_type.to_string(),
            label: r.room_type.to_string(),
            region: "sewer".to_string(),
            landmark: false,
            prop_types: vec![],
        })
        .collect();
    // the sewer's own props (pipes, valves, pump, outflow, gates, lamps, decks), indexed now for the loot rolls' `can_occupy`
    // and again by the `rebuild_grids` at the end of the chunk, which builds the hunters' grid from them
    f.props = super::sewer_props::props(sc, &f.rooms, cat);
    // the pulsing emergency lights are their own layer over the generator's props (they are not in the golden layout)
    f.props.extend(super::sewer_emergency::props(sc, seed, k.cx, k.cy, k.level, &f.rooms));
    // the channel water for the maps: the cells the swim, slip and landing code read as water (`water_depth_at_level`)
    let wet: Vec<Cell> = (0..CHUNK)
        .flat_map(|z| (0..CHUNK).map(move |x| (x, z)))
        .filter(|&(x, z)| super::route::sewer::water::water_depth_at_level(&sc.sewer_grid, x, z, k.level) > 0.0)
        .map(|(x, z)| Cell::new(x, z))
        .collect();
    f.set_water_cells(&wet);
    f.rebuild_grids();
    f.hiding.clear();
    f.room_lights.clear();
    f.relays.clear();
    f.batteries.clear();
    f.safe_zone = None;
    f.start = Cell::new(CHUNK / 2, CHUNK / 2);
    f.hunter_spawn = f.start;
}

/// The live window: 3x3 chunks around `center` on `level`, merged into one floor, plus the
/// fixtures of the window (the vent ring from the centre chunk, every
/// chunk's coins).
pub fn window(
    seed: &str,
    center: (i32, i32),
    level: i32,
    cat: &Catalog,
    loot: &LootTables,
    cache: &mut HashMap<ChunkKey, (Facility, Fixtures)>,
) -> (Facility, Fixtures) {
    let mut parts = vec![];
    let mut fix = Fixtures {
        vents: vec![],
        coins: vec![],
        extra_hiding: vec![],
    };
    for dy in -1..=1 {
        for dx in -1..=1 {
            let k = ChunkKey {
                cx: center.0 + dx,
                cy: center.1 + dy,
                level,
            };
            let entry = cache.entry(k).or_insert_with(|| {
                let _chunk_build_perf = crate::app::perf::section("chunk build");
                chunk(seed, k, cat, loot)
            });
            let (ox, oy) = ((dx + 1) * CHUNK, (dy + 1) * CHUNK);
            let sh = |c: Cell| Cell::new(c.x + ox, c.y + oy);
            if dx == 0 && dy == 0 {
                fix.vents = entry.1.vents.iter().map(|c| sh(*c)).collect();
            }
            fix.coins.extend(entry.1.coins.iter().map(|c| sh(*c)));
            parts.push((
                entry.0.clone(),
                ox,
                oy,
                regions::region_index(region_for(seed, k)),
            ));
        }
    }
    if cache.len() > 192 {
        // CHUNK_CACHE_LIMIT: drop chunks far from the window
        cache.retain(|k, _| {
            k.level == level && (k.cx - center.0).abs() <= 3 && (k.cy - center.1).abs() <= 3
        });
    }
    let names = regions::NAMES.iter().map(|s| s.to_string()).collect();
    let mid = parts[4].0.start;
    let mut f = Facility::merge(
        parts,
        CHUNK * WINDOW,
        CHUNK * WINDOW,
        names,
        &format!("{seed}:L{level}"),
    );
    f.start = Cell::new(mid.x + CHUNK, mid.y + CHUNK);
    f.hunter_spawn = f.start;
    // no service lift on the Infinite route (has_lift false); the exit marks the centre lobby
    f.exit = f.start;
    f.has_lift = false;
    f.origin = ((center.0 - 1) * CHUNK, (center.1 - 1) * CHUNK);
    // the caverns' noisy floor and ceiling: a pure function of the world text and the world cell
    if crate::world::levels::layer_name(level) == Some("inferno") {
        f.set_terrain(seed, WINDOW);
    }
    (f, fix)
}

/// The chunks the window will need that a runner at window-local `pos` (metres, the window's own frame) has not yet
/// got, when it is within `margin` metres of the edge of the middle chunk: the 3x3 around each centre it can step to
/// (one chunk along each edge it is near, and the diagonal when it is near a corner), in a fixed order. Empty away from
/// the edges. `window` builds missing chunks synchronously, so generating these a few frames ahead (into the same
/// cache) turns the crossing's one long frame into a cache hit.
pub fn approach_chunks(center: (i32, i32), level: i32, pos: (f32, f32), margin: f32) -> Vec<ChunkKey> {
    let (lo, hi) = (CHUNK as f32, (2 * CHUNK) as f32);
    // the edges within the margin on one axis, each with its distance. Both can be (a margin over half a chunk): until
    // 2026-11-12 only the low edge was looked at then, so with the shipped 64 m nothing was ever generated ahead of a
    // crossing to the east or the south.
    let near = |v: f32| -> Vec<(i32, f32)> {
        let mut edges = vec![];
        if v < lo + margin {
            edges.push((-1, v - lo));
        }
        if v >= hi - margin {
            edges.push((1, hi - v));
        }
        edges
    };
    let (xs, ys) = (near(pos.0), near(pos.1));
    // the straight steps, nearest edge first, then the diagonals
    let mut straight: Vec<((i32, i32), f32)> =
        xs.iter().map(|&(s, d)| ((s, 0), d)).chain(ys.iter().map(|&(s, d)| ((0, s), d))).collect();
    straight.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut steps: Vec<(i32, i32)> = straight.into_iter().map(|(s, _)| s).collect();
    for &(sx, _) in &xs {
        for &(sy, _) in &ys {
            steps.push((sx, sy));
        }
    }
    let mut out: Vec<ChunkKey> = vec![];
    for (ax, ay) in steps {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let k = ChunkKey { cx: center.0 + ax + dx, cy: center.1 + ay + dy, level };
                // the window the runner is in already holds its own 3x3
                let in_window = (k.cx - center.0).abs() <= 1 && (k.cy - center.1).abs() <= 1;
                if !in_window && !out.contains(&k) {
                    out.push(k);
                }
            }
        }
    }
    out
}

/// The window centre after the crossing a runner at window-local `pos` is about to make: the centre beyond the edge of
/// the middle chunk it is nearest to, when that edge is within `margin` metres. None away from the edges.
pub fn next_center(center: (i32, i32), pos: (f32, f32), margin: f32) -> Option<(i32, i32)> {
    let (lo, hi) = (CHUNK as f32, (2 * CHUNK) as f32);
    let edges = [((-1, 0), pos.0 - lo), ((1, 0), hi - pos.0), ((0, -1), pos.1 - lo), ((0, 1), hi - pos.1)];
    let ((dx, dy), d) = edges.into_iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    (d < margin).then_some((center.0 + dx, center.1 + dy))
}

/// A throwaway floor a Game holds before its first window is built.
pub fn placeholder(seed: &str, cat: &Catalog) -> Facility {
    Facility::generate(&format!("{seed}:SHELL"), &Options::default(), cat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn cat() -> &'static Catalog {
        static CAT: OnceLock<Catalog> = OnceLock::new();
        CAT.get_or_init(Catalog::load)
    }

    const WORLDS: [&str; 3] = ["1426541270", "4157433391", "2899547009"];

    #[test]
    fn nothing_is_approached_from_the_middle_of_the_chunk() {
        let mid = (1.5 * CHUNK as f32, 1.5 * CHUNK as f32);
        assert!(approach_chunks((4, -2), 0, mid, 8.0).is_empty());
    }

    #[test]
    fn near_an_edge_the_next_three_chunks_are_wanted() {
        // 3 m from the east edge of the middle chunk: the window to the east adds the column cx + 2
        let pos = (2.0 * CHUNK as f32 - 3.0, 1.5 * CHUNK as f32);
        let want = approach_chunks((4, -2), 7, pos, 8.0);
        let mut got: Vec<(i32, i32)> = want.iter().map(|k| (k.cx, k.cy)).collect();
        got.sort();
        assert_eq!(got, vec![(6, -3), (6, -2), (6, -1)]);
        assert!(want.iter().all(|k| k.level == 7));
        // west edge: the column cx - 2
        let pos = (CHUNK as f32 + 1.0, 1.5 * CHUNK as f32);
        let mut got: Vec<(i32, i32)> = approach_chunks((4, -2), 0, pos, 8.0).iter().map(|k| (k.cx, k.cy)).collect();
        got.sort();
        assert_eq!(got, vec![(2, -3), (2, -2), (2, -1)]);
    }

    #[test]
    fn a_margin_over_half_a_chunk_looks_both_ways_nearest_edge_first() {
        // 5 m from the east edge with a 64 m margin: every edge is "near", and the east column comes first
        let pos = (2.0 * CHUNK as f32 - 5.0, 1.5 * CHUNK as f32);
        let want = approach_chunks((0, 0), 0, pos, 64.0);
        assert_eq!(want.len(), 16, "the whole ring round the window");
        assert!(want[..3].iter().all(|k| k.cx == 2), "the east column first: {:?}", &want[..3]);
        assert!(want.iter().any(|k| k.cx == -2) && want.iter().any(|k| k.cy == 2) && want.iter().any(|k| k.cy == -2));
    }

    /// The centre a runner at window-local `pos` will have after its next crossing, when it is within `margin` of one
    /// edge of the middle chunk.
    #[test]
    fn the_next_centre_is_the_one_beyond_the_nearest_edge() {
        let c = CHUNK as f32;
        assert_eq!(next_center((4, -2), (1.5 * c, 1.5 * c), 10.0), None, "the middle: no crossing in sight");
        assert_eq!(next_center((4, -2), (2.0 * c - 3.0, 1.5 * c), 10.0), Some((5, -2)));
        assert_eq!(next_center((4, -2), (c + 2.0, 1.5 * c), 10.0), Some((3, -2)));
        assert_eq!(next_center((4, -2), (1.5 * c, c + 9.0), 10.0), Some((4, -3)));
        // near a corner: the nearer edge
        assert_eq!(next_center((0, 0), (2.0 * c - 2.0, 2.0 * c - 6.0), 10.0), Some((1, 0)));
    }

    #[test]
    fn near_a_corner_the_diagonal_window_is_wanted_without_repeats() {
        let pos = (2.0 * CHUNK as f32 - 1.0, 2.0 * CHUNK as f32 - 1.0);
        let want = approach_chunks((0, 0), 0, pos, 8.0);
        // the east window's new column (3) and the south window's new row (3), and the corner (2, 2) the diagonal
        // window adds, each once: 7 chunks
        assert_eq!(want.len(), 7);
        for k in &want {
            assert!(k.cx == 2 || k.cy == 2, "outside the current window: {k:?}");
            assert!(want.iter().filter(|o| *o == k).count() == 1);
        }
        assert!(want.contains(&ChunkKey { cx: 2, cy: 2, level: 0 }));
    }

    #[test]
    fn the_level_stack_is_zero_down_to_the_inferno() {
        // a test that registers a pack level (world::levels) holds this lock while the level is in the table
        let _lock = crate::world::levels::TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        assert!(level_allowed(0) && level_allowed(-1) && level_allowed(-2) && level_allowed(INFERNO_LEVEL));
        assert!(!level_allowed(1) && !level_allowed(INFERNO_LEVEL - 1));
    }

    #[test]
    fn no_stair_link_leads_above_zero_or_below_the_inferno() {
        for world in WORLDS {
            for cx in -5..=5 {
                for cy in -5..=5 {
                    assert!(stair_link(world, cx, cy, 1).is_none());
                    assert!(stair_link(world, cx, cy, INFERNO_LEVEL).is_none());
                }
            }
            // the origin column links every storey from 0 down to the Inferno
            for upper in (INFERNO_LEVEL + 1)..=0 {
                assert!(stair_link(world, 0, 0, upper).is_some(), "{world} upper {upper}");
            }
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_backpack_draw_grows_with_depth_and_skips_empty_or_unknown_lists() {
        let packs = names(&["backpack_small", "backpack_medium", "backpack_large"]);
        let mut rng = Rng::new("packs");
        // levels 0 to -3: rank 1, always the small pack
        for level in [0, -1, -2, -3] {
            for _ in 0..20 {
                assert_eq!(backpack_pickup(&packs, level, &mut rng), Some(Pickup::BackpackSmall));
            }
        }
        // level -4: rank 2, small or medium, never large
        let draws: Vec<_> = (0..200).map(|_| backpack_pickup(&packs, -4, &mut rng)).collect();
        assert!(draws.contains(&Some(Pickup::BackpackMedium)));
        assert!(!draws.contains(&Some(Pickup::BackpackLarge)));
        // level -12: rank capped at 3, the large pack comes up
        let draws: Vec<_> = (0..200).map(|_| backpack_pickup(&packs, -12, &mut rng)).collect();
        assert!(draws.contains(&Some(Pickup::BackpackLarge)));
        assert_eq!(backpack_pickup(&[], -12, &mut rng), None);
        assert_eq!(backpack_pickup(&names(&["backpack_huge"]), 0, &mut rng), None);
    }

    #[test]
    fn legacy_loot_places_backpacks_and_revive_kits_from_the_table() {
        let table = crate::content::loot::Loot {
            id: "route".into(),
            base: vec![],
            rolls: vec![("backpack".into(), 1.0)],
            tools: vec![],
            backpacks: names(&["backpack_small", "backpack_medium", "backpack_large"]),
            aid: vec![("stim_pack".into(), 1.0, None), ("revive_kit".into(), 1.0, None), ("mystery".into(), 1.0, None)],
        };
        let loot = LootTables { route: table.clone(), sewer: table };
        // the legacy level-0 fallback and the Inferno (the sewer levels place the sewer port's own pickups);
        // both are shallower than BACKPACK_DEPTH_STEP, so rank 1: the small pack
        let table_loot = |f: &Facility| -> Vec<Pickup> {
            f.pickups.iter().map(|p| p.1).filter(|p| *p != Pickup::MusicBox).collect()
        };
        for world in WORLDS {
            for level in [0, INFERNO_LEVEL] {
                let (f, _) = legacy_chunk(world, ChunkKey { cx: 2, cy: 1, level }, cat(), &loot);
                let got = table_loot(&f);
                assert!(got.contains(&Pickup::BackpackSmall), "{world} L{level}: {got:?}");
                assert!(got.contains(&Pickup::Stim), "{world} L{level}: {got:?}");
                assert!(got.contains(&Pickup::ReviveKit), "{world} L{level}: {got:?}");
                assert_eq!(got.len(), 3, "{world} L{level}: an unknown aid item placed something: {got:?}");
            }
        }
        // an empty backpack list rolls the backpack but places nothing
        let mut none = loot.clone();
        none.route.backpacks.clear();
        none.sewer.backpacks.clear();
        none.route.aid.clear();
        none.sewer.aid.clear();
        let (f, _) = legacy_chunk(WORLDS[0], ChunkKey { cx: 2, cy: 1, level: INFERNO_LEVEL }, cat(), &none);
        assert!(table_loot(&f).is_empty(), "{:?}", f.pickups);
    }

    #[test]
    fn an_inferno_chunk_has_a_way_up_and_no_way_down() {
        let loot = LootTables::default();
        for world in WORLDS {
            let (f, _) = chunk(world, ChunkKey { cx: 0, cy: 0, level: INFERNO_LEVEL }, cat(), &loot);
            assert!(f.stairs.iter().any(|s| s.dir > 0), "{world}: no flight up from the Inferno");
            assert!(f.stairs.iter().all(|s| s.dir > 0), "{world}: a flight leads below the Inferno");
        }
    }

    /// Level -2 is the ported lower sewer: it has floor, and every stair down from -1 has the up stair of the same flight on -2.
    #[test]
    fn the_lower_sewer_meets_the_first_sewer_stairs() {
        let loot = LootTables::default();
        let (mut stairs, mut matched) = (0, 0);
        for world in WORLDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    let (low, _) = chunk(world, ChunkKey { cx, cy, level: -2 }, cat(), &loot);
                    assert!(!low.floor_cells().is_empty(), "{world} {cx},{cy}: no walkable cells");
                    let (sew, _) = chunk(world, ChunkKey { cx, cy, level: -1 }, cat(), &loot);
                    for s in sew.stairs.iter().filter(|s| s.dir < 0 && !s.is_ladder()) {
                        stairs += 1;
                        let want = s.flight(-1).unwrap_or_else(|| panic!("{world} {cx},{cy}: the -1 stair has no flight"));
                        let t = low
                            .stairs
                            .iter()
                            .find(|t| t.dir > 0 && !t.is_ladder() && t.flight(-2) == Some(want))
                            .unwrap_or_else(|| panic!("{world} {cx},{cy}: no -2 up stair of {want:?}"));
                        assert!(!low.is_wall_cell(t.cell.x, t.cell.y), "{world} {cx},{cy}: the foot is walled");
                        matched += 1;
                    }
                }
            }
        }
        assert!(stairs > 0 && matched == stairs, "{matched}/{stairs}");
    }

    /// Level -1 is the ported sewer floor, and every level 0 stair has the sewer stair of the same flight.
    #[test]
    fn the_sewer_floor_meets_the_maze_stairs() {
        let loot = LootTables::default();
        let (mut stairs, mut matched) = (0, 0);
        for world in WORLDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    let (sew, _) = chunk(world, ChunkKey { cx, cy, level: -1 }, cat(), &loot);
                    assert!(!sew.floor_cells().is_empty(), "{world} {cx},{cy}: no walkable cells");
                    let (top, _) = chunk(world, ChunkKey { cx, cy, level: 0 }, cat(), &loot);
                    for s in top.stairs.iter().filter(|s| !s.is_ladder()) {
                        stairs += 1;
                        let want = s.flight(0).unwrap_or_else(|| panic!("{world} {cx},{cy}: the level 0 stair has no flight"));
                        let t = sew
                            .stairs
                            .iter()
                            .find(|t| t.dir > 0 && !t.is_ladder() && t.flight(-1) == Some(want))
                            .unwrap_or_else(|| panic!("{world} {cx},{cy}: no sewer stair of {want:?}"));
                        assert!(!sew.is_wall_cell(t.cell.x, t.cell.y), "{world} {cx},{cy}: the foot is walled");
                        matched += 1;
                    }
                }
            }
        }
        assert!(stairs > 0 && matched == stairs, "{matched}/{stairs}");
    }

    #[test]
    fn the_bay_candidates_start_at_the_link_and_stay_in_the_parity_bays() {
        for world in WORLDS {
            for cx in -3..=3 {
                for cy in -3..=3 {
                    for upper in [0, -1, -2, -3] {
                        let c = bay_candidates(world, cx, cy, upper);
                        match stair_link(world, cx, cy, upper) {
                            None => assert!(c.is_empty(), "{world} {cx},{cy} upper {upper}"),
                            Some(link) => {
                                assert_eq!(c.len(), 8, "{world} {cx},{cy} upper {upper}");
                                assert_eq!(c[0], link);
                                let bays = STAIR_BAYS_BY_PARITY[upper.rem_euclid(2) as usize];
                                for (anchor, _) in &c {
                                    assert!(bays.iter().any(|b| STAIR_BAY_ANCHORS[*b].contains(anchor)));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
