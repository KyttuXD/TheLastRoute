//! Stage 3c part 3 of the Infinite route: the music boxes of a level-0 chunk, a port of
//! `StreamedRouteGenerator.music_box_chunk` (streamed_route_generator.gd:5471-5482), `_music_boxes` (5497-5565),
//! `_music_box_table` (5570-5591) and the `MUSIC_BOX_*` constants (5491-5494), with the helpers they reach.
//!
//! `_build_chunk` calls `_music_boxes` after `_pickups` (1039) and before `_ensure_pickups_reachable` (809). The
//! fixture and pickup cells are read as they were placed, so `layout::chunk_layout` passes the pickups from before
//! the ensure pass.
//!
//! RNG:
//! - `"<world>:MUSICBOX:<region_x>:<level>:<region_z>"` (`music_box_chunk`): `MUSIC_BOXES_PER_REGION` = 2 draws from
//!   the 16 chunk slots of a region, each without replacement. It runs for every non-safe chunk.
//! - `"<world>:MUSICBOX-CELL:<chunk>"` (`_music_boxes`): one draw in whichever branch has a candidate (the table
//!   list, the against-wall list, the interior list, or the open-floor list), and none when there is no candidate.
//!
//! Stairs are empty on level 0 (stair_links, 1399), so `_stair_door_exclusion` is empty and no stairs are passed.
//! The RoomCatalog surface helpers come from `furnished` (`is_surface_prop`, `surface_parent_covers`). The height
//! and the `SURFACE_HEIGHTS` keys come from data/rooms/catalog.json (`surface_table`): `catalog.rs` reads the
//! table without its base values, so it cannot answer `surface_height` here.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::fixtures::Fixture;
use super::furnished::{is_surface_prop, surface_parent_covers};
use super::layout::{Coord, Grid, Room, CHUNK, V2};
use super::pickups::Pickup;
use super::props::{self, Prop};
use super::regions::REGION_SIZE_CHUNKS;
use crate::content::embedded;
use crate::world::rng::Rng;
use serde_json::Value;

/// `GameConfig.MUSIC_BOXES_PER_REGION` (game_config.gd:546), read from data/music_box.json (`boxes_per_region`).
pub fn boxes_per_region() -> usize {
    crate::game::music_box::rules().boxes_per_region
}
/// `MUSIC_BOX_FLOOR_Y` (streamed_route_generator.gd:5491).
const MUSIC_BOX_FLOOR_Y: f64 = 0.04;
/// `MUSIC_BOX_TABLE_LIFT` (5492): added to the tabletop height.
const MUSIC_BOX_TABLE_LIFT: f64 = 0.01;
/// `MUSIC_BOX_WALL_NUDGE` (5494): how far a box slides from its cell centre toward the wall or furniture.
const MUSIC_BOX_WALL_NUDGE: f32 = 0.25;

/// One music box of a chunk: the Godot dictionary's `id`, `cell`, `y`, and the optional `offset` and `on`.
#[derive(Clone, Debug, PartialEq)]
pub struct MusicBox {
    pub id: String,
    pub cell: V2,
    pub y: f64,
    /// The table's placement plus mount offset (a table box, zero when the table has none), or the wall nudge (a
    /// floor box against a wall or furniture). `None` for an open floor box, whose Godot entry has no `offset`.
    pub offset: Option<[f32; 2]>,
    /// The surface type of a table box (`on`); `None` on the floor.
    pub on: Option<&'static str>,
    /// The tune id of the region's boxes: both boxes of a 4x4 block carry the same one (`region_tune`).
    pub tune: u32,
}

/// How many tunes a box can carry: `tune_count` of data/music_box.json when present (optional key), else 4.
pub fn tune_count() -> u32 {
    static COUNT: OnceLock<u32> = OnceLock::new();
    *COUNT.get_or_init(|| {
        embedded::data_under("music_box.json")
            .next()
            .and_then(|x| serde_json::from_str::<Value>(x.1).ok())
            .and_then(|j| j["tune_count"].as_u64())
            .map(|n| n.clamp(1, 64) as u32)
            .unwrap_or(4)
    })
}

/// The tune shared by every music box of the region (4x4 block) holding chunk `c` on its level. Deterministic by
/// seed; its own RNG key, so the placement draws are untouched.
pub fn region_tune(world: &str, c: Coord) -> u32 {
    let region_x = c.x.div_euclid(REGION_SIZE_CHUNKS);
    let region_z = c.z.div_euclid(REGION_SIZE_CHUNKS);
    let mut rng = Rng::new(&format!("{world}:MUSICTUNE:{region_x}:{}:{region_z}", c.y));
    rng.next_index(tune_count() as usize) as u32
}

