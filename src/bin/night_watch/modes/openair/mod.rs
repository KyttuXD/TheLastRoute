//! Mode set M8, OPEN AIR (mode id `extra`, code folder `openair`): three outdoor watches (ROOFTOP, BUS STOP, CAMPFIRE), each a
//! walkable 3D set with security cameras, living hunters that walk the set's nav graph and stare at its cameras, and the rules of
//! `openair_logic.gd` unchanged.
//!
//! * `data`  -- tunables, the three maps and their nav graphs (JSON under `data/openair/` and `data/nav/extra_*.json`),
//! * `sim`   -- the rules (pure), the scripted player of the headless run,
//! * `cast`  -- the hunters: A* walking, facing, stare and task behaviour (pure; reads the sim, never changes it),
//! * `world` -- the sets in 3D, props, cameras, lights, weather,
//! * `game`  -- the `Mode`,
//! * `admin` -- the admin panel (`ModeAdmin`).
//!
//! Exposed to the registry: `new_mode`, `new_headless`. Exposed to the hub for the Custom Night chain: `chain_has_more`,
//! `chain_advance`, `defers_record` (see the hooks in the pass notes).
#![allow(dead_code)]

pub mod admin;
pub mod cast;
pub mod data;
pub mod game;
pub mod sim;
pub mod world;

pub use admin::OpenAirAdmin;
pub use game::OpenAirMode;

use crate::core::{Fnv, Headless, Mode, ModesData, Outcome, Session};
use cast::{behaviour_label, Cast};
use data::{custom_watches, watch_hunter, MODE_ID};
use macroquad::prelude::vec3;
use sim::{Autoplay, OpenAirSim, Over};
use std::collections::BTreeMap;
use std::sync::Mutex;

// ------------------------------------------------------------------------------------------------ the Custom Night chain

/// A Custom Night's place in its chain of watches. A Retry (same seed) replays this watch and NEXT WATCH moves on; a Custom
/// Night started from the board has a new seed and begins at its first watch.
#[derive(Clone, Copy, Debug)]
pub struct ChainState {
    seed: Option<u32>,
    step: usize,
}

static CHAIN: Mutex<ChainState> = Mutex::new(ChainState { seed: None, step: 0 });

/// The watches of a Custom Night, and which one this seed is on.
pub fn plan_chain(custom: &BTreeMap<String, i32>, seed: u32, state: &mut ChainState) -> (Vec<usize>, usize) {
    let mut chain = custom_watches(custom);
    if chain.is_empty() {
        chain.push(0);
    }
    if state.seed != Some(seed) {
        state.seed = Some(seed);
        state.step = 0;
    }
    let step = state.step.min(chain.len() - 1);
    (chain, step)
}

/// The watches this session plays and the index of the current one. A normal shift is its one watch (the level).
pub fn chain_for(session: &Session) -> (Vec<usize>, usize) {
    if !session.is_custom {
        return (vec![(session.level() - 1).clamp(0, 2) as usize], 0);
    }
    let seed = session.seed_or_fresh();
    match CHAIN.lock() {
        Ok(mut st) => plan_chain(&session.custom_levels, seed, &mut st),
        Err(_) => (vec![0], 0),
    }
}

/// Does a Custom Night have another watch after this one?
pub fn chain_has_more(session: &Session) -> bool {
    if !session.is_custom {
        return false;
    }
    let (chain, step) = chain_for(session);
    step + 1 < chain.len()
}

/// NEXT WATCH of a Custom Night: move the chain on (the hub then restarts the shift with the same session and seed). False
/// when there is no further watch.
pub fn chain_advance(session: &Session) -> bool {
    if !chain_has_more(session) {
        return false;
    }
    let seed = session.seed_or_fresh();
    if let Ok(mut st) = CHAIN.lock() {
        let (_, step) = plan_chain(&session.custom_levels, seed, &mut st);
        st.step = step + 1;
        return true;
    }
    false
}

/// True when a win of this session is only one watch of a longer Custom Night, so the hub must not record it yet.
pub fn defers_record(session: &Session) -> bool {
    session.mode_id == MODE_ID && chain_has_more(session)
}

// ------------------------------------------------------------------------------------------------ data check

