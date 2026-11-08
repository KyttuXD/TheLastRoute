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
fn a_rooms_protocol_is_listed_as_the_host_sent_it_and_the_creator_reads_back_its_own() {
    // The client refuses a room whose listed `protocol` is not its own fingerprint (BUILD MISMATCH), so the directory must
    // hand back exactly what the host registered, in the shape the client sends: 8 lower-case hex digits
    // (`net/protocol.rs fingerprint()`), through register, the lookup and the heartbeat reply.
    let server = app(|_| {});
    let (status, registered) = post(&server, "/api/enet/rooms/register", json!({ "code": "RUSTA", "transport": "ws", "protocol": "0a1b2c3d", "players": 1 }));
    assert_eq!(status, 201, "{registered}");
    assert_eq!(registered["room"]["protocol"], json!("0a1b2c3d"));
    assert_eq!(get(&server, "/api/enet/rooms/RUSTA").1["room"]["protocol"], json!("0a1b2c3d"));
    let (_, beat) = post(&server, "/api/enet/rooms/heartbeat", json!({ "code": "RUSTA", "token": registered["token"], "players": 2 }));
    assert_eq!(beat["room"]["protocol"], json!("0a1b2c3d"), "a heartbeat does not change the protocol");
    // a host that sent none, or something that is not 8 lower-case hex digits, is listed unversioned ("")
    for (code, sent) in [("RUSTB", json!(null)), ("RUSTC", json!("0A1B2C3D")), ("RUSTD", json!("0a1b2c")), ("RUSTE", json!("0a1b2c3d4")), ("RUSTF", json!("zzzzzzzz"))] {
        let (status, listed) = post(&server, "/api/enet/rooms/register", json!({ "code": code, "transport": "ws", "protocol": sent }));
        assert_eq!(status, 201, "{listed}");
        assert_eq!(get(&server, &format!("/api/enet/rooms/{code}")).1["room"]["protocol"], json!(""), "{code}: {sent}");
    }
}

