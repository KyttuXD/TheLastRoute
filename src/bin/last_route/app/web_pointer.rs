//! Browser pointer lock for the web build. The page's script (dist/web/index.html) registers `lr_pointer_locked`, which
//! reports whether the game canvas really owns the pointer. Browsers can drop the lock on Escape or when the window
//! loses focus, and they only grant a new lock on a user gesture, so the game asks the browser instead of trusting its
//! own flag. Native builds always report the lock as held and never gate a request.

use macroquad::time::get_time;
use std::cell::Cell;

/// Seconds between two lock requests that are not a click (a click is a user gesture and always may ask).
pub const RETRY_SECONDS: f64 = 0.3;

thread_local! {
    static LAST_REQUEST: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
    /// The browser has granted the lock since the game last asked for it.
    static HELD: Cell<bool> = const { Cell::new(false) };
}

/// Call every frame with whether the game believes the mouse is grabbed. True only on the frame a lock the browser had
/// granted is lost (Escape, focus loss). A request the browser has not granted yet, or refused, is not a loss: the
/// grant arrives a moment after the request, so treating "not locked yet" as lost would pause the game on entry.
/// Always false off the web.
pub fn lost_after_held(grabbed: bool) -> bool {
    if !cfg!(target_arch = "wasm32") {
        return false;
    }
    HELD.with(|held| {
        if !grabbed {
            held.set(false);
            false
        } else if locked() {
            held.set(true);
            false
        } else if held.get() {
            held.set(false);
            true
        } else {
            false
        }
    })
}

#[cfg(target_arch = "wasm32")]
mod imp {
    extern "C" {
        fn lr_pointer_locked() -> i32;
    }

    pub fn locked() -> bool {
        unsafe { lr_pointer_locked() != 0 }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    pub fn locked() -> bool {
        true
    }
}

/// True when the browser really holds the pointer lock (always true off the web).
pub fn locked() -> bool {
    imp::locked()
}

/// Whether a lock request may be sent now: a click is a user gesture and always may; otherwise at most every
/// `RETRY_SECONDS`, so a refused request is not repeated every frame. Off the web every request may go out.
pub fn may_request(click: bool) -> bool {
    if !cfg!(target_arch = "wasm32") {
        return true;
    }
    let now = get_time();
    LAST_REQUEST.with(|last| {
        if click || now - last.get() >= RETRY_SECONDS {
            last.set(now);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_builds_always_allow_the_request_and_report_the_lock() {
        assert!(locked());
        assert!(may_request(false));
        assert!(may_request(true));
    }
}
