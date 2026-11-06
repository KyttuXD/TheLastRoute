//! Keeps the story flags between runs. The flags are saved as JSON in the user's folder (`lr_story_flags.json`), under
//! the core pack's id, so a restart keeps what the player has set. Entries for other pack ids are kept as they are.
//!
//! ```text
//! lr_story_flags.json   {"schema": "lr.story.flags/1", "packs": {"<pack id>": ["flag", ...]}}
//! ```
//!
//! Every function takes the file's path, so tests use a temp folder. A file that exists but cannot be read or is not
//! this format is an error: loading gives the caller an error to show, and saving leaves the file alone rather than
//! overwrite what it cannot read.

use crate::model::Pack;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA: &str = "lr.story.flags/1";
pub const FILE_NAME: &str = "lr_story_flags.json";

/// The flags file in the user's folder, or None when the system has no such folder.
pub fn default_path() -> Option<PathBuf> {
    last_route_core::shared::paths::user_file(FILE_NAME)
}

/// The id of the core pack, which the flags are saved under. None only if the pack lists no core pack.
pub fn core_pack_id(pack: &Pack) -> Option<String> {
    pack.packs.iter().find(|p| p.core).map(|p| p.id.clone())
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

/// Saves `flags` under `pack_id` and keeps every other pack's entry. Writes a temp file and renames it over the old one,
/// so an interrupted write leaves the old file in place.
pub fn save_flags(path: &Path, pack_id: &str, flags: &BTreeSet<String>) -> Result<(), String> {
    let mut root = read_root(path)?.unwrap_or_else(|| json!({ "schema": SCHEMA, "packs": {} }));
    let mut packs: Map<String, Value> = root["packs"].as_object().cloned().unwrap_or_default();
    packs.insert(pack_id.to_string(), json!(flags.iter().collect::<Vec<_>>()));
    root["packs"] = Value::Object(packs);

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())? + "\n";
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The parsed file, or None when it does not exist. Any other problem is an error.
fn read_root(path: &Path) -> Result<Option<Value>, String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let root: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    match root.get("schema").and_then(Value::as_str) {
        Some(SCHEMA) => Ok(Some(root)),
        _ => Err(format!("{}: not a story flags file (expected schema '{SCHEMA}')", path.display())),
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
