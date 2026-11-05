//! Solyth (special): heat manifest. Once body temperature reaches 100 the heat latches for the
//! rest of the run and she appears 12 cells away, chasing for good and burning one uncollected
//! battery every 30 s. Dormant on Relaxed (solyth_gimmick.gd).
//!
//! Nightmare only: while she chases and the runner is at 60 or more degrees she pounces with the
//! shared leap (nyxie.rs; hard_abilities.gd pounce, Nightmare row). The party budget and the wall
//! sweep are checked in the shared pounce_frame (nyxie.rs). Data: data/hunters/nightmare_pounce.json.

use super::nyxie::{pounce, pounce_frame, spawn_near, Leap};
use super::{Ctx, Event};
use crate::hunters::brain::{Brain, State, Target};

#[derive(Default)]
pub struct Solyth {
    summoned: bool,
    burn: f32,
    /// Latched by the game when the runner's temperature hits 100.
    pub heat_locked: bool,
    leap: Leap,
}

impl Solyth {
    /// The leap holds her position (the leap moves her itself).
    pub fn holds_position(&self) -> bool {
        self.leap.leaping
    }

    pub fn speed_scale(&self) -> f32 {
        self.leap.speed_scale(pounce())
    }

    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if c.diff == crate::config::Difficulty::Relaxed {
            b.active = false;
            b.state = State::Idle;
            return;
        }
        if !self.summoned {
            if !self.heat_locked {
                b.active = false;
                b.state = State::Idle;
                return;
            }
            self.summoned = true;
            b.active = true;
            b.pos = spawn_near(c, t.pos, 12.0);
        }
        self.burn += dt;
        if self.burn >= 30.0 {
            self.burn -= 30.0;
            if let Some(i) = (0..c.batteries_taken.len()).find(|i| !c.batteries_taken[*i]) {
                ev.push(Event::TakeBattery(i));
            }
        }
        let gate = t.temperature >= pounce().solyth_min_temperature;
        if pounce_frame(&mut self.leap, b, dt, c, t, gate) {
            return;
        }
        b.chase(t.pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Difficulty;

    #[test]
    fn solyth_wakes_only_when_the_heat_is_locked() {
        use crate::content::Content;
        use crate::hunters::gimmicks::{Ctx, Gimmick};
        use crate::world::catalog::Catalog;
        use crate::world::facility::{Facility, Options};
        let fac = Facility::generate("SOLYTH-1", &Options::default(), &Catalog::load());
        let at = fac.start.center();
        let def = Content::load().hunter("solyth").unwrap().clone();
        let mut b = Brain::new(def, at, "SOLYTH", Difficulty::Standard);
        let t = Target { pos: at, facing: (1.0, 0.0), ..Default::default() };
        let ctx = Ctx {
            fac: &fac,
            relays_taken: &[],
            batteries_taken: &[],
            hiding_disabled: &[],
            vents: &[],
            heater_on: true,
            heater_thawed: true,
            diff: Difficulty::Standard,
            music_box_left: 0.0,
            exposure: 0.0,
        };
        for _ in 0..60 {
            b.update(1.0 / 30.0, &ctx, &t, 1, 0, &Difficulty::Standard.profile());
        }
        assert!(!b.active, "a running heater alone does not wake her");
        // a running heater warms the facility and the body until the temperature reaches 100 and latches
        let mut meters = crate::game::meters::Meters::default();
        let calm = crate::game::meters::Situation {
            flashlight_on: false,
            noise: 0.0,
            crouching: false,
            hidden: false,
            in_safe_zone: false,
            heater_on: true,
            local: 0.0,
        };
        for _ in 0..30 * 120 {
            meters.update(1.0 / 30.0, &calm);
            if meters.heat_locked {
                break;
            }
        }
        assert!(meters.heat_locked, "a running heater heats the runner to 100 and latches");
        let Gimmick::Solyth(s) = &mut b.gimmick else { panic!("not solyth") };
        s.heat_locked = meters.heat_locked;
        b.update(1.0 / 30.0, &ctx, &t, 1, 0, &Difficulty::Standard.profile());
        assert!(b.active, "the latched heat wakes her");
    }

    #[test]
    fn solyth_pounce_gate_is_sixty_degrees_from_data() {
        // Godot hard_abilities.gd _pounce_allowed_for_hunter "solyth": temperature >= 60 (line 215-216).
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../data/hunters/nightmare_pounce.json")).unwrap();
        assert_eq!(crate::hunters::gimmicks::nyxie::NightmarePounce::parse(&v).solyth_min_temperature, 60.0);
    }

    #[test]
    fn solyth_pounce_is_nightmare_only() {
        let n = pounce();
        assert!(n.applies(Difficulty::Nightmare));
        assert!(!n.applies(Difficulty::Hard), "Godot lists Solyth in the Hard pounce roster too; this port is Nightmare only for now");
        assert!(!n.applies(Difficulty::Standard));
        assert!(!n.applies(Difficulty::Relaxed));
    }

    #[test]
    fn solyth_uses_the_party_budget_and_sweep_from_data() {
        // Godot hard_abilities.gd BUDGET nightmare 25.0 and SWEEP_STEP .15: the same numbers Nyxie uses.
        let n = pounce();
        assert_eq!(n.budget_seconds, 25.0);
        assert_eq!(n.sweep_step, 0.15);
        assert!(n.applies(Difficulty::Nightmare));
    }

    #[test]
    fn a_fresh_solyth_is_not_leaping_and_has_full_speed() {
        let s = Solyth::default();
        assert!(!s.holds_position());
        assert_eq!(s.speed_scale(), 1.0);
    }
}
