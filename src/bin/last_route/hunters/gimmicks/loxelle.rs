//! Loxelle: passive incinerator. While calm, every 5 s a one-in-four chance she charges a
//! locker in her room: an occupied one throws you out hot (+30 temperature, +11 noise), an
//! empty one burns out for 30 s. Either way she stalls 10 s. She never grabs
//! (loxelle_gimmick.gd). Her interval, odds, windup and run come from data/hunters/charge.json
//! (see charge.rs); the defaults there are the Standard values.

use super::charge::{self, ChargeSpec};
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

/// Seconds the Approach phase may last before she gives up (data/gimmicks/loxelle.json `approach_timeout`);
/// a missing or bad value keeps 8 s.
pub fn approach_timeout(text: &str) -> f32 {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("approach_timeout").and_then(|x| x.as_f64()))
        .filter(|n| *n > 0.0 && n.is_finite())
        .map_or(8.0, |n| n as f32)
}

fn approach_cap() -> f32 {
    static T: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *T.get_or_init(|| approach_timeout(include_str!("../../data/gimmicks/loxelle.json")))
}

pub struct Loxelle {
    /// Loxelle's own timing (data/hunters/charge.json, hunter "loxelle").
    spec: ChargeSpec,
    passive_timer: f32,
    regenerating: Vec<(usize, f32)>,
    phase: Phase,
    spot: usize,
    timer: f32,
    approach_time: f32,
}

impl Default for Loxelle {
    fn default() -> Self {
        let spec = charge::for_hunter("loxelle").expect("data/hunters/charge.json has loxelle").clone();
        Loxelle::with_spec(spec)
    }
}

impl Loxelle {
    pub fn with_spec(spec: ChargeSpec) -> Loxelle {
        Loxelle {
            spec,
            passive_timer: 0.0,
            regenerating: Vec::new(),
            phase: Phase::None,
            spot: 0,
            timer: 0.0,
            approach_time: 0.0,
        }
    }

    /// Her room-door breach timing (charge.json `door`), read by the door hook when it is ported
    /// (loxelle_gimmick.gd `start_door_breach` / `_update_door_charge`).
    pub fn door_timing(&self) -> &charge::Door {
        &self.spec.door
    }

    pub fn holds_position(&self) -> bool {
        matches!(self.phase, Phase::Windup | Phase::Charge)
    }

    /// The mechanic pose she is in: coiled back for the windup, then driving forward for the charge.
    pub fn mechanic(&self) -> Option<&'static str> {
        match self.phase {
            Phase::Windup => Some("charge_windup"),
            Phase::Charge => Some("charge_lunge"),
            _ => None,
        }
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
        if self.passive_timer < self.spec.passive.interval {
            return;
        }
        self.passive_timer = 0.0;
        if !self.spec.passive.roll_lunges(b.next_index(1000)) {
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
        self.approach_time = 0.0;
        b.investigate(c.fac.hiding[spot].cell.center());
    }

