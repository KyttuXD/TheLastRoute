//! Nyxie (special): darkness stalker. 120 s without light wakes her 10 cells away and she hunts
//! you until the beam catches her (instantly dispelled within 7 cells). A dead lamp for 10 s
//! wakes her on top of you: an inescapable catch unless you are hidden or safe
//! (nyxie_gimmick.gd).
//!
//! Nightmare only: while she hunts in the dark she pounces (hard_abilities.gd pounce, Nightmare
//! row). The pounce numbers, the leap and the gates are data in data/hunters/nightmare_pounce.json.
//! Solyth uses the same leap (solyth.rs).

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use super::Ctx;
use crate::config::{Difficulty, HUNTER_RADIUS};
use crate::hunters::brain::{dist, Brain, State, Target};
use crate::world::Facility;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Wake {
    #[default]
    None,
    Timer,
    Empty,
}

#[derive(Default)]
pub struct Nyxie {
    pub wake: Wake,
    darkness: f32,
    empty: f32,
    /// Set the frame she wakes (for the sound cue).
    pub just_woke: bool,
    leap: Leap,
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

    pub fn speed_scale(&self) -> f32 {
        self.leap.speed_scale(pounce())
    }

    /// The leap holds her position (the leap moves her itself).
    pub fn holds_position(&self) -> bool {
        self.leap.leaping
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) {
        self.just_woke = false;
        let lit = t.flashlight_on && t.lamp_charge > 0.0;
        if lit {
            self.darkness = 0.0;
            self.empty = 0.0;
        } else {
            self.darkness += dt;
            self.empty = if t.lamp_charge <= 0.0 {
                self.empty + dt
            } else {
                0.0
            };
        }
        if self.wake == Wake::None {
            if self.empty >= 10.0 {
                self.wake = Wake::Empty;
                self.just_woke = true;
            } else if self.darkness >= 120.0 {
                self.wake = Wake::Timer;
                self.just_woke = true;
            }
        }
        match self.wake {
            Wake::None => {
                b.active = false;
                b.state = State::Idle;
                return;
            }
            Wake::Empty => {
                b.active = true;
                b.pos = t.pos;
            }
            Wake::Timer => {
                if !b.active {
                    b.active = true;
                    b.pos = spawn_near(c, t.pos, 10.0);
                } else if lit && Brain::beam_reaches(c.fac, t.pos, t.facing, b.pos, 7.0, 0.33) {
                    self.wake = Wake::None;
                    b.active = false;
                    b.state = State::Idle;
                    self.darkness = 0.0;
                    return;
                }
            }
        }
        let gate = !pounce().nyxie_requires_dark || !t.flashlight_on;
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
}
