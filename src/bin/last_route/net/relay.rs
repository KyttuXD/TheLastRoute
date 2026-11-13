//! The client side of the LRR1 gameplay relay, from `game/network/enet_relay_tunnel.gd` (wire format in
//! `backend/enet_relay.mjs`): `"LRR1"` | type u8 | session u32 LE | 5-char room code | payload; at most 1472 bytes.
//!
//! The Godot tunnel bridges a local ENet socket to the relay. This port has no ENet, so [`RelayLink`] exposes the
//! relay directly: a host registers (HOST_REGISTER every 3 s, HOST_ALIVE every 250 ms from its own thread and
//! socket, like the original's keep-alive thread), a guest joins (GUEST_JOIN every 750 ms until GUEST_ACK, with a
//! 12-byte random nonce or a reserved room's join ticket), and both exchange opaque datagrams
//! (GUEST_DATA / HOST_DATA) that [`crate::net::session`] fills.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use crate::shared::clock::Instant;
use std::time::Duration;

pub const HEADER_BYTES: usize = 14;
pub const MAX_DATAGRAM_BYTES: usize = 1472;
pub const MAX_PAYLOAD_BYTES: usize = MAX_DATAGRAM_BYTES - HEADER_BYTES;
pub const HOST_REGISTER_INTERVAL: Duration = Duration::from_millis(3000);
pub const GUEST_JOIN_INTERVAL: Duration = Duration::from_millis(750);
pub const HOST_ALIVE_INTERVAL: Duration = Duration::from_millis(250);
pub const HANDOFF_PREFIX: &str = "LRH1";

pub mod frame {
    pub const HOST_REGISTER: u8 = 1;
    pub const HOST_ACK: u8 = 2;
    pub const GUEST_JOIN: u8 = 3;
    pub const GUEST_ACK: u8 = 4;
    pub const GUEST_DATA: u8 = 5;
    pub const HOST_DATA: u8 = 6;
    pub const ERROR: u8 = 7;
    pub const HOST_PROMOTE: u8 = 8;
    pub const HOST_MIGRATING: u8 = 9;
    pub const HOST_LEFT: u8 = 10;
    pub const HOST_ALIVE: u8 = 11;
    /// relay -> would-be host of a hostless public code: a live host exists, join as a guest.
    pub const HOST_JOIN_AS_GUEST: u8 = 12;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub session: u32,
    pub code: String,
    pub payload: Vec<u8>,
}

fn valid_code(code: &str) -> bool {
    code.len() == 5 && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// `encode_frame(kind, session, code, payload)`; None for a bad code or an oversized datagram.
pub fn encode(kind: u8, session: u32, code: &str, payload: &[u8]) -> Option<Vec<u8>> {
    let code = code.trim().to_uppercase();
    if !valid_code(&code) || HEADER_BYTES + payload.len() > MAX_DATAGRAM_BYTES {
        return None;
    }
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len());
    out.extend_from_slice(b"LRR1");
    out.push(kind);
    out.extend_from_slice(&session.to_le_bytes());
    out.extend_from_slice(code.as_bytes());
    out.extend_from_slice(payload);
    Some(out)
}

pub fn decode(bytes: &[u8]) -> Option<Frame> {
    if bytes.len() < HEADER_BYTES || bytes.len() > MAX_DATAGRAM_BYTES || &bytes[..4] != b"LRR1" {
        return None;
    }
    let code = std::str::from_utf8(&bytes[9..14]).ok()?.to_string();
    if !valid_code(&code) {
        return None;
    }
    Some(Frame {
        kind: bytes[4],
        session: u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]),
        code,
        payload: bytes[HEADER_BYTES..].to_vec(),
    })
}

/// `parse_handoff_payload`: `LRH1|epoch` or `LRH1|epoch|token-or-ready` -> (epoch, tail).
pub fn parse_handoff(payload: &[u8]) -> Option<(i64, String)> {
    let text = std::str::from_utf8(payload).ok()?;
    let mut parts = text.splitn(3, '|');
    if parts.next()? != HANDOFF_PREFIX {
        return None;
    }
    let epoch: i64 = parts.next()?.parse().ok()?;
    if epoch <= 0 {
        return None;
    }
    Some((epoch, parts.next().unwrap_or("").to_string()))
}

