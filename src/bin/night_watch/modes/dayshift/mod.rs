//! Mode 7, SALVAGE RUN (`dayshift`): spend the week's budget in the shop, then salvage pieces off the rack in the cargo bay
//! while two door visitors and Ysolde at the window are kept out. Port of `NW\dayshift\*.gd`.
//!
//! * `sim`   -- the rules (`DayshiftSim`), the tables (`data/dayshift/tunables.json`), the shop helpers, the bot (`Autoplay`).
//! * `stage` -- the hunters on the ship's navigation graph (`data/nav/dayshift.json`) and the headless run.
//! * `shop`  -- the day's shop (state and screen).
//! * `world` -- the bay, the ship and the scrap yard, the security cameras and the props (`data/dayshift/map.json`).
//! * `game`  -- the `Mode`.
//! * `admin` -- the dev panel (`DayshiftAdmin`).
//!
//! Hub hooks (integrator): `crate::modes::dayshift::{new_mode, new_headless}` in `registry.rs`; `DayshiftAdmin` in the
//! admin registry; `next_pressed()` when the player picks NEXT on a won week's end card (carries the surplus into the next
//! week's shop); `data_problems()` in the `--check` data checks.
#![allow(dead_code)]

pub mod admin;
pub mod game;
pub mod shop;
pub mod sim;
pub mod stage;
pub mod world;

pub use admin::DayshiftAdmin;

use crate::core::{Headless, Mode, Session};
use std::sync::atomic::{AtomicI32, Ordering};

/// The render-facing mode of the session.
pub fn new_mode(session: &Session) -> Box<dyn Mode> {
    Box::new(game::DayshiftMode::new(session))
}

/// The window-less run (`--check`, tests): the rules, the walking hunters and a seeded bot at the controls.
pub fn new_headless(session: &Session) -> Box<dyn Headless> {
    Box::new(stage::DayshiftHeadless::new(session))
}

// ---- last week's surplus --------------------------------------------------------------------------------------------------------

/// Last week's surplus, handed to the next week's shop only when the player plays on from the end card. Packed as
/// `next_week * 1000 + carry` (a carry is 0..=40) so a carry meant for week 3 is never used by another week. Process-wide,
/// like the Godot `pending_carry` static.
pub static PENDING_CARRY: AtomicI32 = AtomicI32::new(0);
/// The surplus of the week just won, waiting for the player to pick NEXT.
static WIN_CARRY: AtomicI32 = AtomicI32::new(0);

/// A week was won: remember the surplus for `next_week` (called by the mode).
pub fn store_win_carry(next_week: i32, carry: i32) {
    WIN_CARRY.store(next_week * 1000 + carry.clamp(0, 999), Ordering::Relaxed);
}

/// The player picked NEXT on the end card: the won week's surplus becomes the pending carry. Call it from the hub before the
/// next week's mode is made. Anything else (RETRY, MENU, a fresh start) leaves it behind.
pub fn next_pressed() {
    PENDING_CARRY.store(WIN_CARRY.swap(0, Ordering::Relaxed), Ordering::Relaxed);
}

/// The pending carry if it was meant for `week` (and forget it either way).
pub fn take_pending_carry(week: i32) -> i32 {
    let v = PENDING_CARRY.swap(0, Ordering::Relaxed);
    if v > 0 && v / 1000 == week {
        v % 1000
    } else {
        0
    }
}

/// Tests that make a mode (which takes the pending carry) or touch the carry take this lock.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Problems in the mode's data files and map (for `--check`): the tables, the map JSON, the nav graph, and a route from the
/// airlock to every camera room.
pub fn data_problems() -> Vec<String> {
    let mut p = sim::data_problems();
    p.extend(world::data_problems());
    let nav = stage::load_nav();
    p.extend(nav.validate().into_iter().map(|s| format!("dayshift nav: {s}")));
    for c in &world::map().cameras {
        match nav.hub_key(&c.room) {
            Some(hub) => {
                if nav.route("C:airlock", &hub).is_empty() {
                    p.push(format!("dayshift nav: no route to the {} camera room", c.room));
                }
            }
            None => p.push(format!("dayshift nav: room {} has no hub", c.room)),
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Outcome;

    #[test]
    fn the_mode_data_is_clean() {
        let p = data_problems();
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn the_constructors_make_a_mode_and_a_headless_run() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Session::new();
        s.start_mode("dayshift", 2, Default::default(), Some(12345));
        let m = new_headless(&s);
        assert_eq!(m.id(), "dayshift");
        assert_eq!(m.outcome(), Outcome::Running);
        let mode = new_mode(&s);
        assert_eq!(mode.id(), "dayshift");
        assert_eq!(mode.outcome(), Outcome::Running);
    }

    #[test]
    fn a_seeded_headless_night_reaches_an_outcome_with_the_same_digest_twice() {
        let run = |seed: u32| {
            let mut s = Session::new();
            s.start_mode("dayshift", 1, Default::default(), Some(seed));
            let mut h = new_headless(&s);
            let mut steps = 0;
            while h.outcome() == Outcome::Running && steps < 3000 {
                h.step_ms(100.0);
                steps += 1;
            }
            (h.outcome(), h.digest(), h.elapsed_ms() as i64)
        };
        let a = run(12345);
        assert_ne!(a.0, Outcome::Running);
        assert_eq!(a, run(12345));
        assert_ne!(a.1, run(54321).1);
    }

    /// One test owns the carry statics so parallel tests cannot disturb it.
    #[test]
    fn the_surplus_carries_only_into_the_next_week_and_only_after_next() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        PENDING_CARRY.store(0, Ordering::Relaxed);
        WIN_CARRY.store(0, Ordering::Relaxed);
        // a win alone carries nothing (RETRY and MENU leave it behind)
        store_win_carry(3, 25);
        assert_eq!(take_pending_carry(3), 0);
        // NEXT hands it over, to week 3 only, and only once
        store_win_carry(3, 25);
        next_pressed();
        assert_eq!(take_pending_carry(4), 0, "meant for week 3");
        store_win_carry(3, 25);
        next_pressed();
        assert_eq!(take_pending_carry(3), 25);
        assert_eq!(take_pending_carry(3), 0, "used up");
        // a carry of zero is nothing
        store_win_carry(2, 0);
        next_pressed();
        assert_eq!(take_pending_carry(2), 0);
        // the cap
        store_win_carry(5, 40);
        next_pressed();
        assert_eq!(take_pending_carry(5), 40);
    }
}
