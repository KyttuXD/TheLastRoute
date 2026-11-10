//! The story rooms: up to six players share one party. The host writes the scene, the flags and the inventory; the
//! players vote on the scene's choices; every change to the shared state is written to the party save first.
//!
//! Identity is settled by the server before a call gets here (an account session or a held guest name). After that,
//! every call proves membership with the player's `playerToken`, which only that player holds.
//!
//! Rules in short:
//! - a room code is 5 to 8 capital letters or digits; a room is created with a story pack id and its fingerprint;
//! - the pack must be multiplayer (`story.multiplayer` true), and a join must carry the room's fingerprint;
//! - the host is the first player; passing the host role closes nothing; when the host leaves without passing it, the
//!   longest-waiting player takes it; a room closes when its last player leaves (its party save stays on disk);
//! - a vote settles once every connected player has voted, or when its timeout (default 60 s) runs out: the most-voted
//!   choice wins, a tie goes to the lowest index, and the result is recorded in the party save;
//! - a player's pose is clamped into its ranges on every state write;
//! - the room directory is also the relay's room directory (`RelayRooms`): the relay code and token belong to the room,
//!   the token only to its host, and a guest is admitted only with the token of a player in the room.

use crate::enet_relay::{RelayRoom, RelayRooms};
use crate::packs::{PackError, PackRegistry};
use crate::saves::{PartySave, PartySaves, PartyState};
use crate::util::{self, base64url, random_bytes, safe_equal, str_or_empty, Clock, Reply};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const MAX_PLAYERS: usize = 6;
pub const DEFAULT_VOTE_TIMEOUT_SECONDS: u64 = 60;
pub const MAX_VOTE_TIMEOUT_SECONDS: u64 = 3600;
pub const MIN_CHOICES: usize = 2;
pub const MAX_CHOICES: usize = 8;
pub const CHOICE_LABEL_MAX: usize = 120;
pub const SCENE_ID_MAX: usize = 64;
pub const LIST_MAX_ITEMS: usize = 256;
pub const LIST_ITEM_MAX: usize = 64;
/// A player is connected while the server has heard from them (any call with their token) this recently.
pub const PRESENCE_MS: i64 = 15_000;
/// A room nobody has touched for this long leaves memory. Its party save stays on disk, so the code can be recreated.
pub const IDLE_CLOSE_MS: i64 = 30 * 60 * 1000;
/// Pose ranges: x and y in tiles, from minus to plus this; depth is a floor from 0 to this; facing in degrees.
pub const POSE_XY_LIMIT: f64 = 512.0;
pub const POSE_DEPTH_MAX: i64 = 8;
pub const POSE_HELD_MAX: usize = 64;
const ROOM_CODE_LENGTH: usize = 6;
const RELAY_CODE_LENGTH: usize = 5;
/// No I, O, 0 or 1: the codes are read aloud and typed.
const CODE_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// A room code is 5 to 8 characters, each a capital letter or a digit. Nothing is case-folded: the code must already
/// be in that form.
pub fn valid_room_code(code: &str) -> bool {
    (5..=8).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

fn valid_scene_id(id: &str) -> bool {
    (1..=SCENE_ID_MAX).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn random_code(length: usize) -> String {
    random_bytes(length).iter().map(|b| CODE_ALPHABET[(*b as usize) % CODE_ALPHABET.len()] as char).collect()
}

fn refuse(status: u16, code: &str, message: &str) -> Reply {
    (status, json!({ "ok": false, "code": code, "error": message }))
}

/// One player's pose, already clamped into its ranges.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub depth: i64,
    pub facing: f64,
    pub moving: bool,
    pub crouching: bool,
    pub sprinting: bool,
    pub held: Option<String>,
}

impl Pose {
    pub fn rest() -> Pose {
        Pose { x: 0.0, y: 0.0, depth: 0, facing: 0.0, moving: false, crouching: false, sprinting: false, held: None }
    }

    /// The pose a client sent, clamped: x and y to +-512, depth to 0..=8, facing to [0, 360), held to 64 characters.
    /// A missing or non-numeric number is 0, and a flag is true only when it is the boolean `true`.
    pub fn clamped(value: &Value) -> Pose {
        let number = |key: &str| {
            let n = util::js_number(value.get(key));
            if n.is_finite() {
                n
            } else {
                0.0
            }
        };
        let flag = |key: &str| value.get(key) == Some(&Value::Bool(true));
        Pose {
            x: number("x").clamp(-POSE_XY_LIMIT, POSE_XY_LIMIT),
            y: number("y").clamp(-POSE_XY_LIMIT, POSE_XY_LIMIT),
            depth: (number("depth").trunc() as i64).clamp(0, POSE_DEPTH_MAX),
            facing: number("facing").clamp(0.0, 359.999),
            moving: flag("moving"),
            crouching: flag("crouching"),
            sprinting: flag("sprinting"),
            held: match value.get("held") {
                Some(Value::String(text)) if !text.trim().is_empty() => Some(text.trim().chars().take(POSE_HELD_MAX).collect()),
                _ => None,
            },
        }
    }

    pub fn to_value(&self) -> Value {
        json!({
            "x": self.x,
            "y": self.y,
            "depth": self.depth,
            "facing": self.facing,
            "moving": self.moving,
            "crouching": self.crouching,
            "sprinting": self.sprinting,
            "held": self.held,
        })
    }
}

#[derive(Debug, Clone)]
struct Player {
    name: String,
    token: String,
    /// The join order: the smallest is the longest-waiting player.
    seq: u64,
    ready: bool,
    last_seen: i64,
    pose: Pose,
}

#[derive(Debug, Clone)]
struct Vote {
    scene: String,
    choices: Vec<String>,
    opened_at: i64,
    timeout_ms: i64,
    /// player name -> the index of the choice they picked (one ballot each)
    ballots: BTreeMap<String, usize>,
}

impl Vote {
    fn tally(&self) -> Vec<usize> {
        let mut counts = vec![0usize; self.choices.len()];
        for index in self.ballots.values() {
            if let Some(slot) = counts.get_mut(*index) {
                *slot += 1;
            }
        }
        counts
    }
}

/// The most-voted choice; a tie goes to the lowest index.
pub fn winner_of(tally: &[usize]) -> usize {
    let mut winner = 0;
    for (index, count) in tally.iter().enumerate() {
        if *count > tally[winner] {
            winner = index;
        }
    }
    winner
}

#[derive(Debug, Clone)]
struct Room {
    code: String,
    pack: String,
    fingerprint: String,
    allow_solo: bool,
    host: String,
    players: Vec<Player>,
    relay_code: String,
    relay_token: String,
    state: PartyState,
    vote: Option<Vote>,
    last_vote: Option<Value>,
    last_activity: i64,
}

fn connected(player: &Player, now: i64) -> bool {
    now - player.last_seen <= PRESENCE_MS
}

pub struct RoomServiceOptions {
    pub now: Clock,
    pub packs: PackRegistry,
    pub saves: PartySaves,
}

pub struct RoomService {
    now: Clock,
    packs: PackRegistry,
    saves: PartySaves,
    rooms: BTreeMap<String, Room>,
    next_seq: u64,
}

fn parse_choices(value: Option<&Value>) -> Result<Vec<String>, Reply> {
    let items = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(refuse(400, "BAD_CHOICES", "choices must be a list of labels")),
    };
    if items.is_empty() {
        return Ok(Vec::new());
    }
    if items.len() < MIN_CHOICES || items.len() > MAX_CHOICES {
        return Err(refuse(400, "BAD_CHOICES", "a vote needs 2 to 8 choices"));
    }
    let mut labels = Vec::with_capacity(items.len());
    for item in items {
        let label = item.as_str().map(str::trim).unwrap_or("");
        if label.is_empty() || label.chars().count() > CHOICE_LABEL_MAX {
            return Err(refuse(400, "BAD_CHOICES", "each choice is a label of 1 to 120 characters"));
        }
        labels.push(label.to_string());
    }
    Ok(labels)
}