/// What the relay told us.
#[derive(Clone, Debug, PartialEq)]
pub enum RelayEvent {
    /// HOST_ACK (host) / GUEST_ACK (guest, with its session id).
    Ready { session: u32 },
    /// A datagram: for a host, from guest `session`; for a guest, from the host.
    Data { session: u32, payload: Vec<u8> },
    /// ERROR frame text (e.g. "ROOM HOST HAS NOT REACHED THE RELAY", "PRIVATE ROOM // ACCESS REQUIRED").
    Error(String),
    /// HOST_LEFT: the room is gone.
    HostLeft(String),
    /// HOST_PROMOTE: this guest becomes the host with `token` (legacy: raw token; handoff: epoch > 0).
    Promoted { token: String, epoch: i64 },
    /// HOST_MIGRATING: wait for the successor (`ready` once it registered).
    Migrating { epoch: i64, ready: bool },
    /// HOST_JOIN_AS_GUEST: the public route already has a host; this host registration did not take.
    JoinAsGuest,
    /// WebSocket relay only (`/api/ws`): a guest joined the room (host side). The UDP relay never sends it.
    PeerJoined(u32),
    /// WebSocket relay only: a guest left the room (host side).
    PeerLeft(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Host,
    Guest,
}

/// One relay connection (host or guest). Call [`RelayLink::poll`] every frame.
pub struct RelayLink {
    socket: UdpSocket,
    relay: SocketAddr,
    code: String,
    role: Role,
    token: Arc<Mutex<String>>,
    nonce: Vec<u8>,
    session: u32,
    ready: bool,
    last_hello: Option<Instant>,
    alive_stop: Arc<AtomicBool>,
    alive_thread: Option<std::thread::JoinHandle<()>>,
    pub alive_sent: Arc<std::sync::atomic::AtomicU64>,
}

fn resolve(address: &str, port: u16) -> std::io::Result<SocketAddr> {
    (address, port)
        .to_socket_addrs()?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "relay address did not resolve"))
}

impl RelayLink {
    /// `start_host(code, token, ...)`: register with the relay and start the HOST_ALIVE thread.
    pub fn host(code: &str, token: &str, relay_address: &str, relay_port: u16) -> std::io::Result<RelayLink> {
        let mut link = RelayLink::open(code, relay_address, relay_port, Role::Host)?;
        *link.token.lock().unwrap() = token.to_string();
        link.start_keep_alive()?;
        link.hello();
        Ok(link)
    }

    /// `start_guest(code, address, port, join_ticket)`: a reserved room's ticket (base64url) is the nonce,
    /// otherwise 12 random bytes.
    pub fn guest(code: &str, relay_address: &str, relay_port: u16, join_ticket: &str) -> std::io::Result<RelayLink> {
        let mut link = RelayLink::open(code, relay_address, relay_port, Role::Guest)?;
        let ticket = super::base64_decode(join_ticket).unwrap_or_default();
        link.nonce = if (8..=32).contains(&ticket.len()) { ticket } else { super::random_bytes(12) };
        link.hello();
        Ok(link)
    }

