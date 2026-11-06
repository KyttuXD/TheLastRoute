//! The web build's update notice: the page's script (packaging/web/update.js, copied to dist/web) polls `version.json` and
//! exports `lr_update_available` (1 once the deployed build differs from the one this page loaded with) and
//! `lr_update_reload` (reloads the page). This module wraps those two imports and holds [`UpdateScreen`], the pure state
//! machine behind the UPDATING screen (ui/update_screen.rs draws it; the numbers are in data/ui/update_screen.json).
//!
//! The states: `Idle` (nothing to say), `Counting` (the screen is up and counts down to a reload), `Postponed` (the player
//! chose LATER during a run: the screen is hidden until a time), and `Reloading` (the reload was asked for). The machine
//! keeps its own clock from the `dt` it is given, so it has no macroquad call in it and is unit tested natively.
//! Native builds have no page: the stubs always say no update, so the screen never opens there.

#[cfg(target_arch = "wasm32")]
mod imp {
    extern "C" {
        fn lr_update_available() -> i32;
        fn lr_update_reload();
    }

    pub fn available() -> bool {
        // SAFETY: both imports are plain functions of the page's update plugin and take no pointers.
        unsafe { lr_update_available() != 0 }
    }

    pub fn reload() {
        // SAFETY: see above.
        unsafe { lr_update_reload() }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    pub fn available() -> bool {
        false
    }

    pub fn reload() {}
}

/// Whether the deployed version differs from the one this page loaded with (always false off the web).
pub fn update_available() -> bool {
    imp::available()
}

/// Reloads the page (does nothing off the web).
pub fn reload() {
    imp::reload()
}

/// Where the UPDATING screen is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// No update is waiting (or it is not shown): nothing is drawn.
    Idle,
    /// The screen is up; the reload happens in this many seconds.
    Counting(f64),
    /// LATER was chosen in a run: the screen comes back at this time on the machine's own clock.
    Postponed(f64),
    /// The reload was asked for (the page is about to restart).
    Reloading,
}

/// What a tick or a button makes happen outside the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The screen just came up (first time, or back after LATER): the game lets go of the mouse.
    Shown,
    /// Reload the page now (`web_update::reload`).
    Reload,
}

/// The timing numbers (data/ui/update_screen.json).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    /// Seconds the screen counts down before it reloads.
    pub countdown: f64,
    /// Seconds LATER hides the screen for.
    pub postpone: f64,
    /// The most one tick may advance the clock (a tab back from the background must not skip the countdown).
    pub max_step: f64,
}

/// The UPDATING screen's state.
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateScreen {
    timing: Timing,
    phase: Phase,
    /// The machine's clock, in seconds of ticked time.
    now: f64,
    /// Whether the last tick had a run in progress (LATER is allowed only then).
    in_run: bool,
}

