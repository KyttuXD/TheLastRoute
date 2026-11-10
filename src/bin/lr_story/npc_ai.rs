//! NPC behaviour: every NPC tile of every map has an agent that walks tile paths, follows its routine (go somewhere, do
//! something for a while, next step, loop), greets the player, stops while talked to, runs for shelter in rain or snow and
//! flees (or cowers) in a fight. Pure logic with no drawing: `play.rs` owns the agents and ticks them, `npc_anim.rs`
//! turns `action` into a pose.

use crate::body::TILE_M;
use crate::model::{MapDef, NpcDo, Pack, PropAction, Routine, RoutineStep};
use crate::rpg::{find_path, yaw_of, Rng};
use std::f32::consts::PI;

/// Walking speed in metres per second.
pub const WALK_MPS: f32 = 1.4;
/// Turning speed in radians per second.
pub const TURN_RATE: f32 = 6.0;
/// An idle or walking NPC this close (metres) to the player stops and greets.
pub const GREET_RADIUS: f32 = 2.5;
/// Seconds after the player leaves before the NPC carries on.
pub const RESUME_SECS: f32 = 2.0;
/// Seconds one greeting wave lasts.
pub const WAVE_SECS: f32 = 1.2;
/// The NPC body radius used to keep it apart from the player.
pub const NPC_RADIUS: f32 = 0.35;
/// How long a chat step waits for its partner before giving up (seconds).
pub const CHAT_GRACE: f32 = 30.0;

/// What an NPC is doing. The first twelve are the routine's `do` names; the rest are states the AI puts it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NpcAction {
    Idle,
    Sit,
    Lean,
    Work,
    DrawWater,
    Read,
    Sleep,
    WarmHands,
    Look,
    Wave,
    Chat,
    Wander,
    Walk,
    Flee,
    Shelter,
    Cower,
}

#[allow(dead_code)]
impl NpcAction {
    /// The names a routine step's `do` may use.
    pub const NAMES: [&'static str; 12] =
        ["idle", "sit", "lean", "work", "draw_water", "read", "sleep", "warm_hands", "look", "wave", "chat", "wander"];

    pub fn parse(name: &str) -> Result<NpcAction, String> {
        Ok(match name {
            "idle" => NpcAction::Idle,
            "sit" => NpcAction::Sit,
            "lean" => NpcAction::Lean,
            "work" => NpcAction::Work,
            "draw_water" => NpcAction::DrawWater,
            "read" => NpcAction::Read,
            "sleep" => NpcAction::Sleep,
            "warm_hands" => NpcAction::WarmHands,
            "look" => NpcAction::Look,
            "wave" => NpcAction::Wave,
            "chat" => NpcAction::Chat,
            "wander" => NpcAction::Wander,
            other => return Err(format!("unknown action '{other}'; use one of {}", NpcAction::NAMES.join(", "))),
        })
    }

    /// The routine name of an action (the states the AI adds have their own lower-case names).
    pub fn name(self) -> &'static str {
        match self {
            NpcAction::Idle => "idle",
            NpcAction::Sit => "sit",
            NpcAction::Lean => "lean",
            NpcAction::Work => "work",
            NpcAction::DrawWater => "draw_water",
            NpcAction::Read => "read",
            NpcAction::Sleep => "sleep",
            NpcAction::WarmHands => "warm_hands",
            NpcAction::Look => "look",
            NpcAction::Wave => "wave",
            NpcAction::Chat => "chat",
            NpcAction::Wander => "wander",
            NpcAction::Walk => "walk",
            NpcAction::Flee => "flee",
            NpcAction::Shelter => "shelter",
            NpcAction::Cower => "cower",
        }
    }

