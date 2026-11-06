//! Online rooms hosted by the server (design phase 3). No client hosts an online room: CREATE ROOM asks the directory to
//! have a server-side room process opened, waits until the room is listed, and joins it as a guest over the WebSocket
//! relay like everybody else. The account that created the room is its owner, who runs the room through the directory.
//!
//! The Godot client's flow, endpoint for endpoint (`autoload/backend_client.gd`, `autoload/room_state.gd`, server
//! `backend/server.mjs`):
//!
//! | step | request | answer |
//! |---|---|---|
//! | create | `POST /api/rooms/hosted/create {sessionToken, difficulty}` | `202 {ok, code, region, relay, owner}` |
//! | wait | `GET /api/enet/rooms/CODE`, [`Rules::poll_attempts`] times, [`Rules::poll_interval`] apart | `200 {ok, room{..., owner}}` once listed, else `404` |
//! | owner ticket | `POST /api/enet/owner-tickets/issue {code, sessionToken}` | `201 {ok, ticket, room, expiresIn}` (only the owner's account) |
//! | owner command | the ticket and the action travel to the room over the relay (`_owner_command`) | the room answers `_owner_refused` when it refuses |
//! | verify (room side) | `POST /api/enet/owner-tickets/verify {code, ticket, hostToken, action}` | `200 {ok, authorized}`, else `403 {error}` |
//!
//! The room never trusts a client's claim of ownership: it runs a command only after the directory has confirmed the
//! ticket with the room's own host token (`headless/dedicated.rs`). A ticket works once per action.
//!
//! Every player-facing line is data (`data/net/hosted_room.json`). Nothing here logs a session token, a ticket or a host
//! token: [`OwnerTicket`] prints as `<redacted>`.

use super::api::{self, Json};
use super::config::BackendConfig;
use super::world_sync::NetSession;
use super::{NetError, Pending};
use crate::config::difficulty::LOBBY;
use crate::config::Difficulty;
use crate::shared::clock::Instant;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use std::time::Duration;

pub const SCHEMA: &str = "lr.hosted_room/1";

/// A base64url token of 32 bytes is 43 characters (the directory's owner ticket).
pub const TICKET_LEN: usize = 43;
/// The longest action name the directory takes (`slice(0, 32)` in server.mjs).
pub const MAX_ACTION_LEN: usize = 32;

/// The player-facing lines, from `data/net/hosted_room.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Strings {
    pub sign_in_required: String,
    pub offline: String,
    pub creating: String,
    pub unreachable: String,
    pub create_failed: String,
    pub waiting: String,
    pub waiting_check: String,
    pub not_started: String,
    pub opened_owner: String,
    pub lobby_owner_line: String,
    pub lobby_owner_detail: String,
    pub lobby_waiting_line: String,
    pub owner_confirming: String,
    pub owner_busy: String,
    pub owner_sign_in: String,
    pub owner_check_failed: String,
    pub owner_refused: String,
    pub owner_refused_default: String,
    pub owner_not_in_room: String,
    pub label_start: String,
    pub label_end: String,
    pub label_restart: String,
    pub label_kick: String,
    pub tab_owner: String,
    pub owner_heading: String,
    pub owner_lobby_note: String,
    pub owner_pause_note: String,
    pub owner_end_reason: String,
    pub owner_restart_reason: String,
}

/// The parsed `lr.hosted_room/1` file.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    /// How many times the room's listing is checked before the create is given up (Godot `HOSTED_JOIN_ATTEMPTS`).
    pub poll_attempts: u32,
    /// The time between two checks (Godot `HOSTED_JOIN_INTERVAL`).
    pub poll_interval: Duration,
    pub strings: Strings,
}

const STRING_KEYS: [&str; 29] = [
    "sign_in_required",
    "offline",
    "creating",
    "unreachable",
    "create_failed",
    "waiting",
    "waiting_check",
    "not_started",
    "opened_owner",
    "lobby_owner_line",
    "lobby_owner_detail",
    "lobby_waiting_line",
    "owner_confirming",
    "owner_busy",
    "owner_sign_in",
    "owner_check_failed",
    "owner_refused",
    "owner_refused_default",
    "owner_not_in_room",
    "label_start",
    "label_end",
    "label_restart",
    "label_kick",
    "tab_owner",
    "owner_heading",
    "owner_lobby_note",
    "owner_pause_note",
    "owner_end_reason",
    "owner_restart_reason",
];

/// Parses the file. Every key is required and an unknown key is an error, so a typo never goes unnoticed.
pub fn parse(text: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let o = v.as_object().ok_or("hosted_room is not an object")?;
    if let Some(k) = o.keys().find(|k| !["schema", "_about", "poll", "strings"].contains(&k.as_str())) {
        return Err(format!("hosted_room: unknown key {k}"));
    }
    if o.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("hosted_room.schema must be {SCHEMA}"));
    }
    let poll = o.get("poll").and_then(Value::as_object).ok_or("hosted_room.poll is missing")?;
    if let Some(k) = poll.keys().find(|k| !["attempts", "interval_seconds"].contains(&k.as_str())) {
        return Err(format!("hosted_room.poll: unknown key {k}"));
    }
    let attempts = poll.get("attempts").and_then(Value::as_u64).filter(|n| (1..=600).contains(n)).ok_or("hosted_room.poll.attempts must be 1 to 600")?;
    let interval = poll
        .get("interval_seconds")
        .and_then(Value::as_f64)
        .filter(|n| (0.1..=30.0).contains(n))
        .ok_or("hosted_room.poll.interval_seconds must be 0.1 to 30")?;
    let strings = o.get("strings").and_then(Value::as_object).ok_or("hosted_room.strings is missing")?;
    if let Some(k) = strings.keys().find(|k| !STRING_KEYS.contains(&k.as_str())) {
        return Err(format!("hosted_room.strings: unknown key {k}"));
    }
    let get = |key: &str| -> Result<String, String> {
        strings
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| format!("hosted_room.strings.{key} must be non-empty text"))
    };
    Ok(Rules {
        poll_attempts: attempts as u32,
        poll_interval: Duration::from_secs_f64(interval),
        strings: Strings {
            sign_in_required: get("sign_in_required")?,
            offline: get("offline")?,
            creating: get("creating")?,
            unreachable: get("unreachable")?,
            create_failed: get("create_failed")?,
            waiting: get("waiting")?,
            waiting_check: get("waiting_check")?,
            not_started: get("not_started")?,
            opened_owner: get("opened_owner")?,
            lobby_owner_line: get("lobby_owner_line")?,
            lobby_owner_detail: get("lobby_owner_detail")?,
            lobby_waiting_line: get("lobby_waiting_line")?,
            owner_confirming: get("owner_confirming")?,
            owner_busy: get("owner_busy")?,
            owner_sign_in: get("owner_sign_in")?,
            owner_check_failed: get("owner_check_failed")?,
            owner_refused: get("owner_refused")?,
            owner_refused_default: get("owner_refused_default")?,
            owner_not_in_room: get("owner_not_in_room")?,
            label_start: get("label_start")?,
            label_end: get("label_end")?,
            label_restart: get("label_restart")?,
            label_kick: get("label_kick")?,
            tab_owner: get("tab_owner")?,
            owner_heading: get("owner_heading")?,
            owner_lobby_note: get("owner_lobby_note")?,
            owner_pause_note: get("owner_pause_note")?,
            owner_end_reason: get("owner_end_reason")?,
            owner_restart_reason: get("owner_restart_reason")?,
        },
    })
}