fn parse_list(name: &str, value: &Value) -> Result<Vec<String>, Reply> {
    let bad = || refuse(400, "BAD_LIST", &format!("{name} must be a list of up to 256 strings of 1 to 64 characters"));
    let Value::Array(items) = value else { return Err(bad()) };
    if items.len() > LIST_MAX_ITEMS {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let text = item.as_str().map(str::trim).unwrap_or("");
        if text.is_empty() || text.chars().count() > LIST_ITEM_MAX {
            return Err(bad());
        }
        out.push(text.to_string());
    }
    Ok(out)
}

impl RoomService {
    pub fn new(opts: RoomServiceOptions) -> RoomService {
        RoomService { now: opts.now, packs: opts.packs, saves: opts.saves, rooms: BTreeMap::new(), next_seq: 0 }
    }

    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    fn now(&self) -> i64 {
        (self.now)()
    }

    fn bump_seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    fn new_code(&self) -> String {
        loop {
            let code = random_code(ROOM_CODE_LENGTH);
            if !self.rooms.contains_key(&code) && !self.saves.exists(&code) {
                return code;
            }
        }
    }

    fn new_relay_code(&self) -> String {
        loop {
            let code = random_code(RELAY_CODE_LENGTH);
            if !self.rooms.values().any(|room| room.relay_code == code) {
                return code;
            }
        }
    }

    /// Writes the party save before a change is applied: a failed write leaves the room as it was.
    fn persist(&self, code: &str, pack: &str, fingerprint: &str, state: &PartyState) -> Result<(), Reply> {
        let save = PartySave {
            code: code.to_string(),
            pack: pack.to_string(),
            fingerprint: fingerprint.to_string(),
            state: state.clone(),
            updated_at: self.now(),
        };
        self.saves.save(&save).map_err(|why| {
            util::log_error(&format!("[story] could not save party {code}: {why}"));
            refuse(500, "SAVE_FAILED", "The party could not be saved, so nothing changed")
        })
    }

    /// The room and the caller's player index, for a call that carries `code` and `playerToken`. A call counts as the
    /// player being heard from. A vote that is due settles first.
    fn member(&mut self, body: &Value) -> Result<(String, usize), Reply> {
        let code = str_or_empty(body.get("code"));
        if !valid_room_code(&code) {
            return Err(refuse(400, "INVALID_ROOM_CODE", "A room code is 5 to 8 capital letters or digits"));
        }
        if !self.rooms.contains_key(&code) {
            return Err(refuse(404, "ROOM_NOT_FOUND", "No room is open with that code"));
        }
        self.resolve_due(&code);
        let token = str_or_empty(body.get("playerToken"));
        let now = self.now();
        let Some(room) = self.rooms.get_mut(&code) else {
            return Err(refuse(404, "ROOM_NOT_FOUND", "No room is open with that code"));
        };
        let index = if token.is_empty() {
            None
        } else {
            room.players.iter().position(|p| safe_equal(&p.token, &token))
        };
        let Some(index) = index else {
            return Err(refuse(403, "NOT_IN_ROOM", "You are not a player in this room"));
        };
        room.players[index].last_seen = now;
        room.last_activity = now;
        Ok((code, index))
    }