#[test]
fn a_spawned_dedicated_host_is_listed_dedicated_with_its_own_protocol() {
    // the owner's `POST /api/admin/regions spawn` hands a code a spawn ticket; the host that registers with it is the
    // server's own room host: listed `dedicated`, and its protocol is what that host build sent
    let server = app(|_| {});
    {
        let mut state = server.state.lock().unwrap();
        state.spawn_tickets.insert("HSTED".into(), ("ticket-1".into(), i64::MAX));
    }
    let (status, host) = post(&server, "/api/enet/rooms/register", json!({ "code": "HSTED", "transport": "ws", "protocol": "8645f524", "spawnTicket": "ticket-1" }));
    assert_eq!(status, 201, "{host}");
    let room = &get(&server, "/api/enet/rooms/HSTED").1["room"];
    assert_eq!((room["dedicated"].clone(), room["protocol"].clone(), room["transport"].clone()), (json!(true), json!("8645f524"), json!("ws")));
    // a register with no spawn ticket is a player's room, not a dedicated host
    let (_, plain) = post(&server, "/api/enet/rooms/register", json!({ "code": "PLMNB", "transport": "ws", "protocol": "0a1b2c3d" }));
    assert_eq!((plain["ok"].clone(), plain["room"]["dedicated"].clone()), (json!(true), json!(false)), "{plain}");
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

#[test]
fn a_room_kick_is_recorded_against_the_removed_players_verified_account_never_a_body_username() {
    let root = temp_root("mod_room_kick");
    let (server, _) = moderation_app(&root);
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    sign_up(&server, "VICTIM", json!({}));
    let hoster = sign_up(&server, "HOSTER", json!({}));
    let runner = sign_up(&server, "RUNNER", json!({}));
    let join = |room: &str, id: &str, name: &str, account: &str| {
        let mut body = json!({ "room": room, "playerId": id, "name": name });
        if !account.is_empty() {
            body["accountToken"] = json!(account);
        }
        let (status, joined) = post(&server, "/api/rooms/join", body);
        assert_eq!(status, 200, "{joined}");
        joined["sessionToken"].as_str().unwrap().to_string()
    };
    let kick = |room: &str, host_session: &str, target: &str| {
        post(&server, "/api/rooms/moderate", json!({ "room": room, "playerId": "host", "sessionToken": host_session,
            "action": "kick", "targetId": target, "username": "VICTIM", "reason": "speed" }))
    };
    // Fresh room codes from one host account: each kick names VICTIM in the body.
    for room in ["MODRA", "MODRB"] {
        let host_session = join(room, "host", "Ann", &hoster);
        join(room, "p1", "Bob", &runner);
        join(room, "p2", "Guesty", "");
        assert_eq!(kick(room, &host_session, "p1").0, 200);
        assert_eq!(kick(room, &host_session, "p2").0, 200, "a guest is removed, nothing is recorded");
        // a guest who mints the room is still only a guest host, and cannot moderate as someone else
        let (status, refused) = post(&server, "/api/rooms/moderate", json!({ "room": room, "playerId": "host", "sessionToken": "forged",
            "action": "ban", "targetId": "p1", "username": "VICTIM" }));
        assert_eq!((status, refused["code"].clone()), (403, json!("INVALID_SESSION")));
    }
    let (_, list) = post(&server, "/api/admin/flags", json!({ "sessionToken": owner }));
    let accounts = list["accounts"].as_array().unwrap();
    assert!(accounts.iter().all(|e| e["username"] != json!("VICTIM")), "the spoofed name is never flagged: {list}");
    let entry = accounts.iter().find(|e| e["username"] == json!("RUNNER")).expect("the removed account is recorded").clone();
    assert_eq!(entry["kicks"].as_array().unwrap().len(), 2);
    // one host account behind two room codes is one host, so it never reaches the "different hosts" signal
    assert_eq!(entry["reviews"].as_array().unwrap().iter().map(|r| r["reason"].clone()).collect::<Vec<_>>(), vec![json!("kick"), json!("kick")]);
    // A guest host (no accountToken) behind two fresh codes: the kicks are logged as `guest` and never count as
    // two different hosts, so minting codes cannot fake the stronger signal.
    let walker = sign_up(&server, "WALKER", json!({}));
    for room in ["MODRC", "MODRD"] {
        let host_session = join(room, "host", "Ann", "");
        join(room, "p1", "Wal", &walker);
        assert_eq!(kick(room, &host_session, "p1").0, 200);
    }
    let (_, list) = post(&server, "/api/admin/flags", json!({ "sessionToken": owner }));
    let entry = list["accounts"].as_array().unwrap().iter().find(|e| e["username"] == json!("WALKER")).expect("the removed account is recorded").clone();
    assert_eq!(entry["kicks"].as_array().unwrap().iter().map(|k| k["host"].clone()).collect::<Vec<_>>(), vec![json!("guest"), json!("guest")]);
    assert_eq!(entry["reviews"].as_array().unwrap().iter().map(|r| r["reason"].clone()).collect::<Vec<_>>(), vec![json!("kick"), json!("kick")]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_guest_hosted_enet_rooms_report_records_nothing_however_many_fresh_codes_it_registers() {
    let root = temp_root("mod_enet_guest");
    let (server, _) = moderation_app(&root);
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    sign_up(&server, "VICTIM", json!({}));
    for code in ["RMAAD", "RMAAE"] {
        let room = host_room(&server, code, "");
        let result = kick(&server, &room, json!({ "username": "VICTIM" }));
        assert_eq!((result["ok"].clone(), result["recorded"].clone(), result["flagged"].clone()), (json!(true), json!(false), json!(false)));
        assert_eq!(result["status"]["flagged"], json!(false));
    }
    let (_, list) = post(&server, "/api/admin/flags", json!({ "sessionToken": owner }));
    assert!(list["accounts"].as_array().unwrap().iter().all(|e| e["username"] != json!("VICTIM")), "{list}");
    assert_eq!(list["reviews"], json!([]));
    // a signed-in host's report is still recorded, keyed to its account
    let room = host_room(&server, "RMAAF", &sign_up(&server, "HOST_A", json!({})));
    assert_ne!(kick(&server, &room, json!({ "username": "VICTIM" }))["recorded"], json!(false));
    let (_, list) = post(&server, "/api/admin/flags", json!({ "sessionToken": owner }));
    assert_eq!(list["reviews"], json!(["VICTIM"]));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_client_address_is_the_entry_the_trusted_proxy_appended_and_a_direct_client_cannot_pick_one() {
    let at = |remote: &str, forwarded: &str, trust: bool, hops: usize| {
        let mut request = Request::new("GET", "/api/health").with_header("x-forwarded-for", forwarded);
        request.remote = remote.into();
        requester_address(&request, trust, hops)
    };
    // behind Caddy on this machine (loopback peer): the rightmost entry, which Caddy appended
    assert_eq!(at("127.0.0.1", "spoofed, 203.0.113.9", false, 1), "203.0.113.9");
    assert_eq!(at("::1", "1.2.3.4, 203.0.113.9", false, 1), "203.0.113.9");
    assert_eq!(at("::ffff:127.0.0.1", "203.0.113.9", false, 1), "203.0.113.9");
    // TRUST_PROXY for a proxy on another machine, and two proxies in a row
    assert_eq!(at("10.0.0.5", "1.2.3.4, 203.0.113.9", true, 1), "203.0.113.9");
    assert_eq!(at("127.0.0.1", "1.2.3.4, 203.0.113.9, 10.0.0.2", false, 2), "203.0.113.9");
    assert_eq!(at("127.0.0.1", "203.0.113.9", false, 3), "203.0.113.9", "a short header was written by proxies only");
    // a malformed or empty header falls back to the peer
    assert_eq!(at("127.0.0.1", "203.0.113.9, not-an-ip", false, 1), "127.0.0.1");
    assert_eq!(at("127.0.0.1", "203.0.113.9, ", false, 1), "127.0.0.1");
    assert_eq!(at("127.0.0.1", "", false, 1), "127.0.0.1");
    // a direct remote client never chooses its own address
    assert_eq!(at("198.51.100.4", "203.0.113.9", false, 1), "198.51.100.4");
    assert_eq!(at("198.51.100.4", "1.2.3.4, 203.0.113.9", false, 2), "198.51.100.4");
    // the setting comes from the environment, bounded
    let hops = |value: &str| {
        let env: HashMap<String, String> = [("LASTROUTE_TRUSTED_PROXY_HOPS".to_string(), value.to_string())].into_iter().collect();
        options_from_environment(&env, Path::new(".")).unwrap().options.trusted_proxy_hops
    };
    assert_eq!((hops("2"), hops("0"), hops("x"), hops("99")), (2, DEFAULT_TRUSTED_PROXY_HOPS, DEFAULT_TRUSTED_PROXY_HOPS, MAX_TRUSTED_PROXY_HOPS));
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

#[test]
fn health_reports_status_revision_and_uptime_only() {
    let server = app(|_| {});
    let (status, health) = get(&server, "/api/health");
    assert_eq!(status, 200);
    assert_eq!(health["status"], json!("ok"));
    assert!(health["uptimeSeconds"].as_u64().is_some(), "uptime is whole seconds");
    assert!(health["revision"].is_null() || health["revision"].is_string());
    assert!(health.get("token").is_none() && health.get("path").is_none() && health.get("dataDir").is_none());
}

#[test]
fn mods_base_from_the_environment_becomes_the_pack_download_base() {
    let base = |pairs: &[(&str, &str)]| {
        let env: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        options_from_environment(&env, Path::new(".")).unwrap().options.mod_download_base
    };
    assert_eq!(base(&[]), "", "unset: the catalogue has no download links, as before");
    assert_eq!(base(&[("LASTROUTE_MODS_BASE", "https://files.test/mods/")]), "https://files.test/mods");
    assert_eq!(base(&[("LASTROUTE_MOD_DOWNLOAD_BASE", "https://old.test/")]), "https://old.test");
    assert_eq!(
        base(&[("LASTROUTE_MODS_BASE", "https://new.test/mods"), ("LASTROUTE_MOD_DOWNLOAD_BASE", "https://old.test/")]),
        "https://new.test/mods",
        "LASTROUTE_MODS_BASE wins over the older name"
    );
}

// --- admin JOIN ROOM: an owner's unknown code is reserved and hosted; a guest is refused; a live code is joined, not recreated
#[test]
fn admin_join_room_reserves_and_hosts_an_unknown_code_only_for_owners() {
    let root = temp_root("admin_join");
    let r = root.clone();
    let server = app(move |o| {
        o.account_store_path = Some(r.join("accounts.json"));
        o.moderation_store_path = Some(r.join("moderation.json"));
        o.reserved_room_store_path = Some(r.join("reserved_rooms.json"));
        o.admin_password = "server setup proof".into();
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
    });
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    let ayttu = sign_up(&server, "AYTTU", json!({ "adminProof": "server setup proof" }));
    let guest = sign_up(&server, "GUESTY", json!({}));
    let create = |token: &str, code: &str| post(&server, "/api/admin/reserved-rooms", json!({ "sessionToken": token, "action": "create", "code": code }));

    // a guest cannot reserve a code, so the code stays unknown to them
    assert_eq!(create(&guest, "ADMN1").0, 403);
    assert_eq!(get(&server, "/api/enet/rooms/ADMN1").0, 404);

    // an owner reserves the unknown code once, and the owner hosts it
    assert_eq!(create(&owner, "ADMN1").0, 201);
    assert_eq!(create(&ayttu, "ADMN1").1["code"], json!("ROOM_ALREADY_RESERVED"));
    let (status, hosted) = post(&server, "/api/enet/rooms/register", json!({ "code": "ADMN1", "port": 7000, "sessionToken": owner }));
    assert_eq!(status, 201, "{hosted}");
    assert!(hosted["token"].as_str().is_some_and(|t| !t.is_empty()), "{hosted}");
    assert!(hosted["reserved"].is_object(), "{hosted}");

    // a guest is still refused the hosted reserved code
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "ADMN1", "port": 7001, "sessionToken": guest })).1["code"], json!("ROOM_RESERVED"));

    // a code already live (hosted publicly) is joined, not recreated: the owner's create is refused
    let (status, live) = post(&server, "/api/enet/rooms/register", json!({ "code": "KAT29", "port": 7002 }));
    assert_eq!(status, 201, "{live}");
    assert_eq!(create(&owner, "KAT29").1["code"], json!("ROOM_ALREADY_HOSTED"));
    assert_eq!(get(&server, "/api/enet/rooms/KAT29").1["room"]["port"], json!(7002));
    assert_eq!(post(&server, "/api/rooms/reserved/resolve", json!({ "code": "KAT29", "sessionToken": owner })).0, 404);
    let _ = std::fs::remove_dir_all(&root);
}

/// A relay hello on `code` from `socket`: the kind of the answer.
fn relay_hello(socket: &UdpSocket, relay_port: u16, code: &str, token: &str) -> u8 {
    socket.send_to(&encode_relay_frame(RelayFrame::HOST_REGISTER, 0, code, token.as_bytes()).unwrap(), ("127.0.0.1", relay_port)).unwrap();
    let mut buf = [0u8; 1500];
    let n = socket.recv(&mut buf).unwrap();
    decode_relay_frame(&buf[..n]).unwrap().kind
}

fn relay_socket() -> UdpSocket {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    socket
}

#[test]
fn the_public_route_host_registers_on_the_relay_with_no_token_or_a_wrong_one() {
    let (clock, time) = manual_clock(10_000);
    let server = app(|o| {
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
        o.now = clock;
    });
    let relay_port = server.start_enet_relay("127.0.0.1", 0).unwrap();
    // an address that never passed the HTTP host check cannot make the entry; once it did, the relay hello recreates it
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 404);
    let host = relay_socket();
    assert_eq!(relay_hello(&host, relay_port, "PUBLC", ""), RelayFrame::ERROR);
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "PUBLC", "port": 7000 })).0, 201);
    time.advance(16_000);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 404);
    assert_eq!(relay_hello(&host, relay_port, "PUBLC", ""), RelayFrame::HOST_ACK);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 200);
    // a wrong token is no longer an authorization failure
    assert_eq!(relay_hello(&host, relay_port, "PUBLC", "wrong-token"), RelayFrame::HOST_ACK);
    // the entry expires, the host registers on: the entry is back and lives 15 s from this hello
    time.advance(120_000);
    assert_eq!(relay_hello(&host, relay_port, "PUBLC", "stale-token"), RelayFrame::HOST_ACK);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 200);
    time.advance(14_000);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 200);
    time.advance(2_000);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 404);
}

