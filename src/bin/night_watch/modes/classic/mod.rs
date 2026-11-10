//! Mode 1: CLASSIC (set M1). Two doors, two door lights, eight cameras and one power meter; Night 7 is Kathy alone.
//!
//! * `stalker`   one hunter's move, wait and attack rules (port of `night_watch_stalker.gd`), pure.
//! * `sim`       the night: power, Nyxie's blackout, Kathy's Night 7, the combos and the lines at the doors (port of
//!               `night_watch_classic.gd`), pure.
//! * `placement` where the hunters physically are: the sim's moves become walks on the nav graph (A*), idle stare / task
//!               behaviour runs between moves, pure.
//! * `view`      presentation numbers and texts (data/classic/presentation.json), end cards, the hunter pose choice, pure.
//! * `world`     bbmodel props placed in the building rooms (data/classic/props.json).
//! * `game`      the `Mode`: office, building, cameras, hunters, input, HUD data, admin rows.
//! * `headless`  the window-less autoplay run for `--check`.
//!
//! The map itself (the eight camera rooms at the Godot grid, the walled security room with two door slabs, vents, lights,
//! lockers, counters, tables) is `gfx::building` and `gfx::office`; the navigation graph is `data/nav/classic.json`.
#![allow(dead_code)]

pub mod game;
pub mod headless;
pub mod placement;
pub mod sim;
pub mod stalker;
pub mod view;
pub mod world;

use crate::core::{Headless, Mode, Session};

/// The render-facing mode of a Classic shift.
pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    Box::new(game::ClassicMode::new(session))
}

/// The window-less sim of a Classic shift (autoplay, seeded). Used by `--check` and the tests.
pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    Box::new(headless::ClassicHeadless::new(session))
}

/// Problems in Classic's own data files (for `--check`): the presentation file, the props file, the nav graph and the
/// behaviour file against the pose list.
pub fn check_data() -> Vec<String> {
    let mut p = Vec::new();
    if let Err(e) = view::View::parse(view::VIEW_JSON) {
        p.push(e);
    }
    match world::parse_props(world::PROPS_JSON) {
        Err(e) => p.push(e),
        Ok(defs) => {
            let rooms = crate::core::cam_rooms();
            for d in &defs {
                if world::prop_text(&d.model).is_none() {
                    p.push(format!("classic props: {} is not embedded", d.model));
                }
                if !rooms.contains(&d.room) {
                    p.push(format!("classic props: {} is in {}, which is not a camera room", d.model, d.room));
                }
            }
        }
    }
    let nav = crate::hunters::NavMap::classic();
    for problem in nav.validate() {
        p.push(format!("classic nav: {problem}"));
    }
    for room in crate::core::cam_rooms() {
        match nav.hub_key(&room) {
            Some(k) if !nav.route("C:stage", &k).is_empty() => {}
            _ => p.push(format!("classic nav: no route from the stage to {room}")),
        }
        if nav.cameras(&room).len() != 1 {
            p.push(format!("classic nav: {room} needs exactly one camera node"));
        }
    }
    let ids = crate::hunters::poses::pose_ids(crate::assets::POSES_JSON);
    for problem in crate::hunters::BehaviourData::embedded().validate(&ids) {
        p.push(format!("behaviour: {problem}"));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn session(night: i32) -> Session {
        let mut s = Session::new();
        s.start_mode("classic", night, BTreeMap::new(), Some(12345));
        s
    }

    #[test]
    fn classics_own_data_checks_clean() {
        let problems = check_data();
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn the_constructors_build_both_halves() {
        let s = session(2);
        let m = new_mode(&s);
        assert_eq!(m.id(), "classic");
        let h = new_headless(&s);
        assert_eq!(h.id(), "classic");
        assert_eq!(h.elapsed_ms(), 0.0);
    }

    #[test]
    fn every_night_of_the_headless_run_is_deterministic() {
        use crate::core::Outcome;
        for night in 1..=7 {
            let run = || {
                let mut h = new_headless(&session(night));
                let mut t = 0.0;
                while t < 271_000.0 && h.outcome() == Outcome::Running {
                    h.step_ms(100.0);
                    t += 100.0;
                }
                (h.outcome(), h.digest())
            };
            let (a, da) = run();
            let (b, db) = run();
            assert_ne!(a, Outcome::Running, "night {night}");
            assert_eq!((a, da), (b, db), "night {night}");
        }
    }
}