/// Problems in Open Air's data (empty = fine): the tunables, the three maps, their nav graphs, the behaviour file. For `--check`.
pub fn check() -> Vec<String> {
    let mut out = Vec::new();
    for k in &data::tun().missing {
        out.push(format!("openair tunables: missing {k}"));
    }
    if data::maps().len() != 3 {
        out.push(format!("openair map: {} watches, not 3", data::maps().len()));
        return out;
    }
    for w in 0..3usize {
        let map = data::map_of(w);
        let nav = data::build_nav(w);
        for p in nav.validate() {
            out.push(format!("openair nav {}: {p}", map.id));
        }
        if map.cameras.is_empty() || map.cameras.len() > 8 {
            out.push(format!("openair map {}: {} cameras", map.id, map.cameras.len()));
        }
        for c in &map.cameras {
            for room in &c.rooms {
                if !nav.rooms().iter().any(|r| r.id == *room) {
                    out.push(format!("openair map {}: camera {} watches unknown nav room {room}", map.id, c.id));
                }
            }
        }
        for p in &map.props {
            if !world::prop_known(&p.model) {
                out.push(format!("openair map {}: prop {} is not embedded", map.id, p.model));
            }
        }
        for s in &map.stations {
            if s.prompt.trim().is_empty() {
                out.push(format!("openair map {}: station {} has no prompt", map.id, s.id));
            }
        }
    }
    let poses = crate::hunters::poses::pose_ids(crate::assets::POSES_JSON);
    for p in data::behaviour().validate(&poses) {
        out.push(format!("openair behaviour: {p}"));
    }
    out
}

// ------------------------------------------------------------------------------------------------ constructors

/// The rules of a session's watch: the hunter's slider and, at the campfire, Verrine's and Quenelle's (only in a Custom Night).
pub fn make_sim(session: &Session, watch: usize, step_index: usize) -> OpenAirSim {
    let hunter = watch_hunter(watch);
    let c = |id: &str| session.custom_levels.get(id).copied().unwrap_or(0);
    let (own, verrine, quenelle) = if session.is_custom { (c(hunter), c("verrine"), c("quenelle")) } else { (-1, -1, -1) };
    let seed = session.seed_or_fresh().wrapping_add(step_index as u32);
    OpenAirSim::new(watch, seed, own, verrine, quenelle, false)
}

pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    let (chain, step) = chain_for(session);
    let watch = chain[step.min(chain.len() - 1)];
    let has_more = session.is_custom && step + 1 < chain.len();
    let scored = ModesData::load().get(MODE_ID).map(|m| m.scored).unwrap_or(3);
    Box::new(OpenAirMode::new(session, watch, step, has_more, scored))
}

/// The window-less run: the rules, a seeded scripted player, and the hunters walking their nav graph. Deterministic for a seed.
pub struct OpenAirHeadless {
    sim: OpenAirSim,
    ap: Autoplay,
    cast: Cast,
}

pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    let (chain, step) = chain_for(session);
    let watch = chain[step.min(chain.len() - 1)];
    let sim = make_sim(session, watch, step);
    let seed = session.seed_or_fresh().wrapping_add(step as u32);
    let mut cast = Cast::new(watch, seed, data::map_of(watch).spawn + vec3(0.0, 1.7, 0.0));
    cast.place_all(&sim);
    Box::new(OpenAirHeadless { sim, ap: Autoplay::new(seed), cast })
}

