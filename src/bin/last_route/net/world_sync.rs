//! Shared rooms for the Rust client: hosting and joining by code over the directory and the LRR1 relay, the route seed
//! handed to the guests, and the position exchange that puts each runner into the others' world.
//!
//! Flow (the Godot room flow, `room_state.gd` / `backend_client.gd`, with the Rust transport of `net::session`):
//! * Host: only the public route is hosted by a client now (`join_public_room`: `api::register_room`, the relay host
//!   link, a directory heartbeat every 5 s). Every other online room is hosted by a server process
//!   (`headless/dedicated.rs`, `net/hosted.rs`) and joined as a guest. The route seed is the room text
//!   (`ROOM-<code>-<hex>`), so the lobby shows the code the guests type.
//! * Guest: [`join_room`] looks the code up (`api::lookup_room`), refuses a room that is not on the relay or that runs
//!   another build's protocol, then joins the relay (`RelayLink::guest`).
//! * Handshake: a guest's first reliable packet is `RegisterPeer`; the host answers `Room` (the seed) and, while a
//!   route runs, `Start`. A guest enters the same seeded world with `Start`.
//! * Positions: each runner sends a [`Sample`] at 20 Hz on the STATE channel. A guest sends `Place` to the host; the
//!   host sends its own sample and each accepted guest sample to every guest as `PeerPlace` (the sender's peer id).
//!   A sample is `null` while nothing is playing: that keeps the relay session alive, nothing else.

use super::api::{self, Heartbeat, RegisterRoom, Registered, ReservedHostView};
use super::guest_name;
use super::hunter_events::{self, Tracker};
use super::interp::{self, SendClock, Track};
use super::config::BackendConfig;
use super::protocol::{self, Message};
use super::relay::{RelayEvent, RelayLink, Role};
use super::room_link::RoomLink;
use super::session::{CoopSession, SessionEvent, HOST_PEER_ID};
use super::ws_link::WsLink;
use crate::shared::task::IN_BROWSER;
use super::valve_events::{self, BurstRequest, ValveRequest};
use super::{store, NetError, Pending};
use crate::config::difficulty::{ALL, LOBBY};
use crate::config::Difficulty;
use crate::game::player::Player;
use crate::game::Item;
use crate::game::Game;
use crate::render::anim::runner::Pose;
use crate::world::vertical::STOREY;
use crate::shared::assets::Assets;
use macroquad::prelude::*;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use crate::shared::clock::Instant;
use std::time::Duration;

/// A remote runner with no sample for this long is not drawn (`data/net/sync.json` `remote_timeout_s`, 2 s). The send
/// rate is `send_hz` there (30; the original sends 20 Hz `_place`).
/// When the next directory heartbeat is due after one failed: at once for the public room (its entry lives 15 s),
/// otherwise as already scheduled.
fn retry_beat_at(code: &str, now: Instant, scheduled: Instant) -> Instant {
    if code == super::public_route::rules().code {
        now.min(scheduled)
    } else {
        scheduled
    }
}

/// A heartbeat goes out when none is in flight, no relist runs, and the schedule has come.
fn beat_due(in_flight: bool, relisting: bool, now: Instant, next: Instant) -> bool {
    !in_flight && !relisting && now >= next
}

fn remote_timeout() -> Duration {
    Duration::from_secs_f32(interp::rules().remote_timeout_s)
}
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
#[cfg(test)]
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

/// The highest inventory slot a guest's taser report may name (Godot `clampi(slot, -1, 63)`). Protocol bound, not tuning.
const MAX_TASER_SLOT: i64 = 63;
/// The skins by id, each with its figure colours by peer id. DEFAULT is the only one so far; an id no skin has
/// is drawn as DEFAULT (see [`skin_colour`]).
pub(crate) const SKINS: [(&str, [Color; 4]); 1] = [(SKIN_DEFAULT, PEER_COLOURS)];

/// The locker a hidden runner is in, in absolute cells like the rest of a sample, so a peer in another streamed window
/// finds the same locker: its cell and the way its front faces (from the back wall into the room).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HideSpot {
    pub cell: (i32, i32),
    pub facing: (i32, i32),
}

/// One runner's sample: the `_place` packet `{seq, depth, x, y, yaw, moving, sprinting, crouching}`.
/// `x` and `y` are the runner's absolute ground cell (its window position moved by the window's origin, see
/// [`to_absolute`]), so runners in different streamed windows agree on where each one stands. `depth` is its floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub seq: u32,
    pub depth: i32,
    /// The runner's absolute feet height (continuous levels). `None` from an older client, which is then filtered by
    /// `depth` alone.
    pub h: Option<f32>,
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
    /// The pose the runner shows (Godot `clip`). `None` from an older client: its flags then pick the pose.
    pub clip: Option<Pose>,
    /// Vertical speed in metres per second (0 on the ground and from an older client).
    pub vy: f32,
    /// Ground velocity in cells per second, measured by the sender (0 from an older client or a standing runner).
    pub vx: f32,
    pub vz: f32,
    /// The runner wears slim arms (its ARMS setting resolved for its skin).
    pub slim: bool,
    /// The locker the runner is hidden in, if any (Godot `hidden` + `hiding_spot`).
    pub hide: Option<HideSpot>,
    /// The item the runner holds (`held`, see `net::held_wire`); None when it holds nothing, from an older client, or
    /// when the id is one this build does not know.
    pub held: Option<Item>,
    /// The runner's flashlight: `Some(pitch)` while its beam is lit (the aim is its yaw and this pitch, radians), None
    /// when it is off or the sender is older. Wire: `"flash": true` and `"pitch"` (only when not level).
    pub flash_pitch: Option<f32>,
    /// The runner's aim pitch in radians (+ up), whatever its lamp does: it tilts the head. Wire: `"look"`, sent only
    /// when not level. An older peer never sends it, so its head stays level.
    pub look_pitch: f32,
    /// The emote the runner is playing (an id from data/player/emotes.json) and its start counter. Wire: `"emote"` and
    /// `"emote_seq"`, both sent only while an emote plays; an older peer never sends or reads them.
    pub emote: Option<EmoteTag>,
    pub emote_seq: Option<u32>,
}

/// An emote id on the wire, held inline so a [`Sample`] stays `Copy`: 1 to [`EmoteTag::MAX`] ASCII letters, digits, `_` or `-`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmoteTag {
    len: u8,
    bytes: [u8; EmoteTag::MAX],
}

impl EmoteTag {
    pub const MAX: usize = 24;

    /// The tag for `id`; None for an empty, over-long or odd-looking id (a hostile peer cannot send anything else).
    pub fn new(id: &str) -> Option<EmoteTag> {
        let b = id.as_bytes();
        if b.is_empty() || b.len() > Self::MAX || !b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-') {
            return None;
        }
        let mut bytes = [0u8; Self::MAX];
        bytes[..b.len()].copy_from_slice(b);
        Some(EmoteTag { len: b.len() as u8, bytes })
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

impl Sample {
    /// The sample of a runner's body in the window whose origin is `origin`; `seq` is filled in when it is sent.
    /// The run state flags start false: [`Sample::with_run_state`] sets them.
    pub fn of(p: &Player, depth: i32, origin: (i32, i32)) -> Sample {
        let (x, y) = to_absolute(p.pos, origin);
        Sample {
            seq: 0,
            depth,
            h: Some(p.y()),
            x,
            y,
            yaw: p.yaw,
            moving: p.moving,
            sprinting: p.sprinting,
            crouching: p.crouching,
            spectating: false,
            game_over: false,
            escape_active: false,
            clip: None,
            vy: 0.0,
            vx: 0.0,
            vz: 0.0,
            slim: false,
            hide: None,
            held: None,
            flash_pitch: p.beam_lit().then_some(p.pitch),
            look_pitch: p.pitch,
            emote: None,
            emote_seq: None,
        }
    }

    /// The sample with the item the runner holds.
    pub fn with_held(mut self, held: Option<Item>) -> Sample {
        self.held = held;
        self
    }

    /// The sample with the runner's body state: its pose, vertical speed, slim arms and locker.
    pub fn with_body(mut self, clip: Pose, vy: f32, slim: bool, hide: Option<HideSpot>) -> Sample {
        self.clip = Some(clip);
        self.vy = vy;
        self.slim = slim;
        self.hide = hide;
        self
    }

    /// The sample with the emote the runner is playing (`EmoteState::wire`): its id and start counter.
    pub fn with_emote(mut self, wire: Option<(&str, u32)>) -> Sample {
        if let Some((id, seq)) = wire {
            self.emote = EmoteTag::new(id);
            self.emote_seq = self.emote.map(|_| seq);
        }
        self
    }

    /// The sample with this runner's run state: `game_over` once its route has ended (a catch or an abandon),
    /// `escape_active` while a hunter holds it in the escape struggle.
    pub fn with_run_state(mut self, game_over: bool, escape_active: bool) -> Sample {
        self.game_over = game_over;
        self.escape_active = escape_active;
        self
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "seq": self.seq, "depth": self.depth, "x": self.x, "y": self.y, "yaw": self.yaw,
            "moving": self.moving, "sprinting": self.sprinting, "crouching": self.crouching,
            "spectating": self.spectating, "game_over": self.game_over, "escape_active": self.escape_active,
        });
        if let Some(o) = v.as_object_mut() {
            if let Some(h) = self.h {
                o.insert("h".into(), json!(h));
            }
            // the body state is optional on the wire: an older client never sends it and ignores it
            if let Some(c) = self.clip {
                o.insert("clip".into(), json!(c.wire()));
            }
            if self.vy != 0.0 {
                o.insert("vy".into(), json!(self.vy));
            }
            if self.vx != 0.0 || self.vz != 0.0 {
                o.insert("vx".into(), json!(self.vx));
                o.insert("vz".into(), json!(self.vz));
            }
            if self.slim {
                o.insert("slim".into(), json!(true));
            }
            if let Some(h) = self.hide {
                o.insert("hide".into(), json!([h.cell.0, h.cell.1, h.facing.0, h.facing.1]));
            }
            if let Some(i) = self.held {
                o.insert("held".into(), json!(super::held_wire::wire(i)));
            }
            // the flashlight is optional too: a lit beam sends its flag and, when it is not level, its pitch
            if let Some(pitch) = self.flash_pitch {
                o.insert("flash".into(), json!(true));
                if pitch != 0.0 {
                    o.insert("pitch".into(), json!(pitch));
                }
            }
            // the aim tilts the head whether or not the lamp is on; an older peer ignores the key
            if self.look_pitch != 0.0 {
                o.insert("look".into(), json!(self.look_pitch));
            }
            // an emote that plays: its id and start counter (both optional on the wire)
            if let Some(tag) = self.emote {
                o.insert("emote".into(), json!(tag.as_str()));
                if let Some(seq) = self.emote_seq {
                    o.insert("emote_seq".into(), json!(seq));
                }
            }
        }
        v
    }

    /// Parses a received sample. `None` for anything else (a keep-alive is `null`) and for a
    /// non-finite position, so a bad packet can never place a figure at NaN.
    pub fn from_json(v: &Value) -> Option<Sample> {
        let o = v.as_object()?;
        let num = |k: &str| o.get(k).and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite());
        let flag = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
        // a hostile or broken peer's velocity is scaled down to the speed cap, never trusted as sent
        let (vx, vz) = interp::clamp_velocity(num("vx").unwrap_or(0.0), num("vz").unwrap_or(0.0), interp::rules().max_speed_mps);
        Some(Sample {
            seq: o.get("seq")?.as_u64()? as u32,
            depth: i32::try_from(o.get("depth")?.as_i64()?).ok()?,
            h: num("h"),
            x: num("x")?,
            y: num("y")?,
            yaw: num("yaw").unwrap_or(0.0),
            moving: flag("moving"),
            sprinting: flag("sprinting"),
            crouching: flag("crouching"),
            spectating: flag("spectating"),
            game_over: flag("game_over"),
            escape_active: flag("escape_active"),
            clip: o.get("clip").and_then(Value::as_str).and_then(Pose::from_wire),
            vy: num("vy").unwrap_or(0.0),
            vx,
            vz,
            slim: flag("slim"),
            hide: o.get("hide").and_then(hide_spot_from_json),
            held: o.get("held").and_then(Value::as_str).and_then(super::held_wire::from_wire),
            // a pitch past straight up or down is clamped: a hostile sample cannot aim the beam through the floor
            flash_pitch: flag("flash").then(|| num("pitch").unwrap_or(0.0).clamp(-FLASH_PITCH_LIMIT, FLASH_PITCH_LIMIT)),
            look_pitch: num("look").unwrap_or(0.0).clamp(-LOOK_PITCH_LIMIT, LOOK_PITCH_LIMIT),
            emote: o.get("emote").and_then(Value::as_str).and_then(EmoteTag::new),
            emote_seq: o.get("emote_seq").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        })
    }
}

/// The steepest pitch a received flashlight aim may have (radians, about 70 degrees).
pub const FLASH_PITCH_LIMIT: f32 = 1.2;
/// The steepest aim pitch a received head may be given (radians): the same bound as the flashlight's.
pub const LOOK_PITCH_LIMIT: f32 = FLASH_PITCH_LIMIT;

/// `[cell x, cell y, facing x, facing y]`; anything else is no locker.
fn hide_spot_from_json(v: &Value) -> Option<HideSpot> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let n = |i: usize| a[i].as_i64().and_then(|n| i32::try_from(n).ok());
    Some(HideSpot { cell: (n(0)?, n(1)?), facing: (n(2)?.clamp(-1, 1), n(3)?.clamp(-1, 1)) })
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
    /// The runner's name from its profile (empty until the profile arrives).
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub yaw: f32,
    /// The runner's absolute feet height (an old client's sample gives `depth * STOREY`).
    pub h: f32,
    /// The level that owns `h`.
    pub level: i32,
    /// The runner's skin id (`SKIN_DEFAULT` until its profile arrives).
    pub skin: String,
    /// From its last sample: the figure's walk, sprint and crouch.
    pub moving: bool,
    pub sprinting: bool,
    pub crouching: bool,
    /// Slim arms (the ARMS setting resolved for its skin); false until the skin's layout is known here.
    pub slim: bool,
    /// Ground speed and vertical speed in metres per second, from the sample's gait.
    pub speed: f32,
    pub vy: f32,
    /// The pose the sample names (None from an older client).
    pub clip: Option<Pose>,
    /// The locker the runner is hidden in, in this window's cells; None when it is out in the open.
    pub hide: Option<HideSpot>,
    /// The item the runner holds in its hand, if any.
    pub held: Option<Item>,
    /// `Some(pitch)` while the runner's flashlight is lit (its sample's aim); None when it is off.
    pub flash_pitch: Option<f32>,
    /// The runner's aim pitch from its last sample (radians, + up): the head tilts to it.
    pub look_pitch: f32,
    /// The emote the runner is playing: its id and start counter (None when it plays none or the sender is older).
    pub emote: Option<(String, u32)>,
    /// Whether the runner was out in the open `HIDE_MOVE_SECONDS` ago: a runner going into a locker stays visible while
    /// it slides in (Godot remote_players.gd 542-563). Set by [`NetSession::remotes_at`].
    pub hide_since: Option<std::time::Duration>,
}

/// What the figure code needs of a remote runner's last sample, moved into the window whose origin is `origin`.
fn remote_view(peer: u32, name: String, r: &Remote, skin: String, origin: (i32, i32), now: Instant) -> RemoteView {
    let shown = r.shown(now);
    let (x, y) = to_window((shown.x, shown.y), origin);
    let h = shown.h.unwrap_or(shown.depth as f32 * STOREY);
    RemoteView {
        peer,
        name,
        x,
        y,
        yaw: shown.yaw,
        h,
        level: crate::game::level_stack::owning_level(h),
        skin,
        moving: shown.moving,
        sprinting: shown.sprinting,
        crouching: shown.crouching,
        slim: shown.slim,
        clip: shown.clip,
        held: shown.held,
        flash_pitch: shown.flash_pitch,
        look_pitch: shown.look_pitch,
        emote: shown.emote.map(|t| (t.as_str().to_string(), shown.emote_seq.unwrap_or(0))),
        hide: shown.hide.and_then(|h| Some(HideSpot { cell: (h.cell.0.checked_sub(origin.0)?, h.cell.1.checked_sub(origin.1)?), facing: h.facing })),
        hide_since: r.hide_since.map(|t| now.duration_since(t)),
        speed: if !shown.moving {
            0.0
        } else if shown.vx != 0.0 || shown.vz != 0.0 {
            let (vx, vz) = interp::clamp_velocity(shown.vx, shown.vz, interp::rules().max_speed_mps);
            vx.hypot(vz)
        } else if shown.sprinting {
            crate::config::PLAYER_SPRINT_SPEED
        } else if shown.crouching {
            crate::config::PLAYER_CROUCH_SPEED
        } else {
            crate::config::PLAYER_WALK_SPEED
        },
        vy: shown.vy,
    }
}

/// A guest as a dedicated room's hunters see it ([`NetSession::guest_poses`]): the last sample, in absolute cells.
#[derive(Clone, Debug, PartialEq)]
pub struct GuestPose {
    pub peer: u32,
    pub name: String,
    pub x: f32,
    pub y: f32,
    /// Absolute feet height, and the level that owns it.
    pub h: f32,
    pub level: i32,
    pub yaw: f32,
    pub moving: bool,
    pub sprinting: bool,
    pub crouching: bool,
    pub hidden: bool,
    pub huntable: bool,
    /// The guest's flashlight is lit (its sample's `flash`): a hunter sees the beam, as it sees the local runner's.
    pub flashlight: bool,
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
    /// The host gave this guest another name because the one it asked for was taken by an earlier guest (guest side).
    /// The app keeps the new name for the next run.
    Renamed(String),
    /// The room's owner sent a route command (host side, reserved rooms only). The app carries it out, as Godot's
    /// `RoomState._on_reserved_commands` does, and the command is acked when it is handed over.
    RouteCommand(RouteCommand),
    /// The host ended the route (guest side): the run is over, the room stays open. The text is the host's reason.
    RouteEnded(String),
    /// The host's hunter frame (guest side): each hunter's state, as `hunter_events::state_name` names it. Display only.
    Hunters(Vec<Value>),
    /// A world event the host relayed (guest side): an object with `what` (see `net::hunter_events`).
    WorldEvent(Map<String, Value>),
    /// A guest asked the room for an owner command (host side, a server-side room): `peer` is the guest's session, `ticket`
    /// the directory's owner ticket. The room runs the command only after the directory has confirmed the ticket.
    OwnerCommand { peer: u32, ticket: String, action: String, args: Map<String, Value> },
    /// The room refused this runner's owner command (guest side); the text is the reason to show the owner.
    OwnerRefused(String),
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

/// A capture the host is waiting on: when it was sent, which hunter took the guest and where (host-held, this
/// window's cells).
struct PendingCapture {
    sent: Instant,
    hunter: Option<String>,
    at: Option<(f32, f32)>,
}

struct Remote {
    /// The newest sample kept.
    sample: Sample,
    last_seen: Instant,
    /// When this runner's samples first said it was hidden in a locker (None while it is in the open).
    hide_since: Option<Instant>,
    /// Its recent samples, played back late so the figure moves smoothly (see `net::interp`).
    track: Track,
}

impl Remote {
    fn new(sample: Sample, last_seen: Instant, hide_since: Option<Instant>) -> Remote {
        let rules = interp::rules();
        let mut track = Track::new(rules);
        track.push(interp::secs(last_seen), sample, rules);
        Remote { sample, last_seen, hide_since, track }
    }

