//! Solyth (special): heat manifest. Once body temperature reaches 100 the heat latches for the
//! rest of the run and she appears 12 cells away, chasing for good and burning one uncollected
//! battery every 30 s. Dormant on Relaxed (solyth_gimmick.gd).

use super::nyxie::spawn_near;
use super::{Ctx, Event};
use crate::hunters::brain::{Brain, State, Target};

#[derive(Default)]
pub struct Solyth {
    summoned: bool,
    burn: f32,
    /// Latched by the game when the runner's temperature hits 100.
    pub heat_locked: bool,
}

impl Solyth {
    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if c.diff == crate::config::Difficulty::Relaxed {
            b.active = false;
            b.state = State::Idle;
            return;
        }
        if !self.summoned {
            if !self.heat_locked {
                b.active = false;
                b.state = State::Idle;
                return;
            }
            self.summoned = true;
            b.active = true;
            b.pos = spawn_near(c, t.pos, 12.0);
        }
        self.burn += dt;
        if self.burn >= 30.0 {
            self.burn -= 30.0;
            if let Some(i) = (0..c.batteries_taken.len()).find(|i| !c.batteries_taken[*i]) {
                ev.push(Event::TakeBattery(i));
            }
        }
        b.chase(t.pos);
    }
}
