//! Sleep and drowsiness (TODO "Sleep and drowsiness"). The meter itself is `Meters::drowsiness`
//! (game/meters.rs); this file holds the data (`data/sleep.json`, `lr.sleep/1`) and the sleeping behaviour: the
//! toggle, waking, and the jolt when a hunter comes near a sleeper.
//!
//! Hooks (the integrate agent): `mod sleep;` in game/mod.rs; a [`Sleep`] beside the player; the sleep key (or lying on
//! a bed from seating.rs) calls [`Sleep::toggle_sleep`]; each step calls `meters.update_drowsiness(dt, moving,
//! sleep.sleeping(), sleep_cfg())` then [`Sleep::step`] with the nearest hunter distance, and applies the returned
//! [`Jolt`] (exposure and noise) to the meters and freezes input for `jolt_seconds`. Only the side that owns the
//! player runs this; hunters are never changed here.

use serde_json::Value;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.sleep/1";
const TEXT: &str = include_str!("../data/sleep.json");

#[derive(Clone, Debug, PartialEq)]
pub struct SleepCfg {
    pub max: f32,
    pub awake_rise_per_s: f32,
    pub moving_rise_scale: f32,
    pub still_rise_scale: f32,
    pub sleep_fall_per_s: f32,
    pub pass_out: f32,
    pub may_sleep_below: f32,
    pub wake_at: f32,
    pub disturb_radius: f32,
    pub disturb_radius_sprinting: f32,
    pub disturb_chase_always: bool,
    pub jolt_seconds: f32,
    pub jolt_exposure: f32,
    pub jolt_noise: f32,
    pub slow_below: f32,
    pub slow_scale_at_max: f32,
}

fn n(v: &Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .map(|x| x as f32)
        .ok_or_else(|| format!("sleep.json: '{key}' missing or not a number of at least 0"))
}

pub fn parse(text: &str) -> Result<SleepCfg, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("sleep.json: {e}"))?;
    if v["schema"].as_str() != Some(SCHEMA) {
        return Err(format!("sleep.json: 'schema' must be {SCHEMA}"));
    }
    let c = SleepCfg {
        max: n(&v, "max")?,
        awake_rise_per_s: n(&v, "awake_rise_per_s")?,
        moving_rise_scale: n(&v, "moving_rise_scale")?,
        still_rise_scale: n(&v, "still_rise_scale")?,
        sleep_fall_per_s: n(&v, "sleep_fall_per_s")?,
        pass_out: n(&v, "pass_out")?,
        may_sleep_below: n(&v, "may_sleep_below")?,
        wake_at: n(&v, "wake_at")?,
        disturb_radius: n(&v, "disturb_radius")?,
        disturb_radius_sprinting: n(&v, "disturb_radius_sprinting")?,
        disturb_chase_always: v["disturb_chase_always"].as_bool().unwrap_or(true),
        jolt_seconds: n(&v, "jolt_seconds")?,
        jolt_exposure: n(&v, "jolt_exposure")?,
        jolt_noise: n(&v, "jolt_noise")?,
        slow_below: n(&v, "slow_below")?,
        slow_scale_at_max: n(&v, "slow_scale_at_max")?,
    };
    if c.max <= 0.0 || c.slow_scale_at_max <= 0.0 || c.slow_scale_at_max > 1.0 {
        return Err("sleep.json: 'max' must be above 0 and 'slow_scale_at_max' in (0, 1]".into());
    }
    Ok(c)
}

pub fn sleep_cfg() -> &'static SleepCfg {
    static C: OnceLock<SleepCfg> = OnceLock::new();
    C.get_or_init(|| parse(TEXT).unwrap_or_else(|e| panic!("{e}")))
}

impl SleepCfg {
    /// The drowsiness after `dt` seconds.
    pub fn step_drowsiness(&self, d: f32, dt: f32, moving: bool, sleeping: bool) -> f32 {
        let dt = dt.max(0.0);
        let next = if sleeping {
            d - self.sleep_fall_per_s * dt
        } else {
            let scale = if moving { self.moving_rise_scale } else { self.still_rise_scale };
            d + self.awake_rise_per_s * scale * dt
        };
        next.clamp(0.0, self.max)
    }

    /// Movement multiplier: 1 until `slow_below` drowsiness, then down to `slow_scale_at_max` at the top.
    pub fn tired_scale(&self, d: f32) -> f32 {
        if d <= self.slow_below || self.max <= self.slow_below {
            return 1.0;
        }
        let t = ((d - self.slow_below) / (self.max - self.slow_below)).clamp(0.0, 1.0);
        1.0 + (self.slow_scale_at_max - 1.0) * t
    }

    pub fn passes_out(&self, d: f32) -> bool {
        d >= self.pass_out
    }
}

/// What a hunter coming near a sleeper does to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Jolt {
    pub exposure: f32,
    pub noise: f32,
    pub frozen_seconds: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sleep {
    sleeping: bool,
}

