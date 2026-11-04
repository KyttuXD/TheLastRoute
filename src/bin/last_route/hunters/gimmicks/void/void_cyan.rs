//! Cyan Void: signal ping (void_cyan_gimmick.gd). Two and a half seconds of unbroken direct sight of the runner
//! (not hidden, not in a safe zone) broadcasts the runner's exact position as a facility alert to every other active
//! hunter. A successful ping starts a 30 s cooldown; sight during the cooldown counts for nothing.
//!
//! Integrator: `Gimmick::VoidCyan(void::void_cyan::VoidCyan)` in the enum, `"void_cyan" => Gimmick::VoidCyan(Default::default())`
//! in `for_id`, and `Gimmick::VoidCyan(g) => g.after(b, dt, t, ev)` in `after_perception`. No player-side effect is needed:
//! the only outputs are `Event::FacilityAlert { except_self: true }` (the other hunters investigate `t.pos`) and the
//! `Event::Message` callout, which `silence_callouts` drops while `ANNOUNCE_HUNTER_EVENTS` is off, as in Godot.

use crate::hunters::brain::{Brain, Target};
use crate::hunters::gimmicks::Event;

/// void_cyan_gimmick.gd:7 `SIGHT_REQUIRED := 2.5`: seconds of unbroken sight before the ping fires.
pub const SIGHT_REQUIRED: f32 = 2.5;
/// void_cyan_gimmick.gd:8 `INTERVAL := 30.0`: cooldown after a successful ping (void_cyan_gimmick.gd:25).
pub const INTERVAL: f32 = 30.0;
/// void_cyan_gimmick.gd:27 `"message"` of the facility alert.
pub const PING_MESSAGE: &str = "CYAN SIGNAL // YOUR POSITION WAS BROADCAST";

#[derive(Default)]
pub struct VoidCyan {
    /// Unbroken sight accumulated so far (void_cyan_gimmick.gd `_seen`).
    seen: f32,
    /// Seconds left before the next ping can start (void_cyan_gimmick.gd `_cooldown`).
    cooldown: f32,
}

/// The sight rule (void_cyan_gimmick.gd:18): the brain sees the runner directly (`direct_chase_visible`, here
/// `Brain::direct_visible`), and the runner is neither hidden nor in a safe zone.
pub fn sighted(direct_visible: bool, t: &Target) -> bool {
    direct_visible && !t.hidden && !t.in_safe_zone
}

impl VoidCyan {
    /// Seconds left on the cooldown; zero when the ping is ready.
    pub fn cooldown_left(&self) -> f32 {
        self.cooldown
    }

    /// Seconds of unbroken sight accumulated toward the next ping.
    pub fn sight_built(&self) -> f32 {
        self.seen
    }

    /// One tick of the ping clock (void_cyan_gimmick.gd:15-25). Returns true on the tick the ping fires.
    /// The cooldown runs down first; while it is still running the tick returns before any sight is counted
    /// (line 16-17), so `seen` is neither built nor reset during the cooldown.
    pub fn tick(&mut self, dt: f32, sighted: bool) -> bool {
        self.cooldown = (self.cooldown - dt).max(0.0);
        if self.cooldown > 0.0 {
            return false;
        }
        // line 18-21: any break in sight resets the clock to zero
        self.seen = if sighted { self.seen + dt } else { 0.0 };
        // line 22-23
        if self.seen < SIGHT_REQUIRED {
            return false;
        }
        // line 24-25
        self.seen = 0.0;
        self.cooldown = INTERVAL;
        true
    }

