//! Thessine: sound mimic. Every 10 s, a one-in-two chance of a fake cue, rarely an uncanny
//! one (thessine_gimmick.gd).

use super::Event;
use crate::hunters::brain::Brain;

pub struct Thessine {
    timer: f32,
}

impl Thessine {
    pub fn new() -> Thessine {
        Thessine { timer: 10.0 }
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, ev: &mut Vec<Event>) {
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        self.timer += 10.0;
        if b.next_float() >= 0.5 {
            return;
        }
        let rare = b.next_float() < 0.08;
        let _choice = b.next_float();
        ev.push(Event::Mimic { rare });
    }
}