    /// The action a routine step names with "do".
    pub fn from_do(d: NpcDo) -> NpcAction {
        match d {
            NpcDo::Idle => NpcAction::Idle,
            NpcDo::Sit => NpcAction::Sit,
            NpcDo::Lean => NpcAction::Lean,
            NpcDo::Work => NpcAction::Work,
            NpcDo::DrawWater => NpcAction::DrawWater,
            NpcDo::Read => NpcAction::Read,
            NpcDo::Sleep => NpcAction::Sleep,
            NpcDo::WarmHands => NpcAction::WarmHands,
            NpcDo::Look => NpcAction::Look,
            NpcDo::Wave => NpcAction::Wave,
            NpcDo::Chat => NpcAction::Chat,
            NpcDo::Wander => NpcAction::Wander,
        }
    }

    /// The action an NPC does with a prop that has this use.
    pub fn from_prop(a: PropAction) -> NpcAction {
        match a {
            PropAction::Sit => NpcAction::Sit,
            PropAction::Lean => NpcAction::Lean,
            PropAction::Work => NpcAction::Work,
            PropAction::DrawWater => NpcAction::DrawWater,
            PropAction::Read => NpcAction::Read,
            PropAction::Sleep => NpcAction::Sleep,
            PropAction::WarmHands => NpcAction::WarmHands,
            PropAction::Look => NpcAction::Look,
        }
    }
}

/// One NPC in the world. `action` is what to draw (it already includes a greeting wave and idle glances); `base` is
/// what the routine or the AI state says underneath.
#[derive(Clone, Debug, PartialEq)]
pub struct NpcAgent {
    pub map: String,
    /// The tile the NPC was placed on in the map.
    pub home: (i32, i32),
    /// Metres on the ground plane.
    pub pos: (f32, f32),
    /// Facing, in the body's convention (see `rpg::yaw_of`).
    pub yaw: f32,
    pub action: NpcAction,
    /// The routine step being walked to or done.
    pub step: usize,
    /// Seconds left in the current step's action.
    pub timer: f32,
    /// Tiles still to walk, next one first.
    pub path: Vec<(i32, i32)>,
    /// Whether the feet moved this tick (the walk clip plays).
    pub moving: bool,
    /// Set while the player talks to this NPC.
    pub talk_lock: bool,
    /// Whether the greeting wave was done for the player's current visit.
    pub greeted: bool,
    /// The NPC this one is chatting with.
    pub chat_with: Option<String>,
    pub name: String,
    pub routine: Option<String>,
    pub base: NpcAction,
    /// A short action laid over the base one: the greeting wave or an idle glance, with seconds left.
    pub over: Option<(NpcAction, f32)>,
    /// Seconds still to wait because the player is (or just was) close.
    pub hold: f32,
    /// Whether the first step has been started.
    pub started: bool,
    /// Whether the NPC is waiting out the weather at its shelter.
    pub sheltered: bool,
    /// Seconds the displayed action has lasted.
    pub age: f32,
    /// Where an idle glance turns to.
    pub face: Option<f32>,
    /// Seconds to the next idle glance.
    pub idle_t: f32,
    pub wait_t: f32,
    /// Where a wander step started, the centre of its roaming.
    pub anchor: (f32, f32),
    rng: Rng,
}

pub fn tile_centre(t: (i32, i32)) -> (f32, f32) {
    ((t.0 as f32 + 0.5) * TILE_M, (t.1 as f32 + 0.5) * TILE_M)
}

