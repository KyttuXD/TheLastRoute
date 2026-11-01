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
