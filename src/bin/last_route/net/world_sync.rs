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

use super::api::{self, Heartbeat, RegisterRoom, Registered, ReservedHostView};
use super::hunter_events::{self, Tracker};
use super::config::{self, BackendConfig};
use super::protocol::{self, Message};
use super::relay::{RelayEvent, RelayLink, Role};
use super::session::{CoopSession, SessionEvent, HOST_PEER_ID};
use super::{store, NetError, Pending};
use crate::config::difficulty::{ALL, LOBBY};
use crate::config::Difficulty;
use crate::game::player::Player;
use crate::game::Game;
use crate::shared::assets::Assets;
use macroquad::prelude::*;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// A runner's sample goes out this often (the original's 20 Hz `_place`).
pub const SEND_INTERVAL: Duration = Duration::from_millis(50);
/// A remote runner with no sample for this long is neither drawn nor kept.
pub const REMOTE_TIMEOUT: Duration = Duration::from_secs(2);
/// A runner whose sample has been quiet this long is listed as DISCONNECTED (Godot `room_player_list.gd`
/// `QUIET_DISCONNECT_USEC` = 8 s).
pub const QUIET_DISCONNECT: Duration = Duration::from_secs(8);
/// A runner's last state is kept this long for the PLAYERS list, so a silent one reads DISCONNECTED before it drops.
/// Godot drops a silent peer "a little later" without a figure in `room_player_list.gd`; this is the guest idle time.
const ROSTER_KEEP: Duration = GUEST_IDLE;
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
/// The skin a runner wears when none is chosen. It is the id the wire profile carries.
pub const SKIN_DEFAULT: &str = "DEFAULT";
/// Longest skin id a profile or the settings keep.
const MAX_SKIN: usize = 24;
/// The skins by id, each with its figure colours by peer id. DEFAULT is the only one so far; an id no skin has
/// is drawn as DEFAULT (see [`skin_colour`]).
const SKINS: [(&str, [Color; 4]); 1] = [(SKIN_DEFAULT, PEER_COLOURS)];

/// One runner's sample: the `_place` packet `{seq, depth, x, y, yaw, moving, sprinting, crouching}`.
/// `x` and `y` are the runner's absolute ground cell (its window position moved by the window's origin, see
/// [`to_absolute`]), so runners in different streamed windows agree on where each one stands. `depth` is its floor.
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
    /// The runner's run state for the PLAYERS list (Godot `remote_players.gd` state: `spectating`, `game_over`,
    /// `escape_active`). A runner that is not spectating sends `false`; see [`Sample::with_run_state`].
    pub spectating: bool,
    pub game_over: bool,
    pub escape_active: bool,
}

impl Sample {
    /// The sample of a runner's body in the window whose origin is `origin`; `seq` is filled in when it is sent.
    /// The run state flags start false: [`Sample::with_run_state`] sets them.
    pub fn of(p: &Player, depth: i32, origin: (i32, i32)) -> Sample {
        let (x, y) = to_absolute(p.pos, origin);
        Sample {
            seq: 0,
            depth,
            x,
            y,
            yaw: p.yaw,
            moving: p.moving,
            sprinting: p.sprinting,
            crouching: p.crouching,
            spectating: false,
            game_over: false,
            escape_active: false,
        }
    }

    /// The sample with this runner's run state: `game_over` once its route has ended (a catch or an abandon),
    /// `escape_active` while a hunter holds it in the escape struggle.
    pub fn with_run_state(mut self, game_over: bool, escape_active: bool) -> Sample {
        self.game_over = game_over;
        self.escape_active = escape_active;
        self
    }

    pub fn to_json(&self) -> Value {
        json!({
            "seq": self.seq, "depth": self.depth, "x": self.x, "y": self.y, "yaw": self.yaw,
            "moving": self.moving, "sprinting": self.sprinting, "crouching": self.crouching,
            "spectating": self.spectating, "game_over": self.game_over, "escape_active": self.escape_active,
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
            spectating: flag("spectating"),
            game_over: flag("game_over"),
            escape_active: flag("escape_active"),
        })
    }
}

/// A window position in absolute cells: what a sample carries, so every runner's sample means the same cell.
pub fn to_absolute(at: (f32, f32), origin: (i32, i32)) -> (f32, f32) {
    (at.0 + origin.0 as f32, at.1 + origin.1 as f32)
}

/// An absolute cell in a window with origin `origin`: the inverse of [`to_absolute`].
pub fn to_window(at: (f32, f32), origin: (i32, i32)) -> (f32, f32) {
    (at.0 - origin.0 as f32, at.1 - origin.1 as f32)
}

/// `a` is a newer sample than `last`, with wrap-around (the unreliable-ordered rule of `net::session`).
pub fn newer(seq: u32, last: u32) -> bool {
    seq != last && seq.wrapping_sub(last) < u32::MAX / 2
}

/// What the PLAYERS list knows of a runner's state: its last sample's floor and run flags, and how long ago that sample
/// arrived (Godot `room_player_list.gd` `states` entry: `{depth, spectating, game_over, escape_active, received_usec}`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunnerState {
    pub depth: i32,
    pub spectating: bool,
    pub game_over: bool,
    pub escape_active: bool,
    /// Time since the last sample, zero for the local runner.
    pub quiet: Duration,
}

/// One other runner in the room, for the PLAYERS list. `depth` is its floor while it sends samples; `sending` is whether
/// it has sent one lately; `ping` is its round trip in ms, None until one is measured; `state` is its last sample's
/// state, None until one arrives (a runner that just joined).
#[derive(Clone, Debug, PartialEq)]
pub struct RosterEntry {
    pub peer: u32,
    pub name: String,
    pub skin: String,
    pub host: bool,
    pub depth: Option<i32>,
    pub sending: bool,
    pub ping: Option<u32>,
    pub state: Option<RunnerState>,
}

/// A remote runner that is on this floor and fresh enough to draw.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteView {
    pub peer: u32,
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
    /// The runner's skin id (`SKIN_DEFAULT` until its profile arrives).
    pub skin: String,
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
    /// The room's mod pack ids changed (guest side). The guest shows them; a pack it lacks must be installed, then
    /// the game restarted, because packs are read at boot.
    Packs(Vec<String>),
    /// The room is gone (the host quit, or the directory forgot it).
    HostLeft(String),
    /// Something to show the player.
    Problem(String),
    /// The room's owner sent a route command (host side, reserved rooms only). The app carries it out, as Godot's
    /// `RoomState._on_reserved_commands` does, and the command is acked when it is handed over.
    RouteCommand(RouteCommand),
    /// The host ended the route (guest side): the run is over, the room stays open. The text is the host's reason.
    RouteEnded(String),
    /// The host's hunter frame (guest side): each hunter's state, as `hunter_events::state_name` names it. Display only.
    Hunters(Vec<Value>),
    /// A world event the host relayed (guest side): an object with `what` (see `net::hunter_events`).
    WorldEvent(Map<String, Value>),
}

/// An owner route command for a reserved room, as Godot `RoomState.carry_out_route_command` carries it out.
#[derive(Clone, Debug, PartialEq)]
pub enum RouteCommand {
    /// RESTART ROUTE: a new run. `same_seed` keeps the room's seed (`args.sameSeed`). `seed` is `args.settings.seed`,
    /// trimmed (empty when none). `difficulty` is `args.settings.difficulty` when the settings carry one.
    Restart { same_seed: bool, seed: String, difficulty: Option<Difficulty> },
    /// END ROUTE: the run ends for everyone and the room waits in its lobby.
    End,
}

