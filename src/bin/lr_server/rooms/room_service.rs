//! Port of `backend/room_service.mjs`: in-memory web rooms (join/state sync, host settings, Hide & Seek rounds,
//! Last Route launch/abandon, custom maps, shared dialog), with player and room expiry.
//!
//! Every handler takes the parsed JSON request body and returns a `Reply` (`[status, body]` in the original), with
//! the same status codes, error codes, error strings and body key order.

use crate::hunter_id_aliases::canonical_hunter_id_value;
use crate::util::{
    field, hex, js_len, js_number, js_slice, js_trim, num, random_bytes, safe_equal_values, str_or_empty, system_clock,
    truthy, Clock, Reply,
};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub const MAX_PLAYERS: usize = 16;
pub const MAX_ROOMS: usize = 512;
pub const HIDE_PHASE_MS: i64 = 30_000;
pub const ITEM_LIMIT: usize = 96;
/// Most item records a room keeps for one depth. A standard floor has 5 relays and 7 batteries plus its props; an
/// Infinite chunk adds 3 relays and 4 batteries plus props, so this holds hundreds of chunks. Claims for new keys past
/// it are ignored; existing keys still update. A depth advance clears the map.
pub const ITEM_TOTAL_LIMIT: usize = 4096;
pub const DISCOVERY_LIMIT: usize = 512;
pub const DISCOVERY_TOTAL_LIMIT: usize = 30_000;
/// Discovery cap for endless (Infinite) rooms: far above a standard map, but bounded so memory cannot grow forever.
/// Matches DISCOVERY_ENDLESS_LIMIT in backend/room_service.mjs.
pub const DISCOVERY_ENDLESS_LIMIT: usize = 250_000;
/// Most floors one host `claimDepth` may move the room past its `activeDepth`. The client only ever goes one floor
/// down at a time (the service lift is `depth += 1`; stairs, ladders, route lifts and the sewer stay inside one depth's
/// level space), so a real host claims `activeDepth + 1`. Three leaves room for a host that took two more lifts
/// between two state posts. A room at depth 0 (new, or reset by a route start or abandon) takes the host's first claim
/// at any depth up to the 9 999 cap, because that is where the host's run starts (a resumed run can begin deep).
/// A larger jump is ignored, not an error, so the room keeps its items, world and discovery.
/// Matches MAX_DEPTH_CLAIM_STEP in backend/room_service.mjs.
pub const MAX_DEPTH_CLAIM_STEP: i64 = 3;

const PERSONAS: [&str; 27] = [
    "none", "aki", "briselle", "ceryth", "dovelle", "elise", "ferryn", "glyra", "haelith", "ilyra", "javelle", "kathy",
    "loxelle", "mavrine", "nyxie", "opalie", "porcelle", "quenelle", "ravelle", "solyth", "thessine", "umbriel",
    "verrine", "wynelle", "xivvy", "ysolde", "zellia",
];

/// `DEFAULT_ROOM_SETTINGS` (the standard Last Route lobby).
pub fn default_room_settings() -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("mapScale".into(), json!("standard"));
    map.insert("difficulty".into(), json!("standard"));
    map.insert("mode".into(), json!("lastRoute"));
    map.insert("fakeRunners".into(), json!(false));
    map
}

const SETTING_KEYS: [&str; 6] = ["mapScale", "difficulty", "mode", "hiderNpcs", "seekerNpcs", "fakeRunners"];

