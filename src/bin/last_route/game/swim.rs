//! Swimmable sewer water, movement (Godot `game/player/player_controller_3d.gd`: `begin_plunge` / `_update_plunge`,
//! lines 1154-1229, and the wading speed by depth, lines 336-338, 365-372, 1237-1256, 1893). Pure state and speed
//! functions: the caller feeds them the water depth at the feet (`water_depth_at`) and applies the result.
//!
//! The plunge is the deliberate waterfall jump: an arc off the ledge to the pool (`Fall`), a muffled moment under
//! the surface (`Splash`), a slow swim to the walkway ring (`Swim`, `swim_speed`) and a climb out (`ClimbOut`), then
//! `soaked_seconds` of soaked. Collision is off for the duration; the caller ignores input while a `Plunge` runs.
//! Noise is only the figure hunters hear; the player's own danger readout, heartbeat and chase cue are not driven by
//! it, so bots and chased fake players never reach those cues from here.
//!
//! Every number is in data/swim/movement.json (`lr.swim.movement/1`), parsed once; defaults match the Godot values.

use serde_json::Value;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WadeRules {
    pub deep_depth: f32,
    pub max_flood_depth: f32,
    pub speed_scale: f32,
    pub tiring_depth: f32,
    pub stamina_drain: f32,
    pub noise: f32,
    pub deep_noise: f32,
    pub splash_every: f32,
    pub splash_gain: f32,
    pub soaked_floor_fraction: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlungeRules {
    pub hop_speed: f32,
    pub gravity: f32,
    pub min_drop: f32,
    pub splash_seconds: f32,
    pub swim_speed: f32,
    pub min_swim_seconds: f32,
    pub bob_amplitude: f32,
    pub bob_rate: f32,
    pub climb_out_seconds: f32,
    pub climb_out_hop: f32,
    pub splash_noise: f32,
    pub splash_noise_seconds: f32,
    pub swim_noise: f32,
    pub climb_out_noise: f32,
    pub soaked_seconds: f32,
    pub soaked_noise_scale: f32,
    /// Metres from the water surface down to the bed (Godot CHANNEL_DEPTH + WATER_SURFACE); the swim runs on the bed.
    pub channel_depth: f32,
    pub water_surface: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwimRules {
    pub wade: WadeRules,
    pub plunge: PlungeRules,
}

impl Default for SwimRules {
    fn default() -> Self {
        SwimRules {
            wade: WadeRules {
                deep_depth: 0.9,
                max_flood_depth: 0.35,
                speed_scale: 0.45,
                tiring_depth: 0.6,
                stamina_drain: 6.0,
                noise: 12.0,
                deep_noise: 17.0,
                splash_every: 0.42,
                splash_gain: 0.6,
                soaked_floor_fraction: 0.5,
            },
            plunge: PlungeRules {
                hop_speed: 1.6,
                gravity: 9.8,
                min_drop: 0.5,
                splash_seconds: 0.55,
                swim_speed: 1.5,
                min_swim_seconds: 2.0,
                bob_amplitude: 0.04,
                bob_rate: 3.0,
                climb_out_seconds: 0.6,
                climb_out_hop: 0.12,
                splash_noise: 3.0,
                splash_noise_seconds: 1.2,
                swim_noise: 0.8,
                climb_out_noise: 0.3,
                soaked_seconds: 30.0,
                soaked_noise_scale: 1.45,
                channel_depth: 0.0,
                water_surface: 0.0,
            },
        }
    }
}

fn num(o: &Value, k: &str, d: f32) -> f32 {
    o.get(k).and_then(Value::as_f64).map(|x| x as f32).unwrap_or(d)
}

/// A strictly positive finite value, else the default.
fn pos(o: &Value, k: &str, d: f32) -> f32 {
    let x = num(o, k, d);
    if x.is_finite() && x > 0.0 {
        x
    } else {
        d
    }
}

/// A non-negative finite value, else the default.
fn nn(o: &Value, k: &str, d: f32) -> f32 {
    let x = num(o, k, d);
    if x.is_finite() && x >= 0.0 {
        x
    } else {
        d
    }
}

impl SwimRules {
    /// Reads the rules from parsed JSON; a missing or wrong-typed field keeps its default. Values are kept sane
    /// (non-negative; gravity, speeds and durations above zero) so a bad edit cannot divide by zero.
    pub fn parse(v: &Value) -> SwimRules {
        let w = &v["wade"];
        let p = &v["plunge"];
        let d = SwimRules::default();
        SwimRules {
            wade: WadeRules {
                deep_depth: pos(w, "deep_depth", d.wade.deep_depth),
                max_flood_depth: pos(w, "max_flood_depth", d.wade.max_flood_depth),
                speed_scale: nn(w, "speed_scale", d.wade.speed_scale).min(1.0),
                tiring_depth: nn(w, "tiring_depth", d.wade.tiring_depth),
                stamina_drain: nn(w, "stamina_drain", d.wade.stamina_drain),
                noise: nn(w, "noise", d.wade.noise),
                deep_noise: nn(w, "deep_noise", d.wade.deep_noise),
                splash_every: pos(w, "splash_every", d.wade.splash_every),
                splash_gain: nn(w, "splash_gain", d.wade.splash_gain),
                soaked_floor_fraction: nn(w, "soaked_floor_fraction", d.wade.soaked_floor_fraction).min(1.0),
            },
            plunge: PlungeRules {
                hop_speed: nn(p, "hop_speed", d.plunge.hop_speed),
                gravity: pos(p, "gravity", d.plunge.gravity),
                min_drop: nn(p, "min_drop", d.plunge.min_drop),
                splash_seconds: pos(p, "splash_seconds", d.plunge.splash_seconds),
                swim_speed: pos(p, "swim_speed", d.plunge.swim_speed),
                min_swim_seconds: pos(p, "min_swim_seconds", d.plunge.min_swim_seconds),
                bob_amplitude: nn(p, "bob_amplitude", d.plunge.bob_amplitude),
                bob_rate: nn(p, "bob_rate", d.plunge.bob_rate),
                climb_out_seconds: pos(p, "climb_out_seconds", d.plunge.climb_out_seconds),
                climb_out_hop: nn(p, "climb_out_hop", d.plunge.climb_out_hop),
                splash_noise: nn(p, "splash_noise", d.plunge.splash_noise),
                splash_noise_seconds: nn(p, "splash_noise_seconds", d.plunge.splash_noise_seconds),
                swim_noise: nn(p, "swim_noise", d.plunge.swim_noise),
                climb_out_noise: nn(p, "climb_out_noise", d.plunge.climb_out_noise),
                soaked_seconds: nn(p, "soaked_seconds", d.plunge.soaked_seconds),
                soaked_noise_scale: nn(p, "soaked_noise_scale", d.plunge.soaked_noise_scale),
                channel_depth: nn(p, "channel_depth", d.plunge.channel_depth),
                water_surface: nn(p, "water_surface", d.plunge.water_surface),
            },
        }
    }
}

/// The embedded rules (data/swim/movement.json); the defaults when the file is missing or broken.
pub fn rules() -> &'static SwimRules {
    static RULES: OnceLock<SwimRules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("swim/movement.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
            .map(|v| SwimRules::parse(&v))
            .unwrap_or_default()
    })
}

