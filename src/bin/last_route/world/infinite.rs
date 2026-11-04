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
use super::flight_links::{bay_flight, carve_cells, lower_stair, sewer_well_flights, upper_stair};
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
/// Relays and batteries per chunk (estimated: "denser interactables", spread over many chunks).
const CHUNK_RELAYS: usize = 3;
const CHUNK_BATTERIES: usize = 4;

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
    (INFERNO_LEVEL..=0).contains(&level)
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

/// The old builder: a `Facility::generate` floor re-dressed as streamed rooms, with Rust's own
/// loot, seams, stair flights and fixtures.
pub fn legacy_chunk(seed: &str, k: ChunkKey, cat: &Catalog, loot: &LootTables) -> (Facility, Fixtures) {
    let origin = k.cx == 0 && k.cy == 0 && k.level == 0;
    let text = format!("{seed}:L{}:C{},{}", k.level, k.cx, k.cy);
    let opt = Options {
        relay_count: CHUNK_RELAYS,
        battery_count: CHUNK_BATTERIES,
        safe_zone_count: if origin { 2 } else { 0 },
        ..Default::default()
    };
    let mut f = Facility::generate(&text, &opt, cat);
    f.level = k.level;
    // CREATE ROOM uses Godot's streamed route, whose room banks and shared halls
    // differ from the finite floor generator. Rebuild the chunk topology before
    // seams, stairs and fixtures are added so every downstream system sees the
    // same bounded rooms and circulation mask.
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
        Some(sc) => apply_sewer_floor(&mut f, sc),
        None => f.apply_godot_streamed_layout(&text, region, cat),
    }
    // level -3: carve the caverns, halls and crevices (world::inferno) over the maze layout
    if k.level == INFERNO_LEVEL {
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
    let loot = loot.table_for(region);
    let mut rng = Rng::new(&format!("{text}:LOOT"));
    let mut free: Vec<Cell> = f
        .floor_cells()
        .into_iter()
        .filter(|c| {
            c.len_to(f.start) >= 3.0
                && !f.relays.contains(c)
                && !f.batteries.contains(c)
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
            _ => {}
        }
    }
    for (item, chance) in &loot.aid {
        if item == "stim_pack" && (rng.next_float() as f32) < *chance {
            place(Pickup::Stim, &mut rng, &mut pickups);
        }
    }
    f.pickups = pickups;
    let mut sewer_coins: Vec<Cell> = vec![];
    if let Some(sc) = &sewer_floor {
        sewer_coins = apply_sewer_dressing(&mut f, seed, sc);
    }

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
    // the sewer's ladder bottoms: the same cells as the maze's ladder tops, so both levels agree
    for s in crate::world::level_links::ladders(seed, k) {
        f.open_cells(s.cell.x - 1, s.cell.y - 1, 3, 3);
        f.stairs.push(s);
    }
    f.level = k.level;
    f.rebuild_grids();
    let mut fix = fixtures::place(&f);
    fix.coins.extend(sewer_coins);
    (f, fix)
}

/// Opens the room a flight needs in `f`: the bounding box of its carve cells (hole, landing, foot, both clear margins)
/// grown by one and kept inside the chunk border is cleared of props. With `hall`, it is also opened and a 3-wide hall
/// runs from it to the lobby so the flight is always reachable. Without, only the carve cells themselves are opened
/// (the sewer's wells are already joined to their own tunnels).
fn carve_bay(f: &mut Facility, flight: &Flight, hall: bool) {
    let cells = carve_cells(flight);
    let x0 = (cells.iter().map(|c| c.x).min().unwrap_or(1) - 1).max(1);
    let x1 = (cells.iter().map(|c| c.x).max().unwrap_or(1) + 1).min(CHUNK - 2);
    let y0 = (cells.iter().map(|c| c.y).min().unwrap_or(1) - 1).max(1);
    let y1 = (cells.iter().map(|c| c.y).max().unwrap_or(1) + 1).min(CHUNK - 2);
    f.clear_props_in(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
    if hall {
        f.open_cells(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
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
        .filter(|x| x.kind == "hiding_spot" && x.hide.is_none())
        .map(|x| super::facility::Hiding {
            cell: Cell::new(x.cell.x, x.cell.y),
            facing: (x.open_direction.x, x.open_direction.y),
            room: rooms
                .iter()
                .position(|r| x.cell.x >= r.x && x.cell.y >= r.y && x.cell.x < r.x + r.w && x.cell.y < r.y + r.h),
        })
        .collect();
    coins
}

/// Overwrites `f`'s floor with the sewer chunk: SOLID and FALL are walls, every other code (dry, water, bridge, ramp,
/// shaft) is open. The sewer's rooms replace the facility's; props start empty. The start is the junction.
fn apply_sewer_floor(f: &mut Facility, sc: &super::route::sewer::SewerChunk) {
    use super::route::sewer::core::{FALL, SOLID};
    f.wall = (0..CHUNK as usize)
        .map(|z| (0..CHUNK as usize).map(|x| matches!(sc.sewer_grid[z][x], SOLID | FALL)).collect())
        .collect();
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
    f.props.clear();
    f.hiding.clear();
    f.room_lights.clear();
    f.relays.clear();
    f.batteries.clear();
    f.safe_zone = None;
    f.start = Cell::new(CHUNK / 2, CHUNK / 2);
    f.hunter_spawn = f.start;
}

/// The live window: 3x3 chunks around `center` on `level`, merged into one floor, plus the
/// fixtures of the window (heater and music box from the centre chunk, its vent ring, every
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
        heater: None,
        music_box: None,
        vents: vec![],
        coins: vec![],
    };
    for dy in -1..=1 {
        for dx in -1..=1 {
            let k = ChunkKey {
                cx: center.0 + dx,
                cy: center.1 + dy,
                level,
            };
            let entry = cache.entry(k).or_insert_with(|| chunk(seed, k, cat, loot));
            let (ox, oy) = ((dx + 1) * CHUNK, (dy + 1) * CHUNK);
            let sh = |c: Cell| Cell::new(c.x + ox, c.y + oy);
            if dx == 0 && dy == 0 {
                fix.heater = entry.1.heater.map(sh);
                fix.music_box = entry.1.music_box.map(sh);
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
    (f, fix)
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
    fn the_level_stack_is_zero_down_to_the_inferno() {
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
