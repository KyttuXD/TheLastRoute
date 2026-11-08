//! Player bots (NPC teammates): the admin spawns bots in an offline room and they follow the runner, and a normal run
//! starts with a few more. This is the offline core of Godot's `game/player/npc_teammates.gd` (spawn, the follow rule,
//! the walk and the turn spring) and of the host's spawn spot rule (`infinite_route_test.gd` 6769-6779).
//! No server is involved: the bots live on the `Game` for the run and nothing is sent anywhere.
//!
//! An NPC here is a bot standing in for a player, not an in-world character. It has the runner's body
//! (`PLAYER_RADIUS`) and speed (`PLAYER_WALK_SPEED`) and obeys the runner's rules: it does not open a door by
//! walking up to it (hunters do); it stops at a shut door, faces it, waits a short seeded think delay and presses
//! interact through the same `press_door` path as the runner, then walks on once the opening fits its body.
//!
//! The bots play like fake players. Their choices come from data/npcs.json: `intents` lists what a bot tries, in
//! order, and the first that applies this frame runs. "hide" sends a chased bot into a locker; "flee" runs it from a
//! chasing hunter; "relay" and "locker" are errands that take a bot to a relay or locker near the runner when nothing
//! is chasing it and no hunter is searching there; "follow" walks it back to the runner. A hidden bot comes out once
//! the chase is over. The "life" intent (data `life`, before "follow") makes a bot with nothing to do act like a
//! player: it walks to a hunter point of interest near the runner (room centre, locker, relay, door) or stays put,
//! looks around, checks a locker or a door, inspects a relay, idles, crouches, sprints a short burst or plays an emote,
//! pauses, and never goes past `life.range` from the runner (the follow intent walks it back). A bot avoids the points hunters searched when it picks an errand spot, and it shuts an open door
//! once it has walked clear of it.
//!
//! Bots are lively (data `lively`): each has a personality (cautious, bold, curious) that tilts its activities, its
//! pauses and how far from the runner it stands; its head turns to sounds, hunters it sees, the runner and the
//! teammate it chats with, and fidgets when idle; a walking bot stops now and then to chat; it answers an emote of the
//! runner or a teammate with one of its own; a door check may be a crouched peek. The head look is host-only life state
//! (`LifeState::head_yaw` / `head_pitch`); a guest draws what the wire sends.
//!
//! Levels are one continuous space (docs/STAIRS_CONTINUOUS_DESIGN.md, phase 8). A bot has an absolute `y` and a level
//! derived from it (`level_at`). Every resident level keeps simulating its bots: a bot that is not on the runner's level
//! asks `StackNav::next_hop` for the flight or ladder to take, walks to its entrance, and walks the edge at the bot's
//! walk speed (a ladder at the climb speed times the data `ladder_speed_scale`). Doors, errands and hiding stay
//! own-level intents and only run on the runner's level. A bot on a level that is not resident stands still (bots
//! follow the runner, so this only happens after a rebuild).
//!
//! A chasing hunter that reaches a joined bot puts it down (revive.rs `npc_catches`); a bot walks to a downed teammate
//! (the "revive" intent).
//!
//! Deferred (see docs/NPC_TEAMMATES in the Godot tree): per-NPC commands,
//! chat, co-op packets, name tags, saves, and the mannequin body. A bot's relay errand stands at the
//! relay but does not claim it.

use super::level_stack::{floor_y_of, level_at, local_ground};
use super::pdist;
use super::Game;
use super::Item;
use crate::hunters::brain::State;
use crate::hunters::flight_edges::{self, EdgeWalk, Hop, Node, StackNav};
use crate::world::rng::Rng;
use crate::config::{INTERACT_REACH, PLAYER_RADIUS, PLAYER_SPRINT_SPEED, PLAYER_WALK_SPEED};
use crate::render::textures::rng::GodotRng;
use crate::world::route::run_seed_of_text;
use crate::world::vertical::LADDER_CLIMB_SPEED;
use crate::world::{Cell, Facility};
use serde_json::Value;
use std::f32::consts::PI;
use std::sync::OnceLock;

/// Godot `npc_teammates.gd` 17: the ring radius of the first eight slots.
pub const SLOT_RADIUS: f32 = 2.4;
/// Godot `npc_teammates.gd` 20: slots past this many widen the ring.
pub const SLOT_RING_SLOTS: f32 = 8.0;
/// Godot `npc_teammates.gd` 23: each slot's first repath phase.
pub const PATH_STAGGER: f32 = 0.137;
/// Godot `npc_teammates.gd` 27: seconds between path plans (scaled shorter by smartness).
pub const REPATH: f32 = 0.4;

/// How much smarter `n` acts: 0.55 at smart 0, 1.55 at smart 100. Used to shorten goal cool-downs and path replans.
fn smart_rate(n: &Npc) -> f32 {
    (0.55 + n.smart as f32 / 100.0).clamp(0.55, 1.55)
}

/// Seconds before a bot may pick another goal: base `replan` divided by its smart rate.
fn goal_cool_for(n: &Npc, t: &Tuning) -> f32 {
    (t.goals.replan / smart_rate(n)).max(0.35)
}

/// Seconds between path plans for this bot: base `REPATH` divided by its smart rate.
fn repath_for(n: &Npc) -> f32 {
    (REPATH / smart_rate(n)).max(0.18)
}
/// Godot `npc_teammates.gd` 39: time constant of the heading ease.
pub const HEADING_SMOOTH: f32 = 0.3;
/// Godot `npc_teammates.gd` 40: the body-turn spring.
pub const TURN_SPRING: f32 = 60.0;
/// Godot `npc_teammates.gd` 41: rad/s cap on the body turn.
pub const TURN_RATE: f32 = 5.0;
/// The bot's body radius: the runner's (Godot's nav literal was `.23`, npc_teammates.gd 190, 348). Nav cells must
/// fit it, and the doors block a bot with it.
pub const NAV_RADIUS: f32 = PLAYER_RADIUS;
/// Godot literal `2.39996` (npc_teammates.gd 76, 187, 329): the golden angle between slots.
pub const SLOT_ANGLE: f32 = 2.39996;
/// Godot `admin_npc_panel.gd` 155: the crosshair ray that picks the spawn point, in metres.
pub const SPAWN_RAY_RANGE: f32 = 30.0;
/// Godot `infinite_route_test.gd` 6772: a crosshair point farther than this from the admin is not used.
pub const SPAWN_NEAR_ADMIN: f32 = 40.0;
/// Godot `admin_npc_panel.gd` 149-160: the ray's step while it looks for the floor (the window's quarter cell).
const RAY_STEP: f32 = 0.25;
/// How far ahead along the planned route a bot looks for a straight line it fits on, in cells.
const LOOKAHEAD_CELLS: usize = 4;
/// Metres between the body checks along a step. A body is 0.46 m across, so this never skips a wall corner.
const SWEEP_STEP: f32 = 0.05;

/// The bots' thresholds, priorities and errand list, from the `intents`, `threat`, `follow`, `door`, `tasks`, `hide`,
/// `errand`, `searched` and `doors` keys of data/npcs.json. Every number the bot uses lives there; the code keeps only
/// the behaviour.
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    /// Godot `THREAT_RANGE` (npc_teammates.gd 25): metres a chasing hunter is noticed at, before the smart scale.
    pub threat_range: f32,
    /// Godot `.5` in `threat_range * (.5 + smart / 100)` (npc_teammates.gd 307).
    pub smart_base: f32,
    /// Godot 324-326: a fleeing NPC runs at the sprint speed (true) rather than walking.
    pub flee_running: bool,
    /// Godot `FOLLOW_RADIUS` (npc_teammates.gd 24): metres from the runner before the NPC walks to its slot again.
    pub follow_radius: f32,
    /// Seconds a bot waits at a shut door before it presses interact: this base plus up to `door_think_jitter`, seeded.
    pub door_think: f32,
    /// The seeded spread added to `door_think`.
    pub door_think_jitter: f32,
    /// The intents a bot tries, in order (data `intents`): "hide", "flee", "relay", "pickup", "locker", "follow". The first that
    /// applies this frame runs. A name the bot does not know is skipped. Only a joined bot runs an intent but "flee".
    pub intents: Vec<String>,
    /// Metres a chasing hunter is seen at for a bot to go and hide (data `hide.chase_range`).
    pub hide_chase_range: f32,
    /// Metres from a bot to the nearest locker it may run to and hide in when chased (data `hide.reach`).
    pub hide_reach: f32,
    /// Metres from a locker's centre at which the bot is inside it (data `hide.enter_radius`).
    pub enter_radius: f32,
    /// Metres from a hidden bot to a chasing hunter that keeps it in the locker (data `hide.leave_range`).
    pub hide_leave_range: f32,
    /// Seconds with no chaser within `hide_leave_range` before a hidden bot comes out (data `hide.leave_delay`).
    pub hide_leave_delay: f32,
    /// Metres from a relay or locker within which a hunter makes it unsafe for an errand (data `errand.safe_range`).
    pub errand_safe_range: f32,
    /// Metres from an errand's spot at which the bot is there (data `errand.reach`).
    pub errand_reach: f32,
    /// Metres from the runner a relay may be for a bot to go to it (data `errand.relay.leash`).
    pub relay_leash: f32,
    /// Seconds a bot stands at a relay (data `errand.relay.dwell`).
    pub relay_dwell: f32,
    /// Seconds a bot stands at a relay before it recovers it (data `errand.relay.claim_time`, optional: the dwell).
    pub relay_claim_time: f32,
    /// Metres from the runner a loose item or battery may be for a bot to go and take it (data `pickup.leash`).
    pub pickup_leash: f32,
    /// The item ids (net/held_wire.rs `wire`) a bot takes (data `pickup.useful`); any other item is left lying.
    pub pickup_useful: Vec<String>,
    /// Metres from the runner a locker may be for a bot to go to it (data `errand.locker.leash`).
    pub locker_leash: f32,
    /// Seconds a bot stays hidden in a locker on a locker errand (data `errand.locker.dwell`).
    pub locker_dwell: f32,
    /// Searched points a bot remembers (data `searched.memory`).
    pub searched_memory: usize,
    /// Metres round a searched point, or a hunter searching now, that a bot picks no spot in (data `searched.avoid_radius`).
    pub searched_avoid: f32,
    /// Metres of water over the bed from which a bot keeps out of a cell unless it flees (data `water.avoid_depth`).
    pub water_avoid_depth: f32,
    /// Path cost of a step into a cell the bot keeps out of (deep water) when it plans across levels (data
    /// `water.deep_step_cost`, optional: `DEFAULT_DEEP_STEP_COST` while the key is missing).
    pub deep_step_cost: f32,
    /// Metres from a door's centre at which a bot counts itself at that door (data `doors.near`).
    pub door_near: f32,
    /// Metres from a door's centre at which a bot is clear of it and shuts it behind itself (data `doors.close_clear`).
    pub close_clear: f32,
    /// The tasks a bot does. Only "door" (press shut doors for the runner) is built.
    pub tasks: Vec<String>,
    /// The level-wide goal planner of the "goal" intent (data `goals`).
    pub goals: GoalTuning,
    /// Sight, hearing and the route of a bot running from a hunter (data `flee`).
    pub flee: FleeTuning,
    /// How a bot outside the window travels (data `coarse`).
    pub coarse: CoarseTuning,
    /// A bot walks to a downed teammate within this many times `follow_radius` (data `revive.range_factor`).
    pub revive_range_factor: f32,
    /// ... and stands within this fraction of the revive reach (data `revive.stand_fraction`).
    pub revive_stand: f32,
    /// The most bots a room holds in all, for owner summons (data `summon.room_cap`).
    pub summon_room_cap: usize,
    /// What a bot does in its spare time (data `life`).
    pub life: LifeTuning,
    /// How lively a bot looks and acts (data `lively`).
    pub lively: Lively,
}

/// The coarse travel of a bot outside the loaded window (data `coarse`). A missing key keeps the default.
#[derive(Clone, Debug, PartialEq)]
pub struct CoarseTuning {
    /// Metres per second it covers, chunk to chunk, toward the runner.
    pub speed: f32,
}

impl Default for CoarseTuning {
    fn default() -> CoarseTuning {
        CoarseTuning { speed: PLAYER_WALK_SPEED }
    }
}

impl CoarseTuning {
    pub fn parse(v: &Value) -> CoarseTuning {
        let d = CoarseTuning::default();
        CoarseTuning { speed: v["speed"].as_f64().map_or(d.speed, |x| x as f32).max(0.0) }
    }
}

/// The next point a bot outside the window walks to: the centre of the adjacent chunk toward the runner's chunk (the
/// axis with the wider gap first, as the chunk links run), or the runner itself once they share a chunk. Positions are
/// window cells; chunks are `chunk` cells square from the window's (0, 0).
pub fn coarse_waypoint(at: (f32, f32), runner: (f32, f32), chunk: f32) -> (f32, f32) {
    let cell = |p: (f32, f32)| ((p.0 / chunk).floor() as i32, (p.1 / chunk).floor() as i32);
    let (a, r) = (cell(at), cell(runner));
    if a == r {
        return runner;
    }
    let (dx, dy) = (r.0 - a.0, r.1 - a.1);
    let next = if dx.abs() >= dy.abs() { (a.0 + dx.signum(), a.1) } else { (a.0, a.1 + dy.signum()) };
    ((next.0 as f32 + 0.5) * chunk, (next.1 as f32 + 0.5) * chunk)
}

/// One coarse step: `speed * dt` metres toward the next waypoint (never past it).
pub fn coarse_step(at: (f32, f32), runner: (f32, f32), chunk: f32, speed: f32, dt: f32) -> (f32, f32) {
    let w = coarse_waypoint(at, runner, chunk);
    let d = pdist(at, w);
    let reach = speed * dt;
    if d <= reach || d < 1e-4 {
        return w;
    }
    (at.0 + (w.0 - at.0) * reach / d, at.1 + (w.1 - at.1) * reach / d)
}

/// True when `p` lies on the facility's cell grid (the loaded window).
fn on_floor(fac: &Facility, p: (f32, f32)) -> bool {
    p.0 >= 0.0 && p.1 >= 0.0 && p.0 < fac.cols as f32 && p.1 < fac.rows as f32
}

/// One frame of a bot outside the window: it steps toward the runner on the coarse level. When the step lands on the
/// window floor it is placed on the nearest dry open cell.
fn coarse_travel(n: &mut Npc, home: &Facility, runner: (f32, f32), chunk: f32, c: &CoarseTuning, dt: f32) {
    n.walk = None;
    n.edge_clock = 0.0;
    n.hop = None;
    n.path.clear();
    n.path_age = REPATH;
    n.walking = false;
    let next = coarse_step(n.at, runner, chunk, c.speed, dt);
    let (dx, dy) = (next.0 - n.at.0, next.1 - n.at.1);
    if dx != 0.0 || dy != 0.0 {
        n.facing = (-dx).atan2(-dy);
    }
    n.at = next;
    if on_floor(home, next) {
        if let Some(at) = home.nearest_dry_open(next.0, next.1, NAV_RADIUS) {
            n.at = at;
            n.y = floor_y_of(n.level) + local_ground(home, at.0, at.1);
        }
    }
}

/// The "goal" intent's numbers (data `goals`). A bot scores every relay, battery, heal item, pickup and the exit on the
/// resident levels as `weight - min(path_cost * distance_weight, distance_cap)`, so a kind with a higher weight always
/// beats a lower one while `distance_cap` is smaller than the gap between weights. Missing keys keep the defaults.
#[derive(Clone, Debug, PartialEq)]
pub struct GoalTuning {
    pub relay: f32,
    /// A loose or fixed battery, wanted while the team's lamp is under `low_lamp`.
    pub battery: f32,
    /// The `heal_item` lying about, wanted while the runner's stamina is under `low_stamina`.
    pub heal: f32,
    pub pickup: f32,
    /// The exit, wanted once the relays of the level are recovered.
    pub exit: f32,
    /// Score lost per path step.
    pub distance_weight: f32,
    /// The most the path steps take off a score.
    pub distance_cap: f32,
    /// Path cost of one step into a cell a hunter is searching (a plain step is 1).
    pub search_cost: f32,
    /// Path steps one flight or ladder counts for when a goal is scored.
    pub edge_cost: f32,
    /// Seconds a bot waits before it picks a goal again after it found none or dropped one.
    pub replan: f32,
    /// Seconds after which a goal that is not reached is given up for good.
    pub give_up: f32,
    /// How many of the nearest spots are scored with a path search.
    pub candidates: usize,
    /// Metres of straight distance one storey counts for when the nearest spots are picked.
    pub level_metres: f32,
    /// Lamp charge fraction (0..1) under which a battery is wanted.
    pub low_lamp: f32,
    /// Stamina fraction (0..1) under which the heal item is wanted.
    pub low_stamina: f32,
    /// The item id (net/held_wire.rs `wire`) that heals.
    pub heal_item: String,
    /// The stuck recovery of a bot walking to a goal (data `goals.stuck`).
    pub stuck: StuckTuning,
    /// Metres from the runner beyond which a bot walking to a goal counts as away (data `goals.leash.max_distance`).
    pub leash_distance: f32,
    /// Seconds a bot may be away before it drops its goal and goes back to the runner (data `goals.leash.max_away`).
    pub leash_away: f32,
}

/// How a bot walking to a goal notices it is stuck and gets free (data `goals.stuck`; Rust-only figures, the ladder of
/// steps is `headless/bot.rs` stuck recovery): after `seconds` without moving `progress` metres it repaths, then presses
/// a shut door within `door_reach`, then steps aside for `aside_seconds`, then gives the goal up for `ban_seconds`.
#[derive(Clone, Debug, PartialEq)]
pub struct StuckTuning {
    pub seconds: f32,
    pub progress: f32,
    pub aside_seconds: f32,
    pub door_reach: f32,
    pub ban_seconds: f32,
}

impl Default for StuckTuning {
    fn default() -> StuckTuning {
        StuckTuning { seconds: 3.0, progress: 0.3, aside_seconds: 0.8, door_reach: 2.5, ban_seconds: 30.0 }
    }
}

impl StuckTuning {
    pub fn parse(v: &Value) -> StuckTuning {
        let d = StuckTuning::default();
        let num = |key: &str, d: f32| v[key].as_f64().map_or(d, |x| x as f32).max(0.0);
        StuckTuning {
            seconds: num("seconds", d.seconds).max(0.1),
            progress: num("progress", d.progress),
            aside_seconds: num("aside_seconds", d.aside_seconds),
            door_reach: num("door_reach", d.door_reach),
            ban_seconds: num("ban_seconds", d.ban_seconds),
        }
    }
}

/// A bot's stuck watch while it walks to a goal.
#[derive(Clone, Debug, Default)]
pub struct StuckState {
    /// Where the bot was when it last made progress.
    pub anchor: (f32, f32),
    /// Seconds without progress.
    pub still: f32,
    /// Steps taken so far in this jam (0 repath, 1 door, 2 aside, 3 give up are the next).
    pub stage: u32,
    /// Seconds of stepping aside left, and the way.
    pub aside: f32,
    pub aside_dir: (f32, f32),
    /// Jams recovered from or given up on, for the seed of the aside direction.
    pub count: u32,
}

/// How a bot notices a hunter and runs from it (data `flee`; Rust-only figures, Godot reacts to a chase inside the threat range only).
#[derive(Clone, Debug, PartialEq)]
pub struct FleeTuning {
    /// Metres a hunter of any state is seen at, inside the view cone and with a clear line.
    pub sight_range: f32,
    /// Degrees either side of the bot's heading that it sees (2D).
    pub sight_half_angle: f32,
    /// Metres at which a hunter in the clear is noticed from any direction.
    pub sense_range: f32,
    /// Metres from a bot that heard a noise within which a hunter alarms it (a sound tells it a hunter is about).
    pub hear_range: f32,
    /// Metres a flight route keeps off every hunter.
    pub clearance: f32,
    /// Least seconds between two flight plans while a hunter crosses the route.
    pub replan: f32,
    /// Longest flight route, in cells.
    pub depth: usize,
    /// Most cells one flight plan looks at (the cost cap of the search).
    pub max_cells: usize,
    /// Seconds a bot stays alarmed after it last saw or heard a hunter (it keeps running or hiding as the view turns).
    pub alert: f32,
}

impl Default for FleeTuning {
    fn default() -> FleeTuning {
        FleeTuning { sight_range: 18.0, sight_half_angle: 70.0, sense_range: 4.0, hear_range: 20.0, clearance: 3.0, replan: 0.4, depth: 18, max_cells: 600, alert: 3.0 }
    }
}

impl FleeTuning {
    pub fn parse(v: &Value) -> FleeTuning {
        let d = FleeTuning::default();
        let num = |key: &str, d: f32| v[key].as_f64().map_or(d, |x| x as f32).max(0.0);
        FleeTuning {
            sight_range: num("sight_range", d.sight_range),
            sight_half_angle: num("sight_half_angle", d.sight_half_angle).min(180.0),
            sense_range: num("sense_range", d.sense_range),
            hear_range: num("hear_range", d.hear_range),
            clearance: num("clearance", d.clearance),
            replan: num("replan", d.replan),
            depth: v["depth"].as_u64().map_or(d.depth, |n| n.clamp(1, 200) as usize),
            max_cells: v["max_cells"].as_u64().map_or(d.max_cells, |n| n.clamp(1, 20000) as usize),
            alert: num("alert", d.alert),
        }
    }
}

/// What a fleeing bot has settled on: the cell it runs to and the seconds since it last planned.
#[derive(Clone, Debug)]
pub struct FleeState {
    pub goal: Option<(i32, i32)>,
    /// Seconds since the last plan, found or not (a bot that has not planned yet plans at once).
    pub since: f32,
    /// Seconds of alarm left after the bot last saw or heard a hunter.
    pub alert: f32,
}

impl Default for FleeState {
    fn default() -> FleeState {
        FleeState { goal: None, since: f32::MAX, alert: 0.0 }
    }
}

impl FleeState {
    /// Ends the flight in progress but keeps the alarm.
    fn stop(&mut self) {
        self.goal = None;
        self.since = f32::MAX;
    }
}

impl Default for GoalTuning {
    fn default() -> GoalTuning {
        GoalTuning {
            relay: 100.0,
            battery: 70.0,
            heal: 70.0,
            pickup: 40.0,
            exit: 20.0,
            distance_weight: 0.1,
            distance_cap: 25.0,
            search_cost: 12.0,
            edge_cost: 6.0,
            replan: 4.0,
            give_up: 120.0,
            candidates: 8,
            level_metres: 40.0,
            low_lamp: 0.3,
            low_stamina: 0.35,
            heal_item: "stim_pack".to_string(),
            stuck: StuckTuning::default(),
            leash_distance: 30.0,
            leash_away: 40.0,
        }
    }
}

impl GoalTuning {
    pub fn parse(v: &Value) -> GoalTuning {
        let d = GoalTuning::default();
        let num = |key: &str, d: f32| v[key].as_f64().map_or(d, |x| x as f32);
        let w = &v["weights"];
        let wnum = |key: &str, d: f32| w[key].as_f64().map_or(d, |x| x as f32);
        GoalTuning {
            relay: wnum("relay", d.relay),
            battery: wnum("battery", d.battery),
            heal: wnum("heal", d.heal),
            pickup: wnum("pickup", d.pickup),
            exit: wnum("exit", d.exit),
            distance_weight: num("distance_weight", d.distance_weight).max(0.0),
            distance_cap: num("distance_cap", d.distance_cap).max(0.0),
            search_cost: num("search_cost", d.search_cost),
            edge_cost: num("edge_cost", d.edge_cost),
            replan: num("replan", d.replan).max(0.0),
            give_up: num("give_up", d.give_up),
            candidates: v["candidates"].as_u64().map_or(d.candidates, |n| n.clamp(1, 64) as usize),
            level_metres: num("level_metres", d.level_metres),
            low_lamp: num("low_lamp", d.low_lamp),
            low_stamina: num("low_stamina", d.low_stamina),
            heal_item: v["heal_item"].as_str().map_or(d.heal_item, str::to_string),
            stuck: StuckTuning::parse(&v["stuck"]),
            leash_distance: v["leash"]["max_distance"].as_f64().map_or(d.leash_distance, |x| x as f32).max(0.0),
            leash_away: v["leash"]["max_away"].as_f64().map_or(d.leash_away, |x| x as f32).max(0.0),
        }
    }

    fn weight(&self, kind: SpotKind) -> f32 {
        match kind {
            SpotKind::Relay => self.relay,
            SpotKind::Battery => self.battery,
            SpotKind::Heal => self.heal,
            SpotKind::Pickup => self.pickup,
            SpotKind::Exit => self.exit,
        }
    }
}

impl Tuning {
    /// Reads the tuning from a parsed data/npcs.json. None when a key is missing or has the wrong type.
    pub fn parse(v: &Value) -> Option<Tuning> {
        let num = |node: &Value, key: &str| node[key].as_f64().map(|x| x as f32);
        let threat = &v["threat"];
        let door = &v["door"];
        let hide = &v["hide"];
        let errand = &v["errand"];
        let searched = &v["searched"];
        let doors = &v["doors"];
        Some(Tuning {
            threat_range: num(threat, "range")?,
            smart_base: num(threat, "smart_base")?,
            flee_running: threat["flee_running"].as_bool()?,
            follow_radius: num(&v["follow"], "radius")?,
            door_think: num(door, "think")?,
            door_think_jitter: num(door, "think_jitter")?,
            intents: v["intents"]
                .as_array()?
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect(),
            hide_chase_range: num(hide, "chase_range")?,
            hide_reach: num(hide, "reach")?,
            enter_radius: num(hide, "enter_radius")?,
            hide_leave_range: num(hide, "leave_range")?,
            hide_leave_delay: num(hide, "leave_delay")?,
            errand_safe_range: num(errand, "safe_range")?,
            errand_reach: num(errand, "reach")?,
            relay_leash: num(&errand["relay"], "leash")?,
            relay_dwell: num(&errand["relay"], "dwell")?,
            relay_claim_time: num(&errand["relay"], "claim_time").map_or(num(&errand["relay"], "dwell")?, |c| c.min(num(&errand["relay"], "dwell").unwrap_or(c))),
            pickup_leash: num(&v["pickup"], "leash").unwrap_or(0.0),
            pickup_useful: v["pickup"]["useful"]
                .as_array()
                .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            locker_leash: num(&errand["locker"], "leash")?,
            locker_dwell: num(&errand["locker"], "dwell")?,
            searched_memory: num(searched, "memory")?.max(0.0) as usize,
            searched_avoid: num(searched, "avoid_radius")?,
            water_avoid_depth: num(&v["water"], "avoid_depth")?,
            deep_step_cost: num(&v["water"], "deep_step_cost").unwrap_or(DEFAULT_DEEP_STEP_COST),
            door_near: num(doors, "near")?,
            close_clear: num(doors, "close_clear")?,
            tasks: v["tasks"]
                .as_array()?
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect(),
            goals: GoalTuning::parse(&v["goals"]),
            flee: FleeTuning::parse(&v["flee"]),
            coarse: CoarseTuning::parse(&v["coarse"]),
            revive_range_factor: num(&v["revive"], "range_factor").unwrap_or(2.0).max(0.0),
            revive_stand: num(&v["revive"], "stand_fraction").unwrap_or(0.6).clamp(0.0, 1.0),
            summon_room_cap: v["summon"]["room_cap"].as_u64().map_or(24, |x| x as usize),
            life: LifeTuning::parse(&v["life"]),
            lively: Lively::parse(&v["lively"]),
        })
    }

    /// The tuning of the embedded data/npcs.json, read once.
    pub fn get() -> &'static Tuning {
        static TUNING: OnceLock<Tuning> = OnceLock::new();
        TUNING.get_or_init(|| {
            // Read through the content loader (content/npcs.rs), so the embedded default is the one place it lives
            let v = crate::content::npcs::embedded_json();
            Tuning::parse(&v).expect("data/npcs.json has every key the bots read")
        })
    }

    /// True when the bots do this task (a name in `tasks`).
    pub fn does(&self, task: &str) -> bool {
        self.tasks.iter().any(|t| t == task)
    }
}

/// The deep-cell step cost a bot plans across levels with while data/npcs.json has no `water.deep_step_cost`.
pub const DEFAULT_DEEP_STEP_COST: f32 = 8.0;
/// A bot starts an edge once it is this close to the hop's entrance (its body radius).
const EDGE_ENTER_REACH: f32 = NAV_RADIUS;
/// A bot whose route to the entrance has run out starts the edge from this far (one cell).
const EDGE_ENTER_SNAP: f32 = 1.0;

// ---- the seam to hunters/flight_edges.rs (task 1) ----------------------------------------------------------------
// Everything the bots need from the shared flight graph goes through these few functions: `flight_edges::rules()`,
// `StackNav::next_hop` (a `Hop` names the edge and the entrance cell), `StackNav::start_walk` and `EdgeWalk::advance`.

/// Metres per second along a ladder edge.
fn ladder_speed() -> f32 {
    LADDER_CLIMB_SPEED * flight_edges::rules().ladder_speed_scale
}

/// Seconds past an edge's own length after which a bot on it is put on the nearer end.
fn edge_stuck_seconds() -> f32 {
    flight_edges::rules().edge_stuck_seconds
}

/// The scale a sound one storey away is heard at (data `hear_adjacent_scale`).
fn hear_adjacent_scale() -> f32 {
    flight_edges::rules().hear_adjacent_scale
}

/// Metres per second a bot moves along `w`: the walk speed on a flight, the climb speed on a ladder.
pub fn edge_speed(w: &EdgeWalk) -> f32 {
    if w.is_ladder() { ladder_speed() } else { PLAYER_WALK_SPEED }
}

/// Where a bot stands after `dt` seconds along `w`: x/z, absolute y and whether it has reached the far end. A flight is
/// walked at the bot's walk speed, a ladder at the data climb speed (the edge walk applies it).
fn edge_advance(w: &mut EdgeWalk, dt: f32) -> ((f32, f32), f32, bool) {
    let s = w.advance(PLAYER_WALK_SPEED, dt, flight_edges::rules());
    ((s.x, s.z), s.y, s.done)
}

/// An edge walk that starts at the hop's entrance on `level`.
fn edge_start(nav: &StackNav, level: i32, hop: &Hop) -> Option<EdgeWalk> {
    nav.start_walk(level, hop)
}

/// The next hop from `from` toward `to` over the stack's edges. `cost` is the multiplier of a step on a level (water);
/// a value under 1 counts as 1 and `None` forbids the step.
fn next_hop(nav: &StackNav, from: Node, to: Node, cost: &dyn Fn(i32, Cell, Cell) -> Option<f32>) -> Option<Hop> {
    nav.next_hop(from, to, cost)
}

/// The cell a floor point is in.
fn cell_at(p: (f32, f32)) -> Cell {
    Cell::new(p.0.floor() as i32, p.1.floor() as i32)
}

/// A hunter as a bot sees it: its floor position and whether it is chasing.
type Seen = ((f32, f32), bool);

/// A relay or locker a bot goes to on an errand (data `errand`), by index into `fac.relays` or `fac.hiding`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Errand {
    Relay(usize),
    Locker(usize),
}

/// What a bot claims once it stands at it, by index into `fac.relays`, `fac.batteries` or `Game::pickups`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    Relay(usize),
    Battery(usize),
    Pickup(usize),
}

/// What a level-wide goal is (data `goals.weights` names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpotKind {
    Relay,
    Battery,
    Heal,
    Pickup,
    Exit,
}

/// A place a bot may make its goal, on any resident level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub kind: SpotKind,
    pub level: i32,
    /// The open cell nearest the spot on its level (where the route ends).
    pub cell: Cell,
    pub pos: (f32, f32),
    /// What reaching it claims (the active level only).
    pub claim: Option<Claim>,
}

impl Spot {
    fn same(&self, o: &Spot) -> bool {
        self.kind == o.kind && self.level == o.level && self.cell == o.cell
    }

    fn key(&self) -> (i32, i32, i32) {
        (self.level, self.cell.x, self.cell.y)
    }
}

/// A bot's level-wide goal (the "goal" intent).
#[derive(Clone, Debug, Default)]
pub struct GoalState {
    pub spot: Option<Spot>,
    /// Seconds before the bot picks a goal again.
    pub cool: f32,
    /// Seconds since the goal was picked.
    pub age: f32,
    /// Seconds stood at a relay.
    pub dwell: f32,
    /// Spots reached or given up, as (level, cell x, cell y).
    pub done: Vec<(i32, i32, i32)>,
    /// Goals picked so far.
    pub picks: u32,
    /// Spots given up on after a jam, with the seconds before the bot may pick them again.
    pub banned: Vec<((i32, i32, i32), f32)>,
    pub stuck: StuckState,
    /// Seconds spent farther than `goals.leash.max_distance` from the runner.
    pub away: f32,
}

/// One player bot (NPC teammate). Fields for the deferred features (catch, revive, chat, admin commands) are
/// added when those land.
#[derive(Clone, Debug)]
pub struct Npc {
    /// Never reused within a run (Godot `_next`).
    pub id: u32,
    /// The guest name, `R-04821` (Godot `GuestNames.from_seed(run_seed + id)`).
    pub name: String,
    /// `id + 1` (Godot `slot = _next + 1`): sets the NPC's ring slot and its phase.
    pub slot: u32,
    /// Position on the window floor, in the same frame as `Game::p.pos`.
    pub at: (f32, f32),
    /// The infinite level the NPC stands on (0 on a finite floor), derived from `y` with `level_at` every frame.
    pub level: i32,
    /// Absolute height of the feet (floor of `level` plus the ground under it; the edge's surface on a flight or ladder).
    pub y: f32,
    /// The flight, ladder or drop the bot is walking, if any (`at` is then its x/z and `y` follows the edge).
    pub walk: Option<EdgeWalk>,
    /// Seconds spent on the current edge (the stuck give-up).
    pub edge_clock: f32,
    /// The hop toward the runner's level the bot is walking to the entrance of.
    pub hop: Option<Hop>,
    /// Body yaw in radians, Godot's `facing`.
    pub facing: f32,
    /// The turn spring's velocity (Godot `turn_v`).
    pub turn_v: f32,
    /// The smoothed walking direction (Godot `heading`), unit length once set.
    pub heading: (f32, f32),
    /// Grid cells still to walk. The front is the cell the NPC stands in until it is popped.
    pub path: Vec<(i32, i32)>,
    /// Seconds since the last plan.
    pub path_age: f32,
    /// Walk clip (true) or idle (false).
    pub walking: bool,
    /// Metres walked: drives the stride swing of the drawn body.
    pub stride: f32,
    /// Smartness, 0..100, as set at spawn: scales how far off a chasing hunter makes the NPC run.
    pub smart: i32,
    /// The run-seed key of a run NPC (`{level seed}:NPC:{n}`), so a rebuild never makes it twice. Admin NPCs have none.
    pub key: Option<String>,
    /// False while a run NPC waits where it spawned; true once it has joined the runner (admin NPCs start joined).
    pub joined: bool,
    /// A waiting NPC joins once the runner is this close and in sight (data/npcs.json `join_radius`).
    pub join_radius: f32,
    /// The shut door the bot stands at and the think time left before it presses interact on it.
    pub door: Option<(Cell, f32)>,
    /// The locker (an index into `fac.hiding`) the bot is inside, while it hides.
    pub hidden: Option<usize>,
    /// The relay or locker errand the bot is on.
    pub errand: Option<Errand>,
    /// A relay or item the bot reached this frame; `Game::npc_claims` applies it through the player's interact effects.
    pub claim: Option<Claim>,
    /// Seconds the bot has stood at its errand spot (a relay) or hidden on its locker errand.
    pub dwell: f32,
    /// Relays and lockers this bot has already been to on this floor, so it does not loop on one spot.
    pub visited: Vec<Errand>,
    /// Seconds hidden with no chaser within `hide_leave_range`.
    pub clear_time: f32,
    /// Points hunters were searching, most recent last (at most `searched_memory`; a point near a kept one is not new).
    pub searched: Vec<(f32, f32)>,
    /// The door the bot last came near, which it shuts once it has walked clear of it.
    pub last_door: Option<Cell>,
    /// Cells with water at least `water_avoid_depth` deep, set by the host from the sewer water layer (`deep_cells_of`).
    pub deep_cells: Vec<(i32, i32)>,
    /// True while this frame's walk is a flight from a chasing hunter: deep water is then fine.
    pub fleeing: bool,
    /// A noise event the bot heard (hunters/noise_events.rs): where, and the seconds it still turns toward it.
    pub heard: Option<((f32, f32), f32)>,
    /// The level-wide goal and what it has reached (the "goal" intent).
    pub goal: GoalState,
    /// The flight in progress (the "flee" intent).
    pub flee: FleeState,
    /// What the bot does in its spare time (the "life" intent).
    pub life: LifeState,
}

