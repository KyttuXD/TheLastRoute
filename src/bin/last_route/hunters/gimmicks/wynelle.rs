//! Wynelle: cold materializer. Below 60 degrees a hidden meter fills (faster the colder, up to
//! 100); then she appears on you and you have 10 s to thaw the heater or reach the safe zone
//! before you freeze. Dormant on Relaxed (wynelle_gimmick.gd).

use super::{Ctx, Event};
use crate::hunters::brain::{Brain, State, Target};

#[derive(Default)]
pub struct Wynelle {
    materialized: bool,
    progress: f32,
    death_timer: f32,
}

impl Wynelle {
    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if c.diff == crate::config::Difficulty::Relaxed {
            b.active = false;
            b.state = State::Idle;
            self.progress = 0.0;
            return;
        }
        if self.materialized {
            if c.heater_thawed || t.in_safe_zone {
                self.materialized = false;
                self.progress = 50.0;
                b.active = false;
                b.state = State::Idle;
                return;
            }
            self.death_timer -= dt;
            if self.death_timer <= 0.0 {
                ev.push(Event::InescapableCapture);
                self.progress = 50.0;
                self.materialized = false;
                b.active = false;
            }
            return;
        }
        b.active = false;
        b.state = State::Idle;
        if c.heater_on || t.in_safe_zone {
            self.progress = 0.0;
            return;
        }
        let temp = t.temperature;
        if temp >= 60.0 {
            self.progress = 0.0;
            return;
        }
        let rate = if temp < 30.0 {
            1.0
        } else {
            (60.0 - temp) * 0.5
        };
        self.progress = (self.progress + rate * dt).min(100.0);
        if self.progress < 100.0 {
            return;
        }
        self.materialized = true;
        b.active = true;
        b.pos = t.pos;
        b.chase(t.pos);
        self.death_timer = 10.0;
    }
}