pub fn tile_of(p: (f32, f32)) -> (i32, i32) {
    ((p.0 / TILE_M).floor() as i32, (p.1 / TILE_M).floor() as i32)
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// The signed shortest turn from `from` to `to`, in -PI..=PI.
pub fn angle_diff(from: f32, to: f32) -> f32 {
    let mut d = (to - from) % (2.0 * PI);
    if d > PI {
        d -= 2.0 * PI;
    } else if d < -PI {
        d += 2.0 * PI;
    }
    d
}

/// Turns `cur` toward `target` by at most `max` radians.
pub fn turn_toward(cur: f32, target: f32, max: f32) -> f32 {
    let d = angle_diff(cur, target);
    if d.abs() <= max {
        cur + d
    } else {
        cur + d.signum() * max
    }
}

fn unit(rng: &mut Rng) -> f32 {
    rng.next_u32() as f32 / u32::MAX as f32
}

/// The tile a step's `go` names: a spawn of the map, or `prop:<x>,<y>`.
pub fn resolve_go(go: &str, map: &MapDef) -> Option<(i32, i32)> {
    if let Some(rest) = go.strip_prefix("prop:") {
        let (x, y) = rest.split_once(',')?;
        return Some((x.trim().parse().ok()?, y.trim().parse().ok()?));
    }
    map.spawns.get(go).copied()
}

impl NpcAgent {
    /// An agent standing at the centre of its home tile, facing south, doing nothing.
    pub fn new(map: &str, home: (i32, i32), name: &str, routine: Option<String>) -> NpcAgent {
        let mut rng = Rng::new(Rng::seed_from(&format!("{map}@{},{}", home.0, home.1)));
        let idle_t = 2.0 + unit(&mut rng) * 4.0;
        let pos = tile_centre(home);
        NpcAgent {
            map: map.to_string(),
            home,
            pos,
            yaw: PI,
            action: NpcAction::Idle,
            step: 0,
            timer: 0.0,
            path: Vec::new(),
            moving: false,
            talk_lock: false,
            greeted: false,
            chat_with: None,
            name: name.to_string(),
            routine,
            base: NpcAction::Idle,
            over: None,
            hold: 0.0,
            started: false,
            sheltered: false,
            age: 0.0,
            face: None,
            idle_t,
            wait_t: 0.0,
            anchor: pos,
            rng,
        }
    }

    pub fn tile(&self) -> (i32, i32) {
        tile_of(self.pos)
    }

    /// An NPC with no routine stands at home and blocks its tile.
    pub fn is_static(&self) -> bool {
        self.routine.is_none()
    }
}

/// What a chat step needs to know about the other NPCs on the map.
#[derive(Clone, Debug, PartialEq)]
pub struct Partner {
    pub name: String,
    pub pos: (f32, f32),
    /// The partner's underlying action (not a wave laid over it).
    pub base: NpcAction,
    pub chat_with: Option<String>,
}

pub fn partner_of(a: &NpcAgent) -> Partner {
    Partner { name: a.name.clone(), pos: a.pos, base: a.base, chat_with: a.chat_with.clone() }
}

/// The world as one agent sees it for one tick.
pub struct Env<'a> {
    pub pack: &'a Pack,
    pub map: &'a MapDef,
    /// Whether a tile stops a walker.
    pub blocks: &'a dyn Fn(i32, i32) -> bool,
    pub player: (f32, f32),
    /// A fight is on in this map.
    pub danger: bool,
    /// It is raining, snowing or storming here.
    pub wet: bool,
    pub partners: &'a [Partner],
}

/// A path from the agent to tile `goal` (empty when it is already standing at its centre).
fn plan(a: &NpcAgent, goal: (i32, i32), env: &Env) -> Option<Vec<(i32, i32)>> {
    let start = a.tile();
    if start == goal {
        return Some(if dist(a.pos, tile_centre(goal)) > 0.05 { vec![goal] } else { Vec::new() });
    }
    find_path(start, goal, env.blocks)
}

/// Walks along `a.path` for `dt` seconds. True when the path is used up.
fn follow(a: &mut NpcAgent, dt: f32) -> bool {
    let mut left = WALK_MPS * dt;
    a.moving = false;
    while let Some(&t) = a.path.first() {
        let c = tile_centre(t);
        let d = dist(a.pos, c);
        if d > 1e-3 {
            let want = yaw_of(c.0 - a.pos.0, c.1 - a.pos.1);
            let diff = angle_diff(a.yaw, want);
            a.yaw = turn_toward(a.yaw, want, TURN_RATE * dt);
            // A sharp turn is made on the spot before walking on.
            if diff.abs() > 1.0 {
                break;
            }
        }
        if left <= 0.0 {
            break;
        }
        a.moving = true;
        if d <= left {
            a.pos = c;
            left -= d;
            a.path.remove(0);
        } else {
            a.pos = (a.pos.0 + (c.0 - a.pos.0) / d * left, a.pos.1 + (c.1 - a.pos.1) / d * left);
            left = 0.0;
        }
    }
    a.path.is_empty()
}

