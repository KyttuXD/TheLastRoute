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
    /// Where the values are saved (None: kept in memory only, as in unit tests, so a test never
    /// edits the real admin.cfg).
    file: Option<std::path::PathBuf>,
}

fn path() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::app::paths::user_file("admin.cfg")
}

impl Prefs {
    pub fn load() -> Prefs {
        Prefs::load_from(path())
    }

    /// Loads the values saved in `file` (missing or unreadable: the defaults); later `set`s save there.
    pub fn load_from(file: Option<std::path::PathBuf>) -> Prefs {
        let mut p = Prefs {
            values: KEYS.to_vec(),
            file,
        };
        if let Some(text) = p.file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()) {
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

    /// Puts every key back to its default and saves once (the admin menu's RESET ALL).
    pub fn reset_all(&mut self) {
        if self.values.iter().zip(KEYS.iter()).all(|(a, b)| a.1 == b.1) {
            return;
        }
        self.values = KEYS.to_vec();
        self.save();
    }

    fn save(&self) {
        let Some(f) = self.file.as_ref() else { return };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_admin_prefs_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("admin.cfg")
    }

    #[test]
    fn a_switch_set_in_one_session_is_read_back_in_the_next() {
        let f = temp_file("persist");
        let mut p = Prefs::load_from(Some(f.clone()));
        assert!(!p.get("god_mode"));
        p.set("god_mode", true);
        p.set("perf_overlay", true);
        let again = Prefs::load_from(Some(f.clone()));
        assert!(again.get("god_mode") && again.get("perf_overlay"));
        assert!(!again.get("noclip"));
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn reset_all_saves_the_defaults() {
        let f = temp_file("reset");
        let mut p = Prefs::load_from(Some(f.clone()));
        p.set("noclip", true);
        p.set("map_reveal", true);
        p.reset_all();
        assert!(!p.get("noclip") && !p.get("map_reveal"));
        let again = Prefs::load_from(Some(f.clone()));
        assert!(!again.get("noclip") && !again.get("map_reveal"));
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn unknown_keys_and_junk_lines_are_ignored() {
        let f = temp_file("junk");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, "bogus=true\nnoclip=true\nno equals sign\ngod_mode=maybe\n").unwrap();
        let p = Prefs::load_from(Some(f.clone()));
        assert!(p.get("noclip"));
        assert!(!p.get("god_mode"), "anything but true is off");
        assert!(!p.get("bogus"));
        let _ = std::fs::remove_dir_all(f.parent().unwrap());
    }

    #[test]
    fn a_memory_only_prefs_never_touches_disk() {
        let mut p = Prefs::load_from(None);
        p.set("god_mode", true);
        assert!(p.get("god_mode"));
    }
}
