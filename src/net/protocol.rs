//! What the Godot co-op code syncs between players, as typed messages (`game/autoload/room_state.gd`,
//! `game/network/remote_players.gd`, `game/network/voice_chat.gd`, `game/network/net_channels.gd`,
//! `game/network/net_protocol.gd`).
//!
//! ## The original
//! The host is authoritative (Godot "authority" = peer 1). ENet channels (NetChannels):
//! * 0 EVENTS, reliable + ordered: lobby/room control, world events, interactions, snapshots, profiles, chat.
//! * 1 STATE, unreliable_ordered: each runner's 20 Hz movement sample (`PlayerStateWire`), `_place` / `_xnpc_place`.
//! * 2 HUNTERS, unreliable_ordered: host hunter frames; 3 HUNTERS_LARGE, reliable: frames too big for one datagram.
//! * 4 VOICE, unreliable: proximity voice (`_voice_up` guest->host, `_voice_down` host->all).
//!
//! Guests send requests (`any_peer` RPCs: `_register_peer`, `_submit_event`, `_request_interaction`,
//! `_request_snapshot`, `_submit_chat`, `_submit_hunter_event`, `_place`); the host validates them and broadcasts
//! results (`authority` RPCs: `_receive_room`, `_receive_start`, `_event`, `_snapshot`, `_hunters`, ...). The host
//! also sends `_receive_host_beat` (unreliable) so guests notice a frozen host, and runs the deliberate host
//! handoff (`_handoff_*`, task 010) whose relay side is `LRH1|epoch|token` (see `relay.rs`).
//!
//! ## This port
//! Godot encodes RPCs with SceneMultiplayer (node-path caches, RPC ids by sorted method name, Variant-encoded
//! arguments) inside ENet. That stack is not reimplemented. [`Message`] carries the same RPCs with the same argument
//! lists as JSON objects `{"t": "<rpc name>", ...args}` (Godot `Dictionary` arguments stay JSON objects; their keys
//! are whatever the game layer puts in them). [`crate::net::session`] carries them over the relay on the same channel
//! numbers. [`fingerprint`] is this port's protocol tag: it never equals a Godot build's, so the directory's
//! "BUILD MISMATCH" check keeps Godot and Rust peers apart.

use serde_json::{json, Map, Value};

/// NetChannels.
pub mod channel {
    pub const EVENTS: u8 = 0;
    pub const STATE: u8 = 1;
    pub const HUNTERS: u8 = 2;
    pub const HUNTERS_LARGE: u8 = 3;
    pub const VOICE: u8 = 4;
    pub const COUNT: usize = 5;
}

/// How a channel is delivered (the RPC's transfer mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    Reliable,
    UnreliableOrdered,
    Unreliable,
}

pub fn delivery(channel_id: u8) -> Delivery {
    match channel_id {
        channel::STATE | channel::HUNTERS => Delivery::UnreliableOrdered,
        channel::VOICE => Delivery::Unreliable,
        _ => Delivery::Reliable,
    }
}

/// Who may send an RPC: the host only ("authority") or any peer (guests send, the host validates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sender {
    Host,
    AnyPeer,
}

