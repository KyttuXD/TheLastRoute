//! Nyxie (special): darkness stalker. 120 s without light wakes her 10 cells away and she hunts
//! you until the beam catches her (instantly dispelled within 7 cells). A DEAD flashlight (charge
//! empty, the lamp cannot light; `Ctx::flashlight_dead`) brings her in after only a few seconds
//! (data/hunters/nyxie_wake.json `dead_delay`, 3 s) instead of the 120 s, and a recharged beam
//! sends her back the same way. A lamp left dead for 10 s wakes her on top of you: an inescapable
//! catch unless you are hidden or safe (nyxie_gimmick.gd). The delays and distances are data in
//! data/hunters/nyxie_wake.json.
//!
//! The hunter activation count (the relays a difficulty needs before any hunter is out, `endless::population`) gates the
//! route's own hunters; it never gates a dead flashlight (`ignore_activation_gate` in the wake file). Level 0 of the
//! Infinite route has no Nyxie in its lineup, so the game makes one the moment the flashlight is dead there
//! (`Game::keep_level0_nyxie`, marked `dead_only`): the dead lamp (and the 10 s lamp kill it grows into) wakes her with no
//! relay recovered on any difficulty. Level 0 never had her 120 s darkness rule, and she keeps none there.
//!
//! Nightmare only: while she hunts in the dark she pounces (hard_abilities.gd pounce, Nightmare
//! row). The pounce numbers, the leap and the gates are data in data/hunters/nightmare_pounce.json.
//! Solyth uses the same leap (solyth.rs).

use std::sync::{Mutex, OnceLock};
use crate::shared::clock::Instant;

use super::Ctx;
use crate::config::{Difficulty, HUNTER_RADIUS};
use crate::hunters::brain::{dist, Brain, State, Target};
use crate::world::Facility;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Wake {
    #[default]
    None,
    /// The lamp stayed unlit `timer_delay` seconds.
    Timer,
    /// The flashlight is dead (charge empty): she appears `dead_delay` seconds in. Dispelled by a live beam like Timer.
    Dead,
    /// The lamp stayed dead `empty_delay` seconds: on top of the runner, an inescapable catch.
    Empty,
}

#[derive(Default)]
pub struct Nyxie {
    pub wake: Wake,
    darkness: f32,
    empty: f32,
    /// Seconds the flashlight has been dead (`Ctx::flashlight_dead`).
    dead: f32,
    /// Set the frame she wakes (for the sound cue).
    pub just_woke: bool,
    leap: Leap,
    /// The runner, set by the host for the frame only when she chases a bot instead (hard and nightmare,
    /// data/hunters/targets.json). Her wake, dispel, darkness clock and pounce gate read the runner's lamp and beam, as
    /// Godot's target always is the runner; the chase itself still follows the bot she was given. `before` takes it.
    pub runner: Option<Target>,
    /// This is the level-0 Nyxie of the Infinite route (`Game::keep_level0_nyxie`), made when the flashlight dies there:
    /// only a dead flashlight (and the lamp kill it grows into) wakes her, not the 120 s darkness rule, which level 0's
    /// lineup never had. False for every other Nyxie.
    pub dead_only: bool,
}

/// The floor cell whose centre distance to `p` is closest to `d` (first match).
pub fn spawn_near(c: &Ctx, p: (f32, f32), d: f32) -> (f32, f32) {
    let mut best = (f32::MAX, p);
    for cell in c.fac.floor_cells() {
        let cc = cell.center();
        let err = (crate::hunters::brain::dist(cc, p) - d).abs();
        if err < best.0 {
            best = (err, cc);
        }
    }
    best.1
}

// ---------------------------------------------------------------------------------------- wake rules

pub const WAKE_SCHEMA: &str = "lr.nyxie_wake/1";

/// When and where Nyxie wakes (data/hunters/nyxie_wake.json). The Godot numbers are GameConfig.NYXIE_*; the dead-lamp
/// appearance (`dead_delay`, `dead_spawn_distance`) is the owner's.
#[derive(Clone, Debug, PartialEq)]
pub struct WakeRules {
    /// Seconds the lamp stays unlit before she wakes (NYXIE_TIMER_DELAY).
    pub timer_delay: f32,
    /// Seconds the lamp stays dead before she is on top of the runner (NYXIE_EMPTY_DELAY).
    pub empty_delay: f32,
    /// Seconds a dead flashlight takes to bring her in.
    pub dead_delay: f32,
    /// Cells from the runner where the timer wake puts her (NYXIE_SPAWN_DISTANCE).
    pub spawn_distance: f32,
    /// Cells from the runner where the dead-flashlight wake puts her.
    pub dead_spawn_distance: f32,
    /// A live beam reaches her this far (NYXIE_DESPAWN_RANGE).
    pub despawn_range: f32,
    /// Half angle of the beam that dispels her, radians (NYXIE_DISPEL_HALF_ANGLE).
    pub dispel_half_angle: f32,
    /// A dead flashlight brings her even before the difficulty's hunter activation count (Relaxed: 3 relays; Standard: 1;
    /// Hard and Nightmare: 0). On: the Infinite route's level 0, whose streamed lineup has no Nyxie (the count keeps its
    /// hunters out), makes a dormant one the moment the flashlight is dead there, and she wakes after `dead_delay` (then
    /// the `empty_delay` lamp kill) with no relay recovered. Off: level 0 has no Nyxie, as in Godot.
    pub ignore_activation_gate: bool,
}

const WAKE_KEYS: [&str; 8] = [
    "timer_delay",
    "empty_delay",
    "dead_delay",
    "spawn_distance",
    "dead_spawn_distance",
    "despawn_range",
    "dispel_half_angle",
    "ignore_activation_gate",
];

impl WakeRules {
    /// Reads the file. Every key is required; an unknown key (one not starting with `_`), a wrong schema or a value outside
    /// its range refuses the file with the reason.
    pub fn parse(text: &str) -> Result<WakeRules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("nyxie wake: {e}"))?;
        let o = v.as_object().ok_or("nyxie wake: the file must be an object")?;
        if o.get("schema").and_then(|s| s.as_str()) != Some(WAKE_SCHEMA) {
            return Err(format!("nyxie wake: schema must be \"{WAKE_SCHEMA}\""));
        }
        for k in o.keys() {
            if k != "schema" && !k.starts_with('_') && !WAKE_KEYS.contains(&k.as_str()) {
                return Err(format!("nyxie wake: unknown key \"{k}\""));
            }
        }
        let get = |k: &str, lo: f64, hi: f64| -> Result<f32, String> {
            match o.get(k).and_then(|x| x.as_f64()) {
                Some(x) if x.is_finite() && (lo..=hi).contains(&x) => Ok(x as f32),
                _ => Err(format!("nyxie wake: {k} must be a number from {lo} to {hi}")),
            }
        };
        let r = WakeRules {
            timer_delay: get("timer_delay", 1.0, 3600.0)?,
            empty_delay: get("empty_delay", 0.0, 600.0)?,
            dead_delay: get("dead_delay", 0.0, 600.0)?,
            spawn_distance: get("spawn_distance", 0.0, 100.0)?,
            dead_spawn_distance: get("dead_spawn_distance", 0.0, 100.0)?,
            despawn_range: get("despawn_range", 0.0, 100.0)?,
            dispel_half_angle: get("dispel_half_angle", 0.01, std::f64::consts::PI)?,
            ignore_activation_gate: o
                .get("ignore_activation_gate")
                .and_then(|x| x.as_bool())
                .ok_or("nyxie wake: ignore_activation_gate must be true or false")?,
        };
        if r.dead_delay > r.empty_delay {
            return Err("nyxie wake: dead_delay must not exceed empty_delay".to_string());
        }
        Ok(r)
    }
}

/// The embedded wake rules (data/hunters/nyxie_wake.json). A broken shipped file is caught by the tests.
pub fn wake_rules() -> &'static WakeRules {
    static R: OnceLock<WakeRules> = OnceLock::new();
    R.get_or_init(|| {
        WakeRules::parse(include_str!("../../data/hunters/nyxie_wake.json"))
            .unwrap_or_else(|e| panic!("data/hunters/nyxie_wake.json: {e}"))
    })
}

// ---------------------------------------------------------------------------------- Nightmare pounce data

