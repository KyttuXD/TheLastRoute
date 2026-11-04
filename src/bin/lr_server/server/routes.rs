//! The request routing of `server.mjs` (the `createServer` callback), in the same order as the original.

use super::*;
use std::sync::MutexGuard;
use crate::crash_reports::CRASH_MAX_BODY_BYTES;
use crate::game_saves::{match_save_path, SAVE_MAX_BYTES};
use crate::lr_regex as regex;
use crate::moderation::Who;
use crate::public_route::JoinOptions;
use crate::util::{hex, iso_time, js_number, log_info, log_warn, str_nullish, str_or_empty, timing_safe_equal, truthy, Reply};

fn not_owner() -> Response {
    json_response(403, &json!({ "ok": false, "code": "ADMIN_OWNER_REQUIRED", "error": "Full admin account required" }), &[])
}

fn skins_unavailable() -> Response {
    json_response(503, &json!({ "ok": false, "code": "SKINS_UNAVAILABLE", "error": "Skins are not available on this server" }), &[])
}

/// The skin id of a `GET /api/v1/skins/<id>` request.
fn skin_read_id(method: &str, path: &str) -> Option<String> {
    if method != "GET" {
        return None;
    }
    regex!(r"^/api/v1/skins/([^/]{1,128})$").captures(path).map(|caps| caps[1].to_string())
}

/// A stored skin: the PNG bytes, cached for good (the id is the hash of the bytes, so it never changes).
fn png_response(png: Vec<u8>) -> Response {
    let length = png.len().to_string();
    let extra = [
        ("content-type", "image/png".to_string()),
        ("content-length", length),
        ("cache-control", "public, max-age=31536000, immutable".to_string()),
    ];
    Response { status: 200, headers: merge_headers(response_headers(), &extra), body: png }
}

fn unknown_endpoint() -> Reply {
    (404, json!({ "ok": false, "code": "UNKNOWN_ENDPOINT", "error": "Unknown endpoint" }))
}

fn or_default(text: String, fallback: &str) -> String {
    if text.is_empty() {
        fallback.to_string()
    } else {
        text
    }
}

/// One line per account event: the time, the event, the account name, the outcome and the source address. The body and
/// the tokens are never logged.
fn log_account_event(now: i64, path: &str, body: &Value, result: &Reply, address: &str) {
    let event = match path {
        "/api/account/register" => "register",
        "/api/account/login" => "login",
        "/api/account/session" => "session",
        "/api/account/logout" => "logout",
        "/api/account/password" => "password",
        "/api/account/rename" => "rename",
        _ => return,
    };
    let ok = result.1.get("ok") == Some(&json!(true));
    let name = if ok { str_or_empty(result.1.get("username")) } else { str_or_empty(body.get("username")) };
    let name: String = name.trim().to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(24).collect();
    let name = if name.is_empty() { "-".to_string() } else { name };
    let outcome = match result.1.get("code").and_then(Value::as_str) {
        _ if ok => "ok".to_string(),
        Some(code) => code.to_string(),
        None => format!("HTTP {}", result.0),
    };
    let line = format!("account {event} at {} user={name} outcome={outcome} from={address}", iso_time(now));
    if ok {
        log_info(&line);
    } else {
        log_warn(&line);
    }
}

fn owner_of(state: &State, body: &Value) -> Option<String> {
    let owner = state.accounts.session_username(&str_or_empty(body.get("sessionToken")))?;
    state.accounts.is_admin_owner(&owner).then_some(owner)
}

/// `body.sessionToken ? accountService.sessionUsername(body.sessionToken) || "" : ""`.
fn session_user(state: &State, body: &Value) -> String {
    if truthy(body.get("sessionToken")) {
        state.accounts.session_username(&str_or_empty(body.get("sessionToken"))).unwrap_or_default()
    } else {
        String::new()
    }
}

fn owner_identity(accounts: &AccountService, body: &Value) -> bool {
    accounts
        .session_admin_identity(&str_or_empty(body.get("sessionToken")))
        .map(|identity| identity.get("adminOwner") == Some(&Value::Bool(true)))
        .unwrap_or(false)
}

/// The build revision baked in at compile time from `LASTROUTE_BUILD_REVISION`; null when the build did not set it.
const BUILD_REVISION: Option<&str> = option_env!("LASTROUTE_BUILD_REVISION");

impl LastRouteServer {
    fn rate_limit(&self, state: &mut State, request: &Request, pathname: &str) -> i64 {
        let sensitive = ACCOUNT_RATE_LIMIT_PATHS.contains(&pathname);
        let mods = !sensitive && pathname.starts_with("/api/mods/");
        let maximum = if sensitive {
            self.config.account_rate_limit_max
        } else if mods {
            self.config.mod_rate_limit_max
        } else {
            self.config.api_rate_limit_max
        }
        .max(1);
        let window = self.config.rate_limit_window_ms.max(1000);
        let timestamp = self.now();
        let address = or_default(requester_address(request, self.config.trust_proxy), "unknown");
        let key = format!("{}|{address}", if sensitive { "account" } else if mods { "mods" } else { "api" });
        let rate = state.request_rates.entry(key).or_insert((0, timestamp + window));
        if timestamp >= rate.1 {
            *rate = (0, timestamp + window);
        }
        rate.0 += 1;
        if rate.0 <= maximum {
            0
        } else {
            (((rate.1 - timestamp) as f64) / 1000.0).ceil().max(1.0) as i64
        }
    }

    /// `developerRequest(body)`: a live dev token, the setup password as `adminProof`, or an owner session.
    fn developer_request(&self, state: &mut State, body: &Value) -> bool {
        self.developer_proof(state, body) || owner_identity(&state.accounts, body)
    }

    /// The part of `developerRequest` that needs only the server state: a live dev token or the setup password.
    fn developer_proof(&self, state: &mut State, body: &Value) -> bool {
        if truthy(body.get("devToken")) {
            let key = str_or_empty(body.get("devToken"));
            if let Some(expires) = state.dev_tokens.get(&key).copied() {
                if expires > self.now() {
                    return true;
                }
                state.dev_tokens.remove(&key);
            }
        }
        !self.config.admin_password.is_empty() && safe_equal(&str_or_empty(body.get("adminProof")), &self.config.admin_password)
    }