impl Npc {
    /// Moves the NPC by `d` after the window recentres. The path is in the old frame, so it is dropped and
    /// replanned at once.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.at = (self.at.0 + d.0, self.at.1 + d.1);
        self.forget_frame();
    }

    /// Drops what the bot knew about the old frame: its route, its door, its hide, its errand, its visits and the points
    /// it saw searched, its goal and flight and its claim (all positions and indices in the old floor).
    fn forget_frame(&mut self) {
        self.path.clear();
        self.path_age = REPATH;
        self.door = None;
        self.hidden = None;
        self.errand = None;
        self.dwell = 0.0;
        self.visited.clear();
        self.clear_time = 0.0;
        self.searched.clear();
        self.deep_cells.clear();
        self.heard = None;
        self.last_door = None;
        self.walk = None;
        self.edge_clock = 0.0;
        self.hop = None;
        self.goal.spot = None;
        self.goal.done.clear();
        self.goal.banned.clear();
        self.goal.stuck = StuckState::default();
        self.goal.cool = 0.0;
        self.goal.away = 0.0;
        self.flee.stop();
        self.claim = None;
        self.life = LifeState::default();
    }
}

/// Godot `SLOT_RADIUS * maxf(1.0, sqrt(slot / SLOT_RING_SLOTS))`.
pub fn slot_radius(slot: u32) -> f32 {
    SLOT_RADIUS * (slot as f32 / SLOT_RING_SLOTS).sqrt().max(1.0)
}

/// The point a slot holds round its runner: `leader + (cos, sin)(slot * SLOT_ANGLE) * ring`.
fn slot_point(leader: (f32, f32), slot: u32, ring: f32) -> (f32, f32) {
    let angle = slot as f32 * SLOT_ANGLE;
    (
        leader.0 + angle.cos() * ring,
        leader.1 + angle.sin() * ring,
    )
}

/// The NPC name for a seed: one letter, a dash, then five digits, like `K-04817`. The same seed
/// always gives the same name. The generator is Godot's PCG32 port (`render::textures::rng::GodotRng`).
pub fn guest_name_from_seed(seed: u64) -> String {
    const LETTERS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGITS: &[u8] = b"0123456789";
    let mut rng = GodotRng::new(seed);
    let mut name = String::with_capacity(7);
    name.push(LETTERS[(rng.randi() % LETTERS.len() as u32) as usize] as char);
    name.push('-');
    for _ in 0..5 {
        name.push(DIGITS[(rng.randi() % DIGITS.len() as u32) as usize] as char);
    }
    name
}

/// Godot `wrapf`: `value` folded into `[min, max)`.
fn wrapf(value: f32, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range.abs() < f32::EPSILON {
        return min;
    }
    value - range * ((value - min) / range).floor()
}

/// The body yaw that faces `heading` on the floor: the inverse of `Player::forward2`, (-sin, -cos).
pub fn heading_yaw(heading: (f32, f32)) -> f32 {
    (-heading.0).atan2(-heading.1)
}

/// Godot `_turn`: the body yaw through a critically damped spring, its rate capped at `TURN_RATE`.
fn turn(n: &mut Npc, want: f32, dt: f32) {
    let error = wrapf(want - n.facing, -PI, PI);
    let v = n.turn_v
        + (TURN_SPRING * error - 2.0 * TURN_SPRING.sqrt() * n.turn_v) * dt;
    n.turn_v = v.clamp(-TURN_RATE, TURN_RATE);
    n.facing = wrapf(n.facing + n.turn_v * dt, -PI, PI);
}

/// The bot stands at the door `door` that blocks its next step. A door still swinging open (or one the bot is too far
/// from to reach) is waited out; a shut one is faced, and after the seeded think delay its cell is queued in
/// `presses` for the interact press (`Game::update_npcs` makes it through `press_door`, the runner's path). A bot
/// without the "door" task waits at the door and never presses it.
fn wait_at_door(n: &mut Npc, fac: &Facility, door: Cell, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) {
    n.walking = false;
    let Some(centre) = fac.door_centre(door) else { return };
    let (dx, dz) = (centre.0 - n.at.0, centre.1 - n.at.1);
    turn(n, (-dx).atan2(-dz), dt);
    let reach = pdist(n.at, fac.door_nearest_to(door, n.at).unwrap_or(centre)) <= INTERACT_REACH;
    if fac.door_is_open(door) || !reach {
        n.door = None;
        return;
    }
    let left = match n.door {
        Some((c, t)) if c == door => t - dt,
        _ => {
            let mut rng = Rng::new(&format!("{}:door:{}:{}", n.name, door.x, door.y));
            // smarter bots wait less at shut doors
            let think = (t.door_think + t.door_think_jitter * rng.next_f32()) / smart_rate(n);
            think - dt
        }
    };
    if left <= 0.0 {
        n.door = None;
        if t.does("door") {
            presses.push(door);
        }
    } else {
        n.door = Some((door, left));
    }
}

/// What a body of `NAV_RADIUS` meets on a straight line.
enum Sweep {
    Clear,
    /// A room door (or its swinging leaf) is in the way at this threshold.
    Door(Cell),
    /// A wall, prop or locker is in the way.
    Blocked,
}

/// Samples the straight line from `a` to `b` every `SWEEP_STEP` metres, up to and including `b`, and reports the first
/// thing a body meets on it.
fn sweep(fac: &Facility, a: (f32, f32), b: (f32, f32)) -> Sweep {
    let steps = ((pdist(a, b) / SWEEP_STEP).ceil() as i32).max(1);
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        if let Some(door) = fac.door_in_way(x, y, NAV_RADIUS) {
            return Sweep::Door(door);
        }
        // a standing bot is as tall as a standing runner, so a crawl pipe stops it the same way
        if !fac.body_fits(x, y, NAV_RADIUS) || fac.crawl_stops(x, y, NAV_RADIUS, crate::config::PLAYER_HEIGHT) {
            return Sweep::Blocked;
        }
    }
    Sweep::Clear
}

/// The cells of `depths` (cell, metres of water over the bed) a bot keeps out of: at least `avoid_depth` deep.
pub fn deep_cells_of(depths: &[((i32, i32), f32)], avoid_depth: f32) -> Vec<(i32, i32)> {
    depths.iter().filter(|(_, d)| *d >= avoid_depth).map(|(c, _)| *c).collect()
}

/// How many cells round a bot it reads the water field over (a bot plans short routes).
const WATER_LOOK: i32 = 12;

/// The cells within `WATER_LOOK` of `at` whose water (`depth_of`, metres) is at least `avoid_depth`: a bot's deep cells,
/// read from the shared water field.
pub fn deep_cells_near(at: (f32, f32), avoid_depth: f32, depth_of: impl Fn(Cell) -> f32) -> Vec<(i32, i32)> {
    let (cx, cy) = (at.0.floor() as i32, at.1.floor() as i32);
    let mut out = vec![];
    for y in cy - WATER_LOOK..=cy + WATER_LOOK {
        for x in cx - WATER_LOOK..=cx + WATER_LOOK {
            if depth_of(Cell::new(x, y)) >= avoid_depth {
                out.push((x, y));
            }
        }
    }
    out
}

/// The noise-event hearing of the bots with explicit rules (see `Game::npc_hear`).
pub fn bots_hear(
    npcs: &mut [Npc],
    events: &[crate::hunters::noise_events::NoiseEvent],
    r: &crate::hunters::noise_events::Rules,
) {
    bots_hear_with(npcs, events, r, hear_adjacent_scale());
}

/// `bots_hear` with the scale a sound one storey away is heard at, so a test can vary it. A sound on the bot's own
/// level is heard at `bot_hearing_scale`; one a storey above or below at `bot_hearing_scale * adjacent` (only when no
/// sound of its own level is heard); two or more storeys away not at all.
pub fn bots_hear_with(
    npcs: &mut [Npc],
    events: &[crate::hunters::noise_events::NoiseEvent],
    r: &crate::hunters::noise_events::Rules,
    adjacent: f32,
) {
    for n in npcs.iter_mut() {
        let mut heard = None;
        for (gap, scale) in [(0, r.bot_hearing_scale), (1, r.bot_hearing_scale * adjacent)] {
            let here: Vec<crate::hunters::noise_events::NoiseEvent> =
                events.iter().filter(|e| (e.level - n.level).abs() == gap).cloned().collect();
            heard = crate::hunters::noise_events::heard_with(r, &here, n.at, scale);
            if heard.is_some() {
                break;
            }
        }
        if let Some(ear) = heard {
            n.heard = Some((ear.at, r.bot_look_seconds));
        }
    }
}

/// A dry route: 8-way breadth-first over the hunter-open cells, with no corner cutting, never entering a `deep` cell.
/// Empty when `to` cannot be reached dry. The path includes `from`, like `Facility::find_path`.
fn dry_route(fac: &Facility, from: Cell, to: Cell, deep: &[(i32, i32)]) -> Vec<Cell> {
    let ok = |c: Cell| fac.hunter_open(c) && !deep.contains(&(c.x, c.y));
    if !ok(from) || !ok(to) {
        return vec![];
    }
    let mut came: std::collections::HashMap<Cell, Cell> = std::collections::HashMap::new();
    let mut queue = std::collections::VecDeque::from([from]);
    let mut seen = std::collections::HashSet::from([from]);
    while let Some(c) = queue.pop_front() {
        if c == to {
            let mut path = vec![c];
            let mut at = c;
            while let Some(p) = came.get(&at) {
                at = *p;
                path.push(at);
            }
            path.reverse();
            return path;
        }
        for dy in -1..=1 {
            for dx in -1..=1 {
                let nb = Cell::new(c.x + dx, c.y + dy);
                if (dx == 0 && dy == 0) || seen.contains(&nb) || !ok(nb) {
                    continue;
                }
                if dx != 0 && dy != 0 && (!ok(Cell::new(c.x + dx, c.y)) || !ok(Cell::new(c.x, c.y + dy))) {
                    continue;
                }
                seen.insert(nb);
                came.insert(nb, c);
                queue.push_back(nb);
            }
        }
    }
    vec![]
}

/// Whether a route crosses a deep-water cell (the cell it starts in does not count).
fn crosses_deep(path: &[Cell], deep: &[(i32, i32)]) -> bool {
    path.iter().skip(1).any(|c| deep.contains(&(c.x, c.y)))
}

/// The route a bot takes: the shortest one, unless it crosses deep water and the bot is not fleeing and a dry route
/// exists, in which case the dry one. With no dry way round the bot wades, as it has to.
fn route_for(fac: &Facility, from: Cell, to: Cell, deep: &[(i32, i32)], fleeing: bool) -> Vec<Cell> {
    let direct = fac.find_path(from, to);
    if fleeing || deep.is_empty() || !crosses_deep(&direct, deep) {
        return direct;
    }
    let dry = dry_route(fac, from, to, deep);
    if dry.is_empty() { direct } else { dry }
}

/// The way out of a cell the hunters' grid closes (channel water behind a railing, say): the search from such a cell finds
/// nothing, so the bot first walks out by its own footing, never across a railing, to the nearest cell the grid opens
/// (a dry one first, else any), and routes on from there. Empty when no way out exists.
fn leave_closed(fac: &Facility, from: Cell, to: Cell, deep: &[(i32, i32)], fleeing: bool) -> Vec<Cell> {
    let out = closed_exit(fac, from, |c| fac.hunter_open(c) && !deep.contains(&(c.x, c.y)))
        .or_else(|| closed_exit(fac, from, |c| fac.hunter_open(c)));
    let Some(out) = out else { return vec![] };
    let mut came: std::collections::HashMap<Cell, Cell> = std::collections::HashMap::new();
    let mut seen = std::collections::HashSet::from([from]);
    let mut queue = std::collections::VecDeque::from([from]);
    let mut found = false;
    while let Some(c) = queue.pop_front() {
        if c == out {
            found = true;
            break;
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nb = Cell::new(c.x + dx, c.y + dy);
            if seen.contains(&nb) || !step_free(fac, c, nb) {
                continue;
            }
            seen.insert(nb);
            came.insert(nb, c);
            queue.push_back(nb);
        }
    }
    if !found {
        return vec![];
    }
    let mut path = vec![out];
    let mut at = out;
    while let Some(p) = came.get(&at) {
        at = *p;
        path.push(at);
    }
    path.reverse();
    path.extend(route_for(fac, out, to, deep, fleeing).into_iter().skip(1));
    path
}

/// The nearest cell reached from `from` by footing alone (`step_free`) that `accept` takes, not counting `from`.
fn closed_exit(fac: &Facility, from: Cell, accept: impl Fn(Cell) -> bool) -> Option<Cell> {
    let mut seen = std::collections::HashSet::from([from]);
    let mut queue = std::collections::VecDeque::from([from]);
    while let Some(c) = queue.pop_front() {
        if c != from && accept(c) {
            return Some(c);
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nb = Cell::new(c.x + dx, c.y + dy);
            if !seen.contains(&nb) && step_free(fac, c, nb) {
                seen.insert(nb);
                queue.push_back(nb);
            }
        }
    }
    None
}

/// Whether a bot's body of `NAV_RADIUS` can step from cell `a` into the orthogonal neighbour `b` by its footing: `b` fits,
/// and no railing or crawl pipe is on the edge between them. Water is floor for the body, so this crosses it.
fn step_free(fac: &Facility, a: Cell, b: Cell) -> bool {
    !fac.rail_between(a, b)
        && fac.body_fits(b.x as f32 + 0.5, b.y as f32 + 0.5, NAV_RADIUS)
        && !fac.crawl_stops(b.x as f32 + 0.5, b.y as f32 + 0.5, NAV_RADIUS, crate::config::PLAYER_HEIGHT)
}

/// Plans a route from the bot's cell to the open cell nearest `goal`, and drops the cell the bot stands in.
fn plan(n: &mut Npc, fac: &Facility, goal: (f32, f32)) {
    n.path_age = 0.0;
    let from = Cell::new(n.at.0.floor() as i32, n.at.1.floor() as i32);
    let to = fac.nearest_open(goal.0, goal.1, NAV_RADIUS).unwrap_or(goal);
    let to = Cell::new(to.0.floor() as i32, to.1.floor() as i32);
    let route = if fac.hunter_open(from) {
        route_for(fac, from, to, &n.deep_cells, n.fleeing)
    } else {
        leave_closed(fac, from, to, &n.deep_cells, n.fleeing)
    };
    n.path = route.into_iter().map(|c| (c.x, c.y)).collect();
    if !n.path.is_empty() {
        n.path.remove(0);
    }
}

/// The door (its threshold cell) whose centre is nearer `p` than `r` metres, if there is one.
fn door_near(fac: &Facility, p: (f32, f32), r: f32) -> Option<Cell> {
    fac.doors()
        .find(|&(_, c)| pdist(p, c) < r)
        .map(|(i, _)| fac.props[i].cell)
}

/// Godot `_walk_toward`: one step toward `goal` along the planned route (`path`, replanned every `REPATH` seconds).
/// The bot aims at the furthest of the next `LOOKAHEAD_CELLS` cells it reaches in a straight line its body fits on, so
/// it follows the route round a corner and never cuts one. A shut door in the way stops it (`wait_at_door`). A step
/// that runs into a wall is not slid along: the route is replanned at once and the bot waits for its next step.
/// Passing a door records it as the bot's last door, to be shut once the bot is clear of it.
fn walk_toward(n: &mut Npc, fac: &Facility, goal: (f32, f32), running: bool, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) {
    let here = n.at;
    n.path_age += dt;
    // a flight keeps the route `flee_route` chose: a replan here would run the shortest way, past the hunter
    let repath = repath_for(n);
    if n.path_age >= repath && !(n.flee.goal.is_some() && !n.path.is_empty()) {
        plan(n, fac, goal);
    }
    if n.path.is_empty() {
        n.walking = false;
        return;
    }
    if let Some(d) = door_near(fac, here, t.door_near) {
        n.last_door = Some(d);
    }
    // a bot already inside a door's slab is let out, and one inside a wall is not checked for walls (as before)
    let in_door = fac.door_in_way(here.0, here.1, NAV_RADIUS).is_some();
    let fits = fac.body_fits(here.0, here.1, NAV_RADIUS);
    // the frame after a fresh route (a blocked step replans) aims at its first cell: cutting the route short again is what
    // aimed a bot hugging a corner at the same blocked step every frame
    let just_planned = n.path_age <= dt * 1.5;
    if fits && !in_door && !just_planned {
        let reach = n.path.len().min(LOOKAHEAD_CELLS);
        let ahead = (1..reach).rev().find(|&k| {
            let c = (n.path[k].0 as f32 + 0.5, n.path[k].1 as f32 + 0.5);
            // the line must be clear, and so must the one step the bot is about to take along it: a long sweep can
            // step over a corner that the short one clips, which would leave a bot hugging a wall frozen
            let len = pdist(here, c);
            let step_to = if len > 1e-6 {
                let s = (if running { PLAYER_SPRINT_SPEED } else { PLAYER_WALK_SPEED } * dt).min(len);
                (here.0 + (c.0 - here.0) / len * s, here.1 + (c.1 - here.1) / len * s)
            } else {
                c
            };
            matches!(sweep(fac, here, c), Sweep::Clear) && matches!(sweep(fac, here, step_to), Sweep::Clear)
        });
        if let Some(k) = ahead {
            n.path.drain(..k);
        }
    }
    let next = n.path[0];
    let target = (next.0 as f32 + 0.5, next.1 as f32 + 0.5);
    let step = (target.0 - here.0, target.1 - here.1);
    let len = (step.0 * step.0 + step.1 * step.1).sqrt();
    let dir = if len > 1e-6 {
        (step.0 / len, step.1 / len)
    } else {
        (0.0, 0.0)
    };
    if len > 0.01 {
        // the heading starts on the first step and then eases toward it (Godot: lerp, then normalise)
        let k = 1.0 - (-dt / HEADING_SMOOTH).exp();
        let h = if n.heading == (0.0, 0.0) {
            dir
        } else {
            (
                n.heading.0 + (dir.0 - n.heading.0) * k,
                n.heading.1 + (dir.1 - n.heading.1) * k,
            )
        };
        let hl = (h.0 * h.0 + h.1 * h.1).sqrt();
        if hl > 1e-6 {
            n.heading = (h.0 / hl, h.1 / hl);
        }
    }
    // a runner's speed: the sprint when the NPC runs from a hunter, the walk otherwise (Godot `_move_speed`)
    let speed = if running { PLAYER_SPRINT_SPEED } else { PLAYER_WALK_SPEED };
    let arrives = len <= speed * dt;
    let (moved, to) = if arrives {
        (len, target)
    } else {
        (speed * dt, (here.0 + dir.0 * speed * dt, here.1 + dir.1 * speed * dt))
    };
    if !in_door {
        match sweep(fac, here, to) {
            Sweep::Clear => {}
            // a shut door in the way stops the bot, as it stops the runner
            Sweep::Door(door) => {
                wait_at_door(n, fac, door, t, dt, presses);
                return;
            }
            // a wall in the way: no slide along it, a fresh route, and the next step waits for it. A step cut across a corner
            // (a railing's end beside a gap) is the one a body off its cell's centre line cannot take: it first moves
            // sideways, across the step, to that centre line, and the straight step through the gap is then clear.
            Sweep::Blocked if fits => {
                let centre = cell_at(here).center();
                let across = (centre.0 - here.0) * -dir.1 + (centre.1 - here.1) * dir.0;
                if len > 1e-6 && across.abs() > 0.01 {
                    let s = (if running { PLAYER_SPRINT_SPEED } else { PLAYER_WALK_SPEED } * dt).min(across.abs());
                    let side = (-dir.1 * s * across.signum(), dir.0 * s * across.signum());
                    let to = (here.0 + side.0, here.1 + side.1);
                    if matches!(sweep(fac, here, to), Sweep::Clear) {
                        n.at = to;
                        n.walking = true;
                        return;
                    }
                }
                plan(n, fac, goal);
                n.walking = false;
                return;
            }
            Sweep::Blocked => {}
        }
    }
    n.door = None;
    if arrives {
        n.path.remove(0);
    }
    n.at = to;
    let heading = n.heading;
    turn(n, heading_yaw(heading), dt);
    n.stride += moved;
    n.walking = true;
}

/// Godot `_think` for phase one, same-level only: follow the runner to the NPC's slot when it has
/// drifted past `follow_radius + ring - SLOT_RADIUS` (or is mid-walk), else stand still.
fn follow(n: &mut Npc, fac: &Facility, leader: (f32, f32), t: &Tuning, dt: f32, presses: &mut Vec<Cell>) {
    if pdist(n.at, leader) > follow_range(n, t) || !n.path.is_empty() {
        // each walk back aims for a fresh distance (data `lively.follow`), always inside the follow range
        if n.life.ring_fresh {
            n.life.ring_fresh = false;
            n.life.ring_var = between(t.lively.ring_scale, roll_of(n, "ring"));
        }
        let ring = follow_ring(n, t);
        let goal = slot_point(leader, n.slot, ring);
        walk_toward(n, fac, goal, false, t, dt, presses);
    } else {
        n.life.ring_fresh = true;
        n.walking = false;
    }
}

/// Metres from the runner past which a bot walks back: the follow radius widened by its slot's ring.
fn follow_range(n: &Npc, t: &Tuning) -> f32 {
    t.follow_radius + slot_radius(n.slot) - SLOT_RADIUS
}

/// The distance from the runner a bot walking back aims for: its slot's radius times its walk's scale and its
/// personality's follow scale, never closer than `lively.follow.min_ring` nor past the follow range.
pub fn follow_ring(n: &mut Npc, t: &Tuning) -> f32 {
    let range = follow_range(n, t);
    let lo = t.lively.min_ring.min(range);
    let hi = (range - t.lively.ring_margin).max(lo);
    let scale = n.life.ring_var * personality_of(n, &t.lively).follow_scale;
    (slot_radius(n.slot) * scale).clamp(lo, hi)
}

// ---- liveliness: personalities, head look, glances, fidgets, chatter stops and emote answers --------------------------
// Every number is in data/npcs.json `lively`. Host-only: a guest only draws what the wire sends.

/// One kind of bot (data `lively.personalities.*`). `acts` multiplies the weight of a life act the bot may pick; it never
/// adds an act the spot does not offer.
#[derive(Clone, Debug, PartialEq)]
pub struct Personality {
    pub name: String,
    /// Share of the draw at spawn.
    pub weight: f32,
    pub acts: Vec<(LifeAct, f32)>,
    /// Scales the glance rate at hunters.
    pub glance_scale: f32,
    /// Scales the follow ring a bot walks back to.
    pub follow_scale: f32,
    /// Scales the pause between two activities.
    pub pause_scale: f32,
    /// Scales the idle fidget rate.
    pub fidget_scale: f32,
    /// The chance a door check is done crouched, peeking.
    pub peek_chance: f32,
    /// Scales the rate of chatter stops while walking.
    pub stop_scale: f32,
    /// Scales the chance of answering an emote.
    pub emote_scale: f32,
}

impl Personality {
    /// The personality of data with none listed: every scale 1, no peek.
    pub fn neutral() -> Personality {
        Personality {
            name: "neutral".into(),
            weight: 1.0,
            acts: vec![],
            glance_scale: 1.0,
            follow_scale: 1.0,
            pause_scale: 1.0,
            fidget_scale: 1.0,
            peek_chance: 0.0,
            stop_scale: 1.0,
            emote_scale: 1.0,
        }
    }

    /// The factor on the weight of `act` (1 when the data does not name it).
    pub fn act_factor(&self, act: LifeAct) -> f32 {
        self.acts.iter().find(|(a, _)| *a == act).map_or(1.0, |(_, f)| *f)
    }
}

/// How lively the bots look and act (data `lively`). The default is inert: no head look, no glances, no stops, a fixed ring.
#[derive(Clone, Debug, PartialEq)]
pub struct Lively {
    pub personalities: Vec<Personality>,
    /// Head limits (radians) and how fast it turns (per second); the body turns once the head would pass `body_over`.
    pub max_yaw: f32,
    pub max_pitch: f32,
    pub rate: f32,
    pub body_over: f32,
    pub nod_rate: f32,
    pub nod_amount: f32,
    pub glance_range: f32,
    pub glance_hold: (f32, f32),
    pub glance_gap: (f32, f32),
    pub glance_chance: f32,
    pub runner_range: f32,
    pub runner_hold: (f32, f32),
    pub runner_gap: (f32, f32),
    pub fidget_gap: (f32, f32),
    pub fidget_hold: (f32, f32),
    pub fidget_yaw: f32,
    pub fidget_pitch: f32,
    pub ring_scale: (f32, f32),
    pub min_ring: f32,
    pub ring_margin: f32,
    pub stop_gap: (f32, f32),
    pub stop_hold: (f32, f32),
    pub chat_range: f32,
    pub react_range: f32,
    pub react_delay: (f32, f32),
    pub react_chance: f32,
    pub react_cool: f32,
    pub replies: Vec<(String, f32)>,
}

impl Default for Lively {
    fn default() -> Lively {
        Lively {
            personalities: vec![],
            max_yaw: 0.0,
            max_pitch: 0.0,
            rate: 7.0,
            body_over: PI,
            nod_rate: 5.0,
            nod_amount: 0.0,
            glance_range: 0.0,
            glance_hold: (1.0, 1.0),
            glance_gap: (1e9, 1e9),
            glance_chance: 0.0,
            runner_range: 0.0,
            runner_hold: (1.0, 1.0),
            runner_gap: (1e9, 1e9),
            fidget_gap: (1e9, 1e9),
            fidget_hold: (1.0, 1.0),
            fidget_yaw: 0.0,
            fidget_pitch: 0.0,
            ring_scale: (1.0, 1.0),
            min_ring: 0.0,
            ring_margin: 0.0,
            stop_gap: (1e9, 1e9),
            stop_hold: (0.0, 0.0),
            chat_range: 0.0,
            react_range: 0.0,
            react_delay: (1.0, 1.0),
            react_chance: 0.0,
            react_cool: 8.0,
            replies: vec![],
        }
    }
}

fn lnum(node: &Value, key: &str, d: f32) -> f32 {
    node[key].as_f64().map_or(d, |x| x as f32).max(0.0)
}

fn lpair(node: &Value, key: &str, d: (f32, f32)) -> (f32, f32) {
    match (node[key][0].as_f64(), node[key][1].as_f64()) {
        (Some(a), Some(b)) => {
            let (a, b) = (a.max(0.0) as f32, b.max(0.0) as f32);
            (a.min(b), a.max(b))
        }
        _ => d,
    }
}

impl Lively {
    pub fn parse(v: &Value) -> Lively {
        let d = Lively::default();
        let (head, glance, runner, fidget, follow, stop, react) =
            (&v["head"], &v["glance"], &v["runner_look"], &v["fidget"], &v["follow"], &v["walk_stop"], &v["react"]);
        // angles are degrees in the data, radians in the struct
        let rad = |node: &Value, key: &str, d: f32| lnum(node, key, d.to_degrees()).to_radians();
        Lively {
            personalities: v["personalities"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(name, p)| Personality {
                            name: name.clone(),
                            weight: lnum(p, "weight", 1.0),
                            acts: p["acts"]
                                .as_object()
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|(k, f)| Some((LifeAct::from_name(k)?, (f.as_f64()? as f32).max(0.0))))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            glance_scale: lnum(p, "glance_scale", 1.0),
                            follow_scale: lnum(p, "follow_scale", 1.0),
                            pause_scale: lnum(p, "pause_scale", 1.0),
                            fidget_scale: lnum(p, "fidget_scale", 1.0),
                            peek_chance: lnum(p, "peek_chance", 0.0).min(1.0),
                            stop_scale: lnum(p, "stop_scale", 1.0),
                            emote_scale: lnum(p, "emote_scale", 1.0),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            max_yaw: rad(head, "max_yaw", d.max_yaw),
            max_pitch: rad(head, "max_pitch", d.max_pitch),
            rate: lnum(head, "rate", d.rate),
            body_over: rad(head, "body_over", d.body_over),
            nod_rate: lnum(head, "nod_rate", d.nod_rate),
            nod_amount: rad(head, "nod_amount", d.nod_amount),
            glance_range: lnum(glance, "range", d.glance_range),
            glance_hold: lpair(glance, "hold", d.glance_hold),
            glance_gap: lpair(glance, "gap", d.glance_gap),
            glance_chance: lnum(glance, "chance", d.glance_chance).min(1.0),
            runner_range: lnum(runner, "range", d.runner_range),
            runner_hold: lpair(runner, "hold", d.runner_hold),
            runner_gap: lpair(runner, "gap", d.runner_gap),
            fidget_gap: lpair(fidget, "gap", d.fidget_gap),
            fidget_hold: lpair(fidget, "hold", d.fidget_hold),
            fidget_yaw: rad(fidget, "yaw", d.fidget_yaw),
            fidget_pitch: rad(fidget, "pitch", d.fidget_pitch),
            ring_scale: lpair(follow, "ring_scale", d.ring_scale),
            min_ring: lnum(follow, "min_ring", d.min_ring),
            ring_margin: lnum(follow, "margin", d.ring_margin),
            stop_gap: lpair(stop, "gap", d.stop_gap),
            stop_hold: lpair(stop, "hold", d.stop_hold),
            chat_range: lnum(stop, "chat_range", d.chat_range),
            react_range: lnum(react, "range", d.react_range),
            react_delay: lpair(react, "delay", d.react_delay),
            react_chance: lnum(react, "chance", d.react_chance).min(1.0),
            react_cool: lnum(react, "cool", d.react_cool),
            replies: react["replies"]
                .as_object()
                .map(|m| m.iter().filter_map(|(k, w)| w.as_f64().map(|w| (k.clone(), (w as f32).max(0.0)))).collect())
                .unwrap_or_default(),
        }
    }
}

/// The personality a bot named `name` in `slot` gets: a weighted draw seeded by both, so a rebuild keeps it.
pub fn pick_personality(l: &Lively, name: &str, slot: u32) -> usize {
    let list: Vec<(usize, f32)> = l.personalities.iter().enumerate().map(|(i, p)| (i, p.weight)).collect();
    let mut rng = Rng::new(&format!("{name}:{slot}:personality"));
    weighted(&list, rng.next_f32()).unwrap_or(0)
}

/// The bot's personality (drawn on first use and kept in its life state); the neutral one when the data lists none.
pub fn personality_of<'a>(n: &mut Npc, l: &'a Lively) -> &'a Personality {
    static NEUTRAL: OnceLock<Personality> = OnceLock::new();
    if l.personalities.is_empty() {
        return NEUTRAL.get_or_init(Personality::neutral);
    }
    let i = match n.life.personality {
        Some(i) if i < l.personalities.len() => i,
        _ => {
            let i = pick_personality(l, &n.name, n.slot);
            n.life.personality = Some(i);
            i
        }
    };
    &l.personalities[i]
}

/// The act weights of a spot with a personality's factors applied (an act the spot lacks stays absent).
pub fn scale_acts(acts: &[(LifeAct, f32)], p: &Personality) -> Vec<(LifeAct, f32)> {
    acts.iter().map(|(a, w)| (*a, w * p.act_factor(*a))).collect()
}

/// A fresh seeded roll in 0..1 for the bot's lively choices.
fn roll_of(n: &mut Npc, salt: &str) -> f32 {
    n.life.rolls = n.life.rolls.wrapping_add(1);
    Rng::new(&format!("{}:{}:{}:{}", n.name, n.slot, salt, n.life.rolls)).next_f32()
}

/// A walking bot's chatter stop: counts down to the next one and starts it. True the frame a stop is due.
fn stop_due(n: &mut Npc, lv: &Lively, dt: f32) -> bool {
    if lv.stop_hold.1 <= 0.0 {
        return false;
    }
    if n.life.stop_cool < 0.0 {
        n.life.stop_cool = between(lv.stop_gap, roll_of(n, "stop_first"));
    }
    n.life.stop_cool -= dt;
    if n.life.stop_cool > 0.0 {
        return false;
    }
    let scale = personality_of(n, lv).stop_scale.max(0.1);
    n.life.hold = between(lv.stop_hold, roll_of(n, "stop_hold"));
    n.life.stop_cool = between(lv.stop_gap, roll_of(n, "stop_gap")) / scale;
    n.life.stops += 1;
    true
}

/// Another bot as a bot sees it for chats and emote answers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Peer {
    pub id: u32,
    pub at: (f32, f32),
    pub level: i32,
    pub emoting: bool,
}

/// What a bot can look at this frame.
struct Around<'a> {
    fac: &'a Facility,
    leader: (f32, f32),
    runner_level: i32,
    runner_emoting: bool,
    peers: &'a [Peer],
    seen: &'a [Seen],
}

fn nearest_visible(n: &Npc, ar: &Around, range: f32) -> Option<(f32, f32)> {
    ar.seen
        .iter()
        .map(|&(c, _)| c)
        .filter(|c| pdist(n.at, *c) <= range && ar.fac.has_line_of_sight(n.at, *c))
        .min_by(|a, b| pdist(n.at, *a).total_cmp(&pdist(n.at, *b)))
}

/// Who a bot that has stopped to chat faces: the runner, else the nearest teammate, within `chat_range`.
fn chat_target(n: &Npc, ar: &Around, lv: &Lively) -> Option<(f32, f32)> {
    if n.level == ar.runner_level && pdist(n.at, ar.leader) <= lv.chat_range {
        return Some(ar.leader);
    }
    ar.peers
        .iter()
        .filter(|q| q.id != n.id && q.level == n.level && pdist(n.at, q.at) <= lv.chat_range)
        .min_by(|a, b| pdist(n.at, a.at).total_cmp(&pdist(n.at, b.at)))
        .map(|q| q.at)
}

/// Plays an answering emote (data `lively.react.replies`) where the bot stands. False when none can play.
fn start_reply(n: &mut Npc, lv: &Lively) -> bool {
    let Some(set) = crate::render::anim::emote_pose::set() else { return false };
    let list: Vec<(&str, f32)> = lv.replies.iter().filter(|(id, _)| set.find(id).is_some()).map(|(id, w)| (id.as_str(), *w)).collect();
    let roll = roll_of(n, "reply");
    let Some(id) = weighted(&list, roll) else { return false };
    let len = set.find(id).map_or(2.0, |e| e.duration.max(0.1));
    n.walking = false;
    n.path.clear();
    n.life.act = Some(LifeAct::Emote);
    n.life.phase = LifePhase::Do;
    n.life.emote = Some(id.to_string());
    n.life.emote_seq += 1;
    n.life.since_emote = 0.0;
    n.life.clock = len;
    n.life.total = len;
    n.life.picked.push(LifeAct::Emote);
    if n.life.picked.len() > PICKED_KEPT {
        n.life.picked.remove(0);
    }
    n.life.replies += 1;
    true
}

