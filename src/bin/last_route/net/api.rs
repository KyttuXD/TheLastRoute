//! One blocking function per server route the Godot client calls, with the request bodies and
//! response shapes of `backend_client.gd` / `run_saves.gd` and the server (`backend/server.mjs`).
//! [`super::Backend`] runs these on worker threads; tests call them directly.
//!
//! Every request carries `Accept: application/json` and `Content-Type: application/json`
//! (`_headers()`); the room lookup and health check send only `Accept: application/json`, the
//! guest-name call only `content-type: application/json`, the cloud saves add
//! `X-Session-Token: <token>` — exactly as the original.
//!
//! A non-2xx answer becomes [`NetError::Http`] with the server's `{code, error}`; callers map that
//! to the original's player-facing strings (kept next to each call in [`super::backend`]).

use super::config::{self, BackendConfig};
use super::http::{Method, Request, Response};
use super::NetError;
use serde_json::{json, Map, Value};
use std::time::Duration;

pub type Json = Map<String, Value>;

fn s(map: &Json, key: &str) -> String {
    match map.get(key) {
        Some(Value::String(v)) => v.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn i(map: &Json, key: &str) -> i64 {
    match map.get(key) {
        Some(Value::Number(n)) => n.as_i64().unwrap_or_else(|| n.as_f64().unwrap_or(0.0) as i64),
        Some(Value::Bool(b)) => *b as i64,
        Some(Value::String(t)) => t.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn b(map: &Json, key: &str) -> bool {
    match map.get(key) {
        Some(Value::Bool(v)) => *v,
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(t)) => !t.is_empty(),
        _ => false,
    }
}

fn obj(map: &Json, key: &str) -> Option<Json> {
    match map.get(key) {
        Some(Value::Object(o)) => Some(o.clone()),
        _ => None,
    }
}

/// `_headers()`.
fn json_request(method: Method, url: String) -> Request {
    Request::new(method, url).header("Accept", "application/json").header("Content-Type", "application/json")
}

/// Turns a response into the JSON object on 2xx, or [`NetError::Http`].
pub fn check(response: Response) -> Result<Json, NetError> {
    let payload = response.json();
    if response.ok() {
        Ok(payload)
    } else {
        Err(NetError::Http { status: response.status, code: s(&payload, "code"), error: s(&payload, "error"), body: Value::Object(payload) })
    }
}

fn guard(cfg: &BackendConfig) -> Result<(), NetError> {
    if cfg.offline {
        Err(NetError::Offline)
    } else {
        Ok(())
    }
}

/// POST `path` with a JSON body through `_headers()`.
pub fn post(cfg: &BackendConfig, path: &str, body: &Value) -> Result<Json, NetError> {
    post_with_timeout(cfg, path, body, super::http::DEFAULT_TIMEOUT)
}

pub fn post_with_timeout(cfg: &BackendConfig, path: &str, body: &Value, timeout: Duration) -> Result<Json, NetError> {
    guard(cfg)?;
    check(json_request(Method::Post, cfg.url(path)).json_body(body).timeout(timeout).send()?)
}

// ---- skins --------------------------------------------------------------------------------------

/// The largest PNG the website takes for a skin (`skins.mjs` SKIN_MAX_BYTES: 64 KB; the one number lives in the shared
/// validator, so the client and `lr_server` cannot drift apart).
pub const SKIN_MAX_BYTES: usize = crate::shared::skin_validate::SKIN_MAX_BYTES;

/// The body of `POST /api/v1/skins`: the session token and the PNG as standard base64.
pub fn skin_upload_body(session_token: &str, png: &[u8]) -> Value {
    json!({"sessionToken": session_token, "png": super::base64_encode(png)})
}

/// `POST /api/v1/skins {sessionToken, png}`. The website answers `{ok, id}` with the id it computed (the SHA-256 of the PNG).
pub fn upload_skin(cfg: &BackendConfig, session_token: &str, png: &[u8]) -> Result<String, NetError> {
    // The server refuses these too; not sending them saves the round trip and the rate-limit slot.
    if let Err(why) = crate::shared::skin_validate::check_skin_png(png, SKIN_MAX_BYTES) {
        return Err(NetError::Refused(why));
    }
    let p = match post_with_timeout(cfg, "/api/v1/skins", &skin_upload_body(session_token, png), Duration::from_secs(10)) {
        Ok(p) => p,
        // A server without the v1 route may still have the Godot one (lr_server keeps both: `POST /api/skins/upload`, raw PNG).
        Err(e) if is_unknown_endpoint(&e) => upload_skin_legacy(cfg, session_token, png)?,
        Err(e) => return Err(e),
    };
    Ok(s(&p, "id"))
}

/// True for a 404 that means the route does not exist (lr_server's `UNKNOWN_ENDPOINT`, or a bare 404 with no JSON body),
/// not one the route itself answered (an account that was not found).
pub fn is_unknown_endpoint(error: &NetError) -> bool {
    matches!(error, NetError::Http { status: 404, code, error, .. } if code == "UNKNOWN_ENDPOINT" || (code.is_empty() && error.is_empty()))
}

/// `POST /api/skins/upload`: the PNG as the raw body, the token in `x-session-token` (the Godot backend route).
fn upload_skin_legacy(cfg: &BackendConfig, session_token: &str, png: &[u8]) -> Result<Json, NetError> {
    guard(cfg)?;
    let mut request = Request::new(Method::Post, cfg.url("/api/skins/upload"))
        .header("Accept", "application/json")
        .header("Content-Type", "image/png")
        .header("x-session-token", session_token)
        .timeout(Duration::from_secs(10));
    request.body = Some(png.to_vec());
    check(request.send()?)
}

/// `POST /api/v1/skins/clear {sessionToken}`: the account's stored skin is removed (a player chose DEFAULT; Godot
/// `SkinSync.clear_own`). The website answers `{ok}`.
pub fn clear_skin(cfg: &BackendConfig, session_token: &str) -> Result<(), NetError> {
    post_with_timeout(cfg, "/api/v1/skins/clear", &json!({"sessionToken": session_token}), Duration::from_secs(10)).map(|_| ())
}

// ---- health -------------------------------------------------------------------------------------

/// `GET /api/health` (5 s). `Ok(true)` only for 200 with `ok: true` ("DIRECTORY ONLINE").
pub fn health(cfg: &BackendConfig) -> Result<Json, NetError> {
    guard(cfg)?;
    let r = Request::new(Method::Get, cfg.url("/api/health"))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(config::HEALTH_TIMEOUT_SECS))
        .send()?;
    let status = r.status;
    let payload = check(r)?;
    if status == 200 && b(&payload, "ok") {
        Ok(payload)
    } else {
        Err(NetError::Protocol("DIRECTORY UNAVAILABLE".into()))
    }
}

// ---- update manifest ------------------------------------------------------------------------------

/// The updater's own timeout for its check (`app/updater.rs`).
pub const UPDATE_CHECK_TIMEOUT_SECS: u64 = 5;

/// The newest build published for a platform (`public_update_manifest` in server.mjs).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublishedBuild {
    pub build: i64,
    /// The commit the build was made from (the server lists the short hash).
    pub revision: String,
}

/// The update answer. `channel` is empty when the reply names none; `update` is `None` when nothing is
/// published for the platform.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateManifest {
    pub channel: String,
    pub update: Option<PublishedBuild>,
}

