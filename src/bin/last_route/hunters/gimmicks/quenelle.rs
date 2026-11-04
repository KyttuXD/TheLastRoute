//! Quenelle: flashlight tracker. 8 s of continuous light locks her onto you for good; every
//! flick of the torch after that makes her faster while it is lit, up to x1.45
//! (quenelle_gimmick.gd).

use crate::config::Difficulty;
use crate::hunters::brain::{Brain, Target};

pub struct Quenelle {
    tracking: bool,
    elapsed: f32,
    activations: i32,
    was_on: bool,
    /// Continuous torch time before the permanent lock: QUENELLE_FLASHLIGHT_LOCK_SECONDS (8 s) on
    /// Standard, QUENELLE_NIGHTMARE_LOCK_SECONDS (5 s) on Nightmare.
    lock_seconds: f32,
    pub speed_scale: f32,
}

/// game_config.gd QUENELLE_FLASHLIGHT_LOCK_SECONDS.
pub const FLASHLIGHT_LOCK_SECONDS: f32 = 8.0;
/// game_config.gd QUENELLE_NIGHTMARE_LOCK_SECONDS.
pub const NIGHTMARE_LOCK_SECONDS: f32 = 5.0;

impl Default for Quenelle {
    fn default() -> Self {
        Quenelle {
            tracking: false,
            elapsed: 0.0,
            activations: 0,
            was_on: false,
            lock_seconds: FLASHLIGHT_LOCK_SECONDS,
            speed_scale: 1.0,
        }
    }
}

impl Quenelle {
    /// Quenelle on Nightmare: the lock is 5 s. Relaxed, Standard and Hard keep the 8 s lock.
    pub fn nightmare() -> Quenelle {
        Quenelle {
            lock_seconds: NIGHTMARE_LOCK_SECONDS,
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
            }
        }
        let progress = (self.activations as f32 / 10.0).min(1.0);
        self.speed_scale = if self.tracking && lit {
            1.0 + progress * 0.45
        } else {
            1.0
        };
        if self.tracking && !t.in_safe_zone {
            b.chase(t.pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard keeps the 8 s lock; Nightmare locks on at 5 s (game_config.gd).
    #[test]
    fn lock_on_is_eight_seconds_standard_and_five_on_nightmare() {
        assert_eq!(Quenelle::default().lock_seconds(), 8.0);
        assert_eq!(Quenelle::nightmare().lock_seconds(), 5.0);
        assert_eq!(Quenelle::for_difficulty(Difficulty::Nightmare).lock_seconds(), 5.0);
        assert_eq!(Quenelle::for_difficulty(Difficulty::Standard).lock_seconds(), 8.0);
        assert_eq!(Quenelle::for_difficulty(Difficulty::Hard).lock_seconds(), 8.0);
        assert_eq!(FLASHLIGHT_LOCK_SECONDS, 8.0);
        assert_eq!(NIGHTMARE_LOCK_SECONDS, 5.0);
    }

    /// Nightmare keeps the Standard speed ramp: x1.45 cap after 10 flicks.
    #[test]
    fn nightmare_only_changes_the_lock_time() {
        let q = Quenelle::nightmare();
        assert_eq!(q.speed_scale, 1.0);
        assert!(!q.tracking);
        assert_eq!(q.activations, 0);
    }
}
