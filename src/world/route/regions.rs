//! Region rules of the Infinite route: a port of StreamedRouteGenerator.region_for and the
//! helpers it uses (game/world/generation/streamed_route_generator.gd, lines 1207-1290, with
//! REGION_SIZE_CHUNKS, SPAWN_MAZE_SHARE and TOP_LEVEL_ONLY_REGIONS from lines 17, 66 and 68).
//!
//! The draws come from the same seed texts and in the same order as Godot, so a chunk's region
//! is a pure function of (world seed text, chunk x, chunk z, level). The world seed text is the
//! decimal of the room seed's djb2 (`route::world_seed`, Godot's `str(run_seed)`), not the room
//! text itself. Coordinates use Godot's names: `cx` is x, `cz` is z (the Rust ChunkKey's `cy`),
//! and `level` is y.
//!
//! Not ported, because they are disabled in the Godot build (MEZZANINE_ENABLED false, KT 118;
//! VoidRegion.ENABLED false; _dual_level_pair_active returns false): the void footprint
//! ("mezzanine"), the deck patches and the dual-level carnival. With them off, the highest
//! walkable level is always 0, so greenhouse is never allowed.

use crate::world::rng::Rng;

/// Chunks per region side (streamed_route_generator.gd:17).
pub const REGION_SIZE_CHUNKS: i32 = 4;
/// Share of seeds whose spawn region (level 0 around chunk (0, 0)) is the maze (line 68).
pub const SPAWN_MAZE_SHARE: f64 = 0.6;
/// The highest walkable level of every column (MEZZANINE_ENABLED false).
pub const HIGHEST_LEVEL: i32 = 0;

const SEWER_LEVEL: i32 = -1;
const LOWER_SEWER_LEVEL: i32 = -2;
const INFERNO_LEVEL: i32 = -3;

/// Every region name the Godot generator can return, in a fixed order (the index is the
/// value Facility::merge stores per cell).
pub const NAMES: [&str; 14] = [
    "maze",
    "mansion",
    "hotel",
    "apartment",
    "beach",
    "gothic",
    "prison",
    "carnival",
    "hospital",
    "lab",
    "greenhouse",
    "mall",
    "sewer",
    "inferno",
];

/// The level-0 choice list, duplicates kept: they are the weights (streamed_route_generator.gd:1232-1234).
const BASE_CHOICES: [&str; 16] = [
    "maze",
    "mansion",
    "hotel",
    "apartment",
    "beach",
    "gothic",
    "prison",
    "carnival",
    "mansion",
    "hotel",
    "maze",
    "hospital",
    "lab",
    "lab",
    "greenhouse",
    "mall",
];

/// Index of a region name in NAMES. Panics on a name Godot never produces.
pub fn region_index(name: &str) -> usize {
    NAMES
        .iter()
        .position(|n| *n == name)
        .unwrap_or_else(|| panic!("unknown region {name}"))
}

/// region_allowed_at: a TOP_LEVEL_ONLY_REGIONS region (greenhouse) only above level 0 on the
/// highest walkable level of its column.
pub fn region_allowed_at(region: &str, level: i32) -> bool {
    if region != "greenhouse" {
        return true;
    }
    level > 0 && level == HIGHEST_LEVEL
}

/// `_pick_region`: one draw from `choices`; a pick that is not allowed here is redrawn from the
/// allowed part of `choices`, on the same stream.
fn pick(level: i32, choices: &[&'static str], rng: &mut Rng) -> &'static str {
    let first = choices[rng.next_index(choices.len())];
    if region_allowed_at(first, level) {
        return first;
    }
    let pool: Vec<&'static str> = choices
        .iter()
        .copied()
        .filter(|c| region_allowed_at(c, level))
        .collect();
    pool[rng.next_index(pool.len())]
}

/// The region of chunk (cx, cz) on `level` in the world with seed text `world`.
pub fn region_for(world: &str, cx: i32, cz: i32, level: i32) -> &'static str {
    // the sewer layers and the Inferno are whole levels: no draw
    if level == SEWER_LEVEL || level == LOWER_SEWER_LEVEL {
        return "sewer";
    }
    if level == INFERNO_LEVEL {
        return "inferno";
    }
    // Godot floors the float quotient, which is euclidean division for a positive divisor.
    let rx = cx.div_euclid(REGION_SIZE_CHUNKS);
    let rz = cz.div_euclid(REGION_SIZE_CHUNKS);
    let mut rng = Rng::new(&format!("{world}:BIOME:{rx}:{level}:{rz}"));
    let mut choices: Vec<&'static str> = BASE_CHOICES.to_vec();
    if level == 0 {
        if rx == 0 && rz == 0 {
            // spawn block: the maze SPAWN_MAZE_SHARE of the time, else the same stream picks from
            // the non-maze choices
            let mut spawn = Rng::new(&format!("{world}:SPAWN-REGION"));
            if spawn.next_float() < SPAWN_MAZE_SHARE {
                return "maze";
            }
            let others: Vec<&'static str> =
                choices.iter().copied().filter(|c| *c != "maze").collect();
            return pick(level, &others, &mut spawn);
        }
        return pick(level, &choices, &mut rng);
    }
    // Appends Godot makes below level 0 (streamed_route_generator.gd:1245-1263), and above it.
    if level <= -1 {
        choices.extend(["gothic", "gothic"]);
    }
    if level <= -2 {
        choices.extend(["prison", "prison"]);
    }
    if level <= -3 {
        choices.extend(["inferno", "inferno"]);
        if level <= -6 {
            choices.push("inferno");
        }
    }
    if level >= 2 {
        choices.extend(["hotel", "apartment", "beach"]);
    } else if level >= 0 {
        choices.push("beach");
    }
    pick(level, &choices, &mut rng)
}

/// Godot's deprecated alias of region_for (biome_for).
pub fn biome_for(world: &str, cx: i32, cz: i32, level: i32) -> &'static str {
    region_for(world, cx, cz, level)
}

