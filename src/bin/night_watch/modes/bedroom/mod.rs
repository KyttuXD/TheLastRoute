//! Mode M4, Bedtime (`bedroom`): the child's bedroom and the house wing around it, Loxelle in the closet, Javelle in the hall,
//! Haelith creeping across the floor. Ports `game/night_watch/bedroom/*.gd`.
//!
//! * `map`   -- the floor plan and the numbers of the bedroom (pure, from `data/bedroom/map.json`).
//! * `sim`   -- the rules (`BedroomLogic`), the living house (`Sim`: rules plus a `Crowd` walking `data/nav/bedroom.json`),
//!              the sounds (`CueClock`) and the scripted player used by `--check` (`BedroomHeadless`). Pure.
//! * `world` -- the 3D house from cubes and bbmodel props, the lights.
//! * `game`  -- the `Mode`: hands, look, HUD, end cards, admin hooks.
//! * `admin` -- the `ModeAdmin` panel.
#![allow(dead_code)]

pub mod admin;
pub mod game;
pub mod map;
pub mod sim;
pub mod world;

pub use admin::BedroomAdmin;

use crate::core::{Headless, Mode, Session};

pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    Box::new(game::BedroomMode::new(session))
}

pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    Box::new(sim::BedroomHeadless::new(session))
}

/// Problems in Bedtime's data files (empty = fine), for `--check`: the rules, the map, the nav graph, the task spots and the
/// prop models.
pub fn check_data() -> Vec<String> {
    let mut out = Vec::new();
    if let Err(e) = sim::Rules::parse(sim::RULES_JSON) {
        out.push(e);
    }
    let map = match map::BedroomMap::parse(map::MAP_JSON) {
        Ok(m) => Some(m),
        Err(e) => {
            out.push(e);
            None
        }
    };
    match crate::hunters::NavMap::from_json(sim::NAV_JSON) {
        Ok(nav) => {
            for p in nav.validate() {
                out.push(format!("bedroom nav: {p}"));
            }
            let behaviour = crate::hunters::BehaviourData::embedded();
            for n in nav.nodes() {
                if let Some(t) = &n.task {
                    if !behaviour.tasks.contains_key(t) {
                        out.push(format!("bedroom nav: {} has unknown task {t}", n.key));
                    }
                }
            }
            if let Some(m) = &map {
                for r in &m.rooms {
                    match nav.hub_key(&r.id) {
                        None => out.push(format!("bedroom nav: room {} has no waypoint", r.id)),
                        Some(hub) => {
                            if nav.distance("C:living_room", &hub).is_none() {
                                out.push(format!("bedroom nav: room {} is not reachable", r.id));
                            }
                        }
                    }
                }
            }
        }
        Err(e) => out.push(e),
    }
    if let Some(m) = &map {
        for p in &m.props {
            if world::prop_text(&p.model).is_none() {
                out.push(format!("bedroom map: prop {} uses unknown model {}", p.id, p.model));
            }
            if m.room(&p.room).is_none() {
                out.push(format!("bedroom map: prop {} is in unknown room {}", p.id, p.room));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn the_data_files_are_clean() {
        let problems = check_data();
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn the_constructors_make_a_mode_and_a_headless_sim() {
        let mut s = Session::new();
        s.start_mode("bedroom", 2, BTreeMap::new(), Some(12345));
        assert_eq!(new_mode(&s).id(), "bedroom");
        let mut h = new_headless(&s);
        assert_eq!(h.id(), "bedroom");
        for _ in 0..50 {
            h.step_ms(100.0);
        }
        assert_eq!(h.elapsed_ms(), 5000.0);
    }

    #[test]
    fn check_runs_every_night_to_an_outcome_twice_with_one_digest() {
        for night in 1..=6 {
            let mut s = Session::new();
            s.start_mode("bedroom", night, BTreeMap::new(), Some(12345));
            let run = |s: &Session| {
                let mut h = new_headless(s);
                let mut t = 0.0;
                while h.outcome() == crate::core::Outcome::Running && t < 600_000.0 {
                    h.step_ms(100.0);
                    t += 100.0;
                }
                (h.outcome(), h.digest())
            };
            let (o1, d1) = run(&s);
            let (o2, d2) = run(&s);
            assert_ne!(o1, crate::core::Outcome::Running, "night {night}");
            assert_eq!((o1, d1), (o2, d2), "night {night}");
        }
    }
}
