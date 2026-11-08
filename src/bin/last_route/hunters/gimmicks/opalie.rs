//! Opalie: relay guardian. She stands on the first open relay; while on station nothing within
//! 1.5 cells can be used and she cannot grab. She leaves her post only to chase
//! (opalie_gimmick.gd).

use super::{open_near, Ctx};
use crate::hunters::brain::{dist, Brain, State, Target};

#[derive(Default)]
pub struct Opalie {
    pub on_station: bool,
    station: (f32, f32),
}

impl Opalie {
    /// The window moved by `d`.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.station = (self.station.0 + d.0, self.station.1 + d.1);
    }

    pub fn after(&mut self, b: &mut Brain, c: &Ctx, t: &Target) {
        if c.fac.relays.is_empty() {
            return;
        }
        let i = (0..c.relays_taken.len())
            .find(|i| !c.relays_taken[*i])
            .unwrap_or(0);
        let relay = c.fac.relays[i].center();
        let post = open_near(c.fac, relay);
        if b.state == State::Chase && !t.hidden {
            self.on_station = false;
            return;
        }
        if dist(b.pos, post) > 0.75 {
            self.on_station = false;
            b.state = State::Investigate;
            b.last_known = Some(post);
            return;
        }
        self.on_station = true;
        self.station = relay;
    }

    /// The mechanic pose she is in: standing guard on her relay.
    pub fn mechanic(&self) -> Option<&'static str> {
        self.on_station.then_some("guard_post")
    }

    /// The interaction veto: anything within 1.5 cells of her post.
    pub fn blocks(&self, p: (f32, f32)) -> bool {
        self.on_station && dist(self.station, p) <= 1.5
    }
}
