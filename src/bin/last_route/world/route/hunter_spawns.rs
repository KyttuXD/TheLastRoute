//! Stage 3c part 3 of the Infinite route: the hunter spawns of a level-0 chunk, a port of
//! `StreamedRouteGenerator._hunter_spawns` (streamed_route_generator.gd:5635-5703), `_open_spawn_neighbourhood`
//! (5706-5713), `hunter_id` (1535) and `STREAMED_HUNTER_IDS` (499-503).
//!
//! `_build_chunk` calls `_hunter_spawns` at 1040, after the pickups and music boxes, with the region props after the
//! post passes. Stairs are empty on level 0 (stair_links, 1399), so the stair clearance is empty here. The Godot rule
//! is the stair anchor cell only, at radius 2 (5655-5659), not the full flight that fixtures and pickups use. It is
//! kept as Godot has it: `stairs` is passed through, anchors only.
//!
//! RNG: `"<world>:HUNTERS:<chunk>"`.
//! - The spawn roll: one `next_float` per non-safe chunk, except the primary chunk (1, 0, 0), which skips it. It is
//!   drawn before the candidate check, so an empty pool still spends it. At level 0 the chance is
//!   `lerpf(0.16, 0.42, danger)` with danger 0.48 (difficulty_for_level, 1290-1299), which is 0.2848.
//! - Per hunter: `next_index(candidates)` for the cell, then `next_index(14)` for the identity from
//!   `STREAMED_HUNTER_IDS`. The primary chunk places two hunters, the others one.
//! - The level -6 and -2 extra rolls (5675, 5689) are below level 0 and are not ported here.
//! The candidates are the open cells in 3..=41 with the open 5-cell neighbourhood, and the cells that are not near a
//! stair anchor (radius 2) or a region prop (radius 1). After each placed hunter the candidates closer than 6.0 to
//! it are dropped, with no draw.

use std::collections::HashSet;

use super::layout::{Coord, Grid, CHUNK, V2};
use super::danger::{DangerProfile, OFFICE_VOID_CHANCE};
use super::props::{self, Prop};
use crate::world::rng::Rng;

/// `STREAMED_HUNTER_IDS` (streamed_route_generator.gd:499-503): the hunter roster the streamed route spawns, in order.
pub const STREAMED_HUNTER_IDS: [&str; 14] = [
    "dovelle", "quenelle", "elise", "xivvy", "thessine", "umbriel", "aki", "porcelle", "glyra", "haelith", "opalie",
    "ysolde", "briselle", "verrine",
];

/// One hunter spawn of a chunk: the Godot dictionary's `id`, `hunter_id`, `cell`, `level` and `origin_chunk`.
#[derive(Clone, Debug, PartialEq)]
pub struct HunterSpawn {
    pub id: String,
    pub hunter_id: &'static str,
    pub cell: V2,
    pub level: i32,
    pub origin_chunk: Coord,
}

/// `hunter_id` (streamed_route_generator.gd:1535): `"<cx>:<lvl>:<cz>:HUNTER:<n>"`.
pub fn hunter_id(c: Coord, index: usize) -> String {
    format!("{}:HUNTER:{}", props::chunk_tag(c), index)
}

