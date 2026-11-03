//! Quenelle: flashlight tracker. 8 s of continuous light locks her onto you for good; every
//! flick of the torch after that makes her faster while it is lit, up to x1.45
//! (quenelle_gimmick.gd).

use crate::hunters::brain::{Brain, Target};

pub struct Quenelle {
    tracking: bool,
    elapsed: f32,
    activations: i32,
    was_on: bool,
    pub speed_scale: f32,
}

impl Default for Quenelle {
    fn default() -> Self {
        Quenelle {
            tracking: false,
            elapsed: 0.0,
            activations: 0,
            was_on: false,
            speed_scale: 1.0,
        }
    }
}

impl Quenelle {
    pub fn before(&mut self, b: &mut Brain, dt: f32, t: &Target) {
        let lit = t.flashlight_on && t.lamp_charge > 0.0 && !t.hidden;
        if lit && !self.was_on {
            self.activations += 1;
        }
        self.was_on = lit;
        if !self.tracking {
            self.elapsed = if lit { self.elapsed + dt } else { 0.0 };
            if self.elapsed >= 8.0 {
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
