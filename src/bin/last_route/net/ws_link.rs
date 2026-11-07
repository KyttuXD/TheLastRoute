//! The browser build's room transport: the WebSocket relay the Godot web build joins (`/api/ws` on the backend;
//! `game/network/ws_room_peer.gd` and `backend/ws_relay.mjs` are the reference). It implements [`RoomLink`], so the co-op
//! layer ([`super::session`]) runs over it unchanged. The UDP relay stays the native transport ([`super::relay`]).
//!
//! Wire (the Godot spec):
//! * Text frames are JSON control. The client sends `{"t":"hello","code","role":"host","token"}` (host) or
//!   `{"t":"hello","code","role":"guest","ticket"?}` (guest) as the FIRST message. The server answers
//!   `{"t":"welcome","id","role"}` (the host is id 1; a guest gets a random id in [2, 2^31-2]). A host also gets
//!   `{"t":"peer","id","on"}` as guests come and go. A guest gets `{"t":"host","on":false}` then close 4001 when the host
//!   leaves. `{"t":"error","message"}` is followed by close 4002. `{"t":"ping"}` from the client is a keepalive; the
//!   server closes an idle socket after 70 s.
//! * Binary frames carry data in both directions: bytes 0..4 are an int32 LE peer, byte 4 the transfer mode (0 unreliable,
//!   1 unreliable-ordered, 2 reliable: the relay does not read it), byte 5 the channel, bytes 6.. the payload. From a
//!   client, the peer is the TARGET (1 = host, N = guest N, 0 = everyone but the sender). From the server, the peer is the
//!   SENDER id (the hub rewrites it).
//!
//! The payload is the same session packet the UDP relay carries ([`super::session`]: channel, kind, seq, body), so nothing
//! above this module changes. Rust peers talk to each other this way; a Godot peer does not read these payloads (see the
//! INTEROP NOTE in [`super`]).
//!
//! Layout: the frame codec, the control messages and [`WsCore`] (the state machine, no I/O) are plain code and tested on
//! every target. [`WsLink`] drives [`super::web_bridge`] (the page's WebSocket) and exists only where the page does.

use super::protocol::{self, Delivery};
use super::relay::{RelayEvent, Role};
use super::room_link::RoomLink;
use super::session::HOST_PEER_ID;
use crate::shared::clock::Instant;
use serde_json::{json, Value};
use std::cell::Cell;
use std::time::Duration;

/// The binary frame header: int32 peer, u8 mode, u8 channel.
pub const FRAME_HEADER_BYTES: usize = 6;
/// The largest payload a frame carries (the Godot relay's cap, ws_relay.mjs).
pub const MAX_FRAME_PAYLOAD: usize = 65_535;
/// Transfer modes (Godot's MultiplayerPeer.TransferMode numbering).
pub const MODE_UNRELIABLE: u8 = 0;
pub const MODE_UNRELIABLE_ORDERED: u8 = 1;
pub const MODE_RELIABLE: u8 = 2;
/// An unreliable frame is dropped while more than this is queued in the page (ws_room_peer.gd: 256 KB).
pub const UNRELIABLE_BUFFER_LIMIT: u32 = 256 * 1024;
/// The client's keepalive interval (ws_room_peer.gd KEEPALIVE_MSEC).
pub const KEEPALIVE: Duration = Duration::from_secs(20);
/// The relay's path on the backend.
pub const RELAY_PATH: &str = "/api/ws";

/// One binary frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub peer: i32,
    pub mode: u8,
    pub channel: u8,
    pub payload: Vec<u8>,
}

/// The header of a frame carrying `payload`. None when the payload is over the relay's cap.
pub fn encode_frame(peer: i32, mode: u8, channel: u8, payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() > MAX_FRAME_PAYLOAD {
        return None;
    }
    let mut out = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    out.extend_from_slice(&peer.to_le_bytes());
    out.push(mode);
    out.push(channel);
    out.extend_from_slice(payload);
    Some(out)
}

/// Reads one binary frame. None for a frame shorter than its header or longer than the cap (the relay drops those too).
pub fn decode_frame(bytes: &[u8]) -> Option<Frame> {
    if bytes.len() < FRAME_HEADER_BYTES || bytes.len() > FRAME_HEADER_BYTES + MAX_FRAME_PAYLOAD {
        return None;
    }
    Some(Frame {
        peer: i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        mode: bytes[4],
        channel: bytes[5],
        payload: bytes[FRAME_HEADER_BYTES..].to_vec(),
    })
}

