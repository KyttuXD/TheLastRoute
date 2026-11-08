//! Tests of the room rules: reserved reconnect slots, host leave, capacity, KICK/BAN, admin rooms, listing and sweep.
//! The windows and limits are tunables of the service, so the tests set them and check behaviour, not the defaults.

use super::admin_rooms::JoinContext;
use super::reserved_rooms::{ReservedRooms, ReservedRoomsOptions};
use super::room_service::{
    apply_item_claim_within, claim_discovery_cells, discovery_cap, RoomService, RoomServiceOptions,
    DISCOVERY_ENDLESS_LIMIT, DISCOVERY_LIMIT, DISCOVERY_TOTAL_LIMIT, HOST_LEFT_REASON, ITEM_LIMIT, ITEM_TOTAL_LIMIT,
};
use std::collections::HashSet;
use crate::util::{manual_clock, ManualClock};
use serde_json::{json, Map, Value};

struct Clocked {
    service: RoomService,
    clock: ManualClock,
}

fn service(reserve_ms: i64) -> Clocked {
    let (clock, handle) = manual_clock(1_000_000);
    let mut service = RoomService::new(RoomServiceOptions { now: clock, player_ttl_ms: 120_000, room_ttl_ms: 600_000 });
    service.reserve_ms = reserve_ms;
    service.close_notice_ms = 5_000;
    Clocked { service, clock: handle }
}

fn join(s: &mut RoomService, room: &str, id: &str, name: &str) -> Value {
    let (status, payload) = s.join_room(&json!({ "room": room, "playerId": id, "name": name }));
    assert_eq!(status, 200, "{payload}");
    payload
}

fn token(payload: &Value) -> String {
    payload["sessionToken"].as_str().unwrap().to_string()
}

fn auth(room: &str, id: &str, payload: &Value) -> Value {
    json!({ "room": room, "playerId": id, "sessionToken": token(payload) })
}

fn with(base: Value, extra: Value) -> Value {
    let mut map = base.as_object().unwrap().clone();
    for (key, value) in extra.as_object().unwrap() {
        map.insert(key.clone(), value.clone());
    }
    Value::Object(map)
}

#[test]
fn a_dropped_peers_slot_is_held_within_the_window_and_freed_after() {
    let mut c = service(10_000);
    let s = &mut c.service;
    let host = join(s, "HOLD1", "host", "Ann");
    let guest = join(s, "HOLD1", "guest", "Bob");
    s.rooms.get_mut("HOLD1").unwrap().capacity = 2;

    let dropped = s.leave_room(&with(auth("HOLD1", "guest", &guest), json!({ "dropped": true })));
    assert_eq!(dropped.1["held"], true);
    // The held slot still counts against capacity: a stranger cannot take it.
    assert_eq!(s.join_room(&json!({ "room": "HOLD1", "playerId": "late", "name": "Cy" })).1["code"], "ROOM_FULL");
    // A different peer cannot claim it, the same peer token can, and gets the same spawn index back.
    let wrong = s.join_room(&json!({ "room": "HOLD1", "playerId": "guest", "name": "Bob", "sessionToken": "nope" }));
    assert_eq!(wrong.1["code"], "INVALID_SESSION");
    c.clock.advance(5_000);
    let s = &mut c.service;
    let back = s.join_room(&with(auth("HOLD1", "guest", &guest), json!({ "name": "Bob" })));
    assert_eq!(back.0, 200);
    assert_eq!(back.1["spawnIndex"], guest["spawnIndex"]);
    assert_eq!(back.1["roomSize"], 2);
    assert_eq!(token(&back.1), token(&guest));

    // Dropped again and left past the window: the slot is freed and the old token no longer restores it.
    s.leave_room(&with(auth("HOLD1", "guest", &guest), json!({ "dropped": true })));
    c.clock.advance(10_001);
    let s = &mut c.service;
    let late = join(s, "HOLD1", "late", "Cy");
    assert_eq!(late["roomSize"], 2);
    let stale = s.join_room(&with(auth("HOLD1", "guest", &guest), json!({ "name": "Bob" })));
    assert_eq!(stale.1["code"], "ROOM_FULL");
    assert_eq!(host["room"], "HOLD1");
}