/// `GET /api/updates/debug/<platform>` on the primary origin, the request the updater makes on start. The
/// server answers 404 `NO UPDATE PUBLISHED` when nothing is published for the platform: the server was reached,
/// so that is an answer, not an error.
pub fn update_manifest(platform: &str) -> Result<UpdateManifest, NetError> {
    let r = Request::new(Method::Get, format!("{}/api/updates/debug/{platform}", config::PRIMARY_BASE_URL))
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(UPDATE_CHECK_TIMEOUT_SECS))
        .send()?;
    let p = match check(r) {
        Ok(p) => p,
        Err(NetError::Http { status: 404, error, .. }) if error == "NO UPDATE PUBLISHED" => return Ok(UpdateManifest::default()),
        Err(e) => return Err(e),
    };
    Ok(UpdateManifest {
        channel: s(&p, "channel"),
        update: obj(&p, "update").map(|u| PublishedBuild { build: i(&u, "build"), revision: s(&u, "revision") }),
    })
}

// ---- accounts -----------------------------------------------------------------------------------

/// The register / login / session / rename answer (`accountResult` in account_service.mjs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountReply {
    pub username: String,
    pub session_token: String,
    pub admin: bool,
    /// `adminOwner` was present in the answer (the legacy-owner migration needs to know).
    pub has_admin_owner: bool,
    pub admin_owner: bool,
    /// `adminPermissions` was present in the answer.
    pub has_admin_permissions: bool,
    pub admin_permissions: Vec<String>,
    /// The account progress document (`progress`), when it is an object.
    pub progress: Option<Json>,
    pub saved_at: i64,
    /// Issued to an owner account on register / login (server.mjs); the client does not use it.
    pub dev_token: String,
}

impl AccountReply {
    pub fn from_json(p: &Json) -> AccountReply {
        let perms = match p.get("adminPermissions") {
            Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
            _ => Vec::new(),
        };
        AccountReply {
            username: s(p, "username").trim().to_uppercase(),
            session_token: s(p, "sessionToken"),
            admin: b(p, "admin"),
            has_admin_owner: p.contains_key("adminOwner"),
            admin_owner: b(p, "adminOwner"),
            has_admin_permissions: p.contains_key("adminPermissions"),
            admin_permissions: perms,
            progress: obj(p, "progress"),
            saved_at: i(p, "savedAt"),
            dev_token: s(p, "devToken"),
        }
    }
}

/// `_send_credentials`: `{username: UPPERCASE trimmed, password, adminProof?}`.
pub fn credentials_body(username: &str, password: &str, admin_proof: &str) -> Value {
    let mut body = json!({"username": username.trim().to_uppercase(), "password": password});
    if !admin_proof.is_empty() {
        body["adminProof"] = Value::String(admin_proof.to_string());
    }
    body
}

/// `POST /api/account/register` — `admin_proof` is the ADMIN SETUP KEY field, only needed (and
/// only shown) for the reserved owner names KYTTU / AYTTU.
pub fn register(cfg: &BackendConfig, username: &str, password: &str, admin_proof: &str) -> Result<AccountReply, NetError> {
    post(cfg, "/api/account/register", &credentials_body(username, password, admin_proof)).map(|p| AccountReply::from_json(&p))
}

/// `POST /api/account/login`.
pub fn login(cfg: &BackendConfig, username: &str, password: &str) -> Result<AccountReply, NetError> {
    post(cfg, "/api/account/login", &credentials_body(username, password, "")).map(|p| AccountReply::from_json(&p))
}

/// `POST /api/account/session {sessionToken}`: restores a stored login.
pub fn session(cfg: &BackendConfig, token: &str) -> Result<AccountReply, NetError> {
    post(cfg, "/api/account/session", &json!({"sessionToken": token})).map(|p| AccountReply::from_json(&p))
}

/// `POST /api/account/logout {sessionToken}` (always 200).
pub fn logout(cfg: &BackendConfig, token: &str) -> Result<(), NetError> {
    post(cfg, "/api/account/logout", &json!({"sessionToken": token})).map(|_| ())
}

/// `POST /api/account/password {sessionToken, password}` -> `{ok, username}`. Other devices' sessions
/// are revoked; this one stays.
pub fn change_password(cfg: &BackendConfig, token: &str, password: &str) -> Result<String, NetError> {
    post(cfg, "/api/account/password", &json!({"sessionToken": token, "password": password})).map(|p| s(&p, "username"))
}

/// `POST /api/account/rename {sessionToken, username: UPPERCASE, adminProof?}`.
pub fn rename(cfg: &BackendConfig, token: &str, username: &str, admin_proof: &str) -> Result<AccountReply, NetError> {
    let mut body = json!({"sessionToken": token, "username": username.trim().to_uppercase()});
    if !admin_proof.is_empty() {
        body["adminProof"] = Value::String(admin_proof.to_string());
    }
    post(cfg, "/api/account/rename", &body).map(|p| AccountReply::from_json(&p))
}

/// The legacy one-document account progress (`/api/account/save|load`): `{progress, savedAt}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProgressReply {
    pub progress: Option<Json>,
    /// Server milliseconds (`now()` in account_service.mjs).
    pub saved_at: i64,
}

/// `POST /api/account/save {sessionToken, progress}` (`progress: null` clears; 32 KB cap).
pub fn save_progress(cfg: &BackendConfig, token: &str, progress: Option<&Json>) -> Result<ProgressReply, NetError> {
    let progress = progress.map(|p| Value::Object(p.clone())).unwrap_or(Value::Null);
    post(cfg, "/api/account/save", &json!({"sessionToken": token, "progress": progress}))
        .map(|p| ProgressReply { progress: obj(&p, "progress"), saved_at: i(&p, "savedAt") })
}

/// `POST /api/account/load {sessionToken}`.
pub fn load_progress(cfg: &BackendConfig, token: &str) -> Result<ProgressReply, NetError> {
    post(cfg, "/api/account/load", &json!({"sessionToken": token}))
        .map(|p| ProgressReply { progress: obj(&p, "progress"), saved_at: i(&p, "savedAt") })
}