/// The embedded rules (`data/net/hosted_room.json`), read once. The file is compiled in and checked by a test.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| parse(include_str!("../data/net/hosted_room.json")).expect("data/net/hosted_room.json is valid"))
}

/// A line with `{code}`, `{n}`, `{max}` and `{reason}` filled in.
pub fn fill(template: &str, code: &str, n: u32, max: u32, reason: &str) -> String {
    template.replace("{code}", code).replace("{n}", &n.to_string()).replace("{max}", &max.to_string()).replace("{reason}", reason)
}

// ---- who sees the owner controls ---------------------------------------------------------------------------------

/// Whether the signed-in `account` is the owner of the room whose directory entry names `room_owner`. Account names are
/// upper case everywhere on the server; the comparison ignores case anyway.
pub fn is_room_owner(room_owner: &str, signed_in: bool, account: &str) -> bool {
    signed_in && !room_owner.trim().is_empty() && !account.trim().is_empty() && room_owner.trim().eq_ignore_ascii_case(account.trim())
}

/// The owner-controls rule: the controls are shown only to the room's owner, in an online room they joined as a guest.
/// Nobody else sees them: not another guest, not a signed-out player, not a solo room, and not the runner who hosts the
/// public route.
pub fn owner_controls_visible(online_guest: bool, room_owner: &str, signed_in: bool, account: &str) -> bool {
    online_guest && is_room_owner(room_owner, signed_in, account)
}

// ---- create ------------------------------------------------------------------------------------------------------

/// What the directory answers to a create (`202 {ok, code, region, relay, owner}`).
#[derive(Clone, Debug, PartialEq)]
pub struct Created {
    pub code: String,
    pub region: String,
    pub relay: String,
    /// The signed-in account that owns the room (upper case).
    pub owner: String,
}

/// The index the directory takes for a difficulty (Godot's `difficulty` argument; the region agent passes it on as
/// `--difficulty N`): its place in the lobby's row, RELAXED 0, STANDARD 1, HARD 2, NIGHTMARE 3.
pub fn difficulty_index(d: Difficulty) -> usize {
    LOBBY.iter().position(|x| *x == d).unwrap_or(1)
}

/// The difficulty a wire value names: an index or a name; None when it is neither.
pub fn difficulty_from_wire(v: &Value) -> Option<Difficulty> {
    match v {
        Value::Number(n) => n.as_u64().and_then(|i| LOBBY.get(i as usize).copied()),
        Value::String(t) => LOBBY
            .iter()
            .copied()
            .find(|d| d.id().eq_ignore_ascii_case(t.trim()))
            .or_else(|| t.trim().parse::<usize>().ok().and_then(|i| LOBBY.get(i).copied())),
        _ => None,
    }
}

/// The body of `POST /api/rooms/hosted/create`.
pub fn create_request(session_token: &str, difficulty: Difficulty) -> Value {
    json!({"sessionToken": session_token, "difficulty": difficulty_index(difficulty)})
}

fn text(o: &Json, key: &str) -> String {
    o.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// The create answer. A code that is not five letters or digits is no room.
pub fn parse_created(payload: &Json) -> Result<Created, NetError> {
    let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let code = text(payload, "code").to_uppercase();
    if !ok || !api::valid_join_code(&code) {
        return Err(NetError::Protocol(rules().strings.create_failed.clone()));
    }
    Ok(Created { code, region: text(payload, "region"), relay: text(payload, "relay"), owner: text(payload, "owner").to_uppercase() })
}

/// CREATE ROOM (online): `POST /api/rooms/hosted/create`. Blocking. Signed out is refused here without a request.
pub fn create(cfg: &BackendConfig, session_token: &str, difficulty: Difficulty) -> Result<Created, NetError> {
    if session_token.is_empty() {
        return Err(NetError::Refused(rules().strings.sign_in_required.clone()));
    }
    let payload = api::post(cfg, "/api/rooms/hosted/create", &create_request(session_token, difficulty))?;
    parse_created(&payload)
}

/// The line a failed create shows: signed out (401) asks to sign in, the server's own words for any other refusal, the
/// fixed lines for the rest. Always upper case.
pub fn create_error_text(e: &NetError) -> String {
    let s = &rules().strings;
    match e {
        NetError::Http { status: 401, .. } => s.sign_in_required.clone(),
        NetError::Http { error, .. } if !error.trim().is_empty() => error.trim().to_uppercase(),
        NetError::Offline => s.offline.clone(),
        NetError::Refused(t) if !t.trim().is_empty() => t.to_uppercase(),
        NetError::Protocol(t) if !t.trim().is_empty() => t.to_uppercase(),
        NetError::Http { .. } => s.create_failed.clone(),
        _ => s.unreachable.clone(),
    }
}

// ---- waiting for the room, then joining it -------------------------------------------------------------------------

/// What a failed look at the room's listing means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PollStep {
    /// Not listed yet (404), or the server hiccuped: look again.
    Retry,
    /// Something that will not get better by waiting (a protocol mismatch, a refusal): stop.
    Fail,
}