/// The head, glances, fidgets and emote answers of one bot for one frame, run after its intents. `life_ran` is true when
/// the "life" intent acted (the bot is idle in the spare-time sense).
fn liven(n: &mut Npc, ar: &Around, life_ran: bool, t: &Tuning, dt: f32) {
    let lv = &t.lively;
    if lv.max_yaw <= 0.0 && lv.max_pitch <= 0.0 {
        return;
    }
    let p = personality_of(n, lv);
    let at = n.at;
    let same = n.level == ar.runner_level;
    {
        let s = &mut n.life;
        s.glance_cool -= dt;
        s.runner_cool -= dt;
        s.fidget_cool -= dt;
        s.react_cool -= dt;
        s.nod_clock += dt;
        if s.fidget_left > 0.0 {
            s.fidget_left -= dt;
        }
        if s.gaze_left > 0.0 {
            s.gaze_left -= dt;
            if s.gaze_left <= 0.0 {
                s.gaze = None;
            }
        }
    }
    // an emote to answer: a standing bot near the runner or a teammate who emotes may answer after a short delay
    if !life_ran {
        n.life.react = None;
    } else if n.life.react.is_none()
        && n.life.react_cool <= 0.0
        && lv.react_chance > 0.0
        && n.life.phase == LifePhase::Pause
        && n.life.act.is_none()
        && !n.life.returning
        && !n.life.follow_only
        && n.life.since_emote >= t.life.emote_gap
    {
        let emoter = if ar.runner_emoting && same && pdist(at, ar.leader) <= lv.react_range {
            Some(ar.leader)
        } else {
            ar.peers.iter().find(|q| q.emoting && q.id != n.id && q.level == n.level && pdist(at, q.at) <= lv.react_range).map(|q| q.at)
        };
        if let Some(e) = emoter {
            n.life.react_cool = lv.react_cool;
            if roll_of(n, "react") < lv.react_chance * p.emote_scale {
                let delay = between(lv.react_delay, roll_of(n, "delay"));
                n.life.react = Some((delay, e));
            }
        }
    }
    if let Some((left, e)) = n.life.react {
        if left - dt <= 0.0 {
            n.life.react = None;
            if n.life.phase == LifePhase::Pause && n.life.act.is_none() {
                start_reply(n, lv);
            }
        } else {
            n.life.react = Some((left - dt, e));
        }
    }
    // what the head is on
    let mut want: Option<((f32, f32), Gaze)> = None;
    if let Some((s, _)) = n.heard {
        want = Some((s, Gaze::Sound));
        n.life.gaze_left = n.life.gaze_left.max(0.4);
    } else if let Some((_, e)) = n.life.react {
        want = Some((e, Gaze::Peer));
        n.life.gaze_left = 0.3;
    } else if n.life.hold > 0.0 {
        want = chat_target(n, ar, lv).map(|c| (c, Gaze::Chat));
        n.life.gaze_left = 0.3;
    } else if let Some((c, k)) = n.life.gaze {
        want = match k {
            Gaze::Hunter => nearest_visible(n, ar, lv.glance_range).map(|c| (c, k)),
            Gaze::Runner => Some((ar.leader, k)),
            Gaze::Chat => None,
            _ => Some((c, k)),
        };
    }
    if want.is_none() {
        if lv.glance_chance > 0.0 && n.life.glance_cool <= 0.0 {
            if let Some(c) = nearest_visible(n, ar, lv.glance_range) {
                let scale = p.glance_scale.max(0.1);
                n.life.glance_cool = between(lv.glance_gap, roll_of(n, "glance_gap")) / scale;
                if roll_of(n, "glance") < (lv.glance_chance * scale).min(1.0) {
                    want = Some((c, Gaze::Hunter));
                    n.life.gaze_left = between(lv.glance_hold, roll_of(n, "glance_hold"));
                    n.life.glances += 1;
                }
            }
        }
        if want.is_none() && same && n.life.runner_cool <= 0.0 && lv.runner_range > 0.0 && pdist(at, ar.leader) <= lv.runner_range {
            n.life.runner_cool = between(lv.runner_gap, roll_of(n, "runner_gap"));
            n.life.gaze_left = between(lv.runner_hold, roll_of(n, "runner_hold"));
            n.life.runner_looks += 1;
            want = Some((ar.leader, Gaze::Runner));
        }
    }
    n.life.gaze = want;
    if want.is_none() {
        n.life.gaze_left = 0.0;
    }
    let standing = !n.walking;
    // an idle fidget: the head wanders a little while nothing holds it
    let (mut gy, mut gp) = (0.0, 0.0);
    if want.is_none() && standing && lv.fidget_yaw > 0.0 {
        if n.life.fidget_left <= 0.0 && n.life.fidget_cool <= 0.0 {
            let (a, b, h, g) = (roll_of(n, "fy"), roll_of(n, "fp"), roll_of(n, "fh"), roll_of(n, "fg"));
            n.life.fidget = ((a * 2.0 - 1.0) * lv.fidget_yaw, (b * 2.0 - 1.0) * lv.fidget_pitch);
            n.life.fidget_left = between(lv.fidget_hold, h);
            n.life.fidget_cool = between(lv.fidget_gap, g) / p.fidget_scale.max(0.1);
            n.life.fidgets += 1;
        }
        if n.life.fidget_left > 0.0 {
            (gy, gp) = n.life.fidget;
        }
    }
    if let Some((c, k)) = want {
        let d = (c.0 - at.0, c.1 - at.1);
        let to = if d.0.abs() + d.1.abs() < 0.05 { n.facing } else { heading_yaw(d) };
        let raw = wrapf(to - n.facing, -PI, PI);
        gy = raw.clamp(-lv.max_yaw, lv.max_yaw);
        gp = if k == Gaze::Chat { lv.nod_amount * (n.life.nod_clock * lv.nod_rate).sin() } else { 0.0 };
        // a standing bot turns its body to a teammate it answers or chats with, and to what the head cannot reach
        if standing && (matches!(k, Gaze::Peer | Gaze::Chat) || raw.abs() > lv.body_over) {
            turn(n, to, dt);
        }
    }
    let gp = gp.clamp(-lv.max_pitch, lv.max_pitch);
    let k = 1.0 - (-lv.rate * dt).exp();
    n.life.head_yaw += (gy - n.life.head_yaw) * k;
    n.life.head_pitch += (gp - n.life.head_pitch) * k;
}

// ---- the "life" intent: a bot that acts like a player who has stopped to look about -----------------------------------

/// What a bot does at a spot (data `life.poi.*.acts` and `life.durations` name them in lower case).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifeAct {
    LookAround,
    CheckLocker,
    CheckDoor,
    InspectRelay,
    Idle,
    Crouch,
    SprintBurst,
    Emote,
}

impl LifeAct {
    pub const ALL: [LifeAct; 8] = [
        LifeAct::LookAround,
        LifeAct::CheckLocker,
        LifeAct::CheckDoor,
        LifeAct::InspectRelay,
        LifeAct::Idle,
        LifeAct::Crouch,
        LifeAct::SprintBurst,
        LifeAct::Emote,
    ];

    pub fn name(self) -> &'static str {
        match self {
            LifeAct::LookAround => "look_around",
            LifeAct::CheckLocker => "check_locker",
            LifeAct::CheckDoor => "check_door",
            LifeAct::InspectRelay => "inspect_relay",
            LifeAct::Idle => "idle",
            LifeAct::Crouch => "crouch",
            LifeAct::SprintBurst => "sprint_burst",
            LifeAct::Emote => "emote",
        }
    }

    pub fn from_name(s: &str) -> Option<LifeAct> {
        LifeAct::ALL.into_iter().find(|a| a.name() == s)
    }
}

/// One kind of spot a bot may go to (data `life.poi`): `kind` is a hunter points-of-interest id (`PoiKind::id`) or
/// "here" (stay where it stands), `weight` is its share of the draw, `acts` what it may do there.
#[derive(Clone, Debug, PartialEq)]
pub struct LifePoi {
    pub kind: String,
    pub weight: f32,
    pub acts: Vec<(LifeAct, f32)>,
}

/// The "life" intent's numbers (data `life`). A missing key keeps the default; with no `poi` the intent never acts.
#[derive(Clone, Debug, PartialEq)]
pub struct LifeTuning {
    /// Metres from the runner a bot goes to at most. Past it the follow intent walks the bot back.
    pub range: f32,
    /// Metres from the runner at which a returning bot is back (also capped by the follow range).
    pub return_distance: f32,
    /// Seconds a bot may stay returning; after that it follows plainly until it is back inside `range`.
    pub return_timeout: f32,
    /// Seconds a plain follow (`follow_only`) may go without closing in on the runner by `goals.stuck.progress` before
    /// it is dropped with a fresh route.
    pub follow_only_timeout: f32,
    /// Metres a spot must be from the bot to be picked.
    pub min_distance: f32,
    /// Metres from a spot at which the bot is there.
    pub arrive: f32,
    /// Seconds after which a walk that has not arrived starts the activity where the bot stands.
    pub walk_timeout: f32,
    /// Spots remembered so none is picked twice in a row.
    pub recent: usize,
    /// [min, max] seconds of standing between two activities, and before the first one after spawning.
    pub pause: (f32, f32),
    pub spawn_pause: (f32, f32),
    /// The look-around sweep: total degrees and how many times it goes across.
    pub look_degrees: f32,
    pub look_sweeps: f32,
    /// Least seconds between two emotes of one bot.
    pub emote_gap: f32,
    /// Emote ids (data/player/emotes.json) and their weights.
    pub emotes: Vec<(String, f32)>,
    pub poi: Vec<LifePoi>,
    /// [min, max] seconds of each activity.
    pub durations: Vec<(LifeAct, (f32, f32))>,
}

impl Default for LifeTuning {
    fn default() -> LifeTuning {
        LifeTuning {
            range: 9.0,
            return_distance: 6.0,
            return_timeout: 8.0,
            follow_only_timeout: 10.0,
            min_distance: 2.0,
            arrive: 0.7,
            walk_timeout: 14.0,
            recent: 4,
            pause: (1.5, 5.0),
            spawn_pause: (0.5, 2.0),
            look_degrees: 140.0,
            look_sweeps: 1.5,
            emote_gap: 20.0,
            emotes: vec![],
            poi: vec![],
            durations: vec![],
        }
    }
}