// ---- admin roster (owner accounts) ---------------------------------------------------------------

/// One roster row: `{username, admin, adminOwner, adminPermissions}`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RosterEntry {
    pub username: String,
    pub admin_owner: bool,
    pub admin_permissions: Vec<String>,
}

fn roster_entry(v: &Value) -> Option<RosterEntry> {
    let o = v.as_object()?;
    let username = s(o, "username");
    if username.is_empty() {
        return None;
    }
    let perms = o.get("adminPermissions").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Some(RosterEntry { username, admin_owner: b(o, "adminOwner"), admin_permissions: perms })
}

/// `POST /api/admin/roster {sessionToken}`.
pub fn admin_roster(cfg: &BackendConfig, token: &str) -> Result<Vec<RosterEntry>, NetError> {
    let p = post(cfg, "/api/admin/roster", &json!({"sessionToken": token}))?;
    Ok(p.get("roster").and_then(|r| r.as_array()).map(|a| a.iter().filter_map(roster_entry).collect()).unwrap_or_default())
}

/// `POST /api/admin/roster/set {sessionToken, username, permissions}` -> the updated entry.
pub fn admin_roster_set(cfg: &BackendConfig, token: &str, username: &str, permissions: &[String]) -> Result<RosterEntry, NetError> {
    let p = post(cfg, "/api/admin/roster/set", &json!({"sessionToken": token, "username": username, "permissions": permissions}))?;
    p.get("entry").and_then(roster_entry).ok_or_else(|| NetError::Protocol("NO ROSTER ENTRY".into()))
}

/// `POST /api/admin/roster/remove {sessionToken, username}` -> the removed username.
pub fn admin_roster_remove(cfg: &BackendConfig, token: &str, username: &str) -> Result<String, NetError> {
    post(cfg, "/api/admin/roster/remove", &json!({"sessionToken": token, "username": username})).map(|p| s(&p, "username"))
}

/// What an admin password reset answered. `temporary_password` is the one-time password the server generated (None
/// when the admin typed the password themselves). The password is never shown by `Debug`.
#[derive(Clone, Default, PartialEq)]
pub struct PasswordReset {
    pub username: String,
    pub temporary_password: Option<String>,
    pub sessions_revoked: i64,
}

impl std::fmt::Debug for PasswordReset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordReset")
            .field("username", &self.username)
            .field("temporary_password", &self.temporary_password.as_ref().map(|_| "[hidden]"))
            .field("sessions_revoked", &self.sessions_revoked)
            .finish()
    }
}

/// The request body of `POST /api/admin/account/reset-password`: no `newPassword` key unless one was typed (the server
/// then generates a temporary one).
pub fn admin_reset_password_body(token: &str, username: &str, new_password: Option<&str>) -> Value {
    let mut body = json!({"sessionToken": token, "username": username.trim().to_uppercase()});
    if let Some(password) = new_password.filter(|p| !p.is_empty()) {
        body["newPassword"] = Value::String(password.to_string());
    }
    body
}

/// `POST /api/admin/account/reset-password {sessionToken, username, newPassword?}`: a full admin owner sets another
/// account's password; every session of that account ends. Without `newPassword` the answer carries the generated
/// temporary password, once.
pub fn admin_reset_password(cfg: &BackendConfig, token: &str, username: &str, new_password: Option<&str>) -> Result<PasswordReset, NetError> {
    let p = post(cfg, "/api/admin/account/reset-password", &admin_reset_password_body(token, username, new_password))?;
    Ok(PasswordReset {
        username: s(&p, "username"),
        temporary_password: Some(s(&p, "temporaryPassword")).filter(|t| !t.is_empty()),
        sessions_revoked: i(&p, "sessionsRevoked"),
    })
}

// ---- guest names ------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuestName {
    pub name: String,
    pub token: String,
    pub renewed: bool,
}

/// `POST /api/guest/name {previous, token}` (8 s; header `content-type: application/json` only).
/// The original applies the answer only for a 200.
pub fn guest_name(cfg: &BackendConfig, previous: &str, token: &str) -> Result<GuestName, NetError> {
    guard(cfg)?;
    let r = Request::new(Method::Post, cfg.url("/api/guest/name"))
        .header("content-type", "application/json")
        .json_body(&json!({"previous": previous, "token": token}))
        .timeout(Duration::from_secs(config::GUEST_NAME_TIMEOUT_SECS))
        .send()?;
    if r.status != 200 {
        return Err(check(r).err().unwrap_or(NetError::Protocol("GUEST NAME NOT ISSUED".into())));
    }
    let p = r.json();
    Ok(GuestName { name: s(&p, "name"), token: s(&p, "token"), renewed: b(&p, "renewed") })
}

/// `GuestNames.valid`: one letter (no I/O), a dash, four digits 2-9.
pub fn valid_guest_name(name: &str) -> bool {
    const LETTERS: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGITS: &str = "23456789";
    let c: Vec<char> = name.chars().collect();
    c.len() == 6 && LETTERS.contains(c[0]) && c[1] == '-' && c[2..].iter().all(|d| DIGITS.contains(*d))
}

/// `GuestNames.random_name`: the local name used until the server answers (and offline).
pub fn random_guest_name() -> String {
    const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGITS: &[u8] = b"23456789";
    let mut name = String::new();
    name.push(LETTERS[super::random_below(LETTERS.len())] as char);
    name.push('-');
    for _ in 0..4 {
        name.push(DIGITS[super::random_below(DIGITS.len())] as char);
    }
    name
}

// ---- room directory ------------------------------------------------------------------------------------

/// `CODE_ALPHABET` (room_state.gd): read aloud and typed in, so no I/1, O/0, lowercase.
pub const CODE_ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const CODE_LENGTH: usize = 5;
pub const MAX_PLAYERS: u32 = 16;
/// ENet port a host opens (`RoomState.DEFAULT_PORT`); sent as `port` on register.
pub const DEFAULT_ENET_PORT: u16 = 8421;
/// The persistent server-owned route's code.
pub const PUBLIC_CODE: &str = "PUBLC";
pub const GAME_LAST_ROUTE: &str = "last_route";

/// `RoomState._make_code`.
pub fn make_room_code() -> String {
    let alpha = CODE_ALPHABET.as_bytes();
    (0..CODE_LENGTH).map(|_| alpha[super::random_below(alpha.len())] as char).collect()
}

/// `RoomState.valid_room_code`: a generated code (the only kind a non-reserved host may register).
pub fn valid_room_code(value: &str) -> bool {
    let clean = value.trim().to_uppercase();
    clean.chars().count() == CODE_LENGTH && clean.chars().all(|c| CODE_ALPHABET.contains(c))
}