/// `music_box_chunk` (streamed_route_generator.gd:5471-5482): whether this chunk holds one of its region's music boxes.
/// `world` is the world seed text; the region's own seed picks the chunks, so every chunk of the region agrees.
pub fn music_box_chunk(world: &str, c: Coord) -> bool {
    let region_x = c.x.div_euclid(REGION_SIZE_CHUNKS);
    let region_z = c.z.div_euclid(REGION_SIZE_CHUNKS);
    let mut rng = Rng::new(&format!("{world}:MUSICBOX:{region_x}:{}:{region_z}", c.y));
    let mut slots: Vec<i32> = (0..REGION_SIZE_CHUNKS * REGION_SIZE_CHUNKS).collect();
    let local = (c.x - region_x * REGION_SIZE_CHUNKS) + (c.z - region_z * REGION_SIZE_CHUNKS) * REGION_SIZE_CHUNKS;
    for _ in 0..boxes_per_region() {
        let pick = rng.next_index(slots.len());
        if slots[pick] == local {
            return true;
        }
        slots.remove(pick);
    }
    false
}

/// `_music_boxes` (streamed_route_generator.gd:5497-5565) for a level-0 chunk. `grid` is the chunk's final grid,
/// `region_props` the props after the post passes, `fixtures` the chunk's fixtures, `pickups` the pickups before
/// `ensure_pickups_reachable`, and `rooms` the chunk's rooms. `safe` is the safe-room flag.
pub fn chunk_music_boxes(
    world: &str,
    c: Coord,
    grid: &Grid,
    region_props: &[Prop],
    fixtures: &[Fixture],
    pickups: &[Pickup],
    rooms: &[Room],
    safe: bool,
) -> Vec<MusicBox> {
    if safe || !music_box_chunk(world, c) {
        return Vec::new();
    }
    let tag = props::chunk_tag(c);
    let mut rng = Rng::new(&format!("{world}:MUSICBOX-CELL:{tag}"));
    let box_id = format!("{tag}:music_box");
    let tune = region_tune(world, c);
    // `_stair_door_exclusion(stairs)` with no stairs: empty.
    let stair_doors: HashSet<V2> = HashSet::new();
    // 5504-5506: the cells of the fixtures and the pickups (the table skips them, the floor keeps off them).
    let item_cells: Vec<V2> = fixtures
        .iter()
        .map(|f| f.cell)
        .chain(pickups.iter().map(|p| p.cell))
        .collect();
    let item_set: HashSet<V2> = item_cells.iter().copied().collect();
    if let Some(table) = table_prop(grid, region_props, &stair_doors, &item_set, &mut rng) {
        // 5510-5513: the table's placement and mount offsets, its surface type and the tabletop height.
        let placement = table.placement_offset.unwrap_or([0.0, 0.0]);
        let mount = table.mount_offset.unwrap_or([0.0, 0.0]);
        return vec![MusicBox {
            id: box_id,
            cell: table.cell,
            offset: Some([placement[0] + mount[0], placement[1] + mount[1]]),
            on: Some(table.prop_type),
            tune,
            y: surface_height(table.prop_type) + MUSIC_BOX_TABLE_LIFT,
        }];
    }
    // 5514-5520: the furniture (prop cells and their occupied cells).
    let mut furniture: HashSet<V2> = HashSet::new();
    for prop in region_props {
        furniture.insert(prop.cell);
        furniture.extend(prop.occupied_cells.iter().copied());
    }
    // 5521-5522: radius 1 round each fixture and pickup.
    let mut excluded: HashSet<V2> = stair_doors.clone();
    for cell in &item_cells {
        exclude_neighbourhood(&mut excluded, *cell, 1);
    }
    // 5523-5547: room floor inside the walls, off the doorways, not furniture; a cell that has a wall or furniture
    // beside it goes to the against list with that side.
    let mut against: Vec<(V2, V2)> = Vec::new();
    let mut interior: Vec<V2> = Vec::new();
    let mut interior_set: HashSet<V2> = HashSet::new();
    for room in rooms {
        let rect = room.rect;
        if rect.w < 3 || rect.h < 3 || room.room_type == "stairwell" {
            continue;
        }
        let mut door_clear: HashSet<V2> = HashSet::new();
        for door in room.doorway.iter().chain(room.extra_doorways.iter().flatten()) {
            exclude_neighbourhood(&mut door_clear, *door, 1);
        }
        // `rect.grow(-1)`: one cell in from each side, end-exclusive.
        let (x0, z0) = (rect.x + 1, rect.y + 1);
        let (x1, z1) = (rect.x + rect.w - 1, rect.y + rect.h - 1);
        for z in z0.max(1)..z1.min(CHUNK - 1) {
            for x in x0.max(1)..x1.min(CHUNK - 1) {
                let cell = V2::new(x, z);
                if grid[z as usize][x as usize] != 0
                    || excluded.contains(&cell)
                    || door_clear.contains(&cell)
                    || furniture.contains(&cell)
                    || interior_set.contains(&cell)
                {
                    continue;
                }
                interior_set.insert(cell);
                interior.push(cell);
                // LEFT, RIGHT, UP, DOWN: the first side that is wall or furniture.
                for side in [V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)] {
                    let at = cell.add(side);
                    if grid[at.y as usize][at.x as usize] != 0 || furniture.contains(&at) {
                        against.push((cell, side));
                        break;
                    }
                }
            }
        }
    }
    // 5548-5551: against a wall or furniture, nudged toward it.
    if !against.is_empty() {
        let (cell, side) = against[rng.next_index(against.len())];
        return vec![MusicBox {
            id: box_id,
            cell,
            offset: Some([side.x as f32 * MUSIC_BOX_WALL_NUDGE, side.y as f32 * MUSIC_BOX_WALL_NUDGE]),
            on: None,
            tune,
            y: MUSIC_BOX_FLOOR_Y,
        }];
    }
    // 5552-5553: any room interior cell.
    if !interior.is_empty() {
        let cell = interior[rng.next_index(interior.len())];
        return vec![MusicBox { id: box_id, cell, offset: None, on: None, tune, y: MUSIC_BOX_FLOOR_Y }];
    }
    // 5554-5565: no room interior in this chunk. Open floor clear of props, fixtures and pickups.
    for prop in region_props {
        exclude_neighbourhood(&mut excluded, prop.cell, 1);
    }
    let mut candidates: Vec<V2> = Vec::new();
    for z in 1..CHUNK - 1 {
        for x in 1..CHUNK - 1 {
            let cell = V2::new(x, z);
            if grid[z as usize][x as usize] == 0 && !excluded.contains(&cell) && open_spawn_neighbourhood(grid, cell) {
                candidates.push(cell);
            }
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    let cell = candidates[rng.next_index(candidates.len())];
    vec![MusicBox { id: box_id, cell, offset: None, on: None, tune, y: MUSIC_BOX_FLOOR_Y }]
}

/// `_music_box_table` (streamed_route_generator.gd:5570-5591): a free tabletop in a room. A solid surface parent with a
/// room id, inside the chunk, off the stair doors, not under a fixture or pickup, and not already carrying dressing.
/// Non-dining tables first; a dining table only when no other is free. One draw when there is a table.
fn table_prop<'a>(
    grid: &Grid,
    region_props: &'a [Prop],
    stair_doors: &HashSet<V2>,
    item_cells: &HashSet<V2>,
    rng: &mut Rng,
) -> Option<&'a Prop> {
    // 5572-5573: dressing that sits on a surface (a surface prop with a mount height).
    let dressing: Vec<&Prop> = region_props
        .iter()
        .filter(|e| is_surface_prop(e.prop_type) && e.mount_y > 0.0)
        .collect();
    let mut tables: Vec<&'a Prop> = Vec::new();
    let mut dining: Vec<&'a Prop> = Vec::new();
    for prop in region_props {
        let kind = prop.prop_type;
        if !has_surface_height(kind) || prop.room_id.is_empty() || !prop.solid {
            continue;
        }
        let cell = prop.cell;
        if cell.x < 1
            || cell.y < 1
            || cell.x >= CHUNK - 1
            || cell.y >= CHUNK - 1
            || grid[cell.y as usize][cell.x as usize] != 0
            || stair_doors.contains(&cell)
            || item_cells.contains(&cell)
        {
            continue;
        }
        if dressing.iter().any(|child| surface_parent_covers(prop, child)) {
            continue;
        }
        if kind == "diningTable" {
            dining.push(prop);
        } else {
            tables.push(prop);
        }
    }
    if tables.is_empty() {
        tables = dining;
    }
    if tables.is_empty() {
        return None;
    }
    Some(tables[rng.next_index(tables.len())])
}

