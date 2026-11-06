//! Javelle: hiding timeout. Standing within 2 cells of your locker for 8 s, she pulls you out
//! and is left winded for 2.2 s (javelle_gimmick.gd).

use super::{hiding_exit, Ctx, Event};
use crate::hunters::brain::{dist, Brain, State, Target};

#[derive(Default)]
pub struct Javelle {
    held: f32,
}

impl Javelle {
    /// The mechanic pose she is in: leaning over the locker with the runner inside, hands out to pull the door.
    pub fn mechanic(&self) -> Option<&'static str> {
        (self.held > 0.0).then_some("locker_loom")
    }

    pub fn after(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        let Some(spot) = t.hiding_spot.filter(|_| t.hidden) else {
            self.held = 0.0;
            return;
        };
        let centre = c.fac.hiding[spot].cell.center();
        if dist(b.pos, centre) >= 2.0 {
            self.held = 0.0;
            return;
        }
        b.state = State::Investigate;
        b.last_known = Some(centre);
        b.search_visits = 0;
        self.held += dt;
        if self.held < 8.0 {
            return;
        }
        self.held = 0.0;
        ev.push(Event::Eject {
            to: hiding_exit(c.fac, spot),
        });
        b.apply_stun(2.2, false);
    }
}
