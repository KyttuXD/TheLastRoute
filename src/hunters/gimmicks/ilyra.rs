//! Ilyra: corner observer. She stands in a room corner and never walks. Once she has seen you
//! and you leave her sight, 15 s later you are pounced: stunned 5 s wherever you are. Holding
//! the torch on her banishes her for 30 s. Sight x1.8; she never grabs (ilyra_gimmick.gd).

use super::{open_near, Ctx, Event};
use crate::hunters::brain::{Brain, Target};

#[derive(Default)]
pub struct Ilyra {
    has_corner: bool,
    hidden_remaining: f32,
    saw_player: bool,
    tagged: bool,
    pounce_remaining: f32,
}

impl Ilyra {
    pub fn banished(&self) -> bool {
        self.hidden_remaining > 0.0
    }

    fn respawn_corner(&mut self, b: &mut Brain, c: &Ctx) {
        let rooms = &c.fac.rooms;
        let r = &rooms[b.next_index(rooms.len())];
        let (x, y, w, h) = (r.x as f32, r.y as f32, r.w as f32, r.h as f32);
        let corners = [
            (x + 1.5, y + 1.5),
            (x + w - 1.5, y + 1.5),
            (x + 1.5, y + h - 1.5),
            (x + w - 1.5, y + h - 1.5),
        ];
        let corner = corners[b.next_index(4)];
        b.pos = open_near(c.fac, corner);
        self.has_corner = true;
    }

    pub fn after(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if !self.has_corner {
            self.respawn_corner(b, c);
        }
        if self.hidden_remaining > 0.0 {
            self.hidden_remaining -= dt;
            return;
        }
        if b.flash_triggered {
            self.hidden_remaining = 30.0;
            self.saw_player = false;
            self.tagged = false;
            self.pounce_remaining = 0.0;
            return;
        }
        let her_room = c.fac.room_at(b.pos.0 as f64, b.pos.1 as f64);
        let shares = !t.hidden
            && !t.in_safe_zone
            && her_room.is_some()
            && her_room == c.fac.room_at(t.pos.0 as f64, t.pos.1 as f64);
        let sees = b.direct_visible || shares;
        if sees {
            self.saw_player = true;
        } else if self.saw_player && !self.tagged {
            self.tagged = true;
            self.pounce_remaining = 15.0;
            self.saw_player = false;
        }
        if self.tagged {
            self.pounce_remaining -= dt;
            if self.pounce_remaining <= 0.0 {
                ev.push(Event::Stun(5.0));
                self.tagged = false;
                self.respawn_corner(b, c);
            }
        }
    }
}
