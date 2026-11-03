//! Shared rooms for the Rust client: hosting and joining by code over the directory and the LRR1 relay, the route seed
//! handed to the guests, and the position exchange that puts each runner into the others' world.
//!
//! Flow (the Godot room flow, `room_state.gd` / `backend_client.gd`, with the Rust transport of `net::session`):
//! * Host: [`host_room`] registers a code (`api::register_room`, retrying a reserved one), opens the relay
//!   (`RelayLink::host`) and heartbeats the directory every 20 s. The route seed is the room text
//!   (`ROOM-<code>-<hex>`), so the lobby shows the code the guests type.
//! * Guest: [`join_room`] looks the code up (`api::lookup_room`), refuses a room that is not on the relay or that runs
//!   another build's protocol, then joins the relay (`RelayLink::guest`).
//! * Handshake: a guest's first reliable packet is `RegisterPeer`; the host answers `Room` (the seed) and, while a
//!   route runs, `Start`. A guest enters the same seeded world with `Start`.
//! * Positions: each runner sends a [`Sample`] at 20 Hz on the STATE channel. A guest sends `Place` to the host; the
//!   host sends its own sample and each accepted guest sample to every guest as `PeerPlace` (the sender's peer id).
//!   A sample is `null` while nothing is playing: that keeps the relay session alive, nothing else.

use super::api::{self, Heartbeat, RegisterRoom, Registered};
use super::config::{self, BackendConfig};
use super::protocol::{self, Message};
use super::relay::{RelayEvent, RelayLink, Role};
use super::session::{CoopSession, SessionEvent, HOST_PEER_ID};
use super::{store, NetError, Pending};
use crate::config::difficulty::ALL;
use crate::config::Difficulty;
use crate::game::player::Player;
use crate::game::Game;
use crate::render::builder::Builder;
use crate::render::{scene, shader};
use crate::shared::assets::Assets;
use macroquad::prelude::*;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A runner's sample goes out this often (the original's 20 Hz `_place`).
pub const SEND_INTERVAL: Duration = Duration::from_millis(50);
/// A remote runner with no sample for this long is neither drawn nor kept.
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(2);
/// The relay drops a guest session idle this long; the keep-alive sample is far more frequent than that.
const GUEST_IDLE: Duration = Duration::from_secs(45);
/// A guest whose relay join is not answered in this long is told so.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(10);
/// Registrations tried while the drawn code is a reserved one (ROOM_RESERVED).
const REGISTER_TRIES: u32 = 6;
/// Longest name a runner shows in a room.
const MAX_NAME: usize = 24;
/// Colours of the remote figures, by peer id.
const PEER_COLOURS: [Color; 4] = [
    Color::new(0.39, 0.85, 0.95, 1.0),
    Color::new(0.98, 0.72, 0.30, 1.0),
    Color::new(0.55, 0.95, 0.55, 1.0),
    Color::new(0.95, 0.55, 0.85, 1.0),
];

/// One runner's sample: the `_place` packet `{seq, depth, x, y, yaw, moving, sprinting, crouching}`.
/// `x` and `y` are the runner's ground position (`Player.pos`), `depth` its floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub seq: u32,
    pub depth: i32,
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
    pub moving: bool,
    pub sprinting: bool,
    pub crouching: bool,
}

impl Sample {
    /// The sample of a runner's body; `seq` is filled in when it is sent.
    pub fn of(p: &Player, depth: i32) -> Sample {
        Sample {
            seq: 0,
            depth,
            x: p.pos.0,
            y: p.pos.1,
            yaw: p.yaw,
            moving: p.moving,
            sprinting: p.sprinting,
            crouching: p.crouching,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "seq": self.seq, "depth": self.depth, "x": self.x, "y": self.y, "yaw": self.yaw,
            "moving": self.moving, "sprinting": self.sprinting, "crouching": self.crouching,
        })
    }

    /// Parses a received sample. `None` for anything else (a keep-alive is `null`) and for a
    /// non-finite position, so a bad packet can never place a figure at NaN.
    pub fn from_json(v: &Value) -> Option<Sample> {
        let o = v.as_object()?;
        let num = |k: &str| o.get(k).and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite());
        let flag = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(Sample {
            seq: o.get("seq")?.as_u64()? as u32,
            depth: o.get("depth")?.as_i64()? as i32,
            x: num("x")?,
            y: num("y")?,
            yaw: num("yaw").unwrap_or(0.0),
            moving: flag("moving"),
            sprinting: flag("sprinting"),
            crouching: flag("crouching"),
        })
    }
}

