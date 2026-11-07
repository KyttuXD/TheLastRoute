//! Stage 3d-iii part 2: the hunter spawns of a sewer chunk (levels -1 and -2), a port of
//! `SewerRouteGenerator._hunter_spawns` (game/world/generation/sewer_route_generator.gd:388-464), with the helpers it reaches:
//! - `is_safe_room` (streamed_route_generator.gd:1403, the private route). It is copied from `layout.rs` `Gen::is_safe_room`,
//!   which is private to that module; the same tag `SAFE:x:y:z` and the same chance (0.055 times `safe_room_bias`).
//! - `difficulty_for_level(level).danger` (streamed_route_generator.gd:1290-1293).
//! - `HunterRoster.sewer_affinity` (hunter_roster.gd:199-208, `SEWER_PREFER` and `SEWER_AVOID`) and
//!   `HunterRoster.office_void_for` (hunter_roster.gd:285-287, `OFFICE_VOID_IDS` 138-141). These have no Rust home yet.
//! - `STREAMED_HUNTER_IDS` and `hunter_id` (streamed_route_generator.gd:499-503 and 1535) are reused from
//!   `hunter_spawns.rs`: the sewer's spawn id is `surface.hunter_id(coord, index)`, the streamed chunk tag with `:HUNTER:n`.
//!
//! The inputs Godot reads from the chunk's own schema (`data`), by key:
//! - `sealed_cells`: each cell excluded exactly.
//! - `breakables`, `extras.cells`: radius 2 (pipes and seals alike).
//! - `links`, `from_cell` and `to_cell`: radius 2.
//! - `ramps`: the `top`, `bottom` and `ramp` rectangles grown by 2, and the link's `from_cell` and `to_cell` at radius 2.
//! - `stairwells` (the maze wells that reach the chunk): each `box` grown by 2.
//! - `falls` (when there are falls): `overlook` and `void` grown by 1.
//! - `features`, `region_props`, `pickups` and `fixtures`: each entry's `cell` at radius 1, and its `occupied_cells` and
//!   `cells` at radius 1. A crawl pipe's `behind` cells are excluded exactly (only the raw features carry `behind`; it is
//!   `SewerChunk.behind`).
//! The pickups are not ported yet (stage 3d-iv). The caller passes their cells in `pickup_cells`.
//!
//! RNG: `"<world>:SEWER-HUNTERS:<sewer chunk id>"`, in this order:
//! 1. one `next_float`, compared with `lerpf(0.16, 0.42, danger)`, before the candidate check (so an empty pool still spends it);
//! 2. the candidate list (no draws): the cells 3..=41 with a dry 5-cell neighbourhood (`DRY` only, not the bridge), not excluded;
//! 3. `next_float` for the extra second spawn at level -6 and below (not reached at -1 or -2, ported for fidelity);
//! 4. per spawn: `next_index(candidates)` for the cell, `next_index(identities)` for the identity, and at level -2 and
//!    below one `next_float < 0.015` office void roll. The void identity comes from `office_void_for("<world>:SEWER:x:y:z:<n>")`.
//!    After each spawn, candidates within distance 6.0 of it are dropped (no draw).

use std::collections::HashSet;

use crate::world::rng::Rng;
use crate::world::route::danger::DangerProfile;
use crate::world::route::fixtures::Fixture;
use crate::world::route::hunter_spawns::{hunter_id, HunterSpawn, STREAMED_HUNTER_IDS};
use crate::world::route::layout::{Coord, CHUNK, V2};

use super::breach::Extras;
use super::core::{chunk_id, is_chunk_allowed, Rect, SewerChunk, DRY};

/// `HunterRoster.SEWER_PREFER` (hunter_roster.gd:199): the hunters who go below.
pub const SEWER_PREFER: [&str; 6] = ["verrine", "umbriel", "wynelle", "glyra", "loxelle", "ravelle"];
/// `HunterRoster.SEWER_AVOID` (hunter_roster.gd:200): the hunters who will not go below.
pub const SEWER_AVOID: [&str; 4] = ["kathy", "ferryn", "solyth", "porcelle"];
/// `HunterRoster.OFFICE_VOID_IDS` (hunter_roster.gd:138-141).
pub const OFFICE_VOID_IDS: [&str; 10] = [
    "void_red", "void_blue", "void_yellow", "void_green", "void_pink", "void_orange", "void_purple", "void_white", "void_gray",
    "void_black",
];