/// The pounce chance and timing (hard_abilities.gd POUNCE, nightmare row).
#[derive(Clone, Debug, PartialEq)]
pub struct PounceNumbers {
    /// Closest and farthest distance (cells) a pounce can launch from.
    pub min: f32,
    pub max: f32,
    /// Chance per eligible second (the roll is `rate * dt` each frame).
    pub rate: f32,
    /// Seconds between two pounces by the same hunter.
    pub cooldown: f32,
    /// Pounces allowed in one chase.
    pub per_chase: u32,
    /// Seconds she stumbles (slowed, cannot pounce) after a clean leap.
    pub stumble: f32,
    /// Seconds one leap lasts.
    pub seconds: f32,
}

/// The leap itself and how it is slowed or stopped.
#[derive(Clone, Debug, PartialEq)]
pub struct PounceLeap {
    /// Cells per second during the leap.
    pub leap_speed: f32,
    /// The leap aims this many seconds ahead of the runner's estimated velocity.
    pub lead_seconds: f32,
    /// Stun when a wall stops the leap.
    pub bonk_stun: f32,
    /// Speed factor while stumbling.
    pub stumble_speed: f32,
}

/// data/hunters/nightmare_pounce.json. The Nightmare values are the Godot ones (hard_abilities.gd).
#[derive(Clone, Debug, PartialEq)]
pub struct NightmarePounce {
    /// The difficulty ids the pounce is active on (Nightmare only by default).
    pub difficulties: Vec<String>,
    pub numbers: PounceNumbers,
    pub leap: PounceLeap,
    /// Nyxie pounces only while the lamp is off.
    pub nyxie_requires_dark: bool,
    /// Solyth pounces only at or above this body temperature.
    pub solyth_min_temperature: f32,
    /// Seconds between two pounces anywhere in the party (hard_abilities.gd BUDGET, nightmare).
    pub budget_seconds: f32,
    /// Cells between the samples of the wall sweep from hunter to runner (hunter_brain.gd SWEEP_STEP).
    pub sweep_step: f32,
}

impl Default for NightmarePounce {
    fn default() -> Self {
        NightmarePounce {
            budget_seconds: 25.0,
            sweep_step: 0.15,
            difficulties: vec!["nightmare".to_string()],
            numbers: PounceNumbers {
                min: 2.2,
                max: 5.5,
                rate: 0.56,
                cooldown: 18.0,
                per_chase: 2,
                stumble: 0.8,
                seconds: 0.5,
            },
            leap: PounceLeap {
                leap_speed: 7.2,
                lead_seconds: 0.15,
                bonk_stun: 0.8,
                stumble_speed: 0.3,
            },
            nyxie_requires_dark: true,
            solyth_min_temperature: 60.0,
        }
    }
}

fn num(v: &serde_json::Value, key: &str, base: f32) -> f32 {
    v[key].as_f64().map(|x| x as f32).unwrap_or(base)
}

impl NightmarePounce {
    /// Reads the file. A missing or wrong value keeps the default.
    pub fn parse(v: &serde_json::Value) -> NightmarePounce {
        let mut p = NightmarePounce::default();
        if let Some(a) = v["difficulties"].as_array() {
            let ids: Vec<String> = a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
            p.difficulties = ids;
        }
        let n = &v["numbers"];
        let d = p.numbers.clone();
        p.numbers = PounceNumbers {
            min: num(n, "min", d.min).max(0.0),
            max: num(n, "max", d.max).max(0.0),
            rate: num(n, "rate", d.rate).clamp(0.0, 1.0),
            cooldown: num(n, "cooldown", d.cooldown).max(0.0),
            per_chase: n["per_chase"].as_u64().map(|x| x.min(99) as u32).unwrap_or(d.per_chase),
            stumble: num(n, "stumble", d.stumble).max(0.0),
            seconds: num(n, "seconds", d.seconds).max(0.0),
        };
        let l = &v["leap"];
        let e = p.leap.clone();
        p.leap = PounceLeap {
            leap_speed: num(l, "leap_speed", e.leap_speed).max(0.0),
            lead_seconds: num(l, "lead_seconds", e.lead_seconds).max(0.0),
            bonk_stun: num(l, "bonk_stun", e.bonk_stun).max(0.0),
            stumble_speed: num(l, "stumble_speed", e.stumble_speed).clamp(0.0, 1.0),
        };
        let g = &v["gates"];
        if let Some(b) = g["nyxie_requires_dark"].as_bool() {
            p.nyxie_requires_dark = b;
        }
        p.solyth_min_temperature = num(g, "solyth_min_temperature", p.solyth_min_temperature);
        p.budget_seconds = num(&v["budget"], "seconds", p.budget_seconds).max(0.0);
        p.sweep_step = num(&v["sweep"], "step", p.sweep_step).max(0.01);
        p
    }

    /// True when the pounce is on for this difficulty (Nightmare only in the data).
    pub fn applies(&self, diff: Difficulty) -> bool {
        self.difficulties.iter().any(|d| d == diff.id())
    }
}