    /// The after-perception hook. On a ping it queues the alert (void_cyan_gimmick.gd:26-27): every other active hunter
    /// investigates the runner's position.
    pub fn after(&mut self, b: &Brain, dt: f32, t: &Target, ev: &mut Vec<Event>) {
        if self.tick(dt, sighted(b.direct_visible, t)) {
            ev.push(Event::FacilityAlert {
                at: t.pos,
                except_self: true,
            });
            ev.push(Event::Message(PING_MESSAGE.into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::gimmicks::silence_callouts;

    fn runner() -> Target {
        Target::default()
    }

    /// Ticks `n` times at `dt` with the given sight and returns the 1-based tick numbers that fired.
    fn fire_ticks(g: &mut VoidCyan, dt: f32, sight: bool, n: usize) -> Vec<usize> {
        let mut fired = Vec::new();
        for i in 1..=n {
            if g.tick(dt, sight) {
                fired.push(i);
            }
        }
        fired
    }

    #[test]
    fn sight_rule_matches_godot() {
        let open = runner();
        assert!(sighted(true, &open));
        assert!(!sighted(false, &open), "no direct sight, no ping");
        let hidden = Target { hidden: true, ..runner() };
        assert!(!sighted(true, &hidden), "a hidden runner is never sighted");
        let safe = Target { in_safe_zone: true, ..runner() };
        assert!(!sighted(true, &safe), "a runner in a safe zone is never sighted");
    }

    #[test]
    fn pings_after_exactly_two_and_a_half_seconds() {
        let mut g = VoidCyan::default();
        // 0.5 s steps are exact in f32, so the fifth step lands on 2.5 s and fires.
        assert_eq!(fire_ticks(&mut g, 0.5, true, 5), vec![5]);
        assert_eq!(g.sight_built(), 0.0, "the sight clock resets when the ping fires");
        assert_eq!(g.cooldown_left(), INTERVAL);
    }

    #[test]
    fn a_break_in_sight_resets_the_clock() {
        let mut g = VoidCyan::default();
        assert!(fire_ticks(&mut g, 2.0, true, 1).is_empty());
        assert!(fire_ticks(&mut g, 0.1, false, 1).is_empty());
        assert_eq!(g.sight_built(), 0.0, "one unsighted tick wipes the 2.0 s already built");
        // a fresh 2.0 s is not enough; a further 0.5 s is
        assert!(fire_ticks(&mut g, 2.0, true, 1).is_empty());
        assert_eq!(fire_ticks(&mut g, 0.5, true, 1), vec![1]);
    }

    #[test]
    fn hidden_or_safe_runner_resets_sight() {
        let mut g = VoidCyan::default();
        g.tick(2.0, true);
        let hidden = Target { hidden: true, ..runner() };
        g.tick(0.1, sighted(true, &hidden));
        assert_eq!(g.sight_built(), 0.0);
    }

    #[test]
    fn cooldown_is_thirty_seconds_and_sight_does_not_count_during_it() {
        let mut g = VoidCyan::default();
        assert_eq!(fire_ticks(&mut g, 2.5, true, 1), vec![1]);
        // 30 one-second ticks of unbroken sight: the cooldown eats all of them, nothing fires
        assert!(fire_ticks(&mut g, 1.0, true, 29).is_empty());
        assert_eq!(g.cooldown_left(), 1.0, "29 of the 30 s have run");
        // the 30th second ends the cooldown; that same tick is the first to build sight (1.0 s)
        assert!(fire_ticks(&mut g, 1.0, true, 1).is_empty());
        assert_eq!(g.cooldown_left(), 0.0);
        assert_eq!(g.sight_built(), 1.0, "only the post-cooldown tick counts, cooldown ticks build none");
        // the ping then needs 2.5 s of sight again: 2.0 s after this point, the second tick fires
        assert_eq!(fire_ticks(&mut g, 1.0, true, 2), vec![2]);
    }

    #[test]
    fn cooldown_is_rearmed_after_each_ping() {
        let mut g = VoidCyan::default();
        assert_eq!(fire_ticks(&mut g, 2.5, true, 1), vec![1]);
        // run the cooldown out, then ping again with another full 2.5 s of sight
        assert!(fire_ticks(&mut g, 30.0, false, 1).is_empty());
        assert_eq!(fire_ticks(&mut g, 2.5, true, 1), vec![1]);
    }

    #[test]
    fn ping_queues_facility_alert_then_silenced_callout() {
        let mut ev = Vec::new();
        let mut g = VoidCyan::default();
        // emulate the ping branch of `after` without a Brain: same tick + the two pushes
        if g.tick(2.5, true) {
            ev.push(Event::FacilityAlert { at: (3.0, 4.0), except_self: true });
            ev.push(Event::Message(PING_MESSAGE.into()));
        }
        silence_callouts(&mut ev);
        assert_eq!(ev.len(), 1, "the callout is dropped while ANNOUNCE_HUNTER_EVENTS is off");
        assert!(matches!(ev[0], Event::FacilityAlert { at: (x, y), except_self: true } if x == 3.0 && y == 4.0));
    }
}