#[test]
fn holding_can_be_switched_off_by_a_zero_window() {
    let mut c = service(0);
    let s = &mut c.service;
    join(s, "HOLD2", "host", "Ann");
    let guest = join(s, "HOLD2", "guest", "Bob");
    let left = s.leave_room(&with(auth("HOLD2", "guest", &guest), json!({ "dropped": true })));
    assert!(left.1.get("held").is_none());
    assert_eq!(s.rooms["HOLD2"].players.len(), 1);
}

#[test]
fn the_host_leaving_closes_the_room_with_a_reason_that_guests_hear_for_a_while() {
    let mut c = service(10_000);
    let s = &mut c.service;
    let host = join(s, "CLOSE1", "host", "Ann");
    let guest = join(s, "CLOSE1", "guest", "Bob");
    let left = s.leave_room(&auth("CLOSE1", "host", &host));
    assert_eq!(left.1["closed"], true);
    assert_eq!(left.1["reason"], HOST_LEFT_REASON);
    assert!(!s.rooms.contains_key("CLOSE1"));
    let heard = s.update_room(&auth("CLOSE1", "guest", &guest));
    assert_eq!(heard.0, 410);
    assert_eq!(heard.1["code"], "ROOM_CLOSED");
    assert_eq!(heard.1["error"], HOST_LEFT_REASON);
    // After the notice the code is simply unknown again, and a new host can open it fresh.
    c.clock.advance(5_001);
    let s = &mut c.service;
    assert_eq!(s.update_room(&auth("CLOSE1", "guest", &guest)).1["code"], "ROOM_NOT_FOUND");
    assert_eq!(join(s, "CLOSE1", "host", "Ann")["roomSize"], 1);
}

#[test]
fn a_guest_leaving_keeps_the_room_open() {
    let mut c = service(10_000);
    let s = &mut c.service;
    let host = join(s, "STAY1", "host", "Ann");
    let guest = join(s, "STAY1", "guest", "Bob");
    assert_eq!(s.leave_room(&auth("STAY1", "guest", &guest)).1, json!({ "ok": true }));
    assert_eq!(s.update_room(&auth("STAY1", "host", &host)).1["roomSize"], 1);
}

#[test]
fn a_full_room_refuses_and_the_limit_follows_the_room_capacity() {
    let mut c = service(10_000);
    let s = &mut c.service;
    join(s, "FULL1", "host", "Ann");
    s.rooms.get_mut("FULL1").unwrap().capacity = 3;
    join(s, "FULL1", "p2", "Bob");
    let p3 = join(s, "FULL1", "p3", "Cy");
    let refused = s.join_room(&json!({ "room": "FULL1", "playerId": "p4", "name": "Di" }));
    assert_eq!(refused.0, 409);
    assert_eq!(refused.1["code"], "ROOM_FULL");
    assert_eq!(refused.1["error"], "Room capacity is 3 players");
    // Someone already inside can still rejoin when the room is full.
    let rejoin = s.join_room(&with(auth("FULL1", "p3", &p3), json!({ "name": "Cy" })));
    assert_eq!(rejoin.0, 200);
    assert_eq!(rejoin.1["roomSize"], 3);
}

