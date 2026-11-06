//! The runner's sprint stamina rules (data/player/stamina.json): how long a full bar lasts while sprinting, and how the
//! run speed weakens as the bar falls under `weak_below`. The curve is one function, [`StaminaRules::speed_scale`], that
//! `Player::update_movement` multiplies into the sprint speed: stairs, flats, a fall carried in a sprint and the head
//! bob / footstep cadence (both follow the real ground speed) all share it. Ladders and swimming do not read the sprint
//! speed, so they do not use it.

use crate::config::MAX_STAMINA;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaminaRules {
    /// Seconds a full bar lasts while sprinting at the base drain.
    pub sprint_seconds: f32,
    /// Fraction of the bar (0..1] under which the run speed weakens.
    pub weak_below: f32,
    /// Share of the full sprint speed left at an empty bar (0..1].
    pub min_speed_scale: f32,
}

impl Default for StaminaRules {
    fn default() -> Self {
        StaminaRules { sprint_seconds: 4.6875, weak_below: 0.45, min_speed_scale: 0.70 }
    }
}

impl StaminaRules {
    /// Stamina points lost per second of sprinting, before the stim pack and overheating scales.
    pub fn sprint_drain(&self) -> f32 {
        MAX_STAMINA / self.sprint_seconds
    }

    /// The multiplier on the sprint speed at `stamina` points: 1 from `weak_below` of the bar up, falling smoothly
    /// (smoothstep: no kink at the threshold, gentle at the very bottom) to `min_speed_scale` at an empty bar.
    pub fn speed_scale(&self, stamina: f32) -> f32 {
        let frac = if stamina.is_finite() { (stamina / MAX_STAMINA).clamp(0.0, 1.0) } else { 1.0 };
        if frac >= self.weak_below {
            return 1.0;
        }
        let t = frac / self.weak_below;
        let eased = t * t * (3.0 - 2.0 * t);
        self.min_speed_scale + (1.0 - self.min_speed_scale) * eased
    }

    pub fn parse(text: &str) -> Result<StaminaRules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("stamina.json: {e}"))?;
        let d = StaminaRules::default();
        let num = |key: &str, default: f32, lo: f32, hi: f32| -> Result<f32, String> {
            match v.get(key) {
                None => Ok(default),
                Some(x) => {
                    let n = x.as_f64().ok_or_else(|| format!("stamina.json: {key} is not a number"))? as f32;
                    if n.is_finite() && n >= lo && n <= hi {
                        Ok(n)
                    } else {
                        Err(format!("stamina.json: {key} {n} is outside {lo}..={hi}"))
                    }
                }
            }
        };
        Ok(StaminaRules {
            sprint_seconds: num("sprint_seconds", d.sprint_seconds, 0.5, 120.0)?,
            weak_below: num("weak_below", d.weak_below, 0.05, 1.0)?,
            min_speed_scale: num("min_speed_scale", d.min_speed_scale, 0.1, 1.0)?,
        })
    }
}

/// The shipped rules; a broken file keeps the defaults (the same numbers; a test holds the shipped file valid).
pub fn rules() -> &'static StaminaRules {
    static RULES: std::sync::OnceLock<StaminaRules> = std::sync::OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("player/stamina.json")
            .next()
            .and_then(|(_, t)| StaminaRules::parse(t).ok())
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> StaminaRules {
        let (_, text) = crate::content::embedded::data_under("player/stamina.json").next().expect("stamina.json is embedded");
        StaminaRules::parse(text).expect("the shipped stamina.json parses")
    }

    #[test]
    fn the_shipped_file_matches_the_defaults_and_is_in_use() {
        assert_eq!(shipped(), StaminaRules::default());
        assert_eq!(*rules(), shipped());
    }

    #[test]
    fn the_curve_is_full_at_and_above_the_threshold_and_weakest_when_empty() {
        let r = shipped();
        for s in [100.0, 60.0, 45.0] {
            assert_eq!(r.speed_scale(s), 1.0, "{s} percent");
        }
        assert!((r.speed_scale(0.0) - 0.70).abs() < 1e-6);
        // 30 percent: t = 2/3, smoothstep 0.7407, 0.7 + 0.3 * 0.7407
        assert!((r.speed_scale(30.0) - 0.9222).abs() < 1e-3, "{}", r.speed_scale(30.0));
        // half way down the weak band the scale is half way between the ends (smoothstep(0.5) = 0.5)
        assert!((r.speed_scale(22.5) - 0.85).abs() < 1e-5);
    }

    #[test]
    fn the_curve_only_falls_and_has_no_step_at_the_threshold() {
        let r = shipped();
        let mut last = 2.0;
        let mut s = 100.0;
        while s >= 0.0 {
            let k = r.speed_scale(s);
            assert!(k <= last + 1e-6, "rose at {s}: {k} after {last}");
            assert!((0.70 - 1e-6..=1.0).contains(&k), "{s}: {k} out of bounds");
            last = k;
            s -= 0.25;
        }
        // just under the threshold the drop is tiny, not a jump
        assert!(1.0 - r.speed_scale(44.9) < 0.001);
        // out-of-range and bad readings stay in bounds
        assert_eq!(r.speed_scale(-5.0), r.min_speed_scale);
        assert_eq!(r.speed_scale(250.0), 1.0);
        assert_eq!(r.speed_scale(f32::NAN), 1.0);
    }

    #[test]
    fn a_full_bar_lasts_one_and_a_half_times_the_old_thirty_two_per_second() {
        let r = shipped();
        assert!((r.sprint_seconds / (MAX_STAMINA / 32.0) - 1.5).abs() < 1e-5, "old 3.125 s, new {}", r.sprint_seconds);
        assert!((r.sprint_drain() - 100.0 / 4.6875).abs() < 1e-4);
        assert!(r.sprint_drain() < 32.0);
    }

    #[test]
    fn data_values_change_the_behaviour_and_bad_values_are_refused() {
        let r = StaminaRules::parse(r#"{"sprint_seconds":10,"weak_below":0.5,"min_speed_scale":0.5}"#).unwrap();
        assert_eq!(r.sprint_drain(), 10.0);
        assert_eq!(r.speed_scale(50.0), 1.0);
        assert!((r.speed_scale(0.0) - 0.5).abs() < 1e-6);
        assert!((r.speed_scale(25.0) - 0.75).abs() < 1e-6);
        assert_eq!(StaminaRules::parse("{}").unwrap(), StaminaRules::default());
        for bad in [
            r#"{"sprint_seconds":0}"#,
            r#"{"sprint_seconds":"long"}"#,
            r#"{"weak_below":0}"#,
            r#"{"weak_below":1.5}"#,
            r#"{"min_speed_scale":0}"#,
            r#"{"min_speed_scale":1.2}"#,
            "not json",
        ] {
            assert!(StaminaRules::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_weakest_sprint_is_still_faster_than_the_walk_on_every_difficulty() {
        use crate::config::{PLAYER_SPRINT_SPEED, PLAYER_WALK_SPEED};
        let min = shipped().min_speed_scale;
        for d in crate::config::difficulty::ALL {
            let p = d.profile();
            let weakest = PLAYER_SPRINT_SPEED * p.sprint_speed * min;
            let walk = PLAYER_WALK_SPEED * p.player_speed;
            assert!(weakest > walk * 1.05, "{}: weakest sprint {weakest} vs walk {walk}", d.name());
        }
    }
}
