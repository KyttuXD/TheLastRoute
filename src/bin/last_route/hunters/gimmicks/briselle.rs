//! Briselle: relay tracker. Every relay recovered after she wakes draws her to it for 12 s at
//! x1.25 speed; holding the torch on her breaks the lock and stuns her 3 s (briselle_gimmick.gd).

use super::Ctx;
use crate::hunters::brain::{Brain, State};

pub struct Briselle {
    locked: bool,
    lock_remaining: f32,
    relay: (f32, f32),
    seen: i32,
    known: Vec<usize>,
}

impl Default for Briselle {
    fn default() -> Self {
        Briselle {
            locked: false,
            lock_remaining: 0.0,
            relay: (0.0, 0.0),
            seen: -1,
            known: vec![],
        }
    }
}

impl Briselle {
    /// The window moved by `d`.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.relay = (self.relay.0 + d.0, self.relay.1 + d.1);
    }

    pub fn after(&mut self, b: &mut Brain, dt: f32, c: &Ctx) {
        let collected: Vec<usize> = (0..c.relays_taken.len())
            .filter(|i| c.relays_taken[*i])
            .collect();
        if self.seen < 0 {
            self.seen = collected.len() as i32;
        } else if collected.len() as i32 > self.seen {
            let newest = collected
                .iter()
                .rev()
                .find(|i| !self.known.contains(i))
                .copied()
                .unwrap_or(*collected.last().unwrap());
            self.relay = c.fac.relays[newest].center();
            self.locked = true;
            self.lock_remaining = 12.0;
            b.investigate(self.relay);
        }
        self.seen = collected.len() as i32;
        self.known = collected;
        if !self.locked {
            return;
        }
        if b.flash_triggered {
            self.locked = false;
            self.lock_remaining = 0.0;
            b.apply_stun(3.0, false);
            b.state = State::Search;
            b.last_known = None;
            return;
        }
        self.lock_remaining -= dt;
        if self.lock_remaining <= 0.0 {
            self.locked = false;
            return;
        }
        if b.state != State::Chase {
            b.state = State::Investigate;
            b.last_known = Some(self.relay);
        }
    }

    /// The mechanic pose she is in: locked onto a relay (an arm out toward it) until the lock runs out or a beam breaks it.
    pub fn mechanic(&self) -> Option<&'static str> {
        self.locked.then_some("relay_lock")
    }

    pub fn speed_scale(&self) -> f32 {
        if self.locked {
            1.25
        } else {
            1.0
        }
    }
}
