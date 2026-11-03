//! Port of `backend/public_route.mjs`: the persistent, server-owned PUBLIC route.
//!
//! Gameplay itself is still the Godot ENet host-authoritative session; this module is the part that must outlive any
//! one host: the world seed, relays collected, claimed items (which respawn on a timer), the last sanitized hunter
//! snapshot, per-player resume records, the UTC day and the player-scaled hunter target. Whichever client holds the
//! ENet host token for room code PUBLIC is the authority and may post snapshots/claims; everyone may post their own
//! position and inventory. All input is clamped and rate limited here, never trusted.
//!
//! Differences forced by the synchronous port:
//! - `load()` / `flush()` are blocking.
//! - The JS 1.5 s debounced save (`setTimeout`) becomes a pending-save deadline on the injected clock. It is
//!   written by `tick()` (call it periodically, e.g. from the router's maintenance loop, about once a second) and
//!   also opportunistically at the start of `join` / `post` / `leave` / `status`. `flush()` writes immediately.

use crate::util::{
    base64url, field, iso_time, js_len, js_number, js_slice, js_string_of, js_trim, log_error, log_info, log_warn,
    num, random_bytes, str_or_empty, stringify, system_clock, truthy, write_atomic, Clock, Reply,
};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Five characters from the room-code alphabet: the relay wire format carries exactly five. Players see it as
/// "PUBLIC ROUTE", never as a code to type.
pub const PUBLIC_CODE: &str = "PUBLC";
/// Each game has its own public room: its own code, seed, store file and state. (game id, code), in JS key order.
pub const PUBLIC_CODES: [(&str, &str); 2] = [("last_route", PUBLIC_CODE), ("city", "CTYPR")];
pub const PUBLIC_RESUME_TTL_MS: i64 = 6 * 60 * 60 * 1000;
pub const PUBLIC_ITEM_RESPAWN_MS: i64 = 3 * 60 * 1000;
pub const PUBLIC_JOIN_GRACE_MS: i64 = 10_000;
pub const PUBLIC_PLAYER_TTL_MS: i64 = 20_000;
pub const PUBLIC_AUTHORITY_STALE_MS: i64 = 15_000;
pub const PUBLIC_HUNTER_CAP: i64 = 8;
const SNAPSHOT_MAX_CHARS: usize = 400_000;
const WORLD_LIMIT: f64 = 1_000_000.0;
const MAX_CLAIMS: usize = 20_000;
const STATE_MIN_INTERVAL_MS: i64 = 250;
const JOIN_LIMIT_PER_MINUTE: u32 = 20;
const SAVE_DEBOUNCE_MS: i64 = 1500;

/// `PUBLIC_CODES[game]`.
pub fn public_code_for(game: &str) -> Option<&'static str> {
    PUBLIC_CODES.iter().find(|(id, _)| *id == game).map(|(_, code)| *code)
}

fn player_key_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-f0-9]{16,48}$").unwrap())
}

fn claim_id_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // JS `\d` is ASCII-only; Rust's is Unicode, so spell it out.
    RE.get_or_init(|| Regex::new(r"^-?[0-9]+:-?[0-9]+:-?[0-9]+:PICKUP:[A-Z_]{1,16}:[0-9]{1,6}$").unwrap())
}

/// `utcDayKey(ms)`: "YYYY-MM-DD" of the UTC day.
pub fn utc_day_key(ms: i64) -> String {
    iso_time(ms)[..10].to_string()
}

/// 1 + floor((players - 1) / 2), clamped to 1..cap (same rule as the 2D build).
pub fn public_hunter_target_count(players: f64) -> i64 {
    let n = if players == 0.0 || players.is_nan() { 1.0 } else { players };
    let count = 1f64.max(n.floor());
    let target = 1.0 + ((count - 1.0) / 2.0).floor();
    1f64.max((PUBLIC_HUNTER_CAP as f64).min(target)) as i64
}

fn finite(value: Option<&Value>, fallback: f64, low: f64, high: f64) -> f64 {
    let number = js_number(value);
    if !number.is_finite() {
        return fallback;
    }
    low.max(high.min(number))
}

