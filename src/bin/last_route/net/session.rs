//! A Rust-to-Rust co-op transport over the LRR1 relay (PORT DEVIATION: replaces ENet + Godot SceneMultiplayer).
//!
//! Each relay datagram payload is one packet: `channel u8 | kind u8 | seq u32 LE | body`, where the kind is one of the
//! `KIND_` constants and the body of a data packet is one encoded [`Message`]. Delivery follows the original channel
//! modes ([`protocol::delivery`]): reliable channels (EVENTS, HUNTERS_LARGE) are acknowledged, resent every 200 ms and
//! delivered in order; unreliable_ordered channels (STATE, HUNTERS) drop anything older than the newest seen;
//! VOICE is plain unreliable. Peer ids: the host is peer 1 (Godot's authority id); a guest is its relay session id.
//! A message must fit one datagram (1450 bytes); bigger hunter frames are not fragmented (ENet does that for Godot).
//! A ping is a `KIND_PING` packet the other side answers with a `KIND_PONG` carrying the same seq: the round trip
//! through the relay is that runner's ping. The host reports each guest's ping to the guests in a `KIND_PINGS` packet.

use super::protocol::{self, Delivery, Message};
use super::relay::{RelayEvent, Role, MAX_PAYLOAD_BYTES};
use super::room_link::RoomLink;
use std::collections::{BTreeMap, HashMap};
use crate::shared::clock::Instant;
use std::time::Duration;

pub const HOST_PEER_ID: u32 = 1;
pub const RESEND_INTERVAL: Duration = Duration::from_millis(200);
const PING_INTERVAL: Duration = Duration::from_secs(1);
const PACKET_HEADER: usize = 6;
pub const MAX_MESSAGE_BYTES: usize = MAX_PAYLOAD_BYTES - PACKET_HEADER;
const KIND_DATA: u8 = 0;
const KIND_ACK: u8 = 1;
const KIND_PING: u8 = 2;
const KIND_PONG: u8 = 3;
const KIND_PINGS: u8 = 4;

#[derive(Default)]
struct Channel {
    next_send: u32,
    unacked: BTreeMap<u32, (Vec<u8>, Instant)>,
    next_receive: u32,
    held: BTreeMap<u32, Vec<u8>>,
    newest_seen: Option<u32>,
}

#[derive(Default)]
struct Peer {
    channels: [Channel; protocol::channel::COUNT],
}

#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// The relay accepted us (host registered / guest joined).
    Ready,
    /// A guest sent its first packet (host side).
    PeerSeen(u32),
    Message { from: u32, message: Message },
    /// Something the relay itself said (errors, HOST_LEFT, promotion, migration).
    Relay(RelayEvent),
}

pub struct CoopSession {
    /// The relay connection: UDP ([`super::relay::RelayLink`]) natively, WebSocket ([`super::ws_link::WsLink`]) in the browser.
    pub link: Box<dyn RoomLink>,
    peers: HashMap<u32, Peer>,
    /// The last measured round trip per runner, in ms. A guest holds its own to the host (HOST_PEER_ID) and the host's
    /// report of the other guests; the host holds its own to each guest.
    pings: HashMap<u32, u32>,
    /// A ping sent and not answered yet, per runner: its seq and when it went out.
    pending_pings: HashMap<u32, (u32, Instant)>,
    next_ping: Instant,
}

fn packet(channel: u8, kind: u8, seq: u32, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PACKET_HEADER + body.len());
    out.push(channel);
    out.push(kind);
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(body);
    out
}

