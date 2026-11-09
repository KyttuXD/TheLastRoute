//! The world a run takes place in: the seeded random generator, the room catalogue, Godot's
//! sort order and the floor generator itself.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub mod catalog;
pub mod door_clearance;
pub mod facility;
pub mod fixtures;
pub mod godot_sort;
pub mod inferno;
pub mod inferno_caves;
pub mod inferno_terrain;
pub mod infinite;
pub mod levels;
pub mod sewer_emergency;
pub mod sewer_props;
pub mod level_links;
pub mod rng;
pub mod route;
pub mod vertical;
pub mod flight;
pub mod flight_links;
#[cfg(test)]
mod stack_contract_tests;
#[cfg(test)]
mod spiral_stairs_tests;
#[cfg(test)]
mod sewer_height_tests;

pub use facility::{Cell, Facility};

/// Texture set used for a room region (the finite route's rooms carry catalogue regions, the infinite route's rooms
/// carry district names; the floor/wall pictures are the original's region swatches). The mapping is data
/// (`data/region_textures.json`); a region it does not list uses its `default` (the maze set).
pub fn region_texture(region: &str) -> &'static str {
    let table = region_texture_table();
    table.map.get(region).copied().unwrap_or(table.default)
}

struct RegionTextures {
    default: &'static str,
    map: std::collections::HashMap<String, &'static str>,
}

fn region_texture_table() -> &'static RegionTextures {
    use std::sync::OnceLock;
    static CELL: OnceLock<RegionTextures> = OnceLock::new();
    CELL.get_or_init(|| {
        let text = crate::content::embedded::data_under("region_textures.json").next().map(|(_, t)| t).unwrap_or("{}");
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
        let set = |name: &str| TEXTURE_SETS.iter().copied().find(|s| *s == name);
        let default = v.get("default").and_then(|d| d.as_str()).and_then(set).unwrap_or("maze");
        let map = v
            .get("map")
            .and_then(|m| m.as_object())
            .map(|m| m.iter().filter_map(|(k, d)| Some((k.clone(), set(d.as_str()?)?))).collect())
            .unwrap_or_default();
        RegionTextures { default, map }
    })
}

pub const TEXTURE_SETS: [&str; 13] = [
    "maze",
    "mansion",
    "hotel",
    "apartment",
    "prison",
    "gothic",
    "beach",
    "carnival",
    "greenhouse",
    "inferno",
    "lab",
    "hospital",
    "mall",
];

#[cfg(test)]
mod region_texture_tests {
    use super::*;

    #[test]
    fn every_district_name_maps_to_a_non_maze_set() {
        for d in ["prison", "lab", "carnival", "inferno", "greenhouse", "sewer", "mansion", "hotel", "apartment", "beach", "gothic", "hospital", "mall"] {
            assert_ne!(region_texture(d), "maze", "{d}");
            assert!(TEXTURE_SETS.contains(&region_texture(d)), "{d}");
        }
    }

    #[test]
    fn nether_and_inferno_agree_and_unknown_is_maze() {
        assert_eq!(region_texture("inferno"), region_texture("nether"));
        assert_eq!(region_texture("maze"), "maze");
        assert_eq!(region_texture("no_such_region"), "maze");
    }

    #[test]
    fn every_texture_set_maps_to_itself_by_name() {
        for s in TEXTURE_SETS {
            assert_eq!(region_texture(s), s);
        }
    }
}