impl LifeTuning {
    pub fn parse(v: &Value) -> LifeTuning {
        let d = LifeTuning::default();
        let num = |node: &Value, key: &str, d: f32| node[key].as_f64().map_or(d, |x| x as f32).max(0.0);
        let pair = |node: &Value, d: (f32, f32)| -> (f32, f32) {
            match (node[0].as_f64(), node[1].as_f64()) {
                (Some(a), Some(b)) => {
                    let (a, b) = (a.max(0.0) as f32, b.max(0.0) as f32);
                    (a.min(b), a.max(b))
                }
                _ => d,
            }
        };
        let weights = |node: &Value| -> Vec<(String, f32)> {
            node.as_object()
                .map(|m| m.iter().filter_map(|(k, w)| w.as_f64().map(|w| (k.clone(), (w as f32).max(0.0)))).collect())
                .unwrap_or_default()
        };
        LifeTuning {
            range: num(v, "range", d.range),
            return_distance: num(v, "return_distance", d.return_distance),
            return_timeout: num(v, "return_timeout", d.return_timeout),
            follow_only_timeout: num(v, "follow_only_timeout", d.follow_only_timeout),
            min_distance: num(v, "min_distance", d.min_distance),
            arrive: num(v, "arrive", d.arrive),
            walk_timeout: num(v, "walk_timeout", d.walk_timeout),
            recent: v["recent"].as_u64().map_or(d.recent, |n| n.min(64) as usize),
            pause: pair(&v["pause"], d.pause),
            spawn_pause: pair(&v["spawn_pause"], d.spawn_pause),
            look_degrees: num(&v["look"], "degrees", d.look_degrees),
            look_sweeps: num(&v["look"], "sweeps", d.look_sweeps),
            emote_gap: num(v, "emote_gap", d.emote_gap),
            emotes: weights(&v["emotes"]),
            poi: v["poi"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(kind, p)| LifePoi {
                            kind: kind.clone(),
                            weight: num(p, "weight", 0.0),
                            acts: weights(&p["acts"])
                                .into_iter()
                                .filter_map(|(a, w)| LifeAct::from_name(&a).map(|a| (a, w)))
                                .collect(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            durations: v["durations"]
                .as_object()
                .map(|m| m.iter().filter_map(|(k, p)| LifeAct::from_name(k).map(|a| (a, pair(p, (2.0, 3.5))))).collect())
                .unwrap_or_default(),
        }
    }

    fn duration(&self, a: LifeAct, roll: f32) -> f32 {
        let (lo, hi) = self.durations.iter().find(|(x, _)| *x == a).map_or((2.0, 3.5), |(_, p)| *p);
        lo + (hi - lo) * roll.clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifePhase {
    /// Standing before the next pick.
    Pause,
    /// Walking to the spot.
    Walk,
    /// Doing the activity.
    Do,
}

/// What a bot is doing in its spare time. The wire reads `crouching`, `emote` and `emote_seq`.
#[derive(Clone, Debug)]
pub struct LifeState {
    pub phase: LifePhase,
    pub act: Option<LifeAct>,
    pub started: bool,
    pub target: (f32, f32),
    pub focus: (f32, f32),
    /// The facing the activity began with (the look sweep is centred on it).
    pub base: (f32, f32),
    /// Seconds left in the phase, and the length of the activity.
    pub clock: f32,
    pub total: f32,
    /// Picks and pauses so far: the seed of every roll.
    pub count: u32,
    pub recent: Vec<(i32, i32)>,
    /// True while the bot is past `life.range` and the follow intent walks it back.
    pub returning: bool,
    /// Seconds spent returning, and whether the timeout passed (plain follow until back inside `life.range`).
    pub return_clock: f32,
    pub follow_only: bool,
    /// While `follow_only`: the closest the bot has come to the runner, and seconds since it last closed in.
    pub follow_best: f32,
    pub follow_still: f32,
    pub crouching: bool,
    /// The emote playing now, and how many the bot has started (a repeat of one id is told apart by the count).
    pub emote: Option<String>,
    pub emote_seq: u32,
    pub since_emote: f32,
    /// The last activities done, newest last (at most `PICKED_KEPT`).
    pub picked: Vec<LifeAct>,
    /// The bot's personality (an index into `lively.personalities`), drawn once from its name and slot.
    pub personality: Option<usize>,
    /// Where the head looks: the offset from the body's facing (radians, + left) and the pitch (+ up). Host-only life
    /// state: a guest draws what the wire sends (net/npc_sync.rs `look`).
    pub head_yaw: f32,
    pub head_pitch: f32,
    /// What the bot looks at, and the seconds it still does.
    pub gaze: Option<((f32, f32), Gaze)>,
    pub gaze_left: f32,
    /// Seconds until the next glance at a hunter, look at the runner, fidget, and answer to an emote.
    pub glance_cool: f32,
    pub runner_cool: f32,
    pub fidget_cool: f32,
    pub react_cool: f32,
    /// The idle fidget in progress: seconds left, and its head offset (yaw, pitch).
    pub fidget_left: f32,
    pub fidget: (f32, f32),
    /// A chatter stop in progress (seconds left), and seconds until the next one (negative: not drawn yet).
    pub hold: f32,
    pub stop_cool: f32,
    /// An emote answer pending: seconds left, and where the emoter stands.
    pub react: Option<(f32, (f32, f32))>,
    /// The follow ring's scale for the current walk back, and whether the next walk back draws a fresh one.
    pub ring_var: f32,
    pub ring_fresh: bool,
    /// Draws made for the lively rolls, and the head nod clock.
    pub rolls: u32,
    pub nod_clock: f32,
    /// Counts of what the bot has done, for the tests and the logs.
    pub glances: u32,
    pub runner_looks: u32,
    pub fidgets: u32,
    pub stops: u32,
    pub replies: u32,
}

/// What a bot's head is turned to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gaze {
    /// A noise it heard.
    Sound,
    /// A hunter it sees.
    Hunter,
    /// The runner.
    Runner,
    /// A teammate (or the runner) that is emoting.
    Peer,
    /// The runner or a teammate it has stopped to chat with.
    Chat,
}

const PICKED_KEPT: usize = 32;

impl Default for LifeState {
    fn default() -> LifeState {
        LifeState {
            phase: LifePhase::Pause,
            act: None,
            started: false,
            target: (0.0, 0.0),
            focus: (0.0, 0.0),
            base: (0.0, 1.0),
            clock: 0.0,
            total: 0.0,
            count: 0,
            recent: vec![],
            returning: false,
            return_clock: 0.0,
            follow_only: false,
            follow_best: 0.0,
            follow_still: 0.0,
            crouching: false,
            emote: None,
            emote_seq: 0,
            since_emote: f32::MAX,
            picked: vec![],
            personality: None,
            head_yaw: 0.0,
            head_pitch: 0.0,
            gaze: None,
            gaze_left: 0.0,
            glance_cool: 0.0,
            runner_cool: 0.0,
            fidget_cool: 0.0,
            react_cool: 0.0,
            fidget_left: 0.0,
            fidget: (0.0, 0.0),
            hold: 0.0,
            stop_cool: -1.0,
            react: None,
            ring_var: 1.0,
            ring_fresh: true,
            rolls: 0,
            nod_clock: 0.0,
            glances: 0,
            runner_looks: 0,
            fidgets: 0,
            stops: 0,
            replies: 0,
        }
    }
}

impl LifeState {
    /// Ends what the bot was doing and stands for `pause` seconds (another intent took over, or the activity ran out).
    pub fn interrupt(&mut self, pause: f32) {
        self.phase = LifePhase::Pause;
        self.act = None;
        self.crouching = false;
        self.emote = None;
        self.hold = 0.0;
        self.clock = pause;
    }
}

/// A weighted draw: `roll` in 0..1 picks from `list`. None when every weight is zero.
fn weighted<T: Copy>(list: &[(T, f32)], roll: f32) -> Option<T> {
    let total: f32 = list.iter().map(|(_, w)| w.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut left = roll.clamp(0.0, 0.999_999) * total;
    for (v, w) in list {
        let w = w.max(0.0);
        if left < w {
            return Some(*v);
        }
        left -= w;
    }
    list.last().map(|(v, _)| *v)
}

/// A fresh seeded roll source for the bot's next decision.
fn life_rng(n: &mut Npc) -> Rng {
    n.life.count += 1;
    Rng::new(&format!("{}:life:{}", n.name, n.life.count))
}

fn between(p: (f32, f32), roll: f32) -> f32 {
    p.0 + (p.1 - p.0) * roll.clamp(0.0, 1.0)
}

/// Starts the activity where the bot stands.
fn life_begin(n: &mut Npc, l: &LifeTuning, lv: &Lively, rng: &mut Rng) {
    let act = n.life.act.unwrap_or(LifeAct::Idle);
    n.life.phase = LifePhase::Do;
    n.life.hold = 0.0;
    n.walking = false;
    n.path.clear();
    let mut length = l.duration(act, rng.next_f32());
    match act {
        LifeAct::Crouch => n.life.crouching = true,
        // a door check may be a crouch-peek round the corner
        LifeAct::CheckDoor => n.life.crouching = rng.next_f32() < personality_of(n, lv).peek_chance,
        LifeAct::Emote => {
            let set = crate::render::anim::emote_pose::set();
            let list: Vec<(&str, f32)> = l
                .emotes
                .iter()
                .filter(|(id, _)| set.map_or(false, |s| s.find(id).is_some()))
                .map(|(id, w)| (id.as_str(), *w))
                .collect();
            if let Some(id) = weighted(&list, rng.next_f32()) {
                length = set.and_then(|s| s.find(id)).map_or(length, |e| e.duration.max(0.1));
                n.life.emote = Some(id.to_string());
                n.life.emote_seq += 1;
                n.life.since_emote = 0.0;
            }
        }
        _ => {}
    }
    n.life.clock = length;
    n.life.total = length;
    n.life.picked.push(act);
    if n.life.picked.len() > PICKED_KEPT {
        n.life.picked.remove(0);
    }
}

/// Picks the next spot and activity: a weighted draw over "here" and the hunter points of interest of the bot's level
/// that lie within `range` of the runner, are far enough from the bot, are not the recent ones and are not near a point
/// hunters searched.
fn life_pick(n: &mut Npc, sc: &Scene, t: &Tuning) {
    use crate::hunters::activities::{dir_to, points_of_interest, Rules};
    let l = &t.life;
    let mut rng = life_rng(n);
    let pois = points_of_interest(sc.fac, Rules::get());
    let searched = n.searched.clone();
    let avoid = t.searched_avoid;
    let blocked = |p: (f32, f32)| sc.searching.iter().chain(searched.iter()).any(|s| pdist(*s, p) < avoid);
    let kind_of = |k: &str| l.poi.iter().find(|p| p.kind == k);
    // None is "stay here"
    let mut cands: Vec<(Option<usize>, f32)> = vec![];
    if let Some(p) = kind_of("here") {
        cands.push((None, p.weight));
    }
    for (i, poi) in pois.iter().enumerate() {
        let Some(p) = kind_of(poi.kind.id()) else { continue };
        let burnt = poi.locker.map_or(false, |k| sc.disabled.get(k).copied().unwrap_or(false));
        if p.weight <= 0.0
            || burnt
            || pdist(poi.stand, sc.leader) > l.range
            || pdist(poi.stand, n.at) < l.min_distance
            || n.life.recent.contains(&poi.id())
            || blocked(poi.stand)
        {
            continue;
        }
        cands.push((Some(i), p.weight));
    }
    let facing_now = (-n.facing.sin(), -n.facing.cos());
    let Some(choice) = weighted(&cands, rng.next_f32()) else {
        n.life.interrupt(between(l.pause, rng.next_f32()));
        return;
    };
    let (kind, target, focus) = match choice {
        None => ("here", n.at, (n.at.0 + facing_now.0, n.at.1 + facing_now.1)),
        Some(i) => (pois[i].kind.id(), pois[i].stand, pois[i].focus),
    };
    let emote_ready = n.life.since_emote >= l.emote_gap
        && crate::render::anim::emote_pose::set().map_or(false, |s| l.emotes.iter().any(|(id, w)| *w > 0.0 && s.find(id).is_some()));
    let acts: Vec<(LifeAct, f32)> = kind_of(kind)
        .map(|p| p.acts.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|(a, _)| match a {
            LifeAct::CheckLocker => choice.map_or(false, |i| pois[i].locker.is_some()),
            LifeAct::CheckDoor => choice.map_or(false, |i| pois[i].door.is_some()),
            LifeAct::Emote => emote_ready,
            _ => true,
        })
        .collect();
    let acts = scale_acts(&acts, personality_of(n, &t.lively));
    let act = weighted(&acts, rng.next_f32()).unwrap_or(LifeAct::Idle);
    n.life.act = Some(act);
    n.life.target = target;
    n.life.focus = focus;
    n.life.base = dir_to(target, focus, facing_now);
    if let Some(i) = choice {
        n.life.recent.push(pois[i].id());
        while n.life.recent.len() > l.recent {
            n.life.recent.remove(0);
        }
    }
    if choice.is_none() || pdist(n.at, target) <= l.arrive {
        life_begin(n, l, &t.lively, &mut rng);
    } else {
        n.life.phase = LifePhase::Walk;
        n.life.clock = l.walk_timeout;
        n.path.clear();
        n.path_age = REPATH;
    }
}

/// The "life" intent: a bot with nothing else to do stands, walks to a spot near the runner, does something there and
/// pauses, like a player. False (so the follow intent runs) when the bot is on another level, when `life.range` is passed
/// (it is walked back and resumes once in follow range) or when the data lists no spot.
fn life(n: &mut Npc, sc: &Scene, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    use crate::hunters::activities::sweep_facing;
    let l = &t.life;
    if l.poi.is_empty() || n.level != sc.runner_level {
        return false;
    }
    let d = pdist(n.at, sc.leader);
    if n.life.follow_only {
        if d <= l.range {
            n.life.follow_only = false;
            n.life.follow_still = 0.0;
        } else {
            // a plain follow that cannot close in (no route, a jam) is dropped after `follow_only_timeout` with a fresh
            // route: bots have no teleport to the runner, so this keeps follow_only from holding a bot for good
            if d < n.life.follow_best - t.goals.stuck.progress {
                n.life.follow_best = d;
                n.life.follow_still = 0.0;
            } else {
                n.life.follow_still += dt;
            }
            if n.life.follow_still >= l.follow_only_timeout {
                n.life.follow_only = false;
                n.life.follow_still = 0.0;
                n.path.clear();
                n.hop = None;
                n.path_age = REPATH;
            }
            return false;
        }
    }
    if n.life.returning {
        let follow_range = follow_range(n, t);
        n.life.return_clock += dt;
        if d <= follow_range.min(l.return_distance).min(l.range) && n.path.is_empty() {
            n.life.returning = false;
            n.life.return_clock = 0.0;
        } else if n.life.return_clock >= l.return_timeout {
            // the timeout: plain follow until the bot is back inside the life range
            n.life.returning = false;
            n.life.return_clock = 0.0;
            n.life.follow_only = d > l.range;
            n.life.follow_best = d;
            n.life.follow_still = 0.0;
            return false;
        } else {
            return false;
        }
    }
    if d > l.range {
        n.life.interrupt(l.pause.0);
        n.life.returning = true;
        n.life.return_clock = 0.0;
        return false;
    }
    n.life.since_emote = (n.life.since_emote + dt).min(f32::MAX / 2.0);
    if !n.life.started {
        n.life.started = true;
        let mut rng = life_rng(n);
        n.life.clock = between(l.spawn_pause, rng.next_f32());
    }
    match n.life.phase {
        LifePhase::Pause => {
            n.walking = false;
            n.life.clock -= dt;
            if n.life.clock <= 0.0 {
                life_pick(n, sc, t);
            }
        }
        LifePhase::Walk => {
            n.life.clock -= dt;
            let running = n.life.act == Some(LifeAct::SprintBurst);
            if pdist(n.at, n.life.target) <= l.arrive || n.life.clock <= 0.0 {
                let mut rng = life_rng(n);
                life_begin(n, l, &t.lively, &mut rng);
            } else if n.life.hold > 0.0 {
                // a chatter stop: the bot stands a moment, looking at the runner or a teammate
                n.life.hold -= dt;
                n.walking = false;
            } else {
                if !running && stop_due(n, &t.lively, dt) {
                    n.walking = false;
                } else {
                    let target = n.life.target;
                    walk_toward(n, sc.fac, target, running, t, dt, presses);
                }
            }
        }
        LifePhase::Do => {
            n.walking = false;
            n.life.clock -= dt;
            let progress = if n.life.total > 1e-4 { (1.0 - n.life.clock / n.life.total).clamp(0.0, 1.0) } else { 1.0 };
            let look = match n.life.act {
                Some(LifeAct::LookAround) | Some(LifeAct::SprintBurst) => {
                    Some(sweep_facing(n.life.base, progress, l.look_degrees, l.look_sweeps))
                }
                Some(LifeAct::CheckLocker) | Some(LifeAct::CheckDoor) | Some(LifeAct::InspectRelay) => {
                    Some(crate::hunters::activities::dir_to(n.at, n.life.focus, n.life.base))
                }
                _ => None,
            };
            if let Some(dir) = look {
                turn(n, heading_yaw(dir), dt);
            }
            if n.life.clock <= 0.0 {
                let mut rng = life_rng(n);
                let scale = personality_of(n, &t.lively).pause_scale;
                n.life.interrupt(between(l.pause, rng.next_f32()) * scale);
            }
        }
    }
    true
}

/// True when a hunter of any state is in the bot's view: inside `sight_range` with a clear line and either inside the
/// view cone about the bot's heading (2D) or inside `sense_range`. A bot that has not moved yet sees all round.
fn sees(n: &Npc, fac: &Facility, seen: &[Seen], f: &FleeTuning) -> bool {
    let cos_half = f.sight_half_angle.to_radians().cos();
    seen.iter().any(|&(c, _)| {
        let d = pdist(n.at, c);
        if d > f.sight_range || !fac.has_line_of_sight(n.at, c) {
            return false;
        }
        if d <= f.sense_range || n.heading == (0.0, 0.0) {
            return true;
        }
        ((c.0 - n.at.0) * n.heading.0 + (c.1 - n.at.1) * n.heading.1) / d >= cos_half
    })
}

/// True when the bot heard a noise event and a hunter is within `hear_range` of it.
fn hears_hunter(n: &Npc, seen: &[Seen], f: &FleeTuning) -> bool {
    n.heard.is_some() && seen.iter().any(|&(c, _)| pdist(n.at, c) < f.hear_range)
}

/// The flight route (cells, the bot's own first): a breadth-first walk over the open dry cells of the bot's level, never
/// within `clearance` of a hunter (a bot already closer never steps nearer), at most `depth` cells long, ending on the
/// cell farthest from the hunters. A flight never leaves the level: no link is taken. None when no cell is farther
/// from the hunters than the bot stands.
fn flee_route(n: &Npc, fac: &Facility, threats: &[(f32, f32)], f: &FleeTuning) -> Option<Vec<(i32, i32)>> {
    let start = cell_at(n.at);
    let far = |c: Cell| threats.iter().map(|&p| pdist(c.center(), p)).fold(f32::MAX, f32::min);
    let d0 = far(start);
    let floor = f.clearance.min(d0);
    let ok = |c: Cell| fac.hunter_open(c) && !n.deep_cells.contains(&(c.x, c.y)) && far(c) >= floor;
    let mut came: std::collections::HashMap<Cell, Cell> = std::collections::HashMap::new();
    let mut depth_of: std::collections::HashMap<Cell, usize> = std::collections::HashMap::from([(start, 0)]);
    let mut queue = std::collections::VecDeque::from([start]);
    let mut best: Option<(Cell, f32)> = None;
    while let Some(c) = queue.pop_front() {
        if depth_of.len() > f.max_cells {
            break;
        }
        let depth = depth_of[&c];
        let score = far(c) - 0.05 * depth as f32;
        if c != start && far(c) > d0 && best.map_or(true, |(_, s)| score > s) {
            best = Some((c, score));
        }
        if depth >= f.depth {
            continue;
        }
        for dy in -1..=1 {
            for dx in -1..=1 {
                let nb = Cell::new(c.x + dx, c.y + dy);
                if (dx == 0 && dy == 0) || depth_of.contains_key(&nb) || !ok(nb) {
                    continue;
                }
                if dx != 0 && dy != 0 && (!ok(Cell::new(c.x + dx, c.y)) || !ok(Cell::new(c.x, c.y + dy))) {
                    continue;
                }
                depth_of.insert(nb, depth + 1);
                came.insert(nb, c);
                queue.push_back(nb);
            }
        }
    }
    let (end, _) = best?;
    let mut path = vec![end];
    let mut at = end;
    while let Some(&p) = came.get(&at) {
        at = p;
        path.push(at);
    }
    path.reverse();
    Some(path.into_iter().map(|c| (c.x, c.y)).collect())
}

/// Godot `_think`'s flee (npc_teammates.gd 305-315), widened: a bot runs from a chasing hunter inside the smart-scaled
/// threat range, from a hunter it sees (`sees`) and from a hunter near a noise it heard (`hears_hunter`). It runs the
/// route `flee_route` picks, and keeps it until the goal is reached, the route is blocked, or a hunter comes within
/// `clearance` of what is left of it (then it re-plans, at most every `flee.replan` seconds). A bot with nowhere
/// farther to go stands. True when the bot acted.
fn flee(n: &mut Npc, fac: &Facility, seen: &[Seen], t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    let f = &t.flee;
    let range = t.threat_range * (t.smart_base + n.smart as f32 / 100.0);
    let alarmed = seen.iter().any(|&(c, chasing)| chasing && pdist(n.at, c) < range)
        || sees(n, fac, seen, f)
        || hears_hunter(n, seen, f)
        || (n.flee.alert > 0.0 && !seen.is_empty());
    if !alarmed {
        if n.flee.goal.is_some() {
            n.path.clear();
        }
        n.flee.stop();
        return false;
    }
    let threats: Vec<(f32, f32)> = seen.iter().map(|s| s.0).collect();
    n.flee.since += dt;
    let near_route = |n: &Npc| {
        let floor = f.clearance.min(threats.iter().map(|&p| pdist(n.at, p)).fold(f32::MAX, f32::min));
        n.path.iter().any(|&(x, y)| threats.iter().any(|&p| pdist(Cell::new(x, y).center(), p) < floor))
    };
    let stuck = match n.flee.goal {
        None => true,
        Some(g) => n.path.is_empty() || cell_at(n.at) == Cell::new(g.0, g.1) || near_route(n),
    };
    if stuck && n.flee.since >= f.replan {
        n.flee.since = 0.0;
        match flee_route(n, fac, &threats, f) {
            Some(route) => {
                n.flee.goal = route.last().copied();
                n.path = route.into_iter().skip(1).collect();
                n.path_age = 0.0;
            }
            None => {
                n.flee.goal = None;
                n.path.clear();
            }
        }
    }
    let Some(g) = n.flee.goal else {
        n.walking = false;
        return true;
    };
    walk_toward(n, fac, Cell::new(g.0, g.1).center(), t.flee_running, t, dt, presses);
    true
}

/// What a bot sees on one frame: the facility of its own level, which lockers are burnt out, the runner, the hunters
/// of its level that it sees and the points they are searching now.
struct Scene<'a> {
    fac: &'a Facility,
    disabled: &'a [bool],
    leader: (f32, f32),
    seen: &'a [Seen],
    searching: &'a [(f32, f32)],
    /// The level the runner stands on.
    runner_level: i32,
    /// The graph of flights, ladders and drops between the resident levels (None on a finite floor).
    nav: Option<&'a StackNav>,
    /// The batteries and loose items on the active level a bot may take: what it is, and where.
    loot: &'a [(Claim, (f32, f32))],
    /// The places a bot may make its goal, on every resident level.
    spots: &'a [Spot],
    /// The points hunters are searching, with their level.
    search_all: &'a [(i32, (f32, f32))],
    /// The downed bodies a bot may walk to: level, place, and the bot id (None for the runner).
    downed: &'a [(i32, (f32, f32), Option<u32>)],
}

/// The step cost a bot plans across levels with: deep water on its own level, and with `avoid_search` the cells within
/// `searched_avoid` of a point a hunter is searching (data `goals.search_cost`).
fn step_cost<'a>(n: &'a Npc, sc: &'a Scene, t: &'a Tuning, avoid_search: bool) -> impl Fn(i32, Cell, Cell) -> Option<f32> + 'a {
    move |level: i32, _from: Cell, to: Cell| {
        let mut c = 1.0f32;
        if level == n.level && n.deep_cells.contains(&(to.x, to.y)) {
            c = c.max(t.deep_step_cost);
        }
        if avoid_search {
            let p = to.center();
            if sc.search_all.iter().any(|(l, s)| *l == level && pdist(*s, p) < t.searched_avoid) {
                c = c.max(t.goals.search_cost);
            }
        }
        Some(c)
    }
}

/// Sets the bot's cross-level walk going: the flight or ladder it takes toward the runner's level. It plans to the
/// hop's entrance on its own level, walks there (`walk_toward`), and starts the `EdgeWalk` once it is there. With no
/// route the bot stands and asks again in `REPATH` seconds.
fn follow_across(n: &mut Npc, sc: &Scene, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) {
    let target = Node { level: sc.runner_level, cell: cell_at(sc.leader) };
    cross_to(n, sc, target, false, t, dt, presses);
}

/// Walks the bot toward `target` on another level: the hop it takes, its entrance, then the edge. `avoid_search` makes
/// cells hunters are searching costly on the route.
fn cross_to(n: &mut Npc, sc: &Scene, target: Node, avoid_search: bool, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) {
    let Some(nav) = sc.nav else {
        n.walking = false;
        return;
    };
    let repath = repath_for(n);
    if n.hop.is_none() || n.path_age >= repath {
        let hop = {
            let cost = step_cost(n, sc, t, avoid_search);
            next_hop(nav, Node { level: n.level, cell: cell_at(n.at) }, target, &cost)
        };
        // a fresh hop is planned to on this frame; with no route the next ask waits a full repath
        n.path_age = if hop.is_some() { repath } else { 0.0 };
        n.hop = hop;
    }
    let Some(hop) = n.hop.clone() else {
        n.walking = false;
        n.path.clear();
        return;
    };
    let entrance = hop.entrance.center();
    let d = pdist(n.at, entrance);
    if d <= EDGE_ENTER_REACH || (n.path.is_empty() && n.path_age < REPATH && d <= EDGE_ENTER_SNAP) {
        let Some(walk) = edge_start(nav, n.level, &hop) else {
            // the edge is gone (a rebuild dropped it): plan again
            n.hop = None;
            n.path_age = REPATH;
            return;
        };
        n.walk = Some(walk);
        n.edge_clock = 0.0;
        n.hop = None;
        n.path.clear();
        n.door = None;
        n.walking = true;
        return;
    }
    walk_toward(n, sc.fac, entrance, false, t, dt, presses);
}

/// Moves the bot to `level`. Errand indices and visits belong to one storey, so a change of storey drops them.
fn set_level(n: &mut Npc, level: i32) {
    if n.level != level {
        n.level = level;
        n.visited.clear();
        n.errand = None;
        n.dwell = 0.0;
    }
}

/// Puts the bot on the nearer end of the edge it was walking, with its route and hop cleared so it plans again.
fn settle_on_nearer_end(n: &mut Npc, w: &EdgeWalk) {
    let p = if w.fraction() < 0.5 { w.start_point() } else { w.end_point() };
    let (at, y) = ((p[0], p[1]), p[2]);
    n.at = at;
    n.y = y;
    set_level(n, level_at(y, n.level));
    n.walk = None;
    n.edge_clock = 0.0;
    n.hop = None;
    n.path.clear();
    n.path_age = REPATH;
    n.walking = false;
}

/// One frame along the bot's edge: it moves `speed * dt` along the flight (walk speed) or ladder (climb speed), its y
/// follows the edge, and its level is derived from that y. At the far end the walk and the route are cleared. A bot on
/// an edge for longer than its length at that speed plus the data `edge_stuck_seconds` is put on the nearer end.
fn advance_edge(n: &mut Npc, dt: f32) {
    let Some(mut w) = n.walk.take() else { return };
    n.edge_clock += dt;
    let speed = edge_speed(&w);
    if n.edge_clock > w.len / speed + edge_stuck_seconds() {
        settle_on_nearer_end(n, &w);
        return;
    }
    let from = n.at;
    let (at, y, done) = edge_advance(&mut w, dt);
    let moved = pdist(from, at);
    if moved > 1e-4 {
        n.heading = ((at.0 - from.0) / moved, (at.1 - from.1) / moved);
        let heading = n.heading;
        turn(n, heading_yaw(heading), dt);
    }
    n.stride += moved + (y - n.y).abs();
    n.at = at;
    n.y = y;
    set_level(n, level_at(y, n.level));
    n.walking = true;
    n.door = None;
    n.path.clear();
    if done {
        n.edge_clock = 0.0;
        n.hop = None;
        n.path_age = REPATH;
    } else {
        n.walk = Some(w);
    }
}

/// True when a chasing hunter is nearer `p` than `range` metres.
fn chaser_within(seen: &[Seen], p: (f32, f32), range: f32) -> bool {
    seen.iter().any(|&(c, chasing)| chasing && pdist(p, c) < range)
}

/// True when one of `points` is nearer `p` than `r` metres.
fn near_any(points: &[(f32, f32)], p: (f32, f32), r: f32) -> bool {
    points.iter().any(|&q| pdist(p, q) < r)
}

/// Records the points hunters are searching now. A point within `searched_avoid` of one already kept is not new. The
/// newest `searched_memory` points are kept.
fn note_searched(n: &mut Npc, searching: &[(f32, f32)], t: &Tuning) {
    for &p in searching {
        if !near_any(&n.searched, p, t.searched_avoid) {
            n.searched.push(p);
        }
    }
    let excess = n.searched.len().saturating_sub(t.searched_memory);
    n.searched.drain(..excess);
}

/// The bot is inside locker `i`: hidden, at the locker's centre, not moving.
fn hide_in(n: &mut Npc, centre: (f32, f32), i: usize) {
    n.flee.stop();
    n.hidden = Some(i);
    n.at = centre;
    n.path.clear();
    n.walking = false;
    n.door = None;
    n.clear_time = 0.0;
}

/// The bot comes out of locker `i` at its front (`hiding_exit`) and forgets the route it had. `i` must index `fac.hiding`.
fn come_out(n: &mut Npc, fac: &Facility, i: usize) {
    n.hidden = None;
    n.clear_time = 0.0;
    n.at = crate::hunters::gimmicks::hiding_exit(fac, i);
    n.path.clear();
    n.path_age = REPATH;
}

/// Walks to locker `i` (running when `running`). Once within `enter_radius` of its centre the bot is inside it.
fn go_to_locker(n: &mut Npc, sc: &Scene, i: usize, running: bool, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) {
    let centre = sc.fac.locker_center(&sc.fac.hiding[i]);
    if pdist(n.at, centre) <= t.enter_radius {
        hide_in(n, centre, i);
    } else {
        walk_toward(n, sc.fac, centre, running, t, dt, presses);
    }
}

/// Hide (data `hide`): a chasing hunter within `chase_range`, a hunter in view, or a hunter near a noise the bot heard,
/// and a locker within `reach` that the bot gets to before any hunter and that is clear of them, send the bot running to
/// the nearest such locker, not burnt out. True when the bot acted.
fn hide(n: &mut Npc, sc: &Scene, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    let f = &t.flee;
    if !(chaser_within(sc.seen, n.at, t.hide_chase_range)
            || sees(n, sc.fac, sc.seen, f)
            || hears_hunter(n, sc.seen, f)
            || (n.flee.alert > 0.0 && !sc.seen.is_empty()))
    {
        return false;
    }
    let hunter_to = |p: (f32, f32)| sc.seen.iter().map(|&(c, _)| pdist(p, c)).fold(f32::MAX, f32::min);
    let near = sc
        .fac
        .hiding
        .iter()
        .enumerate()
        .filter(|(i, _)| !sc.disabled.get(*i).copied().unwrap_or(false))
        .map(|(i, h)| (i, sc.fac.locker_center(h)))
        .map(|(i, c)| (i, pdist(n.at, c), hunter_to(c)))
        .filter(|&(_, d, hd)| d <= t.hide_reach && hd > d && hd >= f.clearance)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let Some((i, _, _)) = near else { return false };
    n.flee.stop();
    go_to_locker(n, sc, i, true, t, dt, presses);
    true
}

/// A hidden bot stays put. On a locker errand it comes out once its dwell is up and no chaser is near. Otherwise it
/// comes out once no chaser has been within `hide_leave_range` for `hide_leave_delay` seconds.
fn hidden_tick(n: &mut Npc, sc: &Scene, t: &Tuning, dt: f32) {
    n.walking = false;
    let Some(i) = n.hidden else { return };
    if i >= sc.fac.hiding.len() {
        n.hidden = None;
        return;
    }
    let chased = sc.seen.iter().any(|&(c, _)| pdist(n.at, c) < t.hide_leave_range);
    n.clear_time = if chased { 0.0 } else { n.clear_time + dt };
    let on_errand = n.errand == Some(Errand::Locker(i));
    let leave = if on_errand {
        n.dwell += dt;
        n.dwell >= t.locker_dwell && !chased
    } else {
        n.clear_time >= t.hide_leave_delay
    };
    if leave {
        if on_errand {
            n.visited.push(Errand::Locker(i));
            n.errand = None;
            n.dwell = 0.0;
        }
        come_out(n, sc.fac, i);
    }
}

/// The centre of an errand's spot, or None for an index the floor does not have.
fn errand_spot(fac: &Facility, e: Errand) -> Option<(f32, f32)> {
    match e {
        Errand::Relay(i) => fac.relays.get(i).map(|c| c.center()),
        Errand::Locker(i) => fac.hiding.get(i).map(|h| fac.locker_center(h)),
    }
}

/// Whether a spot is safe for an errand: no chasing hunter within `errand_safe_range`, no hunter searching within
/// `searched_avoid`, and no point the bot itself saw searched within `searched_avoid`.
fn spot_safe(n: &Npc, sc: &Scene, spot: (f32, f32), t: &Tuning) -> bool {
    !chaser_within(sc.seen, spot, t.errand_safe_range)
        && !near_any(sc.searching, spot, t.searched_avoid)
        && !near_any(&n.searched, spot, t.searched_avoid)
}

/// The nearest spot of the kind (a relay, or a locker not burnt out) the bot has not been to, within `leash` of the
/// runner and safe to go to.
fn pick_errand(n: &Npc, sc: &Scene, locker: bool, leash: f32, t: &Tuning) -> Option<Errand> {
    let count = if locker { sc.fac.hiding.len() } else { sc.fac.relays.len() };
    let mut best: Option<(Errand, f32)> = None;
    for i in 0..count {
        let e = if locker { Errand::Locker(i) } else { Errand::Relay(i) };
        if n.visited.contains(&e) || (locker && sc.disabled.get(i).copied().unwrap_or(false)) {
            continue;
        }
        let Some(spot) = errand_spot(sc.fac, e) else { continue };
        if pdist(spot, sc.leader) > leash || !spot_safe(n, sc, spot, t) {
            continue;
        }
        let d = pdist(n.at, spot);
        if best.map_or(true, |(_, bd)| d < bd) {
            best = Some((e, d));
        }
    }
    best.map(|(e, _)| e)
}

/// A relay or locker errand (data `errand`; `locker` picks which). A joined bot with nothing chasing it heads for the
/// nearest safe spot within the kind's leash of the runner, stands there for the kind's dwell and then goes back to
/// following. A locker errand hides the bot in the locker for its dwell (`hidden_tick` keeps the time). True when the
/// bot acted.
fn errand(n: &mut Npc, sc: &Scene, locker: bool, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    let (leash, dwell) = if locker {
        (t.locker_leash, t.locker_dwell)
    } else {
        (t.relay_leash, t.relay_dwell)
    };
    let is_mine = |e: &Errand| matches!(e, Errand::Locker(_)) == locker;
    match n.errand {
        Some(e) if is_mine(&e) => {
            let safe = match errand_spot(sc.fac, e) {
                Some(spot) => spot_safe(n, sc, spot, t),
                None => false,
            };
            if !safe {
                n.errand = None;
                n.dwell = 0.0;
                return false;
            }
        }
        // the other kind's errand is in hand; its own intent carries it on
        Some(_) => return false,
        None => match pick_errand(n, sc, locker, leash, t) {
            Some(e) => {
                n.errand = Some(e);
                n.dwell = 0.0;
            }
            None => return false,
        },
    }
    let Some(e) = n.errand else { return false };
    let Some(spot) = errand_spot(sc.fac, e) else {
        n.errand = None;
        return false;
    };
    if pdist(n.at, spot) > t.errand_reach {
        walk_toward(n, sc.fac, spot, false, t, dt, presses);
        return true;
    }
    match e {
        Errand::Locker(i) => hide_in(n, spot, i),
        Errand::Relay(_) => {
            n.walking = false;
            n.dwell += dt;
            if n.dwell >= t.relay_claim_time {
                if let Errand::Relay(i) = e {
                    n.claim = Some(Claim::Relay(i));
                }
            }
            if n.dwell >= dwell {
                n.visited.push(e);
                n.errand = None;
                n.dwell = 0.0;
            }
        }
    }
    true
}

/// The pickup intent (data `pickup`): a joined bot with nothing chasing it walks to the nearest useful battery or item
/// within `pickup_leash` of the runner, safe by the errand rules, and claims it once it stands within `errand_reach`.
/// True when the bot acted.
fn fetch(n: &mut Npc, sc: &Scene, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    let mut best: Option<(Claim, (f32, f32), f32)> = None;
    for &(claim, pos) in sc.loot {
        if pdist(pos, sc.leader) > t.pickup_leash || !spot_safe(n, sc, pos, t) {
            continue;
        }
        let d = pdist(n.at, pos);
        if best.map_or(true, |b| d < b.2) {
            best = Some((claim, pos, d));
        }
    }
    let Some((claim, pos, d)) = best else { return false };
    if d > t.errand_reach {
        walk_toward(n, sc.fac, pos, false, t, dt, presses);
    } else {
        n.walking = false;
        n.claim = Some(claim);
    }
    true
}

/// The path cost of a route (a plain step is the step cost, an edge `goals.edge_cost`), None when there is none.
fn route_cost(n: &Npc, sc: &Scene, t: &Tuning, from: Node, to: Node) -> Option<f32> {
    // the same level: the straight search of the facility needs no stack, and is all a finite floor (an empty graph) has
    let direct = if from.level == to.level { sc.fac.find_path(from.cell, to.cell) } else { vec![] };
    let direct_cost = (!direct.is_empty()).then(|| direct.len() as f32);
    let Some(nav) = sc.nav else { return direct_cost };
    let cost = step_cost(n, sc, t, true);
    // a same-level route that meets no deep water or searched cell is the nav search's answer too: skip the second search
    if direct_cost.is_some() && direct.windows(2).all(|w| cost(from.level, w[0], w[1]).map_or(false, |c| c <= 1.0)) {
        return Some((direct.len() - 1) as f32);
    }
    let route = nav.find_path(from, to, &cost);
    if route.is_empty() {
        return direct_cost;
    }
    let mut total = 0.0;
    for w in route.windows(2) {
        total += if w[1].via.is_some() {
            t.goals.edge_cost
        } else {
            cost(w[1].node.level, w[0].node.cell, w[1].node.cell).unwrap_or(1.0).max(1.0)
        };
    }
    Some(total)
}

/// The best spot for the bot: the nearest `goals.candidates` by straight distance (a storey counts `level_metres`),
/// scored `weight - min(cost * distance_weight, distance_cap)`; a spot the bot has done, that is unsafe on its own level
/// or that has no route is left out. The first of equal scores wins, so a pick is the same on every run.
fn pick_goal(n: &Npc, sc: &Scene, t: &Tuning) -> Option<Spot> {
    let g = &t.goals;
    let mut near: Vec<(f32, &Spot)> = sc
        .spots
        .iter()
        .filter(|s| g.weight(s.kind) > 0.0 && !n.goal.done.contains(&s.key()) && !n.goal.banned.iter().any(|(k, _)| *k == s.key()))
        .filter(|s| s.level != n.level || spot_safe(n, sc, s.pos, t))
        .map(|s| (pdist(n.at, s.pos) + (s.level - n.level).abs() as f32 * g.level_metres, s))
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let from = Node { level: n.level, cell: sc.fac.nearest_open(n.at.0, n.at.1, NAV_RADIUS).map_or(cell_at(n.at), cell_at) };
    let mut best: Option<(f32, Spot)> = None;
    for (_, s) in near.into_iter().take(g.candidates) {
        let Some(cost) = route_cost(n, sc, t, from, Node { level: s.level, cell: s.cell }) else { continue };
        let score = g.weight(s.kind) - (cost * g.distance_weight).min(g.distance_cap);
        if best.map_or(true, |(b, _)| score > b) {
            best = Some((score, *s));
        }
    }
    best.map(|(_, s)| s)
}

/// The "goal" intent (data `goals`): a joined bot with nothing chasing it picks a goal anywhere on the resident levels
/// (relay, then battery or heal when low, then useful pickups, then the exit once the relays are done) and walks to it
/// across stairs and ladders. It picks again only when it has no goal and `replan` seconds have passed since it found
/// none or dropped one, and a goal not reached in `give_up` seconds is dropped for good. A goal that stops being
/// available, or becomes unsafe on the bot's own level, is dropped. True when the bot acted.
fn run_goal(n: &mut Npc, sc: &Scene, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    n.goal.cool = (n.goal.cool - dt).max(0.0);
    n.goal.age += dt;
    for b in n.goal.banned.iter_mut() {
        b.1 -= dt;
    }
    n.goal.banned.retain(|b| b.1 > 0.0);
    // the leash: a bot farther than `leash_distance` from the runner for more than `leash_away` seconds drops its goal
    // and takes no new one until it is back within the distance (the follow intent brings it)
    n.goal.away = if pdist(n.at, sc.leader) > t.goals.leash_distance { n.goal.away + dt } else { 0.0 };
    if n.goal.away > t.goals.leash_away {
        if n.goal.spot.take().is_some() {
            n.goal.cool = goal_cool_for(n, t);
            n.goal.stuck = StuckState::default();
            n.path.clear();
            n.hop = None;
        }
        return false;
    }
    let mut cur = None;
    if let Some(s) = n.goal.spot {
        cur = sc.spots.iter().find(|x| x.same(&s)).copied().filter(|x| x.level != n.level || spot_safe(n, sc, x.pos, t));
        if cur.is_none() {
            n.goal.spot = None;
            n.goal.cool = goal_cool_for(n, t);
        } else if n.goal.age > t.goals.give_up {
            n.goal.done.push(s.key());
            n.goal.spot = None;
            n.goal.cool = goal_cool_for(n, t);
            cur = None;
        }
    }
    let spot = match cur {
        Some(s) => s,
        None => {
            if n.goal.cool > 0.0 {
                return false;
            }
            let Some(s) = pick_goal(n, sc, t) else {
                n.goal.cool = goal_cool_for(n, t);
                return false;
            };
            n.goal = GoalState {
                spot: Some(s),
                picks: n.goal.picks + 1,
                done: std::mem::take(&mut n.goal.done),
                banned: std::mem::take(&mut n.goal.banned),
                ..GoalState::default()
            };
            n.hop = None;
            n.path_age = REPATH;
            s
        }
    };
    n.goal.spot = Some(spot);
    if spot.level != n.level || pdist(n.at, spot.pos) > t.errand_reach {
        if stuck_watch(n, sc, spot, t, dt, presses) {
            return true;
        }
    } else {
        n.goal.stuck = StuckState::default();
    }
    if spot.level != n.level {
        cross_to(n, sc, Node { level: spot.level, cell: spot.cell }, true, t, dt, presses);
        return true;
    }
    if pdist(n.at, spot.pos) > t.errand_reach {
        walk_toward(n, sc.fac, spot.pos, false, t, dt, presses);
        return true;
    }
    n.walking = false;
    n.path.clear();
    match spot.kind {
        SpotKind::Exit => {}
        SpotKind::Relay => {
            n.goal.dwell += dt;
            if n.goal.dwell >= t.relay_claim_time {
                n.claim = spot.claim;
            }
            if n.goal.dwell >= t.relay_dwell {
                n.goal.done.push(spot.key());
                n.goal.spot = None;
            }
        }
        _ => {
            n.claim = spot.claim;
            n.goal.done.push(spot.key());
            n.goal.spot = None;
        }
    }
    true
}

/// The bot's stuck watch while it walks to `spot` (data `goals.stuck`). No progress for `seconds` steps up one rung of
/// the ladder each window: a fresh route, a press on a shut door beside it, a step aside (a seeded direction), and last
/// the goal given up for `ban_seconds`. Progress ends the jam. True when the bot used this frame (a step aside, or the
/// give-up).
fn stuck_watch(n: &mut Npc, sc: &Scene, spot: Spot, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    let s = &t.goals.stuck;
    if n.goal.stuck.aside > 0.0 {
        let dir = n.goal.stuck.aside_dir;
        n.goal.stuck.aside -= dt;
        let to = (n.at.0 + dir.0 * PLAYER_WALK_SPEED * dt, n.at.1 + dir.1 * PLAYER_WALK_SPEED * dt);
        if matches!(sweep(sc.fac, n.at, to), Sweep::Clear) {
            n.at = to;
            n.heading = dir;
            let heading = n.heading;
            turn(n, heading_yaw(heading), dt);
            n.stride += PLAYER_WALK_SPEED * dt;
            n.walking = true;
        } else {
            n.goal.stuck.aside = 0.0;
        }
        if n.goal.stuck.aside <= 0.0 {
            n.goal.stuck.anchor = n.at;
            n.goal.stuck.still = 0.0;
            n.path.clear();
            n.hop = None;
            n.path_age = REPATH;
        }
        return true;
    }
    let st = &mut n.goal.stuck;
    if pdist(n.at, st.anchor) >= s.progress {
        *st = StuckState { anchor: n.at, count: st.count, ..StuckState::default() };
        return false;
    }
    st.still += dt;
    if st.still < s.seconds {
        return false;
    }
    st.still = 0.0;
    st.stage += 1;
    st.count += 1;
    match st.stage {
        1 => {
            n.path.clear();
            n.hop = None;
            n.path_age = REPATH;
        }
        2 => {
            if let Some(d) = door_near(sc.fac, n.at, s.door_reach) {
                if !sc.fac.door_is_open(d) && t.does("door") {
                    presses.push(d);
                }
            }
        }
        3 => {
            let mut rng = Rng::new(&format!("{}:aside:{}:{}", n.name, n.goal.picks, n.goal.stuck.count));
            let a0 = rng.next_f32() * 2.0 * PI;
            for k in 0..8 {
                let a = a0 + k as f32 * PI / 4.0;
                let dir = (a.sin(), a.cos());
                let to = (n.at.0 + dir.0 * 0.5, n.at.1 + dir.1 * 0.5);
                if sc.fac.body_fits(to.0, to.1, NAV_RADIUS) && matches!(sweep(sc.fac, n.at, to), Sweep::Clear) {
                    n.goal.stuck.aside = s.aside_seconds;
                    n.goal.stuck.aside_dir = dir;
                    break;
                }
            }
        }
        _ => {
            n.goal.banned.push((spot.key(), s.ban_seconds));
            n.goal.spot = None;
            n.goal.cool = goal_cool_for(n, t);
            n.goal.stuck = StuckState { anchor: n.at, count: n.goal.stuck.count, ..StuckState::default() };
            n.path.clear();
            n.hop = None;
            n.walking = false;
            return true;
        }
    }
    false
}

/// The "revive" intent: a bot walks to the nearest downed teammate on its floor inside `revive_range_factor` times the follow
/// radius, and stands beside it (the hold itself is `Game::tick_revives`). False when there is none.
fn revive_walk(n: &mut Npc, sc: &Scene, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    let range = t.follow_radius * t.revive_range_factor;
    let target = sc
        .downed
        .iter()
        .filter(|d| d.0 == n.level && d.2 != Some(n.id) && pdist(n.at, d.1) <= range)
        .min_by(|a, b| pdist(n.at, a.1).total_cmp(&pdist(n.at, b.1)));
    let Some(&(_, spot, _)) = target else { return false };
    if pdist(n.at, spot) <= super::revive::rules().reach * t.revive_stand {
        n.walking = false;
        n.path.clear();
    } else {
        walk_toward(n, sc.fac, spot, false, t, dt, presses);
    }
    true
}

/// One intent of the bot's list (data `intents`). Returns true when it acted. "flee" acts on any bot. The others need
/// the bot joined to the runner. A name the bot does not know does nothing.
fn run_intent(n: &mut Npc, sc: &Scene, intent: &str, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    match intent {
        "flee" => flee(n, sc.fac, sc.seen, t, dt, presses),
        "revive" if n.joined && n.hidden.is_none() => revive_walk(n, sc, t, dt, presses),
        _ if !n.joined => false,
        "goal" => run_goal(n, sc, t, dt, presses),
        // hiding and errands are own-level intents: they only run on the runner's level
        "hide" | "relay" | "locker" | "pickup" if n.level != sc.runner_level => false,
        "pickup" => fetch(n, sc, t, dt, presses),
        "hide" => hide(n, sc, t, dt, presses),
        "relay" => errand(n, sc, false, t, dt, presses),
        "locker" => errand(n, sc, true, t, dt, presses),
        "life" => life(n, sc, t, dt, presses),
        "follow" if n.level != sc.runner_level => {
            follow_across(n, sc, t, dt, presses);
            true
        }
        "follow" => {
            follow(n, sc.fac, sc.leader, t, dt, presses);
            true
        }
        _ => false,
    }
}

impl Game {
    /// The infinite level the runner stands on (0 on a finite route).
    pub fn runner_level(&self) -> i32 {
        self.inf.as_ref().map_or(0, |i| i.level)
    }

    /// Godot `RoomState.run_seed() + id` for the NPC's name.
    fn npc_name(&self, id: u32) -> String {
        guest_name_from_seed(run_seed_of_text(&self.room_text) as u64 + id as u64)
    }

    /// The crosshair's floor point (Godot `crosshair_floor`): a ray from the eye along `dir`, up to
    /// `SPAWN_RAY_RANGE` metres, that must reach the floor over open cells. A wall in the way gives no point.
    fn npc_crosshair_floor(&self, eye: (f32, f32, f32), dir: (f32, f32, f32), floor_y: f32) -> Option<(f32, f32)> {
        let mut d = 0.0f32;
        while d <= SPAWN_RAY_RANGE {
            let p = (eye.0 + dir.0 * d, eye.1 + dir.1 * d, eye.2 + dir.2 * d);
            if !self
                .fac
                .hunter_open(Cell::new(p.0.floor() as i32, p.2.floor() as i32))
            {
                return None;
            }
            if p.1 <= floor_y + 0.05 {
                return Some((p.0, p.2));
            }
            d += RAY_STEP;
        }
        None
    }

    /// The admin spawn (Godot `NpcTeammates.spawn` with the host's spot rule, `infinite_route_test.gd`
    /// 6769-6779). `count` NPCs go round the crosshair floor point when it is open and within
    /// `SPAWN_NEAR_ADMIN`; otherwise round the runner. `eye` and `dir` are the crosshair ray (the
    /// runner's eye and forward). Returns how many were made. No server is involved.
    pub fn npc_spawn(
        &mut self,
        count: i32,
        smart: i32,
        eye: (f32, f32, f32),
        dir: (f32, f32, f32),
    ) -> usize {
        if count <= 0 {
            return 0;
        }
        // a bot is only ever put down on a dry, walkable spot (never channel water, never a cell the grid closes)
        let spot = self.npc_crosshair_spot(eye, dir);
        let level = self.runner_level();
        self.npc_spawn_at(count, smart, spot, level)
    }

    /// The spot an admin spawn goes to from the crosshair ray: the crosshair floor point when it is open and near the
    /// runner, else the runner's own cell (the rule [`Game::npc_spawn`] uses). A room sends this spot to its host.
    pub fn npc_crosshair_spot(&self, eye: (f32, f32, f32), dir: (f32, f32, f32)) -> (f32, f32) {
        let runner = self.p.pos;
        let floor_y = (self.p.height + self.p.feet_y).max(0.0);
        match self.npc_crosshair_floor(eye, dir, floor_y) {
            Some(p)
                if pdist(p, runner) < SPAWN_NEAR_ADMIN
                    && self.fac.body_fits(p.0, p.1, NAV_RADIUS)
                    && self.fac.hunter_may_spawn(cell_at(p)) =>
            {
                p
            }
            _ => runner,
        }
    }

    /// Makes `count` NPCs round `spot` on `level`: the loop of [`Game::npc_spawn`], for a spot chosen elsewhere (a
    /// dedicated room's host spawns the NPCs an owner asks for at the spot the owner's crosshair named). Returns how
    /// many were made; a spot with no dry floor near it makes none.
    pub fn npc_spawn_at(&mut self, count: i32, smart: i32, spot: (f32, f32), level: i32) -> usize {
        if count <= 0 {
            return 0;
        }
        // hostile or broken input (the agent protocol) cannot ask for more than a summon may
        let count = count.min(crate::net::hosted::MAX_SUMMON_NPCS as i32);
        let mut made = 0;
        for _ in 0..count {
            let slot = self.npc_next + 1;
            // Godot: one NPC stands on the spot itself, more fan out round it
            let ring = if count == 1 { 0.0 } else { slot_radius(slot) };
            let want = slot_point(spot, slot, ring);
            let Some(at) = self
                .fac
                .nearest_dry_open(want.0, want.1, NAV_RADIUS)
                .or_else(|| self.fac.nearest_dry_open(spot.0, spot.1, NAV_RADIUS))
            else {
                // no dry spot on this floor at all: no bot is put in the water
                break;
            };
            self.push_npc(at, level, smart, None, 0.0);
            made += 1;
        }
        made
    }

    /// Makes one NPC at `at` (id, slot, name and repath phase as Godot's `spawn`) and returns its id. A `key` marks a
    /// run NPC that waits until the runner comes within `join_radius`; no key means the admin's, which follows at once.
    fn push_npc(&mut self, at: (f32, f32), level: i32, smart: i32, key: Option<String>, join_radius: f32) -> u32 {
        let id = self.npc_next;
        let slot = id + 1;
        let phase = (slot as f32 * PATH_STAGGER) % REPATH;
        let name = self.npc_name(id);
        self.npcs.push(Npc {
            id,
            name,
            slot,
            at,
            level,
            y: floor_y_of(level),
            walk: None,
            edge_clock: 0.0,
            hop: None,
            facing: 0.0,
            turn_v: 0.0,
            heading: (0.0, 0.0),
            path: vec![],
            path_age: REPATH - phase,
            walking: false,
            stride: 0.0,
            smart: smart.clamp(0, 100),
            joined: key.is_none(),
            key,
            join_radius,
            door: None,
            hidden: None,
            errand: None,
            claim: None,
            dwell: 0.0,
            visited: vec![],
            clear_time: 0.0,
            searched: vec![],
            last_door: None,
            deep_cells: Vec::new(),
            fleeing: false,
            heard: None,
            goal: GoalState::default(), flee: FleeState::default(), life: LifeState::default(),
        });
        self.npc_next += 1;
        id
    }

    /// A normal run's NPCs (data/npcs.json): on level 0 the first `count(difficulty)` homes are drawn from the level seed
    /// (`{seed}:NPC:{n}`), each on an open cell `min_distance`..`max_distance` from the runner and out of its sight. They
    /// wait there until the runner comes close, then follow. An NPC whose key already exists is not made again, so the
    /// same seed always gives the same NPCs and a call after a rebuild adds nothing. Returns how many were made.
    /// Nothing calls it on a room start or join any more (a room holds only real players); it is kept for tests and
    /// for any caller that wants the data/npcs.json run NPCs.
    pub(super) fn npc_ensure_run(&mut self, content: &crate::content::Content) -> usize {
        if self.runner_level() != 0 || self.inf.is_none() {
            return 0;
        }
        let rules = &content.npcs;
        let seed = self.level_seed();
        let mut made = 0;
        for n in 0..rules.count(self.difficulty) {
            let key = format!("{seed}:NPC:{n}");
            if self.npcs.iter().any(|x| x.key.as_deref() == Some(key.as_str())) {
                continue;
            }
            let mut rng = Rng::new(&key);
            let runner = self.p.pos;
            let cands: Vec<Cell> = self
                .fac
                .floor_cells()
                .into_iter()
                .filter(|c| {
                    let p = c.center();
                    let d = pdist(p, runner);
                    d >= rules.min_distance
                        && d <= rules.max_distance
                        && self.fac.hunter_may_spawn(*c)
                        && self.fac.body_fits(p.0, p.1, NAV_RADIUS)
                        && !self.fac.has_line_of_sight(p, runner)
                })
                .collect();
            let at = if cands.is_empty() {
                // a window with no hidden open cell in range: stand beside the runner, still deterministic
                self.fac.nearest_dry_open(runner.0 + rules.min_distance, runner.1, NAV_RADIUS).unwrap_or(runner)
            } else {
                cands[rng.next_index(cands.len())].center()
            };
            let smart = rules.smart[rng.next_index(rules.smart.len())];
            self.push_npc(at, 0, smart, Some(key), rules.join_radius);
            made += 1;
        }
        made
    }

    /// Admin REMOVE NEAREST: the NPC nearest the runner, by distance on the floor. Returns its name.
    pub fn npc_remove_nearest(&mut self) -> Option<String> {
        let runner = self.p.pos;
        let mut best: Option<(usize, f32)> = None;
        for (i, n) in self.npcs.iter().enumerate() {
            let d = pdist(n.at, runner);
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        let (i, _) = best?;
        Some(self.npcs.remove(i).name)
    }

    /// Admin REMOVE ALL. Returns how many were removed.
    pub fn npc_remove_all(&mut self) -> usize {
        let n = self.npcs.len();
        self.npcs.clear();
        n
    }

    /// Re-places every NPC beside the runner after the floor is rebuilt (a new finite floor replaces the
    /// facility, so old positions mean nothing). Ring rule as at spawn. Only a floor rebuild (`start_floor_sim`)
    /// calls it: each bot's y becomes the runner's and any edge walk is dropped.
    pub(super) fn npc_place_beside_runner(&mut self) {
        let level = self.runner_level();
        let spot = self.p.pos;
        let y = self.p.y();
        let total = self.npcs.len();
        let fac = &self.fac;
        for n in self.npcs.iter_mut() {
            let ring = if total == 1 { 0.0 } else { slot_radius(n.slot) };
            let want = slot_point(spot, n.slot, ring);
            n.at = fac
                .nearest_dry_open(want.0, want.1, NAV_RADIUS)
                .or_else(|| fac.nearest_dry_open(spot.0, spot.1, NAV_RADIUS))
                .unwrap_or(spot);
            n.level = level;
            n.y = y;
            n.forget_frame();
            n.walking = false;
        }
    }

    /// After a window recentre: every bot of the active level whose position is now on the window floor is set down on
    /// the nearest dry open cell; a bot still outside keeps its coarse position (it travels in `update_npcs_in`).
    /// Returns how many were moved.
    pub(super) fn npc_rejoin_window(&mut self) -> usize {
        let mut placed = 0;
        let fac = &self.fac;
        for n in self.npcs.iter_mut().filter(|n| n.level == fac.level) {
            if on_floor(fac, n.at) {
                if let Some(at) = fac.nearest_dry_open(n.at.0, n.at.1, NAV_RADIUS) {
                    if at != n.at {
                        n.at = at;
                        placed += 1;
                    }
                    n.y = floor_y_of(n.level) + local_ground(fac, at.0, at.1);
                }
            }
        }
        placed
    }

    /// Test tick with the embedded tuning and the test content (see `update_npcs_in`).
    #[cfg(test)]
    pub(super) fn update_npcs(&mut self, dt: f32) {
        self.update_npcs_with(dt, Tuning::get());
    }

    /// Test tick with a given tuning: the content is loaded once per test thread, never per frame.
    #[cfg(test)]
    pub(super) fn update_npcs_with(&mut self, dt: f32, t: &Tuning) {
        thread_local! {
            static TEST_CONTENT: &'static crate::content::Content = Box::leak(Box::new(crate::content::Content::load()));
        }
        let content = TEST_CONTENT.with(|c| *c);
        self.update_npcs_in(dt, t, content);
    }

    /// The per-frame tick: an NPC on a resident level tries its intents in order (data `intents`): it hides or runs
    /// from a chasing hunter, goes on a relay or locker errand, or follows the runner. A run NPC not yet joined waits
    /// until the runner is close and in sight. A bot on another resident level follows over the flights and ladders; a bot on a non-resident level is set down on the runner's level.
    /// Bots are re-placed beside the runner only by a floor rebuild (`start_floor_sim`), never by a level change.
    /// `t` is the tuning, so the tests can vary one value of the data; `content` is the frame's content (relay claims).
    pub(super) fn update_npcs_in(&mut self, dt: f32, t: &Tuning, content: &crate::content::Content) {
        // The host runs the bots and sends their places (net/npc_sync.rs `_xnpc_place`); a guest only draws the host's
        // copies, so it never moves a bot of its own list or changes its life (returning, its clocks, follow_only) here.
        if self.npcs.is_empty() || self.remote_hunters {
            return;
        }
        self.npc_sync_water(t);
        let level = self.runner_level();
        let leader = self.p.pos;
        // the hunters of every level, as each bot sees them: only those on the bot's own level count
        let mut by_level: Vec<(i32, Vec<Seen>, Vec<(f32, f32)>)> = vec![];
        for h in self.hunters.iter().chain(self.resident_hunters.iter_all()).filter(|h| h.active) {
            let i = match by_level.iter().position(|e| e.0 == h.level) {
                Some(i) => i,
                None => {
                    by_level.push((h.level, vec![], vec![]));
                    by_level.len() - 1
                }
            };
            by_level[i].1.push((h.pos, matches!(h.state, State::Chase)));
            if matches!(h.state, State::Search) {
                by_level[i].2.push(h.pos);
            }
        }
        // the facility of each resident level: the active one and the stack's (a bot elsewhere stands still)
        let empty_stack = super::level_stack::LevelStack::EMPTY;
        let stack = self.inf.as_ref().map_or(&empty_stack, |i| &i.stack);
        let active_level = self.fac.level;
        let facs: Vec<(i32, &Facility)> = std::iter::once((active_level, &self.fac))
            .chain(stack.residents.iter().map(|r| (r.level, &r.fac)))
            .collect();
        let nav: Option<&StackNav> = Some(&*self.stack_nav);
        let no_seen: &[Seen] = &[];
        let no_points: &[(f32, f32)] = &[];
        let mut presses: Vec<Cell> = vec![];
        let mut closes: Vec<Cell> = vec![];
        let loot = self.npc_loot(t);
        let spots = self.npc_spots(t, &facs);
        let search_all: Vec<(i32, (f32, f32))> = by_level.iter().flat_map(|e| e.2.iter().map(move |p| (e.0, *p))).collect();
        let chunk = crate::world::infinite::CHUNK as f32;
        let infinite = self.inf.is_some();
        // the downed bodies a bot may walk to revive: the runner, and the bots that are down
        let mut downed: Vec<(i32, (f32, f32), Option<u32>)> = vec![];
        if self.downed.is_some() {
            downed.push((level, leader, None));
        }
        for n in self.npcs.iter().filter(|n| self.downed_npcs.contains_key(&n.id)) {
            downed.push((n.level, n.at, Some(n.id)));
        }
        // the teammates a bot may look at or answer, and whether the runner is emoting
        let peers: Vec<Peer> = self.npcs.iter().map(|q| Peer { id: q.id, at: q.at, level: q.level, emoting: q.life.emote.is_some() }).collect();
        let runner_emoting = self.emote.active().is_some();
        for n in self.npcs.iter_mut() {
            // a downed bot lies still until it is revived or leaves the floor
            if self.downed_npcs.contains_key(&n.id) {
                n.walking = false;
                n.path.clear();
                n.walk = None;
                n.life.interrupt(t.life.pause.0);
                n.life.head_yaw = 0.0;
                n.life.head_pitch = 0.0;
                continue;
            }
            // a bot off the window floor keeps travelling toward the runner a chunk at a time; once its position is on
            // the window floor it is set down on the nearest dry open cell and acts as any other bot
            if infinite {
                let own_fac = facs.iter().find(|(l, _)| *l == n.level).or_else(|| facs.iter().find(|(l, _)| *l == level));
                if let Some(&(_, home)) = own_fac {
                    if !on_floor(home, n.at) {
                        coarse_travel(n, home, leader, chunk, &t.coarse, dt);
                        continue;
                    }
                }
            }
            let Some(&(_, fac)) = facs.iter().find(|(l, _)| *l == n.level) else {
                // a level that is not resident (a summon with a stale level, or a landing on an evicted one): the bot
                // drops its edge and route and is set down on the runner's level beside where it was
                n.walk = None;
                n.edge_clock = 0.0;
                n.hop = None;
                n.path.clear();
                n.path_age = REPATH;
                n.walking = false;
                if let Some(&(_, home)) = facs.iter().find(|(l, _)| *l == level) {
                    if let Some(at) = home.nearest_dry_open(n.at.0, n.at.1, NAV_RADIUS) {
                        n.at = at;
                    }
                    set_level(n, level);
                    n.y = floor_y_of(level) + local_ground(home, n.at.0, n.at.1);
                }
                continue;
            };
            let (seen, searching) = by_level
                .iter()
                .find(|e| e.0 == n.level)
                .map_or((no_seen, no_points), |e| (&e.1[..], &e.2[..]));
            let own = n.level == active_level;
            let sc = Scene {
                fac,
                disabled: if own { &self.hiding_disabled[..] } else { &[] },
                leader,
                seen,
                searching,
                runner_level: level,
                nav,
                loot: if own { &loot[..] } else { &[] },
                spots: &spots,
                search_all: &search_all,
                downed: &downed,
            };
            // a bot on a flight or ladder finishes the edge before it thinks about anything else
            if n.walk.is_some() {
                // the head eases back to the body on the edge (liven does not run here), so it never freezes mid-glance
                let k = 1.0 - (-t.lively.rate * dt).exp();
                n.life.head_yaw -= n.life.head_yaw * k;
                n.life.head_pitch -= n.life.head_pitch * k;
                n.life.gaze = None;
                n.life.gaze_left = 0.0;
                advance_edge(n, dt);
                continue;
            }
            n.y = floor_y_of(n.level) + local_ground(fac, n.at.0, n.at.1);
            // doors are the active facility's: another level's bot never queues a press or a close on it
            let mut sink: Vec<Cell> = vec![];
            let door_presses = if own { &mut presses } else { &mut sink };
            note_searched(n, searching, t);
            n.flee.alert = if sees(n, fac, seen, &t.flee) || hears_hunter(n, seen, &t.flee) { t.flee.alert } else { (n.flee.alert - dt).max(0.0) };
            if n.hidden.is_some() {
                n.life.interrupt(t.life.pause.0);
                n.life.head_yaw = 0.0;
                n.life.head_pitch = 0.0;
                hidden_tick(n, &sc, t, dt);
                continue;
            }
            if !n.joined && n.level == level && pdist(n.at, leader) <= n.join_radius && sc.fac.has_line_of_sight(n.at, leader) {
                n.joined = true;
            }
            let mut ran = false;
            let mut life_ran = false;
            for intent in &t.intents {
                ran = run_intent(n, &sc, intent, t, dt, door_presses);
                if ran {
                    life_ran = intent == "life";
                    // any intent but an errand ends the errand: the bot hid, ran or followed instead
                    if !matches!(intent.as_str(), "relay" | "locker") {
                        n.errand = None;
                        n.dwell = 0.0;
                    }
                    // any intent but life ends what the bot was doing in its spare time
                    if intent != "life" {
                        n.life.interrupt(t.life.pause.0);
                    }
                    break;
                }
            }
            if !ran {
                n.walking = false;
                n.path.clear();
            }
            // a bot that heard a noise event turns toward the sound while it stands (hunters/noise_events.rs)
            if let Some((p, left)) = n.heard {
                if left <= dt {
                    n.heard = None;
                } else {
                    n.heard = Some((p, left - dt));
                    if !n.walking {
                        turn(n, (-(p.0 - n.at.0)).atan2(-(p.1 - n.at.1)), dt);
                    }
                }
            }
            let around = Around { fac: sc.fac, leader, runner_level: level, runner_emoting, peers: &peers, seen };
            liven(n, &around, life_ran, t, dt);
            // a bot clear of the door it last passed shuts it (the runner's press refuses a shut leaf on a body)
            if let Some(d) = n.last_door {
                let clear = sc.fac.door_centre(d).map_or(true, |c| pdist(n.at, c) >= t.close_clear);
                if clear {
                    n.last_door = None;
                    if own && sc.fac.door_is_open(d) {
                        closes.push(d);
                    }
                }
            }
        }
        self.npc_press_doors(presses);
        self.npc_close_doors(closes);
        self.npc_claims(content);
        self.npc_catches();
    }

    /// How many more bots an owner summon may add before the room cap (data `summon.room_cap`).
    pub fn npc_room_space(&self) -> usize {
        Tuning::get().summon_room_cap.saturating_sub(self.npcs.len())
    }

    /// The places the bots may make their goal (data `goals`): every relay of every resident level (the active level's
    /// not yet recovered), the active level's batteries and floor items the team has room for (a battery while the
    /// lamp is low, the heal item while stamina is low, any other useful item as a pickup) and its exit once its relays
    /// are all recovered.
    fn npc_spots(&self, t: &Tuning, facs: &[(i32, &Facility)]) -> Vec<Spot> {
        let g = &t.goals;
        let active = self.fac.level;
        let open = |fac: &Facility, p: (f32, f32)| fac.nearest_open(p.0, p.1, NAV_RADIUS).map_or(cell_at(p), cell_at);
        let mut v = vec![];
        for &(level, fac) in facs {
            for (i, c) in fac.relays.iter().enumerate() {
                if level == active && self.relays_taken.get(i).copied().unwrap_or(true) {
                    continue;
                }
                let pos = c.center();
                let claim = (level == active).then_some(Claim::Relay(i));
                v.push(Spot { kind: SpotKind::Relay, level, cell: open(fac, pos), pos, claim });
            }
        }
        let lamp_low = self.p.lamp < g.low_lamp * crate::config::MAX_LAMP_CHARGE;
        let stamina_low = self.p.stamina < g.low_stamina * crate::config::MAX_STAMINA;
        let useful = |item: Item| {
            t.pickup_useful.iter().any(|w| w == crate::net::held_wire::wire(item)) && self.can_grant(item)
        };
        let kind_of = |item: Item| {
            let wire = crate::net::held_wire::wire(item);
            if wire == g.heal_item && stamina_low {
                SpotKind::Heal
            } else if item == Item::Battery && lamp_low {
                SpotKind::Battery
            } else {
                SpotKind::Pickup
            }
        };
        if useful(Item::Battery) {
            for (i, c) in self.fac.batteries.iter().enumerate() {
                if !self.batteries_taken.get(i).copied().unwrap_or(true) {
                    let pos = c.center();
                    let kind = kind_of(Item::Battery);
                    v.push(Spot { kind, level: active, cell: open(&self.fac, pos), pos, claim: Some(Claim::Battery(i)) });
                }
            }
        }
        for (i, p) in self.pickups.iter().enumerate() {
            if !p.taken && useful(p.item) {
                v.push(Spot { kind: kind_of(p.item), level: active, cell: open(&self.fac, p.pos), pos: p.pos, claim: Some(Claim::Pickup(i)) });
            }
        }
        if self.fac.has_lift && self.relays_collected >= self.fac.relays.len() as i32 {
            let pos = self.fac.exit.center();
            v.push(Spot { kind: SpotKind::Exit, level: active, cell: open(&self.fac, pos), pos, claim: None });
        }
        v
    }

    /// The untaken batteries and floor items of the active level that the bots take (data `pickup.useful`) and that the
    /// team has room for.
    fn npc_loot(&self, t: &Tuning) -> Vec<(Claim, (f32, f32))> {
        let useful = |item: Item| {
            t.pickup_useful.iter().any(|w| w == crate::net::held_wire::wire(item)) && self.can_grant(item)
        };
        let mut v = vec![];
        if useful(Item::Battery) {
            for (i, c) in self.fac.batteries.iter().enumerate() {
                if !self.batteries_taken.get(i).copied().unwrap_or(true) {
                    v.push((Claim::Battery(i), c.center()));
                }
            }
        }
        for (i, p) in self.pickups.iter().enumerate() {
            if !p.taken && useful(p.item) {
                v.push((Claim::Pickup(i), p.pos));
            }
        }
        v
    }

    /// Applies what the bots reached this frame, through the same functions as the player's interact (`claim_relay`,
    /// `take_battery`, `take_pickup`). A claim on a level the bot has left is dropped.
    fn npc_claims(&mut self, content: &crate::content::Content) {
        let active = self.fac.level;
        let claims: Vec<(u32, Claim)> = self
            .npcs
            .iter_mut()
            .filter_map(|n| n.claim.take().filter(|_| n.level == active).map(|c| (n.id, c)))
            .collect();
        if claims.is_empty() {
            return;
        }
        for (id, claim) in claims {
            let actor = super::interact::Actor::Npc(id);
            match claim {
                Claim::Relay(i) => {
                    self.claim_relay(i, actor, content);
                }
                Claim::Battery(i) => {
                    self.take_battery(i, actor);
                }
                Claim::Pickup(i) => {
                    self.take_pickup(i, actor);
                }
            }
        }
    }

    /// The bots on the events' level, and one storey away at the data `hear_adjacent_scale`, hear this frame's noise
    /// events (hunters/noise_events.rs `heard`, at the data `bot_hearing_scale`): the event heard by the widest margin
    /// is remembered for `bot_look_seconds`, and the bot turns toward it while it stands. A bot never changes any hunter, meter or cue because of a sound.
    pub(super) fn npc_hear(&mut self, events: &[crate::hunters::noise_events::NoiseEvent]) {
        if events.is_empty() || self.npcs.is_empty() {
            return;
        }
        bots_hear(&mut self.npcs, events, crate::hunters::noise_events::rules());
    }

    /// Fills each bot's `deep_cells` from the hunters' water field (the same field the hunter brains read): the cells
    /// round the bot with at least `water_avoid_depth` of water. Left as it was with no field yet.
    pub(super) fn npc_sync_water(&mut self, t: &Tuning) {
        let Some(hw) = self.hunter_water.as_ref() else { return };
        let (field, water_level) = (hw.field.clone(), hw.key.level);
        // the field belongs to one level: a bot on another storey reads it as dry
        for n in self.npcs.iter_mut() {
            n.deep_cells = if n.level == water_level {
                deep_cells_near(n.at, t.water_avoid_depth, |c| field.depth_at((c.x, c.y)))
            } else {
                vec![]
            };
        }
    }

    /// The bots' interact presses, through the runner's path (`press_door`). A door already open (another bot or the
    /// runner got there first) is left alone, since a press would shut it.
    fn npc_press_doors(&mut self, presses: Vec<Cell>) {
        for cell in presses {
            if !self.fac.door_is_open(cell) {
                self.press_door(cell);
            }
        }
    }

    /// The bots shutting doors behind them: a press on an open door, through the same path, which a body in the
    /// leaf's sweep refuses.
    fn npc_close_doors(&mut self, closes: Vec<Cell>) {
        for cell in closes {
            if self.fac.door_is_open(cell) {
                self.press_door(cell);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;
    use crate::world::facility::DoorToggle;

    /// A finite route (no server, no renderer, no assets): the offline game the admin plays.
    fn offline_game() -> (Game, Content, Catalog) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        (g, content, catalog)
    }

    /// A ray from above the runner straight down (the crosshair at the runner's own feet).
    fn down_at(p: (f32, f32)) -> ((f32, f32, f32), (f32, f32, f32)) {
        ((p.0, 1.6, p.1), (0.0, -1.0, 0.0))
    }

    /// A ray pointing at the sky: the crosshair finds no floor.
    fn sky(p: (f32, f32)) -> ((f32, f32, f32), (f32, f32, f32)) {
        ((p.0, 1.6, p.1), (0.0, 1.0, 0.0))
    }

    /// The embedded data/npcs.json with one edit made to its JSON, parsed as the bots read it.
    fn tuning_with(edit: impl Fn(&mut Value)) -> Tuning {
        let (_, text) = crate::content::embedded::data_under("npcs.json")
            .next()
            .expect("data/npcs.json is embedded");
        let mut v: Value = serde_json::from_str(text).expect("data/npcs.json parses");
        edit(&mut v);
        Tuning::parse(&v).expect("data/npcs.json still has every key")
    }

    /// A JSON array of strings.
    fn names(list: &[&str]) -> Value {
        Value::Array(list.iter().map(|s| Value::from(*s)).collect())
    }

    /// The embedded tuning with only these intents, in this order.
    fn only(intents: &[&str]) -> Tuning {
        tuning_with(|v| v["intents"] = names(intents))
    }

    /// The embedded tuning for a relay errand that is not cut short by distance: a long leash, a short dwell.
    fn relay_trip() -> Tuning {
        tuning_with(|v| {
            v["intents"] = names(&["relay"]);
            v["errand"]["relay"]["leash"] = Value::from(1000.0);
            v["errand"]["relay"]["dwell"] = Value::from(0.5);
        })
    }

    /// The cells reachable from `s` with their step counts, sorted by cell. `distance_map` is a HashMap, whose order
    /// changes every process, so a pick from it must go through this instead.
    fn sorted_reach(fac: &Facility, s: Cell) -> Vec<(Cell, i32)> {
        let mut reach: Vec<(Cell, i32)> = fac.distance_map(s).into_iter().collect();
        reach.sort_by_key(|(c, _)| (c.x, c.y));
        reach
    }

    #[test]
    fn spawn_count_and_smartness_are_kept() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        assert_eq!(g.npc_spawn(3, 90, e, d), 3);
        assert_eq!(g.npcs.len(), 3);
        assert!(g.npcs.iter().all(|n| n.smart == 90));
        assert_eq!(g.npc_spawn(2, 30, e, d), 2);
        assert_eq!(g.npcs.len(), 5);
        assert_eq!(g.npcs[4].smart, 30);
        // ids and slots never repeat, as Godot's _next
        let ids: Vec<u32> = g.npcs.iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![0, 1, 2, 3, 4]);
        assert!(g.npcs.iter().all(|n| n.slot == n.id + 1));
        // a smartness out of range is clamped, as Godot clampi(smart, 0, 100)
        g.npc_spawn(1, 250, e, d);
        assert_eq!(g.npcs.last().unwrap().smart, 100);
        // zero or less makes nothing
        assert_eq!(g.npc_spawn(0, 70, e, d), 0);
        assert_eq!(g.npc_spawn(-2, 70, e, d), 0);
        assert_eq!(g.npcs.len(), 6);
    }

    #[test]
    fn names_follow_the_guest_name_shape() {
        for seed in [1u64, 2, 99, 4_000_000_000] {
            let name = guest_name_from_seed(seed);
            let b = name.as_bytes();
            assert_eq!(b.len(), 7, "{name}");
            assert!(b"ABCDEFGHJKLMNPQRSTUVWXYZ".contains(&b[0]), "{name}");
            assert_eq!(b[1], b'-');
            for &c in &b[2..] {
                assert!(c.is_ascii_digit(), "{name}");
            }
            assert_eq!(guest_name_from_seed(seed), name, "same seed, same name");
        }
    }

    #[test]
    fn spawned_names_use_the_room_seed_plus_id() {
        let (mut g, _, _) = offline_game();
        let (e, d) = sky(g.p.pos);
        g.npc_spawn(2, 70, e, d);
        let base = run_seed_of_text(&g.room_text) as u64;
        assert_eq!(g.npcs[0].name, guest_name_from_seed(base));
        assert_eq!(g.npcs[1].name, guest_name_from_seed(base + 1));
    }

    #[test]
    fn slot_radius_is_godot_s() {
        assert!((slot_radius(1) - 2.4).abs() < 1e-6);
        assert!((slot_radius(8) - 2.4).abs() < 1e-6);
        assert!(slot_radius(9) > 2.4);
        assert!((slot_radius(32) - 4.8).abs() < 1e-4, "sqrt(32/8) = 2");
    }

    #[test]
    fn remove_nearest_then_all() {
        let (mut g, _, _) = offline_game();
        assert!(g.npc_remove_nearest().is_none(), "no NPCs, nothing to remove");
        assert_eq!(g.npc_remove_all(), 0);
        let (e, d) = sky(g.p.pos);
        g.npc_spawn(2, 70, e, d);
        // move one NPC far away: the other is the nearest and goes first
        let far = (g.p.pos.0 + 20.0, g.p.pos.1);
        g.npcs[0].at = far;
        let near_name = g.npcs[1].name.clone();
        assert_eq!(g.npc_remove_nearest(), Some(near_name));
        assert_eq!(g.npcs.len(), 1);
        assert_eq!(g.npcs[0].id, 0, "the far NPC is kept");
        assert_eq!(g.npc_remove_all(), 1);
        assert!(g.npcs.is_empty());
        // spawning again after a clear carries on the id counter (ids never reused)
        g.npc_spawn(1, 70, e, d);
        assert_eq!(g.npcs[0].id, 2);
    }

    #[test]
    fn crosshair_floor_is_used_when_open() {
        let (mut g, _, _) = offline_game();
        // an open floor cell a few metres off, looked at from above
        let runner = g.p.pos;
        let spot = (-12..=12)
            .flat_map(|dx| (-12..=12).map(move |dy| (dx, dy)))
            .map(|(dx, dy)| (runner.0 + dx as f32 * 0.5, runner.1 + dy as f32 * 0.5))
            .find(|&p| pdist(p, runner) > 2.0 && g.fac.body_fits(p.0, p.1, NAV_RADIUS));
        let Some(spot) = spot else {
            panic!("no open floor 2 m or more from the runner in the test floor");
        };
        let (e, d) = down_at(spot);
        g.npc_spawn(1, 70, e, d);
        let at = g.npcs[0].at;
        // the NPC stands on its cell's centre (nearest_open), so it is within half a cell diagonal of the point
        assert!(pdist(at, spot) <= 0.71, "spawned at the crosshair point, got {at:?} for {spot:?}");
    }

    #[test]
    fn crosshair_into_the_sky_spawns_beside_the_runner() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        g.npc_spawn(1, 70, e, d);
        let at = g.npcs[0].at;
        assert!(pdist(at, runner) < 1.0, "beside the runner, got {at:?} for {runner:?}");
    }

    #[test]
    fn an_npc_follows_the_runner_and_then_stands_still() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        g.npc_spawn(1, 70, e, d);
        // only the follow intent here: the errands would send the bot off to a relay
        let follow_only = only(&["follow"]);
        // put the NPC on an open cell reachable from the runner, about 9 m away
        let fac = &g.fac;
        let start = Cell::new(runner.0.floor() as i32, runner.1.floor() as i32);
        let far = sorted_reach(fac, start)
            .into_iter()
            .filter(|(c, dist)| *dist >= 8 && *dist <= 12 && fac.body_fits(c.center().0, c.center().1, NAV_RADIUS))
            .map(|(c, _)| c.center())
            .next();
        let Some(far) = far else {
            panic!("no reachable open cell 8 to 12 cells away in the test floor");
        };
        g.npcs[0].at = far;
        let before = pdist(far, runner);
        assert!(before > Tuning::get().follow_radius, "starts out of follow range");
        // 40 simulated seconds at 60 Hz: long enough to walk ~9 m at 4 m/s, with room to spare
        for _ in 0..(40 * 60) {
            g.update_npcs_with(1.0 / 60.0, &follow_only);
        }
        let after = pdist(g.npcs[0].at, g.p.pos);
        assert!(after < before, "the NPC closed in: {before} -> {after}");
        assert!(
            after <= Tuning::get().follow_radius + 0.5,
            "the NPC is within follow range of the runner: {after}"
        );
        // once in range and not walking, it stands still
        let settled = g.npcs[0].at;
        for _ in 0..120 {
            g.update_npcs_with(1.0 / 60.0, &follow_only);
        }
        assert!(!g.npcs[0].walking, "idle once settled in range");
        assert_eq!(g.npcs[0].at, settled, "no drift while idle");
    }

    #[test]
    fn an_npc_on_a_level_that_is_not_held_is_set_down_on_the_runners_level() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        g.npc_spawn(1, 70, e, d);
        g.npcs[0].level = 1; // the runner is on level 0
        g.npcs[0].at = (runner.0 + 9.0, runner.1);
        for _ in 0..120 {
            g.update_npcs(1.0 / 60.0);
        }
        // an offline floor holds no level 1: the bot is not left waiting there, it rejoins the runner's level
        assert_eq!(g.npcs[0].level, 0, "the bot is set down on the runner's level");
        assert!(g.npcs[0].walk.is_none());
    }

