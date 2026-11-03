//! The region catalogue lives in `crate::packs::regions`, which depends only on serde_json and std so that
//! `lr_server` can use it without the game. This module re-exports it, so the game's paths do not change.

pub use crate::packs::regions::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::embedded;

    #[test]
    fn the_standalone_builtin_files_match_the_embedded_data() {
        let embedded: Vec<(&str, &str)> = embedded::data_under("regions/").collect();
        assert_eq!(embedded, BUILTIN_FILES.to_vec());
    }

    #[test]
    fn the_standalone_defaults_match_the_game_constants() {
        assert_eq!(DEFAULT_WALL_HEIGHT, crate::config::WALL_HEIGHT);
        assert_eq!(DEFAULT_ROOM_MIN, crate::world::facility::MIN_AUTHORED_ROOM_SIZE);
        assert_eq!(MAX_ROOM_SIZE, crate::world::facility::MAX_AUTHORED_ROOM_SIZE);
    }
}