/// The surface parents of data/rooms/catalog.json: `SURFACE_HEIGHTS` (the base heights, whose keys are the table's
/// membership) and the hunter scale of each parent (`PropModelCatalog.hunter_scale`).
///
/// `surface_heights` holds one object per parent with its `base` (the SURFACE_HEIGHTS value) and a `value` that is not
/// the Godot height (woodenTable: value 1.185, base 0.75 at scale 1.5, while the golden's box sits at 1.125 + lift).
/// So the height is `base * hunter_scale`, never `value`.
struct SurfaceTable {
    base: HashMap<String, f64>,
    scale: HashMap<String, f64>,
}

fn surface_table() -> &'static SurfaceTable {
    static TABLE: OnceLock<SurfaceTable> = OnceLock::new();
    TABLE.get_or_init(|| {
        let text = embedded::data_under("rooms/catalog.json")
            .next()
            .map(|x| x.1)
            .unwrap_or("{}");
        let j: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let base = j["surface_heights"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v["base"].as_f64().or_else(|| v.as_f64()).map(|n| (k.clone(), n)))
                    .collect()
            })
            .unwrap_or_default();
        let scale = j["hunter_scale_for_surface_parents"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| v.as_f64().map(|n| (k.clone(), n))).collect())
            .unwrap_or_default();
        SurfaceTable { base, scale }
    })
}

