//! Ilyra: corner observer. She stands in a room corner and never walks. Once she has seen you
//! and you leave her sight, 15 s later you are pounced: stunned 5 s wherever you are. Holding
//! the torch on her banishes her for 30 s. Sight x1.8; she never grabs (ilyra_gimmick.gd).

use super::{open_near, Ctx, Event};
use crate::config::Difficulty;
use crate::hunters::brain::{Brain, Target};

/// Seconds from losing sight to the pounce on `diff` (data/gimmicks/ilyra.json): `pounce_delay`, or the
/// difficulty entry (Nightmare 0 = at once). A bad value keeps 15 s.
pub fn pounce_delay(text: &str, diff: Difficulty) -> f32 {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return 15.0 };
    let ok = |x: &serde_json::Value| x.as_f64().filter(|n| *n >= 0.0 && n.is_finite()).map(|n| n as f32);
    let base = v.get("pounce_delay").and_then(ok).unwrap_or(15.0);
    v.get("difficulty").and_then(|d| d.get(diff.id())).and_then(ok).unwrap_or(base)
}

fn delay_for(diff: Difficulty) -> f32 {
    pounce_delay(include_str!("../../data/gimmicks/ilyra.json"), diff)
}

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

    /// The mechanic pose she is in: standing in her corner, watching (not while a beam has banished her).
    pub fn mechanic(&self) -> Option<&'static str> {
        (self.has_corner && !self.banished()).then_some("corner_watch")
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
            self.pounce_remaining = delay_for(c.diff);
            self.saw_player = false;
        }
        if self.tagged {
            self.pounce_remaining -= dt;
            if self.pounce_remaining <= 0.0 {
                ev.push(Event::Stun(5.0));
                // the curse lands: one arm thrown out at the runner wherever they are
                b.raise_cue("pounce_curse");
                self.tagged = false;
                self.respawn_corner(b, c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard keeps the 15 s grace; Nightmare pounces at once (ilyra_gimmick.gd line 52).
    #[test]
    fn shipped_data_pounces_at_once_on_nightmare_only() {
        assert_eq!(delay_for(Difficulty::Nightmare), 0.0);
        for d in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard] {
            assert_eq!(delay_for(d), 15.0);
        }
    }

    /// A data edit changes the delay; bad values keep the default.
    #[test]
    fn data_edits_change_the_delay() {
        let t = r#"{"pounce_delay": 9, "difficulty": {"hard": 2}}"#;
        assert_eq!(pounce_delay(t, Difficulty::Standard), 9.0);
        assert_eq!(pounce_delay(t, Difficulty::Hard), 2.0);
        assert_eq!(pounce_delay("nope", Difficulty::Hard), 15.0);
        assert_eq!(pounce_delay(r#"{"pounce_delay": -3}"#, Difficulty::Hard), 15.0);
    }
}