    /// The runner as it is drawn at `now`: the buffered samples played back late.
    fn shown(&self, now: Instant) -> Sample {
        self.track.shown(interp::secs(now), interp::rules()).unwrap_or(self.sample)
    }
}

struct Guest {
    name: String,
    last_seen: Instant,
}

/// What a runner says about its skin next to the id: the arm style it wears and whether others may copy the skin.
/// Godot sends both with the hash (`skin_slim`, `skin_copy`, docs/PLAYER_SKINS.md); an older client sends neither, which
/// reads as classic arms and not copyable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkinFlags {
    /// Slim (Alex) arms: the runner's ARMS setting resolved for its skin.
    pub slim: bool,
    /// OTHERS MAY COPY.
    pub copy: bool,
}

/// The wire profile of a runner: `{peer_id, name, skin, skin_slim, skin_copy}`. A guest sends its own (the host reads the
/// sender from the relay); the host forwards each profile with its `peer_id`, and sends a new guest every profile in the room.
fn profile_message(peer: u32, name: &str, skin: &str, flags: SkinFlags) -> Message {
    let mut profile = Map::new();
    profile.insert("peer_id".into(), json!(peer));
    profile.insert("name".into(), json!(name));
    profile.insert("skin".into(), json!(skin));
    profile.insert("skin_slim".into(), json!(flags.slim));
    profile.insert("skin_copy".into(), json!(flags.copy));
    Message::WireProfile { profile }
}

/// The skin flags a received profile carries (a missing flag is false).
fn profile_flags(profile: &Map<String, Value>) -> SkinFlags {
    let flag = |key: &str| profile.get(key).and_then(Value::as_bool).unwrap_or(false);
    SkinFlags { slim: flag("skin_slim"), copy: flag("skin_copy") }
}

/// The `skin_slim` a received profile carries; None when it has none (an old client) or it is not a boolean. Unlike
/// [`profile_flags`] this tells "classic" from "not said", so a missing word can fall back to the downloaded picture.
fn profile_slim(profile: &Map<String, Value>) -> Option<bool> {
    profile.get("skin_slim").and_then(Value::as_bool)
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

/// The skin id a runner publishes for its chosen skin, read against the website's uploaded list now: the id once the
/// website holds it, else DEFAULT.
fn publishable_skin(skin: &str) -> String {
    let uploaded = if is_skin_hash(&clean_skin(skin)) { crate::shared::skins::uploaded_ids() } else { Vec::new() };
    published_skin(skin, &uploaded)
}

/// The skin a republish sends: `Some(new id)` when the chosen skin, read against `uploaded`, publishes as a different id
/// than `current`; `None` when nothing changes (the room needs no new profile). A host that uploads after opening stays
/// on DEFAULT until this returns the hash.
pub fn republished_skin(current: &str, chosen: &str, uploaded: &[String]) -> Option<String> {
    let next = published_skin(chosen, uploaded);
    (next != current).then_some(next)
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
    /// This runner's own skin flags, sent in its wire profile.
    flags: SkinFlags,
    /// The skin flags of each other runner, by peer id, from its wire profile.
    peer_flags: HashMap<u32, SkinFlags>,
    /// The arm style each other runner's profile explicitly named (`skin_slim`), by peer id; absent when it named none.
    skin_slims: HashMap<u32, bool>,
    /// The name of each other runner, by peer id, from its wire profile (a guest keeps the others' names too).
    names: HashMap<u32, String>,
    guests: HashMap<u32, Guest>,
    remotes: HashMap<u32, Remote>,
    seq: u32,
    /// The fixed-step clock of this runner's samples and the last one sent (its time in seconds), for the velocity.
    send_clock: SendClock,
    /// The NPC teammates the host runs: the host's sends, and a guest's copy of them ([`super::npc_sync`]).
    npcs: super::npc_sync::NpcSync,
    last_sent: Option<(f64, Sample)>,
    next_heartbeat: Instant,
    heartbeat: Option<Pending<api::HeartbeatReply>>,
    relist: Option<Pending<Registered>>,
    opened: Instant,
    /// Host: whether this room talks to the directory (heartbeat, relist). A dedicated room started with a host token has
    /// one (it is the room's owner of record); a test room over an in-process hub does not.
    directory: bool,
    /// Host: a server-side room, whose host is no runner: the room's player count is its guests.
    dedicated: bool,
    timed_out: bool,
    /// Host: the owner-command cursors of a reserved room (see [`OwnerCursor`]).
    owner: OwnerCursor,
    /// Host: owner events from the registration reply, handed to the app on the first tick.
    backlog: Vec<NetEvent>,
    /// Host: the hunter event state (last states, grab, frame timing), see [`hunter_events::Tracker`].
    hunter_tracker: Tracker,
    /// Host: guests the host captured and has not yet heard back from (Godot `_pending_captures`). A guest's escaped,
    /// caught or capture_blocked report is accepted only from one of these, and only once.
    /// Each with the moment the host sent the capture, so one that never answers expires (`capture.timeout_s`).
    pending_captures: HashMap<u32, PendingCapture>,
    /// Host: the last taser intent accepted from each guest, for the data's least gap (`guest.taser_min_gap_s`).
    last_taser: HashMap<u32, Instant>,
    /// Host: per-guest chat moderation (rates, duplicates, mutes).
    chat_moderation: super::chat::ChatModeration,
    /// Chat lines received (guest) or accepted (host) and not yet taken by the app ([`NetSession::take_chat_lines`]).
    chat_inbox: Vec<super::chat::ChatLine>,
    /// Host: guests' sewer valve turn requests waiting for the game ([`NetSession::take_valve_requests`]).
    valve_requests: Vec<ValveRequest>,
    /// Host: guests' Axe pipe breach requests waiting for the game ([`NetSession::take_burst_requests`]).
    burst_requests: Vec<BurstRequest>,
    /// Host: guest hunter events accepted this tick (taser, or a resolved capture with `peer_id`), taken by the app.
    guest_events: Vec<Map<String, Value>>,
    /// Host: the hunter frame last sent, and the sabotaged relay count, for the snapshot a late joiner is sent.
    hunters_now: Vec<Value>,
    relays_sabotaged: u32,
    /// Guest: the snapshot the host sent on join, until the app takes it ([`NetSession::take_snapshot`]).
    snapshot: Option<hunter_events::Snapshot>,
    /// The account that owns the room (upper case; empty for a room with no owner of record), from the directory entry.
    room_owner: String,
    /// Guest: the session id of each name in `players`, from the host's last room message (0 when the host sent none).
    player_peers: Vec<u32>,
    /// Guest: whether the host is a server process (its last room message said so), not a runner. A room whose host
    /// was a server process is not handed to a runner the moment it goes (net/public_route.rs `Reform`).
    host_dedicated: bool,
    /// Guest: whether the host's room message (the roster with session ids) has arrived. A runner that joined as the host
    /// left has not been told who is in the room, so it does not know its place in line (`reform_plan`).
    room_seen: bool,
}

/// CREATE a room on the directory and open its relay as the host. The request blocks, so the frame loop polls the result.
/// The game no longer hosts online rooms from a client (a server process does, `net/hosted.rs`); this is the test
/// harness' way to stand up a host on a test directory.
#[cfg(test)]
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
    // the browser build hosts over the WebSocket relay (a ws room); the native build over the UDP relay (a relay room)
    let link: Box<dyn RoomLink> = if IN_BROWSER && reg.room.transport == "ws" {
        match WsLink::host(&code, &reg.token, &cfg.base_url) {
            Ok(link) => Box::new(link),
            Err(e) => {
                let _ = api::unregister_room(&cfg, &code, &reg.token);
                return Err(NetError::Transport(e.to_string()));
            }
        }
    } else if reg.room.transport != "relay" || reg.room.address.is_empty() {
        // this client has no direct path: give the code back rather than leave a room nobody can join
        let _ = api::unregister_room(&cfg, &code, &reg.token);
        return Err(NetError::Unsupported("THE SERVER DOES NOT RELAY ROOMS".into()));
    } else {
        Box::new(RelayLink::host(&code, &reg.token, &reg.room.address, reg.room.port).map_err(|e| NetError::Transport(e.to_string()))?)
    };
    let skin = publishable_skin(&skin);
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
        let lookup = api::lookup_room(&cfg, &code);
        // the browser build joins a private room with its join ticket (the page has no password prompt yet)
        if IN_BROWSER && matches!(&lookup, Err(NetError::Http { status: 403, code: c, .. }) if c == "ROOM_RESERVED") {
            return join_reserved(cfg, code, name, skin, session_token);
        }
        match lookup_outcome(lookup, is_admin, &session_token)? {
            JoinStep::Found(room) => join_found(cfg, code, room, name, skin, ""),
            JoinStep::Reserve => reserve_and_host(cfg, code, name, skin, session_token),
        }
    })
}

/// PUBLIC ROUTE: looks up the public code (`data/net/public_route.json`). A runner holding it is joined as a guest; a
/// miss opens the room and holds the route as its stand-in host (the route has no player host once a server process
/// holds it: `client_host_fallback` in the data turns the stand-in off). A code someone registered between the lookup and
/// the open joins that room instead. Blocking, so polled like [`host_room`].
pub fn join_public_room(cfg: BackendConfig, name: String, skin: String) -> Pending<NetSession> {
    Pending::spawn(move || {
        if cfg.offline {
            return Err(NetError::Offline);
        }
        let ops = DirectoryOps { cfg, name, skin };
        match enter_public(&ops, super::public_route::rules().client_host_fallback)? {
            Entry::Hosting(session) | Entry::Joined(session) => Ok(session),
            // the stand-in is off and no server process holds the route
            Entry::NoRoom => Err(NetError::Refused(crate::content::public_roster::PublicRoster::load().messages.unavailable)),
        }
    })
}

/// What the public join does to the directory and the relay, so the join and the handover share one path and a test can
/// stand a hub behind it.
pub(crate) trait PublicOps: Send + Sync {
    /// The directory entry of the public code (404: nobody holds the route).
    fn lookup(&self) -> Result<api::RoomEndpoint, NetError>;
    /// Registers the code and opens it as the host. [`HostOutcome::HasHost`] when somebody opened it in the same moment.
    fn open(&self) -> Result<HostOutcome, NetError>;
    /// Joins the room the directory returned as a guest.
    fn join(&self, room: api::RoomEndpoint) -> Result<NetSession, NetError>;
}

pub(crate) enum HostOutcome {
    Hosting(NetSession),
    HasHost,
    /// The code was registered but the relay link could not be opened: the error is the answer (nothing to join).
    Failed(NetError),
}

/// How a try to enter the public route ended.
pub(crate) enum Entry {
    /// Nobody held the route: this runner opened it.
    Hosting(NetSession),
    /// Somebody holds the route (a server process or a runner): this runner is its guest.
    Joined(NetSession),
    /// Nobody holds the route and this try may not open it.
    NoRoom,
}

/// One try to enter the public route: join the room that is there; when there is none, open it if `may_host`. Two runners
/// that try at the same moment cannot both host: only one registration of the code succeeds, and the other joins it.
pub(crate) fn enter_public(ops: &dyn PublicOps, may_host: bool) -> Result<Entry, NetError> {
    match super::public_route::step(ops.lookup())? {
        super::public_route::Step::Join(room) => Ok(Entry::Joined(ops.join(room)?)),
        super::public_route::Step::Host if !may_host => Ok(Entry::NoRoom),
        super::public_route::Step::Host => match ops.open() {
            Ok(HostOutcome::Hosting(session)) => Ok(Entry::Hosting(session)),
            Ok(HostOutcome::HasHost) => Ok(Entry::Joined(ops.join(ops.lookup()?)?)),
            Ok(HostOutcome::Failed(e)) => Err(e),
            Err(first) => match ops.lookup() {
                Ok(room) => Ok(Entry::Joined(ops.join(room)?)),
                Err(_) => Err(first),
            },
        },
    }
}

/// The public join over the real directory and relay.
pub(crate) struct DirectoryOps {
    cfg: BackendConfig,
    name: String,
    skin: String,
}

impl DirectoryOps {
    pub(crate) fn new(cfg: BackendConfig, name: String, skin: String) -> DirectoryOps {
        DirectoryOps { cfg, name, skin }
    }
}

impl PublicOps for DirectoryOps {
    fn lookup(&self) -> Result<api::RoomEndpoint, NetError> {
        api::lookup_room(&self.cfg, &super::public_route::rules().code)
    }

    fn open(&self) -> Result<HostOutcome, NetError> {
        let code = super::public_route::rules().code.clone();
        let request = RegisterRoom {
            code: code.clone(),
            device_id: store::device_id(&store::default_dir()),
            ..RegisterRoom::default()
        };
        let reg = api::register_room(&self.cfg, &request)?;
        let mut session = match open_host(self.cfg.clone(), code, crate::world::route::new_room_text(), self.name.clone(), self.skin.clone(), reg) {
            Ok(session) => session,
            Err(e) => return Ok(HostOutcome::Failed(e)),
        };
        // somebody opened the public room in the same moment: the relay says so, and this runner joins
        match probe_public_host(session.coop.link.as_mut(), PUBLIC_HOST_PROBE) {
            HostProbe::Hosting => Ok(HostOutcome::Hosting(session)),
            HostProbe::HasHost => {
                session.coop.link.close();
                Ok(HostOutcome::HasHost)
            }
        }
    }

    fn join(&self, room: api::RoomEndpoint) -> Result<NetSession, NetError> {
        join_found(self.cfg.clone(), super::public_route::rules().code.clone(), room, self.name.clone(), self.skin.clone(), "")
    }
}

/// How long a runner that just opened the public room waits for the relay to say it already has a host.
const PUBLIC_HOST_PROBE: Duration = Duration::from_millis(1500);

/// The relay's first word to a public host registration.
#[derive(Debug, PartialEq, Eq)]
enum HostProbe {
    /// An ack, or no answer in time: this runner holds the route.
    Hosting,
    /// The public route has a live host (`HOST_JOIN_AS_GUEST`, or the old host-authorization refusal): join it.
    HasHost,
}

/// Polls a fresh host link until the relay acks or sends the host away, or `wait` passes.
fn probe_public_host(link: &mut dyn RoomLink, wait: Duration) -> HostProbe {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        for event in link.poll() {
            match event {
                RelayEvent::JoinAsGuest => return HostProbe::HasHost,
                RelayEvent::Error(text) if is_host_refusal(&text) => return HostProbe::HasHost,
                RelayEvent::Ready { .. } => return HostProbe::Hosting,
                _ => {}
            }
        }
        crate::shared::task::pause(Duration::from_millis(20));
    }
    HostProbe::Hosting
}

