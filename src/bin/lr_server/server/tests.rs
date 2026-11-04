//! HTTP-level tests ported from server.test.mjs, reserved_rooms.test.mjs, moderation.test.mjs,
//! guest_names.test.mjs and enet_relay.test.mjs (the cases that go through server.mjs).

use super::*;
use crate::enet_relay::{decode_relay_frame, encode_relay_frame, RelayFrame};
use crate::util::{manual_clock, ManualClock};
use std::net::UdpSocket;
use std::time::Duration;
use crate::util::stringify;

fn temp_root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lr_server_{name}_{}", crate::util::hex(&random_bytes(6))));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app(f: impl FnOnce(&mut ServerOptions)) -> LastRouteServer {
    let mut options = ServerOptions::default();
    f(&mut options);
    LastRouteServer::new(options)
}

fn post(server: &LastRouteServer, path: &str, body: Value) -> (u16, Value) {
    let response = server.handle(&Request::json("POST", path, &body));
    (response.status, response.json_body())
}

fn get(server: &LastRouteServer, path: &str) -> (u16, Value) {
    let response = server.handle(&Request::new("GET", path));
    (response.status, response.json_body())
}

const PASSWORD: &str = "correct horse battery";

fn sign_up(server: &LastRouteServer, username: &str, extra: Value) -> String {
    let mut body = json!({ "username": username, "password": PASSWORD });
    if let Value::Object(map) = extra {
        for (k, v) in map {
            body[k] = v;
        }
    }
    let (status, reply) = post(server, "/api/account/register", body);
    assert_eq!(status, 200, "{reply}");
    reply["sessionToken"].as_str().unwrap().to_string()
}

#[test]
fn deployment_separates_legacy_pterodactyl_http_and_enet_allocations() {
    let env = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
    assert_eq!(deployment_ports(&env(&[("SERVER_PORT", "1105")])), (8105, 1105));
    assert_eq!(deployment_ports(&env(&[("PORT", "1105"), ("LASTROUTE_HTTP_PORT", "8105"), ("LASTROUTE_RELAY_PORT", "1105")])), (8105, 1105));
    assert_eq!(deployment_ports(&env(&[("PORT", "8787")])), (8787, 1105));
}

#[test]
fn health_register_resolve_heartbeat_and_unregister() {
    let server = app(|o| {
        o.relay_address = "relay.test".into();
        o.relay_port = 1105;
    });
    let (status, health) = get(&server, "/api/health");
    assert_eq!(status, 200);
    assert_eq!(health["rooms"], json!(0));
    assert_eq!(health["updates"], json!({}));
    let (status, registration) = post(&server, "/api/enet/rooms/register", json!({ "code": "ABCDE", "port": 8421, "players": 2 }));
    assert_eq!(status, 201);
    assert!(registration["token"].as_str().unwrap().len() > 10);
    assert_eq!(registration["room"]["address"], json!("relay.test"));
    assert_eq!(registration["room"]["transport"], json!("relay"));
    let (_, resolved) = get(&server, "/api/enet/rooms/ABCDE");
    assert_eq!((resolved["room"]["address"].clone(), resolved["room"]["port"].clone(), resolved["room"]["players"].clone()), (json!("relay.test"), json!(1105), json!(2)));
    assert!(resolved["room"].get("token").is_none());
    let token = registration["token"].clone();
    let (status, beat) = post(&server, "/api/enet/rooms/heartbeat", json!({ "code": "ABCDE", "token": token, "players": 3, "sessionActive": true }));
    assert_eq!(status, 200);
    assert_eq!(beat["room"]["sessionActive"], json!(true));
    assert_eq!(post(&server, "/api/enet/rooms/unregister", json!({ "code": "ABCDE", "token": token })).0, 200);
    assert_eq!(get(&server, "/api/enet/rooms/ABCDE").0, 404);
}

#[test]
fn rejects_collisions_invalid_input_and_forged_host_tokens() {
    let server = app(|_| {});
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "Q2W3E", "port": 8421 })).0, 201);
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "Q2W3E", "port": 8421 })).0, 409);
    assert_eq!(post(&server, "/api/enet/rooms/heartbeat", json!({ "code": "Q2W3E", "token": "forged" })).0, 403);
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "O0I1L", "port": 70000 })).0, 400);
}