    /// Runs `call` against the account service with the state lock released, then takes the lock again. The account
    /// service synchronizes itself, and a forwarded call can wait on the website for seconds.
    fn with_accounts<'a, T>(&'a self, guard: MutexGuard<'a, State>, call: impl FnOnce(&AccountService) -> T) -> (T, MutexGuard<'a, State>) {
        let accounts = guard.accounts.clone();
        drop(guard);
        let value = call(&accounts);
        (value, self.state.lock().unwrap())
    }

    /// `developerRequest` for the account routes: the upstream session lookup runs without the state lock.
    fn developer_request_unlocked<'a>(&'a self, mut guard: MutexGuard<'a, State>, body: &Value) -> (bool, MutexGuard<'a, State>) {
        if self.developer_proof(&mut guard, body) {
            return (true, guard);
        }
        self.with_accounts(guard, |accounts| owner_identity(accounts, body))
    }

    fn issue_dev_token(&self, state: &mut State) -> String {
        let token = hex(&random_bytes(24));
        let timestamp = self.now();
        state.dev_tokens.retain(|_, expires| *expires > timestamp);
        let ttl = if self.config.dev_token_ttl_ms > 0 { self.config.dev_token_ttl_ms } else { DEFAULT_DEV_TOKEN_TTL_MS };
        state.dev_tokens.insert(token.clone(), timestamp + ttl.max(1000));
        token
    }

    pub(crate) fn route(&self, request: &Request) -> Response {
        let method = request.method.as_str();
        if method == "OPTIONS" {
            return json_response(204, &json!({}), &[]);
        }
        let mut guard = self.state.lock().unwrap();
        let state = &mut *guard;
        state.dir.prune();
        let path = request.path.as_str();
        let retry_after = self.rate_limit(state, request, path);
        if retry_after > 0 {
            return json_response(
                429,
                &json!({ "ok": false, "code": "RATE_LIMITED", "error": "Too many requests; try again shortly" }),
                &[("retry-after", retry_after.to_string())],
            );
        }
        // Skin GET: the store may forward to the website, so the state lock is released before it runs.
        if let Some(caps) = skin_read_id(method, path) {
            let skins = state.skins.clone();
            drop(guard);
            return match skins.read(&caps) {
                Ok(png) => png_response(png),
                Err(refused) => reply(refused),
            };
        }
        // Save routes: the state lock is released first, because a save writes files and may wait on the website.
        if method == "GET" || method == "PUT" {
            if let Some((game_id, slot)) = match_save_path(path) {
                let saves = state.game_saves.clone();
                let accounts = state.accounts.clone();
                drop(guard);
                return self.save_route(&saves, &accounts, request, &game_id, &slot);
            }
        }
        // Skin checks: the website session lookup runs with the state lock released (see `skin_put_checks`).
        if (method == "PUT" && regex!(r"^/api/v1/skins/([A-Za-z0-9_]{1,32})$").is_match(path)) || (method == "POST" && path == "/api/skins/upload") {
            let (refused, _guard) = self.skin_put_checks(guard, request);
            return refused.unwrap_or_else(skins_unavailable);
        }
        if let Some(response) = self.route_reads(state, request) {
            return response;
        }
        if method != "POST" {
            if let Some(found) = try_static_file(method, self.config.public_directory.as_deref(), path) {
                return found;
            }
            if method == "GET" && path == "/" {
                return json_response(200, &json!({ "ok": true, "service": "Last Route backend", "health": "/api/health",
                    "staticFiles": self.config.public_directory.is_some() }), &[]);
            }
            return json_response(404, &json!({ "ok": false, "code": "NOT_FOUND", "error": "Not found" }), &[]);
        }
        if path == "/api/regions/report" {
            let raw = String::from_utf8_lossy(&request.body).to_string();
            let region = state.regions.verify(&request.headers, &raw);
            if region.is_empty() {
                return json_response(403, &json!({ "ok": false, "code": "BAD_AGENT_SIGNATURE", "error": "Unknown region or bad signature" }), &[]);
            }
            let Ok(report) = serde_json::from_str::<Value>(&raw) else {
                return json_response(400, &json!({ "ok": false, "error": "INVALID JSON" }), &[]);
            };
            return json_response(200, &json!({ "ok": true, "actions": state.regions.report(&region, &report) }), &[]);
        }
        // readJson(request, maxBytes)
        let max = match path {
            "/api/admin/mods/register" => MOD_REGISTER_MAX_BODY_BYTES,
            "/api/v1/crash-reports" => CRASH_MAX_BODY_BYTES,
            _ => MAX_BODY_BYTES,
        };
        if request.body_overflow || request.body.len() > max {
            return json_response(413, &json!({ "ok": false, "error": "REQUEST BODY TOO LARGE" }), &[]);
        }
        let body: Value = if request.body.is_empty() {
            json!({})
        } else {
            match std::str::from_utf8(&request.body).ok().and_then(|t| serde_json::from_str::<Value>(t).ok()) {
                Some(v @ Value::Object(_)) => v,
                _ => return json_response(400, &json!({ "ok": false, "error": "INVALID JSON BODY" }), &[]),
            }
        };
        // Skin upload: forwarded to the website when one is set, so the state lock is released before it runs.
        if path == "/api/v1/skins" {
            let skins = state.skins.clone();
            let accounts = state.accounts.clone();
            drop(guard);
            return reply(skins.upload(&*accounts, &body));
        }
        if let Some(response) = self.route_posts(state, request, &body) {
            return response;
        }
        let result = self.route_accounts(guard, &body, path);
        log_account_event(self.now(), path, &body, &result, &requester_address(request, self.config.trust_proxy));
        reply(result)
    }

    /// GET/PUT `/api/v1/saves/:gameId/:slot`, run without the state lock. Only the save store's own lock is held.
    fn save_route(&self, saves: &Mutex<GameSaves>, accounts: &AccountService, request: &Request, game_id: &str, slot: &str) -> Response {
        let token = request.header("x-session-token");
        if request.method == "GET" {
            return reply(saves.lock().unwrap().get(accounts, &token, game_id, slot));
        }
        if request.body_overflow || request.body.len() > SAVE_MAX_BYTES {
            return json_response(413, &json!({ "ok": false, "code": "SAVE_TOO_LARGE",
                "error": format!("A save is at most {} KB", SAVE_MAX_BYTES / 1024) }), &[]);
        }
        let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
            return json_response(400, &json!({ "ok": false, "code": "BAD_SAVE", "error": "INVALID JSON BODY" }), &[]);
        };
        reply(saves.lock().unwrap().put(accounts, &token, game_id, slot, &body))
    }

    /// The size and moderation checks a skin upload passes before the (unported) skin store. The session lookup can
    /// forward to the website, so it runs with the state lock released; the moderation check takes the lock again.
    fn skin_put_checks<'a>(&'a self, guard: MutexGuard<'a, State>, request: &Request) -> (Option<Response>, MutexGuard<'a, State>) {
        if request.body_overflow || request.body.len() > SKIN_MAX_BYTES {
            return (Some(json_response(413, &json!({ "ok": false, "code": "BAD_SKIN", "error": "Skin refused: the file is too large" }), &[])), guard);
        }
        let (user, guard) = self.with_accounts(guard, |accounts| accounts.session_username(&request.header(SKIN_TOKEN_HEADER)).unwrap_or_default());
        (guard.moderation.check_skin_upload(&user).map(reply), guard)
    }

    /// GET/HEAD/PUT routes that come before the POST-only section.
    fn route_reads(&self, state: &mut State, request: &Request) -> Option<Response> {
        let method = request.method.as_str();
        let path = request.path.as_str();
        let now = self.now();
        let (relay_address, relay_port) = (self.config.relay_address.as_str(), self.config.relay_port);
        let get_or_head = method == "GET" || method == "HEAD";

        if method == "GET" && path == "/api/health" {
            let relay = state.relay.as_ref().map(|r| r.relay_status()).unwrap_or(json!({ "enabled": false }));
            return Some(json_response(200, &json!({
                "ok": true, "status": "ok", "revision": BUILD_REVISION, "uptimeSeconds": self.started.elapsed().as_secs(),
                "runtime": "rust", "service": "last-route-backend", "instance": self.config.instance,
                "port": request.local_port, "rooms": state.dir.enet_rooms.len(), "relay": relay,
                "wsRelay": { "enabled": false, "path": WS_PATH, "rooms": 0, "guests": 0, "packets": 0, "bytes": 0 },
                "webRooms": state.room_service.rooms_len(), "reloadId": state.reload_id,
                "updates": public_update_status(&self.current_updates()), "now": iso_time(now),
            }), &[]));
        }
        if get_or_head && path.starts_with("/api/city-maps/files/") {
            return Some(json_response(404, &json!({ "ok": false, "code": "CITY_MAPS_UNAVAILABLE", "error": "City maps are not part of this server" }), &[]));
        }
        if get_or_head && path.starts_with("/api/skins/files/") {
            return Some(skins_unavailable());
        }
        if method == "GET" && path == "/api/mods/packs" {
            let result = state.mod_packs.list();
            return Some(json_response(result.0, &result.1, &[("cache-control", "public, max-age=60".into())]));
        }
        if method == "GET" {
            if let Some(caps) = regex!(r"^/api/mods/packs/([a-z0-9][a-z0-9_]{2,31})$").captures(path) {
                let result = state.mod_packs.get(&caps[1], request.query_param("version").as_deref());
                return Some(json_response(result.0, &result.1, &[("cache-control", "public, max-age=60".into())]));
            }
        }
        if get_or_head {
            if let Some(caps) = regex!(r"(?i)^/api/custom-maps/([A-Za-z0-9][A-Za-z0-9._-]{0,180}\.png)$").captures(path) {
                let found = try_static_file(method, self.config.custom_map_directory.as_deref(), &format!("/{}", &caps[1]));
                return Some(found.unwrap_or_else(|| json_response(404, &json!({ "ok": false, "error": "CUSTOM MAP NOT FOUND" }), &[])));
            }
        }
        if method == "GET" {
            if let Some(caps) = regex!(r"^/api/updates/(debug|release)/(linux|windows|windows_legacy|macos|web|steamvr|quest|phone)$").captures(path) {
                return Some(match public_update_manifest(&self.current_updates(), &caps[1], &caps[2]) {
                    None => json_response(404, &json!({ "ok": false, "error": "NO UPDATE PUBLISHED" }), &[]),
                    Some(update) => json_response(200, &json!({ "ok": true, "channel": &caps[1], "platform": &caps[2], "update": update }), &[]),
                });
            }
        }
        if get_or_head {
            if let Some(caps) = regex!(r"^/api/updates/files/([^/]+)$").captures(path) {
                // the game files are not public: when a key is configured, the request must carry it
                let key = &self.config.update_key;
                if !key.is_empty() && !timing_safe_equal(request.header("x-download-key").as_bytes(), key.as_bytes()) {
                    return Some(json_response(403, &json!({ "ok": false, "code": "DOWNLOAD_KEY_REQUIRED", "error": "A DOWNLOAD KEY IS REQUIRED" }), &[]));
                }
                let asset = published_update_path(&self.current_updates(), &caps[1]);
                let found = if asset.is_empty() { None } else { try_static_file(method, self.config.update_directory.as_deref(), &asset) };
                return Some(found.unwrap_or_else(|| json_response(404, &json!({ "ok": false, "error": "UPDATE FILE NOT FOUND" }), &[])));
            }
        }
        if method == "GET" {
            if let Some(caps) = regex!(r"^/api/enet/rooms/([^/]+)$").captures(path) {
                let code = code_of(&percent_decode(&caps[1]).unwrap_or_default());
                if state.dir.reserved.as_ref().map(|r| r.is_reserved(&code)).unwrap_or(false) {
                    return Some(json_response(403, &json!({ "ok": false, "code": "ROOM_RESERVED", "reserved": true,
                        "error": "PRIVATE ROOM // SIGN IN WITH AN INVITE OR ENTER THE ROOM PASSWORD" }), &[]));
                }
                return Some(match state.dir.enet_rooms.get(&code).filter(|_| !code.is_empty()) {
                    Some(room) => json_response(200, &json!({ "ok": true, "room": public_enet_room(room, now, relay_address, relay_port) }), &[]),
                    None => json_response(404, &json!({ "ok": false, "error": "ROOM NOT FOUND" }), &[]),
                });
            }
            if path == "/api/regions" {
                return Some(json_response(200, &json!({ "ok": true, "regions": state.regions.public_list() }), &[]));
            }
            if path == "/api/public/status" {
                let game = public_game_id(request.query_param("game").as_deref());
                return Some(json_response(200, &state.dir.route_mut(&game).status(), &[]));
            }
        }
        None
    }

    /// POST routes with a parsed JSON body, up to the account section.
    fn route_posts(&self, state: &mut State, request: &Request, body: &Value) -> Option<Response> {
        let path = request.path.as_str();
        let now = self.now();
        let (relay_address, relay_port) = (self.config.relay_address.as_str(), self.config.relay_port);
        let address = requester_address(request, self.config.trust_proxy);

        if path == "/api/v1/crash-reports" {
            let Some(crash) = state.crash_reports.as_mut() else { return Some(reply(unknown_endpoint())) };
            let device: String = str_or_empty(body.get("device_id")).chars().take(64).collect();
            let client = format!("{}|{device}", or_default(address.clone(), "unknown"));
            return Some(reply(crash.submit(body, &client)));
        }
        if path == "/api/admin/crash-reports" {
            if owner_of(state, body).is_none() {
                return Some(not_owner());
            }
            return Some(match state.crash_reports.as_ref() {
                None => json_response(200, &json!({ "ok": true, "reports": [], "total": 0 }), &[]),
                Some(crash) => reply(crash.list(body.get("since"), body.get("limit"))),
            });
        }

        if path == "/api/public/join" || path == "/api/public/state" || path == "/api/public/leave" {
            let game = public_game_id(Some(&str_nullish(body.get("game"))));
            let code = public_code(&game);
            let (listed, host_token_valid) = match state.dir.enet_rooms.get_mut(code) {
                Some(room) => {
                    let valid = truthy(body.get("hostToken")) && safe_equal(&str_or_empty(body.get("hostToken")), &room.token);
                    if valid {
                        room.expires_at = now + PUBLIC_ENET_TTL_MS;
                    }
                    (true, valid)
                }
                None => (false, false),
            };
            if path == "/api/public/join" {
                let username = state.accounts.session_username(&str_or_empty(body.get("sessionToken"))).unwrap_or_default();
                let who = Who { username, device_id: body.get("deviceId").cloned(), address: address.clone() };
                if let Some(refused) = state.moderation.check_public_join(&who) {
                    return Some(reply(refused));
                }
            }
            let route = state.dir.route_mut(&game);
            let result = match path {
                "/api/public/join" => route.join(body, JoinOptions { address: address.clone(), host_token_valid, enet_listed: listed }),
                "/api/public/state" => route.post(body, host_token_valid),
                _ => route.leave(body),
            };
            return Some(reply(result.to_reply()));
        }

        if path == "/api/rooms/reserved/resolve" {
            let code = normalize_enet_code(body.get("code"));
            if code.is_empty() || !state.dir.reserved.as_ref().map(|r| r.is_reserved(&code)).unwrap_or(false) {
                return Some(json_response(404, &json!({ "ok": false, "error": "ROOM NOT FOUND" }), &[]));
            }
            let username = session_user(state, body);
            let owner = !username.is_empty() && state.accounts.is_admin_owner(&username);
            let password = str_or_empty(body.get("password"));
            let live = state.dir.enet_rooms.get(&code).cloned();
            let reserved = state.dir.reserved.as_mut().unwrap();
            if !reserved.has_access(&code, &username, &password, owner) {
                return Some(json_response(403, &json!({ "ok": false, "code": "ROOM_ACCESS_DENIED", "reserved": true,
                    "error": "NOT ON THIS ROOM'S INVITE LIST // WRONG ROOM PASSWORD" }), &[]));
            }
            let stale = live.as_ref().map(|l| now - l.last_seen > RESERVED_TAKEOVER_MS).unwrap_or(true);
            let room = match (&live, stale) {
                (Some(l), false) => public_enet_room(l, now, relay_address, relay_port),
                _ => Value::Null,
            };
            let ticket = if stale { String::new() } else { reserved.issue_ticket(&code) };
            return Some(json_response(200, &json!({ "ok": true, "reserved": true, "live": !stale, "room": room,
                "joinTicket": ticket, "canHost": stale, "settings": reserved.settings(&code) }), &[]));
        }

        if path == "/api/admin/regions" {
            if owner_of(state, body).is_none() {
                return Some(not_owner());
            }
            let action = or_default(str_or_empty(body.get("action")), "list");
            let region = str_or_empty(body.get("region"));
            let code = normalize_enet_code(body.get("code"));
            return Some(match action.as_str() {
                "list" => json_response(200, &json!({ "ok": true, "regions": state.regions.owner_list() }), &[]),
                "stop" => reply(state.regions.request_stop(&region, &code)),
                "spawn" => {
                    if code.is_empty() {
                        return Some(json_response(400, &json!({ "ok": false, "error": "INVALID ROOM CODE" }), &[]));
                    }
                    if state.dir.enet_rooms.get(&code).map(|l| now - l.last_seen <= RESERVED_TAKEOVER_MS).unwrap_or(false) {
                        return Some(json_response(409, &json!({ "ok": false, "code": "ROOM_ALREADY_HOSTED", "error": "Someone is hosting that code" }), &[]));
                    }
                    let ticket = base64url(&random_bytes(18));
                    state.spawn_tickets.insert(code.clone(), (ticket.clone(), now + 5 * 60_000));
                    let options = json!({ "seed": body.get("seed"), "difficulty": body.get("difficulty"),
                        "idleMinutes": body.get("idleMinutes"), "spawnTicket": ticket });
                    let result = state.regions.request_spawn(&region, &code, &options);
                    if result.0 != 202 {
                        state.spawn_tickets.remove(&code);
                    }
                    reply(result)
                }
                _ => json_response(400, &json!({ "ok": false, "code": "INVALID_ACTION", "error": "Actions: list, spawn, stop" }), &[]),
            });
        }

        if path == "/api/admin/reserved-rooms" {
            let Some(owner) = owner_of(state, body) else { return Some(not_owner()) };
            if state.dir.reserved.is_none() {
                return Some(reply(unknown_endpoint()));
            }
            if body.get("action").and_then(Value::as_str) == Some("command") {
                let live = state.dir.enet_rooms.get(&normalize_enet_code(body.get("code")));
                if live.map(|l| now - l.last_seen > RESERVED_TAKEOVER_MS).unwrap_or(true) {
                    return Some(json_response(409, &json!({ "ok": false, "code": "ROOM_NOT_LIVE", "error": "Nobody is hosting that room right now" }), &[]));
                }
            }
            // an owner's JOIN ROOM on a code someone is already hosting joins that room: never reserve over it
            if body.get("action").and_then(Value::as_str) == Some("create") {
                let live = state.dir.enet_rooms.get(&normalize_enet_code(body.get("code"))).filter(|l| now - l.last_seen <= RESERVED_TAKEOVER_MS);
                if live.is_some() {
                    return Some(json_response(409, &json!({ "ok": false, "code": "ROOM_ALREADY_HOSTED", "error": "Someone is hosting that code" }), &[]));
                }
            }
            let mut result = state.dir.reserved.as_mut().unwrap().admin(body, &owner);
            if let Some(Value::Array(rooms)) = result.1.get_mut("rooms") {
                for room in rooms.iter_mut() {
                    let code = str_nullish(room.get("code"));
                    let live = state.dir.enet_rooms.get(&code).filter(|l| now - l.last_seen <= RESERVED_TAKEOVER_MS);
                    if let Value::Object(map) = room {
                        map.insert("live".into(), json!(live.is_some()));
                        map.insert("players".into(), live.map(|l| num(l.players)).unwrap_or(json!(0)));
                        map.insert("sessionActive".into(), json!(live.map(|l| l.session_active).unwrap_or(false)));
                    }
                }
            }
            return Some(reply(result));
        }

        if path == "/api/enet/rooms/register" {
            return Some(self.register_room(state, request, body, &address));
        }
        if path.starts_with("/api/enet/") {
            if let Some(response) = self.route_enet(state, body, path) {
                return Some(response);
            }
        }

        if path == "/api/moderation/report" || path == "/api/moderation/peers" {
            let code = normalize_enet_code(body.get("code"));
            let room = state.dir.enet_rooms.get(&code).cloned();
            let Some(room) = room.filter(|r| safe_equal(&str_or_empty(body.get("hostToken")), &r.token)) else {
                return Some(json_response(403, &json!({ "ok": false, "code": "INVALID_HOST_TOKEN", "error": "INVALID HOST TOKEN" }), &[]));
            };
            if path == "/api/moderation/peers" {
                return Some(reply(state.moderation.peers(body)));
            }
            let host = if room.host_user.is_empty() { format!("room:{}", room.instance) } else { format!("user:{}", room.host_user) };
            let ctx = ModCtx { accounts: &state.accounts, updates: self.current_updates() };
            return Some(reply(state.moderation.report_kick(&ctx, body, &code, &host)));
        }
        if path == "/api/moderation/integrity" {
            let username = session_user(state, body);
            let ctx = ModCtx { accounts: &state.accounts, updates: self.current_updates() };
            return Some(reply(state.moderation.integrity(&ctx, body, &username, &address)));
        }
        if path == "/api/moderation/status" {
            let who = Who { username: session_user(state, body), device_id: body.get("deviceId").cloned(), address: address.clone() };
            return Some(json_response(200, &json!({ "ok": true, "status": state.moderation.status(&who) }), &[]));
        }
        if path.starts_with("/api/admin/mods/") {
            let Some(owner) = owner_of(state, body) else { return Some(not_owner()) };
            return Some(reply(match path {
                "/api/admin/mods/register" => state.mod_packs.register(body, &owner),
                "/api/admin/mods/block" => state.mod_packs.block(body, &owner),
                "/api/admin/mods/unblock" => state.mod_packs.unblock(body, &owner),
                _ => unknown_endpoint(),
            }));
        }
        if path.starts_with("/api/admin/flags") || path.starts_with("/api/admin/builds/") {
            let Some(owner) = owner_of(state, body) else { return Some(not_owner()) };
            let ctx = ModCtx { accounts: &state.accounts, updates: self.current_updates() };
            return Some(reply(match path {
                "/api/admin/flags" => state.moderation.admin_list(),
                "/api/admin/flags/set" => state.moderation.admin_set(&ctx, body, &owner),
                "/api/admin/builds/add" => state.moderation.add_build(body, &owner),
                "/api/admin/builds/remove" => state.moderation.remove_build(body),
                _ => unknown_endpoint(),
            }));
        }
        if path == "/api/dev/login" {
            if self.config.admin_password.is_empty() {
                return Some(json_response(503, &json!({ "ok": false, "code": "DEV_LOGIN_DISABLED", "error": "Developer login is not configured" }), &[]));
            }
            if !safe_equal(&str_or_empty(body.get("password")), &self.config.admin_password) {
                return Some(json_response(401, &json!({ "ok": false, "code": "INVALID_DEV_PASSWORD", "error": "Invalid developer password" }), &[]));
            }
            let token = self.issue_dev_token(state);
            return Some(json_response(200, &json!({ "ok": true, "developer": true, "admin": true, "devToken": token }), &[]));
        }
        if path == "/api/dev/reload" {
            if !self.developer_request(state, body) {
                return Some(json_response(401, &json!({ "ok": false, "code": "NOT_AUTHORIZED", "error": "Developer proof required" }), &[]));
            }
            state.reload_id += 1;
            return Some(json_response(200, &json!({ "ok": true, "reloadId": state.reload_id }), &[]));
        }
        None
    }

    /// heartbeat, unregister, release-host and the admin tickets.
    fn route_enet(&self, state: &mut State, body: &Value, path: &str) -> Option<Response> {
        let now = self.now();
        let (relay_address, relay_port) = (self.config.relay_address.as_str(), self.config.relay_port);
        let code = normalize_enet_code(body.get("code"));
        let token_ok = |room: &EnetRoom, key: &str| safe_equal(&str_or_empty(body.get(key)), &room.token);
        match path {
            "/api/enet/rooms/heartbeat" => {
                let ttl = self.config.enet_room_ttl_ms;
                let Some(room) = state.dir.enet_rooms.get_mut(&code) else {
                    return Some(json_response(404, &json!({ "ok": false, "error": "ROOM NOT FOUND" }), &[]));
                };
                if !token_ok(room, "token") {
                    return Some(json_response(403, &json!({ "ok": false, "error": "INVALID HOST TOKEN" }), &[]));
                }
                room.players = finite_players(body.get("players"), room.players).trunc();
                room.session_active = truthy(body.get("sessionActive"));
                room.daytime = truthy(body.get("daytime"));
                room.expires_at = now + if is_public_code(&room.code) { PUBLIC_ENET_TTL_MS } else { ttl };
                room.last_seen = now;
                let room = room.clone();
                let mut out = Map::new();
                out.insert("ok".into(), json!(true));
                out.insert("room".into(), public_enet_room(&room, now, relay_address, relay_port));
                if room.reserved {
                    let view = state.dir.reserved.as_mut().and_then(|r| r.host_view(&room.code, body.get("commandSince"), body.get("commandAck")));
                    if let Some(view) = view {
                        out.insert("reserved".into(), view);
                    }
                }
                Some(json_response(200, &Value::Object(out), &[]))
            }
            "/api/enet/rooms/unregister" => {
                let Some(room) = state.dir.enet_rooms.get(&code) else {
                    return Some(json_response(200, &json!({ "ok": true, "removed": false }), &[]));
                };
                if !token_ok(room, "token") {
                    return Some(json_response(403, &json!({ "ok": false, "error": "INVALID HOST TOKEN" }), &[]));
                }
                // an explicit quit: guests are told now and the code stops resolving now
                let State { relay, dir, .. } = state;
                if let Some(relay) = relay.as_mut() {
                    relay.close_room(&code, None, dir);
                }
                dir.forget(&code);
                Some(json_response(200, &json!({ "ok": true, "removed": true }), &[]))
            }
            "/api/enet/rooms/release-host" => {
                let Some(room) = state.dir.enet_rooms.get(&code) else {
                    return Some(json_response(200, &json!({ "ok": true, "migrating": false }), &[]));
                };
                if !token_ok(room, "token") {
                    return Some(json_response(403, &json!({ "ok": false, "error": "INVALID HOST TOKEN" }), &[]));
                }
                let ws = room.transport == "ws";
                let State { relay, dir, .. } = state;
                if let Some(handoff @ Value::Object(_)) = body.get("handoff") {
                    let Some(relay) = relay.as_mut().filter(|_| !ws) else {
                        return Some(json_response(200, &json!({ "ok": true, "migrating": false, "unsupported": true,
                            "error": "HOST TRANSFER IS NOT SUPPORTED FOR THIS ROOM" }), &[]));
                    };
                    let result = relay.request_host_handoff(&code, handoff.get("candidateSession"), handoff.get("epoch"), dir);
                    let mut out = Map::new();
                    out.insert("ok".into(), json!(true));
                    out.insert("migrating".into(), json!(result.migrating));
                    if let Some(error) = result.error {
                        out.insert("error".into(), json!(error));
                    }
                    return Some(json_response(200, &Value::Object(out), &[]));
                }
                let migrating = !ws && relay.as_mut().map(|r| r.request_host_migration(&code, dir)).unwrap_or(false);
                if !migrating {
                    dir.enet_rooms.shift_remove(&code);
                }
                Some(json_response(200, &json!({ "ok": true, "migrating": migrating }), &[]))
            }
            "/api/enet/admin-tickets/issue" => {
                let Some(room) = state.dir.enet_rooms.get(&code).cloned() else {
                    return Some(json_response(404, &json!({ "ok": false, "error": "ROOM NOT FOUND" }), &[]));
                };
                let identity = state.accounts.session_admin_identity(&str_or_empty(body.get("sessionToken"))).unwrap_or(Value::Null);
                let permissions: Vec<Value> = identity.get("adminPermissions").and_then(Value::as_array).cloned().unwrap_or_default();
                if !truthy(identity.get("admin")) || permissions.is_empty() {
                    return Some(json_response(403, &json!({ "ok": false, "error": "ADMIN ACCOUNT REQUIRED" }), &[]));
                }
                let ticket = base64url(&random_bytes(32));
                let configured = if self.config.admin_ticket_ttl_ms > 0 { self.config.admin_ticket_ttl_ms } else { DEFAULT_ADMIN_TICKET_TTL_MS };
                let ttl = configured.min(DEFAULT_ADMIN_TICKET_TTL_MS).max(5_000);
                state.dir.admin_tickets.insert(ticket.clone(), AdminTicket {
                    code: code.clone(),
                    host_token: room.token.clone(),
                    expires_at: now + ttl,
                    username: str_nullish(identity.get("username")),
                    permissions: permissions.clone(),
                    owner: identity.get("adminOwner") == Some(&Value::Bool(true)),
                });
                Some(json_response(201, &json!({ "ok": true, "ticket": ticket, "room": code, "permissions": permissions,
                    "expiresIn": ((ttl as f64) / 1000.0).ceil().max(1.0) as i64 }), &[]))
            }
            "/api/enet/admin-tickets/verify" => {
                let Some(room) = state.dir.enet_rooms.get(&code).cloned() else {
                    return Some(json_response(404, &json!({ "ok": false, "error": "ROOM NOT FOUND" }), &[]));
                };
                if !token_ok(&room, "hostToken") {
                    return Some(json_response(403, &json!({ "ok": false, "error": "INVALID HOST TOKEN" }), &[]));
                }
                let ticket = str_or_empty(body.get("ticket"));
                let proof = state.dir.admin_tickets.get(&ticket).cloned()
                    .filter(|p| p.expires_at > now && p.code == code && safe_equal(&p.host_token, &room.token));
                let Some(proof) = proof else {
                    state.dir.admin_tickets.shift_remove(&ticket);
                    return Some(json_response(403, &json!({ "ok": false, "authorized": false, "error": "INVALID OR EXPIRED ADMIN TICKET" }), &[]));
                };
                Some(json_response(200, &json!({ "ok": true, "authorized": true, "room": code, "permissions": proof.permissions,
                    "owner": proof.owner, "expiresIn": (((proof.expires_at - now) as f64) / 1000.0).ceil().max(1.0) as i64 }), &[]))
            }
            _ => None,
        }
    }

    /// The final dispatch table of server.mjs (legacy rooms, guest names, accounts, roster). The account calls may be
    /// forwarded to the website, so they run with the state lock released and take it again only for the state they touch.
    fn route_accounts<'a>(&'a self, guard: MutexGuard<'a, State>, body: &Value, path: &str) -> Reply {
        let mut guard = guard;
        let mut result: Reply = match path {
            "/api/rooms/join" => guard.room_service.join_room(body),
            "/api/rooms/state" => guard.room_service.update_room(body),
            "/api/rooms/settings" => guard.room_service.update_room_settings(body),
            "/api/rooms/start" => guard.room_service.start_room_round(body),
            "/api/rooms/default-start" => guard.room_service.start_default_route(body),
            "/api/rooms/abandon" => guard.room_service.abandon_route(body),
            "/api/rooms/custom-map" => guard.room_service.set_custom_map(body),
            "/api/rooms/dialog-start" => guard.room_service.start_dialog(body),
            "/api/rooms/dialog-advance" => guard.room_service.advance_dialog(body),
            "/api/skins/mine" | "/api/skins/clear" | "/api/skins/remove" => {
                (503, json!({ "ok": false, "code": "SKINS_UNAVAILABLE", "error": "Skins are not available on this server" }))
            }
            "/api/city-maps/list" | "/api/city-maps/ticket" => {
                (404, json!({ "ok": false, "code": "CITY_MAPS_UNAVAILABLE", "error": "City maps are not part of this server" }))
            }
            "/api/rooms/leave" => guard.room_service.leave_room(body),
            "/api/guest/name" => guard.guest_names.issue(body),
            "/api/account/register" | "/api/account/login" => {
                let register = path == "/api/account/register";
                let (allow, g) = if register { self.developer_request_unlocked(guard, body) } else { (false, guard) };
                let (mut r, g) = self.with_accounts(g, |accounts| if register { accounts.register(body, allow) } else { accounts.login(body) });
                guard = g;
                // The website mints its own devToken for an owner, so a forwarded reply keeps it.
                if !guard.accounts.is_upstream() && r.1.get("ok") == Some(&json!(true)) && truthy(r.1.get("adminOwner")) {
                    r.1["devToken"] = json!(self.issue_dev_token(&mut guard));
                }
                r
            }
            "/api/account/session" => {
                let (r, g) = self.with_accounts(guard, |accounts| accounts.current_session(body));
                guard = g;
                r
            }
            "/api/account/guest" => {
                let (r, g) = self.with_accounts(guard, |accounts| accounts.forward(path, body).unwrap_or_else(unknown_endpoint));
                guard = g;
                r
            }
            "/api/account/password" => {
                let (r, g) = self.with_accounts(guard, |accounts| accounts.change_password(body));
                guard = g;
                r
            }
            "/api/account/rename" => {
                let (previous, g) =
                    self.with_accounts(guard, |accounts| accounts.session_username(&str_or_empty(body.get("sessionToken"))));
                let (allow, g) = self.developer_request_unlocked(g, body);
                let (r, g) = self.with_accounts(g, |accounts| accounts.rename(body, allow));
                guard = g;
                if let (Some(true), Some(previous)) = (r.1.get("ok").and_then(Value::as_bool), previous) {
                    guard.moderation.rename_account(&previous, &str_nullish(r.1.get("username")));
                }
                r
            }
            "/api/account/save" => {
                let (r, g) = self.with_accounts(guard, |accounts| accounts.save(body));
                guard = g;
                r
            }
            "/api/account/load" => {
                let (r, g) = self.with_accounts(guard, |accounts| accounts.load_progress(body));
                guard = g;
                r
            }
            "/api/account/logout" => {
                let (r, g) = self.with_accounts(guard, |accounts| accounts.logout(body));
                guard = g;
                r
            }
            "/api/admin/roster" => {
                let (r, g) = self.with_accounts(guard, |accounts| accounts.list_admin_roster(body));
                guard = g;
                r
            }
            "/api/admin/roster/set" | "/api/admin/roster/remove" => {
                let set = path == "/api/admin/roster/set";
                let (r, g) = self.with_accounts(guard, |accounts| {
                    if set {
                        accounts.set_admin_roster(body)
                    } else {
                        accounts.remove_admin_roster(body)
                    }
                });
                guard = g;
                if r.1.get("ok") == Some(&json!(true)) {
                    let name = if set { r.1.get("entry").and_then(|e| e.get("username")) } else { r.1.get("username") };
                    let name = name.map(|n| str_nullish(Some(n)));
                    guard.dir.admin_tickets.retain(|_, proof| Some(&proof.username) != name.as_ref());
                }
                r
            }
            _ => unknown_endpoint(),
        };
        if let Value::Object(map) = &mut result.1 {
            if truthy(map.get("room")) && !map.contains_key("ok") {
                map.insert("reloadId".into(), json!(guard.reload_id));
            }
        }
        result
    }

    fn register_room(&self, state: &mut State, request: &Request, body: &Value, address: &str) -> Response {
        let now = self.now();
        let code = normalize_enet_code(body.get("code"));
        let web_room = body.get("transport").and_then(Value::as_str) == Some("ws");
        let port = if web_room { 0.0 } else { js_number(body.get("port")) };
        if code.is_empty() {
            return json_response(400, &json!({ "ok": false, "error": "INVALID ROOM CODE" }), &[]);
        }
        if !web_room && !(port.is_finite() && port.fract() == 0.0 && (1.0..=65_535.0).contains(&port)) {
            return json_response(400, &json!({ "ok": false, "error": "INVALID ENET PORT" }), &[]);
        }
        let host_user = session_user(state, body);
        let reserved = state.dir.reserved.as_ref().map(|r| r.is_reserved(&code)).unwrap_or(false);
        let spawned = state.spawn_tickets.get(&code).map(|(ticket, expires)| {
            *expires > now && truthy(body.get("spawnTicket")) && safe_equal(&str_or_empty(body.get("spawnTicket")), ticket)
        }) == Some(true);
        if spawned {
            state.spawn_tickets.remove(&code);
        }
        if !reserved && !regex!(r"^[A-HJ-NP-Z2-9]{5}$").is_match(&code) {
            return json_response(400, &json!({ "ok": false, "error": "INVALID ROOM CODE" }), &[]);
        }
        if reserved && !spawned {
            let owner = !host_user.is_empty() && state.accounts.is_admin_owner(&host_user);
            let password = str_or_empty(body.get("roomPassword"));
            if !state.dir.reserved.as_ref().unwrap().has_access(&code, &host_user, &password, owner) {
                return json_response(403, &json!({ "ok": false, "code": "ROOM_RESERVED", "reserved": true,
                    "error": "THAT ROOM CODE IS RESERVED // INVITE OR ROOM PASSWORD REQUIRED" }), &[]);
            }
        }
        if let Some(live) = state.dir.enet_rooms.get(&code) {
            // a reserved room's crashed host is replaced by the next invited player once its heartbeat has gone quiet
            if !(reserved && now - live.last_seen > RESERVED_TAKEOVER_MS) {
                return json_response(409, &json!({ "ok": false, "error": "ROOM CODE ALREADY REGISTERED" }), &[]);
            }
        }
        let public = is_public_code(&code);
        if public {
            let who = Who { username: host_user.clone(), device_id: body.get("deviceId").cloned(), address: address.to_string() };
            if let Some(refused) = state.moderation.check_host_public(&who) {
                return reply(refused);
            }
        }
        let token = base64url(&random_bytes(24));
        let protocol = str_or_empty(body.get("protocol"));
        let region = if spawned {
            or_default(state.regions.region_of(&code), &str_or_empty(body.get("region")).chars().take(24).collect::<String>())
        } else {
            String::new()
        };
        let room = EnetRoom {
            code: code.clone(),
            token: token.clone(),
            address: requester_address(request, self.config.trust_proxy),
            port: port as i64,
            transport: if web_room { "ws".into() } else { "enet".into() },
            host_user,
            instance: base64url(&random_bytes(9)),
            players: finite_players(body.get("players"), 1.0).trunc(),
            session_active: truthy(body.get("sessionActive")),
            daytime: truthy(body.get("daytime")),
            game: if public {
                PUBLIC_CODES.iter().find(|(_, c)| *c == code).map(|(g, _)| g.to_string()).unwrap_or_default()
            } else {
                clean_game(body.get("game"))
            },
            expires_at: now + if public { PUBLIC_ENET_TTL_MS } else { self.config.enet_room_ttl_ms },
            last_seen: now,
            protocol: if regex!(r"^[a-f0-9]{8}$").is_match(&protocol) { protocol } else { String::new() },
            dedicated: spawned,
            region,
            reserved,
        };
        let view = public_enet_room(&room, now, &self.config.relay_address, self.config.relay_port);
        state.dir.enet_rooms.insert(code.clone(), room);
        let mut out = Map::new();
        out.insert("ok".into(), json!(true));
        out.insert("token".into(), json!(token));
        out.insert("room".into(), view);
        if reserved {
            // a new host starts clean: commands meant for the previous one are dropped
            if let Some(view) = state.dir.reserved.as_mut().unwrap().host_view(&code, None, Some(&json!(9_007_199_254_740_991i64))) {
                out.insert("reserved".into(), view);
            }
        }
        json_response(201, &Value::Object(out), &[])
    }
}

