//! Stage 3c part 2 of the Infinite route: the pickups of a level-0 chunk, a port of `StreamedRouteGenerator._pickups`
//! (streamed_route_generator.gd:5393-5466), `_ensure_pickups_reachable` (737-787) and `pickup_id` (1528), with the
//! helpers they reach.
//!
//! `_build_chunk` calls `_pickups` after the fixtures (1038), with the sealed cells empty on level 0 (1032-1037 skip
//! the sealed wing). `chunk()` calls `_ensure_pickups_reachable` last (809), after the music boxes and hunters. That
//! function reads only the pickups, the grid and the solid region props, so the music boxes and hunters are not an
//! input to it here.
//!
//! RNG (level 0, sealed empty, stairs empty, coop false):
//! - `"<world>:PICKUPS:<chunk>"`: `roll` (one `next_float` per roll entry), then per planned pickup one `next_index` over
//!   the candidates and the item draw (`pick_tool` or `pick_backpack`).
//! - `"<world>:PICKUPS-AID:<chunk>"`: `roll_aid` (one `next_float` per aid entry), then one `next_index` per placed
//!   aid item, on the candidates left over. Starved on a crowded chunk.
//! The bonus relay roll (5427) and the guaranteed tool (5429) short-circuit on the empty sealed set and draw nothing.
//!
//! Helpers: `_stair_door_exclusion` (5177, a copy of the private fixtures helper, with stairs always empty here),
//! `_exclude_neighbourhood` (5600, radius 1 round the props and the fixtures), `_open_spawn_neighbourhood` (5706).

use std::collections::{HashSet, VecDeque};

use super::fixtures::Fixture;
use super::layout::{Coord, Grid, CHUNK, V2};
use super::loot;
use super::props::{self, Prop};
use crate::world::rng::Rng;

/// `STAIR_RUN_CELLS` (streamed_route_generator.gd:62), as in fixtures.rs.
const STAIR_RUN_CELLS: i32 = 8;
/// `CHUNK_SIZE / 2`, the chunk's centre row and column.
const CENTER: i32 = CHUNK / 2;

/// One pickup of a chunk: the Godot dictionary's `id`, `cell`, `kind` and `item`.
#[derive(Clone, Debug, PartialEq)]
pub struct Pickup {
    pub id: String,
    pub cell: V2,
    /// "relay", "battery", "tool", "coin" or "backpack".
    pub kind: &'static str,
    /// The tool or backpack name, or the aid item (a tool); "" for relays, batteries and coins.
    pub item: &'static str,
}

/// `pickup_id` (streamed_route_generator.gd:1528): `"<cx>:<lvl>:<cz>:PICKUP:<KIND>:<n>"`.
pub fn pickup_id(c: Coord, index: usize, kind: &str) -> String {
    format!("{}:PICKUP:{}:{}", props::chunk_tag(c), kind.to_uppercase(), index)
}

