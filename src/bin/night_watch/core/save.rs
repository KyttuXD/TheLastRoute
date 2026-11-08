//! Night Watch progress and settings in one JSON blob: native in the user folder (`night_watch_save.json`), in the
//! browser through quad-storage. Every failure gives defaults. Pure logic except the two storage calls at the bottom.
#![allow(dead_code)]

use super::data::{ModesData, MAX_AI, NIGHTS, POINTS_PER_LEVEL};
use super::session::Session;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const SAVE_FILE: &str = "night_watch_save.json";
pub const SAVE_KEY: &str = "night_watch_save";
pub const SCARE_SETTINGS: [&str; 3] = ["full", "short", "off"];
pub const DEFAULT_MOUSE_SENS: f32 = 0.0075;

#[derive(Clone, Debug)]
pub struct Save {
    pub cleared: BTreeMap<String, Vec<u32>>,
    pub unlocked: u32,
    pub stars: BTreeMap<String, Vec<String>>,
    pub best: BTreeMap<String, i32>,
    pub custom: BTreeMap<String, BTreeMap<String, i32>>,
    pub subtitles: bool,
    pub calm: bool,
    /// "full", "short" or "off".
    pub scare: String,
    /// "GUEST_%08X", made once.
    pub guest: String,
    /// action name -> KeyCode name (see controls::keycode_name)
    pub keys: BTreeMap<String, String>,
    /// 0..100
    pub master_volume: f32,
    pub mouse_sens: f32,
    /// A dev control was used this run: nothing is saved. Never persisted.
    pub debug_run: bool,
    /// The signed-in account name (UPPERCASE) and its server session token; both empty for a guest. The password is never kept.
    pub account_name: String,
    pub session_token: String,
}

impl Default for Save {
    fn default() -> Self {
        Save {
            cleared: BTreeMap::new(),
            unlocked: 1,
            stars: BTreeMap::new(),
            best: BTreeMap::new(),
            custom: BTreeMap::new(),
            subtitles: true,
            calm: false,
            scare: "full".to_string(),
            guest: new_guest_name(),
            keys: BTreeMap::new(),
            master_volume: 100.0,
            mouse_sens: DEFAULT_MOUSE_SENS,
            debug_run: false,
            account_name: String::new(),
            session_token: String::new(),
        }
    }
}

fn new_guest_name() -> String {
    format!("GUEST_{:08X}", Session::fresh_seed())
}

fn read_map<T>(v: &Value, key: &str, f: impl Fn(&Value) -> Option<T>) -> BTreeMap<String, T> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(o)) = v.get(key) {
        for (k, x) in o {
            if let Some(t) = f(x) {
                out.insert(k.clone(), t);
            }
        }
    }
    out
}

impl Save {
    /// Native: the user folder file; wasm: browser storage. Any failure gives `Save::default()`.
    pub fn load() -> Save {
        match read_raw() {
            Some(text) => Save::from_json(&text),
            None => Save::default(),
        }
    }

    /// Write to the device. A no-op for a debug run.
    pub fn store(&self) {
        if self.debug_run {
            return;
        }
        write_raw(&self.to_json());
    }