/// Every networked RPC of the original: (name, sender, channel, delivery-overriding unreliable flag).
pub const RPC_TABLE: &[(&str, Sender, u8)] = &[
    ("_receive_host_beat", Sender::Host, 99),
    ("_receive_host_hidden", Sender::Host, channel::EVENTS),
    ("_request_peer_leave", Sender::AnyPeer, channel::EVENTS),
    ("_register_peer", Sender::AnyPeer, channel::EVENTS),
    ("_receive_room", Sender::Host, channel::EVENTS),
    ("_receive_packs", Sender::Host, channel::EVENTS),
    ("_receive_daytime", Sender::Host, channel::EVENTS),
    ("_receive_start", Sender::Host, channel::EVENTS),
    ("_receive_end", Sender::Host, channel::EVENTS),
    ("_receive_room_closed", Sender::Host, channel::EVENTS),
    ("_handoff_hello", Sender::AnyPeer, channel::EVENTS),
    ("_handoff_issue_resume", Sender::Host, channel::EVENTS),
    ("_handoff_present_resume", Sender::AnyPeer, channel::EVENTS),
    ("_handoff_prepare", Sender::Host, channel::EVENTS),
    ("_handoff_prepared", Sender::AnyPeer, channel::EVENTS),
    ("_handoff_checkpoint", Sender::Host, channel::EVENTS),
    ("_handoff_checkpoint_result", Sender::AnyPeer, channel::EVENTS),
    ("_handoff_abort", Sender::Host, channel::EVENTS),
    ("_submit_chat", Sender::AnyPeer, channel::EVENTS),
    ("_chat", Sender::Host, channel::EVENTS),
    ("_submit_chat_payload", Sender::AnyPeer, channel::EVENTS),
    ("_chat_payload", Sender::Host, channel::EVENTS),
    ("_receive_pings", Sender::Host, 99),
    ("_wire_profile", Sender::AnyPeer, channel::EVENTS),
    ("_place", Sender::AnyPeer, channel::STATE),
    ("_hunters_large", Sender::Host, channel::HUNTERS_LARGE),
    ("_hunters", Sender::Host, channel::HUNTERS),
    ("_submit_event", Sender::AnyPeer, channel::EVENTS),
    ("_event", Sender::Host, channel::EVENTS),
    ("_request_interaction", Sender::AnyPeer, channel::EVENTS),
    ("_request_snapshot", Sender::AnyPeer, channel::EVENTS),
    ("_snapshot", Sender::Host, channel::EVENTS),
    ("_player_event", Sender::Host, channel::EVENTS),
    ("_submit_hunter_event", Sender::AnyPeer, channel::EVENTS),
    ("_xnpc_place", Sender::Host, channel::STATE),
    ("_xnpc_remove", Sender::Host, channel::EVENTS),
    ("_voice_up", Sender::AnyPeer, channel::VOICE),
    ("_voice_down", Sender::Host, channel::VOICE),
    ("_set_force_muted", Sender::Host, channel::EVENTS),
];
// Channel 99 above = the original's plain "unreliable" on the default channel 0; sent here on VOICE-like
// unreliable delivery (see `Message::channel`).

/// The co-op messages (one per RPC; arguments in the original order).
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    HostBeat,
    HostHidden { hidden: bool },
    PeerLeave,
    RegisterPeer { requested_name: String },
    /// `_receive_room(code, seed, mode, scale, ...)`: the lobby settings; extra arguments travel in `rest`.
    Room { code: String, seed: String, mode: i64, scale: i64, rest: Map<String, Value> },
    Packs { packs: Map<String, Value> },
    Daytime { daytime: bool },
    Start { code: String, seed: String, mode: i64, scale: i64, rest: Map<String, Value> },
    End { reason: String },
    RoomClosed { reason: String },
    HandoffHello { version: i64, session: i64 },
    HandoffIssueResume { secret: String },
    HandoffPresentResume { secret: String },
    HandoffPrepare { epoch: i64, successor: i64 },
    HandoffPrepared { epoch: i64 },
    HandoffCheckpoint { envelope: Map<String, Value> },
    HandoffCheckpointResult { epoch: i64, accepted: bool, reason: String },
    HandoffAbort { epoch: i64, reason: String },
    SubmitChat { text: String },
    Chat { peer_id: i64, display_name: String, message: String },
    SubmitChatPayload { payload: Map<String, Value> },
    ChatPayload { peer_id: i64, display_name: String, payload: Map<String, Value> },
    Pings { pings: Map<String, Value> },
    WireProfile { profile: Map<String, Value> },
    /// `_place(packet)`: the 20 Hz runner sample (PlayerStateWire); any JSON value.
    Place { packet: Value },
    HuntersLarge { data: Vec<Value> },
    Hunters { data: Vec<Value> },
    SubmitEvent { event: Map<String, Value> },
    Event { event: Map<String, Value> },
    RequestInteraction { event: Map<String, Value> },
    RequestSnapshot,
    Snapshot { state: Map<String, Value> },
    PlayerEvent { event: Map<String, Value> },
    SubmitHunterEvent { event: Map<String, Value> },
    XnpcPlace { id: i64, state: Map<String, Value> },
    XnpcRemove { id: i64 },
    VoiceUp { packet: Vec<u8> },
    VoiceDown { sender: i64, packet: Vec<u8> },
    SetForceMuted { muted: bool },
}

