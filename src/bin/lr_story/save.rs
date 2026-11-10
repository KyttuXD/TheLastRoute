//! Keeps the story flags between runs. The flags are saved as JSON in the user's folder (`lr_story_flags.json`), under
//! the story's id (the pack id), so a restart keeps what the player has set in that story. Each story keeps its own
//! flags; entries for other ids are kept as they are. The sample story's id is the core pack id, so its saves carry over.
//!
//! ```text
//! lr_story_flags.json   {"schema": "lr.story.flags/1", "packs": {"<story id>": ["flag", ...]}}
//! ```
//!
//! Every function takes the file's path, so tests use a temp folder. A file that exists but cannot be read or is not
//! this format is an error: loading gives the caller an error to show, and saving leaves the file alone rather than
//! overwrite what it cannot read.
//!
//! Browser build (wasm32): the same JSON is kept in the page's localStorage under `lr_story_flags` (through the
//! quad-storage crate), and the path is only a name. If the browser refuses the storage (private window, storage
//! disabled), the first failure logs one line and the game runs without keeping flags.

use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use std::fs;

pub const SCHEMA: &str = "lr.story.flags/1";
pub const FILE_NAME: &str = "lr_story_flags.json";
/// The localStorage key the flags JSON is kept under in the browser build.
#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "lr_story_flags";

/// The flags file in the user's folder, or None when the system has no such folder. The browser build has no folder:
/// its flags go to localStorage, so the name stands in for the path.
pub fn default_path() -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        Some(PathBuf::from(FILE_NAME))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        last_route_core::shared::paths::user_file(FILE_NAME)
    }
}

/// The flags saved for `pack_id`. A missing file, or no entry for the pack, is no flags.
pub fn load_flags(path: &Path, pack_id: &str) -> Result<BTreeSet<String>, String> {
    let Some(root) = read_root(path)? else {
        return Ok(BTreeSet::new());
    };
    let Some(list) = root["packs"].get(pack_id) else {
        return Ok(BTreeSet::new());
    };
    let names = list
        .as_array()
        .ok_or_else(|| format!("{}: the flags for '{pack_id}' must be a list", path.display()))?;
    names
        .iter()
        .map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("{}: flag names must be text", path.display())))
        .collect()
}

/// Saves `flags` under `pack_id` and keeps every other pack's entry. Native: writes a temp file and renames it over the
/// old one, so an interrupted write leaves the old file in place.
pub fn save_flags(path: &Path, pack_id: &str, flags: &BTreeSet<String>) -> Result<(), String> {
    let mut root = read_root(path)?.unwrap_or_else(|| json!({ "schema": SCHEMA, "packs": {} }));
    let mut packs: Map<String, Value> = root["packs"].as_object().cloned().unwrap_or_default();
    packs.insert(pack_id.to_string(), json!(flags.iter().collect::<Vec<_>>()));
    root["packs"] = Value::Object(packs);

    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())? + "\n";
    write_text(path, &text)
}

/// The party progress kept per story, in the same file as the flags under `"party"`:
/// `"party": {"<story id>": {"hp", "max_hp"}}`. Old files without it still load, and old entries that also hold
/// `"level"` and `"xp"` load too: those two fields are ignored (there are no levels) and dropped on the next save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartySave {
    pub hp: i32,
    pub max_hp: i32,
}

/// The party saved for `story_id`. A missing file, a file with no `"party"`, or no entry for the story is None.
pub fn load_party(path: &Path, story_id: &str) -> Result<Option<PartySave>, String> {
    let Some(root) = read_root(path)? else {
        return Ok(None);
    };
    let Some(entry) = root.get("party").and_then(|p| p.get(story_id)) else {
        return Ok(None);
    };
    let bad = || format!("{}: the party for '{story_id}' must have hp and max_hp as numbers", path.display());
    let num = |key: &str| entry.get(key).and_then(Value::as_i64).ok_or_else(bad);
    let max_hp = num("max_hp")?.max(1) as i32;
    // Hp 0 is never trusted (an old file may hold one): at least 1, at most the max.
    let hp = (num("hp")? as i32).clamp(1, max_hp);
    Ok(Some(PartySave { hp, max_hp }))
}

