//! Green Void: vent relocation (void_green_gimmick.gd, ventRelocate). Every INTERVAL seconds, if she is at least
//! MINIMUM_DISTANCE cells from the runner and the runner is not in a safe room, she submerges for SUBMERGE_SECONDS
//! (hidden, still, uncatchable) and surfaces DROP_DISTANCE cells from the runner in a random direction, investigating.
//! A death or restart rearms her full interval (`on_runner_respawn`).
//!
//! Ported from void_green_gimmick.gd and the chart in void_hunter_gimmicks/void_hunter_gimmick_chart.gd (the "void_green"
//! entry, which says the same numbers in player-facing text).

use crate::config::HUNTER_RADIUS;
use crate::hunters::brain::{dist, Brain, State, Target};
use crate::hunters::gimmicks::{Ctx, Event};
use std::f32::consts::TAU;

/// void_green_gimmick.gd:10. The cooldown between dives, in seconds.
pub const INTERVAL: f32 = 22.0;
/// void_green_gimmick.gd:11. She only dives when the runner is at least this far away, in cells.
pub const MINIMUM_DISTANCE: f32 = 14.0;
/// void_green_gimmick.gd:12. She surfaces this far from the runner, in cells.
pub const DROP_DISTANCE: f32 = 9.0;
/// void_green_gimmick.gd:13. How long she stays submerged, in seconds.
pub const SUBMERGE_SECONDS: f32 = 2.0;
/// void_green_gimmick.gd:34. The announcement callout on surfacing. The shared silencer drops it, as in Godot.
pub const VENT_CLATTER: &str = "A VENT GRATE CLATTERS NEARBY";

/// Green Void's state (void_green_gimmick.gd `_cooldown`, `submerged`).
pub struct VoidGreen {
    /// Seconds until she may dive again (`_cooldown`). Starts at INTERVAL.
    cooldown: f32,
    /// Seconds left submerged (`submerged`). Above zero while she is hidden, still and uncatchable.
    pub submerged: f32,
}

impl Default for VoidGreen {
    fn default() -> Self {
        VoidGreen {
            cooldown: INTERVAL,
            submerged: 0.0,
        }
    }
}

impl VoidGreen {
    /// void_green_gimmick.gd `after_perception` (lines 19-43), run after perception and before movement.
    pub fn after(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        if self.submerged > 0.0 {
            if self.tick_submerged(dt) {
                self.surface(b, c, t, ev);
            }
            return;
        }
        self.tick_cooldown(dt, t.in_safe_zone, dist(b.pos, t.pos));
    }

    /// Counts the submerged time down (void_green_gimmick.gd:20-22). Returns true on the tick she surfaces
    /// (`submerged` is no longer above zero).
    fn tick_submerged(&mut self, dt: f32) -> bool {
        self.submerged -= dt;
        self.submerged <= 0.0
    }

    /// The dive trigger (void_green_gimmick.gd:35-42). The cooldown runs down every tick. The dive starts only when it
    /// has run out, the runner is not in a safe room (line 36) and the runner is at least MINIMUM_DISTANCE away
    /// (lines 39-40). Returns true on the tick she submerges.
    fn tick_cooldown(&mut self, dt: f32, in_safe_zone: bool, distance: f32) -> bool {
        self.cooldown -= dt;
        if self.cooldown > 0.0 || in_safe_zone || distance < MINIMUM_DISTANCE {
            return false;
        }
        self.cooldown = INTERVAL;
        self.submerged = SUBMERGE_SECONDS;
        true
    }

    /// The surfacing (void_green_gimmick.gd:23-34): a random bearing from the runner at DROP_DISTANCE, moved to the
    /// nearest open spot there. With no open spot she stays where she is. She then investigates the runner's last
    /// position.
    fn surface(&mut self, b: &mut Brain, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        let at = t.pos;
        let angle = b.next_float() * TAU;
        let wanted = (
            at.0 + angle.cos() * DROP_DISTANCE,
            at.1 + angle.sin() * DROP_DISTANCE,
        );
        if let Some(open) = c.fac.nearest_open(wanted.0, wanted.1, HUNTER_RADIUS) {
            b.pos = open;
        }
        // Godot sets state, has_last_known and last_known directly (no path clear), so this does too.
        b.state = State::Investigate;
        b.last_known = Some(at);
        ev.push(Event::Message(VENT_CLATTER.into()));
    }

    /// The runner restarted after a death (void_green_gimmick.gd:45-47): forget the ended run and rearm the full interval.
    pub fn on_runner_respawn(&mut self) {
        self.submerged = 0.0;
        self.cooldown = INTERVAL;
    }

    /// Pinned in place while submerged (void_green_gimmick.gd:50-51).
    pub fn holds_position(&self) -> bool {
        self.submerged > 0.0
    }