/// JavaScript `Math.round` (halves round toward +Infinity).
fn js_math_round(f: f64) -> f64 {
    if !f.is_finite() {
        return f;
    }
    let floor = f.floor();
    if f - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `sanitizePosition(value)`: `{ x, z }` rounded to centimetres, or None.
pub fn sanitize_position(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    let x = js_number(field(value, "x"));
    let z = js_number(field(value, "z"));
    if !x.is_finite() || !z.is_finite() || x.abs() > WORLD_LIMIT || z.abs() > WORLD_LIMIT {
        return None;
    }
    Some(json!({ "x": num(js_math_round(x * 100.0) / 100.0), "z": num(js_math_round(z * 100.0) / 100.0) }))
}

/// `sanitizeInventory(value)`: always a JSON array of `{ id, durability, slot }`.
pub fn sanitize_inventory(value: Option<&Value>) -> Value {
    let Some(Value::Array(entries)) = value else { return Value::Array(Vec::new()) };
    let mut result = Vec::new();
    for entry in entries.iter().take(24) {
        if !truthy(Some(entry)) || !(entry.is_object() || entry.is_array()) {
            continue;
        }
        let id = str_or_empty(field(entry, "id"));
        let valid = (1..=24).contains(&id.chars().count()) && id.chars().all(|c| c.is_ascii_lowercase() || c == '_');
        if !valid {
            continue;
        }
        result.push(json!({
            "id": id,
            "durability": num(finite(field(entry, "durability"), 0.0, 0.0, 1000.0)),
            "slot": num(finite(field(entry, "slot"), 0.0, 0.0, 64.0).trunc()),
        }));
    }
    Value::Array(result)
}

fn clean_name(value: Option<&Value>) -> String {
    let upper = str_or_empty(value).to_uppercase();
    let kept: String =
        upper.chars().filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, ' ' | '_' | '-')).collect();
    let name = js_slice(js_trim(&kept), 32);
    if name.is_empty() {
        "RUNNER".to_string()
    } else {
        name
    }
}

/// A string-keyed map that iterates in insertion order like a JS `Map` (re-setting a key keeps its position,
/// deleting and re-adding moves it to the end).
struct OrderedMap<V> {
    entries: HashMap<String, (u64, V)>,
    order: BTreeMap<u64, String>,
    next: u64,
}

impl<V> OrderedMap<V> {
    fn new() -> Self {
        OrderedMap { entries: HashMap::new(), order: BTreeMap::new(), next: 0 }
    }
    fn len(&self) -> usize {
        self.entries.len()
    }
    fn get(&self, key: &str) -> Option<&V> {
        self.entries.get(key).map(|(_, value)| value)
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.entries.get_mut(key).map(|(_, value)| value)
    }
    fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }
    fn insert(&mut self, key: String, value: V) {
        if let Some(slot) = self.entries.get_mut(&key) {
            slot.1 = value;
            return;
        }
        let seq = self.next;
        self.next += 1;
        self.order.insert(seq, key.clone());
        self.entries.insert(key, (seq, value));
    }
    fn remove(&mut self, key: &str) -> Option<V> {
        let (seq, value) = self.entries.remove(key)?;
        self.order.remove(&seq);
        Some(value)
    }
    fn keys(&self) -> Vec<String> {
        self.order.values().cloned().collect()
    }
    fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.order.values().map(move |key| (key, &self.entries[key].1))
    }
}

#[derive(Clone, Debug)]
struct Claim {
    kind: String,
    claimed_at: f64,
}

#[derive(Clone, Debug)]
struct Resume {
    position: Value,
    inventory: Value,
    saved_at: f64,
}

#[derive(Clone, Debug)]
struct LivePlayer {
    session_id: String,
    player_key: String,
    name: String,
    joined_at: i64,
    last_seen: i64,
    last_state_at: i64,
    position: Option<Value>,
    inventory: Value,
}

struct State {
    seed: String,
    /// bumps when the route is rebuilt from scratch
    epoch: i64,
    /// pickup id -> { kind, claimedAt }
    claims: OrderedMap<Claim>,
    /// opaque, authority-posted, size-limited
    snapshot: String,
    snapshot_at: f64,
    hunter_target: i64,
    /// playerKey -> { position, inventory, savedAt }
    resumes: OrderedMap<Resume>,
}

#[derive(Clone, Debug, Default)]
struct Authority {
    session_id: String,
    last_beat: i64,
}

/// The `{ status, body }` / `{ status, error }` objects `join` / `post` / `leave` return.
#[derive(Clone, Debug, PartialEq)]
pub struct PublicResult {
    pub status: u16,
    pub body: Option<Value>,
    pub error: Option<String>,
}

