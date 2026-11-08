//! The region catalogue lives in `crate::packs::regions`, which depends only on serde_json and std so that
//! `lr_server` can use it without the game. This module re-exports it, so the game's paths do not change.

pub use crate::packs::regions::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::embedded;

    #[test]
    fn the_standalone_builtin_files_match_the_embedded_data() {
        // Files beside the region catalogue that are not `lr.region/1` (maze variants, sewer room
        // tables) are read by their own modules, not through BUILTIN_FILES.
        const OTHER: [&str; 2] = [
            "regions/maze_",
            "regions/sewer_rooms.json",
        ];
        let mut embedded: Vec<(&str, &str)> = embedded::data_under("regions/")
            .filter(|(p, _)| !OTHER.iter().any(|o| p.starts_with(o)))
            .collect();
        // the built-in list is in draw order, the embedded listing is by name: the same files either way
        let mut builtin = BUILTIN_FILES.to_vec();
        embedded.sort();
        builtin.sort();
        assert_eq!(embedded, builtin);
    }

    /// The sewer ceiling is Godot's 3.2 m x 1.25 = 4.0 m on both sewer storeys, 1 m under the next floor (a 1 m slab
    /// that only shafts and flights are lined through). Everything that reads a region's wall height (xivvy's
    /// headroom, prop mounts, ceiling fixtures) reads this number; the readers are listed in docs/SEWERS.md.
    #[test]
    fn both_sewer_regions_have_the_four_metre_ceiling() {
        for file in ["regions/sewer.json", "regions/sewer2.json"] {
            let (_, text) = embedded::data_under(file).next().unwrap_or_else(|| panic!("{file} is embedded"));
            let v: serde_json::Value = serde_json::from_str(text).unwrap();
            let h = v["geometry"]["wall_height"].as_f64().unwrap_or_else(|| panic!("{file} has a wall_height")) as f32;
            assert_eq!(h, 4.0, "{file}");
            assert!(h < crate::world::vertical::STOREY, "{file}: the ceiling is under the next floor");
        }
    }

    /// The embedded region JSON is read (mall and hospital included): every region Godot's draw can return has its file,
    /// parsed with no warning, with real facts in it. `godot_region_for` stays the placement reference; the table owns the facts.
    #[test]
    fn every_region_the_godot_draw_returns_has_a_parsed_region_file() {
        use crate::world::route::regions::{godot_region_for, NAMES};
        let mut warnings = vec![];
        let table = RegionTable::builtin(&mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        let mut seen = std::collections::BTreeSet::new();
        for world in ["parity-a", "parity-b", "parity-c"] {
            for level in [2, 1, 0, -1, -2, -3, -4, -6] {
                for cx in (-64..64).step_by(4) {
                    for cz in (-64..64).step_by(4) {
                        seen.insert(godot_region_for(world, cx, cz, level));
                    }
                }
            }
        }
        assert!(seen.len() >= 10, "the sweep should reach most regions: {seen:?}");
        for id in &seen {
            // the Inferno is two data regions (inferno0, inferno_caves), not one file called "inferno"
            if *id == "inferno" {
                continue;
            }
            let def = table.def(id).unwrap_or_else(|| panic!("region {id} has no region file"));
            assert_eq!(def.source, BUILTIN, "{id}");
            assert!(!def.hall_floor.is_empty() && !def.room_default_floor.is_empty(), "{id}: floors");
        }
        for id in ["mall", "hospital"] {
            let def = table.def(id).unwrap_or_else(|| panic!("{id}"));
            assert!(!def.room_roles.is_empty() && !def.prop_set.is_empty(), "{id}: the JSON facts are read");
        }
        // every named region that has a file in the table is a known name (the table does not invent placed regions)
        for def in &table.defs {
            let known = NAMES.contains(&def.id.as_str()) || def.id.starts_with("maze") || def.id.starts_with("sewer") || def.id.starts_with("inferno");
            assert!(known, "region file {} is not a region name the placement knows", def.id);
        }
    }

    /// A pack region's `safe_light` and the defaults a region without the block gets (the checkpoint light of
    /// `render::world_mesh::safe_light_for`).
    #[test]
    fn a_pack_region_carries_its_own_safe_light_and_the_rest_default() {
        let region = |extra: &str| {
            format!(r#"{{"schema":"lr.region/1","id":"lit","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[]{extra}}}"#)
        };
        let plain = parse_file("lit.json", &region("")).unwrap();
        assert_eq!(plain.safe_light, None);
        let lit = parse_file("lit.json", &region(r#","safe_light":{"color":[0.1,0.4,0.9],"energy":3.5}"#)).unwrap();
        assert_eq!(lit.safe_light, Some(RegionSafeLight { color: [0.1, 0.4, 0.9], energy: 3.5 }));
        let dim = parse_file("lit.json", &region(r#","safe_light":{"energy":0.5}"#)).unwrap();
        assert_eq!(dim.safe_light, Some(RegionSafeLight { color: SAFE_LIGHT_COLOR, energy: 0.5 }));
        assert!(parse_file("lit.json", &region(r#","safe_light":{"energy":99}"#)).is_err(), "energy is range checked");
    }

    #[test]
    fn the_standalone_defaults_match_the_game_constants() {
        assert_eq!(DEFAULT_WALL_HEIGHT, crate::config::WALL_HEIGHT);
        assert_eq!(DEFAULT_ROOM_MIN, crate::world::facility::MIN_AUTHORED_ROOM_SIZE);
        assert_eq!(MAX_ROOM_SIZE, crate::world::facility::MAX_AUTHORED_ROOM_SIZE);
    }
}
