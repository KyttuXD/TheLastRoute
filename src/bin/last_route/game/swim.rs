//! Swimmable sewer water, movement (Godot `game/player/player_controller_3d.gd`: the wading speed by depth, lines
//! 336-338, 365-372, 1237-1256, 1893) and the landing rule of a fall. Pure state and speed functions: the caller
//! feeds them the water depth at the feet (`water_depth_at`) and applies the result.
//!
//! There is no scripted waterfall plunge any more: the runner falls like anywhere else (`Player::falling`) and
//! `Game::apply_landing` applies `FallRules` (data/swim/fall.json) at the touchdown, a splash in the pool or a
//! stun on the ground. In the pool the runner then swims freely with the wade rules.
//! Noise is only the figure hunters hear; the player's own danger readout, heartbeat and chase cue are not driven by
//! it, so bots and chased fake players never reach those cues from here.
//!
//! Every number is in data/swim/movement.json (`lr.swim.movement/1`) and data/swim/fall.json (`lr.swim.fall/1`),
//! parsed once; defaults match the Godot values.

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
    pub splash_noise: f32,
    pub splash_noise_seconds: f32,
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
                splash_gain: 0.3,
                soaked_floor_fraction: 0.5,
            },
            plunge: PlungeRules {
                splash_noise: 3.0,
                splash_noise_seconds: 1.2,
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
    /// (non-negative; durations above zero) so a bad edit cannot divide by zero.
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
                splash_noise: nn(p, "splash_noise", d.plunge.splash_noise),
                splash_noise_seconds: nn(p, "splash_noise_seconds", d.plunge.splash_noise_seconds),
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

// ---------------------------------------------------------------------------------------------------------------
// The landing rule of a fall (data/swim/fall.json, `lr.swim.fall/1`)

/// What a landing in deep enough water does: a splash and a soak, never a stun.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PoolLanding {
    /// Water at least this deep (metres) takes the body.
    pub min_depth: f32,
    pub splash_gain: f32,
    pub noise: f32,
    pub noise_seconds: f32,
    pub flicker_seconds: f32,
    pub soak: bool,
    /// The chance (0 to 1) that the item in hand slips away into the pool on the landing (Godot `FALLS_ITEM_DROP_CHANCE`
    /// .08, infinite_route_test.gd:58).
    pub item_slip_chance: f32,
}

/// What a landing on dry floor does. There are no hit points: a stun, a stamina loss, exposure and a noise burst. The
/// stun is Godot's hard-landing curve of the touchdown speed (`PLAYER_FALL_STUN_SPEED` 8.25 m/s up to
/// `PLAYER_FALL_STUN_FULL_SPEED` 14 m/s mapped to 0.4 to 1.4 s, game_config.gd:107-110) and is none under the first speed;
/// the stamina loss, exposure and noise are scaled by `(drop_height / STOREY).clamp(scale_min, scale_max)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundLanding {
    /// The touchdown speed (m/s) from which a landing stuns (for `stun_min_seconds`); slower is no stun.
    pub stun_from_speed: f32,
    /// The touchdown speed (m/s) at and above which the stun is `stun_max_seconds`.
    pub stun_full_speed: f32,
    pub stun_min_seconds: f32,
    pub stun_max_seconds: f32,
    pub stamina_loss: f32,
    pub exposure_add: f32,
    pub noise: f32,
    pub noise_seconds: f32,
    pub flicker_seconds: f32,
    pub scale_min: f32,
    pub scale_max: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FallRules {
    /// A fall shorter than this (metres) has no landing effect.
    pub min_height: f32,
    /// The share of the ground speed the body keeps while it falls (steering in the air).
    pub air_control: f32,
    /// A drop shorter than this (metres) is a step the height eases down; a longer one is a fall.
    pub min_gap: f32,
    /// A fall whose drop is at least this (metres) reports a `Landing` (the game then applies `min_height` and the rest).
    pub landing_report_gap: f32,
    pub pool: PoolLanding,
    pub ground: GroundLanding,
}