/// `a` is a newer sample than `last`, with wrap-around (the unreliable-ordered rule of `net::session`).
pub fn newer(seq: u32, last: u32) -> bool {
    seq != last && seq.wrapping_sub(last) < u32::MAX / 2
}

/// A remote runner that is on this floor and fresh enough to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemoteView {
    pub peer: u32,
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
}

/// What the room tells the game. The app acts on these between frames.
#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    /// A guest registered (host side).
    GuestJoined(String),
    /// A guest left or went silent (host side).
    GuestLeft(String),
    /// The host started the route: enter the same seeded world (guest side).
    Start { seed: String, difficulty: String },
    /// The room's route seed (guest side).
    Seed(String),
    /// The room is gone (the host quit, or the directory forgot it).
    HostLeft(String),
    /// Something to show the player.
    Problem(String),
}

struct Remote {
    sample: Sample,
    last_seen: Instant,
}

struct Guest {
    name: String,
    last_seen: Instant,
}

/// An open room, host or guest. Call [`NetSession::tick`] once per frame.
pub struct NetSession {
    pub cfg: BackendConfig,
    pub code: String,
    role: Role,
    coop: CoopSession,
    /// The host's room token (heartbeat and unregister authority).
    token: String,
    /// The route seed: the host's room text (a guest keeps the last one it was sent).
    room_seed: String,
    /// The route the host started, sent to guests who join later: (seed, difficulty name).
    run: Option<(String, String)>,
    name: String,
    guests: HashMap<u32, Guest>,
    remotes: HashMap<u32, Remote>,
    seq: u32,
    next_send: Instant,
    next_heartbeat: Instant,
    heartbeat: Option<Pending<api::HeartbeatReply>>,
    relist: Option<Pending<Registered>>,
    opened: Instant,
    timed_out: bool,
}

/// CREATE a room on the directory and open its relay. The request blocks, so the frame loop polls the result.
pub fn host_room(cfg: BackendConfig, route_seed: String, name: String) -> Pending<NetSession> {
    Pending::spawn(move || {
        if cfg.offline {
            return Err(NetError::Offline);
        }
        let device_id = store::device_id(&store::default_dir());
        for _ in 0..REGISTER_TRIES {
            let code = api::make_room_code();
            let request = RegisterRoom { code: code.clone(), device_id: device_id.clone(), ..RegisterRoom::default() };
            match api::register_room(&cfg, &request) {
                Ok(reg) => return open_host(cfg, code, route_seed, name, reg),
                Err(NetError::Http { status: 403, code: reserved, .. }) if reserved == "ROOM_RESERVED" => continue,
                Err(e) => return Err(e),
            }
        }
        Err(NetError::Refused("NO FREE ROOM CODE // TRY AGAIN".into()))
    })
}

fn open_host(cfg: BackendConfig, code: String, route_seed: String, name: String, reg: Registered) -> Result<NetSession, NetError> {
    if reg.room.transport != "relay" || reg.room.address.is_empty() {
        // this client has no direct path: give the code back rather than leave a room nobody can join
        let _ = api::unregister_room(&cfg, &code, &reg.token);
        return Err(NetError::Unsupported("THE SERVER DOES NOT RELAY ROOMS".into()));
    }
    let link = RelayLink::host(&code, &reg.token, &reg.room.address, reg.room.port)
        .map_err(|e| NetError::Transport(e.to_string()))?;
    Ok(NetSession::new(cfg, code, Role::Host, CoopSession::new(link), reg.token, route_seed, name))
}

/// JOIN by code: the directory lookup, the build check, then the relay. Blocking, so polled like [`host_room`].
pub fn join_room(cfg: BackendConfig, code: String, name: String) -> Pending<NetSession> {
    Pending::spawn(move || {
        if cfg.offline {
            return Err(NetError::Offline);
        }
        let room = api::lookup_room(&cfg, &code)?;
        if room.transport != "relay" || room.address.is_empty() {
            return Err(NetError::Refused("THAT ROOM CANNOT BE JOINED FROM THIS BUILD".into()));
        }
        let refused = api::protocol_mismatch(&room.protocol, &protocol::fingerprint());
        if !refused.is_empty() {
            return Err(NetError::Refused(refused));
        }
        let link = RelayLink::guest(&code, &room.address, room.port, "").map_err(|e| NetError::Transport(e.to_string()))?;
        Ok(NetSession::new(cfg, code, Role::Guest, CoopSession::new(link), String::new(), String::new(), name))
    })
}

