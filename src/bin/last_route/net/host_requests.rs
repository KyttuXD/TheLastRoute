//! Guest-to-host request handlers (TODO 1022; Godot `remote_players.gd` clamps every field of what a guest asks for).
//!
//! A guest asks the host for something with a `SubmitEvent` whose `what` names the request. The host never trusts the
//! event: [`handle`] drops any `what` that is not in the whitelist of `data/net/anticheat.json` "requests.allowed",
//! drops owner-only asks from a guest that is not the room owner, and clamps every field (text length, ids, names,
//! coordinates) to the data's limits. What comes out is a typed [`Request`] the host game then carries out; the guest
//! builds the same enum and sends it with [`Request::event`] / [`Request::message`], so both sides share one catalogue.
//!
//! Hooks: where the host handles `NetEvent`/`SubmitEvent` from a guest (net/world_sync.rs), call
//! `host_requests::handle(host_requests::rules(), &event, is_owner)` before acting on the event.

use super::protocol::Message;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

/// The "requests" limits of `data/net/anticheat.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub allowed: Vec<String>,
    pub owner_only: Vec<String>,
    pub route_actions: Vec<String>,
    pub chat_max: usize,
    pub name_max: usize,
    pub emote_max: usize,
    pub id_max: i64,
    pub coord_limit: f64,
    pub hash_max: usize,
    pub pack_ids_max: usize,
}

impl Default for Rules {
    fn default() -> Rules {
        let l = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        Rules {
            allowed: l(&["route_command", "room_chat", "hunter_emote", "toggle_hunter", "npc_spawn", "npc_remove", "pack_hash"]),
            owner_only: l(&["route_command", "toggle_hunter", "npc_spawn", "npc_remove"]),
            route_actions: l(&["start", "end", "restart"]),
            chat_max: 80,
            name_max: 24,
            emote_max: 24,
            id_max: 100_000,
            coord_limit: 100_000.0,
            hash_max: 16,
            pack_ids_max: 16,
        }
    }
}

/// Parses the file's "requests" object over the defaults.
pub fn parse(text: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("anticheat.json: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some(super::anticheat::SCHEMA) {
        return Err("anticheat.json: wrong schema".into());
    }
    let mut r = Rules::default();
    let Some(q) = v.get("requests") else { return Ok(r) };
    let strings = |k: &str, into: &mut Vec<String>| {
        if let Some(a) = q.get(k).and_then(Value::as_array) {
            *into = a.iter().filter_map(Value::as_str).map(|s| s.to_ascii_lowercase()).collect();
        }
    };
    strings("allowed", &mut r.allowed);
    strings("owner_only", &mut r.owner_only);
    strings("route_actions", &mut r.route_actions);
    let count = |k: &str, into: &mut usize| {
        if let Some(n) = q.get(k).and_then(Value::as_u64).filter(|n| *n > 0) {
            *into = n.min(100_000) as usize;
        }
    };
    count("chat_max", &mut r.chat_max);
    count("name_max", &mut r.name_max);
    count("emote_max", &mut r.emote_max);
    count("hash_max", &mut r.hash_max);
    count("pack_ids_max", &mut r.pack_ids_max);
    if let Some(n) = q.get("id_max").and_then(Value::as_i64).filter(|n| *n > 0) {
        r.id_max = n;
    }
    if let Some(n) = q.get("coord_limit").and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0) {
        r.coord_limit = n;
    }
    Ok(r)
}

/// The embedded limits, read once.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| crate::content::embedded::data_under("net/anticheat.json").next().and_then(|(_, t)| parse(t).ok()).unwrap_or_default())
}

/// A request a guest may make, after the host clamped it.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    RouteCommand { action: String },
    RoomChat { text: String },
    HunterEmote { hunter: i64, emote: String },
    ToggleHunter { hunter: i64, on: bool },
    NpcSpawn { name: String, x: f32, y: f32 },
    NpcRemove { id: i64 },
    PackHash { hash: String, ids: Vec<String> },
}

