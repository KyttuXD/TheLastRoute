//! Aki: phantom exposure. Flashing her starts a 10 s fuse, then three bursts 5 s apart that
//! spike the runner's exposure to 100 (aki_gimmick.gd).

use super::{Ctx, Event};
use crate::hunters::brain::Brain;

#[derive(Default)]
pub struct Aki {
    armed: bool,
    bursts_remaining: i32,
    timer: f32,
}

impl Aki {
    pub fn before(&mut self, b: &mut Brain, dt: f32, _c: &Ctx, ev: &mut Vec<Event>) {
        if !self.armed {
            // reads last frame's flash, as the original's hook order does
            if b.flash_triggered {
                self.armed = true;
                self.timer = 10.0;
                self.bursts_remaining = 3;
            }
            return;
        }
        self.timer = (self.timer - dt).max(0.0);
        if self.timer > 0.0 {
            return;
        }
        ev.push(Event::Meter {
            exposure_add: 100.0,
            noise_add: 0.0,
            temperature_add: 0.0,
        });
        self.bursts_remaining = (self.bursts_remaining - 1).max(0);
        if self.bursts_remaining <= 0 {
            self.armed = false;
            self.timer = 0.0;
        } else {
            self.timer = 5.0;
        }
    }
}
