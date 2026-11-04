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
        // tables, catwalk tables) are read by their own modules, not through BUILTIN_FILES.
        const OTHER: [&str; 4] = [
            "regions/catwalk.json",
            "regions/catwalk_links.json",
            "regions/maze_",
            "regions/sewer_rooms.json",
        ];
        let embedded: Vec<(&str, &str)> = embedded::data_under("regions/")
            .filter(|(p, _)| !OTHER.iter().any(|o| p.starts_with(o)))
            .collect();
        assert_eq!(embedded, BUILTIN_FILES.to_vec());
    }

    #[test]
    fn the_standalone_defaults_match_the_game_constants() {
        assert_eq!(DEFAULT_WALL_HEIGHT, crate::config::WALL_HEIGHT);
        assert_eq!(DEFAULT_ROOM_MIN, crate::world::facility::MIN_AUTHORED_ROOM_SIZE);
        assert_eq!(MAX_ROOM_SIZE, crate::world::facility::MAX_AUTHORED_ROOM_SIZE);
    }
}