/// An older relay refuses a public host with this text: for the public route it means "join as a guest", never a failure.
fn is_host_refusal(text: &str) -> bool {
    text.to_uppercase().contains("HOST AUTHORIZATION")
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
/// A guest publishes its chosen skin id in its profile, the same as a host (only once the website holds it, see
/// [`publishable_skin`]); remote runners fetch the skin by id. The guest's own screen keeps its local skin (Godot
/// "Local-only", `PLAYER_SKINS.md`).
/// `ticket` is a reserved room's join ticket (base64url) from the reserved resolve; empty for an open room. A ws room (a
/// hosted room, or a browser host's) is joined over the WebSocket relay by both builds; a relay room over UDP by the native build.
fn join_found(cfg: BackendConfig, code: String, room: api::RoomEndpoint, name: String, skin: String, ticket: &str) -> Result<NetSession, NetError> {
    let ws = room.transport == "ws";
    if !ws && (room.transport != "relay" || room.address.is_empty()) {
        return Err(NetError::Refused("THAT ROOM CANNOT BE JOINED FROM THIS BUILD".into()));
    }
    let refused = api::room_mismatch(&room, &protocol::fingerprint());
    if !refused.is_empty() {
        return Err(NetError::Refused(refused));
    }
    let link: Box<dyn RoomLink> = if ws {
        // the browser build joins through the page's WebSocket; the native build opens its own (net/ws_native.rs)
        #[cfg(target_arch = "wasm32")]
        let link = WsLink::guest(&code, ticket, &cfg.base_url);
        #[cfg(not(target_arch = "wasm32"))]
        let link = super::ws_native::NativeWsLink::guest(&code, ticket, &cfg.base_url);
        // the connect's own reason (COULD NOT REACH THE SERVER (...), SECURE CONNECTION FAILED) is shown instead of the generic join text
        Box::new(link.map_err(|e| NetError::Refused(e.to_string()))?)
    } else {
        Box::new(RelayLink::guest(&code, &room.address, room.port, ticket).map_err(|e| NetError::Transport(e.to_string()))?)
    };
    let mut session = NetSession::new(cfg, code, Role::Guest, CoopSession::new(link), String::new(), String::new(), name, publishable_skin(&skin));
    session.room_owner = room.owner.to_uppercase();
    Ok(session)
}

/// A JOIN to a private (reserved) room, browser build: the directory resolves the code for this player (her session's
/// invite, or the room password, which the page does not ask for yet: empty), and a live room is joined with the ticket it
/// hands back. A room that is not open yet refuses.
fn join_reserved(cfg: BackendConfig, code: String, name: String, skin: String, session_token: String) -> Result<NetSession, NetError> {
    let resolved = api::resolve_reserved(&cfg, &code, &session_token, "")?;
    match resolved.room {
        Some(room) if resolved.live => join_found(cfg, code, room, name, skin, &resolved.join_ticket),
        _ => Err(NetError::Refused("THAT PRIVATE ROOM IS NOT OPEN".into())),
    }
}

/// An owner's miss: reserve the typed code for her session, then register and open it as the host (the same open as
/// [`host_room`]). A code someone started hosting between the lookup and the reserve joins that live room instead.
fn reserve_and_host(cfg: BackendConfig, code: String, name: String, skin: String, session_token: String) -> Result<NetSession, NetError> {
    match api::admin_reserve_room(&cfg, &code, &session_token) {
        Ok(()) => {}
        Err(NetError::Http { code: c, .. }) if c == "ROOM_ALREADY_HOSTED" => {
            let room = api::lookup_room(&cfg, &code)?;
            return join_found(cfg, code, room, name, skin, "");
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
        let first_beat = Duration::from_secs_f64(super::public_route::heartbeat_seconds(&code));
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
            flags: SkinFlags::default(),
            peer_flags: HashMap::new(),
            skin_slims: HashMap::new(),
            names: HashMap::new(),
            guests: HashMap::new(),
            remotes: HashMap::new(),
            seq: 0,
            send_clock: SendClock::default(),
            npcs: Default::default(),
            last_sent: None,
            next_heartbeat: now + first_beat,
            heartbeat: None,
            relist: None,
            opened: now,
            directory: true,
            dedicated: false,
            timed_out: false,
            owner: OwnerCursor::default(),
            backlog: Vec::new(),
            hunter_tracker: Tracker::default(),
            pending_captures: HashMap::new(),
            last_taser: HashMap::new(),
            chat_moderation: super::chat::ChatModeration::default(),
            chat_inbox: Vec::new(),
            valve_requests: Vec::new(),
            burst_requests: Vec::new(),
            guest_events: Vec::new(),
            hunters_now: Vec::new(),
            relays_sabotaged: 0,
            snapshot: None,
            room_owner: String::new(),
            player_peers: Vec::new(),
            host_dedicated: false,
            room_seen: false,
        }
    }

    /// A room hosted by a server-side process (`headless/dedicated.rs`): the link is already open as the room's host with
    /// the token the directory issued, and no client plays here. `name` is what the guests see as the host (the Godot
    /// dedicated host is called SERVER). `directory` keeps the heartbeat on (player count and route flag for the room
    /// list); the host sends only keep-alives as its own sample, never a position.
    pub fn dedicated_host(cfg: BackendConfig, code: String, link: Box<dyn RoomLink>, token: String, route_seed: String, name: String, directory: bool) -> NetSession {
        let mut session = NetSession::new(cfg, code, Role::Host, CoopSession::new(link), token, route_seed, name, SKIN_DEFAULT.to_string());
        session.directory = directory;
        session.dedicated = true;
        session
    }

    /// A guest over a link the caller opened (a native client on [`super::ws_native::NativeWsLink`], or a test).
    pub fn guest_over(cfg: BackendConfig, code: String, link: Box<dyn RoomLink>, name: String, skin: String) -> NetSession {
        NetSession::new(cfg, code, Role::Guest, CoopSession::new(link), String::new(), String::new(), name, publishable_skin(&skin))
    }

    pub fn is_host(&self) -> bool {
        self.role == Role::Host
    }

    /// Whether this is the public route's room. It has no owner and no host-only controls whoever holds it: a runner
    /// that holds the room as a stand-in for the server process (net/public_route.rs) runs it like any runner.
    pub fn is_public_route(&self) -> bool {
        super::public_route::is_public(&self.code)
    }

    /// Whether the controls of a room's host or owner (KICK, END ROUTE, RESTART, a difficulty choice, the HOST tag in the
    /// PLAYERS list) exist for this runner: never on the public route, and for the host of any other room only.
    pub fn host_controls(&self) -> bool {
        self.is_host() && !self.is_public_route()
    }

    /// Where this runner stands in the line to open the public room again when its host is gone (or, for a stand-in host,
    /// when its own relay link is gone), from the host's last room message (net/public_route.rs `plan`).
    /// - A guest that has not yet had the host's room message (it joined as the host left) knows nobody in the room: it
    ///   goes behind the first in line, so a runner who never saw the world does not win the room from those playing in it.
    /// - A stand-in host whose link ended for good ([`NetSession::lost_public_link`]) goes behind the guests it had.
    /// - A host that lost the race for the room (the relay says another host holds it) only joins.
    pub fn reform_plan(&self) -> super::public_route::Plan {
        if self.is_host() {
            if self.lost_public_link() {
                return super::public_route::Plan { rank: Some(self.guests.len()), was_dedicated: false };
            }
            return super::public_route::Plan::join_only();
        }
        if !self.room_seen {
            return super::public_route::Plan { rank: Some(1), was_dedicated: false };
        }
        super::public_route::plan(self.coop.link.session(), &self.player_peers, self.host_dedicated)
    }

    /// Whether this is the public route's stand-in host and its relay link is over for good (a dropped connection, a
    /// second host replacing it). The relay has then closed the room on its guests, who open it again among themselves;
    /// this runner is no longer the host of anything and must not keep its directory entry alive (the heartbeat would
    /// list a room nobody can join), so the app lets it go and brings it back through the handover.
    pub fn lost_public_link(&self) -> bool {
        self.is_host() && self.is_public_route() && !self.dedicated && !self.coop.link.alive()
    }

    /// Host: how many runners the host itself counts for in the room: one, or none in a dedicated room.
    fn runners(&self) -> u32 {
        u32::from(!self.dedicated)
    }

    /// The account that owns this room, from the directory entry (upper case); empty when the room has no owner of record.
    pub fn room_owner(&self) -> &str {
        &self.room_owner
    }

    /// Guest: sends the room an owner command with the directory's owner ticket (Godot `_owner_command.rpc_id(1, ...)`).
    /// False on the host, or when the message could not be sent. The room checks the ticket with the directory.
    /// Host: sends the NPCs this host's game runs to every guest (see [`super::npc_sync`]). Nothing for a guest.
    pub fn publish_npcs(&mut self, npcs: &[crate::game::npc::Npc], origin: (i32, i32), downed: &std::collections::BTreeSet<u32>) {
        if !self.is_host() {
            return;
        }
        let states: Vec<(u32, serde_json::Map<String, serde_json::Value>)> =
            npcs.iter().map(|n| (n.id, super::npc_sync::wire_state_with(n, origin, downed.contains(&n.id)))).collect();
        let messages = self.npcs.host_messages(&states, Instant::now());
        if messages.is_empty() {
            return;
        }
        let guests: Vec<u32> = self.guests.keys().copied().collect();
        for message in &messages {
            for peer in &guests {
                self.coop.send(*peer, message);
            }
        }
    }

    /// The NPC teammates the host runs, as a guest draws them in its window (`origin`). Empty for the host.
    pub fn remote_npc_views(&self, origin: (i32, i32)) -> Vec<super::npc_sync::RemoteNpcView> {
        self.npcs.views(origin, Instant::now())
    }

    pub fn send_owner_command(&mut self, ticket: &str, action: &super::hosted::OwnerAction) -> bool {
        if self.is_host() {
            return false;
        }
        let message = Message::OwnerCommand { ticket: ticket.to_string(), action: action.wire_name().to_string(), args: action.args() };
        self.coop.send(HOST_PEER_ID, &message)
    }

    /// Host: tells guest `peer` the room refused its owner command.
    pub fn send_owner_refused(&mut self, peer: u32, reason: &str) -> bool {
        self.is_host() && self.coop.send(peer, &Message::OwnerRefused { reason: reason.to_string() })
    }

    /// Host: removes guest `peer` from the room. The relay closes its socket (the guest hears YOU WERE REMOVED FROM THE
    /// ROOM), the guest is dropped from the roster at once, and the others get the new roster. False when no such guest is
    /// in the room.
    pub fn kick_guest(&mut self, peer: u32) -> bool {
        // nobody removes a runner from the public route: it has no owner
        if !self.host_controls() || !self.guests.contains_key(&peer) {
            return false;
        }
        self.coop.kick(peer);
        self.forget_guest(peer);
        self.send_room_and_roster();
        true
    }

    /// Host: drops everything kept for guest `peer`; the guest's name when it was in the room.
    fn forget_guest(&mut self, peer: u32) -> Option<String> {
        self.remotes.remove(&peer);
        self.skins.remove(&peer);
        self.peer_flags.remove(&peer);
        self.skin_slims.remove(&peer);
        self.pending_captures.remove(&peer);
        self.last_taser.remove(&peer);
        self.guests.remove(&peer).map(|g| g.name)
    }

    /// Whether a server process hosts this room (this process is the dedicated host, or the host told us it is).
    pub fn hosted_by_server(&self) -> bool {
        self.dedicated || self.host_dedicated
    }

    /// This runner's peer id in the room, as chat lines carry it (the host is `HOST_PEER_ID`).
    pub fn local_peer_id(&self) -> i64 {
        if self.is_host() {
            HOST_PEER_ID as i64
        } else {
            self.coop.link.session() as i64
        }
    }

    /// Host: how many guests the room holds now (the host itself is no runner in a dedicated room).
    pub fn guest_count(&self) -> usize {
        self.guests.len()
    }

    /// Host only: how many runners the room holds now, the host included (the same count the heartbeat
    /// sends). None on a guest, which does not see the other runners.
    pub fn player_count(&self) -> Option<u32> {
        self.is_host().then(|| self.runners() + self.guests.len() as u32)
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

    /// Sets this runner's chosen skin id and publishes it at once. The published id is the chosen one only when the
    /// website holds it (`uploaded`, the list `shared::skins::uploaded_ids` gives), else DEFAULT. A host sends the new
    /// profile to every guest; a guest sends it to the host, which relays it. Nothing is sent when the published id
    /// stays the same, so a host that uploads after opening is re-published the moment the caller passes the new list.
    pub fn set_skin(&mut self, chosen: &str, uploaded: &[String]) {
        let Some(next) = republished_skin(&self.skin, chosen, uploaded) else { return };
        self.skin = next;
        self.publish_profile();
    }

    /// Sets this runner's skin flags (the arm style it wears, whether others may copy the skin) and publishes them at once
    /// when they changed.
    pub fn set_skin_flags(&mut self, flags: SkinFlags) {
        if flags == self.flags {
            return;
        }
        self.flags = flags;
        self.publish_profile();
    }

    /// Sends this runner's profile: a host to every guest, a guest to the host (which relays it).
    fn publish_profile(&mut self) {
        if self.is_host() {
            let profile = profile_message(HOST_PEER_ID, &self.name, &self.skin, self.flags);
            for peer in self.coop.peers() {
                self.coop.send(peer, &profile);
            }
        } else {
            let profile = profile_message(self.coop.link.session(), &self.name, &self.skin, self.flags);
            self.coop.send(HOST_PEER_ID, &profile);
        }
    }

    /// Records (or forgets) the arm style a peer's profile named.
    fn note_peer_slim(&mut self, peer: u32, slim: Option<bool>) {
        match slim {
            Some(slim) => self.skin_slims.insert(peer, slim),
            None => self.skin_slims.remove(&peer),
        };
    }

    /// Whether a peer's arms are slim by its profile, else by its picture once it is here (classic until then).
    fn peer_slim(&self, peer: u32) -> bool {
        self.skin_slims
            .get(&peer)
            .copied()
            .or_else(|| self.skins.get(&peer).filter(|s| is_skin_hash(s)).and_then(|s| crate::shared::skins::skin_is_slim(s)))
            .unwrap_or(false)
    }

    /// The other runners whose skin can be copied (COPY SKIN): they allow it, publish a real skin id, and are in the room.
    /// The arm style is the owner's, since the pixels were normalised.
    pub fn copy_candidates(&self) -> Vec<(String, String, bool)> {
        self.roster()
            .into_iter()
            .filter_map(|e| {
                let flags = self.peer_flags.get(&e.peer).copied().unwrap_or_default();
                (flags.copy && is_skin_hash(&e.skin)).then(|| (e.name, e.skin, flags.slim))
            })
            .collect()
    }

    /// This runner's published skin id (DEFAULT until a chosen skin the website holds is set).
    pub fn published_skin(&self) -> &str {
        &self.skin
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

    /// The other runners of the room with their session ids, for the owner's lobby (KICK needs the id). Host: the guests.
    /// Guest: the others in the host's last room message, this runner and the host left out. A name with no id from the host
    /// is left out too.
    pub fn roster_peers(&self) -> Vec<(u32, String)> {
        if self.is_host() {
            let mut v: Vec<(u32, String)> = self.guests.iter().filter(|(_, g)| !g.name.is_empty()).map(|(p, g)| (*p, g.name.clone())).collect();
            v.sort_by_key(|(p, _)| *p);
            return v;
        }
        let me = self.coop.link.session();
        self.players
            .iter()
            .zip(self.player_peers.iter())
            .filter(|(name, peer)| **peer > HOST_PEER_ID && **peer != me && !name.is_empty() && !name.eq_ignore_ascii_case(&self.host_name))
            .map(|(name, peer)| (*peer, name.clone()))
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
                .filter(|r| now.duration_since(r.last_seen) < remote_timeout())
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
                // no HOST tag on the public route: whoever runs the room, it has no player host
                host: peer == HOST_PEER_ID && !self.is_public_route(),
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
        // a server process that holds a room is not a runner: it has no row on the public route
        let hosted_by_server = self.host_dedicated && self.is_public_route();
        if !hosted_by_server && (!self.host_name.is_empty() || fresh(HOST_PEER_ID).is_some()) {
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

    /// False once the relay connection is over for good (see [`RoomLink::alive`]).
    pub fn link_alive(&self) -> bool {
        self.coop.link.alive()
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
        // the session id of each name above, in the same order (the owner's KICK names a guest by it)
        let mut named: Vec<(&u32, &Guest)> = self.guests.iter().filter(|(_, g)| !g.name.is_empty()).collect();
        named.sort_by_key(|(peer, _)| **peer);
        rest.insert("peers".into(), Value::Array(named.into_iter().map(|(peer, _)| json!(peer)).collect()));
        rest.insert("difficulty".into(), Value::String(self.difficulty.clone()));
        // a server process holds this room: its guests do not hand it to a runner the moment it goes (public_route::Reform)
        if self.dedicated {
            rest.insert("dedicated".into(), Value::Bool(true));
        }
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
        self.hunters_now.clone_from(&frame);
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

    /// Host: sends the world events the game queued this frame (see [`hunter_events::Outbox`], which applied each kind's
    /// difficulty and gap) to every guest, in order. Nothing is sent by a guest.
    /// Host: the host captured this guest, so its next escaped, caught or capture_blocked report is expected (Godot
    /// `_pending_captures[peer_id]` set where the host sends a capture). Does nothing on a guest.
    pub fn expect_capture(&mut self, peer: u32) {
        self.expect_capture_by(peer, None, None);
    }

    /// Host: as [`NetSession::expect_capture`], and records which hunter (`hunter`, its def id) took the guest and
    /// where (`at`, this window's cells). The guest's later escaped report is resolved against this record only: the
    /// hunter id and place the guest claims are dropped.
    pub fn expect_capture_by(&mut self, peer: u32, hunter: Option<String>, at: Option<(f32, f32)>) {
        if self.is_host() {
            self.pending_captures.insert(peer, PendingCapture { sent: Instant::now(), hunter, at });
        }
    }

    /// Host: stops waiting for the captured guests that have not answered within `capture.timeout_s`. Each one is
    /// handed to the app as a `capture_timeout` guest event (with its `peer_id`), once, and a late report from it is
    /// dropped like any report that was not expected. Returns how many expired.
    fn expire_captures(&mut self, now: Instant) -> usize {
        let limit = hunter_events::rules().capture_timeout;
        let mut expired: Vec<u32> = self
            .pending_captures
            .iter()
            .filter(|(_, p)| now.saturating_duration_since(p.sent) >= limit)
            .map(|(peer, _)| *peer)
            .collect();
        expired.sort_unstable();
        for peer in &expired {
            self.pending_captures.remove(peer);
            let mut ev = Map::new();
            ev.insert("what".into(), json!("capture_timeout"));
            ev.insert("peer_id".into(), json!(peer));
            self.guest_events.push(ev);
        }
        expired.len()
    }

    /// Host: tells one guest something about itself (Godot `_player_event`: a `capture` when a hunter takes it).
    pub fn send_player_event(&mut self, peer: u32, event: Map<String, Value>) -> bool {
        self.is_host() && self.guests.contains_key(&peer) && self.coop.send(peer, &Message::PlayerEvent { event })
    }

    /// Host: the guests with a fresh sample as the hunters of a dedicated room need them: absolute cell and height, the
    /// level that owns it, the flags that make noise, and whether a hunter may take it. In peer order.
    pub fn guest_poses(&self) -> Vec<GuestPose> {
        if !self.is_host() {
            return Vec::new();
        }
        let now = Instant::now();
        let mut out: Vec<GuestPose> = self
            .remotes
            .iter()
            .filter(|(peer, r)| self.guests.contains_key(peer) && now.duration_since(r.last_seen) < remote_timeout())
            .map(|(peer, r)| {
                let s = &r.sample;
                let h = s.h.unwrap_or(s.depth as f32 * STOREY);
                GuestPose {
                    peer: *peer,
                    name: self.peer_name(*peer),
                    x: s.x,
                    y: s.y,
                    h,
                    level: crate::game::level_stack::owning_level(h),
                    yaw: s.yaw,
                    moving: s.moving,
                    sprinting: s.sprinting,
                    crouching: s.crouching,
                    hidden: s.hide.is_some(),
                    // spectating, over, or already in a hunter's grip: not a target
                    huntable: !(s.spectating || s.game_over || s.escape_active),
                    flashlight: s.flash_pitch.is_some(),
                }
            })
            .collect();
        out.sort_by_key(|g| g.peer);
        out
    }

    /// Sends a typed chat line to the room. A guest submits it to the host (which moderates and rebroadcasts it, so the
    /// guest sees its own line when it comes back); the host sends it to everyone and keeps it itself. False when the
    /// line is empty or could not be sent.
    pub fn send_chat(&mut self, text: &str, channel: &str, at: [f64; 2], world: [f64; 3], depth: i64) -> bool {
        let Some(message) = super::chat::submit(text, channel, at, world, depth) else { return false };
        if !self.is_host() {
            return self.coop.send(HOST_PEER_ID, &message);
        }
        let Message::SubmitChatPayload { payload } = message else { return false };
        let line = super::chat::ChatLine::sanitize(HOST_PEER_ID as i64, &self.name, &payload, false);
        self.deliver_chat(Message::ChatPayload { peer_id: line.peer_id, display_name: line.display_name.clone(), payload: line.payload() });
        true
    }

    /// The chat lines that arrived since the last call, oldest first (feed them to a [`super::chat::ChatFeed`] and the TTS).
    pub fn take_chat_lines(&mut self) -> Vec<super::chat::ChatLine> {
        std::mem::take(&mut self.chat_inbox)
    }

    /// Host: sends an accepted chat message to every guest and keeps its line for the host's own screen.
    fn deliver_chat(&mut self, message: Message) {
        for peer in self.coop.peers() {
            self.coop.send(peer, &message);
        }
        self.keep_chat(&message);
    }

    fn keep_chat(&mut self, message: &Message) {
        if let Some(line) = super::chat::ChatLine::from_message(message) {
            self.chat_inbox.push(line);
            let extra = self.chat_inbox.len().saturating_sub(super::chat::ChatRules::load().inbox_max);
            self.chat_inbox.drain(..extra);
        }
    }

    /// Host: the guests' valve turn requests since the last call, oldest first. A guest always gets nothing.
    pub fn take_valve_requests(&mut self) -> Vec<ValveRequest> {
        std::mem::take(&mut self.valve_requests)
    }

    /// Host: the guests' Axe pipe breach requests since the last call, oldest first. A guest always gets nothing.
    pub fn take_burst_requests(&mut self) -> Vec<BurstRequest> {
        std::mem::take(&mut self.burst_requests)
    }

    /// Host: where a guest last stood (absolute cell and depth), for the checks on its requests: the host's own copy
    /// of the last accepted sample, never a place the request itself carries. `None` for a peer with no fresh sample
    /// (older than [`remote_timeout`]), or on a guest.
    pub fn guest_position(&self, peer: u32) -> Option<(f32, f32, i32)> {
        let now = Instant::now();
        self.is_host()
            .then(|| self.remotes.get(&peer))
            .flatten()
            .filter(|r| now.duration_since(r.last_seen) < remote_timeout())
            .map(|r| (r.sample.x, r.sample.y, r.sample.depth))
    }

    /// Guest: sends the host a hunter event (Godot `_submit_hunter_event`): a `taser` intent with its `slot`, or the
    /// `escaped`, `caught` and `capture_blocked` reports that answer a capture. Any other kind is not sent. The host
    /// re-checks everything from its own state. Returns whether it was sent; a host never sends one.
    pub fn submit_hunter_event(&mut self, event: Map<String, Value>) -> bool {
        let what = event.get("what").and_then(Value::as_str).unwrap_or_default();
        if self.is_host() || !["taser", "escaped", "caught", "capture_blocked"].contains(&what) {
            return false;
        }
        self.coop.send(HOST_PEER_ID, &Message::SubmitHunterEvent { event })
    }

    /// Guest: asks the host to turn a sewer valve (the host runs the turn and publishes a `stream_valve` event that
    /// every machine applies). Does nothing on the host, which turns valves itself.
    pub fn request_valve(&mut self, valve_id: &str, effect: &str, gate: &str, valve_at: (f32, f32)) {
        if !self.is_host() {
            let event = valve_events::request_message(valve_id, effect, gate, valve_at);
            self.coop.send(HOST_PEER_ID, &Message::RequestInteraction { event });
        }
    }

    /// Guest: asks the host to burst a sewer pipe the guest's Axe breached (the host checks the guest's place and
    /// publishes a `stream_pipe_burst` event). Does nothing on the host, which bursts pipes itself.
    pub fn request_burst(&mut self, flow_id: &str) {
        if !self.is_host() {
            let event = valve_events::burst_request_message(flow_id);
            self.coop.send(HOST_PEER_ID, &Message::RequestInteraction { event });
        }
    }

    /// Host: checks a guest's valve or breach request and queues it for the game (bounded by `guest.max_valve_requests`).
    fn accept_valve_request(&mut self, from: u32, event: &Map<String, Value>) {
        if from <= HOST_PEER_ID || !self.guests.contains_key(&from) {
            return;
        }
        let max = hunter_events::rules().max_valve_requests;
        if event.get("what").and_then(Value::as_str) == Some(valve_events::BURST_REQUEST) {
            if self.burst_requests.len() < max {
                if let Some(request) = valve_events::parse_burst_request(from, event) {
                    self.burst_requests.push(request);
                }
            }
            return;
        }
        if self.valve_requests.len() >= max {
            return;
        }
        if let Some(request) = valve_events::parse_request(from, event) {
            self.valve_requests.push(request);
        }
    }

    /// Host: takes the guest hunter events accepted since the last call, oldest first. Each taser has `what`, `peer_id`
    /// and `slot`; each resolved capture report keeps its fields and gets `peer_id`. A guest always gets nothing.
    pub fn take_guest_events(&mut self) -> Vec<Map<String, Value>> {
        std::mem::take(&mut self.guest_events)
    }

    /// Host: checks a guest's hunter event. A taser is only an intent (the host re-checks position, facing and line of
    /// sight from its own state), so it is passed on with the sender and a slot clamped to -1..=63 like Godot does. The
    /// three report kinds are accepted only when the host expects that guest's capture, and that expectation is used up.
    /// Host-only senders (peer 1 or lower) and other kinds are dropped.
    fn accept_guest_event(&mut self, from: u32, event: Map<String, Value>, now: Instant) {
        if from <= HOST_PEER_ID {
            return;
        }
        let what = event.get("what").and_then(Value::as_str).unwrap_or_default().to_string();
        match what.as_str() {
            "taser" => {
                // one intent per `guest.taser_min_gap_s`: a faster stream from one guest is dropped
                let gap = hunter_events::rules().guest_taser_gap;
                if self.last_taser.get(&from).is_some_and(|t| now.saturating_duration_since(*t) < gap) {
                    return;
                }
                self.last_taser.insert(from, now);
                let slot = event.get("slot").and_then(Value::as_f64).map_or(-1, |v| v as i64);
                let slot = slot.clamp(-1, MAX_TASER_SLOT);
                let mut out = Map::new();
                out.insert("what".into(), json!("taser"));
                out.insert("peer_id".into(), json!(from));
                out.insert("slot".into(), json!(slot));
                self.guest_events.push(out);
            }
            "escaped" | "caught" | "capture_blocked" => {
                let Some(pending) = self.pending_captures.remove(&from) else { return };
                let mut received = event;
                if what == "escaped" {
                    // the escape is resolved against what the host recorded at the capture; the guest's own claim of
                    // the hunter and the place is dropped
                    for key in ["hunter_id", "entity_id", "x", "y", "yaw"] {
                        received.remove(key);
                    }
                    if let Some(id) = pending.hunter {
                        received.insert("hunter_id".into(), json!(id));
                    }
                    if let Some((x, y)) = pending.at {
                        received.insert("x".into(), json!(x));
                        received.insert("y".into(), json!(y));
                    }
                }
                received.insert("peer_id".into(), json!(from));
                self.guest_events.push(received);
            }
            _ => {}
        }
    }

    pub fn send_world_events(&mut self, events: Vec<Map<String, Value>>) {
        if !self.is_host() {
            return;
        }
        for event in events {
            let message = Message::Event { event };
            for peer in self.coop.peers() {
                self.coop.send(peer, &message);
            }
        }
    }

    /// Host: the sabotaged relay count the snapshot for a late joiner carries (Godot `_build_snapshot`). A guest's call does nothing.
    pub fn set_relays_sabotaged(&mut self, count: u32) {
        if self.is_host() {
            self.relays_sabotaged = count;
        }
    }

    /// Guest: takes the snapshot the host sent on join, once. The app applies its relays; the hunters came as
    /// `NetEvent::Hunters` already.
    pub fn take_snapshot(&mut self) -> Option<hunter_events::Snapshot> {
        self.snapshot.take()
    }

    /// Host: the hunter state as the snapshot carries it: the last hunter frame and the sabotaged relay count.
    fn snapshot_message(&self) -> Message {
        Message::Snapshot { state: hunter_events::snapshot_state(&self.hunters_now, self.relays_sabotaged) }
    }

    /// Host: the snapshot a runner joining now is sent. Only while a route runs: with none there are no hunters to show.
    fn join_snapshot(&self) -> Option<Message> {
        (self.is_host() && self.run.is_some()).then(|| self.snapshot_message())
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

    /// The name a peer's profile gave (a guest's own on the host, the host's on a guest); empty when none arrived yet.
    /// The name over a peer's figure, cleaned like a profile name (the host's room message and the guest list are not).
    /// Empty stays empty: no name, no tag.
    fn peer_name(&self, peer: u32) -> String {
        let raw = if let Some(n) = self.names.get(&peer).filter(|n| !n.is_empty()) {
            n.clone()
        } else if self.is_host() {
            self.guests.get(&peer).map(|g| g.name.clone()).unwrap_or_default()
        } else if peer == HOST_PEER_ID {
            self.host_name.clone()
        } else {
            String::new()
        };
        if raw.is_empty() {
            raw
        } else {
            clean_name(&raw)
        }
    }

    /// The other runners near the viewer that are fresh enough to draw, in peer order: with a height, those within
    /// one storey of `viewer_h`; from an old client (no height), those on `depth`. `origin` is this runner's window
    /// origin: the samples are absolute cells, so they are moved into this window before they are returned.
    pub fn remotes_at(&self, viewer_h: f32, depth: i32, origin: (i32, i32)) -> Vec<RemoteView> {
        self.remotes_where(origin, |r| match r.sample.h {
            Some(h) => (h - viewer_h).abs() <= STOREY + 0.05,
            None => r.sample.depth == depth,
        })
    }

    /// Every other runner with a fresh sample, on any floor, in peer order and moved into the window whose origin is
    /// `origin`. The dedicated room's hunters choose their target among these (there is no viewer to be near).
    pub fn remotes_all(&self, origin: (i32, i32)) -> Vec<RemoteView> {
        self.remotes_where(origin, |_| true)
    }

    fn remotes_where(&self, origin: (i32, i32), keep: impl Fn(&Remote) -> bool) -> Vec<RemoteView> {
        let now = Instant::now();
        let mut out: Vec<RemoteView> = self
            .remotes
            .iter()
            .filter(|(_, r)| keep(r) && now.duration_since(r.last_seen) < remote_timeout())
            .map(|(peer, r)| {
                let mut v = remote_view(*peer, self.peer_name(*peer), r, self.skins.get(peer).cloned().unwrap_or_else(|| SKIN_DEFAULT.into()), origin, now);
                // arm style: the sample's own flag, else the profile's word, else read from the picture once it is here
                v.slim = v.slim || self.peer_slim(*peer);
                v
            })
            .collect();
        out.sort_by_key(|r| r.peer);
        out
    }

    /// Leaves the room. The host unregisters, so every guest gets HOST_LEFT at once; a guest says goodbye and drops its
    /// link (the host also drops it after the sample timeout).
    pub fn leave(mut self) {
        if self.is_host() {
            if self.directory {
                let _ = api::unregister_room(&self.cfg, &self.code, &self.token);
            }
        } else {
            self.coop.send(HOST_PEER_ID, &Message::PeerLeave);
        }
        self.coop.close();
    }

    fn service_directory(&mut self, now: Instant, out: &mut Vec<NetEvent>) {
        if !self.is_host() || !self.directory {
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
                    // a public beat that failed is tried again at once: the directory forgets the entry after 15 s
                    Err(_) => self.next_heartbeat = retry_beat_at(&self.code, now, self.next_heartbeat),
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
                    // the public route is hostless: the relay hello keeps its entry alive, so a failed relist is no problem
                    Err(_) if super::public_route::is_public(&self.code) => {}
                    Err(_) => out.push(NetEvent::Problem("ROOM LOST // THE DIRECTORY FORGOT IT".into())),
                }
            }
        }
        if beat_due(self.heartbeat.is_some(), self.relist.is_some(), now, self.next_heartbeat) {
            self.next_heartbeat = now + Duration::from_secs_f64(super::public_route::heartbeat_seconds(&self.code));
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
            players: self.runners() + self.guests.len() as u32,
            session_active: self.run.is_some(),
            daytime: false,
            command_since: self.owner.seen,
            command_ack: self.owner.ack,
        }
    }

    fn start_relist(&mut self) {
        let cfg = self.cfg.clone();
        let mut request = RegisterRoom {
            code: self.code.clone(),
            device_id: store::device_id(&store::default_dir()),
            ..RegisterRoom::default()
        };
        // a server-side room is always a WebSocket room (headless/dedicated.rs)
        if self.dedicated {
            request.transport = "ws".into();
        }
        self.relist = Some(Pending::spawn(move || api::register_room(&cfg, &request)));
    }

    fn on_session_event(&mut self, event: SessionEvent, now: Instant, out: &mut Vec<NetEvent>) {
        match event {
            SessionEvent::Ready => {
                if !self.is_host() {
                    let hello = Message::RegisterPeer { requested_name: self.name.clone() };
                    self.coop.send(HOST_PEER_ID, &hello);
                    let profile = profile_message(self.coop.link.session(), &self.name, &self.skin, self.flags);
                    self.coop.send(HOST_PEER_ID, &profile);
                }
            }
            SessionEvent::PeerSeen(peer) => {
                self.guests.entry(peer).or_insert_with(|| Guest { name: String::new(), last_seen: now });
            }
            SessionEvent::Message { from, message } => self.on_message(from, message, now, out),
            // the WebSocket relay says a guest's socket closed: a closed tab or a dropped connection sends no PeerLeave, and a
            // PeerLeave sent as the guest closes can be lost. The guest goes now, not after the 45 s idle limit (a ghost would
            // count in the room, hold its place in the public route's line to open the room again, and keep the room "busy").
            SessionEvent::Relay(RelayEvent::PeerLeft(peer)) if self.is_host() => {
                if let Some(name) = self.forget_guest(peer).filter(|n| !n.is_empty()) {
                    out.push(NetEvent::GuestLeft(name));
                    self.send_room_and_roster();
                }
            }
            SessionEvent::Relay(RelayEvent::HostLeft(reason)) => out.push(NetEvent::HostLeft(reason)),
            SessionEvent::Relay(RelayEvent::JoinAsGuest) => out.push(NetEvent::HostLeft("THE PUBLIC ROUTE HAS A HOST // JOIN IT".into())),
            SessionEvent::Relay(RelayEvent::Error(text)) if self.is_host() && super::public_route::is_public(&self.code) && is_host_refusal(&text) => {
                out.push(NetEvent::HostLeft("THE PUBLIC ROUTE HAS A HOST // JOIN IT".into()))
            }
            SessionEvent::Relay(RelayEvent::Error(text)) => out.push(NetEvent::Problem(text)),
            SessionEvent::Relay(_) => {}
        }
    }

    fn on_message(&mut self, from: u32, message: Message, now: Instant, out: &mut Vec<NetEvent>) {
        match message {
            Message::RegisterPeer { requested_name } if self.is_host() => {
                // two guests of one room never share a guest name: the later one is given a free number
                let (name, renamed) = self.settle_guest_name(clean_name(&requested_name), from);
                let guest = self.guests.entry(from).or_insert_with(|| Guest { name: String::new(), last_seen: now });
                guest.name = name.clone();
                guest.last_seen = now;
                out.push(NetEvent::GuestJoined(name.clone()));
                if renamed {
                    self.send_rename(from, &name);
                }
                self.send_room_and_roster();
                if let Some((seed, difficulty)) = self.run.clone() {
                    self.coop.send(from, &start_message(&self.code, &seed, &difficulty));
                }
                self.send_profiles_to(from);
                if let Some(snapshot) = self.join_snapshot() {
                    self.coop.send(from, &snapshot);
                }
            }
            Message::WireProfile { profile } if self.is_host() => {
                let skin = profile_skin(&profile);
                let mut name = clean_name(profile.get("name").and_then(Value::as_str).unwrap_or_default());
                let flags = profile_flags(&profile);
                self.skins.insert(from, skin.clone());
                self.peer_flags.insert(from, flags);
                self.note_peer_slim(from, profile_slim(&profile));
                // a guest still sending the name it asked for, after the host gave it another: the given name stands
                let given = self.guests.get(&from).map(|g| g.name.clone()).unwrap_or_default();
                let (settled, tell) = self.settle_profile_name(name, &given, from);
                name = settled;
                if tell {
                    self.send_rename(from, &name);
                }
                let forward = profile_message(from, &name, &skin, flags);
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
                    // the host's profile for this very guest: a new name when the one it asked for was taken
                    let name = clean_name(profile.get("name").and_then(Value::as_str).unwrap_or_default());
                    if guest_name::is_guest_name(&self.name) && guest_name::is_guest_name(&name) && name != self.name {
                        self.name = name.clone();
                        out.push(NetEvent::Renamed(name));
                    }
                    return;
                }
                self.skins.insert(peer, profile_skin(&profile));
                self.peer_flags.insert(peer, profile_flags(&profile));
                self.note_peer_slim(peer, profile_slim(&profile));
                let name = clean_name(profile.get("name").and_then(Value::as_str).unwrap_or_default());
                self.names.insert(peer, name);
            }
            Message::PeerLeave if self.is_host() => {
                if let Some(name) = self.forget_guest(from) {
                    out.push(NetEvent::GuestLeft(name));
                    self.send_room_and_roster();
                }
            }
            // an owner command from a registered guest: the app (the room process) checks the ticket and acts
            // the public route has no owner: whoever asks is told so, and nothing reaches the app or the directory
            Message::OwnerCommand { .. } if self.is_host() && self.is_public_route() => {
                self.coop.send(from, &Message::OwnerRefused { reason: super::public_route::NO_OWNER_TEXT.to_string() });
            }
            Message::OwnerCommand { ticket, action, args } if self.is_host() => {
                let registered = self.guests.get(&from).is_some_and(|g| !g.name.is_empty());
                if registered && ticket.len() <= super::hosted::TICKET_LEN && action.len() <= super::hosted::MAX_ACTION_LEN {
                    if let Some(guest) = self.guests.get_mut(&from) {
                        guest.last_seen = now;
                    }
                    out.push(NetEvent::OwnerCommand { peer: from, ticket, action, args });
                }
            }
            // the room refused this runner's owner command
            Message::OwnerRefused { reason } if !self.is_host() => out.push(NetEvent::OwnerRefused(reason)),
            // the host's NPC teammates: a guest keeps their latest places (the host draws nothing from these)
            Message::XnpcPlace { id, state } if !self.is_host() => self.npcs.receive_place(id, &state, now),
            Message::XnpcRemove { id } if !self.is_host() => self.npcs.receive_remove(id),
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
            // a guest's taser, escape or capture report (Godot `_submit_hunter_event`): the host alone resolves it
            Message::SubmitHunterEvent { event } if self.is_host() => {
                if let Some(guest) = self.guests.get_mut(&from) {
                    guest.last_seen = now;
                }
                self.accept_guest_event(from, event, now);
            }
            // a guest's request to turn a sewer valve (Godot `_request_valve`): the host's game decides
            Message::RequestInteraction { event } if self.is_host() => {
                if let Some(guest) = self.guests.get_mut(&from) {
                    guest.last_seen = now;
                }
                self.accept_valve_request(from, &event);
            }
            Message::Room { seed, rest, .. } if !self.is_host() => {
                self.room_seed = seed.clone();
                self.host_name = rest.get("host").and_then(Value::as_str).unwrap_or_default().to_string();
                self.players = rest
                    .get("players")
                    .and_then(Value::as_array)
                    .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                self.player_peers = rest
                    .get("peers")
                    .and_then(Value::as_array)
                    .map(|ids| ids.iter().map(|i| i.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0)).collect())
                    .unwrap_or_default();
                if let Some(d) = rest.get("difficulty").and_then(Value::as_str) {
                    self.difficulty = difficulty_from_name(d).name().to_string();
                }
                self.host_dedicated = rest.get("dedicated").and_then(Value::as_bool).unwrap_or(false);
                self.room_seen = true;
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
            // the host's snapshot on join (Godot `_snapshot`): the hunters now, and the relay count kept for the app
            Message::Snapshot { state } if !self.is_host() => {
                if let Some(snapshot) = hunter_events::parse_snapshot(&state) {
                    out.push(NetEvent::Hunters(snapshot.hunters.clone()));
                    self.snapshot = Some(snapshot);
                }
            }
            // a guest's chat line: the host moderates it and rebroadcasts it (Godot `_submit_chat_payload`)
            Message::SubmitChatPayload { payload } if self.is_host() => {
                let Some(guest) = self.guests.get_mut(&from) else { return };
                guest.last_seen = now;
                let name = guest.name.clone();
                let ms = i64::try_from(now.duration_since(self.opened).as_millis()).unwrap_or(i64::MAX);
                if let Some(chat) = super::chat::relay_submission(&mut self.chat_moderation, from as i64, &name, &payload, ms) {
                    self.deliver_chat(chat);
                }
            }
            Message::ChatPayload { .. } if !self.is_host() => self.keep_chat(&message),
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

    /// Host: whether `name` is the host's or belongs to a guest other than `except` (compared without regard to case).
    fn name_in_use(&self, name: &str, except: u32) -> bool {
        name.eq_ignore_ascii_case(&self.name) || self.guests.iter().any(|(peer, g)| *peer != except && !g.name.is_empty() && g.name.eq_ignore_ascii_case(name))
    }

    /// Host: the name guest `from` goes by. A guest-shaped name that is taken, or that is only a placeholder (a runner
    /// that sent no name), is replaced by a free `GUEST_####` (the flag says so); every other name stays as it is, since
    /// two runners of one account are not for the host to tell apart.
    fn settle_guest_name(&self, name: String, from: u32) -> (String, bool) {
        if guest_name::is_guest_name(&name) && (!guest_name::is_drawn_name(&name) || self.name_in_use(&name, from)) {
            (guest_name::unique_guest_name(&|n| self.name_in_use(n, from)), true)
        } else {
            (name, false)
        }
    }

    /// Host: the name a guest's profile may show, given the name `given` it registered under. A guest-shaped name that
    /// is not the given one is replaced by it when the given one is guest-shaped too (a stale name after a re-roll: the
    /// flag says to tell the guest), and also when another runner of the room already holds it (a guest cannot pose as
    /// another guest by sending a profile). Any other name stays as it is.
    fn settle_profile_name(&self, name: String, given: &str, from: u32) -> (String, bool) {
        if !guest_name::is_guest_name(&name) || given.is_empty() || name == given {
            return (name, false);
        }
        if guest_name::is_guest_name(given) {
            (given.to_string(), true)
        } else if self.name_in_use(&name, from) {
            (given.to_string(), false)
        } else {
            (name, false)
        }
    }

    /// Host: tells guest `to` the name it goes by in this room, as a profile for its own session (an older guest ignores it).
    fn send_rename(&mut self, to: u32, name: &str) {
        let skin = self.skins.get(&to).cloned().unwrap_or_else(|| SKIN_DEFAULT.to_string());
        let flags = self.peer_flags.get(&to).copied().unwrap_or_default();
        let profile = profile_message(to, name, &skin, flags);
        self.coop.send(to, &profile);
    }

    /// Host: sends a guest that just registered every runner's profile in the room, the host's first, then each
    /// other guest's. Those that have not sent a profile yet are DEFAULT (their own profile follows).
    fn send_profiles_to(&mut self, to: u32) {
        let mut profiles = vec![profile_message(HOST_PEER_ID, &self.name, &self.skin, self.flags)];
        for (peer, guest) in &self.guests {
            if *peer != to {
                let skin = self.skins.get(peer).map_or(SKIN_DEFAULT, String::as_str);
                let flags = self.peer_flags.get(peer).copied().unwrap_or_default();
                profiles.push(profile_message(*peer, &guest.name, skin, flags));
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
        let hide_since = match (sample.hide, self.remotes.get(&peer)) {
            (None, _) => None,
            (Some(_), Some(r)) if r.hide_since.is_some() => r.hide_since,
            (Some(_), _) => Some(now),
        };
        match self.remotes.get_mut(&peer) {
            Some(r) => {
                r.track.push(interp::secs(now), sample, interp::rules());
                r.sample = sample;
                r.last_seen = now;
                r.hide_since = hide_since;
            }
            None => {
                self.remotes.insert(peer, Remote::new(sample, now, hide_since));
            }
        }
        true
    }

    fn prune(&mut self, now: Instant, out: &mut Vec<NetEvent>) {
        // a stale runner is not drawn (remotes_at checks the timeout) but its state stays for the PLAYERS list
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
                self.peer_flags.remove(&peer);
                self.skin_slims.remove(&peer);
                self.pending_captures.remove(&peer);
                self.last_taser.remove(&peer);
                // the UDP relay never says a guest left: stop sending and resending to the dead session
                self.coop.forget_peer(peer);
                if let Some(guest) = self.guests.remove(&peer) {
                    out.push(NetEvent::GuestLeft(guest.name));
                }
            }
            self.expire_captures(now);
        } else if !self.coop.link.ready() && !self.timed_out && now.duration_since(self.opened) >= JOIN_TIMEOUT {
            self.timed_out = true;
            out.push(NetEvent::Problem("NO ANSWER FROM THE ROOM'S RELAY".into()));
        }
    }

    fn send_due(&mut self, sample: Option<Sample>, now: Instant) {
        let rules = interp::rules();
        let t = interp::secs(now);
        let early = match (&sample, &self.last_sent) {
            (Some(s), Some((_, last))) => {
                (s.x - last.x).hypot(s.y - last.y) > rules.send_early_move_m
                    || ((s.yaw - last.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs() > rules.send_early_yaw_rad
                    || s.clip != last.clip
                    || s.hide.is_some() != last.hide.is_some()
                    || s.held != last.held
            }
            (Some(_), None) => true,
            _ => false,
        };
        if !self.send_clock.due(t, early, rules) {
            return;
        }
        let sample = sample.map(|mut s| {
            // the measured ground velocity: the move since the last sample over the time between them
            if let Some((lt, last)) = &self.last_sent {
                let dt = (t - lt) as f32;
                let step = (s.x - last.x).hypot(s.y - last.y);
                // a respawn, ladder snap or teleport is a jump, not a speed: it carries no velocity
                if last.depth == s.depth && dt > 1e-3 && dt < 0.5 && step <= rules.snap_distance_m && step / dt <= rules.max_speed_mps {
                    s.vx = (s.x - last.x) / dt;
                    s.vz = (s.y - last.y) / dt;
                }
            }
            s
        });
        self.seq = self.seq.wrapping_add(1);
        let packet = match sample {
            Some(mut s) => {
                s.seq = self.seq;
                self.last_sent = Some((t, s));
                s.to_json()
            }
            None => {
                self.last_sent = None;
                Value::Null
            }
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
/// Empty becomes `GUEST_0000`: a guest-shaped name nobody draws ([`guest_name::placeholder`]), so a name that cleans to
/// nothing still reads as a guest (and the host gives it a real number if it registers).
pub fn clean_name(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(MAX_NAME)
        .collect();
    let name = kept.trim().to_ascii_uppercase();
    if name.is_empty() {
        guest_name::placeholder(0)
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
    crate::net::runner_figure::update_remote_doors(g, remotes);
    if remotes.is_empty() {
        return;
    }
    crate::net::runner_figure::draw_runners(g, a, mat, rt, remotes);
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_failed_public_beat_retries_at_once_and_a_stalled_beat_delays_the_next_by_its_timeout_only() {
        let t0 = Instant::now();
        let public = super::super::public_route::rules().code.clone();
        let interval = Duration::from_secs_f64(super::super::public_route::heartbeat_seconds(&public));
        let timeout = Duration::from_secs(super::super::public_route::HEARTBEAT_TIMEOUT_SECONDS);
        // beat sent at t0 stalls until its timeout: the schedule has passed, so the next goes out the moment it ends
        let next = t0 + interval;
        assert!(!beat_due(true, false, t0 + interval, next), "one beat at a time");
        assert!(beat_due(false, false, t0 + timeout.max(interval), next));
        // worst case gap between two beats reaching the directory stays under its 15 s TTL
        let gap = interval.max(timeout) + timeout;
        assert!(gap.as_secs_f64() < super::super::public_route::DIRECTORY_PUBLIC_TTL_SECONDS, "{gap:?}");
        // a failure retries now for the public code, and leaves a private room on its schedule
        assert_eq!(retry_beat_at(&public, t0 + timeout, t0 + interval), t0 + timeout);
        assert_eq!(retry_beat_at("ABCDE", t0 + timeout, t0 + interval), t0 + interval);
        assert_eq!(retry_beat_at(&public.to_lowercase(), t0 + timeout, t0 + interval), t0 + interval, "exact match");
        assert!(!beat_due(false, true, t0 + interval, next), "no beat during a relist");
    }

    use super::*;
    use crate::net::relay::{decode, encode, frame};
    use std::net::UdpSocket;

    pub(super) fn sample(seq: u32, x: f32, y: f32) -> Sample {
        Sample {
            seq,
            depth: 1,
            h: Some(5.0),
            x,
            y,
            yaw: 0.5,
            moving: true,
            sprinting: false,
            crouching: true,
            spectating: false,
            game_over: false,
            escape_active: false,
            clip: None,
            vy: 0.0,
            vx: 0.0,
            vz: 0.0,
            slim: false,
            hide: None,
            held: None,
            flash_pitch: None,
            look_pitch: 0.0,
            emote: None,
            emote_seq: None,
        }
    }

    #[test]
    fn send_due_attaches_the_velocity_and_drops_it_after_a_jump() {
        // the shared timeline starts before `base`, so no sample time is negative
        interp::secs(Instant::now());
        let base = Instant::now();
        let ms = |n: u64| base + Duration::from_millis(n);
        let mut s = offline_session(false, "GUEST");
        let sent = |s: &NetSession| s.last_sent.unwrap().1;
        s.send_due(Some(sample(0, 0.0, 0.0)), ms(0));
        assert_eq!((s.seq, sent(&s).vx), (1, 0.0), "the first sample has nothing to measure");
        s.send_due(Some(sample(0, 0.17, 0.0)), ms(34));
        assert!((sent(&s).vx - 5.0).abs() < 0.1, "5 m/s: {}", sent(&s).vx);
        s.send_due(Some(sample(0, 20.0, 0.0)), ms(68));
        assert_eq!((s.seq, sent(&s).vx), (3, 0.0), "a teleport carries no velocity");
        let mut up = sample(0, 20.1, 0.0);
        up.depth = 2;
        s.send_due(Some(up), ms(102));
        assert_eq!((s.seq, sent(&s).vx), (4, 0.0), "another floor carries none either");
        s.send_due(Some(sample(0, 20.8, 0.0)), ms(104));
        assert_eq!(s.seq, 4, "an early send keeps the minimum gap");
        s.send_due(Some(sample(0, 20.8, 0.0)), ms(122));
        assert_eq!(s.seq, 5, "and goes out once the gap has passed");
        s.send_due(Some(sample(0, 20.9, 0.0)), ms(155));
        assert_eq!(s.seq, 6);
        assert!((sent(&s).vx - 3.0).abs() < 0.2, "{}", sent(&s).vx);
    }

    #[test]
    fn send_due_sends_early_when_the_held_item_changes() {
        // the shared timeline starts before `base`, so no sample time is negative
        interp::secs(Instant::now());
        let base = Instant::now();
        let ms = |n: u64| base + Duration::from_millis(n);
        let mut s = offline_session(false, "GUEST");
        s.send_due(Some(sample(0, 0.0, 0.0)), ms(0));
        assert_eq!(s.seq, 1);
        s.send_due(Some(sample(0, 0.0, 0.0)), ms(20));
        assert_eq!(s.seq, 1, "nothing changed, so the step is not due yet");
        s.send_due(Some(sample(0, 0.0, 0.0).with_held(Some(Item::Axe))), ms(20));
        assert_eq!(s.seq, 2, "a new item goes out ahead of the step");
        assert_eq!(s.last_sent.unwrap().1.held, Some(Item::Axe));
        s.send_due(Some(sample(0, 0.0, 0.0).with_held(None)), ms(25));
        assert_eq!(s.seq, 2, "the minimum gap still holds");
        s.send_due(Some(sample(0, 0.0, 0.0).with_held(None)), ms(45));
        assert_eq!(s.seq, 3, "putting it away is an early send too");
        assert_eq!(s.last_sent.unwrap().1.held, None);
    }

    #[test]
    fn a_held_item_rides_the_sample_and_an_unknown_one_is_none() {
        for item in [Item::Flashlight, Item::Taser, Item::Crowbar, Item::Axe, Item::Battery, Item::Stim, Item::MusicBox] {
            let s = sample(2, 1.0, 2.0).with_held(Some(item));
            assert_eq!(Sample::from_json(&s.to_json()), Some(s), "{item:?} round trips");
        }
        let none = sample(2, 1.0, 2.0).to_json();
        assert!(none.get("held").is_none(), "nothing held sends no key, so an older peer sees the old packet");
        let mut v = sample(3, 1.0, 2.0).to_json();
        v["held"] = json!("lockpick");
        let s = Sample::from_json(&v).expect("an unknown item never refuses the sample");
        assert_eq!((s.held, s.x), (None, 1.0));
        v["held"] = json!(7);
        assert_eq!(Sample::from_json(&v).map(|s| s.held), Some(None), "a non-string is no item");
        let host_view = Sample::from_json(&sample(4, 1.0, 2.0).with_held(Some(Item::Axe)).to_json()).unwrap();
        assert_eq!(host_view.held, Some(Item::Axe));
    }

    #[test]
    fn a_lit_flashlight_rides_the_sample_with_its_aim() {
        // off: no key, so an older peer sees the old packet
        let off = sample(5, 1.0, 2.0);
        assert!(off.to_json().get("flash").is_none());
        assert_eq!(Sample::from_json(&off.to_json()).unwrap().flash_pitch, None);
        // lit and level: the flag only
        let mut lit = sample(6, 1.0, 2.0);
        lit.flash_pitch = Some(0.0);
        let v = lit.to_json();
        assert_eq!(v["flash"], json!(true));
        assert!(v.get("pitch").is_none(), "a level aim sends no pitch");
        assert_eq!(Sample::from_json(&v).unwrap().flash_pitch, Some(0.0));
        // lit and aimed up: the pitch rides along
        lit.flash_pitch = Some(0.4);
        assert_eq!(Sample::from_json(&lit.to_json()).unwrap().flash_pitch, Some(0.4));
        // a hostile pitch is clamped to the limit
        let mut hostile = lit.to_json();
        hostile["pitch"] = json!(99.0);
        assert_eq!(Sample::from_json(&hostile).unwrap().flash_pitch, Some(FLASH_PITCH_LIMIT));
    }

    #[test]
    fn an_aim_pitch_rides_the_sample_whether_or_not_the_lamp_is_on() {
        // level and off: no key, so an older peer sees the old packet and the head stays level
        let level = sample(7, 1.0, 2.0);
        assert!(level.to_json().get("look").is_none());
        assert_eq!(Sample::from_json(&level.to_json()).unwrap().look_pitch, 0.0);
        // aimed up with the lamp off: the head still tilts
        let mut up = sample(8, 1.0, 2.0);
        up.look_pitch = 0.6;
        let v = up.to_json();
        assert!((v["look"].as_f64().unwrap() - 0.6).abs() < 1e-4, "{}", v["look"]);
        assert!(v.get("flash").is_none(), "the lamp stays off");
        assert_eq!(Sample::from_json(&v).unwrap().look_pitch, 0.6);
        // a hostile aim is clamped like the flashlight's
        let mut hostile = up.to_json();
        hostile["look"] = json!(-99.0);
        assert_eq!(Sample::from_json(&hostile).unwrap().look_pitch, -LOOK_PITCH_LIMIT);
    }

    #[test]
    fn an_emote_rides_the_sample_and_an_older_sample_has_none() {
        let plain = sample(1, 0.0, 0.0);
        let v = plain.to_json();
        assert!(v.get("emote").is_none() && v.get("emote_seq").is_none(), "no emote, no keys");
        let waving = plain.with_emote(Some(("wave", 3)));
        let got = Sample::from_json(&waving.to_json()).unwrap();
        assert_eq!(got.emote.map(|t| t.as_str().to_string()), Some("wave".to_string()));
        assert_eq!(got.emote_seq, Some(3));
        assert_eq!(Sample::from_json(&plain.to_json()).unwrap().emote, None);
        let mut hostile = waving.to_json();
        hostile["emote"] = json!("a very long emote id that is far too long");
        assert_eq!(Sample::from_json(&hostile).unwrap().emote, None, "an over-long id is dropped");
        hostile["emote"] = json!("no spaces!");
        assert_eq!(Sample::from_json(&hostile).unwrap().emote, None);
    }

    #[test]
    fn sample_json_round_trips_and_ignores_keep_alives() {
        let s = Sample {
            seq: 9,
            depth: -2,
            h: Some(-9.5),
            x: 12.5,
            y: -3.25,
            yaw: 1.57,
            moving: true,
            sprinting: true,
            crouching: false,
            spectating: false,
            game_over: true,
            escape_active: true,
            clip: Some(Pose::Climb),
            vy: -3.5,
            vx: 2.5,
            vz: -1.0,
            slim: true,
            hide: Some(HideSpot { cell: (-40, 17), facing: (0, -1) }),
            held: Some(Item::Crowbar),
            flash_pitch: Some(-0.25),
            look_pitch: 0.3,
            emote: EmoteTag::new("wave"),
            emote_seq: Some(7),
        };
        assert_eq!(Sample::from_json(&s.to_json()), Some(s));
        assert_eq!(Sample::from_json(&Value::Null), None, "a keep-alive is not a sample");
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": "far", "y": 0})), None);
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": 1e39, "y": 0})), None, "infinity is refused");
        let hostile = Sample::from_json(&json!({"seq": 1, "depth": 0, "x": 0.0, "y": 0.0, "vx": 1e38, "vz": -1e38})).unwrap();
        assert!(hostile.vx.hypot(hostile.vz) <= interp::rules().max_speed_mps + 1e-3, "a hostile velocity is capped");
        assert_eq!(Sample::from_json(&json!({"seq": 1, "depth": 0, "x": 0.0, "y": 0.0}))
            .map(|s| (s.yaw, s.moving)), Some((0.0, false)), "missing flags read as false");
    }

    #[test]
    fn an_older_sample_has_no_body_state_and_a_newer_one_survives_unknown_names() {
        let old = json!({"seq": 4, "depth": 1, "x": 2.0, "y": 3.0, "moving": true, "sprinting": true});
        let s = Sample::from_json(&old).expect("an older client's sample still parses");
        assert_eq!((s.clip, s.vy, s.slim, s.hide), (None, 0.0, false, None));
        let v = sample(1, 0.0, 0.0).to_json();
        for key in ["clip", "vy", "vx", "vz", "slim", "hide"] {
            assert!(v.get(key).is_none(), "a sample with no body state sends no {key}");
        }
        let newer = json!({"seq": 4, "depth": 1, "x": 2.0, "y": 3.0, "clip": "backflip", "hide": [1, 2]});
        let s = Sample::from_json(&newer).expect("an unknown pose name never rejects a packet");
        assert_eq!((s.clip, s.hide), (None, None), "an unknown pose and a short locker are ignored");
        for p in Pose::ALL {
            let s = sample(1, 0.0, 0.0).with_body(p, 0.0, false, None);
            assert_eq!(Sample::from_json(&s.to_json()).and_then(|s| s.clip), Some(p), "{p:?} round trips");
        }
    }

    #[test]
    fn the_body_state_reaches_the_remote_view() {
        let now = Instant::now();
        let smp = sample(1, 13.0, 14.0).with_body(Pose::Climb, 1.5, true, Some(HideSpot { cell: (13, 14), facing: (1, 0) }));
        let r = Remote::new(smp, now, Some(now));
        let v = remote_view(5, "Ann".into(), &r, "DEFAULT".into(), (10, 10), now);
        assert_eq!((v.clip, v.vy, v.slim), (Some(Pose::Climb), 1.5, true));
        assert_eq!((v.x, v.y), (3.0, 4.0), "the position moves into this window");
        assert_eq!(v.hide, Some(HideSpot { cell: (3, 4), facing: (1, 0) }), "so does the locker");
        assert!(v.hide_since.is_some());
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
        match profile_message(3, "RUNNER", &hash, SkinFlags::default()) {
            Message::WireProfile { profile } => {
                assert_eq!(profile.get("skin").and_then(Value::as_str), Some(hash.as_str()));
                assert_eq!(profile_skin(&profile), hash);
                assert_eq!(profile_skin(&Map::new()), SKIN_DEFAULT, "a profile without a skin is DEFAULT");
            }
            _ => panic!("a profile message is a WireProfile"),
        }
    }

    #[test]
    fn the_profile_carries_skin_slim_and_skin_copy_and_a_peer_reads_them_back() {
        let hash = "d".repeat(64);
        for flags in [
            SkinFlags { slim: true, copy: true },
            SkinFlags { slim: true, copy: false },
            SkinFlags { slim: false, copy: true },
            SkinFlags::default(),
        ] {
            let Message::WireProfile { profile } = profile_message(4, "ALEX", &hash, flags) else {
                panic!("a profile message is a WireProfile")
            };
            assert_eq!(profile.get("skin_slim").and_then(Value::as_bool), Some(flags.slim), "the wire names Godot's skin_slim");
            assert_eq!(profile.get("skin_copy").and_then(Value::as_bool), Some(flags.copy), "the wire names Godot's skin_copy");
            assert_eq!(profile_flags(&profile), flags);
            assert_eq!(profile_slim(&profile), Some(flags.slim));
        }
        assert_eq!(profile_flags(&Map::new()), SkinFlags::default(), "an old client's profile reads as classic and not copyable");
        assert_eq!(profile_slim(&Map::new()), None, "an old client's profile names no arm style");
        let mut odd = Map::new();
        odd.insert("skin_slim".into(), json!("yes"));
        assert_eq!(profile_slim(&odd), None, "only a boolean counts");
    }

    #[test]
    fn a_remote_runners_arm_style_follows_its_profile_else_its_picture() {
        let hash = "d".repeat(64);
        let mut host = offline_session(true, "HOST");
        host.skins.insert(7, hash.clone());
        host.remotes.insert(7, Remote::new(sample(1, 0.0, 0.0), Instant::now(), None));
        assert!(!host.remotes_at(0.0, 1, (0, 0))[0].slim, "no word and no picture: classic");
        host.note_peer_slim(7, Some(true));
        assert!(host.remotes_at(0.0, 1, (0, 0))[0].slim, "the profile says slim");
        host.note_peer_slim(7, Some(false));
        assert!(!host.remotes_at(0.0, 1, (0, 0))[0].slim, "the profile says classic");
        host.note_peer_slim(7, None);
        assert!(!host.skin_slims.contains_key(&7), "a profile that names none clears the old word");
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
    fn a_republish_happens_only_when_the_published_skin_changes() {
        let hash = "b".repeat(64);
        assert_eq!(republished_skin(SKIN_DEFAULT, &hash, &[]), None, "an unuploaded skin stays DEFAULT: no change");
        assert_eq!(republished_skin(SKIN_DEFAULT, &hash, &[hash.clone()]), Some(hash.clone()), "uploaded: republish the id");
        assert_eq!(republished_skin(&hash, &hash, &[hash.clone()]), None, "the same id is not sent again");
        assert_eq!(republished_skin(&hash, SKIN_DEFAULT, &[hash.clone()]), Some(SKIN_DEFAULT.to_string()), "back to DEFAULT");
        assert_eq!(republished_skin(SKIN_DEFAULT, "night-owl", &["NIGHTOWL".into()]), None, "only a skin hash republishes");
    }

    #[test]
    fn a_chosen_skin_is_published_only_as_a_skin_hash_the_website_holds() {
        assert_eq!(publishable_skin("night-owl"), SKIN_DEFAULT, "a non-hash id is never published");
        assert_eq!(publishable_skin(SKIN_DEFAULT), SKIN_DEFAULT);
    }

    #[test]
    fn a_host_that_uploads_after_opening_republishes_its_skin() {
        let hash = "f".repeat(64);
        let mut host = offline_session(true, "HOST");
        host.skin = SKIN_DEFAULT.into();
        host.set_skin(&hash, &[]);
        assert_eq!(host.published_skin(), SKIN_DEFAULT, "not uploaded yet: still DEFAULT");
        host.set_skin(&hash, &[hash.clone()]);
        assert_eq!(host.published_skin(), hash, "uploaded: the new id is published");
        host.set_skin(SKIN_DEFAULT, &[hash.clone()]);
        assert_eq!(host.published_skin(), SKIN_DEFAULT, "a DEFAULT choice publishes DEFAULT");
    }

    #[test]
    fn a_guest_publishes_its_chosen_skin_id_once_the_website_holds_it() {
        let hash = "9".repeat(64);
        let mut guest = offline_session(false, "GUEST");
        guest.skin = SKIN_DEFAULT.into();
        guest.set_skin(&hash, &[]);
        assert_eq!(guest.published_skin(), SKIN_DEFAULT);
        guest.set_skin(&hash, &[hash.clone()]);
        assert_eq!(guest.published_skin(), hash);
    }

    #[test]
    fn copy_skin_offers_only_runners_who_allow_it_with_their_arm_style() {
        let (hash, other) = ("7".repeat(64), "6".repeat(64));
        let mut host = offline_session(true, "HOST");
        host.guests.insert(7, Guest { name: "BOB".into(), last_seen: Instant::now() });
        host.guests.insert(8, Guest { name: "EVE".into(), last_seen: Instant::now() });
        host.skins.insert(7, hash.clone());
        host.skins.insert(8, other);
        host.peer_flags.insert(7, SkinFlags { slim: true, copy: true });
        host.peer_flags.insert(8, SkinFlags { slim: false, copy: false });
        assert_eq!(host.copy_candidates(), vec![("BOB".to_string(), hash.clone(), true)], "EVE does not allow copies");
        host.peer_flags.insert(8, SkinFlags { slim: false, copy: true });
        host.skins.insert(8, SKIN_DEFAULT.into());
        assert_eq!(host.copy_candidates().len(), 1, "the default skin is nothing to copy");
        host.set_skin_flags(SkinFlags { slim: true, copy: false });
        assert_eq!(host.flags, SkinFlags { slim: true, copy: false }, "the runner's own flags are kept for its profile");
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
        assert_eq!(clean_name("!!!"), "GUEST_0000");
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
            done.0 = host.remotes_at(5.0, 1, (0, 0)).iter().any(|r| r.peer == 42 && (r.x - 7.5).abs() < 1e-4 && (r.y - 1.25).abs() < 1e-4 && r.skin == "NIGHT");
            done.1 = guest.remotes_at(5.0, 1, (0, 0)).iter().any(|r| r.peer == HOST_PEER_ID && (r.x - 3.0).abs() < 1e-4 && r.skin == "DAWN");
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

    /// A tiny http server for the skin fetch: the first `misses` requests are answered 404 (the file is not uploaded yet),
    /// every later one serves `body`. Stops when the returned flag is set.
    fn skin_server(body: Vec<u8>, misses: usize) -> (u16, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        std::thread::spawn(move || {
            let mut served = 0usize;
            while !stop2.load(Ordering::SeqCst) {
                let Ok((mut conn, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                conn.set_nonblocking(false).unwrap();
                conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let (mut got, mut b) = (Vec::new(), [0u8; 2048]);
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match conn.read(&mut b) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&b[..n]),
                    }
                }
                served += 1;
                let reply = if served <= misses {
                    b"HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}".to_vec()
                } else {
                    [format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes(), body.clone()].concat()
                };
                let _ = conn.write_all(&reply);
            }
        });
        (port, stop)
    }

    /// Two peers over one fake relay: each announces its skin id once the website holds it, the other sees the id on the
    /// remote body, fetches the picture (the first try finds nothing, the retry finds it), verifies and decodes it.
    #[test]
    fn two_peers_announce_fetch_and_apply_a_skin() {
        use crate::shared::skins;
        let rgba: Vec<u8> = (0..64 * 64).flat_map(|i| [(i % 251) as u8, 90, 160, 255]).collect();
        let png = skins::encode_png(&rgba, 64, 64);
        let hash = skins::sha256_hex(&png);
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        relay.set_nonblocking(true).unwrap();
        let port = relay.local_addr().unwrap().port();
        let mut host = NetSession::new(
            BackendConfig::loopback(18105),
            "ABCDE".into(),
            Role::Host,
            CoopSession::new(RelayLink::host("ABCDE", "tok", "127.0.0.1", port).unwrap()),
            "tok".into(),
            "ROOM-ABCDE-0123".into(),
            "HOST".into(),
            SKIN_DEFAULT.into(),
        );
        let mut guest = NetSession::new(
            BackendConfig::loopback(18105),
            "ABCDE".into(),
            Role::Guest,
            CoopSession::new(RelayLink::guest("ABCDE", "127.0.0.1", port, "").unwrap()),
            String::new(),
            String::new(),
            "GUEST".into(),
            SKIN_DEFAULT.into(),
        );
        // the guest's upload has not finished: it still announces DEFAULT; the host's has
        guest.set_skin(&hash, &[]);
        host.set_skin(&hash, &[hash.clone()]);
        assert_eq!((host.published_skin(), guest.published_skin()), (hash.as_str(), SKIN_DEFAULT));
        let (mut host_addr, mut guest_addr) = (None, None);
        let mut buf = [0u8; 2048];
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut guest_sees_host, mut host_sees_guest_default, mut guest_republished) = (false, false, false);
        while Instant::now() < deadline && !(guest_sees_host && guest_republished) {
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
            host.tick(Some(sample(0, 3.0, -2.0)));
            guest.tick(Some(sample(0, 7.5, 1.25)));
            guest_sees_host = guest_sees_host || guest.remotes_at(5.0, 1, (0, 0)).iter().any(|r| r.peer == HOST_PEER_ID && r.skin == hash);
            host_sees_guest_default = host_sees_guest_default || host.remotes_at(5.0, 1, (0, 0)).iter().any(|r| r.peer == 42 && r.skin == SKIN_DEFAULT);
            if host_sees_guest_default && !guest_republished {
                // the upload finishes: the guest announces the id (the app's periodic check does this)
                guest.set_skin(&hash, &[hash.clone()]);
                guest_republished = guest.published_skin() == hash;
                assert!(guest_republished, "an uploaded skin is announced");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host_sees_guest_default, "before its upload the guest shows as DEFAULT on the host");
        assert!(guest_sees_host, "the guest sees the host's skin id on the remote body");
        // the host sees the guest's id after the republish
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut host_sees_guest = false;
        while Instant::now() < deadline && !host_sees_guest {
            while let Ok((n, from)) = relay.recv_from(&mut buf) {
                let f = decode(&buf[..n]).unwrap();
                if f.kind == frame::GUEST_DATA {
                    relay.send_to(&encode(frame::GUEST_DATA, 42, "ABCDE", &f.payload).unwrap(), host_addr.unwrap()).unwrap();
                } else if f.kind == frame::HOST_DATA {
                    relay.send_to(&encode(frame::HOST_DATA, 42, "ABCDE", &f.payload).unwrap(), guest_addr.unwrap()).unwrap();
                }
                let _ = from;
            }
            host.tick(Some(sample(0, 3.0, -2.0)));
            guest.tick(Some(sample(0, 7.5, 1.25)));
            host_sees_guest = host.remotes_at(5.0, 1, (0, 0)).iter().any(|r| r.peer == 42 && r.skin == hash);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(host_sees_guest, "the host sees the guest's skin id once it is announced");

        // fetch: the first try finds nothing (not uploaded yet), the retry gets the picture; verified, cached, decoded
        let cfg = skins::SkinConfig { host_base: "https://skins.invalid".to_string(), ..skins::config().unwrap() };
        let (server_port, stop) = skin_server(png.clone(), 2);
        let base = format!("http://127.0.0.1:{server_port}");
        let dir = std::env::temp_dir().join(format!("lr_two_peer_skin_{}", crate::shared::task::process_id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!skins::fetch_skin_blocking(&cfg, &base, &dir, &hash), "not on the server yet: nothing cached");
        assert!(skins::read_skin_in(&[Some(dir.clone())], &hash).is_none());
        assert!(skins::fetch_skin_blocking(&cfg, &base, &dir, &hash), "the retry finds it");
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let cached = skins::read_skin_in(&[Some(dir.clone())], &hash).expect("the verified file is cached");
        let pixels = skins::remote_skin_rgba(&cached).expect("the cached skin decodes");
        assert_eq!(pixels.len(), 64 * 64 * 4);
        let _ = std::fs::remove_dir_all(&dir);
        host.coop.close();
        guest.coop.close();
    }

    /// KT: "runner (skinless guest) doesn't see my skin". A guest with no account (no session, no skin of its own) and no
    /// cache folder (the browser) is in a room whose host wears a skin the website holds. The figure exists before the host's
    /// profile arrives (DEFAULT meanwhile); when the id arrives the guest fetches it with no session and shows it.
    #[test]
    fn a_guest_with_no_session_and_no_cache_folder_shows_the_hosts_skin_once_it_arrives_and_is_fetched() {
        use crate::net::runner_figure::{pick, Pick};
        use crate::shared::skins;
        let rgba: Vec<u8> = (0..64 * 64).flat_map(|i| [(i % 241) as u8, 33, 200, 255]).collect();
        let png = skins::encode_png(&rgba, 64, 64);
        let hash = skins::sha256_hex(&png);
        let now = Instant::now();
        let mut host = offline_session(true, "KYTTU");
        let mut guest = offline_session(false, "RUNNER");
        assert!(guest.token.is_empty(), "the guest is not signed in");
        host.set_skin(&hash, &[hash.clone()]);
        assert_eq!(host.published_skin(), hash, "the host announces the skin the website holds");

        // the host's figure is on the guest's screen before its profile has arrived
        assert!(guest.store_remote(HOST_PEER_ID, sample(1, 3.0, -2.0), now));
        let view = guest.remotes_at(5.0, 1, (0, 0)).into_iter().find(|r| r.peer == HOST_PEER_ID).expect("the host is drawn");
        assert_eq!(view.skin, SKIN_DEFAULT, "DEFAULT until the profile arrives");
        let have = |id: &str| skins::read_skin_in(&[None, None], id).is_some();
        assert_eq!(pick(&view.skin, &have), Pick::Atlas);

        // the profile arrives late: the guest applies it to the figure that already exists
        let mut out = Vec::new();
        guest.on_message(HOST_PEER_ID, profile_message(HOST_PEER_ID, "KYTTU", host.published_skin(), SkinFlags { slim: false, copy: true }), now, &mut out);
        let view = guest.remotes_at(5.0, 1, (0, 0)).into_iter().find(|r| r.peer == HOST_PEER_ID).unwrap();
        assert_eq!(view.skin, hash, "the late hash is applied");
        assert_eq!(pick(&view.skin, &have), Pick::Atlas, "the picture is not here yet: the default body shows meanwhile");

        // the fetch: a server like the Node backend (the legacy file route only, public, no session asked for)
        let cfg = skins::SkinConfig { host_base: "https://skins.invalid".to_string(), ..skins::config().unwrap() };
        let (server_port, stop) = skin_server(png.clone(), 1);
        assert!(skins::start_download(cfg, format!("http://127.0.0.1:{server_port}"), None, &hash), "the guest starts a download with no cache folder");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !have(&hash) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(have(&hash), "the guest ends up holding the host's skin");
        assert_eq!(pick(&view.skin, &have), Pick::Skin(hash.clone()), "and the figure wears it");
        let bytes = skins::read_skin_in(&[None, None], &hash).unwrap();
        assert_eq!(skins::remote_skin_rgba(&bytes).unwrap().len(), 64 * 64 * 4);
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
    fn a_late_joiner_is_sent_the_hosts_hunters_and_sabotaged_relays_during_a_run() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        assert!(host.join_snapshot().is_none(), "no route running: nothing to send on join");
        host.start_run("ROOM-ABCDE-0123", "STANDARD");
        let frame = vec![json!({"index": 0, "id": "aki", "state": "CHASE", "position": [1.0, 2.0]})];
        host.send_hunter_frame(frame.clone(), Difficulty::Standard, None, now);
        host.set_relays_sabotaged(2);
        let sent = host.join_snapshot().expect("a run is on: the joiner is sent the snapshot");
        let mut guest = offline_session(false, "LATE");
        let mut out = Vec::new();
        guest.on_message(HOST_PEER_ID, sent, now, &mut out);
        assert_eq!(out, vec![NetEvent::Hunters(frame.clone())], "the hunters now show at once");
        assert_eq!(
            guest.take_snapshot(),
            Some(hunter_events::Snapshot { hunters: frame, relays_sabotaged: 2 }),
            "the relay count is kept for the app"
        );
        assert_eq!(guest.take_snapshot(), None, "a snapshot is taken once");
    }

    #[test]
    fn only_a_host_sends_a_snapshot_and_only_a_guest_takes_one() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        host.start_run("ROOM-ABCDE-0123", "STANDARD");
        let mut out = Vec::new();
        host.on_message(2, host.snapshot_message(), now, &mut out);
        assert!(out.is_empty(), "a host ignores a snapshot sent to it");
        assert!(host.take_snapshot().is_none());
        let mut guest = offline_session(false, "GUEST");
        assert!(guest.join_snapshot().is_none(), "a guest never sends a snapshot");
        guest.on_message(HOST_PEER_ID, Message::Snapshot { state: Map::new() }, now, &mut out);
        assert!(out.is_empty(), "a snapshot without its schema is ignored");
        assert!(guest.take_snapshot().is_none());
    }

    #[test]
    fn a_guests_relay_count_is_not_set_by_the_guest() {
        let mut guest = offline_session(false, "GUEST");
        guest.set_relays_sabotaged(5);
        assert_eq!(guest.snapshot_message(), Message::Snapshot { state: hunter_events::snapshot_state(&[], 0) });
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
    pub(super) fn guest_report(what: &str) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("what".into(), json!(what));
        m
    }

    #[test]
    fn a_guests_taser_reaches_the_host_with_its_peer_and_a_clamped_slot() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        let mut high = guest_report("taser");
        high.insert("slot".into(), json!(99));
        host.on_message(2, Message::SubmitHunterEvent { event: high }, now, &mut out);
        let mut low = guest_report("taser");
        low.insert("slot".into(), json!(-7));
        host.on_message(3, Message::SubmitHunterEvent { event: low }, now, &mut out);
        let mut none = guest_report("taser");
        none.insert("slot".into(), json!("nope"));
        // peer 2's second intent comes after the data's least gap
        let later = now + hunter_events::rules().guest_taser_gap + Duration::from_millis(1);
        host.on_message(2, Message::SubmitHunterEvent { event: none }, later, &mut out);
        let got = host.take_guest_events();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0]["what"], json!("taser"));
        assert_eq!(got[0]["peer_id"], json!(2));
        assert_eq!(got[0]["slot"], json!(63), "the slot is clamped to the top of the inventory");
        assert_eq!(got[1]["peer_id"], json!(3));
        assert_eq!(got[1]["slot"], json!(-1), "a low slot clamps to -1");
        assert_eq!(got[2]["slot"], json!(-1), "a slot that is not a number is -1");
        assert!(host.take_guest_events().is_empty(), "taking the events empties the queue");
        assert!(out.is_empty(), "a guest submission is not a room event");
    }

    #[test]
    fn a_capture_report_is_taken_only_for_a_guest_the_host_captured_and_only_once() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        // no capture was sent to peer 2: its escape report is dropped
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("escaped") }, now, &mut out);
        assert!(host.take_guest_events().is_empty());
        host.expect_capture(2);
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("caught") }, now, &mut out);
        let got = host.take_guest_events();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["what"], json!("caught"));
        assert_eq!(got[0]["peer_id"], json!(2));
        // the expectation is used up: a second report is dropped
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("caught") }, now, &mut out);
        assert!(host.take_guest_events().is_empty(), "the report is one-shot");
    }

    #[test]
    fn an_escape_is_resolved_against_the_hunter_and_place_the_host_recorded_not_the_guests_claim() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        host.expect_capture_by(2, Some("aki".into()), Some((7.5, 8.5)));
        let mut claim = guest_report("escaped");
        claim.insert("hunter_id".into(), json!("someone_else"));
        claim.insert("entity_id".into(), json!("another"));
        claim.insert("x".into(), json!(99.0));
        claim.insert("y".into(), json!(-4.0));
        host.on_message(2, Message::SubmitHunterEvent { event: claim.clone() }, now, &mut out);
        let got = host.take_guest_events();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["hunter_id"], json!("aki"));
        assert_eq!((got[0]["x"].as_f64(), got[0]["y"].as_f64()), (Some(7.5), Some(8.5)));
        assert!(!got[0].contains_key("entity_id"));
        // a capture with no record carries no claim through either
        host.expect_capture(2);
        host.on_message(2, Message::SubmitHunterEvent { event: claim }, now, &mut out);
        let got = host.take_guest_events();
        assert!(!got[0].contains_key("hunter_id") && !got[0].contains_key("x") && !got[0].contains_key("y"));
    }

    #[test]
    fn a_capture_expectation_belongs_to_its_guest_and_each_report_kind_uses_it() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        host.expect_capture(2);
        // another guest cannot use peer 2's expectation
        host.on_message(3, Message::SubmitHunterEvent { event: guest_report("escaped") }, now, &mut out);
        assert!(host.take_guest_events().is_empty());
        let mut blocked = guest_report("capture_blocked");
        blocked.insert("hunter_id".into(), json!("aki"));
        host.on_message(2, Message::SubmitHunterEvent { event: blocked }, now, &mut out);
        let got = host.take_guest_events();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["what"], json!("capture_blocked"));
        assert_eq!(got[0]["hunter_id"], json!("aki"), "the report keeps its own fields");
        assert_eq!(got[0]["peer_id"], json!(2));
    }

    #[test]
    fn an_unknown_report_kind_does_not_use_up_the_expectation() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        host.expect_capture(2);
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("teleported") }, now, &mut out);
        assert!(host.take_guest_events().is_empty());
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("escaped") }, now, &mut out);
        assert_eq!(host.take_guest_events().len(), 1, "the expectation is still there for a real report");
    }

    #[test]
    fn a_guest_that_leaves_loses_its_pending_capture() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        host.expect_capture(2);
        host.on_message(2, Message::PeerLeave, now, &mut out);
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("caught") }, now, &mut out);
        assert!(host.take_guest_events().is_empty(), "a left guest's report is not accepted");
    }

    #[test]
    fn a_host_sender_and_a_guest_never_apply_a_submitted_event() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        host.on_message(1, Message::SubmitHunterEvent { event: guest_report("taser") }, now, &mut out);
        host.expect_capture(1);
        host.on_message(1, Message::SubmitHunterEvent { event: guest_report("caught") }, now, &mut out);
        assert!(host.take_guest_events().is_empty(), "the host's own id is never a guest");

        let mut guest = offline_session(false, "GUEST");
        guest.expect_capture(2);
        guest.on_message(2, Message::SubmitHunterEvent { event: guest_report("taser") }, now, &mut out);
        guest.on_message(2, Message::SubmitHunterEvent { event: guest_report("caught") }, now, &mut out);
        assert!(guest.take_guest_events().is_empty(), "a guest never resolves a submission");
        assert!(out.is_empty());
    }

    pub(super) fn offline_session(host: bool, name: &str) -> NetSession {
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
    fn the_name_over_a_figure_is_cleaned_whatever_its_source() {
        let mut guest = offline_session(false, "GUEST");
        guest.host_name = "ho\u{202e}st \u{1F600}".to_string() + &"x".repeat(60);
        let host = guest.peer_name(HOST_PEER_ID);
        assert!(host.len() <= MAX_NAME && host.starts_with("HOST "), "{host}");
        assert!(host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_')));
        guest.names.insert(9, "n\u{e9}on".into());
        assert_eq!(guest.peer_name(9), "NON");
        assert_eq!(guest.peer_name(77), "", "unknown peer: no name, no tag");
        guest.host_name.clear();
        assert_eq!(guest.peer_name(HOST_PEER_ID), "");
        guest.coop.close();
        let mut host = offline_session(true, "HOST");
        host.guests.insert(5, Guest { name: "a".repeat(50), last_seen: Instant::now() });
        assert_eq!(host.peer_name(5).len(), MAX_NAME);
        host.coop.close();
    }

    #[test]
    fn the_roster_lists_the_guests_for_the_host_and_the_host_first_for_a_guest() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        host.guests.insert(42, Guest { name: "NIGHTWALK".into(), last_seen: now });
        host.guests.insert(7, Guest { name: "QUIET".into(), last_seen: now });
        host.remotes.insert(42, Remote::new(sample(1, 0.0, 0.0), now, None));
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
        guest.remotes.insert(HOST_PEER_ID, Remote::new(sample(1, 3.0, -2.0), now, None));
        guest.remotes.insert(9, Remote::new(sample(1, 5.0, 5.0), now, None));
        let roster = guest.roster();
        assert_eq!(roster.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["HOST", "NEON"]);
        assert!(roster[0].host && roster[0].sending, "the host sends samples");
        // a runner quiet for longer than QUIET_DISCONNECT stays listed, with its quiet time for the DISCONNECTED status
        let long_ago = now.checked_sub(QUIET_DISCONNECT + Duration::from_secs(1)).unwrap_or(now);
        guest.remotes.insert(9, Remote::new(sample(2, 5.0, 5.0), long_ago, None));
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
            host_sees = host.remotes_at(5.0, 1, (0, 0)).iter().any(|r| (r.x - 7.5).abs() < 1e-4);
            guest_sees = guest.remotes_at(5.0, 1, (0, 0)).iter().any(|r| (r.x - 3.0).abs() < 1e-4);
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
    use super::tests::{guest_report, offline_session, sample};
    use super::*;
    use crate::net::relay::{decode, encode, frame};
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

    /// A fake relay that answers the first HOST_REGISTER with `answer`.
    fn probe_against(answer: u8) -> HostProbe {
        let relay = UdpSocket::bind("127.0.0.1:0").unwrap();
        relay.set_read_timeout(Some(Duration::from_millis(1000))).unwrap();
        let port = relay.local_addr().unwrap().port();
        let mut link = RelayLink::host("PUBLC", "stale-token", "127.0.0.1", port).unwrap();
        let answering = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            let from = loop {
                let (n, from) = relay.recv_from(&mut buf).unwrap();
                if decode(&buf[..n]).unwrap().kind == frame::HOST_REGISTER {
                    break from;
                }
            };
            let payload: &[u8] = if answer == frame::ERROR { b"HOST AUTHORIZATION FAILED" } else { b"" };
            relay.send_to(&encode(answer, 0, "PUBLC", payload).unwrap(), from).unwrap();
        });
        let out = probe_public_host(&mut link, Duration::from_secs(2));
        answering.join().unwrap();
        link.close();
        out
    }

    #[test]
    fn a_public_host_the_relay_sends_away_joins_as_a_guest_instead_of_failing() {
        assert_eq!(probe_against(frame::HOST_JOIN_AS_GUEST), HostProbe::HasHost);
        assert_eq!(probe_against(frame::ERROR), HostProbe::HasHost, "an older relay's refusal is a join, not a failure");
        assert_eq!(probe_against(frame::HOST_ACK), HostProbe::Hosting);
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

    #[test]
    fn only_a_guest_sends_only_the_four_hunter_event_kinds() {
        let mut host = offline_session(true, "HOST");
        assert!(!host.submit_hunter_event(guest_report("taser")), "the host resolves events, it never submits them");
        let mut guest = offline_session(false, "GUEST");
        assert!(!guest.submit_hunter_event(guest_report("hunter_state")), "a world event kind is not a guest report");
        assert!(!guest.submit_hunter_event(Map::new()), "an event with no kind is not sent");
        assert!(!guest.submit_hunter_event(guest_report("escape")), "the kind must be exact");
    }

    fn registered(host: &mut NetSession, peer: u32, now: Instant) {
        host.guests.insert(peer, Guest { name: format!("G{peer}"), last_seen: now });
    }

    #[test]
    fn a_captured_guest_that_never_answers_is_released_once_after_the_data_timeout() {
        let mut host = offline_session(true, "HOST");
        host.expect_capture(2);
        let limit = hunter_events::rules().capture_timeout;
        let sent = host.pending_captures[&2].sent;
        assert_eq!(host.expire_captures(sent + limit - Duration::from_millis(1)), 0, "not yet");
        assert!(host.take_guest_events().is_empty());
        assert_eq!(host.expire_captures(sent + limit), 1);
        let got = host.take_guest_events();
        assert_eq!((got.len(), got[0]["what"].clone(), got[0]["peer_id"].clone()), (1, json!("capture_timeout"), json!(2)));
        assert_eq!(host.expire_captures(sent + limit * 2), 0, "the timeout is reported once");
        // a late report from the released guest is not an expected one
        let mut out = Vec::new();
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("caught") }, sent + limit, &mut out);
        assert!(host.take_guest_events().is_empty());
    }

    #[test]
    fn a_report_inside_the_timeout_is_still_accepted_and_the_prune_runs_the_expiry() {
        let mut host = offline_session(true, "HOST");
        host.expect_capture(2);
        let sent = host.pending_captures[&2].sent;
        let mut out = Vec::new();
        host.prune(sent + Duration::from_millis(10), &mut out);
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("escaped") }, sent, &mut out);
        assert_eq!(host.take_guest_events().len(), 1);
        host.expect_capture(3);
        let sent3 = host.pending_captures[&3].sent;
        host.prune(sent3 + hunter_events::rules().capture_timeout, &mut out);
        assert_eq!(host.take_guest_events()[0]["what"], json!("capture_timeout"), "prune is the call site of the expiry");
    }

    #[test]
    fn a_guests_taser_intents_are_spaced_by_the_data_gap() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("taser") }, now, &mut out);
        host.on_message(2, Message::SubmitHunterEvent { event: guest_report("taser") }, now + Duration::from_millis(10), &mut out);
        assert_eq!(host.take_guest_events().len(), 1, "a second intent inside the gap is dropped");
        host.on_message(3, Message::SubmitHunterEvent { event: guest_report("taser") }, now, &mut out);
        assert_eq!(host.take_guest_events().len(), 1, "the gap is per guest");
    }

    #[test]
    fn a_guests_valve_request_reaches_the_host_queue_bounded_by_data() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        let request = valve_events::request_message("SEWER:0:-1:0:PROP:3", "flow", "", (5.5, 6.5));
        host.on_message(2, Message::RequestInteraction { event: request.clone() }, now, &mut out);
        assert!(host.take_valve_requests().is_empty(), "a peer that never registered is ignored");
        registered(&mut host, 2, now);
        host.on_message(2, Message::RequestInteraction { event: request.clone() }, now, &mut out);
        let got = host.take_valve_requests();
        assert_eq!((got.len(), got[0].peer, got[0].valve_id.as_str()), (1, 2, "SEWER:0:-1:0:PROP:3"));
        assert!(host.take_valve_requests().is_empty(), "taking empties the queue");
        for _ in 0..hunter_events::rules().max_valve_requests + 5 {
            host.on_message(2, Message::RequestInteraction { event: request.clone() }, now, &mut out);
        }
        assert_eq!(host.take_valve_requests().len(), hunter_events::rules().max_valve_requests);
        let bad = valve_events::request_message("not a valve", "flow", "", (0.0, 0.0));
        host.on_message(2, Message::RequestInteraction { event: bad }, now, &mut out);
        assert!(host.take_valve_requests().is_empty());
        let mut guest = offline_session(false, "GUEST");
        guest.on_message(2, Message::RequestInteraction { event: request }, now, &mut out);
        assert!(guest.take_valve_requests().is_empty(), "a guest never queues requests");
    }

    #[test]
    fn a_guests_chat_is_moderated_rebroadcast_and_a_guest_hears_the_host() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        let line = |text: &str| match crate::net::chat::submit(text, "team", [0.0; 2], [0.0; 3], 0) {
            Some(Message::SubmitChatPayload { payload }) => Message::SubmitChatPayload { payload },
            _ => panic!(),
        };
        host.on_message(2, line("hello"), now, &mut out);
        assert!(host.take_chat_lines().is_empty(), "a peer that never registered is ignored");
        registered(&mut host, 2, now);
        host.on_message(2, line("hello"), now, &mut out);
        let got = host.take_chat_lines();
        assert_eq!((got.len(), got[0].text.as_str(), got[0].peer_id), (1, "hello", 2));
        host.on_message(2, line("hello"), now, &mut out);
        assert!(host.take_chat_lines().is_empty(), "moderation drops a duplicate");
        assert!(host.send_chat("host line", "team", [0.0; 2], [0.0; 3], 0));
        assert_eq!(host.take_chat_lines()[0].peer_id, HOST_PEER_ID as i64);
        assert!(!host.send_chat("   ", "team", [0.0; 2], [0.0; 3], 0), "an empty line is not sent");
        let mut guest = offline_session(false, "GUEST");
        let heard = Message::ChatPayload { peer_id: 1, display_name: "HOST".into(), payload: crate::net::chat::ChatLine::sanitize(1, "HOST", &Map::new(), false).payload() };
        guest.on_message(HOST_PEER_ID, heard, now, &mut out);
        assert!(guest.take_chat_lines().is_empty(), "an empty line is not shown");
        let mut p = Map::new();
        p.insert("text".into(), json!("run"));
        guest.on_message(HOST_PEER_ID, Message::ChatPayload { peer_id: 1, display_name: "HOST".into(), payload: p }, now, &mut out);
        assert_eq!(guest.take_chat_lines()[0].text, "run");
    }

    #[test]
    fn a_timed_out_guest_leaves_the_session_peers() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        registered(&mut host, 2, now);
        host.coop.send(2, &Message::PeerLeave);
        assert!(host.coop.peers().contains(&2));
        host.prune(now + GUEST_IDLE + ROSTER_KEEP + Duration::from_secs(1), &mut out);
        assert!(!host.coop.peers().contains(&2), "no later broadcast or resend reaches it");
    }

    #[test]
    fn a_hostile_hide_cell_does_not_overflow_the_remote_view() {
        let now = Instant::now();
        let smp = sample(1, 13.0, 14.0).with_body(Pose::Climb, 1.5, true, Some(HideSpot { cell: (i32::MIN, i32::MAX), facing: (1, 0) }));
        let r = Remote::new(smp, now, Some(now));
        let v = remote_view(5, "Ann".into(), &r, "DEFAULT".into(), (10, -10), now);
        assert_eq!(v.hide, None, "an out-of-range cell is dropped, not wrapped");
    }

    #[test]
    fn a_guests_breach_request_reaches_the_host_burst_queue_bounded_by_data() {
        let now = Instant::now();
        let mut host = offline_session(true, "HOST");
        let mut out = Vec::new();
        let request = valve_events::burst_request_message("BRK:0:-1:0:3");
        host.on_message(2, Message::RequestInteraction { event: request.clone() }, now, &mut out);
        assert!(host.take_burst_requests().is_empty(), "a peer that never registered is ignored");
        registered(&mut host, 2, now);
        for _ in 0..hunter_events::rules().max_valve_requests + 5 {
            host.on_message(2, Message::RequestInteraction { event: request.clone() }, now, &mut out);
        }
        let got = host.take_burst_requests();
        assert_eq!((got.len(), got[0].peer, got[0].flow_id.as_str()), (hunter_events::rules().max_valve_requests, 2, "BRK:0:-1:0:3"));
        assert!(host.take_valve_requests().is_empty(), "a breach is not a valve turn");
        let mut guest = offline_session(false, "GUEST");
        guest.on_message(2, Message::RequestInteraction { event: request }, now, &mut out);
        assert!(guest.take_burst_requests().is_empty(), "a guest never queues requests");
    }

    #[test]
    fn a_host_event_for_a_valve_reaches_a_guest_as_a_world_event_the_app_applies() {
        let now = Instant::now();
        let mut guest = offline_session(false, "GUEST");
        let mut out = Vec::new();
        let ev = crate::game::pipes::TurnEvent {
            valve_id: "SEWER:0:-1:0:PROP:3".into(),
            chunk: (0, -1, 0),
            effect: "flow".into(),
            shut: false,
            turns: 1,
            gate: String::new(),
            noise: None,
        };
        let mut event = Map::new();
        event.insert("what".into(), json!(valve_events::VALVE));
        for (k, v) in valve_events::turn_fields(&ev) {
            event.insert(k.into(), v);
        }
        guest.on_message(HOST_PEER_ID, Message::Event { event: event.clone() }, now, &mut out);
        assert_eq!(out, vec![NetEvent::WorldEvent(event.clone())]);
        assert_eq!(valve_events::parse_turn(&event).map(|t| t.turns), Some(1));
        let mut host = offline_session(true, "HOST");
        assert!(host.guest_position(2).is_none());
        host.remotes.insert(2, Remote::new(sample(1, 3.0, 4.0), now, None));
        assert_eq!(host.guest_position(2), Some((3.0, 4.0, 1)));
    }
}

