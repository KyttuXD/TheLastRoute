//! Mode registry: mode id -> constructors, and which modes are playable yet. Hub file (integrator).
//!
//! All nine modes are built. A new mode adds ONE line to each of the three tables below (the `BUILT` row, the
//! `make_mode` arm and the `make_headless` arm).
#![allow(dead_code)]

use crate::core::{Headless, Mode, Session};

pub const MODE_IDS: [&str; 9] = ["classic", "mask", "lure", "bedroom", "story", "descent", "dayshift", "extra", "mixed"];

/// Modes whose code has landed. M-PASS HOOK 1 of 3: add the mode id here.
const BUILT: &[&str] = &["classic", "mask", "lure", "bedroom", "story", "descent", "dayshift", "extra", "mixed"];

/// True when the mode can be started from the menu.
pub fn playable(mode_id: &str) -> bool {
    BUILT.contains(&mode_id)
}

/// The first embedded-data load error of the mode, so a broken data file refuses the start instead of playing on a patch.
pub fn load_error(mode_id: &str) -> Option<String> {
    use crate::modes as m;
    let e = match mode_id {
        "classic" => m::classic::view::load_error().or_else(m::classic::world::load_error),
        "mask" => m::mask::data::load_error(),
        "lure" => m::lure::data::load_error().or_else(m::lure::map::load_error).or_else(m::lure::presence::load_error),
        "bedroom" => m::bedroom::map::load_error().or_else(m::bedroom::sim::load_error),
        "story" => m::story::sim::load_error().or_else(m::story::world::load_error),
        "dayshift" => m::dayshift::sim::load_error().or_else(m::dayshift::stage::load_error).or_else(m::dayshift::world::load_error),
        "extra" => m::openair::data::load_error(),
        "descent" => m::descent::map::load_error(),
        "mixed" => m::mixed::world::try_nav().err(),
        _ => None,
    };
    e.map(|e| format!("{}: {e}", mode_id.to_uppercase()))
}

/// The render-facing mode of the session, or None for a mode that is not built.
pub fn make_mode(session: &Session) -> Option<Box<dyn Mode>> {
    match session.mode_id.as_str() {
        "classic" => Some(crate::modes::classic::new_mode(session)),
        "mask" => Some(crate::modes::mask::new_mode(session)),
        "lure" => Some(crate::modes::lure::new_mode(session)),
        "bedroom" => Some(crate::modes::bedroom::new_mode(session)),
        "story" => Some(crate::modes::story::new_mode(session)),
        "descent" => Some(crate::modes::descent::new_mode(session)),
        "dayshift" => Some(crate::modes::dayshift::new_mode(session)),
        "extra" => Some(crate::modes::openair::new_mode(session)),
        "mixed" => Some(crate::modes::mixed::new_mode(session)),
        _ => None,
    }
}

/// The window-less sim of the session, or None for a mode that is not built.
pub fn make_headless(session: &Session) -> Option<Box<dyn Headless>> {
    match session.mode_id.as_str() {
        "classic" => Some(crate::modes::classic::new_headless(session)),
        "mask" => Some(crate::modes::mask::new_headless(session)),
        "lure" => Some(crate::modes::lure::new_headless(session)),
        "bedroom" => Some(crate::modes::bedroom::new_headless(session)),
        "story" => Some(crate::modes::story::new_headless(session)),
        "descent" => Some(crate::modes::descent::new_headless(session)),
        "dayshift" => Some(crate::modes::dayshift::new_headless(session)),
        "extra" => Some(crate::modes::openair::new_headless(session)),
        "mixed" => Some(crate::modes::mixed::new_headless(session)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_the_data_list() {
        assert_eq!(MODE_IDS, crate::core::MODE_ID_LIST);
    }

    #[test]
    fn built_modes_match_the_tables() {
        assert!(MODE_IDS.iter().all(|id| playable(id)));
        let mut s = Session::new();
        for id in MODE_IDS {
            s.start_mode(id, 1, Default::default(), Some(1));
            // make_mode builds GL resources, so the tests only construct the window-less side
            assert_eq!(playable(id), make_headless(&s).is_some());
        }
    }
}
