//! Verrine: vent lurker (the finite floor's ring of four grates). She is invisible and inert
//! until you enter the vents; then she appears two grates on and hunts you, getting faster the
//! longer you stay in (x0.4 rising 0.025/s to x2.4) (verrine_gimmick.gd, legacy mode).

use crate::hunters::brain::{Brain, State, Target};

pub struct Verrine {
    pub active_in_vent: bool,
    pub speed_scale: f32,
}

impl Default for Verrine {
    fn default() -> Self {
        Verrine {
            active_in_vent: false,
            speed_scale: 1.0,
        }
    }
}

impl Verrine {
    pub fn after(&mut self, b: &mut Brain, t: &Target) {
        self.active_in_vent = t.in_vent;
        if !t.in_vent {
            b.state = State::Idle;
            b.idle_timer = 1.0;
            b.last_known = None;
            b.lost_sight = 0.0;
            return;
        }
        b.chase(t.pos);
        self.speed_scale = (0.4 + t.vent_time_in * 0.025).clamp(0.0, 2.4);
    }
}
