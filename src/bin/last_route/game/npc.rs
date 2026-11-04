//! Player bots (NPC teammates), phase one: the admin spawns bots in an offline room and they follow the runner.
//! This is the offline core of Godot's `game/player/npc_teammates.gd` (spawn, the follow rule, the
//! walk and the turn spring) and of the host's spawn spot rule (`infinite_route_test.gd` 6769-6779).
//! No server is involved: the bots live on the `Game` for the run and nothing is sent anywhere.
//!
//! An NPC here is a bot standing in for a player, not an in-world character. It has the runner's body
//! (`PLAYER_RADIUS`) and speed (`PLAYER_WALK_SPEED`) and obeys the runner's rules: it does not open a door by
//! walking up to it (hunters do); it stops at a shut door, faces it, waits a short seeded think delay and presses
//! interact through the same `press_door` path as the runner, then walks on once the opening fits its body.
//!
//! Deferred (see docs/NPC_TEAMMATES in the Godot tree): per-NPC commands, catching,
//! chat, storey crossing, co-op packets, name tags, saves, and the mannequin body.

use super::pdist;
use super::Game;
use crate::hunters::brain::State;
use crate::world::rng::Rng;
use crate::config::{INTERACT_REACH, PLAYER_RADIUS, PLAYER_SPRINT_SPEED, PLAYER_WALK_SPEED};
use crate::render::textures::rng::GodotRng;
use crate::world::route::run_seed_of_text;
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
/// Godot `npc_teammates.gd` 27: seconds between path plans.
pub const REPATH: f32 = 0.5;
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

/// The bots' thresholds and task list, from the `threat`, `follow`, `door` and `tasks` keys of data/npcs.json. Every
/// number the bot uses lives there; the code keeps only the behaviour.
#[derive(Clone, Debug, PartialEq)]
pub struct Tuning {
    /// Godot `THREAT_RANGE` (npc_teammates.gd 25): metres a chasing hunter is noticed at, before the smart scale.
    pub threat_range: f32,
    /// Godot `.5` in `threat_range * (.5 + smart / 100)` (npc_teammates.gd 307).
    pub smart_base: f32,
    /// Godot 312: how far ahead of itself a fleeing NPC aims, in metres.
    pub flee_step: f32,
    /// Godot 324-326: a fleeing NPC runs at the sprint speed (true) rather than walking.
    pub flee_running: bool,
    /// Godot 311: an away vector this short points the bot right (Godot `Vector2.RIGHT`).
    pub min_away: f32,
    /// Godot `FOLLOW_RADIUS` (npc_teammates.gd 24): metres from the runner before the NPC walks to its slot again.
    pub follow_radius: f32,
    /// Seconds a bot waits at a shut door before it presses interact: this base plus up to `door_think_jitter`, seeded.
    pub door_think: f32,
    /// The seeded spread added to `door_think`.
    pub door_think_jitter: f32,
    /// The tasks a bot does. Only "door" (press shut doors for the runner) is built; relays and lockers are not.
    pub tasks: Vec<String>,
}

impl Tuning {
    /// Reads the tuning from a parsed data/npcs.json. None when a key is missing or has the wrong type.
    pub fn parse(v: &Value) -> Option<Tuning> {
        let num = |node: &Value, key: &str| node[key].as_f64().map(|x| x as f32);
        let threat = &v["threat"];
        let door = &v["door"];
        Some(Tuning {
            threat_range: num(threat, "range")?,
            smart_base: num(threat, "smart_base")?,
            flee_step: num(threat, "flee_step")?,
            flee_running: threat["flee_running"].as_bool()?,
            min_away: num(threat, "min_away")?,
            follow_radius: num(&v["follow"], "radius")?,
            door_think: num(door, "think")?,
            door_think_jitter: num(door, "think_jitter")?,
            tasks: v["tasks"]
                .as_array()?
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect(),
        })
    }

    /// The tuning of the embedded data/npcs.json, read once.
    pub fn get() -> &'static Tuning {
        static TUNING: OnceLock<Tuning> = OnceLock::new();
        TUNING.get_or_init(|| {
            // Read through the content loader (content/npcs.rs), so the embedded default is the one place it lives
            let v = crate::content::npcs::embedded_json();
            Tuning::parse(&v).expect("data/npcs.json has the threat, follow, door and tasks keys")
        })
    }

    /// True when the bots do this task (a name in `tasks`).
    pub fn does(&self, task: &str) -> bool {
        self.tasks.iter().any(|t| t == task)
    }
}

