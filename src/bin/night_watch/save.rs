//! Night Watch progress, kept apart from Last Route's: its own file under the game user folder
//! (`night-watch/progress.json`). Same shape as the Godot save's `night_watch` section: the nights
//! cleared and the highest night unlocked for Classic, `cleared_<mode>` for every other mode, plus
//! stars, best scores and the Custom Night sliders per mode. The website save link comes later.

use last_route_core::shared::paths;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub const NIGHTS: u32 = 7;
pub const CLASSIC: &str = "classic";

/// The Night Watch game id on the server's save routes (/api/v1/saves/night-watch-rust/progress).
pub const SERVER_GAME_ID: &str = "night-watch-rust";
/// The slot that holds all nights' progress on the server.
pub const SERVER_SLOT: &str = "progress";

const FOLDER: &str = "night-watch";
const FILE: &str = "progress.json";
const SCARES: [&str; 3] = ["full", "short", "off"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NightWatchSave {
    root: Map<String, Value>,
}

pub fn default_path() -> Option<PathBuf> {
    paths::user_dir().map(|d| d.join(FOLDER).join(FILE))
}

fn cleared_key(mode: &str) -> String {
    if mode == CLASSIC {
        "cleared".to_string()
    } else {
        format!("cleared_{mode}")
    }
}

impl NightWatchSave {
    pub fn from_json(text: &str) -> Self {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(root)) => Self { root },
            _ => Self::default(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&Value::Object(self.root.clone())).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|t| Self::from_json(&t))
            .unwrap_or_default()
    }

    pub fn load() -> Self {
        default_path().map(|p| Self::load_from(&p)).unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(&tmp, path)
    }

    pub fn save(&self) -> std::io::Result<()> {
        match default_path() {
            Some(p) => self.save_to(&p),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no user folder",
            )),
        }
    }

    fn uint(&self, key: &str) -> Option<u32> {
        self.root
            .get(key)
            .and_then(Value::as_u64)
            .map(|n| n.min(u32::MAX as u64) as u32)
    }

    fn list(&self, key: &str) -> Vec<Value> {
        self.root
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    }

    /// Highest night the player may start (1 at first; a cleared night unlocks the next).
    pub fn unlocked_night(&self) -> u32 {
        self.uint("unlocked").unwrap_or(1).clamp(1, NIGHTS)
    }

    /// Levels cleared in a mode, 1-based, sorted, without duplicates.
    pub fn cleared_levels(&self, mode: &str) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .list(&cleared_key(mode))
            .iter()
            .filter_map(Value::as_u64)
            .filter(|&n| n >= 1 && n <= u32::MAX as u64)
            .map(|n| n as u32)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn record_level_clear(&mut self, mode: &str, level: u32) {
        if level < 1 || level == u32::MAX || (mode == CLASSIC && level > NIGHTS) {
            return;
        }
        let mut list = self.cleared_levels(mode);
        if !list.contains(&level) {
            list.push(level);
            list.sort_unstable();
        }
        self.root.insert(cleared_key(mode), json!(list));
        if mode == CLASSIC {
            let next = self.unlocked_night().max(level + 1).clamp(1, NIGHTS);
            self.root.insert("unlocked".into(), json!(next));
        }
    }

    /// A level is open when it is the first, the one before it is cleared, or the mode opens everything.
    pub fn level_open(&self, mode: &str, level: u32, open_all: bool) -> bool {
        open_all || level <= 1 || self.cleared_levels(mode).contains(&(level - 1))
    }

    /// A mode's Custom Night opens once its Night 5 is cleared.
    pub fn custom_unlocked(&self, mode: &str) -> bool {
        self.cleared_levels(mode).contains(&5)
    }

    pub fn stars(&self, mode: &str) -> Vec<String> {
        self.list(&format!("stars_{mode}"))
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    }

    pub fn record_star(&mut self, mode: &str, preset_id: &str) {
        let mut list = self.stars(mode);
        if !list.iter().any(|s| s == preset_id) {
            list.push(preset_id.to_string());
        }
        self.root.insert(format!("stars_{mode}"), json!(list));
    }

    pub fn best_score(&self, mode: &str) -> u32 {
        self.uint(&format!("best_{mode}")).unwrap_or(0)
    }

    pub fn record_best(&mut self, mode: &str, score: u32) {
        if score > self.best_score(mode) {
            self.root.insert(format!("best_{mode}"), json!(score));
        }
    }

    /// The Custom Night sliders last used in a mode: hunter id to 0..=20.
    pub fn custom_values(&self, mode: &str) -> Vec<(String, u32)> {
        self.root
            .get(&format!("custom_{mode}"))
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n.min(20) as u32)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_custom_values(&mut self, mode: &str, values: &[(String, u32)]) {
        let map: Map<String, Value> = values
            .iter()
            .map(|(k, v)| (k.clone(), json!((*v).min(20))))
            .collect();
        self.root.insert(format!("custom_{mode}"), Value::Object(map));
    }

    pub fn subtitles(&self) -> bool {
        self.root
            .get("subtitles")
            .and_then(Value::as_bool)
            .unwrap_or(true)
    }

    pub fn set_subtitles(&mut self, on: bool) {
        self.root.insert("subtitles".into(), json!(on));
    }

    /// Calm mode: no lamp flicker or glitching.
    pub fn calm(&self) -> bool {
        self.root.get("calm").and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn set_calm(&mut self, on: bool) {
        self.root.insert("calm".into(), json!(on));
    }

    /// Scare setting: "full", "short" or "off".
    pub fn scare(&self) -> &str {
        self.root
            .get("scare")
            .and_then(Value::as_str)
            .and_then(|s| SCARES.iter().copied().find(|&k| k == s))
            .unwrap_or("full")
    }

    pub fn set_scare(&mut self, value: &str) {
        if SCARES.contains(&value) {
            self.root.insert("scare".into(), json!(value));
        }
    }

    pub fn reset(&mut self) {
        self.root.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_a_night_unlocks_the_next() {
        let mut s = NightWatchSave::default();
        assert_eq!(s.unlocked_night(), 1);
        s.record_level_clear(CLASSIC, 1);
        s.record_level_clear(CLASSIC, 1);
        assert_eq!(s.cleared_levels(CLASSIC), vec![1]);
        assert_eq!(s.unlocked_night(), 2);
        s.record_level_clear(CLASSIC, NIGHTS);
        assert_eq!(s.unlocked_night(), NIGHTS);
        s.record_level_clear(CLASSIC, NIGHTS + 1);
        assert_eq!(s.cleared_levels(CLASSIC), vec![1, NIGHTS]);
    }

    #[test]
    fn modes_keep_separate_keys() {
        let mut s = NightWatchSave::default();
        s.record_level_clear("mask", 5);
        assert!(s.custom_unlocked("mask"));
        assert!(!s.custom_unlocked(CLASSIC));
        assert!(s.level_open("mask", 6, false));
        assert!(!s.level_open("mask", 3, false));
        assert!(s.level_open("mask", 3, true));
        assert!(s.to_json().contains("cleared_mask"));
    }

    #[test]
    fn round_trips_and_survives_garbage() {
        let mut s = NightWatchSave::default();
        s.record_star("mask", "a");
        s.record_star("mask", "a");
        s.record_best("mask", 40);
        s.record_best("mask", 10);
        s.set_scare("short");
        s.set_scare("bogus");
        s.set_custom_values("mask", &[("kathy".into(), 99)]);
        let back = NightWatchSave::from_json(&s.to_json());
        assert_eq!(back, s);
        assert_eq!(back.stars("mask"), vec!["a"]);
        assert_eq!(back.best_score("mask"), 40);
        assert_eq!(back.scare(), "short");
        assert_eq!(back.custom_values("mask"), vec![("kathy".to_string(), 20)]);
        assert_eq!(NightWatchSave::from_json("not json"), NightWatchSave::default());
        assert!(NightWatchSave::default().subtitles());
    }

    #[test]
    fn saves_to_disk() {
        let root = std::env::temp_dir().join("nw_save_test");
        let path = root.join("night-watch").join("progress.json");
        let mut s = NightWatchSave::default();
        s.record_level_clear(CLASSIC, 2);
        s.save_to(&path).unwrap();
        assert_eq!(NightWatchSave::load_from(&path).unlocked_night(), 3);
        let _ = std::fs::remove_dir_all(root);
    }
}