    /// A spawn never lands on channel water: with every floor cell near the runner made water, every bot is put on a dry,
    /// routable cell further out, and a floor with no dry cell at all makes no bot rather than one in the water.
    #[test]
    fn a_spawned_bot_never_lands_in_water() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let wet: Vec<Cell> = g.fac.floor_cells().into_iter().filter(|c| pdist(c.center(), runner) <= 12.0).collect();
        assert!(!wet.is_empty(), "the runner has floor round it");
        g.fac.set_water_cells(&wet);
        g.fac.rebuild_grids();
        let (e, d) = down_at(runner);
        let made = g.npc_spawn(4, 50, e, d);
        assert_eq!(made, 4, "four dry spots are found further out");
        for n in &g.npcs {
            let c = cell_at(n.at);
            assert!(!g.fac.is_water(c), "a bot at {:?} is in the water", n.at);
            assert!(g.fac.hunter_open(c), "a bot at {:?} stands on a cell the grid opens", n.at);
        }
        // no dry spot anywhere: nothing is spawned into the water
        let all: Vec<Cell> = g.fac.floor_cells();
        g.fac.set_water_cells(&all);
        g.fac.rebuild_grids();
        let before = g.npcs.len();
        assert_eq!(g.npc_spawn(2, 50, e, d), 0);
        assert_eq!(g.npcs.len(), before);
    }

    #[test]
    fn spawn_needs_no_server() {
        // the whole path runs on a headless game: no network, no assets, no window
        let (mut g, _, _) = offline_game();
        let (e, d) = sky(g.p.pos);
        assert_eq!(g.npc_spawn(4, 50, e, d), 4);
        for _ in 0..60 {
            g.update_npcs(1.0 / 60.0);
        }
        assert_eq!(g.npc_remove_all(), 4);
    }

    /// A normal Infinite run (the pattern of the endless tests): level 0, seed text fixed. A run no longer makes NPCs
    /// by itself, so the data/npcs.json run NPCs are asked for here, for the tests of that system.
    pub(super) fn run_game(diff: Difficulty, run_seed: u32, text: &str) -> (Game, Content) {
        crate::world::route::layout::godot_regions_for_this_test();
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_infinite_sim(diff, run_seed, text, 0, &content, &catalog);
        g.npc_ensure_run(&content);
        (g, content)
    }

    #[test]
    fn starting_or_joining_a_room_spawns_no_npc() {
        let content = Content::load();
        let catalog = Catalog::load();
        for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
            let g = Game::new_infinite_sim(diff, 7, "ROOM-TEST-7", 0, &content, &catalog);
            assert!(g.npcs.is_empty(), "{diff:?}: a new room started with {} NPC(s)", g.npcs.len());
        }
        // entering the window again (a continued checkpoint, a rejoin) adds none either
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-TEST-7", 0, &content, &catalog);
        g.enter_infinite(&content, &catalog);
        assert!(g.npcs.is_empty(), "re-entering the room made an NPC");
        // the NPC system itself still works when asked for: the admin spawn and the run rule
        let (e, d) = sky(g.p.pos);
        assert_eq!(g.npc_spawn(1, 50, e, d), 1);
        assert_eq!(g.npcs.len(), 1);
    }

    #[test]
    fn a_normal_run_starts_with_the_ruled_count_on_open_cells() {
        for (diff, want) in [(Difficulty::Relaxed, 1), (Difficulty::Standard, 1), (Difficulty::Hard, 2)] {
            let (g, content) = run_game(diff, 7, "ROOM-TEST-7");
            assert_eq!(content.npcs.count(diff), want);
            assert_eq!(g.npcs.len() as i32, want, "{diff:?}");
            for n in &g.npcs {
                assert!(g.fac.body_fits(n.at.0, n.at.1, NAV_RADIUS), "NPC {} stands in a wall at {:?}", n.name, n.at);
                let d = pdist(n.at, g.p.pos);
                assert!(d >= content.npcs.min_distance - 0.01, "NPC inside the start area: {d} m");
                assert!(!n.joined && n.key.is_some(), "a run NPC waits for the runner");
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_npcs() {
        let (a, _) = run_game(Difficulty::Hard, 7, "ROOM-TEST-7");
        let (b, _) = run_game(Difficulty::Hard, 7, "ROOM-TEST-7");
        let key = |g: &Game| g.npcs.iter().map(|n| (n.name.clone(), n.at, n.smart, n.key.clone())).collect::<Vec<_>>();
        assert_eq!(key(&a), key(&b));
        let (c, _) = run_game(Difficulty::Hard, 8, "ROOM-TEST-8");
        assert_ne!(key(&a), key(&c), "another seed, other NPCs");
    }

    #[test]
    fn ensure_run_never_makes_an_npc_twice() {
        let (mut g, content) = run_game(Difficulty::Hard, 7, "ROOM-TEST-7");
        let before = g.npcs.len();
        assert_eq!(g.npc_ensure_run(&content), 0);
        assert_eq!(g.npcs.len(), before);
        // a removed run NPC comes back on the next call
        g.npc_remove_all();
        assert_eq!(g.npc_ensure_run(&content), before);
    }

    #[test]
    fn a_waiting_npc_stays_until_the_runner_is_close_and_then_follows() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let home = g.npcs[0].at;
        for _ in 0..120 {
            g.update_npcs(1.0 / 60.0);
        }
        assert_eq!(g.npcs[0].at, home, "it waits");
        assert!(!g.npcs[0].joined);
        // the runner steps beside it
        g.p.pos = (home.0 + 1.0, home.1);
        g.update_npcs(1.0 / 60.0);
        assert!(g.npcs[0].joined, "close and in sight: joined");
    }

    #[test]
    fn an_npc_runs_from_a_chasing_hunter() {
        let (mut g, content) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let i = 0;
        g.npcs[i].joined = true;
        let start = g.npcs[i].at;
        let mut h = crate::hunters::brain::Brain::new(
            content.hunters[0].clone(),
            (start.0 + 3.0, start.1),
            "T",
            Difficulty::Standard,
        );
        h.active = true;
        h.state = State::Chase;
        g.hunters.push(h);
        let before = pdist(g.npcs[i].at, g.hunters.last().unwrap().pos);
        // only the flee intent: hiding is tested on its own
        let flee_only = only(&["flee"]);
        for _ in 0..60 {
            g.update_npcs_with(1.0 / 60.0, &flee_only);
        }
        assert!(pdist(g.npcs[i].at, g.hunters.last().unwrap().pos) > before, "it ran away");
    }

    /// The pursuit inputs `audio::world::update` derives from the hunter list (world.rs 86-96): the nearest active
    /// hunter's distance to the listener, and whether an active chaser pursues the listener.
    pub(super) fn pursuit_inputs(g: &Game) -> (Option<f64>, bool) {
        let mut nearest: Option<f64> = None;
        let mut pursued = false;
        for h in g.hunters.iter().filter(|h| h.active) {
            let d = pdist(h.pos, g.p.pos);
            nearest = Some(nearest.map_or(d as f64, |n| n.min(d as f64)));
            if h.state == State::Chase && h.stun <= 0.0 && (h.direct_visible || d <= 18.0) {
                pursued = true;
            }
        }
        (nearest, pursued)
    }

    /// A bot must not feed the danger readout, the vignette or the heartbeat/chase cues: a hunter chasing a bot (the
    /// runner far off and out of sight) gives the same danger and pursuit inputs with or without that bot.
    #[test]
    fn a_bot_being_chased_does_not_change_the_danger_inputs() {
        // (danger, nearest hunter distance, pursued, heartbeat danger, bot moved, hunter position)
        let flee_only = only(&["flee"]);
        let play = |with_bot: bool| {
            let (mut g, content) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
            let bot_before = g.npcs[0].at;
            if with_bot {
                g.npcs[0].joined = true;
            }
            // the runner is 30 cells off and the hunter cannot see them
            g.p.pos = (bot_before.0 - 30.0, bot_before.1);
            let hunter_at = (bot_before.0 + 3.0, bot_before.1);
            let mut h = crate::hunters::brain::Brain::new(
                content.hunters[0].clone(),
                hunter_at,
                "T",
                Difficulty::Standard,
            );
            h.active = true;
            h.state = State::Chase;
            h.direct_visible = false;
            g.hunters.push(h);
            for _ in 0..60 {
                g.update_npcs_with(1.0 / 60.0, &flee_only);
            }
            let bot_moved = pdist(g.npcs[0].at, bot_before) > 0.5;
            let (nearest, pursued) = pursuit_inputs(&g);
            let heart = crate::audio::cues::rules::heartbeat_danger(nearest, pursued, g.in_safe_zone());
            let hunter_pos = g.hunters.last().unwrap().pos;
            (g.danger(), nearest, pursued, heart, bot_moved, hunter_pos)
        };
        let without = play(false);
        let with = play(true);
        assert!(with.4, "the bot really ran, so the comparison is not vacuous");
        assert_eq!(with.0, without.0, "danger() with a bot equals danger() without one");
        assert_eq!(with.1, without.1, "nearest hunter distance is unchanged");
        assert_eq!(with.2, without.2, "pursued flag is unchanged");
        assert_eq!(with.3, without.3, "heartbeat danger is unchanged");
        assert_eq!(with.5, without.5, "the hunter did not move because of the bot");
    }

    /// A game with a room door a bot can cross in a straight line: the door's threshold cell and the open points a
    /// metre either side of it. Doors are registered (ticked) and shut.
    fn door_case() -> (Game, Cell, (f32, f32), (f32, f32)) {
        for seed in 1..60u32 {
            let (mut g, _) = run_game(Difficulty::Standard, seed, &format!("ROOM-DOOR-{seed}"));
            g.fac.update_doors(&[], &[], 0.0);
            let doors: Vec<(usize, (f32, f32))> = g.fac.doors().collect();
            for (i, c) in doors {
                let cell = g.fac.props[i].cell;
                if g.fac.door_is_open(cell) {
                    continue;
                }
                for axis in [(1.0f32, 0.0f32), (0.0, 1.0)] {
                    let a = (c.0 - axis.0, c.1 - axis.1);
                    let b = (c.0 + axis.0, c.1 + axis.1);
                    if !(g.fac.body_fits(a.0, a.1, NAV_RADIUS) && g.fac.body_fits(b.0, b.1, NAV_RADIUS)) {
                        continue;
                    }
                    let cell_of = |p: (f32, f32)| Cell::new(p.0.floor() as i32, p.1.floor() as i32);
                    // a doorway is a straight three-cell line a, door, b; a detour round the walls is longer
                    if g.fac.find_path(cell_of(a), cell_of(b)).len() == 3 {
                        return (g, cell, a, b);
                    }
                }
            }
        }
        panic!("no crossable room door in 59 seeds");
    }

    /// One frame of a bot heading for `goal`: its step, its interact press and the door swing, as `Game::update` runs them.
    fn bot_frame(g: &mut Game, goal: (f32, f32), dt: f32) {
        let mut presses = vec![];
        walk_toward(&mut g.npcs[0], &g.fac, goal, false, Tuning::get(), dt, &mut presses);
        g.npc_press_doors(presses);
        let bodies: Vec<(f32, f32)> = std::iter::once(g.p.pos).chain(g.npcs.iter().map(|n| n.at)).collect();
        g.fac.update_doors(&[], &bodies, dt);
    }

    #[test]
    fn a_bot_opens_a_shut_door_by_interact_and_then_passes() {
        let (mut g, cell, a, b) = door_case();
        g.p.pos = (a.0 + 40.0, a.1 + 40.0); // the runner is elsewhere
        g.npcs[0].joined = true;
        g.npcs[0].at = a;
        let dt = 1.0 / 60.0;
        let (mut stopped, mut pressed) = (None, None);
        for f in 0..(8 * 60) {
            bot_frame(&mut g, b, dt);
            let n = &g.npcs[0];
            if stopped.is_none() && n.door.is_some() {
                stopped = Some(f);
                assert!(!g.fac.door_is_open(cell), "it stops at a shut door");
                assert!(!n.walking);
            }
            if pressed.is_none() && g.fac.door_is_open(cell) {
                pressed = Some(f);
            }
        }
        let (stopped, pressed) = (stopped.expect("the bot reached the door"), pressed.expect("the bot opened the door"));
        let waited = (pressed - stopped) as f32 * dt;
        let t = Tuning::get();
        assert!(waited >= t.door_think - dt && waited <= t.door_think + t.door_think_jitter + 2.0 * dt, "think delay {waited}");
        assert!(g.fac.door_is_open(cell) && g.fac.door_in_way(b.0, b.1, NAV_RADIUS).is_none());
        assert!(pdist(g.npcs[0].at, b) < 0.6, "it walked on through the door, got {:?} for {b:?}", g.npcs[0].at);
    }

    #[test]
    fn a_bot_does_not_open_a_door_by_proximity() {
        let (mut g, cell, a, _) = door_case();
        g.npcs[0].joined = false; // it waits right at the door
        g.npcs[0].at = a;
        g.p.pos = (a.0 + 40.0, a.1 + 40.0);
        for _ in 0..180 {
            g.update_npcs(1.0 / 60.0);
            // the openers are the hunters alone (game/mod.rs): none here
            let bodies = [g.p.pos, g.npcs[0].at];
            g.fac.update_doors(&[], &bodies, 1.0 / 60.0);
        }
        assert!(!g.fac.door_is_open(cell));
        assert_eq!(g.fac.door_open_amount(cell), 0.0, "a bot beside a shut door leaves it shut");
    }

    #[test]
    fn a_bot_press_refuses_to_close_on_someone_in_the_doorway() {
        let (mut g, cell, a, _) = door_case();
        g.p.pos = (a.0 + 40.0, a.1 + 40.0);
        g.npcs[0].at = a;
        assert_eq!(g.press_door(cell), Some(DoorToggle::Opened));
        g.fac.update_doors(&[], &[], 1.0);
        // a bot stands in the doorway: the same press as the runner's is refused
        g.npcs[0].at = g.fac.door_centre(cell).unwrap();
        assert_eq!(g.press_door(cell), Some(DoorToggle::Blocked));
        assert!(g.fac.door_is_open(cell));
        // and it treats the bot as a body: the leaf does not swing shut through it
        g.fac.update_doors(&[], &[g.npcs[0].at], 1.0);
        assert_eq!(g.fac.door_open_amount(cell), 1.0);
    }

    /// A floor point with a wall between it and a floor point 4 to 12 m off that is at least six cells away by route
    /// (no line of sight, so the straight way is blocked).
    fn walled_pair(g: &Game) -> Option<((f32, f32), (f32, f32))> {
        let fac = &g.fac;
        for s in fac.floor_cells() {
            let a = s.center();
            if !fac.body_fits(a.0, a.1, NAV_RADIUS) {
                continue;
            }
            for (c, steps) in sorted_reach(fac, s) {
                let b = c.center();
                if steps >= 6
                    && (4.0..=12.0).contains(&pdist(a, b))
                    && fac.body_fits(b.0, b.1, NAV_RADIUS)
                    && !fac.has_line_of_sight(a, b)
                {
                    return Some((a, b));
                }
            }
        }
        None
    }

    #[test]
    fn an_npc_walks_round_a_wall_to_its_goal() {
        let (mut g, _, _) = offline_game();
        let Some((a, b)) = walled_pair(&g) else {
            panic!("no point behind a wall in the test floor");
        };
        let (e, d) = sky(g.p.pos);
        g.npc_spawn(1, 70, e, d);
        g.npcs[0].joined = true;
        g.npcs[0].at = a;
        // 60 simulated seconds: far more than the walk round the wall needs
        for f in 0..(60 * 60) {
            bot_frame(&mut g, b, 1.0 / 60.0);
            let at = g.npcs[0].at;
            assert!(g.fac.body_fits(at.0, at.1, NAV_RADIUS), "frame {f}: the bot is in a wall at {at:?}");
            if pdist(at, b) < 0.3 {
                return;
            }
        }
        panic!("the bot did not reach {b:?}: it stopped at {:?}", g.npcs[0].at);
    }

    #[test]
    fn a_blocked_step_replans_instead_of_sliding() {
        let (mut g, _, _) = offline_game();
        let (open, wall, goal) = {
            let fac = &g.fac;
            let (open, wall) = fac
                .floor_cells()
                .into_iter()
                .filter(|c| {
                    let p = c.center();
                    fac.body_fits(p.0, p.1, NAV_RADIUS)
                })
                .find_map(|c| {
                    [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .into_iter()
                        .map(|(dx, dy)| Cell::new(c.x + dx, c.y + dy))
                        .find(|w| !fac.is_floor(w.x as f64 + 0.5, w.y as f64 + 0.5))
                        .map(|w| (c, w))
                })
                .expect("an open cell beside a wall");
            let goal = sorted_reach(fac, open)
                .into_iter()
                .filter(|&(c, steps)| steps >= 4 && fac.body_fits(c.center().0, c.center().1, NAV_RADIUS))
                .map(|(c, _)| c.center())
                .next()
                .expect("a reachable goal");
            (open, wall, goal)
        };
        let (e, d) = sky(g.p.pos);
        g.npc_spawn(1, 70, e, d);
        // a quarter of a cell from the open cell's centre toward the wall: the next step runs into it
        let (cx, cy) = open.center();
        let (dx, dy) = ((wall.x - open.x) as f32, (wall.y - open.y) as f32);
        let at = (cx + dx * 0.25, cy + dy * 0.25);
        let n = &mut g.npcs[0];
        n.joined = true;
        n.at = at;
        n.path = vec![(wall.x, wall.y)];
        n.path_age = 0.0;
        let mut presses = vec![];
        walk_toward(&mut g.npcs[0], &g.fac, goal, false, Tuning::get(), 1.0 / 60.0, &mut presses);
        let n = &g.npcs[0];
        assert_eq!(n.at, at, "the bot stops at the wall rather than sliding along it");
        assert!(!n.walking);
        assert_eq!(n.path_age, 0.0, "the blocked step replanned the route");
        assert!(!n.path.contains(&(wall.x, wall.y)), "the new route avoids the wall");
        assert!(!n.path.is_empty(), "and still leads to the goal");
    }

    #[test]
    fn shift_drops_the_path_and_forces_a_replan() {
        let mut n = Npc {
            id: 0,
            name: guest_name_from_seed(1),
            slot: 1,
            at: (3.0, 4.0),
            level: 0,
            y: 0.0,
            walk: None,
            edge_clock: 0.0,
            hop: None,
            facing: 0.0,
            turn_v: 0.0,
            heading: (0.0, 1.0),
            path: vec![(1, 2), (1, 3)],
            path_age: 0.1,
            walking: true,
            stride: 0.0,
            smart: 70,
            key: None,
            joined: true,
            join_radius: 0.0,
            door: None,
            hidden: Some(2),
            errand: Some(Errand::Relay(1)),
            claim: None,
            dwell: 1.0,
            visited: vec![Errand::Relay(0)],
            clear_time: 1.0,
            searched: vec![(0.0, 0.0)],
            last_door: None,
            deep_cells: Vec::new(),
            fleeing: false,
            heard: None,
            goal: GoalState::default(), flee: FleeState::default(), life: LifeState::default(),
        };
        n.shift((-16.0, 32.0));
        assert_eq!(n.at, (-13.0, 36.0));
        assert!(n.path.is_empty());
        assert!(n.path_age >= REPATH);
        assert_eq!(
            (n.hidden, n.errand, n.visited.len(), n.searched.len()),
            (None, None, 0, 0),
            "the old frame's hides, trips and memory are gone"
        );
    }

    #[test]
    fn a_bot_with_no_flight_route_backs_off_instead_of_planning_every_frame() {
        let (mut g, content) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        g.npcs[0].joined = true;
        let start = g.npcs[0].at;
        let mut h = crate::hunters::brain::Brain::new(content.hunters[0].clone(), (start.0 + 3.0, start.1), "T", Difficulty::Standard);
        h.active = true;
        h.state = State::Chase;
        g.hunters.push(h);
        // a search of one cell finds no route: the plan fails
        let capped = tuning_with(|v| {
            v["intents"] = names(&["flee"]);
            v["flee"]["max_cells"] = Value::from(1);
        });
        for _ in 0..10 {
            g.update_npcs_with(1.0 / 60.0, &capped);
        }
        assert_eq!(g.npcs[0].flee.goal, None, "the capped search found nothing");
        assert!(g.npcs[0].flee.since > 0.1, "the failed plan started the replan wait: {}", g.npcs[0].flee.since);
    }

    #[test]
    fn a_world_shift_clears_the_goal_the_flight_and_the_claim() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let n = &mut g.npcs[0];
        n.goal.done.push((0, 1, 2));
        n.goal.banned.push(((0, 3, 4), 9.0));
        n.goal.stuck.anchor = (5.0, 6.0);
        n.goal.stuck.still = 2.0;
        n.goal.away = 5.0;
        n.flee.goal = Some((7, 8));
        n.flee.since = 0.0;
        n.claim = Some(Claim::Relay(0));
        n.shift((10.0, 10.0));
        assert!(n.goal.spot.is_none() && n.goal.done.is_empty() && n.goal.banned.is_empty());
        assert_eq!((n.goal.stuck.anchor, n.goal.stuck.still, n.goal.away), ((0.0, 0.0), 0.0, 0.0));
        assert_eq!(n.flee.goal, None);
        assert!(n.flee.since >= 1.0e9, "the next alarm plans at once");
        assert_eq!(n.claim, None);
    }

    #[test]
    fn a_bot_drops_its_goal_after_the_away_time_and_takes_none_until_it_is_back() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        g.hunters.clear();
        g.npcs[0].joined = true;
        let leash = tuning_with(|v| {
            v["intents"] = names(&["goal"]);
            v["goals"]["leash"] = serde_json::json!({ "max_distance": 0.0, "max_away": 1.0 });
        });
        let dt = 1.0 / 60.0;
        for _ in 0..(3 * 60) {
            g.npcs[0].at = (g.p.pos.0 + 3.0, g.p.pos.1);
            g.update_npcs_with(dt, &leash);
        }
        let picks = g.npcs[0].goal.picks;
        assert!(g.npcs[0].goal.spot.is_none(), "the goal was dropped once the bot was away for over a second");
        for _ in 0..(3 * 60) {
            g.npcs[0].at = (g.p.pos.0 + 3.0, g.p.pos.1);
            g.update_npcs_with(dt, &leash);
        }
        assert_eq!(g.npcs[0].goal.picks, picks, "a bot that is away takes no new goal");
    }

    #[test]
    fn a_short_batteries_taken_list_does_not_panic_the_bots() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        g.npcs[0].joined = true;
        g.batteries_taken.clear();
        for _ in 0..30 {
            g.update_npcs_with(1.0 / 60.0, Tuning::get());
        }
    }

    #[test]
    fn a_far_bot_claim_counts_and_grows_the_population_but_does_not_save() {
        let (mut g, content) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        g.hunters.clear();
        let far = g.fac.relays.iter().position(|c| pdist(c.center(), g.p.pos) > Tuning::get().goals.leash_distance);
        let (near, near_i) = (g.fac.relays.iter().position(|c| pdist(c.center(), g.p.pos) <= Tuning::get().goals.leash_distance), far);
        let Some(i) = near_i else { return };
        g.last_autosave = "";
        let bot = super::super::interact::Actor::Npc(g.npcs[0].id);
        assert!(g.claim_relay(i, bot, &content));
        assert_eq!(g.relays_collected, 1, "the team's count and flags are the same as a player's claim");
        assert!(g.relays_taken[i]);
        assert_eq!(g.last_autosave, "", "a claim far from the runner does not save");
        assert!(!g.hunters.is_empty(), "but it releases the hunters at once, as a runner's claim does");
        if let Some(j) = near {
            assert!(g.claim_relay(j, bot, &content));
            assert_eq!(g.last_autosave, "relay", "a claim near the runner saves as before");
        }
    }

    #[test]
    fn standard_tuning_matches_the_game() {
        let t = Tuning::get();
        assert_eq!(t.threat_range, 16.0, "raised so every smartness notices hunters sooner");
        assert_eq!(t.smart_base, 0.6, "raised base of the smart scale (.6 + smart / 100)");
        assert!(t.flee_running, "npc_teammates.gd 324-326 runs");
        assert_eq!(t.follow_radius, 5.0, "npc_teammates.gd 24 FOLLOW_RADIUS");
        assert_eq!(t.door_think, 0.12, "shorter door think so bots clear doors faster");
        assert_eq!(t.door_think_jitter, 0.08, "door think jitter");
        assert_eq!(t.tasks, vec!["door".to_string()]);
        let intents: Vec<String> = ["hide", "flee", "revive", "goal", "locker", "life", "follow"].iter().map(|s| s.to_string()).collect();
        assert_eq!(t.intents, intents, "the order the bots try their intents in");
        assert_eq!(t.hide_chase_range, 14.0, "a chase is seen at the threat range");
        assert_eq!(t.hide_reach, 8.0);
        assert_eq!(t.enter_radius, 1.0);
        assert_eq!(t.hide_leave_range, 16.0);
        assert_eq!(t.hide_leave_delay, 3.0);
        assert_eq!(t.errand_safe_range, 14.0);
        assert_eq!(t.errand_reach, 1.2);
        assert_eq!((t.relay_leash, t.relay_dwell, t.relay_claim_time), (12.0, 4.0, 2.0));
        assert_eq!(t.pickup_leash, 10.0);
        assert_eq!(t.pickup_useful, vec!["battery".to_string(), "stim_pack".to_string()]);
        assert_eq!((t.locker_leash, t.locker_dwell), (10.0, 6.0));
        assert_eq!(t.searched_memory, 8);
        assert_eq!(t.searched_avoid, 4.0);
        assert_eq!(t.door_near, 1.5);
        assert_eq!(t.close_clear, 2.5);
        assert_eq!(*t, tuning_with(|_| {}), "the cached tuning is the embedded file");
    }

    #[test]
    fn the_threat_range_in_data_decides_when_a_bot_runs() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let start = g.npcs[0].at;
        let dt = 1.0 / 60.0;
        let chasing_5m = vec![((start.0 + 5.0, start.1), true)];
        let wide = tuning_with(|v| v["threat"]["range"] = Value::from(14.0));
        let narrow = tuning_with(|v| v["threat"]["range"] = Value::from(1.0));
        let mut presses = vec![];
        {
            let fac = &g.fac;
            assert!(flee(&mut g.npcs[0], fac, &chasing_5m, &wide, dt, &mut presses), "5 m is inside a 14 m range");
            assert!(!flee(&mut g.npcs[0], fac, &chasing_5m, &narrow, dt, &mut presses), "5 m is outside a 1 m range");
        }
        // a hunter that is only wandering and beyond sight range does not start a run
        let far_wandering = vec![((start.0 + 30.0, start.1), false)];
        assert!(!flee(&mut g.npcs[0], &g.fac, &far_wandering, &wide, dt, &mut presses));
    }

    /// An open cell 7 to 12 m from `at` with a clear line to it, for scripted hunter positions.
    fn sight_cell(fac: &Facility, at: (f32, f32)) -> (f32, f32) {
        fac.floor_cells()
            .into_iter()
            .map(|c| c.center())
            .find(|&p| (7.0..12.0).contains(&pdist(at, p)) && fac.has_line_of_sight(at, p) && fac.has_line_of_sight(p, at))
            .expect("an open cell in sight of the bot")
    }

    fn unit(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
        let d = pdist(a, b);
        ((b.0 - a.0) / d, (b.1 - a.1) / d)
    }

    #[test]
    fn a_bot_flees_a_hunter_it_sees_and_not_one_behind_it() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let t = Tuning::get();
        let at = g.npcs[0].at;
        let h = sight_cell(&g.fac, at);
        let dt = 1.0 / 60.0;
        let mut presses = vec![];
        let wandering = vec![(h, false)];
        g.npcs[0].heading = unit(at, h);
        assert!(flee(&mut g.npcs[0], &g.fac, &wandering, t, dt, &mut presses), "a wandering hunter in the view cone is a threat");
        assert!(g.npcs[0].flee.goal.is_some());
        // the same hunter behind the bot, beyond the sense range, is not seen
        let mut behind = g.npcs[0].clone();
        behind.flee = FleeState::default();
        behind.heading = (-unit(at, h).0, -unit(at, h).1);
        assert!(pdist(at, h) > t.flee.sense_range);
        assert!(!flee(&mut behind, &g.fac, &wandering, t, dt, &mut presses), "outside the cone and out of sense range");
        let narrow = tuning_with(|v| v["flee"]["sight_range"] = Value::from(2.0));
        let mut blind = g.npcs[0].clone();
        blind.flee = FleeState::default();
        assert!(!flee(&mut blind, &g.fac, &wandering, &narrow, dt, &mut presses), "the sight range is data");
    }

    #[test]
    fn a_bot_that_heard_a_noise_runs_from_a_hunter_beyond_sight() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let t = Tuning::get();
        let at = g.npcs[0].at;
        let dt = 1.0 / 60.0;
        let mut presses = vec![];
        let hunter = vec![((at.0 + 19.0, at.1), false)];
        assert!(!flee(&mut g.npcs[0], &g.fac, &hunter, t, dt, &mut presses), "19 m is beyond sight and nothing was heard");
        g.npcs[0].heard = Some((at, 2.0));
        assert!(flee(&mut g.npcs[0], &g.fac, &hunter, t, dt, &mut presses), "a heard noise with a hunter within 20 m");
    }

    #[test]
    fn the_flight_route_keeps_clear_of_the_hunter_and_never_wades() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let t = Tuning::get();
        let at = g.npcs[0].at;
        let h = sight_cell(&g.fac, at);
        let d0 = pdist(at, h);
        let route = flee_route(&g.npcs[0], &g.fac, &[h], &t.flee).expect("a route away from the hunter");
        let floor = t.flee.clearance.min(d0);
        for &(x, y) in &route {
            assert!(pdist(Cell::new(x, y).center(), h) >= floor - 1e-3 || (x, y) == route[0], "cell {x},{y} is clear of the hunter");
        }
        let end = *route.last().unwrap();
        assert!(pdist(Cell::new(end.0, end.1).center(), h) > d0, "it ends farther off than it began");
        // the cells of that route turn to deep water: the next route uses none of them
        g.npcs[0].deep_cells = route.iter().skip(1).copied().collect();
        if let Some(dry) = flee_route(&g.npcs[0], &g.fac, &[h], &t.flee) {
            assert!(dry.iter().skip(1).all(|c| !g.npcs[0].deep_cells.contains(c)), "no flight cell is deep water");
        }
    }

    #[test]
    fn a_bot_replans_when_the_hunter_crosses_its_route() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let t = Tuning::get();
        let dt = 1.0 / 60.0;
        let at = g.npcs[0].at;
        let h = sight_cell(&g.fac, at);
        let hunters = vec![(h, true)];
        let mut presses = vec![];
        assert!(flee(&mut g.npcs[0], &g.fac, &hunters, t, dt, &mut presses));
        let first = g.npcs[0].flee.goal.expect("a flight goal");
        assert!(g.npcs[0].path.len() >= 3, "a route of some length");
        // the hunter steps onto the route ahead of the bot: the route is dropped for one that avoids it
        let onto = g.npcs[0].path[2];
        let crossing = vec![(Cell::new(onto.0, onto.1).center(), true)];
        for _ in 0..60 {
            flee(&mut g.npcs[0], &g.fac, &crossing, t, dt, &mut presses);
        }
        let d = pdist(g.npcs[0].at, crossing[0].0);
        let floor = t.flee.clearance.min(d);
        assert!(
            g.npcs[0].path.iter().all(|&(x, y)| pdist(Cell::new(x, y).center(), crossing[0].0) >= floor - 1e-3),
            "the new route keeps off the hunter (first goal {first:?})"
        );
    }

    #[test]
    fn a_bot_hides_in_a_close_locker_from_a_hunter_it_sees() {
        let (mut g, _, at, centre) = locker_case();
        g.hunters[0].state = State::Wander;
        g.hunters[0].pos = g
            .fac
            .floor_cells()
            .into_iter()
            .map(|c| c.center())
            .find(|&p| (5.0..10.0).contains(&pdist(at, p)) && g.fac.has_line_of_sight(at, p) && pdist(p, centre) > pdist(at, centre) + 3.0)
            .expect("an open cell in sight, farther from the locker than the bot");
        g.npcs[0].heading = unit(at, g.hunters[0].pos);
        let dt = 1.0 / 60.0;
        for _ in 0..(10 * 60) {
            g.update_npcs(dt);
            if g.npcs[0].hidden.is_some() {
                break;
            }
        }
        assert!(g.npcs[0].hidden.is_some(), "a wandering hunter in view sends the bot into the locker beside it");
    }

    #[test]
    fn a_hidden_bot_stays_the_leave_delay_after_the_hunter_goes() {
        let (mut g, _, _, _) = locker_case();
        let dt = 1.0 / 60.0;
        for _ in 0..(10 * 60) {
            g.update_npcs(dt);
            if g.npcs[0].hidden.is_some() {
                break;
            }
        }
        assert!(g.npcs[0].hidden.is_some());
        g.hunters.clear();
        for _ in 0..(2.5 * 60.0) as usize {
            g.update_npcs(dt);
        }
        assert!(g.npcs[0].hidden.is_some(), "still in the locker 2.5 s after the hunter left (delay 3 s)");
        for _ in 0..(1.5 * 60.0) as usize {
            g.update_npcs(dt);
        }
        assert!(g.npcs[0].hidden.is_none(), "out once the delay is up");
    }

    #[test]
    fn a_fleeing_bot_does_not_flip_direction_back_and_forth() {
        let (mut g, content) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let dt = 1.0 / 60.0;
        let at = g.npcs[0].at;
        let h = sight_cell(&g.fac, at);
        let mut hunter = crate::hunters::brain::Brain::new(content.hunters[0].clone(), h, "T", Difficulty::Standard);
        hunter.active = true;
        hunter.state = State::Chase;
        g.hunters.push(hunter);
        g.npcs[0].joined = true;
        let flee_only = only(&["flee"]);
        let mut last = g.npcs[0].at;
        let mut last_dir: Option<(f32, f32)> = None;
        let mut flips = 0;
        let seconds = 6;
        for frame in 0..(seconds * 60) {
            // the hunter closes in on the bot at a walk
            let p = g.hunters[0].pos;
            let to = g.npcs[0].at;
            if pdist(p, to) > 1.0 {
                let u = unit(p, to);
                g.hunters[0].pos = (p.0 + u.0 * 1.5 * dt, p.1 + u.1 * 1.5 * dt);
            }
            g.update_npcs_with(dt, &flee_only);
            let now = g.npcs[0].at;
            if frame % 6 == 5 {
                let d = (now.0 - last.0, now.1 - last.1);
                if d.0.hypot(d.1) > 0.05 {
                    if let Some(l) = last_dir {
                        if l.0 * d.0 + l.1 * d.1 < 0.0 {
                            flips += 1;
                        }
                    }
                    last_dir = Some(d);
                }
                last = now;
            }
        }
        assert!(flips <= seconds, "{flips} reversals in {seconds} s: at most one a second");
    }

    #[test]
    fn a_bot_steps_at_the_speed_it_is_asked_for() {
        let (g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let runner = g.p.pos;
        let start = Cell::new(runner.0.floor() as i32, runner.1.floor() as i32);
        let far = sorted_reach(&g.fac, start)
            .into_iter()
            .filter(|(c, steps)| *steps >= 8 && *steps <= 12 && g.fac.body_fits(c.center().0, c.center().1, NAV_RADIUS))
            .map(|(c, _)| c.center())
            .next()
            .expect("a reachable open cell 8 to 12 cells away");
        let dt = 1.0 / 60.0;
        let one_frame = |running: bool| -> f32 {
            let mut n = g.npcs[0].clone();
            n.at = far;
            n.path.clear();
            n.path_age = REPATH;
            n.heading = (0.0, 0.0);
            n.stride = 0.0;
            let mut presses = vec![];
            walk_toward(&mut n, &g.fac, runner, running, Tuning::get(), dt, &mut presses);
            n.stride
        };
        let walk = one_frame(false);
        let sprint = one_frame(true);
        assert!((walk - PLAYER_WALK_SPEED * dt).abs() < 1e-4, "walking covers the walk speed: {walk}");
        assert!((sprint - PLAYER_SPRINT_SPEED * dt).abs() < 1e-4, "running covers the sprint speed: {sprint}");
    }

    #[test]
    fn the_door_think_in_data_sets_the_wait() {
        let dt = 1.0 / 60.0;
        let quick = tuning_with(|v| {
            v["door"]["think"] = Value::from(0.0);
            v["door"]["think_jitter"] = Value::from(0.0);
        });
        let (mut g, cell, a, _) = door_case();
        g.npcs[0].at = a;
        g.p.pos = (a.0 + 40.0, a.1 + 40.0);
        let mut presses = vec![];
        wait_at_door(&mut g.npcs[0], &g.fac, cell, &quick, dt, &mut presses);
        assert_eq!(presses, vec![cell], "no think time: it presses on the first frame");

        let (mut g, cell, a, _) = door_case();
        g.npcs[0].at = a;
        g.p.pos = (a.0 + 40.0, a.1 + 40.0);
        let mut presses = vec![];
        wait_at_door(&mut g.npcs[0], &g.fac, cell, Tuning::get(), dt, &mut presses);
        assert!(presses.is_empty(), "the standard think time (0.25 s) has not passed after one frame");
    }

    #[test]
    fn a_bot_without_the_door_task_never_presses() {
        let dt = 1.0 / 60.0;
        let idle = tuning_with(|v| {
            v["door"]["think"] = Value::from(0.0);
            v["door"]["think_jitter"] = Value::from(0.0);
            v["tasks"] = Value::Array(vec![]);
        });
        assert!(!idle.does("door"));
        let (mut g, cell, a, _) = door_case();
        g.npcs[0].at = a;
        g.p.pos = (a.0 + 40.0, a.1 + 40.0);
        let mut presses = vec![];
        for _ in 0..30 {
            wait_at_door(&mut g.npcs[0], &g.fac, cell, &idle, dt, &mut presses);
        }
        assert!(presses.is_empty(), "no door task, no press");
    }

    /// A run game with a bot joined 2 m in front of locker 0 and a hunter chasing the bot from 4 m. Returns the game,
    /// the content, the bot's start and the locker's centre.
    fn locker_case() -> (Game, Content, (f32, f32), (f32, f32)) {
        let (mut g, content) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        assert!(!g.fac.hiding.is_empty(), "the test floor has a locker");
        let h = g.fac.hiding[0].clone();
        let centre = g.fac.locker_center(&h);
        let front = (centre.0 + h.facing.0 as f32 * 2.0, centre.1 + h.facing.1 as f32 * 2.0);
        let at = g.fac.nearest_open(front.0, front.1, NAV_RADIUS).expect("an open cell in front of the locker");
        assert!(pdist(at, centre) < 3.0, "the bot starts near the locker");
        g.npcs[0].joined = true;
        g.npcs[0].at = at;
        let mut hunter = crate::hunters::brain::Brain::new(
            content.hunters[0].clone(),
            (at.0 - 4.0, at.1),
            "T",
            Difficulty::Standard,
        );
        hunter.active = true;
        hunter.state = State::Chase;
        g.hunters.push(hunter);
        (g, content, at, centre)
    }

    #[test]
    fn a_chased_bot_hides_in_a_locker_and_comes_out_when_the_chase_is_over() {
        let (mut g, _, _, _) = locker_case();
        let dt = 1.0 / 60.0;
        for _ in 0..(10 * 60) {
            g.update_npcs(dt);
            if g.npcs[0].hidden.is_some() {
                break;
            }
        }
        let li = g.npcs[0].hidden.expect("a chased bot with a locker in reach hides in it");
        let centre = g.fac.locker_center(&g.fac.hiding[li]);
        assert!(pdist(g.npcs[0].at, centre) < 0.01, "it stands at the locker's centre");
        // the hunter is gone: once the leave delay (3 s) has passed the bot comes out
        g.hunters.clear();
        for _ in 0..(5 * 60) {
            g.update_npcs(dt);
        }
        assert_eq!(g.npcs[0].hidden, None, "the bot came out once no hunter was near");
    }

    #[test]
    fn the_hide_reach_in_data_decides_whether_a_bot_hides() {
        let (mut g, _, _, _) = locker_case();
        let dt = 1.0 / 60.0;
        // no locker is within 0.5 m of a bot 2 m out: it never goes in
        let short = tuning_with(|v| v["hide"]["reach"] = Value::from(0.5));
        for _ in 0..(10 * 60) {
            g.update_npcs_with(dt, &short);
            assert_eq!(g.npcs[0].hidden, None, "no locker within 0.5 m");
        }
    }

    #[test]
    fn a_hidden_bot_stays_while_a_hunter_is_within_the_leave_range() {
        let (mut g, _, _, _) = locker_case();
        let dt = 1.0 / 60.0;
        let centre = g.fac.locker_center(&g.fac.hiding[0]);
        g.npcs[0].hidden = Some(0);
        g.npcs[0].at = centre;
        // the hunter stands 5 m off the locker, inside the 16 m leave range
        g.hunters[0].pos = (centre.0 + 5.0, centre.1);
        for _ in 0..(10 * 60) {
            g.update_npcs(dt);
        }
        assert_eq!(g.npcs[0].hidden, Some(0), "still hidden with a hunter that near");
    }

    #[test]
    fn the_leave_delay_in_data_sets_how_long_a_bot_stays_hidden() {
        let (mut g, _, _, _) = locker_case();
        let dt = 1.0 / 60.0;
        g.hunters.clear();
        g.npcs[0].hidden = Some(0);
        g.npcs[0].at = g.fac.locker_center(&g.fac.hiding[0]);
        let quick = tuning_with(|v| v["hide"]["leave_delay"] = Value::from(0.5));
        for _ in 0..20 {
            g.update_npcs_with(dt, &quick);
        }
        assert_eq!(g.npcs[0].hidden, Some(0), "a third of a second is short of the 0.5 s delay");
        for _ in 0..60 {
            g.update_npcs_with(dt, &quick);
        }
        assert_eq!(g.npcs[0].hidden, None, "past the 0.5 s delay it came out");
    }

    #[test]
    fn a_bot_goes_to_a_relay_when_it_is_safe_and_then_goes_back_to_following() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        assert!(!g.fac.relays.is_empty(), "the test floor has a relay");
        g.npcs[0].joined = true;
        let trip = relay_trip();
        let dt = 1.0 / 60.0;
        for _ in 0..(120 * 60) {
            g.update_npcs_with(dt, &trip);
            if !g.npcs[0].visited.is_empty() {
                break;
            }
        }
        assert!(matches!(g.npcs[0].visited.first(), Some(Errand::Relay(_))), "the bot reached a relay and stood there for its dwell");
        assert_eq!(g.npcs[0].errand, None, "the trip is over");
    }

    /// The bot on the runner's spot, joined, with a pickup of `item` at its feet.
    fn bot_with_loot(items: &[Item]) -> Game {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        g.npcs[0].joined = true;
        g.hunters.clear();
        g.npcs[0].at = g.p.pos;
        g.pickups.clear();
        g.fac.batteries.clear();
        g.batteries_taken.clear();
        for &item in items {
            g.pickups.push(super::super::FloorPickup { pos: g.p.pos, item, n: 1, taken: false });
        }
        g
    }

    #[test]
    fn a_bot_at_a_relay_recovers_it_once() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        g.npcs[0].joined = true;
        g.hunters.clear();
        let trip = tuning_with(|v| {
            v["intents"] = names(&["relay"]);
            v["errand"]["relay"]["leash"] = Value::from(1000.0);
            v["errand"]["relay"]["dwell"] = Value::from(1.0);
            v["errand"]["relay"]["claim_time"] = Value::from(0.5);
        });
        assert_eq!(g.relays_collected, 0);
        for _ in 0..(120 * 60) {
            g.update_npcs_with(1.0 / 60.0, &trip);
            if !g.npcs[0].visited.is_empty() {
                break;
            }
        }
        let Some(Errand::Relay(i)) = g.npcs[0].visited.first().copied() else { panic!("the bot reached a relay") };
        assert!(g.relays_taken[i], "the relay is claimed");
        assert_eq!(g.relays_collected, 1, "the relay count went up once");
        for _ in 0..(5 * 60) {
            g.update_npcs_with(1.0 / 60.0, &trip);
        }
        assert_eq!(g.relays_collected, 1, "standing on, or going to another relay, never counts the first twice");
        assert!(g.relays_taken.iter().filter(|t| **t).count() as i32 == g.relays_collected);
    }

    #[test]
    fn a_bot_takes_useful_items_and_leaves_junk() {
        let mut g = bot_with_loot(&[Item::Taser, Item::Stim]);
        let t = only(&["pickup"]);
        for _ in 0..120 {
            g.update_npcs_with(1.0 / 60.0, &t);
        }
        assert!(!g.pickups[0].taken, "a taser is junk to a bot");
        assert!(g.pickups[1].taken, "a stim pack is useful");
        assert!(g.slots.iter().flatten().any(|s| s.item == Item::Stim));
        assert!(!g.slots.iter().flatten().any(|s| s.item == Item::Taser));
    }

    #[test]
    fn a_bot_leaves_an_item_the_team_has_no_room_for() {
        let mut g = bot_with_loot(&[Item::Stim]);
        for s in g.slots.iter_mut() {
            *s = Some(super::super::items::Slot { item: Item::Axe, n: 1 });
        }
        for _ in 0..120 {
            g.update_npcs_with(1.0 / 60.0, &only(&["pickup"]));
        }
        assert!(!g.pickups[0].taken);
    }

    #[test]
    fn a_bot_s_claims_equal_the_player_s_interact() {
        use super::super::interact::{Actor, Focus};
        let content = Content::load();
        let sounds = crate::audio::Sounds::default();
        let (mut a, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let (mut b, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        for g in [&mut a, &mut b] {
            g.hunters.clear();
            g.pickups.push(super::super::FloorPickup { pos: g.p.pos, item: Item::Stim, n: 1, taken: false });
        }
        let catalog = Catalog::load();
        let bot = Actor::Npc(b.npcs[0].id);
        assert!(!a.fac.relays.is_empty() && !a.fac.batteries.is_empty());
        a.interact(Focus::Relay(0), &content, &catalog, &sounds);
        a.interact(Focus::Battery(0), &content, &catalog, &sounds);
        a.interact(Focus::Pickup(a.pickups.len() - 1), &content, &catalog, &sounds);
        assert!(b.claim_relay(0, bot, &content));
        assert!(b.take_battery(0, bot));
        let last = b.pickups.len() - 1;
        assert!(b.take_pickup(last, bot));
        assert!(!b.claim_relay(0, bot, &content), "a claimed relay is not claimed twice");
        assert_eq!((a.relays_collected, a.score), (b.relays_collected, b.score));
        assert_eq!(a.relays_taken, b.relays_taken);
        assert_eq!(a.batteries_taken, b.batteries_taken);
        let held = |g: &Game| g.slots.iter().map(|s| s.map(|s| (s.item, s.n))).collect::<Vec<_>>();
        assert_eq!(held(&a), held(&b));
    }

    #[test]
    fn a_relay_leash_of_zero_sends_no_bot_to_a_relay() {
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        g.npcs[0].joined = true;
        let no_leash = tuning_with(|v| {
            v["intents"] = names(&["relay"]);
            v["errand"]["relay"]["leash"] = Value::from(0.0);
        });
        for _ in 0..(10 * 60) {
            g.update_npcs_with(1.0 / 60.0, &no_leash);
        }
        assert!(g.npcs[0].visited.is_empty(), "no relay is within a zero leash of the runner");
        assert_eq!(g.npcs[0].errand, None);
    }

    #[test]
    fn a_hunter_searching_a_relay_keeps_the_bot_off_it() {
        let (mut g, content) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        assert!(!g.fac.relays.is_empty(), "the test floor has a relay");
        g.npcs[0].joined = true;
        let relay0 = g.fac.relays[0].center();
        let mut searcher = crate::hunters::brain::Brain::new(content.hunters[0].clone(), relay0, "T", Difficulty::Standard);
        searcher.active = true;
        searcher.state = State::Search;
        g.hunters.push(searcher);
        let trip = relay_trip();
        for _ in 0..(20 * 60) {
            g.update_npcs_with(1.0 / 60.0, &trip);
            assert_ne!(g.npcs[0].errand, Some(Errand::Relay(0)), "a bot does not pick the relay a hunter is searching");
        }
        assert!(!g.npcs[0].visited.contains(&Errand::Relay(0)));
    }

    /// Whether the door a bot walked through is still open once the bot has stepped clear of it and the shut-behind
    /// rule has run with `close_clear` (data `doors.close_clear`).
    fn door_open_after_passing(close_clear: f32) -> bool {
        let (mut g, cell, a, b) = door_case();
        g.p.pos = (a.0 + 40.0, a.1 + 40.0);
        g.npcs[0].joined = true;
        g.npcs[0].at = a;
        let dt = 1.0 / 60.0;
        for _ in 0..(8 * 60) {
            bot_frame(&mut g, b, dt);
            if pdist(g.npcs[0].at, b) < 0.3 {
                break;
            }
        }
        // no errands or hides: the bot only runs the close-behind rule
        let t = tuning_with(|v| {
            v["intents"] = names(&[]);
            v["doors"]["close_clear"] = Value::from(close_clear as f64);
        });
        g.update_npcs_with(dt, &t);
        for _ in 0..(3 * 60) {
            g.fac.update_doors(&[], &[], dt);
        }
        g.fac.door_is_open(cell)
    }

    #[test]
    fn a_bot_shuts_the_door_behind_it_once_it_is_clear_of_it() {
        assert!(!door_open_after_passing(0.5), "a bot 1 m past the door, clear at 0.5 m, shuts it");
        assert!(door_open_after_passing(5.0), "the same bot is not clear at 5 m, so the door stays open");
    }

    #[test]
    fn the_intent_order_in_data_decides_whether_a_chased_bot_hides_or_runs() {
        let dt = 1.0 / 60.0;
        // flee alone: the bot runs from the chaser and never reaches a locker
        let (mut g, _, _, _) = locker_case();
        let flee_only = only(&["flee"]);
        for _ in 0..(5 * 60) {
            g.update_npcs_with(dt, &flee_only);
        }
        assert_eq!(g.npcs[0].hidden, None, "with no hide intent the bot only runs");
        // hide before flee: the chased bot hides instead
        let (mut g, _, _, _) = locker_case();
        let hide_first = only(&["hide", "flee"]);
        for _ in 0..(10 * 60) {
            g.update_npcs_with(dt, &hide_first);
        }
        assert!(g.npcs[0].hidden.is_some(), "with hide first the chased bot hides");
    }
}

#[cfg(test)]
mod water_tests {
    use super::tests::{pursuit_inputs, run_game};
    use super::*;
    use crate::config::Difficulty;
    use crate::world::facility::Options;

    /// A bot on the first cell of the facility's reach, with the reachable cells by step count (nearest first).
    fn goal_field() -> (Facility, Npc, Vec<Cell>) {
        let fac = facility();
        let start = fac.floor_cells()[0];
        let mut reach: Vec<(Cell, i32)> = fac.distance_map(start).into_iter().filter(|(_, d)| *d >= 3).collect();
        reach.sort_by_key(|(c, d)| (*d, c.x, c.y));
        assert!(reach.len() >= 8, "a facility with room for four spots");
        let cells: Vec<Cell> = reach.iter().map(|(c, _)| *c).collect();
        let n = Npc {
            id: 0, name: String::new(), slot: 1, at: start.center(), level: 0, y: 0.0, walk: None, edge_clock: 0.0, hop: None, facing: 0.0, turn_v: 0.0, heading: (0.0, 0.0),
            path: vec![], path_age: 0.0, walking: false, stride: 0.0, smart: 50, key: None, joined: true,
            join_radius: 0.0, door: None, hidden: None, errand: None, dwell: 0.0, visited: vec![], clear_time: 0.0,
            claim: None,
            searched: vec![], last_door: None, deep_cells: vec![], fleeing: false, heard: None, goal: GoalState::default(), flee: FleeState::default(), life: LifeState::default(),
        };
        (fac, n, cells)
    }

    #[test]
    fn a_bot_walks_to_a_downed_teammate_in_range_and_ignores_one_out_of_range() {
        let (fac, mut n, _) = goal_field();
        let t = Tuning::get();
        let at = n.at;
        let near = [(0, (at.0 + t.follow_radius, at.1), None::<u32>)];
        let sc = Scene { downed: &near, ..goal_scene(&fac, &[]) };
        let mut presses = vec![];
        assert!(revive_walk(&mut n, &sc, t, 0.016, &mut presses), "a downed teammate in range is walked to");
        let far = [(0, (at.0 + t.follow_radius * t.revive_range_factor + 5.0, at.1), None::<u32>)];
        let sc = Scene { downed: &far, ..goal_scene(&fac, &[]) };
        assert!(!revive_walk(&mut n, &sc, t, 0.016, &mut presses), "one out of range is left");
        let own = [(0, at, Some(n.id))];
        let sc = Scene { downed: &own, ..goal_scene(&fac, &[]) };
        assert!(!revive_walk(&mut n, &sc, t, 0.016, &mut presses), "a bot never walks to itself");
        let other_floor = [(1, at, None::<u32>)];
        let sc = Scene { downed: &other_floor, ..goal_scene(&fac, &[]) };
        assert!(!revive_walk(&mut n, &sc, t, 0.016, &mut presses), "a body on another floor is left");
    }

    fn spot(kind: SpotKind, c: Cell) -> Spot {
        Spot { kind, level: 0, cell: c, pos: c.center(), claim: None }
    }

    fn goal_scene<'a>(fac: &'a Facility, spots: &'a [Spot]) -> Scene<'a> {
        Scene { fac, disabled: &[], leader: (0.0, 0.0), seen: &[], searching: &[], runner_level: 0, nav: None, loot: &[], spots, search_all: &[], downed: &[] }
    }

    #[test]
    fn goals_are_picked_relay_then_battery_then_pickup_then_exit_whatever_the_distance() {
        let (fac, n, cells) = goal_field();
        let t = Tuning::get();
        let last = cells.len() - 1;
        // the relay is the farthest and the pickup the nearest: the kind still decides
        let mut spots = vec![
            spot(SpotKind::Pickup, cells[0]),
            spot(SpotKind::Exit, cells[last / 2]),
            spot(SpotKind::Battery, cells[last / 3]),
            spot(SpotKind::Relay, cells[last]),
        ];
        let mut order = vec![];
        while !spots.is_empty() {
            let sc = goal_scene(&fac, &spots);
            let pick = pick_goal(&n, &sc, t).expect("a goal");
            order.push(pick.kind);
            spots.retain(|s| s.kind != pick.kind);
        }
        assert_eq!(order, vec![SpotKind::Relay, SpotKind::Battery, SpotKind::Pickup, SpotKind::Exit]);
        let heal = [spot(SpotKind::Pickup, cells[0]), spot(SpotKind::Heal, cells[last])];
        assert_eq!(pick_goal(&n, &goal_scene(&fac, &heal), t).map(|s| s.kind), Some(SpotKind::Heal));
        let mut weights = t.clone();
        weights.goals.pickup = 90.0;
        let two = [spot(SpotKind::Pickup, cells[0]), spot(SpotKind::Battery, cells[last])];
        assert_eq!(pick_goal(&n, &goal_scene(&fac, &two), &weights).map(|s| s.kind), Some(SpotKind::Pickup), "the order is the data's weights");
        assert_eq!(pick_goal(&n, &goal_scene(&fac, &two), t).map(|s| s.kind), Some(SpotKind::Battery));
    }

    #[test]
    fn a_bot_picks_a_goal_again_only_after_the_replan_wait() {
        let (fac, mut n, cells) = goal_field();
        let t = Tuning::get();
        let (a, b) = (spot(SpotKind::Relay, cells[4]), spot(SpotKind::Relay, cells[5]));
        let mut presses = vec![];
        let dt = 1.0 / 60.0;
        let first = [a];
        assert!(run_goal(&mut n, &goal_scene(&fac, &first), t, dt, &mut presses));
        assert_eq!((n.goal.picks, n.goal.spot.map(|s| s.cell)), (1, Some(a.cell)));
        // the goal vanishes and another stands: the bot drops it and waits, acting as the follow fallback would
        let second = [b];
        for _ in 0..((t.goals.replan - 0.5) / dt) as usize {
            let acted = run_goal(&mut n, &goal_scene(&fac, &second), t, dt, &mut presses);
            assert!(!acted, "no goal while the replan wait runs");
        }
        assert_eq!(n.goal.picks, 1, "no goal spam inside the wait");
        for _ in 0..(1.0 / dt) as usize {
            run_goal(&mut n, &goal_scene(&fac, &second), t, dt, &mut presses);
        }
        assert_eq!((n.goal.picks, n.goal.spot.map(|s| s.cell)), (2, Some(b.cell)), "and picks again once it is over");
        // nothing to do at all: no pick is made and the bot does not act, so "follow" runs
        let (fac2, mut idle, _) = goal_field();
        for _ in 0..600 {
            assert!(!run_goal(&mut idle, &goal_scene(&fac2, &[]), t, dt, &mut presses));
        }
        assert_eq!(idle.goal.picks, 0);
    }

    /// A bot held on its start cell (a body wedged in a corner) for `pinned` seconds while it works towards a relay, then
    /// let go: the frames it took to reach the relay (None when it did not in 150 s), where it ended and its jam count.
    fn boxed_in_run(pinned: f32) -> (Option<usize>, (f32, f32), u32) {
        let (fac, mut n, cells) = goal_field();
        let t = Tuning::get();
        let start = n.at;
        let spots = [spot(SpotKind::Relay, cells[cells.len() - 1])];
        let dt = 1.0 / 60.0;
        let mut presses = vec![];
        for f in 0..(150 * 60) {
            run_goal(&mut n, &goal_scene(&fac, &spots), t, dt, &mut presses);
            if (f as f32) * dt < pinned {
                n.at = start;
            } else if pdist(n.at, spots[0].pos) <= t.errand_reach {
                return (Some(f), n.at, n.goal.stuck.count);
            }
        }
        (None, n.at, n.goal.stuck.count)
    }

    #[test]
    fn a_boxed_in_bot_recovers_and_reaches_its_goal() {
        let s = &Tuning::get().goals.stuck;
        let (done, _, jams) = boxed_in_run(s.seconds * 2.5);
        assert!(done.is_some(), "the bot reached the relay after the jam");
        assert!(jams >= 2, "it noticed the jam and climbed the ladder: {jams}");
    }

    #[test]
    fn a_bot_that_stays_boxed_in_gives_the_goal_up_for_a_while() {
        let (fac, mut n, cells) = goal_field();
        let t = Tuning::get();
        let s = &t.goals.stuck;
        let (start, dt) = (n.at, 1.0 / 60.0);
        let spots = [spot(SpotKind::Relay, cells[cells.len() - 1])];
        let mut presses = vec![];
        let frames = |secs: f32| (secs / dt) as usize;
        for _ in 0..frames(s.seconds * 5.0 + 2.0) {
            run_goal(&mut n, &goal_scene(&fac, &spots), t, dt, &mut presses);
            n.at = start;
        }
        assert_eq!(n.goal.picks, 1);
        assert!(n.goal.spot.is_none(), "the goal is given up");
        assert_eq!(n.goal.banned.len(), 1, "and kept off the list for now");
        assert!(!n.goal.done.contains(&spots[0].key()), "it is not given up for good");
        for _ in 0..frames(s.ban_seconds - 8.0) {
            run_goal(&mut n, &goal_scene(&fac, &spots), t, dt, &mut presses);
            n.at = start;
        }
        assert_eq!(n.goal.picks, 1, "no pick while the ban runs");
        for _ in 0..frames(8.0) {
            run_goal(&mut n, &goal_scene(&fac, &spots), t, dt, &mut presses);
            n.at = start;
        }
        assert!(n.goal.picks >= 2, "the spot is wanted again once the ban is over");
    }

    #[test]
    fn a_jammed_bot_does_the_same_on_every_run_of_a_seed() {
        let a = boxed_in_run(Tuning::get().goals.stuck.seconds * 3.5);
        let b = boxed_in_run(Tuning::get().goals.stuck.seconds * 3.5);
        assert_eq!(a, b);
        assert!(a.0.is_some());
    }

    #[test]
    fn a_bot_reaches_the_exit_once_the_relays_are_claimed() {
        let (content, catalog) = (crate::content::Content::load(), crate::world::catalog::Catalog::load());
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        let t = {
            let mut t = Tuning::get().clone();
            t.intents = vec!["goal".to_string()];
            t
        };
        g.hunters.clear();
        g.fac.has_lift = true;
        let exit = g.fac.nearest_open(g.fac.exit.center().0, g.fac.exit.center().1, NAV_RADIUS).map_or(g.fac.exit.center(), |p| p);
        let from = cell_at(exit);
        let mut reach: Vec<(Cell, i32)> = g.fac.distance_map(from).into_iter().collect();
        reach.sort_by_key(|(c, d)| (*d, c.x, c.y));
        let start = reach
            .into_iter()
            .filter(|&(c, steps)| steps >= 12 && g.fac.body_fits(c.center().0, c.center().1, NAV_RADIUS))
            .map(|(c, _)| c.center())
            .next()
            .expect("a start away from the exit");
        let (e, d) = ((g.p.pos.0, 1.6, g.p.pos.1), (0.0, 1.0, 0.0));
        g.npc_spawn(1, 70, e, d);
        g.npcs[0].joined = true;
        g.npcs[0].at = start;
        g.p.pos = start;
        let all = g.fac.relays.len();
        g.relays_taken = vec![true; all];
        g.relays_collected = all as i32;
        let dt = 1.0 / 60.0;
        let mut reached = false;
        for _ in 0..(150 * 60) {
            g.update_npcs_with(dt, &t);
            let bodies: Vec<(f32, f32)> = std::iter::once(g.p.pos).chain(g.npcs.iter().map(|n| n.at)).collect();
            g.fac.update_doors(&[], &bodies, dt);
            if pdist(g.npcs[0].at, g.fac.exit.center()) <= t.errand_reach + 0.5 {
                reached = true;
                break;
            }
        }
        assert!(reached, "the bot walked to the exit: at {:?}, exit {:?}, start {start:?} goal {:?} banned {:?} done {:?} path {:?} stuck {:?}", g.npcs[0].at, g.fac.exit.center(), g.npcs[0].goal.spot, g.npcs[0].goal.banned, g.npcs[0].goal.done, g.npcs[0].path, g.npcs[0].goal.stuck);
    }

    fn facility() -> Facility {
        let cat = crate::world::catalog::Catalog::load();
        Facility::generate("BOTWATER-1", &Options::default(), &cat)
    }

    /// A start, a goal, a direct route of at least six cells and one of its middle cells that a dry detour avoids.
    fn detour_case(fac: &Facility) -> Option<(Cell, Cell, (i32, i32))> {
        let cells = fac.floor_cells();
        for a in cells.iter().step_by(11).take(80) {
            for b in cells.iter().step_by(17).take(80) {
                let p = fac.find_path(*a, *b);
                if p.len() < 6 {
                    continue;
                }
                let mid = p[p.len() / 2];
                if !dry_route(fac, *a, *b, &[(mid.x, mid.y)]).is_empty() {
                    return Some((*a, *b, (mid.x, mid.y)));
                }
            }
        }
        None
    }

    #[test]
    fn standard_water_threshold_is_data_and_filters_the_depths() {
        let t = Tuning::get();
        assert!(t.water_avoid_depth > 0.0, "a bot avoids water of some depth");
        let depths = [((1, 1), 0.2), ((2, 1), t.water_avoid_depth), ((3, 1), t.water_avoid_depth + 0.5)];
        assert_eq!(deep_cells_of(&depths, t.water_avoid_depth), vec![(2, 1), (3, 1)]);
        assert!(deep_cells_of(&depths, 99.0).is_empty(), "a changed data value changes which cells are deep");
    }

    /// A bot already in deep channel water behind a railing (a cell the hunters' grid closes) walks out to the walkway by
    /// the gap, and never crosses the railing. Before the fix its route search started from the closed cell and found
    /// nothing, so it stood still in the water.
    #[test]
    fn a_bot_in_deep_railed_water_walks_out_to_the_walkway() {
        let fac = crate::world::facility::railed_pool();
        let start = Cell::new(9, 11);
        assert!(!fac.hunter_open(start), "the water behind the railing is closed to the grid");
        assert!(fac.body_fits(9.5, 11.5, NAV_RADIUS), "a body fits in the water: it is not a wall");
        let pool: Vec<(i32, i32)> = (8..=14).map(|x| (x, 11)).collect();
        let mut n = Npc {
            id: 0, name: String::new(), slot: 1, at: (9.5, 11.5), level: 0, y: 0.0, walk: None, edge_clock: 0.0, hop: None, facing: 0.0, turn_v: 0.0, heading: (0.0, 0.0),
            path: vec![], path_age: 0.0, walking: false, stride: 0.0, smart: 50, key: None, joined: true,
            join_radius: 0.0, door: None, hidden: None, errand: None, dwell: 0.0, visited: vec![], clear_time: 0.0,
            claim: None,
            searched: vec![], last_door: None, deep_cells: pool, fleeing: false, heard: None, goal: GoalState::default(), flee: FleeState::default(), life: LifeState::default(),
        };
        let t = Tuning::get();
        let mut presses = vec![];
        let goal = (9.5, 10.5);
        for _ in 0..400 {
            walk_toward(&mut n, &fac, goal, false, t, 0.1, &mut presses);
            if n.at.1 < 11.0 {
                break;
            }
        }
        assert!(n.at.1 < 11.0, "the bot is out of the water and on the walkway, at {:?}", n.at);
        assert!(fac.hunter_open(cell_at(n.at)), "it stands on a cell the grid opens");
        assert!(!fac.is_water(cell_at(n.at)));
    }

    #[test]
    fn a_walking_bot_detours_round_deep_water_and_a_fleeing_bot_wades() {
        let fac = facility();
        let (a, b, mid) = detour_case(&fac).expect("a generated floor has a route with a detour");
        let deep = [mid];
        let direct = fac.find_path(a, b);
        assert!(crosses_deep(&direct, &deep));
        let walking = route_for(&fac, a, b, &deep, false);
        assert!(!walking.is_empty() && !crosses_deep(&walking, &deep), "walking: dry route");
        assert_eq!(walking.last(), Some(&b));
        let fleeing = route_for(&fac, a, b, &deep, true);
        assert_eq!(fleeing, direct, "fleeing: the shortest route, water or not");
    }

    #[test]
    fn plan_uses_the_dry_route_for_a_bot_that_is_not_fleeing() {
        let fac = facility();
        let (a, b, mid) = detour_case(&fac).expect("a detour case");
        let mut n = Npc {
            id: 0, name: String::new(), slot: 1, at: a.center(), level: 0, y: 0.0, walk: None, edge_clock: 0.0, hop: None, facing: 0.0, turn_v: 0.0, heading: (0.0, 0.0),
            path: vec![], path_age: 0.0, walking: false, stride: 0.0, smart: 50, key: None, joined: true,
            join_radius: 0.0, door: None, hidden: None, errand: None, dwell: 0.0, visited: vec![], clear_time: 0.0,
            claim: None,
            searched: vec![], last_door: None, deep_cells: vec![mid], fleeing: false, heard: None, goal: GoalState::default(), flee: FleeState::default(), life: LifeState::default(),
        };
        plan(&mut n, &fac, b.center());
        assert!(!n.path.contains(&mid), "the planned route keeps out of the deep cell");
        n.fleeing = true;
        plan(&mut n, &fac, b.center());
        assert!(n.path.contains(&mid), "a fleeing bot takes the short way through it");
    }

    #[test]
    fn with_no_dry_way_a_bot_wades_rather_than_stands_still() {
        let fac = facility();
        let cells = fac.floor_cells();
        let a = cells[0];
        let b = *cells.iter().find(|c| fac.find_path(a, **c).len() >= 3).expect("a reachable far cell");
        let all: Vec<(i32, i32)> = cells.iter().filter(|c| **c != a && **c != b).map(|c| (c.x, c.y)).collect();
        let r = route_for(&fac, a, b, &all, false);
        assert_eq!(r, fac.find_path(a, b));
    }

    #[test]
    fn deep_cells_near_reads_the_water_field_and_the_threshold_decides() {
        let deep = |c: Cell| if c.x == 5 && c.y == 5 { 0.7 } else if c.x == 6 && c.y == 5 { 0.2 } else { 0.0 };
        assert_eq!(deep_cells_near((5.5, 5.5), 0.5, deep), vec![(5, 5)]);
        assert_eq!(deep_cells_near((5.5, 5.5), 0.1, deep), vec![(5, 5), (6, 5)]);
        assert!(deep_cells_near((5.5, 5.5), 0.9, deep).is_empty());
        assert!(deep_cells_near((500.5, 5.5), 0.5, deep).is_empty(), "far from the water, nothing is near");
    }

    #[test]
    fn a_bot_avoids_a_flooded_cell_from_a_pipe_flood() {
        use crate::game::hunting::{HunterWater, WaterKey};
        use crate::hunters::water::WaterField;
        use std::sync::Arc;
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let t = Tuning::get();
        // a start, a goal and a middle cell of the direct route that a dry detour avoids, close enough to be in the
        // bot's water look
        let cells = g.fac.floor_cells();
        let mut found = None;
        'search: for a in cells.iter().step_by(7).take(120) {
            for b in cells.iter().step_by(13).take(120) {
                let p = g.fac.find_path(*a, *b);
                if p.len() < 6 {
                    continue;
                }
                let mid = p[p.len() / 2];
                let near = (mid.x - a.x).abs().max((mid.y - a.y).abs()) <= WATER_LOOK;
                if near && !dry_route(&g.fac, *a, *b, &[(mid.x, mid.y)]).is_empty() {
                    found = Some((*a, *b, (mid.x, mid.y)));
                    break 'search;
                }
            }
        }
        let (a, b, mid) = found.expect("a generated floor has a route with a detour near its start");
        g.npcs[0].at = a.center();
        g.npcs[0].level = g.runner_level();
        // a hand-built field: a pipe flood over that one cell (flooded, deeper than the bots avoid)
        let field = Arc::new(WaterField::from_depths(&[(mid, t.water_avoid_depth + 1.0)], &[mid]));
        let origin = g.window_origin();
        let key = |level: i32| WaterKey { chunk: None, origin, level, bucket: 1, pipe_cells: 1, hunters: 0 };
        g.hunter_water = Some(HunterWater { key: key(g.runner_level()), field: field.clone(), splash_in: vec![], last_pos: vec![] });
        g.npc_sync_water(t);
        assert!(g.npcs[0].deep_cells.contains(&mid), "the flooded cell is deep to the bot");
        let mut n = g.npcs[0].clone();
        plan(&mut n, &g.fac, b.center());
        assert!(!n.path.contains(&mid), "the bot's route goes round the flooded cell");
        // the same field on another storey is a dry floor to this bot: the direct route again
        g.hunter_water = Some(HunterWater { key: key(g.runner_level() + 1), field, splash_in: vec![], last_pos: vec![] });
        g.npc_sync_water(t);
        assert!(g.npcs[0].deep_cells.is_empty());
        let mut n = g.npcs[0].clone();
        plan(&mut n, &g.fac, b.center());
        assert!(n.path.contains(&mid), "with no water on its storey the bot takes the direct route through that cell");
    }

    #[test]
    fn a_valve_turn_is_heard_by_a_bot_in_range_and_not_out_of_it() {
        use crate::hunters::noise_events::{NoiseEvent, NoiseKind};
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let at = g.npcs[0].at;
        let level = g.npcs[0].level;
        g.npc_hear(&[NoiseEvent::new(NoiseKind::ValveTurn, (at.0 + 2.0, at.1), level)]);
        let (p, left) = g.npcs[0].heard.expect("the bot heard the valve");
        assert_eq!(p, (at.0 + 2.0, at.1));
        assert!((left - crate::hunters::noise_events::rules().bot_look_seconds).abs() < 1e-6);

        g.npcs[0].heard = None;
        g.npc_hear(&[NoiseEvent::new(NoiseKind::ValveTurn, (at.0 + 300.0, at.1), level)]);
        assert!(g.npcs[0].heard.is_none());
        g.npc_hear(&[NoiseEvent::new(NoiseKind::ValveTurn, (at.0 + 2.0, at.1), level + 2)]);
        assert!(g.npcs[0].heard.is_none(), "a sound two storeys away is not heard");
    }

    #[test]
    fn a_sound_one_storey_away_is_heard_at_the_adjacent_scale_only() {
        use crate::hunters::noise_events::{rules, NoiseEvent, NoiseKind};
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let r = rules();
        let at = g.npcs[0].at;
        let level = g.npcs[0].level;
        let probe = NoiseEvent::new(NoiseKind::ValveTurn, at, level + 1);
        let scale = 0.5;
        let reach = r.radius_of(&probe, r.bot_hearing_scale * scale);
        assert!(reach > 0.5, "a valve turn carries some way");
        // inside the scaled reach: heard from the storey above and below
        for gap in [1, -1] {
            g.npcs[0].heard = None;
            let near = NoiseEvent::new(NoiseKind::ValveTurn, (at.0 + reach * 0.5, at.1), level + gap);
            bots_hear_with(&mut g.npcs, &[near], r, scale);
            assert!(g.npcs[0].heard.is_some(), "a sound {gap} storey away inside the scaled reach is heard");
        }
        // outside the scaled reach but inside the full reach: heard on the bot's own level, not a storey away
        let between = (at.0 + (reach + r.radius_of(&probe, r.bot_hearing_scale)) * 0.5, at.1);
        assert!(between.0 - at.0 > reach, "the case lies between the two reaches");
        g.npcs[0].heard = None;
        bots_hear_with(&mut g.npcs, &[NoiseEvent::new(NoiseKind::ValveTurn, between, level + 1)], r, scale);
        assert!(g.npcs[0].heard.is_none(), "past the scaled reach the adjacent storey does not hear it");
        bots_hear_with(&mut g.npcs, &[NoiseEvent::new(NoiseKind::ValveTurn, between, level)], r, scale);
        assert!(g.npcs[0].heard.is_some(), "the same sound on the bot's own level is heard");
        // a changed data value changes what is heard
        g.npcs[0].heard = None;
        bots_hear_with(&mut g.npcs, &[NoiseEvent::new(NoiseKind::ValveTurn, between, level + 1)], r, 1.0);
        assert!(g.npcs[0].heard.is_some(), "at scale 1.0 the adjacent storey hears as well as its own");
    }

    #[test]
    fn a_bot_reads_the_water_of_its_own_level_only() {
        use crate::game::hunting::{HunterWater, WaterKey};
        use crate::hunters::water::WaterField;
        use std::sync::Arc;
        let (mut g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let t = Tuning::get();
        let at = g.npcs[0].at;
        let cell = (at.0.floor() as i32 + 1, at.1.floor() as i32);
        // a pipe flood: the cell is flooded, whatever the bed
        let field = Arc::new(WaterField::from_depths(&[(cell, t.water_avoid_depth + 1.0)], &[cell]));
        let origin = g.window_origin();
        let key = |level: i32| WaterKey { chunk: None, origin, level, bucket: 1, pipe_cells: 1, hunters: 0 };
        let level = g.npcs[0].level;
        g.hunter_water = Some(HunterWater { key: key(level), field: field.clone(), splash_in: vec![], last_pos: vec![] });
        g.npc_sync_water(t);
        assert!(g.npcs[0].deep_cells.contains(&cell), "the flooded cell is deep for a bot on the field's level");
        g.hunter_water = Some(HunterWater { key: key(level - 1), field, splash_in: vec![], last_pos: vec![] });
        g.npc_sync_water(t);
        assert!(g.npcs[0].deep_cells.is_empty(), "a bot on another storey reads a dry floor");
    }

    #[test]
    fn a_heard_noise_fades_and_never_changes_a_hunter_or_the_danger_inputs() {
        use crate::hunters::noise_events::{NoiseEvent, NoiseKind};
        let (mut g, content) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let at = g.npcs[0].at;
        let level = g.npcs[0].level;
        let mut h = crate::hunters::brain::Brain::new(content.hunters[0].clone(), (at.0 + 3.0, at.1), "T", Difficulty::Standard);
        h.active = true;
        h.state = State::Wander;
        g.hunters.push(h);
        let danger = g.danger();
        let (nearest, pursued) = pursuit_inputs(&g);
        g.npc_hear(&[NoiseEvent::new(NoiseKind::PipeBurst, (at.0 + 2.0, at.1), level)]);
        assert_eq!(g.hunters.last().unwrap().state, State::Wander, "a bot hearing a sound moves no hunter");
        assert_eq!(g.danger(), danger);
        assert_eq!(pursuit_inputs(&g), (nearest, pursued));
        let look = crate::hunters::noise_events::rules().bot_look_seconds;
        let t = Tuning::get();
        for _ in 0..((look * 60.0) as usize + 10) {
            g.update_npcs_with(1.0 / 60.0, t);
        }
        assert!(g.npcs[0].heard.is_none(), "the look lasts bot_look_seconds and then ends");
    }
}

/// Bots across levels (docs/STAIRS_CONTINUOUS_DESIGN.md, phase 8): a bot on any resident level keeps simulating and
/// follows the runner over the same flights and ladders. The scenes are real Infinite windows with the neighbour level
/// resident (`keep_stack`), stepped through `Game::step` so the flight graph is refreshed as in a run.
#[cfg(test)]
mod level_tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::Controls;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Stair;
    use crate::world::flight::FlightShape;
    use crate::world::vertical::STOREY;

    const FRAME: f32 = 1.0 / 60.0;
    /// No bot may take longer than this to reach a runner one storey away.
    const CAP_SECONDS: f32 = 40.0;
    /// Levels whose downward flights include each shape (as game/stack_walk_tests.rs).
    const CASES: [(FlightShape, i32); 5] =
        [(FlightShape::Switchback, 0), (FlightShape::Switchback, -1), (FlightShape::Straight, -1), (FlightShape::Spiral, 0), (FlightShape::Spiral, -2)];

    /// A window with the runner standing at one end of a link and one bot at the other, on the neighbour level.
    struct Case {
        g: Game,
        content: Content,
        catalog: Catalog,
    }

    fn tick(c: &mut Case) {
        c.g.hunters.clear();
        c.g.step(FRAME, &Controls::default(), &c.content, &c.catalog, &Sounds::default());
    }

    /// Steps until `done` or `secs`; the seconds it took.
    fn run(c: &mut Case, secs: f32, mut done: impl FnMut(&Game) -> bool) -> Option<f32> {
        for i in 0..(secs / FRAME) as usize {
            tick(c);
            if done(&c.g) {
                return Some((i + 1) as f32 * FRAME);
            }
        }
        None
    }

    /// The bot is on the runner's level, off any edge.
    fn arrived(g: &Game) -> bool {
        g.npcs[0].level == g.runner_level() && g.npcs[0].walk.is_none()
    }

    /// A window on `level`: the runner on the end of the link on that level (`dir` -1: the landing of a flight down, the
    /// hatch of a ladder; `dir` 1: the foot of a flight up), the neighbour level resident and one joined bot at the far
    /// end of the link on it. None (with a message) when 200 seeds have no such link.
    fn case(level: i32, dir: i32, pick: impl Fn(&Stair) -> bool) -> Option<Case> {
        let (content, catalog) = (Content::load(), Catalog::load());
        let sounds = Sounds::default();
        for n in 0..200 {
            let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, &format!("ROOM-STACK-{n}"), level, &content, &catalog);
            let choose = |g: &Game| {
                g.fac
                    .stairs
                    .iter()
                    .copied()
                    .filter(|s| s.dir == dir && pick(s))
                    .min_by(|a, b| pdist(a.cell.center(), g.p.pos).total_cmp(&pdist(b.cell.center(), g.p.pos)))
            };
            let Some(first) = choose(&g) else { continue };
            g.p.pos = first.cell.center();
            g.step(FRAME, &Controls::default(), &content, &catalog, &sounds);
            let Some(stair) = choose(&g) else { continue };
            g.p.pos = stair.cell.center();
            for _ in 0..8 {
                g.keep_stack(&content, &catalog);
            }
            let other = level + if dir < 0 { -1 } else { 1 };
            if !g.inf.as_ref().is_some_and(|i| i.stack.has(other)) {
                continue;
            }
            let end = if stair.is_ladder() {
                stair.cell.center()
            } else {
                let path = stair.walk_path(level, 0.25);
                if dir < 0 { *path.last().unwrap() } else { path[0] }
            };
            g.npcs.clear();
            // these scenes test following: no relay, item or exit anywhere gives the bot a goal of its own
            g.fac.relays.clear();
            g.relays_taken.clear();
            g.fac.batteries.clear();
            g.batteries_taken.clear();
            g.pickups.clear();
            g.fac.has_lift = false;
            for r in g.inf.as_mut().unwrap().stack.residents.iter_mut() {
                r.fac.relays.clear();
            }
            let at = {
                let r = g.inf.as_ref().unwrap().stack.get(other).unwrap();
                r.fac.nearest_open(end.0, end.1, NAV_RADIUS).unwrap_or(end)
            };
            g.push_npc(at, other, 70, None, 0.0);
            g.hunters.clear();
            return Some(Case { g, content, catalog });
        }
        eprintln!("SKIPPED: no window on level {level} has a dir {dir} link in 200 seeds");
        None
    }

    fn ladder_case() -> Option<Case> {
        [-1, -2].into_iter().find_map(|l| case(l, -1, |s| s.is_ladder()))
    }

    fn up_case(shape: FlightShape, level: i32) -> Option<Case> {
        case(level, -1, move |s| !s.is_ladder() && s.shape == shape)
    }

    fn any_switchback() -> Option<Case> {
        up_case(FlightShape::Switchback, 0).or_else(|| up_case(FlightShape::Switchback, -1))
    }

    #[test]
    fn a_bot_follows_across_levels() {
        let mut cases: Vec<(String, Case)> = CASES
            .iter()
            .filter_map(|&(shape, level)| up_case(shape, level).map(|c| (format!("{shape:?} {level}"), c)))
            .collect();
        if let Some(c) = ladder_case() {
            cases.push(("ladder".to_string(), c));
        }
        for (what, mut c) in cases {
            let floor = floor_y_of(c.g.runner_level());
            assert_eq!(c.g.npcs[0].level, c.g.runner_level() - 1, "{what}: the bot starts a storey below");
            let t = run(&mut c, CAP_SECONDS, arrived);
            assert!(t.is_some(), "{what}: the bot reached the runner's level within {CAP_SECONDS} s");
            let n = &c.g.npcs[0];
            assert_eq!(n.level, c.g.runner_level(), "{what}");
            assert!((n.y - floor).abs() < 0.05, "{what}: the bot stands on the floor it reached: y {} floor {floor}", n.y);
        }
    }

    /// The case of `a_bot_follows_down_a_flight`: the bot on level 0, the runner one storey down, and a relay put on the
    /// runner's level at least `GOAL_STEPS` steps from the runner (a goal, since a follower stands within 5 m).
    fn relay_case() -> Option<(Case, usize)> {
        let mut c = case(-1, 1, |s| !s.is_ladder())?;
        let from = cell_at(c.g.p.pos);
        let mut far: Vec<(Cell, i32)> = c.g.fac.distance_map(from).into_iter().filter(|(_, d)| *d >= GOAL_STEPS && *d <= GOAL_STEPS + 12).collect();
        far.sort_by_key(|(c, d)| (*d, c.x, c.y));
        let relay = far.first()?.0;
        c.g.fac.relays.push(relay);
        c.g.relays_taken.push(false);
        assert_eq!(c.g.npcs[0].level, 0);
        Some((c, 0))
    }

    /// Steps of the relay from the runner in `relay_case`.
    const GOAL_STEPS: i32 = 14;

    fn relay_run(c: &mut Case, i: usize) -> Option<f32> {
        run(c, 120.0, |g| g.relays_taken[i])
    }

    #[test]
    fn a_bot_on_level_0_reaches_a_relay_on_another_level() {
        let Some((mut c, i)) = relay_case() else { return };
        let t = relay_run(&mut c, i);
        assert!(t.is_some(), "the bot recovered the relay a storey down, away from the runner");
        let n = &c.g.npcs[0];
        assert_eq!(n.level, -1);
        assert!(n.goal.picks >= 1, "by a goal, not by following");
        assert!(pdist(n.at, c.g.p.pos) > 5.0, "the runner never came to it");
        assert_eq!(c.g.relays_collected, 1);
    }

    #[test]
    fn a_bot_goes_back_to_following_when_the_goals_are_done() {
        let Some((mut c, i)) = relay_case() else { return };
        assert!(relay_run(&mut c, i).is_some());
        run(&mut c, 60.0, |g| g.npcs[0].goal.spot.is_none() && pdist(g.npcs[0].at, g.p.pos) <= 6.0);
        let n = &c.g.npcs[0];
        assert!(n.goal.spot.is_none());
        assert!(pdist(n.at, c.g.p.pos) <= 6.0, "with nothing left to do it stays with the runner: {}", pdist(n.at, c.g.p.pos));
    }

    #[test]
    fn the_goal_walk_is_the_same_on_every_run() {
        let go = || {
            let (mut c, i) = relay_case()?;
            let t = relay_run(&mut c, i);
            let n = &c.g.npcs[0];
            Some((t, n.at, n.level, n.goal.picks, n.goal.done.clone()))
        };
        let (a, b) = (go(), go());
        assert_eq!(a, b);
    }

    #[test]
    fn a_bot_follows_down_a_flight() {
        let Some(mut c) = [-1, -2].into_iter().find_map(|l| case(l, 1, |s| !s.is_ladder())) else { return };
        let runner = c.g.runner_level();
        assert_eq!(c.g.npcs[0].level, runner + 1, "the bot starts a storey above");
        let t = run(&mut c, CAP_SECONDS, arrived);
        assert!(t.is_some(), "the bot walked down to the runner within {CAP_SECONDS} s");
        let n = &c.g.npcs[0];
        assert_eq!(n.level, runner);
        assert!((n.y - floor_y_of(runner)).abs() < 0.05, "it stands on the lower floor: y {}", n.y);
    }

    #[test]
    fn a_bot_on_a_resident_floor_keeps_simulating() {
        let Some(mut c) = any_switchback() else { return };
        let (before, level) = (c.g.npcs[0].at, c.g.npcs[0].level);
        assert!(c.g.inf.as_ref().unwrap().stack.has(level), "its floor is resident, not the active one");
        run(&mut c, 3.0, |_| false);
        let n = &c.g.npcs[0];
        assert!(
            pdist(n.at, before) > 0.1 || n.walk.is_some() || n.level != level,
            "a bot on a resident floor moves: {before:?} -> {:?}",
            n.at
        );
    }

    #[test]
    fn a_bot_on_a_non_resident_floor_is_set_down_on_the_runners_floor() {
        let Some(mut c) = any_switchback() else { return };
        let far = c.g.runner_level() + 9;
        assert!(!c.g.inf.as_ref().unwrap().stack.has(far));
        c.g.npcs[0].level = far;
        let here = c.g.runner_level();
        run(&mut c, 3.0, |_| false);
        let n = &c.g.npcs[0];
        assert_eq!(n.level, here, "a bot on a level that is not resident is set down on the runner's floor");
        assert!(n.y.is_finite());
    }

    #[test]
    fn a_bot_uses_the_ladder_at_climb_speed() {
        let Some(mut c) = ladder_case() else { return };
        let mut on_edge: Vec<f32> = vec![];
        run(&mut c, CAP_SECONDS, |g| {
            let n = &g.npcs[0];
            if n.walk.is_some() {
                on_edge.push(n.y);
            }
            arrived(g)
        });
        assert!(on_edge.len() > 2, "the bot climbed the ladder as an edge walk");
        let rise = (on_edge[on_edge.len() - 1] - on_edge[0]).abs();
        let secs = on_edge.len() as f32 * FRAME;
        let speed = LADDER_CLIMB_SPEED * flight_edges::rules().ladder_speed_scale;
        assert!(rise > STOREY * 0.5, "it climbed most of a storey: {rise}");
        assert!(secs + 2.0 * FRAME >= rise / speed, "the climb took {secs} s but {rise} m at {speed} m/s needs {}", rise / speed);
    }

    #[test]
    fn a_bot_does_not_get_stuck_on_a_flight() {
        for (shape, level) in CASES {
            let Some(mut c) = up_case(shape, level) else { continue };
            let mut worst = 0.0f32;
            let t = run(&mut c, CAP_SECONDS, |g| {
                worst = worst.max(g.npcs[0].edge_clock);
                arrived(g)
            });
            assert!(t.is_some(), "{shape:?} at {level}: the bot reached the runner within {CAP_SECONDS} s");
            assert!(worst <= CAP_SECONDS, "{shape:?} at {level}: no edge lasted past the cap");
            // and it keeps following: nothing leaves it on an edge or off the level afterwards
            run(&mut c, 3.0, |_| false);
            assert_eq!(c.g.npcs[0].level, c.g.runner_level());
            assert!(c.g.npcs[0].walk.is_none());
        }
    }

    #[test]
    fn a_bot_stuck_on_an_edge_is_put_on_the_nearer_end() {
        let Some(mut c) = any_switchback() else { return };
        let started = run(&mut c, CAP_SECONDS, |g| g.npcs[0].walk.is_some());
        assert!(started.is_some(), "the bot reached the flight");
        c.g.npcs[0].edge_clock = 1.0e6;
        tick(&mut c);
        let n = &c.g.npcs[0];
        assert!(n.walk.is_none(), "the give-up ends the edge");
        let on_floor = [n.level - 1, n.level, n.level + 1].iter().any(|l| (n.y - floor_y_of(*l)).abs() < 0.3);
        assert!(on_floor, "it stands on one of the two floors, not mid-air: y {}", n.y);
        // it does not loop: it still arrives
        assert!(run(&mut c, CAP_SECONDS, arrived).is_some(), "after the give-up the bot still reaches the runner");
    }

    #[test]
    fn a_bots_head_eases_back_on_a_flight() {
        let Some(mut c) = any_switchback() else { return };
        let started = run(&mut c, CAP_SECONDS, |g| g.npcs[0].walk.is_some());
        assert!(started.is_some(), "the bot reached the flight");
        assert!(Tuning::get().lively.rate > 0.0, "the data turns the head");
        c.g.npcs[0].life.head_yaw = 1.0;
        c.g.npcs[0].life.head_pitch = 0.5;
        c.g.npcs[0].life.gaze_left = 2.0;
        tick(&mut c);
        let n = &c.g.npcs[0];
        assert!(n.life.head_yaw.abs() < 1.0, "the head turns back on the edge: {}", n.life.head_yaw);
        assert!(n.life.head_pitch.abs() < 0.5, "the pitch eases too: {}", n.life.head_pitch);
        assert!(n.life.gaze.is_none() && n.life.gaze_left == 0.0, "no gaze is held on the edge");
    }

    #[test]
    fn npc_place_beside_runner_resets_y_and_walk() {
        let Some(mut c) = any_switchback() else { return };
        let started = run(&mut c, CAP_SECONDS, |g| g.npcs[0].walk.is_some());
        assert!(started.is_some(), "the bot reached the flight");
        c.g.npcs[0].y = 123.0;
        c.g.npcs[0].edge_clock = 2.0;
        c.g.npc_place_beside_runner();
        let n = &c.g.npcs[0];
        assert!(n.walk.is_none() && n.hop.is_none(), "a rebuild drops the edge walk");
        assert_eq!(n.edge_clock, 0.0);
        assert_eq!(n.y, c.g.p.y(), "the bot takes the runner's height");
        assert_eq!(n.level, c.g.runner_level());
    }

    #[test]
    fn a_spawned_bot_stands_on_its_levels_floor() {
        let (mut g, _) = super::tests::run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        g.npcs.clear();
        let (e, d) = ((g.p.pos.0, 1.6, g.p.pos.1), (0.0, 1.0, 0.0));
        g.npc_spawn(1, 70, e, d);
        let n = &g.npcs[0];
        assert_eq!(n.y, floor_y_of(n.level));
        assert!(n.walk.is_none() && n.hop.is_none() && n.edge_clock == 0.0);
    }

    #[test]
    fn a_summon_count_is_capped() {
        let (content, catalog) = (crate::content::Content::load(), crate::world::catalog::Catalog::load());
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        let spot = g.p.pos;
        let made = g.npc_spawn_at(2_000_000_000, 70, spot, g.runner_level());
        assert!(made <= crate::net::hosted::MAX_SUMMON_NPCS as usize, "a huge count makes at most one summon's worth");
        assert_eq!(g.npcs.len(), made);
    }

    #[test]
    fn a_bot_on_a_non_resident_level_is_set_down_on_the_runners_level() {
        let (content, catalog) = (crate::content::Content::load(), crate::world::catalog::Catalog::load());
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        g.hunters.clear();
        let spot = g.p.pos;
        let here = g.runner_level();
        g.npc_spawn_at(1, 70, spot, here + 40);
        assert_eq!(g.npcs[0].level, here + 40);
        g.update_npcs_with(1.0 / 60.0, Tuning::get());
        assert_eq!(g.npcs[0].level, here, "the bot no longer waits on a level that is gone");
        assert!(g.npcs[0].walk.is_none());
    }

    #[test]
    fn a_change_of_storey_drops_the_visited_errands() {
        let (content, catalog) = (crate::content::Content::load(), crate::world::catalog::Catalog::load());
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        let spot = g.p.pos;
        g.npc_spawn_at(1, 70, spot, g.runner_level());
        let n = &mut g.npcs[0];
        n.visited = vec![Errand::Locker(2)];
        n.errand = Some(Errand::Locker(2));
        let same = n.level;
        set_level(n, same);
        assert_eq!(n.visited.len(), 1, "the same storey keeps its visits");
        set_level(n, same - 1);
        assert!(n.visited.is_empty() && n.errand.is_none(), "another storey starts with none");
    }
}