#[cfg(test)]
mod skin_lock_tests {
    use super::*;
    use crate::upstream::UPSTREAM_TIMEOUT;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    /// A website that holds its first reply until `release` fires, so a forwarded skin call stays in flight.
    fn slow_website(release: mpsc::Receiver<()>, reply: &'static str) -> Upstream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(UPSTREAM_TIMEOUT)).unwrap();
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let _ = release.recv();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        Upstream::parse(&format!("http://127.0.0.1:{port}")).unwrap()
    }

    #[test]
    fn a_save_load_in_flight_on_the_website_does_not_block_another_request() {
        let (release, released) = mpsc::channel();
        let upstream = slow_website(released, r#"{"ok":false,"code":"NO_SAVE"}"#);
        let server = Arc::new(LastRouteServer::new(ServerOptions { account_upstream: Some(upstream), ..Default::default() }));

        let loading = {
            let server = server.clone();
            std::thread::spawn(move || server.handle(&Request::new("GET", "/api/v1/saves/last-route-rust/run")))
        };
        std::thread::sleep(Duration::from_millis(200));

        let (done_tx, done_rx) = mpsc::channel();
        let health_server = server.clone();
        std::thread::spawn(move || {
            let response = health_server.handle(&Request::new("GET", "/api/health"));
            let _ = done_tx.send(response.status);
        });
        let status = done_rx.recv_timeout(Duration::from_secs(2)).expect("the health request waited on a save load");
        assert_eq!(status, 200);

        release.send(()).unwrap();
        let _ = loading.join().unwrap();
    }

    #[test]
    fn a_forwarded_skin_upload_in_flight_does_not_block_another_request() {
        let (release, released) = mpsc::channel();
        let upstream = slow_website(released, r#"{"ok":true,"id":"stored-by-website"}"#);
        let server = Arc::new(LastRouteServer::new(ServerOptions { account_upstream: Some(upstream), ..Default::default() }));

        let uploading = {
            let server = server.clone();
            std::thread::spawn(move || {
                let request = Request::json("POST", "/api/v1/skins", &json!({ "sessionToken": "token", "png": "AAAA" }));
                server.handle(&request)
            })
        };
        // Give the upload time to reach the slow website and sit there.
        std::thread::sleep(Duration::from_millis(200));

        let (done_tx, done_rx) = mpsc::channel();
        let health_server = server.clone();
        std::thread::spawn(move || {
            let response = health_server.handle(&Request::new("GET", "/api/health"));
            let _ = done_tx.send(response.status);
        });
        let status = done_rx.recv_timeout(Duration::from_secs(2)).expect("the health request waited on the skin upload");
        assert_eq!(status, 200);

        release.send(()).unwrap();
        let upload = uploading.join().unwrap();
        assert_eq!((upload.status, upload.json_body()["id"].clone()), (200, json!("stored-by-website")));
    }

    #[test]
    fn a_skin_upload_session_lookup_in_flight_does_not_block_another_request() {
        let (release, released) = mpsc::channel();
        let upstream = slow_website(released, r#"{"ok":true,"username":"KYTTU"}"#);
        let server = Arc::new(LastRouteServer::new(ServerOptions { account_upstream: Some(upstream), ..Default::default() }));

        let uploading = {
            let server = server.clone();
            std::thread::spawn(move || {
                let request = Request::new("POST", "/api/skins/upload").with_header(SKIN_TOKEN_HEADER, "token");
                server.handle(&request)
            })
        };
        // Give the session lookup time to reach the slow website and sit there.
        std::thread::sleep(Duration::from_millis(200));

        let (done_tx, done_rx) = mpsc::channel();
        let health_server = server.clone();
        std::thread::spawn(move || {
            let response = health_server.handle(&Request::new("GET", "/api/health"));
            let _ = done_tx.send(response.status);
        });
        let status = done_rx.recv_timeout(Duration::from_secs(2)).expect("the health request waited on the skin session lookup");
        assert_eq!(status, 200);

        release.send(()).unwrap();
        let upload = uploading.join().unwrap();
        assert_eq!((upload.status, upload.json_body()["code"].clone()), (503, json!("SKINS_UNAVAILABLE")));
    }
}
