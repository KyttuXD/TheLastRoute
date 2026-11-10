//! Mode 6, THE DESCENT (Godot `game/night_watch/descent/*.gd`): ride a service elevator down a shaft and repair it stage by
//! stage (wires, signal pads, pipes, the door hold) while Elise comes along the sill outside the caged window.
//!
//! * `sim`      the rules (descent_logic.gd, descent_data.gd), pure and seeded
//! * `map`      view numbers, the map file (rooms, flights, props) and the nav graph (descent_world.gd, descent_map.gd)
//! * `elise`    Elise as a living hunter on the nav graph: rides in from the stairwell, walks the sill, idles
//! * `panel`    the service panel's layout, hit tests and drawing (descent_panel.gd)
//! * `world`    the 3D set: car, windows, shaft, sliding dress, the stairwell/gallery/closet/hall rooms and their props
//! * `game`     the `Mode` (descent_game.gd)
//! * `headless` the window-less run for `--check` and the tests
//! * `admin`    the dev panel (descent_admin.gd)
#![allow(dead_code)]

pub mod admin;
pub mod elise;
pub mod game;
pub mod headless;
pub mod map;
pub mod panel;
pub mod sim;
pub mod world;

pub use admin::DescentAdmin;

use crate::core::{Headless, Mode, Session};

/// The render-facing mode of a Descent shift.
pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    Box::new(game::DescentMode::new(session))
}

/// The window-less sim (scripted player, Elise walking the nav graph) of a Descent shift.
pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    Box::new(headless::DescentHeadless::new(session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Outcome;
    use std::collections::BTreeMap;

    fn session(level: i32) -> Session {
        let mut s = Session::new();
        s.start_mode("descent", level, BTreeMap::new(), Some(0x5eed));
        s
    }

    #[test]
    fn the_constructors_give_a_descent_mode_and_a_descent_sim() {
        let s = session(2);
        assert_eq!(new_mode(&s).id(), "descent");
        let h = new_headless(&s);
        assert_eq!(h.id(), "descent");
        assert_eq!(h.outcome(), Outcome::Running);
        assert_eq!(h.elapsed_ms(), 0.0);
    }

    #[test]
    fn a_seeded_headless_shift_reaches_an_outcome_on_the_check_seed() {
        for level in 1..=5 {
            let s = session(level);
            let mut h = new_headless(&s);
            let mut t = 0.0;
            while t < 600_000.0 && h.outcome() == Outcome::Running {
                h.step_ms(100.0);
                t += 100.0;
            }
            assert_ne!(h.outcome(), Outcome::Running, "descent {level}");
        }
    }

    #[test]
    fn the_mode_data_matches_the_five_descents() {
        let modes = crate::core::ModesData::load();
        let m = modes.get("descent").expect("descent entry");
        assert_eq!(m.levels.len(), 5);
        assert_eq!(m.cast, vec!["elise".to_string()]);
        assert!(m.custom.as_ref().map_or(false, |c| c.iter().any(|(id, _)| id == "elise")));
    }
}
