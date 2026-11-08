//! Route-level tests for the account, admin, crash-report, mod-pack and save routes: session lifetime, the password
//! change and account deletion endpoints, the sign-in limiter, the server-side admin check, log redaction and the
//! route table (every POST route has its own body limit).

use super::rate_limit::{is_auth_path, limited_message, RateLimiter, AUTH_RATE_LIMIT_PATHS};
use super::routes::{account_event_line, body_limit, POST_ROUTES};
use super::*;
use crate::util::{hex, manual_clock, ManualClock};

const PASSWORD: &str = "correct horse battery";
const SETUP_PROOF: &str = "route test setup proof";

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lr_server_routes_{name}_{}", hex(&random_bytes(6))));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app(f: impl FnOnce(&mut ServerOptions)) -> LastRouteServer {
    let mut options = ServerOptions::default();
    // the tests below make many requests from one address; the limiter has its own tests
    options.api_rate_limit_max = 100_000;
    options.account_rate_limit_max = 100_000;
    options.mod_rate_limit_max = 100_000;
    f(&mut options);
    LastRouteServer::new(options)
}

fn clocked_app(f: impl FnOnce(&mut ServerOptions)) -> (LastRouteServer, ManualClock) {
    let (clock, time) = manual_clock(1_700_000_000_000);
    (app(|o| {
        o.now = clock;
        f(o);
    }), time)
}

fn post(server: &LastRouteServer, path: &str, body: Value) -> (u16, Value) {
    let response = server.handle(&Request::json("POST", path, &body));
    (response.status, response.json_body())
}

fn sign_up(server: &LastRouteServer, name: &str) -> String {
    let (status, reply) = post(server, "/api/account/register", json!({ "username": name, "password": PASSWORD }));
    assert_eq!(status, 200, "{reply}");
    reply["sessionToken"].as_str().unwrap().to_string()
}

fn sign_up_owner(server: &LastRouteServer) -> String {
    let (status, reply) = post(
        server,
        "/api/account/register",
        json!({ "username": "KYTTU", "password": PASSWORD, "adminProof": SETUP_PROOF }),
    );
    assert_eq!(status, 200, "{reply}");
    reply["sessionToken"].as_str().unwrap().to_string()
}

fn with_proof(o: &mut ServerOptions) {
    o.admin_password = SETUP_PROOF.into();
}

fn session_status(server: &LastRouteServer, token: &str) -> u16 {
    post(server, "/api/account/session", json!({ "sessionToken": token })).0
}

// --- sessions -------------------------------------------------------------------------------------------------

#[test]
fn a_session_expires_after_the_configured_lifetime_and_a_signed_out_token_is_refused() {
    let (server, time) = clocked_app(|o| o.session_ttl_ms = 10 * 60_000);
    let pc = sign_up(&server, "SESSION_PC");
    let (_, login) = post(&server, "/api/account/login", json!({ "username": "session_pc", "password": PASSWORD }));
    let laptop = login["sessionToken"].as_str().unwrap().to_string();
    assert_eq!(session_status(&server, &pc), 200);
    time.advance(9 * 60_000);
    assert_eq!(session_status(&server, &pc), 200, "still inside the lifetime");
    // sign out on the PC: its token is refused on the very next request, the laptop's is not
    assert_eq!(post(&server, "/api/account/logout", json!({ "sessionToken": pc })).0, 200);
    assert_eq!(session_status(&server, &pc), 401, "the signed-out token is refused");
    assert_eq!(post(&server, "/api/account/load", json!({ "sessionToken": pc })).0, 401);
    assert_eq!(session_status(&server, &laptop), 200);
    time.advance(2 * 60_000);
    assert_eq!(session_status(&server, &laptop), 401, "the lifetime ended");
    assert_eq!(post(&server, "/api/account/save", json!({ "sessionToken": laptop, "progress": {} })).0, 401);
}

#[test]
fn the_session_lifetime_and_the_sign_in_limit_are_environment_settings() {
    let options = |pairs: &[(&str, &str)]| {
        let env: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        options_from_environment(&env, Path::new(".")).unwrap().options
    };
    let unset = options(&[]);
    assert_eq!(unset.session_ttl_ms, 30 * 24 * 60 * 60 * 1000, "thirty days unless told otherwise");
    assert_eq!(unset.auth_rate_limit_max, 10);
    let set = options(&[("LASTROUTE_SESSION_TTL_HOURS", "2"), ("LASTROUTE_AUTH_RATE_LIMIT", "3")]);
    assert_eq!((set.session_ttl_ms, set.auth_rate_limit_max), (2 * 3_600_000, 3));
}

