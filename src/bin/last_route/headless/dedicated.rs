//! `--dedicated-host`: a room hosted by a server process. The room is the headless [`Sim`] running the Infinite route with
//! NO local runner (`Game::no_local_runner`), hosting a WebSocket room on the relay of the backend; every player,
//! the room's owner included, joins it as a guest. No client is ever the host. The hunters hunt the guests
//! (game/remote_targets.rs), the room keeps simulating while guests come and go, and it idles (almost no CPU) while nobody
//! is in it.
//!
//! This is the Rust counterpart of Godot's `--dedicated-host`, started the same way (the Node region agent,
//! `backend/region_agent.mjs`, passes `--headless -- --dedicated-host CODE --port P --idle-minutes M [--seed S]
//! [--difficulty D] --ws-rooms [--backend=URL]` and `LASTROUTE_SPAWN_TICKET` / `LASTROUTE_REGION` in the environment).
//! Dispatched before `--headless` in [`super::dispatch`], because the agent's command line carries both.
//!
//! # Flags and environment
//!
//! | flag | environment | meaning |
//! |---|---|---|
//! | `--dedicated-host CODE` | | the room code, 5 characters A-Z / 0-9 (required) |
//! | `--host-token T` | `LASTROUTE_HOST_TOKEN` | the host token the directory issued for the room. Prefer the environment: a command line shows in `ps`. |
//! | | `LASTROUTE_SPAWN_TICKET` | a spawn ticket instead of a token (below) |
//! | `--region R` | `LASTROUTE_REGION` | the region name, sent with the registration |
//! | `--backend=URL` | | the directory (policy of `net/config.rs`: the primary origin, or a loopback / test origin) |
//! | `--relay URL` | `LASTROUTE_RELAY_URL` | the relay address when it is not the backend's: an origin or `ws://host:port[/api/ws]` |
//! | `--seed TEXT` | | the room seed text (default: a fresh one) |
//! | `--difficulty D` | | `relaxed`, `standard`, `hard`, `nightmare` or Godot's index 0 to 3 (default standard) |
//! | `--idle-minutes M` | | exit after the room has been empty this long (default 10, at least 0.5) |
//! | `--start-relays N` | | the relays the room counts as recovered, which sets the hunter population (default: the difficulty's activation count, so the route starts with one hunter) |
//! | `--reconnect-seconds S` | | how long to keep trying to reopen a lost relay link (default 30, 0 never) |
//! | `--name TEXT` | | what the guests see as the host (default SERVER) |
//! | `--no-heartbeat` | | do not talk to the directory at all after the start (standalone and test hubs) |
//! | `--port N`, `--ws-rooms`, `--mode N`, `--scale N` | | accepted and ignored (Godot's ENet port and map options; every Rust room is a WebSocket room) |
//!
//! # The token
//!
//! The process registers nothing of its own accord. It is started with what the server already issued:
//! * a host token (`--host-token` / `LASTROUTE_HOST_TOKEN`): it connects to the relay as the host with it and registers
//!   nothing; or
//! * a spawn ticket (`LASTROUTE_SPAWN_TICKET`, what the Node region agent passes): the one `POST
//!   /api/enet/rooms/register` that exchanges the ticket for the host token (transport `ws`), the only way the Node
//!   directory mints one for a hosted code. Nothing else is registered, and the ticket is spent by that call.
//!
//! # What the loop does, 60 times a second while a guest is in
//!
//! The relay session ticks; the guests' samples become targets and the party anchor is chosen
//! ([`crate::game::remote_targets::choose_anchor`]); the guests' taser, escape and valve reports are applied; the sim steps
//! one frame; the hunters' frame, the world events and the catches go out. A catch tells the guest (`_player_event`
//! `capture`) and opens the net layer's wait for its `escaped` / `caught` report.
//!
//! # The owner
//!
//! The room's owner joins as a guest like everybody else and runs the room with owner commands (`net/hosted.rs`): START,
//! END, RESTART, KICK and DIFFICULTY. A command arrives as a `_owner_command` message with the directory's owner ticket.
//! The room never acts on the guest's word: it asks the directory to verify the ticket with the room's own host token
//! (`POST /api/enet/owner-tickets/verify`), on a worker thread, and acts only on a yes. A refusal goes back to the guest as
//! `_owner_refused` with the directory's reason. The directory's check is once per (ticket, action). While the route is ended
//! the room steps nothing and tells no guest to enter; START or RESTART runs it again.
//!
//! # Limits
//!
//! * `ws://` only (see `net/ws_native.rs`): the process talks to the hub on loopback on the same box.
//! * No signal handling (no dependency): SIGTERM ends the process at once, the relay drops the room, the directory's
//!   entry lapses on its TTL. The idle exit unregisters properly (token path).
//! * The world's shared state (relays, pickups, doors) is still local to each guest; the room's hunter population is set at
//!   the start (`--start-relays`) and does not grow. One window: only guests inside the party anchor's window are hunted.
//! * A guest's flashlight is not on the wire, so hunters see guests unlit.

use super::sim::{parse_difficulty, Sim};
use super::Args;
use crate::config::difficulty::LOBBY;
use crate::config::Difficulty;
use crate::game::remote_targets::{choose_anchor, AnchorCandidate, GuestCatch, RemoteTarget};
use crate::game::Game;
use crate::net::api::{self, RegisterRoom};
use crate::net::config::BackendConfig;
use crate::net::hosted::{self, OwnerAction};
use crate::net::Pending;
use crate::net::room_link::RoomLink;
use crate::net::world_sync::{GuestPose, NetEvent, NetSession};
use crate::net::ws_native::NativeWsLink;
use crate::shared::clock::Instant;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;

/// What the guests see as the host of a dedicated room (Godot `DEDICATED_NAME`).
pub const DEFAULT_NAME: &str = "SERVER";
/// The fixed step of the room's simulation.
pub const STEP: f32 = 1.0 / 60.0;
/// At most this many steps are run to catch up after a stall; the rest of the backlog is dropped.
pub const MAX_CATCH_UP_STEPS: u32 = 5;
/// The loop's sleep with a guest in the room, and with none (the room only has to notice a join).
pub const BUSY_SLEEP: Duration = Duration::from_millis(2);
pub const IDLE_SLEEP: Duration = Duration::from_millis(40);
/// The Godot agent's default `--idle-minutes`, and the smallest value taken.
pub const DEFAULT_IDLE_MINUTES: f64 = 10.0;
pub const MIN_IDLE_MINUTES: f64 = 0.5;
/// How long a lost relay link is retried, and how often.
pub const DEFAULT_RECONNECT_SECONDS: f64 = 30.0;
pub const RECONNECT_EVERY: Duration = Duration::from_secs(2);

/// Why [`parse_options`] refused the command line.
pub type OptionError = String;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub code: String,
    pub token: String,
    pub spawn_ticket: String,
    pub region: String,
    /// Where the relay is: an origin or a WebSocket address (`ws_native::relay_address` makes the final address).
    pub relay: String,
    /// The room seed text.
    pub room_text: String,
    pub difficulty: Difficulty,
    pub idle_minutes: f64,
    pub start_relays: Option<i32>,
    pub reconnect_seconds: f64,
    pub name: String,
    /// Whether the directory is told how the room is (heartbeat).
    pub heartbeat: bool,
}