/// Saves the party under `story_id`, keeping the flags and every other story's party as they are.
pub fn save_party(path: &Path, story_id: &str, party: &PartySave) -> Result<(), String> {
    let mut root = read_root(path)?.unwrap_or_else(|| json!({ "schema": SCHEMA, "packs": {} }));
    let mut all: Map<String, Value> = root.get("party").and_then(Value::as_object).cloned().unwrap_or_default();
    // Hp 0 is never saved: a fallen party is written with 1 hp.
    let max_hp = party.max_hp.max(1);
    all.insert(story_id.to_string(), json!({ "hp": party.hp.clamp(1, max_hp), "max_hp": max_hp }));
    root["party"] = Value::Object(all);
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())? + "\n";
    write_text(path, &text)
}

/// The parsed file, or None when it does not exist. Any other problem is an error.
fn read_root(path: &Path) -> Result<Option<Value>, String> {
    let Some(text) = read_text(path)? else {
        return Ok(None);
    };
    let root: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    match root.get("schema").and_then(Value::as_str) {
        Some(SCHEMA) => Ok(Some(root)),
        _ => Err(format!("{}: not a story flags file (expected schema '{SCHEMA}')", path.display())),
    }
}

/// The saved text at `path`, or None when there is no file yet. Any other problem is an error.
#[cfg(not(target_arch = "wasm32"))]
fn read_text(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Replaces the saved text at `path`. A temp file is written and renamed over the old one.
#[cfg(not(target_arch = "wasm32"))]
fn write_text(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // One temp file per process, flushed to disk before the rename, so a crash leaves either the old file or the new one.
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut file = fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        file.write_all(text.as_bytes()).map_err(|e| format!("{}: {e}", tmp.display()))?;
        file.sync_all().map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// Browser: the text under `STORAGE_KEY`. None when nothing is stored, or the browser will not say (the path is unused).
#[cfg(target_arch = "wasm32")]
fn read_text(_path: &Path) -> Result<Option<String>, String> {
    Ok(browser::get())
}

/// Browser: stores the text under `STORAGE_KEY`. A refused save is logged once and the game goes on (the path is unused).
#[cfg(target_arch = "wasm32")]
fn write_text(_path: &Path, text: &str) -> Result<(), String> {
    browser::set(text);
    Ok(())
}

/// The browser's localStorage through quad-storage. quad-storage's calls return nothing when the browser refuses, so a
/// set is checked by reading it back. Every failure logs one line for the whole run, and the game goes on without flags.
#[cfg(target_arch = "wasm32")]
mod browser {
    use super::STORAGE_KEY;
    use std::sync::atomic::{AtomicBool, Ordering};

    static LOGGED: AtomicBool = AtomicBool::new(false);

    fn log_once(problem: &str) {
        if !LOGGED.swap(true, Ordering::Relaxed) {
            eprintln!("lr_story: browser storage {problem}; progress is not kept in this browser");
        }
    }

    pub fn get() -> Option<String> {
        match quad_storage::STORAGE.lock() {
            Ok(storage) => storage.get(STORAGE_KEY),
            Err(_) => {
                log_once("is unavailable");
                None
            }
        }
    }

    pub fn set(text: &str) {
        let mut storage = match quad_storage::STORAGE.lock() {
            Ok(storage) => storage,
            Err(_) => {
                log_once("is unavailable");
                return;
            }
        };
        storage.set(STORAGE_KEY, text);
        if storage.get(STORAGE_KEY).as_deref() != Some(text) {
            log_once("refused the save (private window or storage disabled?)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_story_flags_{name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_round_trip_through_the_file() {
        let dir = scratch("round");
        let path = dir.join(FILE_NAME);
        save_flags(&path, "lr_story", &set(&["asked", "went_left"])).unwrap();
        assert_eq!(load_flags(&path, "lr_story").unwrap(), set(&["asked", "went_left"]));
        // A second save replaces the pack's own list, it does not add to it.
        save_flags(&path, "lr_story", &set(&["went_right"])).unwrap();
        assert_eq!(load_flags(&path, "lr_story").unwrap(), set(&["went_right"]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_pack_id_keeps_its_own_flags() {
        let dir = scratch("keyed");
        let path = dir.join(FILE_NAME);
        save_flags(&path, "lr_story", &set(&["a"])).unwrap();
        save_flags(&path, "lore_other", &set(&["b", "c"])).unwrap();
        assert_eq!(load_flags(&path, "lr_story").unwrap(), set(&["a"]));
        assert_eq!(load_flags(&path, "lore_other").unwrap(), set(&["b", "c"]));
        assert!(load_flags(&path, "nobody").unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_file_means_no_flags_and_saving_creates_it() {
        let dir = scratch("missing");
        let path = dir.join("nested").join(FILE_NAME);
        assert!(load_flags(&path, "lr_story").unwrap().is_empty());
        save_flags(&path, "lr_story", &set(&["x"])).unwrap();
        assert!(path.is_file());
        assert_eq!(load_flags(&path, "lr_story").unwrap(), set(&["x"]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_party_round_trips_beside_the_flags_and_keeps_each_story_apart() {
        let dir = scratch("party");
        let path = dir.join(FILE_NAME);
        assert_eq!(load_party(&path, "lr_story").unwrap(), None, "no file");
        let a = PartySave { hp: 12, max_hp: 35 };
        let b = PartySave { hp: 30, max_hp: 30 };
        save_flags(&path, "lr_story", &set(&["asked"])).unwrap();
        save_party(&path, "lr_story", &a).unwrap();
        save_party(&path, "other", &b).unwrap();
        assert_eq!(load_party(&path, "lr_story").unwrap(), Some(a.clone()));
        assert_eq!(load_party(&path, "other").unwrap(), Some(b));
        assert_eq!(load_party(&path, "nobody").unwrap(), None);
        // Saving flags afterwards leaves the party alone, and the flags are still there.
        save_flags(&path, "lr_story", &set(&["asked", "went_left"])).unwrap();
        assert_eq!(load_party(&path, "lr_story").unwrap(), Some(a));
        assert_eq!(load_flags(&path, "lr_story").unwrap(), set(&["asked", "went_left"]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_save_without_a_party_still_loads() {
        let dir = scratch("oldsave");
        let path = dir.join(FILE_NAME);
        fs::write(&path, "{\"schema\": \"lr.story.flags/1\", \"packs\": {\"lr_story\": [\"asked\"]}}").unwrap();
        assert_eq!(load_party(&path, "lr_story").unwrap(), None);
        assert_eq!(load_flags(&path, "lr_story").unwrap(), set(&["asked"]));
        // Saving a party into the old file keeps its flags.
        save_party(&path, "lr_story", &PartySave { hp: 1, max_hp: 30 }).unwrap();
        assert_eq!(load_flags(&path, "lr_story").unwrap(), set(&["asked"]));
        assert_eq!(load_party(&path, "lr_story").unwrap().unwrap().max_hp, 30);
        // A malformed party entry is an error, not a silent reset.
        fs::write(
            &path,
            "{\"schema\": \"lr.story.flags/1\", \"packs\": {}, \"party\": {\"lr_story\": {\"hp\": \"x\"}}}",
        )
        .unwrap();
        assert!(load_party(&path, "lr_story").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_party_entry_with_level_and_xp_still_loads_and_hp_zero_is_never_kept() {
        let dir = scratch("oldparty");
        let path = dir.join(FILE_NAME);
        fs::write(
            &path,
            "{\"schema\": \"lr.story.flags/1\", \"packs\": {}, \"party\": {\"lr_story\": {\"hp\": 12, \"max_hp\": 35, \"level\": 2, \"xp\": 5}}}",
        )
        .unwrap();
        assert_eq!(load_party(&path, "lr_story").unwrap(), Some(PartySave { hp: 12, max_hp: 35 }));
        // Saving drops level and xp from the file.
        save_party(&path, "lr_story", &PartySave { hp: 12, max_hp: 35 }).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("level") && !text.contains("xp"), "{text}");
        // Hp 0 is saved as 1.
        save_party(&path, "lr_story", &PartySave { hp: 0, max_hp: 35 }).unwrap();
        assert_eq!(load_party(&path, "lr_story").unwrap().unwrap().hp, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_flags_file_is_an_error_and_is_not_overwritten() {
        let dir = scratch("bad");
        let path = dir.join(FILE_NAME);
        fs::write(&path, "{\"schema\": \"something/else\"}").unwrap();
        assert!(load_flags(&path, "lr_story").unwrap_err().contains("not a story flags file"));
        assert!(save_flags(&path, "lr_story", &set(&["x"])).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"schema\": \"something/else\"}");

        fs::write(&path, "not json").unwrap();
        assert!(load_flags(&path, "lr_story").is_err());
        assert!(save_flags(&path, "lr_story", &set(&["x"])).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json");
        let _ = fs::remove_dir_all(&dir);
    }
}