/// The text on a stunning hard landing (Godot `HARD LANDING // STUNNED %.1fs`, player_controller_3d.gd:2126); the stun
/// seconds follow it.
pub const HARD_LANDING_MESSAGE: &str = "HARD LANDING // STUNNED";

/// The shipped file, the single source of the standard values.
const FALL_JSON: &str = include_str!("../data/swim/fall.json");

impl Default for FallRules {
    /// The standard values: the embedded data/swim/fall.json, parsed (there is no second copy of the numbers). The shipped
    /// file is checked by the tests, so a refusal here is a broken build.
    fn default() -> Self {
        serde_json::from_str::<Value>(FALL_JSON)
            .map_err(|e| e.to_string())
            .and_then(|v| FallRules::parse(&v))
            .expect("the embedded data/swim/fall.json is valid")
    }
}

/// A finite number in `lo..=hi` at `section.key`, or the reason it is refused.
fn limited(v: &Value, section: &str, key: &str, lo: f32, hi: f32) -> Result<f32, String> {
    let x = v
        .get(section)
        .and_then(|s| s.get(key))
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("fall.json: {section}.{key} is missing or not a number"))? as f32;
    if x.is_finite() && x >= lo && x <= hi {
        Ok(x)
    } else {
        Err(format!("fall.json: {section}.{key} = {x} is outside {lo}..={hi}"))
    }
}

impl FallRules {
    /// Reads and checks a fall file. Every field must be present and inside its limits; a bad file is refused whole.
    pub fn parse(v: &Value) -> Result<FallRules, String> {
        if v.get("schema").and_then(Value::as_str) != Some("lr.swim.fall/1") {
            return Err("fall.json: schema must be lr.swim.fall/1".into());
        }
        let top = |k: &str, lo: f32, hi: f32| -> Result<f32, String> {
            let x = v.get(k).and_then(Value::as_f64).ok_or_else(|| format!("fall.json: {k} is missing or not a number"))? as f32;
            if x.is_finite() && x >= lo && x <= hi {
                Ok(x)
            } else {
                Err(format!("fall.json: {k} = {x} is outside {lo}..={hi}"))
            }
        };
        let soak = v
            .pointer("/pool/soak")
            .and_then(Value::as_bool)
            .ok_or_else(|| "fall.json: pool.soak is missing or not a boolean".to_string())?;
        let rules = FallRules {
            min_height: top("min_height", 0.0, 100.0)?,
            air_control: top("air_control", 0.0, 1.0)?,
            min_gap: top("min_gap", 0.01, 10.0)?,
            landing_report_gap: top("landing_report_gap", 0.0, 100.0)?,
            pool: PoolLanding {
                min_depth: limited(v, "pool", "min_depth", 0.01, 10.0)?,
                splash_gain: limited(v, "pool", "splash_gain", 0.0, 4.0)?,
                noise: limited(v, "pool", "noise", 0.0, 100.0)?,
                noise_seconds: limited(v, "pool", "noise_seconds", 0.0, 30.0)?,
                flicker_seconds: limited(v, "pool", "flicker_seconds", 0.0, 30.0)?,
                soak,
                item_slip_chance: limited(v, "pool", "item_slip_chance", 0.0, 1.0)?,
            },
            ground: GroundLanding {
                stun_from_speed: limited(v, "ground", "stun_from_speed", 0.0, 100.0)?,
                stun_full_speed: limited(v, "ground", "stun_full_speed", 0.01, 100.0)?,
                stun_min_seconds: limited(v, "ground", "stun_min_seconds", 0.0, 30.0)?,
                stun_max_seconds: limited(v, "ground", "stun_max_seconds", 0.0, 30.0)?,
                stamina_loss: limited(v, "ground", "stamina_loss", 0.0, 1000.0)?,
                exposure_add: limited(v, "ground", "exposure_add", 0.0, 100.0)?,
                noise: limited(v, "ground", "noise", 0.0, 100.0)?,
                noise_seconds: limited(v, "ground", "noise_seconds", 0.0, 30.0)?,
                flicker_seconds: limited(v, "ground", "flicker_seconds", 0.0, 30.0)?,
                scale_min: limited(v, "ground", "scale_min", 0.01, 10.0)?,
                scale_max: limited(v, "ground", "scale_max", 0.01, 10.0)?,
            },
        };
        if rules.ground.scale_max < rules.ground.scale_min {
            return Err("fall.json: ground.scale_max is below scale_min".into());
        }
        if rules.ground.stun_full_speed <= rules.ground.stun_from_speed {
            return Err("fall.json: ground.stun_full_speed must be above stun_from_speed".into());
        }
        if rules.ground.stun_max_seconds < rules.ground.stun_min_seconds {
            return Err("fall.json: ground.stun_max_seconds is below stun_min_seconds".into());
        }
        if rules.landing_report_gap < rules.min_gap {
            return Err("fall.json: landing_report_gap is below min_gap".into());
        }
        Ok(rules)
    }