/// The surface level the sewer's spawns read the safe room of (sewer:SURFACE_LEVEL 0).
const SURFACE_LEVEL: i32 = 0;

/// `HunterRoster.sewer_affinity` (hunter_roster.gd:203-208): "prefer", "avoid" or "follow".
pub fn sewer_affinity(id: &str) -> &'static str {
    if SEWER_PREFER.contains(&id) {
        return "prefer";
    }
    if SEWER_AVOID.contains(&id) {
        return "avoid";
    }
    "follow"
}

/// `HunterRoster.office_void_for` (hunter_roster.gd:285-287): one seeded pick from `OFFICE_VOID_IDS`.
pub fn office_void_for(seed_text: &str) -> &'static str {
    let mut rng = Rng::new(&format!("{seed_text}:OFFICE-VOID"));
    OFFICE_VOID_IDS[rng.next_index(OFFICE_VOID_IDS.len())]
}

/// `difficulty_for_level(level).danger` (streamed_route_generator.gd:1293): `clampf(0.48 - level * 0.075, 0.12, 1.0)`.
fn danger_for_level(level: i32) -> f64 {
    DangerProfile::for_level(level).danger
}

/// `is_safe_room` (streamed_route_generator.gd:1403) on the private route. The origin is always safe.
pub fn is_safe_room(world: &str, c: Coord) -> bool {
    if c == Coord::new(0, 0, 0) {
        return true;
    }
    let mut rng = Rng::new(&format!("{world}:SAFE:{}:{}:{}", c.x, c.y, c.z));
    // 0.055 * safe_room_bias, safe_room_bias = lerpf(1.45, 0.55, danger)
    rng.next_float() < DangerProfile::for_level(c.y).safe_room_chance()
}

/// `_hunter_spawns` (sewer_route_generator.gd:388-464) for one sewer chunk. `fixtures` is the chunk's `_fixtures`
/// (`fixtures::fixtures`), `pickup_cells` the cells of its pickups (`_pickups`, not ported yet).
pub fn chunk_hunter_spawns(world: &str, chunk: &SewerChunk, fixtures: &[Fixture], pickup_cells: &[V2]) -> Vec<HunterSpawn> {
    let mut result = base_hunter_spawns(world, chunk, fixtures, pickup_cells);
    if let Some(room) = &chunk.room {
        let coord = chunk.coord;
        // the same exits as the base pass: the arrival column and a safe surface room hold no hunters
        if (coord.x.abs() > 1 || coord.z.abs() > 1) && !is_safe_room(world, Coord::new(coord.x, SURFACE_LEVEL, coord.z)) {
            add_room_spawns(world, chunk, room, &mut result);
        }
    }
    result
}

/// The static roster id for a name in a room's prefer list (`SEWER_PREFER` first, then the streamed roster).
fn roster_id(name: &str) -> Option<&'static str> {
    SEWER_PREFER.iter().chain(STREAMED_HUNTER_IDS.iter()).find(|id| **id == name).copied()
}

/// The room's hunter cells (`Placed::hunter_spawns`, chance lerped by the level's danger) as spawns. The identity is
/// drawn from the room's prefer list minus its avoid list, and by `sewer_affinity` never from the hunters who avoid
/// the sewer; with no usable preference it falls back to the roster the base pass uses.
fn add_room_spawns(world: &str, chunk: &SewerChunk, room: &super::rooms::Placed<'_>, out: &mut Vec<HunterSpawn>) {
    let coord = chunk.coord;
    let sewer_id = chunk_id(coord);
    let danger = danger_for_level(coord.y);
    for (n, (cell, prefer, avoid)) in room.hunter_spawns(world, &sewer_id, danger).into_iter().enumerate() {
        let mut pool: Vec<&'static str> = prefer
            .iter()
            .filter(|p| !avoid.contains(p))
            .filter_map(|p| roster_id(p))
            .filter(|id| sewer_affinity(id) != "avoid")
            .collect();
        if pool.is_empty() {
            pool = STREAMED_HUNTER_IDS
                .iter()
                .copied()
                .filter(|id| sewer_affinity(id) != "avoid" && !avoid.iter().any(|a| a == id))
                .collect();
        }
        if pool.is_empty() || out.iter().any(|s| s.cell == cell) {
            continue;
        }
        let mut rng = Rng::new(&format!("{world}:SEWER-ROOMS:{sewer_id}:IDENTITY:{n}"));
        let identity = pool[rng.next_index(pool.len())];
        let index = out.len();
        out.push(HunterSpawn { id: hunter_id(coord, index), hunter_id: identity, cell, level: coord.y, origin_chunk: coord });
    }
}

