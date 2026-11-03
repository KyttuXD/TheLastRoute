//! A Rust-to-Rust co-op transport over the LRR1 relay (PORT DEVIATION: replaces ENet + Godot SceneMultiplayer).
//!
//! Each relay datagram payload is one packet: `channel u8 | kind u8 (0 data, 1 ack) | seq u32 LE | body`, where the
//! body of a data packet is one encoded [`Message`]. Delivery follows the original channel modes
//! ([`protocol::delivery`]): reliable channels (EVENTS, HUNTERS_LARGE) are acknowledged, resent every 200 ms and
//! delivered in order; unreliable_ordered channels (STATE, HUNTERS) drop anything older than the newest seen;
//! VOICE is plain unreliable. Peer ids: the host is peer 1 (Godot's authority id); a guest is its relay session id.
//! A message must fit one datagram (1450 bytes); bigger hunter frames are not fragmented (ENet does that for Godot).

use super::protocol::{self, Delivery, Message};
use super::relay::{RelayEvent, RelayLink, Role, MAX_PAYLOAD_BYTES};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

pub const HOST_PEER_ID: u32 = 1;
pub const RESEND_INTERVAL: Duration = Duration::from_millis(200);
const PACKET_HEADER: usize = 6;
pub const MAX_MESSAGE_BYTES: usize = MAX_PAYLOAD_BYTES - PACKET_HEADER;

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
    pub link: RelayLink,
    peers: HashMap<u32, Peer>,
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
    pub fn new(link: RelayLink) -> CoopSession {
        CoopSession { link, peers: HashMap::new() }
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
        let bytes = packet(channel, 0, seq, &body);
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
                other => out.push(SessionEvent::Relay(other)),
            }
        }
        self.resend();
        out
    }

    fn receive(&mut self, session: u32, payload: &[u8], out: &mut Vec<SessionEvent>) {
        if payload.len() < PACKET_HEADER || payload[0] as usize >= protocol::channel::COUNT {
            return;
        }
        let (channel, kind) = (payload[0], payload[1]);
        let seq = u32::from_le_bytes([payload[2], payload[3], payload[4], payload[5]]);
        let body = &payload[PACKET_HEADER..];
        let from = if self.is_host() { session } else { HOST_PEER_ID };
        if self.is_host() && !self.peers.contains_key(&from) {
            out.push(SessionEvent::PeerSeen(from));
        }
        let state = &mut self.peers.entry(from).or_default().channels[channel as usize];
        if kind == 1 {
            state.unacked.remove(&seq);
            return;
        }
        let mut deliver: Vec<Vec<u8>> = Vec::new();
        match protocol::delivery(channel) {
            Delivery::Reliable => {
                let ack = packet(channel, 1, seq, &[]);
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
    use crate::net::relay::{decode, encode, frame};
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
}
