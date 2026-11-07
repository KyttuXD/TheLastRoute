//! Stage 3d-iii part 1: the fixtures of a sewer chunk (levels -1 and -2), a port of `SewerRouteGenerator._fixtures`
//! (game/world/generation/sewer_route_generator.gd:636-657, tag `SEWER-FIXTURES:x:z`, 640).
//!
//! The grid it reads is `_streamed_schema`'s `grid` after the seal frame (`seals::streamed_grid` of the sewer grid, the
//! walkable-or-brick grid). The features it avoids are the region props' cells with a one-cell ring (Godot reads the raw
//! features, whose cells are the props' cells: `_feature_props` 1245-1292, and the crawl and outflow occupied cells).
//!
//! Godot's rule: each room, in order, offers its candidates (walkable, not avoided, exactly one wall side); one
//! `next_index` over them picks the spot. A room with no candidate draws nothing. The fixture is a hiding spot facing
//! the open side of its backing wall. There is no spacing rule and no vent kind (those are level 0's, fixtures.rs).
//!
//! The helpers are copies with the Godot names, because the level-0 versions are private to `fixtures.rs` and
//! `props.rs`: `_exclude_neighbourhood` (streamed_route_generator.gd:5600), `_wall_count` (5607) and
//! `_backing_wall_direction` (5618). `_room_facing_yaw` (5290) is `props::room_facing_yaw`, bit-exact by its own tests.

use std::collections::HashSet;

use crate::world::rng::Rng;
use crate::world::route::fixtures::Fixture;
use crate::world::route::layout::{Grid, V2};
use crate::world::route::props;

use super::core::{chunk_id, SewerChunk, CHUNK};
use super::seals::streamed_grid;

/// `_fixtures` (sewer:636-657) for one sewer chunk. `world` is the world seed text.
pub fn fixtures(world: &str, chunk: &SewerChunk) -> Vec<Fixture> {
    let coord = chunk.coord;
    let grid = streamed_grid(&chunk.sewer_grid);
    // 637-639: every feature's neighbourhood (radius 1) is taken.
    let mut used: HashSet<V2> = HashSet::new();
    for prop in &chunk.region_props {
        exclude_neighbourhood(&mut used, prop.cell, 1);
    }
    // 640: the draw is keyed on the chunk's x and z only (no level), as Godot writes it.
    let mut rng = Rng::new(&format!("{world}:SEWER-FIXTURES:{}:{}", coord.x, coord.z));
    let mut result: Vec<Fixture> = Vec::new();
    for room in &chunk.rooms {
        let rect = room.rect;
        let mut candidates: Vec<V2> = Vec::new();
        for z in rect.y..rect.end().y {
            for x in rect.x..rect.end().x {
                let cell = V2::new(x, z);
                if !used.contains(&cell) && grid[z as usize][x as usize] == 0 && wall_count(&grid, cell) == 1 {
                    candidates.push(cell);
                }
            }
        }
        if candidates.is_empty() {
            continue;
        }
        // `candidates[rng.next_index(candidates.size())]`: one draw, nothing removed.
        let cell = candidates[rng.next_index(candidates.len())];
        let backing = backing_wall_direction(&grid, cell);
        let open = backing.neg();
        let number = result.len();
        result.push(Fixture {
            id: format!("{}:FIXTURE:HIDING_SPOT:{}", chunk_id(coord), number),
            kind: "hiding_spot",
            cell,
            // `_room_facing_yaw(cell, cell - backing)`: the front points at the open floor.
            yaw: props::room_facing_yaw(cell, cell.add(open)),
            backing_wall: backing,
            open_direction: open,
            hide: None,
        });
    }
    result
}

/// Inside the chunk (the bounds of `_wall_count`'s `at.x < 0 or ... >= CHUNK_SIZE` test).
fn inside(at: V2) -> bool {
    at.x >= 0 && at.y >= 0 && at.x < CHUNK && at.y < CHUNK
}

/// A cell as a wall test: outside the chunk, or any non-zero code, is wall.
fn is_wall(grid: &Grid, at: V2) -> bool {
    !inside(at) || grid[at.y as usize][at.x as usize] != 0
}