fn difficulty_arg(text: &str) -> Option<Difficulty> {
    parse_difficulty(text).or_else(|| text.trim().parse::<usize>().ok().and_then(|i| LOBBY.get(i).copied()))
}

/// Reads the command line and the environment (`env` returns a non-empty value or None). `cfg` is the backend the
/// command line named (its base URL is the relay unless `--relay` says otherwise).
pub fn parse_options(a: &Args, env: &dyn Fn(&str) -> Option<String>, cfg: &BackendConfig) -> Result<Options, OptionError> {
    let code = a.value("dedicated-host").unwrap_or_default().trim().to_uppercase();
    if !api::valid_join_code(&code) {
        return Err(format!("--dedicated-host needs a room code of 5 letters or digits, not \"{code}\""));
    }
    let number = |name: &str, default: f64, min: f64| -> Result<f64, OptionError> {
        match a.value(name) {
            None => Ok(default),
            Some(v) => v.trim().parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n.max(min)).ok_or_else(|| format!("--{name} needs a number, not \"{v}\"")),
        }
    };
    let difficulty = match a.value("difficulty") {
        None => Difficulty::Standard,
        Some(v) => difficulty_arg(&v).ok_or_else(|| format!("--difficulty needs relaxed, standard, hard, nightmare or 0 to 3, not \"{v}\""))?,
    };
    let start_relays = match a.value("start-relays") {
        None => None,
        Some(v) => Some(v.trim().parse::<i32>().ok().filter(|n| (0..=999).contains(n)).ok_or_else(|| format!("--start-relays needs 0 to 999, not \"{v}\""))?),
    };
    let relay = a.value("relay").or_else(|| env("LASTROUTE_RELAY_URL")).unwrap_or_else(|| cfg.base_url.clone());
    let name: String = a.value("name").unwrap_or_else(|| DEFAULT_NAME.into()).trim().chars().take(24).collect();
    Ok(Options {
        code,
        token: a.value("host-token").or_else(|| env("LASTROUTE_HOST_TOKEN")).unwrap_or_default(),
        spawn_ticket: env("LASTROUTE_SPAWN_TICKET").unwrap_or_default(),
        region: a.value("region").or_else(|| env("LASTROUTE_REGION")).unwrap_or_default(),
        relay,
        room_text: a.value("seed").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(crate::world::route::new_room_text),
        difficulty,
        idle_minutes: number("idle-minutes", DEFAULT_IDLE_MINUTES, MIN_IDLE_MINUTES)?,
        start_relays,
        reconnect_seconds: number("reconnect-seconds", DEFAULT_RECONNECT_SECONDS, 0.0)?,
        name: if name.is_empty() { DEFAULT_NAME.into() } else { name },
        heartbeat: !a.flag("no-heartbeat"),
    })
}

/// What a frame of the room tells the loop.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Running,
    /// Nobody has been in the room for `idle_minutes`: shut down.
    IdleTimeout,
    /// The relay connection is over (the hub closed it, the room was removed, the socket broke).
    LinkLost,
}

/// Checks an owner ticket with the directory: (room code, ticket, action) -> Ok, or the reason to show the owner.
pub type TicketCheck = Arc<dyn Fn(&str, &str, &str) -> Result<(), String> + Send + Sync>;

/// At most this many ticket checks are in flight at once; more are told to try again.
pub const MAX_CHECKS: usize = 4;

/// An owner command whose ticket the directory is still checking.
struct Check {
    peer: u32,
    action: OwnerAction,
    answer: Pending<Result<(), String>>,
}

/// The dedicated room: the simulation and the relay session.
pub struct Room {
    pub sim: Sim,
    pub net: NetSession,
    code: String,
    cfg: BackendConfig,
    token: String,
    idle_limit: Duration,
    anchor: Option<u32>,
    /// Seconds of real time with no guest in the room.
    empty_for: Duration,
    /// Every guest catch the room has resolved into a capture, in order (kept for the tests and the log).
    pub captures: Vec<GuestCatch>,
    /// What happened, for the log: joins, leaves, captures.
    pub notes: Vec<String>,
    /// The room's seed text and difficulty now (the owner changes both).
    pub room_text: String,
    pub difficulty: Difficulty,
    start_relays: Option<i32>,
    /// Whether a route is running. While it is not, nothing is simulated and no guest is told to enter.
    pub route_on: bool,
    /// How an owner ticket is checked with the directory (the tests give their own).
    pub ticket_check: TicketCheck,
    checks: Vec<Check>,
}

impl Room {
    /// Builds the room on an open `link` (the host connection to the relay): the game, the relay session and the route
    /// the guests are sent when they join.
    pub fn new(opts: &Options, cfg: BackendConfig, link: Box<dyn RoomLink>, token: String) -> Room {
        let mut sim = Sim::new();
        sim.new_room_run(opts.difficulty, &opts.room_text, true);
        let relays = opts.start_relays.unwrap_or_else(|| opts.difficulty.profile().activation_relays);
        if let Some(g) = sim.game.as_mut() {
            g.relays_collected = relays;
        }
        let net = Room::session(opts, &cfg, link, &token, &opts.room_text, opts.difficulty, true);
        let ticket_check = directory_check(cfg.clone(), token.clone());
        Room {
            sim,
            net,
            code: opts.code.clone(),
            cfg,
            token,
            idle_limit: Duration::from_secs_f64(opts.idle_minutes * 60.0),
            anchor: None,
            empty_for: Duration::ZERO,
            captures: Vec::new(),
            notes: Vec::new(),
            room_text: opts.room_text.clone(),
            difficulty: opts.difficulty,
            start_relays: opts.start_relays,
            route_on: true,
            ticket_check,
            checks: Vec::new(),
        }
    }

    fn session(opts: &Options, cfg: &BackendConfig, link: Box<dyn RoomLink>, token: &str, room_text: &str, difficulty: Difficulty, route_on: bool) -> NetSession {
        let mut net = NetSession::dedicated_host(cfg.clone(), opts.code.clone(), link, token.to_string(), room_text.to_string(), opts.name.clone(), opts.heartbeat);
        net.set_room_difficulty(difficulty);
        // the route is on from the first moment: a guest that joins is sent the seed and the hunters at once
        if route_on {
            net.start_run(room_text, difficulty.name());
        }
        net
    }

    pub fn game(&self) -> &Game {
        self.sim.game.as_ref().expect("a dedicated room always has its game")
    }

    pub fn game_mut(&mut self) -> &mut Game {
        self.sim.game.as_mut().expect("a dedicated room always has its game")
    }

    /// Opens a new relay session on `link` (the old link was lost): the guests are gone with it, the game goes on.
    pub fn reopen(&mut self, link: Box<dyn RoomLink>, opts: &Options) {
        self.net = Room::session(opts, &self.cfg, link, &self.token, &self.room_text, self.difficulty, self.route_on);
        self.checks.clear();
        self.anchor = None;
        self.game_mut().set_remote_targets(Vec::new(), None);
        self.notes.push("relay link reopened".into());
    }

    /// Whether any guest is in the room now.
    pub fn busy(&self) -> bool {
        self.net.guest_count() > 0
    }

