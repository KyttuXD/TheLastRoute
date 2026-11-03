//! Account progress kept per profile name (backend_client.gd account_progress): the hunter
//! dossier discoveries. The original stores them on the account server and only for a
//! signed-in account ("discovery is deliberately account-only"); this offline build keeps them
//! in `profile.json` in the per-user folder, under the local profile name.

use std::collections::BTreeMap;
use std::sync::Mutex;

static CACHE: Mutex<Option<BTreeMap<String, Vec<String>>>> = Mutex::new(None);

/// Env var that moves `profile.json` into another folder, so screen checks can use a scratch
/// profile instead of the player's own file. Unset or empty keeps the per-user folder.
pub const PROFILE_DIR_ENV: &str = "LR_PROFILE_DIR";

/// The profile file: under `dir_override` when set and non-empty, otherwise `user_default`.
fn resolve_path(
    dir_override: Option<std::ffi::OsString>,
    user_default: impl FnOnce() -> Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    match dir_override.filter(|d| !d.is_empty()) {
        Some(dir) => Some(std::path::PathBuf::from(dir).join("profile.json")),
        None => user_default(),
    }
}

fn path() -> Option<std::path::PathBuf> {
    resolve_path(std::env::var_os(PROFILE_DIR_ENV), || {
        super::paths::user_file("profile.json")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn profile_dir_override_moves_profile_json() {
        let scratch = PathBuf::from("scratch").join("profile-dir");
        let got = resolve_path(Some(scratch.clone().into_os_string()), || {
            Some(PathBuf::from("USER").join("profile.json"))
        });
        assert_eq!(got, Some(scratch.join("profile.json")));
        assert_ne!(got, Some(PathBuf::from("USER").join("profile.json")));
    }

    #[test]
    fn profile_dir_default_is_unchanged_without_the_variable() {
        let user = PathBuf::from("USER").join("LastRoute").join("profile.json");
        assert_eq!(resolve_path(None, || Some(user.clone())), Some(user.clone()));
        // an empty value counts as unset
        assert_eq!(
            resolve_path(Some(std::ffi::OsString::new()), || Some(user.clone())),
            Some(user)
        );
    }

    #[test]
    fn profile_dir_env_name_is_stable() {
        assert_eq!(PROFILE_DIR_ENV, "LR_PROFILE_DIR");
    }
}

fn load() -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let Some(v) = path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        return out;
    };
    if let Some(users) = v["users"].as_object() {
        for (name, u) in users {
            let ids = u["hunter_discoveries"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            out.insert(name.clone(), ids);
        }
    }
    out
}

fn save(all: &BTreeMap<String, Vec<String>>) {
    let Some(p) = path() else { return };
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    let users: serde_json::Map<String, serde_json::Value> = all
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::json!({ "hunter_discoveries": v })))
        .collect();
    let v = serde_json::json!({ "schema": 1, "kind": "profile", "users": users });
    let _ = std::fs::write(p, serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// A profile counts as signed in when it has a name (the account panel's local sign-in).
pub fn signed_in(username: &str) -> bool {
    !username.trim().is_empty()
}

/// discovered_hunters(): sorted ids, empty when not signed in.
pub fn discovered(username: &str) -> Vec<String> {
    if !signed_in(username) {
        return vec![];
    }
    let mut c = CACHE.lock().unwrap();
    c.get_or_insert_with(load)
        .get(username)
        .cloned()
        .unwrap_or_default()
}

/// account_discover_hunter(): true when this is a new entry. Not on debug (admin) runs.
pub fn discover(username: &str, id: &str, debug_run: bool) -> bool {
    if debug_run
        || !signed_in(username)
        || !super::checkpoint::ENABLED.load(std::sync::atomic::Ordering::Relaxed)
    {
        return false;
    }
    let mut c = CACHE.lock().unwrap();
    let all = c.get_or_insert_with(load);
    let list = all.entry(username.to_string()).or_default();
    if list.iter().any(|d| d == id) {
        return false;
    }
    list.push(id.to_string());
    list.sort();
    save(all);
    crate::lr_info!("dossier: {username} discovered {id}");
    true
}