/// Why the host dropped an event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reject {
    /// The `what` is not in the whitelist (or is missing).
    NotAllowed(String),
    /// An owner-only ask from a guest that does not own the room.
    NotOwner(String),
    /// The fields were missing, of the wrong kind or empty after clamping.
    Malformed(&'static str),
}

impl Request {
    /// The `what` this request travels under.
    pub fn what(&self) -> &'static str {
        match self {
            Request::RouteCommand { .. } => "route_command",
            Request::RoomChat { .. } => "room_chat",
            Request::HunterEmote { .. } => "hunter_emote",
            Request::ToggleHunter { .. } => "toggle_hunter",
            Request::NpcSpawn { .. } => "npc_spawn",
            Request::NpcRemove { .. } => "npc_remove",
            Request::PackHash { .. } => "pack_hash",
        }
    }

    /// The event a guest sends (`SubmitEvent`).
    pub fn event(&self) -> Map<String, Value> {
        let mut o = Map::new();
        o.insert("what".into(), json!(self.what()));
        match self {
            Request::RouteCommand { action } => {
                o.insert("action".into(), json!(action));
            }
            Request::RoomChat { text } => {
                o.insert("text".into(), json!(text));
            }
            Request::HunterEmote { hunter, emote } => {
                o.insert("hunter".into(), json!(hunter));
                o.insert("emote".into(), json!(emote));
            }
            Request::ToggleHunter { hunter, on } => {
                o.insert("hunter".into(), json!(hunter));
                o.insert("on".into(), json!(on));
            }
            Request::NpcSpawn { name, x, y } => {
                o.insert("name".into(), json!(name));
                o.insert("x".into(), json!(x));
                o.insert("y".into(), json!(y));
            }
            Request::NpcRemove { id } => {
                o.insert("id".into(), json!(id));
            }
            Request::PackHash { hash, ids } => {
                o.insert("hash".into(), json!(hash));
                o.insert("ids".into(), json!(ids));
            }
        }
        o
    }

    /// The typed message a guest sends.
    pub fn message(&self) -> Message {
        Message::SubmitEvent { event: self.event() }
    }

    /// The request a received message carries, unchecked; None for any other message.
    pub fn from_message(m: &Message) -> Option<&Map<String, Value>> {
        match m {
            Message::SubmitEvent { event } if event.get("what").and_then(Value::as_str).is_some() => Some(event),
            _ => None,
        }
    }
}

fn text_of(o: &Map<String, Value>, key: &str, max: usize) -> String {
    let raw = o.get(key).and_then(Value::as_str).unwrap_or("");
    raw.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(max).collect()
}

fn id_of(o: &Map<String, Value>, key: &str, max: i64) -> Option<i64> {
    let v = o.get(key)?;
    let n = v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite() && f.fract() == 0.0).map(|f| f as i64))?;
    (0..=max).contains(&n).then_some(n)
}

fn coord_of(o: &Map<String, Value>, key: &str, limit: f64) -> Option<f32> {
    let f = o.get(key)?.as_f64().filter(|f| f.is_finite())?;
    Some(f.clamp(-limit, limit) as f32)
}

/// `[a-z0-9_]` only, at most `max` long.
fn token_of(o: &Map<String, Value>, key: &str, max: usize) -> String {
    text_of(o, key, max).to_ascii_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').collect()
}

