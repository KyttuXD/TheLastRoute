//! Files the network client keeps on disk, in the formats the Godot client writes.
//!
//! * The account session: `backend_client.gd` keeps `user://last_route_account.cfg` (a Godot
//!   `ConfigFile`; `--profile=NAME` makes it `last_route_account_NAME.cfg`). One section per
//!   server, `[account:<origin lowercased>]`, with `username` and `session_token`; a pre-per-server
//!   `[account]` section belongs to the primary server. Here the file lives in the port's per-user
//!   folder (`crate::app::paths::user_dir()`), same name and same text format.
//! * The guest name lease: the original stores `guest_name` / `guest_name_token` in
//!   `user://settings.cfg` under `[player]`. PORT DEVIATION: the port's settings are a JSON file
//!   that `Settings::save` rewrites with only its own keys, so the lease lives in
//!   `last_route_guest.cfg` (same `[player]` keys, ConfigFile format) beside it.
//!
//! Only what the original stores is stored: never the password, the room password or admin flags
//! (live admin verification is never restored from the editable local file).

use super::config::{BackendConfig, PRIMARY_BASE_URL};
use std::path::{Path, PathBuf};

pub const ACCOUNT_CONFIG_FILE: &str = "last_route_account.cfg";
pub const GUEST_CONFIG_FILE: &str = "last_route_guest.cfg";

/// A Godot `ConfigFile` restricted to what these files hold (string values; anything else is kept
/// verbatim so a file written by Godot round-trips).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigFile {
    sections: Vec<(String, Vec<(String, String)>)>,
}

/// Godot's `String.c_escape()` as `VariantWriter` uses it for strings.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unescape(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

impl ConfigFile {
    pub fn parse(text: &str) -> ConfigFile {
        let mut file = ConfigFile::default();
        let mut current: Option<usize> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let name = line[1..line.len() - 1].to_string();
                current = Some(match file.sections.iter().position(|(n, _)| *n == name) {
                    Some(i) => i,
                    None => {
                        file.sections.push((name, Vec::new()));
                        file.sections.len() - 1
                    }
                });
                continue;
            }
            let (Some(i), Some((k, v))) = (current, line.split_once('=')) else { continue };
            let (k, v) = (k.trim().to_string(), v.trim().to_string());
            let keys = &mut file.sections[i].1;
            match keys.iter_mut().find(|(key, _)| *key == k) {
                Some(slot) => slot.1 = v,
                None => keys.push((k, v)),
            }
        }
        file
    }

    pub fn load(path: &Path) -> Option<ConfigFile> {
        std::fs::read_to_string(path).ok().map(|t| ConfigFile::parse(&t))
    }

    /// Godot's layout: `[section]`, a blank line, `key=value` lines, a blank line between sections.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (i, (name, keys)) in self.sections.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!("[{name}]\n\n"));
            for (k, v) in keys {
                out.push_str(&format!("{k}={v}\n"));
            }
        }
        out
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        // the browser build writes no files (paths::WRITES_TO_DISK): the account and guest files are not kept there
        if !crate::shared::paths::WRITES_TO_DISK {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("cfg.tmp");
        std::fs::write(&temp, self.to_text())?;
        std::fs::rename(&temp, path)
    }

    pub fn has_section(&self, section: &str) -> bool {
        self.sections.iter().any(|(n, _)| n == section)
    }

    pub fn get_string(&self, section: &str, key: &str) -> Option<String> {
        let keys = &self.sections.iter().find(|(n, _)| n == section)?.1;
        let raw = &keys.iter().find(|(k, _)| k == key)?.1;
        unescape(raw).or_else(|| Some(raw.clone()))
    }

    pub fn set_string(&mut self, section: &str, key: &str, value: &str) {
        let i = match self.sections.iter().position(|(n, _)| n == section) {
            Some(i) => i,
            None => {
                self.sections.push((section.to_string(), Vec::new()));
                self.sections.len() - 1
            }
        };
        let keys = &mut self.sections[i].1;
        let v = escape(value);
        match keys.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = v,
            None => keys.push((key.to_string(), v)),
        }
    }
}

/// The per-user folder the game uses (`crate::app::paths::user_dir()`), or the working directory
/// when the system has none.
pub fn default_dir() -> PathBuf {
    crate::app::paths::user_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// The saved account session for one server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoredSession {
    pub username: String,
    pub session_token: String,
}