impl Sleep {
    pub fn sleeping(&self) -> bool {
        self.sleeping
    }

    /// The sleep key: lie down to sleep, or wake. Falling asleep needs the meter below `may_sleep_below` (a wide-awake
    /// body cannot nod off at 100 after passing out is handled by `step`). Returns whether the body now sleeps.
    pub fn toggle_sleep(&mut self, drowsiness: f32, c: &SleepCfg) -> bool {
        if self.sleeping {
            self.sleeping = false;
        } else if drowsiness < c.may_sleep_below || c.passes_out(drowsiness) {
            self.sleeping = true;
        }
        self.sleeping
    }

    /// One step. A sleeper wakes when the meter reaches `wake_at`; a body at `pass_out` falls asleep; a hunter inside
    /// the disturb radius (the larger one if the runner is sprinting, any distance while chased) jolts the sleeper
    /// awake. `hunter_dist` is the nearest hunter, `chased` whether one hunts this runner.
    pub fn step(&mut self, drowsiness: &mut f32, hunter_dist: Option<f32>, sprinting: bool, chased: bool, c: &SleepCfg) -> Option<Jolt> {
        if !self.sleeping {
            if c.passes_out(*drowsiness) {
                self.sleeping = true;
            }
            return None;
        }
        let radius = if sprinting { c.disturb_radius_sprinting } else { c.disturb_radius };
        let near = hunter_dist.is_some_and(|d| d.is_finite() && d <= radius);
        if near || (chased && c.disturb_chase_always) {
            self.sleeping = false;
            *drowsiness = (*drowsiness - c.wake_at).max(0.0).min(*drowsiness);
            return Some(Jolt { exposure: c.jolt_exposure, noise: c.jolt_noise, frozen_seconds: c.jolt_seconds });
        }
        if *drowsiness <= c.wake_at {
            self.sleeping = false;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_data_parses() {
        assert_eq!(&parse(TEXT).unwrap(), sleep_cfg());
        assert!(parse("{}").is_err());
    }

    #[test]
    fn drowsiness_rises_awake_and_falls_asleep() {
        let c = sleep_cfg();
        let up = c.step_drowsiness(10.0, 10.0, false, false);
        assert!(up > 10.0 && up <= c.max);
        assert!(c.step_drowsiness(up, 1.0, false, true) < up);
        assert_eq!(c.step_drowsiness(0.0, 5.0, false, true), 0.0, "never below 0");
        assert!(c.step_drowsiness(10.0, 1.0, false, false) > c.step_drowsiness(10.0, 1.0, true, false));
    }

    #[test]
    fn tired_scale_slows_only_above_the_threshold() {
        let c = sleep_cfg();
        assert_eq!(c.tired_scale(0.0), 1.0);
        assert!((c.tired_scale(c.max) - c.slow_scale_at_max).abs() < 1e-5);
        assert!(c.tired_scale((c.slow_below + c.max) / 2.0) < 1.0);
    }

    #[test]
    fn a_hunter_near_a_sleeper_jolts_it_awake_but_a_far_one_does_not() {
        let c = sleep_cfg();
        let mut s = Sleep::default();
        let mut d = 50.0;
        assert!(s.toggle_sleep(d, c));
        assert!(s.step(&mut d, Some(c.disturb_radius + 20.0), false, false, c).is_none());
        assert!(s.sleeping());
        let jolt = s.step(&mut d, Some(c.disturb_radius - 1.0), false, false, c).expect("jolt");
        assert!(!s.sleeping());
        assert_eq!(jolt.exposure, c.jolt_exposure);
        assert!(jolt.frozen_seconds > 0.0);
    }

    #[test]
    fn a_sprinting_runner_is_heard_from_further_and_a_chase_always_wakes() {
        let c = sleep_cfg();
        let mut d = 50.0;
        let mut s = Sleep::default();
        s.toggle_sleep(d, c);
        let between = (c.disturb_radius + c.disturb_radius_sprinting) / 2.0;
        assert!(s.step(&mut d, Some(between), false, false, c).is_none());
        assert!(s.step(&mut d, Some(between), true, false, c).is_some());
        s.toggle_sleep(d, c);
        assert!(s.step(&mut d, None, false, true, c).is_some(), "chased");
    }

    #[test]
    fn passing_out_and_waking_naturally() {
        let c = sleep_cfg();
        let mut s = Sleep::default();
        let mut d = c.max;
        assert!(s.step(&mut d, None, false, false, c).is_none());
        assert!(s.sleeping(), "full drowsiness passes out");
        for _ in 0..100 {
            d = c.step_drowsiness(d, 1.0, false, true);
            s.step(&mut d, None, false, false, c);
        }
        assert!(!s.sleeping(), "rested sleeper wakes");
        // toggling twice returns to awake
        let mut t = Sleep::default();
        t.toggle_sleep(10.0, c);
        assert!(!t.toggle_sleep(10.0, c));
    }
}