/// Checks and clamps a guest's event. `is_owner` is whether the sender owns the room (a server-side room's owner, or the
/// host's own client).
pub fn handle(rules: &Rules, event: &Map<String, Value>, is_owner: bool) -> Result<Request, Reject> {
    let what = event.get("what").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    if !rules.allowed.iter().any(|a| *a == what) {
        return Err(Reject::NotAllowed(what.chars().take(32).collect()));
    }
    if !is_owner && rules.owner_only.iter().any(|a| *a == what) {
        return Err(Reject::NotOwner(what));
    }
    match what.as_str() {
        "route_command" => {
            let action = token_of(event, "action", 16);
            if rules.route_actions.iter().any(|a| *a == action) {
                Ok(Request::RouteCommand { action })
            } else {
                Err(Reject::Malformed("route action"))
            }
        }
        "room_chat" => {
            let text = text_of(event, "text", rules.chat_max);
            if text.is_empty() {
                Err(Reject::Malformed("empty chat"))
            } else {
                Ok(Request::RoomChat { text })
            }
        }
        "hunter_emote" => {
            let hunter = id_of(event, "hunter", rules.id_max).ok_or(Reject::Malformed("hunter id"))?;
            let emote = token_of(event, "emote", rules.emote_max);
            if emote.is_empty() {
                Err(Reject::Malformed("emote"))
            } else {
                Ok(Request::HunterEmote { hunter, emote })
            }
        }
        "toggle_hunter" => {
            let hunter = id_of(event, "hunter", rules.id_max).ok_or(Reject::Malformed("hunter id"))?;
            let on = event.get("on").and_then(Value::as_bool).ok_or(Reject::Malformed("toggle"))?;
            Ok(Request::ToggleHunter { hunter, on })
        }
        "npc_spawn" => {
            let name: String = text_of(event, "name", rules.name_max)
                .to_uppercase()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '_' || *c == '-')
                .collect();
            let x = coord_of(event, "x", rules.coord_limit).ok_or(Reject::Malformed("npc x"))?;
            let y = coord_of(event, "y", rules.coord_limit).ok_or(Reject::Malformed("npc y"))?;
            if name.trim().is_empty() {
                Err(Reject::Malformed("npc name"))
            } else {
                Ok(Request::NpcSpawn { name, x, y })
            }
        }
        "npc_remove" => Ok(Request::NpcRemove { id: id_of(event, "id", rules.id_max).ok_or(Reject::Malformed("npc id"))? }),
        "pack_hash" => {
            let hash: String = text_of(event, "hash", rules.hash_max).to_ascii_lowercase().chars().filter(char::is_ascii_hexdigit).collect();
            if hash.is_empty() {
                return Err(Reject::Malformed("hash"));
            }
            let ids: Vec<String> = event
                .get("ids")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).take(rules.pack_ids_max).map(|s| s.chars().take(32).collect::<String>()).collect())
                .unwrap_or_default();
            Ok(Request::PackHash { hash, ids })
        }
        // unreachable while the whitelist and this match agree; a whitelisted name with no handler is dropped
        _ => Err(Reject::NotAllowed(what)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn the_embedded_limits_parse() {
        assert!(parse(include_str!("../data/net/anticheat.json")).is_ok());
        let r = rules();
        assert!(r.allowed.iter().any(|a| a == "room_chat") && r.chat_max > 0);
    }

    #[test]
    fn a_non_whitelisted_op_is_dropped() {
        let r = Rules::default();
        for what in ["give_item", "set_difficulty", "kick", "", "ROUTE_COMMAND_X"] {
            assert!(matches!(handle(&r, &ev(json!({"what": what})), true), Err(Reject::NotAllowed(_))), "{what}");
        }
        assert!(matches!(handle(&r, &ev(json!({"text": "hi"})), true), Err(Reject::NotAllowed(_))), "no what at all");
    }

    #[test]
    fn the_whitelist_is_data() {
        let mut r = Rules::default();
        r.allowed.retain(|a| a != "room_chat");
        assert!(matches!(handle(&r, &ev(json!({"what": "room_chat", "text": "hi"})), true), Err(Reject::NotAllowed(_))));
    }

    #[test]
    fn owner_only_asks_need_the_owner() {
        let r = Rules::default();
        let e = ev(json!({"what": "route_command", "action": "restart"}));
        assert_eq!(handle(&r, &e, false), Err(Reject::NotOwner("route_command".into())));
        assert_eq!(handle(&r, &e, true), Ok(Request::RouteCommand { action: "restart".into() }));
        // chat is open to every guest
        assert!(handle(&r, &ev(json!({"what": "room_chat", "text": "hello"})), false).is_ok());
        // an action off the list is malformed even for the owner
        assert!(matches!(handle(&r, &ev(json!({"what": "route_command", "action": "format_disk"})), true), Err(Reject::Malformed(_))));
    }

    #[test]
    fn chat_text_is_clamped_and_cleaned() {
        let r = Rules::default();
        let long = "x".repeat(500);
        match handle(&r, &ev(json!({"what": "room_chat", "text": format!("  {long}\u{7}  ")})), false) {
            Ok(Request::RoomChat { text }) => assert_eq!(text.chars().count(), r.chat_max),
            other => panic!("{other:?}"),
        }
        assert!(matches!(handle(&r, &ev(json!({"what": "room_chat", "text": "  \n "})), false), Err(Reject::Malformed(_))));
    }

    #[test]
    fn ids_names_and_coordinates_are_clamped() {
        let r = Rules::default();
        assert!(matches!(handle(&r, &ev(json!({"what": "hunter_emote", "hunter": -1, "emote": "wave"})), false), Err(Reject::Malformed(_))));
        assert!(matches!(handle(&r, &ev(json!({"what": "hunter_emote", "hunter": r.id_max + 1, "emote": "wave"})), false), Err(Reject::Malformed(_))));
        assert_eq!(
            handle(&r, &ev(json!({"what": "hunter_emote", "hunter": 3.0, "emote": "Wa<ve>!"})), false),
            Ok(Request::HunterEmote { hunter: 3, emote: "wave".into() })
        );
        match handle(&r, &ev(json!({"what": "npc_spawn", "name": "ana<script>", "x": 1e12, "y": -1e12})), true) {
            Ok(Request::NpcSpawn { name, x, y }) => {
                assert_eq!(name, "ANASCRIPT");
                assert_eq!((x as f64, y as f64), (r.coord_limit, -r.coord_limit));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(handle(&r, &ev(json!({"what": "npc_spawn", "name": "ANA", "x": "far", "y": 0})), true), Err(Reject::Malformed(_))));
        assert!(matches!(handle(&r, &ev(json!({"what": "toggle_hunter", "hunter": 2})), true), Err(Reject::Malformed(_))), "a toggle needs on");
    }

    #[test]
    fn a_request_round_trips_through_the_typed_message() {
        let r = Rules::default();
        let sent = vec![
            Request::RoomChat { text: "HELLO".into() },
            Request::HunterEmote { hunter: 4, emote: "spot".into() },
            Request::ToggleHunter { hunter: 4, on: false },
            Request::NpcRemove { id: 9 },
            Request::PackHash { hash: "deadbeef".into(), ids: vec!["neon_halls".into()] },
        ];
        for req in sent {
            let wire = Message::decode(&req.message().encode()).expect("decodes");
            let event = Request::from_message(&wire).expect("is an ask");
            assert_eq!(handle(&r, event, true), Ok(req.clone()), "{}", req.what());
        }
        assert!(Request::from_message(&Message::HostBeat).is_none());
    }

    #[test]
    fn the_pack_hash_keeps_only_hex_and_a_short_id_list() {
        let r = Rules::default();
        let ids: Vec<String> = (0..40).map(|i| format!("pack_{i}")).collect();
        match handle(&r, &ev(json!({"what": "pack_hash", "hash": "AB12zz;DROP", "ids": ids})), false) {
            Ok(Request::PackHash { hash, ids }) => {
                assert_eq!(hash, "ab12d");
                assert_eq!(ids.len(), r.pack_ids_max);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(handle(&r, &ev(json!({"what": "pack_hash", "hash": "xyz"})), false), Err(Reject::Malformed(_))));
    }
}