/// `_hunter_spawns` (streamed_route_generator.gd:5635-5703) for a level-0 chunk. `grid` is the chunk's final grid,
/// `stairs` the stair anchor cells (always empty on level 0), `region_props` the props after the post passes, and
/// `safe` the safe-room flag.
pub fn chunk_hunter_spawns(
    world: &str,
    c: Coord,
    grid: &Grid,
    stairs: &[V2],
    region_props: &[Prop],
    safe: bool,
) -> Vec<HunterSpawn> {
    // 5642-5643: a safe room never spawns a hunter, before any draw.
    if safe {
        return Vec::new();
    }
    let profile = DangerProfile::for_level(c.y);
    let tag = props::chunk_tag(c);
    let mut rng = Rng::new(&format!("{world}:HUNTERS:{tag}"));
    // lerpf(0.16, 0.42, danger)
    let spawn_chance = crate::world::route::maze_regions::hunter_chance_at(profile.hunter_spawn_chance(), world, c.x, c.z, c.y);
    let guaranteed_primary = c == Coord::new(1, 0, 0);
    // 5652: the roll is drawn before the candidate check (an empty pool still spends it).
    if !guaranteed_primary && rng.next_float() >= spawn_chance {
        return Vec::new();
    }
    // 5654-5664: the stair anchors (radius 2), then the region props (radius 1).
    let mut excluded: HashSet<V2> = HashSet::new();
    for anchor in stairs {
        for dz in -2..=2 {
            for dx in -2..=2 {
                excluded.insert(V2::new(anchor.x + dx, anchor.y + dz));
            }
        }
    }
    for prop in region_props {
        for dz in -1..=1 {
            for dx in -1..=1 {
                excluded.insert(V2::new(prop.cell.x + dx, prop.cell.y + dz));
            }
        }
    }
    // 5665-5671: open cells in 3..CHUNK-3 with the open 5-cell neighbourhood, row by row.
    let mut candidates: Vec<V2> = Vec::new();
    for z in 3..CHUNK - 3 {
        for x in 3..CHUNK - 3 {
            let cell = V2::new(x, z);
            if grid[z as usize][x as usize] == 0 && !excluded.contains(&cell) && open_spawn_neighbourhood(grid, cell) {
                candidates.push(cell);
            }
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    // 5674-5676: the primary chunk places two hunters; at level -6 and below any chunk may roll a second one.
    let mut count = if guaranteed_primary { 2 } else { 1 };
    if let Some(pair) = profile.deep_pair_chance() {
        if rng.next_float() < pair {
            count = 2;
        }
    }
    let mut result: Vec<HunterSpawn> = Vec::new();
    for index in 0..count {
        if candidates.is_empty() {
            break;
        }
        let picked = rng.next_index(candidates.len());
        let cell = candidates.remove(picked);
        let mut identity = STREAMED_HUNTER_IDS[rng.next_index(STREAMED_HUNTER_IDS.len())];
        // 5689-5691: the office void roll, at level -2 and below (not reached at level 0).
        if profile.rolls_office_void() && rng.next_float() < OFFICE_VOID_CHANCE {
            identity = crate::world::route::sewer::hunters::office_void_for(&format!("{world}:{tag}:{index}"));
        }
        result.push(HunterSpawn {
            id: hunter_id(c, index),
            hunter_id: identity,
            cell,
            level: c.y,
            origin_chunk: c,
        });
        // 5701-5702: no candidate within 6.0 of a placed hunter (no draw).
        candidates.retain(|other| distance(*other, cell) >= 6.0);
    }
    result
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

/// `Vector2i.distance_to` as a float.
fn distance(a: V2, b: V2) -> f64 {
    let dx = (a.x - b.x) as f64;
    let dz = (a.y - b.y) as f64;
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_zero_spawn_chance_is_the_godot_value() {
        let danger = DangerProfile::for_level(0).danger;
        assert!((danger - 0.48).abs() < 1e-12);
        let chance = 0.16 + (0.42 - 0.16) * danger;
        assert!((chance - 0.2848).abs() < 1e-12);
    }

    #[test]
    fn hunter_ids_follow_the_godot_shape() {
        assert_eq!(hunter_id(Coord::new(-3, 0, 2), 1), "-3:0:2:HUNTER:1");
        assert_eq!(STREAMED_HUNTER_IDS.len(), 14);
        assert_eq!(STREAMED_HUNTER_IDS[0], "dovelle");
        assert_eq!(STREAMED_HUNTER_IDS[13], "verrine");
    }

    #[test]
    fn spacing_drops_candidates_closer_than_six() {
        assert!(distance(V2::new(0, 0), V2::new(6, 0)) >= 6.0);
        assert!(distance(V2::new(0, 0), V2::new(4, 4)) < 6.0);
        assert!(distance(V2::new(0, 0), V2::new(5, 4)) >= 6.0);
    }
}

/// The parity checks of `chunk_hunter_spawns` against the Godot goldens. The committed compact extracts
/// `tests/parity/fields-hunters-<tag>.json` hold the level-0 chunks of each golden; the full goldens
/// `golden-<tag>.json` in PARITY_DIR are checked by the ignored `hunter_spawns_match_godot_goldens`.
#[cfg(test)]
mod parity_tests {
    use super::*;
    use crate::world::route::layout::chunk_layout;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    const TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];
    /// Level-0 chunks across all TAGS that carry a maze variant and are left out of the Godot comparison.
    const EXPECTED_VARIANT_CHUNKS: usize = 28;

    /// The level-0 chunks of one golden: the world seed, and each chunk's coord with its Godot hunter entries.
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
            .join(format!("fields-hunters-{tag}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let v: Value = serde_json::from_str(&text).expect("hunters extract json");
        Golden {
            world: v["world_seed"].as_str().expect("world_seed").to_string(),
            chunks: v["chunks"]
                .as_array()
                .expect("chunks")
                .iter()
                .map(|c| (coord_of(&c["coord"]), c["hunter_spawns"].as_array().expect("hunter_spawns").clone()))
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
            .map(|c| (coord_of(&c["coord"]), c["data"]["hunter_spawns"].as_array().expect("hunter_spawns").clone()))
            .collect();
        Golden { world: v["world_seed"].as_str().expect("world_seed").to_string(), chunks }
    }

    /// The first field that differs between our spawn and the Godot entry, or `None` when they match.
    fn spawn_difference(ours: &HunterSpawn, want: &Value) -> Option<String> {
        if want["id"].as_str() != Some(ours.id.as_str()) {
            return Some(format!("id {} vs Godot {}", ours.id, want["id"]));
        }
        if want["hunter_id"].as_str() != Some(ours.hunter_id) {
            return Some(format!("hunter_id {} vs Godot {}", ours.hunter_id, want["hunter_id"]));
        }
        if cell_of(&want["cell"]) != ours.cell {
            return Some(format!("cell {:?} vs Godot {}", (ours.cell.x, ours.cell.y), want["cell"]));
        }
        if want["level"].as_i64() != Some(i64::from(ours.level)) {
            return Some(format!("level {} vs Godot {}", ours.level, want["level"]));
        }
        let origin = coord_of(&want["origin_chunk"]);
        if origin != ours.origin_chunk {
            return Some(format!("origin_chunk {:?} vs Godot {}", ours.origin_chunk, want["origin_chunk"]));
        }
        None
    }

    /// Our hunter spawns of one chunk against the Godot list, by id. Returns the differences and how many matched.
    fn compare_chunk(ours: &[HunterSpawn], wanted: &[Value]) -> (Vec<String>, usize) {
        let by_id: HashMap<&str, &HunterSpawn> = ours.iter().map(|s| (s.id.as_str(), s)).collect();
        let mut diffs = Vec::new();
        let mut matched = 0;
        for want in wanted {
            let id = want["id"].as_str().unwrap_or("");
            match by_id.get(id) {
                Some(s) => match spawn_difference(s, want) {
                    None => matched += 1,
                    Some(why) => diffs.push(format!("{id}: {why}")),
                },
                None => diffs.push(format!("{id}: not made (Godot at {})", want["cell"])),
            }
        }
        for s in ours {
            if !wanted.iter().any(|w| w["id"].as_str() == Some(s.id.as_str())) {
                diffs.push(format!("{}: made here, not in Godot", s.id));
            }
        }
        (diffs, matched)
    }

    fn check(tags: &[&str], load: fn(&str) -> Golden) {
        let mut report = String::new();
        let mut failures: Vec<String> = Vec::new();
        let mut excluded: Vec<String> = Vec::new();
        for tag in tags {
            let golden = load(tag);
            let (mut chunks, mut matched_chunks, mut made) = (0usize, 0usize, 0usize);
            let (mut entries, mut matched_entries) = (0usize, 0usize);
            for (c, wanted) in &golden.chunks {
                // A maze variant chunk is new content with its own hunter chance; Godot has no such chunk.
                if let Some(v) = crate::world::route::maze_regions::variant_at(&golden.world, c.x, c.z, c.y) {
                    excluded.push(format!("{tag} [{},{},{}] {}", c.x, c.y, c.z, v.id));
                    continue;
                }
                chunks += 1;
                entries += wanted.len();
                let Some(layout) = chunk_layout(&golden.world, *c) else {
                    failures.push(format!("{tag} [{},{},{}]: no layout", c.x, c.y, c.z));
                    continue;
                };
                made += layout.hunter_spawns.len();
                let (diffs, matched) = compare_chunk(&layout.hunter_spawns, wanted);
                matched_entries += matched;
                if diffs.is_empty() {
                    matched_chunks += 1;
                }
                for diff in diffs {
                    failures.push(format!("{tag} [{},{},{}] {diff}", c.x, c.y, c.z));
                }
            }
            report.push_str(&format!(
                "hunter spawns, seed {tag} (world {}): level-0 chunks {chunks}, matched {matched_chunks}; \
                 Godot entries {entries}, matched {matched_entries}; made here {made}\n",
                golden.world
            ));
        }
        eprintln!("{report}");
        eprintln!("EXCLUDED {} maze variant chunks: {}", excluded.len(), excluded.join("; "));
        assert_eq!(excluded.len(), EXPECTED_VARIANT_CHUNKS, "variant chunks excluded: {excluded:?}");
        for line in failures.iter().take(25) {
            eprintln!("MISMATCH {line}");
        }
        assert!(
            failures.is_empty(),
            "{} hunter spawn differences from Godot; first: {}",
            failures.len(),
            failures.first().cloned().unwrap_or_default()
        );
    }

    #[test]
    fn hunter_spawns_match_godot_compact_extracts() {
        check(&TAGS, load_compact);
    }

    #[test]
    #[ignore = "full Godot goldens: set PARITY_DIR to the golden directory and run with --ignored"]
    fn hunter_spawns_match_godot_goldens() {
        check(&TAGS, load_full);
    }
}
