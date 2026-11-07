//! Stage 3d-iv: the pickups of a sewer chunk (levels -1 and -2), a port of `SewerRouteGenerator._pickups`
//! (game/world/generation/sewer_route_generator.gd:662-722, tags `SEWER-PICKUPS:x:z` and `SEWER-PICKUPS-AID:x:z`).
//! Godot line numbers below are that file's.
//!
//! The inputs Godot reads, and where they come from here:
//! - `sewer_grid` (the codes after the seal frame): `SewerChunk.sewer_grid`.
//! - `grid` (the streamed walk grid of `_streamed_schema`, 0 where walkable): `seals::streamed_grid`.
//! - `features` (every raw feature, 665-668): one cell ring (radius 1) round each region prop, which is the same set of
//!   cells as the raw feature cells (see `fixtures.rs`), plus each crawl pipe's `behind` cells, exactly.
//! - `links` (the ladders, 669-670): a radius-2 ring round each link's `to_cell`.
//! - `gate_cells` (the seal specs' cells, `seal_gates` in `chunk` before the pipes, sewer:540-544): a radius-1 ring
//!   round every seal's cells. Only the seal specs, not the pipes, which Godot prepends after this step.
//! - `sealed` (`sealed_cells`, sewer:541): the sealed wing's cells.
//!
//! The fixtures are NOT an exclusion. Godot's `_pickups` takes `features`, not `fixtures` (signature 662-663), and the
//! fixtures are placed in the same schema but never passed here.
//!
//! `surface.coop` (714) is a plain `coop: bool` input. The goldens were made with it false.
//!
//! RNG, in the Godot order (`SeededRandom.new("<world>:SEWER-PICKUPS:<x>:<z>")`, which has no level in it, so levels -1
//! and -2 at the same column share one stream text; that is the Godot seed):
//! 1. `next_float` for the extra relay chance (693): always drawn, the `or` comes after it.
//! 2. The table's chance rolls, one `next_float` each (`loot::roll`, 695).
//! 3. Per planned pickup, in plan order (700-711): a second-relay sealed draw (`next_float`, only for relay index 1 with
//!    sealed candidates), one `next_index` for the cell, and the item draw (`loot::item_for`: a tool draws `pick_tool`).
//! 4. The aid stream `SEWER-PICKUPS-AID` (713-721, `loot::roll_aid`, empty for the sewer table, so no draw), then per
//!    item one `next_index` on the open candidates, else the sealed ones.
//!
//! Helpers: `_exclude_neighbourhood` (streamed_route_generator.gd:5600) and `_open_spawn_neighbourhood` (5706) are
//! copies with the Godot names, because the level-0 versions are private to `pickups.rs`. `pickup_id` (1528) is
//! `route::pickups::pickup_id`. The loot tables are `route::loot` (`table_for("sewer")`); pack overrides are not read.

use std::collections::{HashMap, HashSet};

use crate::world::rng::Rng;
use crate::world::route::layout::{Grid, V2};
use crate::world::route::loot;
use crate::world::route::pickups::{pickup_id, Pickup};

use super::breach::Extras;
use super::core::{SewerChunk, CHUNK, DRY};
use super::seals::{self, SECOND_RELAY_SEALED};

/// `SEWER_EXTRA_RELAY_CHANCE` (sewer:27): how often a sewer chunk holds a second relay (always when it has a sealed room).
pub const SEWER_EXTRA_RELAY_CHANCE: f64 = 0.55;

/// `_pickups` (sewer:662-722) for one sewer chunk. `world` is the world seed text, `coop` is `surface.coop` (714).
pub fn chunk_pickups(world: &str, chunk: &SewerChunk, coop: bool) -> Vec<Pickup> {
    let mut result = base_pickups(world, chunk, coop);
    if let Some(room) = &chunk.room {
        append_room_loot(world, chunk, room, &mut result);
    }
    result
}

/// The tool items a room's loot table may name, as the static names the pickups carry.
const ROOM_TOOLS: [&str; 6] = ["taser", "lockpick", "crowbar", "rescue_axe", "stim_pack", "revive_kit"];

