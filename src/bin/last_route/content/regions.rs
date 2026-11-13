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

    // ------------------------------------------------------------------------------------------------------
    // the regions pass (docs/REGIONS_PASS.md): region props, their models, the seasonal regions

    /// Every prop definition of data/props/*.json, by type id.
    fn prop_defs() -> std::collections::HashMap<String, crate::content::props::PropDef> {
        let mut out = std::collections::HashMap::new();
        for (path, text) in embedded::data_under("props/") {
            let v: serde_json::Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{path}: {e}"));
            for p in crate::content::props::parse_file(&v) {
                out.insert(p.id.clone(), p);
            }
        }
        out
    }

    /// A model's document, its lowest and highest corner in metres (cubes only) and its drawn triangles.
    fn read_model(path: &str) -> (serde_json::Value, [f32; 3], [f32; 3], usize) {
        let bytes = embedded::asset(path).unwrap_or_else(|| panic!("{path} is embedded"));
        let j: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        let (mut lo, mut hi, mut tris) = ([f32::MAX; 3], [f32::MIN; 3], 0usize);
        for e in j["elements"].as_array().unwrap_or_else(|| panic!("{path}: elements")) {
            if e["type"].as_str() != Some("cube") {
                continue;
            }
            for i in 0..3 {
                let (f, t) = (e["from"][i].as_f64().unwrap() as f32 / 16.0, e["to"][i].as_f64().unwrap() as f32 / 16.0);
                assert!(f < t, "{path}: {} has no volume", e["name"]);
                lo[i] = lo[i].min(f);
                hi[i] = hi[i].max(t);
            }
            tris += e["faces"].as_object().map_or(0, |faces| faces.values().filter(|x| !x["texture"].is_null()).count() * 2);
        }
        (j, lo, hi, tris)
    }

    /// Every region's prop set (and its extra set), every room table's prop table and every recipe item names a
    /// prop type of data/props whose models are all embedded and read as models with geometry.
    #[test]
    fn every_region_prop_names_a_prop_type_whose_models_parse() {
        let defs = prop_defs();
        let mut checked = std::collections::BTreeSet::new();
        let mut check = |ty: &str, place: &str| {
            let def = defs.get(ty).unwrap_or_else(|| panic!("{place}: {ty} is not a prop type of data/props"));
            assert!(!def.models.is_empty(), "{place}: {ty} has no model");
            if !checked.insert(ty.to_string()) {
                return;
            }
            for model in &def.models {
                let bytes = embedded::asset(model).unwrap_or_else(|| panic!("{place}: {ty}: {model} is not embedded"));
                let j: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{model}: {e}"));
                assert!(j["elements"].as_array().is_some_and(|e| !e.is_empty()), "{model}: no geometry");
            }
        };
        let mut warnings = vec![];
        let table = RegionTable::builtin(&mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        for def in &table.defs {
            for p in def.prop_set.iter().chain(&def.extra_prop_set) {
                check(&p.prop_type, &format!("regions/{}.json", def.id));
                assert!(p.size.iter().all(|s| *s > 0.0 && *s < 40.0), "{}: {} size {:?}", def.id, p.prop_type, p.size);
            }
        }
        for (id, rooms) in crate::world::route::furnished::region_rooms::all() {
            let place = format!("region_rooms/{id}.json");
            for p in &rooms.props {
                check(&p.prop_type, &place);
            }
            for r in &rooms.rooms {
                for ty in r.recipe_types() {
                    check(ty, &place);
                    assert!(rooms.props.iter().any(|p| p.prop_type == ty), "{place}: {ty} of room {} is in the prop table", r.role);
                }
            }
            for rule in &rooms.tunnel_dressing {
                check(&rule.prop_type, &place);
            }
        }
        assert!(checked.len() > 100, "only {} prop types were checked", checked.len());
    }

    /// A prop type has one authored size wherever the regions pass lists it, so its blocker box is the same in
    /// every region (the room tables' `data_prop_size` takes the first one it finds).
    #[test]
    fn a_regions_pass_prop_has_the_same_size_everywhere() {
        let pass = pass_prop_ids();
        let mut sizes: std::collections::BTreeMap<String, [f32; 3]> = Default::default();
        let mut see = |ty: &str, size: [f32; 3], place: &str| {
            if !pass.contains(&ty.to_string()) {
                return;
            }
            let first = *sizes.entry(ty.to_string()).or_insert(size);
            assert_eq!(first, size, "{place}: {ty}");
        };
        let table = RegionTable::builtin(&mut Vec::new());
        for def in &table.defs {
            for p in def.prop_set.iter().chain(&def.extra_prop_set) {
                see(&p.prop_type, p.size, &def.id);
            }
        }
        for (id, rooms) in crate::world::route::furnished::region_rooms::all() {
            for p in &rooms.props {
                see(&p.prop_type, p.size, id);
            }
        }
        // every type of the pass is placed by at least one region
        for ty in &pass {
            assert!(sizes.contains_key(ty), "{ty} is in data/props/regions_pass.json but no region places it");
        }
    }

    /// The prop types of data/props/regions_pass.json, in file order.
    fn pass_prop_ids() -> Vec<String> {
        let (_, text) = embedded::data_under("props/regions_pass.json").next().expect("props/regions_pass.json is embedded");
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        v["props"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap().to_string()).collect()
    }

    /// The 46 models of the regions pass (tools/props/props_pass_models.py): cubes only, within the triangle budget,
    /// standing on the floor, inside the size the regions give the type, and all painted from the one palette
    /// picture the props-pass models use, so they share a texture and a batch.
    #[test]
    fn the_regions_pass_models_are_small_cube_models_on_the_shared_palette() {
        let defs = prop_defs();
        let (palette_model, _, _, _) = read_model("props/maze/arcade_cabinet_lime.bbmodel");
        let palette = palette_model["textures"][0]["source"].as_str().expect("the props-pass palette").to_string();
        // the size of each type: the first region that lists it
        let table = RegionTable::builtin(&mut Vec::new());
        let size_of = |ty: &str| -> [f32; 3] {
            table
                .defs
                .iter()
                .flat_map(|d| d.prop_set.iter().chain(&d.extra_prop_set))
                .find(|p| p.prop_type == ty)
                .map(|p| p.size)
                .unwrap_or_else(|| panic!("{ty} is in no region's prop set"))
        };
        let ids = pass_prop_ids();
        assert_eq!(ids.len(), 45);
        let mut models = 0;
        for ty in &ids {
            let def = defs.get(ty).unwrap_or_else(|| panic!("{ty}"));
            let size = size_of(ty);
            for model in &def.models {
                models += 1;
                let (j, lo, hi, tris) = read_model(model);
                assert!(j["elements"].as_array().unwrap().iter().all(|e| e["type"].as_str() == Some("cube")), "{model}: cubes only");
                assert!((40..=520).contains(&tris), "{model}: {tris} triangles");
                assert!(lo[1].abs() < 1e-4, "{model}: stands on the floor, lowest point {}", lo[1]);
                // the blocker box is centred on the prop: the model reaches no further than half the size on x and z
                let slack = 0.011;
                assert!(hi[1] <= size[1] + slack, "{model}: {} m tall in a {} m box", hi[1], size[1]);
                for (axis, i) in [("x", 0), ("z", 2)] {
                    let reach = hi[i].max(-lo[i]);
                    assert!(reach <= size[i] * 0.5 + slack, "{model}: reaches {reach} m on {axis}, half the box is {}", size[i] * 0.5);
                    assert!(reach >= size[i] * 0.5 - 0.06, "{model}: the {axis} box ({}) is larger than the model ({reach} each way)", size[i]);
                }
                assert_eq!(j["textures"][0]["source"].as_str(), Some(palette.as_str()), "{model}: the shared palette picture");
            }
        }
        assert_eq!(models, 46);
    }

    /// A palette's `textures` is a texture set the game has, and every region the mix can place has a texture set of
    /// its own in data/region_textures.json or a district (it is not left looking like the maze by accident).
    #[test]
    fn every_palette_names_a_texture_set_the_game_has() {
        let table = RegionTable::builtin(&mut Vec::new());
        for def in &table.defs {
            for p in &def.palettes {
                if let Some(set) = &p.textures {
                    assert!(crate::world::TEXTURE_SETS.contains(&set.as_str()), "{}/{}: {set} is not a texture set", def.id, p.name);
                }
            }
        }
        for (id, _) in &crate::world::route::regions::mix().regions {
            assert_ne!(crate::world::region_texture(id), "maze", "{id} has no row in data/region_textures.json");
        }
    }

    /// The seasonal regions: each has its own prop set of at least nine types of its own, palettes, rooms, and the
    /// lit props carry a modest light (a glow where the prop glows).
    #[test]
    fn the_seasonal_regions_have_their_own_props_palettes_and_lights() {
        let defs = prop_defs();
        let pass = pass_prop_ids();
        let table = RegionTable::builtin(&mut Vec::new());
        for (id, own) in [
            ("festive", ["christmasTree", "wrappedPresents", "festiveWreath", "snowman", "candyCanePost", "stockingMantel", "festiveSleigh", "nutcracker", "stringLightPost"]),
            ("halloween", ["jackOLantern", "witchCauldron", "coffin", "tombstone", "scarecrow", "cobwebCorner", "skeletonProp", "witchBroomRack", "candleCluster"]),
            ("harvest", ["hayBale", "cornucopiaTable", "pumpkinPile", "cornStalkBundle", "harvestFeastTable", "pieStand", "turkeyCentrepiece", "leafPile", "woodenCart"]),
        ] {
            let def = table.def(id).unwrap_or_else(|| panic!("{id} has a region file"));
            for ty in own {
                assert!(pass.contains(&ty.to_string()), "{ty} is a type of the pass");
                assert!(def.prop_set.iter().any(|p| p.prop_type == ty), "{id}: {ty} is in the prop set");
            }
            assert!(def.palettes.len() >= 2, "{id}: palettes");
            assert!(def.room_roles.len() >= 4, "{id}: rooms");
            assert!(crate::world::route::regions::is_extra(id), "{id} is not a Godot region");
        }
        for (ty, glow) in [
            ("christmasTree", true),
            ("stockingMantel", true),
            ("jackOLantern", true),
            ("witchCauldron", true),
            ("candleCluster", false),
            ("stringLightPost", false),
            ("mansionCandelabra", false),
            ("mansionFireplace", false),
        ] {
            let light = defs.get(ty).and_then(|d| d.light.as_ref()).unwrap_or_else(|| panic!("{ty} has a light"));
            assert_eq!(light.glow, glow, "{ty}");
            assert!(light.energy > 0.1 && light.energy <= 0.55, "{ty}: energy {}", light.energy);
            assert!(light.range > 1.0 && light.range <= 4.5, "{ty}: range {}", light.range);
        }
        // no new type is read as a seat, a bed or a mirror by its name, but the piano stool, which is a seat
        for ty in &pass {
            let lowered = ty.to_lowercase();
            let seat = ["chair", "couch", "bench", "stool", "sofa"].iter().any(|w| lowered.contains(w));
            assert_eq!(seat, ty == "pianoStool", "{ty}");
            assert!(!lowered.ends_with("bed") && !lowered.contains("bunk") && !lowered.contains("mirror"), "{ty}");
        }
    }

    #[test]
    fn the_standalone_defaults_match_the_game_constants() {
        assert_eq!(DEFAULT_WALL_HEIGHT, crate::config::WALL_HEIGHT);
        assert_eq!(DEFAULT_ROOM_MIN, crate::world::facility::MIN_AUTHORED_ROOM_SIZE);
        assert_eq!(MAX_ROOM_SIZE, crate::world::facility::MAX_AUTHORED_ROOM_SIZE);
    }
}