/// Phase 8 (continuous levels): the optional absolute height on a sample and the near-storey filter.
#[cfg(test)]
mod height_tests {
    use super::tests::{offline_session, sample};
    use super::*;

    #[test]
    fn old_sample_without_h_still_parses() {
        let old = json!({"seq": 4, "depth": -1, "x": 2.0, "y": 3.0, "yaw": 0.5, "moving": true});
        let s = Sample::from_json(&old).expect("a pre-phase-8 sample parses");
        assert_eq!(s.h, None);
        assert_eq!(s.depth, -1);
        let back = s.to_json();
        assert!(back.get("h").is_none(), "no h key is written when there is none");
        assert_eq!(Sample::from_json(&back), Some(s));
    }

    #[test]
    fn sample_h_round_trips_and_a_non_finite_h_is_ignored() {
        let mut s = sample(1, 1.0, 2.0);
        s.h = Some(-2.5);
        assert_eq!(s.to_json()["h"].as_f64(), Some(-2.5));
        assert_eq!(Sample::from_json(&s.to_json()), Some(s));
        for bad in [json!(null), json!("high"), json!(1e39)] {
            let mut v = s.to_json();
            v["h"] = bad;
            let parsed = Sample::from_json(&v).expect("a bad h never rejects the sample");
            assert_eq!(parsed.h, None);
            assert_eq!(parsed.x, 1.0);
        }
    }