#[test]
fn kick_removes_a_player_and_ban_also_blocks_the_return() {
    let mut c = service(10_000);
    let s = &mut c.service;
    let host = join(s, "KICK1", "host", "Ann");
    let p1 = join(s, "KICK1", "p1", "Bob");
    let p2 = join(s, "KICK1", "p2", "Cy");
    let none = |_: &str| false;

    // Only the host may moderate.
    let by_guest = s.moderate_player(&with(auth("KICK1", "p1", &p1), json!({ "action": "kick", "targetId": "p2" })), &none);
    assert_eq!(by_guest.1["code"], "HOST_ONLY");
    let bad = s.moderate_player(&with(auth("KICK1", "host", &host), json!({ "action": "mute", "targetId": "p1" })), &none);
    assert_eq!(bad.1["code"], "INVALID_ACTION");
    let myself = s.moderate_player(&with(auth("KICK1", "host", &host), json!({ "action": "kick", "targetId": "host" })), &none);
    assert_eq!(myself.1["code"], "CANNOT_REMOVE_HOST");
    let unknown = s.moderate_player(&with(auth("KICK1", "host", &host), json!({ "action": "kick", "targetId": "zz" })), &none);
    assert_eq!(unknown.1["code"], "PLAYER_NOT_IN_ROOM");
    let guarded = s.moderate_player(&with(auth("KICK1", "host", &host), json!({ "action": "kick", "targetName": "Cy" })), &|name| name == "CY");
    assert_eq!(guarded.1["code"], "PROTECTED_PLAYER");

    let kicked = s.moderate_player(&with(auth("KICK1", "host", &host), json!({ "action": "kick", "targetId": "p1" })), &none);
    assert_eq!(kicked.0, 200);
    assert_eq!(kicked.1["roomSize"], 2);
    let told = s.update_room(&auth("KICK1", "p1", &p1));
    assert_eq!((told.0, told.1["code"].as_str().unwrap()), (403, "KICKED"));
    // A kicked player may come back; a banned one may not, under the same id or the same name.
    assert_eq!(join(s, "KICK1", "p1", "Bob")["roomSize"], 3);

    let banned = s.moderate_player(&with(auth("KICK1", "host", &host), json!({ "action": "ban", "targetName": "Cy" })), &none);
    assert_eq!(banned.1["action"], "ban");
    assert_eq!(s.update_room(&auth("KICK1", "p2", &p2)).1["code"], "BANNED");
    assert_eq!(s.join_room(&json!({ "room": "KICK1", "playerId": "p2", "name": "Cy" })).1["code"], "BANNED");
    assert_eq!(s.join_room(&json!({ "room": "KICK1", "playerId": "other", "name": "Cy" })).1["code"], "BANNED");
    assert_eq!(s.join_room(&json!({ "room": "KICK1", "playerId": "other", "name": "Di" })).0, 200);
}

#[test]
fn an_admin_creates_a_room_on_an_unknown_code_and_a_non_admin_gets_the_old_refusal() {
    let mut c = service(10_000);
    let s = &mut c.service;
    let typed = json!({ "room": "ADM01", "playerId": "boss", "name": "Root", "joinOnly": true });
    let plain = s.join_room(&typed);
    assert_eq!((plain.0, plain.1["code"].as_str().unwrap(), plain.1["error"].as_str().unwrap()), (404, "ROOM_NOT_FOUND", "Room not found"));
    assert!(s.rooms.is_empty());

    let made = s.join_with(&typed, &JoinContext::admin(None, None));
    assert_eq!(made.0, 200);
    assert!(s.rooms["ADM01"].admin);
    // Once it exists anyone can join it by code.
    let guest = s.join_room(&json!({ "room": "ADM01", "playerId": "g", "name": "Gus", "joinOnly": true }));
    assert_eq!(guest.0, 200);
    // A room made by the ordinary host flow is not an admin room, even for an admin.
    let normal = s.join_with(&json!({ "room": "NORM1", "playerId": "boss", "name": "Root" }), &JoinContext::admin(None, None));
    assert_eq!(normal.0, 200);
    assert!(!s.rooms["NORM1"].admin);
}

#[test]
fn an_admin_with_an_empty_code_gets_a_new_room_under_a_free_code() {
    let mut c = service(10_000);
    let s = &mut c.service;
    let body = json!({ "room": "", "playerId": "boss", "name": "Root", "joinOnly": true });
    assert_eq!(s.join_room(&body).1["code"], "INVALID_ROOM");
    let taken = |code: &str| code.starts_with('A');
    let made = s.join_with(&body, &JoinContext::admin(None, Some(&taken)));
    assert_eq!(made.0, 200);
    let code = made.1["room"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 5);
    assert!(!code.starts_with('A'));
    assert!(s.rooms[&code].admin);
}

