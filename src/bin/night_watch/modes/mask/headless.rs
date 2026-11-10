#![allow(dead_code)]
//! Masked Hours without a window: the rules, the control machine, the hunters walking the nav graph and a seeded autoplay
//! policy. Used by `--check`, the tests and the integration of `registry::make_headless`. No drawing, no GL.
//!
//! The policy plays like a careful but imperfect guard: mask on while Thessine stands in the office, the monitor up to wind
//! the music box when it runs low, the hall light flashed at Umbriel, the vent light held on the side a vent hunter waits at.
//! It decides every 300 ms and sometimes misses, from its own seeded stream, so a seed always plays the same night.

use super::controls::{MaskControls, Panel};
use super::sim::{HallState, MaskSim, Over, RushState, VentState};
use super::walk::Stage;
use crate::core::{Fnv, Headless, Mulberry32, Outcome, Session};
use macroquad::prelude::vec3;

/// Where the player stands (the hunters at the office face this).
const PLAYER: macroquad::prelude::Vec3 = vec3(0.0, 1.7, 1.35);

pub struct MaskHeadless {
    pub sim: MaskSim,
    pub ctl: MaskControls,
    pub stage: Stage,
    bot: Mulberry32,
    since_decision: f64,
    flip: bool,
    pub night: i32,
}

impl MaskHeadless {
    pub fn new(session: &Session) -> MaskHeadless {
        let custom = session.is_custom;
        let night = if custom { 5 } else { session.night.clamp(1, 6) };
        let seed = session.seed_or_fresh();
        let sim = MaskSim::new(night, seed, session.custom_levels.clone(), custom);
        let mut stage = Stage::new(seed);
        stage.sync(&sim, PLAYER, true);
        MaskHeadless { sim, ctl: MaskControls::new(), stage, bot: Mulberry32::new(seed ^ 0x4d41_534b), since_decision: 1e9, flip: false, night }
    }

    /// The autoplay policy: what the guard does now.
    fn decide(&mut self) {
        let s = &self.sim;
        let office = s.hall.state == HallState::Office;
        let running = matches!(s.rush.state, RushState::Run | RushState::Door);
        let occupant = s.vent_occupant();
        let low_box = s.box_pct < 45.0 || (s.box_pct < 70.0 && occupant.is_none() && !running);
        // the panel to have up
        let want = if office {
            Panel::Mask
        } else if low_box && !running {
            Panel::Monitor
        } else {
            Panel::None
        };
        // an imperfect guard: now and then the decision is simply missed (never when the mask is the only thing between her and the door)
        if self.bot.next() < 0.12 && !office {
            return;
        }
        let have = self.ctl.want;
        if want != have {
            if have != Panel::None && want == Panel::None {
                let a = if have == Panel::Mask { "mask" } else { "monitor" };
                self.ctl.press(a, true);
            } else if want == Panel::Mask {
                self.ctl.press("mask", true);
            } else {
                self.ctl.press("monitor", true);
            }
        }
        self.ctl.press("winKey", want == Panel::Monitor);
        let clear = want == Panel::None;
        // the hall light: quick flashes at Umbriel
        self.flip = !self.flip;
        self.ctl.press("lightH", clear && running && self.flip);
        // the vent light: held on the side a vent hunter waits at
        let mut left = false;
        let mut right = false;
        if let Some(o) = &occupant {
            if o.state == VentState::Vent && clear {
                left = o.side == 'L';
                right = o.side == 'R';
            }
        }
        self.ctl.press("lightL", left);
        self.ctl.press("lightR", right);
    }

    /// FNV-1a of the rules' state and every hunter's position.
    pub fn full_digest(&self) -> u64 {
        let mut f = Fnv::new();
        f.u64(self.sim.digest());
        for (id, x, y, z) in self.stage.positions() {
            f.str(&id).i32(x).i32(y).i32(z);
        }
        f.finish()
    }
}