/// The browser build keeps the session in the page's localStorage (no files there: `paths::WRITES_TO_DISK`), one entry
/// per server, under this key prefix + the server's origin (+ `#profile` for a named profile).
pub const WEB_SESSION_KEY_PREFIX: &str = "last_route.account:";

/// The localStorage key of one server's saved session, e.g. `last_route.account:http://host:8105`.
pub fn web_session_key(config: &BackendConfig) -> String {
    let origin = config.session_section(); // "account:<origin>"
    let origin = origin.strip_prefix("account:").unwrap_or(&origin);
    if config.profile.is_empty() {
        format!("{WEB_SESSION_KEY_PREFIX}{origin}")
    } else {
        format!("{WEB_SESSION_KEY_PREFIX}{origin}#{}", config.profile)
    }
}

/// The stored text of a session: `{"username":"NAME","session_token":"TOKEN"}`. Nothing else is kept (never the password).
pub fn session_to_text(session: &StoredSession) -> String {
    serde_json::json!({ "username": session.username, "session_token": session.session_token }).to_string()
}

/// The session in stored text; empty, damaged or half-filled text is the signed-out state.
pub fn session_from_text(text: &str) -> StoredSession {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else { return StoredSession::default() };
    let field = |k: &str| value.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let session = StoredSession { username: field("username").trim().to_uppercase(), session_token: field("session_token") };
    if session.username.is_empty() || session.session_token.is_empty() {
        return StoredSession::default();
    }
    session
}

/// The saved session for `config` from a key/value store (`get` = the store's read).
pub fn load_session_kv(get: &dyn Fn(&str) -> Option<String>, config: &BackendConfig) -> StoredSession {
    get(&web_session_key(config)).map(|t| session_from_text(&t)).unwrap_or_default()
}

/// Saves the session for `config`; a session with no token or name is a clear. False when the store refused it.
pub fn save_session_kv(set: &mut dyn FnMut(&str, &str) -> bool, config: &BackendConfig, session: &StoredSession) -> bool {
    let text = if session.username.is_empty() || session.session_token.is_empty() { String::new() } else { session_to_text(session) };
    set(&web_session_key(config), &text)
}

/// Forgets the session for `config` (localStorage has no remove through the bridge: an empty text reads as signed out).
pub fn clear_session_kv(set: &mut dyn FnMut(&str, &str) -> bool, config: &BackendConfig) -> bool {
    set(&web_session_key(config), "")
}

#[cfg(target_arch = "wasm32")]
fn web_get(key: &str) -> Option<String> {
    crate::app::web_storage::get(key)
}
#[cfg(not(target_arch = "wasm32"))]
fn web_get(_key: &str) -> Option<String> {
    None
}
#[cfg(target_arch = "wasm32")]
fn web_set(key: &str, value: &str) -> bool {
    crate::app::web_storage::set(key, value)
}
#[cfg(not(target_arch = "wasm32"))]
fn web_set(_key: &str, _value: &str) -> bool {
    false
}

/// `last_route_account.cfg` in `dir`.
#[derive(Clone, Debug)]
pub struct AccountStore {
    pub path: PathBuf,
}

impl AccountStore {
    pub fn new(dir: &Path, config: &BackendConfig) -> AccountStore {
        AccountStore { path: dir.join(config.profile_file(ACCOUNT_CONFIG_FILE)) }
    }

    /// `_load_account_session`.
    pub fn load(&self, config: &BackendConfig) -> StoredSession {
        if !crate::shared::paths::WRITES_TO_DISK {
            return load_session_kv(&web_get, config);
        }
        let Some(file) = ConfigFile::load(&self.path) else { return StoredSession::default() };
        let mut section = config.session_section();
        if !file.has_section(&section) && config.base_url.trim_end_matches('/').eq_ignore_ascii_case(PRIMARY_BASE_URL) {
            section = "account".into(); // sessions saved before they were per server belong to the main server
        }
        StoredSession {
            username: file.get_string(&section, "username").unwrap_or_default().trim().to_uppercase(),
            session_token: file.get_string(&section, "session_token").unwrap_or_default(),
        }
    }

