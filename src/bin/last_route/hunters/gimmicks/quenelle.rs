//! Quenelle: flashlight tracker. 8 s of continuous light locks her onto you for good; every
//! flick of the torch after that makes her faster while it is lit, up to x1.45
//! (quenelle_gimmick.gd). The lock times and the ramp live in data/gimmicks/quenelle.json.

use crate::config::Difficulty;
use crate::hunters::brain::{Brain, Target};

pub struct Quenelle {
    tracking: bool,
    elapsed: f32,
    activations: i32,
    was_on: bool,
    /// Continuous torch time before the permanent lock: "lock_seconds" (8 s) on Standard,
    /// "nightmare_lock_seconds" (5 s) on Nightmare.
    lock_seconds: f32,
    pub speed_scale: f32,
}

/// The tunables (data/gimmicks/quenelle.json); a missing or bad key keeps the Godot value
/// (game_config.gd QUENELLE_FLASHLIGHT_LOCK_SECONDS 8, QUENELLE_NIGHTMARE_LOCK_SECONDS 5,
/// QUENELLE_MAX_SPEED_SCALE 1.45, QUENELLE_SPEED_RAMP_ACTIVATIONS 10).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    pub lock_seconds: f32,
    pub nightmare_lock_seconds: f32,
    pub max_speed_scale: f32,
    pub ramp_activations: i32,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning { lock_seconds: 8.0, nightmare_lock_seconds: 5.0, max_speed_scale: 1.45, ramp_activations: 10 }
    }
}

impl Tuning {
    pub fn parse(text: &str) -> Tuning {
        let mut t = Tuning::default();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            let pick = |k: &str, d: f64| match v.get(k).and_then(|x| x.as_f64()) {
                Some(n) if n > 0.0 && n.is_finite() => n,
                _ => d,
            };
            t.lock_seconds = pick("lock_seconds", t.lock_seconds as f64) as f32;
            t.nightmare_lock_seconds = pick("nightmare_lock_seconds", t.nightmare_lock_seconds as f64) as f32;
            t.max_speed_scale = (pick("max_speed_scale", t.max_speed_scale as f64) as f32).max(1.0);
            t.ramp_activations = (pick("ramp_activations", t.ramp_activations as f64) as i32).max(1);
        }
        t
    }
}

/// The embedded tuning, parsed once.
pub fn tuning() -> Tuning {
    static T: std::sync::OnceLock<Tuning> = std::sync::OnceLock::new();
    *T.get_or_init(|| Tuning::parse(include_str!("../../data/gimmicks/quenelle.json")))
}

/// The chase speed multiplier after `activations` torch flicks (locked on and lit):
/// 1 + min(1, flicks / ramp_activations) * (max - 1), as quenelle_gimmick.gd.
pub fn ramp_scale(activations: i32) -> f32 {
    let t = tuning();
    let progress = (activations as f32 / t.ramp_activations as f32).min(1.0);
    1.0 + progress * (t.max_speed_scale - 1.0)
}

impl Default for Quenelle {
    fn default() -> Self {
        Quenelle {
            tracking: false,
            elapsed: 0.0,
            activations: 0,
            was_on: false,
            lock_seconds: tuning().lock_seconds,
            speed_scale: 1.0,
        }
    }
}

impl Quenelle {
    /// Quenelle on Nightmare: the lock is 5 s. Relaxed, Standard and Hard keep the 8 s lock.
    pub fn nightmare() -> Quenelle {
        Quenelle {
            lock_seconds: tuning().nightmare_lock_seconds,
            ..Quenelle::default()
        }
    }

    /// The Quenelle a run on `diff` gets (the lock time is the only per-difficulty value).
    pub fn for_difficulty(diff: Difficulty) -> Quenelle {
        match diff {
            Difficulty::Nightmare => Quenelle::nightmare(),
            _ => Quenelle::default(),
        }
    }

    pub fn lock_seconds(&self) -> f32 {
        self.lock_seconds
    }

