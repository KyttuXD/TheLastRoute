//! The music box as a collectable: a route pickup that goes into the pack and is wound from it (game/items.rs).
//! Its counts and times are data (data/music_box.json): the boxes per region, the wound time by difficulty, the
//! winddown stretch and the tune's gain. Sources: GameConfig.MUSIC_BOXES_PER_REGION and MUSIC_BOX_WOUND_SECONDS
//! (game_config.gd 543-546) and MusicBoxes.mood_for (music_boxes.gd 31).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

const FILE: &str = "music_box.json";

/// The parsed data/music_box.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// Boxes per route region (`MUSIC_BOXES_PER_REGION`).
    pub boxes_per_region: usize,
    /// Wound seconds by difficulty id. "standard" is the fallback for any other id.
    pub wound_seconds: HashMap<String, f32>,
    /// The last stretch of a wound box, in seconds: Glyra's winding-down mood and the HUD's warning colour. (The tune's
    /// slowing is `slowdown_from_fraction`, not this.)
    pub winddown_seconds: f32,
    /// The tune plays at normal speed while the wind left is at least this fraction of the wound time (0.70 = 70 %),
    /// then slows and sinks as it drains to nothing (audio/cues/music_box.rs `slowdown_factor`).
    pub slowdown_from_fraction: f32,
    /// How fast the tune still plays just before the box empties (a factor of normal speed, tempo and pitch).
    pub slowdown_floor: f32,
    /// The tune loop's gain against the master bus.
    pub tune_gain: f32,
}

/// The rules as the data file gives them. A missing or malformed file is a build defect (the file is embedded), so
/// this panics rather than guessing.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under(FILE)
            .next()
            .map(|x| x.1)
            .unwrap_or_else(|| panic!("data/{FILE} is missing from the build"));
        parse(text)
    })
}

/// Reads the data file text. Split out so the tests can check the shipped file parses.
pub fn parse(text: &str) -> Rules {
    let v: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("data/{FILE}: {e}"));
    let num = |k: &str| v[k].as_f64().unwrap_or_else(|| panic!("data/{FILE}: {k} is not a number")) as f32;
    let wound_seconds = v["wound_seconds"]
        .as_object()
        .unwrap_or_else(|| panic!("data/{FILE}: wound_seconds is not an object"))
        .iter()
        .map(|(id, s)| (id.clone(), s.as_f64().unwrap_or_else(|| panic!("data/{FILE}: wound_seconds.{id}")) as f32))
        .collect::<HashMap<_, _>>();
    assert!(wound_seconds.contains_key("standard"), "data/{FILE}: wound_seconds needs a standard entry");
    Rules {
        boxes_per_region: v["boxes_per_region"]
            .as_u64()
            .unwrap_or_else(|| panic!("data/{FILE}: boxes_per_region is not a count")) as usize,
        wound_seconds,
        winddown_seconds: num("winddown_seconds"),
        slowdown_from_fraction: num("slowdown_from_fraction"),
        slowdown_floor: num("slowdown_floor"),
        tune_gain: num("tune_gain"),
    }
}

/// The seconds a box winds to on `difficulty_id` ("standard" for any id without its own entry).
pub fn wound_seconds(difficulty_id: &str) -> f32 {
    let r = rules();
    r.wound_seconds
        .get(difficulty_id)
        .or_else(|| r.wound_seconds.get("standard"))
        .copied()
        .unwrap_or(45.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_file_parses_with_the_godot_numbers() {
        let r = rules();
        assert_eq!(r.boxes_per_region, 2);
        assert_eq!(r.winddown_seconds, 15.0);
        assert_eq!(r.slowdown_from_fraction, 0.70, "the tune slows below 70 % of the wind");
        assert!(r.slowdown_floor > 0.0 && r.slowdown_floor < 1.0);
        assert!(r.tune_gain > 0.0 && r.tune_gain <= 0.3, "the tune stays quiet");
        assert_eq!(wound_seconds("standard"), 45.0);
        assert_eq!(wound_seconds("hard"), 35.0);
        assert_eq!(wound_seconds("nightmare"), 25.0);
        // any other id winds like the standard box (the Godot fallback)
        assert_eq!(wound_seconds("relaxed"), 45.0);
    }
}