    /// POST /api/rooms/create: `{code?, pack, packFingerprint}` by the identity `identity` (the host).
    pub fn create(&mut self, identity: &str, body: &Value) -> Reply {
        let now = self.now();
        let requested = match body.get("code") {
            None | Some(Value::Null) => None,
            Some(value) => Some(str_or_empty(Some(value))),
        };
        if let Some(code) = &requested {
            if !valid_room_code(code) {
                return refuse(400, "INVALID_ROOM_CODE", "A room code is 5 to 8 capital letters or digits");
            }
        }
        let pack_id = str_or_empty(body.get("pack"));
        let info = match self.packs.load(&pack_id) {
            Ok(info) => info,
            Err(PackError::Invalid(why)) => return refuse(400, "INVALID_PACK", &why),
            Err(PackError::Unknown) => return refuse(404, "UNKNOWN_PACK", "No story pack has that id on this server"),
            Err(PackError::Unreadable(why)) => {
                util::log_error(&format!("[story] pack {pack_id} cannot be read: {why}"));
                return refuse(500, "PACK_UNREADABLE", "That story pack cannot be read");
            }
        };
        if !info.multiplayer {
            return refuse(403, "MULTIPLAYER_DISABLED", "This story pack is not enabled for multiplayer");
        }
        if str_or_empty(body.get("packFingerprint")) != info.fingerprint {
            return refuse(409, "PACK_MISMATCH", "Your copy of this story pack differs from the server's copy");
        }
        let (code, state) = match requested {
            Some(code) => {
                if self.rooms.contains_key(&code) {
                    return refuse(409, "ROOM_CODE_TAKEN", "A room with that code is already open");
                }
                match self.saves.load(&code) {
                    Err(why) => {
                        util::log_error(&format!("[story] party save {code} cannot be read: {why}"));
                        return refuse(500, "PARTY_SAVE_UNREADABLE", "The saved party for that code cannot be read");
                    }
                    Ok(Some(save)) => {
                        if save.pack != info.id || save.fingerprint != info.fingerprint {
                            return refuse(409, "PACK_MISMATCH", "That party was played with another version of this pack");
                        }
                        (code, save.state)
                    }
                    Ok(None) => (code, PartyState::default()),
                }
            }
            None => (self.new_code(), PartyState::default()),
        };
        if let Err(refused) = self.persist(&code, &info.id, &info.fingerprint, &state) {
            return refused;
        }
        let token = base64url(&random_bytes(18));
        let seq = self.bump_seq();
        let host = Player {
            name: identity.to_string(),
            token: token.clone(),
            seq,
            ready: false,
            last_seen: now,
            pose: Pose::rest(),
        };
        let room = Room {
            code: code.clone(),
            pack: info.id.clone(),
            fingerprint: info.fingerprint.clone(),
            allow_solo: info.allow_solo,
            host: identity.to_string(),
            players: vec![host],
            relay_code: self.new_relay_code(),
            relay_token: base64url(&random_bytes(18)),
            state,
            vote: None,
            last_vote: None,
            last_activity: now,
        };
        self.rooms.insert(code.clone(), room);
        let view = self.view(&code, Some(&token));
        (200, json!({ "ok": true, "code": code, "playerToken": token, "room": view }))
    }

    /// POST /api/rooms/join: `{code, packFingerprint}` by the identity `identity`.
    pub fn join(&mut self, identity: &str, body: &Value) -> Reply {
        let now = self.now();
        let code = str_or_empty(body.get("code"));
        if !valid_room_code(&code) {
            return refuse(400, "INVALID_ROOM_CODE", "A room code is 5 to 8 capital letters or digits");
        }
        self.resolve_due(&code);
        let fingerprint = str_or_empty(body.get("packFingerprint"));
        let Some(room) = self.rooms.get(&code) else {
            return refuse(404, "ROOM_NOT_FOUND", "No room is open with that code");
        };
        if fingerprint != room.fingerprint {
            return refuse(409, "PACK_MISMATCH", "Your copy of this story pack differs from the room's");
        }
        if room.players.iter().any(|p| p.name == identity) {
            return refuse(409, "ALREADY_IN_ROOM", "You are already in this room");
        }
        if room.players.len() >= MAX_PLAYERS {
            return refuse(409, "ROOM_FULL", "This room already has six players");
        }
        let token = base64url(&random_bytes(18));
        let seq = self.bump_seq();
        let Some(room) = self.rooms.get_mut(&code) else {
            return refuse(404, "ROOM_NOT_FOUND", "No room is open with that code");
        };
        room.players.push(Player {
            name: identity.to_string(),
            token: token.clone(),
            seq,
            ready: false,
            last_seen: now,
            pose: Pose::rest(),
        });
        room.last_activity = now;
        let view = self.view(&code, Some(&token));
        (200, json!({ "ok": true, "code": code, "playerToken": token, "room": view }))
    }

    /// POST /api/rooms/leave: `{code, playerToken}`. A room closes when its last player leaves.
    pub fn leave(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.member(body) {
            Ok(found) => found,
            Err(refused) => return refused,
        };
        let now = self.now();
        let Some(room) = self.rooms.get_mut(&code) else {
            return refuse(404, "ROOM_NOT_FOUND", "No room is open with that code");
        };
        let left = room.players.remove(index);
        if room.players.is_empty() {
            self.rooms.remove(&code);
            return (200, json!({ "ok": true, "code": code, "closed": true }));
        }
        if room.host == left.name {
            room.host = room.players.iter().min_by_key(|p| p.seq).map(|p| p.name.clone()).unwrap_or_default();
        }
        room.last_activity = now;
        let host = room.host.clone();
        self.resolve_due(&code);
        (200, json!({ "ok": true, "code": code, "closed": false, "host": host }))
    }