#[test]
fn an_expired_public_entry_with_a_registered_token_does_not_block_a_tokenless_host() {
    let (clock, time) = manual_clock(10_000);
    let server = app(|o| {
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
        o.now = clock;
    });
    let relay_port = server.start_enet_relay("127.0.0.1", 0).unwrap();
    let (status, reg) = post(&server, "/api/enet/rooms/register", json!({ "code": "PUBLC", "port": 7000 }));
    assert_eq!(status, 201, "{reg}");
    time.advance(16_000);
    let host = relay_socket();
    assert_eq!(relay_hello(&host, relay_port, "PUBLC", "not-the-token"), RelayFrame::HOST_ACK);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 200);
}

#[test]
fn a_second_public_registrant_is_told_to_join_as_a_guest() {
    let server = app(|o| {
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
    });
    let relay_port = server.start_enet_relay("127.0.0.1", 0).unwrap();
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": "PUBLC", "port": 7000 })).0, 201);
    let first = relay_socket();
    // another address (macOS has no 127.0.0.2 by default: the case is then covered by the relay unit test only)
    let Ok(second) = UdpSocket::bind("127.0.0.2:0") else { return };
    second.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    assert_eq!(relay_hello(&first, relay_port, "PUBLC", ""), RelayFrame::HOST_ACK);
    assert_eq!(relay_hello(&second, relay_port, "PUBLC", "anything"), RelayFrame::HOST_JOIN_AS_GUEST);
    // the holder is unaffected
    assert_eq!(relay_hello(&first, relay_port, "PUBLC", ""), RelayFrame::HOST_ACK);
}