impl RouteCommand {
    /// The route command an owner command names: `restart` or `end` (Godot `RoomState._on_reserved_commands`). None
    /// for the others (kick, settings, transfer), which this build does not carry out.
    pub fn parse(command: &str, args: &Map<String, Value>) -> Option<RouteCommand> {
        match command {
            "restart" => {
                let same_seed = args.get("sameSeed").and_then(Value::as_bool).unwrap_or(false);
                let settings = args.get("settings").and_then(Value::as_object);
                let seed = settings
                    .and_then(|s| s.get("seed"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let difficulty = settings.and_then(|s| s.get("difficulty")).map(difficulty_from_index);
                Some(RouteCommand::Restart { same_seed, seed, difficulty })
            }
            "end" => Some(RouteCommand::End),
            _ => None,
        }
    }

    /// The seed a restarted route runs on (Godot `_apply_reserved_settings` and `reroll_seed`): the room's own seed when
    /// `same_seed`, else the stored `seed` when there is one, else `fresh`.
    pub fn restart_seed(same_seed: bool, seed: &str, current: &str, fresh: &str) -> String {
        if same_seed {
            current.to_string()
        } else if !seed.is_empty() {
            seed.to_string()
        } else {
            fresh.to_string()
        }
    }
}

/// A settings difficulty index as Godot's `_selectable_difficulty_or_standard` reads it: 0 RELAXED, 1 STANDARD, 2 HARD.
/// Nightmare (3) has no port in this build, so it, and any other value, is STANDARD.
fn difficulty_from_index(v: &Value) -> Difficulty {
    v.as_f64()
        .map(|f| f as i64)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| ALL.get(i).copied())
        .unwrap_or(Difficulty::Standard)
}

/// Godot's `_reserved_seen` and `_reserved_ack` (`backend_client.gd`): the highest owner-command seq seen, and the
/// highest one handed on. They go out with each heartbeat as `commandSince` and `commandAck`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnerCursor {
    pub seen: i64,
    pub ack: i64,
}

impl OwnerCursor {
    /// Godot `_take_reserved` and `_on_reserved_commands`: each command above the seen cursor, in order. A route command
    /// becomes an event for the app. Any other command becomes a notice that this build does not carry it out. Each one
    /// is acked as it is taken, so the next heartbeat stops the directory sending it.
    pub fn take(&mut self, view: &ReservedHostView) -> Vec<NetEvent> {
        let mut out = Vec::new();
        for cmd in &view.commands {
            if cmd.seq <= self.seen {
                continue;
            }
            self.seen = cmd.seq;
            match RouteCommand::parse(&cmd.command, &cmd.args) {
                Some(route) => out.push(NetEvent::RouteCommand(route)),
                None => out.push(NetEvent::Problem(format!("OWNER COMMAND {} IS NOT IN THIS BUILD", cmd.command.to_uppercase()))),
            }
            self.ack = self.ack.max(cmd.seq);
        }
        out
    }
}

struct Remote {
    sample: Sample,
    last_seen: Instant,
}

struct Guest {
    name: String,
    last_seen: Instant,
}

/// The wire profile of a runner: `{peer_id, name, skin}`. A guest sends its own (the host reads the sender from the
/// relay); the host forwards each profile with its `peer_id`, and sends a new guest every profile in the room.
fn profile_message(peer: u32, name: &str, skin: &str) -> Message {
    let mut profile = Map::new();
    profile.insert("peer_id".into(), json!(peer));
    profile.insert("name".into(), json!(name));
    profile.insert("skin".into(), json!(skin));
    Message::WireProfile { profile }
}

/// The host's room mod packs as the wire carries them: `{ids: [...]}`.
fn packs_message(ids: &[String]) -> Message {
    let mut packs = Map::new();
    packs.insert("ids".into(), Value::Array(ids.iter().cloned().map(Value::String).collect()));
    Message::Packs { packs }
}

/// The skin a received profile names, cleaned. A profile without one is DEFAULT.
fn profile_skin(profile: &Map<String, Value>) -> String {
    clean_skin(profile.get("skin").and_then(Value::as_str).unwrap_or_default())
}

/// The skin a host publishes in its room: its own skin id only when the website holds that skin (`uploaded` is the
/// list of ids it accepted, see `shared::skins::uploaded_ids`), else DEFAULT. Godot `SkinSync.shared_hash`.
pub fn published_skin(skin: &str, uploaded: &[String]) -> String {
    let id = clean_skin(skin);
    if is_skin_hash(&id) && uploaded.contains(&id) {
        id
    } else {
        SKIN_DEFAULT.into()
    }
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
    /// Guest: the host's name and the other runners' names, from the host's last roster.
    host_name: String,
    players: Vec<String>,
    /// This runner's skin id, sent in its wire profile and to the guests that join.
    skin: String,
    /// The room's mod pack ids: host, the ones it was given ([`NetSession::set_room_mods`]); guest, the ones the host
    /// sent. Sent to each runner with the room.
    mods: Vec<String>,
    /// The room's difficulty name: host, the one it sets ([`NetSession::set_room_difficulty`]); guest, the host's, from
    /// the room and the route start. A guest never sets it.
    difficulty: String,
    /// The skin id of each other runner, by peer id, from its wire profile.
    skins: HashMap<u32, String>,
    /// The name of each other runner, by peer id, from its wire profile (a guest keeps the others' names too).
    names: HashMap<u32, String>,
    guests: HashMap<u32, Guest>,
    remotes: HashMap<u32, Remote>,
    seq: u32,
    next_send: Instant,
    next_heartbeat: Instant,
    heartbeat: Option<Pending<api::HeartbeatReply>>,
    relist: Option<Pending<Registered>>,
    opened: Instant,
    timed_out: bool,
    /// Host: the owner-command cursors of a reserved room (see [`OwnerCursor`]).
    owner: OwnerCursor,
    /// Host: owner events from the registration reply, handed to the app on the first tick.
    backlog: Vec<NetEvent>,
    /// Host: the hunter event state (last states, grab, frame timing), see [`hunter_events::Tracker`].
    hunter_tracker: Tracker,
}

/// CREATE a room on the directory and open its relay. The request blocks, so the frame loop polls the result.
pub fn host_room(cfg: BackendConfig, route_seed: String, name: String, skin: String) -> Pending<NetSession> {
    Pending::spawn(move || {
        if cfg.offline {
            return Err(NetError::Offline);
        }
        let device_id = store::device_id(&store::default_dir());
        for _ in 0..REGISTER_TRIES {
            let code = api::make_room_code();
            let request = RegisterRoom { code: code.clone(), device_id: device_id.clone(), ..RegisterRoom::default() };
            match api::register_room(&cfg, &request) {
                Ok(reg) => return open_host(cfg, code, route_seed, name, skin, reg),
                Err(NetError::Http { status: 403, code: reserved, .. }) if reserved == "ROOM_RESERVED" => continue,
                Err(e) => return Err(e),
            }
        }
        Err(NetError::Refused("NO FREE ROOM CODE // TRY AGAIN".into()))
    })
}

fn open_host(
    cfg: BackendConfig,
    code: String,
    route_seed: String,
    name: String,
    skin: String,
    reg: Registered,
) -> Result<NetSession, NetError> {
    if reg.room.transport != "relay" || reg.room.address.is_empty() {
        // this client has no direct path: give the code back rather than leave a room nobody can join
        let _ = api::unregister_room(&cfg, &code, &reg.token);
        return Err(NetError::Unsupported("THE SERVER DOES NOT RELAY ROOMS".into()));
    }
    let link = RelayLink::host(&code, &reg.token, &reg.room.address, reg.room.port)
        .map_err(|e| NetError::Transport(e.to_string()))?;
    let uploaded = if is_skin_hash(&clean_skin(&skin)) { crate::shared::skins::uploaded_ids() } else { Vec::new() };
    let skin = published_skin(&skin, &uploaded);
    // Godot `_take_reserved(payload.reserved, true)`: a fresh registration takes the commands it lists
    let mut owner = OwnerCursor::default();
    let backlog = reg.reserved.as_ref().map(|view| owner.take(view)).unwrap_or_default();
    let mut session = NetSession::new(cfg, code, Role::Host, CoopSession::new(link), reg.token, route_seed, name, skin);
    session.owner = owner;
    session.backlog = backlog;
    Ok(session)
}

