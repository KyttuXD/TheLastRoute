//! Port of `backend/moderation.mjs`: server-side moderation behind the host-side anti-cheat.
//!
//! The host client detects and kicks cheaters and reports each kick here (authenticated by its ENet host token).
//! Accounts are never restricted automatically: kick and build-integrity reports only open REVIEW entries; only an
//! owner's action (flag/extend, ban, troll) restricts an account, and only for shared spaces (joining the PUBLIC
//! route, hosting a public room, uploading a skin). Raw IP addresses and device ids are never stored, only
//! `sha256(salt + value)`. Store: `moderation.json` (`{version:1, secret?, accounts, cooldowns, deviceAddress,
//! builds}`), same layout as the Node server.
//!
//! The Node module's injected callbacks (`isOwner`, `accountExists`, `updatesProvider`) are the [`ModerationHost`]
//! trait, passed to the methods that need it.

use crate::util::{
    hex, iso_time, js_number, js_slice, log_info, log_warn, num, random_bytes, sha256_hex, str_nullish, str_or_empty,
    stringify, truthy, write_atomic, Clock, Reply,
};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;

const HOUR_MS: i64 = 60 * 60 * 1000;
const DAY_MS: i64 = 24 * HOUR_MS;
pub const KICK_WINDOW_MS: i64 = 7 * DAY_MS;
pub const KICK_FLAG_THRESHOLD: usize = 2;
const AUTOMATIC_COOLDOWNS: bool = false;
pub const MAX_EXTEND_HOURS: f64 = 24.0 * 90.0;
pub const MAX_PEER_ENTRIES: usize = 16;
const MAX_LOG_ENTRIES: usize = 100;
const MAX_HASH_ENTRIES: usize = 50_000;

fn username_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Z0-9_]{3,24}$").unwrap())
}
fn build_hash_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[a-f0-9]{64}$").unwrap())
}
fn commit_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[a-f0-9]{7,40}$").unwrap())
}

/// The Node module's injected callbacks.
pub trait ModerationHost {
    fn is_owner(&self, _name: &str) -> bool {
        false
    }
    fn account_exists(&self, _name: &str) -> bool {
        true
    }
    fn updates(&self) -> Value {
        json!({})
    }
}

fn username_of(value: Option<&Value>) -> String {
    js_slice(&str_or_empty(value).trim().to_uppercase(), 24)
}

fn username_of_str(value: &str) -> String {
    username_of(Some(&Value::String(value.to_string())))
}

/// `text(value, limit)`: control characters become spaces, trimmed, clipped.
fn text(value: Option<&Value>, limit: usize) -> String {
    let raw: String = str_nullish(value)
        .chars()
        .map(|c| if (c as u32) < 0x20 || c as u32 == 0x7f { ' ' } else { c })
        .collect();
    js_slice(crate::util::js_trim(&raw), limit)
}

fn format_utc(ms: f64) -> String {
    let iso = iso_time(ms as i64);
    format!("{} UTC", iso[..16].replacen('T', " ", 1))
}

#[derive(Clone)]
struct Record {
    username: String,
    flags: Vec<Value>,
    kicks: Vec<Value>,
    flagged_until: f64,
    permanent: bool,
    troll: bool,
    ads: bool,
    integrity: Value,
    flagged_hashes: Vec<Value>,
    reviews: Vec<Value>,
}

fn tail(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items[items.len().saturating_sub(MAX_LOG_ENTRIES)..].to_vec(),
        _ => Vec::new(),
    }
}

impl Record {
    fn normalize(value: &Value) -> Record {
        let flagged_until = {
            let n = js_number(value.get("flaggedUntil"));
            if n.is_nan() {
                0.0
            } else {
                n
            }
        };
        Record {
            username: username_of(value.get("username")),
            flags: tail(value.get("flags")),
            kicks: tail(value.get("kicks")),
            flagged_until,
            permanent: truthy(value.get("permanent")),
            troll: truthy(value.get("troll")),
            ads: match value.get("ads") {
                None => true,
                v => truthy(v),
            },
            integrity: match value.get("integrity") {
                Some(Value::Object(map)) => Value::Object(map.clone()),
                Some(Value::Array(list)) => Value::Array(list.clone()),
                _ => Value::Null,
            },
            flagged_hashes: tail(value.get("flaggedHashes")),
            reviews: tail(value.get("reviews")),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "username": self.username,
            "flags": self.flags,
            "kicks": self.kicks,
            "flaggedUntil": num(self.flagged_until),
            "permanent": self.permanent,
            "troll": self.troll,
            "ads": self.ads,
            "integrity": self.integrity,
            "flaggedHashes": self.flagged_hashes,
            "reviews": self.reviews,
        })
    }

    fn open_reviews(&self) -> usize {
        self.reviews.iter().filter(|review| truthy(review.get("open"))).count()
    }
}