fn base_hunter_spawns(world: &str, chunk: &SewerChunk, fixtures: &[Fixture], pickup_cells: &[V2]) -> Vec<HunterSpawn> {
    let coord = chunk.coord;
    // 390-395: not a sewer chunk, the four chunks round the arrival column, a safe surface room: no spawns, no draw.
    if !is_chunk_allowed(coord) {
        return Vec::new();
    }
    if coord.x.abs() <= 1 && coord.z.abs() <= 1 {
        return Vec::new();
    }
    if is_safe_room(world, Coord::new(coord.x, SURFACE_LEVEL, coord.z)) {
        return Vec::new();
    }
    let danger = danger_for_level(coord.y);
    let sewer_id = chunk_id(coord);
    let mut rng = Rng::new(&format!("{world}:SEWER-HUNTERS:{sewer_id}"));
    // 397-398: lerpf(0.16, 0.42, danger), drawn before the candidate check.
    if rng.next_float() >= 0.16 + (0.42 - 0.16) * danger {
        return Vec::new();
    }
    let excluded = excluded_cells(chunk, fixtures, pickup_cells);
    // 429-441: open cells 3..CHUNK-3 with a dry five-cell neighbourhood (DRY only), row by row.
    let mut candidates: Vec<V2> = Vec::new();
    for z in 3..CHUNK - 3 {
        for x in 3..CHUNK - 3 {
            let cell = V2::new(x, z);
            if !excluded.contains(&cell) && dry_neighbourhood(chunk, cell) {
                candidates.push(cell);
            }
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    // 445-447: the identities that do not avoid the sewer.
    let identities: Vec<&'static str> =
        STREAMED_HUNTER_IDS.iter().copied().filter(|id| sewer_affinity(id) != "avoid").collect();
    // 449-451: the extra spawn (level -6 and below) is not reached at -1 or -2.
    let mut count = 1;
    if coord.y <= -6 && rng.next_float() < 0.22f64.min(danger * 0.18) {
        count = 2;
    }
    let mut result: Vec<HunterSpawn> = Vec::new();
    for index in 0..count {
        if candidates.is_empty() {
            break;
        }
        let cell = candidates.remove(rng.next_index(candidates.len()));
        let mut identity = identities[rng.next_index(identities.len())];
        // the office void roll (sewer:458): level -2 and below, 1.5% per spawn
        if coord.y <= -2 && rng.next_float() < 0.015 {
            identity = office_void_for(&format!("{world}:{sewer_id}:{index}"));
        }
        result.push(HunterSpawn { id: hunter_id(coord, index), hunter_id: identity, cell, level: coord.y, origin_chunk: coord });
        // no other candidate within 6.0 of a placed hunter (no draw)
        candidates.retain(|other| distance(*other, cell) >= 6.0);
    }
    result
}

/// The cells no hunter may stand on: the exclusions of `_hunter_spawns` (sewer:400-428).
fn excluded_cells(chunk: &SewerChunk, fixtures: &[Fixture], pickup_cells: &[V2]) -> HashSet<V2> {
    let mut excluded: HashSet<V2> = HashSet::new();
    for cell in &chunk.sealed_cells {
        excluded.insert(*cell);
    }
    for spec in &chunk.breakables {
        let cells = match &spec.extras {
            Extras::Seal(s) => &s.cells,
            Extras::Pipe(p) => &p.cells,
        };
        for cell in cells {
            exclude_neighbourhood(&mut excluded, *cell, 2);
        }
    }
    for link in &chunk.links {
        exclude_neighbourhood(&mut excluded, link.to_cell, 2);
        exclude_neighbourhood(&mut excluded, link.from_cell, 2);
    }
    for ramp in &chunk.ramps {
        for rect in [ramp.top, ramp.bottom, ramp.ramp] {
            exclude_rect(&mut excluded, rect.grow(2));
        }
        exclude_neighbourhood(&mut excluded, ramp.link.from_cell, 2);
        exclude_neighbourhood(&mut excluded, ramp.link.to_cell, 2);
    }
    for well in &chunk.wells {
        exclude_rect(&mut excluded, well.boxed.grow(2));
    }
    if let Some(falls) = &chunk.falls {
        exclude_rect(&mut excluded, falls.overlook.grow(1));
        exclude_rect(&mut excluded, falls.void.grow(1));
    }
    for prop in &chunk.region_props {
        exclude_neighbourhood(&mut excluded, prop.cell, 1);
        for cell in prop.occupied_cells.iter().flatten() {
            exclude_neighbourhood(&mut excluded, *cell, 1);
        }
    }
    for cell in &chunk.behind {
        excluded.insert(*cell);
    }
    for fixture in fixtures {
        exclude_neighbourhood(&mut excluded, fixture.cell, 1);
    }
    for cell in pickup_cells {
        exclude_neighbourhood(&mut excluded, *cell, 1);
    }
    excluded
}

/// The candidate test of `_hunter_spawns` (sewer:436-440): the cell and its four neighbours are all DRY.
fn dry_neighbourhood(chunk: &SewerChunk, cell: V2) -> bool {
    [V2::new(0, 0), V2::new(-1, 0), V2::new(1, 0), V2::new(0, -1), V2::new(0, 1)].iter().all(|offset| {
        let at = cell.add(*offset);
        at.x >= 0 && at.y >= 0 && at.x < CHUNK && at.y < CHUNK && chunk.sewer_grid[at.y as usize][at.x as usize] == DRY
    })
}

/// `_exclude_neighbourhood` (streamed_route_generator.gd:5600-5604): the square of `radius` round `cell`.
fn exclude_neighbourhood(set: &mut HashSet<V2>, cell: V2, radius: i32) {
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            set.insert(V2::new(cell.x + dx, cell.y + dz));
        }
    }
}