#[cfg(test)]
mod coarse_tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    const CHUNK: f32 = 45.0;

    fn bot(at: (f32, f32), g: &mut Game) -> usize {
        let runner = g.p.pos;
        g.npc_spawn_at(1, 50, runner, 0);
        let i = g.npcs.len() - 1;
        g.npcs[i].at = at;
        i
    }

    #[test]
    fn the_embedded_coarse_speed_is_the_walking_speed_and_a_missing_key_keeps_it() {
        assert!((Tuning::get().coarse.speed - PLAYER_WALK_SPEED).abs() < 1e-4);
        assert_eq!(CoarseTuning::parse(&Value::Null), CoarseTuning::default());
        let v: Value = serde_json::from_str(r#"{"speed": 9.0}"#).unwrap();
        assert!((CoarseTuning::parse(&v).speed - 9.0).abs() < 1e-4);
    }

    #[test]
    fn a_bot_outside_the_window_gets_closer_every_second() {
        let runner = (67.5, 67.5);
        let mut at = (-300.0, 20.0);
        let mut last = pdist(at, runner);
        for _ in 0..60 {
            at = coarse_step(at, runner, CHUNK, 4.0, 1.0);
            let d = pdist(at, runner);
            assert!(d < last, "closer each second: {d} vs {last}");
            last = d;
        }
    }

    #[test]
    fn the_coarse_step_covers_the_speed_and_goes_chunk_to_chunk() {
        let runner = (67.5, 67.5);
        let at = (-100.0, 67.5);
        let next = coarse_step(at, runner, CHUNK, 4.0, 0.5);
        assert!((pdist(at, next) - 2.0).abs() < 1e-3);
        // the first stop is the centre of the next chunk along, not the runner
        assert_eq!(coarse_waypoint(at, runner, CHUNK), (-67.5, 67.5));
        // in the runner's own chunk the target is the runner
        assert_eq!(coarse_waypoint((50.0, 50.0), runner, CHUNK), runner);
    }

    #[test]
    fn a_bot_whose_position_comes_onto_the_floor_is_placed_on_a_dry_walkable_cell() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        let i = bot((-0.5, -0.5), &mut g);
        let runner = g.p.pos;
        let home = g.fac.clone();
        let mut n = g.npcs[i].clone();
        // a step that lands inside the floor ends on a dry open cell
        n.at = (home.cols as f32 - 0.2, runner.1);
        let c = CoarseTuning { speed: 100.0 };
        coarse_travel(&mut n, &home, runner, CHUNK, &c, 1.0);
        assert!(on_floor(&home, n.at));
        let cell = Cell::new(n.at.0.floor() as i32, n.at.1.floor() as i32);
        assert!(!home.is_water(cell), "dry");
        assert!(home.nearest_dry_open(n.at.0, n.at.1, 0.5).is_some(), "open");
    }

    #[test]
    fn a_bot_off_the_floor_stays_off_it_until_it_arrives() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        let i = bot((-500.0, -500.0), &mut g);
        let home = g.fac.clone();
        let mut n = g.npcs[i].clone();
        coarse_travel(&mut n, &home, g.p.pos, CHUNK, &CoarseTuning::default(), 1.0);
        assert!(!on_floor(&home, n.at));
        assert!(!n.walking && n.path.is_empty());
    }

    #[test]
    fn a_bot_inside_the_window_is_untouched_by_the_coarse_path() {
        let content = Content::load();
        let catalog = Catalog::load();
        let mut g = Game::new_sim(Difficulty::Standard, 4242, &content, &catalog);
        let i = bot(g.p.pos, &mut g);
        assert!(on_floor(&g.fac, g.npcs[i].at));
        // a finite floor never runs the coarse branch, and the rejoin leaves an on-floor open bot where it stands
        let before = g.npcs[i].at;
        g.update_npcs(0.1);
        assert!(on_floor(&g.fac, g.npcs[i].at));
        assert!(pdist(before, g.npcs[i].at) < 1.0);
    }
}