#[test]
fn rooms_expire_without_a_heartbeat() {
    let (clock, time) = manual_clock(10_000);
    let server = app(|o| {
        o.enet_room_ttl_ms = 1000;
        o.now = clock;
    });
    post(&server, "/api/enet/rooms/register", json!({ "code": "ASDFG", "port": 8421 }));
    time.advance(1001);
    assert_eq!(get(&server, "/api/enet/rooms/ASDFG").0, 404);
}

#[test]
fn a_host_can_securely_release_a_room_for_relay_migration() {
    let server = app(|_| {});
    let (_, registration) = post(&server, "/api/enet/rooms/register", json!({ "code": "M2GRT", "port": 8421, "players": 2 }));
    assert_eq!(post(&server, "/api/enet/rooms/release-host", json!({ "code": "M2GRT", "token": "forged" })).0, 403);
    let (status, body) = post(&server, "/api/enet/rooms/release-host", json!({ "code": "M2GRT", "token": registration["token"] }));
    assert_eq!(status, 200);
    assert_eq!(body["migrating"], json!(false), "a server without a live relay cannot promise migration");
    assert_eq!(get(&server, "/api/enet/rooms/M2GRT").0, 404);
}

#[test]
fn a_deliberate_handoff_release_that_cannot_migrate_keeps_the_room_listed_and_its_host_in_charge() {
    let server = app(|_| {});
    let (_, registration) = post(&server, "/api/enet/rooms/register", json!({ "code": "H4NDF", "port": 8421, "players": 2 }));
    let handoff = json!({ "candidateSession": 5, "epoch": 1 });
    assert_eq!(post(&server, "/api/enet/rooms/release-host", json!({ "code": "H4NDF", "token": "forged", "handoff": handoff })).0, 403);
    let (status, result) = post(&server, "/api/enet/rooms/release-host", json!({ "code": "H4NDF", "token": registration["token"], "handoff": handoff }));
    assert_eq!(status, 200);
    assert_eq!(result["migrating"], json!(false));
    assert!(result["error"].is_string());
    assert_eq!(get(&server, "/api/enet/rooms/H4NDF").0, 200);
    assert_eq!(post(&server, "/api/enet/rooms/heartbeat", json!({ "code": "H4NDF", "token": registration["token"], "players": 2 })).0, 200);
}

#[test]
fn rooms_carry_the_game_they_belong_to_missing_or_invalid_means_last_route() {
    let server = app(|_| {});
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "GAMEA", "port": 1, "game": "city" })).1["room"]["game"], json!("city"));
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "GAMEB", "port": 1 })).1["room"]["game"], json!("last_route"));
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "GAMEC", "port": 1, "game": "Bad Game!" })).1["room"]["game"], json!("last_route"));
}

#[test]
fn rate_limits_sensitive_account_endpoints_per_requester_and_honours_trusted_proxies_only_when_enabled() {
    let server = app(|o| o.account_rate_limit_max = 2);
    let attempt = |remote: &str, forwarded: Option<&str>| {
        let mut request = Request::json("POST", "/api/account/login", &json!({ "username": "nobody", "password": "wrong-pass" }));
        request.remote = remote.into();
        if let Some(f) = forwarded {
            request.headers.insert("x-forwarded-for".into(), f.into());
        }
        server.handle(&request)
    };
    assert_eq!(attempt("203.0.113.5", None).status, 401);
    assert_eq!(attempt("203.0.113.5", None).status, 401);
    let limited = attempt("203.0.113.5", None);
    assert_eq!(limited.status, 429);
    assert_eq!(limited.json_body()["code"], json!("RATE_LIMITED"));
    assert!(limited.header("retry-after").unwrap().parse::<i64>().unwrap() >= 1);
    // an untrusted remote cannot dodge its bucket with X-Forwarded-For
    assert_eq!(attempt("203.0.113.5", Some("198.51.100.1")).status, 429);
    assert_eq!(attempt("203.0.113.6", None).status, 401, "another requester has its own bucket");
    // a loopback proxy is trusted for the client address
    assert_eq!(attempt("127.0.0.1", Some("198.51.100.9")).status, 401);
}