    /// Drawn hidden while submerged (void_green_gimmick.gd:54-55).
    pub fn hidden_from_view(&self) -> bool {
        self.submerged > 0.0
    }

    /// No grab can land while submerged (void_green_gimmick.gd:58-59).
    pub fn immune_to_capture(&self) -> bool {
        self.submerged > 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks at 1/60 s until the cooldown has run out. Returns how many ticks passed when she dives (no safe room,
    /// runner far away throughout).
    fn ticks_until_dive(v: &mut VoidGreen) -> usize {
        let dt = 1.0 / 60.0;
        for n in 1..=100_000 {
            if v.tick_cooldown(dt, false, MINIMUM_DISTANCE + 5.0) {
                return n;
            }
        }
        panic!("she never dove");
    }

    #[test]
    fn constants_match_the_godot_gimmick() {
        assert_eq!(INTERVAL, 22.0);
        assert_eq!(MINIMUM_DISTANCE, 14.0);
        assert_eq!(DROP_DISTANCE, 9.0);
        assert_eq!(SUBMERGE_SECONDS, 2.0);
    }

    #[test]
    fn she_starts_with_a_full_cooldown_and_dives_after_the_interval() {
        let mut v = VoidGreen::default();
        assert_eq!(v.cooldown, INTERVAL);
        assert!(!v.holds_position() && !v.hidden_from_view() && !v.immune_to_capture());
        let ticks = ticks_until_dive(&mut v);
        let elapsed = ticks as f32 / 60.0;
        assert!((elapsed - INTERVAL).abs() < 2.0 / 60.0, "dove after {elapsed} s");
        assert!(v.submerged > 0.0, "the dive starts the submerge");
        assert_eq!(v.cooldown, INTERVAL, "the cooldown is rearmed on the dive");
    }

    #[test]
    fn a_dive_waits_while_the_runner_is_inside_the_minimum_distance() {
        let mut v = VoidGreen::default();
        // run the cooldown out with the runner 13.9 cells away: nothing happens, and the cooldown stays spent
        for _ in 0..(INTERVAL as usize * 60 + 120) {
            assert!(!v.tick_cooldown(1.0 / 60.0, false, MINIMUM_DISTANCE - 0.1));
        }
        assert!(v.cooldown <= 0.0, "the timer keeps running down");
        assert_eq!(v.submerged, 0.0);
        // the first tick at 14 cells, she dives at once
        assert!(v.tick_cooldown(1.0 / 60.0, false, MINIMUM_DISTANCE));
    }

    #[test]
    fn a_runner_in_a_safe_room_is_never_vented_toward() {
        let mut v = VoidGreen::default();
        for _ in 0..(INTERVAL as usize * 60 * 3) {
            assert!(!v.tick_cooldown(1.0 / 60.0, true, 50.0));
        }
        assert_eq!(v.submerged, 0.0);
        // once the runner leaves the safe room she dives straight away, the timer is long spent
        assert!(v.tick_cooldown(1.0 / 60.0, false, 50.0));
    }

    #[test]
    fn submerged_for_two_seconds_then_surfaces_on_that_tick() {
        let mut v = VoidGreen { cooldown: 0.0, submerged: SUBMERGE_SECONDS };
        let dt = 1.0 / 60.0;
        let mut surfaced_at = None;
        for n in 1..=200 {
            if v.tick_submerged(dt) {
                surfaced_at = Some(n);
                break;
            }
        }
        let n = surfaced_at.expect("she surfaces");
        let t = n as f32 * dt;
        assert!((t - SUBMERGE_SECONDS).abs() < 2.0 * dt, "surfaced after {t} s");
        assert!(!v.holds_position() && !v.hidden_from_view() && !v.immune_to_capture(), "visible and catchable again");
    }

    #[test]
    fn the_submerge_shields_her_until_she_surfaces() {
        let v = VoidGreen { cooldown: INTERVAL, submerged: SUBMERGE_SECONDS };
        assert!(v.holds_position());
        assert!(v.hidden_from_view());
        assert!(v.immune_to_capture());
    }

    #[test]
    fn respawn_forgets_the_run_and_rearms_the_full_interval() {
        let mut v = VoidGreen { cooldown: 0.0, submerged: 1.5 };
        v.on_runner_respawn();
        assert_eq!(v.submerged, 0.0);
        assert_eq!(v.cooldown, INTERVAL);
        // a runner who respawns far away gets the full interval again, no instant dive
        assert!(!v.tick_cooldown(1.0, false, 60.0));
    }

    #[test]
    fn the_surfacing_message_is_the_godot_callout() {
        assert_eq!(VENT_CLATTER, "A VENT GRATE CLATTERS NEARBY");
    }
}
