//! Frame cap and focus pause. The FPS LIMIT row (`Settings::frame_cap_fps`) and PAUSE ON FOCUS LOSS meet here: while the
//! window is unfocused and the pause row is on, the loop is held to `frame.unfocused_fps` (data/controls.json) so a
//! background game does not burn the CPU. `app::pacing::Pacer::wait` does the sleeping from the cap this returns.
//! VSYNC follows the window config and `app::pacing::apply_vsync`; whether vsync tears on a given panel is checked by eye.

use super::bindings::Frame;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

static FOCUSED: AtomicBool = AtomicBool::new(true);

/// The window focus, set by whatever sees the focus events (main.rs); defaults to focused.
pub fn set_focused(on: bool) {
    FOCUSED.store(on, Ordering::Relaxed);
}

pub fn focused() -> bool {
    FOCUSED.load(Ordering::Relaxed)
}

/// The cap to hold the loop to: the FPS LIMIT, lowered to the unfocused rate while the window is unfocused and
/// PAUSE ON FOCUS LOSS is on. `None` is unlimited.
pub fn effective_cap(user: Option<u32>, focused: bool, pause_on_focus_loss: bool, f: &Frame) -> Option<u32> {
    if !focused && pause_on_focus_loss && f.unfocused_fps > 0 {
        Some(user.map_or(f.unfocused_fps, |c| c.min(f.unfocused_fps)))
    } else {
        user
    }
}

/// The length of one frame at `fps`.
pub fn period(fps: u32) -> Duration {
    Duration::from_secs_f64(1.0 / fps.max(1) as f64)
}

/// How long to sleep after a frame that took `elapsed`, to hold the loop to `cap`.
pub fn sleep_for(elapsed: Duration, cap: Option<u32>) -> Duration {
    match cap {
        Some(fps) => period(fps).saturating_sub(elapsed),
        None => Duration::ZERO,
    }
}

/// The extra sleep an unfocused paused window takes each frame, so it yields the CPU even when the cap is a no-op.
pub fn unfocused_sleep(focused: bool, pause_on_focus_loss: bool, f: &Frame) -> Duration {
    if !focused && pause_on_focus_loss {
        Duration::from_millis(f.unfocused_sleep_ms)
    } else {
        Duration::ZERO
    }
}

/// The ADS setting as a level: 0 off, 1 gentle, 2 annoying (anything else is off).
pub fn ads_level(ads: &str) -> u8 {
    match ads {
        "gentle" => 1,
        "annoying" => 2,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::bindings::rules;

    fn f() -> &'static Frame {
        &rules().frame
    }

    #[test]
    fn the_cap_math_holds_the_loop_to_the_period() {
        assert_eq!(period(60), Duration::from_secs_f64(1.0 / 60.0));
        let slept = sleep_for(Duration::from_millis(5), Some(60));
        assert!(slept > Duration::from_millis(11) && slept < Duration::from_millis(12), "{slept:?}");
        assert_eq!(sleep_for(Duration::from_millis(40), Some(60)), Duration::ZERO, "a slow frame is not slept on");
        assert_eq!(sleep_for(Duration::from_millis(1), None), Duration::ZERO);
    }

    #[test]
    fn unfocused_lowers_the_cap_only_when_pausing_is_on() {
        let u = f().unfocused_fps;
        assert_eq!(effective_cap(None, true, true, f()), None);
        assert_eq!(effective_cap(None, false, true, f()), Some(u));
        assert_eq!(effective_cap(Some(144), false, true, f()), Some(u));
        assert_eq!(effective_cap(Some(5), false, true, f()), Some(5), "a lower player cap stays");
        assert_eq!(effective_cap(Some(144), false, false, f()), Some(144));
        assert_eq!(effective_cap(None, false, false, f()), None);
    }

    #[test]
    fn unfocused_sleep_is_data_and_zero_when_focused() {
        assert_eq!(unfocused_sleep(true, true, f()), Duration::ZERO);
        assert_eq!(unfocused_sleep(false, false, f()), Duration::ZERO);
        assert_eq!(unfocused_sleep(false, true, f()), Duration::from_millis(f().unfocused_sleep_ms));
        let slow = Frame { unfocused_sleep_ms: 200, ..f().clone() };
        assert_eq!(unfocused_sleep(false, true, &slow), Duration::from_millis(200));
    }

    #[test]
    fn ads_levels() {
        assert_eq!((ads_level("off"), ads_level("gentle"), ads_level("annoying"), ads_level("x")), (0, 1, 2, 0));
    }

    #[test]
    fn focus_flag_round_trips() {
        set_focused(false);
        assert!(!focused());
        set_focused(true);
        assert!(focused());
    }
}