impl PublicResult {
    fn ok(body: Value) -> Self {
        PublicResult { status: 200, body: Some(body), error: None }
    }
    fn fail(status: u16, error: &str) -> Self {
        PublicResult { status, body: None, error: Some(error.to_string()) }
    }
    /// What server.mjs sends: `{ ok: false, error }` with the status when there is an error, otherwise the body.
    pub fn to_reply(&self) -> Reply {
        match &self.error {
            Some(error) => (self.status, json!({ "ok": false, "error": error })),
            None => (self.status, self.body.clone().unwrap_or(Value::Null)),
        }
    }
}

/// The second argument of `join(body, { address, hostTokenValid, enetListed })`.
#[derive(Clone, Debug, Default)]
pub struct JoinOptions {
    pub address: String,
    /// Does the caller hold the ENet host token for PUBLIC right now?
    pub host_token_valid: bool,
    pub enet_listed: bool,
}

pub struct PublicRouteOptions {
    pub now: Clock,
    pub store_path: Option<PathBuf>,
    pub initial_seed: String,
    pub code: String,
    pub item_respawn_ms: i64,
    pub resume_ttl_ms: i64,
}

impl Default for PublicRouteOptions {
    fn default() -> Self {
        PublicRouteOptions {
            now: system_clock(),
            store_path: None,
            initial_seed: "PUBLIC-ROUTE".into(),
            code: PUBLIC_CODE.into(),
            item_respawn_ms: PUBLIC_ITEM_RESPAWN_MS,
            resume_ttl_ms: PUBLIC_RESUME_TTL_MS,
        }
    }
}

pub struct PublicRoute {
    now: Clock,
    store_path: Option<PathBuf>,
    code: String,
    item_respawn_ms: i64,
    resume_ttl_ms: i64,
    state: State,
    /// sessionId -> live player
    players: OrderedMap<LivePlayer>,
    /// address -> (count, resetAt)
    join_rates: HashMap<String, (u32, i64)>,
    authority: Authority,
    /// When the debounced save is due (the JS `saveTimer`).
    save_due_at: Option<i64>,
    loaded: bool,
}

/// `const [a, b] = entry` for an entry of a saved array: arrays and strings are iterable, anything else throws.
fn destructure_pair(entry: &Value) -> Result<(Option<Value>, Option<Value>), String> {
    match entry {
        Value::Array(items) => Ok((items.first().cloned(), items.get(1).cloned())),
        Value::String(text) => {
            let mut chars = text.chars();
            Ok((chars.next().map(|c| json!(c.to_string())), chars.next().map(|c| json!(c.to_string()))))
        }
        other => Err(format!("{} is not iterable", js_string_of(other))),
    }
}

/// `PATTERN.test(value)` (the value is coerced to a string; undefined -> "undefined").
fn key_text(value: &Option<Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(v) => js_string_of(v),
    }
}