// --- password change and account deletion -----------------------------------------------------------------------

#[test]
fn the_password_endpoint_needs_the_current_password() {
    let server = app(|_| {});
    let token = sign_up(&server, "PW_CHANGER");
    let other = {
        let (_, login) = post(&server, "/api/account/login", json!({ "username": "PW_CHANGER", "password": PASSWORD }));
        login["sessionToken"].as_str().unwrap().to_string()
    };
    let change = |current: Option<&str>| {
        let mut body = json!({ "sessionToken": token, "password": "a brand new password" });
        if let Some(current) = current {
            body["currentPassword"] = json!(current);
        }
        post(&server, "/api/account/password", body)
    };
    assert_eq!(change(None).1["code"], json!("CURRENT_PASSWORD_REQUIRED"));
    assert_eq!(change(Some("wrong wrong")).1["code"], json!("WRONG_PASSWORD"));
    assert_eq!(post(&server, "/api/account/login", json!({ "username": "PW_CHANGER", "password": PASSWORD })).0, 200);
    assert_eq!(change(Some(PASSWORD)).0, 200);
    assert_eq!(post(&server, "/api/account/login", json!({ "username": "PW_CHANGER", "password": PASSWORD })).0, 401);
    assert_eq!(post(&server, "/api/account/login", json!({ "username": "PW_CHANGER", "password": "a brand new password" })).0, 200);
    assert_eq!(session_status(&server, &token), 200);
    assert_eq!(session_status(&server, &other), 401, "the other device is signed out");
}

fn run_save() -> Value {
    json!({ "data": { "format": 2, "saved_at": 100, "place": "LEVEL 1", "payload": { "seed_text": "S", "level": 1 } }, "updatedAt": 100 })
}

fn put_save(server: &LastRouteServer, path: &str, token: &str, body: &Value) -> (u16, Value) {
    let mut request = Request::json("PUT", path, body);
    request = request.with_header("x-session-token", token);
    let response = server.handle(&request);
    (response.status, response.json_body())
}

#[test]
fn deleting_an_account_over_http_needs_the_confirm_token_and_takes_its_saves_with_it() {
    let root = temp_root("delete");
    let saves = root.join("saves");
    let (store, saves_dir) = (root.join("accounts.json"), saves.clone());
    let server = app(move |o| {
        o.account_store_path = Some(store);
        o.save_directory = Some(saves_dir);
    });
    let token = sign_up(&server, "KEEPER_GONE");
    let path = "/api/v1/saves/last-route-rust/run";
    assert_eq!(put_save(&server, path, &token, &run_save()).0, 200);
    assert!(saves.join("KEEPER_GONE.json").exists());

    let ask = |extra: Value| {
        let mut body = json!({ "sessionToken": token, "currentPassword": PASSWORD });
        for (key, value) in extra.as_object().cloned().unwrap_or_default() {
            body[key] = value;
        }
        post(&server, "/api/account/delete", body)
    };
    assert_eq!(post(&server, "/api/account/delete", json!({ "currentPassword": PASSWORD })).0, 401, "no session");
    assert_eq!(ask(json!({ "currentPassword": "wrong wrong" })).1["code"], json!("WRONG_PASSWORD"));
    let first = ask(json!({}));
    assert_eq!((first.0, first.1["confirmRequired"].clone()), (200, json!(true)));
    assert!(saves.join("KEEPER_GONE.json").exists(), "asking for the token deletes nothing");
    assert_eq!(ask(json!({ "confirmToken": "not-the-token" })).1["code"], json!("BAD_CONFIRM_TOKEN"));
    let confirm = first.1["confirmToken"].as_str().unwrap().to_string();
    let done = ask(json!({ "confirmToken": confirm }));
    assert_eq!((done.0, done.1["deleted"].clone()), (200, json!(true)), "{}", done.1);

    assert_eq!(session_status(&server, &token), 401);
    assert_eq!(post(&server, "/api/account/login", json!({ "username": "KEEPER_GONE", "password": PASSWORD })).0, 401);
    assert!(!saves.join("KEEPER_GONE.json").exists(), "the cloud saves are deleted with the account");
    let again = sign_up(&server, "KEEPER_GONE");
    let read = server.handle(&Request::new("GET", path).with_header("x-session-token", &again)).json_body();
    assert_eq!(read["data"], Value::Null, "a new account of that name starts with no save");
    let _ = std::fs::remove_dir_all(&root);
}