    fn open(code: &str, relay_address: &str, relay_port: u16, role: Role) -> std::io::Result<RelayLink> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        Ok(RelayLink {
            socket,
            relay: resolve(relay_address, relay_port)?,
            code: code.trim().to_uppercase(),
            role,
            token: Arc::new(Mutex::new(String::new())),
            nonce: Vec::new(),
            session: 0,
            ready: false,
            last_hello: None,
            alive_stop: Arc::new(AtomicBool::new(false)),
            alive_thread: None,
            alive_sent: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    fn start_keep_alive(&mut self) -> std::io::Result<()> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        let (stop, token, code, relay, sent) =
            (self.alive_stop.clone(), self.token.clone(), self.code.clone(), self.relay, self.alive_sent.clone());
        self.alive_thread = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let payload = token.lock().unwrap().clone();
                if let Some(frame) = encode(frame::HOST_ALIVE, 0, &code, payload.as_bytes()) {
                    if socket.send_to(&frame, relay).is_ok() {
                        sent.fetch_add(1, Ordering::Relaxed);
                    }
                }
                std::thread::sleep(HOST_ALIVE_INTERVAL);
            }
        }));
        Ok(())
    }

    fn hello(&mut self) {
        self.last_hello = Some(Instant::now());
        let frame = match self.role {
            Role::Host => encode(frame::HOST_REGISTER, 0, &self.code, self.token.lock().unwrap().as_bytes()),
            Role::Guest => encode(frame::GUEST_JOIN, 0, &self.code, &self.nonce),
        };
        if let Some(frame) = frame {
            let _ = self.socket.send_to(&frame, self.relay);
        }
    }

    pub fn role(&self) -> Role {
        self.role
    }
    pub fn ready(&self) -> bool {
        self.ready
    }
    /// `guest_session()`.
    pub fn session(&self) -> u32 {
        self.session
    }
    pub fn code(&self) -> &str {
        &self.code
    }

    /// `update_host_token(token)` (after a heartbeat rotated it / a promotion).
    pub fn update_host_token(&mut self, token: &str) {
        *self.token.lock().unwrap() = token.to_string();
    }

    /// `promote_to_host(token)`: this guest's socket registers as the room's new host.
    pub fn promote_to_host(&mut self, token: &str) -> std::io::Result<()> {
        self.role = Role::Host;
        self.ready = false;
        self.update_host_token(token);
        if self.alive_thread.is_none() {
            self.start_keep_alive()?;
        }
        self.hello();
        Ok(())
    }

    /// Host -> guest `session`, or guest -> host (`session` ignored).
    pub fn send(&self, session: u32, payload: &[u8]) -> bool {
        let (kind, session) = match self.role {
            Role::Host => (frame::HOST_DATA, session),
            Role::Guest => (frame::GUEST_DATA, self.session),
        };
        if !self.ready && self.role == Role::Guest {
            return false;
        }
        match encode(kind, session, &self.code, payload) {
            Some(frame) => self.socket.send_to(&frame, self.relay).is_ok(),
            None => false,
        }
    }

    /// Drains the socket and re-sends the hello when due.
    pub fn poll(&mut self) -> Vec<RelayEvent> {
        let interval = match self.role {
            Role::Host => HOST_REGISTER_INTERVAL,
            Role::Guest if self.ready => Duration::MAX,
            Role::Guest => GUEST_JOIN_INTERVAL,
        };
        if self.last_hello.map(|t| t.elapsed() >= interval).unwrap_or(true) {
            self.hello();
        }
        let mut events = Vec::new();
        let mut buffer = [0u8; 2048];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((n, from)) => {
                    if from != self.relay {
                        continue;
                    }
                    if let Some(f) = decode(&buffer[..n]) {
                        if f.code == self.code {
                            self.handle(f, &mut events);
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        events
    }

    fn handle(&mut self, f: Frame, events: &mut Vec<RelayEvent>) {
        let text = || String::from_utf8_lossy(&f.payload).to_string();
        match f.kind {
            frame::HOST_ACK if self.role == Role::Host => {
                if !self.ready {
                    self.ready = true;
                    events.push(RelayEvent::Ready { session: 0 });
                }
            }
            frame::GUEST_ACK if self.role == Role::Guest => {
                self.session = f.session;
                if !self.ready {
                    self.ready = true;
                    events.push(RelayEvent::Ready { session: f.session });
                }
            }
            frame::GUEST_DATA if self.role == Role::Host => events.push(RelayEvent::Data { session: f.session, payload: f.payload }),
            frame::HOST_DATA if self.role == Role::Guest => events.push(RelayEvent::Data { session: f.session, payload: f.payload }),
            frame::HOST_JOIN_AS_GUEST if self.role == Role::Host => events.push(RelayEvent::JoinAsGuest),
            frame::ERROR => events.push(RelayEvent::Error(text())),
            frame::HOST_LEFT => events.push(RelayEvent::HostLeft(text())),
            frame::HOST_PROMOTE if self.role == Role::Guest && (f.session == self.session || f.session == 0) => {
                match parse_handoff(&f.payload) {
                    Some((epoch, token)) => events.push(RelayEvent::Promoted { token, epoch }),
                    None => events.push(RelayEvent::Promoted { token: text(), epoch: 0 }),
                }
            }
            frame::HOST_MIGRATING if self.role == Role::Guest => match parse_handoff(&f.payload) {
                Some((epoch, tail)) => events.push(RelayEvent::Migrating { epoch, ready: tail == "ready" }),
                None => events.push(RelayEvent::Migrating { epoch: 0, ready: f.session == 0 }),
            },
            _ => {}
        }
    }

    /// `close()`: stops the keep-alive thread (the HTTP unregister tells the guests).
    pub fn close(&mut self) {
        self.alive_stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.alive_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RelayLink {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_match_the_server_layout() {
        let f = encode(frame::GUEST_DATA, 0x01020304, "abcde", b"x").unwrap();
        assert_eq!(f, b"LRR1\x05\x04\x03\x02\x01ABCDEx".to_vec());
        assert_eq!(decode(&f).unwrap(), Frame { kind: 5, session: 0x01020304, code: "ABCDE".into(), payload: b"x".to_vec() });
        assert!(encode(1, 0, "ab!12", b"").is_none());
        assert!(encode(1, 0, "ABCDE", &[0u8; 1459]).is_none());
        assert_eq!(parse_handoff(b"LRH1|7|tok"), Some((7, "tok".into())));
        assert_eq!(parse_handoff(b"LRH1|7"), Some((7, String::new())));
        assert_eq!(parse_handoff(b"LRH1|0|x"), None);
    }

    #[test]
    fn a_host_told_to_join_as_a_guest_gets_the_event() {
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        relay.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let mut host = RelayLink::host("PUBLC", "stale", "127.0.0.1", relay.local_addr().unwrap().port()).unwrap();
        host.poll();
        let mut buf = [0u8; 2048];
        // a keep-alive can land before the register, so read until the register arrives
        let from = loop {
            let (n, from) = relay.recv_from(&mut buf).unwrap();
            if decode(&buf[..n]).unwrap().kind == frame::HOST_REGISTER {
                break from;
            }
            host.poll();
        };
        relay.send_to(&encode(frame::HOST_JOIN_AS_GUEST, 0, "PUBLC", &[]).unwrap(), from).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut seen = Vec::new();
        while Instant::now() < deadline && seen.is_empty() {
            seen = host.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(seen, vec![RelayEvent::JoinAsGuest]);
        host.close();
    }

    /// A fake relay on loopback: acks, then forwards guest data to the host.
    #[test]
    fn host_and_guest_handshake_against_a_loopback_relay() {
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        relay.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let port = relay.local_addr().unwrap().port();
        let mut host = RelayLink::host("ABCDE", "tok", "127.0.0.1", port).unwrap();
        let mut guest = RelayLink::guest("ABCDE", "127.0.0.1", port, "").unwrap();
        let (mut host_addr, mut guest_addr, mut alive) = (None, None, 0);
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut buf = [0u8; 2048];
        let (mut host_got, mut guest_ready) = (None, false);
        while Instant::now() < deadline && (host_got.is_none() || alive == 0) {
            if let Ok((n, from)) = relay.recv_from(&mut buf) {
                let f = decode(&buf[..n]).unwrap();
                match f.kind {
                    frame::HOST_REGISTER => {
                        assert_eq!(f.payload, b"tok");
                        host_addr = Some(from);
                        relay.send_to(&encode(frame::HOST_ACK, 0, "ABCDE", &[]).unwrap(), from).unwrap();
                    }
                    frame::HOST_ALIVE => alive += 1,
                    frame::GUEST_JOIN => {
                        assert_eq!(f.payload.len(), 12);
                        guest_addr = Some(from);
                        relay.send_to(&encode(frame::GUEST_ACK, 77, "ABCDE", &[]).unwrap(), from).unwrap();
                    }
                    frame::GUEST_DATA => {
                        assert_eq!(f.session, 77);
                        relay.send_to(&encode(frame::GUEST_DATA, 77, "ABCDE", &f.payload).unwrap(), host_addr.unwrap()).unwrap();
                    }
                    _ => {}
                }
            }
            for e in guest.poll() {
                if e == (RelayEvent::Ready { session: 77 }) {
                    guest_ready = true;
                    assert!(guest.send(0, b"hello host"));
                }
            }
            for e in host.poll() {
                if let RelayEvent::Data { session, payload } = e {
                    host_got = Some((session, payload));
                }
            }
        }
        assert!(guest_ready && guest_addr.is_some());
        assert_eq!(host_got, Some((77, b"hello host".to_vec())));
        assert!(alive > 0, "HOST_ALIVE keep-alives were sent");
        host.close();
    }
}