fn s(o: &Map<String, Value>, k: &str) -> String {
    o.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn i(o: &Map<String, Value>, k: &str) -> i64 {
    o.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0)
}
fn b(o: &Map<String, Value>, k: &str) -> bool {
    o.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn m(o: &Map<String, Value>, k: &str) -> Map<String, Value> {
    o.get(k).and_then(Value::as_object).cloned().unwrap_or_default()
}
fn a(o: &Map<String, Value>, k: &str) -> Vec<Value> {
    o.get(k).and_then(Value::as_array).cloned().unwrap_or_default()
}

impl Message {
    /// The RPC this message is.
    pub fn name(&self) -> &'static str {
        use Message::*;
        match self {
            HostBeat => "_receive_host_beat",
            HostHidden { .. } => "_receive_host_hidden",
            PeerLeave => "_request_peer_leave",
            RegisterPeer { .. } => "_register_peer",
            Room { .. } => "_receive_room",
            Packs { .. } => "_receive_packs",
            Daytime { .. } => "_receive_daytime",
            Start { .. } => "_receive_start",
            End { .. } => "_receive_end",
            RoomClosed { .. } => "_receive_room_closed",
            HandoffHello { .. } => "_handoff_hello",
            HandoffIssueResume { .. } => "_handoff_issue_resume",
            HandoffPresentResume { .. } => "_handoff_present_resume",
            HandoffPrepare { .. } => "_handoff_prepare",
            HandoffPrepared { .. } => "_handoff_prepared",
            HandoffCheckpoint { .. } => "_handoff_checkpoint",
            HandoffCheckpointResult { .. } => "_handoff_checkpoint_result",
            HandoffAbort { .. } => "_handoff_abort",
            SubmitChat { .. } => "_submit_chat",
            Chat { .. } => "_chat",
            SubmitChatPayload { .. } => "_submit_chat_payload",
            ChatPayload { .. } => "_chat_payload",
            Pings { .. } => "_receive_pings",
            WireProfile { .. } => "_wire_profile",
            Place { .. } => "_place",
            HuntersLarge { .. } => "_hunters_large",
            Hunters { .. } => "_hunters",
            SubmitEvent { .. } => "_submit_event",
            Event { .. } => "_event",
            RequestInteraction { .. } => "_request_interaction",
            RequestSnapshot => "_request_snapshot",
            Snapshot { .. } => "_snapshot",
            PlayerEvent { .. } => "_player_event",
            SubmitHunterEvent { .. } => "_submit_hunter_event",
            XnpcPlace { .. } => "_xnpc_place",
            XnpcRemove { .. } => "_xnpc_remove",
            VoiceUp { .. } => "_voice_up",
            VoiceDown { .. } => "_voice_down",
            SetForceMuted { .. } => "_set_force_muted",
        }
    }

    /// The channel it travels on (plain "unreliable" RPCs of channel 0 go out unreliably on VOICE's delivery).
    pub fn channel(&self) -> u8 {
        let row = RPC_TABLE.iter().find(|(n, _, _)| *n == self.name()).map(|r| r.2).unwrap_or(channel::EVENTS);
        if row == 99 {
            channel::VOICE
        } else {
            row
        }
    }

    /// Only the host may send it (Godot "authority").
    pub fn host_only(&self) -> bool {
        RPC_TABLE.iter().any(|(n, sender, _)| *n == self.name() && *sender == Sender::Host)
    }

    pub fn to_json(&self) -> Value {
        use Message::*;
        let mut o = match self {
            HostBeat | PeerLeave | RequestSnapshot => json!({}),
            HostHidden { hidden } => json!({ "hidden": hidden }),
            RegisterPeer { requested_name } => json!({ "requested_name": requested_name }),
            Room { code, seed, mode, scale, rest } | Start { code, seed, mode, scale, rest } => {
                let mut o = rest.clone();
                o.insert("p_code".into(), json!(code));
                o.insert("p_seed".into(), json!(seed));
                o.insert("p_mode".into(), json!(mode));
                o.insert("p_scale".into(), json!(scale));
                Value::Object(o)
            }
            Packs { packs } => json!({ "p_packs": packs }),
            Daytime { daytime } => json!({ "p_daytime": daytime }),
            End { reason } | RoomClosed { reason } => json!({ "reason": reason }),
            HandoffHello { version, session } => json!({ "version": version, "session": session }),
            HandoffIssueResume { secret } | HandoffPresentResume { secret } => json!({ "secret": secret }),
            HandoffPrepare { epoch, successor } => json!({ "epoch": epoch, "successor": successor }),
            HandoffPrepared { epoch } => json!({ "epoch": epoch }),
            HandoffCheckpoint { envelope } => json!({ "envelope": envelope }),
            HandoffCheckpointResult { epoch, accepted, reason } => json!({ "epoch": epoch, "accepted": accepted, "reason": reason }),
            HandoffAbort { epoch, reason } => json!({ "epoch": epoch, "reason": reason }),
            SubmitChat { text } => json!({ "text": text }),
            Chat { peer_id, display_name, message } => json!({ "peer_id": peer_id, "display_name": display_name, "message": message }),
            SubmitChatPayload { payload } => json!({ "payload": payload }),
            ChatPayload { peer_id, display_name, payload } => json!({ "peer_id": peer_id, "display_name": display_name, "payload": payload }),
            Pings { pings } => json!({ "pings": pings }),
            WireProfile { profile } => json!({ "profile": profile }),
            Place { packet } => json!({ "packet": packet }),
            HuntersLarge { data } | Hunters { data } => json!({ "data": data }),
            SubmitEvent { event } | Event { event } | RequestInteraction { event } | PlayerEvent { event } | SubmitHunterEvent { event } => {
                json!({ "event": event })
            }
            Snapshot { state } => json!({ "state": state }),
            XnpcPlace { id, state } => json!({ "id": id, "state": state }),
            XnpcRemove { id } => json!({ "id": id }),
            VoiceUp { packet } => json!({ "packet": super::base64_encode(packet) }),
            VoiceDown { sender, packet } => json!({ "sender": sender, "packet": super::base64_encode(packet) }),
            SetForceMuted { muted } => json!({ "muted": muted }),
        };
        o["t"] = json!(self.name());
        o
    }

    pub fn from_json(value: &Value) -> Option<Message> {
        use Message::*;
        let o = value.as_object()?;
        let bytes = |k: &str| super::base64_decode(&s(o, k)).unwrap_or_default();
        let lobby = |o: &Map<String, Value>| {
            let mut rest = o.clone();
            for k in ["t", "p_code", "p_seed", "p_mode", "p_scale"] {
                rest.remove(k);
            }
            (s(o, "p_code"), s(o, "p_seed"), i(o, "p_mode"), i(o, "p_scale"), rest)
        };
        Some(match o.get("t")?.as_str()? {
            "_receive_host_beat" => HostBeat,
            "_receive_host_hidden" => HostHidden { hidden: b(o, "hidden") },
            "_request_peer_leave" => PeerLeave,
            "_register_peer" => RegisterPeer { requested_name: s(o, "requested_name") },
            "_receive_room" => {
                let (code, seed, mode, scale, rest) = lobby(o);
                Room { code, seed, mode, scale, rest }
            }
            "_receive_start" => {
                let (code, seed, mode, scale, rest) = lobby(o);
                Start { code, seed, mode, scale, rest }
            }
            "_receive_packs" => Packs { packs: m(o, "p_packs") },
            "_receive_daytime" => Daytime { daytime: b(o, "p_daytime") },
            "_receive_end" => End { reason: s(o, "reason") },
            "_receive_room_closed" => RoomClosed { reason: s(o, "reason") },
            "_handoff_hello" => HandoffHello { version: i(o, "version"), session: i(o, "session") },
            "_handoff_issue_resume" => HandoffIssueResume { secret: s(o, "secret") },
            "_handoff_present_resume" => HandoffPresentResume { secret: s(o, "secret") },
            "_handoff_prepare" => HandoffPrepare { epoch: i(o, "epoch"), successor: i(o, "successor") },
            "_handoff_prepared" => HandoffPrepared { epoch: i(o, "epoch") },
            "_handoff_checkpoint" => HandoffCheckpoint { envelope: m(o, "envelope") },
            "_handoff_checkpoint_result" => HandoffCheckpointResult { epoch: i(o, "epoch"), accepted: b(o, "accepted"), reason: s(o, "reason") },
            "_handoff_abort" => HandoffAbort { epoch: i(o, "epoch"), reason: s(o, "reason") },
            "_submit_chat" => SubmitChat { text: s(o, "text") },
            "_chat" => Chat { peer_id: i(o, "peer_id"), display_name: s(o, "display_name"), message: s(o, "message") },
            "_submit_chat_payload" => SubmitChatPayload { payload: m(o, "payload") },
            "_chat_payload" => ChatPayload { peer_id: i(o, "peer_id"), display_name: s(o, "display_name"), payload: m(o, "payload") },
            "_receive_pings" => Pings { pings: m(o, "pings") },
            "_wire_profile" => WireProfile { profile: m(o, "profile") },
            "_place" => Place { packet: o.get("packet").cloned().unwrap_or(Value::Null) },
            "_hunters_large" => HuntersLarge { data: a(o, "data") },
            "_hunters" => Hunters { data: a(o, "data") },
            "_submit_event" => SubmitEvent { event: m(o, "event") },
            "_event" => Event { event: m(o, "event") },
            "_request_interaction" => RequestInteraction { event: m(o, "event") },
            "_request_snapshot" => RequestSnapshot,
            "_snapshot" => Snapshot { state: m(o, "state") },
            "_player_event" => PlayerEvent { event: m(o, "event") },
            "_submit_hunter_event" => SubmitHunterEvent { event: m(o, "event") },
            "_xnpc_place" => XnpcPlace { id: i(o, "id"), state: m(o, "state") },
            "_xnpc_remove" => XnpcRemove { id: i(o, "id") },
            "_voice_up" => VoiceUp { packet: bytes("packet") },
            "_voice_down" => VoiceDown { sender: i(o, "sender"), packet: bytes("packet") },
            "_set_force_muted" => SetForceMuted { muted: b(o, "muted") },
            _ => return None,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(&self.to_json()).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> Option<Message> {
        Message::from_json(&serde_json::from_slice(bytes).ok()?)
    }
}

/// The wire version of this port's co-op session (`session.rs` framing + this message table).
pub const RUST_WIRE_VERSION: u32 = 1;

/// `NetProtocol.fingerprint()` for this port: FNV-1a (32-bit) over the wire version and the RPC table, as 8 hex
/// digits. Prefixed with "rust" before hashing so it never collides with a Godot build's String.hash() value by design.
pub fn fingerprint() -> String {
    let mut text = format!("rust;wire{RUST_WIRE_VERSION}");
    for (name, sender, ch) in RPC_TABLE {
        text.push_str(&format!(";{name}={:?}/{ch}", sender));
    }
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_round_trips() {
        let mut ev = Map::new();
        ev.insert("kind".into(), json!("door"));
        let samples = vec![
            Message::HostBeat,
            Message::RegisterPeer { requested_name: "R-4821".into() },
            Message::Start { code: "ABCDE".into(), seed: "S".into(), mode: 1, scale: 2, rest: ev.clone() },
            Message::HandoffCheckpointResult { epoch: 7, accepted: true, reason: String::new() },
            Message::Place { packet: json!([1, 2.5, "x"]) },
            Message::Hunters { data: vec![json!({"id": 3})] },
            Message::Event { event: ev },
            Message::VoiceDown { sender: 2, packet: vec![0, 1, 255] },
        ];
        for message in samples {
            assert_eq!(Message::decode(&message.encode()), Some(message.clone()), "{}", message.name());
        }
        assert_eq!(Message::Place { packet: Value::Null }.channel(), channel::STATE);
        assert_eq!(Message::HostBeat.channel(), channel::VOICE);
        assert!(Message::Snapshot { state: Map::new() }.host_only());
        assert!(!Message::SubmitEvent { event: Map::new() }.host_only());
        assert_eq!(RPC_TABLE.len(), 39);
    }

    #[test]
    fn fingerprint_is_eight_hex_digits() {
        let f = fingerprint();
        assert_eq!(f.len(), 8);
        assert!(f.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(f, fingerprint());
    }
}
