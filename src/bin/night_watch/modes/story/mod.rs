//! M5 THE FACILITY (mode `story`): five rooms in a row and the lift car, a task list to do in order, and three residents who
//! answer to noise, to the dark and to a lift schedule. Port of the Godot `story/` folder.
//!
//! * `sim`   -- the rules (`StoryLogic`), the scripted player of `--check` (`StoryHeadless`). Pure.
//! * `world` -- the 3D map at the Godot coordinates, its stations, props, cameras and lights.
//! * `game`  -- the `Mode`: input, the walk through doors, the keypad, the monitor wall, hunters on the nav graph, the HUD.
//!
//! Data: `data/story/{rules,map,behaviour}.json` and the nav graph `data/nav/story.json`.
#![allow(dead_code)]

pub mod game;
pub mod sim;
pub mod world;

pub use game::{StoryAdmin, StoryGame};
pub use sim::{StoryHeadless, StoryLogic};

use crate::core::{Headless, Mode, Session};

/// The render-facing mode of a session (needs the window only in `Mode::start`).
pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    Box::new(StoryGame::new(session))
}

/// The window-less sim of a session, for `--check` and the tests.
pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    Box::new(StoryHeadless::new(session))
}

/// Problems in the Facility's embedded data (empty = fine), for `core::check`: the rules, the map, the nav graph and the
/// behaviour file, and that they agree with each other.
pub fn check_data() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match sim::Rules::parse(sim::RULES_JSON) {
        Ok(r) => out.extend(r.validate()),
        Err(e) => out.push(e),
    }
    let layout = match world::Layout::parse(world::MAP_JSON) {
        Ok(l) => l,
        Err(e) => {
            out.push(e);
            return out;
        }
    };
    let rules = sim::rules();
    for r in &rules.rooms {
        if r.id != "lift" && !layout.order.contains(&r.id) {
            out.push(format!("story: room {} is in the rules but not on the map", r.id));
        }
        if layout.camera(&r.id).is_none() {
            out.push(format!("story: room {} has no security camera", r.id));
        }
    }
    for t in 1..=6 {
        for task in rules.tasks_for(t) {
            if layout.camera(&task.room).is_none() {
                out.push(format!("story: task '{}' is in a room with no camera", task.label));
            }
        }
    }
    for p in &layout.props {
        if world::prop_text(&p.model).is_none() {
            out.push(format!("story: prop {} is not embedded", p.model));
        }
    }
    match crate::hunters::NavMap::from_json(world::NAV_JSON) {
        Ok(nav) => {
            out.extend(nav.validate().into_iter().map(|p| format!("story nav: {p}")));
            for cam in &layout.cameras {
                if nav.cameras(&cam.room).is_empty() {
                    out.push(format!("story nav: no camera node in {}", cam.room));
                }
                if nav.route("C:control", &format!("C:{}", cam.room)).is_empty() {
                    out.push(format!("story nav: no way from control to {}", cam.room));
                }
            }
        }
        Err(e) => out.push(format!("story nav: {e}")),
    }
    match crate::hunters::BehaviourData::parse(world::BEHAVIOUR_JSON) {
        Ok(b) => {
            let ids = crate::hunters::poses::pose_ids(crate::assets::POSES_JSON);
            out.extend(b.validate(&ids).into_iter().map(|p| format!("story behaviour: {p}")));
        }
        Err(e) => out.push(format!("story behaviour: {e}")),
    }
    for who in sim::Who::ALL {
        if crate::assets::rig_text(who.id()).is_none() || crate::assets::bbmodel_text(who.id()).is_none() {
            out.push(format!("story: {} has no embedded rig or model", who.id()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_data_checks_clean() {
        let problems = check_data();
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn the_constructors_work_without_a_window() {
        let mut s = Session::new();
        s.start_mode("story", 2, Default::default(), Some(12345));
        let mode = new_mode(&s);
        assert_eq!(mode.id(), "story");
        let mut h = new_headless(&s);
        assert_eq!(h.id(), "story");
        h.step_ms(100.0);
        assert_eq!(h.elapsed_ms(), 100.0);
    }

    #[test]
    fn the_check_runs_every_night_twice_and_they_agree() {
        let factory = |s: &Session| -> Option<Box<dyn Headless>> { Some(new_headless(s)) };
        for night in 1..=6 {
            let a = crate::core::run_headless(&factory, "story", night).unwrap_or_else(|e| panic!("{e}"));
            let b = crate::core::run_headless(&factory, "story", night).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!((a.digest, a.event_hash, a.events), (b.digest, b.event_hash, b.events), "night {night}");
            assert!(a.elapsed_ms > 0.0);
        }
    }
}
