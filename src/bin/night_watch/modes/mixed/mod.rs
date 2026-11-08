//! Mode 9, MIXED SHIFT (and its fifth shift, ENDLESS): a drawn roster of threats of different types run by a pacing
//! director, in a transit hub that is dressed as one of nine maps each night.
//!
//! * `data`  -- the tuning (`data/mixed/tuning.json`): pool, shifts, timings, routes, chaos events. Pure.
//! * `skin`  -- the nine skins and their rotation. Pure.
//! * `sim`   -- the rules (`MixedSim`), the seeded autoplayer and the pure headless run. Pure.
//! * `world` -- the hub, the vent run, the cameras, the props, the office's three vents (`data/mixed/map.json`).
//! * `game`  -- the `Mode`: input, hud, audio, the walking hunters (`data/nav/mixed.json`), the admin panel.
#![allow(dead_code)]

pub mod data;
pub mod game;
pub mod sim;
pub mod skin;
pub mod world;

pub use game::{MixedAdmin, MixedGame, WalkingHeadless};
pub use sim::{MixedHeadless, MixedSim};

use crate::core::{Headless, Mode, Session};

/// The render-facing mode for a session (level 1-4 are QUIET, STEADY, RESTLESS and CHAOS, 5 is ENDLESS).
pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    Box::new(MixedGame::new(session))
}

/// The window-less sim for `--check` and tests: the rules fed by a seeded autoplayer, the hunters walking the nav graph.
pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    Box::new(WalkingHeadless::new(session.seed_value, session.level().clamp(1, 5)))
}

/// Problems in Mixed Shift's embedded data (for `--check`): the tuning, the map, the nav graph and the skins.
pub fn check_data() -> Vec<String> {
    let mut out = Vec::new();
    match data::Tuning::parse(data::TUNING_JSON) {
        Ok(t) => out.extend(data::route_problems(&t)),
        Err(e) => out.push(e),
    }
    match world::MapData::parse(world::MAP_JSON) {
        Ok(m) => {
            if m.areas.len() != 8 {
                out.push(format!("mixed map: expected 8 areas, found {}", m.areas.len()));
            }
            for a in &m.areas {
                if a.spots.len() < 2 {
                    out.push(format!("mixed map: area {} has fewer than two dressing spots", a.id));
                }
            }
        }
        Err(e) => out.push(e),
    }
    match crate::hunters::nav::NavMap::from_json(world::NAV_JSON) {
        Ok(n) => out.extend(n.validate().into_iter().map(|s| format!("mixed nav: {s}"))),
        Err(e) => out.push(e),
    }
    for s in skin::SKINS.iter() {
        if s.labels.iter().any(|l| l.is_empty()) {
            out.push(format!("mixed skin {} has an empty label", s.id));
        }
    }
    for k in 0..8 {
        let room = &data::tun().cam_rooms[k];
        if world::map().area(room).is_none() {
            out.push(format!("mixed map: camera room {room} has no area"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn the_embedded_data_checks_clean() {
        assert_eq!(check_data(), Vec::<String>::new());
    }

    #[test]
    fn the_constructors_make_a_mode_and_a_headless_run() {
        let mut s = Session::new();
        s.start_mode("mixed", 2, BTreeMap::new(), Some(12345));
        assert_eq!(new_mode(&s).id(), "mixed");
        let mut h = new_headless(&s);
        assert_eq!(h.id(), "mixed");
        h.step_ms(100.0);
        assert_eq!(h.elapsed_ms(), 100.0);
    }
}