    fn update_charge(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if self.phase == Phase::Approach {
            self.approach_time += dt;
            if self.approach_time > approach_cap() {
                // she cannot reach the locker: drop it and let the brain carry on (no forced Investigate)
                self.phase = Phase::None;
                self.passive_timer = 0.0;
                b.last_known = None;
                b.state = State::Wander;
                return;
            }
        }
        let impact = c.fac.hiding[self.spot].cell.center();
        b.state = State::Investigate;
        b.last_known = Some(impact);
        let d = dist(b.pos, impact);
        let lunge = self.spec.lunge.clone();
        match self.phase {
            Phase::Approach => {
                if d <= lunge.start_distance {
                    self.phase = Phase::Windup;
                    self.timer = lunge.windup;
                }
            }
            Phase::Windup => {
                self.timer -= dt;
                if self.timer <= 0.0 {
                    self.phase = Phase::Charge;
                    self.timer = lunge.seconds;
                }
            }
            Phase::Charge => {
                self.timer -= dt;
                if d > lunge.impact_distance {
                    let travel = (lunge.speed * dt).min(d);
                    let dir = ((impact.0 - b.pos.0) / d, (impact.1 - b.pos.1) / d);
                    b.facing = dir;
                    // the brain's own collided step: swept in short sub-steps against the runner's rule, so a long frame
                    // never carries the charge through a prop, a door, a locker or a well
                    b.move_with_collision(dir.0 * travel, dir.1 * travel, c.fac);
                }
                if dist(b.pos, impact) <= lunge.impact_distance || self.timer <= 0.0 {
                    if t.hidden && t.hiding_spot == Some(self.spot) {
                        ev.push(Event::Eject {
                            to: hiding_exit(c.fac, self.spot),
                        });
                        ev.push(Event::Meter {
                            exposure_add: 0.0,
                            noise_add: self.spec.passive.break_noise,
                            temperature_add: self.spec.passive.break_temperature,
                        });
                    } else {
                        ev.push(Event::SpotDisabled(self.spot, true));
                        self.regenerating.push((self.spot, self.spec.passive.regen));
                    }
                    self.phase = Phase::None;
                    b.apply_stun(self.spec.passive.stall, false);
                }
            }
            Phase::None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped data gives Loxelle the Standard numbers she had before (game_config.gd 497-515).
    #[test]
    fn default_loxelle_uses_the_standard_data() {
        let l = Loxelle::default();
        assert_eq!(l.spec.passive.interval, 5.0);
        assert_eq!(l.spec.passive.regen, 30.0);
        assert_eq!(l.spec.passive.stall, 10.0);
        assert_eq!(l.spec.lunge.windup, 0.42);
        assert_eq!(l.spec.lunge.seconds, 0.70);
        assert_eq!(l.spec.lunge.speed, 6.8);
        assert!(!l.holds_position());
    }

    /// The door breach reads its own windup, run cap and stun from the data (Standard: .42, .42, .65).
    #[test]
    fn default_loxelle_door_uses_the_standard_data() {
        let l = Loxelle::default();
        assert_eq!(l.door_timing().windup, 0.42);
        assert_eq!(l.door_timing().charge_cap, 0.42);
        assert_eq!(l.door_timing().stun, 0.65);
    }

    /// Editing the door block in the data reaches Loxelle without touching the lunge numbers.
    #[test]
    fn a_door_data_edit_reaches_loxelle_and_leaves_the_lunge_alone() {
        let mut spec = charge::for_hunter("loxelle").unwrap().clone();
        spec.door.stun = 2.0;
        spec.door.windup = 0.10;
        let l = Loxelle::with_spec(spec.clone());
        assert_eq!(l.door_timing().stun, 2.0);
        assert_eq!(l.door_timing().windup, 0.10);
        assert_eq!(l.spec.lunge, spec.lunge);
        assert_eq!(l.spec.lunge.windup, 0.42);
    }

    /// Editing the lunge in the data changes the spec Loxelle runs with.
    #[test]
    fn a_data_edit_reaches_loxelle() {
        let mut spec = charge::for_hunter("loxelle").unwrap().clone();
        spec.lunge.windup = 0.10;
        spec.passive.stall = 3.0;
        let l = Loxelle::with_spec(spec);
        assert_eq!(l.spec.lunge.windup, 0.10);
        assert_eq!(l.spec.passive.stall, 3.0);
    }

    /// The Approach phase has a data cap that falls back; bad data keeps 8 s.
    #[test]
    fn approach_timeout_reads_data_and_falls_back() {
        assert_eq!(approach_timeout(r#"{"approach_timeout": 3.5}"#), 3.5);
        assert_eq!(approach_timeout("{}"), 8.0);
        assert_eq!(approach_timeout(r#"{"approach_timeout": 0}"#), 8.0);
        let shipped = approach_timeout(include_str!("../../data/gimmicks/loxelle.json"));
        assert!(shipped > 0.0 && shipped < 60.0);
        assert_eq!(Loxelle::default().approach_time, 0.0);
    }
}