    /// The speed multiplier of the body: `air_control` while it falls, 1 on the ground.
    pub fn air_scale(&self, airborne: bool) -> f32 {
        if airborne {
            self.air_control
        } else {
            1.0
        }
    }

    /// Whether the ground `gap` metres under a standing body is a fall (not a step the height eases down).
    pub fn starts_fall(&self, gap: f32) -> bool {
        gap > self.min_gap
    }

    /// Whether a fall of `drop_height` metres reports a landing.
    pub fn reports_landing(&self, drop_height: f32) -> bool {
        drop_height >= self.landing_report_gap
    }

    /// The stun (seconds) of touching dry floor at `speed` m/s (the downward speed): none under `stun_from_speed`, then
    /// Godot's lerp from `stun_min_seconds` at that speed to `stun_max_seconds` at `stun_full_speed` and above
    /// (player_controller_3d.gd:2117-2124).
    pub fn stun_seconds(&self, speed: f32) -> f32 {
        let g = &self.ground;
        if !(speed >= g.stun_from_speed) {
            return 0.0;
        }
        let severity = ((speed - g.stun_from_speed) / (g.stun_full_speed - g.stun_from_speed)).clamp(0.0, 1.0);
        g.stun_min_seconds + (g.stun_max_seconds - g.stun_min_seconds) * severity
    }

    /// Whether the item in hand slips away into the pool, for a roll in 0..1 (chance `pool.item_slip_chance`).
    pub fn item_slips(&self, roll: f32) -> bool {
        roll < self.pool.item_slip_chance
    }

    /// How hard a landing from `drop_height` metres hits on dry floor: the drop in storeys, between the clamps.
    pub fn ground_scale(&self, drop_height: f32) -> f32 {
        (drop_height / crate::world::vertical::STOREY).clamp(self.ground.scale_min, self.ground.scale_max)
    }
}

/// The embedded landing rules (data/swim/fall.json); the standard values (the same file) if the embedded copy is
/// missing or refused.
pub fn fall_rules() -> &'static FallRules {
    static RULES: OnceLock<FallRules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("swim/fall.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
            .and_then(|v| FallRules::parse(&v).ok())
            .unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------------------------------------------
// The water camera (data/swim/camera.json, `lr.swim.camera/1`)

/// How far the eye sinks while the runner stands in sewer water. The feet stay on the walkway (ground, steps and
/// falls are untouched); only the eye drops, as if the body sank into the channel. Godot never lowers the camera in
/// wading water, its only sink is the plunge (feet on the bed, the eye just clearing the surface), so this follows
/// that idea (owner decision).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraRules {
    /// Share of the walkway-to-bed distance the eye drops at full channel depth.
    pub sink_fraction: f32,
    /// Share of a pipe flood's depth the eye drops over a dry floor (0: the flood lifts the water, not the floor).
    pub flood_sink_fraction: f32,
    /// The deepest the eye ever sinks (metres).
    pub max_sink: f32,
    /// The eye never comes closer than this to the water surface (metres).
    pub min_eye_above_surface: f32,
    /// The eye never goes lower than this above the walkway (metres).
    pub min_eye_above_floor: f32,
    pub ease_down_per_s: f32,
    pub ease_up_per_s: f32,
}