/// The transfer mode a channel is sent with: the co-op layer's delivery rule ([`protocol::delivery`]) in Godot's numbers.
pub fn transfer_mode(channel: u8) -> u8 {
    match protocol::delivery(channel) {
        Delivery::Reliable => MODE_RELIABLE,
        Delivery::UnreliableOrdered => MODE_UNRELIABLE_ORDERED,
        Delivery::Unreliable => MODE_UNRELIABLE,
    }
}

/// A control message from the relay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    Welcome { id: u32 },
    PeerOn(u32),
    PeerOff(u32),
    /// The host left (guests).
    HostOff,
    Error(String),
    Other,
}

/// Reads a text control frame. None when it is not JSON.
pub fn parse_control(text: &str) -> Option<Control> {
    let v: Value = serde_json::from_str(text).ok()?;
    let kind = v.get("t")?.as_str()?;
    let id = || v.get("id").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    Some(match kind {
        "welcome" => Control::Welcome { id: id()? },
        "peer" => {
            let on = v.get("on")?.as_bool()?;
            if on {
                Control::PeerOn(id()?)
            } else {
                Control::PeerOff(id()?)
            }
        }
        "host" if v.get("on").and_then(Value::as_bool) == Some(false) => Control::HostOff,
        "error" => Control::Error(v.get("message").and_then(Value::as_str).unwrap_or("ROOM ERROR").to_string()),
        _ => Control::Other,
    })
}

/// The host's control frame that removes a guest.
pub fn kick_text(id: u32) -> String {
    json!({"t": "kick", "id": id}).to_string()
}

/// The first message a client sends. A guest omits `ticket` when it has none (ws_room_peer.gd does the same).
pub fn hello_text(code: &str, role: Role, credential: &str) -> String {
    match role {
        Role::Host => json!({"t": "hello", "code": code, "role": "host", "token": credential}).to_string(),
        Role::Guest if credential.is_empty() => json!({"t": "hello", "code": code, "role": "guest"}).to_string(),
        Role::Guest => json!({"t": "hello", "code": code, "role": "guest", "ticket": credential}).to_string(),
    }
}

/// The relay URL for an origin: `https://host` -> `wss://host/api/ws`, `http://host` -> `ws://host/api/ws`.
pub fn relay_url(origin: &str) -> String {
    let base = origin.trim().trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}{RELAY_PATH}")
}

/// The connection state machine: control and data in, [`RelayEvent`]s out. No I/O, so it is tested on every target.
pub struct WsCore {
    role: Role,
    code: String,
    session: u32,
    ready: bool,
    /// A terminal event (the room is gone, or the connection failed) was queued: nothing more is reported after it.
    terminal: bool,
    events: Vec<RelayEvent>,
}

impl WsCore {
    pub fn new(role: Role, code: &str) -> WsCore {
        WsCore { role, code: code.trim().to_uppercase(), session: 0, ready: false, terminal: false, events: Vec::new() }
    }

    pub fn role(&self) -> Role {
        self.role
    }
    pub fn ready(&self) -> bool {
        self.ready
    }
    pub fn session(&self) -> u32 {
        self.session
    }
    pub fn code(&self) -> &str {
        &self.code
    }
    /// The room is gone or the connection failed: nothing more is reported.
    pub fn terminal(&self) -> bool {
        self.terminal
    }

    /// A text control frame.
    pub fn on_text(&mut self, text: &str) {
        match parse_control(text) {
            Some(Control::Welcome { id }) => {
                if !self.ready {
                    self.ready = true;
                    self.session = if self.role == Role::Host { 0 } else { id };
                    self.events.push(RelayEvent::Ready { session: self.session });
                }
            }
            Some(Control::PeerOn(id)) if self.role == Role::Host => self.events.push(RelayEvent::PeerJoined(id)),
            Some(Control::PeerOff(id)) if self.role == Role::Host => self.events.push(RelayEvent::PeerLeft(id)),
            Some(Control::HostOff) if self.role == Role::Guest => self.end(RelayEvent::HostLeft("THE HOST LEFT THE ROOM".into())),
            Some(Control::Error(message)) => self.fail(RelayEvent::Error(message)),
            _ => {}
        }
    }

