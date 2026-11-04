//! The data behind the connections between levels. Nothing here fades, holds or places the runner.
//! A stair flight is walked and relabelled (game/endless.rs `relabel`): it is never interacted with and has no landing
//! step. A ladder is climbed by hand (game/ladder.rs): it moves the body in absolute y; the label follows through `relabel`.
//!
//! `prefetch_distance` (data/stairs_links.json, `links()`) is the residency radius (the key keeps its historic name): how close (cells) the runner gets
//! to a connection before the storey it leads to is kept resident by `Game::keep_stack`.
//! Godot reference: game/infinite_route/infinite_route_test.gd (`_update_player_ladder`, `_on_player_ladder_finished`).

use crate::content::embedded;
use std::sync::OnceLock;

/// The data file (under the embedded data root, see build.rs).
pub const STAIRS_LINKS_PATH: &str = "stairs_links.json";

/// The level-link numbers read from data/stairs_links.json.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StairsLinks {
    /// How close (cells) the runner gets to a stair or ladder landing before the storey it leads to is kept resident
    /// (`Game::keep_stack`), so crossing builds no window.
    pub prefetch_distance: f32,
}

/// Reads a `StairsLinks` from the JSON text. A negative value is an error.
pub fn parse(text: &str) -> Result<StairsLinks, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let prefetch_distance = v
        .get("prefetch_distance")
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .ok_or_else(|| "prefetch_distance: a number is needed".to_string())?;
    if !(prefetch_distance >= 0.0) {
        return Err("prefetch_distance must not be negative".into());
    }
    Ok(StairsLinks { prefetch_distance })
}

static LINKS: OnceLock<StairsLinks> = OnceLock::new();

/// The level-link data, loaded once from the embedded data/stairs_links.json.
pub fn links() -> StairsLinks {
    *LINKS.get_or_init(|| {
        let text = embedded::data_under("")
            .find(|(p, _)| *p == STAIRS_LINKS_PATH)
            .map(|(_, t)| t)
            .expect("data/stairs_links.json is embedded by build.rs");
        parse(text).expect("data/stairs_links.json is valid")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_json(key: &str, value: &str) -> String {
        let text = embedded::data_under("")
            .find(|(p, _)| *p == STAIRS_LINKS_PATH)
            .map(|(_, t)| t)
            .expect("embedded");
        let mut v: serde_json::Value = serde_json::from_str(text).expect("json");
        v[key] = serde_json::from_str(value).expect("value");
        v.to_string()
    }

    #[test]
    fn the_embedded_data_file_is_the_standard_values() {
        assert!((links().prefetch_distance - 8.0).abs() < 1e-6);
    }

    #[test]
    fn a_changed_prefetch_distance_in_the_data_is_read() {
        assert!((parse(&with_json("prefetch_distance", "3")).expect("valid").prefetch_distance - 3.0).abs() < 1e-6);
    }

    #[test]
    fn a_negative_value_in_the_data_is_refused() {
        assert!(parse(&with_json("prefetch_distance", "-1")).is_err(), "a negative prefetch distance is refused");
    }
}