impl Headless for MaskHeadless {
    fn id(&self) -> &'static str {
        "mask"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        if self.sim.over != Over::None {
            return Vec::new();
        }
        self.since_decision += dt_ms;
        if self.since_decision >= 300.0 {
            self.since_decision = 0.0;
            self.decide();
        }
        let _ = self.ctl.step(dt_ms);
        let input = self.ctl.control_input();
        let events = self.sim.step(dt_ms, &input);
        self.stage.sync(&self.sim, PLAYER, false);
        let _ = self.stage.tick((dt_ms / 1000.0) as f32);
        events.iter().map(|e| e.name()).collect()
    }

    fn outcome(&self) -> Outcome {
        match self.sim.over {
            Over::None => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn elapsed_ms(&self) -> f64 {
        self.sim.elapsed
    }

    fn digest(&self) -> u64 {
        self.full_digest()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::run_headless;

    fn session(night: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("mask", night, Default::default(), Some(seed));
        s
    }

    fn play(night: i32, seed: u32) -> (Vec<String>, MaskHeadless) {
        let mut h = MaskHeadless::new(&session(night, seed));
        let mut log = Vec::new();
        let mut t = 0.0;
        while t < 300_000.0 && h.outcome() == Outcome::Running {
            log.extend(h.step_ms(100.0));
            t += 100.0;
        }
        (log, h)
    }

    #[test]
    fn a_seeded_night_reaches_an_outcome_on_every_night() {
        for night in 1..=6 {
            let (log, h) = play(night, 12345);
            assert_ne!(h.outcome(), Outcome::Running, "night {night}");
            assert!(!log.is_empty());
            if h.outcome() == Outcome::Won {
                assert!(h.elapsed_ms() >= 270_000.0);
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_events_and_digest() {
        let (a, ha) = play(4, 99);
        let (b, hb) = play(4, 99);
        assert_eq!(a, b);
        assert_eq!(ha.digest(), hb.digest());
        let (c, hc) = play(4, 100);
        assert!(a != c || ha.digest() != hc.digest());
    }

    #[test]
    fn the_check_runner_accepts_the_headless_sim() {
        let factory = |s: &Session| -> Option<Box<dyn Headless>> { Some(Box::new(MaskHeadless::new(s))) };
        for level in 1..=6 {
            let a = run_headless(&factory, "mask", level).expect("first run");
            let b = run_headless(&factory, "mask", level).expect("second run");
            assert_eq!(a.digest, b.digest, "level {level}");
            assert_eq!(a.events, b.events);
            assert_ne!(a.outcome, Outcome::Running, "level {level}");
        }
    }

    #[test]
    fn hunters_walk_during_the_run_instead_of_jumping() {
        let mut h = MaskHeadless::new(&session(3, 7));
        // find a step where Thessine changes room and check she covers ground over several steps
        let mut last = h.stage.pos("thessine").expect("pos");
        let mut biggest_jump: f32 = 0.0;
        let mut moved_total: f32 = 0.0;
        let mut t = 0.0;
        while t < 120_000.0 && h.outcome() == Outcome::Running {
            h.step_ms(100.0);
            let p = h.stage.pos("thessine").expect("pos");
            let d = p.distance(last);
            biggest_jump = biggest_jump.max(d);
            moved_total += d;
            last = p;
            t += 100.0;
        }
        assert!(moved_total > 5.0, "Thessine walks the wing: {moved_total}");
        // 100 ms at the fastest pace (6 m/s) is 0.6 m, plus the task shuffles; a teleport would be metres
        assert!(biggest_jump <= 0.65, "no teleporting: {biggest_jump}");
    }

    #[test]
    fn a_custom_night_uses_the_sliders() {
        let mut s = Session::new();
        let mut lv = std::collections::BTreeMap::new();
        for id in ["verrine", "thessine", "glyra", "nyxie", "umbriel", "haelith", "ilyra"] {
            lv.insert(id.to_string(), 0);
        }
        lv.insert("umbriel".to_string(), 20);
        s.start_mode("mask", 1, lv, Some(5));
        let h = MaskHeadless::new(&s);
        assert!(h.sim.custom);
        assert_eq!(h.sim.base_level("umbriel"), 20);
        assert_eq!(h.sim.base_level("thessine"), 0);
        assert_eq!(h.night, 5);
    }
}