/// True for the chunks of the spawn block: level 0, chunk x and z both in 0..REGION_SIZE_CHUNKS.
pub fn is_spawn_block(cx: i32, cz: i32, level: i32) -> bool {
    level == 0 && cx.div_euclid(REGION_SIZE_CHUNKS) == 0 && cz.div_euclid(REGION_SIZE_CHUNKS) == 0
}

/// The region of the spawn chunk (0, 0) on level 0, where the run starts.
pub fn spawn_region(world: &str) -> &'static str {
    region_for(world, 0, 0, 0)
}

/// The catalogue entry of a region (data/regions/<id>.json, `lr.region/1`, or a pack's override), when
/// it has one. The placement rules above stay in code; the facts a region carries come from here.
pub fn region_def(region: &str) -> Option<&'static crate::content::regions::RegionDef> {
    crate::content::regions::table().def(region)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::world_seed;

    /// One golden seed: the room text, its world seed, and each chunk [x, level, z] with the Godot
    /// region (None when Godot's dump had no data).
    struct Golden {
        world: String,
        chunks: Vec<([i32; 3], Option<String>)>,
    }

    fn load(tag: &str) -> Golden {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("parity")
            .join(format!("regions-{tag}.json"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let v: serde_json::Value = serde_json::from_str(&text).expect("golden json");
        let room = v["seed"].as_str().expect("seed");
        let world = world_seed(room);
        assert_eq!(
            v["world_seed"].as_str(),
            Some(world.as_str()),
            "{tag}: world seed of room text {room}"
        );
        let chunks = v["chunks"]
            .as_array()
            .expect("chunks")
            .iter()
            .map(|c| {
                let a = c["coord"].as_array().expect("coord");
                let coord = [
                    a[0].as_i64().unwrap() as i32,
                    a[1].as_i64().unwrap() as i32,
                    a[2].as_i64().unwrap() as i32,
                ];
                (coord, c["region"].as_str().map(str::to_string))
            })
            .collect();
        Golden { world, chunks }
    }

    #[test]
    fn region_matches_godot_goldens() {
        let mut failures: Vec<String> = vec![];
        for tag in ["parity", "k7qxm", "demo"] {
            let g = load(tag);
            let mut per_level: std::collections::BTreeMap<i32, (usize, usize, usize)> =
                Default::default();
            for (coord, godot) in &g.chunks {
                let [x, level, z] = *coord;
                let entry = per_level.entry(level).or_default();
                let Some(godot) = godot else {
                    entry.2 += 1;
                    continue;
                };
                let rust = region_for(&g.world, x, z, level);
                if rust == godot {
                    entry.0 += 1;
                } else {
                    entry.1 += 1;
                    failures.push(format!(
                        "{tag} [x={x}, level={level}, z={z}] godot={godot} rust={rust}"
                    ));
                }
            }
            for (level, (ok, bad, skipped)) in &per_level {
                eprintln!(
                    "region parity {tag} level {level:>2}: match {ok}, mismatch {bad}, no Godot region {skipped}"
                );
            }
        }
        assert!(failures.is_empty(), "region mismatches:\n{}", failures.join("\n"));
    }

    #[test]
    fn greenhouse_is_never_drawn_on_level_zero() {
        for i in 0..200 {
            let world = format!("ROOM-TEST{i:03}-0000{i:04X}");
            for cx in -5..=5 {
                for cz in -5..=5 {
                    assert_ne!(region_for(&world, cx, cz, 0), "greenhouse");
                }
            }
        }
    }

    #[test]
    fn sewers_and_inferno_are_whole_levels() {
        assert_eq!(region_for("ROOM-X-00000001", 7, -9, -1), "sewer");
        assert_eq!(region_for("ROOM-X-00000001", -4, 3, -2), "sewer");
        assert_eq!(region_for("ROOM-X-00000001", 0, 0, -3), "inferno");
    }

    #[test]
    fn spawn_block_is_maze_or_a_non_maze_region() {
        let mut maze = 0;
        for i in 0..400 {
            let world = format!("ROOM-SPAWN{i:03}-0000{i:04X}");
            let r = spawn_region(&world);
            assert!(is_spawn_block(0, 0, 0));
            assert!(NAMES.contains(&r));
            if r == "maze" {
                maze += 1;
            }
        }
        // SPAWN_MAZE_SHARE is 0.6; the maze also comes from the 40% branch sometimes
        assert!(maze > 200, "maze share {maze}/400");
    }
}