impl Default for CameraRules {
    fn default() -> Self {
        CameraRules {
            sink_fraction: 1.0,
            flood_sink_fraction: 0.0,
            max_sink: 1.0,
            min_eye_above_surface: 0.45,
            min_eye_above_floor: 0.55,
            ease_down_per_s: 5.0,
            ease_up_per_s: 4.0,
        }
    }
}

impl CameraRules {
    /// Reads the rules; a missing, negative or non-finite field keeps its default (rates stay above zero).
    pub fn parse(v: &Value) -> CameraRules {
        let d = CameraRules::default();
        CameraRules {
            sink_fraction: nn(v, "sink_fraction", d.sink_fraction),
            flood_sink_fraction: nn(v, "flood_sink_fraction", d.flood_sink_fraction),
            max_sink: nn(v, "max_sink", d.max_sink),
            min_eye_above_surface: nn(v, "min_eye_above_surface", d.min_eye_above_surface),
            min_eye_above_floor: nn(v, "min_eye_above_floor", d.min_eye_above_floor),
            ease_down_per_s: pos(v, "ease_down_per_s", d.ease_down_per_s),
            ease_up_per_s: pos(v, "ease_up_per_s", d.ease_up_per_s),
        }
    }
}

/// The embedded camera rules (data/swim/camera.json); the defaults when the file is missing or broken.
pub fn camera_rules() -> &'static CameraRules {
    static RULES: OnceLock<CameraRules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("swim/camera.json")
            .next()
            .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
            .map(|v| CameraRules::parse(&v))
            .unwrap_or_default()
    })
}

/// Metres the eye should be lowered: `channel_drop` is the walkway-to-bed distance when standing over a water cell
/// (0 on dry floor), `flood_depth` a pipe flood's depth, `eye_height` the current eye above the walkway and
/// `surface_offset` the (negative) water surface offset from the walkway. Capped by `max_sink`, and never lets the
/// eye fall under `min_eye_above_floor` above the walkway or `min_eye_above_surface` above the water. Never negative.
pub fn sink_target(r: &CameraRules, channel_drop: f32, flood_depth: f32, eye_height: f32, surface_offset: f32) -> f32 {
    let want = (channel_drop.max(0.0) * r.sink_fraction + flood_depth.max(0.0) * r.flood_sink_fraction).clamp(0.0, r.max_sink);
    if want <= 0.0 {
        return 0.0;
    }
    let by_floor = eye_height - r.min_eye_above_floor;
    let by_surface = eye_height - r.min_eye_above_surface - surface_offset.min(0.0);
    want.min(by_floor).min(by_surface).max(0.0)
}