    /// POST /api/rooms/ready: `{code, playerToken, ready?}` (ready defaults to true). The room has started when every
    /// player is ready and there are two players, or one player and a pack that allows a solo start.
    pub fn set_ready(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.member(body) {
            Ok(found) => found,
            Err(refused) => return refused,
        };
        let ready = match body.get("ready") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(flag)) => *flag,
            Some(_) => return refuse(400, "BAD_READY", "ready must be true or false"),
        };
        let token = str_or_empty(body.get("playerToken"));
        if let Some(room) = self.rooms.get_mut(&code) {
            if let Some(player) = room.players.get_mut(index) {
                player.ready = ready;
            }
        }
        let view = self.view(&code, Some(&token));
        (200, json!({ "ok": true, "code": code, "ready": ready, "room": view }))
    }

    /// POST /api/rooms/pass-host: `{code, playerToken, to}`. Only the host passes. The room stays open, and the relay
    /// token changes, so the old host's relay handle stops working and the new host gets the new one.
    pub fn pass_host(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.member(body) {
            Ok(found) => found,
            Err(refused) => return refused,
        };
        let token = str_or_empty(body.get("playerToken"));
        let target = str_or_empty(body.get("to")).trim().to_uppercase();
        let now = self.now();
        let Some(room) = self.rooms.get_mut(&code) else {
            return refuse(404, "ROOM_NOT_FOUND", "No room is open with that code");
        };
        let caller = room.players[index].name.clone();
        if room.host != caller {
            return refuse(403, "NOT_HOST", "Only the host can pass the host role");
        }
        if target == caller {
            return refuse(400, "ALREADY_HOST", "You are already the host");
        }
        if !room.players.iter().any(|p| p.name == target) {
            return refuse(404, "PLAYER_NOT_IN_ROOM", "That player is not in this room");
        }
        room.host = target.clone();
        room.relay_token = base64url(&random_bytes(18));
        room.last_activity = now;
        let view = self.view(&code, Some(&token));
        (200, json!({ "ok": true, "code": code, "host": target, "room": view }))
    }

    /// POST /api/rooms/vote: `{code, playerToken, choice}` where choice is the index of the choice. One ballot each.
    pub fn vote(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.member(body) {
            Ok(found) => found,
            Err(refused) => return refused,
        };
        let token = str_or_empty(body.get("playerToken"));
        let Some(choice) = body.get("choice").and_then(Value::as_u64) else {
            return refuse(400, "BAD_CHOICE", "choice must be the index of one of the choices");
        };
        let Some(room) = self.rooms.get_mut(&code) else {
            return refuse(404, "ROOM_NOT_FOUND", "No room is open with that code");
        };
        let name = room.players[index].name.clone();
        let Some(vote) = room.vote.as_mut() else {
            return refuse(409, "NO_OPEN_VOTE", "There is no vote open right now");
        };
        if vote.ballots.contains_key(&name) {
            return refuse(409, "ALREADY_VOTED", "You have already voted on this choice");
        }
        if choice >= vote.choices.len() as u64 {
            return refuse(400, "BAD_CHOICE", "choice must be the index of one of the choices");
        }
        vote.ballots.insert(name, choice as usize);
        self.resolve_due(&code);
        let view = self.view(&code, Some(&token));
        (200, json!({ "ok": true, "code": code, "voted": choice, "room": view }))
    }

    /// POST /api/rooms/state: `{code, playerToken, scene?, flags?, inventory?, pose?}`. The host writes the scene, the
    /// flags and the inventory (`scene` = `{id, choices?, timeoutSeconds?}`; a list of choices opens a vote). Any player
    /// may send a pose. The reply is the room as this player sees it.
    pub fn write_state(&mut self, body: &Value) -> Reply {
        let (code, index) = match self.member(body) {
            Ok(found) => found,
            Err(refused) => return refused,
        };
        let token = str_or_empty(body.get("playerToken"));
        let now = self.now();
        let (is_host, pack, fingerprint, current, vote_open) = {
            let Some(room) = self.rooms.get(&code) else {
                return refuse(404, "ROOM_NOT_FOUND", "No room is open with that code");
            };
            let caller = &room.players[index].name;
            (room.host == *caller, room.pack.clone(), room.fingerprint.clone(), room.state.clone(), room.vote.is_some())
        };
        let scene = body.get("scene").filter(|v| !v.is_null());
        let flags = body.get("flags").filter(|v| !v.is_null());
        let inventory = body.get("inventory").filter(|v| !v.is_null());
        let persists = scene.is_some() || flags.is_some() || inventory.is_some();
        if persists && !is_host {
            return refuse(403, "NOT_HOST", "Only the host writes the shared party state");
        }
        let mut next = current;
        let mut opened = None;
        if let Some(scene) = scene {
            if vote_open {
                return refuse(409, "VOTE_OPEN", "Wait for the open vote to settle before the scene changes");
            }
            let id = str_or_empty(scene.get("id"));
            if !valid_scene_id(&id) {
                return refuse(400, "BAD_SCENE", "A scene id is 1 to 64 letters, digits, '_' or '-'");
            }
            let labels = match parse_choices(scene.get("choices")) {
                Ok(labels) => labels,
                Err(refused) => return refused,
            };
            let timeout_seconds = match scene.get("timeoutSeconds") {
                None | Some(Value::Null) => DEFAULT_VOTE_TIMEOUT_SECONDS,
                Some(value) => match value.as_u64() {
                    Some(seconds) if (1..=MAX_VOTE_TIMEOUT_SECONDS).contains(&seconds) => seconds,
                    _ => return refuse(400, "BAD_TIMEOUT", "timeoutSeconds must be from 1 to 3600"),
                },
            };
            next.scene = Some(id.clone());
            if !labels.is_empty() {
                opened = Some(Vote {
                    scene: id,
                    choices: labels,
                    opened_at: now,
                    timeout_ms: timeout_seconds as i64 * 1000,
                    ballots: BTreeMap::new(),
                });
            }
        }
        if let Some(list) = flags {
            next.flags = match parse_list("flags", list) {
                Ok(items) => items,
                Err(refused) => return refused,
            };
        }
        if let Some(list) = inventory {
            next.inventory = match parse_list("inventory", list) {
                Ok(items) => items,
                Err(refused) => return refused,
            };
        }
        if persists {
            if let Err(refused) = self.persist(&code, &pack, &fingerprint, &next) {
                return refused;
            }
        }
        let Some(room) = self.rooms.get_mut(&code) else {
            return refuse(404, "ROOM_NOT_FOUND", "No room is open with that code");
        };
        if persists {
            room.state = next;
        }
        if let Some(vote) = opened {
            room.vote = Some(vote);
        }
        if let Some(pose) = body.get("pose").filter(|v| v.is_object()) {
            if let Some(player) = room.players.get_mut(index) {
                player.pose = Pose::clamped(pose);
            }
        }
        room.last_activity = now;
        let view = self.view(&code, Some(&token));
        (200, json!({ "ok": true, "code": code, "room": view }))
    }

    /// GET /api/rooms/{code}: the public view. The relay handle is added for the host only (see `view`).
    pub fn get(&mut self, code: &str) -> Reply {
        if !valid_room_code(code) {
            return refuse(400, "INVALID_ROOM_CODE", "A room code is 5 to 8 capital letters or digits");
        }
        self.resolve_due(code);
        match self.rooms.get(code) {
            Some(_) => (200, json!({ "ok": true, "code": code, "room": self.view(code, None) })),
            None => refuse(404, "ROOM_NOT_FOUND", "No room is open with that code"),
        }
    }

    /// Settles every due vote and drops rooms nobody has touched for `IDLE_CLOSE_MS`. Called on the maintenance timer.
    pub fn tick(&mut self) {
        let codes: Vec<String> = self.rooms.keys().cloned().collect();
        for code in &codes {
            self.resolve_due(code);
        }
        let now = self.now();
        self.rooms.retain(|_, room| now - room.last_activity <= IDLE_CLOSE_MS);
    }

    /// Settles the room's vote when every connected player has voted or the timeout has run out. The most-voted choice
    /// wins (a tie goes to the lowest index) and the result is recorded in the party save. If the save cannot be
    /// written the vote stays open and is tried again on the next call.
    fn resolve_due(&mut self, code: &str) {
        let now = self.now();
        let (pack, fingerprint, state, record) = {
            let Some(room) = self.rooms.get(code) else { return };
            let Some(vote) = room.vote.as_ref() else { return };
            let mut any_connected = false;
            let mut all_voted = true;
            for player in room.players.iter().filter(|p| connected(p, now)) {
                any_connected = true;
                if !vote.ballots.contains_key(&player.name) {
                    all_voted = false;
                }
            }
            let timed_out = now - vote.opened_at >= vote.timeout_ms;
            if !((any_connected && all_voted) || timed_out) {
                return;
            }
            let tally = vote.tally();
            let winner = winner_of(&tally);
            let record = json!({
                "scene": vote.scene,
                "index": winner,
                "label": vote.choices.get(winner).cloned().unwrap_or_default(),
                "tally": tally,
                "at": now,
            });
            let mut state = room.state.clone();
            state.choices.push(record.clone());
            (room.pack.clone(), room.fingerprint.clone(), state, record)
        };
        if self.persist(code, &pack, &fingerprint, &state).is_err() {
            return;
        }
        if let Some(room) = self.rooms.get_mut(code) {
            room.state = state;
            room.vote = None;
            room.last_vote = Some(record);
        }
    }

    /// The room as the caller sees it. `token` is the caller's player token (None for a public read). The relay handle
    /// (`relay.code` and `relay.token`) is included only when the caller is the host.
    pub fn view(&self, code: &str, token: Option<&str>) -> Value {
        let now = self.now();
        let Some(room) = self.rooms.get(code) else { return Value::Null };
        let caller_is_host = token
            .and_then(|t| room.players.iter().find(|p| safe_equal(&p.token, t)))
            .is_some_and(|p| p.name == room.host);
        let mut players: Vec<&Player> = room.players.iter().collect();
        players.sort_by_key(|p| p.seq);
        let players: Vec<Value> = players
            .into_iter()
            .map(|p| {
                let mut entry = json!({
                    "name": p.name,
                    "ready": p.ready,
                    "host": p.name == room.host,
                    "connected": connected(p, now),
                    "pose": p.pose.to_value(),
                });
                if let Some(vote) = &room.vote {
                    entry["voted"] = json!(vote.ballots.contains_key(&p.name));
                }
                entry
            })
            .collect();
        let started = room.players.iter().all(|p| p.ready) && (room.players.len() >= 2 || room.allow_solo);
        let vote = room.vote.as_ref().map(|v| {
            json!({
                "scene": v.scene,
                "choices": v.choices,
                "timeoutSeconds": v.timeout_ms / 1000,
                "endsAt": v.opened_at + v.timeout_ms,
                "tally": v.tally(),
                "voted": v.ballots.keys().collect::<Vec<_>>(),
            })
        });
        let mut out = json!({
            "code": room.code,
            "pack": room.pack,
            "packFingerprint": room.fingerprint,
            "host": room.host,
            "started": started,
            "allowSolo": room.allow_solo,
            "maxPlayers": MAX_PLAYERS,
            "players": players,
            "scene": room.state.scene,
            "flags": room.state.flags,
            "inventory": room.state.inventory,
            "choices": room.state.choices,
            "vote": vote,
            "lastVote": room.last_vote,
        });
        if caller_is_host {
            out["relay"] = json!({ "code": room.relay_code, "token": room.relay_token });
        }
        out
    }
}

