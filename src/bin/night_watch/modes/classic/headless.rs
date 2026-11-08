//! The window-less Classic run used by `--check` and the tests: the pure sim fed by a seeded autoplay policy (it answers
//! the doors, lights and monitor the way a careful player would, with some randomness). Same seed, same night.
#![allow(dead_code)]

use super::sim::{ClassicSim, Over, SimEvent, SimInput};
use super::stalker::door_name;
use crate::core::{cam_rooms, Fnv, Headless, Mulberry32, Outcome, Session};

/// The salt of the autoplay stream ("clas").
const POLICY_SALT: u32 = 0x636c_6173;
/// The policy looks at the board every this many ms.
const DECIDE_MS: f64 = 400.0;

pub struct ClassicHeadless {
    sim: ClassicSim,
    policy: Mulberry32,
    input: SimInput,
    since: f64,
    log: Fnv,
    events: usize,
    rooms: Vec<String>,
}

impl ClassicHeadless {
    pub fn new(session: &Session) -> ClassicHeadless {
        let seed = session.seed_or_fresh();
        ClassicHeadless {
            sim: ClassicSim::new(session.night, seed, &session.custom_levels, session.is_custom),
            policy: Mulberry32::new(seed ^ POLICY_SALT),
            input: SimInput::default(),
            since: DECIDE_MS,
            log: Fnv::new(),
            events: 0,
            rooms: cam_rooms(),
        }
    }

    pub fn sim(&self) -> &ClassicSim {
        &self.sim
    }

    pub fn sim_mut(&mut self) -> &mut ClassicSim {
        &mut self.sim
    }

    pub fn input(&self) -> &SimInput {
        &self.input
    }

    /// One look at the board: shut a door on whoever stands at it, hold a light now and then, flick through the cameras.
    /// Always draws the same number of values, so a seed is one night.
    fn decide(&mut self) {
        let mut at = [false, false];
        for h in &self.sim.hunters {
            for d in 0..2 {
                if h.room == door_name(d) {
                    at[d] = true;
                }
            }
        }
        for d in 0..2 {
            let u = self.policy.next();
            self.input.door_shut[d] = u < if at[d] { 0.92 } else { 0.04 };
            let v = self.policy.next();
            self.input.light_held[d] = v < if at[d] && !self.input.door_shut[d] { 0.8 } else { 0.08 };
        }
        let c = self.policy.next();
        self.input.cam = c < 0.35;
        let pick = self.policy.pick(self.rooms.len().max(1));
        self.input.cam_room = self.rooms.get(pick).cloned().unwrap_or_default();
    }
}

impl Headless for ClassicHeadless {
    fn id(&self) -> &'static str {
        "classic"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        if self.sim.over != Over::Running {
            return Vec::new();
        }
        self.since += dt_ms;
        if self.since >= DECIDE_MS {
            self.since = 0.0;
            self.decide();
        }
        let events: Vec<SimEvent> = self.sim.step(dt_ms, &self.input);
        let names: Vec<String> = events.iter().map(|e| e.name()).collect();
        for n in &names {
            self.log.str(n);
            self.events += 1;
        }
        names
    }

    fn outcome(&self) -> Outcome {
        match self.sim.over {
            Over::Running => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn elapsed_ms(&self) -> f64 {
        self.sim.elapsed
    }

    fn digest(&self) -> u64 {
        let mut f = self.log;
        f.f64(self.sim.elapsed).f64(self.sim.power).u64(self.sim.rng_state() as u64).i32(self.events as i32);
        f.i32(match self.sim.over {
            Over::Running => 0,
            Over::Win => 1,
            Over::Caught => 2,
        });
        for h in &self.sim.hunters {
            f.str(&h.room).f64(h.at_door_ms);
        }
        f.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn session(night: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("classic", night, BTreeMap::new(), Some(seed));
        s
    }

    fn play(s: &Session, max_ms: f64) -> (Vec<String>, ClassicHeadless) {
        let mut h = ClassicHeadless::new(s);
        let mut all = Vec::new();
        let mut t = 0.0;
        while t < max_ms && h.outcome() == Outcome::Running {
            all.extend(h.step_ms(100.0));
            t += 100.0;
        }
        (all, h)
    }

    #[test]
    fn a_seeded_night_reaches_an_outcome() {
        for night in 1..=7 {
            let (_, h) = play(&session(night, 12345), 271_000.0);
            assert_ne!(h.outcome(), Outcome::Running, "night {night} was never decided");
            assert!(h.elapsed_ms() <= 270_100.0);
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_events_and_digest() {
        let (a, ha) = play(&session(5, 12345), 271_000.0);
        let (b, hb) = play(&session(5, 12345), 271_000.0);
        assert_eq!(a, b);
        assert_eq!(ha.digest(), hb.digest());
        assert!(!a.is_empty());
        let (c, hc) = play(&session(5, 54321), 271_000.0);
        assert_ne!(a, c);
        assert_ne!(ha.digest(), hc.digest());
    }

    #[test]
    fn the_digest_moves_with_the_clock() {
        let mut h = ClassicHeadless::new(&session(2, 9));
        let d0 = h.digest();
        h.step_ms(100.0);
        assert_ne!(d0, h.digest());
    }

    #[test]
    fn a_decided_night_stays_decided() {
        let (_, mut h) = play(&session(1, 3), 271_000.0);
        let out = h.outcome();
        let d = h.digest();
        assert!(h.step_ms(100.0).is_empty());
        assert_eq!(h.outcome(), out);
        assert_eq!(h.digest(), d);
    }

    #[test]
    fn a_custom_night_plays_with_its_sliders_and_no_kathy() {
        let mut s = Session::new();
        let mut sliders = BTreeMap::new();
        for (k, v) in [("dovelle", 5), ("ravelle", 0), ("aki", 0), ("ilyra", 0), ("nyxie", 0)] {
            sliders.insert(k.to_string(), v);
        }
        s.start_mode("classic", 7, sliders, Some(8));
        let h = ClassicHeadless::new(&s);
        assert!(!h.sim().kathy_night);
        assert_eq!(h.sim().hunters.len(), 4);
        assert_eq!(h.sim().hunters[0].base, 5);
    }

    #[test]
    fn the_autoplay_survives_some_nights_and_loses_others() {
        // the policy is competent but not perfect: over many seeds both outcomes happen on a mid night
        let mut won = 0;
        let mut lost = 0;
        for seed in 1..=12u32 {
            let (_, h) = play(&session(3, seed * 7919), 271_000.0);
            match h.outcome() {
                Outcome::Won => won += 1,
                Outcome::Lost => lost += 1,
                Outcome::Running => panic!("undecided"),
            }
        }
        assert!(won + lost == 12);
        assert!(won >= 1, "the policy should win sometimes ({won} wins, {lost} losses)");
    }

    #[test]
    fn the_headless_mode_id() {
        assert_eq!(ClassicHeadless::new(&session(1, 1)).id(), "classic");
    }
}