impl NetSession {
    fn new(
        cfg: BackendConfig,
        code: String,
        role: Role,
        coop: CoopSession,
        token: String,
        room_seed: String,
        name: String,
    ) -> NetSession {
        let now = Instant::now();
        NetSession {
            cfg,
            code,
            role,
            coop,
            token,
            room_seed,
            run: None,
            name,
            guests: HashMap::new(),
            remotes: HashMap::new(),
            seq: 0,
            next_send: now,
            next_heartbeat: now + Duration::from_secs_f64(config::HEARTBEAT_SECONDS),
            heartbeat: None,
            relist: None,
            opened: now,
            timed_out: false,
        }
    }

    pub fn is_host(&self) -> bool {
        self.role == Role::Host
    }

    /// Host only: how many runners the room holds now, the host included (the same count the heartbeat
    /// sends). None on a guest, which does not see the other runners.
    pub fn player_count(&self) -> Option<u32> {
        self.is_host().then(|| 1 + self.guests.len() as u32)
    }

    /// Host only: the names of the guests that have registered, in the order they joined (peer id).
    pub fn guest_names(&self) -> Vec<String> {
        if !self.is_host() {
            return Vec::new();
        }
        let mut peers: Vec<(&u32, &Guest)> = self.guests.iter().collect();
        peers.sort_by_key(|(peer, _)| **peer);
        peers
            .into_iter()
            .filter(|(_, g)| !g.name.is_empty())
            .map(|(_, g)| g.name.clone())
            .collect()
    }

    /// True once the relay accepted us (HOST_ACK / GUEST_ACK).
    pub fn linked(&self) -> bool {
        self.coop.link.ready()
    }

    /// The route seed the room runs on (host: its own; guest: the last one received).
    pub fn room_seed(&self) -> &str {
        &self.room_seed
    }

    /// Host: the route starts. Guests in the room enter it now; guests who join later get it on their join.
    pub fn start_run(&mut self, seed: &str, difficulty: &str) {
        self.run = Some((seed.to_string(), difficulty.to_string()));
        if self.is_host() {
            for peer in self.coop.peers() {
                self.coop.send(peer, &start_message(&self.code, seed, difficulty));
            }
        }
    }

    /// Once per frame. Pumps the relay and the directory, applies what the others sent and returns the events for the
    /// app. `sample` is this runner's position while a route is running; `None` sends a keep-alive instead.
    pub fn tick(&mut self, sample: Option<Sample>) -> Vec<NetEvent> {
        let now = Instant::now();
        let mut out = Vec::new();
        self.service_directory(now, &mut out);
        for event in self.coop.poll() {
            self.on_session_event(event, now, &mut out);
        }
        self.prune(now, &mut out);
        self.send_due(sample, now);
        out
    }

    /// The other runners on `depth` that are fresh enough to draw, in peer order.
    pub fn remotes_at(&self, depth: i32) -> Vec<RemoteView> {
        let now = Instant::now();
        let mut out: Vec<RemoteView> = self
            .remotes
            .iter()
            .filter(|(_, r)| r.sample.depth == depth && now.duration_since(r.last_seen) < REMOTE_TIMEOUT)
            .map(|(peer, r)| RemoteView { peer: *peer, x: r.sample.x, y: r.sample.y, yaw: r.sample.yaw })
            .collect();
        out.sort_by_key(|r| r.peer);
        out
    }

    /// Leaves the room. The host unregisters, so every guest gets HOST_LEFT at once; a guest says goodbye and drops its
    /// link (the host also drops it after the sample timeout).
    pub fn leave(mut self) {
        if self.is_host() {
            let _ = api::unregister_room(&self.cfg, &self.code, &self.token);
        } else {
            self.coop.send(HOST_PEER_ID, &Message::PeerLeave);
        }
        self.coop.close();
    }

