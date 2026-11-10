//! HTTP-level tests of the story server: the health route, the guest-name and account identities, and the room routes.

use super::*;
use crate::guest_names::LEASE_MS;
use crate::packs::PackRegistry;
use crate::util::{hex, manual_clock, random_bytes, ManualClock};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

struct Harness {
    server: StoryServer,
    clock: ManualClock,
    root: PathBuf,
}

fn harness(name: &str) -> Harness {
    let root = std::env::temp_dir().join(format!("lr_story_http_{name}_{}", hex(&random_bytes(6))));
    let folder = root.join("mods").join("party_pack");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("pack.json"), r#"{"id":"party_pack","version":"1.0.0","story":{"multiplayer":true}}"#).unwrap();
    fs::write(folder.join("scenes.json"), "{}").unwrap();
    let (clock, handle) = manual_clock(1_700_000_000_000);
    let server = StoryServer::new(ServerOptions {
        data_dir: root.join("data"),
        packs_dir: root.join("mods"),
        now: clock,
        trust_proxy: false,
        rate_limit_window_ms: 60_000,
        api_rate_limit_max: 1000,
        account_rate_limit_max: 1000,
        account_upstream: None,
    });
    Harness { server, clock: handle, root }
}

fn fingerprint(h: &Harness) -> String {
    PackRegistry::new(h.root.join("mods")).load("party_pack").unwrap().fingerprint
}

fn post(server: &StoryServer, path: &str, body: Value) -> (u16, Value) {
    let response = server.handle(&Request::json("POST", path, &body));
    (response.status, response.json_body())
}

fn get(server: &StoryServer, path: &str) -> (u16, Value) {
    let response = server.handle(&Request::new("GET", path));
    (response.status, response.json_body())
}

/// A guest name and its lease token, from POST /api/guest/name.
fn take_guest_name(server: &StoryServer) -> (String, String) {
    let (status, lease) = post(server, "/api/guest/name", json!({}));
    assert_eq!(status, 200);
    (lease["name"].as_str().unwrap().to_string(), lease["token"].as_str().unwrap().to_string())
}

#[test]
fn health_answers_and_unknown_routes_are_refused() {
    let h = harness("health");
    let (status, body) = get(&h.server, "/api/health");
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(body["service"], "Last Route story server");
    assert_eq!(body["relay"], false, "the relay is not started in this test");
    let (status, body) = post(&h.server, "/api/rooms/nothing", json!({}));
    assert_eq!(status, 404);
    assert_eq!(body["code"], "UNKNOWN_ENDPOINT");
    let (status, body) = post(&h.server, "/api/rooms/create", json!([1, 2]));
    assert_eq!(status, 400, "a body that is not a JSON object is refused: {body}");
}

#[test]
fn a_guest_name_lease_admits_its_holder_to_a_room_and_expires() {
    let h = harness("lease");
    let (name, token) = take_guest_name(&h.server);
    let fp = fingerprint(&h);
    let (status, created) = post(
        &h.server,
        "/api/rooms/create",
        json!({ "pack": "party_pack", "packFingerprint": fp, "guestName": name, "guestToken": token }),
    );
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["room"]["host"], name.as_str());
    let code = created["code"].as_str().unwrap().to_string();

    let (status, refused) = post(
        &h.server,
        "/api/rooms/create",
        json!({ "pack": "party_pack", "packFingerprint": fp, "guestName": name, "guestToken": "forged" }),
    );
    assert_eq!((status, refused["code"].clone()), (401, json!("IDENTITY_REQUIRED")));

    h.clock.advance(LEASE_MS + 1);
    let (status, refused) = post(
        &h.server,
        "/api/rooms/join",
        json!({ "code": code, "packFingerprint": fp, "guestName": name, "guestToken": token }),
    );
    assert_eq!((status, refused["code"].clone()), (401, json!("IDENTITY_REQUIRED")), "an expired lease proves nothing");
}

#[test]
fn an_account_session_hosts_a_room_under_its_name() {
    let h = harness("account");
    let (status, registered) = post(&h.server, "/api/account/register", json!({ "username": "PILOT_ONE", "password": "hunter22" }));
    assert_eq!(status, 200, "{registered}");
    let session = registered["sessionToken"].as_str().unwrap().to_string();
    let fp = fingerprint(&h);
    let (status, created) = post(
        &h.server,
        "/api/rooms/create",
        json!({ "pack": "party_pack", "packFingerprint": fp, "sessionToken": session }),
    );
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["room"]["host"], "PILOT_ONE");
    let (status, refused) = post(
        &h.server,
        "/api/rooms/create",
        json!({ "pack": "party_pack", "packFingerprint": fp, "sessionToken": "not-a-session" }),
    );
    assert_eq!((status, refused["code"].clone()), (401, json!("SESSION_INVALID")));
}

#[test]
fn the_room_routes_run_over_http_and_the_seventh_guest_is_refused() {
    let h = harness("rooms");
    let fp = fingerprint(&h);
    let (host, host_token) = take_guest_name(&h.server);
    let (status, created) = post(
        &h.server,
        "/api/rooms/create",
        json!({ "pack": "party_pack", "packFingerprint": fp, "guestName": host, "guestToken": host_token }),
    );
    assert_eq!(status, 200);
    let code = created["code"].as_str().unwrap().to_string();
    let player_token = created["playerToken"].as_str().unwrap().to_string();
    for _ in 0..5 {
        let (name, token) = take_guest_name(&h.server);
        let (status, _) = post(
            &h.server,
            "/api/rooms/join",
            json!({ "code": code, "packFingerprint": fp, "guestName": name, "guestToken": token }),
        );
        assert_eq!(status, 200);
    }
    let (name, token) = take_guest_name(&h.server);
    let (status, refused) = post(
        &h.server,
        "/api/rooms/join",
        json!({ "code": code, "packFingerprint": fp, "guestName": name, "guestToken": token }),
    );
    assert_eq!((status, refused["code"].clone()), (409, json!("ROOM_FULL")));

    let (status, public) = get(&h.server, &format!("/api/rooms/{code}"));
    assert_eq!(status, 200);
    assert_eq!(public["room"]["players"].as_array().unwrap().len(), 6);
    assert!(public["room"].get("relay").is_none(), "the public view has no relay handle");
    let (status, _) = get(&h.server, "/api/rooms/ZZZZZZ");
    assert_eq!(status, 404);

    let (status, refused) = post(&h.server, "/api/rooms/state", json!({ "code": code, "scene": { "id": "hall" } }));
    assert_eq!((status, refused["code"].clone()), (403, json!("NOT_IN_ROOM")), "a call without a player token is refused");
    let (status, written) = post(
        &h.server,
        "/api/rooms/state",
        json!({ "code": code, "playerToken": player_token, "scene": { "id": "hall" }, "pose": { "x": 3 } }),
    );
    assert_eq!(status, 200, "{written}");
    assert_eq!(written["room"]["scene"], "hall");
}
