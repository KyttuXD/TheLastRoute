//! Port of `backend/reserved_rooms.mjs`: reserved private rooms.
//!
//! Store and echo only. A reservation is a room code the owner keeps: it never expires, nobody else can register it,
//! and only invited accounts (or holders of the room password, kept as a scrypt hash) may host or join it. The live
//! ENet session on that code still comes and goes with whoever hosts it; the reservation, its stored route settings,
//! invite list and mods live in one JSON file (`{"rooms": {...}}`, one-space indented like the Node
//! `JSON.stringify(value, null, 1)`) under the data directory and survive restarts and deploys.
//!
//! Owner controls (restart, end, kick, settings, transfer host) are queued as commands with a sequence number and
//! echoed to the current host on its heartbeat, which carries them out and acks them.

use crate::util::{
    base64url, from_base64, hex, iso_time, js_number, js_string_of, log_error, random_bytes, str_nullish, str_or_empty,
    timing_safe_equal, truthy, Clock, Reply,
};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

pub struct ReservedLimits {
    pub rooms: usize,
    pub invites: usize,
    pub commands: usize,
    pub mods: usize,
    pub password_min: usize,
    pub password_max: usize,
    pub ticket_ttl_ms: i64,
    pub tickets_per_room: usize,
}

pub const RESERVED_LIMITS: ReservedLimits = ReservedLimits {
    rooms: 50,
    invites: 64,
    commands: 20,
    mods: 32,
    password_min: 6,
    password_max: 128,
    ticket_ttl_ms: 10 * 60 * 1000,
    tickets_per_room: 64,
};
pub const RESERVED_ACTIONS: [&str; 5] = ["restart", "end", "kick", "settings", "transfer"];

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid regex"))
}
fn code_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Z0-9]{5}$")
}
fn username_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Za-z0-9_]{3,20}$")
}
fn seed_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Za-z0-9 _-]{0,40}$")
}
fn mod_id_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[a-z0-9][a-z0-9_]{2,31}$")
}
fn display_target_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"^[A-Za-z0-9 _-]{1,24}$")
}

/// `normalizeReservedCode(value)`.
pub fn normalize_reserved_code(value: Option<&Value>) -> String {
    let code = str_nullish(value).trim().to_uppercase();
    if code_pattern().is_match(&code) {
        code
    } else {
        String::new()
    }
}

fn normalize_code_str(code: &str) -> String {
    normalize_reserved_code(Some(&Value::String(code.to_string())))
}

fn scrypt_bytes(password: &str, salt: &[u8], len: usize) -> Vec<u8> {
    let params = scrypt::Params::new(14, 8, 1, len.max(1)).expect("scrypt params");
    let mut out = vec![0u8; len.max(1)];
    scrypt::scrypt(password.as_bytes(), salt, &params, &mut out).expect("scrypt");
    out.truncate(len);
    out
}

/// `scrypt$<salt b64url>$<32-byte hash b64url>`.
pub fn hash_password(password: &str) -> String {
    let salt = random_bytes(16);
    format!("scrypt${}${}", base64url(&salt), base64url(&scrypt_bytes(password, &salt, 32)))
}

pub fn password_matches(stored: &str, password: &str) -> bool {
    let mut parts = stored.split('$');
    let (kind, salt, hash) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if kind != "scrypt" || salt.is_empty() || hash.is_empty() || password.is_empty() {
        return false;
    }
    let expected = from_base64(hash);
    if expected.is_empty() {
        return false;
    }
    let actual = scrypt_bytes(password, &from_base64(salt), expected.len());
    timing_safe_equal(&expected, &actual)
}

/// `cleanRoomSettings(value, previous)`: route settings a reservation remembers; anything else is dropped.
pub fn clean_room_settings(value: &Value, previous: &Map<String, Value>) -> Map<String, Value> {
    let mut settings = previous.clone();
    let get = |key: &str| value.get(key);
    if let Some(seed) = get("seed") {
        let text = js_string_of(seed);
        let next = if seed_pattern().is_match(&text) {
            Value::String(text)
        } else {
            Value::String(str_or_empty(previous.get("seed")))
        };
        settings.insert("seed".into(), next);
    }
    for key in ["mode", "scale", "difficulty"] {
        if let Some(raw) = get(key) {
            let n = js_number(Some(raw));
            if n.is_finite() && n.fract() == 0.0 && n >= 0.0 && n < 32.0 {
                settings.insert(key.into(), Value::from(n as i64));
            }
        }
    }
    if let Some(day) = get("daytime") {
        settings.insert("daytime".into(), Value::Bool(truthy(Some(day))));
    }
    if let Some(Value::Array(mods)) = get("mods") {
        let mut seen: Vec<String> = Vec::new();
        for id in mods.iter().map(js_string_of) {
            if mod_id_pattern().is_match(&id) && !seen.contains(&id) {
                seen.push(id);
            }
        }
        seen.truncate(RESERVED_LIMITS.mods);
        settings.insert("mods".into(), Value::Array(seen.into_iter().map(Value::String).collect()));
    }
    settings
}