/// One frame of easing `current` towards `target`: exponential, faster down than up, never overshooting.
pub fn ease_sink(r: &CameraRules, current: f32, target: f32, dt: f32) -> f32 {
    let rate = if target > current { r.ease_down_per_s } else { r.ease_up_per_s };
    let next = current + (target - current) * (1.0 - (-rate * dt.max(0.0)).exp());
    if (target - next).abs() < 1e-4 {
        target
    } else {
        next
    }
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

/// Footstep noise while soaked (hunters hear a wet runner from further away).
pub fn soaked_noise(rules: &PlungeRules, base_noise: f32, soaked_left: f32) -> f32 {
    if soaked_left > 0.0 {
        base_noise * rules.soaked_noise_scale
    } else {
        base_noise
    }
}

/// An absolute cell split into its chunk (x, z) and the cell inside that chunk (floor division, so negative cells work).
pub fn split_cell(abs: (i32, i32), chunk: i32) -> ((i32, i32), (i32, i32)) {
    (
        (abs.0.div_euclid(chunk), abs.1.div_euclid(chunk)),
        (abs.0.rem_euclid(chunk), abs.1.rem_euclid(chunk)),
    )
}

/// The footstep noise to store on the runner for a water step of `water_noise` (already soaked-scaled by
/// `SwimState::update`). The hunters' hearing multiplies the stored noise by the soaked `scale` again
/// (`Game::runner_target`), so the water step is divided by it here: hunters then hear exactly `water_noise`.
pub fn merge_water_noise(current: f32, water_noise: f32, scale: f32) -> f32 {
    let stored = if scale > 0.0 { water_noise / scale } else { water_noise };
    current.max(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the water camera ---

    #[test]
    fn camera_json_parses_to_the_defaults() {
        let text = include_str!("../data/swim/camera.json");
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["schema"], "lr.swim.camera/1");
        assert_eq!(CameraRules::parse(&v), CameraRules::default());
        assert_eq!(*camera_rules(), CameraRules::default());
        assert!(
            crate::content::embedded::data_under("swim/camera.json").next().is_some(),
            "camera.json must be embedded, or editing it would silently do nothing"
        );
        let custom: Value = serde_json::from_str(r#"{"max_sink": 0.4, "ease_down_per_s": 9, "flood_sink_fraction": 0.25}"#).unwrap();
        let c = CameraRules::parse(&custom);
        assert_eq!((c.max_sink, c.ease_down_per_s, c.flood_sink_fraction), (0.4, 9.0, 0.25));
        assert_eq!(c.sink_fraction, CameraRules::default().sink_fraction, "unset fields keep their defaults");
        let bad: Value = serde_json::from_str(r#"{"max_sink": -1, "ease_down_per_s": 0, "sink_fraction": "x"}"#).unwrap();
        assert_eq!(CameraRules::parse(&bad), CameraRules::default(), "bad values keep their defaults");
    }

    #[test]
    fn sink_target_follows_the_water_and_keeps_the_eye_above_it() {
        let r = CameraRules::default();
        let (eye, crouched, surface) = (1.7072, 1.1, -0.3);
        assert_eq!(sink_target(&r, 0.0, 0.0, eye, surface), 0.0, "no water, no sink");
        assert_eq!(sink_target(&r, 0.0, 0.5, eye, surface), 0.0, "a flood over dry floor does not sink the floor");
        assert!(sink_target(&r, 0.5, 0.0, eye, surface) < sink_target(&r, 1.0, 0.0, eye, surface));
        assert!((sink_target(&r, 1.0, 0.0, eye, surface) - 1.0).abs() < 1e-5);
        assert_eq!(sink_target(&r, 5.0, 0.0, eye, surface), r.max_sink, "capped by max_sink");
        for e in [eye, crouched] {
            let s = sink_target(&r, 1.0, 0.0, e, surface);
            assert!(e - s >= r.min_eye_above_floor - 1e-5, "never under the floor limit");
            assert!(e - s - surface >= r.min_eye_above_surface - 1e-5, "never nearer the surface than the limit");
        }
        let flood = CameraRules { flood_sink_fraction: 0.5, ..r };
        assert!((sink_target(&flood, 0.0, 0.4, eye, surface) - 0.2).abs() < 1e-5);
    }

    #[test]
    fn ease_sink_is_monotone_and_never_overshoots() {
        let r = CameraRules::default();
        let mut x = 0.0;
        for _ in 0..200 {
            let n = ease_sink(&r, x, 1.0, 1.0 / 60.0);
            assert!(n >= x && n <= 1.0);
            x = n;
        }
        assert_eq!(x, 1.0);
        let down = ease_sink(&r, 0.0, 1.0, 0.1);
        let up = 1.0 - ease_sink(&r, 1.0, 0.0, 0.1);
        assert!(down > up, "the eye drops faster than it rises");
        assert_eq!(ease_sink(&r, 0.5, 0.5, 0.1), 0.5);
        let mut y = 1.0;
        for _ in 0..200 {
            let n = ease_sink(&r, y, 0.0, 1.0 / 60.0);
            assert!(n <= y && n >= 0.0);
            y = n;
        }
        assert_eq!(y, 0.0);
    }

    #[test]
    fn split_cell_handles_negative_cells() {
        assert_eq!(split_cell((50, 3), 45), ((1, 0), (5, 3)));
        assert_eq!(split_cell((-1, -45), 45), ((-1, -1), (44, 0)));
    }

    #[test]
    fn hunters_hear_the_water_step_once_not_twice() {
        let scale = 1.45;
        let stored = merge_water_noise(0.0, 17.4, scale);
        assert!((stored * scale - 17.4).abs() < 1e-4);
        assert_eq!(merge_water_noise(30.0, 17.4, scale), 30.0, "a louder footstep stays");
        assert_eq!(merge_water_noise(2.0, 0.0, scale), 2.0);
    }

    fn standard() -> SwimRules {
        SwimRules::default()
    }

    #[test]
    fn embedded_data_matches_godot_standard() {
        assert_eq!(*rules(), standard());
        assert_eq!(rules().wade.speed_scale, 0.45);
        assert_eq!(rules().plunge.soaked_seconds, 30.0);
    }

    #[test]
    fn changed_data_changes_behaviour() {
        let v = serde_json::json!({ "wade": { "speed_scale": 0.2 }, "plunge": { "soaked_noise_scale": 3.0 } });
        let r = SwimRules::parse(&v);
        assert!(wade_speed_scale(&r.wade, 0.9) < wade_speed_scale(&standard().wade, 0.9));
        assert!(soaked_noise(&r.plunge, 10.0, 5.0) > soaked_noise(&standard().plunge, 10.0, 5.0));
    }

    #[test]
    fn bad_data_falls_back() {
        let v = serde_json::json!({ "plunge": { "soaked_seconds": -1 }, "wade": { "deep_depth": "x" } });
        assert_eq!(SwimRules::parse(&v), standard());
    }

    // --- the landing rule (data/swim/fall.json) ---

    fn fall_text() -> &'static str {
        include_str!("../data/swim/fall.json")
    }

    fn fall_json() -> Value {
        serde_json::from_str(fall_text()).unwrap()
    }

    #[test]
    fn fall_json_matches_standard_values() {
        let r = FallRules::parse(&fall_json()).expect("the shipped file parses");
        assert_eq!(r, FallRules::default());
        assert_eq!(*fall_rules(), r, "the embedded file is the one in the tree");
        assert_eq!(r.min_height, 1.5);
        assert_eq!((r.air_control, r.min_gap, r.landing_report_gap), (0.6, 0.6, 1.2));
        // Godot's hard-landing curve (game_config.gd:107-110) and the pool's item slip (infinite_route_test.gd:58)
        assert_eq!((r.ground.stun_from_speed, r.ground.stun_full_speed), (8.25, 14.0));
        assert_eq!((r.ground.stun_min_seconds, r.ground.stun_max_seconds), (0.4, 1.4));
        assert_eq!(r.pool.item_slip_chance, 0.08);
        assert_eq!((r.ground.stamina_loss, r.ground.exposure_add), (60.0, 12.0));
        assert_eq!((r.ground.noise, r.ground.noise_seconds, r.ground.flicker_seconds), (18.0, 1.5, 0.8));
        assert_eq!((r.ground.scale_min, r.ground.scale_max), (0.4, 2.0));
        assert!(r.pool.soak);
        // the pool splash is the old plunge splash
        assert_eq!(r.pool.noise, rules().plunge.splash_noise);
        assert_eq!(r.pool.noise_seconds, rules().plunge.splash_noise_seconds);
    }

    #[test]
    fn the_pool_splash_is_quieter_than_before_and_still_audible() {
        const OLD_POOL_GAIN: f32 = 1.0;
        const OLD_WADE_GAIN: f32 = 0.6;
        let r = FallRules::parse(&fall_json()).unwrap();
        assert!(r.pool.splash_gain < OLD_POOL_GAIN && r.pool.splash_gain >= 0.2);
        assert!(rules().wade.splash_gain < OLD_WADE_GAIN && rules().wade.splash_gain >= 0.1);
        // the gameplay noise is untouched
        assert_eq!(r.pool.noise, 3.0);
    }

    #[test]
    fn a_bad_fall_file_is_refused() {
        let ok = fall_json();
        let with = |section: &str, key: &str, val: Value| {
            let mut v = ok.clone();
            if section.is_empty() {
                v[key] = val;
            } else {
                v[section][key] = val;
            }
            v
        };
        let neg = serde_json::json!(-1.0);
        let huge = serde_json::json!(1.0e9);
        let text = serde_json::json!("x");
        let cases = [
            ("", "min_height", neg.clone()),
            ("", "min_height", huge.clone()),
            ("", "min_height", text.clone()),
            ("", "air_control", neg.clone()),
            ("", "air_control", serde_json::json!(1.5)),
            ("", "min_gap", serde_json::json!(0.0)),
            ("", "min_gap", huge.clone()),
            ("", "landing_report_gap", neg.clone()),
            ("", "landing_report_gap", serde_json::json!(0.1)),
            ("pool", "min_depth", neg.clone()),
            ("pool", "min_depth", serde_json::json!(0.0)),
            ("pool", "splash_gain", neg.clone()),
            ("pool", "noise", huge.clone()),
            ("pool", "noise_seconds", neg.clone()),
            ("pool", "flicker_seconds", huge.clone()),
            ("pool", "soak", serde_json::json!(1)),
            ("pool", "item_slip_chance", neg.clone()),
            ("pool", "item_slip_chance", serde_json::json!(1.5)),
            ("ground", "stun_from_speed", neg.clone()),
            ("ground", "stun_full_speed", huge.clone()),
            ("ground", "stun_full_speed", serde_json::json!(8.0)),
            ("ground", "stun_min_seconds", neg.clone()),
            ("ground", "stun_max_seconds", huge.clone()),
            ("ground", "stun_max_seconds", serde_json::json!(0.1)),
            ("ground", "stamina_loss", huge.clone()),
            ("ground", "exposure_add", serde_json::json!(101.0)),
            ("ground", "noise", neg.clone()),
            ("ground", "noise_seconds", huge.clone()),
            ("ground", "flicker_seconds", neg.clone()),
            ("ground", "scale_min", serde_json::json!(0.0)),
            ("ground", "scale_max", huge.clone()),
        ];
        for (section, key, val) in cases {
            assert!(FallRules::parse(&with(section, key, val.clone())).is_err(), "{section}.{key} = {val} must be refused");
        }
        // a missing field, a wrong schema and an inverted clamp
        let mut missing = ok.clone();
        missing["ground"].as_object_mut().unwrap().remove("stun_from_speed");
        assert!(FallRules::parse(&missing).is_err());
        let mut no_slip = ok.clone();
        no_slip["pool"].as_object_mut().unwrap().remove("item_slip_chance");
        assert!(FallRules::parse(&no_slip).is_err());
        assert!(FallRules::parse(&with("", "schema", serde_json::json!("lr.swim.fall/2"))).is_err());
        let mut inverted = ok.clone();
        inverted["ground"]["scale_min"] = serde_json::json!(3.0);
        assert!(FallRules::parse(&inverted).is_err(), "scale_max below scale_min");
    }

    #[test]
    fn the_hard_landing_stun_follows_godots_speed_curve() {
        let r = FallRules::default();
        // none under 8.25 m/s, 0.4 s at it, 1.4 s from 14 m/s up
        assert_eq!(r.stun_seconds(0.0), 0.0);
        assert_eq!(r.stun_seconds(8.24), 0.0, "just under the threshold");
        assert!((r.stun_seconds(8.25) - 0.4).abs() < 1e-5);
        assert!((r.stun_seconds(14.0) - 1.4).abs() < 1e-5);
        assert_eq!(r.stun_seconds(40.0), r.stun_seconds(14.0), "capped past the full speed");
        // the middle of the curve: (11.125 - 8.25) / 5.75 = 0.5 gives 0.9 s
        assert!((r.stun_seconds(11.125) - 0.9).abs() < 1e-4, "{}", r.stun_seconds(11.125));
        // monotone between the ends
        let mut last = 0.0;
        for i in 0..=100 {
            let s = r.stun_seconds(8.25 + 5.75 * i as f32 / 100.0);
            assert!(s >= last - 1e-6, "stun never shortens as the landing gets harder");
            last = s;
        }
        // a storey of free fall (v = sqrt(2 g 5) = 10.95 m/s) is a stun of about 0.87 s: inside the curve, not at an end
        let storey = (2.0 * crate::config::PLAYER_GRAVITY * crate::world::vertical::STOREY).sqrt();
        assert!(r.stun_seconds(storey) > 0.4 && r.stun_seconds(storey) < 1.4, "{}", r.stun_seconds(storey));
        // the message is Godot's
        assert_eq!(HARD_LANDING_MESSAGE, "HARD LANDING // STUNNED");
    }

    #[test]
    fn the_pool_item_slip_is_eight_percent() {
        let r = FallRules::default();
        assert!(r.item_slips(0.0) && r.item_slips(0.079) && !r.item_slips(0.08) && !r.item_slips(0.99));
        let mut v = fall_json();
        v["pool"]["item_slip_chance"] = serde_json::json!(0.0);
        assert!(!FallRules::parse(&v).unwrap().item_slips(0.0), "a zero chance never slips");
    }

    #[test]
    fn changed_fall_data_changes_the_scale() {
        let mut v = fall_json();
        v["ground"]["scale_max"] = serde_json::json!(4.0);
        let wide = FallRules::parse(&v).unwrap();
        let drop = 3.0 * crate::world::vertical::STOREY;
        assert!(wide.ground_scale(drop) > FallRules::default().ground_scale(drop));
    }

    #[test]
    fn changed_fall_data_changes_the_body_rules() {
        let base = FallRules::default();
        let mut v = fall_json();
        v["air_control"] = serde_json::json!(0.2);
        v["min_gap"] = serde_json::json!(1.0);
        v["landing_report_gap"] = serde_json::json!(3.0);
        let r = FallRules::parse(&v).unwrap();
        assert!(r.air_scale(true) < base.air_scale(true), "less air control");
        assert_eq!((r.air_scale(false), base.air_scale(false)), (1.0, 1.0), "the ground speed is never scaled");
        assert!(base.starts_fall(0.8) && !r.starts_fall(0.8), "a wider gap is still a step");
        assert!(base.reports_landing(2.0) && !r.reports_landing(2.0), "a higher report gap hides a short landing");
        assert!(!base.starts_fall(base.min_gap) && base.starts_fall(base.min_gap + 0.01));
        assert!(base.reports_landing(base.landing_report_gap) && !base.reports_landing(base.landing_report_gap - 0.01));
    }

    #[test]
    fn the_default_is_the_shipped_file() {
        assert_eq!(FallRules::default(), FallRules::parse(&fall_json()).unwrap());
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

    #[test]
    fn soaked_makes_steps_louder() {
        let r = standard().plunge;
        assert!(soaked_noise(&r, 10.0, 5.0) > 10.0);
        assert_eq!(soaked_noise(&r, 10.0, 0.0), 10.0);
    }
}