pub struct ModerationOptions {
    pub store_path: Option<PathBuf>,
    pub salt: String,
    pub now: Clock,
}

impl Default for ModerationOptions {
    fn default() -> Self {
        ModerationOptions { store_path: None, salt: String::new(), now: crate::util::system_clock() }
    }
}

/// Context of a status / refusal check (`{username, deviceId, address}`).
#[derive(Default, Clone)]
pub struct Who {
    pub username: String,
    pub device_id: Option<Value>,
    pub address: String,
}

pub struct Moderation {
    store_path: Option<PathBuf>,
    salt: String,
    now: Clock,
    accounts: Vec<Record>,
    cooldowns: Vec<(String, Vec<f64>, f64)>,
    device_address: Vec<(String, String, f64)>,
    builds: Vec<(String, Value)>,
    secret: String,
}

impl Moderation {
    pub fn new(opts: ModerationOptions) -> Moderation {
        let mut this = Moderation {
            store_path: opts.store_path,
            salt: opts.salt.clone(),
            now: opts.now,
            accounts: Vec::new(),
            cooldowns: Vec::new(),
            device_address: Vec::new(),
            builds: Vec::new(),
            secret: opts.salt,
        };
        if let Some(path) = this.store_path.clone() {
            match std::fs::read_to_string(&path) {
                Ok(text) => match serde_json::from_str::<Value>(&text) {
                    Ok(saved) => this.load(&saved),
                    Err(error) => log_warn(&format!("Moderation store was not loaded: {error}")),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log_warn(&format!("Moderation store was not loaded: {error}")),
            }
        }
        if this.secret.is_empty() {
            this.secret = hex(&random_bytes(32));
        }
        if this.store_path.is_some() {
            this.persist();
        }
        this
    }

    fn load(&mut self, saved: &Value) {
        if self.secret.is_empty() {
            if let Some(Value::String(secret)) = saved.get("secret") {
                if secret.encode_utf16().count() >= 32 {
                    self.secret = secret.clone();
                }
            }
        }
        if let Some(Value::Array(list)) = saved.get("accounts") {
            for value in list {
                let username = username_of(value.get("username"));
                if username_pattern().is_match(&username) {
                    let mut record = Record::normalize(value);
                    record.username = username.clone();
                    self.upsert(record);
                }
            }
        }
        if let Some(Value::Array(list)) = saved.get("cooldowns") {
            for pair in list {
                let hash = pair.get(0).map(crate::util::js_string_of).unwrap_or_default();
                if build_hash_pattern().is_match(&hash) {
                    let value = pair.get(1).cloned().unwrap_or(Value::Null);
                    let kicks = match value.get("kicks") {
                        Some(Value::Array(items)) => {
                            items.iter().map(|v| js_number(Some(v))).filter(|n| n.is_finite()).collect()
                        }
                        _ => Vec::new(),
                    };
                    let until = crate::util::number_or(value.get("until"), 0.0);
                    self.cooldowns.retain(|(h, _, _)| *h != hash);
                    self.cooldowns.push((hash, kicks, until));
                }
            }
        }
        if let Some(Value::Array(list)) = saved.get("deviceAddress") {
            for pair in list {
                let hash = pair.get(0).map(crate::util::js_string_of).unwrap_or_default();
                let value = pair.get(1).cloned().unwrap_or(Value::Null);
                let ip = str_nullish(value.get("ip"));
                if build_hash_pattern().is_match(&hash) && matches!(value.get("ip"), Some(Value::String(_)))
                    && build_hash_pattern().is_match(&ip)
                {
                    let at = crate::util::number_or(value.get("at"), 0.0);
                    self.device_address.retain(|(h, _, _)| *h != hash);
                    self.device_address.push((hash, ip, at));
                }
            }
        }
        if let Some(Value::Array(list)) = saved.get("builds") {
            for value in list {
                if let Some(Value::String(hash)) = value.get("buildHash") {
                    if build_hash_pattern().is_match(hash) {
                        self.set_build(hash.clone(), value.clone());
                    }
                }
            }
        }
    }

    fn set_build(&mut self, hash: String, value: Value) {
        if let Some(slot) = self.builds.iter_mut().find(|(h, _)| *h == hash) {
            slot.1 = value;
        } else {
            self.builds.push((hash, value));
        }
    }

    fn upsert(&mut self, record: Record) {
        if let Some(slot) = self.accounts.iter_mut().find(|r| r.username == record.username) {
            *slot = record;
        } else {
            self.accounts.push(record);
        }
    }

    fn builds_json(&self) -> Value {
        Value::Array(self.builds.iter().map(|(_, v)| v.clone()).collect())
    }

    fn persist(&self) {
        let Some(path) = &self.store_path else { return };
        let mut store = Map::new();
        store.insert("version".into(), json!(1));
        if self.salt.is_empty() {
            store.insert("secret".into(), Value::String(self.secret.clone()));
        }
        store.insert("accounts".into(), Value::Array(self.accounts.iter().map(Record::to_json).collect()));
        store.insert(
            "cooldowns".into(),
            Value::Array(
                self.cooldowns
                    .iter()
                    .map(|(h, kicks, until)| {
                        json!([h, { "kicks": kicks.iter().map(|k| num(*k)).collect::<Vec<_>>(), "until": num(*until) }])
                    })
                    .collect(),
            ),
        );
        store.insert(
            "deviceAddress".into(),
            Value::Array(self.device_address.iter().map(|(h, ip, at)| json!([h, { "ip": ip, "at": num(*at) }])).collect()),
        );
        store.insert("builds".into(), self.builds_json());
        if let Err(error) = write_atomic(path, stringify(&Value::Object(store)).as_bytes()) {
            crate::util::log_error(&format!("Moderation store write failed: {error}"));
        }
    }

    /// `hashOf(value)`: sha256(secret + value), "" for an empty value.
    pub fn hash_of(&self, value: Option<&Value>) -> String {
        let raw = js_slice(str_nullish(value).trim(), 256);
        if raw.is_empty() {
            String::new()
        } else {
            sha256_hex(format!("{}{}", self.secret, raw))
        }
    }

    fn hash_str(&self, value: &str) -> String {
        self.hash_of(Some(&Value::String(value.to_string())))
    }

    pub fn prune(&mut self) {
        let timestamp = (self.now)() as f64;
        for entry in self.cooldowns.iter_mut() {
            entry.1.retain(|at| timestamp - at < KICK_WINDOW_MS as f64);
        }
        self.cooldowns.retain(|(_, kicks, until)| !(kicks.is_empty() && *until <= timestamp));
        self.device_address.retain(|(_, _, at)| !(timestamp - at > KICK_WINDOW_MS as f64));
        while self.device_address.len() > MAX_HASH_ENTRIES {
            self.device_address.remove(0);
        }
        while self.cooldowns.len() > MAX_HASH_ENTRIES {
            self.cooldowns.remove(0);
        }
    }

    fn index_of(&self, username: &str) -> Option<usize> {
        let name = username_of_str(username);
        if !username_pattern().is_match(&name) {
            return None;
        }
        self.accounts.iter().position(|r| r.username == name)
    }

    fn record_create(&mut self, username: &str) -> Option<usize> {
        let name = username_of_str(username);
        if !username_pattern().is_match(&name) {
            return None;
        }
        if let Some(i) = self.accounts.iter().position(|r| r.username == name) {
            return Some(i);
        }
        self.accounts.push(Record::normalize(&json!({ "username": name })));
        Some(self.accounts.len() - 1)
    }

    fn active(entry: &Record, timestamp: f64) -> bool {
        entry.permanent || entry.flagged_until > timestamp
    }

    fn last_reason(entry: &Record) -> Value {
        entry.flags.last().and_then(|flag| flag.get("reason").cloned()).unwrap_or(Value::String(String::new()))
    }

    fn add_review(&mut self, index: usize, host: &dyn ModerationHost, reason: &str, detail: &str) -> bool {
        let at = (self.now)();
        let entry = &mut self.accounts[index];
        if host.is_owner(&entry.username) {
            return false;
        }
        let detail = text(Some(&Value::String(detail.to_string())), 200);
        entry.reviews.push(json!({ "at": at, "reason": reason, "detail": detail, "open": true }));
        let excess = entry.reviews.len().saturating_sub(MAX_LOG_ENTRIES);
        entry.reviews.drain(..excess);
        log_info(&format!("moderation: review opened for {} ({reason})", entry.username));
        true
    }

    fn link_device(&mut self, device_id: Option<&Value>, address: &str) {
        let device = self.hash_of(device_id);
        let ip = self.hash_str(address);
        if device.is_empty() || ip.is_empty() {
            return;
        }
        let previous = self.device_address.iter().position(|(h, _, _)| *h == device).map(|i| self.device_address.remove(i));
        self.device_address.push((device, ip.clone(), (self.now)() as f64));
        if previous.map(|p| p.1) != Some(ip) {
            self.persist();
        }
    }

    fn cooldown_until(&self, _device_id: Option<&Value>, _address: &str) -> f64 {
        if !AUTOMATIC_COOLDOWNS {
            return 0.0;
        }
        0.0
    }

    /// `status({username, deviceId, address})`.
    pub fn status(&self, who: &Who) -> Value {
        let timestamp = (self.now)() as f64;
        let entry = self.index_of(&who.username).map(|i| &self.accounts[i]);
        let flagged = entry.map(|e| Self::active(e, timestamp)).unwrap_or(false);
        let permanent = entry.map(|e| e.permanent).unwrap_or(false);
        let until = if flagged && !permanent { num(entry.unwrap().flagged_until) } else { Value::Null };
        let cooldown = self.cooldown_until(who.device_id.as_ref(), &who.address);
        let message = if permanent {
            "Your account is banned from shared play: you cannot join the Public Route, host public rooms, or upload skins. Solo play and private rooms still work.".to_string()
        } else if flagged {
            format!("Your account is flagged until {}: until then you cannot join the Public Route, host public rooms, or upload skins. Solo play and private rooms still work.", format_utc(entry.unwrap().flagged_until))
        } else if cooldown != 0.0 {
            format!("You were recently kicked from a game: you cannot join the Public Route until {}. Solo play and private rooms still work.", format_utc(cooldown))
        } else {
            String::new()
        };
        json!({
            "flagged": flagged,
            "permanent": permanent,
            "until": until,
            "reason": if flagged { Self::last_reason(entry.unwrap()) } else { json!("") },
            "troll": entry.map(|e| e.troll).unwrap_or(false),
            "restrictions": { "publicRoute": flagged || cooldown != 0.0, "hostPublic": flagged, "skinUpload": flagged },
            "cooldownUntil": if cooldown != 0.0 { num(cooldown) } else { Value::Null },
            "message": message,
        })
    }

    fn refusal(current: &Value, restriction: &str) -> Option<Reply> {
        if !truthy(current["restrictions"].get(restriction)) {
            return None;
        }
        let permanent = truthy(current.get("permanent"));
        let flagged = truthy(current.get("flagged"));
        let until = if permanent {
            Value::Null
        } else if flagged {
            current["until"].clone()
        } else {
            current["cooldownUntil"].clone()
        };
        Some((
            403,
            json!({ "ok": false, "code": "FLAGGED", "error": current["message"], "until": until,
                "permanent": permanent, "cooldown": !flagged }),
        ))
    }

    pub fn check_public_join(&mut self, who: &Who) -> Option<Reply> {
        if truthy(who.device_id.as_ref()) && !who.address.is_empty() {
            self.link_device(who.device_id.as_ref(), &who.address.clone());
        }
        Self::refusal(&self.status(who), "publicRoute")
    }

    pub fn check_host_public(&self, who: &Who) -> Option<Reply> {
        Self::refusal(&self.status(who), "hostPublic")
    }

    pub fn check_skin_upload(&self, username: &str) -> Option<Reply> {
        Self::refusal(&self.status(&Who { username: username.to_string(), ..Who::default() }), "skinUpload")
    }

    fn known_build(&self, host: &dyn ModerationHost, build_hash: &str, platform: &str, channel: &str, username: &str) -> bool {
        if channel == "debug" && !username.is_empty() && host.is_owner(username) {
            return true;
        }
        if self.builds.iter().any(|(h, _)| h == build_hash) {
            return true;
        }
        let updates = host.updates();
        match updates.get(channel).and_then(|c| c.get(platform)) {
            Some(entry) if truthy(Some(entry)) => str_or_empty(entry.get("sha256")).trim().to_lowercase() == build_hash,
            _ => false,
        }
    }

    /// `integrity(body, {username, address})`.
    pub fn integrity(&mut self, host: &dyn ModerationHost, body: &Value, username: &str, address: &str) -> Reply {
        let build_hash = str_or_empty(body.get("buildHash")).trim().to_lowercase();
        if !build_hash_pattern().is_match(&build_hash) {
            return (400, json!({ "ok": false, "code": "INVALID_BUILD_HASH", "error": "buildHash must be a lowercase hex sha256" }));
        }
        let channel = str_or_empty(body.get("channel")).trim().to_lowercase();
        if channel != "debug" && channel != "release" {
            return (400, json!({ "ok": false, "code": "INVALID_CHANNEL", "error": "channel must be debug or release" }));
        }
        let platform = text(body.get("platform"), 32).to_lowercase();
        let commit = str_or_empty(body.get("commit")).trim().to_lowercase();
        if truthy(body.get("deviceId")) && !address.is_empty() {
            self.link_device(body.get("deviceId"), address);
        }
        let known = self.known_build(host, &build_hash, &platform, &channel, username);
        if !username.is_empty() {
            if let Some(index) = self.record_create(username) {
                let at = (self.now)();
                self.accounts[index].integrity = json!({
                    "commit": if commit_pattern().is_match(&commit) { commit.clone() } else { String::new() },
                    "hash": build_hash, "known": known, "at": at,
                });
                let hash_value = Value::String(build_hash.clone());
                if !known && !self.accounts[index].flagged_hashes.contains(&hash_value) {
                    let context = {
                        let c = text(body.get("context"), 8);
                        if c.is_empty() {
                            "signin".to_string()
                        } else {
                            c
                        }
                    };
                    let detail = format!("unknown {channel}/{platform} build {} ({context})", &build_hash[..12]);
                    if self.add_review(index, host, "integrity", &detail) {
                        self.accounts[index].flagged_hashes.push(hash_value);
                    }
                }
            }
            self.persist();
        }
        let status = self.status(&Who {
            username: username.to_string(),
            device_id: body.get("deviceId").cloned(),
            address: address.to_string(),
        });
        (200, json!({ "ok": true, "known": known, "status": status }))
    }

    /// `reportKick(body, {room, host})`.
    pub fn report_kick(&mut self, host_ctx: &dyn ModerationHost, body: &Value, room: &str, host: &str) -> Reply {
        let kind = {
            let k = str_or_empty(body.get("kind"));
            if k.is_empty() {
                "kick".to_string()
            } else {
                k
            }
        };
        if kind != "kick" {
            return (400, json!({ "ok": false, "code": "INVALID_KIND", "error": "kind must be kick" }));
        }
        self.prune();
        let timestamp = (self.now)();
        let reason = {
            let r = text(body.get("reason"), 80);
            if r.is_empty() {
                "kick".to_string()
            } else {
                r
            }
        };
        let flagged = false;
        let username = username_of(body.get("username"));
        if username_pattern().is_match(&username) && host_ctx.account_exists(&username) {
            let index = self.record_create(&username).expect("valid username");
            let host_hash: String = self.hash_str(&format!("host:{}", if host.is_empty() { room } else { host })).chars().take(16).collect();
            let strikes = {
                let n = crate::util::number_or(body.get("strikes"), 0.0).trunc();
                n.min(99.0).max(0.0)
            };
            let entry = &mut self.accounts[index];
            entry.kicks.push(json!({ "at": timestamp, "room": room, "host": host_hash, "reason": reason, "strikes": num(strikes) }));
            let excess = entry.kicks.len().saturating_sub(MAX_LOG_ENTRIES);
            entry.kicks.drain(..excess);
            let recent: HashSet<String> = entry
                .kicks
                .iter()
                .filter(|kick| (timestamp as f64) - js_number(kick.get("at")) < KICK_WINDOW_MS as f64)
                .map(|kick| {
                    let h = str_or_empty(kick.get("host"));
                    if h.is_empty() {
                        format!("{}:{}", str_nullish(kick.get("room")), str_nullish(kick.get("at")))
                    } else {
                        h
                    }
                })
                .collect();
            let recent = recent.len();
            if recent >= KICK_FLAG_THRESHOLD {
                self.add_review(index, host_ctx, "kicks", &format!("kicked by {recent} different hosts in 7 days (last: {reason})"));
            } else {
                self.add_review(index, host_ctx, "kick", &format!("kicked: {reason}"));
            }
        }
        self.persist();
        let status = self.status(&Who { username, device_id: body.get("deviceId").cloned(), address: String::new() });
        (200, json!({ "ok": true, "flagged": flagged, "status": status }))
    }

    pub fn peers(&self, body: &Value) -> Reply {
        let mut players = Map::new();
        if let Some(Value::Array(names)) = body.get("usernames") {
            for value in names.iter().take(MAX_PEER_ENTRIES) {
                let username = username_of(Some(value));
                if !username_pattern().is_match(&username) {
                    continue;
                }
                let current = self.status(&Who { username: username.clone(), ..Who::default() });
                let flagged = truthy(current.get("flagged"));
                let ads = self.index_of(&username).map(|i| self.accounts[i].ads).unwrap_or(true);
                players.insert(
                    username,
                    json!({ "flagged": flagged, "permanent": current["permanent"], "troll": truthy(current.get("troll")) || flagged,
                        "ads": ads, "reason": current["reason"] }),
                );
            }
        }
        let mut devices = Map::new();
        if let Some(Value::Array(ids)) = body.get("deviceIds") {
            for value in ids.iter().take(MAX_PEER_ENTRIES) {
                let key = js_slice(&str_nullish(Some(value)), 256);
                if key.is_empty() {
                    continue;
                }
                let cooldown = self.cooldown_until(Some(&Value::String(key.clone())), "") != 0.0;
                devices.insert(key, json!({ "cooldown": cooldown, "troll": false }));
            }
        }
        (200, json!({ "ok": true, "players": players, "devices": devices }))
    }

    fn summary(&self, entry: &Record) -> Value {
        let timestamp = (self.now)() as f64;
        json!({
            "username": entry.username,
            "flagged": Self::active(entry, timestamp),
            "permanent": entry.permanent,
            "flaggedUntil": if entry.flagged_until != 0.0 { num(entry.flagged_until) } else { Value::Null },
            "troll": entry.troll,
            "ads": entry.ads,
            "reason": Self::last_reason(entry),
            "flags": entry.flags,
            "kicks": entry.kicks,
            "integrity": entry.integrity,
            "reviews": entry.reviews,
            "reviewOpen": entry.open_reviews(),
        })
    }

    fn latest_at(entry: &Record) -> f64 {
        let mut values = vec![0.0];
        values.extend(entry.flags.iter().map(|f| js_number(f.get("at"))));
        values.extend(entry.kicks.iter().map(|k| js_number(k.get("at"))));
        values.extend(entry.reviews.iter().map(|r| js_number(r.get("at"))));
        values.push(crate::util::number_or(entry.integrity.get("at"), 0.0));
        if values.iter().any(|v| v.is_nan()) {
            return f64::NAN;
        }
        values.into_iter().fold(f64::NEG_INFINITY, f64::max)
    }

    pub fn admin_list(&self) -> Reply {
        let timestamp = (self.now)() as f64;
        let mut list: Vec<&Record> = self
            .accounts
            .iter()
            .filter(|e| Self::active(e, timestamp) || !e.kicks.is_empty() || e.troll || !e.flags.is_empty() || !e.reviews.is_empty())
            .collect();
        list.sort_by(|a, b| {
            let open = (b.open_reviews() > 0) as i32 - (a.open_reviews() > 0) as i32;
            if open != 0 {
                return open.cmp(&0);
            }
            let diff = Self::latest_at(b) - Self::latest_at(a);
            if diff > 0.0 {
                std::cmp::Ordering::Greater
            } else if diff < 0.0 {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        let accounts: Vec<Value> = list.iter().map(|e| self.summary(e)).collect();
        let reviews: Vec<Value> = list.iter().filter(|e| e.open_reviews() > 0).map(|e| json!(e.username)).collect();
        (200, json!({ "ok": true, "accounts": accounts, "reviews": reviews, "builds": self.builds_json() }))
    }

    pub fn admin_set(&mut self, host: &dyn ModerationHost, body: &Value, by: &str) -> Reply {
        let username = username_of(body.get("username"));
        if !username_pattern().is_match(&username) || !host.account_exists(&username) {
            return (404, json!({ "ok": false, "code": "ACCOUNT_NOT_FOUND", "error": "That account was not found" }));
        }
        let action = str_or_empty(body.get("action"));
        let index = self.record_create(&username).expect("valid username");
        let timestamp = (self.now)();
        {
            let entry = &mut self.accounts[index];
            match action.as_str() {
                "clear" => entry.flagged_until = 0.0,
                "extend" => {
                    let hours = if body.get("hours").is_none() { 24.0 } else { js_number(body.get("hours")) };
                    if !hours.is_finite() || hours <= 0.0 || hours > MAX_EXTEND_HOURS {
                        return (400, json!({ "ok": false, "code": "INVALID_HOURS", "error": format!("hours must be between 1 and {}", MAX_EXTEND_HOURS as i64) }));
                    }
                    entry.flagged_until = (timestamp as f64).max(entry.flagged_until) + crate::util::js_round(hours * HOUR_MS as f64);
                }
                "ban" => entry.permanent = true,
                "unban" => entry.permanent = false,
                "troll_on" => entry.troll = true,
                "troll_off" => entry.troll = false,
                "ads_on" => entry.ads = true,
                "ads_off" => entry.ads = false,
                "dismiss" => {}
                _ => return (400, json!({ "ok": false, "code": "INVALID_ACTION", "error": "Unknown moderation action" })),
            }
            let until = if entry.permanent || entry.flagged_until == 0.0 { Value::Null } else { num(entry.flagged_until) };
            entry.flags.push(json!({ "at": timestamp, "reason": format!("admin:{action}"), "detail": text(body.get("note"), 200),
                "until": until, "by": by }));
            let excess = entry.flags.len().saturating_sub(MAX_LOG_ENTRIES);
            entry.flags.drain(..excess);
            for review in entry.reviews.iter_mut() {
                if truthy(review.get("open")) {
                    if let Value::Object(map) = review {
                        map.insert("open".into(), Value::Bool(false));
                        map.insert("closedBy".into(), Value::String(by.to_string()));
                        map.insert("closedAt".into(), json!(timestamp));
                        map.insert("action".into(), Value::String(action.clone()));
                    }
                }
            }
        }
        self.persist();
        let record = self.summary(&self.accounts[index]);
        (200, json!({ "ok": true, "record": record }))
    }

    pub fn add_build(&mut self, body: &Value, by: &str) -> Reply {
        let build_hash = str_or_empty(body.get("buildHash")).trim().to_lowercase();
        if !build_hash_pattern().is_match(&build_hash) {
            return (400, json!({ "ok": false, "code": "INVALID_BUILD_HASH", "error": "buildHash must be a lowercase hex sha256" }));
        }
        let commit = str_or_empty(body.get("commit")).trim().to_lowercase();
        let entry = json!({ "buildHash": build_hash, "commit": if commit_pattern().is_match(&commit) { commit } else { String::new() },
            "note": text(body.get("note"), 200), "by": by, "at": (self.now)() });
        self.set_build(build_hash, entry.clone());
        self.persist();
        (200, json!({ "ok": true, "build": entry, "builds": self.builds_json() }))
    }

    pub fn remove_build(&mut self, body: &Value) -> Reply {
        let build_hash = str_or_empty(body.get("buildHash")).trim().to_lowercase();
        let before = self.builds.len();
        self.builds.retain(|(h, _)| *h != build_hash);
        let removed = self.builds.len() != before;
        if removed {
            self.persist();
        }
        (200, json!({ "ok": true, "removed": removed, "builds": self.builds_json() }))
    }

    /// Keep a record attached to its account when the player renames, so a rename cannot shed a flag.
    pub fn rename_account(&mut self, from: &str, to: &str) {
        let source = username_of_str(from);
        let target = username_of_str(to);
        let Some(index) = self.accounts.iter().position(|r| r.username == source) else { return };
        if source == target {
            return;
        }
        let mut entry = self.accounts.remove(index);
        entry.username = target;
        self.upsert(entry);
        self.persist();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::manual_clock;

    struct Host;
    impl ModerationHost for Host {
        fn is_owner(&self, name: &str) -> bool {
            name == "KYTTU"
        }
        fn account_exists(&self, name: &str) -> bool {
            name != "NOBODY_HERE" && name != "GHOST"
        }
        fn updates(&self) -> Value {
            json!({ "release": { "windows": { "sha256": "b".repeat(64) } } })
        }
    }

    fn store(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("lr_moderation_{name}_{}", hex(&random_bytes(6)))).join("moderation.json")
    }

    #[test]
    fn kicks_open_reviews_but_never_restrict_and_distinct_hosts_escalate_the_reason() {
        let (clock, _) = manual_clock(1_793_188_800_000);
        let mut m = Moderation::new(ModerationOptions { store_path: None, salt: "s".into(), now: clock });
        for host in ["user:HOST_A", "user:HOST_A", "user:HOST_B", "user:HOST_B"] {
            let (status, body) = m.report_kick(&Host, &json!({ "username": "cheater", "kind": "kick", "reason": "speed", "strikes": 3 }), "RMAAA", host);
            assert_eq!(status, 200);
            assert_eq!(body["flagged"], json!(false));
            assert_eq!(body["status"]["restrictions"], json!({ "publicRoute": false, "hostPublic": false, "skinUpload": false }));
        }
        let (_, list) = m.admin_list();
        assert_eq!(list["reviews"], json!(["CHEATER"]));
        let reasons: Vec<_> = list["accounts"][0]["reviews"].as_array().unwrap().iter().map(|r| r["reason"].clone()).collect();
        assert_eq!(reasons, vec![json!("kick"), json!("kick"), json!("kicks"), json!("kicks")]);
        assert_eq!(list["accounts"][0]["reviewOpen"], json!(4));
        // owners and unknown accounts never get records
        m.report_kick(&Host, &json!({ "username": "KYTTU" }), "RMAAA", "user:HOST_A");
        m.report_kick(&Host, &json!({ "username": "NOBODY_HERE" }), "RMAAA", "user:HOST_A");
        let (_, list) = m.admin_list();
        assert_eq!(list["accounts"].as_array().unwrap().len(), 2, "KYTTU's kick is logged but never reviewed");
        assert_eq!(list["reviews"], json!(["CHEATER"]));
    }

    #[test]
    fn owner_flags_restrict_shared_play_and_extend_from_the_current_until() {
        let (clock, handle) = manual_clock(1_793_188_800_000);
        let path = store("flags");
        let mut m = Moderation::new(ModerationOptions { store_path: Some(path.clone()), salt: String::new(), now: clock });
        let t = handle.get() as f64;
        let (_, r) = m.admin_set(&Host, &json!({ "username": "target", "action": "extend", "hours": 5, "note": "manual" }), "KYTTU");
        assert_eq!(r["record"]["flaggedUntil"], num(t + 5.0 * HOUR_MS as f64));
        let (_, r) = m.admin_set(&Host, &json!({ "username": "target", "action": "extend" }), "KYTTU");
        assert_eq!(r["record"]["flaggedUntil"], num(t + 29.0 * HOUR_MS as f64));
        assert_eq!(m.admin_set(&Host, &json!({ "username": "target", "action": "extend", "hours": 24 * 91 }), "KYTTU").0, 400);
        let refused = m.check_public_join(&Who { username: "TARGET".into(), ..Who::default() }).unwrap();
        assert_eq!(refused.0, 403);
        assert_eq!(refused.1["code"], json!("FLAGGED"));
        assert!(refused.1["error"].as_str().unwrap().contains("Public Route"));
        assert!(m.check_skin_upload("TARGET").is_some());
        m.admin_set(&Host, &json!({ "username": "target", "action": "clear" }), "KYTTU");
        let (_, r) = m.admin_set(&Host, &json!({ "username": "target", "action": "ban" }), "KYTTU");
        assert_eq!(r["record"]["permanent"], json!(true));
        let refused = m.check_host_public(&Who { username: "TARGET".into(), ..Who::default() }).unwrap();
        assert_eq!((refused.1["permanent"].clone(), refused.1["until"].clone()), (json!(true), Value::Null));
        assert_eq!(m.admin_set(&Host, &json!({ "username": "target", "action": "explode" }), "KYTTU").0, 400);
        assert_eq!(m.admin_set(&Host, &json!({ "username": "GHOST", "action": "ban" }), "KYTTU").0, 404);
        // a rename carries the record; nothing raw is stored
        m.check_public_join(&Who { username: String::new(), device_id: Some(json!("secret-device-id-42")), address: "127.0.0.1".into() });
        m.rename_account("TARGET", "NEW_NAME");
        assert_eq!(m.status(&Who { username: "NEW_NAME".into(), ..Who::default() })["permanent"], json!(true));
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(stored.starts_with("{\"version\":1,\"secret\":\""));
        assert!(!stored.contains("secret-device-id-42") && !stored.contains("127.0.0.1") && stored.contains("NEW_NAME"));
        // the persisted secret keeps hashes stable across a restart
        let (clock2, _) = manual_clock(handle.get());
        let again = Moderation::new(ModerationOptions { store_path: Some(path.clone()), salt: String::new(), now: clock2 });
        assert_eq!(again.hash_of(Some(&json!("x"))), m.hash_of(Some(&json!("x"))));
        assert_eq!(again.status(&Who { username: "NEW_NAME".into(), ..Who::default() })["permanent"], json!(true));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn integrity_reviews_unknown_builds_once_per_hash_and_accepts_manifest_allow_list_and_owner_debug() {
        let (clock, _) = manual_clock(1_793_188_800_000);
        let mut m = Moderation::new(ModerationOptions { store_path: None, salt: "s".into(), now: clock });
        let b = "b".repeat(64);
        let a = "a".repeat(64);
        assert_eq!(m.integrity(&Host, &json!({ "buildHash": a, "channel": "release", "platform": "windows" }), "", "").1["known"], json!(false));
        assert_eq!(m.integrity(&Host, &json!({ "buildHash": b, "channel": "release", "platform": "windows" }), "MODDER", "").1["known"], json!(true));
        assert_eq!(m.integrity(&Host, &json!({ "buildHash": b, "channel": "release", "platform": "linux" }), "MODDER", "").1["known"], json!(false));
        m.integrity(&Host, &json!({ "buildHash": b, "channel": "release", "platform": "linux" }), "MODDER", "");
        let (_, list) = m.admin_list();
        assert_eq!(list["accounts"][0]["reviews"].as_array().unwrap().len(), 1);
        assert_eq!(list["accounts"][0]["integrity"]["hash"], json!(b));
        assert_eq!(m.integrity(&Host, &json!({ "buildHash": a, "channel": "debug", "platform": "macos" }), "KYTTU", "").1["known"], json!(true));
        assert_eq!(m.add_build(&json!({ "buildHash": a, "commit": "1234567", "note": "test" }), "KYTTU").1["ok"], json!(true));
        assert_eq!(m.integrity(&Host, &json!({ "buildHash": a, "channel": "release", "platform": "windows" }), "MODDER", "").1["known"], json!(true));
        assert_eq!(m.remove_build(&json!({ "buildHash": a })).1["removed"], json!(true));
        assert_eq!(m.integrity(&Host, &json!({ "buildHash": "zz", "channel": "release" }), "", "").0, 400);
    }

    #[test]
    fn peers_report_troll_ads_and_never_cool_devices_down() {
        let (clock, _) = manual_clock(1_793_188_800_000);
        let mut m = Moderation::new(ModerationOptions { store_path: None, salt: "s".into(), now: clock });
        m.admin_set(&Host, &json!({ "username": "PRANKER", "action": "troll_on" }), "KYTTU");
        let (_, r) = m.peers(&json!({ "usernames": ["pranker", "KYTTU"], "deviceIds": ["dev-x", "dev-y"] }));
        assert_eq!(r["players"]["PRANKER"], json!({ "flagged": false, "permanent": false, "troll": true, "ads": true, "reason": "" }));
        assert_eq!(r["players"]["KYTTU"]["troll"], json!(false));
        assert_eq!(r["devices"], json!({ "dev-x": { "cooldown": false, "troll": false }, "dev-y": { "cooldown": false, "troll": false } }));
        m.admin_set(&Host, &json!({ "username": "PRANKER", "action": "ads_off" }), "KYTTU");
        assert_eq!(m.peers(&json!({ "usernames": ["PRANKER"] })).1["players"]["PRANKER"]["ads"], json!(false));
        let ids: Vec<String> = (0..30).map(|i| format!("d{i}")).collect();
        assert_eq!(m.peers(&json!({ "deviceIds": ids })).1["devices"].as_object().unwrap().len(), 16);
    }
}