fn clean_invites(list: Option<&Value>) -> Option<Vec<String>> {
    let Some(Value::Array(items)) = list else { return None };
    let mut out: Vec<String> = Vec::new();
    for name in items.iter().map(|v| js_string_of(v).trim().to_string()) {
        if !username_pattern().is_match(&name) {
            continue;
        }
        let upper = name.to_uppercase();
        if !out.contains(&upper) {
            out.push(upper);
        }
    }
    out.truncate(RESERVED_LIMITS.invites);
    Some(out)
}

fn strings(values: &[String]) -> Value {
    Value::Array(values.iter().cloned().map(Value::String).collect())
}

/// What a reservation looks like to its owner (never the password hash).
pub fn owner_view(room: &Value) -> Value {
    json!({
        "code": room.get("code").cloned().unwrap_or(Value::Null),
        "settings": room.get("settings").cloned().unwrap_or(Value::Null),
        "invites": room.get("invites").cloned().unwrap_or(Value::Null),
        "hasPassword": truthy(room.get("passwordHash")),
        "commands": room.get("commands").cloned().unwrap_or(Value::Null),
        "commandSeq": room.get("commandSeq").cloned().unwrap_or(Value::Null),
        "createdBy": room.get("createdBy").cloned().unwrap_or(Value::Null),
        "updatedAt": room.get("updatedAt").cloned().unwrap_or(Value::Null),
        "fromEnvironment": truthy(room.get("fromEnvironment")),
    })
}

pub struct ReservedRoomsOptions {
    pub store_path: PathBuf,
    pub codes: Vec<String>,
    pub now: Clock,
}

pub struct ReservedRooms {
    store_path: PathBuf,
    now: Clock,
    rooms: Map<String, Value>,
    /// code -> (ticket hex -> expiresAt), insertion ordered; memory only.
    tickets: HashMap<String, Vec<(String, i64)>>,
}