    #[test]
    fn remotes_at_returns_peers_within_one_storey() {
        let now = Instant::now();
        let mut s = offline_session(true, "HOST");
        let mut near = sample(1, 1.0, 1.0);
        near.h = Some(-4.2);
        near.depth = -1;
        let mut far = sample(1, 2.0, 2.0);
        far.h = Some(-6.0);
        far.depth = -1;
        let mut old = sample(1, 3.0, 3.0);
        old.h = None;
        old.depth = -1;
        for (peer, smp) in [(2, near), (3, far), (4, old)] {
            s.remotes.insert(peer, Remote::new(smp, now, None));
        }
        let seen: Vec<u32> = s.remotes_at(0.0, 0, (0, 0)).iter().map(|r| r.peer).collect();
        assert_eq!(seen, vec![2], "h=-4.2 shown, h=-6.0 not, an old sample on depth -1 not for depth 0");
        let views = s.remotes_at(-5.0, -1, (0, 0));
        assert_eq!(views.iter().map(|r| r.peer).collect::<Vec<_>>(), vec![2, 3, 4]);
        assert_eq!(views[2].h, -STOREY, "an old sample's height is depth * STOREY");
        assert_eq!(views[0].level, -1);
        s.coop.close();
    }
}


/// Guest names in a room: the host keeps them apart, and everyone sees the same name for the same guest.
#[cfg(test)]
mod guest_name_tests {
    use super::tests::offline_session;
    use super::*;