#[test]
fn serves_validated_debug_release_update_manifests() {
    let sha = "c".repeat(64);
    let server = app(|o| {
        o.updates = json!({
            "debug": { "windows": { "version": "v0.0.1.0", "file": "KyttusLab-Windows-Debug.zip", "sha256": sha, "revision": "abcdef1", "build": 12, "bytes": 99 } },
            "release": { "linux": { "version": "1.0.0", "url": "http://insecure", "sha256": sha, "revision": "abcdef1", "build": 1 },
                         "web": { "version": "1.0.0", "file": "web.zip", "sha256": sha } },
        });
    });
    let (status, body) = get(&server, "/api/updates/debug/windows");
    assert_eq!(status, 200);
    assert_eq!(body["update"]["version"], json!("0.0.1.0"));
    assert_eq!(body["update"]["url"], json!("/api/updates/files/KyttusLab-Windows-Debug.zip"));
    assert_eq!(body["update"]["build"], json!(12));
    assert_eq!(get(&server, "/api/updates/release/linux").0, 404, "a non-https url without a file is refused");
    assert_eq!(get(&server, "/api/updates/release/web").0, 200, "web needs no build number");
    let (_, health) = get(&server, "/api/health");
    assert_eq!(health["updates"]["debug"], json!({ "version": "0.0.1.0", "revision": "abcdef1", "platforms": ["windows"] }));
    assert_eq!(health["updates"]["release"]["revision"], json!("unreported"));
    assert_eq!(get(&server, "/api/updates/files/../../secret").0, 404);
}