/// `_exclude_neighbourhood` (streamed_route_generator.gd:5600-5604): the square of `radius` round `cell`.
fn exclude_neighbourhood(set: &mut HashSet<V2>, cell: V2, radius: i32) {
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            set.insert(V2::new(cell.x + dx, cell.y + dz));
        }
    }
}

/// `_wall_count` (streamed_route_generator.gd:5607-5615): the sides of `cell` (left, right, up, down) that are wall.
fn wall_count(grid: &Grid, cell: V2) -> i32 {
    [V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)]
        .iter()
        .filter(|side| is_wall(grid, cell.add(**side)))
        .count() as i32
}

/// `_backing_wall_direction` (streamed_route_generator.gd:5618-5625): the first wall of up, right, down, else left.
fn backing_wall_direction(grid: &Grid, cell: V2) -> V2 {
    if is_wall(grid, V2::new(cell.x, cell.y - 1)) {
        return V2::new(0, -1);
    }
    if is_wall(grid, V2::new(cell.x + 1, cell.y)) {
        return V2::new(1, 0);
    }
    if is_wall(grid, V2::new(cell.x, cell.y + 1)) {
        return V2::new(0, 1);
    }
    V2::new(-1, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::layout::Coord;
    use crate::world::route::sewer::core::{chunk_base as chunk, Inputs, N};
    use serde_json::Value;

    const TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];
    /// Godot prints a float with 15 significant digits, and yaw snaps to eighths of a turn: 1e-9 is far below any
    /// real difference (a wrong yaw is off by at least PI / 4).
    const YAW_TOLERANCE: f64 = 1e-9;

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

    /// One golden chunk: its coordinate and its `fixtures` list (the golden dictionaries).
    struct Golden {
        coord: Coord,
        fixtures: Vec<Value>,
    }

    /// One seed's sewer chunks (levels -1 and -2): the world seed text and the goldens.
    struct Seed {
        tag: &'static str,
        world: String,
        goldens: Vec<Golden>,
    }

    fn golden_list(chunks: &Value) -> Vec<Golden> {
        chunks
            .as_array()
            .expect("chunks")
            .iter()
            .filter(|c| c["coord"][1].as_i64().is_some_and(|y| y == -1 || y == -2))
            .map(|c| Golden {
                coord: coord_of(&c["coord"]),
                fixtures: c["data"]["fixtures"].as_array().expect("fixtures").clone(),
            })
            .collect()
    }

    /// The compact extract `tests/parity/sewer-fixtures-<tag>.json`.
    fn load_compact(tag: &'static str) -> Seed {
        let v = read_json(&compact_dir().join(format!("sewer-fixtures-{tag}.json")));
        Seed { tag, world: v["world_seed"].as_str().expect("world_seed").to_string(), goldens: golden_list(&v["chunks"]) }
    }

    /// The full dump `golden-<tag>.json` in `dir`, levels -1 and -2 only.
    fn load_full(tag: &'static str, dir: &str) -> Seed {
        let v = read_json(&std::path::Path::new(dir).join(format!("golden-{tag}.json")));
        Seed { tag, world: v["world_seed"].as_str().expect("world_seed").to_string(), goldens: golden_list(&v["chunks"]) }
    }

    /// The first difference between one fixture and its golden dictionary, or `None`.
    fn fixture_difference(ours: &Fixture, want: &Value) -> Option<String> {
        if want["id"].as_str() != Some(ours.id.as_str()) {
            return Some(format!("id {} vs {}", ours.id, want["id"]));
        }
        if want["kind"].as_str() != Some(ours.kind) {
            return Some(format!("kind {} vs {}", ours.kind, want["kind"]));
        }
        if cell_of(&want["cell"]) != ours.cell {
            return Some(format!("cell {:?} vs {}", ours.cell, want["cell"]));
        }
        if cell_of(&want["backing_wall"]) != ours.backing_wall {
            return Some(format!("backing_wall {:?} vs {}", ours.backing_wall, want["backing_wall"]));
        }
        if cell_of(&want["open_direction"]) != ours.open_direction {
            return Some(format!("open_direction {:?} vs {}", ours.open_direction, want["open_direction"]));
        }
        let want_yaw = want["yaw"].as_f64().expect("yaw");
        if (ours.yaw - want_yaw).abs() > YAW_TOLERANCE {
            return Some(format!("yaw {} vs {}", ours.yaw, want_yaw));
        }
        None
    }

    /// Checks one seed. The port's list must have the golden's length and the same fixtures in order.
    fn check_seed(seed: &Seed) -> (Vec<String>, usize, usize) {
        let mut mismatches = Vec::new();
        let mut fixtures_checked = 0;
        for g in &seed.goldens {
            let c = chunk(g.coord, &Inputs { world: &seed.world }).expect("sewer chunk");
            let ours = fixtures(&seed.world, &c);
            fixtures_checked += g.fixtures.len();
            if ours.len() != g.fixtures.len() {
                mismatches.push(format!("{} {:?}: {} fixtures vs {}", seed.tag, g.coord, ours.len(), g.fixtures.len()));
                continue;
            }
            for (i, (o, w)) in ours.iter().zip(&g.fixtures).enumerate() {
                if let Some(d) = fixture_difference(o, w) {
                    mismatches.push(format!("{} {:?} fixture {i}: {d}", seed.tag, g.coord));
                }
            }
        }
        (mismatches, seed.goldens.len(), fixtures_checked)
    }

    fn check_all(seeds: &[Seed]) {
        let mut failures: Vec<String> = Vec::new();
        for seed in seeds {
            let (mismatches, chunks, checked) = check_seed(seed);
            eprintln!("fixtures {}: {chunks} chunks, {checked} golden fixtures, {} mismatches", seed.tag, mismatches.len());
            failures.extend(mismatches);
        }
        for m in failures.iter().take(20) {
            eprintln!("MISMATCH {m}");
        }
        assert!(failures.is_empty(), "{} mismatches; first: {}", failures.len(), failures.first().cloned().unwrap_or_default());
    }

    #[test]
    fn wall_count_and_backing_follow_the_godot_order() {
        // a pocket with walls above and on the left of (5,5)
        let mut grid: Grid = [[0u8; N]; N];
        grid[4][5] = 1;
        grid[5][4] = 1;
        let cell = V2::new(5, 5);
        assert_eq!(wall_count(&grid, cell), 2);
        // up is the first wall in the Godot order (up, right, down, left)
        assert_eq!(backing_wall_direction(&grid, cell), V2::new(0, -1));
        // outside the chunk reads as wall: the corner (0,0) has two outside sides
        assert_eq!(wall_count(&grid, V2::new(0, 0)), 2);
        assert_eq!(backing_wall_direction(&grid, V2::new(0, 0)), V2::new(0, -1));
        // no wall at all: the default is left
        let open: Grid = [[0u8; N]; N];
        assert_eq!(backing_wall_direction(&open, V2::new(10, 10)), V2::new(-1, 0));
    }

    /// The compact extracts: every sewer chunk of levels -1 and -2 on the three seeds, field `fixtures`.
    #[test]
    fn fixtures_match_godot_goldens() {
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
        let seeds: Vec<Seed> = TAGS.iter().map(|tag| load_compact(tag)).collect();
        check_all(&seeds);
    }

    /// The full goldens (PARITY_DIR), all 98 sewer chunks of each seed. Without PARITY_DIR it returns early; the
    /// compact test checks the same field on the same chunks.
    #[test]
    fn fixtures_match_full_goldens() {
        let Ok(dir) = std::env::var("PARITY_DIR") else {
            eprintln!("PARITY_DIR unset: full sewer fixtures golden check skipped");
            return;
        };
        let seeds: Vec<Seed> = TAGS.iter().map(|tag| load_full(tag, &dir)).collect();
        for seed in &seeds {
            assert_eq!(seed.goldens.len(), 98, "{}: 98 sewer chunks", seed.tag);
        }
        check_all(&seeds);
    }
}
