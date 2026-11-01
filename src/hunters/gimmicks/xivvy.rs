//! Xivvy: chase accelerator. Coming within a short radius (0.75 cells, up to 1.5 with full
//! exposure) locks her on without sight; the longer a chase runs the faster she gets, up to
//! x1.8, taking wing after 0.65 s (xivvy_gimmick.gd).

use super::Ctx;
use crate::hunters::brain::{dist, Brain, State, Target};

pub struct Xivvy {
    chase_elapsed: f32,
    pub speed_scale: f32,
    pub flying: bool,
}

impl Default for Xivvy {
    fn default() -> Self {
        Xivvy {
            chase_elapsed: 0.0,
            speed_scale: 1.0,
            flying: false,
        }
    }
}

impl Xivvy {
    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target) {
        let lock = 1.5 * (0.5 + c.exposure / 200.0);
        if !t.hidden && !t.in_safe_zone && dist(b.pos, t.pos) <= lock {
            b.state = State::Chase;
            b.last_known = Some(t.pos);
        }
        self.chase_elapsed = if b.state == State::Chase {
            self.chase_elapsed + dt
        } else {
            0.0
        };
        self.flying = self.chase_elapsed >= 0.65;
        self.speed_scale = (1.0 + self.chase_elapsed * 0.035).min(1.8);
    }
}