    /// `_save_account_session`: keeps the other servers' sessions.
    pub fn save(&self, config: &BackendConfig, session: &StoredSession) {
        if !crate::shared::paths::WRITES_TO_DISK {
            let _ = save_session_kv(&mut web_set, config, session);
            return;
        }
        let mut file = ConfigFile::load(&self.path).unwrap_or_default();
        let section = config.session_section();
        file.set_string(&section, "username", &session.username);
        file.set_string(&section, "session_token", &session.session_token);
        let _ = file.save(&self.path);
    }

    /// `_clear_account_session` (the file part).
    pub fn clear(&self, config: &BackendConfig) {
        if !crate::shared::paths::WRITES_TO_DISK {
            let _ = clear_session_kv(&mut web_set, config);
            return;
        }
        let mut file = ConfigFile::load(&self.path).unwrap_or_default();
        let section = config.session_section();
        file.set_string(&section, "username", "");
        file.set_string(&section, "session_token", "");
        if config.is_primary() && file.has_section("account") {
            file.set_string("account", "username", "");
            file.set_string("account", "session_token", "");
        }
        let _ = file.save(&self.path);
    }
}

/// The guest name lease (`Settings.guest_name` / `guest_name_token`).
#[derive(Clone, Debug)]
pub struct GuestStore {
    pub path: PathBuf,
}

impl GuestStore {
    pub fn new(dir: &Path, config: &BackendConfig) -> GuestStore {
        GuestStore { path: dir.join(config.profile_file(GUEST_CONFIG_FILE)) }
    }

    /// (name, token); empty strings until the first answer.
    pub fn load(&self) -> (String, String) {
        let Some(file) = ConfigFile::load(&self.path) else { return (String::new(), String::new()) };
        (
            file.get_string("player", "guest_name").unwrap_or_default(),
            file.get_string("player", "guest_name_token").unwrap_or_default(),
        )
    }

    /// `Settings.set_guest_name`.
    pub fn save(&self, name: &str, token: &str) {
        let mut file = ConfigFile::load(&self.path).unwrap_or_default();
        file.set_string("player", "guest_name", &name.to_uppercase());
        file.set_string("player", "guest_name_token", token);
        let _ = file.save(&self.path);
    }
}

