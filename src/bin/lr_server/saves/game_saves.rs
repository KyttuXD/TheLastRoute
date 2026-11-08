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
//!
//! With a website upstream the network calls run without the store's lock: the router asks the store for an
//! `UpstreamStep` under the lock (allow-list, body shape, rate limit and a reserved write time), releases it, and runs
//! the website calls; PUTs of one session, game and slot are serialized by a per-key lock so newest-wins holds.

use crate::upstream::Upstream;
use crate::util::{self, js_number, num, sha256_hex, stringify, truthy, Clock, Reply, SessionLookup};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const SAVE_MAX_BYTES: usize = 64 * 1024;
/// Keep the pre-revision slot intact; beta world revisions use distinct slots.
pub const ALLOWED_SAVES: &[(&str, &[&str])] = &[
    ("last-route-godot", &["run", "run-world-1"]),
    ("last-route-rust", &["run", "run-world-1"]),
    ("night-watch-rust", &["progress"]),
];

/// Longest game id or slot path segment a save route accepts.
pub const SAVE_SEGMENT_MAX_LEN: usize = 64;

/// A save path segment (game id or slot): 1..=SAVE_SEGMENT_MAX_LEN of `[A-Za-z0-9_-]`. Checked before the segment is
/// compared with the allow-list or goes near a file or an upstream path.
pub fn save_segment_valid(segment: &str) -> bool {
    (1..=SAVE_SEGMENT_MAX_LEN).contains(&segment.len()) && segment.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

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
    /// username (local files) or sha256(token) (website) -> recent write times
    writes: HashMap<String, Vec<i64>>,
    /// (sha256(token), game, slot) -> the lock that serializes website PUTs of that slot. Read and written only while
    /// the store's own lock is held; the Arc is cloned out and locked after that lock is released.
    put_locks: HashMap<String, Arc<Mutex<()>>>,
}

/// What a save request does once the store's lock is released.
pub enum UpstreamStep<T> {
    /// No website: the store answers from its files, under its lock.
    Local,
    /// Already answered (unknown slot, bad body, rate limited).
    Answered(Reply),
    /// Go to the website with this, without the store's lock.
    Ready(T),
}

/// A website PUT that passed the checks under the store's lock and holds a reserved write time.
pub struct UpstreamPut {
    upstream: Upstream,
    /// The rate-limit key (sha256 of the session token).
    rate_key: String,
    /// The write time reserved in the rate window; released again when the save is not written.
    reserved_at: i64,
    /// The upload's time in game seconds.
    requested: f64,
    /// Serializes PUTs of the same session, game and slot.
    slot_lock: Arc<Mutex<()>>,
}

/// The end of a website PUT.
pub struct UpstreamPutOutcome {
    pub reply: Reply,
    /// The save was refused (newest wins) or the stored save could not be read: the reserved write time goes back.
    pub release: bool,
}

impl UpstreamPut {
    /// Load, newest-wins and save on the website, under the per-slot lock only. Superseded run-world slots are pruned
    /// after that lock is released.
    pub fn run(&self, token: &str, game_id: &str, slot: &str, body: &Value) -> UpstreamPutOutcome {
        let held = self.slot_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (status, stored) =
            self.upstream.call("/api/v1/saves/load", &json!({ "sessionToken": token, "gameId": game_id, "slotId": slot }));
        if status != 200 || stored.get("ok") != Some(&json!(true)) {
            return UpstreamPutOutcome { reply: (status, stored), release: true };
        }
        let stored_at = stored_client_seconds(&stored);
        if let Some(reply) = newest_wins_reply(game_id, slot, stored_at, self.requested) {
            return UpstreamPutOutcome { reply, release: true };
        }
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        let (status, saved) = self
            .upstream
            .call("/api/v1/saves/save", &json!({ "sessionToken": token, "gameId": game_id, "slotId": slot, "data": data }));
        drop(held);
        if status != 200 || saved.get("ok") != Some(&json!(true)) {
            return UpstreamPutOutcome { reply: (status, saved), release: false };
        }
        prune_older_revisions(&self.upstream, token, game_id, slot);
        let updated_at = if saved.get("savedAt").is_some() { saved_seconds(saved.get("savedAt")) } else { self.requested };
        UpstreamPutOutcome { reply: (200, json!({ "ok": true, "gameId": game_id, "slot": slot, "updatedAt": num(updated_at) })), release: false }
    }
}