impl Headless for OpenAirHeadless {
    fn id(&self) -> &'static str {
        MODE_ID
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        let mut out = Vec::new();
        if self.sim.over != Over::Running {
            return out;
        }
        let (input, acts) = self.ap.plan(&self.sim, dt_ms);
        Autoplay::apply(&mut self.sim, &acts);
        for e in self.sim.step(dt_ms, &input) {
            out.push(e.label());
        }
        self.cast.sync(&self.sim, false);
        for e in self.cast.tick((dt_ms / 1000.0) as f32) {
            out.push(behaviour_label(&e));
        }
        out
    }

    fn outcome(&self) -> Outcome {
        self.sim.outcome()
    }

    fn elapsed_ms(&self) -> f64 {
        self.sim.elapsed
    }

    fn digest(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.sim.digest());
        self.cast.hash_into(&mut h);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::run_headless;

    fn session(level: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode(MODE_ID, level, BTreeMap::new(), Some(seed));
        s
    }

    fn custom(levels: &[(&str, i32)], seed: u32) -> Session {
        let mut s = Session::new();
        let map: BTreeMap<String, i32> = levels.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        s.start_mode(MODE_ID, 1, map, Some(seed));
        s
    }

    #[test]
    fn the_data_check_finds_nothing() {
        let problems = check();
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn the_level_is_the_watch() {
        for (level, watch) in [(1, 0usize), (2, 1), (3, 2)] {
            let (chain, step) = chain_for(&session(level, 9));
            assert_eq!((chain, step), (vec![watch], 0));
        }
        assert_eq!(chain_for(&session(9, 9)).0, vec![2], "past the last watch it stays on the campfire");
        assert!(!chain_has_more(&session(1, 9)));
        assert!(!defers_record(&session(1, 9)));
    }

    #[test]
    fn a_custom_night_chains_the_watches_whose_hunters_are_on() {
        let mut st = ChainState { seed: None, step: 0 };
        let mut ai = BTreeMap::new();
        ai.insert("verrine".to_string(), 5);
        ai.insert("quenelle".to_string(), 3);
        let (chain, step) = plan_chain(&ai, 77, &mut st);
        assert_eq!((chain.clone(), step), (vec![0, 2], 0));
        st.step = 1;
        assert_eq!(plan_chain(&ai, 77, &mut st), (vec![0, 2], 1), "a retry or the next watch keeps the seed");
        assert_eq!(plan_chain(&ai, 78, &mut st), (vec![0, 2], 0), "a new Custom Night starts over");
        assert_eq!(plan_chain(&BTreeMap::new(), 78, &mut st), (vec![0], 0), "nobody on still plays the rooftop");
    }

    #[test]
    fn next_watch_moves_the_global_chain_on() {
        let s = custom(&[("verrine", 6), ("wynelle", 6)], 424_242);
        assert!(chain_has_more(&s));
        assert!(defers_record(&s));
        assert!(chain_advance(&s));
        let (chain, step) = chain_for(&s);
        assert_eq!((chain, step), (vec![0, 1], 1));
        assert!(!chain_has_more(&s));
        assert!(!chain_advance(&s), "the last watch has no next");
        let m = new_mode(&s);
        assert_eq!(m.id(), "extra");
    }

    #[test]
    fn custom_sliders_set_the_levels_of_the_watch() {
        let s = custom(&[("verrine", 15), ("quenelle", 4), ("ysolde", 0)], 5);
        let sim = make_sim(&s, 0, 0);
        assert_eq!(sim.level(), 15, "the rooftop's Verrine takes her slider");
        let camp = make_sim(&s, 2, 1);
        assert_eq!(camp.level(), 0, "Ysolde is off");
        assert_eq!(camp.level_of("verrine"), 15);
        assert_eq!(camp.level_of("quenelle"), 4);
        let plain = make_sim(&session(2, 5), 1, 0);
        assert_eq!(plain.level(), 8);
    }

    #[test]
    fn the_mode_is_made_without_a_window() {
        let mut m = new_mode(&session(3, 21));
        assert_eq!(m.id(), "extra");
        assert_eq!(m.outcome(), Outcome::Running);
        assert!(m.killer().is_none());
        assert!(!m.wants_mouse_free());
        m.debug_lose();
        assert_eq!(m.outcome(), Outcome::Lost);
        assert!(m.killer().is_some());
        let mut w = new_mode(&session(1, 21));
        w.debug_win();
        assert_eq!(w.outcome(), Outcome::Won);
    }

    #[test]
    fn every_watch_reaches_an_outcome_and_replays_the_same() {
        let modes = ModesData::load();
        let levels = modes.get(MODE_ID).map(|m| m.levels.len()).unwrap_or(0) as i32;
        assert_eq!(levels, 3);
        let factory = |s: &Session| -> Option<Box<dyn Headless>> { Some(new_headless(s)) };
        for level in 1..=levels {
            let a = run_headless(&factory, MODE_ID, level).expect("run");
            let b = run_headless(&factory, MODE_ID, level).expect("run");
            assert_ne!(a.outcome, Outcome::Running, "level {level} ends within the limit");
            assert_eq!((a.digest, a.event_hash, a.events, a.outcome), (b.digest, b.event_hash, b.events, b.outcome));
        }
    }

    #[test]
    fn a_headless_watch_is_won_or_lost_by_270_seconds_and_its_hunters_walk() {
        let mut h = new_headless(&session(3, 12345));
        let mut names: Vec<String> = Vec::new();
        let mut t = 0.0;
        while t < 300_000.0 && h.outcome() == Outcome::Running {
            names.extend(h.step_ms(100.0));
            t += 100.0;
        }
        assert_ne!(h.outcome(), Outcome::Running);
        assert!(h.elapsed_ms() <= 270_000.0 + 1.0);
        assert!(names.iter().any(|n| n.starts_with("close:")), "somebody stepped closer");
        assert!(names.iter().any(|n| n.starts_with("b:")), "somebody stared or worked");
        assert!(h.step_ms(100.0).is_empty(), "nothing happens once it is decided");
    }

    #[test]
    fn different_seeds_play_different_watches() {
        let run = |seed: u32| {
            let mut h = new_headless(&session(1, seed));
            let mut all: Vec<String> = Vec::new();
            for _ in 0..2_700 {
                all.extend(h.step_ms(100.0));
                if h.outcome() != Outcome::Running {
                    break;
                }
            }
            all
        };
        assert_ne!(run(1), run(2));
        assert_eq!(run(7), run(7));
    }
}
