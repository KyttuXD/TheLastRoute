#![allow(dead_code)]
//! Mode set M2: Masked Hours (mode id `mask`, menu "2. MASKED HOURS"). Port of `game/night_watch/mask/*.gd`.
//!
//! * `data`     -- the cast, rooms, graph, tuning tables and 3D map numbers (JSON under `data/mask/` and `data/nav/mask.json`).
//! * `sim`      -- the rules (`MaskSim`), pure and seeded; the original's `createMaskGame`.
//! * `controls` -- the exclusive mask / monitor panel machine and the held controls (`MaskControls`).
//! * `walk`     -- the hunters walking the nav graph: targets from the rules, the shared `Crowd` carries them there.
//! * `props`    -- the Blockbench props that dress the wing.
//! * `world`    -- the office and the wing as cubes in named groups, the lights, the station table.
//! * `game`     -- the `Mode` (input, sounds, HUD, the office view, the live camera feeds, end cards, admin hooks).
//! * `headless` -- the window-less run with a seeded autoplay, for `--check` and the tests.
//! * `admin`    -- the admin panel rows (`MaskAdmin`).
//!
//! The integrator wires `new_mode` / `new_headless` into `registry.rs`, adds `pub mod mask;` to `modes/mod.rs` and registers
//! `MaskAdmin` in the admin registry.

pub mod admin;
pub mod controls;
pub mod data;
pub mod game;
pub mod headless;
pub mod props;
pub mod sim;
pub mod walk;
pub mod world;

pub use admin::MaskAdmin;

use crate::core::{Headless, Mode, Session};

/// The render-facing mode of a Masked Hours shift.
pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    Box::new(game::MaskMode::new(session))
}

/// The window-less sim of a Masked Hours shift (seeded autoplay).
pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    Box::new(headless::MaskHeadless::new(session))
}

/// Every problem in the mode's data files (for `--check`).
pub fn check_data() -> Vec<String> {
    data::check()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Outcome;

    #[test]
    fn the_constructors_make_a_mode_and_a_sim() {
        let mut s = Session::new();
        s.start_mode("mask", 2, Default::default(), Some(9));
        let m = new_mode(&s);
        assert_eq!(m.id(), "mask");
        assert_eq!(m.outcome(), Outcome::Running);
        let h = new_headless(&s);
        assert_eq!(h.id(), "mask");
        assert_eq!(h.outcome(), Outcome::Running);
    }

    #[test]
    fn the_data_is_clean() {
        assert!(check_data().is_empty(), "{:?}", check_data());
    }
}
