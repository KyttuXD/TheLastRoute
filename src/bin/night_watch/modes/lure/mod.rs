//! M3: Dead Air (mode id `lure`). A radio station at night: one real hunter crawls toward the booth's vent openings, three
//! phantoms break your systems, and the building has a temperature. Port of `game/night_watch/lure/*.gd`.
//!
//! * `data`     the tables (data/lure/tuning.json), rooms, graph, systems
//! * `sim`      the rules (lure_logic.gd), pure
//! * `map`      the station's floor plan (data/lure/map.json) and the booth's constants
//! * `presence` the walking hunters on the nav graph (data/nav/lure.json), phantoms, idle behaviour; pure
//! * `world`    the 3D booth and station, props from bbmodels
//! * `game`     the Mode (input, monitor panel, feeds, HUD, end cards)
//! * `admin`    the dev panel
//! * `headless` the window-less run for `--check` and the tests
#![allow(dead_code)]

pub mod admin;
pub mod data;
pub mod game;
pub mod headless;
pub mod map;
pub mod presence;
pub mod sim;
pub mod world;

pub use admin::LureAdmin;

use crate::core::{Headless, Mode, Session};

/// The render-facing mode for a session.
pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    Box::new(game::new_mode(session))
}

/// The window-less sim for a session (the autoplay policy is inside).
pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    Box::new(headless::new_headless(session))
}

/// Problems in Dead Air's embedded data (empty = fine), for `--check`: the tuning, the map, the nav graph and the behaviour
/// tasks the map names.
pub fn check_data() -> Vec<String> {
    let mut p: Vec<String> = Vec::new();
    if let Err(e) = data::Tuning::parse(data::TUNING_JSON) {
        p.push(e);
    }
    match map::LureMap::parse(map::MAP_JSON) {
        Ok(m) => {
            for t in &m.tasks {
                if m.by_sim(&t.room).is_none() {
                    p.push(format!("lure map: task {} in unknown room {}", t.task, t.room));
                }
            }
            for prop in &m.props {
                if !world::PROP_FILES.iter().any(|(n, _)| *n == prop.model) {
                    p.push(format!("lure map: prop {} is not embedded", prop.model));
                }
            }
        }
        Err(e) => p.push(e),
    }
    let nav = presence::nav_map();
    if nav.is_empty() {
        p.push("lure nav: the graph is empty or does not parse".to_string());
    }
    for s in nav.validate() {
        p.push(format!("lure nav: {s}"));
    }
    let ids = crate::hunters::poses::pose_ids(crate::assets::POSES_JSON);
    for s in crate::hunters::behaviour::BehaviourData::embedded().validate(&ids) {
        p.push(format!("behaviour: {s}"));
    }
    for sim in data::ROOMS {
        if nav.stand_node(sim, 0).is_none() || nav.cameras(sim).is_empty() {
            p.push(format!("lure nav: {sim} has no stand spot or camera"));
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_data_checks_clean() {
        let p = check_data();
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn the_constructors_make_the_mode_and_the_headless_run() {
        let mut s = Session::new();
        s.start_mode("lure", 2, Default::default(), Some(5));
        let m = new_mode(&s);
        assert_eq!(m.id(), "lure");
        let mut h = new_headless(&s);
        assert_eq!(h.id(), "lure");
        h.step_ms(100.0);
        assert_eq!(h.elapsed_ms(), 100.0);
    }
}