#[test]
fn a_private_room_still_refuses_a_wrong_host_token() {
    let server = app(|o| {
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
    });
    let relay_port = server.start_enet_relay("127.0.0.1", 0).unwrap();
    let (status, reg) = post(&server, "/api/enet/rooms/register", json!({ "code": "BCDEF", "port": 7000 }));
    assert_eq!(status, 201, "{reg}");
    let token = reg["token"].as_str().unwrap().to_string();
    let host = relay_socket();
    assert_eq!(relay_hello(&host, relay_port, "BCDEF", "wrong-token"), RelayFrame::ERROR);
    assert_eq!(relay_hello(&host, relay_port, "BCDEF", ""), RelayFrame::ERROR);
    assert_eq!(relay_hello(&host, relay_port, "BCDEF", &token), RelayFrame::HOST_ACK);
    // an unregistered private code has no room to host
    assert_eq!(relay_hello(&host, relay_port, "GHJKM", ""), RelayFrame::ERROR);
}

#[test]
fn a_late_host_of_a_reserved_public_code_revives_the_gated_entry_only_with_its_token() {
    let root = temp_root("late_reserved");
    let (clock, time) = manual_clock(1_793_188_800_000);
    let r = root.clone();
    let server = app(move |o| {
        o.account_store_path = Some(r.join("accounts.json"));
        o.moderation_store_path = Some(r.join("moderation.json"));
        o.reserved_room_store_path = Some(r.join("reserved_rooms.json"));
        o.reserved_room_codes = vec!["PUBLC".into()];
        o.admin_password = "server setup proof".into();
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
        o.now = clock;
    });
    let relay_port = server.start_enet_relay("127.0.0.1", 0).unwrap();
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    let (status, reg) = post(&server, "/api/enet/rooms/register", json!({ "code": "PUBLC", "port": 7000, "sessionToken": owner }));
    assert_eq!(status, 201, "{reg}");
    let token = reg["token"].as_str().unwrap().to_string();
    let host = UdpSocket::bind("127.0.0.1:0").unwrap();
    host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let register = |token: &str| {
        host.send_to(&encode_relay_frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", token.as_bytes()).unwrap(), ("127.0.0.1", relay_port)).unwrap();
        let mut buf = [0u8; 1500];
        let n = host.recv(&mut buf).unwrap();
        decode_relay_frame(&buf[..n]).unwrap().kind
    };
    // a wrong token leaves the entry expired: once the directory pruned it, even the right token finds nothing
    time.advance(16_000);
    assert_eq!(register("wrong-token"), RelayFrame::ERROR);
    let _ = get(&server, "/api/enet/rooms/PUBLC");
    assert_eq!(register(&token), RelayFrame::ERROR);
    // the host registers again, comes late, and the right token revives the gated entry (hidden without access)
    let (status, reg) = post(&server, "/api/enet/rooms/register", json!({ "code": "PUBLC", "port": 7000, "sessionToken": owner }));
    assert_eq!(status, 201, "{reg}");
    let token = reg["token"].as_str().unwrap().to_string();
    time.advance(16_000);
    assert_eq!(register(&token), RelayFrame::HOST_ACK);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 403);
    // the revived entry still carries the reserved flag, and the reservation is intact
    let (_, listed) = post(&server, "/api/admin/reserved-rooms", json!({ "sessionToken": owner, "action": "list" }));
    assert_eq!(listed["rooms"][0]["code"], json!("PUBLC"));
    let _ = std::fs::remove_dir_all(&root);
}

// --- hostless public route: who may beat, create, unregister and replace the relay host
const PUBLIC_HOSTLESS: &str = "PUBLC";

fn public_hello(host: &UdpSocket, relay_port: u16, kind: u8) -> u8 {
    host.send_to(&encode_relay_frame(kind, 0, PUBLIC_HOSTLESS, b"").unwrap(), ("127.0.0.1", relay_port)).unwrap();
    let mut buf = [0u8; 1500];
    let n = host.recv(&mut buf).unwrap();
    decode_relay_frame(&buf[..n]).unwrap().kind
}

fn hostless_app() -> (LastRouteServer, ManualClock, u16) {
    let (clock, time) = manual_clock(1_793_188_800_000);
    let server = app(move |o| {
        o.account_rate_limit_max = 10_000;
        o.api_rate_limit_max = 10_000;
        o.now = clock;
    });
    let relay_port = server.start_enet_relay("127.0.0.1", 0).unwrap();
    (server, time, relay_port)
}

fn udp_host() -> UdpSocket {
    let host = UdpSocket::bind("127.0.0.1:0").unwrap();
    host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    host
}

