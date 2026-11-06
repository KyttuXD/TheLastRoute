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
        let embedded: Vec<(&str, &str)> = embedded::data_under("regions/")
            .filter(|(p, _)| !OTHER.iter().any(|o| p.starts_with(o)))
            .collect();
        assert_eq!(embedded, BUILTIN_FILES.to_vec());
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

    #[test]
    fn the_standalone_defaults_match_the_game_constants() {
        assert_eq!(DEFAULT_WALL_HEIGHT, crate::config::WALL_HEIGHT);
        assert_eq!(DEFAULT_ROOM_MIN, crate::world::facility::MIN_AUTHORED_ROOM_SIZE);
        assert_eq!(MAX_ROOM_SIZE, crate::world::facility::MAX_AUTHORED_ROOM_SIZE);
    }
}