    fn service_directory(&mut self, now: Instant, out: &mut Vec<NetEvent>) {
        if !self.is_host() {
            return;
        }
        if let Some(p) = self.heartbeat.as_mut() {
            if let Some(result) = p.poll() {
                self.heartbeat = None;
                if let Err(e) = result {
                    // the directory forgot the room (restart or expiry): list it again under the same code
                    if matches!(e.status(), Some(403) | Some(404)) {
                        self.start_relist();
                    }
                }
            }
        }
        if let Some(p) = self.relist.as_mut() {
            if let Some(result) = p.poll() {
                self.relist = None;
                match result {
                    Ok(reg) => {
                        self.token = reg.token;
                        self.coop.link.update_host_token(&self.token);
                    }
                    Err(_) => out.push(NetEvent::Problem("ROOM LOST // THE DIRECTORY FORGOT IT".into())),
                }
            }
        }
        if self.heartbeat.is_none() && self.relist.is_none() && now >= self.next_heartbeat {
            self.next_heartbeat = now + Duration::from_secs_f64(config::HEARTBEAT_SECONDS);
            let beat = Heartbeat {
                code: self.code.clone(),
                token: self.token.clone(),
                players: 1 + self.guests.len() as u32,
                session_active: self.run.is_some(),
                daytime: false,
                command_since: 0,
                command_ack: 0,
            };
            let cfg = self.cfg.clone();
            self.heartbeat = Some(Pending::spawn(move || api::heartbeat(&cfg, &beat)));
        }
    }

    fn start_relist(&mut self) {
        let cfg = self.cfg.clone();
        let request = RegisterRoom {
            code: self.code.clone(),
            device_id: store::device_id(&store::default_dir()),
            ..RegisterRoom::default()
        };
        self.relist = Some(Pending::spawn(move || api::register_room(&cfg, &request)));
    }

    fn on_session_event(&mut self, event: SessionEvent, now: Instant, out: &mut Vec<NetEvent>) {
        match event {
            SessionEvent::Ready => {
                if !self.is_host() {
                    let hello = Message::RegisterPeer { requested_name: self.name.clone() };
                    self.coop.send(HOST_PEER_ID, &hello);
                }
            }
            SessionEvent::PeerSeen(peer) => {
                self.guests.entry(peer).or_insert_with(|| Guest { name: String::new(), last_seen: now });
            }
            SessionEvent::Message { from, message } => self.on_message(from, message, now, out),
            SessionEvent::Relay(RelayEvent::HostLeft(reason)) => out.push(NetEvent::HostLeft(reason)),
            SessionEvent::Relay(RelayEvent::Error(text)) => out.push(NetEvent::Problem(text)),
            SessionEvent::Relay(_) => {}
        }
    }

    fn on_message(&mut self, from: u32, message: Message, now: Instant, out: &mut Vec<NetEvent>) {
        match message {
            Message::RegisterPeer { requested_name } if self.is_host() => {
                let name = clean_name(&requested_name);
                let guest = self.guests.entry(from).or_insert_with(|| Guest { name: String::new(), last_seen: now });
                guest.name = name.clone();
                guest.last_seen = now;
                out.push(NetEvent::GuestJoined(name));
                let room = Message::Room { code: self.code.clone(), seed: self.room_seed.clone(), mode: 0, scale: 0, rest: Map::new() };
                self.coop.send(from, &room);
                if let Some((seed, difficulty)) = self.run.clone() {
                    self.coop.send(from, &start_message(&self.code, &seed, &difficulty));
                }
            }
            Message::PeerLeave if self.is_host() => {
                self.remotes.remove(&from);
                if let Some(guest) = self.guests.remove(&from) {
                    out.push(NetEvent::GuestLeft(guest.name));
                }
            }
            Message::Place { packet } if self.is_host() => {
                if let Some(guest) = self.guests.get_mut(&from) {
                    guest.last_seen = now;
                }
                // a keep-alive (null) or a malformed sample only refreshes the session
                let Some(sample) = Sample::from_json(&packet) else { return };
                if !self.store_remote(from, sample, now) {
                    return;
                }
                let others: Vec<u32> = self.coop.peers().into_iter().filter(|p| *p != from).collect();
                for peer in others {
                    self.coop.send(peer, &Message::PeerPlace { peer_id: from as i64, packet: packet.clone() });
                }
            }
            Message::Room { seed, .. } if !self.is_host() => {
                self.room_seed = seed.clone();
                out.push(NetEvent::Seed(seed));
            }
            Message::Start { seed, rest, .. } if !self.is_host() => {
                let difficulty = rest.get("difficulty").and_then(Value::as_str).unwrap_or("STANDARD").to_string();
                out.push(NetEvent::Start { seed, difficulty });
            }
            Message::PeerPlace { peer_id, packet } if !self.is_host() => {
                // the host's own sample is peer 1; a guest's is its session id (never this session: that is us)
                let Ok(peer) = u32::try_from(peer_id) else { return };
                if peer == self.coop.link.session() {
                    return;
                }
                if let Some(sample) = Sample::from_json(&packet) {
                    self.store_remote(peer, sample, now);
                }
            }
            _ => {}
        }
    }