impl ReservedRooms {
    pub fn new(opts: ReservedRoomsOptions) -> ReservedRooms {
        let mut rooms = Map::new();
        if opts.store_path.exists() {
            match std::fs::read_to_string(&opts.store_path).map_err(|e| e.to_string()).and_then(|text| {
                serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())
            }) {
                Ok(parsed) => {
                    if let Some(Value::Object(stored)) = parsed.get("rooms") {
                        rooms = stored.clone();
                    }
                }
                Err(error) => log_error(&format!(
                    "Reserved rooms: could not read {}: {error}",
                    opts.store_path.display()
                )),
            }
        }
        let mut this = ReservedRooms { store_path: opts.store_path, now: opts.now, rooms, tickets: HashMap::new() };
        for room in this.rooms.values_mut() {
            let invites = clean_invites(room.get("invites")).unwrap_or_default();
            if let Value::Object(map) = room {
                map.insert("invites".into(), strings(&invites));
            }
        }
        let mut changed = false;
        for value in &opts.codes {
            let code = normalize_code_str(value);
            if code.is_empty() {
                continue;
            }
            if !this.rooms.contains_key(&code) {
                let room = this.fresh(&code, "environment", true);
                this.rooms.insert(code, room);
                changed = true;
            } else if let Some(Value::Object(map)) = this.rooms.get_mut(&code) {
                map.insert("fromEnvironment".into(), Value::Bool(true));
            }
        }
        if changed {
            this.save();
        }
        this
    }

    fn now_iso(&self) -> String {
        iso_time((self.now)())
    }

    fn save(&self) {
        let value = json!({ "rooms": Value::Object(self.rooms.clone()) });
        let mut out = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
        let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
        serde::Serialize::serialize(&value, &mut serializer).ok();
        if let Err(error) = crate::util::write_atomic(&self.store_path, &out) {
            log_error(&format!("Reserved rooms: could not write {}: {error}", self.store_path.display()));
        }
    }

    fn fresh(&self, code: &str, created_by: &str, from_environment: bool) -> Value {
        let settings = clean_room_settings(&json!({ "seed": "", "mods": [] }), &Map::new());
        json!({
            "code": code, "settings": settings, "invites": [], "passwordHash": "", "commands": [],
            "commandSeq": 0, "createdBy": created_by, "updatedAt": self.now_iso(), "fromEnvironment": from_environment,
        })
    }

    pub fn is_reserved(&self, code: &str) -> bool {
        self.rooms.contains_key(&normalize_code_str(code))
    }

    fn invites_of(room: &Value) -> Vec<String> {
        match room.get("invites") {
            Some(Value::Array(items)) => items.iter().map(js_string_of).collect(),
            _ => Vec::new(),
        }
    }

    /// May this person host or join? The owner always; an invited account; or anyone with the room password.
    pub fn has_access(&self, code: &str, username: &str, password: &str, owner: bool) -> bool {
        let Some(room) = self.rooms.get(&normalize_code_str(code)) else { return false };
        if owner {
            return true;
        }
        if !username.is_empty() && Self::invites_of(room).contains(&username.to_uppercase()) {
            return true;
        }
        !password.is_empty() && password_matches(&str_or_empty(room.get("passwordHash")), password)
    }

    /// A short-lived ticket the relay accepts as the guest's join nonce for this code (base64url, 18 bytes).
    pub fn issue_ticket(&mut self, code: &str) -> String {
        let clean = normalize_code_str(code);
        let ticket = random_bytes(18);
        let timestamp = (self.now)();
        let map = self.tickets.entry(clean).or_default();
        map.retain(|(_, expires)| *expires > timestamp);
        while map.len() >= RESERVED_LIMITS.tickets_per_room {
            map.remove(0);
        }
        map.push((hex(&ticket), timestamp + RESERVED_LIMITS.ticket_ttl_ms));
        base64url(&ticket)
    }

    /// `ticketValid(code, payload)`: payload is the raw join nonce bytes.
    pub fn ticket_valid(&self, code: &str, payload: &[u8]) -> bool {
        let key = hex(payload);
        let now = (self.now)();
        self.tickets
            .get(&normalize_code_str(code))
            .and_then(|map| map.iter().find(|(k, _)| *k == key))
            .map(|(_, expires)| *expires > now)
            .unwrap_or(false)
    }

    fn room_mut(&mut self, code: &str) -> &mut Map<String, Value> {
        match self.rooms.get_mut(code) {
            Some(Value::Object(map)) => map,
            _ => unreachable!("reserved room records are objects"),
        }
    }

    /// Owner admin: `admin(body, owner)`.
    pub fn admin(&mut self, body: &Value, owner: &str) -> Reply {
        let action = {
            let a = str_or_empty(body.get("action"));
            if a.is_empty() {
                "list".to_string()
            } else {
                a
            }
        };
        let code = normalize_reserved_code(body.get("code"));
        if action == "list" {
            let rooms: Vec<Value> = self.rooms.values().map(owner_view).collect();
            return (200, json!({ "ok": true, "rooms": rooms }));
        }
        if code.is_empty() {
            return (400, json!({ "ok": false, "code": "INVALID_ROOM_CODE", "error": "Reserved codes are 5 letters/digits" }));
        }
        if action == "create" {
            if self.rooms.contains_key(&code) {
                return (409, json!({ "ok": false, "code": "ROOM_ALREADY_RESERVED", "error": "That code is already reserved" }));
            }
            if self.rooms.len() >= RESERVED_LIMITS.rooms {
                return (409, json!({ "ok": false, "code": "TOO_MANY_RESERVED_ROOMS", "error": "Reservation limit reached" }));
            }
            let room = self.fresh(&code, owner, false);
            self.rooms.insert(code.clone(), room);
        }
        if !self.rooms.contains_key(&code) {
            return (404, json!({ "ok": false, "code": "ROOM_NOT_RESERVED", "error": "That code is not reserved" }));
        }
        let now_iso = self.now_iso();
        if action == "delete" {
            if truthy(self.rooms[&code].get("fromEnvironment")) {
                return (409, json!({ "ok": false, "code": "ROOM_FROM_ENVIRONMENT", "error": "Remove it from LASTROUTE_RESERVED_ROOMS instead" }));
            }
            self.rooms.remove(&code);
            self.tickets.remove(&code);
            self.save();
            return (200, json!({ "ok": true, "deleted": code }));
        }
        if action == "create" || action == "update" {
            if truthy(body.get("settings")) {
                let previous = self.rooms[&code].get("settings").and_then(Value::as_object).cloned().unwrap_or_default();
                let next = clean_room_settings(&body["settings"], &previous);
                self.room_mut(&code).insert("settings".into(), Value::Object(next));
            }
            if let Some(invites) = clean_invites(body.get("invites")) {
                self.room_mut(&code).insert("invites".into(), strings(&invites));
            }
            if body.get("password").is_some() {
                let password = str_nullish(body.get("password"));
                let len = crate::util::js_len(&password);
                if !password.is_empty() && (len < RESERVED_LIMITS.password_min || len > RESERVED_LIMITS.password_max) {
                    return (400, json!({ "ok": false, "code": "INVALID_ROOM_PASSWORD", "error": format!(
                        "Room passwords are {}-{} characters", RESERVED_LIMITS.password_min, RESERVED_LIMITS.password_max) }));
                }
                let hash = if password.is_empty() { String::new() } else { hash_password(&password) };
                self.room_mut(&code).insert("passwordHash".into(), Value::String(hash));
                self.tickets.remove(&code); // a new password revokes outstanding join tickets
            }
            self.room_mut(&code).insert("updatedAt".into(), Value::String(now_iso));
            self.save();
            let view = owner_view(&self.rooms[&code]);
            return (if action == "create" { 201 } else { 200 }, json!({ "ok": true, "room": view }));
        }
        if action == "whitelist" {
            let op = {
                let o = str_or_empty(body.get("op"));
                if o.is_empty() {
                    "list".to_string()
                } else {
                    o
                }
            };
            let name = str_or_empty(body.get("name")).trim().to_uppercase();
            if op != "list" {
                if !username_pattern().is_match(&name) {
                    return (400, json!({ "ok": false, "code": "INVALID_ACCOUNT_NAME", "error": "Account names are 3-20 letters, digits or _" }));
                }
                let mut invites = Self::invites_of(&self.rooms[&code]);
                if op == "add" && !invites.contains(&name) {
                    if invites.len() >= RESERVED_LIMITS.invites {
                        return (409, json!({ "ok": false, "code": "WHITELIST_FULL", "error": "Whitelist is full" }));
                    }
                    invites.push(name);
                } else if op == "remove" {
                    invites.retain(|entry| *entry != name);
                } else if op != "add" {
                    return (400, json!({ "ok": false, "code": "INVALID_WHITELIST_OP", "error": "op: add, remove or list" }));
                }
                let room = self.room_mut(&code);
                room.insert("invites".into(), strings(&invites));
                room.insert("updatedAt".into(), Value::String(now_iso));
                self.save();
            }
            let whitelist = self.rooms[&code].get("invites").cloned().unwrap_or(json!([]));
            return (200, json!({ "ok": true, "code": code, "whitelist": whitelist }));
        }
        if action == "command" {
            let command = str_or_empty(body.get("command"));
            if !RESERVED_ACTIONS.contains(&command.as_str()) {
                return (400, json!({ "ok": false, "code": "INVALID_ROOM_COMMAND", "error": format!("Commands: {}", RESERVED_ACTIONS.join(", ")) }));
            }
            let mut args = Map::new();
            if command == "restart" {
                args.insert("sameSeed".into(), Value::Bool(truthy(body.get("sameSeed"))));
            }
            if command == "kick" || command == "transfer" {
                let target = str_or_empty(body.get("target")).trim().to_string();
                if !username_pattern().is_match(&target) && !display_target_pattern().is_match(&target) {
                    return (400, json!({ "ok": false, "code": "INVALID_COMMAND_TARGET", "error": "Name the player (account or display name)" }));
                }
                args.insert("target".into(), Value::String(target));
            }
            if command == "settings" || command == "restart" {
                if truthy(body.get("settings")) {
                    let previous = self.rooms[&code].get("settings").and_then(Value::as_object).cloned().unwrap_or_default();
                    let next = clean_room_settings(&body["settings"], &previous);
                    self.room_mut(&code).insert("settings".into(), Value::Object(next));
                }
                args.insert("settings".into(), self.rooms[&code].get("settings").cloned().unwrap_or(Value::Null));
            }
            let room = self.room_mut(&code);
            let seq = js_number(room.get("commandSeq")).max(0.0) as i64 + 1;
            room.insert("commandSeq".into(), Value::from(seq));
            let mut commands = match room.get("commands") {
                Some(Value::Array(list)) => list.clone(),
                _ => Vec::new(),
            };
            commands.push(json!({ "seq": seq, "command": command, "args": args, "by": owner, "at": now_iso }));
            let excess = commands.len().saturating_sub(RESERVED_LIMITS.commands);
            commands.drain(..excess);
            let queued = commands.last().cloned().unwrap_or(Value::Null);
            room.insert("commands".into(), Value::Array(commands));
            room.insert("updatedAt".into(), Value::String(now_iso));
            self.save();
            let view = owner_view(&self.rooms[&code]);
            return (202, json!({ "ok": true, "queued": queued, "room": view }));
        }
        (400, json!({ "ok": false, "code": "INVALID_ACTION", "error": "Actions: list, create, update, delete, whitelist, command" }))
    }

    /// The host's view on register/heartbeat: stored settings and the commands after `since`, then acks drop up to `ack`.
    pub fn host_view(&mut self, code: &str, since: Option<&Value>, ack: Option<&Value>) -> Option<Value> {
        let code = normalize_code_str(code);
        if !self.rooms.contains_key(&code) {
            return None;
        }
        let acked = {
            let n = js_number(ack);
            if n.is_nan() {
                0.0
            } else {
                n
            }
        };
        let seq_of = |entry: &Value| js_number(entry.get("seq"));
        let commands = match self.rooms[&code].get("commands") {
            Some(Value::Array(list)) => list.clone(),
            _ => Vec::new(),
        };
        if acked > 0.0 && commands.iter().any(|entry| seq_of(entry) <= acked) {
            let kept: Vec<Value> = commands.iter().filter(|entry| seq_of(entry) > acked).cloned().collect();
            self.room_mut(&code).insert("commands".into(), Value::Array(kept));
            self.save();
        }
        let since = {
            let n = js_number(since);
            if n.is_nan() {
                0.0
            } else {
                n
            }
        };
        let after = since.max(acked);
        let room = &self.rooms[&code];
        let pending: Vec<Value> = match room.get("commands") {
            Some(Value::Array(list)) => list.iter().filter(|entry| seq_of(entry) > after).cloned().collect(),
            _ => Vec::new(),
        };
        Some(json!({
            "reserved": true,
            "settings": room.get("settings").cloned().unwrap_or(Value::Null),
            "commands": pending,
            "commandSeq": room.get("commandSeq").cloned().unwrap_or(Value::Null),
        }))
    }

    /// The stored settings of a reservation (`hostView(code).settings` without touching acks).
    pub fn settings(&self, code: &str) -> Value {
        self.rooms
            .get(&normalize_code_str(code))
            .and_then(|room| room.get("settings").cloned())
            .unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::system_clock;

    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lr_reserved_{name}_{}", hex(&random_bytes(6))));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open(store: &PathBuf, codes: &[&str]) -> ReservedRooms {
        ReservedRooms::new(ReservedRoomsOptions {
            store_path: store.clone(),
            codes: codes.iter().map(|c| c.to_string()).collect(),
            now: system_clock(),
        })
    }

    #[test]
    fn reservations_persist_keep_environment_codes_hash_the_password_and_queue_owner_commands() {
        let root = temp_root("persist");
        let store = root.join("data").join("reserved_rooms.json");
        let mut rooms = open(&store, &["ktprv", "bad!"]);
        assert!(rooms.is_reserved("KTPRV") && !rooms.is_reserved("ABCDE"));
        assert_eq!(rooms.admin(&json!({"action": "delete", "code": "KTPRV"}), "KYTTU").0, 409);
        let (status, created) = rooms.admin(&json!({"action": "create", "code": "PRV22", "invites": ["Alice", "bob_2", "no way!"],
            "password": "open sesame", "settings": {"seed": "NEON 7", "mode": 2, "difficulty": 1, "mods": ["neon_halls", "Bad-Id"], "junk": 1}}), "KYTTU");
        assert_eq!(status, 201);
        assert_eq!(created["room"]["invites"], json!(["ALICE", "BOB_2"]));
        assert_eq!(created["room"]["settings"]["mods"], json!(["neon_halls"]));
        assert!(created["room"]["settings"].get("junk").is_none());
        assert_eq!(created["room"]["hasPassword"], json!(true));
        let text = serde_json::to_string(&created).unwrap();
        assert!(!text.contains("open sesame") && !text.contains("scrypt"));
        assert!(!std::fs::read_to_string(&store).unwrap().contains("open sesame"));
        assert_eq!(rooms.admin(&json!({"action": "create", "code": "PRV22"}), "KYTTU").0, 409);
        assert_eq!(rooms.admin(&json!({"action": "update", "code": "PRV22", "password": "123"}), "KYTTU").0, 400);

        assert!(rooms.has_access("PRV22", "ALICE", "", false));
        assert!(rooms.has_access("PRV22", "", "open sesame", false));
        assert!(!rooms.has_access("PRV22", "mallory", "guess", false));
        assert!(rooms.has_access("PRV22", "", "", true));

        assert_eq!(rooms.admin(&json!({"action": "command", "code": "PRV22", "command": "explode"}), "KYTTU").0, 400);
        rooms.admin(&json!({"action": "command", "code": "PRV22", "command": "restart", "sameSeed": true}), "KYTTU");
        rooms.admin(&json!({"action": "command", "code": "PRV22", "command": "kick", "target": "bob_2"}), "KYTTU");
        let host = rooms.host_view("PRV22", None, None).unwrap();
        let names = |h: &Value| h["commands"].as_array().unwrap().iter().map(|c| c["command"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&host), vec!["restart", "kick"]);
        assert_eq!(host["commands"][0]["args"]["sameSeed"], json!(true));
        let first = host["commands"][0]["seq"].clone();
        let host = rooms.host_view("PRV22", None, Some(&first)).unwrap();
        assert_eq!(names(&host), vec!["kick"]);
        for _ in 0..30 {
            rooms.admin(&json!({"action": "command", "code": "PRV22", "command": "end"}), "KYTTU");
        }
        assert!(rooms.host_view("PRV22", None, None).unwrap()["commands"].as_array().unwrap().len() <= RESERVED_LIMITS.commands);

        assert_eq!(rooms.admin(&json!({"action": "whitelist", "code": "PRV22", "op": "add", "name": "carol"}), "KYTTU").1["whitelist"],
            json!(["ALICE", "BOB_2", "CAROL"]));
        assert!(rooms.has_access("PRV22", "Carol", "", false));
        assert_eq!(rooms.admin(&json!({"action": "whitelist", "code": "PRV22", "op": "remove", "name": "BOB_2"}), "KYTTU").1["whitelist"],
            json!(["ALICE", "CAROL"]));
        assert!(!rooms.has_access("PRV22", "bob_2", "", false));
        assert_eq!(rooms.admin(&json!({"action": "whitelist", "code": "PRV22", "op": "add", "name": "x"}), "KYTTU").0, 400);
        assert_eq!(rooms.admin(&json!({"action": "whitelist", "code": "PRV22"}), "KYTTU").1["whitelist"], json!(["ALICE", "CAROL"]));
        assert_eq!(rooms.admin(&json!({"action": "create", "code": "akiri"}), "KYTTU").0, 201);
        assert!(rooms.is_reserved("AKIRI"));

        let mut again = open(&store, &["KTPRV"]);
        assert!(again.is_reserved("PRV22") && again.is_reserved("KTPRV"));
        assert!(again.has_access("PRV22", "", "open sesame", false));
        assert_eq!(again.host_view("PRV22", None, None).unwrap()["settings"]["seed"], json!("NEON 7"));

        let ticket = again.issue_ticket("PRV22");
        assert!(again.ticket_valid("PRV22", &from_base64(&ticket)));
        assert!(!again.ticket_valid("PRV22", b"not a ticket at all"));
        again.admin(&json!({"action": "update", "code": "PRV22", "password": "a new secret"}), "KYTTU");
        assert!(!again.ticket_valid("PRV22", &from_base64(&ticket)));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn store_layout_is_one_space_indented_like_node() {
        let root = temp_root("layout");
        let store = root.join("reserved_rooms.json");
        let _rooms = open(&store, &["KATHY"]);
        let text = std::fs::read_to_string(&store).unwrap();
        assert!(text.starts_with("{\n \"rooms\": {\n  \"KATHY\": {\n   \"code\": \"KATHY\",\n   \"settings\": {\n    \"seed\": \"\",\n    \"mods\": []"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
