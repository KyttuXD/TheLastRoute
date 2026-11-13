//! Port of `backend/enet_relay.mjs`: a tiny authenticated UDP tunnel for Godot's ENet transport (UDP 1105).
//!
//! Both players make outbound UDP connections to this public socket, so a host behind CGNAT never needs an inbound
//! port, UPnP or router configuration. The relay never parses ENet: it wraps opaque datagrams.
//!
//! ## Wire format (byte-identical to the Node relay)
//! Every datagram is `"LRR1"` (4 bytes) | type u8 | sessionId u32 little-endian | room code (5 ASCII `[A-Z0-9]`)
//! | payload (the remaining bytes); header 14 bytes, whole datagram at most 1472 bytes.
//!
//! | type | name | direction | session | payload |
//! |---|---|---|---|---|
//! | 1 | HOST_REGISTER | host -> relay | 0 | host token (from `/api/enet/rooms/register`) |
//! | 2 | HOST_ACK | relay -> host | 0 | empty |
//! | 3 | GUEST_JOIN | guest -> relay | 0 | nonce, 8..32 bytes (reserved rooms: the join ticket bytes) |
//! | 4 | GUEST_ACK | relay -> guest | new session id | empty |
//! | 5 | GUEST_DATA | guest -> relay -> host | guest session | raw ENet datagram |
//! | 6 | HOST_DATA | host -> relay -> guest | guest session | raw ENet datagram |
//! | 7 | ERROR | relay -> anyone | 0 | ASCII message (<= 160 chars) |
//! | 8 | HOST_PROMOTE | relay -> chosen guest | its session | new host token, or `LRH1|<epoch>|<token>` |
//! | 9 | HOST_MIGRATING | relay -> other guests | session or 0 | empty, `LRH1|<epoch>` or `LRH1|<epoch>|ready` |
//! | 10 | HOST_LEFT | relay -> guests | session | reason (`HOST LEFT // ROOM CLOSED`) |
//! | 11 | HOST_ALIVE | host keep-alive thread -> relay | 0 | host token, every ~250 ms |
//! | 12 | HOST_JOIN_AS_GUEST | relay -> would-be host | 0 | empty (hostless public code: a live host exists, join as a guest) |
//!
//! A hostless public code (`RelayRooms::hostless_room`) takes HOST_REGISTER and HOST_ALIVE without a token, but HOST_ALIVE only
//! from the current host's endpoint, and a registrant must pass `hostless_host_allowed`.
//!
//! Timers: a host that sends HOST_ALIVE is closed 1 s after the last one (older hosts: 15 s after the last
//! HOST_REGISTER); guest sessions expire after 45 s idle; a deliberate handoff whose successor never registers closes
//! the room after 20 s; migrations are re-announced every 2 s; prune runs every 250 ms.
//!
//! ## Design (port deviation: sans-IO)
//! [`EnetRelay`] is a synchronous state machine: [`EnetRelay::handle_datagram`], [`EnetRelay::prune`],
//! [`EnetRelay::close_room`], ... queue outgoing datagrams in an outbox ([`EnetRelay::take_outbox`]). The room
//! directory (the Node `roomByCode` / `promoteRoomHost` / `onRoomClosed` callbacks) is the [`RelayRooms`] trait,
//! passed to each call, so the router can keep both in one mutex without lock-order problems. [`RelayDriver`] owns
//! the UDP socket and a thread that feeds datagrams and 250 ms ticks into a handler closure.

use crate::util::{hex, random_bytes, safe_equal, Clock};
use indexmap::IndexMap;
use serde_json::{json, Map, Value};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const RELAY_MAGIC: &[u8; 4] = b"LRR1";
pub const RELAY_HEADER_BYTES: usize = 14;
pub const RELAY_MAX_DATAGRAM_BYTES: usize = 1472;

#[allow(non_snake_case)]
pub mod RelayFrame {
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
    pub const HOST_JOIN_AS_GUEST: u8 = 12;
}

pub const HOST_LEFT_MESSAGE: &str = "HOST LEFT // ROOM CLOSED";
pub const HANDOFF_FAILED_MESSAGE: &str = "HOST TRANSFER FAILED // ROOM CLOSED";
pub const HANDOFF_PREFIX: &str = "LRH1";

pub fn handoff_payload(epoch: i64, tail: &str) -> Vec<u8> {
    if tail.is_empty() {
        format!("{HANDOFF_PREFIX}|{epoch}").into_bytes()
    } else {
        format!("{HANDOFF_PREFIX}|{epoch}|{tail}").into_bytes()
    }
}