fn face_point(a: &mut NpcAgent, p: (f32, f32), dt: f32) {
    if dist(a.pos, p) > 1e-3 {
        a.yaw = turn_toward(a.yaw, yaw_of(p.0 - a.pos.0, p.1 - a.pos.1), TURN_RATE * dt);
    }
}

fn tick_over(a: &mut NpcAgent, dt: f32) {
    if let Some(o) = a.over.as_mut() {
        o.1 -= dt;
        if o.1 <= 0.0 {
            a.over = None;
            a.face = None;
        }
    }
}

/// Settles what to draw: the layered action, or the base one.
fn finish(a: &mut NpcAgent, dt: f32) {
    let held = a.hold > 0.0 && matches!(a.base, NpcAction::Walk | NpcAction::Shelter);
    let shown = a.over.map(|o| o.0).unwrap_or(if held { NpcAction::Idle } else { a.base });
    if shown == a.action {
        a.age += dt;
    } else {
        a.age = 0.0;
        a.action = shown;
    }
}

/// The occasional glance of a standing NPC: turn the head and body a little and look back.
fn idle_variety(a: &mut NpcAgent, dt: f32) {
    a.idle_t -= dt;
    if a.idle_t <= 0.0 && a.over.is_none() {
        let off = (unit(&mut a.rng) * 2.0 - 1.0) * 1.2;
        a.face = Some(a.yaw + off);
        a.over = Some((NpcAction::Look, 2.4));
        a.idle_t = 5.0 + unit(&mut a.rng) * 7.0;
    }
    if a.over.map_or(false, |o| o.0 == NpcAction::Look) {
        if let Some(y) = a.face {
            a.yaw = turn_toward(a.yaw, y, TURN_RATE * 0.5 * dt);
        }
    }
}

/// Starts the action of a step the NPC has reached.
fn arrive(a: &mut NpcAgent, step: &RoutineStep) {
    a.path.clear();
    a.moving = false;
    a.base = NpcAction::from_do(step.do_);
    a.timer = step.secs.max(0.1);
    a.wait_t = 0.0;
    a.anchor = a.pos;
    a.chat_with = if step.do_ == NpcDo::Chat { step.with.clone() } else { None };
}

/// Starts step `i`: walk to where it goes, or do it at once when already there (or when there is no way).
fn begin_step(a: &mut NpcAgent, env: &Env, r: &Routine, i: usize) {
    a.step = i;
    a.chat_with = None;
    a.wait_t = 0.0;
    a.path.clear();
    let step = &r.steps[i];
    let path = step.go.as_deref().and_then(|g| resolve_go(g, env.map)).and_then(|g| plan(a, g, env)).unwrap_or_default();
    if path.is_empty() {
        arrive(a, step);
    } else {
        a.path = path;
        a.base = NpcAction::Walk;
        a.timer = 0.0;
    }
}

fn wander(a: &mut NpcAgent, env: &Env, step: &RoutineStep, dt: f32) {
    a.timer -= dt;
    if !a.path.is_empty() {
        if follow(a, dt) {
            a.wait_t = 1.0 + unit(&mut a.rng) * 2.0;
        }
        return;
    }
    a.moving = false;
    a.wait_t -= dt;
    if a.wait_t > 0.0 {
        return;
    }
    let radius = step.radius.max(TILE_M);
    let r = (radius / TILE_M).ceil() as i32;
    let cur = a.tile();
    for _ in 0..6 {
        let dx = a.rng.below((2 * r + 1) as u32) as i32 - r;
        let dy = a.rng.below((2 * r + 1) as u32) as i32 - r;
        let t = (cur.0 + dx, cur.1 + dy);
        if t == cur || dist(tile_centre(t), a.anchor) > radius || (env.blocks)(t.0, t.1) {
            continue;
        }
        if let Some(p) = find_path(cur, t, env.blocks) {
            if !p.is_empty() && p.len() as i32 <= 2 * r + 2 {
                a.path = p;
                return;
            }
        }
    }
    a.wait_t = 1.5;
}

