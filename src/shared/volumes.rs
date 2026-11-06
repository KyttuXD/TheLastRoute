//! The player's three volume sliders, for the executables other than the game. The game keeps its settings in
//! `app/settings.rs`; both read the same per-user `settings.json` (`paths::user_file`), with the same keys and defaults.
//! Night Watch has no settings screen, so it reads the sliders once, when it starts: a change made in the game reaches
//! Night Watch on its next launch.

use serde_json::Value;

/// Master, GAMEPLAY (the SFX bus) and MENU / UI, each a linear gain 0..1. Defaults to 1.0, like the game.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volumes {
    pub master: f32,
    pub gameplay: f32,
    pub ui: f32,
}

impl Default for Volumes {
    fn default() -> Self {
        Volumes { master: 1.0, gameplay: 1.0, ui: 1.0 }
    }
}

impl Volumes {
    /// The sliders from a parsed settings file. A missing or non-numeric key keeps 1.0; values clamp to 0..1.
    pub fn from_json(v: &Value) -> Volumes {
        let f = |k: &str| v.get(k).and_then(Value::as_f64).map(|x| (x as f32).clamp(0.0, 1.0)).unwrap_or(1.0);
        Volumes { master: f("master_volume"), gameplay: f("gameplay_volume"), ui: f("ui_volume") }
    }

    /// The saved sliders, or the defaults when there is no settings file or it does not parse.
    pub fn load() -> Volumes {
        super::paths::user_file("settings.json")
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|v| Volumes::from_json(&v))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn saved_sliders_are_read_and_missing_ones_default_to_full() {
        let v = Volumes::from_json(&json!({ "master_volume": 0.4, "ui_volume": 0.25 }));
        assert_eq!(v, Volumes { master: 0.4, gameplay: 1.0, ui: 0.25 });
        assert_eq!(Volumes::from_json(&json!({})), Volumes::default());
    }

    #[test]
    fn out_of_range_or_non_numeric_values_are_clamped_or_ignored() {
        let v = Volumes::from_json(&json!({ "master_volume": 3.0, "gameplay_volume": -1.0, "ui_volume": "loud" }));
        assert_eq!(v, Volumes { master: 1.0, gameplay: 0.0, ui: 1.0 });
    }
}