/// `RoomCatalog.SURFACE_HEIGHTS.has(type)`.
fn has_surface_height(kind: &str) -> bool {
    surface_table().base.contains_key(kind)
}

/// `RoomCatalog.surface_height` (room_catalog.gd:666): `SURFACE_HEIGHTS.get(type, 0.8) * PropModelCatalog.hunter_scale(type)`.
pub fn surface_height(kind: &str) -> f64 {
    let table = surface_table();
    table.base.get(kind).copied().unwrap_or(0.8) * table.scale.get(kind).copied().unwrap_or(1.0)
}

/// `_exclude_neighbourhood` (streamed_route_generator.gd:5600-5604): the square of `radius` round `cell`.
fn exclude_neighbourhood(excluded: &mut HashSet<V2>, cell: V2, radius: i32) {
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            excluded.insert(V2::new(cell.x + dx, cell.y + dz));
        }
    }
}

/// `_open_spawn_neighbourhood` (streamed_route_generator.gd:5706-5713): the cell and its four neighbours are open
/// and inside the chunk.
fn open_spawn_neighbourhood(grid: &Grid, cell: V2) -> bool {
    [V2::new(0, 0), V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)]
        .iter()
        .all(|offset| {
            let at = cell.add(*offset);
            at.x >= 0 && at.y >= 0 && at.x < CHUNK && at.y < CHUNK && grid[at.y as usize][at.x as usize] == 0
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_heights_are_the_base_times_the_hunter_scale() {
        // woodenTable: SURFACE_HEIGHTS .75 with hunter scale 1.5 (catalog.json, the golden's table boxes at 1.135)
        assert!((surface_height("woodenTable") - 1.125).abs() < 1e-12);
        assert!((surface_height("labBench") - 0.92).abs() < 1e-12);
        assert!(has_surface_height("diningTable"));
        assert!(!has_surface_height("singleBed"));
        // an unknown parent falls back to 0.8 at scale 1 (not reached from the table branch)
        assert!((surface_height("nopeTable") - 0.8).abs() < 1e-12);
    }

    #[test]
    fn music_box_slots_follow_the_region() {
        // every chunk of a region agrees on the set of two chunks; each region of the same level picks its own
        let world = "1426541270";
        let region_chunks = |rx: i32, rz: i32| -> Vec<Coord> {
            let mut out = Vec::new();
            for z in rz * REGION_SIZE_CHUNKS..(rz + 1) * REGION_SIZE_CHUNKS {
                for x in rx * REGION_SIZE_CHUNKS..(rx + 1) * REGION_SIZE_CHUNKS {
                    out.push(Coord::new(x, 0, z));
                }
            }
            out
        };
        for (rx, rz) in [(0, 0), (-1, 2), (3, -2)] {
            let hits = region_chunks(rx, rz).iter().filter(|c| music_box_chunk(world, **c)).count();
            assert_eq!(hits, boxes_per_region(), "region ({rx}, {rz})");
        }
    }

    #[test]
    fn each_block_has_exactly_two_box_chunks_over_many_seeds() {
        for seed in 0..60 {
            let world = format!("seed-{seed}");
            for (rx, rz, y) in [(0, 0, 0), (-3, 2, 0), (5, -4, 1)] {
                let hits = (0..REGION_SIZE_CHUNKS * REGION_SIZE_CHUNKS)
                    .filter(|i| {
                        let x = rx * REGION_SIZE_CHUNKS + i % REGION_SIZE_CHUNKS;
                        let z = rz * REGION_SIZE_CHUNKS + i / REGION_SIZE_CHUNKS;
                        music_box_chunk(&world, Coord::new(x, y, z))
                    })
                    .count();
                assert_eq!(hits, 2, "{world} block ({rx},{rz}) level {y}");
            }
        }
    }

    #[test]
    fn boxes_of_a_block_share_one_tune_and_it_is_deterministic() {
        let mut seen = HashSet::new();
        for seed in 0..60 {
            let world = format!("seed-{seed}");
            let first = region_tune(&world, Coord::new(0, 0, 0));
            for i in 0..REGION_SIZE_CHUNKS * REGION_SIZE_CHUNKS {
                let c = Coord::new(i % REGION_SIZE_CHUNKS, 0, i / REGION_SIZE_CHUNKS);
                assert_eq!(region_tune(&world, c), first, "{world} chunk {i}");
            }
            assert!(first < tune_count());
            seen.insert(first);
        }
        assert!(tune_count() == 1 || seen.len() > 1, "seeds should vary the tune");
    }

    #[test]
    fn the_wall_nudge_is_a_quarter_cell() {
        assert_eq!(MUSIC_BOX_WALL_NUDGE, 0.25);
        assert!((MUSIC_BOX_FLOOR_Y - 0.04).abs() < 1e-12);
        assert!((MUSIC_BOX_TABLE_LIFT - 0.01).abs() < 1e-12);
    }
}

/// The parity checks of `music_box_chunk` and `chunk_music_boxes` against the Godot goldens. The committed compact
/// extracts `tests/parity/fields-music-<tag>.json` hold the level-0 chunks of each golden; the full goldens
/// `golden-<tag>.json` in PARITY_DIR are checked by the ignored `music_boxes_match_godot_goldens`.
#[cfg(test)]
mod parity_tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    const TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];
    /// Tolerance for the f32 and f64 numbers (the goldens print about six digits).
    const TOLERANCE: f64 = 1e-5;

    /// The level-0 chunks of one golden: the world seed, and each chunk's coord with its Godot music box entries.
    struct Golden {
        world: String,
        chunks: Vec<(Coord, Vec<Value>)>,
    }

    fn coord_of(v: &Value) -> Coord {
        let a = v.as_array().expect("coord");
        Coord::new(
            a[0].as_i64().unwrap() as i32,
            a[1].as_i64().unwrap() as i32,
            a[2].as_i64().unwrap() as i32,
        )
    }

    fn cell_of(v: &Value) -> V2 {
        let a = v.as_array().expect("cell");
        V2::new(a[0].as_i64().unwrap() as i32, a[1].as_i64().unwrap() as i32)
    }

    fn load_compact(tag: &str) -> Golden {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("parity")
            .join(format!("fields-music-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let v: Value = serde_json::from_str(&text).expect("music extract json");
        Golden {
            world: v["world_seed"].as_str().expect("world_seed").to_string(),
            chunks: v["chunks"]
                .as_array()
                .expect("chunks")
                .iter()
                .map(|c| (coord_of(&c["coord"]), c["music_boxes"].as_array().expect("music_boxes").clone()))
                .collect(),
        }
    }

    /// The full Godot golden `golden-<tag>.json` in PARITY_DIR (required). Only its level-0 chunks are read.
    fn load_full(tag: &str) -> Golden {
        let dir = std::env::var("PARITY_DIR").expect("PARITY_DIR must point at the Godot goldens");
        let path = PathBuf::from(dir).join(format!("golden-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let v: Value = serde_json::from_str(&text).expect("golden json");
        let chunks = v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .filter(|c| coord_of(&c["coord"]).y == 0)
            .map(|c| (coord_of(&c["coord"]), c["data"]["music_boxes"].as_array().expect("music_boxes").clone()))
            .collect();
        Golden { world: v["world_seed"].as_str().expect("world_seed").to_string(), chunks }
    }

    /// The first field that differs between our music box and the Godot entry, or `None` when they match.
    fn box_difference(ours: &MusicBox, want: &Value) -> Option<String> {
        if want["id"].as_str() != Some(ours.id.as_str()) {
            return Some(format!("id {} vs Godot {}", ours.id, want["id"]));
        }
        if cell_of(&want["cell"]) != ours.cell {
            return Some(format!("cell {:?} vs Godot {}", (ours.cell.x, ours.cell.y), want["cell"]));
        }
        let want_y = want["y"].as_f64().unwrap_or(f64::NAN);
        if (ours.y - want_y).abs() > TOLERANCE {
            return Some(format!("y {} vs Godot {}", ours.y, want_y));
        }
        match (ours.offset, want.get("offset")) {
            (None, None) => {}
            (None, Some(_)) => return Some(format!("offset in Godot {}, none made here", want["offset"])),
            (Some(_), None) => return Some("offset made here, none in Godot".to_string()),
            (Some(o), Some(w)) => {
                let wanted = w.as_array().expect("offset");
                let ok = (o[0] as f64 - wanted[0].as_f64().unwrap()).abs() <= TOLERANCE
                    && (o[1] as f64 - wanted[1].as_f64().unwrap()).abs() <= TOLERANCE;
                if !ok {
                    return Some(format!("offset {:?} vs Godot {}", o, w));
                }
            }
        }
        match (ours.on, want.get("on")) {
            (None, None) => None,
            (Some(a), Some(b)) if b.as_str() == Some(a) => None,
            (a, b) => Some(format!("on {:?} vs Godot {:?}", a, b)),
        }
    }

    /// Our music boxes of one chunk against the Godot list, by id. Returns the differences and how many matched.
    fn compare_chunk(ours: &[MusicBox], wanted: &[Value]) -> (Vec<String>, usize) {
        let by_id: HashMap<&str, &MusicBox> = ours.iter().map(|b| (b.id.as_str(), b)).collect();
        let mut diffs = Vec::new();
        let mut matched = 0;
        for want in wanted {
            let id = want["id"].as_str().unwrap_or("");
            match by_id.get(id) {
                Some(b) => match box_difference(b, want) {
                    None => matched += 1,
                    Some(why) => diffs.push(format!("{id}: {why}")),
                },
                None => diffs.push(format!("{id}: not made (Godot at {})", want["cell"])),
            }
        }
        for b in ours {
            if !wanted.iter().any(|w| w["id"].as_str() == Some(b.id.as_str())) {
                diffs.push(format!("{}: made here, not in Godot", b.id));
            }
        }
        (diffs, matched)
    }

    fn check(tags: &[&str], load: fn(&str) -> Golden) {
        let mut report = String::new();
        let mut failures: Vec<String> = Vec::new();
        for tag in tags {
            let golden = load(tag);
            let (mut chunks, mut matched_chunks, mut boxes_made) = (0usize, 0usize, 0usize);
            let (mut entries, mut matched_entries) = (0usize, 0usize);
            for (c, wanted) in &golden.chunks {
                chunks += 1;
                entries += wanted.len();
                let Some(layout) = crate::world::route::layout::chunk_layout_godot(&golden.world, *c) else {
                    failures.push(format!("{tag} [{},{},{}]: no layout", c.x, c.y, c.z));
                    continue;
                };
                boxes_made += layout.music_boxes.len();
                let (diffs, matched) = compare_chunk(&layout.music_boxes, wanted);
                matched_entries += matched;
                if diffs.is_empty() {
                    matched_chunks += 1;
                }
                for diff in diffs {
                    failures.push(format!("{tag} [{},{},{}] {diff}", c.x, c.y, c.z));
                }
            }
            report.push_str(&format!(
                "music boxes, seed {tag} (world {}): level-0 chunks {chunks}, matched {matched_chunks}; \
                 Godot entries {entries}, matched {matched_entries}; made here {boxes_made}\n",
                golden.world
            ));
        }
        eprintln!("{report}");
        for line in failures.iter().take(25) {
            eprintln!("MISMATCH {line}");
        }
        assert!(
            failures.is_empty(),
            "{} music box differences from Godot; first: {}",
            failures.len(),
            failures.first().cloned().unwrap_or_default()
        );
    }

    #[test]
    fn music_boxes_match_godot_compact_extracts() {
        crate::world::route::layout::with_godot_regions(|| {
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
        let _godot = crate::world::route::furnished::godot_layout();
        check(&TAGS, load_compact);
            });
    }

    #[test]
    #[ignore = "full Godot goldens: set PARITY_DIR to the golden directory and run with --ignored"]
    fn music_boxes_match_godot_goldens() {
        check(&TAGS, load_full);
    }
}
