//! Port of `backend/game_saves.mjs`.
//!
//! Account cloud saves for the Godot games (Last Route's one-run autosave, RunSaves in the client). One JSON record per
//! account, stored as <directory>/<USERNAME>.json (`{"games":{gameId:{slot:{data,updatedAt}}}}`) and replaced
//! atomically (temp file + rename).
//!
//!   GET /api/v1/saves/:gameId/:slot   header x-session-token            -> {ok, data|null, updatedAt}
//!   PUT /api/v1/saves/:gameId/:slot   header x-session-token, {data, updatedAt}
//!                                                                       -> {ok, updatedAt} | 409 STALE {updatedAt}
//!
//! Only allow-listed game/slot pairs exist. `data: null` deletes the slot (a run that ended). Newest wins: a PUT whose
//! updatedAt (unix seconds, the client's saved_at) is older than the stored one is refused with 409 and the stored time,
//! so a stale offline copy can never overwrite a newer run. Bodies are capped (SAVE_MAX_BYTES, enforced by the router
//! before parsing) and writes rate-limited per account.

use crate::upstream::Upstream;
use crate::util::{self, js_number, num, sha256_hex, stringify, truthy, Clock, Reply, SessionLookup};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;

pub const SAVE_MAX_BYTES: usize = 64 * 1024;
/// Keep the pre-revision slot intact; beta world revisions use distinct slots.
pub const ALLOWED_SAVES: &[(&str, &[&str])] = &[
    ("last-route-godot", &["run", "run-world-1"]),
    ("last-route-rust", &["run", "run-world-1"]),
    ("night-watch-rust", &["progress"]),
];

/// `SAVE_PATH.exec(pathname)` -> `(gameId, slot)` (raw, never decoded).
pub fn match_save_path(pathname: &str) -> Option<(String, String)> {
    let rest = pathname.strip_prefix("/api/v1/saves/")?;
    let (game_id, slot) = rest.split_once('/')?;
    if game_id.is_empty() || slot.is_empty() || slot.contains('/') {
        return None;
    }
    Some((game_id.to_string(), slot.to_string()))
}