/// The relay asks the room directory: a relay code names one room, and only a player of that room is admitted (with
/// their player token as the relay's join nonce). The host's relay token is the room's relay token.
impl RelayRooms for RoomService {
    fn room_by_code(&mut self, code: &str) -> Option<RelayRoom> {
        self.rooms
            .values()
            .find(|room| room.relay_code == code)
            .map(|room| RelayRoom { token: room.relay_token.clone(), gated: true })
    }

    fn admit_guest(&mut self, code: &str, nonce: &[u8]) -> bool {
        let Ok(token) = std::str::from_utf8(nonce) else { return false };
        self.rooms
            .values()
            .any(|room| room.relay_code == code && room.players.iter().any(|p| safe_equal(&p.token, token)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::{hex, manual_clock, ManualClock};
    use std::fs;
    use std::path::{Path, PathBuf};

    struct Fixture {
        service: RoomService,
        clock: ManualClock,
        root: PathBuf,
    }

    fn write_pack(root: &Path, id: &str, story: &str) {
        let folder = root.join("mods").join(id);
        fs::create_dir_all(folder.join("scenes")).unwrap();
        fs::write(folder.join("pack.json"), format!(r#"{{"id":"{id}","version":"1.0.0","story":{story}}}"#)).unwrap();
        fs::write(folder.join("scenes/main.json"), "{}").unwrap();
    }

    fn fixture(name: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("lr_story_rooms_{name}_{}", hex(&random_bytes(6))));
        write_pack(&root, "party_pack", r#"{"multiplayer": true}"#);
        write_pack(&root, "solo_only", r#"{"multiplayer": true, "allow_solo": false}"#);
        write_pack(&root, "closed_pack", r#"{"multiplayer": false}"#);
        let (clock, handle) = manual_clock(1_700_000_000_000);
        let service = RoomService::new(RoomServiceOptions {
            now: clock,
            packs: PackRegistry::new(root.join("mods")),
            saves: PartySaves::new(root.join("parties")),
        });
        Fixture { service, clock: handle, root }
    }

    /// The fingerprint of a pack; a pack that does not load gets a placeholder (the refusal then comes from the pack).
    fn fingerprint_of(f: &Fixture, pack: &str) -> String {
        f.service.packs.load(pack).map(|info| info.fingerprint).unwrap_or_else(|_| "no-such-pack".to_string())
    }

    fn create(f: &mut Fixture, host: &str, pack: &str, code: Option<&str>) -> Reply {
        let mut body = json!({ "pack": pack, "packFingerprint": fingerprint_of(f, pack) });
        if let Some(code) = code {
            body["code"] = json!(code);
        }
        f.service.create(host, &body)
    }

    fn join(f: &mut Fixture, name: &str, code: &str, pack: &str) -> Reply {
        let fingerprint = fingerprint_of(f, pack);
        f.service.join(name, &json!({ "code": code, "packFingerprint": fingerprint }))
    }

    fn code_of(reply: &Reply) -> String {
        reply.1["code"].as_str().unwrap().to_string()
    }

    fn token_of(reply: &Reply) -> String {
        reply.1["playerToken"].as_str().unwrap().to_string()
    }

    fn open_vote(f: &mut Fixture, code: &str, host_token: &str, timeout: u64, choices: &[&str]) -> Reply {
        f.service.write_state(&json!({
            "code": code,
            "playerToken": host_token,
            "scene": { "id": "hall", "choices": choices, "timeoutSeconds": timeout },
        }))
    }

    fn vote(f: &mut Fixture, code: &str, token: &str, choice: u64) -> Reply {
        f.service.vote(&json!({ "code": code, "playerToken": token, "choice": choice }))
    }

    #[test]
    fn room_codes_are_five_to_eight_uppercase_letters_or_digits() {
        for good in ["ABCDE", "ABCD2345", "X7Y8Z", "K2M4P6Q8"] {
            assert!(valid_room_code(good), "{good}");
        }
        for bad in ["ABCD", "ABCDEFGHI", "abcde", "ABC-D", "ABC D", "", "ABCD!", "ÄBCDE"] {
            assert!(!valid_room_code(bad), "{bad}");
        }
    }

    #[test]
    fn create_refuses_a_malformed_requested_code_and_generates_a_valid_one() {
        let mut f = fixture("code");
        for bad in ["abc", "TOOLONGCODE1", "AB-CD", "abcde"] {
            let refused = create(&mut f, "P1", "party_pack", Some(bad));
            assert_eq!(refused.0, 400, "{bad}");
            assert_eq!(refused.1["code"], "INVALID_ROOM_CODE");
        }
        let created = create(&mut f, "P1", "party_pack", None);
        assert_eq!(created.0, 200);
        assert!(valid_room_code(&code_of(&created)));
    }

    #[test]
    fn a_room_takes_six_players_and_the_seventh_is_refused() {
        let mut f = fixture("six");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        for name in ["P2", "P3", "P4", "P5", "P6"] {
            assert_eq!(join(&mut f, name, &code, "party_pack").0, 200, "{name} joins");
        }
        let seventh = join(&mut f, "P7", &code, "party_pack");
        assert_eq!(seventh.0, 409);
        assert_eq!(seventh.1["code"], "ROOM_FULL");
        let again = join(&mut f, "P2", &code, "party_pack");
        assert_eq!(again.0, 409, "a player cannot join the same room twice");
        assert_eq!(again.1["code"], "ALREADY_IN_ROOM");
        assert_eq!(f.service.view(&code, None)["players"].as_array().unwrap().len(), 6);
    }

    #[test]
    fn a_different_pack_fingerprint_is_refused_on_create_and_on_join() {
        let mut f = fixture("mismatch");
        let refused = f.service.create("P1", &json!({ "pack": "party_pack", "packFingerprint": "00" }));
        assert_eq!(refused.0, 409);
        assert_eq!(refused.1["code"], "PACK_MISMATCH");
        let code = code_of(&create(&mut f, "P1", "party_pack", None));
        let refused = f.service.join("P2", &json!({ "code": code, "packFingerprint": "not-the-pack" }));
        assert_eq!(refused.0, 409);
        assert_eq!(refused.1["code"], "PACK_MISMATCH");
        assert_eq!(join(&mut f, "P2", &code, "party_pack").0, 200, "the right fingerprint joins");
    }

    #[test]
    fn a_pack_that_is_not_multiplayer_cannot_open_a_room() {
        let mut f = fixture("multiplayer");
        let refused = create(&mut f, "P1", "closed_pack", None);
        assert_eq!(refused.0, 403);
        assert_eq!(refused.1["code"], "MULTIPLAYER_DISABLED");
        assert_eq!(f.service.room_count(), 0);
        assert_eq!(create(&mut f, "P1", "no_such_pack", None).0, 404);
        assert_eq!(create(&mut f, "P1", "../party_pack", None).0, 400);
    }

    #[test]
    fn the_host_can_pass_the_role_and_the_room_stays_open() {
        let mut f = fixture("pass");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let host_token = token_of(&created);
        let guest_token = token_of(&join(&mut f, "P2", &code, "party_pack"));
        let refused = f.service.pass_host(&json!({ "code": code, "playerToken": guest_token, "to": "P2" }));
        assert_eq!(refused.0, 403, "only the host passes the role");
        let passed = f.service.pass_host(&json!({ "code": code, "playerToken": host_token, "to": "P2" }));
        assert_eq!(passed.0, 200);
        assert_eq!(f.service.view(&code, None)["host"], "P2");
        assert_eq!(f.service.room_count(), 1, "passing the host closes nothing");
        assert!(f.service.view(&code, Some(&host_token)).get("relay").is_none(), "the old host lost the relay handle");
        assert!(f.service.view(&code, Some(&guest_token))["relay"]["token"].is_string(), "the new host has it");
    }

    #[test]
    fn when_the_host_leaves_without_passing_the_longest_waiting_player_takes_over() {
        let mut f = fixture("succession");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let host_token = token_of(&created);
        let p2 = token_of(&join(&mut f, "P2", &code, "party_pack"));
        let p3 = token_of(&join(&mut f, "P3", &code, "party_pack"));
        let p4 = token_of(&join(&mut f, "P4", &code, "party_pack"));
        assert_eq!(f.service.leave(&json!({ "code": code, "playerToken": p3 })).0, 200);
        let left = f.service.leave(&json!({ "code": code, "playerToken": host_token }));
        assert_eq!(left.1["host"], "P2", "P2 has waited longest of those still here");
        assert_eq!(f.service.view(&code, None)["host"], "P2");
        assert_eq!(f.service.leave(&json!({ "code": code, "playerToken": p2 })).1["host"], "P4");
        let last = f.service.leave(&json!({ "code": code, "playerToken": p4 }));
        assert_eq!(last.1["closed"], true, "the room closes when its last player leaves");
        assert_eq!(f.service.room_count(), 0);
    }

    #[test]
    fn a_room_starts_with_one_player_only_when_the_pack_allows_it() {
        let mut f = fixture("solo");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let token = token_of(&created);
        f.service.set_ready(&json!({ "code": code, "playerToken": token, "ready": true }));
        assert_eq!(f.service.view(&code, None)["started"], true, "default pack: one ready player starts");

        let strict = create(&mut f, "Q1", "solo_only", None);
        let strict_code = code_of(&strict);
        let strict_token = token_of(&strict);
        f.service.set_ready(&json!({ "code": strict_code, "playerToken": strict_token }));
        assert_eq!(f.service.view(&strict_code, None)["started"], false, "this pack wants two players");
        let second = token_of(&join(&mut f, "Q2", &strict_code, "solo_only"));
        f.service.set_ready(&json!({ "code": strict_code, "playerToken": second }));
        assert_eq!(f.service.view(&strict_code, None)["started"], true);
    }

    #[test]
    fn a_vote_settles_on_the_most_voted_choice_once_every_connected_player_has_voted() {
        let mut f = fixture("majority");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let host = token_of(&created);
        let p2 = token_of(&join(&mut f, "P2", &code, "party_pack"));
        let p3 = token_of(&join(&mut f, "P3", &code, "party_pack"));
        assert_eq!(open_vote(&mut f, &code, &host, 60, &["Open it", "Leave it", "Run"]).0, 200);
        assert_eq!(f.service.view(&code, None)["vote"]["timeoutSeconds"], 60);
        assert_eq!(vote(&mut f, &code, &host, 1).0, 200);
        assert_eq!(vote(&mut f, &code, &p2, 2).0, 200);
        assert!(f.service.view(&code, None)["vote"].is_object(), "one player has not voted: still open");
        assert_eq!(vote(&mut f, &code, &p3, 2).0, 200);
        let view = f.service.view(&code, None);
        assert!(view["vote"].is_null());
        assert_eq!(view["lastVote"]["index"], 2);
        assert_eq!(view["lastVote"]["label"], "Run");
        assert_eq!(view["lastVote"]["tally"], json!([0, 1, 2]));
        assert_eq!(view["choices"][0]["index"], 2, "the result is recorded in the shared state");
    }

    #[test]
    fn a_tie_goes_to_the_lowest_index() {
        let mut f = fixture("tie");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let host = token_of(&created);
        let p2 = token_of(&join(&mut f, "P2", &code, "party_pack"));
        assert_eq!(open_vote(&mut f, &code, &host, 60, &["Left", "Right"]).0, 200);
        vote(&mut f, &code, &host, 1);
        vote(&mut f, &code, &p2, 0);
        assert_eq!(f.service.view(&code, None)["lastVote"]["index"], 0, "one vote each: lowest index wins");
        assert_eq!(winner_of(&[0, 0, 0]), 0, "no votes at all also goes to index 0");
        assert_eq!(winner_of(&[1, 2, 2]), 1);
    }

    #[test]
    fn a_vote_times_out_and_the_votes_in_hand_decide_it() {
        let mut f = fixture("timeout");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let host = token_of(&created);
        let p2 = token_of(&join(&mut f, "P2", &code, "party_pack"));
        let _p3 = token_of(&join(&mut f, "P3", &code, "party_pack"));
        assert_eq!(open_vote(&mut f, &code, &host, 30, &["A", "B"]).0, 200);
        vote(&mut f, &code, &host, 1);
        vote(&mut f, &code, &p2, 1);
        f.clock.advance(29_000);
        f.service.tick();
        assert!(f.service.view(&code, None)["vote"].is_object(), "29 s in: the timeout has not run out");
        f.clock.advance(1_000);
        f.service.tick();
        let view = f.service.view(&code, None);
        assert!(view["vote"].is_null(), "30 s in: the vote settles");
        assert_eq!(view["lastVote"]["index"], 1);
        assert_eq!(view["lastVote"]["tally"], json!([0, 2]));
    }

    #[test]
    fn a_player_votes_once_and_only_while_a_vote_is_open() {
        let mut f = fixture("ballot");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let host = token_of(&created);
        let p2 = token_of(&join(&mut f, "P2", &code, "party_pack"));
        let _p3 = token_of(&join(&mut f, "P3", &code, "party_pack"));
        assert_eq!(vote(&mut f, &code, &host, 0).0, 409, "no vote is open yet");
        open_vote(&mut f, &code, &host, 60, &["A", "B"]);
        assert_eq!(vote(&mut f, &code, &host, 0).0, 200);
        let twice = vote(&mut f, &code, &host, 1);
        assert_eq!((twice.0, twice.1["code"].clone()), (409, json!("ALREADY_VOTED")));
        assert_eq!(vote(&mut f, &code, &p2, 9).0, 400, "a choice that does not exist");
        assert_eq!(f.service.write_state(&json!({ "code": code, "playerToken": host, "scene": { "id": "x" } })).0, 409, "the scene waits for the vote");
    }

    #[test]
    fn the_shared_save_is_written_on_each_change_and_reloaded_by_code() {
        let mut f = fixture("save");
        let created = create(&mut f, "P1", "party_pack", Some("SAVE7"));
        assert_eq!(created.0, 200);
        let host = token_of(&created);
        let p2 = token_of(&join(&mut f, "P2", "SAVE7", "party_pack"));
        let written = f.service.write_state(&json!({
            "code": "SAVE7",
            "playerToken": host,
            "scene": { "id": "cellar_door" },
            "flags": ["heard_the_bell"],
            "inventory": ["lantern"],
        }));
        assert_eq!(written.0, 200);
        let saved = f.service.saves.load("SAVE7").unwrap().unwrap();
        assert_eq!(saved.state.scene.as_deref(), Some("cellar_door"));
        assert_eq!(saved.state.flags, vec!["heard_the_bell".to_string()]);
        assert_eq!(saved.state.inventory, vec!["lantern".to_string()]);

        let guest_write = f.service.write_state(&json!({ "code": "SAVE7", "playerToken": p2, "flags": [] }));
        assert_eq!(guest_write.0, 403, "only the host writes the shared save");
        assert_eq!(f.service.saves.load("SAVE7").unwrap().unwrap().state.flags.len(), 1);

        open_vote(&mut f, "SAVE7", &host, 60, &["Open it", "Leave it"]);
        vote(&mut f, "SAVE7", &host, 0);
        vote(&mut f, "SAVE7", &p2, 0);
        assert_eq!(f.service.saves.load("SAVE7").unwrap().unwrap().state.choices.len(), 1, "the vote result is saved");

        assert_eq!(f.service.leave(&json!({ "code": "SAVE7", "playerToken": p2 })).0, 200);
        assert_eq!(f.service.leave(&json!({ "code": "SAVE7", "playerToken": host })).1["closed"], true);
        assert_eq!(f.service.room_count(), 0);
        assert!(f.root.join("parties").join("SAVE7.json").is_file());

        let other_pack = create(&mut f, "P9", "solo_only", Some("SAVE7"));
        assert_eq!(other_pack.1["code"], "PACK_MISMATCH", "a save only loads with the pack it was played with");
        assert_eq!(f.service.room_count(), 0, "a refused create opens nothing");

        let again = create(&mut f, "P9", "party_pack", Some("SAVE7"));
        assert_eq!(again.0, 200, "the host recreates the room with the same code");
        let view = f.service.view("SAVE7", None);
        // the vote's scene ("hall") was the last one the host wrote before the room closed
        assert_eq!(view["scene"], "hall");
        assert_eq!(view["flags"], json!(["heard_the_bell"]));
        assert_eq!(view["inventory"], json!(["lantern"]));
        assert_eq!(view["choices"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_pose_is_clamped_into_its_ranges_and_relayed_on_the_state_route() {
        let mut f = fixture("pose");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let host = token_of(&created);
        let wild = json!({ "x": 9999, "y": -9999, "depth": 99, "facing": 720, "moving": true, "crouching": "yes", "sprinting": false, "held": "lantern" });
        let reply = f.service.write_state(&json!({ "code": code, "playerToken": host, "pose": wild }));
        assert_eq!(reply.0, 200);
        let pose = &f.service.view(&code, None)["players"][0]["pose"];
        assert_eq!(pose["x"], 512.0);
        assert_eq!(pose["y"], -512.0);
        assert_eq!(pose["depth"], 8);
        let facing = pose["facing"].as_f64().unwrap();
        assert!((359.9..360.0).contains(&facing), "facing wraps into the clamp: {facing}");
        assert_eq!(pose["moving"], true);
        assert_eq!(pose["crouching"], false, "only a boolean true is true");
        assert_eq!(pose["held"], "lantern");

        let low = Pose::clamped(&json!({ "x": "abc", "depth": -4, "facing": -10, "held": "x".repeat(100) }));
        assert_eq!((low.x, low.depth, low.facing), (0.0, 0, 0.0));
        assert_eq!(low.held.as_ref().map(|h| h.chars().count()), Some(POSE_HELD_MAX));
    }

    #[test]
    fn the_relay_admits_only_players_of_the_room_and_hands_the_token_to_the_host() {
        let mut f = fixture("relay");
        let created = create(&mut f, "P1", "party_pack", None);
        let code = code_of(&created);
        let host = token_of(&created);
        let guest = token_of(&join(&mut f, "P2", &code, "party_pack"));
        let view = f.service.view(&code, Some(&host));
        let relay_code = view["relay"]["code"].as_str().unwrap().to_string();
        let relay_token = view["relay"]["token"].as_str().unwrap().to_string();
        let room = f.service.room_by_code(&relay_code).unwrap();
        assert_eq!(room.token, relay_token);
        assert!(room.gated);
        assert!(f.service.admit_guest(&relay_code, guest.as_bytes()));
        assert!(!f.service.admit_guest(&relay_code, b"not-a-player-token"));
        assert!(f.service.room_by_code("ZZZZZ").is_none());
        assert!(f.service.view(&code, Some(&guest)).get("relay").is_none(), "only the host sees the relay handle");
    }
}