#[test]
fn admin_codes_must_be_well_formed_and_unused() {
    let mut c = service(10_000);
    let s = &mut c.service;
    let ask = |room: &str| json!({ "room": room, "playerId": "boss", "name": "Root", "joinOnly": true });
    let taken = |code: &str| code == "TAKEN1";
    for bad in ["ab", "AB-CD12", "ABCDEFGHI", "ab cd ef"] {
        let reply = s.join_with(&ask(bad), &JoinContext::admin(None, Some(&taken)));
        assert_eq!((reply.0, reply.1["code"].as_str().unwrap()), (400, "INVALID_ROOM"), "{bad}");
    }
    let busy = s.join_with(&ask("taken1"), &JoinContext::admin(None, Some(&taken)));
    assert_eq!((busy.0, busy.1["code"].as_str().unwrap()), (409, "ROOM_IN_USE"));
    assert!(s.rooms.is_empty());
    // Lower case is accepted and stored upper case.
    assert_eq!(s.join_with(&ask("fine99"), &JoinContext::admin(None, Some(&taken))).0, 200);
    assert!(s.rooms.contains_key("FINE99"));
}

#[test]
fn admin_room_settings_persist_in_the_store_across_a_restart() {
    let dir = std::env::temp_dir().join(format!("lr_admin_rooms_{}", crate::util::hex(&crate::util::random_bytes(6))));
    std::fs::create_dir_all(&dir).unwrap();
    let store = dir.join("reserved.json");
    let open = || {
        ReservedRooms::new(ReservedRoomsOptions {
            store_path: store.clone(),
            codes: Vec::new(),
            now: crate::util::system_clock(),
            idle_ttl_ms: 0,
        })
    };

    let mut c = service(10_000);
    let s = &mut c.service;
    let made = s.join_with(&json!({ "room": "KEEP1", "playerId": "boss", "name": "Root", "joinOnly": true }), &JoinContext::admin(None, None));
    assert_eq!(made.0, 200);
    let seed = made.1["worldSeed"].as_str().unwrap().to_string();
    let set = s.update_room_settings(&with(auth("KEEP1", "boss", &made.1), json!({ "settings": { "mode": "hideAndSeek", "difficulty": "nightmare" } })));
    assert_eq!(set.0, 200);
    let snapshot = s.admin_snapshot("KEEP1").expect("admin rooms have a snapshot");
    open().remember_admin_room("KEEP1", "ROOT", &snapshot);

    // New process: the store is read again and a fresh service recreates the room with the same seed and settings.
    let saved = open().admin_room("keep1").expect("kept across the restart");
    assert_eq!(saved["settings"]["difficulty"], "nightmare");
    let mut fresh = service(10_000);
    let again = fresh.service.join_with(
        &json!({ "room": "KEEP1", "playerId": "boss", "name": "Root", "joinOnly": true }),
        &JoinContext::admin(Some(saved), None),
    );
    assert_eq!(again.0, 200);
    assert_eq!(again.1["worldSeed"], seed);
    assert_eq!(again.1["settings"]["mode"], "hideAndSeek");
    assert_eq!(again.1["settings"]["difficulty"], "nightmare");

    // A room that is not an admin room has nothing to store, and forgetting removes the entry.
    assert!(fresh.service.admin_snapshot("NOPE1").is_none());
    let mut reserved = open();
    reserved.forget_admin_room("KEEP1");
    assert!(open().admin_room("KEEP1").is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn empty_rooms_are_swept_and_the_listing_has_capacity_and_the_admin_flag() {
    let mut c = service(10_000);
    let s = &mut c.service;
    join(s, "LIST1", "a", "Ann");
    s.join_with(&json!({ "room": "LIST2", "playerId": "boss", "name": "Root", "joinOnly": true }), &JoinContext::admin(None, None));
    s.rooms.get_mut("LIST1").unwrap().capacity = 4;
    let listing = s.list_rooms();
    assert_eq!(listing.len(), 2);
    assert_eq!(listing[0]["room"], "LIST1");
    assert_eq!(listing[0]["capacity"], 4);
    assert_eq!(listing[0]["players"], 1);
    assert_eq!(listing[0]["admin"], false);
    assert_eq!(listing[0]["full"], false);
    assert_eq!(listing[1]["admin"], true);

    // Everyone goes quiet past the player timeout: the rooms are swept and no longer listed.
    c.clock.advance(120_001);
    let s = &mut c.service;
    assert_eq!(s.sweep_empty(), 2);
    assert!(s.list_rooms().is_empty());
    assert_eq!(s.rooms_len(), 0);
}

#[test]
fn a_room_closed_by_its_host_does_not_wait_for_held_slots() {
    let mut c = service(10_000);
    let s = &mut c.service;
    join(s, "HELD3", "host", "Ann");
    let guest = join(s, "HELD3", "guest", "Bob");
    s.leave_room(&with(auth("HELD3", "guest", &guest), json!({ "dropped": true })));
    // The host leaves for good while the guest's slot is held: the room closes, nobody is left to wait for.
    let host_token = s.rooms["HELD3"].players[0].session_token.clone();
    let left = s.leave_room(&json!({ "room": "HELD3", "playerId": "host", "sessionToken": host_token }));
    assert_eq!(left.1["closed"], true);
    assert_eq!(s.sweep_empty(), 0);
    assert!(s.list_rooms().is_empty());
}

fn battery_claim(n: usize, depth: i64) -> Value {
    json!({ "kind": "battery", "id": format!("b{n}"), "depth": depth, "changes": { "collected": true } })
}

#[test]
fn item_records_stop_at_the_total_limit_but_existing_keys_still_update_and_a_new_depth_clears_them() {
    // Infinite chunks stream 3 relays and 4 batteries each, plus props: the cap must hold hundreds of chunks.
    assert!(ITEM_TOTAL_LIMIT >= 512 * 7, "the item cap is too tight for a long Infinite depth");
    let mut c = service(0);
    let s = &mut c.service;
    let host = join(s, "ITEMS", "host", "Ann");
    let base = auth("ITEMS", "host", &host);
    // The first record is a prop so a later claim can change it.
    let prop = json!({ "kind": "prop", "id": "p0", "depth": 0, "changes": {} });
    let (status, _) = s.update_room(&with(base.clone(), json!({ "itemClaims": [prop] })));
    assert_eq!(status, 200);
    let mut next = 0;
    while s.rooms["ITEMS"].items.len() < ITEM_TOTAL_LIMIT {
        let claims: Vec<Value> = (next..next + ITEM_LIMIT).map(|n| battery_claim(n, 0)).collect();
        next += ITEM_LIMIT;
        assert_eq!(s.update_room(&with(base.clone(), json!({ "itemClaims": claims }))).0, 200);
    }
    assert_eq!(s.rooms["ITEMS"].items.len(), ITEM_TOTAL_LIMIT);
    // New keys past the cap are ignored.
    let extra: Vec<Value> = (next..next + ITEM_LIMIT).map(|n| battery_claim(n, 0)).collect();
    s.update_room(&with(base.clone(), json!({ "itemClaims": extra })));
    assert_eq!(s.rooms["ITEMS"].items.len(), ITEM_TOTAL_LIMIT);
    assert!(!s.rooms["ITEMS"].items.contains_key(&format!("battery:b{next}")));
    // An existing key still updates at the cap.
    let open = json!({ "kind": "prop", "id": "p0", "depth": 0, "changes": { "interactionState": "OPEN" } });
    s.update_room(&with(base.clone(), json!({ "itemClaims": [open] })));
    assert_eq!(s.rooms["ITEMS"].items["prop:p0"]["interactionState"], "OPEN");
    assert_eq!(s.rooms["ITEMS"].items.len(), ITEM_TOTAL_LIMIT);
    // A depth advance clears the map, so claims on the new depth land again.
    s.update_room(&with(base.clone(), json!({ "claimDepth": 1, "itemClaims": [battery_claim(0, 1)] })));
    assert_eq!(s.rooms["ITEMS"].active_depth, 1);
    assert_eq!(s.rooms["ITEMS"].items.len(), 1);
    assert!(s.rooms["ITEMS"].items.contains_key("battery:b0"));
}

#[test]
fn the_item_cap_helper_ignores_new_keys_at_its_limit() {
    let mut items = Map::new();
    for n in 0..5 {
        apply_item_claim_within(&mut items, Some(&battery_claim(n, 0)), 3);
    }
    assert_eq!(items.len(), 3);
    assert!(items.contains_key("battery:b2") && !items.contains_key("battery:b3"));
    let relay = json!({ "kind": "relay", "id": "b0", "changes": { "sabotaged": true } });
    apply_item_claim_within(&mut items, Some(&relay), 3);
    assert_eq!(items.len(), 3, "a new kind:id is a new key and is refused at the cap");
}

#[test]
fn endless_discovery_is_capped_at_its_own_limit() {
    let mut settings = Map::new();
    settings.insert("mapScale".into(), json!("standard"));
    assert_eq!(discovery_cap(&settings), DISCOVERY_TOTAL_LIMIT);
    settings.insert("mapScale".into(), json!("endless"));
    assert_eq!(discovery_cap(&settings), DISCOVERY_ENDLESS_LIMIT);
    assert!(DISCOVERY_ENDLESS_LIMIT > DISCOVERY_TOTAL_LIMIT && DISCOVERY_ENDLESS_LIMIT < usize::MAX);

    // The helper with a test-sized cap: claims past it are ignored, repeats never count.
    let (mut seen, mut list) = (HashSet::new(), Vec::new());
    let cells: Vec<Value> = (0..10).map(|n| json!(format!("C{n}"))).collect();
    claim_discovery_cells(&mut seen, &mut list, &cells, 4);
    assert_eq!(list, vec!["C0", "C1", "C2", "C3"]);
    claim_discovery_cells(&mut seen, &mut list, &cells, 4);
    assert_eq!(seen.len(), 4);
    let many: Vec<Value> = (0..DISCOVERY_LIMIT + 20).map(|n| json!(format!("M{n}"))).collect();
    let (mut seen, mut list) = (HashSet::new(), Vec::new());
    claim_discovery_cells(&mut seen, &mut list, &many, usize::MAX);
    assert_eq!(list.len(), DISCOVERY_LIMIT, "one update still adds at most DISCOVERY_LIMIT cells");
}

#[test]
fn an_endless_room_stops_discovering_at_the_limit_and_a_depth_advance_resets_it() {
    let mut c = service(0);
    let s = &mut c.service;
    let host = join(s, "ENDLS", "host", "Ann");
    let base = auth("ENDLS", "host", &host);
    {
        let room = s.rooms.get_mut("ENDLS").unwrap();
        room.settings.insert("mapScale".into(), json!("endless"));
        // Fill to two short of the cap directly; sending 250k cells through updates would only slow the test.
        for n in 0..DISCOVERY_ENDLESS_LIMIT - 2 {
            let cell = format!("F{n}");
            room.discovered.insert(cell.clone());
            room.discovered_list.push(cell);
        }
    }
    let epoch = s.rooms["ENDLS"].discovered_epoch;
    let cursor = json!({ "discoveryEpoch": epoch, "discoverySince": DISCOVERY_ENDLESS_LIMIT });
    let claims = json!({ "discoveryClaims": ["N1", "N2", "N3", "N4", "N5"] });
    let (status, payload) = s.update_room(&with(with(base.clone(), cursor.clone()), claims));
    assert_eq!(status, 200, "{payload}");
    assert_eq!(s.rooms["ENDLS"].discovered.len(), DISCOVERY_ENDLESS_LIMIT);
    assert!(s.rooms["ENDLS"].discovered.contains("N2") && !s.rooms["ENDLS"].discovered.contains("N3"));
    s.update_room(&with(with(base.clone(), cursor), json!({ "discoveryClaims": ["N6"] })));
    assert_eq!(s.rooms["ENDLS"].discovered.len(), DISCOVERY_ENDLESS_LIMIT, "claims past the cap are ignored");
    // A depth advance clears discovery, so the next depth's cells land again.
    s.update_room(&with(base.clone(), json!({ "claimDepth": 1, "discoveryClaims": ["D1"] })));
    assert_eq!(s.rooms["ENDLS"].discovered_epoch, epoch + 1);
    assert_eq!(s.rooms["ENDLS"].discovered_list, vec!["D1".to_string()]);
}
