//! The player's settings: mouse, view, interface, textures and skin, plus the key bindings (`input::Bindings`).
//!
//! ```text
//! lr_story_settings.json   {"schema": "lr.story.settings/1", "mouse_sensitivity": 1.0, "invert_y": false, ...,
//!                           "bindings": {"forward": "W", ...}}
//! ```
//!
//! Loading never fails: a missing or unreadable file gives the defaults, a value out of range is clamped, a value of the
//! wrong type falls back to its default, unknown keys are ignored. Native: the file sits beside the flags file in the
//! user's folder. Browser (wasm32): localStorage key `lr_story_settings` through quad-storage.
//!
//! `get()` returns the cached settings (loaded once); `set()` replaces the cache and saves. A failed save is logged
//! and ignored.

use crate::input::Bindings;
use serde_json::{json, Map, Value};
use std::cell::RefCell;

pub const SCHEMA: &str = "lr.story.settings/1";
#[cfg(not(target_arch = "wasm32"))]
pub const FILE_NAME: &str = "lr_story_settings.json";
#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "lr_story_settings";

pub const MOUSE_SENS_RANGE: (f32, f32) = (0.25, 3.0);
pub const FOV_RANGE: (f32, f32) = (60.0, 100.0);
pub const UI_SCALE_RANGE: (f32, f32) = (0.8, 1.3);
/// The id of the built-in default figure.
pub const DEFAULT_SKIN: &str = "mannequin";

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub mouse_sensitivity: f32,
    pub invert_y: bool,
    pub fov_deg: f32,
    pub ui_scale: f32,
    pub flat_textures: bool,
    pub show_name_tags: bool,
    pub hotbar_fade: bool,
    pub player_skin: String,
    pub bindings: Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            mouse_sensitivity: 1.0,
            invert_y: false,
            fov_deg: 78.0,
            ui_scale: 1.0,
            flat_textures: false,
            show_name_tags: true,
            hotbar_fade: true,
            player_skin: DEFAULT_SKIN.to_string(),
            bindings: Bindings::default(),
        }
    }
}

impl Settings {
    /// Pulls every value into its range (a NaN becomes the default).
    pub fn clamped(mut self) -> Settings {
        let d = Settings::default();
        let fix = |v: f32, (lo, hi): (f32, f32), fallback: f32| if v.is_finite() { v.clamp(lo, hi) } else { fallback };
        self.mouse_sensitivity = fix(self.mouse_sensitivity, MOUSE_SENS_RANGE, d.mouse_sensitivity);
        self.fov_deg = fix(self.fov_deg, FOV_RANGE, d.fov_deg);
        self.ui_scale = fix(self.ui_scale, UI_SCALE_RANGE, d.ui_scale);
        if self.player_skin.trim().is_empty() {
            self.player_skin = DEFAULT_SKIN.to_string();
        }
        self
    }

    pub fn to_json(&self) -> String {
        let names: Map<String, Value> = self.bindings.to_names().into_iter().map(|(k, v)| (k, json!(v))).collect();
        let root = json!({
            "schema": SCHEMA,
            "mouse_sensitivity": self.mouse_sensitivity,
            "invert_y": self.invert_y,
            "fov_deg": self.fov_deg,
            "ui_scale": self.ui_scale,
            "flat_textures": self.flat_textures,
            "show_name_tags": self.show_name_tags,
            "hotbar_fade": self.hotbar_fade,
            "player_skin": self.player_skin,
            "bindings": Value::Object(names),
        });
        serde_json::to_string_pretty(&root).unwrap_or_default() + "\n"
    }

    /// Settings from JSON text. Never fails: junk gives the defaults, bad single values fall back one by one.
    pub fn from_json(text: &str) -> Settings {
        let d = Settings::default();
        let Ok(root) = serde_json::from_str::<Value>(text) else {
            return d;
        };
        if !root.is_object() {
            return d;
        }
        let num = |key: &str, fallback: f32| root.get(key).and_then(Value::as_f64).map(|v| v as f32).unwrap_or(fallback);
        let flag = |key: &str, fallback: bool| root.get(key).and_then(Value::as_bool).unwrap_or(fallback);
        let bindings = match root.get("bindings").and_then(Value::as_object) {
            Some(map) => Bindings::from_names(map.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.as_str(), s)))),
            None => Bindings::default(),
        };
        Settings {
            mouse_sensitivity: num("mouse_sensitivity", d.mouse_sensitivity),
            invert_y: flag("invert_y", d.invert_y),
            fov_deg: num("fov_deg", d.fov_deg),
            ui_scale: num("ui_scale", d.ui_scale),
            flat_textures: flag("flat_textures", d.flat_textures),
            show_name_tags: flag("show_name_tags", d.show_name_tags),
            hotbar_fade: flag("hotbar_fade", d.hotbar_fade),
            player_skin: root
                .get("player_skin")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or(d.player_skin),
            bindings,
        }
        .clamped()
    }
}

