//! Elise: running-data ratchet. Sprinting outside the safe zone fills a meter that never
//! decays; after 50 s of it she chases you forever, through lockers (elise_gimmick.gd).

use crate::hunters::brain::{Brain, Target};

#[derive(Default)]
pub struct Elise {
    progress: f32,
    locked: bool,
}

impl Elise {
    pub fn before(&mut self, b: &mut Brain, dt: f32, t: &Target) {
        if !self.locked {
            if t.sprinting && !t.in_safe_zone {
                self.progress = (self.progress + dt / 50.0).min(1.0);
            }
            if self.progress >= 1.0 {
                self.locked = true;
                // the fifty seconds are up: she snaps round to the runner for good
                b.raise_cue("lock_on");
            }
        }
        if self.locked {
            b.chase(t.pos);
        }
    }
}