fn chat(a: &mut NpcAgent, env: &Env, dt: f32) {
    a.moving = false;
    let partner = env.partners.iter().find(|p| {
        Some(&p.name) == a.chat_with.as_ref() && p.base == NpcAction::Chat && p.chat_with.as_deref() == Some(a.name.as_str()) && dist(p.pos, a.pos) < 5.0
    });
    match partner {
        Some(p) => {
            face_point(a, p.pos, dt);
            a.timer -= dt;
        }
        None => {
            a.wait_t += dt;
            if a.wait_t > CHAT_GRACE {
                a.timer = 0.0;
            }
        }
    }
}

fn run_routine(a: &mut NpcAgent, env: &Env, r: &Routine, dt: f32) {
    let n = r.steps.len();
    if a.step >= n {
        if r.looping {
            a.step = 0;
        } else {
            a.base = NpcAction::Idle;
            a.moving = false;
            idle_variety(a, dt);
            return;
        }
    }
    if !a.started {
        a.started = true;
        begin_step(a, env, r, a.step);
    }
    if a.base == NpcAction::Walk {
        if follow(a, dt) {
            let step = &r.steps[a.step];
            arrive(a, step);
        }
        return;
    }
    let step = &r.steps[a.step];
    match a.base {
        NpcAction::Idle => {
            a.moving = false;
            a.timer -= dt;
            idle_variety(a, dt);
        }
        NpcAction::Wander => wander(a, env, step, dt),
        NpcAction::Chat => chat(a, env, dt),
        _ => {
            a.moving = false;
            a.timer -= dt;
        }
    }
    if a.timer <= 0.0 {
        let next = a.step + 1;
        if next >= n && !r.looping {
            a.step = n;
            a.base = NpcAction::Idle;
            a.chat_with = None;
        } else {
            begin_step(a, env, r, next % n);
        }
    }
}

/// A fight is on: run to the routine's `flee` spawn and cower there, or cower in place.
fn danger(a: &mut NpcAgent, env: &Env, routine: Option<&Routine>, dt: f32) {
    a.hold = 0.0;
    a.over = None;
    a.sheltered = false;
    a.chat_with = None;
    if !matches!(a.base, NpcAction::Flee | NpcAction::Cower) {
        let spot = routine.and_then(|r| r.flee.as_deref()).and_then(|n| resolve_go(n, env.map));
        match spot.and_then(|s| plan(a, s, env)) {
            Some(p) if !p.is_empty() => {
                a.path = p;
                a.base = NpcAction::Flee;
            }
            _ => {
                a.path.clear();
                a.base = NpcAction::Cower;
            }
        }
    }
    if a.base == NpcAction::Flee {
        if follow(a, dt) {
            a.base = NpcAction::Cower;
        }
    } else {
        a.moving = false;
    }
}