impl PublicRoute {
    pub fn new(opts: PublicRouteOptions) -> PublicRoute {
        PublicRoute {
            now: opts.now,
            store_path: opts.store_path,
            code: opts.code,
            item_respawn_ms: opts.item_respawn_ms,
            resume_ttl_ms: opts.resume_ttl_ms,
            state: State {
                seed: opts.initial_seed,
                epoch: 1,
                claims: OrderedMap::new(),
                snapshot: String::new(),
                snapshot_at: 0.0,
                hunter_target: 1,
                resumes: OrderedMap::new(),
            },
            players: OrderedMap::new(),
            join_rates: HashMap::new(),
            authority: Authority::default(),
            save_due_at: None,
            loaded: false,
        }
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    /// The route's room code (`PUBLC` / `CTYPR`).
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The current authority snapshot (`_state.snapshot` in the JS tests).
    pub fn snapshot(&self) -> &str {
        &self.state.snapshot
    }

    /// Restore the saved state once (missing file = fresh route).
    pub fn load(&mut self) {
        let Some(path) = self.store_path.clone() else { return };
        if self.loaded {
            return;
        }
        self.loaded = true;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                log_warn(&format!("PUBLIC route state could not be loaded: {error}"));
                return;
            }
        };
        let saved: Value = match serde_json::from_str(&text) {
            Ok(saved) => saved,
            Err(error) => {
                log_warn(&format!("PUBLIC route state could not be loaded: {error}"));
                return;
            }
        };
        if let Err(message) = self.restore(&saved) {
            log_warn(&format!("PUBLIC route state could not be loaded: {message}"));
        }
    }

    fn restore(&mut self, saved: &Value) -> Result<(), String> {
        if saved.is_null() {
            return Err("Cannot read properties of null (reading 'seed')".into());
        }
        if let Some(Value::String(seed)) = field(saved, "seed") {
            if !seed.is_empty() {
                self.state.seed = js_slice(seed, 64);
            }
        }
        let or_one = |value: Option<&Value>| {
            let n = js_number(value);
            if n == 0.0 || n.is_nan() {
                1.0
            } else {
                n
            }
        };
        self.state.epoch = 1f64.max(or_one(field(saved, "epoch")).trunc()) as i64;
        self.state.hunter_target =
            1f64.max((PUBLIC_HUNTER_CAP as f64).min(or_one(field(saved, "hunterTarget")).trunc())) as i64;
        if let Some(Value::String(snapshot)) = field(saved, "snapshot") {
            if js_len(snapshot) <= SNAPSHOT_MAX_CHARS {
                self.state.snapshot = snapshot.clone();
                let at = js_number(field(saved, "snapshotAt"));
                self.state.snapshot_at = if at.is_nan() { 0.0 } else { at };
            }
        }
        if let Some(Value::Array(claims)) = field(saved, "claims") {
            for entry in claims {
                let (id, claim) = destructure_pair(entry)?;
                let id = key_text(&id);
                if claim_id_pattern().is_match(&id) && self.state.claims.len() < MAX_CLAIMS {
                    let claim = claim.unwrap_or(Value::Null);
                    let kind = js_slice(&str_or_empty(field(&claim, "kind")), 16);
                    let at = js_number(field(&claim, "claimedAt"));
                    let claimed_at = if at == 0.0 || at.is_nan() { self.now() as f64 } else { at };
                    self.state.claims.insert(id, Claim { kind, claimed_at });
                }
            }
        }
        let timestamp = self.now();
        if let Some(Value::Array(resumes)) = field(saved, "resumes") {
            for entry in resumes {
                let (key, resume) = destructure_pair(entry)?;
                let resume = resume.unwrap_or(Value::Null);
                let position = sanitize_position(field(&resume, "position"));
                let key = key_text(&key);
                let saved_at = js_number(field(&resume, "savedAt"));
                if let Some(position) = position {
                    if player_key_pattern().is_match(&key) && timestamp as f64 - saved_at <= self.resume_ttl_ms as f64 {
                        let inventory = sanitize_inventory(field(&resume, "inventory"));
                        self.state.resumes.insert(key, Resume { position, inventory, saved_at });
                    }
                }
            }
        }
        log_info(&format!(
            "PUBLIC route restored: {} claims, {} resumes",
            self.state.claims.len(),
            self.state.resumes.len()
        ));
        Ok(())
    }

    fn serialise(&self) -> Value {
        let claims: Vec<Value> = self
            .state
            .claims
            .iter()
            .map(|(id, claim)| json!([id, { "kind": claim.kind, "claimedAt": num(claim.claimed_at) }]))
            .collect();
        let resumes: Vec<Value> = self
            .state
            .resumes
            .iter()
            .map(|(key, resume)| {
                json!([key, { "position": resume.position, "inventory": resume.inventory, "savedAt": num(resume.saved_at) }])
            })
            .collect();
        json!({
            "version": 1,
            "seed": self.state.seed,
            "epoch": self.state.epoch,
            "hunterTarget": self.state.hunter_target,
            "snapshot": self.state.snapshot,
            "snapshotAt": num(self.state.snapshot_at),
            "claims": claims,
            "resumes": resumes,
        })
    }

    fn save_now(&self) {
        let Some(path) = &self.store_path else { return };
        if let Err(error) = write_atomic(path, stringify(&self.serialise()).as_bytes()) {
            log_error(&format!("PUBLIC route state could not be saved: {error}"));
        }
    }

    fn schedule_save(&mut self) {
        if self.store_path.is_none() || self.save_due_at.is_some() {
            return;
        }
        self.save_due_at = Some(self.now() + SAVE_DEBOUNCE_MS);
    }

    /// Write a pending debounced save once it is due (the JS save timer firing). Call periodically.
    pub fn tick(&mut self) {
        if let Some(due) = self.save_due_at {
            if self.now() >= due {
                self.save_due_at = None;
                self.save_now();
            }
        }
    }

    /// Cancel the debounce and write now.
    pub fn flush(&mut self) {
        self.save_due_at = None;
        self.save_now();
    }

    fn respawn_due(&mut self) -> Vec<String> {
        let timestamp = self.now() as f64;
        let mut respawned = Vec::new();
        for id in self.state.claims.keys() {
            let claim = self.state.claims.get(&id).expect("claim present");
            if claim.kind != "RELAY" && timestamp - claim.claimed_at >= self.item_respawn_ms as f64 {
                self.state.claims.remove(&id);
                respawned.push(id);
            }
        }
        respawned
    }

    fn prune(&mut self) {
        let timestamp = self.now();
        for session_id in self.players.keys() {
            let expired = self.players.get(&session_id).map(|p| timestamp - p.last_seen > PUBLIC_PLAYER_TTL_MS);
            if expired == Some(true) {
                let player = self.players.remove(&session_id).expect("player present");
                self.save_resume(&player);
                if self.authority.session_id == session_id {
                    self.authority = Authority::default();
                }
            }
        }
        for key in self.state.resumes.keys() {
            let stale = self.state.resumes.get(&key).map(|r| timestamp as f64 - r.saved_at > self.resume_ttl_ms as f64);
            if stale == Some(true) {
                self.state.resumes.remove(&key);
            }
        }
        self.join_rates.retain(|_, (_, reset_at)| timestamp < *reset_at);
    }

    fn save_resume(&mut self, player: &LivePlayer) {
        let Some(position) = &player.position else { return };
        if !player_key_pattern().is_match(&player.player_key) {
            return;
        }
        self.state.resumes.insert(
            player.player_key.clone(),
            Resume { position: position.clone(), inventory: player.inventory.clone(), saved_at: player.last_seen as f64 },
        );
        self.schedule_save();
    }

    fn relays_collected(&self) -> usize {
        self.state.claims.iter().filter(|(_, claim)| claim.kind == "RELAY").count()
    }

    fn online_count(&mut self) -> usize {
        self.prune();
        self.players.len()
    }

    fn public_status(&mut self, extra: Map<String, Value>) -> Value {
        let online = self.online_count();
        self.state.hunter_target = self.state.hunter_target.max(public_hunter_target_count(online as f64));
        let mut body = Map::new();
        body.insert("ok".into(), json!(true));
        body.insert("code".into(), json!(self.code));
        body.insert("seed".into(), json!(self.state.seed));
        body.insert("epoch".into(), json!(self.state.epoch));
        body.insert("day".into(), json!(utc_day_key(self.now())));
        body.insert("playersOnline".into(), json!(online));
        body.insert("relaysCollected".into(), json!(self.relays_collected()));
        body.insert("hunterTargetCount".into(), json!(self.state.hunter_target));
        for (key, value) in extra {
            body.insert(key, value);
        }
        Value::Object(body)
    }

    /// `publicRoute.status()`: the public status body (GET /api/public/status).
    pub fn status(&mut self) -> Value {
        self.tick();
        self.public_status(Map::new())
    }

    fn rate_limited(&mut self, address: &str) -> bool {
        let timestamp = self.now();
        let mut rate = match self.join_rates.get(address) {
            Some(&(count, reset_at)) if timestamp < reset_at => (count, reset_at),
            _ => (0, timestamp + 60_000),
        };
        rate.0 += 1;
        self.join_rates.insert(address.to_string(), rate);
        rate.0 > JOIN_LIMIT_PER_MINUTE
    }

    pub fn join(&mut self, body: &Value, opts: JoinOptions) -> PublicResult {
        self.tick();
        self.prune();
        let address = if opts.address.is_empty() { "unknown" } else { opts.address.as_str() };
        if self.rate_limited(address) {
            return PublicResult::fail(429, "TOO MANY JOIN REQUESTS");
        }
        let player_key = str_or_empty(field(body, "playerKey"));
        if !player_key_pattern().is_match(&player_key) {
            return PublicResult::fail(400, "INVALID PLAYER KEY");
        }
        let timestamp = self.now();
        // One live session per player key: a reload or crash replaces the old one.
        for session_id in self.players.keys() {
            if self.players.get(&session_id).map(|p| p.player_key == player_key) == Some(true) {
                let existing = self.players.remove(&session_id).expect("player present");
                self.save_resume(&existing);
                if self.authority.session_id == session_id {
                    self.authority = Authority::default();
                }
            }
        }
        let session_id = base64url(&random_bytes(18));
        let resume = self.state.resumes.get(&player_key).cloned();
        let player = LivePlayer {
            session_id: session_id.clone(),
            player_key,
            name: clean_name(field(body, "name")),
            joined_at: timestamp,
            last_seen: timestamp,
            last_state_at: 0,
            position: resume.as_ref().map(|r| r.position.clone()),
            inventory: resume.as_ref().map(|r| r.inventory.clone()).unwrap_or_else(|| json!([])),
        };
        self.players.insert(session_id.clone(), player);
        self.respawn_due();
        let authority_live = opts.host_token_valid
            || opts.enet_listed
            || (!self.authority.session_id.is_empty()
                && timestamp - self.authority.last_beat < PUBLIC_AUTHORITY_STALE_MS);
        let mut extra = Map::new();
        extra.insert("sessionId".into(), json!(session_id));
        extra.insert("joinGraceMs".into(), json!(PUBLIC_JOIN_GRACE_MS));
        extra.insert(
            "resume".into(),
            match &resume {
                Some(r) => json!({
                    "position": r.position,
                    "inventory": r.inventory,
                    "ageMs": num(timestamp as f64 - r.saved_at),
                }),
                None => Value::Null,
            },
        );
        extra.insert("claimed".into(), json!(self.state.claims.keys()));
        extra.insert("snapshot".into(), json!(self.state.snapshot));
        extra.insert("authorityLive".into(), json!(authority_live));
        PublicResult::ok(self.public_status(extra))
    }

    pub fn post(&mut self, body: &Value, host_token_valid: bool) -> PublicResult {
        self.tick();
        self.prune();
        let session_id = str_or_empty(field(body, "sessionId"));
        if !self.players.contains_key(&session_id) {
            return PublicResult::fail(404, "UNKNOWN SESSION // JOIN AGAIN");
        }
        let timestamp = self.now();
        {
            let player = self.players.get_mut(&session_id).expect("player present");
            if timestamp - player.last_state_at < STATE_MIN_INTERVAL_MS {
                return PublicResult::fail(429, "STATE UPDATES TOO FAST");
            }
            player.last_state_at = timestamp;
            player.last_seen = timestamp;
            if let Some(position) = sanitize_position(field(body, "position")) {
                player.position = Some(position);
            }
            if let Some(inventory) = field(body, "inventory") {
                player.inventory = sanitize_inventory(Some(inventory));
            }
            if truthy(field(body, "name")) {
                player.name = clean_name(field(body, "name"));
            }
        }
        let mut respawned = Vec::new();
        if host_token_valid {
            self.authority = Authority { session_id: session_id.clone(), last_beat: timestamp };
            if let Some(Value::String(snapshot)) = field(body, "snapshot") {
                let length = js_len(snapshot);
                if length > 0 && length <= SNAPSHOT_MAX_CHARS {
                    self.state.snapshot = snapshot.clone();
                    self.state.snapshot_at = timestamp as f64;
                }
            }
            if let Some(Value::Array(claims)) = field(body, "claims") {
                for entry in claims.iter().take(200) {
                    let id = str_or_empty(field(entry, "id"));
                    if !claim_id_pattern().is_match(&id) || self.state.claims.len() >= MAX_CLAIMS {
                        continue;
                    }
                    if !self.state.claims.contains_key(&id) {
                        let kind = id.split(':').nth(4).unwrap_or("").to_string();
                        self.state.claims.insert(id, Claim { kind, claimed_at: timestamp as f64 });
                    }
                }
            }
            respawned = self.respawn_due();
        }
        self.schedule_save();
        let mut extra = Map::new();
        extra.insert("authority".into(), json!(host_token_valid));
        extra.insert("respawned".into(), json!(respawned));
        // everyone learns about claims so a guest's HUD and a future authority agree
        extra.insert("claimCount".into(), json!(self.state.claims.len()));
        PublicResult::ok(self.public_status(extra))
    }

    pub fn leave(&mut self, body: &Value) -> PublicResult {
        self.tick();
        let session_id = str_or_empty(field(body, "sessionId"));
        let Some(player) = self.players.remove(&session_id) else {
            return PublicResult::ok(json!({ "ok": true }));
        };
        self.save_resume(&player);
        if self.authority.session_id == player.session_id {
            self.authority = Authority::default();
        }
        PublicResult::ok(json!({ "ok": true }))
    }

    /// True when no authority has beaten recently: the ENet entry should be dropped.
    pub fn authority_stale(&self) -> bool {
        !self.authority.session_id.is_empty() && self.now() - self.authority.last_beat > PUBLIC_AUTHORITY_STALE_MS
    }

    pub fn reset_authority(&mut self) {
        self.authority = Authority::default();
    }

    /// `players()`: `[{ name, position, joinedAt }]` of the live sessions.
    pub fn players(&self) -> Vec<Value> {
        self.players
            .iter()
            .map(|(_, p)| json!({ "name": p.name, "position": p.position.clone().unwrap_or(Value::Null), "joinedAt": p.joined_at }))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{manual_clock, parse_iso_time, read_json_file};

    fn key_a() -> String {
        "a".repeat(24)
    }
    fn key_b() -> String {
        "b".repeat(24)
    }

    fn route_at(start: i64) -> (PublicRoute, crate::util::ManualClock) {
        let (clock, handle) = manual_clock(start);
        (PublicRoute::new(PublicRouteOptions { now: clock, ..Default::default() }), handle)
    }

    fn addr(address: &str) -> JoinOptions {
        JoinOptions { address: address.into(), ..Default::default() }
    }

    fn body(result: PublicResult) -> Value {
        result.body.expect("a body")
    }

    #[test]
    fn hunter_target_scales_with_players_capped_at_8() {
        let counts: Vec<i64> =
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 15.0, 16.0, 40.0].iter().map(|n| public_hunter_target_count(*n)).collect();
        assert_eq!(counts, vec![1, 1, 2, 2, 3, 3, 8, 8, 8]);
    }

    #[test]
    fn utc_day_key_rolls_over_at_midnight_utc() {
        assert_eq!(utc_day_key(parse_iso_time("2026-10-26T23:59:59Z").unwrap()), "2026-10-26");
        assert_eq!(utc_day_key(parse_iso_time("2026-10-27T00:00:00Z").unwrap()), "2026-10-27");
    }

    #[test]
    fn positions_and_inventories_are_clamped_and_sanitised() {
        assert_eq!(sanitize_position(Some(&json!({ "x": "NaN", "z": 1 }))), None);
        assert_eq!(sanitize_position(Some(&json!({ "x": 5e9, "z": 1 }))), None);
        assert_eq!(sanitize_position(Some(&json!({ "x": 3.14159, "z": -2 }))), Some(json!({ "x": 3.14, "z": -2 })));
        assert_eq!(
            sanitize_inventory(Some(&json!([{ "id": "flashlight", "durability": 5000 }, { "id": "BAD ID" }, null]))),
            json!([{ "id": "flashlight", "durability": 1000, "slot": 0 }])
        );
        assert_eq!(stringify(&sanitize_position(Some(&json!({ "x": 3.14159, "z": -2 }))).unwrap()), r#"{"x":3.14,"z":-2}"#);
    }

    #[test]
    fn join_returns_the_stable_seed_and_a_resume_record_after_a_crash_within_the_ttl() {
        let (mut route, clock) = route_at(1_000_000);
        let first = body(route.join(&json!({ "playerKey": key_a(), "name": "Aki" }), addr("1.1.1.1")));
        assert_eq!(first["code"], "PUBLC");
        assert_eq!(first["resume"], Value::Null);
        clock.advance(1_000);
        let posted = route.post(
            &json!({ "sessionId": first["sessionId"], "position": { "x": 12.5, "z": -40 }, "inventory": [{ "id": "taser", "durability": 3 }] }),
            false,
        );
        assert_eq!(posted.status, 200);
        clock.advance(5_000); // the client crashes: no leave call, the session just goes quiet
        clock.advance(30_000);
        let second = body(route.join(&json!({ "playerKey": key_a(), "name": "Aki" }), addr("1.1.1.1")));
        assert_eq!(second["seed"], first["seed"]);
        assert_eq!(second["resume"]["position"], json!({ "x": 12.5, "z": -40 }));
        assert_eq!(second["resume"]["inventory"][0]["id"], "taser");
        clock.advance(7 * 60 * 60 * 1000); // resume TTL is six hours
        let third = body(route.join(&json!({ "playerKey": key_a(), "name": "Aki" }), addr("1.1.1.1")));
        assert_eq!(third["resume"], Value::Null);
    }

    #[test]
    fn only_the_enet_host_token_holder_can_post_snapshots_and_claims_items_respawn_relays_do_not() {
        let (mut route, clock) = route_at(5_000_000);
        let host = body(route.join(&json!({ "playerKey": key_a(), "name": "A" }), addr("x")));
        let guest = body(route.join(&json!({ "playerKey": key_b(), "name": "B" }), addr("y")));
        let claims = json!([{ "id": "1:0:0:PICKUP:RELAY:0" }, { "id": "1:0:0:PICKUP:BATTERY:3" }, { "id": "not an id" }]);
        clock.advance(1000);
        route.post(&json!({ "sessionId": guest["sessionId"], "snapshot": "forged", "claims": claims }), false);
        assert_eq!(route.status()["relaysCollected"], 0, "a guest must not forge claims");
        clock.advance(1000);
        let posted = body(route.post(&json!({ "sessionId": host["sessionId"], "snapshot": "(snap)", "claims": claims }), true));
        assert_eq!(posted["relaysCollected"], 1);
        assert_eq!(posted["hunterTargetCount"], 1);
        clock.advance(1000);
        assert_eq!(route.status()["playersOnline"], 2);
        assert_eq!(route.snapshot(), "(snap)");
        // the authority keeps posting (every few seconds); respawns come back on the first post after the timer
        let mut again = Value::Null;
        let mut respawned: Vec<Value> = Vec::new();
        let mut elapsed = 0;
        while elapsed <= PUBLIC_ITEM_RESPAWN_MS + 10_000 {
            clock.advance(5_000);
            again = body(route.post(&json!({ "sessionId": host["sessionId"] }), true));
            respawned.extend(again["respawned"].as_array().unwrap().iter().cloned());
            elapsed += 5_000;
        }
        assert_eq!(respawned, vec![json!("1:0:0:PICKUP:BATTERY:3")]);
        assert_eq!(again["relaysCollected"], 1);
        let late_join = body(route.join(&json!({ "playerKey": "c".repeat(24), "name": "C" }), addr("z")));
        assert_eq!(late_join["claimed"], json!(["1:0:0:PICKUP:RELAY:0"]));
        assert_eq!(late_join["snapshot"], "(snap)");
    }

    #[test]
    fn state_posts_are_rate_limited_per_session_and_joins_per_address() {
        let (mut route, clock) = route_at(1);
        let joined = body(route.join(&json!({ "playerKey": key_a() }), addr("r")));
        clock.advance(1000);
        assert_eq!(route.post(&json!({ "sessionId": joined["sessionId"], "position": { "x": 1, "z": 1 } }), false).status, 200);
        assert_eq!(route.post(&json!({ "sessionId": joined["sessionId"], "position": { "x": 2, "z": 2 } }), false).status, 429);
        let mut blocked = 0;
        for _ in 0..30 {
            if route.join(&json!({ "playerKey": key_b() }), addr("spam")).status == 429 {
                blocked += 1;
            }
        }
        assert!(blocked > 5);
        assert_eq!(route.post(&json!({ "sessionId": "nope" }), false).status, 404);
        assert_eq!(
            route.post(&json!({ "sessionId": "nope" }), false).to_reply(),
            (404, json!({ "ok": false, "error": "UNKNOWN SESSION // JOIN AGAIN" }))
        );
    }

    #[test]
    fn hunter_target_is_sticky_and_grows_with_the_party() {
        let (mut route, _clock) = route_at(1);
        for n in ["1", "2", "3", "4", "5"] {
            let key = n.repeat(24);
            route.join(&json!({ "playerKey": key }), addr(&key));
        }
        assert_eq!(route.status()["hunterTargetCount"], 3);
    }

    #[test]
    fn state_persists_to_disk_and_survives_a_server_restart() {
        let dir = std::env::temp_dir().join(format!(
            "public-route-{}-{}",
            std::process::id(),
            crate::util::hex(&random_bytes(6))
        ));
        let store = dir.join("lastroute").join("godot_publicroom.json");
        let options = || PublicRouteOptions { store_path: Some(store.clone()), ..Default::default() };
        let mut first = PublicRoute::new(options());
        first.load();
        let joined = body(first.join(&json!({ "playerKey": key_a() }), addr("p")));
        first.post(
            &json!({ "sessionId": joined["sessionId"], "position": { "x": 7, "z": 8 }, "snapshot": "s", "claims": [{ "id": "2:0:3:PICKUP:RELAY:1" }] }),
            true,
        );
        first.leave(&json!({ "sessionId": joined["sessionId"] }));
        first.flush();
        let saved = read_json_file(&store).expect("store written");
        assert_eq!(saved["claims"].as_array().unwrap().len(), 1);
        let mut second = PublicRoute::new(options());
        second.load();
        let back = body(second.join(&json!({ "playerKey": key_a() }), addr("p")));
        assert_eq!(back["resume"]["position"], json!({ "x": 7, "z": 8 }));
        assert_eq!(back["claimed"], json!(["2:0:3:PICKUP:RELAY:1"]));
        assert_eq!(back["snapshot"], "s");
        assert_eq!(back["seed"], joined["seed"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