/// An expired directory entry whose address passed the HTTP host check: the relay may now recreate it.
fn vouched_and_expired(server: &LastRouteServer, time: &ManualClock) {
    assert_eq!(post(server, "/api/enet/rooms/register", json!({ "code": PUBLIC_HOSTLESS, "port": 7000 })).0, 201);
    time.advance(16_000);
    assert_eq!(get(server, "/api/enet/rooms/PUBLC").0, 404);
}

#[test]
fn a_spoofed_hostless_keep_alive_from_another_endpoint_does_not_keep_a_dead_host_fresh() {
    let (server, time, relay_port) = hostless_app();
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": PUBLIC_HOSTLESS, "port": 7000 })).0, 201);
    let (host, spoofer) = (udp_host(), udp_host());
    assert_eq!(public_hello(&host, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::HOST_ACK);
    host.send_to(&encode_relay_frame(RelayFrame::HOST_ALIVE, 0, PUBLIC_HOSTLESS, b"").unwrap(), ("127.0.0.1", relay_port)).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    time.advance(900);
    spoofer.send_to(&encode_relay_frame(RelayFrame::HOST_ALIVE, 0, PUBLIC_HOSTLESS, b"").unwrap(), ("127.0.0.1", relay_port)).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    time.advance(200);
    let mut gone = false;
    for _ in 0..40 {
        if get(&server, "/api/enet/rooms/PUBLC").0 == 404 {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    server.close();
    assert!(gone, "a keep-alive from another endpoint kept the dead host's route listed");
}

#[test]
fn a_banned_registrant_cannot_create_or_take_the_hostless_public_route() {
    let root = temp_root("hostless_ban");
    let (server, _time) = moderation_app(&root);
    let relay_port = server.start_enet_relay("127.0.0.1", 0).unwrap();
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": "server setup proof" }));
    let player = sign_up(&server, "MODDER", json!({}));
    post(&server, "/api/admin/flags/set", json!({ "sessionToken": owner, "username": "MODDER", "action": "ban" }));
    let (status, result) = post(&server, "/api/enet/rooms/register", json!({ "code": PUBLIC_HOSTLESS, "port": 9000, "sessionToken": player }));
    assert_eq!((status, result["code"].clone()), (403, json!("FLAGGED")));
    let host = udp_host();
    assert_eq!(public_hello(&host, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::ERROR);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 404);
    // an address nobody vouched for cannot create the entry either
    let other = UdpSocket::bind("127.0.0.2:0");
    if let Ok(other) = other {
        other.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        other.send_to(&encode_relay_frame(RelayFrame::HOST_REGISTER, 0, PUBLIC_HOSTLESS, b"").unwrap(), ("127.0.0.1", relay_port)).unwrap();
        let mut buf = [0u8; 1500];
        let n = other.recv(&mut buf).unwrap();
        assert_eq!(decode_relay_frame(&buf[..n]).unwrap().kind, RelayFrame::ERROR);
    }
    server.close();
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_relay_host_unregisters_its_relay_made_entry_and_a_tokenless_beat_only_refreshes_expiry() {
    let (server, time, relay_port) = hostless_app();
    vouched_and_expired(&server, &time);
    let host = udp_host();
    assert_eq!(public_hello(&host, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::HOST_ACK);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 200);
    let as_other = |path: &str, body: Value| {
        server.handle(&Request::json("POST", path, &body).with_header("x-forwarded-for", "9.9.9.9")).status
    };
    // a tokenless beat from the relay host's address refreshes expiry only: no player count
    let (status, beat) = post(&server, "/api/enet/rooms/heartbeat", json!({ "code": PUBLIC_HOSTLESS, "players": 9, "sessionActive": true }));
    assert_eq!((status, beat["room"]["players"].clone(), beat["room"]["sessionActive"].clone()), (200, json!(1), json!(false)));
    // other addresses can neither beat nor unregister it
    assert_eq!(as_other("/api/enet/rooms/heartbeat", json!({ "code": PUBLIC_HOSTLESS })), 403);
    assert_eq!(as_other("/api/enet/rooms/unregister", json!({ "code": PUBLIC_HOSTLESS })), 403);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 200);
    let (status, result) = post(&server, "/api/enet/rooms/unregister", json!({ "code": PUBLIC_HOSTLESS }));
    assert_eq!((status, result["removed"].clone()), (200, json!(true)));
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 404);
    server.close();
}

#[test]
fn hostless_entry_creation_is_rate_limited_per_source_address() {
    let (server, time, relay_port) = hostless_app();
    vouched_and_expired(&server, &time);
    let host = udp_host();
    for round in 0..HOSTLESS_CREATE_LIMIT {
        assert_eq!(public_hello(&host, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::HOST_ACK, "round {round}");
        assert_eq!(post(&server, "/api/enet/rooms/unregister", json!({ "code": PUBLIC_HOSTLESS })).0, 200);
    }
    assert_eq!(public_hello(&host, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::ERROR);
    time.advance(HOSTLESS_CREATE_WINDOW_MS + 1);
    assert_eq!(public_hello(&host, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::HOST_ACK);
    server.close();
}

#[test]
fn a_real_host_re_registering_from_the_same_address_on_a_new_port_replaces_the_old_host() {
    let (server, _time, relay_port) = hostless_app();
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": PUBLIC_HOSTLESS, "port": 7000 })).0, 201);
    let (old, new) = (udp_host(), udp_host());
    assert_eq!(public_hello(&old, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::HOST_ACK);
    old.send_to(&encode_relay_frame(RelayFrame::HOST_ALIVE, 0, PUBLIC_HOSTLESS, b"").unwrap(), ("127.0.0.1", relay_port)).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(public_hello(&new, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::HOST_ACK);
    server.close();
}

fn register_from(server: &LastRouteServer, forwarded: &str) -> Value {
    let request = Request::json("POST", "/api/enet/rooms/register", &json!({ "code": PUBLIC_HOSTLESS, "port": 7000 })).with_header("x-forwarded-for", forwarded);
    let response = server.handle(&request);
    assert_eq!(response.status, 201);
    response.json_body()
}

fn public_hello_with(host: &UdpSocket, relay_port: u16, payload: &[u8]) -> u8 {
    host.send_to(&encode_relay_frame(RelayFrame::HOST_REGISTER, 0, PUBLIC_HOSTLESS, payload).unwrap(), ("127.0.0.1", relay_port)).unwrap();
    let mut buf = [0u8; 1500];
    let n = host.recv(&mut buf).unwrap();
    decode_relay_frame(&buf[..n]).unwrap().kind
}

#[test]
fn a_registered_public_host_with_the_entry_token_is_accepted_from_a_different_ip_string() {
    let (server, _time, relay_port) = hostless_app();
    let token = register_from(&server, "9.9.9.9")["token"].as_str().unwrap().to_string();
    let host = udp_host();
    // no token: this address was never vouched, so the relay refuses it
    assert_eq!(public_hello_with(&host, relay_port, b""), RelayFrame::ERROR);
    assert_eq!(public_hello_with(&host, relay_port, b"wrong"), RelayFrame::ERROR);
    assert_eq!(public_hello_with(&host, relay_port, token.as_bytes()), RelayFrame::HOST_ACK);
    server.close();
}

#[test]
fn a_v4_mapped_http_address_matches_the_relay_sources_plain_ipv4() {
    assert_eq!(canonical_address("::ffff:127.0.0.1"), "127.0.0.1");
    assert_eq!(canonical_address("127.0.0.1"), "127.0.0.1");
    assert_eq!(canonical_address("not an ip"), "not an ip");
    let (server, _time, relay_port) = hostless_app();
    register_from(&server, "::ffff:127.0.0.1");
    let host = udp_host();
    assert_eq!(public_hello_with(&host, relay_port, b""), RelayFrame::HOST_ACK);
    server.close();
}

#[test]
fn a_keep_alive_from_the_relay_host_keeps_its_relay_made_entry_past_the_ttl_and_another_endpoint_does_not() {
    let alive = |frame_from_host: bool| {
        let (server, time, relay_port) = hostless_app();
        vouched_and_expired(&server, &time);
        let (host, other) = (udp_host(), udp_host());
        assert_eq!(public_hello(&host, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::HOST_ACK);
        let beat = if frame_from_host { &host } else { &other };
        // 16.2 s in steps under the relay's own 1 s host timeout
        for _ in 0..18 {
            beat.send_to(&encode_relay_frame(RelayFrame::HOST_ALIVE, 0, PUBLIC_HOSTLESS, b"").unwrap(), ("127.0.0.1", relay_port)).unwrap();
            std::thread::sleep(Duration::from_millis(60));
            time.advance(900);
        }
        let listed = get(&server, "/api/enet/rooms/PUBLC").0;
        server.close();
        listed
    };
    assert_eq!(alive(true), 200, "the live relay host's entry lapsed");
    assert_eq!(alive(false), 404, "a keep-alive from another endpoint refreshed the entry");
}

// --- POST /api/admin/account/reset-password: an owner sets another account's password
const RESET_PATH: &str = "/api/admin/account/reset-password";
// obviously fake test passwords
const TYPED_PASSWORD: &str = "fake-typed-password-9";
const OWNER_PROOF: &str = "fake setup proof";

fn reset_app(store: Option<PathBuf>) -> LastRouteServer {
    app(move |o| {
        o.account_store_path = store;
        o.admin_password = OWNER_PROOF.into();
        o.account_rate_limit_max = 10_000;
    })
}

fn login_status(server: &LastRouteServer, name: &str, password: &str) -> u16 {
    post(server, "/api/account/login", json!({ "username": name, "password": password })).0
}

#[test]
fn an_owner_resets_a_password_with_a_generated_one_shown_once_and_the_accounts_sessions_end() {
    let root = temp_root("pwreset");
    let store = root.join("accounts.json");
    let server = reset_app(Some(store.clone()));
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": OWNER_PROOF }));
    let first = sign_up(&server, "Runner", json!({}));
    let (_, second_login) = post(&server, "/api/account/login", json!({ "username": "RUNNER", "password": PASSWORD }));
    let second = second_login["sessionToken"].as_str().unwrap().to_string();
    let bystander = sign_up(&server, "BYSTANDER", json!({}));
    assert_eq!(post(&server, "/api/account/save", json!({ "sessionToken": first, "progress": { "depth": 6 } })).0, 200);

    let (status, reply) = post(&server, RESET_PATH, json!({ "sessionToken": owner, "username": " runner " }));
    assert_eq!(status, 200, "{reply}");
    assert_eq!(
        (reply["username"].clone(), reply["generated"].clone(), reply["sessionsRevoked"].clone()),
        (json!("RUNNER"), json!(true), json!(2))
    );
    let temporary = reply["temporaryPassword"].as_str().expect("a generated password is returned once").to_string();
    assert!(temporary.len() >= 12 && temporary != PASSWORD);
    assert!(reply.get("hash").is_none() && reply.get("salt").is_none());

    // every session of the account ended; others did not
    for token in [&first, &second] {
        let (status, body) = post(&server, "/api/account/session", json!({ "sessionToken": token }));
        assert_eq!((status, body["code"].clone()), (401, json!("NO_SESSION")));
    }
    assert_eq!(post(&server, "/api/account/session", json!({ "sessionToken": bystander })).0, 200);
    assert_eq!(post(&server, "/api/account/session", json!({ "sessionToken": owner })).0, 200);
    // the old password fails, the new one signs in, and the account's data is untouched
    assert_eq!(login_status(&server, "runner", PASSWORD), 401);
    let (status, login) = post(&server, "/api/account/login", json!({ "username": "runner", "password": temporary }));
    assert_eq!(status, 200);
    assert_eq!(login["progress"]["depth"], json!(6));

    // it survives a restart, and no password is written to the file
    drop(server);
    let again = reset_app(Some(store.clone()));
    assert_eq!(login_status(&again, "RUNNER", &temporary), 200);
    assert_eq!(login_status(&again, "RUNNER", PASSWORD), 401);
    let text = std::fs::read_to_string(&store).unwrap();
    assert!(!text.contains(&temporary) && !text.contains(PASSWORD));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_owner_can_choose_the_password_and_it_is_never_echoed_back() {
    let server = reset_app(None);
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": OWNER_PROOF }));
    sign_up(&server, "RUNNER", json!({}));
    let (status, reply) =
        post(&server, RESET_PATH, json!({ "sessionToken": owner, "username": "RUNNER", "newPassword": TYPED_PASSWORD }));
    assert_eq!(status, 200, "{reply}");
    assert_eq!(reply["generated"], json!(false));
    assert!(reply.get("temporaryPassword").is_none());
    assert!(!reply.to_string().contains(TYPED_PASSWORD));
    assert_eq!(login_status(&server, "RUNNER", TYPED_PASSWORD), 200);
    assert_eq!(login_status(&server, "RUNNER", PASSWORD), 401);
    // another owner can be reset by an owner too (the lost-password worst case), but never your own account
    let other = sign_up(&server, "AYTTU", json!({ "adminProof": OWNER_PROOF }));
    let (status, _) = post(&server, RESET_PATH, json!({ "sessionToken": owner, "username": "AYTTU" }));
    assert_eq!(status, 200);
    assert_eq!(post(&server, "/api/account/session", json!({ "sessionToken": other })).0, 401);
    let (status, own) = post(&server, RESET_PATH, json!({ "sessionToken": owner, "username": "kyttu" }));
    assert_eq!((status, own["code"].clone()), (400, json!("USE_PASSWORD_CHANGE")));
    assert_eq!(post(&server, "/api/account/session", json!({ "sessionToken": owner })).0, 200, "a refused reset signs nobody out");
}

#[test]
fn the_password_reset_is_for_owners_only_and_checks_its_input() {
    let server = reset_app(None);
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": OWNER_PROOF }));
    let player = sign_up(&server, "PLAYER", json!({}));
    let limited = sign_up(&server, "MODERATOR", json!({}));
    let granted = json!({ "sessionToken": owner, "username": "MODERATOR", "permissions": ["world"] });
    assert_eq!(post(&server, "/api/admin/roster/set", granted).0, 200);
    sign_up(&server, "TARGET", json!({}));

    // no token or a dead one: 401. A live session that is not an owner (even a limited admin): 403.
    let dead = [
        json!({ "username": "TARGET" }),
        json!({ "sessionToken": "", "username": "TARGET" }),
        json!({ "sessionToken": "nope", "username": "TARGET" }),
    ];
    for body in dead {
        let (status, reply) = post(&server, RESET_PATH, body);
        assert_eq!((status, reply["code"].clone()), (401, json!("NO_SESSION")));
    }
    for token in [&player, &limited] {
        let body = json!({ "sessionToken": token, "username": "TARGET", "newPassword": TYPED_PASSWORD });
        let (status, reply) = post(&server, RESET_PATH, body);
        assert_eq!((status, reply["code"].clone()), (403, json!("ADMIN_OWNER_REQUIRED")));
    }
    // the setup password alone is not an owner session
    let (status, _) = post(&server, RESET_PATH, json!({ "adminProof": OWNER_PROOF, "username": "TARGET" }));
    assert_eq!(status, 401);
    assert_eq!(login_status(&server, "TARGET", PASSWORD), 200, "nothing was changed");

    let (status, reply) = post(&server, RESET_PATH, json!({ "sessionToken": owner, "username": "NOBODY" }));
    assert_eq!((status, reply["code"].clone()), (404, json!("ACCOUNT_NOT_FOUND")));
    let (status, reply) = post(&server, RESET_PATH, json!({ "sessionToken": owner, "username": "no" }));
    assert_eq!((status, reply["code"].clone()), (400, json!("INVALID_USERNAME")));
    // a name over 24 characters is refused, not cut down to an existing account
    let long = json!({ "sessionToken": owner, "username": "TARGETxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx" });
    assert_eq!(post(&server, RESET_PATH, long).0, 400);
    for weak in [json!("short"), json!("x".repeat(201)), json!(123456), json!(""), json!(["a", "b"])] {
        let body = json!({ "sessionToken": owner, "username": "TARGET", "newPassword": weak });
        let (status, reply) = post(&server, RESET_PATH, body);
        assert_eq!((status, reply["code"].clone()), (400, json!("WEAK_PASSWORD")), "{weak}");
    }
    assert_eq!(login_status(&server, "TARGET", PASSWORD), 200, "refused requests changed nothing");
    let null_password = json!({ "sessionToken": owner, "username": "TARGET", "newPassword": null });
    assert_eq!(post(&server, RESET_PATH, null_password).0, 200, "null means generate one");
}

#[test]
fn the_password_reset_shares_the_login_rate_limit() {
    let server = app(|o| {
        o.admin_password = OWNER_PROOF.into();
        o.account_rate_limit_max = 3;
    });
    let owner = sign_up(&server, "KYTTU", json!({ "adminProof": OWNER_PROOF }));
    let attempt = || post(&server, RESET_PATH, json!({ "sessionToken": owner, "username": "NOBODY" })).0;
    assert_eq!(attempt(), 404); // the sign-up above was the first hit of the shared bucket
    assert_eq!(attempt(), 404);
    assert_eq!(attempt(), 429);
}

#[test]
fn the_reset_audit_line_names_the_owner_and_the_account_but_never_a_password_or_token() {
    let generated = "K7QF-2MXD-H9TA-WP4C";
    let body = json!({ "sessionToken": "tok-secret-token", "username": " runner ", "newPassword": TYPED_PASSWORD });
    let ok = (
        200,
        json!({ "ok": true, "username": "RUNNER", "generated": true, "temporaryPassword": generated, "sessionsRevoked": 2 }),
    );
    let (is_ok, line) = routes::account_event_line(1_700_000_000_000, RESET_PATH, &body, &ok, "203.0.113.5", "kyttu").unwrap();
    assert!(is_ok);
    assert_eq!(line, "account admin-reset-password at 2023-11-14T22:13:20.000Z user=RUNNER actor=KYTTU outcome=ok from=203.0.113.5");
    let refused = (403, json!({ "ok": false, "code": "ADMIN_OWNER_REQUIRED", "error": "Full admin account required" }));
    let (is_ok, refused_line) = routes::account_event_line(0, RESET_PATH, &body, &refused, "203.0.113.5", "PLAYER").unwrap();
    assert!(!is_ok && refused_line.contains("user=RUNNER actor=PLAYER outcome=ADMIN_OWNER_REQUIRED"), "{refused_line}");
    let anonymous = routes::account_event_line(0, RESET_PATH, &body, &refused, "203.0.113.5", "").unwrap().1;
    assert!(anonymous.contains("actor=-"), "{anonymous}");
    for text in [&line, &refused_line, &anonymous] {
        for secret in [generated, TYPED_PASSWORD, "tok-secret-token"] {
            assert!(!text.contains(secret), "{text}");
        }
    }
    // the other account events keep their old shape
    let login = (200, json!({ "ok": true, "username": "RUNNER" }));
    let old = routes::account_event_line(0, "/api/account/login", &json!({}), &login, "a", "ignored").unwrap().1;
    assert!(old.starts_with("account login at ") && !old.contains("actor="), "{old}");
}

#[test]
fn when_the_public_routes_relay_host_goes_its_guests_are_told_and_the_code_is_free_for_a_successor() {
    // The client's handover (net/public_reform.rs) relies on exactly this: the relay ends the room on the guests
    // (HOST_LEFT, they cannot be promoted), the directory forgets the code, and the next registration of it succeeds.
    let (server, time, relay_port) = hostless_app();
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": PUBLIC_HOSTLESS, "port": 7000 })).0, 201);
    let (host, guest) = (udp_host(), udp_host());
    assert_eq!(public_hello(&host, relay_port, RelayFrame::HOST_REGISTER), RelayFrame::HOST_ACK);
    host.send_to(&encode_relay_frame(RelayFrame::HOST_ALIVE, 0, PUBLIC_HOSTLESS, b"").unwrap(), ("127.0.0.1", relay_port)).unwrap();
    guest.send_to(&encode_relay_frame(RelayFrame::GUEST_JOIN, 0, PUBLIC_HOSTLESS, b"a-guest-nonce").unwrap(), ("127.0.0.1", relay_port)).unwrap();
    let mut buf = [0u8; 1500];
    let n = guest.recv(&mut buf).unwrap();
    assert_eq!(decode_relay_frame(&buf[..n]).unwrap().kind, RelayFrame::GUEST_ACK);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 200);
    // the host's keep-alive stops
    std::thread::sleep(Duration::from_millis(100));
    time.advance(1_500);
    let mut kinds = Vec::new();
    for _ in 0..4 {
        match guest.recv(&mut buf) {
            Ok(n) => kinds.push(decode_relay_frame(&buf[..n]).unwrap().kind),
            Err(_) => break,
        }
        if kinds.contains(&RelayFrame::HOST_LEFT) {
            break;
        }
    }
    assert!(kinds.contains(&RelayFrame::HOST_LEFT), "the guest is told the room is gone: {kinds:?}");
    let mut gone = false;
    for _ in 0..40 {
        if get(&server, "/api/enet/rooms/PUBLC").0 == 404 {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(gone, "the directory forgot the code");
    // a guest of the old room registers as the new host, from its own address, with no host token
    assert_eq!(post(&server, "/api/enet/rooms/register", json!({ "code": PUBLIC_HOSTLESS, "port": 7000 })).0, 201);
    assert_eq!(get(&server, "/api/enet/rooms/PUBLC").0, 200);
    server.close();
}

#[test]
fn a_non_admin_typing_an_unknown_room_code_gets_the_normal_refusal_and_no_room_is_made() {
    let server = app(|_| {});
    let (status, refused) = post(&server, "/api/rooms/join", json!({ "room": "NOSUCH", "name": "RUNNER", "playerId": "player-9", "joinOnly": true }));
    assert_eq!(status, 404, "{refused}");
    assert_eq!(refused["code"], json!("ROOM_NOT_FOUND"));
    assert_eq!(refused["error"], json!("Room not found"));
    // Claiming to be an admin in the body changes nothing: the admin standing is checked by the server.
    let (status, still) = post(&server, "/api/rooms/join", json!({ "room": "NOSUCH", "name": "RUNNER", "playerId": "player-9", "joinOnly": true, "admin": true }));
    assert_eq!((status, still["code"].clone()), (404, json!("ROOM_NOT_FOUND")));
    // The plain host flow (no joinOnly) still opens a room on a new code.
    let (status, hosted) = post(&server, "/api/rooms/join", json!({ "room": "NOSUCH", "name": "RUNNER", "playerId": "player-9" }));
    assert_eq!(status, 200, "{hosted}");
    let (status, joined) = post(&server, "/api/rooms/join", json!({ "room": "NOSUCH", "name": "OTHER", "playerId": "player-8", "joinOnly": true }));
    assert_eq!(status, 200, "{joined}");
}