    /// A binary frame: the server's sender id is the session (a guest's is the host's id 1, which the co-op layer reads).
    pub fn on_binary(&mut self, bytes: &[u8]) {
        if self.terminal {
            return;
        }
        if let Some(frame) = decode_frame(bytes) {
            if let Ok(session) = u32::try_from(frame.peer) {
                self.events.push(RelayEvent::Data { session, payload: frame.payload });
            }
        }
    }

    /// The socket closed (after any error or control message). The close code says why.
    pub fn on_close(&mut self, code: u16, reason: &str) {
        if self.terminal {
            return;
        }
        match code {
            // a normal close is ours (we left), so nothing is reported
            1000 => {}
            // the room is gone: host left, room closed by the directory, removed by the host
            4001 => self.end(RelayEvent::HostLeft(non_empty(&reason, "THE HOST LEFT THE ROOM"))),
            4004 => self.end(RelayEvent::HostLeft(non_empty(&reason, "THE ROOM CLOSED"))),
            4005 => self.end(RelayEvent::HostLeft("YOU WERE REMOVED FROM THE ROOM".into())),
            // the relay already sent its error frame (4002) or refused the host (4000): say so once
            4000 => self.fail(RelayEvent::Error("ANOTHER HOST CONNECTED TO THIS ROOM".into())),
            4002 => self.fail(RelayEvent::Error(non_empty(&reason, "THE ROOM REFUSED THE CONNECTION"))),
            4003 => self.fail(RelayEvent::Error("THE ROOM DID NOT ANSWER IN TIME".into())),
            _ if self.ready && self.role == Role::Guest => self.end(RelayEvent::HostLeft("LOST THE CONNECTION TO THE ROOM".into())),
            _ => self.fail(RelayEvent::Error("COULD NOT REACH THE ROOM".into())),
        }
    }

    /// Queues the room-ended event (the connection is over).
    fn end(&mut self, event: RelayEvent) {
        if !self.terminal {
            self.terminal = true;
            self.events.push(event);
        }
    }

    /// Queues a failure. A failure after a terminal event is not reported again.
    fn fail(&mut self, event: RelayEvent) {
        if !self.terminal {
            self.terminal = true;
            self.events.push(event);
        }
    }

    /// The events since the last call.
    pub fn take_events(&mut self) -> Vec<RelayEvent> {
        std::mem::take(&mut self.events)
    }
}

fn non_empty(text: &str, fallback: &str) -> String {
    if text.trim().is_empty() {
        fallback.to_string()
    } else {
        text.to_string()
    }
}

/// One WebSocket relay connection (host or guest) in the browser build.
pub struct WsLink {
    core: WsCore,
    /// The page's socket handle; 0 once closed.
    handle: u32,
    /// The host token (sent in the hello; kept for a token refresh).
    token: String,
    last_out: Cell<Instant>,
}

impl WsLink {
    /// A host connection to room `code` with the token the directory returned for it.
    pub fn host(code: &str, token: &str, origin: &str) -> std::io::Result<WsLink> {
        WsLink::open(Role::Host, code, token, origin)
    }

    /// A guest connection; `ticket` is a reserved room's join ticket (base64url), empty for an open room.
    pub fn guest(code: &str, ticket: &str, origin: &str) -> std::io::Result<WsLink> {
        WsLink::open(Role::Guest, code, ticket, origin)
    }

    fn open(role: Role, code: &str, credential: &str, origin: &str) -> std::io::Result<WsLink> {
        let code = code.trim().to_uppercase();
        let hello = hello_text(&code, role, credential);
        let handle = super::web_bridge::ws_open(&relay_url(origin), &hello).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Unsupported, "THIS BUILD CANNOT OPEN A ROOM CONNECTION")
        })?;
        Ok(WsLink {
            core: WsCore::new(role, &code),
            handle,
            token: if role == Role::Host { credential.to_string() } else { String::new() },
            last_out: Cell::new(Instant::now()),
        })
    }

    fn send_text(&self, text: &str) -> bool {
        self.last_out.set(Instant::now());
        super::web_bridge::ws_send_text(self.handle, text)
    }

    /// Reads everything the page queued into the state machine.
    fn pump(&mut self) {
        if self.handle == 0 {
            return;
        }
        while let Some(item) = super::web_bridge::ws_next(self.handle) {
            match item {
                super::web_bridge::WsItem::Text(bytes) => self.core.on_text(&String::from_utf8_lossy(&bytes)),
                super::web_bridge::WsItem::Binary(bytes) => self.core.on_binary(&bytes),
                super::web_bridge::WsItem::Closed { code, reason } => {
                    self.core.on_close(code, &reason);
                    self.handle = 0;
                    break;
                }
            }
        }
    }
}