/// The room's loot (`Placed::loot`) as pickups: "battery" is a battery, a known tool name is a tool pickup, anything
/// else is skipped. Ids continue each kind's count after the base pickups.
fn append_room_loot(world: &str, chunk: &SewerChunk, room: &super::rooms::Placed<'_>, out: &mut Vec<Pickup>) {
    for (item, cell) in room.loot(world, &super::core::chunk_id(chunk.coord)) {
        let (kind, item): (&'static str, &'static str) = if item == "battery" {
            ("battery", "")
        } else if let Some(tool) = ROOM_TOOLS.iter().find(|t| **t == item) {
            ("tool", *tool)
        } else {
            continue;
        };
        let index = out.iter().filter(|p| p.kind == kind).count();
        out.push(Pickup { id: pickup_id(chunk.coord, index, kind), cell, kind, item });
    }
}

fn base_pickups(world: &str, chunk: &SewerChunk, coop: bool) -> Vec<Pickup> {
    let coord = chunk.coord;
    let grid = seals::streamed_grid(&chunk.sewer_grid);
    let sealed: HashSet<V2> = chunk.sealed_cells.iter().copied().collect();

    // gate_cells (sewer:540-544) and 664-670 (features, behind, links): nothing lies in a seal's gate or against it.
    let mut gate_cells: HashSet<V2> = HashSet::new();
    for spec in &chunk.breakables {
        if let Extras::Seal(seal) = &spec.extras {
            for cell in &seal.cells {
                exclude_neighbourhood(&mut gate_cells, *cell, 1);
            }
        }
    }
    let mut excluded = gate_cells;
    for prop in &chunk.region_props {
        exclude_neighbourhood(&mut excluded, prop.cell, 1);
    }
    for behind in &chunk.behind {
        excluded.insert(*behind);
    }
    for link in &chunk.links {
        exclude_neighbourhood(&mut excluded, link.to_cell, 2);
    }

    // 671-678: the dry cells of the interior, z-major, not excluded and with an open neighbourhood.
    let mut candidates: Vec<V2> = Vec::new();
    for z in 1..CHUNK - 1 {
        for x in 1..CHUNK - 1 {
            let cell = V2::new(x, z);
            if chunk.sewer_grid[z as usize][x as usize] == DRY
                && !excluded.contains(&cell)
                && open_spawn_neighbourhood(&grid, cell)
            {
                candidates.push(cell);
            }
        }
    }
    // 679-687: a sealed chunk splits its candidates; the sealed ones are the second relay's and the guaranteed tool's.
    let mut sealed_candidates: Vec<V2> = Vec::new();
    if !sealed.is_empty() {
        let mut open_candidates: Vec<V2> = Vec::new();
        for candidate in candidates {
            if sealed.contains(&candidate) {
                sealed_candidates.push(candidate);
            } else {
                open_candidates.push(candidate);
            }
        }
        candidates = open_candidates;
    }

    let mut rng = Rng::new(&format!("{}:SEWER-PICKUPS:{}:{}", world, coord.x, coord.z));
    let table = loot::table_for("sewer");
    // 692-697: the first relay and the table's base kinds, then the extra relay, the rolls and the guaranteed tool.
    let mut plans: Vec<&'static str> = std::iter::once("relay").chain(table.base.iter().copied()).collect();
    // `rng.next_float() < CHANCE or not sealed_candidates.is_empty()`: the draw always happens (it is the left side)
    let extra_roll = rng.next_float() < SEWER_EXTRA_RELAY_CHANCE;
    if extra_roll || !sealed_candidates.is_empty() {
        plans.push("relay");
    }
    plans.extend(loot::roll(table, &mut rng));
    if !plans.contains(&"tool") && !sealed.is_empty() && seals::guaranteed_tool(coord) {
        plans.push("tool");
    }

    // 698-711: each planned kind takes a cell (the second relay may take a sealed one) and an item.
    let mut result: Vec<Pickup> = Vec::new();
    let mut kind_indices: HashMap<&'static str, usize> = HashMap::new();
    for &kind in &plans {
        if candidates.is_empty() && sealed_candidates.is_empty() {
            break;
        }
        let index = kind_indices.get(&kind).copied().unwrap_or(0);
        let from_sealed = kind == "relay"
            && index >= 1
            && !sealed_candidates.is_empty()
            && (index >= 2 || rng.next_float() < SECOND_RELAY_SEALED);
        let pool = if from_sealed || candidates.is_empty() { &mut sealed_candidates } else { &mut candidates };
        let pick = rng.next_index(pool.len());
        let cell = pool.remove(pick);
        kind_indices.insert(kind, index + 1);
        let item = loot::item_for(kind, table, &mut rng, coord.y);
        result.push(Pickup { id: pickup_id(coord, index, kind), cell, kind, item });
    }

    // 713-721: aid items roll from their own stream after everything above (none in the built-in sewer table).
    let mut aid_rng = Rng::new(&format!("{}:SEWER-PICKUPS-AID:{}:{}", world, coord.x, coord.z));
    for item in loot::roll_aid(table, &mut aid_rng, coop) {
        let pool = if !candidates.is_empty() { &mut candidates } else { &mut sealed_candidates };
        if pool.is_empty() {
            break;
        }
        let pick = aid_rng.next_index(pool.len());
        let cell = pool.remove(pick);
        let index = kind_indices.get("tool").copied().unwrap_or(0);
        kind_indices.insert("tool", index + 1);
        result.push(Pickup { id: pickup_id(coord, index, "tool"), cell, kind: "tool", item });
    }
    result
}