/// `RoomState.valid_join_code`: also an owner's reserved code (any five letters/digits, KATHY).
pub fn valid_join_code(value: &str) -> bool {
    let clean = value.trim().to_uppercase();
    clean.chars().count() == CODE_LENGTH && clean.chars().all(|c| CODE_ALPHABET.contains(c) || "IO01".contains(c))
}

/// An owner's JOIN ROOM text that is not a code (`parse_join_code` refused it) turned into a valid room code: the
/// text is uppercased, every character outside `CODE_ALPHABET` is dropped, the first `CODE_LENGTH` characters are
/// kept and any shortfall is padded with `random` (which yields characters of `CODE_ALPHABET`). Empty or all-invalid
/// text gives a fully random code. The result always passes [`valid_room_code`].
pub fn admin_code_from_input(typed: &str, mut random: impl FnMut() -> char) -> String {
    let mut code: String = typed.to_uppercase().chars().filter(|c| CODE_ALPHABET.contains(*c)).take(CODE_LENGTH).collect();
    while code.chars().count() < CODE_LENGTH {
        code.push(random());
    }
    code
}

/// A room as the directory publishes it (`publicEnetRoom` in server.mjs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoomEndpoint {
    pub code: String,
    pub game: String,
    pub players: i64,
    pub session_active: bool,
    pub daytime: bool,
    /// The host build's protocol fingerprint ("" from older builds).
    pub protocol: String,
    pub dedicated: bool,
    pub region: String,
    pub expires_in: i64,
    /// "relay" (the gameplay relay), "direct" (relay disabled on the server) or "ws".
    pub transport: String,
    pub address: String,
    pub port: u16,
    pub ws_path: String,
    /// The account that created a hosted room (upper case); empty for a room with no owner of record.
    pub owner: String,
}

impl RoomEndpoint {
    pub fn from_json(o: &Json) -> RoomEndpoint {
        RoomEndpoint {
            code: s(o, "code"),
            game: s(o, "game"),
            players: i(o, "players"),
            session_active: b(o, "sessionActive"),
            daytime: b(o, "daytime"),
            protocol: s(o, "protocol"),
            dedicated: b(o, "dedicated"),
            region: s(o, "region"),
            expires_in: i(o, "expiresIn"),
            transport: s(o, "transport"),
            address: s(o, "address"),
            port: i(o, "port").clamp(0, 65535) as u16,
            ws_path: s(o, "wsPath"),
            owner: s(o, "owner").to_uppercase(),
        }
    }
}

/// One owner command for a reserved room (`{seq, command, args, by, at}`).
#[derive(Clone, Debug, PartialEq)]
pub struct ReservedCommand {
    pub seq: i64,
    /// "restart" | "end" | "kick" | "settings" | "transfer".
    pub command: String,
    pub args: Json,
    pub by: String,
    pub at: String,
}

/// `hostView` (reserved_rooms.mjs): the stored settings and commands not acked yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReservedHostView {
    /// `{seed, mode, scale, difficulty, daytime, mods}` (any subset).
    pub settings: Json,
    pub commands: Vec<ReservedCommand>,
    pub command_seq: i64,
}

impl ReservedHostView {
    pub fn from_json(o: &Json) -> ReservedHostView {
        let commands = o
            .get("commands")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_object())
                    .map(|c| ReservedCommand { seq: i(c, "seq"), command: s(c, "command"), args: obj(c, "args").unwrap_or_default(), by: s(c, "by"), at: s(c, "at") })
                    .collect()
            })
            .unwrap_or_default();
        ReservedHostView { settings: obj(o, "settings").unwrap_or_default(), commands, command_seq: i(o, "commandSeq") }
    }
}

/// `register_room`'s body.
#[derive(Clone, Debug, PartialEq)]
pub struct RegisterRoom {
    pub code: String,
    /// The host's ENet port (0 for a "ws" room).
    pub port: u16,
    /// Clamped 1..16.
    pub players: u32,
    pub session_active: bool,
    pub daytime: bool,
    /// `RoomState.game_id()`: "last_route".
    pub game: String,
    pub session_token: String,
    /// `ModerationClient.device_id()`.
    pub device_id: String,
    /// Only a reserved code needs it.
    pub room_password: String,
    /// This build's protocol fingerprint (8 hex chars).
    pub protocol: String,
    pub spawn_ticket: String,
    pub region: String,
    /// "enet" or "ws".
    pub transport: String,
}

impl Default for RegisterRoom {
    fn default() -> Self {
        RegisterRoom {
            code: String::new(),
            port: DEFAULT_ENET_PORT,
            players: 1,
            session_active: false,
            daytime: false,
            game: GAME_LAST_ROUTE.into(),
            session_token: String::new(),
            device_id: String::new(),
            room_password: String::new(),
            protocol: super::protocol::fingerprint(),
            spawn_ticket: String::new(),
            region: String::new(),
            // the browser build hosts over the WebSocket relay (Godot web: ws_room_peer.gd); native keeps the UDP relay
            transport: if crate::shared::task::IN_BROWSER { "ws" } else { "enet" }.into(),
        }
    }
}

impl RegisterRoom {
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "code": self.code.to_uppercase(),
            "port": self.port,
            "players": self.players.clamp(1, MAX_PLAYERS),
            "sessionActive": self.session_active,
            "daytime": self.daytime,
            "game": self.game,
            "sessionToken": self.session_token,
            "deviceId": self.device_id,
            "roomPassword": self.room_password,
            "protocol": self.protocol,
            "spawnTicket": self.spawn_ticket,
            "region": self.region,
        });
        if self.transport == "ws" {
            v["transport"] = Value::String("ws".into());
        }
        v
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Registered {
    /// The host token (relay HOST_REGISTER / HOST_ALIVE payload, heartbeat auth).
    pub token: String,
    pub room: RoomEndpoint,
    pub reserved: Option<ReservedHostView>,
}

