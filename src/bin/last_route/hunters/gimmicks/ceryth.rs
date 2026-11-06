//! Ceryth: hiding trap. She waits beside a locker with red or blue eyes. Enter it with the
//! light off for red, on for blue, and she runs off to another locker; guess wrong and you are
//! locked inside for 30 s. She never grabs (ceryth_gimmick.gd).

use super::{hiding_exit, Ctx};
use crate::hunters::brain::{dist, Brain, State};
use crate::world::Cell;

pub struct Ceryth {
    trap_spot: Option<usize>,
    pub eyes_red: bool,
    pub dispersed: bool,
    retreating: bool,
    answered: bool,
    next_spot: usize,
    placed: bool,
}

impl Default for Ceryth {
    fn default() -> Self {
        Ceryth {
            trap_spot: None,
            eyes_red: true,
            dispersed: false,
            retreating: false,
            answered: false,
            next_spot: 0,
            placed: false,
        }
    }
}

pub enum Entry {
    Normal,
    Trapped,
    Cleared,
}

impl Ceryth {
    pub fn after(&mut self, b: &mut Brain, c: &Ctx) {
        if self.dispersed {
            return;
        }
        if self.answered && !self.retreating {
            self.answered = false;
            self.start_retreat(b, c);
            if self.dispersed {
                return;
            }
        }
        if self.retreating {
            let perch = hiding_exit(c.fac, self.next_spot);
            b.state = State::Investigate;
            b.last_known = Some(perch);
            if dist(b.pos, perch) < 0.9 {
                self.retreating = false;
                self.trap_spot = Some(self.next_spot);
                self.eyes_red = b.next_index(2) == 0;
                b.pos = perch;
            }
            return;
        }
        if !self.placed {
            self.placed = true;
            if c.fac.hiding.is_empty() {
                return;
            }
            let spot = b.next_index(c.fac.hiding.len());
            self.trap_spot = Some(spot);
            self.eyes_red = b.next_index(2) == 0;
            b.pos = hiding_exit(c.fac, spot);
        }
    }

    fn start_retreat(&mut self, b: &mut Brain, c: &Ctx) {
        let here = Cell::new(b.pos.0 as i32, b.pos.1 as i32);
        let options: Vec<usize> = (0..c.fac.hiding.len())
            .filter(|i| Some(*i) != self.trap_spot && c.fac.hiding[*i].cell.len_to(here) >= 7.0)
            .collect();
        if options.is_empty() {
            self.dispersed = true;
            return;
        }
        self.next_spot = options[b.next_index(options.len())];
        self.retreating = true;
        self.trap_spot = None;
    }

    /// The locker she waits beside (tests enter it).
    #[cfg(test)]
    pub fn trap_spot(&self) -> Option<usize> {
        self.trap_spot
    }

    pub fn holds_position(&self) -> bool {
        !self.dispersed && !self.retreating && self.trap_spot.is_some()
    }

    /// The mechanic pose she is in: standing watch beside her locker, eyes on the door.
    pub fn mechanic(&self) -> Option<&'static str> {
        self.holds_position().then_some("locker_vigil")
    }

    pub fn speed_scale(&self, b: &Brain) -> f32 {
        if self.retreating {
            6.4 / b.def.speed.max(0.1)
        } else {
            1.0
        }
    }

    /// The runner enters a locker; `lit` is the torch on with charge left.
    pub fn resolve_entry(&mut self, spot: usize, lit: bool) -> Entry {
        if self.dispersed || self.trap_spot != Some(spot) {
            return Entry::Normal;
        }
        let right = if self.eyes_red { !lit } else { lit };
        if right {
            self.answered = true;
            Entry::Cleared
        } else {
            Entry::Trapped
        }
    }
}