/// The embedded pounce data (data/hunters/nightmare_pounce.json); the defaults when missing or broken.
pub fn pounce() -> &'static NightmarePounce {
    static P: OnceLock<NightmarePounce> = OnceLock::new();
    P.get_or_init(|| {
        crate::content::embedded::data_under("hunters/nightmare_pounce.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<serde_json::Value>(text).ok())
            .map(|v| NightmarePounce::parse(&v))
            .unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------------------- leap state

/// One hunter's pounce state (hard_abilities.gd leap and cooldown vars).
#[derive(Default)]
pub struct Leap {
    pub leaping: bool,
    /// Increments on every launch (the view can play its footfall on it).
    pub seq: u32,
    dir: (f32, f32),
    left: f32,
    cooldown: f32,
    stumble: f32,
    /// Pounces launched in the current chase.
    count: u32,
    last_target: Option<(f32, f32)>,
    velocity: (f32, f32),
}

impl Leap {
    /// Slowed while stumbling after a clean leap.
    pub fn speed_scale(&self, n: &NightmarePounce) -> f32 {
        if self.stumble > 0.0 {
            n.leap.stumble_speed
        } else {
            1.0
        }
    }

    /// Clears the chase state but keeps the launch counter for the sound cue.
    fn reset(&mut self) {
        let seq = self.seq;
        *self = Leap::default();
        self.seq = seq;
    }

    /// Runner velocity estimate from the last two frames (hard_abilities.gd `_target_velocity`).
    fn track(&mut self, pos: (f32, f32), dt: f32) {
        if let Some(last) = self.last_target {
            if dt > 0.0 {
                self.velocity = ((pos.0 - last.0) / dt, (pos.1 - last.1) / dt);
            }
        }
        self.last_target = Some(pos);
    }

    fn tick(&mut self, dt: f32) {
        self.cooldown = (self.cooldown - dt).max(0.0);
        self.stumble = (self.stumble - dt).max(0.0);
    }

    /// Whether a pounce may launch now, before the chance roll.
    pub fn can_start(&self, n: &NightmarePounce, distance: f32, eligible: bool) -> bool {
        !self.leaping
            && self.cooldown <= 0.0
            && self.stumble <= 0.0
            && self.count < n.numbers.per_chase
            && eligible
            && distance >= n.numbers.min
            && distance <= n.numbers.max
    }

    fn start(&mut self, n: &NightmarePounce, dir: (f32, f32)) {
        self.leaping = true;
        self.dir = dir;
        self.left = n.numbers.seconds;
        self.count += 1;
        self.seq += 1;
    }

    /// Ends the leap: a wall stop stuns (bonk), a full leap stumbles.
    fn finish(&mut self, n: &NightmarePounce, bonk: bool) {
        self.leaping = false;
        self.cooldown = n.numbers.cooldown;
        if !bonk {
            self.stumble = n.numbers.stumble;
        }
    }

    /// One frame of the leap. Returns true (the frame's movement is the leap).
    fn step(&mut self, n: &NightmarePounce, b: &mut Brain, dt: f32, c: &Ctx) -> bool {
        let wanted = n.leap.leap_speed * dt.min(self.left).max(0.0);
        let moved = b.move_with_collision(self.dir.0 * wanted, self.dir.1 * wanted, c.fac);
        b.facing = self.dir;
        self.left -= dt;
        if wanted > 0.001 && moved < wanted * 0.5 {
            // a wall, prop or door stopped the leap: she is stunned for a moment
            self.finish(n, true);
            b.apply_stun(n.leap.bonk_stun, false);
            return true;
        }
        if self.left <= 0.0 {
            self.finish(n, false);
        }
        true
    }
}

/// Runs one frame of the pounce for a hunter that hunts the runner. `gate` is the hunter's own
/// condition (Nyxie: lamp off; Solyth: temperature). Returns true when this frame is the leap, so
/// the caller must not chase.
pub fn pounce_frame(leap: &mut Leap, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, gate: bool) -> bool {
    let n = pounce();
    if !n.applies(c.diff) {
        leap.reset();
        return false;
    }
    leap.track(t.pos, dt);
    leap.tick(dt);
    if leap.leaping {
        return leap.step(n, b, dt, c);
    }
    if b.state != State::Chase {
        leap.count = 0;
        return false;
    }
    let eligible = gate && b.direct_visible && b.stun <= 0.0 && !t.hidden && !t.in_safe_zone && !t.in_vent;
    if !leap.can_start(n, dist(b.pos, t.pos), eligible) {
        return false;
    }
    if !sweep_clear(c.fac, b.pos, t.pos, n.sweep_step) {
        return false;
    }
    if b.next_float() >= n.numbers.rate * dt {
        return false;
    }
    if !party_budget_ok(party_now(), n.budget_seconds, true) {
        return false;
    }
    let lead = n.leap.lead_seconds;
    let predicted = (t.pos.0 + leap.velocity.0 * lead, t.pos.1 + leap.velocity.1 * lead);
    let mut dir = (predicted.0 - b.pos.0, predicted.1 - b.pos.1);
    if dir.0 * dir.0 + dir.1 * dir.1 < 1e-4 {
        dir = (t.pos.0 - b.pos.0, t.pos.1 - b.pos.1);
    }
    let len = (dir.0 * dir.0 + dir.1 * dir.1).sqrt();
    if len > 0.0 {
        dir = (dir.0 / len, dir.1 / len);
    }
    leap.start(n, dir);
    leap.step(n, b, dt, c)
}

/// Seconds on the party clock (real time, like Godot's Time.get_ticks_msec in hard_abilities.gd).
static PARTY_START: OnceLock<Instant> = OnceLock::new();
/// The party's last pounce on the clock (hard_abilities.gd `_last_fire["pounce"]`, shared by every
/// hunter in the party). Not reset between matches yet (Godot's reset_director has no call here).
static PARTY_LAST_POUNCE: Mutex<Option<f32>> = Mutex::new(None);

fn party_now() -> f32 {
    PARTY_START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// True when `budget` seconds have passed since the last party pounce (hard_abilities.gd `_budget_ok`).
pub fn budget_open(last: Option<f32>, now: f32, budget: f32) -> bool {
    match last {
        Some(t) => now - t >= budget,
        None => true,
    }
}

/// The party budget check. With `commit`, a pass records this pounce for the whole party.
fn party_budget_ok(now: f32, budget: f32, commit: bool) -> bool {
    let mut last = match PARTY_LAST_POUNCE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !budget_open(*last, now, budget) {
        return false;
    }
    if commit {
        *last = Some(now);
    }
    true
}

/// How many samples the wall sweep takes over `distance` cells, one every `step` (at least one).
pub fn sweep_samples(distance: f32, step: f32) -> i32 {
    ((distance / step).ceil() as i32).max(1)
}

/// Whether the hunter's body can sweep the straight line from `from` to `to` (hunter_brain.gd
/// `_clear_sweep`). The sample step comes from data; brain.rs keeps its own copy for the walk.
pub fn sweep_clear(fac: &Facility, from: (f32, f32), to: (f32, f32), step: f32) -> bool {
    let steps = sweep_samples(dist(from, to), step);
    (1..=steps).all(|i| {
        let t = i as f32 / steps as f32;
        fac.body_fits(
            from.0 + (to.0 - from.0) * t,
            from.1 + (to.1 - from.1) * t,
            HUNTER_RADIUS,
        )
    })
}

impl Nyxie {
    pub fn inescapable(&self) -> bool {
        self.wake == Wake::Empty
    }

    /// Starts or ends the leap without the chase around it (tests).
    #[cfg(test)]
    pub fn set_leaping(&mut self, on: bool) {
        self.leap.leaping = on;
    }

    pub fn speed_scale(&self) -> f32 {
        self.leap.speed_scale(pounce())
    }

    /// The leap holds her position (the leap moves her itself).
    pub fn holds_position(&self) -> bool {
        self.leap.leaping
    }

    /// The mechanic pose she is in: the pounce in the air (the wake is a cue).
    pub fn mechanic(&self) -> Option<&'static str> {
        self.leap.leaping.then_some("pounce")
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) {
        self.just_woke = false;
        let rules = wake_rules();
        // the runner's lamp, beam and place; `t` is the chase target (a bot may stand in for the runner)
        let runner = self.runner.take();
        let r = runner.as_ref().unwrap_or(t);
        let lit = r.flashlight_on && r.lamp_charge > 0.0;
        if lit {
            self.darkness = 0.0;
            self.empty = 0.0;
            self.dead = 0.0;
        } else {
            self.darkness += dt;
            self.empty = if r.lamp_charge <= 0.0 {
                self.empty + dt
            } else {
                0.0
            };
            // a dead flashlight (not merely a lamp switched off) runs its own, much shorter clock
            self.dead = if c.flashlight_dead { self.dead + dt } else { 0.0 };
        }
        match self.wake {
            Wake::None => {
                if self.empty >= rules.empty_delay {
                    self.wake = Wake::Empty;
                    self.just_woke = true;
                } else if c.flashlight_dead && self.dead >= rules.dead_delay {
                    self.wake = Wake::Dead;
                    self.just_woke = true;
                } else if !self.dead_only && self.darkness >= rules.timer_delay {
                    self.wake = Wake::Timer;
                    self.just_woke = true;
                }
            }
            // she is already out for the dead lamp; a lamp that stays dead ends in the inescapable catch (Godot's EMPTY)
            Wake::Dead if self.empty >= rules.empty_delay => {
                self.wake = Wake::Empty;
                self.just_woke = true;
            }
            _ => {}
        }
        if self.just_woke {
            // she unfolds out of the dark
            b.raise_cue("darkness_wake");
        }
        match self.wake {
            Wake::None => {
                b.active = false;
                b.state = State::Idle;
                return;
            }
            Wake::Empty => {
                b.active = true;
                b.pos = r.pos;
            }
            Wake::Timer | Wake::Dead => {
                if !b.active {
                    b.active = true;
                    let d = if self.wake == Wake::Dead { rules.dead_spawn_distance } else { rules.spawn_distance };
                    b.pos = spawn_near(c, r.pos, d);
                } else if lit && Brain::beam_reaches(c.fac, r.pos, r.facing, b.pos, rules.despawn_range, rules.dispel_half_angle) {
                    self.wake = Wake::None;
                    b.active = false;
                    b.state = State::Idle;
                    self.darkness = 0.0;
                    self.dead = 0.0;
                    return;
                }
            }
        }
        let gate = !pounce().nyxie_requires_dark || !r.flashlight_on;
        if pounce_frame(&mut self.leap, b, dt, c, t, gate) {
            return;
        }
        b.chase(t.pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> NightmarePounce {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../data/hunters/nightmare_pounce.json")).unwrap();
        NightmarePounce::parse(&v)
    }

    fn parse(text: &str) -> NightmarePounce {
        NightmarePounce::parse(&serde_json::from_str::<serde_json::Value>(text).unwrap())
    }

    #[test]
    fn file_matches_godot_nightmare_pounce() {
        // Godot game/hunters/ai/hard_abilities.gd POUNCE["nightmare"] (line 49): min 2.2, max 5.5, rate .56,
        // cooldown 18, per_chase 2, stumble .8, seconds .5; LEAP_SPEED 7.2, LEAD_SECONDS .15, BONK_STUN .8,
        // STUMBLE_SPEED .3 (lines 52-55). Nyxie and Solyth in ROSTER nightmare (line 26) and the gates in _pounce_allowed_for_hunter (line 214-218).
        let n = file();
        assert_eq!(n.numbers.min, 2.2);
        assert_eq!(n.numbers.max, 5.5);
        assert_eq!(n.numbers.rate, 0.56);
        assert_eq!(n.numbers.cooldown, 18.0);
        assert_eq!(n.numbers.per_chase, 2);
        assert_eq!(n.numbers.stumble, 0.8);
        assert_eq!(n.numbers.seconds, 0.5);
        assert_eq!(n.leap.leap_speed, 7.2);
        assert_eq!(n.leap.lead_seconds, 0.15);
        assert_eq!(n.leap.bonk_stun, 0.8);
        assert_eq!(n.leap.stumble_speed, 0.3);
        assert!(n.nyxie_requires_dark);
        assert_eq!(n.solyth_min_temperature, 60.0);
        assert_eq!(n.difficulties, vec!["nightmare".to_string()]);
    }

    #[test]
    fn default_matches_file() {
        assert_eq!(NightmarePounce::default(), file());
    }

    #[test]
    fn only_nightmare_has_the_pounce() {
        let n = file();
        assert!(n.applies(Difficulty::Nightmare));
        assert!(!n.applies(Difficulty::Relaxed));
        assert!(!n.applies(Difficulty::Standard));
        assert!(!n.applies(Difficulty::Hard));
    }

    #[test]
    fn changing_a_value_changes_the_rules() {
        let n = parse(r#"{"numbers": {"cooldown": 30.0, "per_chase": 1, "rate": 0.25}, "gates": {"nyxie_requires_dark": false, "solyth_min_temperature": 75.0}}"#);
        assert_eq!(n.numbers.cooldown, 30.0);
        assert_eq!(n.numbers.per_chase, 1);
        assert_eq!(n.numbers.rate, 0.25);
        assert!(!n.nyxie_requires_dark);
        assert_eq!(n.solyth_min_temperature, 75.0);
        // a missing value keeps the Godot default
        assert_eq!(n.numbers.min, 2.2);

        let mut leap = Leap::default();
        leap.count = 1;
        assert!(!leap.can_start(&n, 3.0, true), "per_chase 1 stops a second pounce");
    }

    #[test]
    fn distance_window_and_gate_decide_a_launch() {
        let n = file();
        let leap = Leap::default();
        assert!(!leap.can_start(&n, 2.1, true));
        assert!(leap.can_start(&n, 2.2, true));
        assert!(leap.can_start(&n, 5.5, true));
        assert!(!leap.can_start(&n, 5.6, true));
        assert!(!leap.can_start(&n, 3.0, false), "the hunter's own gate blocks it");
    }

    #[test]
    fn cooldown_and_stumble_block_a_launch() {
        let n = file();
        let mut leap = Leap::default();
        leap.cooldown = 1.0;
        assert!(!leap.can_start(&n, 3.0, true));
        leap.cooldown = 0.0;
        leap.stumble = 0.1;
        assert!(!leap.can_start(&n, 3.0, true));
        assert_eq!(leap.speed_scale(&n), n.leap.stumble_speed);
    }

    #[test]
    fn a_finished_leap_uses_the_data_cooldown_and_stumble() {
        let n = parse(r#"{"numbers": {"cooldown": 42.0, "stumble": 3.0}, "leap": {"stumble_speed": 0.5}}"#);
        let mut leap = Leap::default();
        leap.leaping = true;
        leap.finish(&n, false);
        assert!(!leap.leaping);
        assert_eq!(leap.cooldown, 42.0);
        assert_eq!(leap.stumble, 3.0);
        assert_eq!(leap.speed_scale(&n), 0.5);
        // a bonk (wall stop) sets the cooldown but no stumble
        let mut bonk = Leap::default();
        bonk.finish(&n, true);
        assert_eq!(bonk.cooldown, 42.0);
        assert_eq!(bonk.stumble, 0.0);
    }

    #[test]
    fn data_loader_falls_back_to_defaults_on_empty_json() {
        assert_eq!(parse("{}"), NightmarePounce::default());
    }

    #[test]
    fn file_matches_godot_budget_and_sweep() {
        // Godot hard_abilities.gd BUDGET nightmare 25.0 (line 17); hunter_brain.gd SWEEP_STEP .15 (line 34).
        let n = file();
        assert_eq!(n.budget_seconds, 25.0);
        assert_eq!(n.sweep_step, 0.15);
    }

    #[test]
    fn party_budget_blocks_a_second_pounce_inside_the_window() {
        // Godot _budget_ok: the first pounce of the party always passes; the next waits the full budget.
        assert!(budget_open(None, 0.0, 25.0));
        assert!(!budget_open(Some(0.0), 24.9, 25.0));
        assert!(budget_open(Some(0.0), 25.0, 25.0));
    }

    #[test]
    fn budget_seconds_from_data_change_the_window() {
        let n = parse(r#"{"budget": {"seconds": 10.0}}"#);
        assert_eq!(n.budget_seconds, 10.0);
        assert!(!budget_open(Some(0.0), 9.9, n.budget_seconds));
        assert!(budget_open(Some(0.0), 10.0, n.budget_seconds));
        // a missing value keeps the Godot default
        assert_eq!(parse("{}").budget_seconds, 25.0);
    }

    #[test]
    fn sweep_step_from_data_changes_the_sample_count() {
        // exact binary fractions so the counts do not depend on float rounding
        assert_eq!(sweep_samples(1.0, 0.25), 4);
        assert_eq!(sweep_samples(1.0, 0.5), 2);
        assert_eq!(sweep_samples(0.0, 0.25), 1);
        let n = parse(r#"{"sweep": {"step": 0.5}}"#);
        assert_eq!(n.sweep_step, 0.5);
        assert_eq!(sweep_samples(1.0, n.sweep_step), 2);
        // a zero or negative step is clamped, so the sweep never divides by zero
        assert!(parse(r#"{"sweep": {"step": 0.0}}"#).sweep_step >= 0.01);
    }

    // ------------------------------------------------------------------------------------------ wake rules

    fn wake_file() -> String {
        include_str!("../../data/hunters/nyxie_wake.json").to_string()
    }

    fn wake_with(edit: impl FnOnce(&mut serde_json::Value)) -> Result<WakeRules, String> {
        let mut v: serde_json::Value = serde_json::from_str(&wake_file()).unwrap();
        edit(&mut v);
        WakeRules::parse(&v.to_string())
    }

    #[test]
    fn the_wake_file_keeps_the_godot_numbers_and_adds_the_dead_flashlight_delay() {
        // Godot game/config/game_config.gd NYXIE_TIMER_DELAY 120, NYXIE_EMPTY_DELAY 10, NYXIE_DESPAWN_RANGE 7,
        // NYXIE_DISPEL_HALF_ANGLE .33, NYXIE_SPAWN_DISTANCE 10 (lines 356-360); the dead flashlight delay is the owner's 3 s.
        let r = wake_rules();
        assert_eq!(r.timer_delay, 120.0);
        assert_eq!(r.empty_delay, 10.0);
        assert_eq!(r.spawn_distance, 10.0);
        assert_eq!(r.despawn_range, 7.0);
        assert_eq!(r.dispel_half_angle, 0.33);
        assert_eq!(r.dead_delay, 3.0);
        assert_eq!(r.dead_spawn_distance, 10.0);
        assert_eq!(WakeRules::parse(&wake_file()).unwrap(), *r);
    }

    #[test]
    fn the_wake_loader_refuses_bad_files_with_a_reason() {
        assert!(wake_with(|v| v["schema"] = "lr.other/1".into()).unwrap_err().contains("schema"));
        assert!(wake_with(|v| v["bogus"] = 1.into()).unwrap_err().contains("unknown key"));
        assert!(wake_with(|v| { v.as_object_mut().unwrap().remove("dead_delay"); }).unwrap_err().contains("dead_delay"));
        assert!(wake_with(|v| v["dead_delay"] = "soon".into()).unwrap_err().contains("dead_delay"));
        assert!(wake_with(|v| v["dead_delay"] = (-1.0).into()).unwrap_err().contains("dead_delay"));
        assert!(wake_with(|v| v["timer_delay"] = 0.0.into()).unwrap_err().contains("timer_delay"));
        assert!(wake_with(|v| v["dispel_half_angle"] = 9.0.into()).unwrap_err().contains("dispel_half_angle"));
        assert!(wake_with(|v| v["dead_delay"] = 11.0.into()).unwrap_err().contains("must not exceed"), "dead_delay above empty_delay");
        assert!(WakeRules::parse("not json").is_err());
        assert!(WakeRules::parse("[]").is_err());
        // notes (keys starting with an underscore) and a changed value are fine
        let r = wake_with(|v| { v["_extra"] = "x".into(); v["dead_delay"] = 1.5.into(); }).unwrap();
        assert_eq!(r.dead_delay, 1.5);
    }

    #[test]
    fn the_activation_gate_switch_is_a_required_bool_in_the_wake_file() {
        assert!(wake_rules().ignore_activation_gate, "shipped on: a dead flashlight ignores the hunter activation count");
        assert!(wake_with(|v| { v.as_object_mut().unwrap().remove("ignore_activation_gate"); })
            .unwrap_err()
            .contains("ignore_activation_gate"));
        assert!(wake_with(|v| v["ignore_activation_gate"] = "yes".into()).unwrap_err().contains("ignore_activation_gate"));
        assert!(wake_with(|v| v["ignore_activation_gate"] = 1.into()).unwrap_err().contains("ignore_activation_gate"));
        assert!(!wake_with(|v| v["ignore_activation_gate"] = false.into()).unwrap().ignore_activation_gate);
        assert!(wake_with(|v| v["ignore_activation_gate"] = true.into()).unwrap().ignore_activation_gate);
    }

    // ------------------------------------------------------------------------------ dead flashlight (Brain::update)

    use crate::content::Content;
    use crate::world::catalog::Catalog;
    use crate::world::Cell;

    const DT: f32 = 1.0 / 30.0;

    fn ctx<'a>(fac: &'a Facility, diff: Difficulty, dead: bool) -> Ctx<'a> {
        Ctx {
            fac,
            relays_taken: &[],
            batteries_taken: &[],
            hiding_disabled: &[],
            vents: &[],
            heater_on: false,
            heater_thawed: true,
            diff,
            music_box_left: 0.0,
            music_boxes: &[],
            exposure: 0.0,
            flashlight_dead: dead,
        }
    }

    /// The runner at `at`: the lamp switched `on` with `charge` left.
    fn runner(at: (f32, f32), on: bool, charge: f32) -> Target {
        Target { pos: at, facing: (1.0, 0.0), flashlight_on: on, lamp_charge: charge, ..Default::default() }
    }

    fn sleeper(fac: &Facility, diff: Difficulty) -> Brain {
        let content = Content::load();
        Brain::new(content.hunter("nyxie").expect("nyxie").clone(), fac.start.center(), "NYXIE-DEAD", diff)
    }

    fn facility() -> Facility {
        Facility::generate("NYXIE-DEAD-WAKE", &Default::default(), &Catalog::load())
    }

    fn state(b: &Brain) -> (&Nyxie, bool) {
        let crate::hunters::gimmicks::Gimmick::Nyxie(n) = &b.gimmick else { panic!("not Nyxie") };
        (n, b.active)
    }

    /// Steps `seconds` of frames; returns the first elapsed time at which she was active.
    fn run(b: &mut Brain, c: &Ctx, t: &Target, diff: Difficulty, seconds: f32) -> Option<f32> {
        let prof = diff.profile();
        let mut first = None;
        let frames = (seconds / DT).round() as usize;
        for f in 0..frames {
            b.update(DT, c, t, 1, 0, &prof);
            if first.is_none() && b.active {
                first = Some((f + 1) as f32 * DT);
            }
        }
        first
    }

    #[test]
    fn a_dead_flashlight_wakes_her_within_the_delay_on_every_difficulty() {
        let fac = facility();
        let at = fac.start.center();
        let delay = wake_rules().dead_delay;
        for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
            let mut b = sleeper(&fac, diff);
            let c = ctx(&fac, diff, true);
            let t = runner(at, false, 0.0);
            let woke = run(&mut b, &c, &t, diff, delay + 0.5).unwrap_or_else(|| panic!("{diff:?}: a dead flashlight wakes her"));
            assert!(woke >= delay - 0.1, "{diff:?}: not before the delay ({woke} s)");
            assert!(woke <= delay + 0.2, "{diff:?}: within the delay ({woke} s)");
            let (n, active) = state(&b);
            assert!(active && n.wake == Wake::Dead, "{diff:?}: the dead-flashlight wake");
            assert!(!n.inescapable(), "{diff:?}: the first catch is a normal grab, not the 10 s lamp kill");
        }
    }

    #[test]
    fn she_appears_near_the_runner_not_two_minutes_later() {
        let fac = facility();
        let at = fac.start.center();
        let mut b = sleeper(&fac, Difficulty::Standard);
        let c = ctx(&fac, Difficulty::Standard, true);
        let t = runner(at, false, 0.0);
        // just before the delay she is still asleep
        run(&mut b, &c, &t, Difficulty::Standard, wake_rules().dead_delay - 0.3);
        assert!(!b.active && state(&b).0.wake == Wake::None);
        // the frame she wakes she stands where the wake puts her (a floor cell about dead_spawn_distance away)
        let prof = Difficulty::Standard.profile();
        let mut woke_at = None;
        for _ in 0..30 {
            b.update(DT, &c, &t, 1, 0, &prof);
            if state(&b).0.just_woke {
                woke_at = Some(b.pos);
                break;
            }
        }
        let p = woke_at.expect("woke");
        let d = crate::hunters::brain::dist(p, at);
        assert!((d - wake_rules().dead_spawn_distance).abs() < 2.0, "she appears about {} cells away, was {d}", wake_rules().dead_spawn_distance);
    }

    #[test]
    fn a_lamp_merely_switched_off_does_not_wake_her_early() {
        let fac = facility();
        let at = fac.start.center();
        let mut b = sleeper(&fac, Difficulty::Standard);
        // charged and off: not dead
        let c = ctx(&fac, Difficulty::Standard, false);
        let t = runner(at, false, 80.0);
        let woke = run(&mut b, &c, &t, Difficulty::Standard, 60.0);
        assert!(woke.is_none(), "a lamp that is off but charged leaves her asleep for the 120 s rule");
        let (n, active) = state(&b);
        assert!(!active && n.wake == Wake::None);
        assert_eq!(n.dead, 0.0, "the dead clock never ran");
        assert!(n.darkness > 59.0, "the 120 s darkness clock still runs");
    }

    #[test]
    fn the_two_minute_darkness_rule_still_wakes_her_when_the_lamp_is_off_but_charged() {
        let fac = facility();
        let at = fac.start.center();
        let mut b = sleeper(&fac, Difficulty::Standard);
        let c = ctx(&fac, Difficulty::Standard, false);
        let t = runner(at, false, 80.0);
        let woke = run(&mut b, &c, &t, Difficulty::Standard, 125.0).expect("120 s of darkness wakes her");
        assert!(woke >= 119.9 && woke <= 120.5, "woke at {woke}");
        assert!(state(&b).0.wake == Wake::Timer);
    }

    #[test]
    fn a_lit_lamp_keeps_her_away_and_a_short_dead_spell_resets() {
        let fac = facility();
        let at = fac.start.center();
        let mut b = sleeper(&fac, Difficulty::Standard);
        let dead = ctx(&fac, Difficulty::Standard, true);
        let ok = ctx(&fac, Difficulty::Standard, false);
        // dead for 2 s, then recharged and lit: the clock restarts, she never appears
        assert!(run(&mut b, &dead, &runner(at, false, 0.0), Difficulty::Standard, 2.0).is_none());
        assert!(run(&mut b, &ok, &runner(at, true, 80.0), Difficulty::Standard, 1.0).is_none());
        assert_eq!(state(&b).0.dead, 0.0);
        assert!(run(&mut b, &dead, &runner(at, false, 0.0), Difficulty::Standard, 2.0).is_none(), "the 2 s do not add up to the earlier 2 s");
        // an off-but-charged spell in between also resets the dead clock
        assert!(run(&mut b, &ok, &runner(at, false, 80.0), Difficulty::Standard, 1.0).is_none());
        assert_eq!(state(&b).0.dead, 0.0);
    }

    #[test]
    fn the_dead_flashlight_wake_does_not_trigger_twice() {
        let fac = facility();
        let at = fac.start.center();
        let mut b = sleeper(&fac, Difficulty::Standard);
        let c = ctx(&fac, Difficulty::Standard, true);
        let t = runner(at, false, 0.0);
        let prof = Difficulty::Standard.profile();
        let mut wakes = 0;
        let mut respawn_jumps = 0;
        let mut last = b.pos;
        // 7 s: past the 3 s delay, short of the 10 s lamp kill
        for _ in 0..(7.0 / DT) as usize {
            b.update(DT, &c, &t, 1, 0, &prof);
            if state(&b).0.just_woke {
                wakes += 1;
            }
            if b.active && crate::hunters::brain::dist(b.pos, last) > 2.0 && wakes > 0 && !state(&b).0.just_woke {
                respawn_jumps += 1;
            }
            last = b.pos;
        }
        assert_eq!(wakes, 1, "one wake for one dead spell");
        assert_eq!(respawn_jumps, 0, "she is not placed again while the flashlight stays dead");
        assert!(state(&b).0.wake == Wake::Dead);
    }

    #[test]
    fn a_flashlight_dead_for_ten_seconds_still_ends_in_the_lamp_kill() {
        let fac = facility();
        let at = fac.start.center();
        let mut b = sleeper(&fac, Difficulty::Standard);
        let c = ctx(&fac, Difficulty::Standard, true);
        let t = runner(at, false, 0.0);
        run(&mut b, &c, &t, Difficulty::Standard, 5.0);
        assert!(state(&b).0.wake == Wake::Dead && !state(&b).0.inescapable());
        run(&mut b, &c, &t, Difficulty::Standard, 6.0);
        let (n, active) = state(&b);
        assert!(active && n.wake == Wake::Empty && n.inescapable(), "the Godot EMPTY wake is the end of a long dead spell");
        assert_eq!(b.pos, at, "she is on top of the runner");
    }

    #[test]
    fn a_dead_flashlight_that_is_recharged_and_aimed_sends_her_back_in_place() {
        // the unit view of the beam rule: lit and facing her, a Dead wake is dispelled exactly like a Timer wake
        let fac = facility();
        // a runner with four open cells and a clear line east
        let from = fac
            .floor_cells()
            .into_iter()
            .find(|c| {
                (0..=4).all(|k| fac.hunter_open(Cell::new(c.x + k, c.y)))
                    && fac.has_line_of_sight(c.center(), Cell::new(c.x + 4, c.y).center())
            })
            .expect("a clear row of four cells");
        let at = from.center();
        let mut b = sleeper(&fac, Difficulty::Standard);
        let dead = ctx(&fac, Difficulty::Standard, true);
        run(&mut b, &dead, &runner(at, false, 0.0), Difficulty::Standard, 3.5);
        assert!(b.active && state(&b).0.wake == Wake::Dead);
        // she stands 4 cells ahead of the runner; the lamp is recharged and switched on, facing her
        b.pos = Cell::new(from.x + 4, from.y).center();
        let ok = ctx(&fac, Difficulty::Standard, false);
        let prof = Difficulty::Standard.profile();
        // a beam that is off or empty does nothing to her
        b.update(DT, &dead, &runner(at, false, 0.0), 1, 0, &prof);
        assert!(b.active, "no beam, no dispel");
        b.update(DT, &ok, &runner(at, true, 80.0), 1, 0, &prof);
        assert!(!b.active, "the recharged beam dispels the dead-flashlight wake");
        assert!(state(&b).0.wake == Wake::None);
    }

    // ------------------------------------------------------------------ the level-0 Nyxie (dead lamp only)

    /// The level-0 Nyxie (`dead_only`).
    fn dead_only() -> Brain {
        let fac = facility();
        let mut b = sleeper(&fac, Difficulty::Relaxed);
        if let crate::hunters::gimmicks::Gimmick::Nyxie(n) = &mut b.gimmick {
            n.dead_only = true;
        }
        b
    }

    #[test]
    fn the_level_zero_nyxie_wakes_for_a_dead_flashlight_and_the_lamp_kill_it_grows_into() {
        let fac = facility();
        let at = fac.start.center();
        let mut b = dead_only();
        let dead = ctx(&fac, Difficulty::Relaxed, true);
        let first = run(&mut b, &dead, &runner(at, false, 0.0), Difficulty::Relaxed, 4.0).expect("the dead lamp wakes her");
        assert!((first - wake_rules().dead_delay).abs() < 0.2, "after the dead delay: {first}");
        assert!(state(&b).0.wake == Wake::Dead);
        run(&mut b, &dead, &runner(at, false, 0.0), Difficulty::Relaxed, 8.0);
        assert!(state(&b).0.wake == Wake::Empty && state(&b).0.inescapable(), "the 10 s lamp kill follows");
    }

    #[test]
    fn the_level_zero_nyxie_has_no_darkness_wake_and_every_other_nyxie_keeps_hers() {
        let fac = facility();
        let at = fac.start.center();
        // a switched-off but charged lamp for 130 s: not a dead flashlight, only the dark
        let ok = ctx(&fac, Difficulty::Relaxed, false);
        let t = runner(at, false, 80.0);
        let mut level0 = dead_only();
        assert!(run(&mut level0, &ok, &t, Difficulty::Relaxed, 130.0).is_none(), "level 0 never had the 120 s rule");
        assert!(state(&level0).0.wake == Wake::None);
        // a floor's own Nyxie is unchanged: the 120 s rule wakes her
        let mut floor = sleeper(&fac, Difficulty::Relaxed);
        let first = run(&mut floor, &ok, &t, Difficulty::Relaxed, 125.0).expect("the 120 s rule");
        assert!((first - wake_rules().timer_delay).abs() < 0.2, "{first}");
        assert!(!state(&floor).0.dead_only);
    }
}

/// The beam dispel through the real frame (`Game::step`), not `before` alone.
#[cfg(test)]
mod beam_tests {
    use super::*;
    use crate::audio::Sounds;
    use crate::config::Difficulty;
    use crate::content::Content;
    use crate::game::{Controls, Game};
    use crate::world::catalog::Catalog;
    use crate::world::Cell;

    const FRAME: f32 = 1.0 / 60.0;

    /// A level-0 window, the runner at `from` facing `dir` (a unit step), and Nyxie awake on a timer wake `gap` cells
    /// ahead of her with open sight, hunting. The runner's lamp is lit and charged.
    fn stalked(level: i32, dir: (i32, i32), gap: i32) -> (Game, Content, Catalog) {
        stalked_on(Difficulty::Standard, level, dir, gap)
    }

    fn stalked_on(diff: Difficulty, level: i32, dir: (i32, i32), gap: i32) -> (Game, Content, Catalog) {
        let (content, catalog) = (Content::load(), Catalog::load());
        for n in 0..80 {
            let mut g = Game::new_infinite_sim(diff, 7, &format!("ROOM-NYXIE-BEAM-{n}"), level, &content, &catalog);
            g.fade = 0.0;
            let spot = g.fac.floor_cells().into_iter().find(|c| {
                (0..=gap + 1).all(|k| {
                    let (x, y) = Cell::new(c.x + dir.0 * k, c.y + dir.1 * k).center();
                    g.fac.hunter_open(Cell::new(c.x + dir.0 * k, c.y + dir.1 * k)) && g.fac.body_fits(x, y, HUNTER_RADIUS.max(crate::config::PLAYER_RADIUS))
                })
            });
            let Some(c) = spot else { continue };
            let from = c.center();
            let to = Cell::new(c.x + dir.0 * gap, c.y + dir.1 * gap).center();
            if !g.fac.has_line_of_sight(from, to) {
                continue;
            }
            g.hunters.clear();
            g.p.pos = from;
            // forward2() = (-sin yaw, -cos yaw)
            g.p.yaw = (-(dir.0 as f32)).atan2(-(dir.1 as f32));
            g.p.flashlight_on = true;
            g.p.lamp = 80.0;
            let mut b = Brain::new(content.hunter("nyxie").expect("nyxie").clone(), to, "NYXIE-BEAM", diff);
            b.active = true;
            b.state = State::Chase;
            b.level = g.runner_level();
            b.y = crate::game::level_stack::floor_y_of(b.level);
            if let crate::hunters::gimmicks::Gimmick::Nyxie(ny) = &mut b.gimmick {
                ny.wake = Wake::Timer;
                ny.darkness = 130.0;
            }
            g.hunters.push(b);
            return (g, content, catalog);
        }
        panic!("no window has a clear row of {gap} cells");
    }

    fn nyxie(g: &Game) -> &Brain {
        g.hunters.iter().find(|h| h.def.id == "nyxie").expect("nyxie is in the list")
    }

    fn run(g: &mut Game, content: &Content, catalog: &Catalog, frames: usize) {
        let sounds = Sounds::default();
        for _ in 0..frames {
            g.step(FRAME, &Controls::default(), content, catalog, &sounds);
        }
    }

    #[test]
    fn a_live_beam_sends_a_timer_wake_back_through_the_real_frame() {
        for dir in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (mut g, content, catalog) = stalked(0, dir, 5);
            assert!(nyxie(&g).active);
            run(&mut g, &content, &catalog, 60);
            let b = nyxie(&g);
            assert!(!b.active, "dir {dir:?}: the beam on her dispels a timer wake");
            assert_eq!(b.state, State::Idle, "dir {dir:?}");
            assert!(g.report.is_none() && g.escape.is_none(), "dir {dir:?}: she did not catch the runner first");
        }
    }

    #[test]
    fn the_beam_dispels_on_every_difficulty() {
        for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
            for gap in [3, 5, 6] {
                let (mut g, content, catalog) = stalked_on(diff, 0, (1, 0), gap);
                run(&mut g, &content, &catalog, 60);
                assert!(!nyxie(&g).active, "{diff:?} gap {gap}: dispelled");
            }
        }
    }

    #[test]
    fn the_same_beam_works_on_the_lower_levels_and_without_a_beam_nothing_happens() {
        for level in [-1, -2] {
            let (mut g, content, catalog) = stalked(level, (1, 0), 5);
            run(&mut g, &content, &catalog, 60);
            assert!(!nyxie(&g).active, "level {level}: dispelled");
        }
        // control: lamp off, the same wake stays
        let (mut g, content, catalog) = stalked(0, (1, 0), 5);
        g.p.flashlight_on = false;
        run(&mut g, &content, &catalog, 20);
        assert!(nyxie(&g).active || g.escape.is_some() || g.report.is_some(), "no beam, no dispel");
    }

    #[test]
    fn a_bot_beside_her_does_not_stop_the_runners_beam() {
        // Hard and Nightmare hunters chase a bot within reach (data/hunters/targets.json). Her target is then the bot,
        // but the dispel is the runner's beam (Godot: the target is always the runner).
        let (mut g, content, catalog) = stalked_on(Difficulty::Hard, 0, (1, 0), 5);
        let her = nyxie(&g).pos;
        let spot = (0..24)
            .map(|k| {
                let a = k as f32 * std::f32::consts::TAU / 24.0;
                (her.0 + 1.5 * a.cos(), her.1 + 1.5 * a.sin())
            })
            .find(|s| g.fac.body_fits(s.0, s.1, HUNTER_RADIUS) && g.fac.has_line_of_sight(her, *s))
            .expect("open floor beside her");
        let p = g.p.pos;
        g.npc_spawn(1, 0, (p.0, 1.5, p.1), (1.0, 0.0, 0.0));
        let bot = g.npcs.last_mut().expect("a bot");
        bot.at = spot;
        bot.y = crate::game::level_stack::floor_y_of(bot.level);
        run(&mut g, &content, &catalog, 60);
        assert!(!nyxie(&g).active, "the runner's beam dispels her while a bot is beside her");
    }

    #[test]
    fn a_full_dark_wake_then_the_flashlight_turned_on_and_aimed_sends_her_back() {
        use crate::hunters::gimmicks::Gimmick;
        let (content, catalog) = (Content::load(), Catalog::load());
        let sounds = Sounds::default();
        let mut g = Game::new_infinite_sim(Difficulty::Standard, 7, "ROOM-NYXIE-FULL", 0, &content, &catalog);
        g.fade = 0.0;
        g.hunters.clear();
        g.p.flashlight_on = false;
        let far = g.fac.floor_cells().into_iter().find(|c| crate::hunters::brain::dist(c.center(), g.p.pos) > 20.0).unwrap().center();
        let mut b = Brain::new(content.hunter("nyxie").unwrap().clone(), far, "NYXIE-FULL", Difficulty::Standard);
        b.active = false;
        g.hunters.push(b);
        let mut woke = None;
        for f in 0..7400 {
            g.step(FRAME, &Controls::default(), &content, &catalog, &sounds);
            if nyxie(&g).active {
                woke = Some(f);
                break;
            }
        }
        let woke = woke.expect("120 s of dark wakes her");
        assert!(woke >= 7190, "woke at frame {woke}");
        let Gimmick::Nyxie(n) = &nyxie(&g).gimmick else { panic!() };
        assert!(n.wake == Wake::Timer);
        // the runner switches the flashlight on and turns toward her
        let mut gone = None;
        for f in 0..180 {
            let hp = nyxie(&g).pos;
            let (dx, dy) = (hp.0 - g.p.pos.0, hp.1 - g.p.pos.1);
            g.p.yaw = (-dx).atan2(-dy);
            let c = Controls { flashlight: f == 0, ..Controls::default() };
            g.step(FRAME, &c, &content, &catalog, &sounds);
            if g.escape.is_some() || g.report.is_some() {
                break;
            }
            if !nyxie(&g).active {
                gone = Some(f);
                break;
            }
        }
        assert!(gone.is_some(), "flashlight on: lit {} lamp {} escape {} pos {:?} / {:?}", g.p.flashlight_on, g.p.lamp, g.escape.is_some(), nyxie(&g).pos, g.p.pos);
    }

    /// A sleeping Nyxie far from the runner on a fresh floor (no other hunters), the runner's lamp `lamp` charged and off.
    /// Level -1: that lineup keeps her on every difficulty. Level 0 has its own dormant Nyxie, tested below
    /// (`fresh_level0`).
    fn asleep_on(diff: Difficulty, lamp: f32) -> (Game, Content, Catalog) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(diff, 7, "ROOM-NYXIE-DEAD", -1, &content, &catalog);
        g.fade = 0.0;
        g.hunters.clear();
        g.p.flashlight_on = false;
        g.p.lamp = lamp;
        let far = g.fac.floor_cells().into_iter().find(|c| crate::hunters::brain::dist(c.center(), g.p.pos) > 20.0).unwrap().center();
        let mut b = Brain::new(content.hunter("nyxie").unwrap().clone(), far, "NYXIE-DEAD", diff);
        b.active = false;
        b.level = g.runner_level();
        b.y = crate::game::level_stack::floor_y_of(b.level);
        g.hunters.push(b);
        (g, content, catalog)
    }

    fn wake_of(g: &Game) -> Wake {
        let crate::hunters::gimmicks::Gimmick::Nyxie(n) = &nyxie(g).gimmick else { panic!("not Nyxie") };
        n.wake
    }

    #[test]
    fn a_dead_flashlight_brings_her_in_through_the_real_frame_on_every_difficulty() {
        for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
            let (mut g, content, catalog) = asleep_on(diff, 0.0);
            let sounds = Sounds::default();
            let delay = wake_rules().dead_delay;
            let mut woke = None;
            for f in 0..((delay + 1.0) / FRAME) as usize {
                g.step(FRAME, &Controls::default(), &content, &catalog, &sounds);
                if nyxie(&g).active {
                    woke = Some((f + 1) as f32 * FRAME);
                    break;
                }
            }
            let woke = woke.unwrap_or_else(|| panic!("{diff:?}: a dead flashlight wakes her"));
            assert!(woke >= delay - 0.1 && woke <= delay + 0.2, "{diff:?}: woke at {woke} s, delay {delay}");
            assert_eq!(wake_of(&g), Wake::Dead, "{diff:?}");
        }
    }

    #[test]
    fn a_charged_lamp_that_is_only_off_leaves_her_asleep_through_the_real_frame() {
        let (mut g, content, catalog) = asleep_on(Difficulty::Standard, 80.0);
        run(&mut g, &content, &catalog, 600);
        assert!(!nyxie(&g).active, "ten seconds of an off but charged lamp is not a dead flashlight");
        assert_eq!(wake_of(&g), Wake::None);
    }

    // ------------------------------------------------- level 0, before the hunter activation count (real frames)

    const ALL: [Difficulty; 4] = [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare];

    /// A new run on level 0 with no relay recovered, the runner's lamp `lamp` charged and switched off, and nothing else
    /// touched: the lineup is the one the run starts with.
    fn fresh_level0(diff: Difficulty, lamp: f32) -> (Game, Content, Catalog) {
        let (content, catalog) = (Content::load(), Catalog::load());
        let mut g = Game::new_infinite_sim(diff, 7, "ROOM-NYXIE-L0", 0, &content, &catalog);
        g.fade = 0.0;
        // the run starts in a safe room whose checkpoint tops the lamp up on the first frames: let that happen, then set the lamp
        run(&mut g, &content, &catalog, 20);
        g.p.flashlight_on = false;
        g.p.lamp = lamp;
        assert_eq!(g.relays_collected, 0);
        (g, content, catalog)
    }

    fn nyxies(g: &Game) -> Vec<&Brain> {
        g.hunters.iter().filter(|h| h.def.id == "nyxie").collect()
    }

    /// The hunters of the list that are not Nyxie (the route's own hunters).
    fn route_hunters(g: &Game) -> usize {
        g.hunters.iter().filter(|h| h.def.id != "nyxie").count()
    }

    #[test]
    fn level_zero_makes_no_nyxie_while_the_flashlight_works() {
        // the lineup of a run whose lamp never dies is unchanged: only the route's population, no extra hunter
        for diff in ALL {
            let (mut g, content, catalog) = fresh_level0(diff, 80.0);
            run(&mut g, &content, &catalog, 90);
            assert!(nyxies(&g).is_empty(), "{diff:?}: no Nyxie on level 0 until the flashlight dies");
            assert_eq!(route_hunters(&g), crate::game::endless::population(diff, 0) as usize, "{diff:?}");
        }
    }

    #[test]
    fn a_dead_flashlight_brings_her_in_on_level_zero_before_any_relay_on_every_difficulty() {
        // Relaxed needs 3 relays, Standard 1, Hard and Nightmare 0 before any route hunter is out; the dead lamp needs none.
        for diff in ALL {
            let (mut g, content, catalog) = fresh_level0(diff, 0.0);
            let sounds = Sounds::default();
            let delay = wake_rules().dead_delay;
            let mut woke = None;
            for f in 0..((delay + 1.0) / FRAME) as usize {
                g.step(FRAME, &Controls::default(), &content, &catalog, &sounds);
                if g.report.is_some() {
                    break;
                }
                if nyxies(&g).first().is_some_and(|b| b.active) {
                    woke = Some((f + 1) as f32 * FRAME);
                    break;
                }
            }
            let woke = woke.unwrap_or_else(|| panic!("{diff:?}: a dead flashlight wakes her on level 0 with no relay"));
            assert!(woke >= delay - 0.1 && woke <= delay + 0.2, "{diff:?}: woke at {woke} s, delay {delay}");
            assert_eq!(nyxies(&g).len(), 1, "{diff:?}: one Nyxie");
            assert_eq!(g.relays_collected, 0, "{diff:?}");
            assert_eq!(wake_of(&g), Wake::Dead, "{diff:?}");
            let d = crate::hunters::brain::dist(nyxie(&g).pos, g.p.pos);
            assert!(d <= wake_rules().dead_spawn_distance + 2.0, "{diff:?}: she appears about {} cells away, {d}", wake_rules().dead_spawn_distance);
            // the activation rule still holds every other hunter back
            assert_eq!(route_hunters(&g), crate::game::endless::population(diff, 0) as usize, "{diff:?}: no route hunter beyond the population");
        }
    }

    #[test]
    fn a_charged_lamp_that_is_only_off_never_brings_the_level_zero_nyxie() {
        // 40 s of an off but charged lamp: past the dead-lamp delay and the 10 s lamp kill, and no flashlight is dead
        for diff in ALL {
            let (mut g, content, catalog) = fresh_level0(diff, 80.0);
            let sounds = Sounds::default();
            for _ in 0..2400 {
                g.step(FRAME, &Controls::default(), &content, &catalog, &sounds);
                if g.report.is_some() {
                    break;
                }
            }
            assert!(nyxies(&g).is_empty(), "{diff:?}: a switched-off lamp is not a dead flashlight");
        }
    }

    #[test]
    fn the_route_hunters_stay_gated_while_a_dead_lamp_has_her_out() {
        let (mut g, content, catalog) = fresh_level0(Difficulty::Relaxed, 0.0);
        let sounds = Sounds::default();
        for _ in 0..(5.0 / FRAME) as usize {
            g.step(FRAME, &Controls::default(), &content, &catalog, &sounds);
        }
        assert!(nyxie(&g).active);
        assert_eq!(route_hunters(&g), 0, "Relaxed has no route hunter before the third relay");
        // the third relay brings the first one, and Nyxie is still the single special on the list
        g.relays_collected = 3;
        run(&mut g, &content, &catalog, 30);
        assert_eq!(route_hunters(&g), 1, "the third relay admits the first route hunter");
        assert_eq!(nyxies(&g).len(), 1);
    }

    #[test]
    fn a_beam_sends_the_level_zero_dead_lamp_wake_back_before_any_relay() {
        let (mut g, content, catalog) = fresh_level0(Difficulty::Relaxed, 0.0);
        run(&mut g, &content, &catalog, 240);
        assert!(nyxie(&g).active && wake_of(&g) == Wake::Dead, "out for the dead lamp");
        // a battery goes in and the lamp is aimed at her: she stands four cells off along a clear line
        let from = g.p.pos;
        let spot = (0..16)
            .map(|k| {
                let a = k as f32 * std::f32::consts::TAU / 16.0;
                (from.0 + 4.0 * a.cos(), from.1 + 4.0 * a.sin())
            })
            .find(|s| g.fac.body_fits(s.0, s.1, HUNTER_RADIUS) && g.fac.has_line_of_sight(from, *s))
            .expect("a clear spot four cells from the runner");
        let hi = g.hunters.iter().position(|h| h.def.id == "nyxie").unwrap();
        g.hunters[hi].pos = spot;
        g.p.lamp = 80.0;
        g.p.flashlight_on = true;
        g.p.yaw = (-(spot.0 - from.0)).atan2(-(spot.1 - from.1));
        run(&mut g, &content, &catalog, 60);
        assert!(g.report.is_none() && g.escape.is_none(), "she did not catch the runner first");
        assert!(!nyxie(&g).active, "the recharged beam dispels her");
        assert_eq!(wake_of(&g), Wake::None);
        // she stays one Nyxie: the lamp dying again wakes the same one, not a second
        g.p.lamp = 0.0;
        g.p.flashlight_on = false;
        run(&mut g, &content, &catalog, 240);
        assert_eq!(nyxies(&g).len(), 1);
        assert!(nyxie(&g).active, "the next dead lamp brings her again");
    }

    #[test]
    fn levels_below_zero_keep_their_ungated_nyxie() {
        // levels below 0 keep their always-present Nyxie, which was never held back by the activation count
        let (g, _, _) = asleep_on(Difficulty::Relaxed, 80.0);
        let crate::hunters::gimmicks::Gimmick::Nyxie(n) = &nyxie(&g).gimmick else { panic!() };
        assert!(!n.dead_only, "only the level-0 Nyxie is made for the dead lamp alone");
    }

    #[test]
    fn a_battery_then_the_beam_sends_the_dead_flashlight_wake_back_through_the_real_frame() {
        let (mut g, content, catalog) = stalked(0, (1, 0), 5);
        // the lamp dies (the game switches it off) and she is out for it
        g.p.lamp = 0.0;
        g.p.flashlight_on = false;
        if let crate::hunters::gimmicks::Gimmick::Nyxie(ny) = &mut g.hunters[0].gimmick {
            ny.wake = Wake::Dead;
            ny.darkness = 0.0;
        }
        run(&mut g, &content, &catalog, 6);
        assert!(nyxie(&g).active, "a dead lamp casts no beam");
        assert_eq!(wake_of(&g), Wake::Dead);
        // the runner puts a battery in and turns the lamp on, still facing her
        g.p.lamp = 80.0;
        g.p.flashlight_on = true;
        run(&mut g, &content, &catalog, 60);
        let b = nyxie(&g);
        assert!(!b.active, "the recharged beam dispels the dead-flashlight wake");
        assert_eq!(b.state, State::Idle);
        assert_eq!(wake_of(&g), Wake::None);
        assert!(g.report.is_none() && g.escape.is_none(), "she did not catch the runner first");
    }
}