#[cfg(test)]
#[path = "rail_npc_tests.rs"]
mod rail_npc_tests;

#[cfg(test)]
mod life_tests {
    use super::*;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::world::catalog::Catalog;

    fn game() -> Game {
        let content = Content::load();
        let catalog = Catalog::load();
        Game::new_sim(Difficulty::Standard, 4242, &content, &catalog)
    }

    /// The embedded data/npcs.json with these intents and one more edit, parsed as the bots read it.
    fn tuned(intents: &[&str], edit: impl Fn(&mut Value)) -> Tuning {
        let (_, text) = crate::content::embedded::data_under("npcs.json").next().expect("data/npcs.json is embedded");
        let mut v: Value = serde_json::from_str(text).expect("data/npcs.json parses");
        v["intents"] = Value::Array(intents.iter().map(|s| Value::from(*s)).collect());
        edit(&mut v);
        Tuning::parse(&v).expect("data/npcs.json still has every key")
    }

    fn with_bot() -> Game {
        let mut g = game();
        let p = g.p.pos;
        assert_eq!(g.npc_spawn(1, 70, (p.0, 1.6, p.1), (0.0, 1.0, 0.0)), 1);
        g
    }

    fn run(g: &mut Game, t: &Tuning, seconds: f32) {
        for _ in 0..(seconds * 60.0) as usize {
            g.update_npcs_with(1.0 / 60.0, t);
        }
    }