fn valid_code(code: &str) -> bool {
    code.len() == 5 && code.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// `encodeRelayFrame(type, sessionId, code, payload)`; Err("INVALID RELAY ROOM CODE") for a bad code.
pub fn encode_relay_frame(kind: u8, session_id: u32, code: &str, payload: &[u8]) -> Result<Vec<u8>, &'static str> {
    let room_code = code.trim().to_uppercase();
    if !valid_code(&room_code) {
        return Err("INVALID RELAY ROOM CODE");
    }
    let mut frame = Vec::with_capacity(RELAY_HEADER_BYTES + payload.len());
    frame.extend_from_slice(RELAY_MAGIC);
    frame.push(kind);
    frame.extend_from_slice(&session_id.to_le_bytes());
    frame.extend_from_slice(room_code.as_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedFrame {
    pub kind: u8,
    pub session_id: u32,
    pub code: String,
    pub payload: Vec<u8>,
}

pub fn decode_relay_frame(frame: &[u8]) -> Option<DecodedFrame> {
    if frame.len() < RELAY_HEADER_BYTES || frame.len() > RELAY_MAX_DATAGRAM_BYTES || &frame[0..4] != RELAY_MAGIC {
        return None;
    }
    let code = std::str::from_utf8(&frame[9..14]).ok()?.to_string();
    if !valid_code(&code) {
        return None;
    }
    Some(DecodedFrame {
        kind: frame[4],
        session_id: u32::from_le_bytes([frame[5], frame[6], frame[7], frame[8]]),
        code,
        payload: frame[RELAY_HEADER_BYTES..].to_vec(),
    })
}

/// What the relay reads from the room directory (`roomByCode(code)`).
#[derive(Debug, Clone)]
pub struct RelayRoom {
    pub token: String,
    /// A reserved private room: guests are admitted only through [`RelayRooms::admit_guest`].
    pub gated: bool,
}

/// The Node relay's injected callbacks.
pub trait RelayRooms {
    fn room_by_code(&mut self, code: &str) -> Option<RelayRoom>;
    /// The room a HOST_REGISTER checks its token against: like `room_by_code`, but a directory may still know a room
    /// that expired a moment ago (the host's heartbeat was late). It revives such an entry only when `token` matches.
    fn room_for_host_register(&mut self, code: &str, _token: &str) -> Option<RelayRoom> {
        self.room_by_code(code)
    }
    /// A hostless code (the public route) needs no host token: its entry is returned, created when missing, so a
    /// HOST_REGISTER from `host` always finds a room. None = a normal room, whose token is checked.
    fn hostless_room(&mut self, _code: &str, _host: SocketAddr) -> Option<RelayRoom> {
        None
    }
    /// Whether `host` may become the relay host of the hostless `code` (moderation and vouching live in the directory).
    /// `token` is the HOST_REGISTER payload: the entry's own token always passes.
    fn hostless_host_allowed(&mut self, _code: &str, _host: SocketAddr, _token: &str) -> bool {
        true
    }
    /// The current relay host of the hostless `code` sent HOST_ALIVE: the directory may refresh a relay-made entry.
    fn hostless_host_alive(&mut self, _code: &str) {}
    /// Whether `code` is hostless (HOST_ALIVE is then accepted without the token).
    fn is_hostless(&mut self, _code: &str) -> bool {
        false
    }
    /// A host proved its token (HOST_REGISTER): the directory may keep the entry alive.
    fn host_registered(&mut self, _code: &str) {}
    /// `promoteRoomHost(code)` -> the new host token. None = unavailable.
    fn promote_room_host(&mut self, _code: &str) -> Option<String> {
        None
    }
    /// `onRoomClosed(code, reason)`.
    fn on_room_closed(&mut self, _code: &str, _reason: &str) {}
    /// `room.admitGuest(nonce)` for gated rooms.
    fn admit_guest(&mut self, _code: &str, _nonce: &[u8]) -> bool {
        false
    }
}

pub type Outgoing = (SocketAddr, Vec<u8>);

pub struct EnetRelayOptions {
    pub now: Clock,
    pub host_ttl_ms: i64,
    pub host_alive_ttl_ms: i64,
    pub session_ttl_ms: i64,
    pub maximum_guests_per_room: usize,
    pub guest_allocation_window_ms: i64,
    pub maximum_guest_allocations_per_address: u32,
    /// Most `code|address` join counters kept at once. UDP source addresses can be forged, so a flood of joins from
    /// made-up addresses stops growing the table here: a new address is refused until old windows end.
    pub maximum_guest_allocation_keys: usize,
    pub handoff_ttl_ms: i64,
    pub socket_buffer_bytes: usize,
}

impl Default for EnetRelayOptions {
    fn default() -> Self {
        EnetRelayOptions {
            now: crate::util::system_clock(),
            host_ttl_ms: 15_000,
            host_alive_ttl_ms: 1_000,
            session_ttl_ms: 45_000,
            maximum_guests_per_room: 15,
            guest_allocation_window_ms: 10_000,
            maximum_guest_allocations_per_address: 4,
            maximum_guest_allocation_keys: 50_000,
            handoff_ttl_ms: 20_000,
            socket_buffer_bytes: 4 * 1024 * 1024,
        }
    }
}

struct Host {
    endpoint: SocketAddr,
    last_seen: i64,
    alive: bool,
}

struct Session {
    code: String,
    guest: SocketAddr,
    guest_key: String,
    endpoint_key: String,
    last_seen: i64,
    joined_at: i64,
}

#[derive(Clone)]
struct Migration {
    session_id: u32,
    token: String,
    epoch: i64,
    handoff: bool,
    started_at: i64,
    last_announced: i64,
}

#[derive(Default, Clone)]
pub struct RelayCounters {
    pub forwarded_guest: u64,
    pub forwarded_host: u64,
    pub forwarded_bytes: u64,
    pub dropped_invalid: u64,
    pub dropped_unknown_session: u64,
    pub dropped_no_host: u64,
    pub dropped_wrong_endpoint: u64,
    pub send_errors: u64,
    pub rooms_closed: u64,
}

pub struct EnetRelay {
    opts: EnetRelayOptions,
    hosts: IndexMap<String, Host>,
    sessions: IndexMap<u32, Session>,
    guest_sessions: IndexMap<String, u32>,
    guest_endpoints: IndexMap<String, u32>,
    guest_allocation_rates: IndexMap<String, (u32, i64)>,
    migrations: IndexMap<String, Migration>,
    listening: bool,
    relay_port: u16,
    last_error: String,
    socket_buffers: Option<(usize, usize)>,
    pub counters: RelayCounters,
    outbox: Vec<Outgoing>,
}

/// `requestHostHandoff` result: `{migrating, error?}`.
#[derive(Debug, Clone, PartialEq)]
pub struct HandoffResult {
    pub migrating: bool,
    pub error: Option<String>,
}

impl HandoffResult {
    fn ok() -> Self {
        HandoffResult { migrating: true, error: None }
    }
    fn refused(error: &str) -> Self {
        HandoffResult { migrating: false, error: Some(error.into()) }
    }
}

impl EnetRelay {
    pub fn new(opts: EnetRelayOptions) -> EnetRelay {
        EnetRelay {
            opts,
            hosts: IndexMap::new(),
            sessions: IndexMap::new(),
            guest_sessions: IndexMap::new(),
            guest_endpoints: IndexMap::new(),
            guest_allocation_rates: IndexMap::new(),
            migrations: IndexMap::new(),
            listening: false,
            relay_port: 0,
            last_error: String::new(),
            socket_buffers: None,
            counters: RelayCounters::default(),
            outbox: Vec::new(),
        }
    }

    fn now(&self) -> i64 {
        (self.opts.now)()
    }

    /// The socket is bound (the Node "listening" event).
    pub fn set_listening(&mut self, port: u16, buffers: Option<(usize, usize)>) {
        self.listening = true;
        self.relay_port = port;
        self.socket_buffers = buffers;
        self.last_error.clear();
    }

    pub fn note_send_error(&mut self) {
        self.counters.send_errors += 1;
    }

    /// The socket closed: forget everything (the Node "close" event).
    pub fn close(&mut self) {
        self.listening = false;
        self.hosts.clear();
        self.sessions.clear();
        self.guest_sessions.clear();
        self.guest_endpoints.clear();
        self.guest_allocation_rates.clear();
        self.migrations.clear();
    }

    pub fn take_outbox(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outbox)
    }

    /// The prune interval (`Math.max(100, Math.min(250, hostAliveTtlMs / 4))`).
    pub fn prune_interval(&self) -> Duration {
        Duration::from_millis((self.opts.host_alive_ttl_ms as f64 / 4.0).min(250.0).max(100.0) as u64)
    }

    fn send(&mut self, kind: u8, session_id: u32, code: &str, payload: &[u8], endpoint: SocketAddr) {
        if !self.listening {
            return;
        }
        if let Ok(frame) = encode_relay_frame(kind, session_id, code, payload) {
            self.outbox.push((endpoint, frame));
        }
    }

    fn send_error(&mut self, code: &str, endpoint: SocketAddr, message: &str) {
        let text: String = message.chars().take(160).collect();
        self.send(RelayFrame::ERROR, 0, code, text.as_bytes(), endpoint);
    }

    fn remove_session(&mut self, session_id: u32) {
        if let Some(session) = self.sessions.shift_remove(&session_id) {
            self.guest_sessions.shift_remove(&session.guest_key);
            self.guest_endpoints.shift_remove(&session.endpoint_key);
        }
    }

    fn allow_guest_allocation(&mut self, code: &str, address: &str) -> bool {
        let timestamp = self.now();
        let key = format!("{code}|{address}");
        let window = self.opts.guest_allocation_window_ms;
        if !self.guest_allocation_rates.contains_key(&key) && self.guest_allocation_rates.len() >= self.opts.maximum_guest_allocation_keys.max(1) {
            return false;
        }
        let entry = self.guest_allocation_rates.entry(key).or_insert((0, timestamp + window));
        if timestamp >= entry.1 {
            *entry = (0, timestamp + window);
        }
        entry.0 += 1;
        entry.0 <= self.opts.maximum_guest_allocations_per_address
    }

    fn remove_host(&mut self, code: &str) {
        self.hosts.shift_remove(code);
        self.migrations.shift_remove(code);
        let ids: Vec<u32> = self.sessions.iter().filter(|(_, s)| s.code == code).map(|(id, _)| *id).collect();
        for id in ids {
            self.remove_session(id);
        }
    }

    fn migration_candidates(&self, code: &str) -> Vec<u32> {
        let timestamp = self.now();
        let mut list: Vec<(u32, i64)> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.code == code && timestamp - s.last_seen <= self.opts.session_ttl_ms)
            .map(|(id, s)| (*id, s.joined_at))
            .collect();
        list.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        list.into_iter().map(|(id, _)| id).collect()
    }

    fn announce_migration(&mut self, code: &str) {
        let Some(migration) = self.migrations.get(code).cloned() else { return };
        let targets: Vec<(u32, SocketAddr)> =
            self.sessions.iter().filter(|(_, s)| s.code == code).map(|(id, s)| (*id, s.guest)).collect();
        for (session_id, guest) in targets {
            if session_id == migration.session_id {
                let payload = if migration.handoff {
                    handoff_payload(migration.epoch, &migration.token)
                } else {
                    migration.token.as_bytes().to_vec()
                };
                self.send(RelayFrame::HOST_PROMOTE, session_id, code, &payload, guest);
            } else {
                let payload = if migration.handoff { handoff_payload(migration.epoch, "") } else { Vec::new() };
                self.send(RelayFrame::HOST_MIGRATING, session_id, code, &payload, guest);
            }
        }
        let now = self.now();
        if let Some(m) = self.migrations.get_mut(code) {
            m.last_announced = now;
        }
    }

    fn live_session(&self, code: &str, session_id: u32) -> bool {
        self.sessions
            .get(&session_id)
            .map(|s| s.code == code && self.now() - s.last_seen <= self.opts.session_ttl_ms)
            .unwrap_or(false)
    }

    fn begin_handoff(&mut self, code: &str, candidate: u32, epoch: i64, rooms: &mut dyn RelayRooms) -> HandoffResult {
        if let Some(existing) = self.migrations.get(code) {
            if existing.handoff && existing.session_id == candidate && existing.epoch == epoch {
                self.announce_migration(code);
                return HandoffResult::ok();
            }
            return HandoffResult::refused("HOST TRANSFER ALREADY IN PROGRESS");
        }
        if !self.hosts.contains_key(code) {
            return HandoffResult::refused("ROOM HOST HAS NOT REACHED THE RELAY");
        }
        if !self.live_session(code, candidate) {
            return HandoffResult::refused("HANDOFF CANDIDATE IS NOT CONNECTED");
        }
        let token = rooms.promote_room_host(code).unwrap_or_default();
        if token.is_empty() {
            return HandoffResult::refused("HOST TRANSFER UNAVAILABLE");
        }
        let now = self.now();
        self.hosts.shift_remove(code);
        self.migrations.insert(
            code.to_string(),
            Migration { session_id: candidate, token, epoch, handoff: true, started_at: now, last_announced: 0 },
        );
        self.announce_migration(code);
        HandoffResult::ok()
    }

    fn begin_host_migration(&mut self, code: &str, rooms: &mut dyn RelayRooms) -> bool {
        if self.migrations.contains_key(code) {
            self.announce_migration(code);
            return true;
        }
        let candidates = self.migration_candidates(code);
        if candidates.is_empty() {
            self.remove_host(code);
            return false;
        }
        let token = rooms.promote_room_host(code).unwrap_or_default();
        if token.is_empty() {
            self.remove_host(code);
            return false;
        }
        let now = self.now();
        self.hosts.shift_remove(code);
        self.migrations.insert(
            code.to_string(),
            Migration { session_id: candidates[0], token, epoch: 0, handoff: false, started_at: now, last_announced: 0 },
        );
        self.announce_migration(code);
        true
    }

    fn host_ttl(&self, host: &Host) -> i64 {
        if host.alive {
            self.opts.host_alive_ttl_ms
        } else {
            self.opts.host_ttl_ms
        }
    }

    fn host_fresh(&self, code: &str) -> bool {
        self.hosts.get(code).map(|h| self.now() - h.last_seen <= self.host_ttl(h)).unwrap_or(false)
    }

    /// `closeRoom(code, reason)`: every guest gets HOST_LEFT (+ ERROR for older clients) and the directory forgets it.
    pub fn close_room(&mut self, code: &str, reason: Option<&str>, rooms: &mut dyn RelayRooms) -> bool {
        let code = code.trim().to_uppercase();
        let reason = reason.unwrap_or(HOST_LEFT_MESSAGE).to_string();
        let mut told = 0;
        let targets: Vec<(u32, SocketAddr)> =
            self.sessions.iter().filter(|(_, s)| s.code == code).map(|(id, s)| (*id, s.guest)).collect();
        for (session_id, guest) in targets {
            self.send(RelayFrame::HOST_LEFT, session_id, &code, reason.as_bytes(), guest);
            self.send_error(&code, guest, &reason);
            self.remove_session(session_id);
            told += 1;
        }
        let known = self.hosts.contains_key(&code) || self.migrations.contains_key(&code);
        self.hosts.shift_remove(&code);
        self.migrations.shift_remove(&code);
        self.counters.rooms_closed += 1;
        rooms.on_room_closed(&code, &reason);
        known || told > 0
    }

    /// `pruneNow()`: timers (host timeouts, idle sessions, migrations).
    pub fn prune(&mut self, rooms: &mut dyn RelayRooms) {
        let timestamp = self.now();
        let codes: Vec<String> = self.hosts.keys().cloned().collect();
        for code in codes {
            let Some(host) = self.hosts.get(&code) else { continue };
            let stale = timestamp - host.last_seen > self.host_ttl(host);
            if rooms.room_by_code(&code).is_none() {
                self.remove_host(&code);
            } else if stale {
                self.close_room(&code, None, rooms);
            }
        }
        let ids: Vec<u32> = self.sessions.keys().cloned().collect();
        for id in ids {
            let Some(session) = self.sessions.get(&id) else { continue };
            let (code, idle) = (session.code.clone(), timestamp - session.last_seen > self.opts.session_ttl_ms);
            if rooms.room_by_code(&code).is_none() || idle {
                self.remove_session(id);
            }
        }
        self.guest_allocation_rates.retain(|_, rate| timestamp < rate.1);
        let codes: Vec<String> = self.migrations.keys().cloned().collect();
        for code in codes {
            let Some(migration) = self.migrations.get(&code).cloned() else { continue };
            if rooms.room_by_code(&code).is_none() {
                self.remove_host(&code);
                continue;
            }
            let candidate = self.sessions.get(&migration.session_id).map(|s| (s.code.clone(), s.last_seen));
            if migration.handoff {
                let gone = match &candidate {
                    Some((c, _)) => *c != code,
                    None => true,
                };
                if gone || timestamp - migration.started_at > self.opts.handoff_ttl_ms {
                    self.close_room(&code, Some(HANDOFF_FAILED_MESSAGE), rooms);
                } else if timestamp - migration.last_announced >= 2_000 {
                    self.announce_migration(&code);
                }
                continue;
            }
            let lost = match &candidate {
                Some((c, seen)) => *c != code || timestamp - seen > self.opts.session_ttl_ms,
                None => true,
            };
            if lost {
                self.migrations.shift_remove(&code);
                self.begin_host_migration(&code, rooms);
            } else if timestamp - migration.last_announced >= 2_000 {
                self.announce_migration(&code);
            }
        }
    }

    /// One received datagram (the Node socket "message" handler).
    pub fn handle_datagram(&mut self, remote: SocketAddr, packet: &[u8], rooms: &mut dyn RelayRooms) {
        let Some(frame) = decode_relay_frame(packet) else {
            self.counters.dropped_invalid += 1;
            return;
        };
        let code = frame.code.clone();
        let payload_text = String::from_utf8_lossy(&frame.payload).to_string();
        let hostless = if frame.kind == RelayFrame::HOST_REGISTER { rooms.hostless_room(&code, remote) } else { None };
        let room = if hostless.is_some() {
            hostless.clone()
        } else if frame.kind == RelayFrame::HOST_REGISTER {
            rooms.room_for_host_register(&code, &payload_text)
        } else {
            rooms.room_by_code(&code)
        };

        if frame.kind == RelayFrame::HOST_REGISTER {
            if hostless.is_some() {
                let holder = self.hosts.get(&code).map(|h| h.endpoint).filter(|_| self.host_fresh(&code));
                // somebody else already holds the public route: the registrant joins as a guest. The same address on a
                // new port is the holder restarting (a new NAT mapping) and replaces it.
                if holder.is_some_and(|h| h != remote && h.ip() != remote.ip()) {
                    self.send(RelayFrame::HOST_JOIN_AS_GUEST, 0, &code, &[], remote);
                    return;
                }
                if !rooms.hostless_host_allowed(&code, remote, &payload_text) {
                    self.send_error(&code, remote, "HOST AUTHORIZATION FAILED");
                    return;
                }
            }
            let authorised =
                hostless.is_some() || room.as_ref().map(|r| safe_equal(&payload_text, &r.token)).unwrap_or(false);
            if !authorised {
                self.send_error(&code, remote, "HOST AUTHORIZATION FAILED");
                return;
            }
            rooms.host_registered(&code);
            let alive = self.hosts.get(&code).map(|h| h.alive).unwrap_or(false);
            let now = self.now();
            self.hosts.insert(code.clone(), Host { endpoint: remote, last_seen: now, alive });
            if let Some(migration) = self.migrations.get(&code).cloned() {
                self.remove_session(migration.session_id);
                self.migrations.shift_remove(&code);
                let targets: Vec<(u32, SocketAddr)> =
                    self.sessions.iter().filter(|(_, s)| s.code == code).map(|(id, s)| (*id, s.guest)).collect();
                for (session_id, guest) in targets {
                    if migration.handoff {
                        self.send(RelayFrame::HOST_MIGRATING, session_id, &code, &handoff_payload(migration.epoch, "ready"), guest);
                    } else {
                        self.send(RelayFrame::HOST_MIGRATING, 0, &code, &[], guest);
                    }
                }
            }
            self.send(RelayFrame::HOST_ACK, 0, &code, &[], remote);
            return;
        }

        if frame.kind == RelayFrame::HOST_ALIVE && rooms.is_hostless(&code) {
            // only the current relay host's endpoint keeps the route fresh: the code is public, so nothing else vouches
            let now = self.now();
            if let Some(host) = self.hosts.get_mut(&code).filter(|h| h.endpoint == remote) {
                host.last_seen = now;
                host.alive = true;
                rooms.hostless_host_alive(&code);
            }
            return;
        }

        if frame.kind == RelayFrame::HOST_ALIVE {
            let Some(room) = room else { return };
            if !self.hosts.contains_key(&code) || !safe_equal(&payload_text, &room.token) {
                return;
            }
            let now = self.now();
            let host = self.hosts.get_mut(&code).unwrap();
            host.last_seen = now;
            host.alive = true;
            return;
        }

        if frame.kind == RelayFrame::GUEST_JOIN {
            let Some(room) = room.filter(|_| self.host_fresh(&code)) else {
                self.send_error(&code, remote, "ROOM HOST HAS NOT REACHED THE RELAY");
                return;
            };
            if frame.payload.len() < 8 || frame.payload.len() > 32 {
                self.send_error(&code, remote, "INVALID GUEST NONCE");
                return;
            }
            if room.gated && !rooms.admit_guest(&code, &frame.payload) {
                self.send_error(&code, remote, "PRIVATE ROOM // ACCESS REQUIRED");
                return;
            }
            let endpoint = endpoint_key(remote);
            let guest_key = format!("{code}|{endpoint}|{}", hex(&frame.payload));
            let endpoint_session_key = format!("{code}|{endpoint}");
            let mut session_id =
                self.guest_sessions.get(&guest_key).copied().or_else(|| self.guest_endpoints.get(&endpoint_session_key).copied());
            let now = self.now();
            match session_id {
                None => {
                    if !self.allow_guest_allocation(&code, &remote.ip().to_string()) {
                        self.send_error(&code, remote, "TOO MANY RELAY JOIN ATTEMPTS");
                        return;
                    }
                    let room_guests = self.sessions.values().filter(|s| s.code == code).count();
                    if room_guests >= self.opts.maximum_guests_per_room {
                        self.send_error(&code, remote, "ROOM RELAY IS FULL");
                        return;
                    }
                    let id = loop {
                        let b = random_bytes(4);
                        let id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        if id != 0 && !self.sessions.contains_key(&id) {
                            break id;
                        }
                    };
                    self.guest_sessions.insert(guest_key.clone(), id);
                    self.guest_endpoints.insert(endpoint_session_key.clone(), id);
                    self.sessions.insert(
                        id,
                        Session {
                            code: code.clone(),
                            guest: remote,
                            guest_key,
                            endpoint_key: endpoint_session_key,
                            last_seen: now,
                            joined_at: now,
                        },
                    );
                    session_id = Some(id);
                }
                Some(id) => {
                    let old_key = self.sessions.get(&id).map(|s| s.guest_key.clone()).unwrap_or_default();
                    self.guest_sessions.shift_remove(&old_key);
                    self.guest_sessions.insert(guest_key.clone(), id);
                    if let Some(session) = self.sessions.get_mut(&id) {
                        session.guest_key = guest_key;
                        session.guest = remote;
                        session.last_seen = now;
                    }
                }
            }
            self.send(RelayFrame::GUEST_ACK, session_id.unwrap(), &code, &[], remote);
            return;
        }

        let known = self.sessions.get(&frame.session_id).map(|s| s.code == code).unwrap_or(false);
        if !known || room.is_none() {
            self.counters.dropped_unknown_session += 1;
            return;
        }
        if frame.kind == RelayFrame::GUEST_DATA {
            if self.sessions[&frame.session_id].guest != remote {
                self.counters.dropped_wrong_endpoint += 1;
                return;
            }
            if !self.host_fresh(&code) {
                self.counters.dropped_no_host += 1;
                return;
            }
            let now = self.now();
            self.sessions.get_mut(&frame.session_id).unwrap().last_seen = now;
            self.counters.forwarded_guest += 1;
            self.counters.forwarded_bytes += frame.payload.len() as u64;
            let host = self.hosts[&code].endpoint;
            self.send(RelayFrame::GUEST_DATA, frame.session_id, &code, &frame.payload, host);
        } else if frame.kind == RelayFrame::HOST_DATA {
            match self.hosts.get(&code) {
                Some(host) if host.endpoint == remote => {}
                _ => {
                    self.counters.dropped_wrong_endpoint += 1;
                    return;
                }
            }
            let now = self.now();
            self.hosts.get_mut(&code).unwrap().last_seen = now;
            let session = self.sessions.get_mut(&frame.session_id).unwrap();
            session.last_seen = now;
            let guest = session.guest;
            self.counters.forwarded_host += 1;
            self.counters.forwarded_bytes += frame.payload.len() as u64;
            self.send(RelayFrame::HOST_DATA, frame.session_id, &code, &frame.payload, guest);
        }
    }

    /// `relayStatus()`.
    pub fn relay_status(&self) -> Value {
        let c = &self.counters;
        let mut status = Map::new();
        status.insert("enabled".into(), json!(true));
        status.insert("listening".into(), json!(self.listening));
        status.insert("port".into(), json!(self.relay_port));
        status.insert("hosts".into(), json!(self.hosts.len()));
        status.insert("sessions".into(), json!(self.sessions.len()));
        status.insert(
            "counters".into(),
            json!({
                "forwardedGuest": c.forwarded_guest, "forwardedHost": c.forwarded_host, "forwardedBytes": c.forwarded_bytes,
                "droppedInvalid": c.dropped_invalid, "droppedUnknownSession": c.dropped_unknown_session,
                "droppedNoHost": c.dropped_no_host, "droppedWrongEndpoint": c.dropped_wrong_endpoint,
                "sendErrors": c.send_errors, "roomsClosed": c.rooms_closed,
            }),
        );
        if self.listening {
            if let Some((receive, send)) = self.socket_buffers {
                status.insert("socketBuffers".into(), json!({ "receive": receive, "send": send }));
            }
        }
        if !self.last_error.is_empty() {
            status.insert("error".into(), json!(self.last_error));
        }
        Value::Object(status)
    }

    /// `requestHostMigration(code)` (legacy release-host without a named candidate).
    pub fn request_host_migration(&mut self, code: &str, rooms: &mut dyn RelayRooms) -> bool {
        self.begin_host_migration(&code.trim().to_uppercase(), rooms)
    }

    /// `requestHostHandoff(code, candidateSession, epoch)`: never closes or demotes a live host on refusal.
    pub fn request_host_handoff(
        &mut self,
        code: &str,
        candidate_session: Option<&Value>,
        epoch: Option<&Value>,
        rooms: &mut dyn RelayRooms,
    ) -> HandoffResult {
        let session = crate::util::js_number(candidate_session);
        let handoff_epoch = crate::util::js_number(epoch);
        let integer = |n: f64| n.is_finite() && n.fract() == 0.0;
        if !integer(session) || session <= 0.0 || session > 4_294_967_295.0 || !integer(handoff_epoch) || handoff_epoch <= 0.0
            || handoff_epoch > 2_147_483_647.0
        {
            return HandoffResult::refused("INVALID HANDOFF REQUEST");
        }
        self.begin_handoff(&code.trim().to_uppercase(), session as u32, handoff_epoch as i64, rooms)
    }

    #[cfg(test)]
    pub fn host_count(&self) -> usize {
        self.hosts.len()
    }
    #[cfg(test)]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

fn endpoint_key(remote: SocketAddr) -> String {
    format!("{}:{}", remote.ip(), remote.port())
}

/// Events the driver thread feeds into the handler.
pub enum RelayEvent<'a> {
    Datagram(SocketAddr, &'a [u8]),
    Tick,
    SendError,
}

/// Binds the relay socket (with the Node relay's 4 MB buffers where the OS allows) and returns it plus the buffer
/// sizes the OS actually granted.
pub fn bind_relay_socket(host: &str, port: u16, buffer_bytes: usize) -> std::io::Result<(UdpSocket, Option<(usize, usize)>)> {
    use socket2::{Domain, Protocol, Socket, Type};
    let address: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{e}")))?;
    let socket = Socket::new(Domain::for_address(address), Type::DGRAM, Some(Protocol::UDP))?;
    let _ = socket.set_recv_buffer_size(buffer_bytes);
    let _ = socket.set_send_buffer_size(buffer_bytes);
    socket.bind(&address.into())?;
    let buffers = match (socket.recv_buffer_size(), socket.send_buffer_size()) {
        (Ok(r), Ok(s)) => Some((r, s)),
        _ => None,
    };
    Ok((socket.into(), buffers))
}

/// Owns the relay socket thread. Dropping (or [`RelayDriver::stop`]) stops it.
pub struct RelayDriver {
    socket: UdpSocket,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RelayDriver {
    /// Runs `handler` for every datagram and every `tick` interval; it returns the datagrams to send (normally
    /// `relay.take_outbox()` after locking the shared state).
    pub fn spawn<F>(socket: UdpSocket, tick: Duration, handler: F) -> std::io::Result<RelayDriver>
    where
        F: Fn(RelayEvent) -> Vec<Outgoing> + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_socket = socket.try_clone()?;
        thread_socket.set_read_timeout(Some(tick.min(Duration::from_millis(100)).max(Duration::from_millis(10))))?;
        let flag = stop.clone();
        let thread = std::thread::Builder::new().name("lr-enet-relay".into()).spawn(move || {
            let mut buffer = vec![0u8; 65_536];
            let mut last_tick = Instant::now();
            while !flag.load(Ordering::SeqCst) {
                let outgoing = match thread_socket.recv_from(&mut buffer) {
                    Ok((len, from)) => handler(RelayEvent::Datagram(from, &buffer[..len])),
                    Err(_) => Vec::new(),
                };
                send_all(&thread_socket, outgoing, &handler);
                if last_tick.elapsed() >= tick {
                    last_tick = Instant::now();
                    let outgoing = handler(RelayEvent::Tick);
                    send_all(&thread_socket, outgoing, &handler);
                }
            }
        })?;
        Ok(RelayDriver { socket, stop, thread: Some(thread) })
    }

    /// A socket clone for datagrams produced outside the driver thread (HTTP-triggered closeRoom/migrations).
    pub fn sender(&self) -> &UdpSocket {
        &self.socket
    }

    pub fn local_port(&self) -> u16 {
        self.socket.local_addr().map(|a| a.port()).unwrap_or(0)
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RelayDriver {
    fn drop(&mut self) {
        self.stop();
    }
}

fn send_all<F: Fn(RelayEvent) -> Vec<Outgoing>>(socket: &UdpSocket, outgoing: Vec<Outgoing>, handler: &F) {
    for (to, frame) in outgoing {
        if socket.send_to(&frame, to).is_err() {
            handler(RelayEvent::SendError);
        }
    }
}

/// Sends datagrams from outside the driver thread; returns how many failed.
pub fn send_outgoing(socket: &UdpSocket, outgoing: Vec<Outgoing>) -> usize {
    outgoing.into_iter().filter(|(to, frame)| socket.send_to(frame, *to).is_err()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::manual_clock;
    use std::sync::Mutex;

    struct Rooms {
        code: String,
        token: String,
        closed: Vec<String>,
        promotions: u32,
        promote_to: Option<String>,
        ticket: Option<Vec<u8>>,
        hostless: bool,
    }

    impl Rooms {
        fn new(code: &str, token: &str) -> Rooms {
            Rooms { code: code.into(), token: token.into(), closed: Vec::new(), promotions: 0, promote_to: None, ticket: None, hostless: false }
        }
    }

    impl RelayRooms for Rooms {
        fn hostless_room(&mut self, code: &str, _host: SocketAddr) -> Option<RelayRoom> {
            (self.hostless && code == self.code).then(|| RelayRoom { token: self.token.clone(), gated: false })
        }
        fn is_hostless(&mut self, code: &str) -> bool {
            self.hostless && code == self.code
        }
        fn room_by_code(&mut self, code: &str) -> Option<RelayRoom> {
            (code == self.code && !self.closed.contains(&code.to_string()))
                .then(|| RelayRoom { token: self.token.clone(), gated: self.ticket.is_some() })
        }
        fn promote_room_host(&mut self, _code: &str) -> Option<String> {
            self.promotions += 1;
            self.token = match &self.promote_to {
                Some(fixed) => fixed.clone(),
                None => format!("successor-secret-{}", self.promotions),
            };
            Some(self.token.clone())
        }
        fn on_room_closed(&mut self, code: &str, reason: &str) {
            self.closed.push(code.into());
            self.closed.push(reason.into());
        }
        fn admit_guest(&mut self, _code: &str, nonce: &[u8]) -> bool {
            self.ticket.as_deref() == Some(nonce)
        }
    }

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    fn frame(kind: u8, session: u32, code: &str, payload: &[u8]) -> Vec<u8> {
        encode_relay_frame(kind, session, code, payload).unwrap()
    }

    /// Send one datagram and return the frames the relay sends back to each endpoint.
    fn exchange(relay: &mut EnetRelay, rooms: &mut Rooms, from: SocketAddr, data: Vec<u8>) -> Vec<(SocketAddr, DecodedFrame)> {
        relay.handle_datagram(from, &data, rooms);
        relay.take_outbox().into_iter().map(|(to, f)| (to, decode_relay_frame(&f).unwrap())).collect()
    }

    fn to(out: &[(SocketAddr, DecodedFrame)], who: SocketAddr) -> Vec<DecodedFrame> {
        out.iter().filter(|(a, _)| *a == who).map(|(_, f)| f.clone()).collect()
    }

    fn relay_with(clock: Clock, f: impl FnOnce(&mut EnetRelayOptions)) -> EnetRelay {
        let mut opts = EnetRelayOptions { now: clock, ..EnetRelayOptions::default() };
        f(&mut opts);
        let mut relay = EnetRelay::new(opts);
        relay.set_listening(1105, Some((4_194_304, 4_194_304)));
        relay
    }

    #[test]
    fn the_join_counter_table_is_bounded_and_recovers_when_its_windows_end() {
        let (clock, time) = manual_clock(0);
        let mut relay = relay_with(clock, |o| {
            o.maximum_guest_allocation_keys = 3;
            o.maximum_guests_per_room = 2;
        });
        let mut rooms = Rooms::new("ABCDE", "host-secret");
        let host = addr(4001);
        exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", b"host-secret"));
        let from = |last: u8| SocketAddr::from(([203, 0, 113, last], 5000));
        // two guests fill the room; a third address is still counted (it gets ROOM RELAY IS FULL)
        for last in 1..=3u8 {
            exchange(&mut relay, &mut rooms, from(last), frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"guest-nonce"));
        }
        assert_eq!(relay.guest_allocation_rates.len(), 3);
        // the table is full: a fourth address is refused and is not remembered
        let out = exchange(&mut relay, &mut rooms, from(4), frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"guest-nonce"));
        let refused = &to(&out, from(4))[0];
        assert_eq!((refused.kind, refused.payload.as_slice()), (RelayFrame::ERROR, &b"TOO MANY RELAY JOIN ATTEMPTS"[..]));
        assert_eq!(relay.guest_allocation_rates.len(), 3);
        // a guest that is already in keeps working
        let out = exchange(&mut relay, &mut rooms, from(1), frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"guest-nonce"));
        assert_eq!(to(&out, from(1))[0].kind, RelayFrame::GUEST_ACK);
        // the windows end (the host stays alive): the table empties and a new address is counted again
        time.advance(10_001);
        exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", b"host-secret"));
        relay.prune(&mut rooms);
        assert_eq!(relay.guest_allocation_rates.len(), 0);
        let out = exchange(&mut relay, &mut rooms, from(4), frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"guest-nonce"));
        assert_eq!(to(&out, from(4))[0].kind, RelayFrame::ERROR, "the room is still full, but the address was counted");
        assert_eq!(relay.guest_allocation_rates.len(), 1);
    }

    #[test]
    fn authenticated_relay_tunnels_enet_datagrams_through_one_public_udp_port() {
        let (clock, _) = manual_clock(0);
        let mut relay = relay_with(clock, |_| {});
        let mut rooms = Rooms::new("ABCDE", "host-secret");
        let (host, guest, attacker) = (addr(4001), addr(4002), addr(4003));
        let out = exchange(&mut relay, &mut rooms, attacker, frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", b"wrong-secret"));
        assert_eq!(to(&out, attacker)[0].kind, RelayFrame::ERROR);
        let out = exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", b"host-secret"));
        assert_eq!(to(&out, host)[0].kind, RelayFrame::HOST_ACK);
        let out = exchange(&mut relay, &mut rooms, guest, frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"guest-01"));
        let joined = to(&out, guest)[0].clone();
        assert_eq!(joined.kind, RelayFrame::GUEST_ACK);
        assert_ne!(joined.session_id, 0);
        for index in 0..20 {
            let nonce = format!("retry-{index:03}");
            let out = exchange(&mut relay, &mut rooms, guest, frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", nonce.as_bytes()));
            let retried = &to(&out, guest)[0];
            assert_eq!(retried.kind, RelayFrame::GUEST_ACK);
            assert_eq!(retried.session_id, joined.session_id);
        }
        let request = b"raw-enet-connect-packet";
        let out = exchange(&mut relay, &mut rooms, guest, frame(RelayFrame::GUEST_DATA, joined.session_id, "ABCDE", request));
        let at_host = &to(&out, host)[0];
        assert_eq!((at_host.kind, at_host.session_id, at_host.payload.as_slice()), (RelayFrame::GUEST_DATA, joined.session_id, &request[..]));
        let response = b"raw-enet-ack-packet";
        let out = exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_DATA, joined.session_id, "ABCDE", response));
        let at_guest = &to(&out, guest)[0];
        assert_eq!((at_guest.kind, at_guest.payload.as_slice()), (RelayFrame::HOST_DATA, &response[..]));
        let status = relay.relay_status();
        assert_eq!(status["enabled"], json!(true));
        assert_eq!(status["listening"], json!(true));
        assert_eq!(status["hosts"], json!(1));
        assert_eq!(status["sessions"], json!(1));
        assert!(status.get("error").is_none());
        assert_eq!(status["counters"]["forwardedGuest"], json!(1));
        assert_eq!(status["counters"]["forwardedHost"], json!(1));
        assert_eq!(status["counters"]["forwardedBytes"], json!(request.len() + response.len()));
        assert!(status["socketBuffers"]["receive"].as_u64().unwrap() > 0);
        exchange(&mut relay, &mut rooms, attacker, frame(RelayFrame::GUEST_DATA, joined.session_id, "ABCDE", request));
        exchange(&mut relay, &mut rooms, attacker, frame(RelayFrame::GUEST_DATA, joined.session_id ^ 1, "ABCDE", request));
        exchange_raw(&mut relay, &mut rooms, attacker, b"not a relay packet");
        let c = &relay.counters;
        assert_eq!((c.dropped_wrong_endpoint, c.dropped_unknown_session, c.dropped_invalid, c.forwarded_guest), (1, 1, 1, 1));
    }

    #[test]
    fn a_hostless_code_registers_without_a_token_and_a_second_registrant_is_told_to_join() {
        let (clock, time) = manual_clock(0);
        let mut relay = relay_with(clock, |_| {});
        let mut rooms = Rooms::new("PUBLC", "server-made");
        rooms.hostless = true;
        let (first, second) = (addr(4101), SocketAddr::from(([127, 0, 0, 2], 4102)));
        let out = exchange(&mut relay, &mut rooms, first, frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", b"stale-token"));
        assert_eq!(to(&out, first)[0].kind, RelayFrame::HOST_ACK);
        let out = exchange(&mut relay, &mut rooms, first, frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", b""));
        assert_eq!(to(&out, first)[0].kind, RelayFrame::HOST_ACK, "the holder may re-register");
        let out = exchange(&mut relay, &mut rooms, second, frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", b"x"));
        assert_eq!(to(&out, second)[0].kind, RelayFrame::HOST_JOIN_AS_GUEST);
        // the keep-alive carries no token either
        exchange_raw(&mut relay, &mut rooms, first, &frame(RelayFrame::HOST_ALIVE, 0, "PUBLC", b"whatever"));
        time.advance(900);
        let out = exchange(&mut relay, &mut rooms, second, frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", b"x"));
        assert_eq!(to(&out, second)[0].kind, RelayFrame::HOST_JOIN_AS_GUEST);
        // the holder went quiet: the next registrant becomes the host
        time.advance(2_000);
        let out = exchange(&mut relay, &mut rooms, second, frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", b"x"));
        assert_eq!(to(&out, second)[0].kind, RelayFrame::HOST_ACK);
    }

    #[test]
    fn a_hostless_keep_alive_counts_only_from_the_current_hosts_endpoint() {
        let (clock, time) = manual_clock(0);
        let mut relay = relay_with(clock, |_| {});
        let mut rooms = Rooms::new("PUBLC", "server-made");
        rooms.hostless = true;
        let (host, spoofer) = (addr(4101), SocketAddr::from(([127, 0, 0, 2], 4102)));
        exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", b""));
        exchange_raw(&mut relay, &mut rooms, host, &frame(RelayFrame::HOST_ALIVE, 0, "PUBLC", b""));
        time.advance(900);
        exchange_raw(&mut relay, &mut rooms, spoofer, &frame(RelayFrame::HOST_ALIVE, 0, "PUBLC", b""));
        time.advance(200);
        // the host's last beat is 1.1 s old: the spoofed one did not refresh it
        relay.prune(&mut rooms);
        assert_eq!(relay.host_count(), 0);
        assert_eq!(rooms.closed.first().map(String::as_str), Some("PUBLC"));
    }

    #[test]
    fn the_same_address_on_a_new_port_replaces_a_fresh_hostless_host() {
        let (clock, _time) = manual_clock(0);
        let mut relay = relay_with(clock, |_| {});
        let mut rooms = Rooms::new("PUBLC", "server-made");
        rooms.hostless = true;
        let (old, new) = (addr(4101), addr(4102));
        exchange(&mut relay, &mut rooms, old, frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", b""));
        exchange_raw(&mut relay, &mut rooms, old, &frame(RelayFrame::HOST_ALIVE, 0, "PUBLC", b""));
        let out = exchange(&mut relay, &mut rooms, new, frame(RelayFrame::HOST_REGISTER, 0, "PUBLC", b""));
        assert_eq!(to(&out, new)[0].kind, RelayFrame::HOST_ACK);
        // the old endpoint no longer keeps the route fresh; the new one does
        assert_eq!(relay.hosts.get("PUBLC").map(|h| h.endpoint), Some(new));
    }

    fn exchange_raw(relay: &mut EnetRelay, rooms: &mut Rooms, from: SocketAddr, data: &[u8]) {
        relay.handle_datagram(from, data, rooms);
        relay.take_outbox();
    }

    #[test]
    fn relay_protocol_rejects_malformed_and_oversized_frames() {
        assert_eq!(decode_relay_frame(b"not a relay packet"), None);
        assert_eq!(encode_relay_frame(RelayFrame::GUEST_JOIN, 0, "ab!12", &[]), Err("INVALID RELAY ROOM CODE"));
        let mut oversized = vec![0u8; 1473];
        oversized[..4].copy_from_slice(b"LRR1");
        oversized[9..14].copy_from_slice(b"ABCDE");
        assert_eq!(decode_relay_frame(&oversized), None);
        // header layout: magic, type, LE session id, code
        assert_eq!(frame(5, 0x01020304, "abcde", b"x"), b"LRR1\x05\x04\x03\x02\x01ABCDEx".to_vec());
    }

    #[test]
    fn a_host_handing_over_on_purpose_promotes_the_oldest_guest_the_other_sessions_survive() {
        let (clock, time) = manual_clock(0);
        let mut relay = relay_with(clock, |o| {
            o.host_ttl_ms = 100;
            o.session_ttl_ms = 1000;
        });
        let mut rooms = Rooms::new("ABCDE", "host-secret");
        rooms.promote_to = Some("migrated-host-secret".into());
        let (host, first, second) = (addr(5001), addr(5002), addr(5003));
        exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", b"host-secret"));
        let first_session = to(&exchange(&mut relay, &mut rooms, first, frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"first-guest")), first)[0].session_id;
        time.advance(1);
        let second_session = to(&exchange(&mut relay, &mut rooms, second, frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"second-guest")), second)[0].session_id;
        time.set(50);
        assert!(relay.request_host_migration("ABCDE", &mut rooms));
        let out: Vec<_> = relay.take_outbox().into_iter().map(|(a, f)| (a, decode_relay_frame(&f).unwrap())).collect();
        let promotion = to(&out, first)[0].clone();
        assert_eq!((promotion.kind, promotion.session_id), (RelayFrame::HOST_PROMOTE, first_session));
        assert_eq!(promotion.payload, b"migrated-host-secret");
        assert_eq!(to(&out, second)[0].kind, RelayFrame::HOST_MIGRATING);
        let out = exchange(&mut relay, &mut rooms, first, frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", &promotion.payload));
        assert_eq!(to(&out, first)[0].kind, RelayFrame::HOST_ACK);
        assert_eq!(to(&out, second)[0].kind, RelayFrame::HOST_MIGRATING);
        let out = exchange(&mut relay, &mut rooms, second, frame(RelayFrame::GUEST_DATA, second_session, "ABCDE", b"enet-reconnect-packet"));
        let at = &to(&out, first)[0];
        assert_eq!((at.kind, at.session_id, at.payload.as_slice()), (RelayFrame::GUEST_DATA, second_session, &b"enet-reconnect-packet"[..]));
    }

    struct Handoff {
        relay: EnetRelay,
        rooms: Rooms,
        time: crate::util::ManualClock,
        host: SocketAddr,
        successor: SocketAddr,
        other: SocketAddr,
        successor_session: u32,
        other_session: u32,
    }

    fn handoff_fixture() -> Handoff {
        let (clock, time) = manual_clock(0);
        let mut relay = relay_with(clock, |o| o.handoff_ttl_ms = 5_000);
        let mut rooms = Rooms::new("HNDOF", "host-secret");
        let (host, successor, other) = (addr(6001), addr(6002), addr(6003));
        exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_REGISTER, 0, "HNDOF", b"host-secret"));
        let successor_session = to(&exchange(&mut relay, &mut rooms, successor, frame(RelayFrame::GUEST_JOIN, 0, "HNDOF", b"successor-nonce")), successor)[0].session_id;
        time.advance(1);
        let other_session = to(&exchange(&mut relay, &mut rooms, other, frame(RelayFrame::GUEST_JOIN, 0, "HNDOF", b"other-guest-nonce")), other)[0].session_id;
        Handoff { relay, rooms, time, host, successor, other, successor_session, other_session }
    }

    fn outbox(relay: &mut EnetRelay) -> Vec<(SocketAddr, DecodedFrame)> {
        relay.take_outbox().into_iter().map(|(a, f)| (a, decode_relay_frame(&f).unwrap())).collect()
    }

    #[test]
    fn a_prepared_handoff_promotes_the_named_session_with_an_epoch_the_others_get_a_tagged_ready_notice() {
        let mut f = handoff_fixture();
        let r = f.relay.request_host_handoff("HNDOF", Some(&json!(f.other_session)), Some(&json!(7)), &mut f.rooms);
        assert_eq!(r, HandoffResult::ok());
        let out = outbox(&mut f.relay);
        let promotion = to(&out, f.other)[0].clone();
        assert_eq!((promotion.kind, promotion.session_id), (RelayFrame::HOST_PROMOTE, f.other_session));
        assert_eq!(String::from_utf8(promotion.payload).unwrap(), format!("LRH1|7|{}", f.rooms.token));
        let waiting = to(&out, f.successor)[0].clone();
        assert_eq!((waiting.kind, waiting.session_id), (RelayFrame::HOST_MIGRATING, f.successor_session));
        assert_eq!(waiting.payload, b"LRH1|7");
        assert_eq!(f.relay.request_host_handoff("HNDOF", Some(&json!(f.other_session)), Some(&json!(7)), &mut f.rooms), HandoffResult::ok());
        assert_eq!(to(&outbox(&mut f.relay), f.other)[0].kind, RelayFrame::HOST_PROMOTE);
        assert_eq!(f.rooms.promotions, 1);
        assert!(!f.relay.request_host_handoff("HNDOF", Some(&json!(f.successor_session)), Some(&json!(8)), &mut f.rooms).migrating);
        let out = exchange(&mut f.relay, &mut f.rooms, f.host, frame(RelayFrame::HOST_REGISTER, 0, "HNDOF", b"host-secret"));
        assert_eq!(to(&out, f.host)[0].kind, RelayFrame::ERROR);
        let token = f.rooms.token.clone();
        let out = exchange(&mut f.relay, &mut f.rooms, f.other, frame(RelayFrame::HOST_REGISTER, 0, "HNDOF", token.as_bytes()));
        assert_eq!(to(&out, f.other)[0].kind, RelayFrame::HOST_ACK);
        let ready = to(&out, f.successor)[0].clone();
        assert_eq!((ready.kind, ready.session_id, ready.payload.as_slice()), (RelayFrame::HOST_MIGRATING, f.successor_session, &b"LRH1|7|ready"[..]));
        let out = exchange(&mut f.relay, &mut f.rooms, f.successor, frame(RelayFrame::GUEST_DATA, f.successor_session, "HNDOF", b"enet-connect"));
        let at = &to(&out, f.other)[0];
        assert_eq!((at.kind, at.session_id), (RelayFrame::GUEST_DATA, f.successor_session));
        assert!(f.rooms.closed.is_empty());
    }

    #[test]
    fn a_handoff_to_an_unknown_or_stale_session_is_refused_and_the_live_host_keeps_its_room() {
        let mut f = handoff_fixture();
        assert!(!f.relay.request_host_handoff("HNDOF", Some(&json!(12345)), Some(&json!(1)), &mut f.rooms).migrating);
        assert!(!f.relay.request_host_handoff("HNDOF", Some(&json!(f.other_session)), Some(&json!(0)), &mut f.rooms).migrating);
        assert!(!f.relay.request_host_handoff("HNDOF", Some(&json!("x")), Some(&json!(1)), &mut f.rooms).migrating);
        assert_eq!(f.rooms.promotions, 0);
        assert_eq!(f.rooms.token, "host-secret");
        let out = exchange(&mut f.relay, &mut f.rooms, f.other, frame(RelayFrame::GUEST_DATA, f.other_session, "HNDOF", b"still-live"));
        assert_eq!(to(&out, f.host)[0].kind, RelayFrame::GUEST_DATA);
        f.time.advance(60_000);
        exchange(&mut f.relay, &mut f.rooms, f.host, frame(RelayFrame::HOST_REGISTER, 0, "HNDOF", b"host-secret"));
        assert!(!f.relay.request_host_handoff("HNDOF", Some(&json!(f.other_session)), Some(&json!(2)), &mut f.rooms).migrating);
        assert!(f.rooms.closed.is_empty());
    }

    #[test]
    fn a_successor_that_never_registers_closes_the_room_after_the_handoff_deadline_no_guess_election() {
        let mut f = handoff_fixture();
        assert!(f.relay.request_host_handoff("HNDOF", Some(&json!(f.other_session)), Some(&json!(3)), &mut f.rooms).migrating);
        outbox(&mut f.relay);
        f.time.advance(4_000);
        f.relay.prune(&mut f.rooms);
        assert!(f.rooms.closed.is_empty());
        outbox(&mut f.relay);
        f.time.advance(2_000);
        f.relay.prune(&mut f.rooms);
        assert_eq!(f.rooms.closed, vec!["HNDOF".to_string(), HANDOFF_FAILED_MESSAGE.to_string()]);
        let out = outbox(&mut f.relay);
        let left = to(&out, f.successor).into_iter().find(|fr| fr.kind == RelayFrame::HOST_LEFT).unwrap();
        assert_eq!(left.payload, HANDOFF_FAILED_MESSAGE.as_bytes());
        assert_eq!(f.rooms.promotions, 1);
    }

    #[test]
    fn a_host_that_quits_or_dies_closes_the_room_guests_get_host_left_the_directory_forgets_the_code() {
        let (clock, time) = manual_clock(0);
        let mut relay = relay_with(clock, |_| {});
        let mut rooms = Rooms::new("ABCDE", "host-secret");
        rooms.promote_to = Some("never".into());
        let (host, keep_alive, guest) = (addr(7001), addr(7002), addr(7003));
        exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", b"host-secret"));
        assert_eq!(to(&exchange(&mut relay, &mut rooms, guest, frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"guest-nonce")), guest)[0].kind, RelayFrame::GUEST_ACK);
        let mut ms = 250;
        while ms <= 5000 {
            time.set(ms);
            exchange(&mut relay, &mut rooms, keep_alive, frame(RelayFrame::HOST_ALIVE, 0, "ABCDE", b"host-secret"));
            relay.prune(&mut rooms);
            ms += 250;
        }
        assert!(rooms.closed.is_empty());
        assert_eq!(relay.host_count(), 1);
        exchange(&mut relay, &mut rooms, keep_alive, frame(RelayFrame::HOST_ALIVE, 0, "ABCDE", b"guess"));
        time.set(5000 + 1001);
        relay.prune(&mut rooms);
        let out = outbox(&mut relay);
        let left = &to(&out, guest)[0];
        assert_eq!(left.kind, RelayFrame::HOST_LEFT);
        assert_eq!(left.payload, b"HOST LEFT // ROOM CLOSED");
        assert_eq!(rooms.closed[0], "ABCDE");
        assert_eq!(relay.session_count(), 0);
        let after = &to(&exchange(&mut relay, &mut rooms, guest, frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"guest-nonce-2")), guest)[0];
        assert!(after.kind == RelayFrame::ERROR || after.kind == RelayFrame::HOST_LEFT);
    }

    #[test]
    fn an_older_host_no_keep_alive_keeps_the_long_timeout_and_close_room_ends_a_room_at_once() {
        let (clock, time) = manual_clock(0);
        let mut relay = relay_with(clock, |_| {});
        let mut rooms = Rooms::new("ABCDE", "host-secret");
        exchange(&mut relay, &mut rooms, addr(8001), frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", b"host-secret"));
        time.set(5_000);
        relay.prune(&mut rooms);
        assert!(rooms.closed.is_empty());
        assert!(relay.close_room("ABCDE", None, &mut rooms));
        assert_eq!(rooms.closed[0], "ABCDE");
    }

    #[test]
    fn the_relay_admits_a_reserved_rooms_guest_only_with_a_join_ticket_as_its_nonce() {
        let (clock, _) = manual_clock(0);
        let mut relay = relay_with(clock, |_| {});
        let mut rooms = Rooms::new("KTPRV", "host-secret");
        rooms.ticket = Some(b"0123456789abcdef01".to_vec());
        let (host, stranger, invited) = (addr(9001), addr(9002), addr(9003));
        assert_eq!(to(&exchange(&mut relay, &mut rooms, host, frame(RelayFrame::HOST_REGISTER, 0, "KTPRV", b"host-secret")), host)[0].kind, RelayFrame::HOST_ACK);
        let refused = to(&exchange(&mut relay, &mut rooms, stranger, frame(RelayFrame::GUEST_JOIN, 0, "KTPRV", b"random-nonce-12")), stranger)[0].clone();
        assert_eq!(refused.kind, RelayFrame::ERROR);
        assert!(String::from_utf8(refused.payload).unwrap().contains("PRIVATE ROOM"));
        assert_eq!(to(&exchange(&mut relay, &mut rooms, invited, frame(RelayFrame::GUEST_JOIN, 0, "KTPRV", b"0123456789abcdef01")), invited)[0].kind, RelayFrame::GUEST_ACK);
    }

    #[test]
    fn a_real_udp_round_trip_through_the_driver() {
        let (socket, buffers) = bind_relay_socket("127.0.0.1", 0, 4 * 1024 * 1024).unwrap();
        let port = socket.local_addr().unwrap().port();
        let mut relay = EnetRelay::new(EnetRelayOptions::default());
        relay.set_listening(port, buffers);
        let shared = Arc::new(Mutex::new((relay, Rooms::new("ABCDE", "host-secret"))));
        let state = shared.clone();
        let mut driver = RelayDriver::spawn(socket, Duration::from_millis(250), move |event| {
            let mut guard = state.lock().unwrap();
            let (relay, rooms) = &mut *guard;
            match event {
                RelayEvent::Datagram(from, data) => relay.handle_datagram(from, data, rooms),
                RelayEvent::Tick => relay.prune(rooms),
                RelayEvent::SendError => relay.note_send_error(),
            }
            relay.take_outbox()
        })
        .unwrap();
        let host = UdpSocket::bind("127.0.0.1:0").unwrap();
        let guest = UdpSocket::bind("127.0.0.1:0").unwrap();
        for s in [&host, &guest] {
            s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        }
        let target = ("127.0.0.1", port);
        let mut buf = [0u8; 2048];
        host.send_to(&frame(RelayFrame::HOST_REGISTER, 0, "ABCDE", b"host-secret"), target).unwrap();
        let n = host.recv(&mut buf).unwrap();
        assert_eq!(decode_relay_frame(&buf[..n]).unwrap().kind, RelayFrame::HOST_ACK);
        guest.send_to(&frame(RelayFrame::GUEST_JOIN, 0, "ABCDE", b"guest-nonce"), target).unwrap();
        let n = guest.recv(&mut buf).unwrap();
        let ack = decode_relay_frame(&buf[..n]).unwrap();
        assert_eq!(ack.kind, RelayFrame::GUEST_ACK);
        guest.send_to(&frame(RelayFrame::GUEST_DATA, ack.session_id, "ABCDE", b"hello host"), target).unwrap();
        let n = host.recv(&mut buf).unwrap();
        assert_eq!(decode_relay_frame(&buf[..n]).unwrap().payload, b"hello host");
        driver.stop();
        assert!(shared.lock().unwrap().0.relay_status()["socketBuffers"]["receive"].as_u64().unwrap() > 0);
    }
}