    fn guest_message(name: &str) -> Message {
        Message::RegisterPeer { requested_name: name.to_string() }
    }

    fn registered_name(host: &NetSession, peer: u32) -> String {
        host.guests.get(&peer).map(|g| g.name.clone()).unwrap_or_default()
    }

    #[test]
    fn clean_name_keeps_a_guest_name_and_an_empty_one_is_a_guest_not_runner() {
        assert_eq!(clean_name("GUEST_4829"), "GUEST_4829");
        assert_eq!(clean_name(" guest_4829 "), "GUEST_4829");
        assert_eq!(clean_name(""), "GUEST_0000");
        assert_eq!(clean_name("!!!"), "GUEST_0000");
        assert!(guest_name::is_guest_name(&clean_name("")));
        // the old style still passes through, and so does a name from an older client
        assert_eq!(clean_name("R-4823"), "R-4823");
        assert_eq!(clean_name("RUNNER"), "RUNNER");
    }

    #[test]
    fn the_wire_profile_carries_the_guest_name_unchanged() {
        let message = profile_message(3, &clean_name("guest_4829"), "DEFAULT", SkinFlags::default());
        match message {
            Message::WireProfile { profile } => assert_eq!(profile["name"], "GUEST_4829"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_free_guest_name_is_kept_and_announced_as_it_is() {
        let mut host = offline_session(true, "SERVER");
        let mut out = Vec::new();
        host.on_message(2, guest_message("GUEST_4829"), Instant::now(), &mut out);
        assert_eq!(registered_name(&host, 2), "GUEST_4829");
        assert_eq!(out, vec![NetEvent::GuestJoined("GUEST_4829".into())]);
        host.coop.close();
    }

    #[test]
    fn the_later_guest_with_a_taken_name_is_given_a_free_one() {
        let mut host = offline_session(true, "SERVER");
        let mut out = Vec::new();
        let now = Instant::now();
        host.on_message(2, guest_message("GUEST_4829"), now, &mut out);
        host.on_message(3, guest_message("GUEST_4829"), now, &mut out);
        let (first, second) = (registered_name(&host, 2), registered_name(&host, 3));
        assert_eq!(first, "GUEST_4829", "the earlier guest keeps its name");
        assert_ne!(second, first, "the later one is re-rolled");
        assert!(guest_name::is_drawn_name(&second), "{second}");
        // the join message names the guest as the room knows it
        assert_eq!(out, vec![NetEvent::GuestJoined(first.clone()), NetEvent::GuestJoined(second.clone())]);
        // the lobby roster lists two different guests
        assert_eq!(host.guest_names(), vec![first.clone(), second.clone()]);
        // a third with a placeholder (an older client's empty name) is a new name too
        host.on_message(4, guest_message(""), now, &mut out);
        let third = registered_name(&host, 4);
        assert!(guest_name::is_drawn_name(&third) && third != first && third != second, "{third}");
        host.coop.close();
    }

    #[test]
    fn a_guest_never_takes_the_hosts_name_and_a_signed_in_name_is_left_alone() {
        let mut host = offline_session(true, "GUEST_5555");
        let mut out = Vec::new();
        let now = Instant::now();
        host.on_message(2, guest_message("GUEST_5555"), now, &mut out);
        assert_ne!(registered_name(&host, 2), "GUEST_5555");
        // two sessions of one account are not for the host to rename
        host.on_message(3, guest_message("ANA"), now, &mut out);
        host.on_message(4, guest_message("ANA"), now, &mut out);
        assert_eq!((registered_name(&host, 3), registered_name(&host, 4)), ("ANA".to_string(), "ANA".to_string()));
        // registering again under the same session keeps the name (it does not collide with itself)
        let kept = registered_name(&host, 2);
        host.on_message(2, guest_message(&kept), now, &mut out);
        assert_eq!(registered_name(&host, 2), kept);
        host.coop.close();
    }

    #[test]
    fn a_profile_cannot_make_a_guest_pose_as_another_guest() {
        let mut host = offline_session(true, "SERVER");
        let mut out = Vec::new();
        let now = Instant::now();
        host.on_message(2, guest_message("GUEST_4829"), now, &mut out);
        host.on_message(3, guest_message("ANA"), now, &mut out);
        // a signed-in guest sending another guest's name in its profile keeps its own name
        assert_eq!(host.settle_profile_name("GUEST_4829".into(), "ANA", 3), ("ANA".to_string(), false));
        // a free guest-shaped name is left alone, and so is the guest's own name
        assert_eq!(host.settle_profile_name("GUEST_2468".into(), "ANA", 3), ("GUEST_2468".to_string(), false));
        assert_eq!(host.settle_profile_name("GUEST_4829".into(), "GUEST_4829", 2), ("GUEST_4829".to_string(), false));
        // a stale name after a re-roll is replaced by the given one and the guest is told
        assert_eq!(host.settle_profile_name("GUEST_4829".into(), "GUEST_2468", 4), ("GUEST_2468".to_string(), true));
        // no registered name yet: nothing to settle against
        assert_eq!(host.settle_profile_name("GUEST_4829".into(), "", 9), ("GUEST_4829".to_string(), false));
        host.coop.close();
    }

    #[test]
    fn the_host_tells_a_renamed_guest_and_the_guest_takes_the_name_once() {
        let mut guest = offline_session(false, "GUEST_4829");
        let me = guest.coop.link.session();
        let mut out = Vec::new();
        let profile = |name: &str| profile_message(me, name, "DEFAULT", SkinFlags::default());
        guest.on_message(HOST_PEER_ID, profile("GUEST_2468"), Instant::now(), &mut out);
        assert_eq!(out, vec![NetEvent::Renamed("GUEST_2468".into())]);
        assert_eq!(guest.name, "GUEST_2468");
        // the same profile again is not a second rename
        out.clear();
        guest.on_message(HOST_PEER_ID, profile("GUEST_2468"), Instant::now(), &mut out);
        assert!(out.is_empty());
        guest.coop.close();
    }

    #[test]
    fn a_signed_in_runner_is_never_renamed_by_a_profile() {
        let mut guest = offline_session(false, "ANA");
        let me = guest.coop.link.session();
        let mut out = Vec::new();
        let profile = profile_message(me, "GUEST_2468", "DEFAULT", SkinFlags::default());
        guest.on_message(HOST_PEER_ID, profile, Instant::now(), &mut out);
        assert!(out.is_empty());
        assert_eq!(guest.name, "ANA");
        guest.coop.close();
    }

    /// Two guests ask for the same number over a real relay: both end up with different names, and each sees the
    /// same names for itself and the other in the lobby roster and in the wire profiles.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn two_guests_asking_for_one_number_see_the_same_two_names_over_the_relay() {
        use crate::net::ws_native::NativeWsLink;
        use crate::net::ws_test_hub::TestHub;
        let hub = TestHub::start();
        hub.add_room("ABCDE", "tok");
        let mut host = NetSession::dedicated_host(
            BackendConfig::default(),
            "ABCDE".into(),
            Box::new(NativeWsLink::host("ABCDE", "tok", &hub.url()).unwrap()),
            "tok".into(),
            "ROOM-ABCDE-X".into(),
            "SERVER".into(),
            false,
        );
        let join = |name: &str| {
            NetSession::guest_over(BackendConfig::default(), "ABCDE".into(), Box::new(NativeWsLink::guest("ABCDE", "", &hub.url()).unwrap()), name.into(), "DEFAULT".into())
        };
        let mut a = join("GUEST_4829");
        let mut events_a = Vec::new();
        let end = std::time::Instant::now() + Duration::from_secs(20);
        while host.guest_names().is_empty() && std::time::Instant::now() < end {
            host.tick(None);
            events_a.extend(a.tick(None));
            std::thread::sleep(Duration::from_millis(3));
        }
        let mut b = join("GUEST_4829");
        let mut events_b = Vec::new();
        let mut both = false;
        while !both && std::time::Instant::now() < end {
            host.tick(None);
            events_a.extend(a.tick(None));
            events_b.extend(b.tick(None));
            both = events_b.iter().any(|e| matches!(e, NetEvent::Renamed(_))) && a.guest_names().len() == 2 && b.guest_names().len() == 2;
            std::thread::sleep(Duration::from_millis(3));
        }
        assert!(both, "no rename: a {events_a:?} b {events_b:?}");
        let renamed = events_b.iter().find_map(|e| if let NetEvent::Renamed(n) = e { Some(n.clone()) } else { None }).unwrap();
        assert!(guest_name::is_drawn_name(&renamed) && renamed != "GUEST_4829", "{renamed}");
        assert_eq!(b.name, renamed, "the guest plays under the name the room gave it");
        assert!(events_a.iter().all(|e| !matches!(e, NetEvent::Renamed(_))), "the earlier guest is not renamed");
        // the lobby: the host lists both, each guest lists the host first and the other guest
        assert_eq!(host.guest_names(), vec!["GUEST_4829".to_string(), renamed.clone()]);
        assert_eq!(a.guest_names(), vec!["SERVER".to_string(), renamed.clone()]);
        assert_eq!(b.guest_names(), vec!["SERVER".to_string(), "GUEST_4829".to_string()]);
        // the wire: the earlier guest hears the later one's profile under its new name
        let peer_b = host.roster_peers().into_iter().find(|(_, n)| *n == renamed).map(|(p, _)| p).unwrap();
        for _ in 0..400 {
            host.tick(None);
            a.tick(None);
            b.tick(None);
            if a.peer_name(peer_b) == renamed {
                break;
            }
            std::thread::sleep(Duration::from_millis(3));
        }
        assert_eq!(a.peer_name(peer_b), renamed);
        for s in [&mut host, &mut a, &mut b] {
            s.coop.close();
        }
    }
}

/// What the other client sends about a skin is the only thing a custom skin puts on this client's wire, so a player with a
/// skin must never be able to blank another player's frame through it: whatever a profile holds, the figure list the frame
/// draws stays sane (a bounded skin id and name, a boolean arm style, no panic).
#[cfg(test)]
mod skin_wire_tests {
    use super::tests::{offline_session, sample};
    use super::*;

    fn profile(v: Value) -> Map<String, Value> {
        v.as_object().cloned().expect("an object")
    }

    fn hostile_profiles() -> Vec<(&'static str, Map<String, Value>)> {
        let hash = "a".repeat(64);
        vec![
            ("a valid skin", profile(json!({"peer_id": 7, "name": "ALEX", "skin": hash, "skin_slim": true, "skin_copy": true}))),
            ("an upper-case hash", profile(json!({"peer_id": 7, "name": "ALEX", "skin": "A".repeat(64)}))),
            ("a 100 KB skin id", profile(json!({"peer_id": 7, "name": "ALEX", "skin": "x".repeat(100_000)}))),
            ("a 100 KB name", profile(json!({"peer_id": 7, "name": "N".repeat(100_000), "skin": "DEFAULT"}))),
            ("a number for the skin", profile(json!({"peer_id": 7, "name": "ALEX", "skin": 12345}))),
            ("null everywhere", profile(json!({"peer_id": 7, "name": null, "skin": null, "skin_slim": null, "skin_copy": null}))),
            ("text for the flags", profile(json!({"peer_id": 7, "skin": "DEFAULT", "skin_slim": "yes", "skin_copy": "true"}))),
            ("emoji and controls", profile(json!({"peer_id": 7, "name": "\u{202e}\u{1F600}\n\0", "skin": "\u{1F600}\n\0\u{202e}"}))),
            ("a path as the skin", profile(json!({"peer_id": 7, "skin": "../../etc/passwd"}))),
            ("a 63 and a 65 character hash", profile(json!({"peer_id": 7, "skin": "b".repeat(63), "name": "c".repeat(65)}))),
            ("nothing at all", Map::new()),
            ("nested objects", profile(json!({"peer_id": 7, "skin": {"a": [1, 2]}, "name": ["x"], "skin_slim": {}}))),
        ]
    }

    fn assert_sane(what: &str, views: &[RemoteView]) {
        for v in views {
            assert!(v.skin == SKIN_DEFAULT || is_skin_hash(&v.skin) || (v.skin.len() <= MAX_SKIN && v.skin.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')), "{what}: skin {:?}", v.skin);
            assert!(v.name.chars().count() <= 24, "{what}: a {} character name", v.name.chars().count());
            assert!(v.x.is_finite() && v.y.is_finite() && v.h.is_finite() && v.yaw.is_finite(), "{what}: a figure at a non-finite place");
        }
    }

    #[test]
    fn a_host_takes_any_profile_and_hands_the_frame_a_sane_figure() {
        let mut host = offline_session(true, "HOST");
        host.guests.insert(7, Guest { name: "ALEX".into(), last_seen: Instant::now() });
        host.remotes.insert(7, Remote::new(sample(1, 3.0, 4.0), Instant::now(), None));
        for (what, p) in hostile_profiles() {
            let mut out = vec![];
            host.on_message(7, Message::WireProfile { profile: p }, Instant::now(), &mut out);
            let views = host.remotes_at(5.0, 1, (0, 0));
            assert_eq!(views.len(), 1, "{what}: the runner stays in the room");
            assert_sane(what, &views);
        }
    }

    #[test]
    fn a_guest_takes_any_relayed_profile_and_hands_the_frame_a_sane_figure() {
        let mut guest = offline_session(false, "GUEST");
        guest.remotes.insert(7, Remote::new(sample(1, 3.0, 4.0), Instant::now(), None));
        for (what, p) in hostile_profiles() {
            let mut out = vec![];
            guest.on_message(HOST_PEER_ID, Message::WireProfile { profile: p }, Instant::now(), &mut out);
            assert_sane(what, &guest.remotes_at(5.0, 1, (0, 0)));
        }
        // a profile that names no runner, a negative one, one far out of range, or this runner's own, is dropped
        for peer in [json!(-1), json!(1e30), json!("7"), json!(null), json!(guest.coop.link.session())] {
            let mut out = vec![];
            let before = guest.skins.clone();
            guest.on_message(HOST_PEER_ID, Message::WireProfile { profile: profile(json!({"peer_id": peer, "skin": "b".repeat(64)})) }, Instant::now(), &mut out);
            assert_eq!(guest.skins, before, "a profile for peer {peer} changes nothing");
        }
    }

    #[test]
    fn a_runner_with_a_skin_the_viewer_does_not_have_is_drawn_with_its_hash_and_slim_arms_and_no_file_is_needed() {
        // B has no skin and no file of A's: the figure list says what A wears, and what B draws is decided by the texture cache
        // (shared/skin_cache.rs: a pending skin draws the default atlas)
        let hash = "c".repeat(64);
        let mut guest = offline_session(false, "B");
        guest.remotes.insert(7, Remote::new(sample(1, 3.0, 4.0), Instant::now(), None));
        let mut out = vec![];
        guest.on_message(HOST_PEER_ID, Message::WireProfile { profile: profile(json!({"peer_id": 7, "name": "A", "skin": hash, "skin_slim": true})) }, Instant::now(), &mut out);
        let views = guest.remotes_at(5.0, 1, (0, 0));
        assert_eq!((views[0].skin.as_str(), views[0].slim), (hash.as_str(), true));
    }
}

/// The public route over an in-process WebSocket hub: who holds the room, what a guest may do, and the handover when the
/// holder leaves (net/public_reform.rs).
#[cfg(all(test, not(target_arch = "wasm32")))]
mod public_tests;