impl UpdateScreen {
    pub fn new(timing: Timing) -> UpdateScreen {
        UpdateScreen { timing, phase: Phase::Idle, now: 0.0, in_run: false }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The screen is up (counting, or reloading): it draws and holds the input.
    pub fn is_shown(&self) -> bool {
        matches!(self.phase, Phase::Counting(_) | Phase::Reloading)
    }

    /// The whole seconds the countdown shows (rounded up, so 10 shows first and 1 shows last), when it is counting.
    pub fn seconds_shown(&self) -> Option<u32> {
        match self.phase {
            Phase::Counting(left) => Some(left.max(0.0).ceil() as u32),
            _ => None,
        }
    }

    /// How much of the countdown is left, 1.0 down to 0.0 (0.0 when not counting).
    pub fn progress(&self) -> f32 {
        match self.phase {
            Phase::Counting(left) => (left / self.timing.countdown).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        }
    }

    /// Whether the LATER button is offered: only while the screen is up over a run.
    pub fn can_postpone(&self) -> bool {
        self.in_run && matches!(self.phase, Phase::Counting(_))
    }

    /// One frame. `dt` is the frame's time, `update_available` what the page says, `in_run` whether a run is in progress.
    pub fn tick(&mut self, dt: f32, update_available: bool, in_run: bool) -> Vec<Event> {
        let step = (dt as f64).clamp(0.0, self.timing.max_step);
        self.now += step;
        self.in_run = in_run;
        let mut events = Vec::new();
        if !update_available && self.phase != Phase::Reloading {
            self.phase = Phase::Idle;
            return events;
        }
        match self.phase {
            Phase::Idle => {
                if update_available {
                    self.phase = Phase::Counting(self.timing.countdown);
                    events.push(Event::Shown);
                }
            }
            Phase::Counting(left) => {
                let left = left - step;
                if left <= 0.0 {
                    self.phase = Phase::Reloading;
                    events.push(Event::Reload);
                } else {
                    self.phase = Phase::Counting(left);
                }
            }
            Phase::Postponed(until) => {
                // out of the run (it ended or was left) the reason to wait is gone: the screen comes back at once
                if self.now >= until || !in_run {
                    self.phase = Phase::Counting(self.timing.countdown);
                    events.push(Event::Shown);
                }
            }
            Phase::Reloading => {}
        }
        events
    }

    /// UPDATE NOW: reload at once. `Some(Reload)` when the screen was up (or postponed), else nothing happens.
    pub fn update_now(&mut self) -> Option<Event> {
        match self.phase {
            Phase::Counting(_) | Phase::Postponed(_) => {
                self.phase = Phase::Reloading;
                Some(Event::Reload)
            }
            Phase::Idle | Phase::Reloading => None,
        }
    }

    /// LATER: hide the screen for the postpone time. Allowed only while the screen is up over a run; false otherwise.
    pub fn later(&mut self) -> bool {
        if !self.can_postpone() {
            return false;
        }
        self.phase = Phase::Postponed(self.now + self.timing.postpone);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Timing = Timing { countdown: 10.0, postpone: 120.0, max_step: 0.5 };

    fn screen() -> UpdateScreen {
        UpdateScreen::new(T)
    }

    /// Ticks `seconds` of time in 0.25 s frames and gathers every event.
    fn run_for(s: &mut UpdateScreen, seconds: f32, available: bool, in_run: bool) -> Vec<Event> {
        let mut all = Vec::new();
        let frames = (seconds / 0.25).round() as usize;
        for _ in 0..frames {
            all.extend(s.tick(0.25, available, in_run));
        }
        all
    }

    #[test]
    fn native_builds_never_see_an_update() {
        assert!(!update_available());
        reload();
    }

    #[test]
    fn no_update_means_idle_and_never_draws() {
        let mut s = screen();
        assert!(run_for(&mut s, 60.0, false, true).is_empty());
        assert_eq!(s.phase(), Phase::Idle);
        assert!(!s.is_shown());
        assert_eq!(s.seconds_shown(), None);
        assert!(!s.can_postpone());
    }

    #[test]
    fn an_update_opens_the_screen_and_counts_down_to_one_reload() {
        let mut s = screen();
        assert_eq!(s.tick(0.016, true, false), vec![Event::Shown]);
        assert!(s.is_shown());
        assert_eq!(s.seconds_shown(), Some(10));
        // 9 seconds in: still counting, 1 second left
        let early = run_for(&mut s, 9.0, true, false);
        assert!(early.is_empty(), "no reload before the countdown ends");
        assert_eq!(s.seconds_shown(), Some(1));
        // the rest of it: exactly one Reload
        let late = run_for(&mut s, 2.0, true, false);
        assert_eq!(late, vec![Event::Reload]);
        assert_eq!(s.phase(), Phase::Reloading);
        assert!(s.is_shown(), "the screen stays up while the page restarts");
        assert!(run_for(&mut s, 5.0, true, false).is_empty(), "the reload is asked for once");
    }

    #[test]
    fn the_countdown_shows_ten_down_to_one() {
        let mut s = screen();
        s.tick(0.0, true, false);
        let mut seen = vec![s.seconds_shown().unwrap()];
        for _ in 0..40 {
            s.tick(0.25, true, false);
            if let Some(n) = s.seconds_shown() {
                if seen.last() != Some(&n) {
                    seen.push(n);
                }
            }
        }
        assert_eq!(seen, vec![10, 9, 8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn update_now_reloads_at_once() {
        let mut s = screen();
        assert_eq!(s.update_now(), None, "nothing to update while idle");
        s.tick(0.016, true, false);
        assert_eq!(s.update_now(), Some(Event::Reload));
        assert_eq!(s.phase(), Phase::Reloading);
        assert_eq!(s.update_now(), None, "asked once");
    }

    #[test]
    fn later_works_only_in_a_run_and_the_screen_comes_back_after_the_postpone() {
        let mut s = screen();
        s.tick(0.016, true, true);
        assert!(s.can_postpone());
        assert!(s.later());
        assert!(!s.is_shown());
        assert!(matches!(s.phase(), Phase::Postponed(_)));
        // 100 s later it is still hidden, the run goes on
        assert!(run_for(&mut s, 100.0, true, true).is_empty());
        assert!(!s.is_shown());
        // past the 120 s it is back, counting from the full countdown again
        let back = run_for(&mut s, 21.0, true, true);
        assert_eq!(back, vec![Event::Shown]);
        assert!(s.is_shown());
        assert!(s.seconds_shown().unwrap() >= 9);
        assert!(s.can_postpone(), "it may be postponed again");
    }

    #[test]
    fn there_is_no_later_outside_a_run() {
        let mut s = screen();
        s.tick(0.016, true, false);
        assert!(!s.can_postpone());
        assert!(!s.later());
        assert!(matches!(s.phase(), Phase::Counting(_)), "the refused LATER changes nothing");
        // a run that ended during the countdown takes LATER away
        let mut s = screen();
        s.tick(0.016, true, true);
        assert!(s.can_postpone());
        s.tick(0.016, true, false);
        assert!(!s.can_postpone());
        assert!(!s.later());
    }

    #[test]
    fn a_postponed_screen_returns_early_when_the_run_ends() {
        let mut s = screen();
        s.tick(0.016, true, true);
        assert!(s.later());
        assert!(run_for(&mut s, 10.0, true, true).is_empty());
        assert_eq!(s.tick(0.016, true, false), vec![Event::Shown], "back at the title: nothing to protect");
    }

    #[test]
    fn update_now_works_from_a_postponed_screen_too() {
        let mut s = screen();
        s.tick(0.016, true, true);
        s.later();
        assert_eq!(s.update_now(), Some(Event::Reload));
    }

    #[test]
    fn a_long_frame_cannot_skip_the_countdown() {
        let mut s = screen();
        s.tick(0.016, true, false);
        // a tab back from the background reports a 30 s frame: it counts as at most max_step
        assert!(s.tick(30.0, true, false).is_empty());
        assert!(s.seconds_shown().unwrap() >= 9);
    }

    #[test]
    fn the_update_going_away_closes_the_screen() {
        let mut s = screen();
        s.tick(0.016, true, false);
        s.tick(0.016, false, false);
        assert_eq!(s.phase(), Phase::Idle);
    }
}
