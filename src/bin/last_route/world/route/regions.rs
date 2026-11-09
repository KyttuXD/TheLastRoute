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
pub const NAMES: [&str; 20] = [
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
    // the map-concept regions (data/region_rooms), appended so the indices above never move; never drawn by Godot's
    // draw, so BASE_CHOICES (a parity reference) does not list them
    "ice",
    "stalls",
    "inferno0",
    "tunnels",
    "offices",
    "utility",
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

/// The region of chunk (cx, cz) on `level` in the world with seed text `world`. While the concept map is enabled
/// (`concept_map::enabled`) level 0 is read from it (`concept_map`): the spawn chunk is the maze, every other chunk takes the region of the map under it, and a
/// chunk of solid rock or tunnel counts as "maze". Every other level, and level 0 with the map off, keeps Godot's draw
/// (`godot_region_for`).
pub fn region_for(world: &str, cx: i32, cz: i32, level: i32) -> &'static str {
    if level == 0 && super::concept_map::enabled() {
        return super::concept_map::region_of_chunk(world, cx, cz);
    }
    // a whole level of the levels table (data/levels.json: the sewers and the Inferno) is its region for every chunk
    if let Some(entry) = crate::world::levels::entry(level) {
        if entry.whole {
            return entry.region.as_str();
        }
    }
    godot_region_for(world, cx, cz, level)
}

/// Godot's `region_for` (the seeded draw per region block), kept as the parity reference. Levels under 0 use it
/// through `region_for`; level 0 uses it only for the Godot goldens (`layout::chunk_layout_godot`).
pub fn godot_region_for(world: &str, cx: i32, cz: i32, level: i32) -> &'static str {
    // the sewer layers and the Inferno are whole levels: no draw
    if level == SEWER_LEVEL || level == LOWER_SEWER_LEVEL {
        return "sewer";
    }
    // the early return keeps level -3 whole; the `level <= -3` extension below runs for every level under it
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

/// The region of the spawn chunk (0, 0) on level 0, where the run starts: the maze under the concept map, Godot's draw otherwise.
pub fn spawn_region(world: &str) -> &'static str {
    region_for(world, 0, 0, 0)
}