/// `/^[A-Z0-9_]{1,32}$/`.
fn username_file(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

pub struct GameSavesOptions {
    /// `directory` (null = saves disabled, every call answers 503).
    pub directory: Option<PathBuf>,
    pub now: Clock,
    /// gameId -> allowed slots.
    pub allowed: Vec<(String, Vec<String>)>,
    pub write_limit: usize,
    pub write_window_ms: i64,
    /// `LASTROUTE_ACCOUNT_UPSTREAM`: when set, the saves live on the website and `directory` is not used.
    pub upstream: Option<Upstream>,
}

impl Default for GameSavesOptions {
    fn default() -> Self {
        GameSavesOptions {
            directory: None,
            now: util::system_clock(),
            allowed: ALLOWED_SAVES
                .iter()
                .map(|(game, slots)| (game.to_string(), slots.iter().map(|s| s.to_string()).collect()))
                .collect(),
            write_limit: 30,
            upstream: None,
            write_window_ms: 60_000,
        }
    }
}

pub struct GameSaves {
    pub enabled: bool,
    directory: PathBuf,
    now: Clock,
    allowed: Vec<(String, Vec<String>)>,
    write_limit: usize,
    write_window_ms: i64,
    upstream: Option<Upstream>,
    /// username -> recent write times
    writes: HashMap<String, Vec<i64>>,
}

fn unknown_save() -> Reply {
    (404, json!({ "ok": false, "code": "UNKNOWN_SAVE", "error": "No such save" }))
}

fn bad_save() -> Reply {
    (400, json!({ "ok": false, "code": "BAD_SAVE", "error": "A save is {data: object|null, updatedAt}" }))
}

fn save_rate_limited() -> Reply {
    (429, json!({ "ok": false, "code": "SAVE_RATE_LIMIT", "error": "Too many saves; try again shortly" }))
}

/// A well-formed save body: `data` is an object or null.
fn well_formed_save(body: &Value) -> bool {
    match body {
        Value::Object(map) => matches!(map.get("data"), Some(Value::Null) | Some(Value::Object(_))),
        _ => false,
    }
}

/// `updatedAt` of a PUT in unix seconds; a missing or unusable time is the server's clock.
fn updated_at_of(body: &Value, now: i64) -> f64 {
    let requested = js_number(body.get("updatedAt"));
    if requested.is_finite() && requested > 0.0 {
        requested.floor()
    } else {
        (now as f64 / 1000.0).floor()
    }
}

/// The website's `savedAt` (Date.now() milliseconds) in the game's unit, unix seconds.
fn saved_seconds(saved_at: Option<&Value>) -> f64 {
    (util::number_or(saved_at, 0.0) / 1000.0).floor()
}

/// The newest-wins answer to a PUT against the website's stored time (both in game seconds): an upload older than the
/// stored save is not written and answers 200 with the stored time. None when the upload may be written.
fn newest_wins_reply(game_id: &str, slot: &str, stored_at: f64, requested: f64) -> Option<Reply> {
    if stored_at <= requested {
        return None;
    }
    Some((200, json!({ "ok": true, "gameId": game_id, "slot": slot, "updatedAt": num(stored_at) })))
}

/// The game's shape for a slot the website returned from `/api/v1/saves/load`.
fn loaded_save(game_id: &str, slot: &str, website: &Value) -> Value {
    json!({
        "ok": true, "gameId": game_id, "slot": slot,
        "data": website.get("data").cloned().unwrap_or(Value::Null),
        "updatedAt": num(saved_seconds(website.get("savedAt"))),
    })
}

/// The revision of a `run-world-N` slot; None for any other slot.
fn run_world_revision(slot: &str) -> Option<u32> {
    slot.strip_prefix("run-world-")?.parse().ok()
}

/// The older `run-world-N` slots to delete once `slot` was saved: every revision below it except the newest of them, so
/// at most one older revision remains beside the one just written.
fn superseded_run_worlds(slots: &[String], slot: &str) -> Vec<String> {
    let Some(current) = run_world_revision(slot) else { return Vec::new() };
    let mut older: Vec<(u32, &String)> = slots
        .iter()
        .filter_map(|name| run_world_revision(name).filter(|revision| *revision < current).map(|revision| (revision, name)))
        .collect();
    older.sort_by(|a, b| b.0.cmp(&a.0));
    older.into_iter().skip(1).map(|(_, name)| name.clone()).collect()
}

/// Deletes the superseded run-world slots of the account on the website. A failed list or delete is logged and never
/// fails the save that already went through.
fn prune_older_revisions(upstream: &Upstream, token: &str, game_id: &str, slot: &str) {
    if run_world_revision(slot).is_none() {
        return;
    }
    let (status, listed) = upstream.call("/api/v1/saves/list", &json!({ "sessionToken": token, "gameId": game_id }));
    if status != 200 || listed.get("ok") != Some(&json!(true)) {
        util::log_warn(&format!("[saves] could not list {game_id} saves to prune behind {slot}: status {status}"));
        return;
    }
    let names: Vec<String> = listed
        .get("slots")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(|row| row.get("slotId")?.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    for old in superseded_run_worlds(&names, slot) {
        let (status, deleted) =
            upstream.call("/api/v1/saves/delete", &json!({ "sessionToken": token, "gameId": game_id, "slotId": old }));
        if status != 200 || deleted.get("ok") != Some(&json!(true)) {
            util::log_warn(&format!("[saves] could not delete superseded {game_id} slot {old}: status {status}"));
        }
    }
}

fn off() -> Reply {
    (503, json!({ "ok": false, "code": "SAVES_DISABLED", "error": "Cloud saves are not configured" }))
}

fn empty_record() -> Value {
    json!({ "games": {} })
}

impl GameSaves {
    pub fn new(opts: GameSavesOptions) -> GameSaves {
        let enabled = opts.directory.as_ref().is_some_and(|d| !d.as_os_str().is_empty());
        let directory = opts.directory.unwrap_or_default();
        if enabled {
            if let Err(error) = std::fs::create_dir_all(&directory) {
                util::log_error(&format!("[saves] cannot create {}: {error}", directory.display()));
            }
        }
        GameSaves {
            enabled,
            directory,
            now: opts.now,
            allowed: opts.allowed,
            write_limit: opts.write_limit,
            write_window_ms: opts.write_window_ms,
            upstream: opts.upstream,
            writes: HashMap::new(),
        }
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn slot_allowed(&self, game_id: &str, slot: &str) -> bool {
        self.allowed.iter().any(|(game, slots)| game == game_id && slots.iter().any(|s| s == slot))
    }

    fn who(&self, accounts: &dyn SessionLookup, token: &str) -> String {
        match accounts.session_username(token) {
            Some(name) if username_file(&name) => name,
            _ => String::new(),
        }
    }

    fn path(&self, username: &str) -> PathBuf {
        self.directory.join(format!("{username}.json"))
    }

    fn read(&self, username: &str) -> Value {
        let path = self.path(username);
        if !path.exists() {
            return empty_record();
        }
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|e| e.to_string()));
        match parsed {
            Ok(value) => match value.get("games") {
                Some(Value::Object(_)) => value,
                _ => empty_record(),
            },
            Err(message) => {
                util::log_error(&format!("[saves] unreadable save file for {username}: {message}"));
                empty_record()
            }
        }
    }

    fn write(&self, username: &str, record: &Value) -> std::io::Result<()> {
        util::write_atomic(&self.path(username), stringify(record).as_bytes())
    }

    /// The write times of `key` still inside the rate window.
    fn recent_writes(&self, key: &str, now: i64) -> Vec<i64> {
        self.writes.get(key).map(|w| w.iter().copied().filter(|at| now - at < self.write_window_ms).collect()).unwrap_or_default()
    }

    fn guard(&self, accounts: &dyn SessionLookup, token: &str, game_id: &str, slot: &str) -> Result<String, Reply> {
        if !self.enabled {
            return Err(off());
        }
        if !self.slot_allowed(game_id, slot) {
            return Err(unknown_save());
        }
        let username = self.who(accounts, token);
        if username.is_empty() {
            return Err((401, json!({ "ok": false, "code": "NOT_SIGNED_IN", "error": "Sign in to sync saves" })));
        }
        Ok(username)
    }

    fn stored<'a>(record: &'a Value, game_id: &str, slot: &str) -> Option<&'a Value> {
        record.get("games")?.get(game_id)?.get(slot)
    }

    /// GET /api/v1/saves/:gameId/:slot. `token` = the `x-session-token` header (`String(header || "")`).
    pub fn get(&self, accounts: &dyn SessionLookup, token: &str, game_id: &str, slot: &str) -> Reply {
        if let Some(upstream) = &self.upstream {
            if !self.slot_allowed(game_id, slot) {
                return unknown_save();
            }
            let (status, website) =
                upstream.call("/api/v1/saves/load", &json!({ "sessionToken": token, "gameId": game_id, "slotId": slot }));
            if status != 200 || website.get("ok") != Some(&json!(true)) {
                return (status, website);
            }
            return (200, loaded_save(game_id, slot, &website));
        }
        let username = match self.guard(accounts, token, game_id, slot) {
            Ok(username) => username,
            Err(refused) => return refused,
        };
        let record = self.read(&username);
        let stored = Self::stored(&record, game_id, slot);
        let data = stored.and_then(|s| s.get("data")).cloned().unwrap_or(Value::Null);
        let updated_at = util::number_or(stored.and_then(|s| s.get("updatedAt")), 0.0);
        (200, json!({ "ok": true, "gameId": game_id, "slot": slot, "data": data, "updatedAt": num(updated_at) }))
    }

    /// A PUT through the website. The stored `savedAt` is read first: an upload older than it is answered 200 with the
    /// stored time and not written (newest wins), because the website stamps its own time on every save.
    fn put_upstream(&mut self, upstream: &Upstream, token: &str, game_id: &str, slot: &str, body: &Value) -> Reply {
        if !self.slot_allowed(game_id, slot) {
            return unknown_save();
        }
        if !well_formed_save(body) {
            return bad_save();
        }
        let now = self.now();
        let key = sha256_hex(token.as_bytes());
        let mut recent = self.recent_writes(&key, now);
        if recent.len() >= self.write_limit {
            return save_rate_limited();
        }
        let requested = updated_at_of(body, now);
        let (status, stored) =
            upstream.call("/api/v1/saves/load", &json!({ "sessionToken": token, "gameId": game_id, "slotId": slot }));
        if status != 200 || stored.get("ok") != Some(&json!(true)) {
            return (status, stored);
        }
        let stored_at = saved_seconds(stored.get("savedAt"));
        if let Some(reply) = newest_wins_reply(game_id, slot, stored_at, requested) {
            return reply;
        }
        recent.push(now);
        self.writes.insert(key, recent);
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        let (status, saved) = upstream
            .call("/api/v1/saves/save", &json!({ "sessionToken": token, "gameId": game_id, "slotId": slot, "data": data }));
        if status != 200 || saved.get("ok") != Some(&json!(true)) {
            return (status, saved);
        }
        prune_older_revisions(upstream, token, game_id, slot);
        let updated_at = if saved.get("savedAt").is_some() { saved_seconds(saved.get("savedAt")) } else { requested };
        (200, json!({ "ok": true, "gameId": game_id, "slot": slot, "updatedAt": num(updated_at) }))
    }

    /// PUT /api/v1/saves/:gameId/:slot. `body` = the parsed JSON body `{data, updatedAt}` (the router answers 413 for
    /// bodies over SAVE_MAX_BYTES and 400 BAD_SAVE "INVALID JSON BODY" for unparsable ones before calling this).
    pub fn put(&mut self, accounts: &dyn SessionLookup, token: &str, game_id: &str, slot: &str, body: &Value) -> Reply {
        if let Some(upstream) = self.upstream.clone() {
            return self.put_upstream(&upstream, token, game_id, slot, body);
        }
        let username = match self.guard(accounts, token, game_id, slot) {
            Ok(username) => username,
            Err(refused) => return refused,
        };
        if !well_formed_save(body) {
            return bad_save();
        }
        let now = self.now();
        let mut recent = self.recent_writes(&username, now);
        if recent.len() >= self.write_limit {
            return save_rate_limited();
        }
        let updated_at = updated_at_of(body, now);
        let mut record = self.read(&username);
        if let Some(stored) = Self::stored(&record, game_id, slot).filter(|s| truthy(Some(s))) {
            let stored_at = js_number(stored.get("updatedAt"));
            if stored_at > updated_at {
                return (
                    409,
                    json!({ "ok": false, "code": "STALE", "error": "A newer save is already stored", "updatedAt": num(stored_at) }),
                );
            }
        }
        recent.push(now);
        self.writes.insert(username.clone(), recent);
        let games = record.get_mut("games").and_then(Value::as_object_mut).expect("read() returns an object `games`");
        let game = games.entry(game_id.to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !game.is_object() {
            *game = Value::Object(Map::new());
        }
        let game = game.as_object_mut().unwrap();
        match body.get("data") {
            Some(Value::Null) | None => {
                game.shift_remove(slot);
            }
            Some(data) => {
                game.insert(slot.to_string(), json!({ "data": data, "updatedAt": num(updated_at) }));
            }
        }
        if let Err(error) = self.write(&username, &record) {
            util::log_error(&format!("[saves] write failed for {username}: {error}"));
            return (500, json!({ "ok": false, "code": "SAVE_WRITE_FAILED", "error": "The save could not be stored" }));
        }
        (200, json!({ "ok": true, "gameId": game_id, "slot": slot, "updatedAt": num(updated_at) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_service::{AccountService, AccountServiceOptions};
    use crate::util::{hex, random_bytes};

    const PATH: &str = "/api/v1/saves/last-route-godot/run";

    struct Fixture {
        root: PathBuf,
        accounts: AccountService,
        saves: GameSaves,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn fixture_with(directory: bool, write_limit: Option<usize>) -> Fixture {
        let root = std::env::temp_dir().join(format!("saves-{}", hex(&random_bytes(6))));
        std::fs::create_dir_all(&root).unwrap();
        let accounts = AccountService::new(AccountServiceOptions {
            store_path: Some(root.join("accounts.json")),
            ..Default::default()
        });
        let mut options = GameSavesOptions {
            directory: if directory { Some(root.join("saves")) } else { None },
            ..Default::default()
        };
        if let Some(limit) = write_limit {
            options.write_limit = limit;
            options.write_window_ms = 60_000;
        }
        Fixture { root, accounts, saves: GameSaves::new(options) }
    }

    fn fixture() -> Fixture {
        fixture_with(true, None)
    }

    impl Fixture {
        fn sign_up(&self, username: &str) -> String {
            let (status, body) = self.accounts.register(&json!({ "username": username, "password": "correct horse battery" }), false);
            assert_eq!(status, 200);
            body["sessionToken"].as_str().unwrap().to_string()
        }
        fn get(&self, path: &str, token: &str) -> Reply {
            match match_save_path(path) {
                Some((game, slot)) => self.saves.get(&self.accounts, token, &game, &slot),
                None => (404, Value::Null),
            }
        }
        fn put(&mut self, path: &str, token: &str, body: Value) -> Reply {
            match match_save_path(path) {
                Some((game, slot)) => self.saves.put(&self.accounts, token, &game, &slot, &body),
                None => (404, Value::Null),
            }
        }
    }

    fn run(level: i64) -> Value {
        json!({ "format": 2, "saved_at": 100, "place": format!("LEVEL {level}"), "payload": { "seed_text": "S", "level": level } })
    }

    #[test]
    fn a_signed_in_account_stores_reads_and_deletes_its_run_accounts_never_see_each_others() {
        let mut server = fixture();
        let kt = server.sign_up("KYTTU_SAVER");
        let other = server.sign_up("OTHER_SAVER");
        assert_eq!(server.get(PATH, &kt).1["data"], Value::Null);
        let put = server.put(PATH, &kt, json!({ "data": run(-2), "updatedAt": 100 }));
        assert_eq!(put.0, 200);
        assert_eq!(put.1["updatedAt"], 100);
        let got = server.get(PATH, &kt);
        assert_eq!(got.0, 200);
        assert_eq!(got.1["data"], run(-2));
        assert_eq!(got.1["updatedAt"], 100);
        assert_eq!(server.get(PATH, &other).1["data"], Value::Null, "another account read KT's run");
        assert!(server.root.join("saves").join("KYTTU_SAVER.json").exists());
        let leftovers = std::fs::read_dir(server.root.join("saves"))
            .unwrap()
            .any(|e| e.unwrap().file_name().to_string_lossy().ends_with(".tmp"));
        assert!(!leftovers, "a temp file was left behind");
        let cleared = server.put(PATH, &kt, json!({ "data": null, "updatedAt": 150 }));
        assert_eq!(cleared.0, 200);
        assert_eq!(server.get(PATH, &kt).1["data"], Value::Null, "a cleared run came back");
        // the file format is the Node one
        let text = std::fs::read_to_string(server.root.join("saves").join("KYTTU_SAVER.json")).unwrap();
        assert_eq!(text, r#"{"games":{"last-route-godot":{}}}"#);
    }

    #[test]
    fn newest_wins_an_older_upload_is_refused_with_the_stored_time() {
        let mut server = fixture();
        let kt = server.sign_up("NEWEST_WINS");
        server.put(PATH, &kt, json!({ "data": run(-5), "updatedAt": 500 }));
        let stale = server.put(PATH, &kt, json!({ "data": run(-1), "updatedAt": 400 }));
        assert_eq!(stale.0, 409);
        assert_eq!(stale.1["code"], "STALE");
        assert_eq!(stale.1["updatedAt"], 500);
        assert_eq!(server.get(PATH, &kt).1["data"], run(-5));
        assert_eq!(server.put(PATH, &kt, json!({ "data": run(-6), "updatedAt": 500 })).0, 200, "an equal time is accepted");
    }

    #[test]
    fn beta_generation_slot_is_isolated_from_the_preserved_old_run() {
        let mut server = fixture();
        let current = "/api/v1/saves/last-route-godot/run-world-1";
        let kt = server.sign_up("WORLD_SAVER");
        let old = run(-2);
        let mut fresh = run(-3);
        fresh["world_generation"] = json!(1);
        assert_eq!(server.put(PATH, &kt, json!({ "data": old, "updatedAt": 100 })).0, 200);
        assert_eq!(server.put(current, &kt, json!({ "data": fresh, "updatedAt": 200 })).0, 200);
        assert_eq!(server.get(PATH, &kt).1["data"], old);
        assert_eq!(server.get(current, &kt).1["data"], fresh);
        assert_eq!(server.put(current, &kt, json!({ "data": null, "updatedAt": 300 })).0, 200);
        assert_eq!(server.get(PATH, &kt).1["data"], old);
        assert_eq!(server.put(current, &kt, json!({ "data": fresh, "updatedAt": 400 })).0, 200);
        assert_eq!(server.put(PATH, &kt, json!({ "data": null, "updatedAt": 500 })).0, 200);
        assert_eq!(server.get(current, &kt).1["data"], fresh);
        for slot in ["run-world-0", "run-world-2", "run-world-999", "run-world-"] {
            let path = format!("/api/v1/saves/last-route-godot/{slot}");
            assert_eq!(server.get(&path, &kt).0, 404);
            assert_eq!(server.put(&path, &kt, json!({ "data": fresh, "updatedAt": 600 })).0, 404);
        }
    }

    #[test]
    fn saves_need_a_session_an_allow_listed_game_slot_a_well_formed_body() {
        let mut server = fixture();
        let kt = server.sign_up("GUARDED_SAVES");
        assert_eq!(server.get(PATH, "").0, 401);
        assert_eq!(server.put(PATH, "not-a-session", json!({ "data": run(-1) })).0, 401);
        assert_eq!(server.get("/api/v1/saves/last-route-godot/slot2", &kt).0, 404);
        assert_eq!(server.put("/api/v1/saves/other-game/run", &kt, json!({ "data": run(-1) })).0, 404);
        assert_eq!(server.get("/api/v1/saves/..%2F..%2Faccounts/run", &kt).0, 404);
        assert_eq!(server.put(PATH, &kt, json!({ "updatedAt": 1 })).0, 400, "a body without data");
        assert_eq!(server.put(PATH, &kt, json!({ "data": [1, 2] })).0, 400, "an array is not a save");
        assert_eq!(server.put(PATH, &kt, json!([1])).0, 400, "an array body is not a save");
        // ("{not json" -> 400 and the SAVE_MAX_BYTES 413 cap are enforced by the router before put() is called.)
        assert_eq!(server.get(PATH, &kt).1["data"], Value::Null);
        assert_eq!(match_save_path("/api/v1/saves/a/b/c"), None);
        assert_eq!(match_save_path("/api/v1/saves/a/b"), Some(("a".into(), "b".into())));
    }

    #[test]
    fn writes_are_rate_limited_per_account() {
        let mut server = fixture_with(true, Some(3));
        let kt = server.sign_up("RATE_SAVES");
        for i in 1..=3 {
            assert_eq!(server.put(PATH, &kt, json!({ "data": run(-i), "updatedAt": i })).0, 200);
        }
        let limited = server.put(PATH, &kt, json!({ "data": run(-4), "updatedAt": 4 }));
        assert_eq!(limited.0, 429);
        assert_eq!(limited.1["code"], "SAVE_RATE_LIMIT");
        assert_eq!(server.get(PATH, &kt).0, 200, "reads are not write-limited");
    }

    #[test]
    fn with_an_upstream_saves_go_to_the_website_and_an_unreachable_one_answers_503() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let upstream = crate::upstream::Upstream::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let mut saves = GameSaves::new(GameSavesOptions { upstream: Some(upstream), ..Default::default() });
        let accounts = AccountService::new(AccountServiceOptions::default());
        let (status, body) = saves.get(&accounts, "token", "last-route-rust", "run");
        assert_eq!((status, body["code"].clone()), (503, json!("UPSTREAM_UNAVAILABLE")));
        assert_eq!(saves.get(&accounts, "token", "last-route-rust", "slot2").0, 404);
        assert_eq!(saves.put(&accounts, "token", "last-route-rust", "run", &json!({ "updatedAt": 1 })).0, 400);
        assert_eq!(saves.put(&accounts, "token", "last-route-rust", "run", &json!({ "data": run(-1), "updatedAt": 100 })).0, 503);
    }

    #[test]
    fn the_website_save_maps_to_the_game_shape_in_unix_seconds() {
        assert_eq!(saved_seconds(Some(&json!(1_700_000_123_456i64))), 1_700_000_123.0);
        assert_eq!(saved_seconds(None), 0.0);
        assert_eq!(updated_at_of(&json!({ "updatedAt": 400 }), 9_000), 400.0);
        assert_eq!(updated_at_of(&json!({}), 9_999), 9.0);
        let loaded = loaded_save("last-route-rust", "run", &json!({ "ok": true, "data": null, "savedAt": 2_000 }));
        assert_eq!(loaded["updatedAt"].as_f64(), Some(2.0));
        assert_eq!((loaded["gameId"].clone(), loaded["slot"].clone(), loaded["data"].clone()), (json!("last-route-rust"), json!("run"), Value::Null));
    }

    #[test]
    fn website_savedat_milliseconds_convert_to_game_seconds_both_ways() {
        // ms -> seconds: the sub-second part is dropped, so a stamp lands in the whole second it falls in.
        assert_eq!(saved_seconds(Some(&json!(999))), 0.0);
        assert_eq!(saved_seconds(Some(&json!(1000))), 1.0);
        assert_eq!(saved_seconds(Some(&json!(1_700_000_123_999i64))), 1_700_000_123.0);
        // seconds -> ms: a game second stored as s * 1000 (plus any sub-second part) reads back as s, and one
        // millisecond earlier reads back as the second before.
        for seconds in [1.0, 60.0, 1_700_000_123.0] {
            for sub_second_ms in [0.0, 1.0, 999.0] {
                assert_eq!(saved_seconds(Some(&num(seconds * 1000.0 + sub_second_ms))), seconds);
            }
            assert_eq!(saved_seconds(Some(&num(seconds * 1000.0 - 1.0))), seconds - 1.0);
        }
    }

    #[test]
    fn newest_wins_refuses_an_older_upload_with_200_and_the_stored_seconds() {
        let stored_at = saved_seconds(Some(&json!(500_999)));
        assert_eq!(stored_at, 500.0);
        let refused = newest_wins_reply("last-route-rust", "run", stored_at, 400.0).expect("an older upload is refused");
        assert_eq!(refused, (200, json!({ "ok": true, "gameId": "last-route-rust", "slot": "run", "updatedAt": 500 })));
        assert_eq!(newest_wins_reply("last-route-rust", "run", stored_at, 500.0), None, "an equal time is written");
        assert_eq!(newest_wins_reply("last-route-rust", "run", stored_at, 501.0), None, "a newer upload is written");
    }

    #[test]
    fn after_a_run_world_save_only_one_older_revision_survives() {
        let slots: Vec<String> = ["run", "run-world-1", "run-world-2", "run-world-3"].iter().map(|s| s.to_string()).collect();
        assert_eq!(superseded_run_worlds(&slots, "run-world-3"), vec!["run-world-1".to_string()]);
        assert!(superseded_run_worlds(&slots, "run-world-2").is_empty());
        assert!(superseded_run_worlds(&slots, "run").is_empty());
    }

    #[test]
    fn without_a_save_directory_the_route_answers_503_not_a_crash() {
        let server = fixture_with(false, None);
        let kt = server.sign_up("NO_SAVE_DIR");
        assert_eq!(server.get(PATH, &kt).0, 503);
        assert!(!server.saves.enabled);
    }
}
