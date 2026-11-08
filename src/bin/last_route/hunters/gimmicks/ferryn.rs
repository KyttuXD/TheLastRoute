//! Ferryn: persistent stalker. Always chasing at a slow 2.75; hiding, the safe zone or a held
//! torch sends her back to her far-off origin room, and a flash alerts every other hunter
//! (ferryn_gimmick.gd).

use super::{open_near, Ctx, Event};
use crate::hunters::brain::{Brain, State, Target};

#[derive(Default)]
pub struct Ferryn {
    origin: Option<(f32, f32)>,
}

impl Ferryn {
    /// The window moved under her by `d`: her room (the return point) moves with the cells.
    pub fn shift(&mut self, d: (f32, f32)) {
        self.origin = self.origin.map(|o| (o.0 + d.0, o.1 + d.1));
    }

    /// Her return point (tests).
    #[cfg(test)]
    pub fn origin(&self) -> Option<(f32, f32)> {
        self.origin
    }

    pub fn after(&mut self, b: &mut Brain, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if self.origin.is_none() {
            let start_room = c
                .fac
                .room_at(c.fac.start.x as f64 + 0.5, c.fac.start.y as f64 + 0.5);
            let cands: Vec<usize> = (0..c.fac.rooms.len())
                .filter(|i| Some(*i) != start_room)
                .collect();
            let o = if cands.is_empty() {
                b.pos
            } else {
                let r = &c.fac.rooms[cands[b.next_index(cands.len())]];
                open_near(c.fac, r.center.center())
            };
            self.origin = Some(o);
            b.pos = o;
        }
        let origin = self.origin.unwrap();
        let flashed = b.flash_triggered;
        if flashed || t.hidden || t.in_safe_zone {
            b.pos = origin;
            b.state = State::Wander;
            b.last_known = None;
            if flashed {
                // the flash drives her back to her room: she is back there, arms over her face
                b.raise_cue("flash_recoil");
                ev.push(Event::Meter {
                    exposure_add: 0.0,
                    noise_add: 100.0,
                    temperature_add: 0.0,
                });
                ev.push(Event::FacilityAlert {
                    at: t.pos,
                    except_self: true,
                });
            }
            return;
        }
        b.chase(t.pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::gimmicks::Gimmick;

    #[test]
    fn a_window_shift_moves_her_return_point_with_the_cells() {
        let mut g = Gimmick::Ferryn(Ferryn { origin: Some((50.0, 10.0)) });
        g.shift((-32.0, 16.0));
        let Gimmick::Ferryn(f) = &g else { unreachable!() };
        assert_eq!(f.origin(), Some((18.0, 26.0)));
        // before she has picked a room there is nothing to move
        let mut none = Gimmick::Ferryn(Ferryn::default());
        none.shift((5.0, 5.0));
        let Gimmick::Ferryn(f) = &none else { unreachable!() };
        assert_eq!(f.origin(), None);
    }
}