/// `ModerationClient.device_id()`: a random id for this install kept in `user://device_id`
/// (32 hex chars); the server stores only its hash. Sent as `deviceId` on room registration.
pub fn device_id(dir: &Path) -> String {
    let path = dir.join("device_id");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let id = text.trim().to_string();
        if id.len() == 32 {
            return id;
        }
    }
    let id: String = super::random_bytes(16).iter().map(|b| format!("{b:02x}")).collect();
    if crate::shared::paths::WRITES_TO_DISK {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, &id);
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_stable() {
        let dir = scratch("device");
        let a = device_id(&dir);
        assert_eq!(a.len(), 32);
        assert_eq!(device_id(&dir), a);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lr_net_store_{name}_{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn godot_layout_round_trip() {
        let text = "[account]\n\nusername=\"OLD\"\nsession_token=\"t0\"\n\n[account:http://127.0.0.1:9]\n\nusername=\"A\\\"B\"\ncount=3\n";
        let f = ConfigFile::parse(text);
        assert_eq!(f.get_string("account", "username").as_deref(), Some("OLD"));
        assert_eq!(f.get_string("account:http://127.0.0.1:9", "username").as_deref(), Some("A\"B"));
        assert_eq!(f.get_string("account:http://127.0.0.1:9", "count").as_deref(), Some("3"));
        assert_eq!(ConfigFile::parse(&f.to_text()), f);
        assert!(f.to_text().starts_with("[account]\n\nusername=\"OLD\"\n"));
    }

    #[test]
    fn sessions_per_server_and_legacy_section() {
        let dir = scratch("acct");
        let primary = BackendConfig::primary();
        let local = BackendConfig::loopback(18105);
        std::fs::write(dir.join(ACCOUNT_CONFIG_FILE), "[account]\n\nusername=\"kyttu\"\nsession_token=\"legacy\"\n").unwrap();
        let store = AccountStore::new(&dir, &primary);
        assert_eq!(store.load(&primary), StoredSession { username: "KYTTU".into(), session_token: "legacy".into() });
        assert_eq!(store.load(&local), StoredSession::default());
        store.save(&local, &StoredSession { username: "LOCAL".into(), session_token: "abc".into() });
        assert_eq!(store.load(&local).session_token, "abc");
        assert_eq!(store.load(&primary).session_token, "legacy");
        store.clear(&primary);
        assert_eq!(store.load(&primary), StoredSession::default());
        assert_eq!(store.load(&local).username, "LOCAL");
        let text = std::fs::read_to_string(dir.join(ACCOUNT_CONFIG_FILE)).unwrap();
        assert!(text.contains("[account:http://127.0.0.1:18105]"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn web_session_key_format_per_server() {
        let primary = BackendConfig::primary();
        let local = BackendConfig::loopback(18105);
        assert_eq!(web_session_key(&primary), "last_route.account:http://lastroute.kyttuslab.org:8105");
        assert_eq!(web_session_key(&local), "last_route.account:http://127.0.0.1:18105");
        let mut named = BackendConfig::primary();
        named.profile = "two".into();
        assert_eq!(web_session_key(&named), "last_route.account:http://lastroute.kyttuslab.org:8105#two");
        assert_ne!(web_session_key(&primary), web_session_key(&local));
    }

    #[test]
    fn web_session_text_round_trip_and_damage() {
        let s = StoredSession { username: "KYTTU".into(), session_token: "tok\"en".into() };
        assert_eq!(session_from_text(&session_to_text(&s)), s);
        // only the name and the token are kept
        let text = session_to_text(&s);
        assert!(!text.contains("password"));
        // the name is canonical uppercase on the way in
        assert_eq!(session_from_text(r#"{"username":" kyttu ","session_token":"t"}"#).username, "KYTTU");
        // empty, damaged and half-filled text is signed out
        for bad in ["", "not json", "[]", r#"{"username":"A"}"#, r#"{"session_token":"t"}"#, r#"{"username":"","session_token":"t"}"#] {
            assert_eq!(session_from_text(bad), StoredSession::default(), "{bad:?}");
        }
    }

    #[test]
    fn web_session_load_save_clear_through_a_key_value_store() {
        use std::cell::RefCell;
        use std::collections::HashMap;
        let map: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
        let get = |k: &str| map.borrow().get(k).cloned();
        let mut set = |k: &str, v: &str| {
            map.borrow_mut().insert(k.to_string(), v.to_string());
            true
        };
        let primary = BackendConfig::primary();
        let local = BackendConfig::loopback(18105);
        assert_eq!(load_session_kv(&get, &primary), StoredSession::default(), "nothing stored: signed out");
        let a = StoredSession { username: "KYTTU".into(), session_token: "a".into() };
        let b = StoredSession { username: "LOCAL".into(), session_token: "b".into() };
        assert!(save_session_kv(&mut set, &primary, &a));
        assert!(save_session_kv(&mut set, &local, &b));
        assert_eq!(load_session_kv(&get, &primary), a);
        assert_eq!(load_session_kv(&get, &local), b);
        // clearing one server leaves the other
        assert!(clear_session_kv(&mut set, &primary));
        assert_eq!(load_session_kv(&get, &primary), StoredSession::default());
        assert_eq!(load_session_kv(&get, &local), b);
        // a session with no token is a clear, not a half entry
        assert!(save_session_kv(&mut set, &local, &StoredSession { username: "LOCAL".into(), session_token: String::new() }));
        assert_eq!(load_session_kv(&get, &local), StoredSession::default());
        // a refusing store (blocked storage) reports false and loads as signed out
        let mut refuse = |_: &str, _: &str| false;
        assert!(!save_session_kv(&mut refuse, &primary, &a));
        let none = |_: &str| None;
        assert_eq!(load_session_kv(&none, &primary), StoredSession::default());
    }

    #[test]
    fn guest_lease_and_profile() {
        let dir = scratch("guest");
        let mut c = BackendConfig::primary();
        c.profile = "two".into();
        let g = GuestStore::new(&dir, &c);
        assert!(g.path.ends_with("last_route_guest_two.cfg"));
        assert_eq!(g.load(), (String::new(), String::new()));
        g.save("r-4821", "tok");
        assert_eq!(g.load(), ("R-4821".into(), "tok".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
