//! The window-less half of Dead Air: the rules plus the walking hunters, driven by a seeded autoplay policy. Used by `--check`
//! and the tests. The policy plays like a careful but imperfect operator: it watches a changing camera, seals the vent Xivvy is
//! at, reboots whatever is down, runs the heater or the fan to keep the temperature livable, and lures now and then.
#![allow(dead_code)]

use super::data::{is_office, ROOMS, SYSTEM_IDS};
use super::presence::{booth_focus, Presence};
use super::sim::{Over, Sim, SimOpts};
use crate::core::{Fnv, Headless, Mulberry32, Outcome, Session};

pub struct LureHeadless {
    pub sim: Sim,
    pub presence: Presence,
    policy: Mulberry32,
    watch: String,
    watch_left_ms: f64,
    lure_wait_ms: f64,
    pub log: Vec<String>,
}

pub fn new_headless(session: &Session) -> LureHeadless {
    let mut opts = SimOpts::from_session(session);
    opts.seed = session.seed_or_fresh();
    LureHeadless {
        sim: Sim::new(opts.clone()),
        presence: Presence::new(opts.seed, false),
        policy: Mulberry32::new(opts.seed ^ 0x5a17_0b0b),
        watch: String::new(),
        watch_left_ms: 0.0,
        lure_wait_ms: 8000.0,
        log: Vec::new(),
    }
}

impl LureHeadless {
    fn act(&mut self, dt_ms: f64, out: &mut Vec<String>) {
        // which camera the operator looks at (a new one every 1.5 to 4 s, sometimes none)
        self.watch_left_ms -= dt_ms;
        if self.watch_left_ms <= 0.0 {
            self.watch_left_ms = 1500.0 + self.policy.next() * 2500.0;
            self.watch = if self.policy.next() < 0.3 { String::new() } else { ROOMS[self.policy.pick(ROOMS.len())].to_string() };
        }
        // seal the vent she is at, open it again when she has gone
        let room = self.sim.killer.room.clone();
        for (side, name) in [('L', "officeL"), ('R', "officeR")] {
            let i = if side == 'L' { 0 } else { 1 };
            let want = room == name;
            if want != self.sim.seals[i] && (want || !is_office(&room)) {
                self.sim.seal(side, Some(want));
                out.push(format!("seal:{side}:{}", if want { "on" } else { "off" }));
            }
        }
        // reboot whatever is down
        for s in SYSTEM_IDS {
            if self.sim.reboot(s) {
                out.push(format!("reboot:{s}"));
            }
        }
        // keep the temperature livable
        if self.sim.has_climate {
            let t = self.sim.temp;
            if t < 66.0 && !self.sim.heater_on && self.sim.toggle("heater") {
                out.push("heater:on".to_string());
            } else if t > 92.0 && self.sim.has_fan && !self.sim.fan_on && self.sim.toggle("fan") {
                out.push("fan:on".to_string());
            } else if t > 74.0 && t < 90.0 {
                if self.sim.heater_on && self.sim.toggle("heater") {
                    out.push("heater:off".to_string());
                }
                if self.sim.fan_on && self.sim.toggle("fan") {
                    out.push("fan:off".to_string());
                }
            }
        }
        // lure now and then
        self.lure_wait_ms -= dt_ms;
        if self.lure_wait_ms <= 0.0 {
            self.lure_wait_ms = 9000.0 + self.policy.next() * 9000.0;
            let room = ROOMS[self.policy.pick(5)];
            if self.policy.next() < 0.5 && self.sim.lure(room) {
                out.push(format!("lure:{room}"));
            }
        }
    }
}

impl Headless for LureHeadless {
    fn id(&self) -> &'static str {
        "lure"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if self.sim.over != Over::None {
            return out;
        }
        self.act(dt_ms, &mut out);
        let watch = if self.sim.online("cams") { self.watch.clone() } else { String::new() };
        for e in self.sim.step(dt_ms, &watch) {
            out.push(e.name());
        }
        self.presence.tick(&self.sim, (dt_ms / 1000.0) as f32, booth_focus());
        self.log.extend(out.iter().cloned());
        out
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
        let mut h = Fnv::new();
        h.u64(self.sim.digest());
        for (id, x, y, z) in self.presence.snapshot() {
            h.str(&id).i32(x).i32(y).i32(z);
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn session(night: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("lure", night, BTreeMap::new(), Some(seed));
        s
    }

    fn play(night: i32, seed: u32) -> (Outcome, f64, u64, Vec<String>) {
        let mut h = new_headless(&session(night, seed));
        for _ in 0..2700 {
            h.step_ms(100.0);
            if h.outcome() != Outcome::Running {
                break;
            }
        }
        (h.outcome(), h.elapsed_ms(), h.digest(), h.log.clone())
    }

    #[test]
    fn a_seeded_night_reaches_an_outcome() {
        for night in 1..=6 {
            let (o, ms, _, _) = play(night, 12345);
            assert_ne!(o, Outcome::Running, "night {night} ended at {ms}");
            if o == Outcome::Won {
                assert_eq!(ms, 270_000.0);
            } else {
                assert!(ms <= 270_000.0);
            }
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_run_and_another_seed_a_different_one() {
        for night in [2, 5, 6] {
            let a = play(night, 12345);
            let b = play(night, 12345);
            assert_eq!(a.0, b.0);
            assert_eq!(a.1, b.1);
            assert_eq!(a.2, b.2, "digest night {night}");
            assert_eq!(a.3, b.3, "event list night {night}");
        }
        assert_ne!(play(5, 12345).2, play(5, 54321).2);
    }

    #[test]
    fn the_operator_survives_the_tutorial_night() {
        let (o, ms, _, log) = play(1, 777);
        assert_eq!(o, Outcome::Won, "night 1 ended at {ms}");
        assert!(log.iter().any(|e| e.starts_with("phantom:ferryn")));
        assert!(log.iter().any(|e| e.starts_with("reboot:")), "the operator reboots what Ferryn breaks");
    }

    #[test]
    fn the_hunters_really_walk_during_the_run() {
        let mut h = new_headless(&session(6, 99));
        let mut moved_visible = false;
        for _ in 0..1500 {
            h.step_ms(100.0);
            if let Some(a) = h.presence.xivvy() {
                let seen = h.presence.crowd.visible_in("lobby").len() + h.presence.crowd.visible_in("arcade").len() + h.presence.crowd.visible_in("party").len();
                if a.walker.moving && seen > 0 {
                    moved_visible = true;
                }
            }
            if h.outcome() != Outcome::Running {
                break;
            }
        }
        assert!(moved_visible || h.outcome() != Outcome::Running);
    }

    #[test]
    fn a_decided_run_stays_decided() {
        let mut h = new_headless(&session(2, 5));
        h.sim.over = Over::Win;
        assert!(h.step_ms(100.0).is_empty());
        assert_eq!(h.outcome(), Outcome::Won);
        assert_eq!(h.id(), "lure");
    }
}