/// A hunter as a bot sees it: its floor position and whether it is chasing.
type Seen = ((f32, f32), bool);

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
    /// The infinite level the NPC stands on (0 on a finite floor).
    pub level: i32,
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
}

impl Npc {
    /// Moves the NPC by `d` after the window recentres. The path is in the old frame, so it is dropped and
    /// replanned at once.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.at = (self.at.0 + d.0, self.at.1 + d.1);
        self.path.clear();
        self.path_age = REPATH;
        self.door = None;
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
            t.door_think + t.door_think_jitter * rng.next_f32() - dt
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
        if !fac.body_fits(x, y, NAV_RADIUS) {
            return Sweep::Blocked;
        }
    }
    Sweep::Clear
}

/// Plans a route from the bot's cell to the open cell nearest `goal`, and drops the cell the bot stands in.
fn plan(n: &mut Npc, fac: &Facility, goal: (f32, f32)) {
    n.path_age = 0.0;
    let from = Cell::new(n.at.0.floor() as i32, n.at.1.floor() as i32);
    let to = fac.nearest_open(goal.0, goal.1, NAV_RADIUS).unwrap_or(goal);
    let to = Cell::new(to.0.floor() as i32, to.1.floor() as i32);
    n.path = fac
        .find_path(from, to)
        .into_iter()
        .map(|c| (c.x, c.y))
        .collect();
    if !n.path.is_empty() {
        n.path.remove(0);
    }
}