/// `POST /api/enet/rooms/register` -> 201 `{ok, token, room, reserved?}`.
pub fn register_room(cfg: &BackendConfig, req: &RegisterRoom) -> Result<Registered, NetError> {
    let p = post(cfg, "/api/enet/rooms/register", &req.to_json())?;
    Ok(Registered {
        token: s(&p, "token"),
        room: obj(&p, "room").map(|r| RoomEndpoint::from_json(&r)).unwrap_or_default(),
        reserved: obj(&p, "reserved").map(|r| ReservedHostView::from_json(&r)),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Heartbeat {
    pub code: String,
    pub token: String,
    pub players: u32,
    pub session_active: bool,
    pub daytime: bool,
    pub command_since: i64,
    pub command_ack: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeartbeatReply {
    pub room: RoomEndpoint,
    pub reserved: Option<ReservedHostView>,
}

/// `POST /api/enet/rooms/heartbeat` (every 20 s). 404 = the directory forgot the room (restart),
/// 403 = the token is not this room's.
pub fn heartbeat(cfg: &BackendConfig, h: &Heartbeat) -> Result<HeartbeatReply, NetError> {
    let p = post_with_timeout(
        cfg,
        "/api/enet/rooms/heartbeat",
        &json!({
            "code": h.code, "token": h.token, "players": h.players.clamp(1, MAX_PLAYERS),
            "sessionActive": h.session_active, "daytime": h.daytime,
            "commandSince": h.command_since, "commandAck": h.command_ack,
        }),
        Duration::from_secs(super::public_route::HEARTBEAT_TIMEOUT_SECONDS),
    )?;
    Ok(HeartbeatReply {
        room: obj(&p, "room").map(|r| RoomEndpoint::from_json(&r)).unwrap_or_default(),
        reserved: obj(&p, "reserved").map(|r| ReservedHostView::from_json(&r)),
    })
}

/// `POST /api/enet/rooms/unregister {code, token}` -> `removed` (an explicit quit: guests are told
/// HOST_LEFT now and the code stops resolving now).
pub fn unregister_room(cfg: &BackendConfig, code: &str, token: &str) -> Result<bool, NetError> {
    post(cfg, "/api/enet/rooms/unregister", &json!({"code": code, "token": token})).map(|p| b(&p, "removed"))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseReply {
    pub migrating: bool,
    pub error: String,
    pub unsupported: bool,
}

/// `POST /api/enet/rooms/release-host {code, token, handoff?: {version: 1, candidateSession, epoch}}`.
pub fn release_host(cfg: &BackendConfig, code: &str, token: &str, handoff: Option<(u32, i64)>) -> Result<ReleaseReply, NetError> {
    let mut body = json!({"code": code, "token": token});
    let timeout = if let Some((session, epoch)) = handoff {
        body["handoff"] = json!({"version": 1, "candidateSession": session, "epoch": epoch});
        Duration::from_secs(config::RELEASE_HANDOFF_TIMEOUT_SECS)
    } else {
        super::http::DEFAULT_TIMEOUT
    };
    let p = post_with_timeout(cfg, "/api/enet/rooms/release-host", &body, timeout)?;
    Ok(ReleaseReply { migrating: b(&p, "migrating"), error: s(&p, "error"), unsupported: b(&p, "unsupported") })
}

/// `GET /api/enet/rooms/:code` (header `Accept: application/json` only). 403 `ROOM_RESERVED` for
/// a reserved code, 404 `ROOM NOT FOUND`.
pub fn lookup_room(cfg: &BackendConfig, code: &str) -> Result<RoomEndpoint, NetError> {
    guard(cfg)?;
    let r = Request::new(Method::Get, cfg.url(&format!("/api/enet/rooms/{code}"))).header("Accept", "application/json").send()?;
    if r.status != 200 {
        return Err(check(r).err().unwrap_or(NetError::Protocol("ROOM LOOKUP FAILED".into())));
    }
    let p = r.json();
    obj(&p, "room").map(|o| RoomEndpoint::from_json(&o)).ok_or_else(|| NetError::Protocol("DIRECTORY RETURNED AN INVALID ENDPOINT".into()))
}

/// `POST /api/rooms/reserved/resolve {code, sessionToken, password}` answer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReservedResolve {
    pub live: bool,
    pub room: Option<RoomEndpoint>,
    /// base64url, 18 bytes: becomes the relay GUEST_JOIN nonce.
    pub join_ticket: String,
    pub can_host: bool,
    pub settings: Json,
}

pub fn resolve_reserved(cfg: &BackendConfig, code: &str, session_token: &str, password: &str) -> Result<ReservedResolve, NetError> {
    guard(cfg)?;
    let r = json_request(Method::Post, cfg.url("/api/rooms/reserved/resolve"))
        .json_body(&json!({"code": code, "sessionToken": session_token, "password": password}))
        .send()?;
    if r.status != 200 {
        return Err(check(r).err().unwrap_or(NetError::Protocol("PRIVATE ROOM LOOKUP FAILED".into())));
    }
    let p = r.json();
    Ok(ReservedResolve {
        live: b(&p, "live"),
        room: obj(&p, "room").map(|o| RoomEndpoint::from_json(&o)),
        join_ticket: s(&p, "joinTicket"),
        can_host: b(&p, "canHost"),
        settings: obj(&p, "settings").unwrap_or_default(),
    })
}

/// `NetProtocol.mismatch`: "" when the host speaks this build's protocol.
pub fn protocol_mismatch(host_protocol: &str, mine: &str) -> String {
    if host_protocol == mine {
        String::new()
    } else if host_protocol.is_empty() {
        "BUILD MISMATCH // HOST UNVERSIONED // UPDATE".into()
    } else {
        format!("BUILD MISMATCH // HOST {} YOU {} // UPDATE", host_protocol.to_uppercase(), mine.to_uppercase())
    }
}

/// What looking up a typed room code ends in (`resolve_room` + `_resolve_reserved`).
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    /// Join it. `join_ticket` is set for a reserved room (used once as the relay nonce).
    Found { room: RoomEndpoint, join_ticket: String },
    /// A reserved room nobody hosts and this player may host, with its stored settings
    /// (`reserved_room_hostable`). A reconnect lookup gets `Failed("PRIVATE ROOM HAS NO LIVE HOST")`.
    Hostable { settings: Json },
    /// A reserved room that wants an invite or its password (`reserved_room_access_needed`).
    AccessNeeded { message: String },
    /// Anything else, with the message the JOIN dialog shows.
    Failed { message: String },
}

/// The whole JOIN ROOM lookup, blocking: `GET /api/enet/rooms/:code`, and for a reserved code the
/// access-checked resolve with this account's token and the typed room password. `reconnect` is the
/// original's `done` callback path (a guest reconnecting never becomes a reserved host).
pub fn resolve_room(cfg: &BackendConfig, code: &str, session_token: &str, room_password: &str, mine: &str, reconnect: bool) -> Resolution {
    let clean = code.trim().to_uppercase();
    if cfg.offline {
        return Resolution::Failed { message: config::OFFLINE_JOIN_MESSAGE.into() };
    }
    match lookup_room(cfg, &clean) {
        Ok(room) => {
            if room.address.is_empty() && room.transport != "ws" {
                return Resolution::Failed { message: "DIRECTORY RETURNED AN INVALID ENDPOINT".into() };
            }
            let refused = protocol_mismatch(&room.protocol, mine);
            if !refused.is_empty() {
                return Resolution::Failed { message: refused };
            }
            Resolution::Found { room, join_ticket: String::new() }
        }
        Err(NetError::Http { status: 403, code: c, .. }) if c == "ROOM_RESERVED" => {
            match resolve_reserved(cfg, &clean, session_token, room_password) {
                Err(e) => {
                    let message = e.message_or(&format!("PRIVATE ROOM {clean} // ACCESS REQUIRED"));
                    if reconnect {
                        Resolution::Failed { message }
                    } else {
                        Resolution::AccessNeeded { message }
                    }
                }
                Ok(r) if r.can_host => {
                    if reconnect {
                        Resolution::Failed { message: "PRIVATE ROOM HAS NO LIVE HOST".into() }
                    } else {
                        Resolution::Hostable { settings: r.settings }
                    }
                }
                Ok(r) => match r.room {
                    None => Resolution::Failed { message: "PRIVATE ROOM RETURNED NO ENDPOINT".into() },
                    Some(room) => {
                        let refused = protocol_mismatch(&room.protocol, mine);
                        if !refused.is_empty() {
                            Resolution::Failed { message: refused }
                        } else {
                            Resolution::Found { room, join_ticket: r.join_ticket }
                        }
                    }
                },
            }
        }
        Err(NetError::Protocol(m)) if m == "DIRECTORY RETURNED AN INVALID ENDPOINT" => Resolution::Failed { message: m },
        Err(e) => Resolution::Failed { message: e.message_or(&format!("ROOM {clean} WAS NOT FOUND")) },
    }
}

/// The body of an owner's JOIN ROOM reserve request (`POST /api/admin/reserved-rooms`).
pub fn admin_reserve_body(code: &str, session_token: &str) -> Value {
    json!({"action": "create", "code": code, "sessionToken": session_token})
}

/// Whether a failed JOIN ROOM lookup may reserve the typed code for this player: only an owner's
/// own lookup that found nothing (404) or hit a code already reserved (403 `ROOM_RESERVED`, which
/// the owner hosts the same way: `admin_reserve_room` answers `ROOM_ALREADY_RESERVED` as ok). Guests,
/// other failures and a code already live keep the `ROOM NOT FOUND` / join behaviour.
pub fn admin_may_reserve_on_miss(is_admin: bool, session_token: &str, err: &NetError) -> bool {
    let miss = err.status() == Some(404)
        || matches!(err, NetError::Http { status: 403, code, .. } if code == "ROOM_RESERVED");
    is_admin && !session_token.is_empty() && miss
}

/// Reserves `code` for the owner before she hosts it. Ok when the code is reserved for her now
/// (created here, or already reserved). A code someone is hosting comes back as the
/// `ROOM_ALREADY_HOSTED` error so the caller joins that room instead of recreating it.
pub fn admin_reserve_room(cfg: &BackendConfig, code: &str, session_token: &str) -> Result<(), NetError> {
    guard(cfg)?;
    let r = json_request(Method::Post, cfg.url("/api/admin/reserved-rooms")).json_body(&admin_reserve_body(code, session_token)).send()?;
    match check(r) {
        Ok(_) => Ok(()),
        Err(NetError::Http { code: c, .. }) if c == "ROOM_ALREADY_RESERVED" => Ok(()),
        Err(e) => Err(e),
    }
}

// ---- reserved-room owner tools --------------------------------------------------------------------------

/// `POST /api/admin/reserved-rooms {sessionToken, action: "command", code, command, ...args}` -> 202.
pub fn reserved_command(cfg: &BackendConfig, token: &str, code: &str, command: &str, args: &Json) -> Result<Json, NetError> {
    let mut body = json!({"sessionToken": token, "action": "command", "code": code, "command": command});
    for (k, v) in args {
        body[k] = v.clone();
    }
    post(cfg, "/api/admin/reserved-rooms", &body)
}

/// `POST /api/admin/reserved-rooms {action: "whitelist", code, op, name}` -> the whitelist.
pub fn reserved_whitelist(cfg: &BackendConfig, token: &str, code: &str, op: &str, name: &str) -> Result<Vec<String>, NetError> {
    let p = post(cfg, "/api/admin/reserved-rooms", &json!({"sessionToken": token, "action": "whitelist", "code": code, "op": op, "name": name}))?;
    Ok(p.get("whitelist").and_then(|w| w.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default())
}

// ---- admin room proofs ---------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdminTicket {
    /// 43 chars (32 random bytes, base64url).
    pub ticket: String,
    pub room: String,
    pub permissions: Vec<String>,
    /// Seconds, clamped 1..300 by the client.
    pub expires_in: i64,
}

/// `POST /api/enet/admin-tickets/issue {code, sessionToken}` -> 201.
pub fn admin_ticket_issue(cfg: &BackendConfig, code: &str, token: &str) -> Result<AdminTicket, NetError> {
    let p = post(cfg, "/api/enet/admin-tickets/issue", &json!({"code": code, "sessionToken": token}))?;
    let perms = p.get("permissions").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Ok(AdminTicket { ticket: s(&p, "ticket"), room: s(&p, "room").trim().to_uppercase(), permissions: perms, expires_in: i(&p, "expiresIn").clamp(1, 300) })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AdminVerify {
    pub authorized: bool,
    pub permissions: Vec<String>,
    pub owner: bool,
    pub expires_in: i64,
}

/// `POST /api/enet/admin-tickets/verify {code, ticket, hostToken}` (host only).
pub fn admin_ticket_verify(cfg: &BackendConfig, code: &str, ticket: &str, host_token: &str) -> Result<AdminVerify, NetError> {
    let p = post(cfg, "/api/enet/admin-tickets/verify", &json!({"code": code, "ticket": ticket, "hostToken": host_token}))?;
    let perms = p.get("permissions").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    Ok(AdminVerify { authorized: b(&p, "authorized"), permissions: perms, owner: b(&p, "owner"), expires_in: i(&p, "expiresIn").clamp(0, 300) })
}

// ---- cloud saves ---------------------------------------------------------------------------------------

/// This game's own cloud save id on the website. The Godot game keeps `last-route-godot`. `CLOUD_SLOT` and the
/// world revision suffix match run_saves.gd.
pub const CLOUD_GAME_ID: &str = "last-route-rust";
pub const CLOUD_SLOT: &str = "run";

/// `_cloud_path()`: `/api/v1/saves/last-route-rust/run-world-<REVISION>`.
pub fn cloud_path(world_revision: i64) -> String {
    format!("/api/v1/saves/{CLOUD_GAME_ID}/{CLOUD_SLOT}-world-{world_revision}")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CloudRecord {
    /// The stored record (`null` -> `None`).
    pub data: Option<Json>,
    pub updated_at: i64,
}

fn cloud_request(cfg: &BackendConfig, method: Method, path: &str, token: &str) -> Request {
    json_request(method, cfg.url(path)).header("X-Session-Token", token)
}

/// `GET /api/v1/saves/:gameId/:slot` with `X-Session-Token`.
pub fn cloud_get(cfg: &BackendConfig, path: &str, token: &str) -> Result<CloudRecord, NetError> {
    guard(cfg)?;
    let p = check(cloud_request(cfg, Method::Get, path, token).send()?)?;
    Ok(CloudRecord { data: obj(&p, "data"), updated_at: i(&p, "updatedAt") })
}

/// How a PUT ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloudPut {
    Stored { updated_at: i64 },
    /// 409 STALE: the account holds a newer record (`updatedAt` of the stored one).
    Stale { stored_updated_at: i64 },
}

/// `PUT /api/v1/saves/:gameId/:slot {data, updatedAt}` (`data: None` deletes the slot).
pub fn cloud_put(cfg: &BackendConfig, path: &str, token: &str, data: Option<&Json>, updated_at: i64) -> Result<CloudPut, NetError> {
    guard(cfg)?;
    let data = data.map(|d| Value::Object(d.clone())).unwrap_or(Value::Null);
    let r = cloud_request(cfg, Method::Put, path, token).json_body(&json!({"data": data, "updatedAt": updated_at})).send()?;
    if r.status == 409 {
        let p = r.json();
        if s(&p, "code") == "STALE" || p.is_empty() {
            return Ok(CloudPut::Stale { stored_updated_at: i(&p, "updatedAt") });
        }
    }
    let p = check(r)?;
    Ok(CloudPut::Stored { updated_at: i(&p, "updatedAt") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_unknown_route_404_falls_back_to_the_legacy_upload() {
        let http = |status, code: &str, error: &str| NetError::Http { status, code: code.into(), error: error.into(), body: Value::Null };
        assert!(is_unknown_endpoint(&http(404, "UNKNOWN_ENDPOINT", "Unknown endpoint")));
        assert!(is_unknown_endpoint(&http(404, "", "")));
        assert!(!is_unknown_endpoint(&http(404, "ACCOUNT_NOT_FOUND", "no such account")));
        assert!(!is_unknown_endpoint(&http(404, "", "ACCOUNT NOT FOUND")));
        assert!(!is_unknown_endpoint(&http(500, "UNKNOWN_ENDPOINT", "")));
    }

    #[test]
    fn room_codes() {
        for _ in 0..50 {
            let c = make_room_code();
            assert!(valid_room_code(&c), "{c}");
        }
        assert!(!valid_room_code("KATH1"));
        assert!(valid_join_code("kathy"));
        assert!(valid_join_code("AKIRI"));
        assert!(valid_join_code("O0I1A"));
        assert!(!valid_join_code("ABCD"));
        assert!(!valid_join_code("AB-CD"));
    }

    #[test]
    fn guest_names() {
        // (the backend README's example "R-4821" contains a 1, which the alphabet excludes: [2-9]{4})
        assert!(valid_guest_name("R-4823"));
        assert!(!valid_guest_name("R-4821"));
        assert!(!valid_guest_name("I-4821"));
        assert!(!valid_guest_name("R-4101"));
        for _ in 0..20 {
            assert!(valid_guest_name(&random_guest_name()));
        }
    }

    #[test]
    fn skin_upload_request_body() {
        let b = skin_upload_body("tok", b"abc");
        assert_eq!(b, json!({"sessionToken": "tok", "png": "YWJj"}));
        assert_eq!(SKIN_MAX_BYTES, 64 * 1024, "skins.mjs SKIN_MAX_BYTES");
    }

    #[test]
    fn bodies() {
        let b = credentials_body(" kyttu ", "pw", "");
        assert_eq!(b, json!({"username": "KYTTU", "password": "pw"}));
        let b = credentials_body("kyttu", "pw", "setup");
        assert_eq!(b["adminProof"], "setup");
        let r = RegisterRoom { code: "abcde".into(), players: 40, protocol: "0011aabb".into(), ..Default::default() };
        let v = r.to_json();
        assert_eq!(v["code"], "ABCDE");
        assert_eq!(v["players"], 16);
        assert_eq!(v["port"], 8421);
        assert!(v.get("transport").is_none());
        assert_eq!(cloud_path(1), "/api/v1/saves/last-route-rust/run-world-1");
    }

    #[test]
    fn mismatch_text() {
        assert_eq!(protocol_mismatch("abc", "abc"), "");
        assert_eq!(protocol_mismatch("", "abc"), "BUILD MISMATCH // HOST UNVERSIONED // UPDATE");
        assert_eq!(protocol_mismatch("0a0b0c0d", "11223344"), "BUILD MISMATCH // HOST 0A0B0C0D YOU 11223344 // UPDATE");
    }

    #[test]
    fn account_reply_parsing() {
        let p: Json = serde_json::from_str(r#"{"ok":true,"username":"kyttu","admin":true,"adminOwner":true,"adminPermissions":["hunters","world"],"sessionToken":"t","progress":{"schema":1},"savedAt":5}"#).unwrap();
        let a = AccountReply::from_json(&p);
        assert_eq!(a.username, "KYTTU");
        assert!(a.admin_owner && a.has_admin_owner && a.has_admin_permissions);
        assert_eq!(a.admin_permissions, vec!["hunters", "world"]);
        assert_eq!(a.saved_at, 5);
        assert!(a.progress.is_some());
    }
}

#[cfg(test)]
mod admin_join_tests {
    use super::*;

    #[test]
    fn admin_invalid_codes_become_valid_room_codes() {
        // a deterministic stand-in for the random source: cycles through the alphabet, one call per padded slot
        fn cycle() -> impl FnMut() -> char {
            let mut i = 0usize;
            move || {
                let c = CODE_ALPHABET.as_bytes()[i % CODE_ALPHABET.len()] as char;
                i += 1;
                c
            }
        }
        let out = admin_code_from_input("ab-c", cycle());
        assert_eq!(out.chars().count(), CODE_LENGTH);
        assert!(out.starts_with("ABC"), "{out}");
        assert!(valid_room_code(&out), "{out}");

        let mut calls = 0;
        let out = admin_code_from_input("IIIII", || {
            calls += 1;
            'K'
        });
        assert_eq!(out, "KKKKK", "every I is dropped, so all five slots are padded");
        assert_eq!(calls, CODE_LENGTH);
        assert!(valid_room_code(&out));

        assert_eq!(admin_code_from_input("ABCDEFG", || 'Z'), "ABCDE", "a long code keeps its first five");
        assert_eq!(admin_code_from_input("ab c1o0", || 'Z'), "ABCZZ", "I/O/0/1 and spaces are dropped, then padded");

        let mut calls = 0;
        let out = admin_code_from_input("", || {
            calls += 1;
            CODE_ALPHABET.as_bytes()[calls % CODE_ALPHABET.len()] as char
        });
        assert_eq!(calls, CODE_LENGTH, "an empty code is five random characters");
        assert!(valid_room_code(&out), "{out}");
        assert!(valid_room_code(&admin_code_from_input("   ", cycle())), "a blank code is random too");

        for typed in ["kathy", "x", "!@#$%", "ZZZZZZZZ", " 1 2 3 "] {
            let out = admin_code_from_input(typed, cycle());
            assert!(valid_room_code(&out), "{typed:?} -> {out}");
        }
    }

    #[test]
    fn a_guest_still_gets_the_join_message_for_an_invalid_code() {
        // the guest path is unchanged: an invalid code is refused before any lookup, and a guest's miss never reserves
        assert!(!valid_join_code("ab-c"));
        assert!(!valid_join_code(""));
        let miss = NetError::Http { status: 404, code: String::new(), error: "ROOM NOT FOUND".into(), body: Value::Null };
        assert!(!admin_may_reserve_on_miss(false, "token", &miss), "a guest's miss is ROOM NOT FOUND, not a reserve");
    }

    #[test]
    fn an_owners_miss_asks_for_the_reserve_request_a_guest_does_not() {
        let miss = NetError::Http { status: 404, code: String::new(), error: "ROOM NOT FOUND".into(), body: Value::Null };
        assert!(admin_may_reserve_on_miss(true, "token", &miss));
        assert!(!admin_may_reserve_on_miss(false, "token", &miss), "a guest never reserves a code");
        assert!(!admin_may_reserve_on_miss(true, "", &miss), "no session, no reserve");
        let other = NetError::Http { status: 500, code: String::new(), error: "DOWN".into(), body: Value::Null };
        assert!(!admin_may_reserve_on_miss(true, "token", &other), "only a not-found code is reserved");
    }

    #[test]
    fn an_owners_reserved_code_is_hosted_like_a_miss() {
        // a code already reserved answers 403 ROOM_RESERVED on lookup: the owner reserves (ok) and hosts it
        let reserved = NetError::Http { status: 403, code: "ROOM_RESERVED".into(), error: "RESERVED".into(), body: Value::Null };
        assert!(admin_may_reserve_on_miss(true, "token", &reserved));
        assert!(!admin_may_reserve_on_miss(false, "token", &reserved), "a guest still gets the access flow");
        assert!(!admin_may_reserve_on_miss(true, "", &reserved), "no session, no reserve");
        let other_403 = NetError::Http { status: 403, code: "FORBIDDEN".into(), error: "NO".into(), body: Value::Null };
        assert!(!admin_may_reserve_on_miss(true, "token", &other_403), "only ROOM_RESERVED is treated as a miss");
    }

    #[test]
    fn the_reserve_request_names_the_code_and_the_owner_session() {
        let body = admin_reserve_body("ADMN1", "tok");
        assert_eq!(body, json!({"action": "create", "code": "ADMN1", "sessionToken": "tok"}));
    }

    #[test]
    fn the_password_reset_body_names_the_target_in_uppercase_and_sends_a_password_only_when_typed() {
        assert_eq!(admin_reset_password_body("tok", " runner ", None), json!({"sessionToken": "tok", "username": "RUNNER"}));
        assert_eq!(admin_reset_password_body("tok", "runner", Some("")), json!({"sessionToken": "tok", "username": "RUNNER"}));
        let typed = admin_reset_password_body("tok", "runner", Some("fake-typed-password"));
        assert_eq!(typed["newPassword"], "fake-typed-password");
    }

    #[test]
    fn a_reset_answer_never_prints_its_password() {
        let reset = PasswordReset { username: "RUNNER".into(), temporary_password: Some("K7QF-2MXD-H9TA-WP4C".into()), sessions_revoked: 2 };
        let shown = format!("{reset:?}");
        assert!(shown.contains("RUNNER") && !shown.contains("K7QF"), "{shown}");
    }

    /// One canned HTTP answer on loopback; returns the port and the request text the client sent.
    fn serve_once(reply: String) -> (u16, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut got = Vec::new();
            let mut chunk = [0u8; 2048];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(at) = text.find("\r\n\r\n") {
                    let length = text[..at]
                        .to_ascii_lowercase()
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if got.len() >= at + 4 + length {
                        break;
                    }
                }
            }
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        (port, handle)
    }

    fn json_reply(status: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    }

    #[test]
    fn the_reset_call_posts_to_the_admin_route_and_returns_the_temporary_password_once() {
        let (port, seen) = serve_once(json_reply(
            "200 OK",
            r#"{"ok":true,"username":"RUNNER","generated":true,"temporaryPassword":"K7QF-2MXD-H9TA-WP4C","sessionsRevoked":2}"#,
        ));
        let reset = admin_reset_password(&BackendConfig::loopback(port), "tok", "runner", None).unwrap();
        assert_eq!(
            reset,
            PasswordReset { username: "RUNNER".into(), temporary_password: Some("K7QF-2MXD-H9TA-WP4C".into()), sessions_revoked: 2 }
        );
        let sent = seen.join().unwrap();
        assert!(sent.starts_with("POST /api/admin/account/reset-password HTTP/1.1\r\n"), "{sent}");
        assert!(sent.ends_with(r#"{"sessionToken":"tok","username":"RUNNER"}"#), "{sent}");

        // a typed password is not echoed by the server, so none comes back
        let (port, _) = serve_once(json_reply("200 OK", r#"{"ok":true,"username":"RUNNER","generated":false,"sessionsRevoked":0}"#));
        let reset = admin_reset_password(&BackendConfig::loopback(port), "tok", "runner", Some("fake-typed-password")).unwrap();
        assert_eq!(reset.temporary_password, None);
    }

    #[test]
    fn the_reset_call_passes_the_servers_refusal_on() {
        let (port, _) = serve_once(json_reply("403 Forbidden", r#"{"ok":false,"code":"ADMIN_OWNER_REQUIRED","error":"Full admin account required"}"#));
        let err = admin_reset_password(&BackendConfig::loopback(port), "tok", "runner", None).unwrap_err();
        assert!(matches!(&err, NetError::Http { status: 403, code, .. } if code == "ADMIN_OWNER_REQUIRED"), "{err:?}");
        let offline = BackendConfig { offline: true, ..BackendConfig::loopback(1) };
        assert!(matches!(admin_reset_password(&offline, "tok", "runner", None), Err(NetError::Offline)));
    }
}