impl CoopSession {
    pub fn new<L: RoomLink + 'static>(link: L) -> CoopSession {
        CoopSession {
            link: Box::new(link),
            peers: HashMap::new(),
            pings: HashMap::new(),
            pending_pings: HashMap::new(),
            next_ping: Instant::now(),
        }
    }

    /// The last measured round trip to a runner through the relay, in ms; None until one is measured.
    pub fn ping_ms(&self, peer: u32) -> Option<u32> {
        self.pings.get(&peer).copied()
    }

    pub fn is_host(&self) -> bool {
        self.link.role() == Role::Host
    }

    /// Relay session of a peer id (guests: the session; the host is reached with session 0 from a guest).
    fn route(&self, peer: u32) -> u32 {
        if self.is_host() {
            peer
        } else {
            0
        }
    }

    /// Sends to one peer (a guest sends to [`HOST_PEER_ID`]). False when the message is too big or a guest sends a
    /// host-only RPC.
    pub fn send(&mut self, peer: u32, message: &Message) -> bool {
        if message.host_only() && !self.is_host() {
            return false;
        }
        let body = message.encode();
        if body.len() > MAX_MESSAGE_BYTES {
            return false;
        }
        let channel = message.channel();
        let key = if self.is_host() { peer } else { HOST_PEER_ID };
        let state = &mut self.peers.entry(key).or_default().channels[channel as usize];
        let seq = state.next_send;
        state.next_send = state.next_send.wrapping_add(1);
        let bytes = packet(channel, KIND_DATA, seq, &body);
        if protocol::delivery(channel) == Delivery::Reliable {
            state.unacked.insert(seq, (bytes.clone(), Instant::now()));
        }
        let session = self.route(peer);
        self.link.send(session, &bytes)
    }

    /// Host: send to every guest seen so far (Godot `rpc()` to all).
    pub fn broadcast(&mut self, message: &Message) {
        let peers: Vec<u32> = self.peers.keys().copied().filter(|p| *p != HOST_PEER_ID).collect();
        for peer in peers {
            self.send(peer, message);
        }
    }

    pub fn peers(&self) -> Vec<u32> {
        self.peers.keys().copied().filter(|p| *p != HOST_PEER_ID).collect()
    }

    pub fn poll(&mut self) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        for event in self.link.poll() {
            match event {
                RelayEvent::Ready { .. } => out.push(SessionEvent::Ready),
                RelayEvent::Data { session, payload } => self.receive(session, &payload, &mut out),
                RelayEvent::PeerLeft(peer) => {
                    // a WebSocket guest left: its unacknowledged reliable packets are dropped, not resent forever
                    self.peers.remove(&peer);
                    self.pings.remove(&peer);
                    self.pending_pings.remove(&peer);
                    out.push(SessionEvent::Relay(RelayEvent::PeerLeft(peer)));
                }
                other => out.push(SessionEvent::Relay(other)),
            }
        }
        self.resend();
        self.ping();
        out
    }

    /// Once per [`PING_INTERVAL`], once linked: a guest pings the host; the host pings each guest and reports every
    /// measured ping to each guest, the guest's own left out.
    fn ping(&mut self) {
        let now = Instant::now();
        if now < self.next_ping || !self.link.ready() {
            return;
        }
        self.next_ping = now + PING_INTERVAL;
        let targets: Vec<u32> = if self.is_host() { self.peers() } else { vec![HOST_PEER_ID] };
        for peer in targets {
            let seq = self.pending_pings.get(&peer).map_or(1, |(seq, _)| seq.wrapping_add(1));
            self.pending_pings.insert(peer, (seq, now));
            self.link.send(self.route(peer), &packet(0, KIND_PING, seq, &[]));
        }
        if self.is_host() {
            for peer in self.peers() {
                let mut body = Vec::new();
                for (other, ms) in self.pings.iter().filter(|(other, _)| **other != peer && **other != HOST_PEER_ID) {
                    body.extend_from_slice(&other.to_le_bytes());
                    body.extend_from_slice(&ms.to_le_bytes());
                }
                self.link.send(peer, &packet(0, KIND_PINGS, 0, &body));
            }
        }
    }

    fn receive(&mut self, session: u32, payload: &[u8], out: &mut Vec<SessionEvent>) {
        if payload.len() < PACKET_HEADER || payload[0] as usize >= protocol::channel::COUNT {
            return;
        }
        let (channel, kind) = (payload[0], payload[1]);
        let seq = u32::from_le_bytes([payload[2], payload[3], payload[4], payload[5]]);
        let body = &payload[PACKET_HEADER..];
        let from = if self.is_host() { session } else { HOST_PEER_ID };
        match kind {
            KIND_PING => {
                self.link.send(self.route(session), &packet(0, KIND_PONG, seq, &[]));
                return;
            }
            KIND_PONG => {
                self.measure(from, seq);
                return;
            }
            KIND_PINGS => {
                if !self.is_host() {
                    self.report(body);
                }
                return;
            }
            _ => {}
        }
        if self.is_host() && !self.peers.contains_key(&from) {
            out.push(SessionEvent::PeerSeen(from));
        }
        let state = &mut self.peers.entry(from).or_default().channels[channel as usize];
        if kind == KIND_ACK {
            state.unacked.remove(&seq);
            return;
        }
        let mut deliver: Vec<Vec<u8>> = Vec::new();
        match protocol::delivery(channel) {
            Delivery::Reliable => {
                let ack = packet(channel, KIND_ACK, seq, &[]);
                let route = if self.is_host() { session } else { 0 };
                self.link.send(route, &ack);
                let state = &mut self.peers.get_mut(&from).unwrap().channels[channel as usize];
                if seq.wrapping_sub(state.next_receive) < u32::MAX / 2 {
                    state.held.insert(seq, body.to_vec());
                }
                while let Some(next) = state.held.remove(&state.next_receive) {
                    deliver.push(next);
                    state.next_receive = state.next_receive.wrapping_add(1);
                }
            }
            Delivery::UnreliableOrdered => {
                let newer = state.newest_seen.map(|n| seq.wrapping_sub(n) != 0 && seq.wrapping_sub(n) < u32::MAX / 2).unwrap_or(true);
                if newer {
                    state.newest_seen = Some(seq);
                    deliver.push(body.to_vec());
                }
            }
            Delivery::Unreliable => deliver.push(body.to_vec()),
        }
        for bytes in deliver {
            if let Some(message) = Message::decode(&bytes) {
                // a guest may not speak for the host (Godot drops non-authority calls to "authority" RPCs)
                if self.is_host() && message.host_only() {
                    continue;
                }
                out.push(SessionEvent::Message { from, message });
            }
        }
    }

    /// Records the round trip of the ping `from` answered with `seq`, when it is the one still pending.
    fn measure(&mut self, from: u32, seq: u32) {
        if let Some((pending, sent)) = self.pending_pings.get(&from).copied() {
            if pending == seq {
                self.pending_pings.remove(&from);
                let ms = u32::try_from(sent.elapsed().as_millis()).unwrap_or(u32::MAX);
                self.pings.insert(from, ms);
            }
        }
    }

    /// Guest: the host's report replaces the other guests' pings. Its own ping to the host is kept.
    fn report(&mut self, body: &[u8]) {
        self.pings.retain(|peer, _| *peer == HOST_PEER_ID);
        for pair in body.chunks_exact(8) {
            let peer = u32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]);
            let ms = u32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]);
            self.pings.insert(peer, ms);
        }
    }

    fn resend(&mut self) {
        let host = self.is_host();
        let mut due: Vec<(u32, Vec<u8>)> = Vec::new();
        for (peer, state) in self.peers.iter_mut() {
            for channel in state.channels.iter_mut() {
                for (bytes, sent) in channel.unacked.values_mut() {
                    if sent.elapsed() >= RESEND_INTERVAL {
                        *sent = Instant::now();
                        due.push((if host { *peer } else { 0 }, bytes.clone()));
                    }
                }
            }
        }
        for (session, bytes) in due {
            self.link.send(session, &bytes);
        }
    }

    pub fn close(&mut self) {
        self.link.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::relay::{decode, encode, frame, RelayLink};
    use serde_json::json;
    use std::net::UdpSocket;

    /// Fake relay forwarding between one host and one guest, dropping every 3rd reliable data packet once.
    #[test]
    fn reliable_messages_arrive_in_order_through_a_lossy_relay() {
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        relay.set_nonblocking(true).unwrap();
        let port = relay.local_addr().unwrap().port();
        let mut host = CoopSession::new(RelayLink::host("ABCDE", "tok", "127.0.0.1", port).unwrap());
        let mut guest = CoopSession::new(RelayLink::guest("ABCDE", "127.0.0.1", port, "").unwrap());
        let (mut host_addr, mut guest_addr) = (None, None);
        let mut dropped = std::collections::HashSet::new();
        let mut counter = 0;
        let mut got = Vec::new();
        let mut sent = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buf = [0u8; 2048];
        while Instant::now() < deadline && got.len() < 10 {
            while let Ok((n, from)) = relay.recv_from(&mut buf) {
                let f = decode(&buf[..n]).unwrap();
                match f.kind {
                    frame::HOST_REGISTER => {
                        host_addr = Some(from);
                        relay.send_to(&encode(frame::HOST_ACK, 0, "ABCDE", &[]).unwrap(), from).unwrap();
                    }
                    frame::GUEST_JOIN => {
                        guest_addr = Some(from);
                        relay.send_to(&encode(frame::GUEST_ACK, 42, "ABCDE", &[]).unwrap(), from).unwrap();
                    }
                    frame::HOST_DATA => {
                        counter += 1;
                        let key = f.payload.clone();
                        if f.payload[1] == 0 && counter % 3 == 0 && dropped.insert(key) {
                            continue;
                        }
                        relay.send_to(&encode(frame::HOST_DATA, 42, "ABCDE", &f.payload).unwrap(), guest_addr.unwrap()).unwrap();
                    }
                    frame::GUEST_DATA => {
                        relay.send_to(&encode(frame::GUEST_DATA, 42, "ABCDE", &f.payload).unwrap(), host_addr.unwrap()).unwrap();
                    }
                    _ => {}
                }
            }
            for e in guest.poll() {
                match e {
                    SessionEvent::Ready => {
                        guest.send(HOST_PEER_ID, &Message::RegisterPeer { requested_name: "R-4821".into() });
                    }
                    SessionEvent::Message { from, message: Message::Chat { message, .. } } => {
                        assert_eq!(from, HOST_PEER_ID);
                        got.push(message);
                    }
                    _ => {}
                }
            }
            for e in host.poll() {
                if let SessionEvent::Message { from, message: Message::RegisterPeer { requested_name } } = e {
                    assert_eq!((from, requested_name.as_str()), (42, "R-4821"));
                    if !sent {
                        sent = true;
                        for i in 0..10 {
                            host.send(42, &Message::Chat { peer_id: 1, display_name: "HOST".into(), message: format!("m{i}") });
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got, (0..10).map(|i| format!("m{i}")).collect::<Vec<_>>());
        assert!(!dropped.is_empty(), "the relay really dropped packets");
        assert!(!guest.send(HOST_PEER_ID, &Message::Snapshot { state: Default::default() }), "guests cannot send host RPCs");
        assert!(!host.send(42, &Message::Place { packet: json!("x".repeat(2000)) }), "a message must fit one datagram");
        host.close();
    }

    /// Host and guest on one fake relay: the guest measures its round trip to the host and the host to the guest.
    #[test]
    fn pings_are_measured_through_the_relay() {
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        relay.set_nonblocking(true).unwrap();
        let port = relay.local_addr().unwrap().port();
        let mut host = CoopSession::new(RelayLink::host("ABCDE", "tok", "127.0.0.1", port).unwrap());
        let mut guest = CoopSession::new(RelayLink::guest("ABCDE", "127.0.0.1", port, "").unwrap());
        let (mut host_addr, mut guest_addr) = (None, None);
        let mut buf = [0u8; 2048];
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && (host.ping_ms(42).is_none() || guest.ping_ms(HOST_PEER_ID).is_none()) {
            while let Ok((n, from)) = relay.recv_from(&mut buf) {
                let f = decode(&buf[..n]).unwrap();
                match f.kind {
                    frame::HOST_REGISTER => {
                        host_addr = Some(from);
                        relay.send_to(&encode(frame::HOST_ACK, 0, "ABCDE", &[]).unwrap(), from).unwrap();
                    }
                    frame::GUEST_JOIN => {
                        guest_addr = Some(from);
                        relay.send_to(&encode(frame::GUEST_ACK, 42, "ABCDE", &[]).unwrap(), from).unwrap();
                    }
                    frame::GUEST_DATA => {
                        relay.send_to(&encode(frame::GUEST_DATA, 42, "ABCDE", &f.payload).unwrap(), host_addr.unwrap()).unwrap();
                    }
                    frame::HOST_DATA => {
                        relay.send_to(&encode(frame::HOST_DATA, 42, "ABCDE", &f.payload).unwrap(), guest_addr.unwrap()).unwrap();
                    }
                    _ => {}
                }
            }
            for e in guest.poll() {
                if e == SessionEvent::Ready {
                    guest.send(HOST_PEER_ID, &Message::RegisterPeer { requested_name: "R-4821".into() });
                }
            }
            host.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host.ping_ms(42).is_some(), "the host measures its round trip to the guest");
        assert!(guest.ping_ms(HOST_PEER_ID).is_some(), "the guest measures its round trip to the host");
        assert!(host.ping_ms(HOST_PEER_ID).is_none(), "the host holds no ping to itself");
        host.close();
        guest.close();
    }
}