/// Godot `_walk_toward`: one step toward `goal` along the planned route (`path`, replanned every `REPATH` seconds).
/// The bot aims at the furthest of the next `LOOKAHEAD_CELLS` cells it reaches in a straight line its body fits on, so
/// it follows the route round a corner and never cuts one. A shut door in the way stops it (`wait_at_door`). A step
/// that runs into a wall is not slid along: the route is replanned at once and the bot waits for its next step.
fn walk_toward(n: &mut Npc, fac: &Facility, goal: (f32, f32), running: bool, t: &Tuning, dt: f32, presses: &mut Vec<Cell>) {
    let here = n.at;
    n.path_age += dt;
    if n.path_age >= REPATH {
        plan(n, fac, goal);
    }
    if n.path.is_empty() {
        n.walking = false;
        return;
    }
    // a bot already inside a door's slab is let out, and one inside a wall is not checked for walls (as before)
    let in_door = fac.door_in_way(here.0, here.1, NAV_RADIUS).is_some();
    let fits = fac.body_fits(here.0, here.1, NAV_RADIUS);
    if fits && !in_door {
        let reach = n.path.len().min(LOOKAHEAD_CELLS);
        let ahead = (1..reach).rev().find(|&k| {
            let c = (n.path[k].0 as f32 + 0.5, n.path[k].1 as f32 + 0.5);
            matches!(sweep(fac, here, c), Sweep::Clear)
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
            // a wall in the way: no slide along it, a fresh route, and the next step waits for it
            Sweep::Blocked if fits => {
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
    turn(n, (-heading.0).atan2(-heading.1), dt);
    n.stride += moved;
    n.walking = true;
}

/// Godot `_think` for phase one, same-level only: follow the runner to the NPC's slot when it has
/// drifted past `follow_radius + ring - SLOT_RADIUS` (or is mid-walk), else stand still.
fn follow(n: &mut Npc, fac: &Facility, leader: (f32, f32), t: &Tuning, dt: f32, presses: &mut Vec<Cell>) {
    let ring = slot_radius(n.slot);
    if pdist(n.at, leader) > t.follow_radius + ring - SLOT_RADIUS || !n.path.is_empty() {
        let goal = slot_point(leader, n.slot, ring);
        walk_toward(n, fac, goal, false, t, dt, presses);
    } else {
        n.walking = false;
    }
}

/// Godot `_think`'s flee (npc_teammates.gd 305-315): the nearest hunter of any state; when it is chasing and inside
/// the smart-scaled threat range the bot runs straight away from it, `flee_step` ahead. True when it ran.
fn flee(n: &mut Npc, fac: &Facility, seen: &[Seen], t: &Tuning, dt: f32, presses: &mut Vec<Cell>) -> bool {
    let range = t.threat_range * (t.smart_base + n.smart as f32 / 100.0);
    let near = seen
        .iter()
        .map(|&(c, chasing)| (c, chasing, pdist(n.at, c)))
        .min_by(|a, b| a.2.total_cmp(&b.2));
    let Some((c, chasing, d)) = near else { return false };
    if !chasing || d >= range {
        return false;
    }
    let away = if d > t.min_away {
        ((n.at.0 - c.0) / d, (n.at.1 - c.1) / d)
    } else {
        (1.0, 0.0)
    };
    let goal = (n.at.0 + away.0 * t.flee_step, n.at.1 + away.1 * t.flee_step);
    walk_toward(n, fac, goal, t.flee_running, t, dt, presses);
    true
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
        let runner = self.p.pos;
        let floor_y = self.p.feet_y.max(0.0);
        let spot = match self.npc_crosshair_floor(eye, dir, floor_y) {
            Some(p) if pdist(p, runner) < SPAWN_NEAR_ADMIN && self.fac.body_fits(p.0, p.1, NAV_RADIUS) => {
                p
            }
            _ => runner,
        };
        let level = self.runner_level();
        for _ in 0..count {
            let slot = self.npc_next + 1;
            // Godot: one NPC stands on the spot itself, more fan out round it
            let ring = if count == 1 { 0.0 } else { slot_radius(slot) };
            let want = slot_point(spot, slot, ring);
            let at = self
                .fac
                .nearest_open(want.0, want.1, NAV_RADIUS)
                .or_else(|| self.fac.nearest_open(spot.0, spot.1, NAV_RADIUS))
                .unwrap_or(spot);
            self.push_npc(at, level, smart, None, 0.0);
        }
        count as usize
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
        });
        self.npc_next += 1;
        id
    }

    /// A normal run's NPCs (data/npcs.json): on level 0 the first `count(difficulty)` homes are drawn from the level seed
    /// (`{seed}:NPC:{n}`), each on an open cell `min_distance`..`max_distance` from the runner and out of its sight. They
    /// wait there until the runner comes close, then follow. An NPC whose key already exists is not made again, so the
    /// same seed always gives the same NPCs and a call after a rebuild adds nothing. Returns how many were made.
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
                        && self.fac.body_fits(p.0, p.1, NAV_RADIUS)
                        && !self.fac.has_line_of_sight(p, runner)
                })
                .collect();
            let at = if cands.is_empty() {
                // a window with no hidden open cell in range: stand beside the runner, still deterministic
                self.fac.nearest_open(runner.0 + rules.min_distance, runner.1, NAV_RADIUS).unwrap_or(runner)
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
    /// facility, so old positions mean nothing). Ring rule as at spawn.
    pub(super) fn npc_place_beside_runner(&mut self) {
        let level = self.runner_level();
        let spot = self.p.pos;
        let total = self.npcs.len();
        let fac = &self.fac;
        for n in self.npcs.iter_mut() {
            let ring = if total == 1 { 0.0 } else { slot_radius(n.slot) };
            let want = slot_point(spot, n.slot, ring);
            n.at = fac
                .nearest_open(want.0, want.1, NAV_RADIUS)
                .or_else(|| fac.nearest_open(spot.0, spot.1, NAV_RADIUS))
                .unwrap_or(spot);
            n.level = level;
            n.path.clear();
            n.path_age = REPATH;
            n.walking = false;
            n.door = None;
        }
    }

    /// The per-frame tick: an NPC on the runner's level runs from a chasing hunter in range, else waits (a run NPC
    /// not yet joined: it joins when the runner is close and in sight) or follows the runner. An NPC on another
    /// level stands still (its level's storey crossing is deferred).
    pub(super) fn update_npcs(&mut self, dt: f32) {
        if self.npcs.is_empty() {
            return;
        }
        let level = self.runner_level();
        let leader = self.p.pos;
        let fac = &self.fac;
        let t = Tuning::get();
        let seen: Vec<Seen> = self
            .hunters
            .iter()
            .filter(|h| h.active)
            .map(|h| (h.pos, matches!(h.state, State::Chase)))
            .collect();
        let mut presses: Vec<Cell> = vec![];
        for n in self.npcs.iter_mut() {
            if n.level != level {
                n.walking = false;
                continue;
            }
            if flee(n, fac, &seen, t, dt, &mut presses) {
                continue;
            }
            if !n.joined {
                if pdist(n.at, leader) <= n.join_radius && fac.has_line_of_sight(n.at, leader) {
                    n.joined = true;
                } else {
                    n.walking = false;
                    n.path.clear();
                    continue;
                }
            }
            follow(n, fac, leader, t, dt, &mut presses);
        }
        self.npc_press_doors(presses);
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
        // put the NPC on an open cell reachable from the runner, about 9 m away
        let fac = &g.fac;
        let start = Cell::new(runner.0.floor() as i32, runner.1.floor() as i32);
        let reach = fac.distance_map(start);
        let far = reach
            .iter()
            .filter(|(c, dist)| **dist >= 8 && **dist <= 12 && fac.body_fits(c.center().0, c.center().1, NAV_RADIUS))
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
            g.update_npcs(1.0 / 60.0);
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
            g.update_npcs(1.0 / 60.0);
        }
        assert!(!g.npcs[0].walking, "idle once settled in range");
        assert_eq!(g.npcs[0].at, settled, "no drift while idle");
    }

    #[test]
    fn an_npc_on_another_level_stands_still() {
        let (mut g, _, _) = offline_game();
        let runner = g.p.pos;
        let (e, d) = sky(runner);
        g.npc_spawn(1, 70, e, d);
        g.npcs[0].level = 1; // the runner is on level 0
        g.npcs[0].at = (runner.0 + 9.0, runner.1);
        let before = g.npcs[0].at;
        for _ in 0..120 {
            g.update_npcs(1.0 / 60.0);
        }
        assert_eq!(g.npcs[0].at, before, "an NPC on another level does not move");
        assert!(!g.npcs[0].walking);
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

    /// A normal Infinite run (the pattern of the endless tests): level 0, seed text fixed.
    fn run_game(diff: Difficulty, run_seed: u32, text: &str) -> (Game, Content) {
        let content = Content::load();
        let catalog = Catalog::load();
        let g = Game::new_infinite_sim(diff, run_seed, text, 0, &content, &catalog);
        (g, content)
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
        for _ in 0..60 {
            g.update_npcs(1.0 / 60.0);
        }
        assert!(pdist(g.npcs[i].at, g.hunters.last().unwrap().pos) > before, "it ran away");
    }

    /// The pursuit inputs `audio::world::update` derives from the hunter list (world.rs 86-96): the nearest active
    /// hunter's distance to the listener, and whether an active chaser pursues the listener.
    fn pursuit_inputs(g: &Game) -> (Option<f64>, bool) {
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
                g.update_npcs(1.0 / 60.0);
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
            // sorted: `distance_map` is a HashMap, whose order changes every process, so the pick must not follow it
            let mut reach: Vec<(Cell, i32)> = fac.distance_map(s).into_iter().collect();
            reach.sort_by_key(|(c, _)| (c.x, c.y));
            for (c, steps) in reach {
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
            let goal = fac
                .distance_map(open)
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
        };
        n.shift((-16.0, 32.0));
        assert_eq!(n.at, (-13.0, 36.0));
        assert!(n.path.is_empty());
        assert!(n.path_age >= REPATH);
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

    #[test]
    fn standard_tuning_matches_the_game() {
        let t = Tuning::get();
        assert_eq!(t.threat_range, 14.0, "Godot npc_teammates.gd 25 THREAT_RANGE");
        assert_eq!(t.smart_base, 0.5, "npc_teammates.gd 307 (.5 + smart / 100)");
        assert_eq!(t.flee_step, 12.0, "npc_teammates.gd 312 flee distance");
        assert!(t.flee_running, "npc_teammates.gd 324-326 runs");
        assert_eq!(t.min_away, 0.1, "npc_teammates.gd 311 away length");
        assert_eq!(t.follow_radius, 5.0, "npc_teammates.gd 24 FOLLOW_RADIUS");
        assert_eq!(t.door_think, 0.25, "DOOR_THINK, the first port's figure");
        assert_eq!(t.door_think_jitter, 0.1, "DOOR_THINK_JITTER, the first port's figure");
        assert_eq!(t.tasks, vec!["door".to_string()]);
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
        // the nearest hunter must be chasing: a nearer hunter that is only wandering stops the run, as Godot's _nearest_hunter
        let nearer_wandering = vec![((start.0 + 2.0, start.1), false), ((start.0 + 5.0, start.1), true)];
        assert!(!flee(&mut g.npcs[0], &g.fac, &nearer_wandering, &wide, dt, &mut presses));
    }

    #[test]
    fn a_bot_steps_at_the_speed_it_is_asked_for() {
        let (g, _) = run_game(Difficulty::Standard, 7, "ROOM-TEST-7");
        let runner = g.p.pos;
        let start = Cell::new(runner.0.floor() as i32, runner.1.floor() as i32);
        let far = g
            .fac
            .distance_map(start)
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
}