/// The next step after a failed join attempt. Godot keeps polling on any error; this keeps polling on the ones that
/// waiting can cure and stops at the ones that name a real refusal, so a BUILD MISMATCH is not retried 60 times.
pub fn poll_outcome(e: &NetError) -> PollStep {
    match e {
        NetError::Http { status, .. } if *status == 404 || *status >= 500 => PollStep::Retry,
        NetError::Transport(_) | NetError::Timeout => PollStep::Retry,
        _ => PollStep::Fail,
    }
}

/// What a [`HostedJoin`] reports after a call to [`HostedJoin::update`].
pub enum JoinProgress {
    /// Still waiting for the room to be listed.
    Waiting,
    /// The room was listed and this runner joined it as a guest.
    Joined(Box<NetSession>),
    /// The create is given up; the text is what the player sees.
    Failed(String),
}

/// The wait between CREATE ROOM and the guest join: one listing check at a time, [`Rules::poll_interval`] apart, at most
/// [`Rules::poll_attempts`] of them. The frame loop calls [`HostedJoin::update`] once per frame.
pub struct HostedJoin {
    pub code: String,
    pub owner: String,
    attempts_made: u32,
    max: u32,
    interval: Duration,
    next_at: Instant,
    pending: Option<Pending<NetSession>>,
}

impl HostedJoin {
    /// The first check is made at once.
    pub fn new(code: &str, owner: &str, now: Instant) -> HostedJoin {
        let r = rules();
        HostedJoin::with(code, owner, now, r.poll_attempts, r.poll_interval)
    }

    pub fn with(code: &str, owner: &str, now: Instant, max: u32, interval: Duration) -> HostedJoin {
        HostedJoin { code: code.to_string(), owner: owner.to_string(), attempts_made: 0, max, interval, next_at: now, pending: None }
    }

    /// How many checks have been started.
    pub fn checks(&self) -> u32 {
        self.attempts_made
    }

    pub fn max_checks(&self) -> u32 {
        self.max
    }

    /// The "WAITING FOR ROOM XXXXX TO START" line and the check counter under it.
    pub fn status_lines(&self) -> (String, String) {
        let s = &rules().strings;
        (fill(&s.waiting, &self.code, 0, 0, ""), fill(&s.waiting_check, &self.code, self.attempts_made.max(1), self.max, ""))
    }

    /// Starts a check when one is due (`attempt` makes the request) and collects the answer of the one in flight.
    pub fn update(&mut self, now: Instant, attempt: impl FnOnce() -> Pending<NetSession>) -> JoinProgress {
        if let Some(p) = self.pending.as_mut() {
            match p.poll() {
                None => return JoinProgress::Waiting,
                Some(Ok(session)) => {
                    self.pending = None;
                    return JoinProgress::Joined(Box::new(session));
                }
                Some(Err(e)) => {
                    self.pending = None;
                    if poll_outcome(&e) == PollStep::Fail {
                        return JoinProgress::Failed(e.message_or(&rules().strings.unreachable).to_uppercase());
                    }
                    if self.attempts_made >= self.max {
                        return JoinProgress::Failed(fill(&rules().strings.not_started, &self.code, 0, 0, ""));
                    }
                    self.next_at = now + self.interval;
                }
            }
        }
        if self.pending.is_none() && now >= self.next_at {
            self.attempts_made += 1;
            self.pending = Some(attempt());
        }
        JoinProgress::Waiting
    }
}

// ---- owner commands ----------------------------------------------------------------------------------------------

/// What the room's owner can ask the room to do.
#[derive(Clone, Debug, PartialEq)]
pub enum OwnerAction {
    /// START ROUTE: the route runs for everyone (nothing happens while it already runs).
    Start,
    /// END ROUTE: the route ends for everyone and the room waits in its lobby.
    End,
    /// RESTART ROUTE: a new route on a new seed, at the room's difficulty.
    Restart,
    /// KICK: removes the guest with this peer id from the room.
    Kick(u32),
    /// The room's difficulty; it applies from the next route.
    Difficulty(Difficulty),
}

impl OwnerAction {
    /// The action name on the wire and in the ticket check (Godot's `start`, `kick`, `end`, `difficulty`; `restart` is
    /// this port's name for the reserved-room command of the same name).
    pub fn wire_name(&self) -> &'static str {
        match self {
            OwnerAction::Start => "start",
            OwnerAction::End => "end",
            OwnerAction::Restart => "restart",
            OwnerAction::Kick(_) => "kick",
            OwnerAction::Difficulty(_) => "difficulty",
        }
    }

    /// The command's arguments, as Godot's `_owner_command` dictionary carries them: `{peer}` for a kick, `{value}`
    /// (the difficulty index) for a difficulty.
    pub fn args(&self) -> Map<String, Value> {
        let mut m = Map::new();
        match self {
            OwnerAction::Kick(peer) => {
                m.insert("peer".into(), json!(peer));
            }
            OwnerAction::Difficulty(d) => {
                m.insert("value".into(), json!(difficulty_index(*d)));
            }
            _ => {}
        }
        m
    }

    /// Reads an action off the wire. None for a name this build does not know or an argument that is missing.
    pub fn parse(action: &str, args: &Map<String, Value>) -> Option<OwnerAction> {
        match action {
            "start" => Some(OwnerAction::Start),
            "end" => Some(OwnerAction::End),
            // Godot's "reroll" (a new seed) is a restart here
            "restart" | "reroll" => Some(OwnerAction::Restart),
            "kick" => args.get("peer").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()).filter(|p| *p > 1).map(OwnerAction::Kick),
            "difficulty" => args.get("value").and_then(difficulty_from_wire).map(OwnerAction::Difficulty),
            _ => None,
        }
    }
}