#[test]
fn serves_static_files_and_only_safe_png_names_from_the_custom_map_directory() {
    let root = temp_root("static");
    let web = root.join("web");
    let maps = root.join("maps");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::write(web.join("index.html"), "<html></html>").unwrap();
    std::fs::write(web.join("game.wasm"), [0u8, 97, 115, 109]).unwrap();
    std::fs::write(maps.join("floor_1.png"), [137u8, 80, 78, 71]).unwrap();
    std::fs::write(root.join("secret.txt"), "no").unwrap();
    let (web2, maps2) = (web.clone(), maps.clone());
    let server = app(move |o| {
        o.public_directory = Some(web2);
        o.custom_map_directory = Some(maps2);
    });
    let index = server.handle(&Request::new("GET", "/"));
    assert_eq!((index.status, index.header("content-type")), (200, Some("text/html; charset=utf-8")));
    assert_eq!(server.handle(&Request::new("GET", "/game.wasm")).header("content-type"), Some("application/wasm"));
    assert_eq!(server.handle(&Request::new("GET", "/../secret.txt")).status, 404);
    assert_eq!(server.handle(&Request::new("GET", "/%2e%2e/secret.txt")).status, 404);
    let map = server.handle(&Request::new("GET", "/api/custom-maps/floor_1.png"));
    assert_eq!((map.status, map.header("content-type")), (200, Some("image/png")));
    assert_eq!(server.handle(&Request::new("GET", "/api/custom-maps/secret.txt")).status, 404);
    assert_eq!(server.handle(&Request::new("GET", "/api/custom-maps/missing.png")).json_body()["error"], json!("CUSTOM MAP NOT FOUND"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn developer_tokens_expire_and_dev_reload_is_rate_limited_as_a_sensitive_endpoint() {
    let (clock, time) = manual_clock(1_000_000);
    let server = app(|o| {
        o.admin_password = "setup secret".into();
        o.dev_token_ttl_ms = 2_000;
        o.account_rate_limit_max = 4;
        o.now = clock;
    });
    assert_eq!(post(&server, "/api/dev/login", json!({ "password": "nope" })).0, 401);
    let (_, login) = post(&server, "/api/dev/login", json!({ "password": "setup secret" }));
    let token = login["devToken"].clone();
    assert_eq!(post(&server, "/api/dev/reload", json!({ "devToken": token })).1["reloadId"], json!(1));
    time.advance(2_001);
    assert_eq!(post(&server, "/api/dev/reload", json!({ "devToken": token })).0, 401, "an expired dev token is refused");
    assert_eq!(post(&server, "/api/dev/reload", json!({ "adminProof": "guess" })).0, 429);
}

#[test]
fn account_api_persists_sessions_and_one_save_slot_across_restarts() {
    let root = temp_root("accounts");
    let store = root.join("accounts.json");
    let s = store.clone();
    let server = app(move |o| o.account_store_path = Some(s));
    let token = sign_up(&server, "Runner_1", json!({}));
    assert_eq!(post(&server, "/api/account/save", json!({ "sessionToken": token, "progress": { "depth": 4 } })).0, 200);
    drop(server);
    let s = store.clone();
    let again = app(move |o| o.account_store_path = Some(s));
    let (status, session) = post(&again, "/api/account/session", json!({ "sessionToken": token }));
    assert_eq!(status, 200);
    assert_eq!(session["username"], json!("RUNNER_1"));
    assert_eq!(post(&again, "/api/account/load", json!({ "sessionToken": token })).1["progress"]["depth"], json!(4));
    assert!(!std::fs::read_to_string(&store).unwrap().contains(&token), "no usable bearer token is written");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn room_bound_admin_tickets_prove_kyttu_without_exposing_an_account_session() {
    let root = temp_root("tickets");
    let s = root.join("accounts.json");
    let server = app(move |o| {
        o.account_store_path = Some(s);
        o.admin_password = "server setup proof".into();
    });
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    let player = sign_up(&server, "PLAYER", json!({}));
    let (_, room) = post(&server, "/api/enet/rooms/register", json!({ "code": "TCKTS", "port": 9000 }));
    assert_eq!(post(&server, "/api/enet/admin-tickets/issue", json!({ "code": "TCKTS", "sessionToken": player })).0, 403);
    let (status, issued) = post(&server, "/api/enet/admin-tickets/issue", json!({ "code": "TCKTS", "sessionToken": owner }));
    assert_eq!(status, 201);
    assert_eq!(issued["permissions"], json!(["hunters", "inventory", "player", "visual", "world"]));
    assert!(!stringify(&issued).contains(&owner));
    let ticket = issued["ticket"].clone();
    assert_eq!(post(&server, "/api/enet/admin-tickets/verify", json!({ "code": "TCKTS", "hostToken": "forged", "ticket": ticket })).0, 403);
    let (status, verified) = post(&server, "/api/enet/admin-tickets/verify", json!({ "code": "TCKTS", "hostToken": room["token"], "ticket": ticket }));
    assert_eq!(status, 200);
    assert_eq!((verified["authorized"].clone(), verified["owner"].clone()), (json!(true), json!(true)));
    post(&server, "/api/enet/rooms/unregister", json!({ "code": "TCKTS", "token": room["token"] }));
    post(&server, "/api/enet/rooms/register", json!({ "code": "TCKTS", "port": 9000 }));
    let (_, again) = post(&server, "/api/enet/rooms/register", json!({ "code": "TCKTT", "port": 9000 }));
    assert_eq!(post(&server, "/api/enet/admin-tickets/verify", json!({ "code": "TCKTT", "hostToken": again["token"], "ticket": ticket })).0, 403,
        "a ticket is bound to its room");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn legacy_room_api_joins_players_and_guest_names_are_issued_over_http() {
    let server = app(|_| {});
    let (status, joined) = post(&server, "/api/rooms/join", json!({ "room": "PRIV1", "name": "RUNNER", "playerId": "player-1" }));
    assert_eq!(status, 200, "{joined}");
    if joined.get("ok").is_none() {
        assert_eq!(joined["reloadId"], json!(0), "room replies without `ok` carry the reload id");
    }
    let (status, first) = post(&server, "/api/guest/name", json!({}));
    assert_eq!(status, 200);
    let name = first["name"].as_str().unwrap().to_string();
    assert!(Regex::new(r"^[A-HJ-NP-Z]-[2-9]{4}$").unwrap().is_match(&name));
    let (_, kept) = post(&server, "/api/guest/name", json!({ "previous": name, "token": first["token"] }));
    assert_eq!(kept["name"], json!(name));
    let (_, other) = post(&server, "/api/guest/name", json!({ "previous": name }));
    assert_ne!(other["name"], json!(name), "a held name is never issued to someone else");
}

#[test]
fn cloud_saves_route_through_the_session_header() {
    let root = temp_root("saves");
    let (a, d) = (root.join("accounts.json"), root.join("saves"));
    let server = app(move |o| {
        o.account_store_path = Some(a);
        o.save_directory = Some(d);
    });
    let token = sign_up(&server, "SAVER", json!({}));
    let put = |body: &str, token: &str| {
        let mut request = Request::new("PUT", "/api/v1/saves/last-route-godot/run").with_header("x-session-token", token);
        request.body = body.as_bytes().to_vec();
        let response = server.handle(&request);
        (response.status, response.json_body())
    };
    assert_eq!(put(r#"{"data":{"depth":3},"updatedAt":100}"#, "").0, 401);
    assert_eq!(put("not json", &token).1["code"], json!("BAD_SAVE"));
    assert_eq!(put(r#"{"data":{"depth":3},"updatedAt":100}"#, &token).0, 200);
    assert_eq!(put(r#"{"data":{"depth":2},"updatedAt":50}"#, &token).1["code"], json!("STALE"));
    let big = format!(r#"{{"data":{{"x":"{}"}}}}"#, "a".repeat(70 * 1024));
    assert_eq!(put(&big, &token).1["code"], json!("SAVE_TOO_LARGE"));
    let got = server.handle(&Request::new("GET", "/api/v1/saves/last-route-godot/run").with_header("x-session-token", &token));
    assert_eq!(got.json_body()["data"]["depth"], json!(3));
    let _ = std::fs::remove_dir_all(&root);
}

// --- reserved_rooms.test.mjs: "reserved codes: hidden from resolve, hosted and joined only with access, taken over when the host dies"
#[test]
fn reserved_codes_hidden_from_resolve_hosted_and_joined_only_with_access_taken_over_when_the_host_dies() {
    let root = temp_root("reserved");
    let (clock, time): (Clock, ManualClock) = manual_clock(1_793_188_800_000);
    let r = root.clone();
    let server = app(move |o| {
        o.account_store_path = Some(r.join("accounts.json"));
        o.moderation_store_path = Some(r.join("moderation.json"));
        o.reserved_room_store_path = Some(r.join("reserved_rooms.json"));
        o.reserved_room_codes = vec!["KTPRV".into(), "AKIRI".into()];
        o.admin_password = "server setup proof".into();
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
        o.now = clock;
    });
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    let alice = sign_up(&server, "ALICE", json!({}));
    let mallory = sign_up(&server, "MALLORY", json!({}));
    assert_eq!(post(&server, "/api/admin/reserved-rooms", json!({ "sessionToken": alice, "action": "list" })).0, 403);
    let (_, listed) = post(&server, "/api/admin/reserved-rooms", json!({ "sessionToken": owner, "action": "list" }));
    let codes: Vec<_> = listed["rooms"].as_array().unwrap().iter().map(|r| r["code"].clone()).collect();
    assert_eq!(codes, vec![json!("KTPRV"), json!("AKIRI")]);
    post(&server, "/api/admin/reserved-rooms", json!({ "sessionToken": owner, "action": "update", "code": "KTPRV",
        "invites": ["alice"], "password": "room pass 1", "settings": { "seed": "KT SEED" } }));
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "KTPRV", "port": 7000, "sessionToken": mallory })).1["code"], json!("ROOM_RESERVED"));
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "KTPRV", "port": 7000 })).0, 403);
    let (status, hosted) = post(&server, "/api/enet/rooms/register", json!({ "code": "KTPRV", "port": 7000, "sessionToken": alice }));
    assert_eq!(status, 201, "{hosted}");
    assert_eq!(hosted["reserved"]["settings"]["seed"], json!("KT SEED"));
    assert_eq!(get(&server, "/api/enet/rooms/KTPRV").1["code"], json!("ROOM_RESERVED"));
    assert_eq!(post(&server, "/api/rooms/reserved/resolve", json!({ "code": "KTPRV", "sessionToken": mallory })).0, 403);
    let (status, by_password) = post(&server, "/api/rooms/reserved/resolve", json!({ "code": "KTPRV", "password": "room pass 1" }));
    assert_eq!(status, 200);
    assert!(by_password["live"] == json!(true) && by_password["joinTicket"].as_str().unwrap().len() > 10 && by_password["room"]["port"] == json!(7000));
    assert_eq!(by_password["canHost"], json!(false));
    post(&server, "/api/admin/reserved-rooms", json!({ "sessionToken": owner, "action": "command", "code": "KTPRV", "command": "restart" }));
    assert_eq!(post(&server, "/api/admin/reserved-rooms", json!({ "sessionToken": owner, "action": "command", "code": "AKIRI", "command": "end" })).1["code"], json!("ROOM_NOT_LIVE"));
    let (_, beat) = post(&server, "/api/enet/rooms/heartbeat", json!({ "code": "KTPRV", "token": hosted["token"], "players": 2 }));
    assert_eq!(beat["reserved"]["commands"].as_array().unwrap().iter().map(|c| c["command"].clone()).collect::<Vec<_>>(), vec![json!("restart")]);
    let seq = beat["reserved"]["commands"][0]["seq"].clone();
    let (_, beat) = post(&server, "/api/enet/rooms/heartbeat", json!({ "code": "KTPRV", "token": hosted["token"], "commandAck": seq }));
    assert_eq!(beat["reserved"]["commands"], json!([]));
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "KTPRV", "port": 7001, "sessionToken": owner })).0, 409);
    time.advance(31_000);
    assert_eq!(post(&server, "/api/rooms/reserved/resolve", json!({ "code": "KTPRV", "sessionToken": alice })).1["canHost"], json!(true));
    post(&server, "/api/admin/reserved-rooms", json!({ "sessionToken": owner, "action": "command", "code": "KTPRV", "command": "end" }));
    let (status, takeover) = post(&server, "/api/enet/rooms/register", json!({ "code": "KTPRV", "port": 7002, "roomPassword": "room pass 1" }));
    assert_eq!(status, 201);
    assert_eq!(takeover["reserved"]["commands"], json!([]));
    assert_ne!(takeover["token"], hosted["token"]);
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "ABCDE", "port": 7003 })).0, 201);
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "OIOIO", "port": 7004 })).0, 400);
    assert_eq!(get(&server, "/api/enet/rooms/ABCDE").0, 200);
    let _ = std::fs::remove_dir_all(&root);
}

