//! `lr_server --check-pack <folder>`: checks a mod pack folder without starting the server or the game.
//!
//! It builds the manifest the index would receive (`pack.json` plus a `files` list of every file in the folder, with
//! its sha256 and size) and runs it through [`validate_pack_manifest`] with [`MOD_LIMITS`]. Then it runs the game's
//! region loader on the folder (`content::regions::load`), the same code the game uses at start-up. Nothing is
//! written and no port is opened.

use crate::mod_packs::{validate_pack_manifest, ManifestError, MOD_LIMITS};
use crate::util;
use last_route_core::content::packs::Pack;
use last_route_core::content::regions;
use serde_json::{json, Value};
use std::path::Path;

/// The notice the region loader gives for a region with a new id: it loads, but no level places it. A notice, not an
/// error: the game still accepts the pack.
const UNPLACED_REGION_NOTICE: &str = "no level places it";

/// What a check found. Each line names the file and, where there is one, the key or code. `errors` make the check
/// fail; `warnings` do not.
#[derive(Debug, Default, PartialEq)]
pub struct PackCheck {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// The codes whose `path` is a file of the pack (the others name a key of pack.json).
const FILE_ERRORS: [&str; 5] = ["INVALID_FILE_PATH", "DUPLICATE_FILE_PATH", "INVALID_FILE_SHA256", "INVALID_FILE_SIZE", "FILE_TOO_LARGE"];

/// `file: CODE` for a file error, `pack.json: "key": CODE` for a manifest key error, `pack.json: CODE` otherwise.
fn manifest_line(refused: ManifestError) -> String {
    match refused.path {
        Some(path) if FILE_ERRORS.contains(&refused.error) => format!("{path}: {}", refused.error),
        Some(key) => format!("pack.json: \"{key}\": {}", refused.error),
        None => format!("pack.json: {}", refused.error),
    }
}

/// Every file under `root` (recursively) as a manifest `files` entry with its pack-relative path, sha256 and size.
fn collect_files(root: &Path, dir: &Path, out: &mut Vec<Value>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            let rel: Vec<String> = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            out.push(json!({ "path": rel.join("/"), "sha256": util::sha256_hex(&bytes), "size": bytes.len() }));
        }
    }
    Ok(())
}

/// Checks one pack folder with the index's manifest rules, then the game's region rules.
pub fn check_pack_folder(dir: &Path) -> PackCheck {
    let mut check = PackCheck::default();
    let text = match std::fs::read_to_string(dir.join("pack.json")) {
        Ok(text) => text,
        Err(_) => {
            check.errors.push("pack.json: PACK_JSON_MISSING (no pack.json in the folder)".into());
            return check;
        }
    };
    let mut manifest: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            check.errors.push(format!("pack.json: not valid JSON ({error})"));
            return check;
        }
    };
    let mut files = Vec::new();
    if let Err(error) = collect_files(dir, dir, &mut files) {
        check.errors.push(error);
        return check;
    }
    if let Value::Object(map) = &mut manifest {
        map.insert("files".into(), Value::Array(files));
    }
    if let Err(refused) = validate_pack_manifest(&manifest, &MOD_LIMITS) {
        check.errors.push(manifest_line(refused));
    }

    // The region loader reads the same folder the game reads. Its messages already name the file and the key.
    let id = util::str_or_empty(manifest.get("id"));
    let pack = Pack {
        id: if id.is_empty() { dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { id },
        name: util::str_or_empty(manifest.get("name")),
        version: util::str_or_empty(manifest.get("version")),
        author: util::str_or_empty(manifest.get("author")),
        description: util::str_or_empty(manifest.get("description")),
        dir: dir.to_path_buf(),
        provides: vec![],
    };
    let mut notes = Vec::new();
    let _ = regions::load(std::slice::from_ref(&pack), &mut notes);
    for note in notes {
        if note.contains(UNPLACED_REGION_NOTICE) {
            check.warnings.push(note);
        } else {
            check.errors.push(note);
        }
    }
    check
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_pack(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("{label}-{}-{}", std::process::id(), util::hex(&util::random_bytes(6))));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    const REGION: &str = r#"{"schema":"lr.region/1","id":"dungeon","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[{"type":"crate","size":[1,1,1]}]}"#;

    #[test]
    fn a_clean_pack_folder_has_no_errors() {
        let dir = temp_pack("pack-check-clean");
        write(&dir, "pack.json", r#"{"schema":"lr.pack/1","id":"neon_halls","name":"Neon Halls","version":"1.0.0","author":"KT"}"#);
        write(&dir, "props/hall.json", "{}");
        write(&dir, "regions/dungeon.json", REGION);
        let check = check_pack_folder(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(check.errors.is_empty(), "{:?}", check.errors);
        assert_eq!(check.warnings.len(), 1, "a region with a new id is a notice, not an error: {:?}", check.warnings);
        assert!(check.warnings[0].contains("dungeon"));
    }

    #[test]
    fn a_bad_pack_folder_reports_each_error_with_its_file_and_key() {
        let dir = temp_pack("pack-check-bad");
        write(&dir, "pack.json", r#"{"schema":"lr.pack/1","id":"neon_halls","name":"Neon Halls","version":"1.0.0","author":"KT","scripts":["x.gd"]}"#);
        write(&dir, "props/hall.json", "{}");
        write(&dir, "regions/dungeon.json", &REGION.replacen("\"id\":\"dungeon\"", "\"id\":\"dungeon\",\"colour\":\"red\"", 1));
        let check = check_pack_folder(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(check.errors.iter().any(|e| e == "pack.json: \"scripts\": UNKNOWN_PACK_FIELD"), "{:?}", check.errors);
        assert!(
            check.errors.iter().any(|e| e.contains("regions/dungeon.json") && e.contains("unknown key \"colour\"")),
            "{:?}",
            check.errors
        );
        assert!(!check.errors.is_empty());
    }
}