    /// Keeps a sample only when it is newer than the last one kept for that peer.
    fn store_remote(&mut self, peer: u32, sample: Sample, now: Instant) -> bool {
        if let Some(r) = self.remotes.get(&peer) {
            if !newer(sample.seq, r.sample.seq) {
                return false;
            }
        }
        self.remotes.insert(peer, Remote { sample, last_seen: now });
        true
    }

    fn prune(&mut self, now: Instant, out: &mut Vec<NetEvent>) {
        self.remotes.retain(|_, r| now.duration_since(r.last_seen) < REMOTE_TIMEOUT);
        if self.is_host() {
            let gone: Vec<u32> = self
                .guests
                .iter()
                .filter(|(_, g)| now.duration_since(g.last_seen) >= GUEST_IDLE)
                .map(|(peer, _)| *peer)
                .collect();
            for peer in gone {
                self.remotes.remove(&peer);
                if let Some(guest) = self.guests.remove(&peer) {
                    out.push(NetEvent::GuestLeft(guest.name));
                }
            }
        } else if !self.coop.link.ready() && !self.timed_out && now.duration_since(self.opened) >= JOIN_TIMEOUT {
            self.timed_out = true;
            out.push(NetEvent::Problem("NO ANSWER FROM THE ROOM'S RELAY".into()));
        }
    }

    fn send_due(&mut self, sample: Option<Sample>, now: Instant) {
        if now < self.next_send {
            return;
        }
        self.next_send = now + SEND_INTERVAL;
        self.seq = self.seq.wrapping_add(1);
        let packet = match sample {
            Some(mut s) => {
                s.seq = self.seq;
                s.to_json()
            }
            None => Value::Null,
        };
        if self.is_host() {
            for peer in self.coop.peers() {
                self.coop.send(peer, &Message::PeerPlace { peer_id: HOST_PEER_ID as i64, packet: packet.clone() });
            }
        } else {
            self.coop.send(HOST_PEER_ID, &Message::Place { packet });
        }
    }
}

fn start_message(code: &str, seed: &str, difficulty: &str) -> Message {
    let mut rest = Map::new();
    rest.insert("difficulty".into(), json!(difficulty));
    Message::Start { code: code.to_string(), seed: seed.to_string(), mode: 0, scale: 0, rest }
}

/// A runner's name in a room: letters, digits, space, dash and underscore, upper-cased, at most 24 characters.
/// Empty becomes RUNNER.
pub fn clean_name(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(MAX_NAME)
        .collect();
    let name = kept.trim().to_ascii_uppercase();
    if name.is_empty() {
        "RUNNER".into()
    } else {
        name
    }
}

/// The join dialog's text as a room code: trimmed and upper-cased, then checked against the code rules
/// (`api::valid_join_code`: five characters of the room alphabet, or the reserved codes' letters). `None` when it is not one.
pub fn parse_join_code(text: &str) -> Option<String> {
    let clean = text.trim().to_ascii_uppercase();
    api::valid_join_code(&clean).then_some(clean)
}

/// The route seed text of a hosted room: `ROOM-<code>-<hex>`, the shape of `route::new_room_text`, so the lobby
/// shows the directory code and the run seed is the text's djb2 as usual. `fresh` supplies the hex.
pub fn room_text_for_code(code: &str, fresh: &str) -> String {
    let hex = fresh.split('-').nth(2).filter(|h| !h.is_empty()).unwrap_or("0000000000000000");
    format!("ROOM-{code}-{hex}")
}

/// A difficulty by its name as `Difficulty::name` writes it; anything unknown is STANDARD.
pub fn difficulty_from_name(name: &str) -> Difficulty {
    ALL.iter().copied().find(|d| d.name() == name).unwrap_or(Difficulty::Standard)
}