thread_local! {
    static CACHE: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

/// The current settings (loaded from storage on the first call).
pub fn get() -> Settings {
    CACHE.with(|c| c.borrow_mut().get_or_insert_with(load).clone())
}

/// Replaces the settings and saves them. A failed save is logged and ignored.
pub fn set(s: Settings) {
    let s = s.clamped();
    if let Err(e) = write_text(&s.to_json()) {
        eprintln!("lr_story: could not save the settings: {e}");
    }
    CACHE.with(|c| *c.borrow_mut() = Some(s));
}

/// Replaces the settings for this run without saving (a slider being dragged); `flush` saves them when the drag ends.
pub fn set_cached(s: Settings) {
    CACHE.with(|c| *c.borrow_mut() = Some(s.clamped()));
}

/// Saves the current settings (after a drag).
pub fn flush() {
    set(get());
}

fn load() -> Settings {
    // Tests never read or depend on the player's real settings file.
    if cfg!(test) {
        return Settings::default();
    }
    match read_text() {
        Some(text) => Settings::from_json(&text),
        None => Settings::default(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn file_path() -> Option<std::path::PathBuf> {
    last_route_core::shared::paths::user_file(FILE_NAME)
}

#[cfg(not(target_arch = "wasm32"))]
fn read_text() -> Option<String> {
    std::fs::read_to_string(file_path()?).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_text(text: &str) -> Result<(), String> {
    use std::io::Write;
    let path = file_path().ok_or("no user folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        file.write_all(text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
        file.sync_all().map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

#[cfg(target_arch = "wasm32")]
fn read_text() -> Option<String> {
    quad_storage::STORAGE.lock().ok()?.get(STORAGE_KEY)
}

#[cfg(target_arch = "wasm32")]
fn write_text(text: &str) -> Result<(), String> {
    let mut storage = quad_storage::STORAGE.lock().map_err(|_| "browser storage is unavailable".to_string())?;
    storage.set(STORAGE_KEY, text);
    if storage.get(STORAGE_KEY).as_deref() != Some(text) {
        return Err("the browser refused the save".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Action;
    use macroquad::prelude::KeyCode;

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert_eq!(s.mouse_sensitivity, 1.0);
        assert!(!s.invert_y);
        assert_eq!(s.fov_deg, 78.0);
        assert_eq!(s.ui_scale, 1.0);
        assert!(!s.flat_textures);
        assert!(s.show_name_tags);
        assert!(s.hotbar_fade);
        assert_eq!(s.player_skin, "mannequin");
        assert_eq!(s.bindings, Bindings::default());
    }

    #[test]
    fn json_round_trip() {
        let mut s = Settings::default();
        s.mouse_sensitivity = 1.75;
        s.invert_y = true;
        s.fov_deg = 90.0;
        s.ui_scale = 1.25;
        s.flat_textures = true;
        s.show_name_tags = false;
        s.hotbar_fade = false;
        s.player_skin = "hunter_blue".into();
        s.bindings.rebind(Action::Jump, KeyCode::X);
        s.bindings.rebind(Action::Forward, KeyCode::S);
        let back = Settings::from_json(&s.to_json());
        assert_eq!(back, s);
        assert!(s.to_json().contains(SCHEMA));
    }

    #[test]
    fn out_of_range_values_clamp() {
        let s = Settings::from_json(r#"{"mouse_sensitivity": 99, "fov_deg": 5, "ui_scale": -3}"#);
        assert_eq!(s.mouse_sensitivity, 3.0);
        assert_eq!(s.fov_deg, 60.0);
        assert_eq!(s.ui_scale, 0.8);
        let t = Settings::from_json(r#"{"mouse_sensitivity": 0.0, "fov_deg": 500, "ui_scale": 9}"#);
        assert_eq!(t.mouse_sensitivity, 0.25);
        assert_eq!(t.fov_deg, 100.0);
        assert_eq!(t.ui_scale, 1.3);
        let mut n = Settings::default();
        n.fov_deg = f32::NAN;
        assert_eq!(n.clamped().fov_deg, 78.0);
    }

    #[test]
    fn junk_falls_back_to_defaults() {
        for text in ["", "not json", "[]", "42", "null", "{"] {
            assert_eq!(Settings::from_json(text), Settings::default(), "{text:?}");
        }
        let s = Settings::from_json(
            r#"{"mouse_sensitivity": "fast", "invert_y": 3, "player_skin": 7, "bindings": [1], "show_name_tags": false}"#,
        );
        let d = Settings::default();
        assert_eq!(s.mouse_sensitivity, d.mouse_sensitivity);
        assert_eq!(s.invert_y, d.invert_y);
        assert_eq!(s.player_skin, d.player_skin);
        assert_eq!(s.bindings, d.bindings);
        assert!(!s.show_name_tags, "the valid value is kept");
        assert_eq!(Settings::from_json(r#"{"player_skin": "  "}"#).player_skin, "mannequin");
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let s = Settings::from_json(r#"{"future_thing": true, "fov_deg": 70, "bindings": {"teleport": "T", "jump": "X"}}"#);
        assert_eq!(s.fov_deg, 70.0);
        assert_eq!(s.bindings.key(Action::Jump), KeyCode::X);
        assert_eq!(s.bindings.key(Action::Forward), KeyCode::W);
    }
}