// --- v1 save slots ----------------------------------------------------------------------------------------------

#[test]
fn the_v1_slot_api_only_serves_the_listed_game_and_slot_pairs_to_signed_in_accounts() {
    let root = temp_root("slots");
    let saves = root.join("saves");
    let server = app(move |o| o.save_directory = Some(saves));
    let token = sign_up(&server, "SLOT_USER");
    let body = run_save();
    for (game, slot) in crate::game_saves::ALLOWED_SAVES.iter().flat_map(|(g, slots)| slots.iter().map(move |s| (*g, *s))) {
        let path = format!("/api/v1/saves/{game}/{slot}");
        let data = if game == "night-watch-rust" { json!({ "data": { "stage": 1 }, "updatedAt": 100 }) } else { body.clone() };
        assert_eq!(put_save(&server, &path, &token, &data).0, 200, "{path}");
        assert_eq!(put_save(&server, &path, "", &data).0, 401, "{path} needs a session");
        assert_eq!(put_save(&server, &path, "forged-token", &data).0, 401, "{path} needs a live session");
    }
    let too_long = format!("/api/v1/saves/last-route-rust/{}", "x".repeat(65));
    for path in [
        "/api/v1/saves/last-route-rust/slot2",
        "/api/v1/saves/last-route-rust/run-world-2",
        "/api/v1/saves/not-a-game/run",
        "/api/v1/saves/last-route-rust/run.json",
        too_long.as_str(),
    ] {
        let (status, reply) = put_save(&server, path, &token, &body);
        assert_eq!((status, reply["code"].clone()), (404, json!("UNKNOWN_SAVE")), "{path}");
        assert_eq!(server.handle(&Request::new("GET", path).with_header("x-session-token", &token)).status, 404, "{path}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

// --- rate limits ------------------------------------------------------------------------------------------------

#[test]
fn the_sign_in_limiter_trips_with_a_clear_message_and_resets_when_the_window_ends() {
    let (server, time) = clocked_app(|o| o.auth_rate_limit_max = 3);
    let attempt = |remote: &str, path: &str| {
        let mut request = Request::json("POST", path, &json!({ "username": "nobody_here", "password": "wrong password" }));
        request.remote = remote.into();
        server.handle(&request)
    };
    for _ in 0..3 {
        assert_eq!(attempt("203.0.113.9", "/api/account/login").status, 401);
    }
    let tripped = attempt("203.0.113.9", "/api/account/login");
    assert_eq!(tripped.status, 429);
    let body = tripped.json_body();
    assert_eq!(body["code"], json!("RATE_LIMITED"));
    let message = body["error"].as_str().unwrap();
    assert!(message.starts_with("TOO MANY SIGN-IN ATTEMPTS"), "{message}");
    assert_eq!(message, message.to_uppercase(), "player-facing text is upper case");
    let wait: i64 = tripped.header("retry-after").unwrap().parse().unwrap();
    assert!((1..=60).contains(&wait), "{wait}");
    assert_eq!(body["retryAfterSeconds"], json!(wait));
    // sign-up shares the bucket, another address and ordinary routes do not
    assert_eq!(attempt("203.0.113.9", "/api/account/register").status, 429);
    assert_eq!(attempt("203.0.113.10", "/api/account/login").status, 401);
    assert_eq!(server.handle(&Request::new("GET", "/api/health")).status, 200);
    // the window ends and the same address may try again
    time.advance(61_000);
    assert_eq!(attempt("203.0.113.9", "/api/account/login").status, 401);
}

#[test]
fn the_limiter_counts_per_key_resets_by_key_and_forgets_old_windows() {
    let mut limiter = RateLimiter::new();
    for _ in 0..2 {
        assert_eq!(limiter.hit("auth|a", 2, 60_000, 1_000), 0);
    }
    let wait = limiter.hit("auth|a", 2, 60_000, 1_000);
    assert!(wait > 0 && wait <= 60, "{wait}");
    assert_eq!(limiter.hit("auth|b", 2, 60_000, 1_000), 0, "another key has its own count");
    limiter.reset("auth|a");
    assert_eq!(limiter.hit("auth|a", 2, 60_000, 1_000), 0, "a reset key starts again");
    // the window rolls over by itself
    for _ in 0..2 {
        limiter.hit("auth|c", 2, 1_000, 5_000);
    }
    assert!(limiter.hit("auth|c", 2, 1_000, 5_500) > 0);
    assert_eq!(limiter.hit("auth|c", 2, 1_000, 6_000), 0);
    limiter.prune(1_000_000);
    assert!(limiter.is_empty());
    assert!(limited_message(true, 7).contains("7 SECONDS") && limited_message(false, 1).contains("1 SECOND."));
    assert!(AUTH_RATE_LIMIT_PATHS.iter().all(|path| is_auth_path(path)) && !is_auth_path("/api/health"));
}

// --- admin status is decided on the server ------------------------------------------------------------------------

#[test]
fn every_admin_route_refuses_anonymous_callers_and_ordinary_accounts() {
    let root = temp_root("admin");
    let (crash, packs) = (root.join("crash"), root.join("packs.json"));
    let server = app(move |o| {
        with_proof(o);
        o.crash_report_directory = Some(crash);
        o.mod_pack_store_path = Some(packs);
    });
    let player = sign_up(&server, "JUST_A_PLAYER");
    let admin_paths: Vec<&str> = POST_ROUTES.iter().map(|(path, _)| *path).filter(|path| path.starts_with("/api/admin/")).collect();
    assert!(admin_paths.len() >= 15, "{admin_paths:?}");
    for path in &admin_paths {
        let anonymous = post(&server, path, json!({}));
        assert!(matches!(anonymous.0, 401 | 403) && anonymous.1["ok"] != json!(true), "{path} anonymous: {anonymous:?}");
        let forged = post(&server, path, json!({ "sessionToken": "forged", "adminProof": "wrong proof", "devToken": "x" }));
        assert!(matches!(forged.0, 401 | 403), "{path} forged: {forged:?}");
        let ordinary = post(&server, path, json!({ "sessionToken": player, "username": "OTHER", "action": "list" }));
        assert_eq!((ordinary.0, ordinary.1["ok"].clone()), (403, json!(false)), "{path} as a player: {ordinary:?}");
    }
    // the same body from the owner gets past the door on the routes that need no extra data
    let owner = sign_up_owner(&server);
    for path in ["/api/admin/flags", "/api/admin/roster", "/api/admin/crash-reports"] {
        assert_eq!(post(&server, path, json!({ "sessionToken": owner })).0, 200, "{path}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn crash_reports_are_listed_and_read_by_the_owner_only() {
    let root = temp_root("crash");
    let crash = root.join("crash");
    std::fs::create_dir_all(&crash).unwrap();
    let id = "20231114T221320-abcdef01";
    std::fs::write(crash.join(format!("{id}.json")), json!({ "id": id, "kind": "crash", "build": "abc1234" }).to_string()).unwrap();
    let dir = crash.clone();
    let server = app(move |o| {
        with_proof(o);
        o.crash_report_directory = Some(dir);
    });
    let owner = sign_up_owner(&server);
    let player = sign_up(&server, "CRASH_PLAYER");

    let listed = post(&server, "/api/admin/crash-reports", json!({ "sessionToken": owner }));
    assert_eq!((listed.0, listed.1["total"].clone()), (200, json!(1)));
    assert_eq!(listed.1["reports"][0]["id"], json!(id));
    let viewed = post(&server, "/api/admin/crash-reports/view", json!({ "sessionToken": owner, "id": id }));
    assert_eq!((viewed.0, viewed.1["report"]["build"].clone()), (200, json!("abc1234")));
    assert_eq!(post(&server, "/api/admin/crash-reports/view", json!({ "sessionToken": owner, "id": "20231114T221320-00000000" })).0, 404);
    for bad in ["../accounts", "..\\accounts", "", "20231114T221320-abcdef01.json"] {
        let refused = post(&server, "/api/admin/crash-reports/view", json!({ "sessionToken": owner, "id": bad }));
        assert_eq!((refused.0, refused.1["code"].clone()), (400, json!("BAD_REPORT_ID")), "{bad:?}");
    }
    for path in ["/api/admin/crash-reports", "/api/admin/crash-reports/view"] {
        for body in [json!({ "id": id }), json!({ "sessionToken": player, "id": id }), json!({ "adminProof": SETUP_PROOF, "id": id })] {
            let (status, reply) = post(&server, path, body);
            assert_eq!((status, reply["code"].clone()), (403, json!("ADMIN_OWNER_REQUIRED")), "{path}");
            assert!(reply.get("reports").is_none() && reply.get("report").is_none());
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

// --- mod packs ----------------------------------------------------------------------------------------------------

#[test]
fn a_blocked_mod_pack_is_listed_but_not_served_for_download() {
    let root = temp_root("packs");
    let store = root.join("packs.json");
    let server = app(move |o| {
        with_proof(o);
        o.mod_pack_store_path = Some(store);
        o.mod_download_base = "https://files.example.test/lr3d/mods".into();
    });
    let owner = sign_up_owner(&server);
    let player = sign_up(&server, "PACK_PLAYER");
    let sha = |text: &str| crate::util::sha256_hex(text);
    let pack = json!({
        "schema": "lr.pack/1", "id": "route_pack", "name": "Route Pack", "version": "1.0.0", "author": "KT",
        "files": [{ "path": "pack.json", "sha256": sha("{}"), "size": 2 }, { "path": "props/hall.json", "sha256": sha("hall"), "size": 4 }],
    });
    let registered = post(&server, "/api/admin/mods/register", json!({ "sessionToken": owner, "pack": pack }));
    assert_eq!(registered.0, 200, "{}", registered.1);
    let get = |path: &str| {
        let response = server.handle(&Request::new("GET", path));
        (response.status, response.json_body())
    };
    let served = get("/api/mods/packs/route_pack/download");
    assert_eq!((served.0, served.1["pack"]["id"].clone()), (200, json!("route_pack")));
    assert_eq!(get("/api/mods/packs/route_pack/download?version=1.0.0").0, 200);
    assert_eq!(get("/api/mods/packs/no_such_pack/download").0, 404);

    // a player cannot block; the owner can, and then the download is refused with the reason
    assert_eq!(post(&server, "/api/admin/mods/block", json!({ "sessionToken": player, "id": "route_pack" })).0, 403);
    assert_eq!(get("/api/mods/packs/route_pack/download").0, 200);
    let blocked = post(&server, "/api/admin/mods/block", json!({ "sessionToken": owner, "id": "route_pack", "reason": "crashes hosts" }));
    assert_eq!(blocked.0, 200, "{}", blocked.1);
    let refused = get("/api/mods/packs/route_pack/download");
    assert_eq!((refused.0, refused.1["code"].clone(), refused.1["reason"].clone()), (403, json!("PACK_BLOCKED"), json!("crashes hosts")));
    assert!(refused.1.get("pack").is_none(), "no file list leaves the server for a blocked pack");
    assert_eq!(get("/api/mods/packs/route_pack/download?version=1.0.0").0, 403);
    let listed = get("/api/mods/packs");
    assert_eq!(listed.1["packs"][0]["blocked"], json!(true), "the catalogue still says it is blocked");
    assert_eq!(post(&server, "/api/admin/mods/unblock", json!({ "sessionToken": owner, "id": "route_pack" })).0, 200);
    assert_eq!(get("/api/mods/packs/route_pack/download").0, 200);
    let _ = std::fs::remove_dir_all(&root);
}

// --- logs ---------------------------------------------------------------------------------------------------------

#[test]
fn a_log_line_never_carries_a_password_or_a_token() {
    let dirty = "account login user=ADA password=hunter2hunter2 sessionToken: abcdef1234567890abcd devToken=zzzz9999yyyy \
        currentPassword=\"old secret pw\" Authorization: Bearer abcdefghijklmnop adminProof=setup-proof-value \
        key=0123456789abcdef0123456789abcdef0123456789abcdef";
    let clean = redact_log_line(dirty);
    for secret in ["hunter2hunter2", "abcdef1234567890abcd", "zzzz9999yyyy", "abcdefghijklmnop", "setup-proof-value", "0123456789abcdef0123456789abcdef"] {
        assert!(!clean.contains(secret), "{secret} survived in: {clean}");
    }
    assert!(clean.contains("[redacted]") && clean.contains("user=ADA"), "{clean}");

    // the audit line is built from a cleaned name and an outcome: a body full of secrets adds nothing
    let body = json!({ "username": "ada", "password": "hunter2hunter2", "sessionToken": "tok-abcdef1234567890", "devToken": "dev-9999999999" });
    let failed = (401u16, json!({ "ok": false, "code": "INVALID_CREDENTIALS" }));
    for path in ["/api/account/login", "/api/account/register", "/api/account/password", "/api/account/delete"] {
        let (_, line) = account_event_line(1_700_000_000_000, path, &body, &failed, "203.0.113.5", "").unwrap();
        for secret in ["hunter2hunter2", "tok-abcdef1234567890", "dev-9999999999"] {
            assert!(!line.contains(secret) && !redact_log_line(&line).contains(secret), "{path}: {line}");
        }
    }
    // even a hostile username cannot smuggle a secret into the line
    let hostile = json!({ "username": "password=hunter2hunter2 token=abcdefgh1234" });
    let (_, line) = account_event_line(0, "/api/account/login", &hostile, &failed, "203.0.113.5", "").unwrap();
    assert!(!line.contains("hunter2hunter2") && !line.contains("abcdefgh1234"), "{line}");
}

// --- the route table ------------------------------------------------------------------------------------------------

/// Every `"/api/..."` string literal in `source` that names a whole route (not a prefix such as `"/api/enet/"`).
fn route_literals(source: &str) -> Vec<String> {
    let pattern = ::regex::Regex::new(r#""(/api/[A-Za-z0-9_/-]+)""#).unwrap();
    pattern.captures_iter(source).map(|caps| caps[1].to_string()).filter(|path| !path.ends_with('/')).collect()
}

#[test]
fn every_post_route_has_its_own_body_limit() {
    let source = include_str!("routes.rs");
    // the POST handlers: `route` (raw and skin routes), `route_posts`, `route_enet` and `route_accounts`
    let section = |from: &str, to: &str| {
        let start = source.find(from).unwrap_or_else(|| panic!("{from} moved"));
        let end = start + source[start..].find(to).unwrap_or_else(|| panic!("{to} moved"));
        source[start..end].to_string()
    };
    let mut handlers = section("pub(crate) fn route(&self", "fn save_route(");
    handlers.push_str(&section("fn route_posts(", "fn register_room("));
    let found = route_literals(&handlers);
    assert!(found.len() > 40, "the scan found only {}: the route source moved", found.len());

    let table: Vec<&str> = POST_ROUTES.iter().map(|(path, _)| *path).collect();
    for path in &found {
        assert!(table.contains(&path.as_str()), "POST route {path} has no entry in POST_ROUTES (give it a body limit)");
    }
    // no stale entries: each is a literal of the source or one of the skin account actions
    for path in &table {
        let skin_action = crate::skins::skins::SkinAction::from_path(path).is_some();
        assert!(found.iter().any(|f| f.as_str() == *path) || skin_action, "POST_ROUTES lists {path}, which no handler names");
    }
    let mut sorted = table.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), table.len(), "a route is listed twice");
    for (path, limit) in POST_ROUTES {
        assert!(*limit > 0 && *limit <= RAW_BODY_HARD_CAP, "{path} has limit {limit}");
        assert_eq!(body_limit(path), *limit, "{path}");
    }
    // the routes with a bigger body keep it, and the default one is the documented 64 KiB
    assert_eq!(body_limit("/api/admin/mods/register"), MOD_REGISTER_MAX_BODY_BYTES);
    assert_eq!(body_limit("/api/v1/crash-reports"), crate::crash_reports::CRASH_MAX_BODY_BYTES);
    assert!(body_limit("/api/v1/skins") > MAX_BODY_BYTES);
    assert_eq!(body_limit("/api/account/login"), 64 * 1024);
}

#[test]
fn a_body_over_the_limit_is_refused_with_413_on_every_json_post_route() {
    let server = app(|_| {});
    for (path, limit) in POST_ROUTES {
        // the raw routes read the bytes themselves (the skin PNG, the signed region report)
        if matches!(*path, "/api/skins/upload" | "/api/regions/report") {
            continue;
        }
        let mut request = Request::new("POST", path);
        request.body = vec![b' '; limit + 1];
        let response = server.handle(&request);
        assert_eq!(response.status, 413, "{path}");
    }
}