/// `_exclude_rect` (sewer_route_generator.gd:467-471): every cell of the rectangle.
fn exclude_rect(set: &mut HashSet<V2>, rect: Rect) {
    for z in rect.y..rect.end().y {
        for x in rect.x..rect.end().x {
            set.insert(V2::new(x, z));
        }
    }
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

    /// A played chunk with a room can add that room's hunters: on room floor, never an avoider, ids unique and the
    /// base spawns come first and unchanged.
    #[test]
    fn played_chunks_add_their_rooms_hunters() {
        use crate::world::route::sewer::core::{chunk as played, Inputs};
        let mut added = 0;
        for x in -12..12 {
            for z in -12..12 {
                let Some(c) = played(Coord::new(x, -2, z), &Inputs { world: "rooms-hunt" }) else { continue };
                let Some(room) = &c.room else { continue };
                let all = chunk_hunter_spawns("rooms-hunt", &c, &[], &[]);
                let base = base_hunter_spawns("rooms-hunt", &c, &[], &[]);
                assert_eq!(&all[..base.len()], &base[..]);
                let mut ids: Vec<&String> = all.iter().map(|s| &s.id).collect();
                ids.sort();
                ids.dedup();
                assert_eq!(ids.len(), all.len());
                for s in &all[base.len()..] {
                    added += 1;
                    assert_ne!(sewer_affinity(s.hunter_id), "avoid");
                    assert_eq!(room.def.code_at(s.cell.x - room.origin.x, s.cell.y - room.origin.y), Some(DRY));
                }
            }
        }
        assert!(added > 0, "no room hunters in the scanned area");
    }
    use crate::world::route::sewer::core::{chunk_base as chunk, Inputs};
    use serde_json::Value;

    const TAGS: [&str; 3] = ["parity", "k7qxm", "demo"];

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

    /// The base-45 alphabet of the seal extracts (`B45` in seals.rs).
    const B45: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHI";

    /// Cells packed as two base-45 symbols each (x, then y).
    fn unpack_cells(packed: &str) -> Vec<V2> {
        let b = packed.as_bytes();
        assert_eq!(b.len() % 2, 0, "base-45 pairs");
        let value = |ch: u8| B45.iter().position(|c| *c == ch).expect("base-45 symbol") as i32;
        b.chunks(2).map(|p| V2::new(value(p[0]), value(p[1]))).collect()
    }

    /// One golden chunk of the sewer levels: its coordinate, the pickup cells and the crawl `behind` cells (golden
    /// raw features, both sides), and the golden `hunter_spawns`.
    struct Golden {
        coord: Coord,
        pickup_cells: Vec<V2>,
        behind: Vec<V2>,
        spawns: Vec<Value>,
    }

    /// One seed's sewer chunks (levels -1 and -2).
    struct Seed {
        tag: &'static str,
        world: String,
        goldens: Vec<Golden>,
    }

    /// The sewer chunks of a dump's `chunks` (levels -1 and -2), with the pickup cells and the crawl `behind` cells
    /// that `pickups` and `behind` return for each chunk.
    fn golden_list(chunks: &Value, pickups: fn(&Value) -> Vec<V2>, behind: fn(&Value) -> Vec<V2>) -> Vec<Golden> {
        chunks
            .as_array()
            .expect("chunks")
            .iter()
            .filter(|c| c["coord"][1].as_i64().is_some_and(|y| y == -1 || y == -2))
            .map(|c| Golden {
                coord: coord_of(&c["coord"]),
                pickup_cells: pickups(c),
                behind: behind(c),
                spawns: c["data"]["hunter_spawns"].as_array().expect("hunter_spawns").clone(),
            })
            .collect()
    }

    /// The compact extract `tests/parity/sewer-hunters-<tag>.json`: `pickup_cells` as arrays, `behind_b45` packed.
    fn load_compact(tag: &'static str) -> Seed {
        let v = read_json(&compact_dir().join(format!("sewer-hunters-{tag}.json")));
        let goldens = golden_list(
            &v["chunks"],
            |c| c["pickup_cells"].as_array().expect("pickup_cells").iter().map(cell_of).collect(),
            |c| unpack_cells(c["behind_b45"].as_str().expect("behind_b45")),
        );
        Seed { tag, world: v["world_seed"].as_str().expect("world_seed").to_string(), goldens }
    }

    /// The full dump `golden-<tag>.json` in `dir`: pickups from the golden `pickups`, `behind` from the golden raw
    /// features (both sides), levels -1 and -2 only.
    fn load_full(tag: &'static str, dir: &str) -> Seed {
        let v = read_json(&std::path::Path::new(dir).join(format!("golden-{tag}.json")));
        let goldens = golden_list(
            &v["chunks"],
            |c| c["data"]["pickups"].as_array().expect("pickups").iter().map(|p| cell_of(&p["cell"])).collect(),
            |c| {
                c["data"]["features"]
                    .as_array()
                    .expect("features")
                    .iter()
                    .filter_map(|f| f.get("behind").and_then(Value::as_array))
                    .flatten()
                    .map(cell_of)
                    .collect()
            },
        );
        Seed { tag, world: v["world_seed"].as_str().expect("world_seed").to_string(), goldens }
    }

    fn spawn_difference(ours: &HunterSpawn, want: &Value) -> Option<String> {
        if want["id"].as_str() != Some(ours.id.as_str()) {
            return Some(format!("id {} vs {}", ours.id, want["id"]));
        }
        if want["hunter_id"].as_str() != Some(ours.hunter_id) {
            return Some(format!("hunter_id {} vs {}", ours.hunter_id, want["hunter_id"]));
        }
        if cell_of(&want["cell"]) != ours.cell {
            return Some(format!("cell {:?} vs {}", ours.cell, want["cell"]));
        }
        if want["level"].as_i64() != Some(ours.level as i64) {
            return Some(format!("level {} vs {}", ours.level, want["level"]));
        }
        if coord_of(&want["origin_chunk"]) != ours.origin_chunk {
            return Some(format!("origin_chunk {:?} vs {}", ours.origin_chunk, want["origin_chunk"]));
        }
        None
    }

    /// Checks one seed: the port's chunk, fixtures and pickup cells in, the golden `hunter_spawns` out. The crawl
    /// `behind` list must equal the golden's too (`SewerChunk.behind`).
    fn check_seed(seed: &Seed) -> (Vec<String>, usize, usize) {
        let mut mismatches = Vec::new();
        let mut spawns_checked = 0;
        for g in &seed.goldens {
            let c = chunk(g.coord, &Inputs { world: &seed.world }).expect("sewer chunk");
            if c.behind != g.behind {
                mismatches.push(format!("{} {:?}: behind {} cells vs {}", seed.tag, g.coord, c.behind.len(), g.behind.len()));
            }
            let fixtures = crate::world::route::sewer::fixtures::fixtures(&seed.world, &c);
            let ours = chunk_hunter_spawns(&seed.world, &c, &fixtures, &g.pickup_cells);
            spawns_checked += g.spawns.len();
            if ours.len() != g.spawns.len() {
                mismatches.push(format!("{} {:?}: {} hunter spawns vs {}", seed.tag, g.coord, ours.len(), g.spawns.len()));
                continue;
            }
            for (i, (o, w)) in ours.iter().zip(&g.spawns).enumerate() {
                if let Some(d) = spawn_difference(o, w) {
                    mismatches.push(format!("{} {:?} spawn {i}: {d}", seed.tag, g.coord));
                }
            }
        }
        (mismatches, seed.goldens.len(), spawns_checked)
    }

    fn check_all(seeds: &[Seed]) {
        let mut failures: Vec<String> = Vec::new();
        for seed in seeds {
            let (mismatches, chunks, spawns) = check_seed(seed);
            eprintln!("hunters {}: {chunks} chunks, {spawns} golden spawns, {} mismatches", seed.tag, mismatches.len());
            failures.extend(mismatches);
        }
        for m in failures.iter().take(20) {
            eprintln!("MISMATCH {m}");
        }
        assert!(failures.is_empty(), "{} mismatches; first: {}", failures.len(), failures.first().cloned().unwrap_or_default());
    }

    #[test]
    fn sewer_affinity_follows_the_roster_lists() {
        for id in SEWER_PREFER {
            assert_eq!(sewer_affinity(id), "prefer");
        }
        for id in SEWER_AVOID {
            assert_eq!(sewer_affinity(id), "avoid");
        }
        assert_eq!(sewer_affinity("dovelle"), "follow");
        assert_eq!(sewer_affinity("void_red"), "follow");
        // the streamed roster's avoid hunter is out of the identity list
        let identities: Vec<&str> =
            STREAMED_HUNTER_IDS.iter().copied().filter(|id| sewer_affinity(id) != "avoid").collect();
        assert!(!identities.contains(&"porcelle"));
        assert_eq!(identities.len(), STREAMED_HUNTER_IDS.len() - 1);
    }

    #[test]
    fn office_void_draws_from_the_ten_void_ids() {
        for seed in ["1426541270:SEWER:-3:-2:2", "x"] {
            let id = office_void_for(seed);
            assert!(OFFICE_VOID_IDS.contains(&id), "{id}");
            assert_eq!(id, office_void_for(seed));
        }
        assert_eq!(OFFICE_VOID_IDS.len(), 10);
    }

    #[test]
    fn danger_and_spawn_chance_follow_the_godot_formulas() {
        // difficulty_for_level(-1).danger = 0.48 + 0.075 = 0.555; (-2) = 0.63
        assert!((danger_for_level(-1) - 0.555).abs() < 1e-12);
        assert!((danger_for_level(-2) - 0.63).abs() < 1e-12);
        // lerpf(0.16, 0.42, 0.555) = 0.16 + 0.26 * 0.555 = 0.3043
        let chance = 0.16 + (0.42 - 0.16) * danger_for_level(-1);
        assert!((chance - 0.3043).abs() < 1e-12);
    }

    /// The compact extracts: every sewer chunk of levels -1 and -2 on the three seeds, field `hunter_spawns`, the
    /// `behind` cells, with the golden pickup cells as the exclusion input.
    #[test]
    fn hunter_spawns_match_godot_goldens() {
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
        let seeds: Vec<Seed> = TAGS.iter().map(|tag| load_compact(tag)).collect();
        check_all(&seeds);
    }

    /// The full goldens (PARITY_DIR). Without PARITY_DIR it returns early; the compact test checks the same field
    /// on the same chunks.
    #[test]
    fn hunter_spawns_match_full_goldens() {
        let Ok(dir) = std::env::var("PARITY_DIR") else {
            eprintln!("PARITY_DIR unset: full sewer hunter golden check skipped");
            return;
        };
        let seeds: Vec<Seed> = TAGS.iter().map(|tag| load_full(tag, &dir)).collect();
        for seed in &seeds {
            assert_eq!(seed.goldens.len(), 98, "{}: 98 sewer chunks", seed.tag);
        }
        check_all(&seeds);
    }
}