/// Advances one agent by `dt` seconds.
pub fn tick_agent(a: &mut NpcAgent, env: &Env, dt: f32) {
    let dt = dt.clamp(0.0, 0.5);
    tick_over(a, dt);
    let routine: Option<&Routine> = a.routine.as_deref().and_then(|r| env.pack.routines.get(r)).filter(|r| !r.steps.is_empty());

    // Being talked to: stand still and face the player.
    if a.talk_lock {
        a.moving = false;
        face_point(a, env.player, dt);
        finish(a, dt);
        return;
    }
    // A fight: flee or cower.
    if env.danger {
        danger(a, env, routine, dt);
        finish(a, dt);
        return;
    }
    if matches!(a.base, NpcAction::Flee | NpcAction::Cower) {
        a.base = NpcAction::Idle;
        a.started = false;
        a.path.clear();
    }
    // Rain or snow: head for the shelter, wait there, come back when it clears.
    let shelter = a.routine.as_deref().and_then(|r| env.pack.routines.get(r)).and_then(|r| r.shelter.as_deref()).and_then(|n| resolve_go(n, env.map));
    if env.wet && shelter.is_some() {
        if !a.sheltered && a.base != NpcAction::Shelter {
            match shelter.and_then(|s| plan(a, s, env)) {
                Some(p) if !p.is_empty() => {
                    a.path = p;
                    a.base = NpcAction::Shelter;
                    a.over = None;
                }
                _ => {
                    a.path.clear();
                    a.base = NpcAction::Idle;
                    a.sheltered = true;
                }
            }
        }
    } else if a.sheltered || a.base == NpcAction::Shelter {
        a.sheltered = false;
        a.base = NpcAction::Idle;
        a.started = false;
        a.path.clear();
    }

    // The player is close: stop, face them, wave once, and carry on 2 s after they leave.
    let near = dist(a.pos, env.player) < GREET_RADIUS;
    let eligible = matches!(a.base, NpcAction::Idle | NpcAction::Walk | NpcAction::Wander | NpcAction::Shelter);
    if near && (eligible || a.hold > 0.0) {
        if !a.greeted {
            a.greeted = true;
            a.over = Some((NpcAction::Wave, WAVE_SECS));
            a.face = None;
        }
        a.hold = RESUME_SECS;
        a.moving = false;
        face_point(a, env.player, dt);
        finish(a, dt);
        return;
    }
    if a.hold > 0.0 {
        a.hold -= dt;
        a.moving = false;
        if a.hold <= 0.0 {
            a.hold = 0.0;
            a.greeted = false;
        }
        finish(a, dt);
        return;
    }

    if a.base == NpcAction::Shelter {
        if follow(a, dt) {
            a.base = NpcAction::Idle;
            a.sheltered = true;
        }
    } else if a.sheltered {
        a.moving = false;
        idle_variety(a, dt);
    } else if let Some(r) = routine {
        run_routine(a, env, r, dt);
    } else {
        a.base = NpcAction::Idle;
        a.moving = false;
        idle_variety(a, dt);
    }
    finish(a, dt);
}

/// How far to move the player and an NPC apart when their bodies overlap: None when they do not, else the unit
/// direction from the NPC to the player and the overlap in metres.
pub fn push_apart(player: (f32, f32), npc: (f32, f32), min: f32) -> Option<((f32, f32), f32)> {
    let d = dist(player, npc);
    if d >= min {
        return None;
    }
    let dir = if d < 1e-4 { (1.0, 0.0) } else { ((player.0 - npc.0) / d, (player.1 - npc.1) / d) };
    Some((dir, min - d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_names_round_trip_and_unknown_ones_list_the_choices() {
        for n in NpcAction::NAMES {
            assert_eq!(NpcAction::parse(n).unwrap().name(), n);
        }
        let e = NpcAction::parse("dance").unwrap_err();
        assert!(e.contains("unknown action 'dance'") && e.contains("idle, sit, lean"), "{e}");
        assert_eq!(NpcAction::from_prop(PropAction::DrawWater), NpcAction::DrawWater);
    }

    #[test]
    fn angles_turn_the_short_way() {
        assert!((angle_diff(3.0, -3.0) - (2.0 * PI - 6.0)).abs() < 1e-4, "across the seam");
        assert!((turn_toward(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
        assert_eq!(turn_toward(0.0, 0.1, 0.25), 0.1);
    }

    #[test]
    fn push_apart_points_from_the_npc_to_the_player() {
        assert!(push_apart((0.0, 0.0), (1.0, 0.0), 0.5).is_none());
        let (dir, o) = push_apart((0.0, 0.0), (0.3, 0.0), 0.6).unwrap();
        assert!((dir.0 + 1.0).abs() < 1e-5 && (o - 0.3).abs() < 1e-5);
        assert!(push_apart((1.0, 1.0), (1.0, 1.0), 0.6).is_some(), "on top of each other still pushes");
    }
}
