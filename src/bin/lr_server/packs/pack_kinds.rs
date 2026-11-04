//! Port of `backend/pack_kinds.mjs`: the folders a mod pack may ship, as the game sees them
//! (`ContentRegistry.PACK_KINDS` in `src/config/content_registry.gd` plus every extension kind declared in
//! `data/kinds/<owner>.json`). The deploy writes them to `<backend dir>/pack_kinds.json`; `mod_packs` reads that
//! file, or the repo when it runs from a checkout, or its own fallback list.
//!
//! In Node the "backend dir" is the folder of the script itself. Here it is [`backend_dir`]:
//! `LASTROUTE_BACKEND_DIR` when set, otherwise the folder of the running executable; the repo root is its parent.

use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};

const PACK_KINDS_FILE_NAME: &str = "pack_kinds.json";

/// `KIND = /^[a-z][a-z0-9_]{0,31}$/`
pub fn is_kind(kind: &str) -> bool {
    let bytes = kind.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// The Node module's `HERE`: `LASTROUTE_BACKEND_DIR`, else the executable's folder, else the working directory.
pub fn backend_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("LASTROUTE_BACKEND_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `readPackKindsFromRepo(root)`: built-in kinds (source order) then extension kinds (sorted); None when `root` is
/// not a checkout.
pub fn read_pack_kinds_from_repo(root: &Path) -> Option<Vec<String>> {
    let registry = root.join("src").join("config").join("content_registry.gd");
    if !registry.exists() {
        return None;
    }
    let source = String::from_utf8_lossy(&std::fs::read(&registry).ok()?).into_owned();
    let block = Regex::new(r"(?m)^const PACK_KINDS\s*:=\s*\[([\s\S]*?)\]").unwrap();
    let inner = block.captures(&source)?.get(1)?.as_str().to_string();
    let quoted = Regex::new(r#""([^"]+)""#).unwrap();
    let builtin: Vec<String> = quoted
        .captures_iter(&inner)
        .map(|m| m[1].to_string())
        .filter(|kind| is_kind(kind))
        .collect();
    let mut extra: Vec<String> = Vec::new();
    let kinds_dir = root.join("data").join("kinds");
    if kinds_dir.exists() {
        let mut names: Vec<String> = std::fs::read_dir(&kinds_dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.ends_with(".json"))
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by(|a, b| crate::mod_packs::js_str_cmp(a, b));
        for name in names {
            // a broken kinds file is the game's error to report; the index just does not learn that kind
            let Ok(bytes) = std::fs::read(kinds_dir.join(&name)) else { continue };
            let Ok(declared) = serde_json::from_str::<Value>(&String::from_utf8_lossy(&bytes)) else { continue };
            // Object.keys(declared || {}): only objects (and arrays/strings, whose index keys never pass KIND) count
            if let Value::Object(map) = &declared {
                for kind in map.keys() {
                    if is_kind(kind) && !builtin.contains(kind) && !extra.contains(kind) {
                        extra.push(kind.clone());
                    }
                }
            }
        }
    }
    extra.sort_by(|a, b| crate::mod_packs::js_str_cmp(a, b));
    let mut kinds = builtin;
    kinds.extend(extra);
    Some(kinds)
}

/// `loadPackKinds(fallback)` with an explicit backend folder (`pack_kinds.json` in `here`, else the repo at
/// `here/..`, else `fallback`).
pub fn load_pack_kinds_from(here: &Path, fallback: &[&str]) -> Vec<String> {
    if let Ok(bytes) = std::fs::read(here.join(PACK_KINDS_FILE_NAME)) {
        if let Ok(written) = serde_json::from_str::<Value>(&String::from_utf8_lossy(&bytes)) {
            if let Some(Value::Array(kinds)) = crate::util::field(&written, "kinds") {
                let strings: Option<Vec<String>> =
                    kinds.iter().map(|kind| kind.as_str().filter(|k| is_kind(k)).map(str::to_string)).collect();
                if let Some(strings) = strings {
                    return strings;
                }
            }
        }
    }
    let absolute = std::fs::canonicalize(here).unwrap_or_else(|_| here.to_path_buf());
    let root = absolute.parent().map(Path::to_path_buf).unwrap_or(absolute);
    read_pack_kinds_from_repo(&root).unwrap_or_else(|| fallback.iter().map(|s| s.to_string()).collect())
}

/// `loadPackKinds(fallback)` from [`backend_dir`].
pub fn load_pack_kinds(fallback: &[&str]) -> Vec<String> {
    load_pack_kinds_from(&backend_dir(), fallback)
}

/// `JSON.stringify(value, null, 1)`.
#[cfg(test)]
fn stringify_indent1(value: &Value) -> String {
    fn write(value: &Value, depth: usize, out: &mut String) {
        let pad = |n: usize| " ".repeat(n);
        match value {
            Value::Array(items) if !items.is_empty() => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    write(item, depth + 1, out);
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(depth));
                out.push(']');
            }
            Value::Object(map) if !map.is_empty() => {
                out.push_str("{\n");
                for (i, (key, item)) in map.iter().enumerate() {
                    out.push_str(&pad(depth + 1));
                    out.push_str(&crate::util::stringify(&Value::String(key.clone())));
                    out.push_str(": ");
                    write(item, depth + 1, out);
                    out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(depth));
                out.push('}');
            }
            other => out.push_str(&crate::util::stringify(other)),
        }
    }
    let mut out = String::new();
    write(value, 0, &mut out);
    out
}

/// The `node backend/pack_kinds.mjs [--write]` CLI: Ok(stdout) or Err(stderr) (exit status 1). `here` is the backend
/// folder the kinds file is written to; the repo is `here/..`.
#[cfg(test)]
pub fn run_cli(here: &Path, write: bool) -> Result<String, String> {
    let absolute = std::fs::canonicalize(here).unwrap_or_else(|_| here.to_path_buf());
    let root = absolute.parent().map(Path::to_path_buf).unwrap_or_else(|| absolute.clone());
    let Some(kinds) = read_pack_kinds_from_repo(&root) else {
        return Err("pack_kinds: run from a repo checkout (game/config/content_registry.gd not found)".to_string());
    };
    let kinds_json = Value::Array(kinds.iter().cloned().map(Value::String).collect());
    if write {
        let file = here.join(PACK_KINDS_FILE_NAME);
        let mut wrapper = serde_json::Map::new();
        wrapper.insert("kinds".into(), kinds_json);
        std::fs::write(&file, format!("{}\n", stringify_indent1(&Value::Object(wrapper))))
            .map_err(|error| format!("pack_kinds: {error}"))?;
        Ok(format!("pack_kinds: wrote {} kinds to {}", kinds.len(), file.display()))
    } else {
        Ok(crate::util::stringify(&kinds_json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_packs::FALLBACK_PACK_KINDS;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "{label}-{}-{}",
            std::process::id(),
            crate::util::hex(&crate::util::random_bytes(6))
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_registry(root: &Path, body: &str) {
        std::fs::create_dir_all(root.join("src").join("config")).unwrap();
        std::fs::write(root.join("src").join("config").join("content_registry.gd"), body).unwrap();
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_checkout_that_is_not_a_repo_has_no_pack_kinds() {
        let root = temp_root("pack-kinds-empty");
        assert_eq!(read_pack_kinds_from_repo(&root), None);
        write_registry(&root, "const OTHER := 1\n");
        assert_eq!(read_pack_kinds_from_repo(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn built_in_kinds_stay_in_source_order_and_extension_kinds_are_sorted_after_them() {
        let root = temp_root("pack-kinds");
        write_registry(
            &root,
            &["const PACK_KINDS := [", &format!("\"props\", \"rooms\", \"{}\", \"9no\", \"Bad-Kind\", \"ok_extra\"", "a".repeat(33)), "]\n"]
                .join("\n"),
        );
        assert_eq!(read_pack_kinds_from_repo(&root), Some(strings(&["props", "rooms", "ok_extra"])));

        std::fs::create_dir_all(root.join("data").join("kinds")).unwrap();
        let declared = format!(r#"{{"zeta":{{}},"alpha":{{}},"props":{{}},"BAD":{{}},"1no":{{}},"{}":{{}}}}"#, "b".repeat(33));
        std::fs::write(root.join("data").join("kinds").join("zeta.json"), declared).unwrap();
        std::fs::write(root.join("data").join("kinds").join("broken.json"), "{ not json").unwrap();
        std::fs::write(root.join("data").join("kinds").join("notes.txt"), "{\"skip\":{}}").unwrap();
        assert_eq!(read_pack_kinds_from_repo(&root), Some(strings(&["props", "rooms", "ok_extra", "alpha", "zeta"])));
        let _ = std::fs::remove_dir_all(&root);
    }

    // Node reads the real checkout here (which has no src/config/content_registry.gd, so the original test fails in
    // this repo). The port builds its own checkout: <root>/backend + <root>/src/config + <root>/data/kinds.
    #[test]
    fn load_pack_kinds_uses_pack_kinds_json_otherwise_the_repo_and_never_the_fallback_while_either_exists() {
        let root = temp_root("pack-kinds-load");
        let backend = root.join("backend");
        std::fs::create_dir_all(&backend).unwrap();
        assert_eq!(load_pack_kinds_from(&backend, &["fallback_only"]), strings(&["fallback_only"]));

        write_registry(
            &root,
            "const PACK_KINDS := [\"props\", \"regions\", \"hunters\", \"grabs\", \"sounds\", \"ambience\", \"floors\", \"walls\", \"ceilings\", \"rooms\",\n\t\"deaths\", \"emotes\"]\n",
        );
        std::fs::create_dir_all(root.join("data").join("kinds")).unwrap();
        std::fs::write(
            root.join("data").join("kinds").join("night_watch.json"),
            r#"{"nw_offices":{},"nw_calls":{},"nw_buildings":{},"nw_casts":{},"nw_modes":{},"nw_rules":{},"nw_scares":{},"nw_sounds":{}}"#,
        )
        .unwrap();
        let loaded = load_pack_kinds_from(&backend, &["fallback_only"]);
        assert!(!loaded.is_empty());
        assert!(loaded.iter().all(|kind| is_kind(kind)));
        assert!(!loaded.contains(&"fallback_only".to_string()));
        let checkout = read_pack_kinds_from_repo(&root).unwrap();
        assert_eq!(loaded, checkout);
        let extras = ["nw_buildings", "nw_calls", "nw_casts", "nw_modes", "nw_offices", "nw_rules", "nw_scares", "nw_sounds"];
        let mut expected: Vec<String> = strings(FALLBACK_PACK_KINDS);
        expected.extend(strings(&extras));
        assert_eq!(checkout, expected);

        std::fs::write(backend.join("pack_kinds.json"), "{\n \"kinds\": [\n  \"props\",\n  \"zeta\"\n ]\n}\n").unwrap();
        assert_eq!(load_pack_kinds_from(&backend, &["fallback_only"]), strings(&["props", "zeta"]));
        std::fs::write(backend.join("pack_kinds.json"), r#"{"kinds":["props","Bad-Kind"]}"#).unwrap();
        assert_eq!(load_pack_kinds_from(&backend, &["fallback_only"]), checkout, "an invalid kinds file falls back to the repo");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_cli_prints_kinds_and_write_stores_them_beside_the_backend() {
        let root = temp_root("pack-kinds-cli");
        let backend = root.join("backend");
        std::fs::create_dir_all(&backend).unwrap();
        let bare = run_cli(&backend, false);
        assert!(bare.unwrap_err().contains("content_registry.gd not found"));

        write_registry(&root, "const PACK_KINDS := [\"props\", \"rooms\"]\n");
        std::fs::create_dir_all(root.join("data").join("kinds")).unwrap();
        std::fs::write(root.join("data").join("kinds").join("extra.json"), r#"{"zeta":{},"alpha":{}}"#).unwrap();
        let wrote = run_cli(&backend, true).unwrap();
        assert!(wrote.contains("wrote 4 kinds"));
        let stored_text = std::fs::read_to_string(backend.join("pack_kinds.json")).unwrap();
        assert_eq!(stored_text, "{\n \"kinds\": [\n  \"props\",\n  \"rooms\",\n  \"alpha\",\n  \"zeta\"\n ]\n}\n");
        let again = run_cli(&backend, false).unwrap();
        assert_eq!(again, r#"["props","rooms","alpha","zeta"]"#);
        assert_eq!(load_pack_kinds_from(&backend, &["fallback_only"]), strings(&["props", "rooms", "alpha", "zeta"]));
        let _ = std::fs::remove_dir_all(&root);
    }
}