/// GET through the website, run without the store's lock (the slot was checked under it).
pub fn upstream_get(upstream: &Upstream, token: &str, game_id: &str, slot: &str) -> Reply {
    let (status, website) = upstream.call("/api/v1/saves/load", &json!({ "sessionToken": token, "gameId": game_id, "slotId": slot }));
    if status != 200 || website.get("ok") != Some(&json!(true)) {
        return (status, website);
    }
    (200, loaded_save(game_id, slot, &website))
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

/// The client's own stamp inside a stored run (`data.saved_at`, unix seconds), when it is a positive number.
fn client_stamp(data: Option<&Value>) -> Option<f64> {
    let stamp = js_number(data?.get("saved_at"));
    (stamp.is_finite() && stamp > 0.0).then(|| stamp.floor())
}

/// The time newest-wins compares an upload with, in the client's clock: the stored run's `saved_at`, falling back to
/// the website's `savedAt` only when the run has none. A cleared slot (data null or missing) is 0, so any upload wins.
/// The website stamps its own clock on every save, so comparing the client's `updatedAt` with `savedAt` would mix two
/// clocks and refuse a newer run whenever the website's clock is ahead.
fn stored_client_seconds(website: &Value) -> f64 {
    match website.get("data") {
        None | Some(Value::Null) => 0.0,
        data => client_stamp(data).unwrap_or_else(|| saved_seconds(website.get("savedAt"))),
    }
}

/// The newest-wins answer to a PUT against the stored time (both in game seconds): an upload older than the
/// stored save is not written and answers 200 with the stored time. None when the upload may be written.
fn newest_wins_reply(game_id: &str, slot: &str, stored_at: f64, requested: f64) -> Option<Reply> {
    if stored_at <= requested {
        return None;
    }
    Some((200, json!({ "ok": true, "gameId": game_id, "slot": slot, "updatedAt": num(stored_at) })))
}

/// The game's shape for a slot the website returned from `/api/v1/saves/load`. `updatedAt` is the run's own
/// `saved_at` (the stamp newest-wins compares), or the website's `savedAt` when the run has none.
fn loaded_save(game_id: &str, slot: &str, website: &Value) -> Value {
    let updated_at = client_stamp(website.get("data")).unwrap_or_else(|| saved_seconds(website.get("savedAt")));
    json!({
        "ok": true, "gameId": game_id, "slot": slot,
        "data": website.get("data").cloned().unwrap_or(Value::Null),
        "updatedAt": num(updated_at),
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
            put_locks: HashMap::new(),
        }
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    /// Removes every cloud save of a deleted account. True when a file was removed. With an upstream the website owns the
    /// saves and deletes them with the account, so nothing happens here.
    pub fn delete_account(&mut self, username: &str) -> bool {
        if !self.enabled || self.upstream.is_some() || !username_file(username) {
            return false;
        }
        self.writes.remove(username);
        std::fs::remove_file(self.path(username)).is_ok()
    }

    /// Moves a renamed account's cloud saves to its new name (`<from>.json` -> `<to>.json`) with its write times. A
    /// `<to>.json` already there is a stale orphan (no live account held that name) and is removed first, never merged.
    /// Nothing happens with an upstream (the website owns the saves), with saves off, for an unchanged name, or for a
    /// name that is not an account file. A failed move is logged and never fails the rename.
    pub fn rename_account(&mut self, from: &str, to: &str) {
        if !self.enabled || self.upstream.is_some() || from == to || !username_file(from) || !username_file(to) {
            return;
        }
        if let Some(times) = self.writes.remove(from) {
            self.writes.insert(to.to_string(), times);
        }
        let (source, target) = (self.path(from), self.path(to));
        if !source.exists() {
            return;
        }
        if target.exists() {
            if let Err(error) = std::fs::remove_file(&target) {
                util::log_warn(&format!("[saves] could not remove the stale save file of {to}: {error}"));
            }
        }
        match std::fs::rename(&source, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => util::log_warn(&format!("[saves] could not move the saves of {from} to {to}: {error}")),
        }
    }

    /// Drops the rate-limit entries with no write left inside the window, and the per-slot PUT locks nobody holds.
    pub fn prune_writes(&mut self) {
        let now = self.now();
        let window = self.write_window_ms;
        self.writes.retain(|_, times| {
            times.retain(|at| now - at < window);
            !times.is_empty()
        });
        self.put_locks.retain(|_, lock| Arc::strong_count(lock) > 1);
    }

    /// The website to read a slot from, checked under the store's lock; the call itself runs after it is released.
    pub fn begin_upstream_get(&self, game_id: &str, slot: &str) -> UpstreamStep<Upstream> {
        let Some(upstream) = &self.upstream else { return UpstreamStep::Local };
        if !self.slot_allowed(game_id, slot) {
            return UpstreamStep::Answered(unknown_save());
        }
        UpstreamStep::Ready(upstream.clone())
    }

    /// The checks of a website PUT, under the store's lock: allow-list, body shape and rate limit. A write time is
    /// reserved in the rate window now (so concurrent PUTs count) and given back with `release_write` when the save is
    /// not written.
    pub fn begin_upstream_put(&mut self, token: &str, game_id: &str, slot: &str, body: &Value) -> UpstreamStep<UpstreamPut> {
        let Some(upstream) = self.upstream.clone() else { return UpstreamStep::Local };
        if !self.slot_allowed(game_id, slot) {
            return UpstreamStep::Answered(unknown_save());
        }
        if !well_formed_save(body) {
            return UpstreamStep::Answered(bad_save());
        }
        let now = self.now();
        let rate_key = sha256_hex(token.as_bytes());
        let mut recent = self.recent_writes(&rate_key, now);
        if recent.len() >= self.write_limit {
            return UpstreamStep::Answered(save_rate_limited());
        }
        recent.push(now);
        self.writes.insert(rate_key.clone(), recent);
        let slot_lock = self.put_locks.entry(format!("{rate_key}\n{game_id}\n{slot}")).or_default().clone();
        UpstreamStep::Ready(UpstreamPut { upstream, rate_key, reserved_at: now, requested: updated_at_of(body, now), slot_lock })
    }

    /// Gives back the write time a website PUT reserved (the save was refused or the stored one could not be read).
    pub fn release_write(&mut self, put: &UpstreamPut) {
        let Some(times) = self.writes.get_mut(&put.rate_key) else { return };
        if let Some(index) = times.iter().position(|at| *at == put.reserved_at) {
            times.remove(index);
        }
        if times.is_empty() {
            self.writes.remove(&put.rate_key);
        }
    }

    #[allow(dead_code)]
    /// The slots a game's saves may use (the website's slot list for that game); empty for an unknown game.
    pub fn slots_of(&self, game_id: &str) -> Vec<String> {
        self.allowed.iter().find(|(game, _)| game == game_id).map(|(_, slots)| slots.clone()).unwrap_or_default()
    }

    fn slot_allowed(&self, game_id: &str, slot: &str) -> bool {
        save_segment_valid(game_id)
            && save_segment_valid(slot)
            && self.allowed.iter().any(|(game, slots)| game == game_id && slots.iter().any(|s| s == slot))
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
    /// With an upstream this holds `&self` across the website call; the router uses `begin_upstream_get` instead.
    pub fn get(&self, accounts: &dyn SessionLookup, token: &str, game_id: &str, slot: &str) -> Reply {
        match self.begin_upstream_get(game_id, slot) {
            UpstreamStep::Local => {}
            UpstreamStep::Answered(reply) => return reply,
            UpstreamStep::Ready(upstream) => return upstream_get(&upstream, token, game_id, slot),
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

    /// PUT /api/v1/saves/:gameId/:slot. `body` = the parsed JSON body `{data, updatedAt}` (the router answers 413 for
    /// bodies over SAVE_MAX_BYTES and 400 BAD_SAVE "INVALID JSON BODY" for unparsable ones before calling this).
    /// With an upstream the stored run's own `saved_at` is read first and an older upload is answered 200 with that time
    /// and not written (newest wins). This holds `&mut self` across the website calls; the router runs the same steps
    /// (`begin_upstream_put`, `UpstreamPut::run`, `release_write`) without the store's lock.
    pub fn put(&mut self, accounts: &dyn SessionLookup, token: &str, game_id: &str, slot: &str, body: &Value) -> Reply {
        match self.begin_upstream_put(token, game_id, slot, body) {
            UpstreamStep::Local => {}
            UpstreamStep::Answered(reply) => return reply,
            UpstreamStep::Ready(put) => {
                let outcome = put.run(token, game_id, slot, body);
                if outcome.release {
                    self.release_write(&put);
                }
                return outcome.reply;
            }
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

    #[test]
    fn save_segments_are_validated_before_the_allow_list() {
        assert!(save_segment_valid("run-world-1") && save_segment_valid("last-route-rust"));
        for bad in ["", "..", "a/b", "a b", "run.json", "%2e%2e", &"x".repeat(SAVE_SEGMENT_MAX_LEN + 1)] {
            assert!(!save_segment_valid(bad), "{bad:?} must be refused");
        }
        let f = fixture_with(true, None);
        assert_eq!(f.saves.get(&f.accounts, "token", "last-route-rust", "..").0, 404);
        assert_eq!(f.saves.get(&f.accounts, "token", "..", "run").0, 404);
        assert!(f.saves.slot_allowed("last-route-rust", "run"));
        assert!(!f.saves.slot_allowed("last-route-rust", &"x".repeat(SAVE_SEGMENT_MAX_LEN + 1)));
    }

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
    fn deleting_an_account_removes_its_cloud_saves_and_the_slot_list_is_the_website_one() {
        let mut server = fixture();
        let kt = server.sign_up("GONE_SAVER");
        assert_eq!(server.put(PATH, &kt, json!({ "data": run(-1), "updatedAt": 100 })).0, 200);
        let file = server.root.join("saves").join("GONE_SAVER.json");
        assert!(file.exists());
        assert!(server.saves.delete_account("GONE_SAVER"));
        assert!(!file.exists(), "the account's saves are gone");
        assert!(!server.saves.delete_account("GONE_SAVER"), "nothing left to delete");
        assert!(!server.saves.delete_account("../ESCAPE"), "a name that is not an account file is never touched");
        assert_eq!(server.saves.slots_of("last-route-rust"), vec!["run".to_string(), "run-world-1".to_string()]);
        assert!(server.saves.slots_of("not-a-game").is_empty());
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
    fn a_superseded_run_world_is_deleted_on_the_website_with_the_session_game_and_slot() {
        use crate::upstream::test_website::fake_website;
        let (upstream, seen) = fake_website(vec![
            r#"{"ok":true,"slots":[{"slotId":"run-world-1"},{"slotId":"run-world-2"},{"slotId":"run-world-3"}]}"#,
            r#"{"ok":true}"#,
        ]);
        prune_older_revisions(&upstream, "session-token", "last-route-godot", "run-world-3");
        // Only run-world-1 goes: run-world-2 is the newest older revision and stays beside the one just saved.
        assert_eq!(
            seen.join().unwrap(),
            vec![
                ("/api/v1/saves/list".to_string(), json!({ "sessionToken": "session-token", "gameId": "last-route-godot" })),
                (
                    "/api/v1/saves/delete".to_string(),
                    json!({ "sessionToken": "session-token", "gameId": "last-route-godot", "slotId": "run-world-1" })
                ),
            ]
        );
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
    fn renaming_an_account_moves_its_saves_and_replaces_a_stale_orphan() {
        let mut server = fixture();
        let ada = server.sign_up("ADA_SAVER");
        assert_eq!(server.put(PATH, &ada, json!({ "data": run(-7), "updatedAt": 100 })).0, 200);
        let dir = server.root.join("saves");
        std::fs::write(dir.join("BEA_SAVER.json"), r#"{"games":{"last-route-godot":{"run":{"data":{"stale":true},"updatedAt":999}}}}"#).unwrap();
        server.saves.rename_account("ADA_SAVER", "BEA_SAVER");
        assert!(!dir.join("ADA_SAVER.json").exists(), "the old name keeps no saves");
        let moved: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("BEA_SAVER.json")).unwrap()).unwrap();
        assert_eq!(moved["games"]["last-route-godot"]["run"]["data"], run(-7), "the orphan was replaced, not merged");
        assert!(server.saves.writes.contains_key("BEA_SAVER") && !server.saves.writes.contains_key("ADA_SAVER"));
        // no file to move, the same name, a name that is not an account file: nothing happens and nothing fails
        server.saves.rename_account("NOBODY_HERE", "SOMEONE");
        server.saves.rename_account("BEA_SAVER", "BEA_SAVER");
        server.saves.rename_account("BEA_SAVER", "../ESCAPE");
        assert!(dir.join("BEA_SAVER.json").exists() && !dir.join("SOMEONE.json").exists());
    }

    #[test]
    fn rename_is_a_no_op_with_an_upstream_or_without_saves() {
        let upstream = crate::upstream::Upstream::parse("http://127.0.0.1:9").unwrap();
        let mut saves = GameSaves::new(GameSavesOptions { upstream: Some(upstream), ..Default::default() });
        saves.writes.insert("ADA".into(), vec![1]);
        saves.rename_account("ADA", "BEA");
        assert!(saves.writes.contains_key("ADA"), "the website owns renamed saves");
        let mut off = fixture_with(false, None);
        off.saves.rename_account("ADA", "BEA");
    }

    #[test]
    fn upstream_newest_wins_compares_the_client_stamp_not_the_website_clock() {
        use crate::upstream::test_website::fake_website;
        // The website's clock (savedAt, ms) is far ahead of every client stamp.
        let (upstream, seen) = fake_website(vec![
            r#"{"ok":true,"data":{"saved_at":100},"savedAt":9999999000000}"#,
            r#"{"ok":true,"savedAt":9999999000000}"#,
            r#"{"ok":true,"data":{"saved_at":300},"savedAt":9999999000000}"#,
            r#"{"ok":true,"data":null,"savedAt":9999999000000}"#,
            r#"{"ok":true}"#,
        ]);
        let mut saves = GameSaves::new(GameSavesOptions { upstream: Some(upstream), ..Default::default() });
        let accounts = AccountService::new(AccountServiceOptions::default());
        let newer = saves.put(&accounts, "token", "last-route-rust", "run", &json!({ "data": run(-1), "updatedAt": 200 }));
        assert_eq!(newer.0, 200, "a later client stamp is written although the website clock is ahead");
        let older = saves.put(&accounts, "token", "last-route-rust", "run", &json!({ "data": run(-2), "updatedAt": 250 }));
        assert_eq!(older, (200, json!({ "ok": true, "gameId": "last-route-rust", "slot": "run", "updatedAt": 300 })), "an earlier one is refused");
        let after_clear = saves.put(&accounts, "token", "last-route-rust", "run", &json!({ "data": run(-3), "updatedAt": 5 }));
        assert_eq!((after_clear.0, after_clear.1["updatedAt"].clone()), (200, json!(5)), "a cleared slot takes any upload");
        let paths: Vec<String> = seen.join().unwrap().into_iter().map(|(path, _)| path).collect();
        assert_eq!(paths, ["/api/v1/saves/load", "/api/v1/saves/save", "/api/v1/saves/load", "/api/v1/saves/load", "/api/v1/saves/save"]);
        // the refused upload gave its reserved write time back: two writes are counted
        assert_eq!(saves.writes.get(&sha256_hex(b"token")).map(Vec::len), Some(2));
    }

    #[test]
    fn a_loaded_website_save_reports_the_client_stamp() {
        let loaded = loaded_save("last-route-rust", "run", &json!({ "ok": true, "data": { "saved_at": 42.7 }, "savedAt": 9_000_000 }));
        assert_eq!(loaded["updatedAt"].as_f64(), Some(42.0));
        let unstamped = loaded_save("last-route-rust", "run", &json!({ "ok": true, "data": { "saved_at": 0 }, "savedAt": 9_000 }));
        assert_eq!(unstamped["updatedAt"].as_f64(), Some(9.0), "no client stamp falls back to the website time");
        assert_eq!(stored_client_seconds(&json!({ "ok": true, "data": null, "savedAt": 9_000 })), 0.0);
        assert_eq!(stored_client_seconds(&json!({ "ok": true, "savedAt": 9_000 })), 0.0);
    }

    #[test]
    fn prune_writes_drops_accounts_with_no_write_inside_the_window() {
        let time = Arc::new(std::sync::atomic::AtomicI64::new(1_000_000));
        let clock_time = time.clone();
        let now: Clock = Arc::new(move || clock_time.load(std::sync::atomic::Ordering::SeqCst));
        let mut saves = GameSaves::new(GameSavesOptions { now, write_window_ms: 60_000, ..Default::default() });
        saves.writes.insert("OLD".into(), vec![900_000]);
        saves.writes.insert("MIXED".into(), vec![900_000, 990_000]);
        saves.writes.insert("FRESH".into(), vec![999_000]);
        let held = Arc::new(Mutex::new(()));
        saves.put_locks.insert("held".into(), held.clone());
        saves.put_locks.insert("idle".into(), Arc::new(Mutex::new(())));
        saves.prune_writes();
        assert!(!saves.writes.contains_key("OLD"));
        assert_eq!(saves.writes["MIXED"], vec![990_000]);
        assert!(saves.writes.contains_key("FRESH"));
        assert!(saves.put_locks.contains_key("held") && !saves.put_locks.contains_key("idle"));
        time.store(2_000_000, std::sync::atomic::Ordering::SeqCst);
        saves.prune_writes();
        assert!(saves.writes.is_empty(), "the map shrinks once the window passes");
        drop(held);
    }

    #[test]
    fn without_a_save_directory_the_route_answers_503_not_a_crash() {
        let server = fixture_with(false, None);
        let kt = server.sign_up("NO_SAVE_DIR");
        assert_eq!(server.get(PATH, &kt).0, 503);
        assert!(!server.saves.enabled);
    }
}
