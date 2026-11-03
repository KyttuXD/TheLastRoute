//! Loxelle: passive incinerator. While calm, every 5 s a one-in-four chance she charges a
//! locker in her room: an occupied one throws you out hot (+30 temperature, +11 noise), an
//! empty one burns out for 30 s. Either way she stalls 10 s. She never grabs
//! (loxelle_gimmick.gd).

use super::{hiding_exit, Ctx, Event};
use crate::hunters::brain::{dist, Brain, State, Target};

#[derive(Clone, Copy, PartialEq, Default)]
enum Phase {
    #[default]
    None,
    Approach,
    Windup,
    Charge,
}

#[derive(Default)]
pub struct Loxelle {
    passive_timer: f32,
    regenerating: Vec<(usize, f32)>,
    phase: Phase,
    spot: usize,
    timer: f32,
}

impl Loxelle {
    pub fn holds_position(&self) -> bool {
        matches!(self.phase, Phase::Windup | Phase::Charge)
    }

    pub fn after(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if t.pesticide {
            return;
        }
        for r in self.regenerating.iter_mut() {
            r.1 -= dt;
            if r.1 <= 0.0 {
                ev.push(Event::SpotDisabled(r.0, false));
            }
        }
        self.regenerating.retain(|r| r.1 > 0.0);
        if self.phase != Phase::None {
            self.update_charge(b, dt, c, t, ev);
            return;
        }
        if matches!(b.state, State::Chase | State::Investigate) {
            self.passive_timer = 0.0;
            return;
        }
        self.passive_timer += dt;
        if self.passive_timer < 5.0 {
            return;
        }
        self.passive_timer = 0.0;
        if b.next_index(1000) >= 250 {
            return;
        }
        let Some(room) = c.fac.room_at(b.pos.0 as f64, b.pos.1 as f64) else {
            return;
        };
        let cand = (0..c.fac.hiding.len())
            .filter(|i| {
                c.fac.hiding[*i].room == Some(room) && !self.regenerating.iter().any(|r| r.0 == *i)
            })
            .min_by(|p, q| {
                let dp = dist(c.fac.hiding[*p].cell.center(), b.pos);
                let dq = dist(c.fac.hiding[*q].cell.center(), b.pos);
                dp.partial_cmp(&dq).unwrap()
            });
        let Some(spot) = cand else { return };
        self.spot = spot;
        self.phase = Phase::Approach;
        b.investigate(c.fac.hiding[spot].cell.center());
    }

    fn update_charge(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        let impact = c.fac.hiding[self.spot].cell.center();
        b.state = State::Investigate;
        b.last_known = Some(impact);
        let d = dist(b.pos, impact);
        match self.phase {
            Phase::Approach => {
                if d <= 0.95 {
                    self.phase = Phase::Windup;
                    self.timer = 0.42;
                }
            }
            Phase::Windup => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.phase = Phase::Charge;
                    self.timer = 0.70;
                }
            }
            Phase::Charge => {
                self.timer -= dt;
                if d > 0.20 {
                    let travel = (6.8 * dt).min(d);
                    let dir = ((impact.0 - b.pos.0) / d, (impact.1 - b.pos.1) / d);
                    b.facing = dir;
                    let next = (b.pos.0 + dir.0 * travel, b.pos.1 + dir.1 * travel);
                    if c.fac.can_occupy(next.0 as f64, next.1 as f64, 0.33) {
                        b.pos = next;
                    }
                }
                if dist(b.pos, impact) <= 0.20 || self.timer <= 0.0 {
                    if t.hidden && t.hiding_spot == Some(self.spot) {
                        ev.push(Event::Eject {
                            to: hiding_exit(c.fac, self.spot),
                        });
                        ev.push(Event::Meter {
                            exposure_add: 0.0,
                            noise_add: 11.0,
                            temperature_add: 30.0,
                        });
                    } else {
                        ev.push(Event::SpotDisabled(self.spot, true));
                        self.regenerating.push((self.spot, 30.0));
                    }
                    self.phase = Phase::None;
                    b.apply_stun(10.0, false);
                }
            }
            Phase::None => {}
        }
    }
}