/// `_pickups` (streamed_route_generator.gd:5393-5466) for a level-0 chunk. `grid` is the chunk's final grid,
/// `region_props` the region props after the post passes, `fixtures` the chunk's fixtures (`fixtures::chunk_fixtures`),
/// and `safe` the safe-room flag. `stairs` is always empty on level 0 (stair_links, 1399); it is kept so the stair
/// clearance is the Godot one. The sealed wing is level -3 only and is not a parameter.
pub fn chunk_pickups(
    world: &str,
    c: Coord,
    grid: &Grid,
    region: &str,
    region_props: &[Prop],
    fixtures: &[Fixture],
    stairs: &[V2],
    safe: bool,
) -> Vec<Pickup> {
    if safe {
        return Vec::new();
    }
    let table = loot::table_for(region);
    // 5397-5403: the full stair flight, then radius 1 round every region prop and every fixture.
    let mut excluded = stair_door_exclusion(stairs);
    for prop in region_props {
        exclude_neighbourhood(&mut excluded, prop.cell, 1);
    }
    for fixture in fixtures {
        exclude_neighbourhood(&mut excluded, fixture.cell, 1);
    }
    // 5404-5410: open, not excluded, and the 5-cell open neighbourhood. Row by row from z=1, x=1 to 43.
    let mut candidates: Vec<V2> = Vec::new();
    for z in 1..CHUNK - 1 {
        for x in 1..CHUNK - 1 {
            let cell = V2::new(x, z);
            if cell_value(grid, x, z) == 0 && !excluded.contains(&cell) && open_spawn_neighbourhood(grid, cell) {
                candidates.push(cell);
            }
        }
    }
    let tag = props::chunk_tag(c);
    // 5416: the main stream. Sealed is empty, so the sealed branches draw nothing.
    let mut rng = Rng::new(&format!("{world}:PICKUPS:{tag}"));
    let mut plans: Vec<&'static str> = vec!["relay", "relay"];
    plans.extend(table.base.iter().copied());
    plans.extend(loot::roll(table, &mut rng));
    let mut result: Vec<Pickup> = Vec::new();
    let mut kind_indices: Vec<(&'static str, usize)> = Vec::new();
    for kind in plans {
        if candidates.is_empty() {
            break;
        }
        let index = index_of(&kind_indices, kind);
        let pick = rng.next_index(candidates.len());
        let cell = candidates.remove(pick);
        bump_index(&mut kind_indices, kind);
        let item = loot::item_for(kind, table, &mut rng, c.y);
        result.push(Pickup { id: pickup_id(c, index, kind), cell, kind, item });
    }
    // 5454-5464: the aid stream, on the cells the main stream left over.
    let mut aid_rng = Rng::new(&format!("{world}:PICKUPS-AID:{tag}"));
    let aid = loot::roll_aid(table, &mut aid_rng, false);
    for item in aid {
        if candidates.is_empty() {
            break;
        }
        let pick = aid_rng.next_index(candidates.len());
        let cell = candidates.remove(pick);
        let index = index_of(&kind_indices, "tool");
        bump_index(&mut kind_indices, "tool");
        result.push(Pickup { id: pickup_id(c, index, "tool"), cell, kind: "tool", item });
    }
    result
}

/// `_ensure_pickups_reachable` (streamed_route_generator.gd:737-787), in place. A flood from the chunk's border ring
/// and its centre row over open cells that no solid prop occupies (door props are skipped). A pickup the flood does
/// not reach moves to the nearest cell the flood reached. The nearest search walks every in-chunk cell, walls
/// included, so it can path through a wall. Deterministic, no draws. Pickups can end up on one cell; they are not
/// deduplicated.
pub fn ensure_pickups_reachable(grid: &Grid, region_props: &[Prop], pickups: &mut [Pickup]) {
    if pickups.is_empty() {
        return;
    }
    let size = CHUNK;
    // 740-746: the occupied cells of the solid, non-door props. Godot reads `occupied_cells` with the cell as the
    // default; a solid prop always has occupied cells in the goldens, so an empty list here stands for the default.
    let mut blocked: HashSet<V2> = HashSet::new();
    for prop in region_props {
        if !prop.solid || prop.room_door {
            continue;
        }
        if prop.occupied_cells.is_empty() {
            blocked.insert(prop.cell);
        } else {
            blocked.extend(prop.occupied_cells.iter().copied());
        }
    }
    let open = |c: V2| -> bool {
        c.x >= 0 && c.y >= 0 && c.x < size && c.y < size && cell_value(grid, c.x, c.y) == 0 && !blocked.contains(&c)
    };
    // 749-766: seeds are the border ring and the row z = size / 2 (integer division, 22), then 4-neighbour flood.
    let mut reached: HashSet<V2> = HashSet::new();
    let mut frontier: Vec<V2> = Vec::new();
    for i in 0..size {
        for c in [
            V2::new(i, 0),
            V2::new(i, size - 1),
            V2::new(0, i),
            V2::new(size - 1, i),
            V2::new(i, size / 2),
        ] {
            if open(c) && reached.insert(c) {
                frontier.push(c);
            }
        }
    }
    while let Some(cur) = frontier.pop() {
        for d in DIRS {
            let n = cur.add(d);
            if open(n) && reached.insert(n) {
                frontier.push(n);
            }
        }
    }
    if reached.is_empty() {
        return;
    }
    // 768-786: each unreached pickup moves to the nearest reached cell. Breadth-first over every in-chunk cell.
    for pickup in pickups.iter_mut() {
        if reached.contains(&pickup.cell) {
            continue;
        }
        let mut seen: HashSet<V2> = HashSet::new();
        seen.insert(pickup.cell);
        let mut queue: VecDeque<V2> = VecDeque::from([pickup.cell]);
        while let Some(cur) = queue.pop_front() {
            if reached.contains(&cur) {
                pickup.cell = cur;
                break;
            }
            for d in DIRS {
                let n = cur.add(d);
                if n.x >= 0 && n.y >= 0 && n.x < size && n.y < size && seen.insert(n) {
                    queue.push_back(n);
                }
            }
        }
    }
}

/// The 4-neighbour steps in the order of the Godot loops: (1,0), (-1,0), (0,1), (0,-1).
const DIRS: [V2; 4] = [V2::new(1, 0), V2::new(-1, 0), V2::new(0, 1), V2::new(0, -1)];

/// `kind_indices.get(kind, 0)`.
fn index_of(kind_indices: &[(&'static str, usize)], kind: &'static str) -> usize {
    kind_indices.iter().find(|(k, _)| *k == kind).map_or(0, |(_, n)| *n)
}

/// `kind_indices[kind] = index + 1`.
fn bump_index(kind_indices: &mut Vec<(&'static str, usize)>, kind: &'static str) {
    match kind_indices.iter_mut().find(|(k, _)| *k == kind) {
        Some((_, n)) => *n += 1,
        None => kind_indices.push((kind, 1)),
    }
}

/// `_stair_door_exclusion` (streamed_route_generator.gd:5177-5187): the full flight of every stair, radius 3 each.
fn stair_door_exclusion(stairs: &[V2]) -> HashSet<V2> {
    let mut excluded = HashSet::new();
    for anchor in stairs {
        for path_cell in stair_path_cells(*anchor) {
            exclude_neighbourhood(&mut excluded, path_cell, 3);
        }
    }
    excluded
}

/// `stair_path_cells` (streamed_route_generator.gd:1655-1663): the anchor and the eight cells that follow it.
fn stair_path_cells(cell: V2) -> Vec<V2> {
    let dx = CENTER - cell.x;
    let dz = CENTER - cell.y;
    let direction = if dx.abs() >= dz.abs() {
        V2::new(dx.signum(), 0)
    } else {
        V2::new(0, dz.signum())
    };
    (0..=STAIR_RUN_CELLS)
        .map(|index| V2::new(cell.x + direction.x * index, cell.y + direction.y * index))
        .collect()
}

/// `_exclude_neighbourhood` (streamed_route_generator.gd:5600-5604): the square of `radius` round `cell`.
fn exclude_neighbourhood(excluded: &mut HashSet<V2>, cell: V2, radius: i32) {
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            excluded.insert(V2::new(cell.x + dx, cell.y + dz));
        }
    }
}

/// `_open_spawn_neighbourhood` (streamed_route_generator.gd:5706-5713): the cell and its four neighbours are open.
/// The chunk bounds are checked as in Godot (a cell outside it is not open).
fn open_spawn_neighbourhood(grid: &Grid, cell: V2) -> bool {
    [V2::new(0, 0), V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)]
        .iter()
        .all(|offset| {
            let at = cell.add(*offset);
            at.x >= 0 && at.y >= 0 && at.x < CHUNK && at.y < CHUNK && cell_value(grid, at.x, at.y) == 0
        })
}

/// A grid value; the caller keeps the coordinates in the chunk.
fn cell_value(grid: &Grid, x: i32, z: i32) -> u8 {
    grid[z as usize][x as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pickup_ids_follow_the_godot_shape() {
        assert_eq!(pickup_id(Coord::new(-3, 0, -3), 0, "relay"), "-3:0:-3:PICKUP:RELAY:0");
        assert_eq!(pickup_id(Coord::new(1, 0, 0), 4, "backpack"), "1:0:0:PICKUP:BACKPACK:4");
    }

    #[test]
    fn ensure_moves_a_sealed_pickup_to_the_nearest_reached_cell() {
        // Open floor with a solid ring round x 19..=25, z 7..=13. The open pocket inside (20..=24, 8..=12) touches
        // neither the border nor the centre row z = 22, so the flood does not reach it.
        let mut grid: Grid = [[1u8; CHUNK as usize]; CHUNK as usize];
        for z in 1..(CHUNK - 1) as usize {
            for x in 1..(CHUNK - 1) as usize {
                grid[z][x] = 0;
            }
        }
        for z in 7..=13usize {
            for x in 19..=25usize {
                if z == 7 || z == 13 || x == 19 || x == 25 {
                    grid[z][x] = 1;
                }
            }
        }
        let inside = V2::new(22, 10);
        let mut pickups = vec![Pickup {
            id: "t".into(),
            cell: inside,
            kind: "battery",
            item: "",
        }];
        ensure_pickups_reachable(&grid, &[], &mut pickups);
        // the flood reaches the open cells outside the ring, so the pickup moves out to a reached cell
        let moved = pickups[0].cell;
        assert_ne!(moved, inside);
        assert!(!(19..=25).contains(&moved.x) || !(7..=13).contains(&moved.y), "{moved:?} is inside the ring");
    }

    #[test]
    fn ensure_leaves_reached_pickups_and_does_nothing_without_pickups() {
        let grid: Grid = [[0u8; CHUNK as usize]; CHUNK as usize];
        let mut pickups = vec![Pickup { id: "a".into(), cell: V2::new(10, 10), kind: "coin", item: "" }];
        ensure_pickups_reachable(&grid, &[], &mut pickups);
        assert_eq!(pickups[0].cell, V2::new(10, 10));
        let mut none: Vec<Pickup> = Vec::new();
        ensure_pickups_reachable(&grid, &[], &mut none);
        assert!(none.is_empty());
    }

    #[test]
    fn stair_flight_and_neighbourhoods_match_the_godot_helpers() {
        // radius 1 is a 3x3 square (9 cells); radius 3 is 7x7
        let mut set = HashSet::new();
        exclude_neighbourhood(&mut set, V2::new(5, 5), 1);
        assert_eq!(set.len(), 9);
        let mut wide = HashSet::new();
        exclude_neighbourhood(&mut wide, V2::new(5, 5), 3);
        assert_eq!(wide.len(), 49);
        assert_eq!(stair_path_cells(V2::new(0, CENTER)).len(), 9);
        assert!(stair_door_exclusion(&[]).is_empty());
    }
}

/// The parity checks of `chunk_pickups` against the Godot goldens. `pickups_match_godot_compact_extracts` reads the
/// committed extracts `tests/parity/fields-pickups-<tag>.json` (the level-0 chunks of each golden) and compares by
/// id: kind, cell and item, and that no pickup is made here that Godot did not make. The full goldens
/// `golden-<tag>.json` in PARITY_DIR are checked by the ignored `pickups_match_godot_goldens`.
#[cfg(test)]
mod parity_tests {
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    const TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];

    /// The level-0 chunks of one golden: the world seed, and each chunk's coord with its Godot pickup entries.
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
            .join(format!("fields-pickups-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let v: Value = serde_json::from_str(&text).expect("pickups extract json");
        Golden {
            world: v["world_seed"].as_str().expect("world_seed").to_string(),
            chunks: v["chunks"]
                .as_array()
                .expect("chunks")
                .iter()
                .map(|c| (coord_of(&c["coord"]), c["pickups"].as_array().expect("pickups").clone()))
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
            .map(|c| (coord_of(&c["coord"]), c["data"]["pickups"].as_array().expect("pickups").clone()))
            .filter(|(c, _)| c.y == 0)
            .collect();
        Golden { world: v["world_seed"].as_str().expect("world_seed").to_string(), chunks }
    }

    /// The first field of our pickup that differs from the Godot entry, or `None` when they match.
    fn pickup_difference(ours: &Pickup, want: &Value) -> Option<String> {
        if want["kind"].as_str() != Some(ours.kind) {
            return Some(format!("kind {} vs Godot {}", ours.kind, want["kind"]));
        }
        if cell_of(&want["cell"]) != ours.cell {
            return Some(format!("cell {:?} vs Godot {}", (ours.cell.x, ours.cell.y), want["cell"]));
        }
        let want_item = want["item"].as_str().unwrap_or("");
        if want_item != ours.item {
            return Some(format!("item {:?} vs Godot {:?}", ours.item, want_item));
        }
        None
    }

    /// Our pickups of one chunk against the Godot list, by id. Returns the differences (empty when the chunk
    /// matches) and how many Godot entries matched.
    fn compare_chunk(ours: &[Pickup], wanted: &[Value]) -> (Vec<String>, usize) {
        let by_id: HashMap<&str, &Pickup> = ours.iter().map(|p| (p.id.as_str(), p)).collect();
        let mut diffs = Vec::new();
        let mut matched = 0;
        for want in wanted {
            let id = want["id"].as_str().unwrap_or("");
            match by_id.get(id) {
                Some(p) => match pickup_difference(p, want) {
                    None => matched += 1,
                    Some(why) => diffs.push(format!("{id}: {why}")),
                },
                None => diffs.push(format!("{id}: not made (Godot {} at {})", want["kind"], want["cell"])),
            }
        }
        let wanted_ids: HashSet<&str> = wanted.iter().filter_map(|w| w["id"].as_str()).collect();
        for p in ours {
            if !wanted_ids.contains(p.id.as_str()) {
                diffs.push(format!("{}: made here, not in Godot", p.id));
            }
        }
        (diffs, matched)
    }

    fn check(tags: &[&str], load: fn(&str) -> Golden) {
        let mut report = String::new();
        let mut failures: Vec<String> = Vec::new();
        for tag in tags {
            let golden = load(tag);
            let (mut chunks, mut matched_chunks, mut no_layout) = (0usize, 0usize, 0usize);
            let (mut entries, mut matched_entries, mut made) = (0usize, 0usize, 0usize);
            for (c, wanted) in &golden.chunks {
                chunks += 1;
                entries += wanted.len();
                let Some(layout) = crate::world::route::layout::chunk_layout_godot(&golden.world, *c) else {
                    no_layout += 1;
                    failures.push(format!("{tag} [{},{},{}]: no layout", c.x, c.y, c.z));
                    continue;
                };
                made += layout.pickups.len();
                let (diffs, matched) = compare_chunk(&layout.pickups, wanted);
                matched_entries += matched;
                if diffs.is_empty() {
                    matched_chunks += 1;
                }
                for diff in diffs {
                    failures.push(format!("{tag} [{},{},{}] {diff}", c.x, c.y, c.z));
                }
            }
            report.push_str(&format!(
                "seed {tag} (world {}): level-0 chunks {chunks}, matched {matched_chunks}, not laid out {no_layout}; \
                 Godot pickups {entries}, matched {matched_entries}; made here {made}\n",
                golden.world
            ));
        }
        eprintln!("{report}");
        for line in failures.iter().take(25) {
            eprintln!("MISMATCH {line}");
        }
        assert!(
            failures.is_empty(),
            "{} pickup differences from Godot; first: {}",
            failures.len(),
            failures.first().cloned().unwrap_or_default()
        );
    }

    #[test]
    fn pickups_match_godot_compact_extracts() {
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
    fn pickups_match_godot_goldens() {
        check(&TAGS, load_full);
    }
}
