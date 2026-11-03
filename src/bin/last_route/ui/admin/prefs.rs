//! The admin preferences (Settings.ADMIN_PREFERENCE_DEFAULTS in game/autoload/settings.gd): the
//! debug toggles the hub ADMIN panel edits and a run's admin menu applies at start and keeps in
//! sync. Stored as `key=true|false` lines in the user folder's admin.cfg.

pub const KEYS: [(&str, bool); 8] = [
    ("no_target", false),
    ("god_mode", false),
    ("noclip", false),
    ("map_reveal", false),
    ("region_map", false),
    ("show_collision", false),
    ("hunter_reveal", false),
    ("perf_overlay", false),
];

#[derive(Clone)]
pub struct Prefs {
    values: Vec<(&'static str, bool)>,
}

fn path() -> Option<std::path::PathBuf> {
    crate::app::paths::user_file("admin.cfg")
}

impl Prefs {
    pub fn load() -> Prefs {
        let mut p = Prefs {
            values: KEYS.to_vec(),
        };
        if let Some(text) = path().and_then(|f| std::fs::read_to_string(f).ok()) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('=') {
                    if let Some(e) = p.values.iter_mut().find(|(key, _)| *key == k.trim()) {
                        e.1 = v.trim() == "true";
                    }
                }
            }
        }
        p
    }

    pub fn get(&self, key: &str) -> bool {
        self.values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|e| e.1)
            .unwrap_or(false)
    }

    /// set_admin_preference: unknown keys are ignored, unchanged values are not re-saved.
    pub fn set(&mut self, key: &str, on: bool) {
        let Some(e) = self.values.iter_mut().find(|(k, _)| *k == key) else {
            return;
        };
        if e.1 == on {
            return;
        }
        e.1 = on;
        self.save();
    }

    fn save(&self) {
        let Some(f) = path() else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text: String = self
            .values
            .iter()
            .map(|(k, v)| format!("{k}={v}\n"))
            .collect();
        let _ = std::fs::write(f, text);
    }
}