impl RoomLink for WsLink {
    fn role(&self) -> Role {
        self.core.role()
    }

    fn max_payload(&self) -> usize {
        MAX_FRAME_PAYLOAD
    }

    fn ready(&self) -> bool {
        self.core.ready()
    }

    /// False once the socket closed or the room ended: the browser host of the public route lets go of a room whose link
    /// is gone (app: `NetSession::lost_public_link`).
    fn alive(&self) -> bool {
        self.handle != 0 && !self.core.terminal()
    }

    fn session(&self) -> u32 {
        self.core.session()
    }

    fn code(&self) -> &str {
        self.core.code()
    }

    /// Host: to guest `session` (the frame's target). Guest: to the host (target 1); `session` is ignored.
    fn send(&self, session: u32, payload: &[u8]) -> bool {
        if !self.core.ready() || self.handle == 0 || payload.is_empty() {
            return false;
        }
        let target = match self.core.role() {
            Role::Host => session as i32,
            Role::Guest => HOST_PEER_ID as i32,
        };
        let channel = payload[0];
        let mode = transfer_mode(channel);
        // unreliable traffic is dropped rather than queued behind a slow link (ws_room_peer.gd)
        if mode == MODE_UNRELIABLE && super::web_bridge::ws_buffered(self.handle) > UNRELIABLE_BUFFER_LIMIT {
            return false;
        }
        let Some(frame) = encode_frame(target, mode, channel, payload) else {
            return false;
        };
        self.last_out.set(Instant::now());
        super::web_bridge::ws_send(self.handle, &frame)
    }

    /// Host: the relay closes guest `session` with 4005 (`{"t":"kick","id":N}`, ws_room_peer.gd `_disconnect_peer`).
    fn kick(&self, session: u32) -> bool {
        self.core.role() == Role::Host && self.core.ready() && self.handle != 0 && self.send_text(&kick_text(session))
    }

    fn poll(&mut self) -> Vec<RelayEvent> {
        self.pump();
        if self.handle != 0 && self.last_out.get().elapsed() >= KEEPALIVE {
            self.send_text(r#"{"t":"ping"}"#);
        }
        self.core.take_events()
    }

    fn update_host_token(&mut self, token: &str) {
        self.token = token.to_string();
    }

    fn close(&mut self) {
        if self.handle != 0 {
            super::web_bridge::ws_close(self.handle, 1000, "");
            self.handle = 0;
        }
    }
}

impl Drop for WsLink {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    #[test]
    fn frame_round_trip_keeps_peer_channel_mode_and_payload() {
        for (peer, mode, channel, payload) in [
            (1i32, MODE_RELIABLE, 0u8, b"hello".to_vec()),
            (42, MODE_UNRELIABLE_ORDERED, 1, vec![0u8, 1, 2, 255]),
            (2_147_483_646, MODE_UNRELIABLE, 2, Vec::new()),
            (0, MODE_RELIABLE, 4, vec![7u8; 1000]),
        ] {
            let bytes = encode_frame(peer, mode, channel, &payload).unwrap();
            assert_eq!(bytes.len(), FRAME_HEADER_BYTES + payload.len());
            assert_eq!(decode_frame(&bytes), Some(Frame { peer, mode, channel, payload }));
        }
    }

    #[test]
    fn negative_peers_are_the_everyone_except_targets() {
        let bytes = encode_frame(-3, MODE_RELIABLE, 1, b"x").unwrap();
        assert_eq!(&bytes[..4], &(-3i32).to_le_bytes());
        assert_eq!(decode_frame(&bytes).unwrap().peer, -3);
    }

    #[test]
    fn header_bytes_are_the_godot_layout() {
        // int32 LE peer, mode, channel, then the payload
        let bytes = encode_frame(0x0102_0304, MODE_UNRELIABLE_ORDERED, 3, b"ab").unwrap();
        assert_eq!(bytes, vec![4, 3, 2, 1, 1, 3, b'a', b'b']);
        assert_eq!(decode_frame(&[4, 3, 2, 1, 2, 0]), Some(Frame { peer: 0x0102_0304, mode: 2, channel: 0, payload: vec![] }));
    }