    /// One frame: `dt` is the real time since the last one (it only feeds the idle clock); the simulation always steps
    /// one fixed [`STEP`].
    pub fn frame(&mut self, dt: Duration) -> Status {
        // the relay session: joins, leaves, samples, reports
        for event in self.net.tick(None) {
            match event {
                NetEvent::GuestJoined(name) => self.notes.push(format!("{name} joined")),
                NetEvent::GuestLeft(name) => self.notes.push(format!("{name} left")),
                NetEvent::HostLeft(reason) => self.notes.push(format!("relay: {reason}")),
                NetEvent::Problem(text) => self.notes.push(format!("relay: {text}")),
                NetEvent::OwnerCommand { peer, ticket, action, args } => self.check_owner(peer, ticket, action, args),
                _ => {}
            }
        }
        self.finish_checks();
        if !self.net.link_alive() {
            return Status::LinkLost;
        }
        let poses = self.net.guest_poses();
        if poses.is_empty() && !self.busy() {
            self.empty_for += dt;
            if self.empty_for >= self.idle_limit {
                return Status::IdleTimeout;
            }
        } else {
            self.empty_for = Duration::ZERO;
        }
        if !self.route_on {
            // the route is ended: what the guests report is dropped, nothing steps, nobody is hunted
            let _ = (self.net.take_guest_events(), self.net.take_valve_requests(), self.net.take_burst_requests());
            return Status::Running;
        }
        // the guests become the hunters' targets, and the window follows the party anchor
        let candidates: Vec<AnchorCandidate> = poses.iter().map(|p| (p.peer, (p.x, p.y), p.level)).collect();
        self.anchor = choose_anchor(&candidates, self.anchor);
        let anchor = self.anchor;
        self.game_mut().set_remote_targets(poses.iter().map(target_of).collect(), anchor);
        self.apply_reports();
        self.sim.step_room();
        self.send_outputs();
        Status::Running
    }

    /// A guest's owner command: parsed here, then its ticket is sent to the directory to check. Nothing is acted on yet.
    fn check_owner(&mut self, peer: u32, ticket: String, action: String, args: Map<String, Value>) {
        let Some(parsed) = OwnerAction::parse(&action, &args) else {
            self.notes.push(format!("owner command {action} from {peer} is not one of ours"));
            self.net.send_owner_refused(peer, &hosted::refusal_text(""));
            return;
        };
        if self.checks.len() >= MAX_CHECKS {
            self.net.send_owner_refused(peer, &hosted::rules().strings.owner_busy);
            return;
        }
        let (check, code, name) = (self.ticket_check.clone(), self.code.clone(), parsed.wire_name());
        let answer = Pending::spawn(move || Ok(check(&code, &ticket, name)));
        self.notes.push(format!("owner command {name} from {peer}: checking the ticket"));
        self.checks.push(Check { peer, action: parsed, answer });
    }

    /// The directory's answers to the tickets sent: a yes runs the command, a no tells the guest why.
    fn finish_checks(&mut self) {
        if self.checks.is_empty() {
            return;
        }
        let mut waiting = Vec::new();
        for mut check in std::mem::take(&mut self.checks) {
            match check.answer.poll() {
                None => waiting.push(check),
                Some(Ok(Ok(()))) => self.apply_owner(check.peer, check.action),
                Some(Ok(Err(reason))) => {
                    self.notes.push(format!("owner command {} from {} refused", check.action.wire_name(), check.peer));
                    self.net.send_owner_refused(check.peer, &reason);
                }
                Some(Err(_)) => {
                    self.net.send_owner_refused(check.peer, &hosted::rules().strings.owner_check_failed);
                }
            }
        }
        waiting.append(&mut self.checks);
        self.checks = waiting;
    }

    /// Carries out a command the directory has confirmed for guest `from`.
    fn apply_owner(&mut self, from: u32, action: OwnerAction) {
        self.notes.push(format!("owner command {} from {from}", action.wire_name()));
        match action {
            OwnerAction::Start => {
                if !self.route_on {
                    self.begin_route(false);
                }
            }
            OwnerAction::End => {
                if self.route_on {
                    self.end_route();
                }
            }
            OwnerAction::Restart => self.begin_route(true),
            OwnerAction::Kick(target) => {
                // the owner's own guest is never removed by its own command, and the host (peer 1) is not a guest
                if target == from || !self.net.kick_guest(target) {
                    self.net.send_owner_refused(from, &hosted::rules().strings.owner_not_in_room);
                } else {
                    self.notes.push(format!("guest {target} was removed by the owner"));
                }
            }
            OwnerAction::Difficulty(d) => {
                self.difficulty = d;
                self.net.set_room_difficulty(d);
            }
        }
    }

    /// Runs the route: a fresh game on the room's seed (a new seed when `fresh_seed`), the guests sent to enter it. A route
    /// that is running ends for everyone first.
    fn begin_route(&mut self, fresh_seed: bool) {
        if self.route_on {
            self.net.end_run(&hosted::rules().strings.owner_restart_reason);
        }
        if fresh_seed {
            self.room_text = crate::world::route::new_room_text();
        }
        self.sim.new_room_run(self.difficulty, &self.room_text, true);
        let relays = self.start_relays.unwrap_or_else(|| self.difficulty.profile().activation_relays);
        self.game_mut().relays_collected = relays;
        self.anchor = None;
        self.game_mut().set_remote_targets(Vec::new(), None);
        self.net.set_room_seed(self.room_text.clone());
        self.net.set_room_difficulty(self.difficulty);
        self.net.start_run(&self.room_text, self.difficulty.name());
        self.route_on = true;
    }

    /// Ends the route for everyone; the room stays open and waits.
    fn end_route(&mut self) {
        self.net.end_run(&hosted::rules().strings.owner_end_reason);
        self.game_mut().set_remote_targets(Vec::new(), None);
        self.anchor = None;
        self.route_on = false;
    }

    /// The guests' reports since the last frame: tasers and escapes (placed from the guest's own sample), valve turns and
    /// pipe breaches. The game decides each one; nothing a guest claims about a place is trusted.
    fn apply_reports(&mut self) {
        let origin = self.game().window_origin();
        let remotes = self.net.remotes_all(origin);
        let events = self.net.take_guest_events();
        let valves = self.net.take_valve_requests();
        let bursts = self.net.take_burst_requests();
        let g = self.sim.game.as_mut().expect("game");
        for mut ev in events {
            crate::net::wiring::place_guest_event(&mut ev, &remotes);
            g.apply_guest_event(ev);
        }
        for req in valves {
            if let Some((x, y, depth)) = self.net.guest_position(req.peer) {
                let at = Game::abs_to_window(g.window_origin(), (x, y));
                if let Some(fields) = g.host_turn_valve(&req, at, depth) {
                    let rules = crate::net::hunter_events::rules();
                    g.hunter_out.emit(rules, g.difficulty, crate::net::valve_events::VALVE, fields, Instant::now());
                }
            }
        }
        for req in bursts {
            if let Some((x, y, depth)) = self.net.guest_position(req.peer) {
                g.host_burst_pipe(&req, (x, y), depth);
            }
        }
        self.net.set_relays_sabotaged(g.relays_sabotaged_count());
    }