    pub fn from_json(s: &str) -> Save {
        let v: Value = match serde_json::from_str(s) {
            Ok(v) => v,
            Err(_) => return Save::default(),
        };
        let mut save = Save::default();
        save.cleared = read_map(&v, "cleared", |x| {
            let mut list: Vec<u32> = x.as_array()?.iter().filter_map(|n| n.as_f64()).filter(|n| *n >= 1.0).map(|n| n as u32).collect();
            list.sort();
            list.dedup();
            Some(list)
        });
        save.unlocked = (v.get("unlocked").and_then(|x| x.as_f64()).unwrap_or(1.0) as i64).clamp(1, NIGHTS as i64) as u32;
        save.stars = read_map(&v, "stars", |x| {
            let mut list: Vec<String> = Vec::new();
            for s in x.as_array()?.iter().filter_map(|s| s.as_str()) {
                if !list.iter().any(|e| e == s) {
                    list.push(s.to_string());
                }
            }
            Some(list)
        });
        save.best = read_map(&v, "best", |x| x.as_f64().map(|n| n as i32));
        save.custom = read_map(&v, "custom", |x| {
            let mut m = BTreeMap::new();
            for (k, n) in x.as_object()? {
                m.insert(k.clone(), (n.as_f64().unwrap_or(0.0) as i32).clamp(0, MAX_AI));
            }
            Some(m)
        });
        save.subtitles = v.get("subtitles").and_then(|x| x.as_bool()).unwrap_or(true);
        save.calm = v.get("calm").and_then(|x| x.as_bool()).unwrap_or(false);
        if let Some(sc) = v.get("scare").and_then(|x| x.as_str()) {
            if SCARE_SETTINGS.contains(&sc) {
                save.scare = sc.to_string();
            }
        }
        if let Some(g) = v.get("guest").and_then(|x| x.as_str()) {
            if !g.is_empty() {
                save.guest = g.to_string();
            }
        }
        save.keys = read_map(&v, "keys", |x| x.as_str().map(|s| s.to_string()));
        save.account_name = v.get("account_name").and_then(|x| x.as_str()).unwrap_or("").trim().to_uppercase();
        save.session_token = v.get("session_token").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if let Some(m) = v.get("master_volume").and_then(|x| x.as_f64()) {
            save.master_volume = (m as f32).clamp(0.0, 100.0);
        }
        if let Some(m) = v.get("mouse_sens").and_then(|x| x.as_f64()) {
            if m.is_finite() && m > 0.0 {
                save.mouse_sens = (m as f32).clamp(0.0005, 0.05);
            }
        }
        save
    }