/// Godot `_smooth`: smoothstep on 0..1.
fn smooth(k: f32) -> f32 {
    let k = k.clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

// ---------------------------------------------------------------------------------------------------------------
// Wading, by depth

/// The speed multiplier for walking in water `depth` metres deep: 1 when dry, `speed_scale` at `deep_depth` and
/// beyond (Godot line 1893: `lerpf(1, WADE_SPEED_SCALE, clamp(depth / DEEP_DEPTH, 0, 1))`).
pub fn wade_speed_scale(rules: &WadeRules, depth: f32) -> f32 {
    if !(depth > 0.0) {
        return 1.0;
    }
    lerp(1.0, rules.speed_scale, (depth / rules.deep_depth).clamp(0.0, 1.0))
}

/// The noise figure of a wading step at `depth` (Godot `_update_wading`): `noise` scaled by how deep the flood is up
/// to its normal maximum, blending to `deep_noise` between `max_flood_depth` and `deep_depth`. 0 when dry.
pub fn wade_noise(rules: &WadeRules, depth: f32) -> f32 {
    if !(depth > 0.0) {
        return 0.0;
    }
    let span = (rules.deep_depth - rules.max_flood_depth).max(1e-4);
    let deep = ((depth - rules.max_flood_depth) / span).clamp(0.0, 1.0);
    lerp(rules.noise * (depth / rules.max_flood_depth).clamp(0.5, 1.0), rules.deep_noise, deep)
}

/// Stamina lost over `dt` seconds of moving in `depth` of water (0 under `tiring_depth` or when dry).
pub fn wade_stamina_cost(rules: &WadeRules, depth: f32, dt: f32) -> f32 {
    if depth > 0.0 && depth >= rules.tiring_depth {
        rules.stamina_drain * dt.max(0.0)
    } else {
        0.0
    }
}

/// The soaked seconds a wader is held at least at while in water (any wet foot).
pub fn wade_soaked_floor(wade: &WadeRules, plunge: &PlungeRules, depth: f32) -> f32 {
    if depth > 0.0 {
        plunge.soaked_seconds * wade.soaked_floor_fraction
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The plunge

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlungePhase {
    Fall,
    Splash,
    Swim,
    ClimbOut,
}

/// What one `Plunge::step` asks of the caller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlungeFrame {
    /// Where the feet are now: x, y (floor or bed height), z.
    pub pos: [f32; 3],
    /// The noise hunters hear this frame.
    pub noise: f32,
    pub phase: PlungePhase,
    /// True on the one frame the body hits the water: play the big splash, muffle sound, flicker the light, and
    /// emit a noise burst of `splash_noise` for `splash_noise_seconds`.
    pub splashed: bool,
    /// True while sound is muffled (from the splash until the swim ends).
    pub muffled: bool,
    /// True on the one frame the swim ends and sound is unmuffled.
    pub unmuffled: bool,
    /// True on the last frame: restore collision, set soaked to `soaked_seconds`; the storey changes by itself as
    /// the feet cross it (direct, no fade, no halt).
    pub finished: bool,
}

/// One waterfall jump in progress. Build with `Plunge::begin`, call `step` every frame until `finished`.
#[derive(Clone, Copy, Debug)]
pub struct Plunge {
    rules: PlungeRules,
    phase: PlungePhase,
    t: f32,
    from: [f32; 3],
    land: [f32; 3],
    exit: [f32; 3],
    surface: f32,
    bed: f32,
    fall_seconds: f32,
    swim_seconds: f32,
    swim_end: [f32; 3],
    pos: [f32; 3],
    muffled: bool,
}

impl Plunge {
    /// `begin_plunge`: from the ledge `from`, an arc to the pool at `land` (water surface at `surface_y`), a swim to
    /// `exit` on the walkway ring.
    pub fn begin(rules: &PlungeRules, from: [f32; 3], land: [f32; 3], exit: [f32; 3], surface_y: f32) -> Plunge {
        let drop = (from[1] - surface_y).max(rules.min_drop);
        let fall_seconds =
            (rules.hop_speed + (rules.hop_speed * rules.hop_speed + 2.0 * rules.gravity * drop).sqrt()) / rules.gravity;
        let bed = surface_y - rules.channel_depth - rules.water_surface;
        let dist = ((land[0] - exit[0]).powi(2) + (land[2] - exit[2]).powi(2)).sqrt();
        Plunge {
            rules: *rules,
            phase: PlungePhase::Fall,
            t: 0.0,
            from,
            land,
            exit,
            surface: surface_y,
            bed,
            fall_seconds,
            swim_seconds: (dist / rules.swim_speed).max(rules.min_swim_seconds),
            swim_end: land,
            pos: from,
            muffled: false,
        }
    }

    pub fn phase(&self) -> PlungePhase {
        self.phase
    }

    pub fn pos(&self) -> [f32; 3] {
        self.pos
    }

    /// Seconds the fall lasts (from the drop height).
    pub fn fall_seconds(&self) -> f32 {
        self.fall_seconds
    }

    /// Seconds the swim lasts (distance over `swim_speed`, at least `min_swim_seconds`).
    pub fn swim_seconds(&self) -> f32 {
        self.swim_seconds
    }

    /// `_update_plunge`: advances by `dt` seconds. The caller ignores input while a plunge runs.
    pub fn step(&mut self, dt: f32) -> PlungeFrame {
        let r = self.rules;
        self.t += dt.max(0.0);
        let (mut splashed, mut unmuffled, mut finished) = (false, false, false);
        let mut noise = 0.0;
        match self.phase {
            PlungePhase::Fall => {
                let k = (self.t / self.fall_seconds).clamp(0.0, 1.0);
                let y = self.from[1] + r.hop_speed * self.t - 0.5 * r.gravity * self.t * self.t;
                self.pos = [lerp(self.from[0], self.land[0], k), y.max(self.surface), lerp(self.from[2], self.land[2], k)];
                if self.t >= self.fall_seconds {
                    self.phase = PlungePhase::Splash;
                    self.t = 0.0;
                    self.muffled = true;
                    splashed = true;
                    noise = r.splash_noise;
                }
            }
            PlungePhase::Splash => {
                let k = (self.t / r.splash_seconds).clamp(0.0, 1.0);
                self.pos = [self.land[0], lerp(self.surface, self.bed, smooth(k)), self.land[2]];
                noise = r.splash_noise;
                if self.t >= r.splash_seconds {
                    self.phase = PlungePhase::Swim;
                    self.t = 0.0;
                    // This frame is reported as Swim, so it carries the swim noise, not the splash burst.
                    noise = r.swim_noise;
                }
            }
            PlungePhase::Swim => {
                let k = smooth(self.t / self.swim_seconds);
                let bob = (self.t * r.bob_rate).sin() * r.bob_amplitude;
                self.pos = [lerp(self.land[0], self.exit[0], k), self.bed + bob, lerp(self.land[2], self.exit[2], k)];
                noise = r.swim_noise;
                if self.t >= self.swim_seconds {
                    self.phase = PlungePhase::ClimbOut;
                    self.t = 0.0;
                    self.swim_end = self.pos;
                    self.muffled = false;
                    unmuffled = true;
                }
            }
            PlungePhase::ClimbOut => {
                let k = (self.t / r.climb_out_seconds).clamp(0.0, 1.0);
                let s = smooth(k);
                let hop = (k * std::f32::consts::PI).sin() * r.climb_out_hop;
                self.pos = [
                    lerp(self.swim_end[0], self.exit[0], s),
                    lerp(self.swim_end[1], self.exit[1], s) + hop,
                    lerp(self.swim_end[2], self.exit[2], s),
                ];
                noise = r.climb_out_noise;
                if self.t >= r.climb_out_seconds {
                    finished = true;
                    self.pos = self.exit;
                }
            }
        }
        PlungeFrame { pos: self.pos, noise, phase: self.phase, splashed, muffled: self.muffled, unmuffled, finished }
    }
}

/// Footstep noise while soaked (hunters hear a wet runner from further away).
pub fn soaked_noise(rules: &PlungeRules, base_noise: f32, soaked_left: f32) -> f32 {
    if soaked_left > 0.0 {
        base_noise * rules.soaked_noise_scale
    } else {
        base_noise
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standard() -> SwimRules {
        SwimRules::default()
    }

    #[test]
    fn embedded_data_matches_godot_standard() {
        assert_eq!(*rules(), standard());
        assert_eq!(rules().plunge.swim_speed, 1.5);
        assert_eq!(rules().wade.speed_scale, 0.45);
        assert_eq!(rules().plunge.soaked_seconds, 30.0);
    }

    #[test]
    fn changed_data_changes_behaviour() {
        let v = serde_json::json!({ "wade": { "speed_scale": 0.2 }, "plunge": { "swim_speed": 3.0 } });
        let r = SwimRules::parse(&v);
        assert!(wade_speed_scale(&r.wade, 0.9) < wade_speed_scale(&standard().wade, 0.9));
        let p = Plunge::begin(&r.plunge, [0.0, 3.0, 0.0], [0.0; 3], [30.0, 0.0, 0.0], 0.0);
        let q = Plunge::begin(&standard().plunge, [0.0, 3.0, 0.0], [0.0; 3], [30.0, 0.0, 0.0], 0.0);
        assert!(p.swim_seconds() < q.swim_seconds());
    }

    #[test]
    fn bad_data_falls_back() {
        let v = serde_json::json!({ "plunge": { "gravity": 0.0, "swim_speed": -1 }, "wade": { "deep_depth": "x" } });
        assert_eq!(SwimRules::parse(&v), standard());
    }

    #[test]
    fn wading_slows_with_depth() {
        let w = standard().wade;
        assert_eq!(wade_speed_scale(&w, 0.0), 1.0);
        let shin = wade_speed_scale(&w, 0.3);
        let knee = wade_speed_scale(&w, 0.9);
        assert!(shin < 1.0 && knee < shin);
        assert!((knee - w.speed_scale).abs() < 1e-5);
        assert_eq!(wade_speed_scale(&w, 5.0), knee);
    }

    #[test]
    fn wading_noise_and_tiring() {
        let w = standard().wade;
        assert_eq!(wade_noise(&w, 0.0), 0.0);
        assert!(wade_noise(&w, 0.35) >= wade_noise(&w, 0.1));
        assert!(wade_noise(&w, 0.9) > wade_noise(&w, 0.35));
        assert_eq!(wade_stamina_cost(&w, 0.3, 1.0), 0.0);
        assert!(wade_stamina_cost(&w, 0.7, 1.0) > 0.0);
        let p = standard().plunge;
        assert_eq!(wade_soaked_floor(&w, &p, 0.0), 0.0);
        assert!(wade_soaked_floor(&w, &p, 0.1) > 0.0);
    }

    fn run(p: &mut Plunge) -> (Vec<PlungePhase>, f32, i32, i32) {
        let (mut phases, mut t, mut splashes, mut unmuffles) = (vec![], 0.0, 0, 0);
        loop {
            let f = p.step(1.0 / 60.0);
            t += 1.0 / 60.0;
            if phases.last() != Some(&f.phase) {
                phases.push(f.phase);
            }
            splashes += f.splashed as i32;
            unmuffles += f.unmuffled as i32;
            if f.finished {
                return (phases, t, splashes, unmuffles);
            }
            assert!(t < 120.0, "plunge never finished");
        }
    }

    #[test]
    fn plunge_runs_all_phases_and_lands_on_exit() {
        let r = standard().plunge;
        let exit = [8.0, 0.4, 2.0];
        let mut p = Plunge::begin(&r, [0.0, 3.0, 0.0], [1.0, 0.0, 0.0], exit, 0.0);
        let (phases, t, splashes, unmuffles) = run(&mut p);
        assert_eq!(phases, vec![PlungePhase::Fall, PlungePhase::Splash, PlungePhase::Swim, PlungePhase::ClimbOut]);
        assert_eq!((splashes, unmuffles), (1, 1));
        assert_eq!(p.pos(), exit);
        assert!(t > 5.0 && t < 15.0, "t = {t}");
    }

    #[test]
    fn swim_has_minimum_and_scales_with_distance() {
        let r = standard().plunge;
        let near = Plunge::begin(&r, [0.0, 2.0, 0.0], [0.0; 3], [0.5, 0.0, 0.0], 0.0);
        let far = Plunge::begin(&r, [0.0, 2.0, 0.0], [0.0; 3], [15.0, 0.0, 0.0], 0.0);
        assert_eq!(near.swim_seconds(), r.min_swim_seconds);
        assert!((far.swim_seconds() - 15.0 / r.swim_speed).abs() < 1e-4);
    }

    #[test]
    fn fall_stays_above_surface_and_muffle_spans_splash_to_swim_end() {
        let r = standard().plunge;
        let mut p = Plunge::begin(&r, [0.0, 4.0, 0.0], [2.0, 0.0, 0.0], [6.0, 0.0, 0.0], 0.0);
        let mut seen_muffled = false;
        for _ in 0..2000 {
            let f = p.step(1.0 / 30.0);
            match f.phase {
                PlungePhase::Fall => assert!(f.pos[1] >= 0.0 && !f.muffled),
                PlungePhase::Splash | PlungePhase::Swim => {
                    assert!(f.muffled);
                    seen_muffled = true;
                }
                PlungePhase::ClimbOut => assert!(!f.muffled),
            }
            if f.finished {
                break;
            }
        }
        assert!(seen_muffled);
    }

    #[test]
    fn swim_is_quieter_than_splash() {
        let r = standard().plunge;
        let mut p = Plunge::begin(&r, [0.0, 3.0, 0.0], [0.0; 3], [6.0, 0.0, 0.0], 0.0);
        let (mut splash, mut swim) = (0.0f32, 0.0f32);
        loop {
            let f = p.step(1.0 / 60.0);
            match f.phase {
                PlungePhase::Splash => splash = splash.max(f.noise),
                PlungePhase::Swim => swim = swim.max(f.noise),
                _ => {}
            }
            if f.finished {
                break;
            }
        }
        assert!(splash > swim && swim > 0.0);
    }

    #[test]
    fn soaked_makes_steps_louder() {
        let r = standard().plunge;
        assert!(soaked_noise(&r, 10.0, 5.0) > 10.0);
        assert_eq!(soaked_noise(&r, 10.0, 0.0), 10.0);
    }
}
