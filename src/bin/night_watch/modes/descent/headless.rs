//! The window-less Descent: the sim, Elise walking the shaft's nav graph, and a seeded scripted player. Used by `--check`
//! and the tests. The player presses the right control after a human delay, now and then the wrong one, holds the door
//! through the last stage, and never reads anything a human could not see.
#![allow(dead_code)]

use super::elise::EliseCtl;
use super::sim::{from_session, Act, Over, Phase, Puzzle, Sim, SimEvent};
use crate::core::{Fnv, Headless, Mulberry32, Outcome, Session};
use crate::hunters::behaviour::BehaviourEvent;

/// Chance that a press is the wrong one.
pub const MISTAKE: f64 = 0.10;

pub struct DescentHeadless {
    pub sim: Sim,
    pub elise: EliseCtl,
    ai: Mulberry32,
    cool: f64,
}

/// The short event name of an idle behaviour event.
pub fn behaviour_name(e: &BehaviourEvent) -> &'static str {
    match e {
        BehaviourEvent::FirstStare { .. } => "first_stare",
        BehaviourEvent::StareStart { .. } => "stare",
        BehaviourEvent::StareEnd { .. } => "stare_end",
        BehaviourEvent::TaskStart { .. } => "task",
        BehaviourEvent::TaskEnd { .. } => "task_end",
    }
}

impl DescentHeadless {
    pub fn new(session: &Session) -> DescentHeadless {
        let seed = session.seed_or_fresh();
        DescentHeadless::with_sim(from_session(session, seed), seed)
    }

    pub fn with_sim(sim: Sim, seed: u32) -> DescentHeadless {
        DescentHeadless { sim, elise: EliseCtl::new(seed), ai: Mulberry32::new(seed ^ 0x4445_5343), cool: 0.0 }
    }

    /// One press of the scripted player; sets the delay before the next one.
    fn play(&mut self) -> Vec<SimEvent> {
        let wrong = self.ai.next() < MISTAKE;
        let (act, cool): (Option<Act>, f64) = match &self.sim.puzzle {
            Puzzle::Wires(z) => {
                let target = z.lit.get(z.step).and_then(|b| z.wire.get(*b)).copied().unwrap_or(0);
                let b = if wrong && z.buttons > 1 { (target + 1 + self.ai.pick(z.buttons - 1)) % z.buttons } else { target };
                (Some(Act::Wire(b)), self.ai.range(650.0, 1250.0))
            }
            Puzzle::Memory(m) => {
                if m.phase == Phase::Input {
                    let pad = m.seq.get(m.step).copied().unwrap_or(0);
                    let p = if wrong { (pad + 1 + self.ai.pick(3)) % 4 } else { pad };
                    (Some(Act::Pad(p)), self.ai.range(420.0, 760.0))
                } else {
                    (None, 150.0)
                }
            }
            Puzzle::Pipes(p) => {
                let next = p.path.iter().find(|(r, c)| p.tiles[*r][*c].rot != p.tiles[*r][*c].solved).copied();
                (next.map(|(r, c)| Act::Pipe(r, c)), self.ai.range(300.0, 600.0))
            }
            Puzzle::Hold | Puzzle::None => (None, 200.0),
        };
        self.cool = cool;
        match act {
            Some(a) => self.sim.act(a),
            None => Vec::new(),
        }
    }
}