    pub fn to_json(&self) -> String {
        let v = json!({
            "version": 1,
            "cleared": self.cleared,
            "unlocked": self.unlocked,
            "stars": self.stars,
            "best": self.best,
            "custom": self.custom,
            "subtitles": self.subtitles,
            "calm": self.calm,
            "scare": self.scare,
            "guest": self.guest,
            "keys": self.keys,
            "master_volume": self.master_volume,
            "mouse_sens": self.mouse_sens,
            "account_name": self.account_name,
            "session_token": self.session_token,
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    pub fn cleared_levels(&self, mode: &str) -> Vec<u32> {
        self.cleared.get(mode).cloned().unwrap_or_default()
    }

    /// A level is open when it is the first, the one before it is cleared, or the mode opens everything.
    pub fn level_open(&self, mode: &str, level: i32, open_all: bool) -> bool {
        open_all || level <= 1 || self.cleared.get(mode).map_or(false, |l| l.contains(&((level - 1) as u32)))
    }

    /// A level is cleared (and opens the next); classic also raises the highest unlocked night.
    pub fn record_level_clear(&mut self, mode: &str, level: i32) {
        if level < 1 {
            return;
        }
        let list = self.cleared.entry(mode.to_string()).or_default();
        if !list.contains(&(level as u32)) {
            list.push(level as u32);
            list.sort();
        }
        if mode == "classic" {
            self.unlocked = (self.unlocked as i64).max(level as i64 + 1).clamp(1, NIGHTS as i64) as u32;
        }
        self.store();
    }

    /// A mode's Custom Night opens once its level 5 is cleared (the 2D rule, kept literally).
    pub fn custom_unlocked(&self, mode: &str) -> bool {
        self.cleared.get(mode).map_or(false, |l| l.contains(&5))
    }

    pub fn record_star(&mut self, mode: &str, preset: &str) {
        let list = self.stars.entry(mode.to_string()).or_default();
        if !list.iter().any(|p| p == preset) {
            list.push(preset.to_string());
        }
        self.store();
    }

    pub fn record_best(&mut self, mode: &str, score: i32) {
        if score > self.best_score(mode) {
            self.best.insert(mode.to_string(), score);
            self.store();
        }
    }

    pub fn best_score(&self, mode: &str) -> i32 {
        self.best.get(mode).copied().unwrap_or(0)
    }

    pub fn star_count(&self, mode: &str) -> usize {
        self.stars.get(mode).map_or(0, |l| l.len())
    }

    pub fn custom_values(&self, mode: &str) -> BTreeMap<String, i32> {
        self.custom.get(mode).cloned().unwrap_or_default()
    }

    pub fn set_custom_values(&mut self, mode: &str, v: BTreeMap<String, i32>) {
        self.custom.insert(mode.to_string(), v);
        self.store();
    }

    /// Short and Off scare settings both keep the death scene short.
    pub fn short_death_scenes(&self) -> bool {
        self.scare != "full"
    }

    pub fn reduced_motion(&self) -> bool {
        self.calm
    }

    /// Erase everything, settings included, and write the empty save.
    pub fn reset(&mut self) {
        let debug = self.debug_run;
        let (account_name, session_token) = (std::mem::take(&mut self.account_name), std::mem::take(&mut self.session_token));
        *self = Save::default();
        self.debug_run = debug;
        // erasing progress does not sign the account out
        self.account_name = account_name;
        self.session_token = session_token;
        self.store();
    }

    /// A finished shift: a normal level is cleared (returns 0); a Custom Night records its best score and the stars of
    /// the presets its sliders equal, and returns the total score.
    pub fn record_win(&mut self, session: &Session, modes: &ModesData) -> i32 {
        if !session.is_custom {
            self.record_level_clear(&session.mode_id, session.night);
            return 0;
        }
        let total: i32 = session.custom_levels.values().map(|v| (*v).clamp(0, MAX_AI)).sum::<i32>() * POINTS_PER_LEVEL;
        self.record_best(&session.mode_id, total);
        if let Some(entry) = modes.get(&session.mode_id) {
            for p in &entry.presets {
                let same = p.ai.iter().all(|(k, v)| session.custom_levels.get(k).copied().unwrap_or(0) == *v);
                if same {
                    self.record_star(&session.mode_id, &p.id);
                }
            }
        }
        total
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_raw() -> Option<String> {
    let path = last_route_core::shared::paths::user_file(SAVE_FILE)?;
    std::fs::read_to_string(path).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn write_raw(text: &str) {
    if let Some(path) = last_route_core::shared::paths::user_file(SAVE_FILE) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, text);
    }
}

#[cfg(target_arch = "wasm32")]
fn read_raw() -> Option<String> {
    match quad_storage::STORAGE.lock() {
        Ok(storage) => storage.get(SAVE_KEY),
        Err(_) => None,
    }
}

#[cfg(target_arch = "wasm32")]
fn write_raw(text: &str) {
    if let Ok(mut storage) = quad_storage::STORAGE.lock() {
        storage.set(SAVE_KEY, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A save whose store() is a no-op, so tests never touch the user's real file.
    fn scratch() -> Save {
        let mut s = Save::default();
        s.debug_run = true;
        s
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let s = Save::default();
        assert_eq!(s.unlocked, 1);
        assert!(s.subtitles && !s.calm);
        assert_eq!(s.scare, "full");
        assert_eq!(s.master_volume, 100.0);
        assert_eq!(s.mouse_sens, 0.0075);
        assert!(s.guest.starts_with("GUEST_") && s.guest.len() == 14);
        assert!(!s.short_death_scenes() && !s.reduced_motion());
    }

    #[test]
    fn json_round_trip_keeps_everything() {
        let mut s = scratch();
        s.record_level_clear("classic", 1);
        s.record_level_clear("classic", 2);
        s.record_level_clear("mask", 3);
        s.record_star("classic", "first-shift");
        s.record_best("classic", 120);
        let mut c = BTreeMap::new();
        c.insert("dovelle".to_string(), 8);
        s.set_custom_values("classic", c.clone());
        s.calm = true;
        s.scare = "short".to_string();
        s.subtitles = false;
        s.master_volume = 55.0;
        s.mouse_sens = 0.01;
        s.keys.insert("nw_lamp".to_string(), "T".to_string());
        let t = Save::from_json(&s.to_json());
        assert_eq!(t.cleared, s.cleared);
        assert_eq!(t.unlocked, 3);
        assert_eq!(t.stars, s.stars);
        assert_eq!(t.best_score("classic"), 120);
        assert_eq!(t.custom_values("classic"), c);
        assert!(t.calm && !t.subtitles);
        assert_eq!(t.scare, "short");
        assert_eq!(t.master_volume, 55.0);
        assert_eq!(t.mouse_sens, 0.01);
        assert_eq!(t.keys.get("nw_lamp").map(|s| s.as_str()), Some("T"));
        assert_eq!(t.guest, s.guest);
        assert!(!t.debug_run);
    }

    #[test]
    fn bad_json_gives_defaults_and_bad_fields_are_repaired() {
        assert_eq!(Save::from_json("not json").unlocked, 1);
        let s = Save::from_json("{\"unlocked\": 99, \"scare\": \"loud\", \"master_volume\": 500, \"cleared\": {\"classic\": [3, 1, 1, 0, -2]}}");
        assert_eq!(s.unlocked, 7);
        assert_eq!(s.scare, "full");
        assert_eq!(s.master_volume, 100.0);
        assert_eq!(s.cleared_levels("classic"), vec![1, 3]);
    }

    #[test]
    fn level_open_rules_are_literal() {
        let mut s = scratch();
        assert!(s.level_open("classic", 1, false));
        assert!(!s.level_open("classic", 2, false));
        assert!(s.level_open("classic", 2, true));
        s.record_level_clear("classic", 1);
        assert!(s.level_open("classic", 2, false));
        assert!(!s.level_open("classic", 3, false));
        assert!(!s.level_open("mask", 2, false));
    }

    #[test]
    fn custom_night_opens_after_level_five_is_cleared() {
        let mut s = scratch();
        for n in 1..=4 {
            s.record_level_clear("mask", n);
        }
        assert!(!s.custom_unlocked("mask"));
        s.record_level_clear("mask", 5);
        assert!(s.custom_unlocked("mask"));
        // a 3 level mode never gets there
        for n in 1..=3 {
            s.record_level_clear("extra", n);
        }
        assert!(!s.custom_unlocked("extra"));
    }

    #[test]
    fn classic_clear_raises_the_unlocked_night() {
        let mut s = scratch();
        s.record_level_clear("classic", 3);
        assert_eq!(s.unlocked, 4);
        s.record_level_clear("classic", 1);
        assert_eq!(s.unlocked, 4);
        s.record_level_clear("classic", 7);
        assert_eq!(s.unlocked, 7);
        s.record_level_clear("mask", 2);
        assert_eq!(s.unlocked, 7);
        s.record_level_clear("classic", 0);
        assert_eq!(s.cleared_levels("classic"), vec![1, 3, 7]);
    }

    #[test]
    fn best_only_goes_up_and_stars_do_not_repeat() {
        let mut s = scratch();
        s.record_best("classic", 50);
        s.record_best("classic", 30);
        assert_eq!(s.best_score("classic"), 50);
        s.record_star("classic", "a");
        s.record_star("classic", "a");
        s.record_star("classic", "b");
        assert_eq!(s.star_count("classic"), 2);
        assert_eq!(s.star_count("mask"), 0);
    }

    #[test]
    fn record_win_for_a_normal_shift_clears_the_level() {
        let modes = ModesData::load();
        let mut s = scratch();
        let mut session = Session::new();
        session.start_mode("classic", 2, BTreeMap::new(), Some(1));
        assert_eq!(s.record_win(&session, &modes), 0);
        assert_eq!(s.cleared_levels("classic"), vec![2]);
    }

    #[test]
    fn record_win_for_a_custom_night_scores_and_stars() {
        let modes = ModesData::load();
        let mut s = scratch();
        let mut session = Session::new();
        let mut c = BTreeMap::new();
        for (k, v) in [("dovelle", 8), ("nyxie", 0), ("ravelle", 0), ("ilyra", 0), ("aki", 0)] {
            c.insert(k.to_string(), v);
        }
        session.start_mode("classic", 1, c, Some(1));
        assert_eq!(s.record_win(&session, &modes), 80);
        assert_eq!(s.best_score("classic"), 80);
        assert_eq!(s.stars.get("classic").map(|v| v.clone()), Some(vec!["first-shift".to_string()]));
        // a different mix scores more but matches no preset
        let mut c2 = BTreeMap::new();
        c2.insert("dovelle".to_string(), 9);
        c2.insert("aki".to_string(), 1);
        session.start_mode("classic", 1, c2, Some(1));
        assert_eq!(s.record_win(&session, &modes), 100);
        assert_eq!(s.star_count("classic"), 1);
        assert_eq!(s.best_score("classic"), 100);
    }

    #[test]
    fn reset_erases_everything_but_the_debug_flag() {
        let mut s = scratch();
        s.record_level_clear("classic", 4);
        s.calm = true;
        s.keys.insert("nw_lamp".to_string(), "T".to_string());
        s.reset();
        assert!(s.cleared.is_empty() && !s.calm && s.keys.is_empty());
        assert_eq!(s.unlocked, 1);
        assert!(s.debug_run);
    }

    #[test]
    fn scare_settings_shorten_the_death_scene() {
        let mut s = scratch();
        s.scare = "short".to_string();
        assert!(s.short_death_scenes());
        s.scare = "off".to_string();
        assert!(s.short_death_scenes());
    }
}