fn moderation_app(root: &Path) -> (LastRouteServer, ManualClock) {
    let (clock, time) = manual_clock(crate::util::parse_iso_time("2026-10-28T12:00:00Z").unwrap());
    let r = root.to_path_buf();
    let server = app(move |o| {
        o.account_store_path = Some(r.join("accounts.json"));
        o.moderation_store_path = Some(r.join("moderation.json"));
        o.admin_password = "server setup proof".into();
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
        o.enet_room_ttl_ms = 10 * 24 * 3_600_000;
        o.updates = json!({ "release": { "windows": { "version": "1.0.0", "file": "w.zip", "sha256": "b".repeat(64), "build": 3, "revision": "abcdef1" } } });
        o.now = clock;
    });
    (server, time)
}

fn host_room(server: &LastRouteServer, code: &str, session: &str) -> Value {
    let (status, body) = post(server, "/api/enet/rooms/register", json!({ "code": code, "port": 9000, "sessionToken": session }));
    assert_eq!(status, 201, "{body}");
    json!({ "code": code, "hostToken": body["token"] })
}

fn kick(server: &LastRouteServer, room: &Value, extra: Value) -> Value {
    let mut body = json!({ "code": room["code"], "hostToken": room["hostToken"], "kind": "kick", "reason": "speed", "strikes": 3 });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    post(server, "/api/moderation/report", body).1
}

