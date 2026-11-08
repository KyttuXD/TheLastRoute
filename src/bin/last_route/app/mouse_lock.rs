//! One owner for the mouse cursor. Many screens (menus, the pause card, the update card, chat, the map, the report card, a
//! death card) used to call `set_cursor_grab` / `show_mouse` themselves and fought over it. Now a screen that needs the
//! cursor free CLAIMS it by name and releases the claim when it closes; the game wants the cursor locked only while it
//! asks for that AND no claim is held. Every grab site calls `set_wanted` (the look wants the lock) instead of the two
//! macroquad calls, and `sync` (once a frame) is the only place that touches the window.

use std::cell::RefCell;

#[derive(Default)]
struct State {
    claims: Vec<String>,
    wanted: bool,
    /// What the window was last told (None before the first sync), so it is only told again when it changes.
    applied: Option<bool>,
}

thread_local!(static STATE: RefCell<State> = RefCell::new(State::default()));

/// Names a reason the cursor must be free. Claiming twice under one name is one claim.
pub fn claim(name: &str) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.claims.iter().any(|c| c == name) {
            s.claims.push(name.to_string());
        }
    });
}

pub fn release(name: &str) {
    STATE.with(|s| s.borrow_mut().claims.retain(|c| c != name));
}

/// Claims or releases by a flag, for a screen that knows each frame whether it is open.
pub fn claim_if(name: &str, open: bool) {
    if open {
        claim(name)
    } else {
        release(name)
    }
}

pub fn any_claim() -> bool {
    STATE.with(|s| !s.borrow().claims.is_empty())
}

pub fn claim_names() -> Vec<String> {
    STATE.with(|s| s.borrow().claims.clone())
}

/// The look asks for the cursor locked (`true`) or free (`false`). A held claim wins.
pub fn set_wanted(grab: bool) {
    STATE.with(|s| s.borrow_mut().wanted = grab);
}

/// Whether the cursor is locked right now: wanted, and nothing claims it.
pub fn locked() -> bool {
    STATE.with(|s| {
        let s = s.borrow();
        decide(s.wanted, !s.claims.is_empty())
    })
}

/// The rule: any claim keeps the cursor free.
pub fn decide(wanted: bool, any_claim: bool) -> bool {
    wanted && !any_claim
}

/// What to tell the window now: `Some(lock)` when it differs from the last time, else `None`. Pure so it can be tested.
pub fn pending() -> Option<bool> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let now = decide(s.wanted, !s.claims.is_empty());
        if s.applied == Some(now) {
            None
        } else {
            s.applied = Some(now);
            Some(now)
        }
    })
}

/// Tells the window the cursor state, when it changed. Call once a frame.
pub fn sync() {
    if let Some(lock) = pending() {
        macroquad::prelude::set_cursor_grab(lock);
        macroquad::prelude::show_mouse(!lock);
    }
}

/// Forgets what the window was told, so the next `sync` sends it again (after the browser dropped the pointer lock).
pub fn forget_applied() {
    STATE.with(|s| s.borrow_mut().applied = None);
}

#[cfg(test)]
pub fn reset_for_test() {
    STATE.with(|s| *s.borrow_mut() = State::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn any_claim_keeps_the_cursor_free() {
        reset_for_test();
        set_wanted(true);
        assert!(locked());
        claim("pause");
        claim("chat");
        assert!(!locked());
        release("pause");
        assert!(!locked(), "chat still holds it");
        release("chat");
        assert!(locked());
    }

    #[test]
    fn a_name_claims_once_and_one_release_frees_it() {
        reset_for_test();
        claim("map");
        claim("map");
        assert_eq!(claim_names(), ["map"]);
        release("map");
        assert!(!any_claim());
        claim_if("map", true);
        claim_if("map", false);
        assert!(!any_claim());
    }

    #[test]
    fn the_window_is_only_told_on_a_change() {
        reset_for_test();
        set_wanted(true);
        assert_eq!(pending(), Some(true));
        assert_eq!(pending(), None);
        claim("update");
        assert_eq!(pending(), Some(false));
        assert_eq!(pending(), None);
        forget_applied();
        assert_eq!(pending(), Some(false), "re-sent after forgetting");
        assert!(!decide(true, true) && decide(true, false) && !decide(false, false));
    }
}