impl Headless for DescentHeadless {
    fn id(&self) -> &'static str {
        "descent"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        if self.sim.over != Over::None {
            return names;
        }
        // she holds the door through the last stage
        let hold = matches!(self.sim.puzzle, Puzzle::Hold);
        for e in self.sim.step(dt_ms, hold) {
            names.push(e.name().to_string());
        }
        self.cool -= dt_ms;
        if self.sim.ride > 0.0 {
            self.cool = self.cool.max(500.0);
        } else if self.cool <= 0.0 && self.sim.over == Over::None {
            for e in self.play() {
                names.push(e.name().to_string());
            }
        }
        let riding = self.sim.is_riding();
        let evs = self.elise.update((dt_ms / 1000.0) as f32, self.sim.side, self.sim.threat, riding, self.sim.level > 0);
        for e in &evs {
            names.push(behaviour_name(e).to_string());
        }
        names
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
        self.sim.digest_into(&mut h);
        h.u64(self.ai.state() as u64).f64(self.cool);
        if let Some((p, yaw)) = self.elise.pose() {
            h.u64(p.x.to_bits() as u64).u64(p.z.to_bits() as u64).u64(yaw.to_bits() as u64);
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn session(level: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("descent", level, BTreeMap::new(), Some(seed));
        s
    }

    fn run(h: &mut DescentHeadless) -> Vec<String> {
        let mut all = Vec::new();
        let mut t = 0.0;
        while t < 600_000.0 && h.outcome() == Outcome::Running {
            all.extend(h.step_ms(100.0));
            t += 100.0;
        }
        all
    }

    #[test]
    fn every_descent_reaches_an_outcome_for_many_seeds() {
        for level in 1..=5 {
            for seed in 1..=12u32 {
                let mut h = DescentHeadless::new(&session(level, seed));
                run(&mut h);
                assert_ne!(h.outcome(), Outcome::Running, "descent {level} seed {seed} at {} ms", h.elapsed_ms());
            }
        }
    }

    #[test]
    fn the_walk_through_is_always_won() {
        for seed in 1..=20u32 {
            let mut h = DescentHeadless::new(&session(1, seed));
            let ev = run(&mut h);
            assert_eq!(h.outcome(), Outcome::Won, "seed {seed}");
            assert!(ev.contains(&"win".to_string()));
            assert!(!ev.contains(&"caught".to_string()));
            assert!(h.elise.agent().is_none(), "level 0: nobody is out there");
        }
    }

    #[test]
    fn a_seeded_run_is_the_same_twice() {
        for level in [2, 4, 5] {
            let mut a = DescentHeadless::new(&session(level, 4242));
            let mut b = DescentHeadless::new(&session(level, 4242));
            let ea = run(&mut a);
            let eb = run(&mut b);
            assert_eq!(ea, eb, "descent {level}");
            assert_eq!(a.digest(), b.digest());
            assert_eq!(a.elapsed_ms(), b.elapsed_ms());
            assert_eq!(a.outcome(), b.outcome());
        }
    }

    #[test]
    fn different_seeds_give_different_runs() {
        let mut digests = Vec::new();
        for seed in 1..=6u32 {
            let mut h = DescentHeadless::new(&session(3, seed));
            run(&mut h);
            digests.push(h.digest());
        }
        digests.sort();
        digests.dedup();
        assert!(digests.len() >= 5, "{digests:?}");
    }

    #[test]
    fn a_hard_custom_night_can_be_lost_and_elise_is_at_the_glass_then() {
        let mut custom = BTreeMap::new();
        custom.insert("elise".to_string(), 20);
        let mut lost = 0;
        for seed in 1..=16u32 {
            let mut s = Session::new();
            s.start_mode("descent", 1, custom.clone(), Some(seed));
            let mut h = DescentHeadless::new(&s);
            assert_eq!(h.sim.descent, 5, "a Custom Night plays the longest descent");
            assert_eq!(h.sim.level, 20);
            run(&mut h);
            if h.outcome() == Outcome::Lost {
                lost += 1;
                let (p, _) = h.elise.pose().expect("she is out there");
                let glass = super::super::map::sill_pos(h.sim.side, 100.0);
                let d = ((p.x - glass.x).powi(2) + (p.z - glass.z).powi(2)).sqrt();
                assert!(d < 1.2, "seed {seed}: {d} m from the glass");
            }
        }
        assert!(lost >= 1, "at level 20 somebody gets caught");
    }

    #[test]
    fn elise_does_idle_things_on_a_long_slow_run() {
        // a calm descent 2 (level 5) takes a while: idle behaviour shows up in the event list
        let mut seen = false;
        for seed in 1..=10u32 {
            let mut h = DescentHeadless::new(&session(2, seed));
            let ev = run(&mut h);
            if ev.iter().any(|e| e == "stare" || e == "task") {
                seen = true;
            }
        }
        assert!(seen, "across ten runs she stared or worked at least once");
    }

    #[test]
    fn the_events_use_the_short_names() {
        let mut h = DescentHeadless::new(&session(2, 9));
        let ev = run(&mut h);
        for e in &ev {
            assert!(
                ["step", "wrong", "stage", "ride", "win", "caught", "first_stare", "stare", "stare_end", "task", "task_end"].contains(&e.as_str()),
                "{e}"
            );
        }
        assert!(ev.iter().filter(|e| *e == "ride").count() >= 1);
    }
}