    #[test]
    fn short_and_oversized_frames_are_refused() {
        assert_eq!(decode_frame(&[1, 0, 0, 0, 2]), None, "a frame without its channel byte is dropped");
        assert_eq!(decode_frame(&[]), None);
        assert!(encode_frame(1, MODE_RELIABLE, 0, &vec![0u8; MAX_FRAME_PAYLOAD]).is_some());
        assert!(encode_frame(1, MODE_RELIABLE, 0, &vec![0u8; MAX_FRAME_PAYLOAD + 1]).is_none());
        assert!(decode_frame(&vec![0u8; FRAME_HEADER_BYTES + MAX_FRAME_PAYLOAD + 1]).is_none());
    }

    #[test]
    fn channels_map_to_godot_transfer_modes() {
        assert_eq!(transfer_mode(protocol::channel::EVENTS), MODE_RELIABLE);
        assert_eq!(transfer_mode(protocol::channel::STATE), MODE_UNRELIABLE_ORDERED);
        assert_eq!(transfer_mode(protocol::channel::HUNTERS), MODE_UNRELIABLE_ORDERED);
        assert_eq!(transfer_mode(protocol::channel::VOICE), MODE_UNRELIABLE);
    }

    #[test]
    fn control_messages_parse() {
        assert_eq!(parse_control(r#"{"t":"welcome","id":77,"role":"guest"}"#), Some(Control::Welcome { id: 77 }));
        assert_eq!(parse_control(r#"{"t":"peer","id":5,"on":true}"#), Some(Control::PeerOn(5)));
        assert_eq!(parse_control(r#"{"t":"peer","id":5,"on":false}"#), Some(Control::PeerOff(5)));
        assert_eq!(parse_control(r#"{"t":"host","on":false}"#), Some(Control::HostOff));
        assert_eq!(parse_control(r#"{"t":"error","message":"ROOM IS PRIVATE"}"#), Some(Control::Error("ROOM IS PRIVATE".into())));
        assert_eq!(parse_control(r#"{"t":"error"}"#), Some(Control::Error("ROOM ERROR".into())));
        assert_eq!(parse_control(r#"{"t":"pong"}"#), Some(Control::Other));
        assert_eq!(parse_control("not json"), None);
    }

    #[test]
    fn hellos_match_the_godot_client() {
        let host: Value = serde_json::from_str(&hello_text("abcde", Role::Host, "tok")).unwrap();
        assert_eq!(host, json!({"t": "hello", "code": "abcde", "role": "host", "token": "tok"}));
        let open: Value = serde_json::from_str(&hello_text("ABCDE", Role::Guest, "")).unwrap();
        assert_eq!(open, json!({"t": "hello", "code": "ABCDE", "role": "guest"}), "no ticket key when there is none");
        let reserved: Value = serde_json::from_str(&hello_text("ABCDE", Role::Guest, "tkt")).unwrap();
        assert_eq!(reserved["ticket"], "tkt");
    }

    #[test]
    fn relay_url_follows_the_page_scheme() {
        assert_eq!(relay_url("https://lastroutegodot.games.kyttuslab.org"), "wss://lastroutegodot.games.kyttuslab.org/api/ws");
        assert_eq!(relay_url("http://localhost:8105/"), "ws://localhost:8105/api/ws");
    }

    fn welcome(core: &mut WsCore, id: u32) {
        core.on_text(&format!(r#"{{"t":"welcome","id":{id},"role":"guest"}}"#));
    }

    #[test]
    fn a_guest_becomes_ready_on_welcome_and_hears_the_host_as_peer_one() {
        let mut core = WsCore::new(Role::Guest, "abcde");
        assert!(!core.ready());
        welcome(&mut core, 912);
        assert!(core.ready());
        assert_eq!(core.session(), 912);
        assert_eq!(core.take_events(), vec![RelayEvent::Ready { session: 912 }]);
        let frame = encode_frame(912, MODE_RELIABLE, 0, b"from host").unwrap();
        core.on_binary(&frame);
        assert_eq!(core.take_events(), vec![RelayEvent::Data { session: 912, payload: b"from host".to_vec() }]);
        // a second welcome does not report the room twice
        welcome(&mut core, 912);
        assert!(core.take_events().is_empty());
    }

    #[test]
    fn a_host_sees_guests_come_and_go_and_their_data() {
        let mut core = WsCore::new(Role::Host, "ABCDE");
        core.on_text(r#"{"t":"welcome","id":1,"role":"host"}"#);
        assert_eq!(core.take_events(), vec![RelayEvent::Ready { session: 0 }]);
        core.on_text(r#"{"t":"peer","id":42,"on":true}"#);
        core.on_binary(&encode_frame(42, MODE_RELIABLE, 0, b"hi").unwrap());
        core.on_text(r#"{"t":"peer","id":42,"on":false}"#);
        assert_eq!(
            core.take_events(),
            vec![
                RelayEvent::PeerJoined(42),
                RelayEvent::Data { session: 42, payload: b"hi".to_vec() },
                RelayEvent::PeerLeft(42),
            ]
        );
    }

    #[test]
    fn the_host_leaving_ends_a_guest_once() {
        let mut core = WsCore::new(Role::Guest, "ABCDE");
        welcome(&mut core, 5);
        core.take_events();
        core.on_text(r#"{"t":"host","on":false}"#);
        core.on_close(4001, "host left");
        core.on_close(1006, "");
        assert_eq!(core.take_events(), vec![RelayEvent::HostLeft("THE HOST LEFT THE ROOM".into())]);
    }

    #[test]
    fn an_error_frame_then_its_close_is_reported_once() {
        let mut core = WsCore::new(Role::Guest, "ABCDE");
        core.on_text(r#"{"t":"error","message":"ROOM IS FULL"}"#);
        core.on_close(4002, "");
        assert_eq!(core.take_events(), vec![RelayEvent::Error("ROOM IS FULL".into())]);
    }

    #[test]
    fn closes_map_to_the_room_outcome() {
        let mut room_closed = WsCore::new(Role::Guest, "ABCDE");
        room_closed.on_close(4004, "");
        assert_eq!(room_closed.take_events(), vec![RelayEvent::HostLeft("THE ROOM CLOSED".into())]);

        let mut kicked = WsCore::new(Role::Guest, "ABCDE");
        kicked.on_close(4005, "");
        assert_eq!(kicked.take_events(), vec![RelayEvent::HostLeft("YOU WERE REMOVED FROM THE ROOM".into())]);

        let mut timeout = WsCore::new(Role::Guest, "ABCDE");
        timeout.on_close(4003, "hello timeout");
        assert_eq!(timeout.take_events(), vec![RelayEvent::Error("THE ROOM DID NOT ANSWER IN TIME".into())]);

        let mut lost = WsCore::new(Role::Guest, "ABCDE");
        welcome(&mut lost, 9);
        lost.take_events();
        lost.on_close(1006, "");
        assert_eq!(lost.take_events(), vec![RelayEvent::HostLeft("LOST THE CONNECTION TO THE ROOM".into())]);

        let mut never_joined = WsCore::new(Role::Guest, "ABCDE");
        never_joined.on_close(1006, "");
        assert_eq!(never_joined.take_events(), vec![RelayEvent::Error("COULD NOT REACH THE ROOM".into())]);

        let mut normal = WsCore::new(Role::Host, "ABCDE");
        normal.on_close(1000, "");
        assert!(normal.take_events().is_empty(), "our own close reports nothing");
    }

    #[test]
    fn a_frame_too_short_to_carry_a_header_is_dropped() {
        let mut core = WsCore::new(Role::Host, "ABCDE");
        core.on_binary(&[1, 0, 0]);
        assert!(core.take_events().is_empty());
    }

    /// What the relay queues for one socket.
    enum Inbound {
        Text(String),
        Binary(Vec<u8>),
    }

    /// An in-memory relay with the server's routing (ws_relay.mjs): a frame goes to its target's socket and its peer
    /// is rewritten to the sender's id on the way.
    #[derive(Default)]
    struct Hub {
        inboxes: HashMap<u32, VecDeque<Inbound>>,
    }

    /// A socket of the in-memory relay, speaking the frames above through [`WsCore`].
    struct HubLink {
        core: WsCore,
        hub: Arc<Mutex<Hub>>,
        id: u32,
    }

    impl HubLink {
        fn join(hub: &Arc<Mutex<Hub>>, role: Role, id: u32) -> HubLink {
            hub.lock().unwrap().inboxes.entry(id).or_default();
            let mut core = WsCore::new(role, "ABCDE");
            let kind = if role == Role::Host { "host" } else { "guest" };
            core.on_text(&format!(r#"{{"t":"welcome","id":{id},"role":"{kind}"}}"#));
            HubLink { core, hub: hub.clone(), id }
        }
    }

    impl RoomLink for HubLink {
        fn role(&self) -> Role {
            self.core.role()
        }
        fn ready(&self) -> bool {
            self.core.ready()
        }
        fn session(&self) -> u32 {
            self.core.session()
        }
        fn code(&self) -> &str {
            self.core.code()
        }
        fn send(&self, session: u32, payload: &[u8]) -> bool {
            if !self.core.ready() || payload.is_empty() {
                return false;
            }
            // a host sends to the guest it names; a guest sends to the host (peer 1)
            let to = if self.core.role() == Role::Host { session } else { HOST_PEER_ID };
            let channel = payload[0];
            let Some(mut frame) = encode_frame(to as i32, transfer_mode(channel), channel, payload) else {
                return false;
            };
            // the relay replaces the target with the sender's id
            frame[..4].copy_from_slice(&(self.id as i32).to_le_bytes());
            self.hub.lock().unwrap().inboxes.entry(to).or_default().push_back(Inbound::Binary(frame));
            true
        }
        fn poll(&mut self) -> Vec<RelayEvent> {
            let items: Vec<Inbound> = self.hub.lock().unwrap().inboxes.entry(self.id).or_default().drain(..).collect();
            for item in items {
                match item {
                    Inbound::Text(text) => self.core.on_text(&text),
                    Inbound::Binary(bytes) => self.core.on_binary(&bytes),
                }
            }
            self.core.take_events()
        }
        fn update_host_token(&mut self, _token: &str) {}
        fn close(&mut self) {}
    }

    /// The co-op layer runs over the WebSocket framing: a guest's request reaches the host with the guest's id, the host's
    /// reliable replies reach the guest in order, and a guest leaving is reported.
    #[test]
    fn a_co_op_session_runs_over_the_websocket_frames() {
        use super::super::protocol::Message;
        use super::super::session::{CoopSession, SessionEvent};
        let hub = Arc::new(Mutex::new(Hub::default()));
        let mut host = CoopSession::new(HubLink::join(&hub, Role::Host, HOST_PEER_ID));
        let mut guest = CoopSession::new(HubLink::join(&hub, Role::Guest, 42));
        hub.lock().unwrap().inboxes.entry(HOST_PEER_ID).or_default().push_back(Inbound::Text(r#"{"t":"peer","id":42,"on":true}"#.into()));
        guest.send(HOST_PEER_ID, &Message::RegisterPeer { requested_name: "R-7".into() });

        let (mut host_heard, mut guest_heard, mut sent) = (Vec::new(), Vec::new(), false);
        let mut host_events = Vec::new();
        for _ in 0..50 {
            for event in host.poll() {
                if let SessionEvent::Message { from, message: Message::RegisterPeer { requested_name } } = &event {
                    assert_eq!(*from, 42, "the relay names the sender");
                    host_heard.push(requested_name.clone());
                    if !sent {
                        sent = true;
                        for i in 0..5 {
                            host.send(42, &Message::Chat { peer_id: 1, display_name: "HOST".into(), message: format!("m{i}") });
                        }
                    }
                }
                host_events.push(event);
            }
            for event in guest.poll() {
                if let SessionEvent::Message { from, message: Message::Chat { message, .. } } = event {
                    assert_eq!(from, HOST_PEER_ID);
                    guest_heard.push(message);
                }
            }
        }
        assert_eq!(host_heard, vec!["R-7".to_string()]);
        assert_eq!(guest_heard, (0..5).map(|i| format!("m{i}")).collect::<Vec<_>>());
        assert!(host_events.contains(&SessionEvent::PeerSeen(42)));

        hub.lock().unwrap().inboxes.entry(HOST_PEER_ID).or_default().push_back(Inbound::Text(r#"{"t":"peer","id":42,"on":false}"#.into()));
        let left: Vec<SessionEvent> = host.poll();
        assert!(left.contains(&SessionEvent::Relay(RelayEvent::PeerLeft(42))));
    }
}