    /// The mechanic pose she is in: locked on and the torch is lit, so she is straining toward it (every flick makes her faster).
    pub fn mechanic(&self) -> Option<&'static str> {
        (self.tracking && self.was_on).then_some("torch_frenzy")
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, t: &Target) {
        let lit = t.flashlight_on && t.lamp_charge > 0.0 && !t.hidden;
        if lit && !self.was_on {
            self.activations += 1;
        }
        self.was_on = lit;
        if !self.tracking {
            self.elapsed = if lit { self.elapsed + dt } else { 0.0 };
            if self.elapsed >= self.lock_seconds {
                self.tracking = true;
                // the lock: she turns on the light and stays on it
                b.raise_cue("lock_on");
            }
        }
        self.speed_scale = if self.tracking && lit { ramp_scale(self.activations) } else { 1.0 };
        if self.tracking && !t.in_safe_zone {
            b.chase(t.pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::gimmicks::bench::{Bench, DT};
    use crate::hunters::gimmicks::Gimmick;

    fn quenelle(x: &Bench) -> &Quenelle {
        let Gimmick::Quenelle(q) = &x.b.gimmick else { panic!("not Quenelle") };
        q
    }

    fn lit_bench(diff: Difficulty) -> Bench {
        let mut x = Bench::with_diff("quenelle", diff);
        x.t.hidden = false;
        x.t.flashlight_on = true;
        x.t.lamp_charge = 1.0;
        x
    }

    /// Standard keeps the 8 s lock; Nightmare locks on at 5 s (game_config.gd).
    #[test]
    fn lock_on_is_eight_seconds_standard_and_five_on_nightmare() {
        assert_eq!(Quenelle::default().lock_seconds(), 8.0);
        assert_eq!(Quenelle::nightmare().lock_seconds(), 5.0);
        assert_eq!(Quenelle::for_difficulty(Difficulty::Nightmare).lock_seconds(), 5.0);
        assert_eq!(Quenelle::for_difficulty(Difficulty::Standard).lock_seconds(), 8.0);
        assert_eq!(Quenelle::for_difficulty(Difficulty::Hard).lock_seconds(), 8.0);
    }

    /// The shipped data file carries the Godot values.
    #[test]
    fn the_shipped_data_matches_the_godot_values() {
        let t = tuning();
        assert_eq!(t, Tuning::default());
        assert_eq!((t.lock_seconds, t.nightmare_lock_seconds, t.max_speed_scale, t.ramp_activations), (8.0, 5.0, 1.45, 10));
    }

    /// A bad or missing key keeps the Godot value; a changed value changes the behaviour.
    #[test]
    fn bad_keys_keep_the_godot_values_and_changed_values_apply() {
        assert_eq!(Tuning::parse("not json"), Tuning::default());
        assert_eq!(Tuning::parse("{\"lock_seconds\": -1}"), Tuning::default());
        let t = Tuning::parse("{\"lock_seconds\": 3, \"max_speed_scale\": 2.0, \"ramp_activations\": 4}");
        assert_eq!((t.lock_seconds, t.max_speed_scale, t.ramp_activations), (3.0, 2.0, 4));
        assert_eq!(t.nightmare_lock_seconds, 5.0);
    }

    /// The lock fires at the Godot time: not before 8 s of continuous light, and by 8 s plus a frame it has.
    #[test]
    fn the_lock_fires_at_the_godot_time() {
        let mut x = lit_bench(Difficulty::Standard);
        x.step(7.5);
        assert!(!quenelle(&x).tracking, "7.5 s of light is not enough");
        x.step(0.5 + 2.0 * DT);
        assert!(quenelle(&x).tracking, "past 8 s she is locked on");

        let mut n = lit_bench(Difficulty::Nightmare);
        n.step(4.5);
        assert!(!quenelle(&n).tracking);
        n.step(0.5 + 2.0 * DT);
        assert!(quenelle(&n).tracking, "Nightmare locks at 5 s");
    }

    /// Darkness resets the lock timer.
    #[test]
    fn darkness_resets_the_lock_timer() {
        let mut x = lit_bench(Difficulty::Standard);
        x.step(6.0);
        x.t.flashlight_on = false;
        x.step(2.0 * DT);
        x.t.flashlight_on = true;
        x.step(6.0);
        assert!(!quenelle(&x).tracking, "6 s + gap + 6 s never reaches 8 continuous seconds");
    }

    /// The ramp reaches the top speed (x1.45) after 10 flicks and stays there.
    #[test]
    fn the_ramp_reaches_the_top_speed() {
        let t = tuning();
        assert_eq!(ramp_scale(0), 1.0);
        let half = ramp_scale(t.ramp_activations / 2);
        assert!(half > 1.0 && half < t.max_speed_scale);
        assert!((ramp_scale(t.ramp_activations) - t.max_speed_scale).abs() < 1e-6);
        assert!((ramp_scale(t.ramp_activations * 3) - t.max_speed_scale).abs() < 1e-6);
        assert!((ramp_scale(10) - 1.45).abs() < 1e-6, "Godot: x1.45 after 10 flicks");
    }

    /// Nightmare keeps the Standard speed ramp: it only changes the lock time.
    #[test]
    fn nightmare_only_changes_the_lock_time() {
        let q = Quenelle::nightmare();
        assert_eq!(q.speed_scale, 1.0);
        assert!(!q.tracking);
        assert_eq!(q.activations, 0);
    }
}