/// An owner ticket (43 characters of base64url). It prints as `<redacted>`: it must never reach a log.
#[derive(Clone, PartialEq)]
pub struct OwnerTicket {
    value: String,
    pub expires_in: u32,
}

impl OwnerTicket {
    pub fn new(value: &str, expires_in: u32) -> OwnerTicket {
        OwnerTicket { value: value.to_string(), expires_in }
    }

    /// The ticket text, for the one message that carries it.
    pub fn reveal(&self) -> &str {
        &self.value
    }
}

impl std::fmt::Debug for OwnerTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OwnerTicket(<redacted>, expires_in {})", self.expires_in)
    }
}

/// Whether `ticket` has the shape of one the directory issues: 43 characters of base64url.
pub fn ticket_shape_ok(ticket: &str) -> bool {
    ticket.len() == TICKET_LEN && ticket.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The body of `POST /api/enet/owner-tickets/issue`.
pub fn issue_request(code: &str, session_token: &str) -> Value {
    json!({"code": code, "sessionToken": session_token})
}

/// The issue answer (`201 {ok, ticket, room, expiresIn}`): the ticket must have the right shape, and it lives at most
/// ten minutes (Godot clamps `expiresIn` to 1..600).
pub fn parse_ticket(payload: &Json) -> Result<OwnerTicket, NetError> {
    let ticket = text(payload, "ticket");
    if !ticket_shape_ok(&ticket) {
        return Err(NetError::Protocol("PRIMARY SERVER RETURNED AN INVALID OWNER TICKET".into()));
    }
    let expires = payload.get("expiresIn").and_then(Value::as_f64).unwrap_or(0.0).clamp(1.0, 600.0) as u32;
    Ok(OwnerTicket { value: ticket, expires_in: expires })
}

/// `POST /api/enet/owner-tickets/issue`: asks the directory for a ticket for this room. Blocking. Only the room's owner
/// gets one (403 `ROOM_OWNER_ONLY` for anybody else).
pub fn issue_owner_ticket(cfg: &BackendConfig, code: &str, session_token: &str) -> Result<OwnerTicket, NetError> {
    if session_token.is_empty() {
        return Err(NetError::Refused(rules().strings.owner_sign_in.clone()));
    }
    parse_ticket(&api::post(cfg, "/api/enet/owner-tickets/issue", &issue_request(code, session_token))?)
}

/// The line a failed ticket request shows: the directory's own words, else the fixed line.
pub fn owner_error_text(e: &NetError) -> String {
    let s = &rules().strings;
    match e {
        NetError::Http { status: 401, .. } => s.owner_sign_in.clone(),
        NetError::Http { error, .. } if !error.trim().is_empty() => error.trim().to_uppercase(),
        NetError::Refused(t) if !t.trim().is_empty() => t.to_uppercase(),
        _ => s.owner_check_failed.clone(),
    }
}

/// The body of `POST /api/enet/owner-tickets/verify` (the room process sends it with its host token).
pub fn verify_request(code: &str, ticket: &str, host_token: &str, action: &str) -> Value {
    json!({"code": code, "ticket": ticket, "hostToken": host_token, "action": action})
}

/// The verify answer: authorized only when the directory says so.
pub fn parse_verify(payload: &Json) -> Result<(), String> {
    if payload.get("authorized").and_then(Value::as_bool).unwrap_or(false) {
        Ok(())
    } else {
        Err(text(payload, "error"))
    }
}

/// The reason shown to an owner whose command the room refused: the directory's reason inside the fixed frame.
pub fn refusal_text(reason: &str) -> String {
    let s = &rules().strings;
    let reason = reason.trim();
    if reason.is_empty() {
        s.owner_refused_default.clone()
    } else {
        fill(&s.owner_refused, "", 0, 0, &reason.to_uppercase())
    }
}

/// Room side: asks the directory whether `ticket` is a live owner ticket for this room that has not yet run `action`.
/// Blocking. Err carries the reason to show the owner. A ticket of the wrong shape is refused without a request.
pub fn verify_owner_ticket(cfg: &BackendConfig, code: &str, ticket: &str, host_token: &str, action: &str) -> Result<(), String> {
    if !ticket_shape_ok(ticket) || action.is_empty() || action.len() > MAX_ACTION_LEN || host_token.is_empty() {
        return Err(refusal_text(""));
    }
    match api::post(cfg, "/api/enet/owner-tickets/verify", &verify_request(code, ticket, host_token, action)) {
        Ok(payload) => parse_verify(&payload).map_err(|reason| refusal_text(&reason)),
        Err(NetError::Http { error, .. }) => Err(refusal_text(&error)),
        Err(NetError::Offline) => Err(refusal_text("")),
        Err(_) => Err(refusal_text("COULD NOT REACH THE PRIMARY SERVER")),
    }
}

/// What the commander wants done next.
#[derive(Debug, PartialEq)]
pub enum OwnerStep {
    /// Nothing to do.
    Idle,
    /// Show this line (a refusal, or the confirming note).
    Say(String),
    /// Send this command to the room now.
    Send { ticket: OwnerTicket, action: OwnerAction },
}

/// The owner's side of a command: asks the directory for a ticket, then hands the command and the ticket on to be sent
/// to the room. One command waits for its ticket at a time; a second is refused, not queued (Godot `owner_send`).
#[derive(Default)]
pub struct OwnerCommander {
    waiting: Option<OwnerAction>,
    request: Option<Pending<OwnerTicket>>,
}

impl OwnerCommander {
    /// Takes a command. Ok is the confirming line; Err is the busy line (nothing was taken).
    pub fn submit(&mut self, action: OwnerAction) -> Result<String, String> {
        let s = &rules().strings;
        if self.waiting.is_some() {
            return Err(s.owner_busy.clone());
        }
        self.waiting = Some(action);
        Ok(s.owner_confirming.clone())
    }

    pub fn busy(&self) -> bool {
        self.waiting.is_some()
    }

    /// Drops a command that is waiting (the room was left, the player signed out).
    pub fn cancel(&mut self) {
        self.waiting = None;
        self.request = None;
    }

    /// Once per frame. `issue` starts the ticket request; it is called once per command.
    pub fn update(&mut self, issue: impl FnOnce() -> Pending<OwnerTicket>) -> OwnerStep {
        if self.waiting.is_none() {
            self.request = None;
            return OwnerStep::Idle;
        }
        if self.request.is_none() {
            self.request = Some(issue());
        }
        let Some(done) = self.request.as_mut().and_then(|p| p.poll()) else { return OwnerStep::Idle };
        self.request = None;
        let action = self.waiting.take();
        match (done, action) {
            (Ok(ticket), Some(action)) => OwnerStep::Send { ticket, action },
            (Err(e), _) => OwnerStep::Say(owner_error_text(&e)),
            (Ok(_), None) => OwnerStep::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Json {
        v.as_object().cloned().unwrap()
    }

    const TICKET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";

    #[test]
    fn the_embedded_file_parses_and_names_godots_poll() {
        let r = rules();
        assert_eq!(r.poll_attempts, 60);
        assert_eq!(r.poll_interval, Duration::from_secs(2));
        assert!(r.strings.sign_in_required.contains("SIGN IN"));
        // every string key of the file is read (STRING_KEYS is the contract)
        let v: Value = serde_json::from_str(include_str!("../data/net/hosted_room.json")).unwrap();
        assert_eq!(v["strings"].as_object().unwrap().len(), STRING_KEYS.len());
    }

    #[test]
    fn a_broken_file_is_refused_with_the_reason() {
        let ok = include_str!("../data/net/hosted_room.json");
        assert!(parse(ok).is_ok());
        assert!(parse(&ok.replace("lr.hosted_room/1", "lr.hosted_room/2")).unwrap_err().contains("schema"));
        assert!(parse(&ok.replace("\"attempts\": 60", "\"attempts\": 0")).unwrap_err().contains("attempts"));
        assert!(parse(&ok.replace("\"label_kick\"", "\"label_boot\"")).unwrap_err().contains("label_boot"));
        assert!(parse(&ok.replace("\"waiting\": \"WAITING FOR ROOM {code} TO START\"", "\"waiting\": \"\"")).unwrap_err().contains("waiting"));
        assert!(parse("[]").is_err());
    }

    #[test]
    fn lines_fill_in_their_fields() {
        let s = &rules().strings;
        assert_eq!(fill(&s.waiting, "ABCDE", 0, 0, ""), "WAITING FOR ROOM ABCDE TO START");
        assert_eq!(fill(&s.waiting_check, "ABCDE", 3, 60, ""), "SERVER CHECK 3 OF 60");
        assert_eq!(refusal_text("invalid or expired owner ticket"), "ROOM OWNER REFUSED: INVALID OR EXPIRED OWNER TICKET");
        assert_eq!(refusal_text("  "), s.owner_refused_default);
    }

    #[test]
    fn the_create_request_has_the_godot_shape() {
        assert_eq!(create_request("tok", Difficulty::Standard), json!({"sessionToken": "tok", "difficulty": 1}));
        assert_eq!(create_request("tok", Difficulty::Relaxed)["difficulty"], json!(0));
        assert_eq!(create_request("tok", Difficulty::Nightmare)["difficulty"], json!(3));
    }

    #[test]
    fn the_create_answer_is_read_and_a_bad_one_is_refused() {
        let c = parse_created(&obj(json!({"ok": true, "code": "k3m9x", "region": "virginia", "relay": "wss://x", "owner": "kyttu"}))).unwrap();
        assert_eq!(c, Created { code: "K3M9X".into(), region: "virginia".into(), relay: "wss://x".into(), owner: "KYTTU".into() });
        for bad in [json!({"ok": false, "code": "K3M9X"}), json!({"ok": true}), json!({"ok": true, "code": "TOOLONG"}), json!({"code": "K3M9X"}), json!({})] {
            assert!(parse_created(&obj(bad.clone())).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_signed_out_create_makes_no_request_and_says_to_sign_in() {
        let cfg = BackendConfig::default();
        let e = create(&cfg, "", Difficulty::Standard).unwrap_err();
        assert_eq!(create_error_text(&e), rules().strings.sign_in_required);
        // the directory's own 401 says the same
        let e = NetError::Http { status: 401, code: "SIGN_IN_REQUIRED".into(), error: "Sign in to host a room".into(), body: Value::Null };
        assert_eq!(create_error_text(&e), rules().strings.sign_in_required);
    }

    #[test]
    fn create_errors_show_the_servers_words_in_upper_case() {
        let busy = NetError::Http { status: 503, code: "NO_FREE_REGION".into(), error: "No server has a free room slot right now".into(), body: Value::Null };
        assert_eq!(create_error_text(&busy), "NO SERVER HAS A FREE ROOM SLOT RIGHT NOW");
        let bare = NetError::Http { status: 500, code: String::new(), error: String::new(), body: Value::Null };
        assert_eq!(create_error_text(&bare), rules().strings.create_failed);
        assert_eq!(create_error_text(&NetError::Timeout), rules().strings.unreachable);
        assert_eq!(create_error_text(&NetError::Transport("refused".into())), rules().strings.unreachable);
        assert_eq!(create_error_text(&NetError::Offline), rules().strings.offline);
    }

    #[test]
    fn the_ticket_request_and_answer_have_the_godot_shapes() {
        assert_eq!(issue_request("ABCDE", "tok"), json!({"code": "ABCDE", "sessionToken": "tok"}));
        let t = parse_ticket(&obj(json!({"ok": true, "ticket": TICKET, "room": "ABCDE", "expiresIn": 300}))).unwrap();
        assert_eq!((t.reveal(), t.expires_in), (TICKET, 300));
        // expiresIn is clamped to 1..600 like Godot
        assert_eq!(parse_ticket(&obj(json!({"ticket": TICKET, "expiresIn": 99999}))).unwrap().expires_in, 600);
        assert_eq!(parse_ticket(&obj(json!({"ticket": TICKET}))).unwrap().expires_in, 1);
        // a ticket of the wrong length or alphabet is refused
        for bad in ["", "short", &TICKET[..42], &format!("{TICKET}x"), &format!("{}!", &TICKET[..42])] {
            assert!(parse_ticket(&obj(json!({"ticket": bad, "expiresIn": 5}))).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_ticket_never_prints() {
        let t = OwnerTicket::new(TICKET, 300);
        let shown = format!("{t:?} {:?}", OwnerStep::Send { ticket: t.clone(), action: OwnerAction::Start });
        assert!(!shown.contains(TICKET) && shown.contains("<redacted>"), "{shown}");
    }

    #[test]
    fn the_verify_request_and_answer_have_the_godot_shapes() {
        assert_eq!(
            verify_request("ABCDE", TICKET, "hosttok", "kick"),
            json!({"code": "ABCDE", "ticket": TICKET, "hostToken": "hosttok", "action": "kick"})
        );
        assert!(parse_verify(&obj(json!({"ok": true, "authorized": true, "room": "ABCDE", "owner": "KYTTU", "action": "kick"}))).is_ok());
        assert_eq!(parse_verify(&obj(json!({"ok": false, "authorized": false, "error": "OWNER TICKET ALREADY USED FOR THIS ACTION"}))).unwrap_err(), "OWNER TICKET ALREADY USED FOR THIS ACTION");
        assert_eq!(parse_verify(&obj(json!({"ok": true}))).unwrap_err(), "");
        // a bad ticket, an empty action or no host token is refused before any request (the config is offline-proof here)
        let cfg = BackendConfig::default();
        for (ticket, token, action) in [("short", "tok", "start"), (TICKET, "", "start"), (TICKET, "tok", ""), (TICKET, "tok", &"x".repeat(33))] {
            assert_eq!(verify_owner_ticket(&cfg, "ABCDE", ticket, token, action).unwrap_err(), rules().strings.owner_refused_default);
        }
    }

    #[test]
    fn actions_round_trip_through_their_wire_form() {
        for action in [OwnerAction::Start, OwnerAction::End, OwnerAction::Restart, OwnerAction::Kick(7), OwnerAction::Difficulty(Difficulty::Hard), OwnerAction::Difficulty(Difficulty::Relaxed)] {
            assert_eq!(OwnerAction::parse(action.wire_name(), &action.args()), Some(action.clone()), "{action:?}");
            assert!(action.wire_name().len() <= MAX_ACTION_LEN);
        }
        // Godot's argument shapes
        assert_eq!(OwnerAction::Kick(7).args(), obj(json!({"peer": 7})));
        assert_eq!(OwnerAction::Difficulty(Difficulty::Hard).args(), obj(json!({"value": 2})));
        assert!(OwnerAction::Start.args().is_empty());
        // a difficulty may arrive as a name too; Godot's reroll is a restart
        assert_eq!(OwnerAction::parse("difficulty", &obj(json!({"value": "nightmare"}))), Some(OwnerAction::Difficulty(Difficulty::Nightmare)));
        assert_eq!(OwnerAction::parse("reroll", &Map::new()), Some(OwnerAction::Restart));
    }

    #[test]
    fn bad_actions_are_not_actions() {
        let none = Map::new();
        assert_eq!(OwnerAction::parse("kick", &none), None, "a kick names a peer");
        assert_eq!(OwnerAction::parse("kick", &obj(json!({"peer": "x"}))), None);
        assert_eq!(OwnerAction::parse("kick", &obj(json!({"peer": 1}))), None, "the host is peer 1 and is never kicked");
        assert_eq!(OwnerAction::parse("kick", &obj(json!({"peer": 0}))), None);
        assert_eq!(OwnerAction::parse("kick", &obj(json!({"peer": -3}))), None);
        assert_eq!(OwnerAction::parse("difficulty", &obj(json!({"value": 9}))), None);
        assert_eq!(OwnerAction::parse("difficulty", &none), None);
        assert_eq!(OwnerAction::parse("transfer", &none), None);
        assert_eq!(OwnerAction::parse("", &none), None);
    }

    #[test]
    fn only_the_rooms_owner_sees_the_controls() {
        // the owner, signed in, a guest of an online room
        assert!(owner_controls_visible(true, "KYTTU", true, "KYTTU"));
        assert!(owner_controls_visible(true, "KYTTU", true, "kyttu"), "case is ignored");
        // another guest
        assert!(!owner_controls_visible(true, "KYTTU", true, "AYTTU"));
        // signed out, even with the owner's name left in the settings
        assert!(!owner_controls_visible(true, "KYTTU", false, "KYTTU"));
        // a room with no owner of record (a browser host's room, the public route)
        assert!(!owner_controls_visible(true, "", true, ""));
        assert!(!owner_controls_visible(true, "", true, "KYTTU"));
        assert!(!owner_controls_visible(true, "KYTTU", true, ""));
        // a solo room or a hosting client is not an online guest
        assert!(!owner_controls_visible(false, "KYTTU", true, "KYTTU"));
        assert!(!is_room_owner("  ", true, "  "));
    }

    fn miss() -> NetError {
        NetError::Http { status: 404, code: String::new(), error: "ROOM NOT FOUND".into(), body: Value::Null }
    }

    #[test]
    fn waiting_codes_retry_and_real_refusals_stop() {
        assert_eq!(poll_outcome(&miss()), PollStep::Retry);
        assert_eq!(poll_outcome(&NetError::Http { status: 502, code: String::new(), error: String::new(), body: Value::Null }), PollStep::Retry);
        assert_eq!(poll_outcome(&NetError::Timeout), PollStep::Retry);
        assert_eq!(poll_outcome(&NetError::Transport("x".into())), PollStep::Retry);
        assert_eq!(poll_outcome(&NetError::Refused("BUILD MISMATCH".into())), PollStep::Fail);
        assert_eq!(poll_outcome(&NetError::Http { status: 403, code: "ROOM_RESERVED".into(), error: String::new(), body: Value::Null }), PollStep::Fail);
        assert_eq!(poll_outcome(&NetError::Offline), PollStep::Fail);
    }

    #[test]
    fn the_wait_checks_at_once_then_on_the_interval_and_gives_up_after_the_last_check() {
        let t0 = Instant::now();
        let mut join = HostedJoin::with("ABCDE", "KYTTU", t0, 3, Duration::from_secs(2));
        let mut started = 0;
        let ask = |started: &mut u32| {
            *started += 1;
            Pending::<NetSession>::done(Err(miss()))
        };
        // check 1 starts at once, its answer (a miss) comes on the next frame
        assert!(matches!(join.update(t0, || ask(&mut started)), JoinProgress::Waiting));
        assert_eq!((join.checks(), started), (1, 1));
        // the miss arrives: nothing new starts before the interval is up
        assert!(matches!(join.update(t0 + Duration::from_millis(16), || ask(&mut started)), JoinProgress::Waiting));
        assert!(matches!(join.update(t0 + Duration::from_millis(1900), || ask(&mut started)), JoinProgress::Waiting));
        assert_eq!(started, 1);
        // 2 s after the answer the next check starts
        assert!(matches!(join.update(t0 + Duration::from_millis(2100), || ask(&mut started)), JoinProgress::Waiting));
        assert_eq!(started, 2);
        // its miss, the third check, its miss: given up with the room's line
        let mut at = t0 + Duration::from_millis(2200);
        let mut outcome = None;
        for _ in 0..10 {
            match join.update(at, || ask(&mut started)) {
                JoinProgress::Failed(t) => {
                    outcome = Some(t);
                    break;
                }
                _ => at = at + Duration::from_secs(3),
            }
        }
        assert_eq!(started, 3, "never more than the configured checks");
        assert_eq!(outcome.as_deref(), Some("ROOM ABCDE DID NOT START // TRY CREATE ROOM AGAIN"));
    }

    #[test]
    fn a_refusal_ends_the_wait_at_once_with_its_words() {
        let t0 = Instant::now();
        let mut join = HostedJoin::with("ABCDE", "KYTTU", t0, 60, Duration::from_secs(2));
        let mut first = true;
        let mut out = None;
        for i in 0..4 {
            let r = join.update(t0 + Duration::from_millis(i * 20), || {
                first = false;
                Pending::<NetSession>::done(Err(NetError::Refused("BUILD MISMATCH // UPDATE THE GAME".into())))
            });
            if let JoinProgress::Failed(t) = r {
                out = Some(t);
                break;
            }
        }
        assert!(!first);
        assert_eq!(out.as_deref(), Some("BUILD MISMATCH // UPDATE THE GAME"));
        assert_eq!(join.checks(), 1);
    }

    #[test]
    fn the_wait_lines_name_the_room_and_count_the_checks() {
        let t0 = Instant::now();
        let mut join = HostedJoin::with("ABCDE", "KYTTU", t0, 60, Duration::from_secs(2));
        assert_eq!(join.status_lines(), ("WAITING FOR ROOM ABCDE TO START".into(), "SERVER CHECK 1 OF 60".into()));
        let _ = join.update(t0, || Pending::<NetSession>::done(Err(miss())));
        assert_eq!(join.status_lines().1, "SERVER CHECK 1 OF 60");
    }

    #[test]
    fn one_owner_command_waits_for_its_ticket_at_a_time() {
        let mut c = OwnerCommander::default();
        assert_eq!(c.update(|| panic!("nothing is waiting")), OwnerStep::Idle);
        assert_eq!(c.submit(OwnerAction::End), Ok(rules().strings.owner_confirming.clone()));
        // a second one is refused, not queued
        assert_eq!(c.submit(OwnerAction::Start), Err(rules().strings.owner_busy.clone()));
        assert!(c.busy());
        // the ticket request starts once and its answer sends the command
        let mut asked = 0;
        let step = c.update(|| {
            asked += 1;
            Pending::done(Ok(OwnerTicket::new(TICKET, 300)))
        });
        assert_eq!(step, OwnerStep::Send { ticket: OwnerTicket::new(TICKET, 300), action: OwnerAction::End });
        assert_eq!(asked, 1);
        assert!(!c.busy());
        assert_eq!(c.update(|| panic!("done")), OwnerStep::Idle);
        // and a new command is accepted again
        assert!(c.submit(OwnerAction::Restart).is_ok());
    }

    #[test]
    fn a_refused_ticket_says_why_and_frees_the_commander() {
        let mut c = OwnerCommander::default();
        c.submit(OwnerAction::Kick(5)).unwrap();
        let refusal = NetError::Http { status: 403, code: "ROOM_OWNER_ONLY".into(), error: "Only the room's owner can control it".into(), body: Value::Null };
        let step = c.update(|| Pending::done(Err(refusal)));
        assert_eq!(step, OwnerStep::Say("ONLY THE ROOM'S OWNER CAN CONTROL IT".into()));
        assert!(!c.busy());
        // an unreachable server uses the fixed line
        c.submit(OwnerAction::End).unwrap();
        assert_eq!(c.update(|| Pending::done(Err(NetError::Timeout))), OwnerStep::Say(rules().strings.owner_check_failed.clone()));
    }

    #[test]
    fn leaving_the_room_drops_a_waiting_command() {
        let mut c = OwnerCommander::default();
        c.submit(OwnerAction::End).unwrap();
        let _ = c.update(|| Pending::<OwnerTicket>::spawn(|| {
            std::thread::sleep(Duration::from_millis(20));
            Ok(OwnerTicket::new(TICKET, 1))
        }));
        c.cancel();
        assert!(!c.busy());
        assert_eq!(c.update(|| panic!("cancelled")), OwnerStep::Idle);
    }
}

/// The owner's messages over a real WebSocket relay (the in-process test hub): what a guest sends, what the room's session
/// makes of it.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod wire_tests {
    use super::*;
    use crate::net::world_sync::NetEvent;
    use crate::net::ws_native::NativeWsLink;
    use crate::net::ws_test_hub::TestHub;

    const TICKET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";

    fn pair(hub: &TestHub) -> (NetSession, NetSession) {
        hub.add_room("ABCDE", "tok");
        let host = NetSession::dedicated_host(
            BackendConfig::default(),
            "ABCDE".into(),
            Box::new(NativeWsLink::host("ABCDE", "tok", &hub.url()).unwrap()),
            "tok".into(),
            "ROOM-ABCDE-X".into(),
            "SERVER".into(),
            false,
        );
        let guest = NetSession::guest_over(BackendConfig::default(), "ABCDE".into(), Box::new(NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap()), "OWNER".into(), "DEFAULT".into());
        (host, guest)
    }

    /// Ticks both sessions until `done` says so; returns every event each of them raised.
    fn pump(host: &mut NetSession, guest: &mut NetSession, mut done: impl FnMut(&[NetEvent], &[NetEvent]) -> bool) -> (Vec<NetEvent>, Vec<NetEvent>) {
        let (mut h, mut g) = (Vec::new(), Vec::new());
        let end = std::time::Instant::now() + Duration::from_secs(20);
        while std::time::Instant::now() < end {
            h.extend(host.tick(None));
            g.extend(guest.tick(None));
            if done(&h, &g) {
                return (h, g);
            }
            std::thread::sleep(Duration::from_millis(3));
        }
        panic!("timed out; host saw {h:?}, guest saw {g:?}");
    }

    fn registered(host: &mut NetSession, guest: &mut NetSession) {
        pump(host, guest, |h, _| h.iter().any(|e| matches!(e, NetEvent::GuestJoined(_))));
    }

    #[test]
    fn a_registered_guests_owner_command_reaches_the_room_with_its_ticket_and_arguments() {
        let hub = TestHub::start();
        let (mut host, mut guest) = pair(&hub);
        registered(&mut host, &mut guest);
        let peer = host.roster_peers()[0].0;
        assert!(guest.send_owner_command(TICKET, &OwnerAction::Kick(peer)));
        let (h, _) = pump(&mut host, &mut guest, |h, _| h.iter().any(|e| matches!(e, NetEvent::OwnerCommand { .. })));
        let got = h.into_iter().find_map(|e| if let NetEvent::OwnerCommand { peer, ticket, action, args } = e { Some((peer, ticket, action, args)) } else { None }).unwrap();
        assert_eq!((got.0, got.1.as_str(), got.2.as_str()), (peer, TICKET, "kick"));
        assert_eq!(OwnerAction::parse(&got.2, &got.3), Some(OwnerAction::Kick(peer)));
    }

    #[test]
    fn a_hosting_session_cannot_send_an_owner_command_and_a_guest_cannot_refuse() {
        let hub = TestHub::start();
        let (mut host, mut guest) = pair(&hub);
        registered(&mut host, &mut guest);
        assert!(!host.send_owner_command(TICKET, &OwnerAction::End), "the room is the one that checks, it sends none");
        assert!(!guest.send_owner_refused(1, "NO"), "only the room refuses");
    }

    #[test]
    fn an_oversized_ticket_is_dropped_and_the_next_command_still_arrives() {
        let hub = TestHub::start();
        let (mut host, mut guest) = pair(&hub);
        registered(&mut host, &mut guest);
        guest.send_owner_command(&"x".repeat(200), &OwnerAction::Start);
        guest.send_owner_command(TICKET, &OwnerAction::End);
        let (h, _) = pump(&mut host, &mut guest, |h, _| h.iter().any(|e| matches!(e, NetEvent::OwnerCommand { .. })));
        let commands: Vec<&NetEvent> = h.iter().filter(|e| matches!(e, NetEvent::OwnerCommand { .. })).collect();
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert!(matches!(commands[0], NetEvent::OwnerCommand { action, .. } if action == "end"));
    }

    #[test]
    fn the_rooms_refusal_reaches_the_guest_that_asked() {
        let hub = TestHub::start();
        let (mut host, mut guest) = pair(&hub);
        registered(&mut host, &mut guest);
        let peer = host.roster_peers()[0].0;
        assert!(host.send_owner_refused(peer, "ROOM OWNER REFUSED: INVALID OR EXPIRED OWNER TICKET"));
        let (_, g) = pump(&mut host, &mut guest, |_, g| g.iter().any(|e| matches!(e, NetEvent::OwnerRefused(_))));
        assert!(g.contains(&NetEvent::OwnerRefused("ROOM OWNER REFUSED: INVALID OR EXPIRED OWNER TICKET".into())));
    }

    #[test]
    fn the_room_names_every_guest_by_session_id_to_the_others() {
        let hub = TestHub::start();
        let (mut host, mut guest) = pair(&hub);
        registered(&mut host, &mut guest);
        // the guest sees the room's host, and (once a second guest is in) the others with their ids; alone it sees nobody else
        assert!(guest.roster_peers().is_empty());
        let mut second = NetSession::guest_over(BackendConfig::default(), "ABCDE".into(), Box::new(NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap()), "SECOND".into(), "DEFAULT".into());
        let end = std::time::Instant::now() + Duration::from_secs(20);
        while guest.roster_peers().is_empty() && std::time::Instant::now() < end {
            host.tick(None);
            guest.tick(None);
            second.tick(None);
            std::thread::sleep(Duration::from_millis(3));
        }
        let seen = guest.roster_peers();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert_eq!(seen[0].1, "SECOND");
        assert_eq!(seen[0].0, host.roster_peers().iter().find(|(_, n)| n == "SECOND").unwrap().0);
        // the owner of record comes from the directory entry, not from the room: a bare guest session has none
        assert_eq!(guest.room_owner(), "");
    }

    #[test]
    fn kicking_a_guest_closes_it_with_the_removed_reason_and_forgets_it() {
        let hub = TestHub::start();
        let (mut host, mut guest) = pair(&hub);
        registered(&mut host, &mut guest);
        let peer = host.roster_peers()[0].0;
        assert!(!host.kick_guest(peer + 50), "nobody has that id");
        assert!(host.kick_guest(peer));
        assert_eq!(host.guest_count(), 0);
        let (_, g) = pump(&mut host, &mut guest, |_, g| g.iter().any(|e| matches!(e, NetEvent::HostLeft(_))));
        assert!(g.contains(&NetEvent::HostLeft("YOU WERE REMOVED FROM THE ROOM".into())), "{g:?}");
        assert!(!guest.kick_guest(peer), "only the room removes a guest");
    }
}