/// The catalogue entry of a region (src/bin/last_route/data/regions/<id>.json, `lr.region/1`, or a pack's override), when
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
        if crate::shared::test_support::skip_without_parity() {
            return;
        }
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
                let rust = godot_region_for(&g.world, x, z, level);
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

    /// Godot's draw never gives a greenhouse on level 0; the concept map does (its green area), on level 0 only.
    #[test]
    fn greenhouse_is_never_drawn_on_level_zero() {
        for i in 0..200 {
            let world = format!("ROOM-TEST{i:03}-0000{i:04X}");
            for cx in -5..=5 {
                for cz in -5..=5 {
                    assert_ne!(godot_region_for(&world, cx, cz, 0), "greenhouse");
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
    fn levels_below_the_inferno_run_the_inferno_extension() {
        // level -3 itself is the whole-level early return; -4 and -6 reach the appended pool
        for seed in 1..40 {
            let world = format!("ROOM-X-{seed:08}");
            assert_eq!(region_for(&world, seed, -seed, INFERNO_LEVEL), "inferno");
        }
        let mut inferno = 0;
        let mut prison = 0;
        for seed in 1..60 {
            let world = format!("ROOM-X-{seed:08}");
            for (cx, cz) in [(0, 0), (9, 9), (-9, 9), (20, -20)] {
                match region_for(&world, cx, cz, INFERNO_LEVEL - 3) {
                    "inferno" => inferno += 1,
                    "prison" => prison += 1,
                    _ => {}
                }
            }
        }
        assert!(inferno > 0 && prison > 0, "inferno {inferno}, prison {prison}");
    }

    #[test]
    fn spawn_block_is_maze_or_a_non_maze_region() {
        // with the concept map on the spawn chunk is the maze for every seed; with it off (the shipped state) it is
        // Godot's draw, which still gives the old 60/40 mix
        let mut godot_maze = 0;
        for i in 0..400 {
            let world = format!("ROOM-SPAWN{i:03}-0000{i:04X}");
            assert!(is_spawn_block(0, 0, 0));
            let g = godot_region_for(&world, 0, 0, 0);
            assert!(NAMES.contains(&g));
            assert_eq!(spawn_region(&world), g);
            super::super::concept_map::with_concept(true, || assert_eq!(spawn_region(&world), "maze"));
            if g == "maze" {
                godot_maze += 1;
            }
        }
        // SPAWN_MAZE_SHARE is 0.6; the maze also comes from the 40% branch sometimes
        assert!(godot_maze > 200, "maze share {godot_maze}/400");
    }

    #[test]
    fn with_the_concept_map_off_level_zero_is_godots_draw() {
        assert!(!super::super::concept_map::enabled());
        for i in 0..20 {
            let world = format!("OFF-{i}");
            for cx in -12..12 {
                for cz in -12..12 {
                    assert_eq!(region_for(&world, cx, cz, 0), godot_region_for(&world, cx, cz, 0), "{world} {cx},{cz}");
                }
            }
        }
    }

    #[test]
    fn level_zero_reads_the_concept_map_and_other_levels_keep_the_draw() {
        super::super::concept_map::concept_for_this_test();
        use super::super::concept_map::{chunk_class, ChunkClass};
        let world = "31337";
        let mut seen = std::collections::BTreeSet::new();
        for cx in -30..30 {
            for cz in -30..30 {
                let r = region_for(world, cx, cz, 0);
                assert!(NAMES.contains(&r));
                match chunk_class(world, cx, cz) {
                    ChunkClass::Region(name) => assert_eq!(r, name),
                    ChunkClass::Hollow => assert_eq!(r, "maze"),
                }
                seen.insert(r);
                assert_eq!(region_for(world, cx, cz, -1), "sewer");
                assert_eq!(region_for(world, cx, cz, -3), "inferno");
            }
        }
        assert!(seen.len() >= 6, "level 0 shows only {seen:?}");
        // below level 0 the answer is Godot's
        assert_eq!(region_for(world, 5, 6, -4), godot_region_for(world, 5, 6, -4));
        // same seed, same answer
        assert_eq!(region_for(world, 11, -7, 0), region_for(world, 11, -7, 0));
    }

    #[test]
    fn catalogue_regions_are_all_names_the_generator_returns() {
        let t = crate::content::regions::RegionTable::builtin(&mut Vec::new());
        assert!(!t.defs.is_empty());
        for d in &t.defs {
            // level layouts (second sewer, pipe sewer, inferno caves) are catalogue rows, never drawn as a district
            if ["sewer2", "sewer_pipes", "inferno_caves"].contains(&d.id.as_str()) {
                continue;
            }
            assert!(NAMES.contains(&d.id.as_str()), "{} is in the catalogue but not in NAMES", d.id);
        }
    }

    /// The rows the consumers read from the catalogue (layout.rs room_type_choices, props.rs prop_set) and the
    /// copies they still hold in code: furnished.rs HOSPITAL_ROOMS, LAB_ROOMS, GREENHOUSE_ROOMS and MALL_ROOMS,
    /// grid.rs ROOM_DOOR_TABLE (mall row) and render/textures/sets.rs District (hospital, mall).
    /// A change to src/bin/last_route/data/regions must change these copies as well, until the consumers read the catalogue.
    #[test]
    fn catalogue_matches_the_hard_coded_copies() {
        let t = crate::content::regions::RegionTable::builtin(&mut Vec::new());
        let hospital = t.def("hospital").unwrap();
        let mall = t.def("mall").unwrap();
        assert_eq!(
            hospital.room_roles,
            ["ward", "nurses_station", "waiting_room", "operating_theatre", "icu", "morgue", "pharmacy", "imaging", "supply_closet"]
        );
        assert_eq!(
            mall.room_roles,
            [
                "donut_shop", "cinema", "corner_store", "food_court", "bakery", "laundromat", "electronics",
                "night_diner", "drugstore", "florist", "barber", "pawn_shop", "stationery", "music_store",
            ]
        );
        // sets.rs District rows
        assert_eq!(hospital.hall_floor, ["vinyl0", "vinyl1"]);
        assert_eq!(hospital.room_default_floor, ["vinyl0", "vinyl1"]);
        assert!(!hospital.hall_runner);
        assert_eq!(mall.hall_floor, ["terrazzo0", "terrazzo1"]);
        assert_eq!(mall.room_default_floor, ["terrazzo0"]);
        assert!(!mall.hall_runner);
        // grid.rs ROOM_DOOR_TABLE, mall row
        assert_eq!(mall.room_door.as_deref(), Some("normalRoomDoor"));
        assert_eq!(mall.double_room_door.as_deref(), Some("normalDoubleRoomDoor"));
        assert_eq!(hospital.room_door, None);
        // the prop_set rows (type, size, solid), read by props.rs prop_set
        let hospital_set: Vec<(&str, [f32; 3], bool)> =
            hospital.prop_set.iter().map(|p| (p.prop_type.as_str(), p.size, p.solid)).collect();
        assert_eq!(
            hospital_set,
            vec![
                ("hospitalBed", [0.95, 1.05, 2.1], true),
                ("wheelchair", [0.65, 0.95, 0.95], true),
                ("ivDripStand", [0.45, 1.95, 0.45], true),
                ("medicalSupplyRack", [1.2, 1.9, 0.5], true),
                ("waitingChairs", [2.0, 0.86, 0.62], true),
                ("hospitalWardSign", [0.95, 0.36, 0.06], false),
                ("xrayLightbox", [1.3, 0.85, 0.08], false),
            ]
        );
        let mall_set: Vec<(&str, [f32; 3], bool)> =
            mall.prop_set.iter().map(|p| (p.prop_type.as_str(), p.size, p.solid)).collect();
        assert_eq!(
            mall_set,
            vec![
                ("mallBench", [1.8, 0.5, 0.6], true),
                ("mallPlanter", [1.3, 2.4, 1.3], true),
                ("shopShelf", [1.2, 1.9, 0.5], true),
                ("vendingMachine", [0.85, 1.9, 0.6], true),
                ("menuBoard", [1.5, 0.9, 0.08], false),
            ]
        );
    }

    #[test]
    fn region_def_reads_the_catalogue() {
        assert_eq!(region_def("mall").map(|d| d.id.as_str()), Some("mall"));
        assert_eq!(region_def("maze").map(|d| d.id.as_str()), Some("maze"));
        // inferno and office have no row: their lists stay in code
        assert!(region_def("inferno").is_none());
        assert!(region_def("office").is_none());
    }

    /// The regions whose rows moved from code into src/bin/last_route/data/regions (the ten `lr.region/1` files that are not
    /// hospital, mall or sewer).
    const MOVED_REGIONS: [&str; 10] =
        ["maze", "mansion", "hotel", "apartment", "beach", "gothic", "prison", "carnival", "lab", "greenhouse"];

    /// The prop set of each moved row is the generated table's row (prop_tables.rs REGION_PROP_SETS), entry for
    /// entry: type, size and solid, in order. The Standard values must not change.
    #[test]
    fn moved_region_prop_sets_match_the_generated_table() {
        use crate::world::route::prop_tables::{lookup, REGION_PROP_SETS};
        for id in MOVED_REGIONS {
            let def = region_def(id).unwrap_or_else(|| panic!("{id} has a row"));
            let code = lookup(REGION_PROP_SETS, id).unwrap_or_else(|| panic!("{id} has a code row"));
            let data: Vec<(&str, [f32; 3], bool)> =
                def.prop_set.iter().map(|p| (p.prop_type.as_str(), p.size, p.solid)).collect();
            let want: Vec<(&str, [f32; 3], bool)> = code.iter().map(|p| (p.prop_type, p.size, p.solid)).collect();
            assert_eq!(data, want, "{id}");
        }
    }

    /// The floors of each moved row (hall floor, runner, room default) are the district rows of render/textures
    /// (sets.rs DISTRICTS, Godot's FloorSets), which the renderer still reads from code.
    #[test]
    fn moved_region_floors_match_the_district_rows() {
        use crate::render::textures::sets::district;
        let own = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        for id in MOVED_REGIONS {
            let def = region_def(id).unwrap_or_else(|| panic!("{id} has a row"));
            let d = district(id).unwrap_or_else(|| panic!("{id} has a district row"));
            assert_eq!(def.hall_floor, own(d.hall_floor), "{id} hall floor");
            assert_eq!(def.hall_runner, d.hall_runner, "{id} hall runner");
            assert_eq!(def.room_default_floor, own(d.room_default), "{id} room default floor");
        }
    }

    #[test]
    fn a_pack_region_with_a_new_id_is_never_placed() {
        let mut t = crate::content::regions::RegionTable::builtin(&mut Vec::new());
        let mut warnings = vec![];
        let dungeon = r#"{"schema":"lr.region/1","id":"dungeon","hall_floor":["stone0"],"hall_runner":false,"room_default_floor":["stone0"],"room_roles":["cell"],"prop_set":[]}"#;
        t.add_pack("alt", &[], &[("dungeon.json".to_string(), dungeon.to_string())], &mut warnings);
        assert!(t.def("dungeon").is_some());
        assert!(warnings.iter().any(|w| w.contains("no level places it")), "{warnings:?}");
        assert!(!NAMES.contains(&"dungeon"));
        for level in [0, 1, 2, -1, -2, -3, -6] {
            for cx in -9..=9 {
                for cz in -9..=9 {
                    assert_ne!(region_for("ROOM-DUNGEON-0001", cx, cz, level), "dungeon");
                }
            }
        }
    }

    /// src/bin/last_route/data/regions/sewer.json against the constants the sewer port holds in world/route/sewer/core.rs
    /// (PUMP_COLLIDER, OUTFLOW_SIZE, ROOM_KINDS) and the district row of render/textures/sets.rs, which
    /// matches Godot's game/look/floor_sets.gd "sewer" row (stone walkways, concrete rooms, no runner).
    /// The Standard values must stay the game's current ones.
    #[test]
    fn sewer_catalogue_matches_the_sewer_constants() {
        use crate::world::route::sewer::core::{OUTFLOW_SIZE, PUMP_COLLIDER, ROOM_KINDS};
        let d = region_def("sewer").expect("src/bin/last_route/data/regions/sewer.json is built in");
        assert_eq!(d.hall_floor, ["stone0", "stone1"]);
        assert!(!d.hall_runner);
        assert_eq!(d.room_default_floor, ["concrete0", "concrete1"]);
        assert_eq!(d.room_roles, ROOM_KINDS);
        let pump = d.prop_set.iter().find(|p| p.prop_type == "sewerPump").expect("pump row");
        assert_eq!(pump.size, PUMP_COLLIDER);
        assert!(pump.solid);
        let outflow = d.prop_set.iter().find(|p| p.prop_type == "sewerOutflowPipe").expect("outflow row");
        assert_eq!(outflow.size, OUTFLOW_SIZE);
        assert!(outflow.solid);
    }

    /// The catalogue is what a consumer reads: a pack that overrides regions/sewer changes the row that
    /// region_def returns, so a data edit reaches the game without a code edit.
    #[test]
    fn a_sewer_override_changes_the_catalogue_row() {
        let mut t = crate::content::regions::RegionTable::builtin(&mut Vec::new());
        let file = r#"{"schema":"lr.region/1","id":"sewer","hall_floor":["stone0"],"hall_runner":true,"room_default_floor":["concrete0"],"room_roles":["pump_room"],"prop_set":[{"type":"sewerPump","size":[2.0,1.5,0.9]}]}"#;
        let mut warnings = vec![];
        t.add_pack(
            "alt",
            &["regions/sewer".to_string()],
            &[("sewer.json".to_string(), file.to_string())],
            &mut warnings,
        );
        let d = t.def("sewer").expect("sewer");
        assert_eq!(d.source, "alt", "{warnings:?}");
        assert_eq!(d.hall_floor, ["stone0"]);
        assert!(d.hall_runner);
        assert_eq!(d.prop_set[0].size, [2.0, 1.5, 0.9]);
        assert_eq!(d.room_roles, ["pump_room"]);
    }
}
