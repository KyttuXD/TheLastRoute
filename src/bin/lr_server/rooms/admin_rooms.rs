//! Admin rooms and room moderation for the web rooms (`RoomService`).
//!
//! * An admin (verified by the route, never by the body) who joins a code nobody uses creates the room, or gets a
//!   fresh free code when the code is left empty. The code must be 5-8 letters/digits and not used by any other
//!   kind of room (`JoinContext::taken` covers reserved and ENet codes). A non-admin who joins an unknown code
//!   with `joinOnly` set gets the unchanged `ROOM_NOT_FOUND` / "Room not found" refusal.
//! * An admin room keeps its seed and settings: `RoomService::admin_snapshot` is what the route stores through
//!   `ReservedRooms::remember_admin_room`, and `JoinContext::saved` hands it back when the code is created again.
//! * The host may KICK or BAN a player (`moderate_player`); accounts the caller marks as protected (owners) are
//!   refused, and the host cannot remove themselves.
//! * `list_rooms` is the listing for the room browser: capacity, held slots and the ADMIN flag; `sweep_empty`
//!   drops rooms nobody is in.

use super::room_service::{
    clean_name, clean_player_id, clean_room, clean_settings, default_room_settings, error, Room, RoomService,
    MAX_BANS_PER_ROOM,
};
use crate::util::{field, random_bytes, str_or_empty, Reply};
use serde_json::{json, Map, Value};

/// Letters and digits a generated code uses (no I, O, 0 or 1, as for the ENet codes).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// Length of a generated code.
const GENERATED_CODE_LEN: usize = 5;
const SEED_LIMIT: usize = 40;

/// What the route knows about the caller, passed to `RoomService::join_with`.
pub struct JoinContext<'a> {
    /// Verified server side (a signed-in admin or owner), never read from the request body.
    pub admin: bool,
    /// The admin-room settings stored for this code, if any (`ReservedRooms::admin_room`).
    pub saved: Option<Value>,
    /// True for a code used by another kind of room (reserved or ENet), which an admin may not take.
    pub taken_by_other: Option<&'a dyn Fn(&str) -> bool>,
}

impl<'a> JoinContext<'a> {
    pub fn plain() -> JoinContext<'static> {
        JoinContext { admin: false, saved: None, taken_by_other: None }
    }

    pub fn admin(saved: Option<Value>, taken_by_other: Option<&'a dyn Fn(&str) -> bool>) -> JoinContext<'a> {
        JoinContext { admin: true, saved, taken_by_other }
    }

    pub fn taken(&self, code: &str) -> bool {
        self.taken_by_other.map(|f| f(code)).unwrap_or(false)
    }
}

/// An admin-typed code: 5-8 letters/digits after trimming and upper-casing, and not in use elsewhere.
pub fn validate_admin_code(raw: &str, taken: &dyn Fn(&str) -> bool) -> Result<String, Reply> {
    let code = raw.trim().to_uppercase();
    let well_formed = (5..=8).contains(&code.len()) && code.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if !well_formed {
        return Err(error(400, "INVALID_ROOM", "Room codes must be 5-8 characters"));
    }
    if taken(&code) {
        return Err(error(409, "ROOM_IN_USE", "That code is already in use"));
    }
    Ok(code)
}

/// Marks a new room as an admin room and applies the settings stored for its code.
pub fn make_admin_room(room: &mut Room, saved: Option<&Value>) {
    room.admin = true;
    let Some(saved) = saved else { return };
    let seed = clean_seed(saved.get("seed"));
    if !seed.is_empty() {
        room.world_seed = seed;
    }
    if let Some(settings) = saved.get("settings") {
        for (key, value) in settings.as_object().cloned().unwrap_or_default() {
            if let Some(next) = clean_settings(Some(&json!({ key: value })), &room.settings) {
                room.settings = next;
            }
        }
    }
}

fn clean_seed(value: Option<&Value>) -> String {
    str_or_empty(value)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '_' || *c == '-')
        .take(SEED_LIMIT)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The part of an admin room the store keeps: `{ "seed": "...", "settings": { mapScale, difficulty, mode, ... } }`.
pub fn clean_snapshot(value: &Value) -> Value {
    let mut settings = default_room_settings();
    for (key, entry) in value.get("settings").and_then(Value::as_object).cloned().unwrap_or_default() {
        if let Some(next) = clean_settings(Some(&json!({ key: entry })), &settings) {
            settings = next;
        }
    }
    json!({ "seed": clean_seed(value.get("seed")), "settings": Value::Object(settings) })
}