/// JOIN by code: the directory lookup, then the join. An owner's lookup that finds nothing (404) or hits a code already
/// reserved (403 `ROOM_RESERVED`) reserves the typed code and hosts it ([`reserve_and_host`]); every other lookup, a
/// guest's miss and a live code included, takes the
/// guest join. Blocking, so polled like [`host_room`].
pub fn join_room(cfg: BackendConfig, code: String, name: String, skin: String, session_token: String, is_admin: bool) -> Pending<NetSession> {
    Pending::spawn(move || {
        if cfg.offline {
            return Err(NetError::Offline);
        }
        match lookup_outcome(api::lookup_room(&cfg, &code), is_admin, &session_token)? {
            JoinStep::Found(room) => join_found(cfg, code, room, name),
            JoinStep::Reserve => reserve_and_host(cfg, code, name, skin, session_token),
        }
    })
}

/// What a JOIN ROOM does with its directory lookup.
#[derive(Debug)]
enum JoinStep {
    /// The code is live: join the room the directory returned.
    Found(api::RoomEndpoint),
    /// An owner's miss: reserve the typed code for her and host it.
    Reserve,
}

/// The step for a lookup result: only an owner's own not-found lookup reserves; a guest's miss and any other failure
/// are returned as they are (`ROOM NOT FOUND` for a guest).
fn lookup_outcome(lookup: Result<api::RoomEndpoint, NetError>, is_admin: bool, session_token: &str) -> Result<JoinStep, NetError> {
    match lookup {
        Ok(room) => Ok(JoinStep::Found(room)),
        Err(miss) if api::admin_may_reserve_on_miss(is_admin, session_token, &miss) => Ok(JoinStep::Reserve),
        Err(e) => Err(e),
    }
}

/// The guest join to a live room: it must be on the relay and run this build's protocol, then the relay joins.
/// A guest publishes no skin: the guest's own skin stays on its own screen (Godot "Local-only", `PLAYER_SKINS.md`).
fn join_found(cfg: BackendConfig, code: String, room: api::RoomEndpoint, name: String) -> Result<NetSession, NetError> {
    if room.transport != "relay" || room.address.is_empty() {
        return Err(NetError::Refused("THAT ROOM CANNOT BE JOINED FROM THIS BUILD".into()));
    }
    let refused = api::protocol_mismatch(&room.protocol, &protocol::fingerprint());
    if !refused.is_empty() {
        return Err(NetError::Refused(refused));
    }
    let link = RelayLink::guest(&code, &room.address, room.port, "").map_err(|e| NetError::Transport(e.to_string()))?;
    Ok(NetSession::new(cfg, code, Role::Guest, CoopSession::new(link), String::new(), String::new(), name, SKIN_DEFAULT.into()))
}