// --- moderation.test.mjs (HTTP)
#[test]
fn kicks_never_restrict_an_account_they_open_review_entries_2_distinct_hosts_marked_as_the_stronger_signal() {
    let root = temp_root("mod_kicks");
    let (server, _) = moderation_app(&root);
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    let player = sign_up(&server, "CHEATER", json!({}));
    let a = host_room(&server, "RMAAA", &sign_up(&server, "HOST_A", json!({})));
    let b = host_room(&server, "RMAAB", &sign_up(&server, "HOST_B", json!({})));
    for room in [&a, &a, &b, &b] {
        let result = kick(&server, room, json!({ "username": "cheater" }));
        assert_eq!(result["flagged"], json!(false));
        assert_eq!(result["status"]["restrictions"], json!({ "publicRoute": false, "hostPublic": false, "skinUpload": false }));
    }
    let (_, status) = post(&server, "/api/moderation/status", json!({ "sessionToken": player }));
    assert_eq!((status["status"]["flagged"].clone(), status["status"]["message"].clone()), (json!(false), json!("")));
    assert_eq!(post(&server, "/api/public/join", json!({ "playerKey": "0123456789abcdef", "name": "RUNNER", "sessionToken": player })).0, 200);
    let (_, list) = post(&server, "/api/admin/flags", json!({ "sessionToken": owner }));
    assert_eq!(list["reviews"], json!(["CHEATER"]));
    let entry = list["accounts"].as_array().unwrap().iter().find(|e| e["username"] == json!("CHEATER")).unwrap().clone();
    assert_eq!(entry["reviewOpen"], json!(4));
    assert_eq!(entry["reviews"].as_array().unwrap().iter().map(|r| r["reason"].clone()).collect::<Vec<_>>(), vec![json!("kick"), json!("kick"), json!("kicks"), json!("kicks")]);
    assert_eq!(entry["kicks"].as_array().unwrap().len(), 4);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn owner_flagged_accounts_cannot_join_or_host_public_private_rooms_still_work() {
    let root = temp_root("mod_flags");
    let (server, time) = moderation_app(&root);
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    let player = sign_up(&server, "MODDER", json!({}));
    post(&server, "/api/admin/flags/set", json!({ "sessionToken": owner, "username": "MODDER", "action": "extend", "hours": 24 }));
    let (status, result) = post(&server, "/api/public/join", json!({ "playerKey": "0123456789abcdef", "name": "RUNNER", "sessionToken": player }));
    assert_eq!(status, 403);
    assert_eq!((result["code"].clone(), result["permanent"].clone()), (json!("FLAGGED"), json!(false)));
    assert_eq!(result["until"], json!(time.get() + 24 * 3_600_000));
    assert!(result["error"].as_str().unwrap().contains("Public Route"));
    let (status, result) = post(&server, "/api/enet/rooms/register", json!({ "code": "PUBLC", "port": 9000, "sessionToken": player }));
    assert_eq!((status, result["code"].clone()), (403, json!("FLAGGED")));
    let upload = server.handle(&Request::new("POST", "/api/skins/upload").with_header("x-session-token", &player));
    assert_eq!((upload.status, upload.json_body()["code"].clone()), (403, json!("FLAGGED")));
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "PRVAT", "port": 9000, "sessionToken": player })).0, 201);
    assert_ne!(post(&server, "/api/rooms/join", json!({ "roomCode": "PRIV1", "name": "MODDER" })).0, 403);
    assert_eq!(post(&server, "/api/public/join", json!({ "playerKey": "0123456789abcdef", "name": "RUNNER" })).0, 200);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn peers_and_report_require_the_rooms_host_token_and_admin_endpoints_require_a_full_owner() {
    let root = temp_root("mod_peers");
    let (server, _) = moderation_app(&root);
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    let player = sign_up(&server, "PRANKER", json!({}));
    let room = host_room(&server, "RMAAC", "");
    assert_eq!(post(&server, "/api/moderation/peers", json!({ "code": "RMAAC", "hostToken": "wrong", "usernames": ["PRANKER"] })).0, 403);
    assert_eq!(post(&server, "/api/moderation/report", json!({ "code": "RMAAC", "kind": "kick", "username": "PRANKER" })).0, 403);
    assert_eq!(post(&server, "/api/moderation/peers", json!({ "code": "ZZZZZ", "hostToken": room["hostToken"] })).0, 403);
    post(&server, "/api/admin/flags/set", json!({ "sessionToken": owner, "username": "PRANKER", "action": "troll_on" }));
    let (status, peers) = post(&server, "/api/moderation/peers", json!({ "code": "RMAAC", "hostToken": room["hostToken"], "usernames": ["pranker", "KYTTU"], "deviceIds": ["dev-x"] }));
    assert_eq!(status, 200);
    assert_eq!(peers["players"]["PRANKER"], json!({ "flagged": false, "permanent": false, "troll": true, "ads": true, "reason": "" }));
    assert_eq!(peers["devices"], json!({ "dev-x": { "cooldown": false, "troll": false } }));
    for path in ["/api/admin/flags", "/api/admin/flags/set", "/api/admin/builds/add", "/api/admin/builds/remove"] {
        assert_eq!(post(&server, path, json!({ "sessionToken": player, "username": "PRANKER", "action": "ban" })).0, 403, "{path}");
        assert_eq!(post(&server, path, json!({})).1["code"], json!("ADMIN_OWNER_REQUIRED"), "{path}");
    }
    // a ban never blocks sign-in; a rename carries the record
    post(&server, "/api/admin/flags/set", json!({ "sessionToken": owner, "username": "PRANKER", "action": "ban" }));
    assert_eq!(post(&server, "/api/account/session", json!({ "sessionToken": player })).0, 200);
    assert_eq!(post(&server, "/api/account/rename", json!({ "sessionToken": player, "username": "NEW_NAME" })).0, 200);
    assert_eq!(post(&server, "/api/moderation/status", json!({ "sessionToken": player })).1["status"]["permanent"], json!(true));
    let stored = std::fs::read_to_string(root.join("moderation.json")).unwrap();
    assert!(stored.contains("NEW_NAME") && !stored.contains("127.0.0.1"));
    let _ = std::fs::remove_dir_all(&root);
}