/// `SETTING_VALUES[key].has(value)` (SameValueZero, no coercion). Returns the value to store.
fn setting_value_allowed(key: &str, value: &Value) -> Option<Value> {
    let strings: &[&str] = match key {
        "mapScale" => &["small", "standard", "large", "huge", "colossal", "endless"],
        "difficulty" => &["relaxed", "standard", "nightmare"],
        "mode" => &["lastRoute", "hideAndSeek"],
        "hiderNpcs" | "seekerNpcs" => {
            return match value {
                Value::Number(n) => {
                    let f = n.as_f64().unwrap_or(f64::NAN);
                    if f.fract() == 0.0 && (0.0..=8.0).contains(&f) {
                        Some(num(f))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
        "fakeRunners" => return matches!(value, Value::Bool(_)).then(|| value.clone()),
        _ => return None,
    };
    match value {
        Value::String(s) if strings.contains(&s.as_str()) => Some(value.clone()),
        _ => None,
    }
}

fn finite(value: Option<&Value>, fallback: f64, minimum: f64, maximum: f64) -> f64 {
    let number = js_number(value);
    if number.is_finite() {
        minimum.max(maximum.min(number))
    } else {
        fallback
    }
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

pub(crate) fn clean_room(value: Option<&Value>) -> String {
    let upper = str_or_empty(value).to_uppercase();
    upper.chars().filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit()).take(8).collect()
}

pub(crate) fn clean_player_id(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .take(72)
            .collect(),
        _ => String::new(),
    }
}

pub(crate) fn clean_name(value: Option<&Value>) -> String {
    let kept: String = str_or_empty(value)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '_' || *c == '-')
        .collect();
    // Only plain spaces survive the filter, so /\s+/g -> " " collapses runs of spaces.
    let mut collapsed = String::with_capacity(kept.len());
    let mut last_space = false;
    for c in kept.chars() {
        if c == ' ' {
            if !last_space {
                collapsed.push(' ');
            }
            last_space = true;
        } else {
            collapsed.push(c);
            last_space = false;
        }
    }
    collapsed.trim().chars().take(16).collect::<String>().to_ascii_uppercase()
}

fn clean_appearance(value: Option<&Value>, fallback: &str) -> String {
    match value {
        Some(Value::String(s)) => {
            let cleaned: String = js_trim(s)
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .take(32)
                .collect();
            if cleaned.is_empty() {
                fallback.to_string()
            } else {
                cleaned
            }
        }
        _ => fallback.to_string(),
    }
}

pub(crate) fn error(status: u16, code: &str, message: &str) -> Reply {
    (status, json!({ "code": code, "error": message }))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub round_id: i64,
    pub phase: String,
    pub seeker_id: String,
    pub seeker_spawn_index: i64,
    pub started_at: i64,
    pub hide_ends_at: i64,
}

impl Round {
    pub fn empty() -> Round {
        Round { round_id: 0, phase: String::new(), seeker_id: String::new(), seeker_spawn_index: -1, started_at: 0, hide_ends_at: 0 }
    }
    pub fn to_json(&self) -> Value {
        json!({
            "roundId": self.round_id,
            "phase": self.phase,
            "seekerId": self.seeker_id,
            "seekerSpawnIndex": self.seeker_spawn_index,
            "startedAt": self.started_at,
            "hideEndsAt": self.hide_ends_at,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub dialog_id: i64,
    pub file: String,
    pub node_id: String,
    pub step_id: i64,
    pub ended: bool,
}

impl Dialog {
    pub fn empty() -> Dialog {
        Dialog { dialog_id: 0, file: String::new(), node_id: String::new(), step_id: 0, ended: false }
    }
    pub fn to_json(&self) -> Value {
        json!({
            "dialogId": self.dialog_id,
            "file": self.file,
            "nodeId": self.node_id,
            "stepId": self.step_id,
            "ended": self.ended,
        })
    }
}

/// Whether a host's `claimDepth` moves a room at `active` depth: deeper, and at most `MAX_DEPTH_CLAIM_STEP` floors
/// deeper unless the room has no depth yet (0).
pub(crate) fn depth_claim_allowed(active: i64, claim: i64) -> bool {
    claim > active && (active == 0 || claim - active <= MAX_DEPTH_CLAIM_STEP)
}

fn empty_world() -> Value {
    json!({ "depth": 0, "endlessExpansion": 0, "hostId": "", "at": 0, "hunters": [], "mannequins": [], "rooms": [] })
}

#[derive(Clone, Debug)]
pub struct Player {
    pub id: String,
    pub session_token: String,
    pub name: String,
    /// The account the player signed in with, as the server verified it from the join's account session (canonical
    /// UPPER CASE), or empty for a guest. Never read from the body. A room KICK/BAN is recorded only against this.
    pub account: String,
    pub spawn_index: i64,
    pub depth: f64,
    pub x: f64,
    pub y: f64,
    pub facing_x: f64,
    pub facing_y: f64,
    pub mode: String,
    pub hidden: bool,
    pub flashlight_on: bool,
    pub radial_active: bool,
    pub fan_on: bool,
    pub heater_on: bool,
    pub frame: f64,
    pub persona: String,
    pub appearance_id: String,
    pub tagged: bool,
    pub noise: f64,
    pub in_safe_zone: bool,
    pub lamp: f64,
    pub travel_speed: f64,
    pub crouching: bool,
    pub vx: f64,
    pub vy: f64,
    pub last_seen: i64,
}

/// A dropped peer's slot, kept until `until` and claimed only with the same peer (session) token.
#[derive(Clone, Debug)]
pub struct Held {
    pub player: Player,
    pub until: i64,
}

/// Why a room was closed by its host (shown to the guests that are still polling).
pub const HOST_LEFT_REASON: &str = "HOST LEFT - ROOM CLOSED";
/// Defaults of the tunables below; each can be overridden by an environment variable (see `env_ms`).
pub const DEFAULT_RECONNECT_RESERVE_MS: i64 = 30_000;
pub const DEFAULT_CLOSE_NOTICE_MS: i64 = 60_000;
pub const MAX_BANS_PER_ROOM: usize = 64;

pub(crate) fn env_ms(name: &str, fallback: i64) -> i64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse::<i64>().ok()).filter(|v| *v >= 0).unwrap_or(fallback)
}

/// Room capacity: `LASTROUTE_ROOM_CAPACITY` clamped to 2..=MAX_PLAYERS (default MAX_PLAYERS).
pub(crate) fn default_capacity() -> usize {
    (env_ms("LASTROUTE_ROOM_CAPACITY", MAX_PLAYERS as i64) as usize).clamp(2, MAX_PLAYERS)
}

#[derive(Clone, Debug)]
pub struct Room {
    pub code: String,
    pub capacity: usize,
    pub admin: bool,
    pub held: Vec<Held>,
    /// Banned player ids and `name:NAME` keys.
    pub bans: HashSet<String>,
    /// Player id -> error code (`KICKED` / `BANNED`), so a removed player's next call says why.
    pub removed: HashMap<String, String>,
    pub world_seed: String,
    pub host_player_id: String,
    pub settings: Map<String, Value>,
    pub round: Round,
    pub round_counter: i64,
    pub launch_id: i64,
    pub abandon_id: i64,
    pub custom_map_file: String,
    pub custom_map_id: i64,
    pub dialog: Dialog,
    pub active_depth: i64,
    pub world: Value,
    pub items: Map<String, Value>,
    pub discovered: HashSet<String>,
    pub discovered_list: Vec<String>,
    pub discovered_epoch: i64,
    pub last_active_at: i64,
    pub players: Vec<Player>,
}

impl Room {
    pub(crate) fn new(code: &str, timestamp: i64) -> Room {
        Room {
            code: code.to_string(),
            capacity: default_capacity(),
            admin: false,
            held: Vec::new(),
            bans: HashSet::new(),
            removed: HashMap::new(),
            world_seed: format!("ROOM-{code}-{}", hex(&random_bytes(10)).to_uppercase()),
            host_player_id: String::new(),
            settings: default_room_settings(),
            round: Round::empty(),
            round_counter: 0,
            launch_id: 0,
            abandon_id: 0,
            custom_map_file: String::new(),
            custom_map_id: 0,
            dialog: Dialog::empty(),
            active_depth: 0,
            world: empty_world(),
            items: Map::new(),
            discovered: HashSet::new(),
            discovered_list: Vec::new(),
            discovered_epoch: 0,
            last_active_at: timestamp,
            players: Vec::new(),
        }
    }

    fn player_index(&self, id: &str) -> Option<usize> {
        self.players.iter().position(|player| player.id == id)
    }

    pub(super) fn reconcile(&mut self) {
        if !self.players.iter().any(|player| player.id == self.host_player_id) {
            let mut sorted: Vec<&Player> = self.players.iter().collect();
            sorted.sort_by_key(|player| player.spawn_index);
            self.host_player_id = sorted.first().map(|player| player.id.clone()).unwrap_or_default();
        }
        let npc_seeker = self.round.seeker_id.starts_with("npc:seeker:");
        let seeker_present =
            npc_seeker || self.players.iter().any(|player| player.spawn_index == self.round.seeker_spawn_index);
        if self.round.round_id > 0 && !seeker_present {
            self.round = Round::empty();
        }
    }

    fn reset_runtime(&mut self) {
        self.active_depth = 0;
        self.world = empty_world();
        self.items = Map::new();
        self.discovered = HashSet::new();
        self.discovered_list = Vec::new();
        self.discovered_epoch += 1;
        self.custom_map_file = String::new();
        self.dialog = Dialog::empty();
        self.round = Round::empty();
        for player in &mut self.players {
            player.tagged = false;
        }
    }

    fn public_player(&self, player: &Player, timestamp: i64) -> Value {
        let round_id = self.round.round_id.max(0);
        let role = if round_id > 0 && player.spawn_index == self.round.seeker_spawn_index {
            "seeker"
        } else if round_id > 0 {
            "hider"
        } else {
            "runner"
        };
        json!({
            "name": player.name,
            "isHost": player.id == self.host_player_id,
            "role": role,
            "released": round_id <= 0 || role == "hider" || timestamp >= self.round.hide_ends_at,
            "tagged": round_id > 0 && player.tagged,
            "roundId": round_id,
            "spawnIndex": player.spawn_index,
            "depth": num(player.depth),
            "x": num(player.x),
            "y": num(player.y),
            "mode": player.mode,
            "facingX": num(player.facing_x),
            "facingY": num(player.facing_y),
            "hidden": player.hidden,
            "flashlightOn": player.flashlight_on,
            "radialActive": player.radial_active,
            "fanOn": player.fan_on,
            "heaterOn": player.heater_on,
            "frame": num(player.frame),
            "persona": player.persona,
            "appearanceId": player.appearance_id,
            "noise": num(player.noise),
            "inSafeZone": player.in_safe_zone,
            "lamp": num(player.lamp),
            "travelSpeed": num(player.travel_speed),
            "crouching": player.crouching,
            "vx": num(player.vx),
            "vy": num(player.vy),
        })
    }

    /// `payload(room, requesterId, timestamp, discovery)`; `discovery` is `(since, epoch)`.
    fn payload(&mut self, requester_id: &str, timestamp: i64, discovery: Option<(i64, i64)>) -> Value {
        self.reconcile();
        let current = self.players.iter().find(|player| player.id == requester_id);
        let cursor_valid = matches!(discovery, Some((_, epoch)) if epoch == self.discovered_epoch);
        let discovered: Vec<String> = match discovery {
            Some((since, _)) if cursor_valid => {
                let start = (since.max(0) as usize).min(self.discovered_list.len());
                self.discovered_list[start..].to_vec()
            }
            _ => self.discovered_list.clone(),
        };
        let mut settings = default_room_settings();
        for (key, value) in &self.settings {
            settings.insert(key.clone(), value.clone());
        }
        let players: Vec<Value> = self
            .players
            .iter()
            .filter(|player| player.id != requester_id)
            .map(|player| self.public_player(player, timestamp))
            .collect();
        json!({
            "room": self.code,
            "worldSeed": self.world_seed,
            "settings": settings,
            "round": self.round.to_json(),
            "launchId": self.launch_id,
            "abandonId": self.abandon_id,
            "customMapFile": self.custom_map_file,
            "customMapId": self.custom_map_id,
            "dialog": self.dialog.to_json(),
            "reloadId": 0,
            "serverNow": timestamp,
            "isHost": current.map(|player| player.id == self.host_player_id).unwrap_or(false),
            "spawnIndex": current.map(|player| player.spawn_index).unwrap_or(-1),
            "roomSize": self.players.len(),
            "maxPlayers": MAX_PLAYERS,
            "sessionToken": current.map(|player| player.session_token.clone()).unwrap_or_default(),
            "activeDepth": self.active_depth,
            "world": self.world,
            "items": self.items,
            "discovered": discovered,
            "discoveredEpoch": self.discovered_epoch,
            "discoveredTotal": self.discovered_list.len(),
            "discoveredReset": !cursor_valid,
            "player": current.map(|player| self.public_player(player, timestamp)).unwrap_or(Value::Null),
            "players": players,
        })
    }
}

pub(crate) fn clean_settings(patch: Option<&Value>, current: &Map<String, Value>) -> Option<Map<String, Value>> {
    let patch = match patch {
        Some(Value::Object(map)) => map,
        _ => return None,
    };
    if patch.keys().any(|key| !SETTING_KEYS.contains(&key.as_str())) {
        return None;
    }
    let mut next = default_room_settings();
    for (key, value) in current {
        next.insert(key.clone(), value.clone());
    }
    for (key, value) in patch {
        match setting_value_allowed(key, value) {
            Some(stored) => {
                next.insert(key.clone(), stored);
            }
            None => {
                // An unknown mode (for example the removed "officeSurvivor" from an old client) falls back to
                // Last Route instead of rejecting the whole settings update.
                if key == "mode" {
                    next.insert("mode".into(), json!("lastRoute"));
                    continue;
                }
                return None;
            }
        }
    }
    Some(next)
}

fn sanitize_entities(source: Option<&Value>, include_name: bool) -> Vec<Value> {
    let entries = match source {
        Some(Value::Array(items)) => items.as_slice(),
        _ => &[],
    };
    let mut output = Vec::new();
    for entry in entries.iter().take(64) {
        let id = match entry {
            Value::Object(map) => match map.get("id") {
                Some(Value::String(id)) if !id.is_empty() => id,
                _ => continue,
            },
            _ => continue,
        };
        let mut out = Map::new();
        out.insert("id".into(), json!(js_slice(id, 48)));
        out.insert("x".into(), num(finite(field(entry, "x"), 0.0, -1_000.0, 1_000.0)));
        out.insert("y".into(), num(finite(field(entry, "y"), 0.0, -1_000.0, 1_000.0)));
        out.insert("facingX".into(), num(finite(field(entry, "facingX"), 1.0, -1.0, 1.0)));
        out.insert("facingY".into(), num(finite(field(entry, "facingY"), 0.0, -1.0, 1.0)));
        out.insert("mode".into(), json!(js_slice(&str_or_empty(field(entry, "mode")), 24)));
        out.insert("frame".into(), num(js_math_round(finite(field(entry, "frame"), 0.0, 0.0, 3.0))));
        let active = match field(entry, "active") {
            None => true,
            other => truthy(other),
        };
        out.insert("active".into(), json!(active));
        if include_name {
            out.insert("name".into(), json!(js_slice(&str_or_empty(field(entry, "name")), 48)));
        } else {
            out.insert("state".into(), json!(js_slice(&str_or_empty(field(entry, "state")), 24)));
            out.insert("alerted".into(), json!(truthy(field(entry, "alerted"))));
            out.insert("adminSummoned".into(), json!(truthy(field(entry, "adminSummoned"))));
        }
        output.push(Value::Object(out));
    }
    output
}

/// `sanitizeWorld(value)`: returns the sanitised world (without hostId/at) or None.
fn sanitize_world(value: &Value) -> Option<Map<String, Value>> {
    if !value.is_object() {
        return None;
    }
    let rooms: Vec<Value> = match field(value, "rooms") {
        Some(Value::Array(items)) => items
            .iter()
            .take(64)
            .filter_map(|entry| match field(entry, "id") {
                Some(Value::String(id)) if !id.is_empty() => Some(json!({
                    "id": js_slice(id, 48),
                    "temperature": num(finite(field(entry, "temperature"), 60.0, 0.0, 100.0)),
                })),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let mut world = Map::new();
    world.insert("depth".into(), num(js_math_round(finite(field(value, "depth"), 0.0, 0.0, 9_999.0))));
    world.insert(
        "endlessExpansion".into(),
        num(js_math_round(finite(field(value, "endlessExpansion"), 0.0, 0.0, 999.0))),
    );
    world.insert("hunters".into(), Value::Array(sanitize_entities(field(value, "hunters"), false)));
    world.insert("mannequins".into(), Value::Array(sanitize_entities(field(value, "mannequins"), true)));
    world.insert("rooms".into(), Value::Array(rooms));
    Some(world)
}

fn has_own(map: &Map<String, Value>, key: &str) -> bool {
    map.contains_key(key)
}

fn apply_item_claim(items: &mut Map<String, Value>, claim: Option<&Value>) {
    apply_item_claim_within(items, claim, ITEM_TOTAL_LIMIT);
}

/// apply_item_claim with the total-records cap passed in (tests use a small one).
pub(super) fn apply_item_claim_within(items: &mut Map<String, Value>, claim: Option<&Value>, limit: usize) {
    let claim = match claim {
        Some(value @ Value::Object(_)) => value,
        _ => return,
    };
    let kind = match field(claim, "kind") {
        Some(Value::String(kind)) if kind == "relay" || kind == "battery" || kind == "prop" => kind.as_str(),
        _ => return,
    };
    let id = match field(claim, "id") {
        Some(Value::String(id)) => js_slice(id, 48),
        _ => String::new(),
    };
    if id.is_empty() {
        return;
    }
    let empty = Map::new();
    let changes = match field(claim, "changes") {
        Some(Value::Object(map)) => map,
        _ => &empty,
    };
    let key = format!("{kind}:{id}");
    if !items.contains_key(&key) && items.len() >= limit {
        return;
    }
    let mut record = match items.get(&key) {
        Some(Value::Object(existing)) => existing.clone(),
        _ => {
            let mut fresh = Map::new();
            fresh.insert("kind".into(), json!(kind));
            fresh.insert("id".into(), json!(id));
            fresh
        }
    };
    if changes.get("collected") == Some(&Value::Bool(true)) {
        record.insert("collected".into(), json!(true));
    }
    if kind == "relay" && has_own(changes, "sabotaged") {
        record.insert("sabotaged".into(), json!(truthy(changes.get("sabotaged"))));
    }
    if kind == "prop" {
        if let Some(Value::String(state)) = changes.get("interactionState") {
            record.insert("interactionState".into(), json!(js_slice(state, 24)));
        }
        if changes.get("fired") == Some(&Value::Bool(true)) {
            let seq = js_number(record.get("stimulusSeq"));
            let seq = if seq == 0.0 || seq.is_nan() { 0.0 } else { seq };
            record.insert("stimulusSeq".into(), num(seq + 1.0));
        }
        if has_own(changes, "cooldownUntil") {
            let previous = record_number_or_zero(&record, "cooldownUntil");
            let next = finite(changes.get("cooldownUntil"), 0.0, 0.0, 1e9);
            record.insert("cooldownUntil".into(), num(js_max(previous, next)));
        }
        if has_own(changes, "x") {
            let previous = record_number_or_zero(&record, "x");
            record.insert("x".into(), num(finite(changes.get("x"), previous, -1_000.0, 1_000.0)));
        }
        if has_own(changes, "y") {
            let previous = record_number_or_zero(&record, "y");
            record.insert("y".into(), num(finite(changes.get("y"), previous, -1_000.0, 1_000.0)));
        }
    }
    items.insert(key, Value::Object(record));
}

/// Total discovered cells a room keeps: DISCOVERY_ENDLESS_LIMIT for endless maps, DISCOVERY_TOTAL_LIMIT otherwise.
pub(super) fn discovery_cap(settings: &Map<String, Value>) -> usize {
    if settings.get("mapScale") == Some(&json!("endless")) {
        DISCOVERY_ENDLESS_LIMIT
    } else {
        DISCOVERY_TOTAL_LIMIT
    }
}

/// Adds up to DISCOVERY_LIMIT new cells from one update; cells past `max` total are ignored.
pub(super) fn claim_discovery_cells(
    discovered: &mut HashSet<String>,
    list: &mut Vec<String>,
    cells: &[Value],
    max: usize,
) {
    for cell in cells.iter().take(DISCOVERY_LIMIT) {
        if let Value::String(cell) = cell {
            if js_len(cell) <= 12 && discovered.len() < max && !discovered.contains(cell) {
                discovered.insert(cell.clone());
                list.push(cell.clone());
            }
        }
    }
}

/// `record.field || 0` for the numbers this module stores.
fn record_number_or_zero(record: &Map<String, Value>, key: &str) -> f64 {
    if truthy(record.get(key)) {
        js_number(record.get(key))
    } else {
        0.0
    }
}

/// `Math.max(a, b)` (NaN-propagating).
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

fn filename_png() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*\.[pP][nN][gG]$").unwrap())
}

fn filename_yaml() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*\.[yY][aA][mM][lL](#[A-Za-z0-9][A-Za-z0-9_-]*)?$").unwrap()
    })
}

/// `String(value || "").trim().slice(0, n)`.
fn trimmed_text(value: Option<&Value>, n: usize) -> String {
    js_slice(js_trim(&str_or_empty(value)), n)
}

pub struct RoomServiceOptions {
    pub now: Clock,
    pub player_ttl_ms: i64,
    pub room_ttl_ms: i64,
}

impl Default for RoomServiceOptions {
    fn default() -> Self {
        RoomServiceOptions { now: system_clock(), player_ttl_ms: 120_000, room_ttl_ms: 300_000 }
    }
}

pub struct RoomService {
    pub rooms: HashMap<String, Room>,
    pub(crate) now: Clock,
    pub(crate) player_ttl_ms: i64,
    pub(crate) room_ttl_ms: i64,
    /// How long a dropped peer's slot is held (`LASTROUTE_RECONNECT_RESERVE_MS`, 0 turns holding off).
    pub(crate) reserve_ms: i64,
    /// How long guests of a host-closed room keep hearing the reason (`LASTROUTE_ROOM_CLOSE_NOTICE_MS`).
    pub(crate) close_notice_ms: i64,
    /// Room code -> (reason, shown until), for rooms the host closed.
    pub(crate) closed: HashMap<String, (String, i64)>,
}

impl RoomService {
    pub fn new(opts: RoomServiceOptions) -> RoomService {
        RoomService {
            rooms: HashMap::new(),
            now: opts.now,
            player_ttl_ms: opts.player_ttl_ms,
            room_ttl_ms: opts.room_ttl_ms,
            reserve_ms: env_ms("LASTROUTE_RECONNECT_RESERVE_MS", DEFAULT_RECONNECT_RESERVE_MS),
            close_notice_ms: env_ms("LASTROUTE_ROOM_CLOSE_NOTICE_MS", DEFAULT_CLOSE_NOTICE_MS),
            closed: HashMap::new(),
        }
    }

    /// `roomService.rooms.size`.
    pub fn rooms_len(&self) -> usize {
        self.rooms.len()
    }

    pub(crate) fn now(&self) -> i64 {
        (self.now)()
    }

    /// `pruneRoom`: true when the room survives.
    pub(crate) fn prune_room(&mut self, code: &str, timestamp: i64) -> bool {
        let (player_ttl, room_ttl) = (self.player_ttl_ms, self.room_ttl_ms);
        let Some(room) = self.rooms.get_mut(code) else { return false };
        room.players.retain(|player| timestamp - player.last_seen <= player_ttl);
        room.held.retain(|held| held.until > timestamp);
        room.reconcile();
        if (room.players.is_empty() && room.held.is_empty()) || timestamp - room.last_active_at > room_ttl {
            self.rooms.remove(code);
            return false;
        }
        true
    }

    pub fn cleanup(&mut self) {
        let timestamp = self.now();
        let codes: Vec<String> = self.rooms.keys().cloned().collect();
        for code in codes {
            self.prune_room(&code, timestamp);
        }
    }

    /// `load(code)`: true when the room exists after pruning.
    pub(crate) fn load(&mut self, code: &str) -> bool {
        if !self.rooms.contains_key(code) {
            return false;
        }
        let timestamp = self.now();
        self.prune_room(code, timestamp)
    }

    /// `authenticate(body, { host })`: the room code and the requesting player's index.
    pub(crate) fn authenticate(&mut self, body: &Value, host: bool) -> Result<(String, usize), Reply> {
        let code = clean_room(field(body, "room"));
        let id = clean_player_id(field(body, "playerId"));
        let exists = self.load(&code);
        if !exists || id.is_empty() {
            let timestamp = self.now();
            self.closed.retain(|_, (_, until)| *until > timestamp);
            if let Some((reason, _)) = self.closed.get(&code) {
                return Err(error(410, "ROOM_CLOSED", reason));
            }
            return Err(error(404, "ROOM_NOT_FOUND", "Room not found"));
        }
        let room = &self.rooms[&code];
        let Some(index) = room.player_index(&id) else {
            if let Some(why) = room.removed.get(&id) {
                let text = if why == "BANNED" { "BANNED FROM THIS ROOM" } else { "KICKED FROM THIS ROOM" };
                return Err(error(403, why, text));
            }
            return Err(error(404, "PLAYER_NOT_JOINED", "Player not joined"));
        };
        let player = &room.players[index];
        if !safe_equal_values(field(body, "sessionToken"), &player.session_token) {
            return Err(error(403, "INVALID_SESSION", "Room session expired; leave and rejoin"));
        }
        if host && player.id != room.host_player_id {
            return Err(error(403, "HOST_ONLY", "Only the room host can do that"));
        }
        Ok((code, index))
    }

    #[allow(dead_code)] // nothing calls it yet
    pub fn join_room(&mut self, body: &Value) -> Reply {
        self.join_with(body, &super::admin_rooms::JoinContext::plain())
    }

    /// `join_room` with the caller's verified standing (admin flag, saved admin-room settings, codes taken elsewhere).
    pub fn join_with(&mut self, body: &Value, ctx: &super::admin_rooms::JoinContext) -> Reply {
        let mut code = clean_room(field(body, "room"));
        let id = clean_player_id(field(body, "playerId"));
        let name = clean_name(field(body, "name"));
        let join_only = truthy(field(body, "joinOnly"));
        let typed_nothing = str_or_empty(field(body, "room")).trim().is_empty();
        let mut admin_create = false;
        if typed_nothing && ctx.admin && !id.is_empty() {
            // An admin who types no code gets a fresh room under a free code.
            code = self.free_code(ctx);
            admin_create = true;
        }
        if code.len() < 5 || id.is_empty() {
            return error(400, "INVALID_ROOM", "Room codes must be 5-8 characters");
        }
        if name.is_empty() {
            return error(400, "INVALID_NAME", "Enter a valid player name");
        }
        let timestamp = self.now();
        if !self.load(&code) {
            self.cleanup();
            if join_only && !ctx.admin {
                return error(404, "ROOM_NOT_FOUND", "Room not found");
            }
            if ctx.admin && join_only && !typed_nothing {
                if let Err(reply) = super::admin_rooms::validate_admin_code(&str_or_empty(field(body, "room")), &|c| ctx.taken(c)) {
                    return reply;
                }
                admin_create = true;
            }
            if self.rooms.len() >= MAX_ROOMS {
                return error(503, "SERVER_BUSY", "Too many active rooms");
            }
            let mut fresh = Room::new(&code, timestamp);
            if admin_create {
                super::admin_rooms::make_admin_room(&mut fresh, ctx.saved.as_ref());
            }
            self.closed.remove(&code);
            self.rooms.insert(code.clone(), fresh);
        }
        let room = self.rooms.get_mut(&code).expect("room exists");
        if room.bans.contains(&id) || room.bans.contains(&format!("name:{name}")) {
            return error(403, "BANNED", "BANNED FROM THIS ROOM");
        }
        // A dropped peer comes back to its held slot with the same peer (session) token.
        if let Some(position) = room.held.iter().position(|held| held.player.id == id) {
            if !safe_equal_values(field(body, "sessionToken"), &room.held[position].player.session_token) {
                return error(403, "INVALID_SESSION", "Room session expired; leave and rejoin");
            }
            let mut held = room.held.remove(position);
            held.player.last_seen = timestamp;
            room.players.push(held.player);
        }
        let existing_index = room.player_index(&id);
        if let Some(index) = existing_index {
            if !safe_equal_values(field(body, "sessionToken"), &room.players[index].session_token) {
                return error(403, "INVALID_SESSION", "Room session expired; leave and rejoin");
            }
        }
        let lower = name.to_lowercase();
        if room.players.iter().chain(room.held.iter().map(|held| &held.player)).any(|player| player.id != id && player.name.to_lowercase() == lower) {
            return error(409, "NAME_TAKEN", "That name is already in use in this room");
        }
        if existing_index.is_none() && room.players.len() + room.held.len() >= room.capacity {
            return error(409, "ROOM_FULL", &format!("Room capacity is {} players", room.capacity));
        }
        let player = match existing_index {
            Some(index) => {
                let mut player = room.players[index].clone();
                player.name = name;
                player.last_seen = timestamp;
                if !ctx.account.is_empty() {
                    player.account = ctx.account.clone();
                }
                player
            }
            None => {
                let occupied: HashSet<i64> =
                    room.players.iter().chain(room.held.iter().map(|held| &held.player)).map(|player| player.spawn_index).collect();
                let spawn_index = (0..MAX_PLAYERS as i64).find(|index| !occupied.contains(index)).unwrap_or(-1);
                Player {
                    id: id.clone(),
                    session_token: hex(&random_bytes(24)),
                    name,
                    account: ctx.account.clone(),
                    spawn_index,
                    depth: 0.0,
                    x: 0.0,
                    y: 0.0,
                    facing_x: 1.0,
                    facing_y: 0.0,
                    mode: "idle".into(),
                    hidden: false,
                    flashlight_on: true,
                    radial_active: false,
                    fan_on: false,
                    heater_on: false,
                    frame: 0.0,
                    persona: "none".into(),
                    appearance_id: "player".into(),
                    tagged: false,
                    noise: 0.0,
                    in_safe_zone: false,
                    lamp: 0.0,
                    travel_speed: 0.0,
                    crouching: false,
                    vx: 0.0,
                    vy: 0.0,
                    last_seen: timestamp,
                }
            }
        };
        match existing_index {
            Some(index) => room.players[index] = player,
            None => room.players.push(player),
        }
        room.reconcile();
        room.last_active_at = timestamp;
        (200, room.payload(&id, timestamp, None))
    }

    pub fn update_room(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.authenticate(body, false) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        if let Some(name) = field(body, "name") {
            if clean_name(Some(name)).to_lowercase() != room.players[index].name.to_lowercase() {
                return error(409, "NAME_LOCKED", "Leave and rejoin to change your name");
            }
        }
        let empty = Value::Object(Map::new());
        let state = match field(body, "state") {
            Some(value) if truthy(Some(value)) => value,
            _ => &empty,
        };
        let round_id = room.round.round_id;
        {
            let current = &mut room.players[index];
            current.depth = js_math_round(finite(field(state, "depth"), current.depth, 0.0, 9_999.0));
            current.x = finite(field(state, "x"), current.x, -1_000.0, 1_000.0);
            current.y = finite(field(state, "y"), current.y, -1_000.0, 1_000.0);
            current.facing_x = finite(field(state, "facingX"), current.facing_x, -1.0, 1.0);
            current.facing_y = finite(field(state, "facingY"), current.facing_y, -1.0, 1.0);
            current.mode = match field(state, "mode") {
                Some(Value::String(mode)) if ["idle", "walk", "sprint", "crouch", "lobby"].contains(&mode.as_str()) => {
                    mode.clone()
                }
                _ => "idle".into(),
            };
            current.hidden = truthy(field(state, "hidden"));
            if let Some(Value::Bool(on)) = field(state, "flashlightOn") {
                current.flashlight_on = *on;
            }
            current.radial_active = truthy(field(state, "radialActive"));
            current.fan_on = truthy(field(state, "fanOn"));
            current.heater_on = truthy(field(state, "heaterOn"));
            current.frame = js_math_round(finite(field(state, "frame"), current.frame, 0.0, 3.0));
            if let Some(Value::String(persona)) = canonical_hunter_id_value(field(state, "persona")) {
                if PERSONAS.contains(&persona.as_str()) {
                    current.persona = persona;
                }
            }
            current.appearance_id = clean_appearance(field(state, "appearanceId"), &current.appearance_id);
            current.tagged = if js_number(field(state, "roundId")) == round_id as f64 && round_id > 0 {
                current.tagged || truthy(field(state, "tagged"))
            } else {
                false
            };
            current.noise = finite(field(state, "noise"), 0.0, 0.0, 100.0);
            current.in_safe_zone = truthy(field(state, "inSafeZone"));
            current.lamp = finite(field(state, "lamp"), current.lamp, 0.0, 100.0);
            current.travel_speed = finite(field(state, "travelSpeed"), 0.0, 0.0, 100.0);
            current.crouching = truthy(field(state, "crouching"));
            current.vx = finite(field(state, "vx"), current.vx, -100.0, 100.0);
            current.vy = finite(field(state, "vy"), current.vy, -100.0, 100.0);
        }
        let timestamp = timestamp_source();
        room.players[index].last_seen = timestamp;
        room.last_active_at = timestamp;
        let current_id = room.players[index].id.clone();
        // Only the host advances the depth (a guest's claim is ignored: it would wipe the room's items, the host's
        // hunter snapshot and discovery), and by at most MAX_DEPTH_CLAIM_STEP floors once the room has a depth.
        let claim_depth = js_math_round(finite(field(body, "claimDepth"), 0.0, 0.0, 9_999.0)) as i64;
        if current_id == room.host_player_id && depth_claim_allowed(room.active_depth, claim_depth) {
            room.active_depth = claim_depth;
            room.items = Map::new();
            room.world = empty_world();
            room.discovered = HashSet::new();
            room.discovered_list = Vec::new();
            room.discovered_epoch += 1;
        }
        if current_id == room.host_player_id && truthy(field(body, "world")) {
            if let Some(mut world) = sanitize_world(field(body, "world").unwrap()) {
                if world.get("depth").and_then(Value::as_f64) == Some(room.active_depth as f64) {
                    world.insert("hostId".into(), json!(current_id));
                    world.insert("at".into(), json!(timestamp));
                    room.world = Value::Object(world);
                }
            }
        }
        if let Some(Value::Array(claims)) = field(body, "itemClaims") {
            for claim in claims.iter().take(ITEM_LIMIT) {
                let depth = js_math_round(finite(field(claim, "depth"), -1.0, 0.0, 9_999.0));
                if depth == room.active_depth as f64 {
                    apply_item_claim(&mut room.items, Some(claim));
                }
            }
        }
        let max_discovery = discovery_cap(&room.settings);
        if let Some(Value::Array(cells)) = field(body, "discoveryClaims") {
            claim_discovery_cells(&mut room.discovered, &mut room.discovered_list, cells, max_discovery);
        }
        if current_id == room.host_player_id && room.round.phase == "seeking" {
            let seeker = room.round.seeker_spawn_index;
            let hiders: Vec<&Player> = room.players.iter().filter(|player| player.spawn_index != seeker).collect();
            if !hiders.is_empty() && hiders.iter().all(|player| player.tagged) {
                room.round.phase = "complete".into();
            }
        }
        let since = js_math_round(finite(field(body, "discoverySince"), 0.0, 0.0, 9_999_999.0)) as i64;
        let epoch = js_math_round(finite(field(body, "discoveryEpoch"), -1.0, -1.0, 9_999_999.0)) as i64;
        (200, room.payload(&current_id, timestamp, Some((since, epoch))))
    }

    pub fn update_room_settings(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.authenticate(body, true) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        let Some(settings) = clean_settings(field(body, "settings"), &room.settings) else {
            return error(400, "INVALID_SETTINGS", "Invalid room settings");
        };
        room.settings = settings;
        room.round = Round::empty();
        let timestamp = timestamp_source();
        room.last_active_at = timestamp;
        room.players[index].last_seen = timestamp;
        let id = room.players[index].id.clone();
        (200, room.payload(&id, timestamp, None))
    }

    pub fn start_room_round(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.authenticate(body, true) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        if room.settings.get("mode") != Some(&json!("hideAndSeek")) {
            return error(409, "WRONG_MODE", "Select Hide & Seek before starting");
        }
        let npc = |key: &str| if truthy(room.settings.get(key)) { js_number(room.settings.get(key)) } else { 0.0 };
        let npc_count = npc("hiderNpcs") + npc("seekerNpcs");
        if room.players.len() as f64 + npc_count < 2.0 {
            return error(409, "NOT_ENOUGH_PLAYERS", "Hide & Seek needs at least two players or NPCs");
        }
        let timestamp = timestamp_source();
        room.round_counter += 1;
        let solo_hider = room.players.len() == 1 && str_or_empty(field(body, "role")).to_lowercase() == "hider";
        let bytes = random_bytes(4);
        let pick = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize % room.players.len();
        let random_player = &room.players[pick];
        room.round = Round {
            round_id: room.round_counter,
            phase: "hiding".into(),
            seeker_id: if solo_hider { "npc:seeker:0".into() } else { random_player.id.clone() },
            seeker_spawn_index: if solo_hider { -1 } else { random_player.spawn_index },
            started_at: timestamp,
            hide_ends_at: timestamp + HIDE_PHASE_MS,
        };
        for player in &mut room.players {
            player.tagged = false;
        }
        room.players[index].last_seen = timestamp;
        room.last_active_at = timestamp;
        let id = room.players[index].id.clone();
        (200, room.payload(&id, timestamp, None))
    }

    fn launch(&mut self, body: &Value, mode: &str) -> Reply {
        let (code, index) = match self.authenticate(body, true) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        if room.settings.get("mode") != Some(&json!(mode)) {
            return error(409, "WRONG_MODE", &format!("Select {mode} before starting"));
        }
        room.reset_runtime();
        if mode == "lastRoute" {
            room.launch_id += 1;
        }
        let timestamp = timestamp_source();
        room.players[index].last_seen = timestamp;
        room.last_active_at = timestamp;
        let id = room.players[index].id.clone();
        (200, room.payload(&id, timestamp, None))
    }

    /// `startDefaultRoute(body)` = `launch(body, "lastRoute")`.
    pub fn start_default_route(&mut self, body: &Value) -> Reply {
        self.launch(body, "lastRoute")
    }

    pub fn abandon_route(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.authenticate(body, true) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        room.abandon_id += 1;
        room.reset_runtime();
        let timestamp = timestamp_source();
        room.players[index].last_seen = timestamp;
        room.last_active_at = timestamp;
        let id = room.players[index].id.clone();
        (200, room.payload(&id, timestamp, None))
    }

    pub fn set_custom_map(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.authenticate(body, true) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        let filename = trimmed_text(field(body, "filename"), 128);
        if !filename.is_empty() && !filename_png().is_match(&filename) {
            return error(400, "INVALID_FILENAME", "Custom map filename must be a plain .png name");
        }
        room.custom_map_file = filename;
        room.custom_map_id += 1;
        let timestamp = timestamp_source();
        room.players[index].last_seen = timestamp;
        room.last_active_at = timestamp;
        let id = room.players[index].id.clone();
        (200, room.payload(&id, timestamp, None))
    }

    pub fn start_dialog(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.authenticate(body, true) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        let file = trimmed_text(field(body, "file"), 128);
        if !filename_yaml().is_match(&file) {
            return error(400, "INVALID_FILENAME", "Dialog filename must be a plain .yaml name");
        }
        room.dialog = Dialog { dialog_id: room.dialog.dialog_id + 1, file, node_id: String::new(), step_id: 0, ended: false };
        let timestamp = timestamp_source();
        room.players[index].last_seen = timestamp;
        room.last_active_at = timestamp;
        let id = room.players[index].id.clone();
        (200, room.payload(&id, timestamp, None))
    }

    pub fn advance_dialog(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.authenticate(body, true) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        if js_number(field(body, "dialogId")) != room.dialog.dialog_id as f64 || room.dialog.dialog_id < 1 {
            return error(409, "STALE_DIALOG", "That dialog is no longer active");
        }
        room.dialog.node_id = trimmed_text(field(body, "nodeId"), 96);
        room.dialog.ended = truthy(field(body, "ended"));
        room.dialog.step_id += 1;
        let timestamp = timestamp_source();
        room.players[index].last_seen = timestamp;
        room.last_active_at = timestamp;
        let id = room.players[index].id.clone();
        (200, room.payload(&id, timestamp, None))
    }

    pub fn leave_room(&mut self, body: &Value) -> Reply {
        let code = clean_room(field(body, "room"));
        let id = clean_player_id(field(body, "playerId"));
        let exists = self.load(&code);
        let index = if exists { self.rooms[&code].player_index(&id) } else { None };
        let Some(index) = index else { return (200, json!({ "ok": true })) };
        let timestamp_source = self.now.clone();
        let room = self.rooms.get_mut(&code).expect("room exists");
        if !safe_equal_values(field(body, "sessionToken"), &room.players[index].session_token) {
            return error(403, "INVALID_SESSION", "Room session expired; leave and rejoin");
        }
        let now = timestamp_source();
        let dropped = truthy(field(body, "dropped"));
        if dropped && self.reserve_ms > 0 {
            // The connection broke: keep the slot for the reserved window, claimable with the same session token.
            let player = room.players.remove(index);
            room.held.push(Held { player, until: now + self.reserve_ms });
            room.last_active_at = now;
            room.reconcile();
            return (200, json!({ "ok": true, "held": true, "reservedMs": self.reserve_ms }));
        }
        let was_host = room.players[index].id == room.host_player_id;
        room.players.remove(index);
        if was_host && (!room.players.is_empty() || !room.held.is_empty()) {
            // Host migration stays open: when the host leaves for good the room closes with a reason.
            self.rooms.remove(&code);
            self.closed.insert(code, (HOST_LEFT_REASON.to_string(), now + self.close_notice_ms));
            return (200, json!({ "ok": true, "closed": true, "reason": HOST_LEFT_REASON }));
        }
        if room.players.is_empty() && room.held.is_empty() {
            self.rooms.remove(&code);
        } else {
            room.last_active_at = now;
            room.reconcile();
        }
        (200, json!({ "ok": true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{manual_clock, stringify, ManualClock};

    struct Clocked {
        service: RoomService,
        clock: ManualClock,
    }

    impl Clocked {
        fn at(&self) -> i64 {
            self.clock.get()
        }
        fn advance(&self, ms: i64) {
            self.clock.advance(ms)
        }
    }

    fn service_at(now: i64) -> Clocked {
        service_with(now, 120_000, 300_000)
    }

    fn service_with(now: i64, player_ttl_ms: i64, room_ttl_ms: i64) -> Clocked {
        let (clock, handle) = manual_clock(now);
        Clocked { service: RoomService::new(RoomServiceOptions { now: clock, player_ttl_ms, room_ttl_ms }), clock: handle }
    }

    fn join(service: &mut RoomService, body: Value) -> Value {
        let (status, payload) = service.join_room(&body);
        assert_eq!(status, 200, "{}", stringify(&payload));
        payload
    }

    /// `{ ...base, ...extra }`.
    fn with(base: &Value, extra: Value) -> Value {
        let mut map = base.as_object().unwrap().clone();
        for (key, value) in extra.as_object().unwrap() {
            map.insert(key.clone(), value.clone());
        }
        Value::Object(map)
    }

    fn token(payload: &Value) -> String {
        payload["sessionToken"].as_str().unwrap().to_string()
    }

    #[test]
    fn default_room_settings_are_the_standard_last_route_lobby() {
        assert_eq!(
            Value::Object(default_room_settings()),
            json!({ "mapScale": "standard", "difficulty": "standard", "mode": "lastRoute", "fakeRunners": false })
        );
        assert_eq!(
            stringify(&Value::Object(default_room_settings())),
            r#"{"mapScale":"standard","difficulty":"standard","mode":"lastRoute","fakeRunners":false}"#
        );
    }

    #[test]
    fn join_creates_a_host_room_rejects_bad_input_and_rejoins_only_with_the_session() {
        let mut c = service_at(1_000_000);
        let s = &mut c.service;
        assert_eq!(s.join_room(&json!({ "room": "abcd", "playerId": "p1", "name": "Ann" })).1["code"], "INVALID_ROOM");
        assert_eq!(s.join_room(&json!({ "room": "HELLO", "playerId": "", "name": "Ann" })).1["code"], "INVALID_ROOM");
        assert_eq!(s.join_room(&json!({ "room": "HELLO", "playerId": "p1", "name": "!!!" })).1["code"], "INVALID_NAME");

        let host = join(s, json!({ "room": "he-llo!", "playerId": "p 1", "name": " ann " }));
        assert_eq!(host["room"], "HELLO");
        assert_eq!(host["isHost"], true);
        assert_eq!(host["spawnIndex"], 0);
        assert_eq!(host["roomSize"], 1);
        assert_eq!(host["maxPlayers"], 16);
        assert!(Regex::new(r"^ROOM-HELLO-[A-F0-9]{20}$").unwrap().is_match(host["worldSeed"].as_str().unwrap()));
        assert_eq!(host["player"]["role"], "runner");
        assert_eq!(host["player"]["name"], "ANN");
        assert_eq!(host["settings"], Value::Object(default_room_settings()));

        let guest = join(s, json!({ "room": "hello", "playerId": "p2", "name": "Bob" }));
        assert_eq!(guest["isHost"], false);
        assert_eq!(guest["spawnIndex"], 1);
        assert_eq!(guest["roomSize"], 2);
        assert_eq!(guest["players"].as_array().unwrap().len(), 1);
        assert_eq!(guest["players"][0]["name"], "ANN");
        assert_eq!(guest["players"][0]["isHost"], true);
        assert!(!stringify(&guest["players"]).contains(&token(&host)));

        assert_eq!(s.join_room(&json!({ "room": "HELLO", "playerId": "p3", "name": "ann" })).1["code"], "NAME_TAKEN");
        let rejected = s.join_room(&json!({ "room": "HELLO", "playerId": "p1", "name": "Ann", "sessionToken": "nope" }));
        assert_eq!(rejected.0, 403);
        assert_eq!(rejected.1["code"], "INVALID_SESSION");
        let renamed = join(s, json!({ "room": "HELLO", "playerId": "p1", "name": "Ada", "sessionToken": token(&host) }));
        assert_eq!(renamed["player"]["name"], "ADA");
        assert_eq!(renamed["sessionToken"], host["sessionToken"]);
        assert_eq!(renamed["spawnIndex"], 0);
    }

    #[test]
    fn nine_character_codes_are_clipped_to_eight_and_land_in_the_same_room() {
        let mut c = service_at(1_000_000);
        let first = join(&mut c.service, json!({ "room": "ABCDEFGH", "playerId": "p1", "name": "Ann" }));
        let second = join(&mut c.service, json!({ "room": "ABCDEFGHI", "playerId": "p2", "name": "Bob" }));
        assert_eq!(second["room"], "ABCDEFGH");
        assert_eq!(second["roomSize"], 2);
        assert_eq!(second["worldSeed"], first["worldSeed"]);
    }

    #[test]
    fn updates_clamp_state_share_host_world_and_items_and_page_discovery() {
        let mut c = service_at(1_000_000);
        let s = &mut c.service;
        let host = join(s, json!({ "room": "HELLO", "playerId": "host", "name": "Ann" }));
        let guest = join(s, json!({ "room": "HELLO", "playerId": "guest", "name": "Bob" }));
        let auth = json!({ "room": "HELLO", "playerId": "host", "sessionToken": token(&host) });

        assert_eq!(
            s.update_room(&json!({ "room": "NOPE", "playerId": "host", "sessionToken": token(&host) })).1["code"],
            "ROOM_NOT_FOUND"
        );
        assert_eq!(s.update_room(&with(&auth, json!({ "playerId": "missing" }))).1["code"], "PLAYER_NOT_JOINED");
        assert_eq!(s.update_room(&with(&auth, json!({ "sessionToken": "nope" }))).1["code"], "INVALID_SESSION");
        assert_eq!(s.update_room(&with(&auth, json!({ "name": "Eve" }))).1["code"], "NAME_LOCKED");

        let claims: Vec<String> = (0..520).map(|index| format!("C{index}")).collect();
        let (status, updated) = s.update_room(&with(
            &auth,
            json!({
                "state": {
                    "depth": -4, "x": 5000, "y": "nope", "facingX": 4, "facingY": -0.2, "mode": "fly", "hidden": true,
                    "flashlightOn": false, "frame": 9, "persona": "eirys", "appearanceId": "My Skin!", "noise": 400,
                    "travelSpeed": 3, "vx": 1, "vy": -2,
                },
                "world": {
                    "depth": 0,
                    "hunters": [{ "id": "h1", "x": 9999, "y": 1, "facingX": 8, "state": "chase", "alerted": 1, "active": false }, { "id": "" }],
                    "mannequins": [{ "id": "m1", "name": "N".repeat(80), "x": 2, "y": 3 }],
                    "rooms": [{ "id": "kitchen", "temperature": 500 }, { "temperature": 10 }],
                },
                "itemClaims": [
                    { "kind": "relay", "id": "r1", "depth": 0, "changes": { "collected": true, "sabotaged": true } },
                    { "kind": "battery", "id": "b1", "depth": 0, "changes": { "collected": true } },
                    { "kind": "prop", "id": "p1", "depth": 0, "changes": { "interactionState": "open", "fired": true, "cooldownUntil": 50, "x": 3, "y": 4 } },
                    { "kind": "nope", "id": "x", "depth": 0, "changes": { "collected": true } },
                    { "kind": "relay", "id": "late", "depth": 3, "changes": { "collected": true } },
                ],
                "discoveryClaims": claims,
            }),
        ));
        assert_eq!(status, 200);
        assert_eq!(updated["player"]["depth"], 0);
        assert_eq!(updated["player"]["x"], 1000);
        assert_eq!(updated["player"]["y"], 0);
        assert_eq!(updated["player"]["facingX"], 1);
        assert_eq!(updated["player"]["mode"], "idle");
        assert_eq!(updated["player"]["flashlightOn"], false);
        assert_eq!(updated["player"]["frame"], 3);
        assert_eq!(updated["player"]["persona"], "elise");
        assert_eq!(updated["player"]["appearanceId"], "MySkin");
        assert_eq!(updated["player"]["noise"], 100);
        assert_eq!(updated["world"]["hunters"][0]["x"], 1000);
        assert_eq!(updated["world"]["hunters"][0]["facingX"], 1);
        assert_eq!(updated["world"]["hunters"][0]["alerted"], true);
        assert_eq!(updated["world"]["hunters"][0]["active"], false);
        assert_eq!(updated["world"]["mannequins"][0]["name"].as_str().unwrap().len(), 48);
        assert_eq!(updated["world"]["rooms"][0]["temperature"], 100);
        assert_eq!(updated["world"]["rooms"].as_array().unwrap().len(), 1);
        assert_eq!(updated["items"]["relay:r1"]["sabotaged"], true);
        assert_eq!(updated["items"]["battery:b1"]["collected"], true);
        assert_eq!(updated["items"]["prop:p1"]["stimulusSeq"], 1);
        assert_eq!(updated["items"]["prop:p1"]["cooldownUntil"], 50);
        assert!(updated["items"].get("relay:late").is_none());
        assert_eq!(updated["discoveredTotal"], 512);
        assert_eq!(updated["discoveredReset"], true);

        let guest_world = s.update_room(&json!({
            "room": "HELLO", "playerId": "guest", "sessionToken": token(&guest),
            "world": { "depth": 0, "hunters": [{ "id": "guest-hunter", "x": 1, "y": 1 }] },
        }));
        assert_eq!(guest_world.1["world"]["hunters"][0]["id"], "h1");

        let paged = s
            .update_room(&with(
                &auth,
                json!({
                    "discoveryEpoch": updated["discoveredEpoch"],
                    "discoverySince": updated["discoveredTotal"],
                    "discoveryClaims": ["1234567890123", "C0", "NEXT", 4],
                }),
            ))
            .1;
        assert_eq!(paged["discovered"], json!(["NEXT"]));
        assert_eq!(paged["discoveredReset"], false);
        assert_eq!(paged["discoveredTotal"], 513);

        let reset = s.update_room(&with(&auth, json!({ "discoveryEpoch": 99, "claimDepth": 2 }))).1;
        assert_eq!(reset["activeDepth"], 2);
        assert_eq!(reset["items"], json!({}));
        assert_eq!(reset["world"]["hunters"], json!([]));
        assert_eq!(reset["discoveredReset"], true);
        assert_eq!(reset["discoveredTotal"], 0);
    }

    #[test]
    fn only_the_host_changes_settings_and_an_invalid_patch_leaves_them_alone() {
        let mut c = service_at(1_000_000);
        let s = &mut c.service;
        let host = join(s, json!({ "room": "HELLO", "playerId": "host", "name": "Ann" }));
        let guest = join(s, json!({ "room": "HELLO", "playerId": "guest", "name": "Bob" }));
        let auth = json!({ "room": "HELLO", "playerId": "host", "sessionToken": token(&host) });
        assert_eq!(
            s.update_room_settings(&json!({ "room": "HELLO", "playerId": "guest", "sessionToken": token(&guest), "settings": { "difficulty": "nightmare" } })).1["code"],
            "HOST_ONLY"
        );
        assert_eq!(s.update_room_settings(&with(&auth, json!({ "settings": [] }))).1["code"], "INVALID_SETTINGS");
        assert_eq!(s.update_room_settings(&with(&auth, json!({ "settings": { "nope": true } }))).1["code"], "INVALID_SETTINGS");
        let applied = s.update_room_settings(&with(&auth, json!({ "settings": { "difficulty": "nightmare", "fakeRunners": true } })));
        assert_eq!(applied.0, 200);
        assert_eq!(applied.1["settings"]["difficulty"], "nightmare");
        assert_eq!(applied.1["settings"]["fakeRunners"], true);
        assert_eq!(applied.1["settings"]["mode"], "lastRoute");
        assert_eq!(s.update_room_settings(&with(&auth, json!({ "settings": { "difficulty": "easy" } }))).0, 400);
        assert_eq!(
            s.join_room(&json!({ "room": "HELLO", "playerId": "host", "name": "Ann", "sessionToken": token(&host) })).1["settings"]["difficulty"],
            "nightmare"
        );
    }

    #[test]
    fn hide_and_seek_starts_in_hiding_holds_the_seeker_until_the_timer_and_tagging_does_not_complete_it() {
        let mut c = service_at(5_000);
        let host = join(&mut c.service, json!({ "room": "HIDE1", "playerId": "host", "name": "Ann" }));
        let host_auth = json!({ "room": "HIDE1", "playerId": "host", "sessionToken": token(&host) });
        assert_eq!(c.service.start_room_round(&host_auth).1["code"], "WRONG_MODE");
        assert_eq!(c.service.update_room_settings(&with(&host_auth, json!({ "settings": { "mode": "hideAndSeek" } }))).0, 200);
        assert_eq!(c.service.start_room_round(&host_auth).1["code"], "NOT_ENOUGH_PLAYERS");

        let guest = join(&mut c.service, json!({ "room": "HIDE1", "playerId": "guest", "name": "Bob" }));
        let guest_auth = json!({ "room": "HIDE1", "playerId": "guest", "sessionToken": token(&guest) });
        assert_eq!(c.service.start_room_round(&guest_auth).1["code"], "HOST_ONLY");
        let started = c.service.start_room_round(&host_auth);
        assert_eq!(started.0, 200);
        assert_eq!(started.1["round"]["phase"], "hiding");
        assert_eq!(started.1["round"]["roundId"], 1);
        assert_eq!(started.1["round"]["hideEndsAt"], c.at() + 30_000);
        assert!(["host", "guest"].contains(&started.1["round"]["seekerId"].as_str().unwrap()));
        let seeker_is_host = started.1["player"]["role"] == "seeker";
        let seeker_auth = if seeker_is_host { host_auth.clone() } else { guest_auth.clone() };
        let hider_auth = if seeker_is_host { guest_auth.clone() } else { host_auth.clone() };
        assert_eq!(c.service.update_room(&with(&seeker_auth, json!({ "state": { "roundId": 1 } }))).1["player"]["released"], false);
        assert_eq!(
            c.service.update_room(&with(&hider_auth, json!({ "state": { "roundId": 1, "tagged": true } }))).1["player"]["released"],
            true
        );
        c.advance(29_999);
        assert_eq!(c.service.update_room(&with(&seeker_auth, json!({ "state": { "roundId": 1 } }))).1["player"]["released"], false);
        c.advance(1);
        let after_timer = c.service.update_room(&with(&seeker_auth, json!({ "state": { "roundId": 1, "tagged": true } }))).1;
        assert_eq!(after_timer["player"]["released"], true);
        assert_eq!(after_timer["round"]["phase"], "hiding");
        let finished = c.service.update_room(&with(&host_auth, json!({ "state": { "roundId": 1, "tagged": true } }))).1;
        assert_eq!(finished["round"]["phase"], "hiding");

        let again = c.service.start_room_round(&host_auth).1;
        assert_eq!(again["round"]["roundId"], 2);
        assert_eq!(again["player"]["tagged"], false);
        assert_eq!(again["players"][0]["tagged"], false);
    }

    #[test]
    fn a_solo_hider_against_an_npc_seeker_is_enough_to_start() {
        let mut c = service_at(1_000_000);
        let s = &mut c.service;
        let host = join(s, json!({ "room": "SOLO1", "playerId": "host", "name": "Ann" }));
        let auth = json!({ "room": "SOLO1", "playerId": "host", "sessionToken": token(&host) });
        s.update_room_settings(&with(&auth, json!({ "settings": { "mode": "hideAndSeek", "seekerNpcs": 1 } })));
        let started = s.start_room_round(&with(&auth, json!({ "role": "hider" })));
        assert_eq!(started.0, 200, "{}", stringify(&started.1));
        assert_eq!(started.1["round"]["seekerId"], "npc:seeker:0");
        assert_eq!(started.1["round"]["seekerSpawnIndex"], -1);
        assert_eq!(started.1["player"]["role"], "hider");
    }

    #[test]
    fn launch_abandon_custom_map_and_dialog_follow_the_host() {
        let mut c = service_at(1_000_000);
        let s = &mut c.service;
        let host = join(s, json!({ "room": "OFFIC1", "playerId": "host", "name": "Ann" }));
        let guest = join(s, json!({ "room": "OFFIC1", "playerId": "guest", "name": "Bob" }));
        let auth = json!({ "room": "OFFIC1", "playerId": "host", "sessionToken": token(&host) });
        s.update_room(&with(
            &auth,
            json!({ "itemClaims": [{ "kind": "battery", "id": "b1", "depth": 0, "changes": { "collected": true } }] }),
        ));
        // The JS test also asserts `service.startOfficeSolo === undefined` (Office Mode launch was removed, task 018):
        // RoomService has no such method, which the compiler guarantees.
        let launched = s.start_default_route(&auth);
        assert_eq!(launched.0, 200);
        assert_eq!(launched.1["launchId"], 1);
        assert_eq!(launched.1["items"], json!({}));
        assert_eq!(s.start_default_route(&auth).1["launchId"], 2);

        // The removed Office Mode no longer locks the room or drops guests: an old client's
        // "officeSurvivor" falls back to Last Route and everyone stays in.
        s.update_room_settings(&with(&auth, json!({ "settings": { "mode": "hideAndSeek" } })));
        let legacy = s.update_room_settings(&with(&auth, json!({ "settings": { "mode": "officeSurvivor" } })));
        assert_eq!(legacy.0, 200);
        assert_eq!(legacy.1["settings"]["mode"], "lastRoute");
        assert_eq!(s.start_default_route(&auth).1["launchId"], 3);
        assert_eq!(s.update_room(&json!({ "room": "OFFIC1", "playerId": "guest", "sessionToken": token(&guest) })).0, 200);
        join(s, json!({ "room": "OFFIC1", "playerId": "late", "name": "Cam" }));
        let rejoined = join(s, json!({ "room": "OFFIC1", "playerId": "host", "name": "Ann", "sessionToken": token(&host) }));
        assert_eq!(rejoined["sessionToken"], host["sessionToken"]);

        assert_eq!(s.set_custom_map(&with(&auth, json!({ "filename": "../x.png" }))).1["code"], "INVALID_FILENAME");
        let mapped = s.set_custom_map(&with(&auth, json!({ "filename": "Floor.PNG" })));
        assert_eq!(mapped.0, 200);
        assert_eq!(mapped.1["customMapFile"], "Floor.PNG");
        assert_eq!(mapped.1["customMapId"], 1);
        assert_eq!(s.start_dialog(&with(&auth, json!({ "file": "../x.yaml" }))).1["code"], "INVALID_FILENAME");
        let late_token = s.rooms["OFFIC1"].players.iter().find(|player| player.id == "late").unwrap().session_token.clone();
        assert_eq!(
            s.start_dialog(&json!({ "room": "OFFIC1", "playerId": "late", "sessionToken": late_token, "file": "intro.yaml" })).1["code"],
            "HOST_ONLY"
        );
        let dialog = s.start_dialog(&with(&auth, json!({ "file": "intro.yaml#start" })));
        assert_eq!(dialog.1["dialog"]["file"], "intro.yaml#start");
        assert_eq!(dialog.1["dialog"]["dialogId"], 1);
        assert_eq!(
            s.advance_dialog(&with(&auth, json!({ "dialogId": 9, "nodeId": "a", "ended": true }))).1["code"],
            "STALE_DIALOG"
        );
        let step = s.advance_dialog(&with(&auth, json!({ "dialogId": 1, "nodeId": " line ", "ended": true })));
        assert_eq!(step.1["dialog"]["nodeId"], "line");
        assert_eq!(step.1["dialog"]["ended"], true);
        assert_eq!(step.1["dialog"]["stepId"], 1);

        let abandoned = s.abandon_route(&auth);
        assert_eq!(abandoned.0, 200);
        assert_eq!(abandoned.1["abandonId"], 1);
        assert_eq!(abandoned.1["customMapFile"], "");
        assert_eq!(abandoned.1["customMapId"], 1);
        assert_eq!(abandoned.1["dialog"]["dialogId"], 0);
        assert_eq!(abandoned.1["items"], json!({}));
        join(s, json!({ "room": "OFFIC1", "playerId": "after", "name": "Dee" }));
    }

    #[test]
    fn leaving_hands_the_room_to_the_next_player_and_the_last_one_out_closes_it() {
        let mut c = service_at(1_000_000);
        let s = &mut c.service;
        assert_eq!(s.leave_room(&json!({ "room": "NONE", "playerId": "p", "sessionToken": "x" })).1["ok"], true);
        let host = join(s, json!({ "room": "LEAVE1", "playerId": "host", "name": "Ann" }));
        let guest = join(s, json!({ "room": "LEAVE1", "playerId": "guest", "name": "Bob" }));
        assert_eq!(
            s.leave_room(&json!({ "room": "LEAVE1", "playerId": "host", "sessionToken": "nope" })).1["code"],
            "INVALID_SESSION"
        );
        // A guest leaving does not move the host; the room stays.
        assert_eq!(
            s.leave_room(&json!({ "room": "LEAVE1", "playerId": "guest", "sessionToken": token(&guest) })).1["ok"],
            true
        );
        let remaining = s.update_room(&json!({ "room": "LEAVE1", "playerId": "host", "sessionToken": token(&host) }));
        assert_eq!(remaining.0, 200);
        assert_eq!(remaining.1["isHost"], true);
        assert_eq!(remaining.1["roomSize"], 1);
        s.leave_room(&json!({ "room": "LEAVE1", "playerId": "host", "sessionToken": token(&host) }));
        assert!(!s.rooms.contains_key("LEAVE1"));

        let fresh = join(s, json!({ "room": "LEAVE1", "playerId": "host", "name": "Ann" }));
        assert_eq!(fresh["roomSize"], 1);
    }

    #[test]
    fn idle_players_and_rooms_expire_and_the_process_stops_at_512_rooms() {
        let mut c = service_with(0, 1_000, 5_000);
        let host = join(&mut c.service, json!({ "room": "HELLO", "playerId": "host", "name": "Ann" }));
        c.advance(900);
        let guest = join(&mut c.service, json!({ "room": "HELLO", "playerId": "guest", "name": "Bob" }));
        c.advance(101);
        let pruned = c.service.update_room(&json!({ "room": "HELLO", "playerId": "guest", "sessionToken": token(&guest) }));
        assert_eq!(pruned.1["roomSize"], 1);
        assert_eq!(pruned.1["isHost"], true);
        assert_eq!(pruned.1["player"]["name"], "BOB");

        let mut room_clock = service_with(0, 10_000, 5_000);
        let seeded = join(&mut room_clock.service, json!({ "room": "WORLD1", "playerId": "host", "name": "Ann" }));
        room_clock.advance(5_001);
        room_clock.service.cleanup();
        let recreated = join(&mut room_clock.service, json!({ "room": "WORLD1", "playerId": "host", "name": "Ann" }));
        assert_ne!(recreated["worldSeed"], seeded["worldSeed"]);

        let mut busy = service_at(1_000_000);
        for index in 0..512 {
            let code = format!("R{index:04}");
            join(&mut busy.service, json!({ "room": code, "playerId": "p", "name": "Ann" }));
        }
        assert_eq!(
            busy.service.join_room(&json!({ "room": "EXTRA1", "playerId": "p", "name": "Ann" })).1["code"],
            "SERVER_BUSY"
        );
        let still_there = join(&mut busy.service, json!({ "room": "R0000", "playerId": "q", "name": "Bob" }));
        assert_eq!(still_there["roomSize"], 2);
        assert_eq!(host["room"], "HELLO");
        assert_eq!(busy.service.rooms_len(), 512);
    }
}