/// `_exclude_neighbourhood` (streamed_route_generator.gd:5600): every cell in the square of this radius round `cell`.
fn exclude_neighbourhood(set: &mut HashSet<V2>, cell: V2, radius: i32) {
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            set.insert(V2::new(cell.x + dx, cell.y + dz));
        }
    }
}

/// `_open_spawn_neighbourhood` (streamed_route_generator.gd:5706): the cell and its four neighbours are all inside the
/// chunk and walkable (grid code 0).
fn open_spawn_neighbourhood(grid: &Grid, cell: V2) -> bool {
    [V2::new(0, 0), V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)].iter().all(|offset| {
        let at = cell.add(*offset);
        at.x >= 0 && at.y >= 0 && at.x < CHUNK && at.y < CHUNK && grid[at.y as usize][at.x as usize] == 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk the game plays (with a room) carries the room's loot in its pickups, on room floor, with unique ids.
    #[test]
    fn played_chunks_carry_their_rooms_loot() {
        use crate::world::route::layout::Coord;
        use crate::world::route::sewer::core::{chunk as played, Inputs};
        let mut with_loot = 0;
        for x in -8..8 {
            for z in -8..8 {
                let Some(c) = played(Coord::new(x, -1, z), &Inputs { world: "rooms-loot" }) else { continue };
                let Some(room) = &c.room else { continue };
                let all = chunk_pickups("rooms-loot", &c, false);
                let base = base_pickups("rooms-loot", &c, false);
                assert!(all.len() >= base.len());
                assert_eq!(&all[..base.len()], &base[..]);
                let mut ids: Vec<&String> = all.iter().map(|p| &p.id).collect();
                ids.sort();
                ids.dedup();
                assert_eq!(ids.len(), all.len(), "({x}, {z}) duplicate pickup id");
                for extra in &all[base.len()..] {
                    let at = (extra.cell.x - room.origin.x, extra.cell.y - room.origin.y);
                    assert_eq!(room.def.code_at(at.0, at.1), Some(DRY));
                    with_loot += 1;
                }
            }
        }
        assert!(with_loot > 0, "no room loot in the scanned area");
    }
    use crate::world::route::layout::Coord;
    use crate::world::route::sewer::core::{chunk_base as chunk, Inputs, N};
    use serde_json::Value;

    /// The golden id text `<x>:<y>:<z>:PICKUP:<suffix>` (`pickup_id` 1528 with the streamed chunk id).
    fn expected_id(coord: Coord, suffix: &str) -> String {
        format!("{}:{}:{}:PICKUP:{}", coord.x, coord.y, coord.z, suffix)
    }

    const TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];
    /// The base-45 alphabet of the compact extracts (the same as the seal and fixture extracts).
    const B45: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHI";

    /// The compact extracts live in the repo (PARITY_DIR is for the full dumps only).
    fn compact_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("parity")
    }

    fn read_json(path: &std::path::Path) -> Value {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text).expect("golden json")
    }

    fn coord_of(v: &Value) -> Coord {
        let a = v.as_array().expect("coord");
        Coord::new(a[0].as_i64().unwrap() as i32, a[1].as_i64().unwrap() as i32, a[2].as_i64().unwrap() as i32)
    }

    fn cell_of(v: &Value) -> V2 {
        let a = v.as_array().expect("cell");
        V2::new(a[0].as_i64().unwrap() as i32, a[1].as_i64().unwrap() as i32)
    }

    /// A two-symbol base-45 cell (x, then y).
    fn b45_cell(text: &str) -> V2 {
        let at = |ch: u8| B45.bytes().position(|c| c == ch).expect("base-45 symbol") as i32;
        let b = text.as_bytes();
        V2::new(at(b[0]), at(b[1]))
    }

    /// One golden pickup: its id, kind, cell and item.
    struct Want {
        id: String,
        kind: String,
        cell: V2,
        item: String,
    }

    /// One golden chunk: its coordinate and its pickups in golden order.
    struct Golden {
        coord: Coord,
        pickups: Vec<Want>,
    }

    /// One seed's sewer chunks (levels -1 and -2): the world seed text and the goldens.
    struct Seed {
        tag: &'static str,
        world: String,
        goldens: Vec<Golden>,
    }

    /// The compact extract `tests/parity/sewer-pickups-<tag>.json`: each pickup is `[suffix, cell, item]`, and the id is
    /// `<x>:<y>:<z>:PICKUP:<suffix>`; the kind is the suffix's kind in lower case.
    fn load_compact(tag: &'static str) -> Seed {
        let v = read_json(&compact_dir().join(format!("sewer-pickups-{tag}.json")));
        let goldens = v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .map(|c| {
                let coord = coord_of(&c["coord"]);
                let pickups = c["pickups"]
                    .as_array()
                    .expect("pickups")
                    .iter()
                    .map(|p| {
                        let suffix = p[0].as_str().expect("suffix");
                        let kind = suffix.split(':').next().expect("kind").to_lowercase();
                        Want {
                            id: expected_id(coord, suffix),
                            kind,
                            cell: b45_cell(p[1].as_str().expect("cell")),
                            item: p[2].as_str().expect("item").to_string(),
                        }
                    })
                    .collect();
                Golden { coord, pickups }
            })
            .collect();
        Seed { tag, world: v["world_seed"].as_str().expect("world_seed").to_string(), goldens }
    }

    /// The full dump `golden-<tag>.json` in `dir`, levels -1 and -2 only.
    fn load_full(tag: &'static str, dir: &str) -> Seed {
        let v = read_json(&std::path::Path::new(dir).join(format!("golden-{tag}.json")));
        let goldens = v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .filter(|c| c["coord"][1].as_i64().is_some_and(|y| y == -1 || y == -2))
            .map(|c| Golden {
                coord: coord_of(&c["coord"]),
                pickups: c["data"]["pickups"]
                    .as_array()
                    .expect("pickups")
                    .iter()
                    .map(|p| Want {
                        id: p["id"].as_str().expect("id").to_string(),
                        kind: p["kind"].as_str().expect("kind").to_string(),
                        cell: cell_of(&p["cell"]),
                        item: p["item"].as_str().expect("item").to_string(),
                    })
                    .collect(),
            })
            .collect();
        Seed { tag, world: v["world_seed"].as_str().expect("world_seed").to_string(), goldens }
    }

    /// The first difference between one pickup and its golden entry, or `None`.
    fn pickup_difference(ours: &Pickup, want: &Want) -> Option<String> {
        if ours.id != want.id {
            return Some(format!("id {} vs {}", ours.id, want.id));
        }
        if ours.kind != want.kind {
            return Some(format!("kind {} vs {}", ours.kind, want.kind));
        }
        if ours.cell != want.cell {
            return Some(format!("cell {:?} vs {:?}", ours.cell, want.cell));
        }
        if ours.item != want.item {
            return Some(format!("item {:?} vs {:?}", ours.item, want.item));
        }
        None
    }

    /// Checks one seed. The port's list must have the golden's length and the same pickups in order.
    fn check_seed(seed: &Seed) -> (Vec<String>, usize, usize) {
        let mut mismatches = Vec::new();
        let mut checked = 0;
        for g in &seed.goldens {
            let c = chunk(g.coord, &Inputs { world: &seed.world }).expect("sewer chunk");
            let ours = chunk_pickups(&seed.world, &c, false);
            checked += g.pickups.len();
            if ours.len() != g.pickups.len() {
                mismatches.push(format!("{} {:?}: {} pickups vs {}", seed.tag, g.coord, ours.len(), g.pickups.len()));
                continue;
            }
            for (i, (o, w)) in ours.iter().zip(&g.pickups).enumerate() {
                if let Some(d) = pickup_difference(o, w) {
                    mismatches.push(format!("{} {:?} pickup {i}: {d}", seed.tag, g.coord));
                }
            }
        }
        (mismatches, seed.goldens.len(), checked)
    }

    fn check_all(seeds: &[Seed]) {
        let mut failures: Vec<String> = Vec::new();
        for seed in seeds {
            let (mismatches, chunks, checked) = check_seed(seed);
            eprintln!("pickups {}: {chunks} chunks, {checked} golden pickups, {} mismatches", seed.tag, mismatches.len());
            failures.extend(mismatches);
        }
        for m in failures.iter().take(20) {
            eprintln!("MISMATCH {m}");
        }
        assert!(failures.is_empty(), "{} mismatches; first: {}", failures.len(), failures.first().cloned().unwrap_or_default());
    }

    #[test]
    fn exclude_neighbourhood_is_the_square_of_the_radius() {
        let mut set = HashSet::new();
        exclude_neighbourhood(&mut set, V2::new(5, 5), 1);
        assert_eq!(set.len(), 9);
        assert!(set.contains(&V2::new(4, 4)) && set.contains(&V2::new(6, 6)));
        let mut wide = HashSet::new();
        exclude_neighbourhood(&mut wide, V2::new(5, 5), 2);
        assert_eq!(wide.len(), 25);
    }

    #[test]
    fn open_spawn_neighbourhood_needs_the_cell_and_four_neighbours_walkable() {
        let mut grid: Grid = [[0u8; N]; N];
        assert!(open_spawn_neighbourhood(&grid, V2::new(10, 10)));
        // a wall to the right of (10,10) is grid[10][11]
        grid[10][11] = 1;
        assert!(!open_spawn_neighbourhood(&grid, V2::new(10, 10)));
        // the chunk's edge is outside: never open there
        let open: Grid = [[0u8; N]; N];
        assert!(!open_spawn_neighbourhood(&open, V2::new(0, 5)));
    }

    /// The compact extracts: every sewer chunk of levels -1 and -2 on the three seeds, field `pickups` (id, kind, cell,
    /// item). The chunk's own inputs come from `chunk()`, so the seal frame, the breakables and the gates are the port's.
    #[test]
    fn pickups_match_godot_goldens() {
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
        let seeds: Vec<Seed> = TAGS.iter().map(|tag| load_compact(tag)).collect();
        for seed in &seeds {
            assert_eq!(seed.goldens.len(), 98, "{}: 98 sewer chunks", seed.tag);
        }
        check_all(&seeds);
    }

    /// The full goldens (PARITY_DIR), all 98 sewer chunks of each seed. Without PARITY_DIR it returns early; the
    /// compact test checks the same field on the same chunks.
    #[test]
    fn pickups_match_full_goldens() {
        let Ok(dir) = std::env::var("PARITY_DIR") else {
            eprintln!("PARITY_DIR unset: full sewer pickups golden check skipped");
            return;
        };
        let seeds: Vec<Seed> = TAGS.iter().map(|tag| load_full(tag, &dir)).collect();
        for seed in &seeds {
            assert_eq!(seed.goldens.len(), 98, "{}: 98 sewer chunks", seed.tag);
        }
        check_all(&seeds);
    }
}