/// Draws the remote runners as simple figures in the 3D view, lit like the scene and glowing so they read in the dark.
/// Draws nothing when `remotes` is empty, so a single-player frame is unchanged.
pub fn draw_remotes(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget, remotes: &[RemoteView]) {
    if remotes.is_empty() {
        return;
    }
    set_camera(&scene::camera(g, rt));
    gl_use_material(mat);
    shader::apply(mat, &scene::lighting(g), &g.world.lamps);
    let mut b = Builder::new(Some(a.white.clone()));
    for r in remotes {
        let colour = PEER_COLOURS[(r.peer as usize) % PEER_COLOURS.len()];
        b.cube(vec3(r.x, 0.85, r.y), vec3(0.42, 1.7, 0.42), r.yaw, colour, 2.0);
    }
    b.draw_and_clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::relay::{decode, encode, frame};
    use std::net::UdpSocket;

    fn sample(seq: u32, x: f32, y: f32) -> Sample {
        Sample { seq, depth: 1, x, y, yaw: 0.5, moving: true, sprinting: false, crouching: true }
    }

    #[test]
    fn sample_json_round_trips_and_ignores_keep_alives() {
        let s = Sample { seq: 9, depth: -2, x: 12.5, y: -3.25, yaw: 1.57, moving: true, sprinting: true, crouching: false };
        assert_eq!(Sample::from_json(&s.to_json()), Some(s));
        assert_eq!(Sample::from_json(&Value::Null), None, "a keep-alive is not a sample");
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": "far", "y": 0})), None);
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": 1e39, "y": 0})), None, "infinity is refused");
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": 0.0, "y": 0.0}))
            .map(|s| (s.yaw, s.moving)), Some((0.0, false)), "missing flags read as false");
    }

    #[test]
    fn newer_follows_the_wrapping_rule() {
        assert!(newer(1, 0));
        assert!(!newer(0, 0));
        assert!(!newer(0, 1));
        assert!(newer(0, u32::MAX), "a wrap is newer");
        assert!(!newer(u32::MAX, 0));
    }

    #[test]
    fn room_codes_parse_from_the_join_text() {
        assert_eq!(parse_join_code("  k7q2p "), Some("K7Q2P".into()));
        assert_eq!(parse_join_code("ABC12"), Some("ABC12".into()));
        assert_eq!(parse_join_code("ABCD"), None, "too short");
        assert_eq!(parse_join_code("ABCDEF"), None, "too long");
        assert_eq!(parse_join_code("AB-12"), None, "not a room character");
        assert_eq!(parse_join_code(""), None);
    }

    #[test]
    fn the_hosted_room_text_carries_its_code_and_a_fresh_seed() {
        let text = room_text_for_code("K7Q2P", "ROOM-AAAAA-0123456789ABCDEF0123");
        assert_eq!(text, "ROOM-K7Q2P-0123456789ABCDEF0123");
        assert_eq!(crate::world::route::room_text_code(&text), "K7Q2P");
        assert_eq!(crate::world::route::run_seed_of_text(&text), crate::world::route::run_seed_of_text("ROOM-K7Q2P-0123456789ABCDEF0123"));
        assert!(room_text_for_code("K7Q2P", "garbage").starts_with("ROOM-K7Q2P-"));
    }

    #[test]
    fn difficulty_names_round_trip_and_unknowns_are_standard() {
        for d in ALL {
            assert!(difficulty_from_name(d.name()) == d);
        }
        assert!(difficulty_from_name("NIGHTMARE-ISH") == Difficulty::Standard);
    }

    #[test]
    fn names_are_cleaned_for_the_room() {
        assert_eq!(clean_name("  kyttu<script>!  "), "KYTTUSCRIPT");
        assert_eq!(clean_name("abcdefghijklmnopqrstuvwxyz0123"), "ABCDEFGHIJKLMNOPQRSTUVWX");
        assert_eq!(clean_name("!!!"), "RUNNER");
    }

    /// Host and guest on one fake relay: each sees the other's sample, and the start is delivered.
    #[test]
    fn host_and_guest_see_each_others_positions_through_a_relay() {
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        relay.set_nonblocking(true).unwrap();
        let port = relay.local_addr().unwrap().port();
        let cfg = BackendConfig::loopback(18105);
        let mut host = NetSession::new(
            cfg.clone(),
            "ABCDE".into(),
            Role::Host,
            CoopSession::new(RelayLink::host("ABCDE", "tok", "127.0.0.1", port).unwrap()),
            "tok".into(),
            "ROOM-ABCDE-0123".into(),
            "HOST".into(),
        );
        let mut guest = NetSession::new(
            cfg,
            "ABCDE".into(),
            Role::Guest,
            CoopSession::new(RelayLink::guest("ABCDE", "127.0.0.1", port, "").unwrap()),
            String::new(),
            String::new(),
            "GUEST".into(),
        );
        host.start_run("ROOM-ABCDE-0123", "STANDARD");
        let (mut host_addr, mut guest_addr) = (None, None);
        let mut buf = [0u8; 2048];
        let mut events = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let (host_sample, guest_sample) = (sample(0, 3.0, -2.0), sample(0, 7.5, 1.25));
        let mut done = (false, false, false);
        while Instant::now() < deadline && !(done.0 && done.1 && done.2) {
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
            events.extend(host.tick(Some(host_sample)));
            events.extend(guest.tick(Some(guest_sample)));
            done.0 = host.remotes_at(1).iter().any(|r| r.peer == 42 && (r.x - 7.5).abs() < 1e-4 && (r.y - 1.25).abs() < 1e-4);
            done.1 = guest.remotes_at(1).iter().any(|r| r.peer == HOST_PEER_ID && (r.x - 3.0).abs() < 1e-4);
            done.2 = events.contains(&NetEvent::Start { seed: "ROOM-ABCDE-0123".into(), difficulty: "STANDARD".into() })
                || guest.room_seed() == "ROOM-ABCDE-0123";
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(done.0, "the host sees the guest's sample");
        assert!(done.1, "the guest sees the host's sample");
        assert!(done.2, "the guest receives the route seed");
        assert!(events.contains(&NetEvent::GuestJoined("GUEST".into())));
        assert!(host.is_host() && !guest.is_host());
    }

    /// Against a running lr_server (`LR_NET_TEST_HTTP_PORT`, relay as the server reports it): host a room, join it by
    /// its code, start the route, exchange positions both ways, then leave. Skipped when the variable is not set.
    #[test]
    fn a_room_on_the_live_server_joins_by_code_and_exchanges_positions() {
        let Some(http) = std::env::var("LR_NET_TEST_HTTP_PORT").ok().and_then(|p| p.parse::<u16>().ok()) else { return };
        room_round_trip(BackendConfig::loopback(http));
    }

    /// One real room on the primary server (the VPS directory and its relay): the same round trip, then the room is
    /// closed. Ignored by default because it writes a room to the live directory; run it with
    /// `LR_NET_TEST_VPS=1 cargo test --bin last_route vps_room_round_trip -- --ignored`.
    #[test]
    #[ignore]
    fn vps_room_round_trip() {
        if std::env::var("LR_NET_TEST_VPS").ok().as_deref() != Some("1") {
            return;
        }
        room_round_trip(BackendConfig::primary());
    }

    fn room_round_trip(cfg: BackendConfig) {
        let route = crate::world::route::new_room_text();
        let mut host = host_room(cfg.clone(), route.clone(), "HOST".into()).wait().expect("the host registers a room");
        assert!(host.is_host());
        let code = host.code.clone();
        assert!(api::valid_room_code(&code), "{code}");
        host.start_run(&route, "STANDARD");
        let mut guest = join_room(cfg.clone(), code.clone(), "GUEST".into()).wait().expect("the guest joins by code");
        assert!(!guest.is_host());
        let unknown = join_room(cfg.clone(), "AAAAA".into(), "X".into()).wait().err().and_then(|e| e.status());
        assert_eq!(unknown, Some(404), "an unknown code is refused by the directory");

        let (host_sample, guest_sample) = (sample(0, 3.0, -2.0), sample(0, 7.5, 1.25));
        let mut guest_events = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(12);
        let (mut host_sees, mut guest_sees, mut seed_seen) = (false, false, false);
        while Instant::now() < deadline && !(host_sees && guest_sees && seed_seen) {
            host.tick(Some(host_sample));
            guest_events.extend(guest.tick(Some(guest_sample)));
            host_sees = host.remotes_at(1).iter().any(|r| (r.x - 7.5).abs() < 1e-4);
            guest_sees = guest.remotes_at(1).iter().any(|r| (r.x - 3.0).abs() < 1e-4);
            seed_seen = guest.room_seed() == route || guest_events.contains(&NetEvent::Start { seed: route.clone(), difficulty: "STANDARD".into() });
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host_sees, "the host sees the guest on the live relay");
        assert!(guest_sees, "the guest sees the host on the live relay");
        assert!(seed_seen, "the guest gets the route seed and the start");
        guest.leave();
        host.leave();
    }
}