    #[test]
    fn the_standard_data_lists_life_before_follow() {
        let t = Tuning::get();
        let at = |name: &str| t.intents.iter().position(|i| i == name).unwrap();
        assert!(at("goal") < at("life") && at("life") < at("follow"), "{:?}", t.intents);
        assert!(!t.life.poi.is_empty(), "the data lists spots");
        assert!(t.life.range >= t.follow_radius, "the wander range covers the follow range");
        for p in &t.life.poi {
            assert!(p.weight > 0.0 && !p.acts.is_empty(), "{}", p.kind);
            let known = p.kind == "here" || crate::hunters::activities::PoiKind::from_id(&p.kind).is_some();
            assert!(known, "{} is a hunter point-of-interest kind", p.kind);
        }
        for (_, (lo, hi)) in &t.life.durations {
            assert!(lo <= hi && *lo > 0.0);
        }
        for a in LifeAct::ALL {
            assert_eq!(LifeAct::from_name(a.name()), Some(a));
        }
    }

    #[test]
    fn every_life_emote_exists_in_the_emote_data() {
        let set = crate::render::anim::emote_pose::set().expect("data/player/emotes.json parses");
        assert!(!Tuning::get().life.emotes.is_empty());
        for (id, w) in &Tuning::get().life.emotes {
            assert!(*w > 0.0 && set.find(id).is_some(), "{id}");
        }
    }

    #[test]
    fn an_idle_bot_picks_activities_over_time() {
        let mut g = with_bot();
        let t = tuned(&["life", "follow"], |_| {});
        run(&mut g, &t, 120.0);
        let picked = &g.npcs[0].life.picked;
        assert!(picked.len() >= 4, "{picked:?}");
        let mut kinds = picked.clone();
        kinds.dedup();
        assert!(kinds.len() >= 2, "more than one kind of activity: {picked:?}");
    }

    #[test]
    fn a_bot_never_strays_past_its_range() {
        let mut g = with_bot();
        let t = tuned(&["life", "follow"], |_| {});
        let mut far = 0.0f32;
        for _ in 0..(120 * 60) {
            g.update_npcs_with(1.0 / 60.0, &t);
            far = far.max(pdist(g.npcs[0].at, g.p.pos));
        }
        // a step can end a body-width past the line before the bot turns back
        assert!(far <= t.life.range + 2.0, "went {far} m from the runner (range {})", t.life.range);
    }

    #[test]
    fn the_data_decides_what_a_bot_does() {
        let mut g = with_bot();
        let t = tuned(&["life"], |v| {
            v["life"]["poi"] = serde_json::json!({ "here": { "weight": 1.0, "acts": { "idle": 1 } } });
        });
        let start = g.npcs[0].at;
        run(&mut g, &t, 30.0);
        let n = &g.npcs[0];
        assert!(!n.life.picked.is_empty());
        assert!(n.life.picked.iter().all(|a| *a == LifeAct::Idle), "{:?}", n.life.picked);
        assert_eq!(n.at, start, "idling in place moves nothing");
    }

    #[test]
    fn a_bot_plays_an_emote_from_the_emote_data() {
        let mut g = with_bot();
        let t = tuned(&["life"], |v| {
            v["life"]["poi"] = serde_json::json!({ "here": { "weight": 1.0, "acts": { "emote": 1 } } });
            v["life"]["emote_gap"] = Value::from(0.0);
            v["life"]["pause"] = serde_json::json!([0.2, 0.4]);
        });
        let set = crate::render::anim::emote_pose::set().expect("emotes parse");
        let mut seen = false;
        for _ in 0..(30 * 60) {
            g.update_npcs_with(1.0 / 60.0, &t);
            if let Some(id) = &g.npcs[0].life.emote {
                assert!(set.find(id).is_some(), "{id}");
                assert!(!g.npcs[0].walking, "an emote is played standing");
                seen = true;
            }
        }
        assert!(seen && g.npcs[0].life.emote_seq >= 2, "emotes started: {}", g.npcs[0].life.emote_seq);
    }

    #[test]
    fn the_emote_gap_spaces_emotes() {
        let mut g = with_bot();
        let t = tuned(&["life"], |v| {
            v["life"]["poi"] = serde_json::json!({ "here": { "weight": 1.0, "acts": { "emote": 1, "idle": 1 } } });
            v["life"]["emote_gap"] = Value::from(1000.0);
        });
        run(&mut g, &t, 60.0);
        assert!(g.npcs[0].life.emote_seq <= 1, "one emote at most inside the gap");
    }

    #[test]
    fn crouching_ends_with_the_activity_and_with_an_interrupt() {
        let mut g = with_bot();
        let t = tuned(&["life"], |v| {
            v["life"]["poi"] = serde_json::json!({ "here": { "weight": 1.0, "acts": { "crouch": 1 } } });
        });
        let mut crouched = false;
        for _ in 0..(20 * 60) {
            g.update_npcs_with(1.0 / 60.0, &t);
            crouched |= g.npcs[0].life.crouching;
        }
        assert!(crouched, "the bot crouched");
        g.npcs[0].life.crouching = true;
        g.npcs[0].life.interrupt(1.0);
        assert!(!g.npcs[0].life.crouching && g.npcs[0].life.emote.is_none());
    }

    #[test]
    fn a_bot_past_its_range_is_walked_back_by_follow() {
        let mut g = with_bot();
        let t = tuned(&["life", "follow"], |_| {});
        let runner = g.p.pos;
        let s = Cell::new(runner.0.floor() as i32, runner.1.floor() as i32);
        let mut reach: Vec<(Cell, i32)> = g.fac.distance_map(s).into_iter().collect();
        reach.sort_by_key(|(c, _)| (c.x, c.y));
        let far = reach
            .into_iter()
            .filter(|(c, d)| *d >= 12 && *d <= 16 && g.fac.body_fits(c.center().0, c.center().1, NAV_RADIUS))
            .map(|(c, _)| c.center())
            .next()
            .expect("an open cell 12 to 16 steps away");
        g.npcs[0].at = far;
        let before = pdist(far, runner);
        run(&mut g, &t, 40.0);
        assert!(g.npcs[0].life.returning || pdist(g.npcs[0].at, runner) < before, "the bot closed in");
        assert!(pdist(g.npcs[0].at, runner) <= t.life.range + 2.0);
    }

    #[test]
    fn the_standard_data_gives_returning_its_own_distance_inside_the_follow_range() {
        let t = Tuning::get();
        assert!(t.life.return_distance > 0.0 && t.life.return_distance <= t.life.range);
        assert!(t.life.return_timeout > 0.0);
    }

    #[test]
    fn a_bot_in_the_widest_ring_slot_leaves_returning_within_the_timeout() {
        let mut g = with_bot();
        let t = tuned(&["life", "follow"], |v| {
            v["life"]["range"] = Value::from(3.0);
            v["life"]["return_distance"] = Value::from(2.0);
            v["life"]["return_timeout"] = Value::from(4.0);
        });
        g.npcs[0].slot = 200;
        let runner = g.p.pos;
        g.npcs[0].life.returning = true;
        g.npcs[0].at = (runner.0 + 7.0, runner.1);
        let mut left = None;
        for i in 0..(30 * 60) {
            g.update_npcs_with(1.0 / 60.0, &t);
            if !g.npcs[0].life.returning {
                left = Some(i as f32 / 60.0);
                break;
            }
        }
        let left = left.expect("returning ended");
        assert!(left <= t.life.return_timeout + 0.1, "left after {left}");
    }

    #[test]
    fn returning_ends_inside_its_own_distance() {
        let mut g = with_bot();
        let t = tuned(&["life", "follow"], |v| {
            v["life"]["return_distance"] = Value::from(1.5);
            v["life"]["return_timeout"] = Value::from(1000.0);
        });
        let runner = g.p.pos;
        g.npcs[0].life.returning = true;
        g.npcs[0].at = (runner.0 + 1.0, runner.1);
        g.update_npcs_with(1.0 / 60.0, &t);
        assert!(!g.npcs[0].life.returning, "a bot inside its return distance is back");
    }

    /// A plain follow that cannot close in (here no follow intent at all, so nothing walks the bot) is dropped after
    /// `life.follow_only_timeout`; a follow that keeps the timeout far off still holds.
    #[test]
    fn follow_only_that_cannot_close_in_is_dropped_after_its_timeout() {
        let t = tuned(&["life"], |v| v["life"]["follow_only_timeout"] = Value::from(2.0));
        let mut g = with_bot();
        let runner = g.p.pos;
        let far = (runner.0 + t.life.range + 6.0, runner.1);
        g.npcs[0].at = far;
        g.npcs[0].life.follow_only = true;
        g.npcs[0].life.follow_best = t.life.range + 6.0;
        let mut dropped = None;
        for i in 0..(10 * 60) {
            g.update_npcs_with(1.0 / 60.0, &t);
            if !g.npcs[0].life.follow_only {
                dropped = Some(i as f32 / 60.0);
                break;
            }
        }
        let dropped = dropped.expect("follow_only was dropped");
        assert!(dropped <= t.life.follow_only_timeout + 0.1, "dropped after {dropped} s");
        assert!(pdist(g.npcs[0].at, runner) > t.life.range, "the bot was never walked in: the timeout dropped it");

        let held = tuned(&["life"], |v| v["life"]["follow_only_timeout"] = Value::from(1000.0));
        let mut g = with_bot();
        g.npcs[0].at = far;
        g.npcs[0].life.follow_only = true;
        g.npcs[0].life.follow_best = t.life.range + 6.0;
        run(&mut g, &held, 5.0);
        assert!(g.npcs[0].life.follow_only, "inside its timeout a plain follow holds");
    }

    /// Guest rule: a guest draws the host's bots (net/npc_sync.rs), so its own update moves no bot and changes no life
    /// state, however far the bot stands and however long it has been returning.
    #[test]
    fn a_guest_never_steps_a_bot_or_its_life() {
        let t = Tuning::get();
        let mut g = with_bot();
        g.remote_hunters = true;
        let runner = g.p.pos;
        g.npcs[0].at = (runner.0 + t.life.range + 4.0, runner.1);
        g.npcs[0].life.returning = true;
        g.npcs[0].life.return_clock = t.life.return_timeout - 0.01;
        let before = (g.npcs[0].at, g.npcs[0].life.returning, g.npcs[0].life.return_clock, g.npcs[0].life.follow_only, g.npcs[0].path.len());
        run(&mut g, t, 2.0);
        let after = (g.npcs[0].at, g.npcs[0].life.returning, g.npcs[0].life.return_clock, g.npcs[0].life.follow_only, g.npcs[0].path.len());
        assert_eq!(before, after, "a guest left the bot as the host last set it");
        // the same game as the host steps it: the timeout ends returning
        g.remote_hunters = false;
        run(&mut g, t, 0.5);
        let host = (g.npcs[0].at, g.npcs[0].life.returning, g.npcs[0].life.return_clock, g.npcs[0].life.follow_only, g.npcs[0].path.len());
        assert_ne!(before, host, "the host steps the bot");
    }

    // ---- liveliness (data/npcs.json `lively`) ----

    /// Steps the first bot for `seconds` and returns the most left and most right its head turned (radians).
    fn peak(g: &mut Game, t: &Tuning, seconds: f32) -> (f32, f32) {
        let (mut hi, mut lo) = (0.0f32, 0.0f32);
        for _ in 0..(seconds * 60.0) as usize {
            g.update_npcs_with(1.0 / 60.0, t);
            hi = hi.max(g.npcs[0].life.head_yaw);
            lo = lo.min(g.npcs[0].life.head_yaw);
        }
        (hi, lo)
    }

    #[test]
    fn the_lively_data_lists_personalities_and_sane_limits() {
        let lv = &Tuning::get().lively;
        let names: Vec<&str> = lv.personalities.iter().map(|p| p.name.as_str()).collect();
        for want in ["cautious", "bold", "curious"] {
            assert!(names.contains(&want), "{want} is in the data: {names:?}");
        }
        assert!(lv.personalities.iter().all(|p| p.weight > 0.0 && !p.acts.is_empty()));
        assert!(lv.max_yaw > 0.0 && lv.max_yaw < PI && lv.max_pitch > 0.0, "the head turns, but never right round");
        assert!(lv.ring_scale.0 > 0.0 && lv.ring_scale.0 < lv.ring_scale.1, "the follow distance varies");
        assert!(lv.glance_range > 0.0 && lv.runner_range > 0.0 && lv.fidget_yaw > 0.0 && lv.stop_hold.1 > 0.0);
        let set = crate::render::anim::emote_pose::set().expect("emotes parse");
        assert!(!lv.replies.is_empty());
        for (id, w) in &lv.replies {
            assert!(*w > 0.0 && set.find(id).is_some(), "{id} is an emote");
        }
    }

    #[test]
    fn an_idle_bot_shows_varied_activities() {
        let mut g = with_bot();
        let t = tuned(&["life", "follow"], |_| {});
        let (hi, lo) = peak(&mut g, &t, 150.0);
        let n = &g.npcs[0];
        let mut kinds = n.life.picked.clone();
        kinds.sort_by_key(|a| a.name());
        kinds.dedup();
        assert!(kinds.len() >= 2, "more than one activity: {:?}", n.life.picked);
        assert!(n.life.runner_looks + n.life.fidgets + n.life.glances >= 3, "looks {} fidgets {}", n.life.runner_looks, n.life.fidgets);
        assert!(hi > 0.05 || lo < -0.05, "the head moved: {hi} {lo}");
        assert!(hi <= t.lively.max_yaw + 1e-3 && lo >= -t.lively.max_yaw - 1e-3, "inside the head limit");
    }

    #[test]
    fn personalities_change_the_mix_of_activities() {
        let lv = &Tuning::get().lively;
        let by = |name: &str| lv.personalities.iter().find(|p| p.name == name).expect("personality in the data");
        let base = vec![
            (LifeAct::LookAround, 1.0),
            (LifeAct::Crouch, 1.0),
            (LifeAct::SprintBurst, 1.0),
            (LifeAct::Emote, 1.0),
            (LifeAct::Idle, 1.0),
        ];
        let share = |p: &Personality, act: LifeAct| -> f32 {
            let acts = scale_acts(&base, p);
            let hits = (0..1000).filter(|i| weighted(&acts, *i as f32 / 1000.0) == Some(act)).count();
            hits as f32 / 1000.0
        };
        let (cautious, bold) = (by("cautious"), by("bold"));
        assert!(share(cautious, LifeAct::Crouch) > share(bold, LifeAct::Crouch) + 0.05, "the cautious crouch more");
        assert!(share(bold, LifeAct::SprintBurst) > share(cautious, LifeAct::SprintBurst) + 0.05, "the bold sprint more");
        assert!(share(bold, LifeAct::Emote) > share(cautious, LifeAct::Emote), "the bold emote more");
        // a personality never adds an act the spot does not offer
        let only = scale_acts(&[(LifeAct::Idle, 1.0)], bold);
        assert_eq!(only.len(), 1);
        assert_eq!(only[0].0, LifeAct::Idle);
    }

    #[test]
    fn a_bots_personality_comes_from_its_name_and_slot() {
        let lv = &Tuning::get().lively;
        let drawn: Vec<usize> = (1..=60u32).map(|slot| pick_personality(lv, &guest_name_from_seed(slot as u64), slot)).collect();
        for i in 0..lv.personalities.len() {
            assert!(drawn.contains(&i), "personality {i} is drawn by some bot: {drawn:?}");
        }
        assert_eq!(pick_personality(lv, "R-00007", 3), pick_personality(lv, "R-00007", 3), "a rebuild keeps it");
        let mut g = with_bot();
        let first = personality_of(&mut g.npcs[0], lv).name.clone();
        assert_eq!(personality_of(&mut g.npcs[0], lv).name, first, "kept in the life state");
        let neutral = personality_of(&mut g.npcs[0], &Lively::default()).name.clone();
        assert_eq!(neutral, "neutral", "data with none listed gives the neutral one");
    }

    #[test]
    fn follow_distances_vary_but_stay_inside_the_follow_range() {
        let t = Tuning::get();
        let mut g = with_bot();
        let mut slot_one: Vec<f32> = vec![];
        for slot in 1..=40u32 {
            for p in 0..t.lively.personalities.len() {
                for var in [t.lively.ring_scale.0, 1.0, t.lively.ring_scale.1] {
                    let n = &mut g.npcs[0];
                    n.slot = slot;
                    n.life.personality = Some(p);
                    n.life.ring_var = var;
                    let ring = follow_ring(n, t);
                    let range = follow_range(n, t);
                    assert!(ring <= range + 1e-4, "slot {slot} personality {p} var {var}: ring {ring} past range {range}");
                    assert!(ring >= t.lively.min_ring.min(range) - 1e-4, "ring {ring} too close");
                    if slot == 1 {
                        slot_one.push(ring);
                    }
                }
            }
        }
        slot_one.sort_by(f32::total_cmp);
        assert!(slot_one.last().unwrap() - slot_one.first().unwrap() > 0.5, "slot 1 aims for different distances: {slot_one:?}");
    }

    #[test]
    fn a_following_bot_never_strays_past_its_follow_range() {
        let t = tuned(&["follow"], |_| {});
        let mut g = with_bot();
        // put the bot at the edge of its life range, then let it walk back and settle
        let runner = g.p.pos;
        let far = g
            .fac
            .floor_cells()
            .into_iter()
            .map(|c| c.center())
            .find(|c| {
                let d = pdist(*c, runner);
                d > 7.0 && d < 9.0 && g.fac.body_fits(c.0, c.1, NAV_RADIUS)
            });
        let Some(far) = far else { return };
        g.npcs[0].at = far;
        run(&mut g, &t, 40.0);
        let range = follow_range(&g.npcs[0], &t);
        let mut worst = 0.0f32;
        for _ in 0..(60 * 20) {
            g.update_npcs_with(1.0 / 60.0, &t);
            worst = worst.max(pdist(g.npcs[0].at, g.p.pos));
        }
        assert!(worst <= range + 0.5, "settled bot went {worst} m from the runner (range {range})");
    }

    #[test]
    fn a_bots_head_turns_toward_a_sound() {
        let t = tuned(&["follow"], |_| {});
        for (dx, left) in [(-6.0f32, true), (6.0, false)] {
            let mut g = with_bot();
            g.npcs[0].facing = 0.0;
            g.npcs[0].turn_v = 0.0;
            let at = g.npcs[0].at;
            g.npcs[0].heard = Some(((at.0 + dx, at.1), 5.0));
            let (hi, lo) = peak(&mut g, &t, 1.0);
            if left {
                assert!(hi > 0.5, "a sound to the left turns the head left: {hi}");
            } else {
                assert!(lo < -0.5, "a sound to the right turns the head right: {lo}");
            }
            assert_eq!(g.npcs[0].life.gaze.map(|g| g.1), Some(Gaze::Sound));
        }
    }

    #[test]
    fn the_data_decides_how_lively_a_bot_is() {
        let lively = tuned(&["follow"], |_| {});
        let still = tuned(&["follow"], |v| {
            v["lively"]["runner_look"]["range"] = Value::from(0.0);
            v["lively"]["fidget"]["yaw"] = Value::from(0.0);
        });
        let mut a = with_bot();
        run(&mut a, &lively, 60.0);
        let mut b = with_bot();
        run(&mut b, &still, 60.0);
        assert!(a.npcs[0].life.runner_looks > 0, "the bot looks at the runner");
        assert_eq!(b.npcs[0].life.runner_looks, 0, "range 0 turns it off");
        assert_eq!(b.npcs[0].life.fidgets, 0, "yaw 0 turns the fidget off");
        // no head limit at all is the old, stiff bot
        let stiff = tuned(&["follow"], |v| {
            v["lively"]["head"]["max_yaw"] = Value::from(0.0);
            v["lively"]["head"]["max_pitch"] = Value::from(0.0);
        });
        let mut c = with_bot();
        let (hi, lo) = peak(&mut c, &stiff, 30.0);
        assert!(hi == 0.0 && lo == 0.0, "no head limit, no head look");
    }

    #[test]
    fn a_bot_answers_the_runners_emote() {
        let t = tuned(&["life", "follow"], |v| {
            v["lively"]["react"]["chance"] = Value::from(1.0);
            v["lively"]["react"]["delay"] = serde_json::json!([0.1, 0.2]);
            v["lively"]["react"]["cool"] = Value::from(0.0);
            if let Some(m) = v["lively"]["personalities"].as_object_mut() {
                for p in m.values_mut() {
                    p["emote_scale"] = Value::from(5.0);
                }
            }
        });
        let mut g = with_bot();
        let set = crate::render::anim::emote_pose::set().expect("emotes parse");
        assert!(g.emote.play(set, "wave", g.time));
        run(&mut g, &t, 8.0);
        let n = &g.npcs[0];
        assert!(n.life.replies >= 1 && n.life.emote_seq >= 1, "the bot answered: {} replies", n.life.replies);
        // with no emote from the runner it stays quiet
        let mut quiet = with_bot();
        run(&mut quiet, &t, 8.0);
        assert_eq!(quiet.npcs[0].life.replies, 0);
    }

    #[test]
    fn a_walking_bot_stops_now_and_then_to_chat() {
        let lv = Lively::parse(&serde_json::json!({
            "walk_stop": { "gap": [0.5, 0.5], "hold": [1.0, 1.0], "chat_range": 8.0 }
        }));
        let mut g = with_bot();
        let n = &mut g.npcs[0];
        let mut stops = 0;
        for _ in 0..200 {
            n.life.hold = 0.0;
            if stop_due(n, &lv, 0.1) {
                stops += 1;
                assert!(n.life.hold >= 1.0 - 1e-4, "a stop holds the bot for its hold seconds");
            }
        }
        assert!(stops >= 5, "stops: {stops}");
        let none = Lively::parse(&serde_json::json!({ "walk_stop": { "hold": [0.0, 0.0] } }));
        assert!(!stop_due(n, &none, 100.0), "a zero hold turns chatter stops off");
    }

    #[test]
    fn a_cautious_bot_checks_a_door_crouched_and_a_bold_one_does_not() {
        let mk = |chance: f64| Lively::parse(&serde_json::json!({ "personalities": { "p": { "peek_chance": chance } } }));
        let l = Tuning::get().life.clone();
        let mut g = with_bot();
        let n = &mut g.npcs[0];
        n.life.act = Some(LifeAct::CheckDoor);
        life_begin(n, &l, &mk(1.0), &mut Rng::new("peek"));
        assert!(n.life.crouching, "peek chance 1 crouches");
        n.life.interrupt(1.0);
        n.life.act = Some(LifeAct::CheckDoor);
        life_begin(n, &l, &mk(0.0), &mut Rng::new("peek"));
        assert!(!n.life.crouching, "peek chance 0 stands");
    }

    #[test]
    fn a_guest_never_moves_a_bots_head() {
        let t = Tuning::get();
        let mut g = with_bot();
        g.remote_hunters = true;
        let at = g.npcs[0].at;
        g.npcs[0].heard = Some(((at.0 - 6.0, at.1), 5.0));
        run(&mut g, t, 2.0);
        assert_eq!(g.npcs[0].life.head_yaw, 0.0, "a guest draws the host's look");
        assert_eq!(g.npcs[0].life.gaze, None);
    }
}