// --- enet_relay.test.mjs: "server: an explicit quit invalidates the code at once; a killed relay host's code is gone within ~1 s"
#[test]
fn server_an_explicit_quit_invalidates_the_code_at_once_a_killed_relay_hosts_code_is_gone_within_about_1_s() {
    let server = app(|o| {
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
    });
    let relay_port = server.start_enet_relay("127.0.0.1", 0).unwrap();
    let (status, first) = post(&server, "/api/enet/rooms/register", json!({ "code": "ABCDE", "port": 7000 }));
    assert_eq!(status, 201);
    assert_eq!(post(&server, "/api/enet/rooms/unregister", json!({ "code": "ABCDE", "token": first["token"] })).0, 200);
    assert_eq!(get(&server, "/api/enet/rooms/ABCDE").0, 404, "a quit room still resolved");
    let (_, second) = post(&server, "/api/enet/rooms/register", json!({ "code": "BCDEF", "port": 7001 }));
    let token = second["token"].as_str().unwrap().to_string();
    let host = UdpSocket::bind("127.0.0.1:0").unwrap();
    host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let target = ("127.0.0.1", relay_port);
    host.send_to(&encode_relay_frame(RelayFrame::HOST_REGISTER, 0, "BCDEF", token.as_bytes()).unwrap(), target).unwrap();
    let mut buf = [0u8; 1500];
    let n = host.recv(&mut buf).unwrap();
    assert_eq!(decode_relay_frame(&buf[..n]).unwrap().kind, RelayFrame::HOST_ACK);
    host.send_to(&encode_relay_frame(RelayFrame::HOST_ALIVE, 0, "BCDEF", token.as_bytes()).unwrap(), target).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(get(&server, "/api/enet/rooms/BCDEF").0, 200);
    let killed = std::time::Instant::now();
    let mut gone = None;
    while killed.elapsed() < Duration::from_secs(3) {
        if get(&server, "/api/enet/rooms/BCDEF").0 == 404 {
            gone = Some(killed.elapsed());
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    server.close();
    let gone = gone.expect("a killed host's room never dropped");
    assert!(gone <= Duration::from_millis(1400), "a killed host's room took {gone:?} to drop (want about 1 s)");
}

#[test]
fn http_transport_end_to_end_over_tiny_http() {
    use std::io::{Read, Write};
    let server = Arc::new(app(|_| {}));
    let http = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
    let port = http.server_addr().to_ip().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let threads = server.serve(http, 2, stop.clone());
    let exchange = |raw: String| {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(raw.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    };
    let body = r#"{"code":"WXYZ2","port":8421}"#;
    let response = exchange(format!("POST /api/enet/rooms/register HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()));
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    assert!(response.to_ascii_lowercase().contains("access-control-allow-origin: *"));
    let health = exchange("GET /api/health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n".into());
    assert!(health.contains(&format!("\"port\":{port}")) && health.contains("\"rooms\":1"), "{health}");
    let bad = exchange("POST /api/account/login HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\nConnection: close\r\n\r\n[1]".into());
    assert!(bad.starts_with("HTTP/1.1 400") && bad.contains("INVALID JSON BODY"), "{bad}");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn the_service_token_counts_only_at_32_characters_and_is_trimmed() {
    let with = |value: &str| HashMap::from([("LASTROUTE_SERVICE_TOKEN".to_string(), value.to_string())]);
    assert_eq!(service_token_from(&HashMap::new()), None);
    assert_eq!(service_token_from(&with(&"s".repeat(31))), None, "a 31-character token is treated as unset");
    assert_eq!(service_token_from(&with(&format!("  {}  ", "s".repeat(32)))), Some("s".repeat(32)));
}

#[test]
fn the_service_token_reaches_the_server_options_only_with_an_upstream_and_a_long_enough_token() {
    let base = |token: Option<String>| {
        let mut env = HashMap::from([("LASTROUTE_ACCOUNT_UPSTREAM".to_string(), "http://127.0.0.1:1".to_string())]);
        if let Some(token) = token {
            env.insert("LASTROUTE_SERVICE_TOKEN".to_string(), token);
        }
        options_from_environment(&env, Path::new(".")).unwrap().options.account_service_token
    };
    assert_eq!(base(None), None, "no token: the account check sends none");
    assert_eq!(base(Some("x".repeat(31))), None, "a short token is unset");
    assert_eq!(base(Some("x".repeat(40))), Some("x".repeat(40)));
}