/// An owner's miss: reserve the typed code for her session, then register and open it as the host (the same open as
/// [`host_room`]). A code someone started hosting between the lookup and the reserve joins that live room instead.
fn reserve_and_host(cfg: BackendConfig, code: String, name: String, skin: String, session_token: String) -> Result<NetSession, NetError> {
    match api::admin_reserve_room(&cfg, &code, &session_token) {
        Ok(()) => {}
        Err(NetError::Http { code: c, .. }) if c == "ROOM_ALREADY_HOSTED" => {
            let room = api::lookup_room(&cfg, &code)?;
            return join_found(cfg, code, room, name);
        }
        Err(e) => return Err(e),
    }
    let request = RegisterRoom {
        code: code.clone(),
        device_id: store::device_id(&store::default_dir()),
        session_token,
        ..RegisterRoom::default()
    };
    let reg = api::register_room(&cfg, &request)?;
    open_host(cfg, code, crate::world::route::new_room_text(), name, skin, reg)
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
        skin: String,
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
            host_name: String::new(),
            players: Vec::new(),
            skin,
            mods: Vec::new(),
            difficulty: Difficulty::Standard.name().to_string(),
            skins: HashMap::new(),
            names: HashMap::new(),
            guests: HashMap::new(),
            remotes: HashMap::new(),
            seq: 0,
            next_send: now,
            next_heartbeat: now + Duration::from_secs_f64(config::HEARTBEAT_SECONDS),
            heartbeat: None,
            relist: None,
            opened: now,
            timed_out: false,
            owner: OwnerCursor::default(),
            backlog: Vec::new(),
            hunter_tracker: Tracker::default(),
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

    /// Host: the room's mod pack ids (the reserved room's `mods`), sent to every runner with the room. Ids that are not
    /// a pack id are dropped, repeats too, and the list is capped at [`MAX_MODS`].
    pub fn set_room_mods(&mut self, ids: &[String]) {
        self.mods = clean_mod_ids(ids);
    }

    /// The room's mod pack ids (host: the ones it was given; guest: the ones the host sent).
    pub fn room_mods(&self) -> &[String] {
        &self.mods
    }

    /// Host: the room's difficulty, sent to every guest with the room. A change goes out at once, so a guest's lobby
    /// shows it. A guest's call does nothing: only the host sets the room's difficulty.
    pub fn set_room_difficulty(&mut self, difficulty: Difficulty) {
        if self.is_host() && self.difficulty != difficulty.name() {
            self.difficulty = difficulty.name().to_string();
            self.send_room_and_roster();
        }
    }

    /// The room's difficulty name (host: the one it set; guest: the host's). Read it with [`difficulty_from_name`].
    pub fn room_difficulty(&self) -> &str {
        &self.difficulty
    }

    /// The other runners for the lobby roster. Host: the guests that have registered, in the order they joined
    /// (peer id). Guest: the host first, then the other guests, as the host's last roster listed them. This runner
    /// and the host are each listed once.
    pub fn guest_names(&self) -> Vec<String> {
        if !self.is_host() {
            let mut names = Vec::new();
            if !self.host_name.is_empty() {
                names.push(self.host_name.clone());
            }
            names.extend(
                self.players
                    .iter()
                    .filter(|p| {
                        !p.is_empty()
                            && !p.eq_ignore_ascii_case(&self.name)
                            && !p.eq_ignore_ascii_case(&self.host_name)
                    })
                    .cloned(),
            );
            return names;
        }
        let mut peers: Vec<(&u32, &Guest)> = self.guests.iter().collect();
        peers.sort_by_key(|(peer, _)| **peer);
        peers
            .into_iter()
            .filter(|(_, g)| !g.name.is_empty())
            .map(|(_, g)| g.name.clone())
            .collect()
    }

    /// The other runners for the PLAYERS list, the host first. A guest lists the host and each other runner it has a
    /// state for; the host lists each guest that registered. `sending` is whether a sample came lately; `state` is the
    /// last sample's state (None until one arrives), and it is kept until the runner drops, so a silent one reads
    /// DISCONNECTED.
    pub fn roster(&self) -> Vec<RosterEntry> {
        let now = Instant::now();
        let fresh = |peer: u32| {
            self.remotes
                .get(&peer)
                .filter(|r| now.duration_since(r.last_seen) < REMOTE_TIMEOUT)
                .map(|r| r.sample.depth)
        };
        let entry = |peer: u32, name: String| {
            let depth = fresh(peer);
            let state = self.remotes.get(&peer).map(|r| RunnerState {
                depth: r.sample.depth,
                spectating: r.sample.spectating,
                game_over: r.sample.game_over,
                escape_active: r.sample.escape_active,
                quiet: now.duration_since(r.last_seen),
            });
            RosterEntry {
                peer,
                name,
                skin: self.skins.get(&peer).cloned().unwrap_or_else(|| SKIN_DEFAULT.into()),
                host: peer == HOST_PEER_ID,
                depth,
                sending: depth.is_some(),
                ping: self.coop.ping_ms(peer),
                state,
            }
        };
        if self.is_host() {
            let mut peers: Vec<(&u32, &Guest)> = self.guests.iter().collect();
            peers.sort_by_key(|(peer, _)| **peer);
            return peers.into_iter().map(|(peer, guest)| entry(*peer, guest.name.clone())).collect();
        }
        let mut out = Vec::new();
        if !self.host_name.is_empty() || fresh(HOST_PEER_ID).is_some() {
            out.push(entry(HOST_PEER_ID, self.host_name.clone()));
        }
        let mut others: Vec<(&u32, &String)> = self
            .names
            .iter()
            .filter(|(peer, _)| {
                **peer != HOST_PEER_ID && **peer != self.coop.link.session() && self.remotes.contains_key(*peer)
            })
            .collect();
        others.sort_by_key(|(peer, _)| **peer);
        out.extend(others.into_iter().map(|(peer, name)| entry(*peer, name.clone())));
        out
    }

    /// True once the relay accepted us (HOST_ACK / GUEST_ACK).
    pub fn linked(&self) -> bool {
        self.coop.link.ready()
    }

    /// The route seed the room runs on (host: its own; guest: the last one received).
    pub fn room_seed(&self) -> &str {
        &self.room_seed
    }

    /// Host: the room text the guests are sent. It must be the text the host runs on.
    pub fn set_room_seed(&mut self, seed: String) {
        self.room_seed = seed;
    }

    /// Host: sends the room (its code, seed and the roster) to every guest. The roster is the host and the guests that
    /// have registered, so a guest can list everyone in the lobby.
    fn send_room_and_roster(&mut self) {
        let mut rest = Map::new();
        rest.insert("host".into(), Value::String(self.name.clone()));
        let players: Vec<Value> = self.guest_names().into_iter().map(Value::String).collect();
        rest.insert("players".into(), Value::Array(players));
        rest.insert("difficulty".into(), Value::String(self.difficulty.clone()));
        let room = Message::Room { code: self.code.clone(), seed: self.room_seed.clone(), mode: 0, scale: 0, rest };
        let packs = packs_message(&self.mods);
        for peer in self.coop.peers() {
            self.coop.send(peer, &room);
            self.coop.send(peer, &packs);
        }
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

    /// Host: sends the hunters' frame to every guest at the rate of `data/net/hunter_events.json`, and the world events
    /// the rules allow (state changes, the grab start and end). `grab` is the index of the hunter holding the runner.
    /// Nothing is sent by a guest, and nothing on a difficulty the rules leave out. Call it once per frame.
    pub fn send_hunter_frame(&mut self, frame: Vec<Value>, diff: Difficulty, grab: Option<usize>, now: Instant) {
        if !self.is_host() {
            return;
        }
        let rules = hunter_events::rules();
        for event in self.hunter_tracker.events(rules, diff, &frame, grab, now) {
            let message = Message::Event { event };
            for peer in self.coop.peers() {
                self.coop.send(peer, &message);
            }
        }
        if !hunter_events::frames_on(rules, diff) || !self.hunter_tracker.frame_due(rules, now) {
            return;
        }
        let bytes = serde_json::to_vec(&frame).map_or(0, |b| b.len());
        let message = if bytes > rules.large_frame_over_bytes {
            Message::HuntersLarge { data: frame }
        } else {
            Message::Hunters { data: frame }
        };
        for peer in self.coop.peers() {
            self.coop.send(peer, &message);
        }
    }

    /// Host: ends the route for every guest (Godot `end_session` and `_receive_end`). The room stays open, and no run is
    /// on until the next [`NetSession::start_run`].
    pub fn end_run(&mut self, reason: &str) {
        self.run = None;
        if self.is_host() {
            for peer in self.coop.peers() {
                self.coop.send(peer, &Message::End { reason: reason.to_string() });
            }
        }
    }

    /// Once per frame. Pumps the relay and the directory, applies what the others sent and returns the events for the
    /// app. `sample` is this runner's position while a route is running; `None` sends a keep-alive instead.
    pub fn tick(&mut self, sample: Option<Sample>) -> Vec<NetEvent> {
        let now = Instant::now();
        let mut out = std::mem::take(&mut self.backlog);
        self.service_directory(now, &mut out);
        for event in self.coop.poll() {
            self.on_session_event(event, now, &mut out);
        }
        self.prune(now, &mut out);
        self.send_due(sample, now);
        out
    }

    /// The other runners on `depth` that are fresh enough to draw, in peer order. `origin` is this runner's window
    /// origin: the samples are absolute cells, so they are moved into this window before they are returned.
    pub fn remotes_at(&self, depth: i32, origin: (i32, i32)) -> Vec<RemoteView> {
        let now = Instant::now();
        let mut out: Vec<RemoteView> = self
            .remotes
            .iter()
            .filter(|(_, r)| r.sample.depth == depth && now.duration_since(r.last_seen) < REMOTE_TIMEOUT)
            .map(|(peer, r)| {
                let (x, y) = to_window((r.sample.x, r.sample.y), origin);
                RemoteView {
                    peer: *peer,
                    x,
                    y,
                    yaw: r.sample.yaw,
                    skin: self.skins.get(peer).cloned().unwrap_or_else(|| SKIN_DEFAULT.into()),
                }
            })
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
                match result {
                    // the owner's commands for this reserved room ride on the heartbeat answer
                    Ok(reply) => {
                        if let Some(view) = reply.reserved.as_ref() {
                            out.extend(self.owner.take(view));
                        }
                    }
                    // the directory forgot the room (restart or expiry): list it again under the same code
                    Err(e) if matches!(e.status(), Some(403) | Some(404)) => self.start_relist(),
                    Err(_) => {}
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
                        // Godot `_take_reserved(..., true)`: a fresh registration starts the owner cursors again
                        self.owner = OwnerCursor::default();
                        if let Some(view) = reg.reserved.as_ref() {
                            out.extend(self.owner.take(view));
                        }
                    }
                    Err(_) => out.push(NetEvent::Problem("ROOM LOST // THE DIRECTORY FORGOT IT".into())),
                }
            }
        }
        if self.heartbeat.is_none() && self.relist.is_none() && now >= self.next_heartbeat {
            self.next_heartbeat = now + Duration::from_secs_f64(config::HEARTBEAT_SECONDS);
            let beat = self.beat();
            let cfg = self.cfg.clone();
            self.heartbeat = Some(Pending::spawn(move || api::heartbeat(&cfg, &beat)));
        }
    }

    /// The heartbeat this host sends now (Godot `_send_heartbeat`): its runners and route, and the owner-command cursors.
    fn beat(&self) -> Heartbeat {
        Heartbeat {
            code: self.code.clone(),
            token: self.token.clone(),
            players: 1 + self.guests.len() as u32,
            session_active: self.run.is_some(),
            daytime: false,
            command_since: self.owner.seen,
            command_ack: self.owner.ack,
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
                    let profile = profile_message(self.coop.link.session(), &self.name, &self.skin);
                    self.coop.send(HOST_PEER_ID, &profile);
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
                self.send_room_and_roster();
                if let Some((seed, difficulty)) = self.run.clone() {
                    self.coop.send(from, &start_message(&self.code, &seed, &difficulty));
                }
                self.send_profiles_to(from);
            }
            Message::WireProfile { profile } if self.is_host() => {
                let skin = profile_skin(&profile);
                let name = clean_name(profile.get("name").and_then(Value::as_str).unwrap_or_default());
                self.skins.insert(from, skin.clone());
                let forward = profile_message(from, &name, &skin);
                let others: Vec<u32> = self.coop.peers().into_iter().filter(|p| *p != from).collect();
                for peer in others {
                    self.coop.send(peer, &forward);
                }
            }
            Message::WireProfile { profile } if !self.is_host() => {
                // the host's relayed profile: `peer_id` is the runner it describes (HOST_PEER_ID for the host)
                let Some(peer) = profile.get("peer_id").and_then(Value::as_i64).and_then(|p| u32::try_from(p).ok()) else {
                    return;
                };
                if peer == self.coop.link.session() {
                    return;
                }
                self.skins.insert(peer, profile_skin(&profile));
                let name = clean_name(profile.get("name").and_then(Value::as_str).unwrap_or_default());
                self.names.insert(peer, name);
            }
            Message::PeerLeave if self.is_host() => {
                self.remotes.remove(&from);
                self.skins.remove(&from);
                if let Some(guest) = self.guests.remove(&from) {
                    out.push(NetEvent::GuestLeft(guest.name));
                    self.send_room_and_roster();
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
            Message::Room { seed, rest, .. } if !self.is_host() => {
                self.room_seed = seed.clone();
                self.host_name = rest.get("host").and_then(Value::as_str).unwrap_or_default().to_string();
                self.players = rest
                    .get("players")
                    .and_then(Value::as_array)
                    .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                if let Some(d) = rest.get("difficulty").and_then(Value::as_str) {
                    self.difficulty = difficulty_from_name(d).name().to_string();
                }
                out.push(NetEvent::Seed(seed));
            }
            Message::Packs { packs } if !self.is_host() => {
                // the host's room mod packs: an event only when the list changes (the host resends it with the roster)
                let listed: Vec<String> = packs
                    .get("ids")
                    .and_then(Value::as_array)
                    .map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let mods = clean_mod_ids(&listed);
                if mods != self.mods {
                    self.mods = mods.clone();
                    out.push(NetEvent::Packs(mods));
                }
            }
            // the host ended the route (Godot `_receive_end`): the run is over here too, the room stays open
            Message::End { reason } if !self.is_host() => out.push(NetEvent::RouteEnded(reason)),
            // the host's hunters and world events (Godot `_hunters` and `_event`): display only, a guest never sends them
            Message::Hunters { data } | Message::HuntersLarge { data } if !self.is_host() => {
                out.push(NetEvent::Hunters(data));
            }
            Message::Event { event } if !self.is_host() => out.push(NetEvent::WorldEvent(event)),
            Message::Start { seed, rest, .. } if !self.is_host() => {
                let difficulty = rest.get("difficulty").and_then(Value::as_str).unwrap_or("STANDARD").to_string();
                self.difficulty = difficulty_from_name(&difficulty).name().to_string();
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

    /// Host: sends a guest that just registered every runner's profile in the room, the host's first, then each
    /// other guest's. Those that have not sent a profile yet are DEFAULT (their own profile follows).
    fn send_profiles_to(&mut self, to: u32) {
        let mut profiles = vec![profile_message(HOST_PEER_ID, &self.name, &self.skin)];
        for (peer, guest) in &self.guests {
            if *peer != to {
                let skin = self.skins.get(peer).map_or(SKIN_DEFAULT, String::as_str);
                profiles.push(profile_message(*peer, &guest.name, skin));
            }
        }
        for profile in &profiles {
            self.coop.send(to, profile);
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
        // a stale runner is not drawn (remotes_at checks REMOTE_TIMEOUT) but its state stays for the PLAYERS list
        self.remotes.retain(|_, r| now.duration_since(r.last_seen) < ROSTER_KEEP);
        if self.is_host() {
            let gone: Vec<u32> = self
                .guests
                .iter()
                .filter(|(_, g)| now.duration_since(g.last_seen) >= GUEST_IDLE)
                .map(|(peer, _)| *peer)
                .collect();
            for peer in gone {
                self.remotes.remove(&peer);
                self.skins.remove(&peer);
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

/// Most mod pack ids a room carries (the reserved-room settings keep at most this many).
pub const MAX_MODS: usize = 32;

/// The room's mod pack ids, cleaned: a pack id is 3 to 32 lower-case letters, digits or underscores that starts with a
/// letter or digit (the directory's rule). Others are dropped, repeats too, and the list is capped at [`MAX_MODS`].
pub fn clean_mod_ids(ids: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for id in ids {
        let b = id.as_bytes();
        let valid = (3..=32).contains(&b.len())
            && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
            && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_');
        if valid && !kept.contains(id) && kept.len() < MAX_MODS {
            kept.push(id.clone());
        }
    }
    kept
}

/// True for a wire skin id: the 64 lowercase hex characters of a SHA-256 (see `shared::skins::chosen_skin`).
pub fn is_skin_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A skin id as the wire and the settings keep it: letters, digits and underscore, upper-cased, at most 24
/// characters. Empty becomes DEFAULT.
pub fn clean_skin(text: &str) -> String {
    // a wire id (the SHA-256 of a skin PNG) is kept as it is
    if is_skin_hash(text) {
        return text.to_string();
    }
    let kept: String = text.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(MAX_SKIN).collect();
    let id = kept.to_ascii_uppercase();
    if id.is_empty() {
        SKIN_DEFAULT.into()
    } else {
        id
    }
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
    LOBBY.iter().copied().find(|d| d.name() == name).unwrap_or(Difficulty::Standard)
}

/// The difficulty a route command may set. Only a host (or solo) runner's command does: a guest's never changes the
/// run's difficulty, so the guest keeps the host's (Godot `room_state.gd` sets it on the host only).
pub fn commanded_difficulty(guest: bool, requested: Option<Difficulty>) -> Option<Difficulty> {
    if guest {
        None
    } else {
        requested
    }
}

/// The figure colour of a runner: its skin's palette by peer id. An id no skin has is drawn as DEFAULT.
pub fn skin_colour(skin: &str, peer: u32) -> Color {
    let palette = SKINS.iter().find(|(id, _)| *id == skin).unwrap_or(&SKINS[0]).1;
    palette[(peer as usize) % palette.len()]
}

/// Draws the remote runners in the 3D view (the figures are drawn by `net::runner_figure`). Draws nothing when `remotes`
/// is empty, so a single-player frame is unchanged.
pub fn draw_remotes(g: &Game, a: &Assets, mat: &Material, rt: &RenderTarget, remotes: &[RemoteView]) {
    if remotes.is_empty() {
        return;
    }
    crate::net::runner_figure::draw_runners(g, a, mat, rt, remotes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::relay::{decode, encode, frame};
    use std::net::UdpSocket;

    fn sample(seq: u32, x: f32, y: f32) -> Sample {
        Sample {
            seq,
            depth: 1,
            x,
            y,
            yaw: 0.5,
            moving: true,
            sprinting: false,
            crouching: true,
            spectating: false,
            game_over: false,
            escape_active: false,
        }
    }

    #[test]
    fn sample_json_round_trips_and_ignores_keep_alives() {
        let s = Sample {
            seq: 9,
            depth: -2,
            x: 12.5,
            y: -3.25,
            yaw: 1.57,
            moving: true,
            sprinting: true,
            crouching: false,
            spectating: false,
            game_over: true,
            escape_active: true,
        };
        assert_eq!(Sample::from_json(&s.to_json()), Some(s));
        assert_eq!(Sample::from_json(&Value::Null), None, "a keep-alive is not a sample");
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": "far", "y": 0})), None);
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": 1e39, "y": 0})), None, "infinity is refused");
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": 0.0, "y": 0.0}))
            .map(|s| (s.yaw, s.moving)), Some((0.0, false)), "missing flags read as false");
    }

    #[test]
    fn a_sample_is_absolute_and_comes_back_into_each_window() {
        let absolute = to_absolute((3.0, -2.5), (100, -40));
        assert_eq!(absolute, (103.0, -42.5));
        assert_eq!(to_window(absolute, (100, -40)), (3.0, -2.5), "the sender's own window");
        assert_eq!(to_window(absolute, (0, 0)), (103.0, -42.5), "a finite floor's origin is (0, 0)");
        assert_eq!(to_window(absolute, (103, -40)), (0.0, -2.5), "a window moved east sees the runner nearer its west edge");
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
    fn nightmare_survives_the_room_wire_round_trip() {
        let wire = Difficulty::Nightmare.name();
        assert_eq!(wire, "NIGHTMARE");
        assert!(difficulty_from_name(wire) == Difficulty::Nightmare);
        assert!(difficulty_from_name(Difficulty::Standard.name()) == Difficulty::Standard);
        assert!(difficulty_from_name(Difficulty::Hard.name()) == Difficulty::Hard);
    }

    #[test]
    fn guest_command_difficulty_is_ignored_and_host_command_difficulty_applies() {
        // a guest's restart or start carries no difficulty of its own: the host's stays
        assert_eq!(commanded_difficulty(true, Some(Difficulty::Hard)), None);
        assert_eq!(commanded_difficulty(true, Some(Difficulty::Relaxed)), None);
        assert_eq!(commanded_difficulty(true, None), None);
        // the host (and a solo runner) sets it
        assert_eq!(commanded_difficulty(false, Some(Difficulty::Hard)), Some(Difficulty::Hard));
        assert_eq!(commanded_difficulty(false, None), None);
    }

    #[test]
    fn the_wire_profile_carries_the_skin_id_and_a_peer_reads_it_back() {
        let hash = "c".repeat(64);
        match profile_message(3, "RUNNER", &hash) {
            Message::WireProfile { profile } => {
                assert_eq!(profile.get("skin").and_then(Value::as_str), Some(hash.as_str()));
                assert_eq!(profile_skin(&profile), hash);
                assert_eq!(profile_skin(&Map::new()), SKIN_DEFAULT, "a profile without a skin is DEFAULT");
            }
            _ => panic!("a profile message is a WireProfile"),
        }
    }

    #[test]
    fn a_host_publishes_its_skin_only_when_the_website_holds_it() {
        let hash = "b".repeat(64);
        assert_eq!(published_skin(&hash, &[hash.clone()]), hash);
        assert_eq!(published_skin(&hash, &[]), SKIN_DEFAULT, "an unuploaded skin is not published");
        assert_eq!(published_skin(SKIN_DEFAULT, &[hash.clone()]), SKIN_DEFAULT);
        assert_eq!(published_skin("night-owl", &["NIGHTOWL".into()]), SKIN_DEFAULT, "only a skin hash is published");
    }

    #[test]
    fn skin_ids_are_cleaned_and_unknown_skins_draw_as_default() {
        assert_eq!(clean_skin(""), SKIN_DEFAULT);
        assert_eq!(clean_skin("  ??  "), SKIN_DEFAULT);
        assert_eq!(clean_skin("night-owl!"), "NIGHTOWL");
        assert_eq!(clean_skin("abcdefghijklmnopqrstuvwxyz"), "ABCDEFGHIJKLMNOPQRSTUVWX");
        let hash = "a".repeat(63) + "9";
        assert!(is_skin_hash(&hash));
        assert_eq!(clean_skin(&hash), hash, "a wire id survives cleaning");
        assert!(!is_skin_hash(&hash.to_uppercase()));
        for peer in 0..8 {
            assert_eq!(skin_colour("NO_SUCH_SKIN", peer), skin_colour(SKIN_DEFAULT, peer));
        }
        assert_eq!(skin_colour(SKIN_DEFAULT, 1), PEER_COLOURS[1]);
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
            "DAWN".into(),
        );
        let mut guest = NetSession::new(
            cfg,
            "ABCDE".into(),
            Role::Guest,
            CoopSession::new(RelayLink::guest("ABCDE", "127.0.0.1", port, "").unwrap()),
            String::new(),
            String::new(),
            "GUEST".into(),
            "NIGHT".into(),
        );
        host.set_room_mods(&["neon_halls".to_string(), "BAD".to_string()]);
        host.start_run("ROOM-ABCDE-0123", "STANDARD");
        let (mut host_addr, mut guest_addr) = (None, None);
        let mut buf = [0u8; 2048];
        let mut events = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        let (host_sample, guest_sample) = (sample(0, 3.0, -2.0), sample(0, 7.5, 1.25));
        let mut done = (false, false, false);
        let mut packs_seen = false;
        while Instant::now() < deadline && !(done.0 && done.1 && done.2 && packs_seen) {
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
            done.0 = host.remotes_at(1, (0, 0)).iter().any(|r| r.peer == 42 && (r.x - 7.5).abs() < 1e-4 && (r.y - 1.25).abs() < 1e-4 && r.skin == "NIGHT");
            done.1 = guest.remotes_at(1, (0, 0)).iter().any(|r| r.peer == HOST_PEER_ID && (r.x - 3.0).abs() < 1e-4 && r.skin == "DAWN");
            done.2 = events.contains(&NetEvent::Start { seed: "ROOM-ABCDE-0123".into(), difficulty: "STANDARD".into() })
                || guest.room_seed() == "ROOM-ABCDE-0123";
            packs_seen = events.contains(&NetEvent::Packs(vec!["neon_halls".into()]));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(done.0, "the host sees the guest's sample and skin");
        assert!(done.1, "the guest sees the host's sample and skin");
        assert!(done.2, "the guest receives the route seed");
        assert!(packs_seen, "the guest receives the room's mod packs, without the id that is not a pack id");
        assert_eq!(guest.room_mods(), ["neon_halls".to_string()]);
        assert!(events.contains(&NetEvent::GuestJoined("GUEST".into())));
        assert!(host.is_host() && !guest.is_host());
    }

    #[test]
    fn a_guest_takes_the_hosts_hunter_frames_and_world_events_and_never_sends_them() {
        let now = Instant::now();
        let mut guest = offline_session(false, "GUEST");
        let frame = vec![json!({"index": 0, "id": "aki", "state": "CHASE", "position": [1.0, 2.0]})];
        let mut out = Vec::new();
        guest.on_message(HOST_PEER_ID, Message::Hunters { data: frame.clone() }, now, &mut out);
        guest.on_message(HOST_PEER_ID, Message::HuntersLarge { data: frame.clone() }, now, &mut out);
        assert_eq!(out, vec![NetEvent::Hunters(frame.clone()), NetEvent::Hunters(frame)]);
        let mut event = Map::new();
        event.insert("what".into(), json!("hunter_grab"));
        event.insert("phase".into(), json!("start"));
        event.insert("hunter_id".into(), json!("aki"));
        out.clear();
        guest.on_message(HOST_PEER_ID, Message::Event { event: event.clone() }, now, &mut out);
        assert_eq!(out, vec![NetEvent::WorldEvent(event)]);
        // the host's own frames are not applied back to it: a host ignores a relayed frame
        let mut host = offline_session(true, "HOST");
        out.clear();
        host.on_message(2, Message::Hunters { data: vec![] }, now, &mut out);
        assert!(out.is_empty());
        // a guest's send of a frame does nothing (the host is the only sender)
        guest.send_hunter_frame(vec![json!({"index": 0})], Difficulty::Standard, None, now);
    }

    #[test]
    fn the_hosts_difficulty_reaches_the_guest_and_a_guest_cannot_set_it() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut guest = offline_session(false, "GUEST");
        assert_eq!(host.room_difficulty(), "STANDARD");
        host.set_room_difficulty(Difficulty::Hard);
        assert_eq!(host.room_difficulty(), "HARD");
        // a guest's call does nothing: the guest keeps the host's difficulty
        guest.set_room_difficulty(Difficulty::Nightmare);
        assert_eq!(guest.room_difficulty(), "STANDARD");
        // the host's room message carries the difficulty, and the guest takes it
        let mut rest = Map::new();
        rest.insert("difficulty".into(), json!(host.room_difficulty()));
        rest.insert("host".into(), json!("HOST"));
        let room = Message::Room { code: "ABCDE".into(), seed: "ROOM-ABCDE-0123".into(), mode: 0, scale: 0, rest };
        let mut out = Vec::new();
        guest.on_message(HOST_PEER_ID, room, now, &mut out);
        assert_eq!(guest.room_difficulty(), "HARD");
        assert_eq!(difficulty_from_name(guest.room_difficulty()), Difficulty::Hard);
    }

    #[test]
    fn a_guest_follows_each_difficulty_the_host_sets_and_an_unknown_name_is_standard() {
        let now = Instant::now();
        let mut guest = offline_session(false, "GUEST");
        for (name, want) in [("RELAXED", Difficulty::Relaxed), ("NIGHTMARE", Difficulty::Nightmare), ("NOPE", Difficulty::Standard)] {
            let mut rest = Map::new();
            rest.insert("difficulty".into(), json!(name));
            let room = Message::Room { code: "ABCDE".into(), seed: "ROOM-ABCDE-0123".into(), mode: 0, scale: 0, rest };
            let mut out = Vec::new();
            guest.on_message(HOST_PEER_ID, room, now, &mut out);
            assert_eq!(difficulty_from_name(guest.room_difficulty()), want, "{name}");
        }
    }

    /// A session on a discarded relay address: only its in-memory state is used.
    fn offline_session(host: bool, name: &str) -> NetSession {
        let (role, link, token) = if host {
            (Role::Host, RelayLink::host("ABCDE", "tok", "127.0.0.1", 9).unwrap(), "tok")
        } else {
            (Role::Guest, RelayLink::guest("ABCDE", "127.0.0.1", 9, "").unwrap(), "")
        };
        NetSession::new(
            BackendConfig::loopback(18105),
            "ABCDE".into(),
            role,
            CoopSession::new(link),
            token.into(),
            String::new(),
            name.into(),
            "DAWN".into(),
        )
    }

    #[test]
    fn the_roster_lists_the_guests_for_the_host_and_the_host_first_for_a_guest() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        host.guests.insert(42, Guest { name: "NIGHTWALK".into(), last_seen: now });
        host.guests.insert(7, Guest { name: "QUIET".into(), last_seen: now });
        host.remotes.insert(42, Remote { sample: sample(1, 0.0, 0.0), last_seen: now });
        let roster = host.roster();
        assert_eq!(roster.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["QUIET", "NIGHTWALK"]);
        assert!(!roster[0].sending && roster[0].depth.is_none(), "a guest with no fresh sample is not sending");
        assert!(roster[0].state.is_none(), "a guest with no sample yet has no state (it just joined)");
        assert!(roster[1].sending && roster[1].depth == Some(1), "a guest sending samples is on its floor");
        assert!(roster[1].state.is_some_and(|s| s.depth == 1 && s.quiet < QUIET_DISCONNECT), "its state is fresh");
        assert!(roster.iter().all(|e| e.ping.is_none() && !e.host), "no round trip is measured yet");
        host.coop.close();

        let mut guest = offline_session(false, "GUEST");
        guest.host_name = "HOST".into();
        guest.names.insert(9, "NEON".into());
        let silent = guest.roster();
        assert_eq!(silent.len(), 1, "a guest that is not sending is not listed");
        assert!(silent[0].host && silent[0].name == "HOST" && !silent[0].sending, "the host is listed first, not sending");
        guest.remotes.insert(HOST_PEER_ID, Remote { sample: sample(1, 3.0, -2.0), last_seen: now });
        guest.remotes.insert(9, Remote { sample: sample(1, 5.0, 5.0), last_seen: now });
        let roster = guest.roster();
        assert_eq!(roster.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["HOST", "NEON"]);
        assert!(roster[0].host && roster[0].sending, "the host sends samples");
        // a runner quiet for longer than QUIET_DISCONNECT stays listed, with its quiet time for the DISCONNECTED status
        let long_ago = now.checked_sub(QUIET_DISCONNECT + Duration::from_secs(1)).unwrap_or(now);
        guest.remotes.insert(9, Remote { sample: sample(2, 5.0, 5.0), last_seen: long_ago });
        let roster = guest.roster();
        let quiet = roster.iter().find(|e| e.peer == 9).and_then(|e| e.state).map(|s| s.quiet);
        assert!(quiet.is_some_and(|q| q > QUIET_DISCONNECT), "a silent runner is still listed, quiet for {quiet:?}");
        assert!(!roster.iter().any(|e| e.peer == 9 && e.sending), "and it is not sending");
        guest.coop.close();
    }

    #[test]
    fn room_mod_ids_keep_only_pack_ids_without_repeats() {
        let ids: Vec<String> = ["neon_halls", "Bad-Id", "ab", "neon_halls", "2nd_pack", "x".repeat(33).as_str()]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(clean_mod_ids(&ids), ["neon_halls".to_string(), "2nd_pack".to_string()]);
        let many: Vec<String> = (0..40).map(|i| format!("pack_{i:02}")).collect();
        assert_eq!(clean_mod_ids(&many).len(), MAX_MODS);
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
        let mut host = host_room(cfg.clone(), route.clone(), "HOST".into(), SKIN_DEFAULT.into()).wait().expect("the host registers a room");
        assert!(host.is_host());
        let code = host.code.clone();
        assert!(api::valid_room_code(&code), "{code}");
        host.start_run(&route, "STANDARD");
        let mut guest = join_room(cfg.clone(), code.clone(), "GUEST".into(), SKIN_DEFAULT.into(), String::new(), false).wait().expect("the guest joins by code");
        assert!(!guest.is_host());
        let unknown = join_room(cfg.clone(), "AAAAA".into(), "X".into(), SKIN_DEFAULT.into(), String::new(), false).wait().err().and_then(|e| e.status());
        assert_eq!(unknown, Some(404), "an unknown code is refused by the directory");

        let (host_sample, guest_sample) = (sample(0, 3.0, -2.0), sample(0, 7.5, 1.25));
        let mut guest_events = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(12);
        let (mut host_sees, mut guest_sees, mut seed_seen) = (false, false, false);
        while Instant::now() < deadline && !(host_sees && guest_sees && seed_seen) {
            host.tick(Some(host_sample));
            guest_events.extend(guest.tick(Some(guest_sample)));
            host_sees = host.remotes_at(1, (0, 0)).iter().any(|r| (r.x - 7.5).abs() < 1e-4);
            guest_sees = guest.remotes_at(1, (0, 0)).iter().any(|r| (r.x - 3.0).abs() < 1e-4);
            seed_seen = guest.room_seed() == route || guest_events.contains(&NetEvent::Start { seed: route.clone(), difficulty: "STANDARD".into() });
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host_sees, "the host sees the guest on the live relay");
        assert!(guest_sees, "the guest sees the host on the live relay");
        assert!(seed_seen, "the guest gets the route seed and the start");
        // the lobby roster: the host lists the guest, and the guest lists the host
        assert!(host.guest_names().iter().any(|n| n == "GUEST"), "the host lists the guest: {:?}", host.guest_names());
        assert!(guest.guest_names().iter().any(|n| n == "HOST"), "the guest lists the host: {:?}", guest.guest_names());
        guest.leave();
        host.leave();
    }
}

#[cfg(test)]
mod admin_join_tests {
    use super::*;

    fn not_found() -> NetError {
        NetError::Http { status: 404, code: String::new(), error: "ROOM NOT FOUND".into(), body: Value::Null }
    }

    #[test]
    fn an_owners_miss_reserves_the_typed_code() {
        assert!(matches!(lookup_outcome(Err(not_found()), true, "tok"), Ok(JoinStep::Reserve)));
    }

    #[test]
    fn a_guests_miss_is_not_reserved() {
        assert_eq!(lookup_outcome(Err(not_found()), false, "tok").err().and_then(|e| e.status()), Some(404));
        assert_eq!(lookup_outcome(Err(not_found()), true, "").err().and_then(|e| e.status()), Some(404), "no session, no reserve");
    }

    #[test]
    fn a_live_code_is_joined_not_recreated() {
        assert!(matches!(lookup_outcome(Ok(api::RoomEndpoint::default()), true, "tok"), Ok(JoinStep::Found(_))));
        assert!(matches!(lookup_outcome(Ok(api::RoomEndpoint::default()), false, ""), Ok(JoinStep::Found(_))));
    }

    #[test]
    fn an_owners_reserved_code_is_reserved_and_hosted() {
        let reserved = NetError::Http { status: 403, code: "ROOM_RESERVED".into(), error: "RESERVED".into(), body: Value::Null };
        assert!(matches!(lookup_outcome(Err(reserved), true, "tok"), Ok(JoinStep::Reserve)));
        let reserved = NetError::Http { status: 403, code: "ROOM_RESERVED".into(), error: "RESERVED".into(), body: Value::Null };
        assert_eq!(lookup_outcome(Err(reserved), false, "tok").err().and_then(|e| e.status()), Some(403), "a guest keeps the access flow");
    }

    #[test]
    fn an_owners_other_failure_is_not_reserved() {
        let down = NetError::Http { status: 500, code: String::new(), error: "DOWN".into(), body: Value::Null };
        assert_eq!(lookup_outcome(Err(down), true, "tok").err().and_then(|e| e.status()), Some(500));
    }
}

/// Owner route commands (RESTART ROUTE, END ROUTE): the parse, the seed a restart runs on, the ack cursors and the
/// guest's end. Godot: `game/autoload/room_state.gd` `_on_reserved_commands` and `carry_out_route_command`, and
/// `game/autoload/backend_client.gd` `_take_reserved`, `ack_reserved_command`, `_send_heartbeat`.
#[cfg(test)]
mod owner_route_tests {
    use super::*;
    use std::net::UdpSocket;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().expect("an object").clone()
    }

    fn view(commands: Value) -> ReservedHostView {
        let o = json!({"commands": commands, "commandSeq": 4});
        ReservedHostView::from_json(o.as_object().expect("an object"))
    }

    #[test]
    fn owner_route_commands_parse_as_godot_reads_them() {
        assert_eq!(
            RouteCommand::parse("restart", &args(json!({"sameSeed": true, "settings": {"seed": "  ROOM-AB-1  ", "difficulty": 2}}))),
            Some(RouteCommand::Restart { same_seed: true, seed: "ROOM-AB-1".into(), difficulty: Some(Difficulty::Hard) })
        );
        assert_eq!(
            RouteCommand::parse("restart", &args(json!({}))),
            Some(RouteCommand::Restart { same_seed: false, seed: String::new(), difficulty: None })
        );
        // `int(settings.difficulty)`: 0 RELAXED, 1 STANDARD; 3 is NIGHTMARE, which has no port here, and anything else is STANDARD
        for (index, want) in [(0, Difficulty::Relaxed), (1, Difficulty::Standard), (3, Difficulty::Standard), (-1, Difficulty::Standard)] {
            assert_eq!(
                RouteCommand::parse("restart", &args(json!({"settings": {"difficulty": index}}))),
                Some(RouteCommand::Restart { same_seed: false, seed: String::new(), difficulty: Some(want) })
            );
        }
        assert_eq!(RouteCommand::parse("end", &args(json!({}))), Some(RouteCommand::End));
        for other in ["kick", "settings", "transfer", "nothing"] {
            assert_eq!(RouteCommand::parse(other, &args(json!({}))), None, "{other} is not a route command");
        }
    }

    #[test]
    fn owner_restart_keeps_the_seed_or_takes_the_stored_or_a_fresh_one() {
        assert_eq!(RouteCommand::restart_seed(true, "STORED", "ROOM-NOW", "FRESH"), "ROOM-NOW");
        assert_eq!(RouteCommand::restart_seed(false, "STORED", "ROOM-NOW", "FRESH"), "STORED");
        assert_eq!(RouteCommand::restart_seed(false, "", "ROOM-NOW", "FRESH"), "FRESH");
    }

    #[test]
    fn owner_cursor_hands_each_command_on_once_and_acks_it() {
        let first = view(json!([
            {"seq": 1, "command": "restart", "args": {"sameSeed": false}},
            {"seq": 2, "command": "kick", "args": {"target": "GUEST"}},
            {"seq": 3, "command": "end", "args": {}},
        ]));
        let mut cursor = OwnerCursor::default();
        assert_eq!(
            cursor.take(&first),
            vec![
                NetEvent::RouteCommand(RouteCommand::Restart { same_seed: false, seed: String::new(), difficulty: None }),
                NetEvent::Problem("OWNER COMMAND KICK IS NOT IN THIS BUILD".into()),
                NetEvent::RouteCommand(RouteCommand::End),
            ]
        );
        assert_eq!(cursor, OwnerCursor { seen: 3, ack: 3 });
        // the same answer again hands nothing on and moves nothing
        assert!(cursor.take(&first).is_empty());
        assert_eq!(cursor, OwnerCursor { seen: 3, ack: 3 });
        // a later command is handed on and acked
        let next = view(json!([{"seq": 4, "command": "end", "args": {}}]));
        assert_eq!(cursor.take(&next), vec![NetEvent::RouteCommand(RouteCommand::End)]);
        assert_eq!(cursor, OwnerCursor { seen: 4, ack: 4 });
        // a fresh registration starts the cursors again, so what it lists is handed on again (Godot fresh=true)
        cursor = OwnerCursor::default();
        assert_eq!(cursor.take(&next), vec![NetEvent::RouteCommand(RouteCommand::End)]);
    }

    #[test]
    fn host_heartbeat_sends_the_cursors_and_a_guest_applies_the_host_end() {
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        relay.set_nonblocking(true).unwrap();
        let port = relay.local_addr().unwrap().port();
        let mut host = NetSession::new(
            BackendConfig::loopback(18106),
            "ABCDE".into(),
            Role::Host,
            CoopSession::new(RelayLink::host("ABCDE", "tok", "127.0.0.1", port).unwrap()),
            "tok".into(),
            "ROOM-ABCDE-0123".into(),
            "HOST".into(),
            "DAWN".into(),
        );
        host.owner.take(&view(json!([{"seq": 7, "command": "end", "args": {}}])));
        let beat = host.beat();
        assert_eq!((beat.command_since, beat.command_ack), (7, 7));

        let mut guest = NetSession::new(
            BackendConfig::loopback(18106),
            "ABCDE".into(),
            Role::Guest,
            CoopSession::new(RelayLink::guest("ABCDE", "127.0.0.1", port, "").unwrap()),
            String::new(),
            String::new(),
            "GUEST".into(),
            "NIGHT".into(),
        );
        let mut out = Vec::new();
        guest.on_message(HOST_PEER_ID, Message::End { reason: "ROUTE RESTARTING".into() }, Instant::now(), &mut out);
        assert_eq!(out, vec![NetEvent::RouteEnded("ROUTE RESTARTING".into())]);
    }
}