impl RoomService {
    /// A random code nobody uses: not a live room and not taken elsewhere.
    pub(crate) fn free_code(&self, ctx: &JoinContext) -> String {
        let mut candidate = String::new();
        for _ in 0..64 {
            candidate = random_bytes(GENERATED_CODE_LEN)
                .iter()
                .map(|byte| CODE_ALPHABET[*byte as usize % CODE_ALPHABET.len()] as char)
                .collect();
            if !self.rooms.contains_key(&candidate) && !ctx.taken(&candidate) {
                break;
            }
        }
        candidate
    }

    /// The settings of an admin room, for `ReservedRooms::remember_admin_room`; `None` for any other room.
    pub fn admin_snapshot(&self, code: &str) -> Option<Value> {
        let room = self.rooms.get(&clean_room(Some(&Value::String(code.to_string()))))?;
        room.admin.then(|| clean_snapshot(&json!({ "seed": room.world_seed, "settings": Value::Object(room.settings.clone()) })))
    }

    /// Rooms nobody is in (and expired ones) are dropped; returns how many went.
    pub fn sweep_empty(&mut self) -> usize {
        let before = self.rooms.len();
        self.cleanup();
        before - self.rooms.len()
    }

    /// The room browser listing, after a sweep: one entry per live room, sorted by code.
    pub fn list_rooms(&mut self) -> Vec<Value> {
        self.sweep_empty();
        let mut codes: Vec<&String> = self.rooms.keys().collect();
        codes.sort();
        codes
            .into_iter()
            .map(|code| {
                let room = &self.rooms[code];
                let players = room.players.len();
                let held = room.held.len();
                let setting = |key: &str| room.settings.get(key).cloned().unwrap_or(Value::Null);
                json!({
                    "room": room.code,
                    "players": players,
                    "held": held,
                    "capacity": room.capacity,
                    "full": players + held >= room.capacity,
                    "admin": room.admin,
                    "mode": setting("mode"),
                    "difficulty": setting("difficulty"),
                })
            })
            .collect()
    }

    /// Host KICK / BAN: `{ room, playerId, sessionToken, action: "kick" | "ban", targetId | targetName }`.
    /// `protected` says an account name the host may not remove (owners, from `ops/moderation.rs`).
    pub fn moderate_player(&mut self, body: &Value, protected: &dyn Fn(&str) -> bool) -> Reply {
        let (code, _) = match self.authenticate(body, true) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        let action = str_or_empty(body.get("action")).trim().to_lowercase();
        if action != "kick" && action != "ban" {
            return error(400, "INVALID_ACTION", "Action must be kick or ban");
        }
        let target_id = clean_player_id(field(body, "targetId"));
        let target_name = clean_name(field(body, "targetName"));
        let now = self.now();
        let room = self.rooms.get_mut(&code).expect("room exists");
        let found = room
            .players
            .iter()
            .map(|p| (&p.id, &p.name))
            .chain(room.held.iter().map(|h| (&h.player.id, &h.player.name)))
            .find(|(id, name)| (!target_id.is_empty() && **id == target_id) || (target_id.is_empty() && !target_name.is_empty() && **name == target_name))
            .map(|(id, name)| (id.clone(), name.clone()));
        let Some((id, name)) = found else {
            return error(404, "PLAYER_NOT_IN_ROOM", "That player is not in this room");
        };
        if id == room.host_player_id {
            return error(400, "CANNOT_REMOVE_HOST", "The host cannot remove themselves");
        }
        if protected(&name) {
            return error(403, "PROTECTED_PLAYER", "That player cannot be removed");
        }
        room.players.retain(|p| p.id != id);
        room.held.retain(|h| h.player.id != id);
        let why = if action == "ban" { "BANNED" } else { "KICKED" };
        if room.removed.len() >= MAX_BANS_PER_ROOM {
            room.removed.clear();
        }
        room.removed.insert(id.clone(), why.to_string());
        if action == "ban" {
            if room.bans.len() >= MAX_BANS_PER_ROOM * 2 {
                return error(409, "BAN_LIST_FULL", "This room cannot ban any more players");
            }
            room.bans.insert(id);
            room.bans.insert(format!("name:{name}"));
        }
        room.last_active_at = now;
        room.reconcile();
        let mut out = Map::new();
        out.insert("ok".into(), Value::Bool(true));
        out.insert("action".into(), Value::String(action));
        out.insert("target".into(), Value::String(name));
        out.insert("roomSize".into(), json!(room.players.len()));
        (200, Value::Object(out))
    }
}