    /// What the game raised this frame, sent to the guests: the captures, the hunters' frame and the world events.
    fn send_outputs(&mut self) {
        let catches = self.game_mut().take_guest_catches();
        for c in catches {
            self.net.expect_capture_by(c.peer, Some(c.hunter_id.clone()), Some(c.at));
            let mut event = Map::new();
            event.insert("what".into(), json!("capture"));
            event.insert("streamed".into(), json!(true));
            event.insert("hunter_id".into(), json!(c.hunter_id));
            event.insert("entity_id".into(), json!(""));
            event.insert("inescapable".into(), json!(c.inescapable));
            event.insert("reason".into(), json!(""));
            self.net.send_player_event(c.peer, event);
            self.notes.push(format!("{} took guest {}", c.hunter_id, c.peer));
            self.captures.push(c);
        }
        if !self.busy() {
            // nobody to tell: drop the frame's events so none piles up
            let _ = self.game_mut().hunter_out.take();
            return;
        }
        let (frame, grab, diff) = {
            let g = self.game();
            (g.hunter_frame(), g.guest_grab_index(), g.difficulty)
        };
        self.net.send_hunter_frame(frame, diff, grab, Instant::now());
        let events = self.game_mut().hunter_out.take();
        self.net.send_world_events(events);
    }

    /// Ends the room: the host unregisters when it talks to the directory, then the link closes.
    pub fn close(self) {
        self.net.leave();
    }

    pub fn code(&self) -> &str {
        &self.code
    }
}

/// The ticket check of a room that talks to the directory: `POST /api/enet/owner-tickets/verify` with the room's host token.
/// The token never leaves this closure and is never logged.
pub fn directory_check(cfg: BackendConfig, token: String) -> TicketCheck {
    Arc::new(move |code, ticket, action| hosted::verify_owner_ticket(&cfg, code, ticket, &token, action))
}

/// A guest's pose as the hunters' target (absolute cells).
fn target_of(p: &GuestPose) -> RemoteTarget {
    RemoteTarget::new(p.peer, (p.x, p.y), p.h, p.level, p.yaw, p.moving, p.sprinting, p.crouching, p.hidden, p.huntable)
}

/// Exchanges a spawn ticket for the host token: the one registration call (transport ws, hosted codes accept only the
/// ticket's holder). Blocking.
pub fn token_from_ticket(cfg: &BackendConfig, opts: &Options) -> Result<String, String> {
    let request = RegisterRoom {
        code: opts.code.clone(),
        spawn_ticket: opts.spawn_ticket.clone(),
        region: opts.region.clone(),
        transport: "ws".into(),
        // the room is empty until the first guest: the heartbeat reports the real count
        players: 1,
        ..RegisterRoom::default()
    };
    api::register_room(cfg, &request).map(|r| r.token).map_err(|e| e.to_string()).and_then(|t| if t.is_empty() { Err("THE DIRECTORY ISSUED NO TOKEN".into()) } else { Ok(t) })
}

/// The loop: paced at [`STEP`] while a guest is in, slow while the room is empty. Returns the process exit code: 0 an
/// idle shutdown, 4 the relay link was lost and could not be reopened within `reconnect_seconds`.
pub fn run(mut room: Room, opts: &Options) -> i32 {
    let mut last = Instant::now();
    let mut owed = Duration::ZERO;
    let mut down_since: Option<Instant> = None;
    let mut next_try = Instant::now();
    let step = Duration::from_secs_f32(STEP);
    loop {
        let now = Instant::now();
        let real = now.duration_since(last);
        last = now;
        let code = room.code().to_string();
        for note in room.notes.drain(..) {
            eprintln!("[dedicated] {code} {note}");
        }
        if let Some(since) = down_since {
            // the link is down: try to reopen it, give up after the grace
            if now >= next_try {
                next_try = now + RECONNECT_EVERY;
                match NativeWsLink::host(&opts.code, &room.token, &opts.relay) {
                    Ok(link) => {
                        room.reopen(Box::new(link), opts);
                        down_since = None;
                        continue;
                    }
                    Err(e) => eprintln!("[dedicated] {} relay link still down: {e}", room.code()),
                }
            }
            if now.duration_since(since).as_secs_f64() >= opts.reconnect_seconds {
                eprintln!("[dedicated] room {} lost its relay link for good", room.code());
                return 4;
            }
            crate::shared::task::pause(IDLE_SLEEP);
            continue;
        }
        // with a guest in, the room steps in fixed steps (a stall is caught up a few steps, then dropped); with none it
        // makes one cheap pass per sleep, which keeps the relay session and the idle clock going
        let mut status = Status::Running;
        if room.busy() {
            owed = (owed + real).min(step * MAX_CATCH_UP_STEPS);
            while owed >= step {
                owed -= step;
                status = room.frame(Duration::ZERO);
                if status != Status::Running {
                    break;
                }
            }
        } else {
            owed = Duration::ZERO;
            status = room.frame(real);
        }
        match status {
            Status::Running => {}
            Status::IdleTimeout => {
                eprintln!("[dedicated] room {} empty for {} min: shutting down", room.code(), opts.idle_minutes);
                room.close();
                return 0;
            }
            Status::LinkLost => {
                if opts.reconnect_seconds <= 0.0 {
                    eprintln!("[dedicated] room {} lost its relay link", room.code());
                    return 4;
                }
                eprintln!("[dedicated] room {} lost its relay link, reconnecting", room.code());
                down_since = Some(Instant::now());
                next_try = Instant::now();
            }
        }
        crate::shared::task::pause(if room.busy() { BUSY_SLEEP } else { IDLE_SLEEP });
    }
}

/// `--dedicated-host`: reads the options, gets the token, opens the relay link and runs the room. Exit codes: 0 a clean
/// idle shutdown, 2 a bad command line, 3 the token or the relay was refused or unreachable, 4 the link was lost.
pub fn main(a: &Args) -> i32 {
    let env = |key: &str| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
    let (cfg, warnings) = BackendConfig::from_args(a.raw.clone(), cfg!(debug_assertions));
    for w in warnings {
        eprintln!("[dedicated] {w}");
    }
    let opts = match parse_options(a, &env, &cfg) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[dedicated] {e}");
            return 2;
        }
    };
    // a dedicated room never writes the player's checkpoint or settings
    crate::app::checkpoint::ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
    let token = if !opts.token.is_empty() {
        opts.token.clone()
    } else if !opts.spawn_ticket.is_empty() {
        match token_from_ticket(&cfg, &opts) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[dedicated] the directory refused the spawn ticket: {e}");
                return 3;
            }
        }
    } else {
        eprintln!("[dedicated] needs a host token (--host-token or LASTROUTE_HOST_TOKEN) or a spawn ticket (LASTROUTE_SPAWN_TICKET)");
        return 2;
    };
    let link = match NativeWsLink::host(&opts.code, &token, &opts.relay) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[dedicated] could not open the relay link: {e}");
            return 3;
        }
    };
    let room = Room::new(&opts, cfg, Box::new(link), token);
    eprintln!(
        "[dedicated] room {} seed {} difficulty {} region {}: hosting as {} (idle exit after {} min)",
        opts.code,
        opts.room_text,
        opts.difficulty.name(),
        if opts.region.is_empty() { "-" } else { &opts.region },
        opts.name,
        opts.idle_minutes
    );
    run(room, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::player::Player;
    use crate::net::world_sync::Sample;
    use serde_json::Value;
    use crate::net::ws_test_hub::TestHub;

    fn args(list: &[&str]) -> Args {
        Args { raw: list.iter().map(|s| s.to_string()).collect() }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn the_godot_agents_command_line_is_understood() {
        // backend/region_agent.mjs: --headless -- --dedicated-host CODE --port P --idle-minutes M [--seed S] --ws-rooms [--difficulty D]
        let a = args(&["--headless", "--", "--dedicated-host", "abcde", "--port", "18100", "--idle-minutes", "7", "--seed", "ROOM-ABCDE-1", "--ws-rooms", "--difficulty", "2"]);
        let o = parse_options(&a, &no_env, &BackendConfig::default()).unwrap();
        assert_eq!(o.code, "ABCDE");
        assert_eq!((o.idle_minutes, o.room_text.as_str(), o.difficulty), (7.0, "ROOM-ABCDE-1", Difficulty::Hard));
        assert_eq!((o.name.as_str(), o.heartbeat, o.start_relays), (DEFAULT_NAME, true, None));
        assert_eq!(o.relay, BackendConfig::default().base_url, "the backend is the relay unless told otherwise");
        assert!(o.token.is_empty() && o.spawn_ticket.is_empty());
        // the same with the equals spelling, a loopback backend, and the ticket and region in the environment
        let a = args(&["--dedicated-host=QWERT", "--backend=http://127.0.0.1:8205", "--difficulty=nightmare"]);
        let (cfg, _) = BackendConfig::from_args(a.raw.clone(), true);
        let env = |k: &str| match k {
            "LASTROUTE_SPAWN_TICKET" => Some("tkt".to_string()),
            "LASTROUTE_REGION" => Some("virginia".to_string()),
            "LASTROUTE_HOST_TOKEN" => Some("tok".to_string()),
            _ => None,
        };
        let o = parse_options(&a, &env, &cfg).unwrap();
        assert_eq!((o.code.as_str(), o.relay.as_str(), o.difficulty), ("QWERT", "http://127.0.0.1:8205", Difficulty::Nightmare));
        assert_eq!((o.token.as_str(), o.spawn_ticket.as_str(), o.region.as_str()), ("tok", "tkt", "virginia"));
        // the flags that override the environment
        let a = args(&["--dedicated-host", "QWERT", "--host-token", "flagtok", "--relay", "ws://127.0.0.1:9", "--region", "texas", "--no-heartbeat", "--start-relays", "4", "--name", "ROOMBOT", "--reconnect-seconds", "0"]);
        let o = parse_options(&a, &env, &BackendConfig::default()).unwrap();
        assert_eq!((o.token.as_str(), o.relay.as_str(), o.region.as_str()), ("flagtok", "ws://127.0.0.1:9", "texas"));
        assert_eq!((o.heartbeat, o.start_relays, o.name.as_str(), o.reconnect_seconds), (false, Some(4), "ROOMBOT", 0.0));
    }

    #[test]
    fn bad_command_lines_are_refused_with_a_reason() {
        let cfg = BackendConfig::default();
        for (list, needle) in [
            (vec!["--dedicated-host"], "5 letters"),
            (vec!["--dedicated-host", "ABC"], "5 letters"),
            (vec!["--dedicated-host", "AB CDE"], "5 letters"),
            (vec!["--dedicated-host", "ABCDE", "--difficulty", "brutal"], "--difficulty"),
            (vec!["--dedicated-host", "ABCDE", "--difficulty", "9"], "--difficulty"),
            (vec!["--dedicated-host", "ABCDE", "--idle-minutes", "soon"], "--idle-minutes"),
            (vec!["--dedicated-host", "ABCDE", "--idle-minutes", "-3"], "--idle-minutes"),
            (vec!["--dedicated-host", "ABCDE", "--start-relays", "1000"], "--start-relays"),
        ] {
            let err = parse_options(&args(&list), &no_env, &cfg).unwrap_err();
            assert!(err.contains(needle), "{list:?}: {err}");
        }
        // too small an idle time is raised to the floor
        let o = parse_options(&args(&["--dedicated-host", "ABCDE", "--idle-minutes", "0.01"]), &no_env, &cfg).unwrap();
        assert_eq!(o.idle_minutes, MIN_IDLE_MINUTES);
    }

    // ---- a dedicated room over the loopback hub, with bot guests ------------------------------------------------

    fn options(hub: &TestHub) -> Options {
        Options {
            code: "ABCDE".into(),
            token: "tok".into(),
            spawn_ticket: String::new(),
            region: String::new(),
            relay: hub.url(),
            room_text: "ROOM-ABCDE-DEDICATED".into(),
            difficulty: Difficulty::Standard,
            idle_minutes: 5.0,
            start_relays: None,
            reconnect_seconds: 0.0,
            name: DEFAULT_NAME.into(),
            heartbeat: false,
        }
    }

    fn open_room(hub: &TestHub, opts: &Options) -> Room {
        hub.add_room("ABCDE", "tok");
        let link = NativeWsLink::host("ABCDE", "tok", &hub.url()).unwrap();
        Room::new(opts, BackendConfig::default(), Box::new(link), "tok".into())
    }

    /// A scripted guest: a real guest session over a native link that stands where it is told.
    struct Bot {
        net: NetSession,
        at: (f32, f32),
        h: f32,
        started: bool,
        hunters: Vec<Value>,
        problems: Vec<String>,
        /// How many hunter frames arrived, empty ones included.
        frames: usize,
        /// How many times the route was started for it, and the reasons the route was ended and an owner command refused.
        starts: usize,
        ended: Vec<String>,
        refused: Vec<String>,
    }

    impl Bot {
        fn join(hub: &TestHub, name: &str, at: (f32, f32)) -> Bot {
            let link = NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap();
            let net = NetSession::guest_over(BackendConfig::default(), "ABCDE".into(), Box::new(link), name.into(), "DEFAULT".into());
            Bot { net, at, h: 0.0, started: false, hunters: Vec::new(), problems: Vec::new(), frames: 0, starts: 0, ended: Vec::new(), refused: Vec::new() }
        }

        fn tick(&mut self) {
            let mut p = Player::new(self.at);
            p.set_abs_y(self.h);
            let sample = Sample::of(&p, 1, (0, 0));
            for event in self.net.tick(Some(sample)) {
                match event {
                    NetEvent::Start { .. } => {
                        self.started = true;
                        self.starts += 1;
                    }
                    NetEvent::RouteEnded(reason) => self.ended.push(reason),
                    NetEvent::OwnerRefused(reason) => self.refused.push(reason),
                    NetEvent::Hunters(data) => {
                        self.frames += 1;
                        if !data.is_empty() {
                            self.hunters = data;
                        }
                    }
                    NetEvent::Problem(text) | NetEvent::HostLeft(text) => self.problems.push(text),
                    _ => {}
                }
            }
        }
    }

    /// Runs the room and the bots until `done` or a timeout; returns the last status.
    fn drive(room: &mut Room, bots: &mut [&mut Bot], mut done: impl FnMut(&mut Room, &mut [&mut Bot]) -> bool) -> Status {
        let end = std::time::Instant::now() + Duration::from_secs(40);
        let mut status = Status::Running;
        while std::time::Instant::now() < end {
            for b in bots.iter_mut() {
                b.tick();
            }
            // four sim frames per pass: the test runs faster than real time
            for _ in 0..4 {
                status = room.frame(Duration::from_millis(1));
            }
            if done(room, bots) {
                return status;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
        let seen: Vec<(bool, usize, usize, Vec<String>)> = bots.iter().map(|b| (b.started, b.hunters.len(), b.frames, b.problems.clone())).collect();
        panic!("timed out; notes {:?}, guests {}, hunters {}, bots {seen:?}", room.notes, room.net.guest_count(), room.game().hunters.len());
    }

    #[test]
    fn a_room_with_two_bot_guests_keeps_simulating_and_a_hunter_targets_a_guest() {
        let hub = TestHub::start();
        let opts = options(&hub);
        let mut room = open_room(&hub, &opts);
        let start = room.game().fac.start.center();
        let origin = room.game().window_origin();
        let at = (start.0 + origin.0 as f32, start.1 + origin.1 as f32);
        let mut a = Bot::join(&hub, "BOT-A", at);
        let mut b = Bot::join(&hub, "BOT-B", (at.0 + 12.0, at.1));

        // both guests are in, the route is on for them, and the first hunter hunts one of them
        drive(&mut room, &mut [&mut a, &mut b], |room, bots| {
            let g = room.game();
            room.net.guest_count() == 2 && bots.iter().all(|b| b.started && !b.hunters.is_empty()) && g.hunter_aim.iter().any(|x| x.is_some())
        });
        // it keeps stepping while they stand there (the first pass is only a few frames)
        drive(&mut room, &mut [&mut a, &mut b], |room, _| room.sim.frame > 150);
        let peers: Vec<u32> = room.net.guest_poses().iter().map(|p| p.peer).collect();
        assert_eq!(peers.len(), 2, "both guests have samples");
        let g = room.game();
        assert!(g.no_local_runner && g.report.is_none());
        assert!(!g.hunters.is_empty(), "the route's first hunter exists");
        let aimed: Vec<u32> = g.hunter_aim.iter().flatten().copied().collect();
        assert!(!aimed.is_empty() && aimed.iter().all(|p| peers.contains(p)), "hunters aim at guests: {aimed:?} of {peers:?}");
        assert!(room.sim.frame > 150, "the sim keeps stepping: {}", room.sim.frame);
        assert_eq!(room.net.player_count(), Some(2), "the server is no player: the room counts its guests only");
        assert!(aimed.iter().all(|p| peers.contains(p)));
        // the guests were sent the route's hunters with the room's window origin
        assert!(a.hunters.iter().all(|h| h["origin"].is_array()), "{:?}", a.hunters.first());
        assert!(room.notes.iter().any(|n| n.contains("BOT-A joined")) && room.notes.iter().any(|n| n.contains("BOT-B joined")), "{:?}", room.notes);
        assert_eq!(hub.guest_ids("ABCDE").len(), 2);

        // a guest steps onto the hunter: the room takes it, tells it, and expects its report
        let (pos, y) = (room.game().hunters[0].pos, room.game().hunters[0].y);
        let o = room.game().window_origin();
        b.at = (pos.0 + o.0 as f32, pos.1 + o.1 as f32);
        b.h = y;
        drive(&mut room, &mut [&mut a, &mut b], |room, _| !room.captures.is_empty());
        assert_eq!(room.captures.len(), 1);
        assert!(room.game().guest_held(room.captures[0].peer));
        assert!(room.notes.iter().any(|n| n.contains("took guest")), "{:?}", room.notes);

        // the guests leave: the room stays up and idles
        let frame_before = room.sim.frame;
        a.net.leave();
        b.net.leave();
        drive(&mut room, &mut [], |room, _| room.net.guest_count() == 0);
        assert!(hub.has_host("ABCDE"), "the room is still open");
        let idle_pos: Vec<(f32, f32)> = room.game().hunters.iter().map(|h| h.pos).collect();
        for _ in 0..120 {
            assert_eq!(room.frame(Duration::from_millis(10)), Status::Running);
        }
        let still: Vec<(f32, f32)> = room.game().hunters.iter().map(|h| h.pos).collect();
        assert_eq!(idle_pos, still, "an empty room moves nothing");
        assert!(room.sim.frame > frame_before);

        // and a later guest is welcome: it is sent the route and the hunters again
        let mut late = Bot::join(&hub, "BOT-LATE", at);
        drive(&mut room, &mut [&mut late], |room, bots| room.net.guest_count() == 1 && bots[0].started && !bots[0].hunters.is_empty());
        assert!(!room.game().hunters.is_empty());
    }

    #[test]
    fn an_empty_room_shuts_down_after_its_idle_time_and_a_busy_one_does_not() {
        let hub = TestHub::start();
        let mut opts = options(&hub);
        opts.idle_minutes = 1.0;
        let mut room = open_room(&hub, &opts);
        // wait for the welcome, then let the idle clock run in big steps
        drive(&mut room, &mut [], |room, _| room.net.link_alive() && room.net.linked());
        assert_eq!(room.frame(Duration::from_secs(30)), Status::Running);
        assert_eq!(room.frame(Duration::from_secs(29)), Status::Running);
        assert_eq!(room.frame(Duration::from_secs(2)), Status::IdleTimeout);
        // a guest resets the clock
        let mut opts = options(&hub);
        opts.idle_minutes = 1.0;
        let mut bot = Bot::join(&hub, "BOT", (10.0, 10.0));
        drive(&mut room, &mut [&mut bot], |room, _| room.net.guest_count() == 1);
        assert_eq!(room.frame(Duration::from_secs(50)), Status::Running);
        assert_eq!(room.frame(Duration::from_secs(50)), Status::Running, "a guest is in, so the room is not idle");
    }

    #[test]
    fn a_lost_link_is_reported_and_the_room_reopens_on_a_new_one() {
        let hub = TestHub::start();
        let opts = options(&hub);
        let mut room = open_room(&hub, &opts);
        let mut bot = Bot::join(&hub, "BOT", (10.0, 10.0));
        drive(&mut room, &mut [&mut bot], |room, _| room.net.guest_count() == 1);
        hub.close_room("ABCDE");
        let status = drive(&mut room, &mut [], |_, _| true);
        let mut status = status;
        for _ in 0..200 {
            if status == Status::LinkLost {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
            status = room.frame(Duration::from_millis(1));
        }
        assert_eq!(status, Status::LinkLost, "the hub closed the room: {:?}", room.notes);
        // reopen: the guests are gone with the old link, the game and its clock go on
        let time = room.game().time;
        let link = NativeWsLink::host("ABCDE", "tok", &hub.url()).unwrap();
        room.reopen(Box::new(link), &opts);
        let mut again = Bot::join(&hub, "BOT-2", (10.0, 10.0));
        drive(&mut room, &mut [&mut again], |room, bots| room.net.guest_count() == 1 && bots[0].started);
        assert!(room.game().time >= time);
    }

    #[test]
    fn the_process_entry_hosts_a_room_from_its_command_line_and_exits_when_the_link_is_lost() {
        let hub = TestHub::start();
        hub.add_room("ABCDE", "tok");
        let relay = hub.url();
        let runner = std::thread::spawn(move || {
            main(&args(&["--headless", "--", "--dedicated-host", "ABCDE", "--port", "18100", "--idle-minutes", "10", "--ws-rooms", "--host-token", "tok", "--relay", &relay, "--no-heartbeat", "--reconnect-seconds", "0"]))
        });
        let end = std::time::Instant::now() + Duration::from_secs(30);
        while !hub.has_host("ABCDE") && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(hub.has_host("ABCDE"), "the process connected as the host with its token");
        // a guest joining it is sent the route
        let mut bot = Bot::join(&hub, "BOT", (10.0, 10.0));
        let until = std::time::Instant::now() + Duration::from_secs(30);
        while !bot.started && std::time::Instant::now() < until {
            bot.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(bot.started, "the guest got the route start from the process: {:?}", bot.problems);
        hub.close_room("ABCDE");
        assert_eq!(runner.join().unwrap(), 4, "the link was lost and --reconnect-seconds 0 gives up");
    }

    #[test]
    fn the_dedicated_host_flag_wins_over_headless_in_the_dispatch() {
        // the region agent passes both; a bare --headless would sit reading stdin, this must answer at once
        let a = args(&["--headless", "--", "--dedicated-host", "AB"]);
        assert_eq!(super::super::dispatch_with(&a), Some(2), "routed to the dedicated host, which refuses the short code");
        // without the flag --headless is still the protocol loop (not run here), and a plain game start dispatches nothing
        assert_eq!(super::super::dispatch_with(&args(&["--offline"])), None);
        assert_eq!(super::super::dispatch_with(&args(&["--agent-window", "--dedicated-host", "ABCDE"])), None, "the windowed agent mode keeps its precedence");
    }

    #[test]
    fn the_process_entry_refuses_what_it_cannot_start_from() {
        // no code
        assert_eq!(main(&args(&["--dedicated-host"])), 2);
        // a code but neither a token nor a ticket
        assert_eq!(main(&args(&["--dedicated-host", "ABCDE", "--no-heartbeat"])), 2);
        // a token, and a relay that is not there
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(main(&args(&["--dedicated-host", "ABCDE", "--host-token", "t", "--relay", &format!("ws://127.0.0.1:{port}")])), 3);
        // a relay that refuses the token
        let hub = TestHub::start();
        hub.add_room("ABCDE", "right");
        let relay = hub.url();
        let runner = std::thread::spawn(move || main(&args(&["--dedicated-host", "ABCDE", "--host-token", "wrong", "--relay", &relay, "--reconnect-seconds", "0", "--no-heartbeat"])));
        assert_eq!(runner.join().unwrap(), 4, "connected, then refused: the link is dead and not retried");
    }

    #[test]
    fn an_empty_room_costs_next_to_nothing_a_frame() {
        let hub = TestHub::start();
        let opts = options(&hub);
        let mut room = open_room(&hub, &opts);
        drive(&mut room, &mut [], |room, _| room.net.linked());
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            assert_eq!(room.frame(Duration::from_millis(1)), Status::Running);
        }
        let per_frame = start.elapsed() / 1000;
        eprintln!("an empty room frame takes {per_frame:?}");
        assert!(per_frame < Duration::from_millis(5), "an idle frame took {per_frame:?}");
        assert!(room.game().hunters.is_empty(), "no hunters are made for nobody");
    }

    #[test]
    fn a_run_with_no_one_in_the_room_returns_zero_after_the_idle_time() {
        let hub = TestHub::start();
        let mut opts = options(&hub);
        opts.idle_minutes = 0.001;
        let room = open_room(&hub, &opts);
        // `run` reads the clock itself: the idle limit is 60 ms of real time
        assert_eq!(run(room, &opts), 0);
        // the hub notices the closed socket a moment later
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while hub.has_host("ABCDE") && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!hub.has_host("ABCDE"), "the host closed its link on the way out");
    }

    // ---- the owner's commands -----------------------------------------------------------------------------------

    /// A ticket of the shape the directory issues (43 base64url characters), and another that is not the room's.
    const GOOD: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
    const GOOD2: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFG";
    const WRONG: &str = "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ";

    type Calls = std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>>;

    /// A stand-in for the directory's verify: GOOD is the room's ticket, each (ticket, action) works once, and every call is
    /// recorded. `delay` makes the answer slow.
    fn fake_directory(delay: Duration) -> (TicketCheck, Calls) {
        let calls: Calls = Default::default();
        let used: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<(String, String)>>> = Default::default();
        let seen = calls.clone();
        let check: TicketCheck = Arc::new(move |code, ticket, action| {
            seen.lock().unwrap().push((code.to_string(), ticket.to_string(), action.to_string()));
            std::thread::sleep(delay);
            if ticket != GOOD && ticket != GOOD2 {
                return Err(hosted::refusal_text("INVALID OR EXPIRED OWNER TICKET"));
            }
            if !used.lock().unwrap().insert((ticket.to_string(), action.to_string())) {
                return Err(hosted::refusal_text("OWNER TICKET ALREADY USED FOR THIS ACTION"));
            }
            Ok(())
        });
        (check, calls)
    }

    /// A room with an owner guest and another guest in it, both running.
    fn room_with_owner(hub: &TestHub, delay: Duration) -> (Room, Bot, Bot, Calls) {
        let opts = options(hub);
        let mut room = open_room(hub, &opts);
        let (check, calls) = fake_directory(delay);
        room.ticket_check = check;
        let start = room.game().fac.start.center();
        let origin = room.game().window_origin();
        let at = (start.0 + origin.0 as f32, start.1 + origin.1 as f32);
        let mut owner = Bot::join(hub, "OWNER", at);
        let mut other = Bot::join(hub, "OTHER", (at.0 + 12.0, at.1));
        drive(&mut room, &mut [&mut owner, &mut other], |room, bots| room.net.guest_count() == 2 && bots.iter().all(|b| b.started));
        (room, owner, other, calls)
    }

    #[test]
    fn the_owner_ends_starts_and_restarts_the_route_only_through_the_directory() {
        let hub = TestHub::start();
        let (mut room, mut owner, mut other, calls) = room_with_owner(&hub, Duration::ZERO);
        assert!(room.route_on);
        let s = &hosted::rules().strings;

        // END: the guests are told with the owner's reason, the room waits, and nothing is simulated meanwhile
        owner.net.send_owner_command(GOOD, &OwnerAction::End);
        drive(&mut room, &mut [&mut owner, &mut other], |room, bots| !room.route_on && bots.iter().all(|b| !b.ended.is_empty()));
        assert_eq!(owner.ended[0], s.owner_end_reason);
        assert_eq!(other.ended[0], s.owner_end_reason);
        assert_eq!(calls.lock().unwrap().as_slice(), [("ABCDE".to_string(), GOOD.to_string(), "end".to_string())], "the directory was asked, with the room's code");
        let frame = room.sim.frame;
        drive(&mut room, &mut [&mut owner, &mut other], |room, _| room.net.guest_count() == 2);
        for _ in 0..30 {
            room.frame(Duration::from_millis(1));
        }
        assert_eq!(room.sim.frame, frame, "an ended route steps nothing");

        // a guest who joins while it is ended is not sent to the route
        let at = owner.at;
        let mut late = Bot::join(&hub, "LATE", at);
        drive(&mut room, &mut [&mut owner, &mut other, &mut late], |room, _| room.net.guest_count() == 3);
        assert_eq!(late.starts, 0);

        // START: a fresh run on the same seed, every guest is sent to it
        let seed = room.room_text.clone();
        let before = (owner.starts, other.starts, late.starts);
        owner.net.send_owner_command(GOOD, &OwnerAction::Start);
        drive(&mut room, &mut [&mut owner, &mut other, &mut late], |room, bots| room.route_on && bots[0].starts > before.0 && bots[1].starts > before.1 && bots[2].starts > before.2);
        assert_eq!(room.room_text, seed);

        // RESTART: a running route ends for everyone, then a new one starts on a new seed
        let before = (owner.starts, other.starts, owner.ended.len());
        owner.net.send_owner_command(GOOD, &OwnerAction::Restart);
        drive(&mut room, &mut [&mut owner, &mut other, &mut late], |room, bots| room.room_text != seed && bots[0].starts > before.0 && bots[1].starts > before.1);
        assert_eq!(owner.ended.len(), before.2 + 1);
        assert_eq!(owner.ended.last().unwrap(), &s.owner_restart_reason);
        assert!(room.route_on);
        assert_eq!(room.net.room_seed(), room.room_text, "the guests are told the new seed");
    }

    #[test]
    fn a_command_with_a_bad_or_spent_ticket_is_refused_with_the_directorys_reason() {
        let hub = TestHub::start();
        let (mut room, mut owner, mut other, calls) = room_with_owner(&hub, Duration::ZERO);
        // a ticket the directory does not know
        owner.net.send_owner_command(WRONG, &OwnerAction::End);
        drive(&mut room, &mut [&mut owner, &mut other], |_, bots| !bots[0].refused.is_empty());
        assert_eq!(owner.refused[0], "ROOM OWNER REFUSED: INVALID OR EXPIRED OWNER TICKET");
        assert!(room.route_on, "a refused command changes nothing");
        assert!(other.refused.is_empty() && other.ended.is_empty(), "only the sender hears of it");
        // the room's own ticket works once per action: the second kick of the same ticket is refused
        owner.net.send_owner_command(GOOD, &OwnerAction::Difficulty(Difficulty::Hard));
        drive(&mut room, &mut [&mut owner, &mut other], |room, _| room.difficulty == Difficulty::Hard);
        owner.net.send_owner_command(GOOD, &OwnerAction::Difficulty(Difficulty::Relaxed));
        drive(&mut room, &mut [&mut owner, &mut other], |_, bots| bots[0].refused.len() == 2);
        assert_eq!(owner.refused[1], "ROOM OWNER REFUSED: OWNER TICKET ALREADY USED FOR THIS ACTION");
        assert_eq!(room.difficulty, Difficulty::Hard);
        assert_eq!(calls.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_command_is_never_run_before_the_directory_has_answered_and_the_room_keeps_simulating_meanwhile() {
        let hub = TestHub::start();
        let (mut room, mut owner, mut other, _calls) = room_with_owner(&hub, Duration::from_millis(400));
        let frame = room.sim.frame;
        owner.net.send_owner_command(GOOD, &OwnerAction::End);
        // while the directory thinks, the route is still on and the room steps
        drive(&mut room, &mut [&mut owner, &mut other], |room, _| room.sim.frame > frame + 30);
        assert!(room.route_on, "the directory has not said yes yet");
        drive(&mut room, &mut [&mut owner, &mut other], |room, _| !room.route_on);
    }

    #[test]
    fn the_owner_kicks_another_guest_and_never_itself() {
        let hub = TestHub::start();
        let (mut room, mut owner, mut other, _calls) = room_with_owner(&hub, Duration::ZERO);
        // the owner's own roster (from the room's message) names the other guest's session id, and not its own
        drive(&mut room, &mut [&mut owner, &mut other], |_, bots| !bots[0].net.roster_peers().is_empty());
        let peers = owner.net.roster_peers();
        assert_eq!(peers.len(), 1, "{peers:?}");
        assert_eq!(peers[0].1, "OTHER");
        let host_side = room.net.roster_peers();
        assert_eq!(host_side.len(), 2);
        let other_peer = peers[0].0;
        let owner_peer = host_side.iter().find(|(_, n)| n == "OWNER").unwrap().0;
        assert!(owner_peer != other_peer && other_peer > 1);

        // kicking oneself is refused
        owner.net.send_owner_command(GOOD2, &OwnerAction::Kick(owner_peer));
        drive(&mut room, &mut [&mut owner, &mut other], |_, bots| !bots[0].refused.is_empty());
        assert_eq!(owner.refused[0], hosted::rules().strings.owner_not_in_room);
        assert_eq!(room.net.guest_count(), 2);

        // kicking the other guest removes it: the relay closes it with the removed reason, the room forgets it
        owner.net.send_owner_command(WRONG, &OwnerAction::Kick(other_peer));
        drive(&mut room, &mut [&mut owner, &mut other], |_, bots| bots[0].refused.len() == 2);
        assert_eq!(room.net.guest_count(), 2, "a wrong ticket removes nobody");
        // the room's own ticket, unspent for a kick (the self-kick above used another)
        owner.net.send_owner_command(GOOD, &OwnerAction::Kick(other_peer));
        drive(&mut room, &mut [&mut owner, &mut other], |room, bots| room.net.guest_count() == 1 && !bots[1].problems.is_empty());
        assert!(other.problems.iter().any(|p| p.contains("REMOVED")), "{:?}", other.problems);
        assert!(room.notes.iter().any(|n| n.contains("was removed by the owner")), "{:?}", room.notes);
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while hub.guest_ids("ABCDE").len() > 1 && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(hub.guest_ids("ABCDE").len(), 1, "the relay dropped the removed guest");
        // the owner is told the new roster: nobody else is in the room
        drive(&mut room, &mut [&mut owner], |_, bots| bots[0].net.roster_peers().is_empty());
    }

    #[test]
    fn the_difficulty_the_owner_picks_applies_from_the_next_route() {
        let hub = TestHub::start();
        let (mut room, mut owner, mut other, _calls) = room_with_owner(&hub, Duration::ZERO);
        assert_eq!(room.game().difficulty, Difficulty::Standard);
        owner.net.send_owner_command(GOOD, &OwnerAction::Difficulty(Difficulty::Hard));
        drive(&mut room, &mut [&mut owner, &mut other], |room, bots| room.difficulty == Difficulty::Hard && bots[1].net.room_difficulty() == "HARD");
        assert_eq!(room.game().difficulty, Difficulty::Standard, "the running route keeps its difficulty");
        owner.net.send_owner_command(GOOD, &OwnerAction::Restart);
        drive(&mut room, &mut [&mut owner, &mut other], |room, bots| room.game().difficulty == Difficulty::Hard && bots.iter().all(|b| b.starts >= 2));
    }

    #[test]
    fn a_dedicated_room_without_the_directory_refuses_every_command() {
        // the production check, with requests switched off: it never says yes on its own
        let cfg = BackendConfig { offline: true, ..BackendConfig::default() };
        let check = directory_check(cfg, "hosttoken".into());
        for action in ["start", "end", "kick", "restart", "difficulty"] {
            assert_eq!(check("ABCDE", GOOD, action), Err(hosted::rules().strings.owner_refused_default.clone()), "{action}");
        }
        // a ticket of the wrong shape is refused without asking anybody
        assert!(check("ABCDE", "short", "start").is_err());
    }

}
