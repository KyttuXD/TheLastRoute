//! Orange Void, systemSaboteur (void_orange_gimmick.gd): every 26 s she takes ONE of the runner's systems offline for 9 s:
//! the flashlight (forced off and cannot be switched on), the climate control (the heater goes off) or the radar (the
//! minimap and map go black). The system comes back by itself. The player side applies it (main_3d.gd system_outage ->
//! PlayerController3D.begin_outage, plus the HUD radar overlay; see void_player_effects.gd `system_outage`).
//!
//! Integration: `before` returns the outage that fired this frame, if any. The integrator turns it into a mechanic event
//! for the player side (e.g. `Event::SystemOutage(Outage)`). The Godot world event also carries a "message" and an
//! "alert" sound, but HunterBrain.emit_world_event strips both while ANNOUNCE_HUNTER_EVENTS is off, so none is emitted here.

use crate::hunters::brain::{Brain, Target};

/// void_orange_gimmick.gd:8 (`const INTERVAL := 26.0`).
pub const INTERVAL: f32 = 26.0;
/// void_orange_gimmick.gd:9 (`const OUTAGE_SECONDS := 9.0`).
pub const OUTAGE_SECONDS: f32 = 9.0;

/// The runner's systems she can take offline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum System {
    /// Forced off and cannot be switched on.
    Flashlight,
    /// The heater goes off.
    Climate,
    /// The minimap and map go black.
    Radar,
}

/// void_orange_gimmick.gd:10 (`const SYSTEMS := ["flashlight", "climate", "radar"]`). The index is drawn with
/// `next_random_index(SYSTEMS.size())`, so the order here is part of the seeded behaviour.
pub const SYSTEMS: [System; 3] = [System::Flashlight, System::Climate, System::Radar];

impl System {
    /// The Godot system string (the "system" key of the `system_outage` event).
    pub fn name(self) -> &'static str {
        match self {
            System::Flashlight => "flashlight",
            System::Climate => "climate",
            System::Radar => "radar",
        }
    }
}

/// One outage that fired this frame (the `system_outage` event: "system" and "seconds").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outage {
    pub system: System,
    pub seconds: f32,
}

#[derive(Default)]
pub struct VoidOrange {
    /// The system that is offline right now (void_orange_gimmick.gd `active_system`, "" when none).
    pub active_system: Option<System>,
    /// Seconds since the last outage fired (void_orange_gimmick.gd `_timer`).
    timer: f32,
    /// Seconds left on the current outage (void_orange_gimmick.gd `_left`).
    left: f32,
}

impl VoidOrange {
    /// void_orange_gimmick.gd `before_perception` (lines 17-29). Returns the outage that fired this frame.
    pub fn before(&mut self, b: &mut Brain, dt: f32, t: &Target) -> Option<Outage> {
        // The Godot guard `target.is_empty()` means there is no runner dict this frame. The Rust brain always
        // has a runner target, so the caller passes `true` here. The timer still runs while a target is missing.
        let _ = t;
        self.tick(dt, true, |n| b.next_index(n))
    }

    /// The timing and trigger rule, with the random draw passed in (`pick(n)` returns an index in `0..n`).
    /// Mirrors void_orange_gimmick.gd lines 18-29 in the same order.
    pub fn tick(&mut self, dt: f32, has_target: bool, pick: impl FnOnce(usize) -> usize) -> Option<Outage> {
        // line 18-20: the outage runs down, and the system is back when it ends
        self.left = (self.left - dt).max(0.0);
        if self.left <= 0.0 {
            self.active_system = None;
        }
        // line 21: the interval clock always runs, even while the outage is on
        self.timer += dt;
        // line 22: not before the interval, and never with no runner
        if self.timer < INTERVAL || !has_target {
            return None;
        }
        // line 24-27: reset the clock, draw the system, take it offline
        self.timer = 0.0;
        let system = SYSTEMS[pick(SYSTEMS.len())];
        self.active_system = Some(system);
        self.left = OUTAGE_SECONDS;
        // line 28-29: the event the player side applies
        Some(Outage {
            system,
            seconds: OUTAGE_SECONDS,
        })
    }

    /// void_orange_gimmick.gd `network_state` (line 32-33): the offline system, for the networked state.
    pub fn network_state(&self) -> Option<System> {
        self.active_system
    }

    /// void_orange_gimmick.gd `apply_network_state` (line 36-37). Sets only the system; the remaining time is not sent.
    pub fn apply_network_state(&mut self, outage: Option<System>) {
        self.active_system = outage;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `frames` ticks of `dt` seconds with a runner present and the draw fixed to `index`. Returns each fired outage
    /// with the frame it fired on.
    fn run(g: &mut VoidOrange, frames: usize, dt: f32, index: usize) -> Vec<(usize, Outage)> {
        let mut fired = vec![];
        for i in 0..frames {
            if let Some(o) = g.tick(dt, true, |n| {
                assert_eq!(n, SYSTEMS.len(), "the draw is over the three systems");
                index
            }) {
                fired.push((i, o));
            }
        }
        fired
    }

    #[test]
    fn the_first_outage_waits_the_full_interval() {
        let mut g = VoidOrange::default();
        // 25 one-second frames: 25 s, still under INTERVAL (26 s)
        assert!(run(&mut g, 25, 1.0, 0).is_empty(), "no outage before 26 s");
        assert_eq!(g.active_system, None);
        // the 26th second fires it
        let fired = run(&mut g, 1, 1.0, 0);
        assert_eq!(fired.len(), 1, "fires at 26 s");
        assert_eq!(fired[0].1.seconds, OUTAGE_SECONDS);
    }

    #[test]
    fn the_draw_picks_flashlight_climate_or_radar_in_godot_order() {
        assert_eq!(SYSTEMS, [System::Flashlight, System::Climate, System::Radar]);
        assert_eq!(SYSTEMS[0].name(), "flashlight");
        assert_eq!(SYSTEMS[1].name(), "climate");
        assert_eq!(SYSTEMS[2].name(), "radar");
        for (index, want) in SYSTEMS.iter().enumerate() {
            let mut g = VoidOrange::default();
            let fired = run(&mut g, 26, 1.0, index);
            assert_eq!(fired[0].1.system, *want, "index {index} picks {want:?}");
            assert_eq!(g.active_system, Some(*want));
        }
    }

    #[test]
    fn the_outage_lasts_nine_seconds_then_the_system_returns() {
        let mut g = VoidOrange::default();
        run(&mut g, 26, 1.0, 2);
        assert_eq!(g.active_system, Some(System::Radar), "offline the moment it fires");
        // 8 seconds in: still out
        run(&mut g, 8, 1.0, 2);
        assert_eq!(g.active_system, Some(System::Radar), "still offline at 8 s of 9 s");
        // the 9th second ends it
        run(&mut g, 1, 1.0, 2);
        assert_eq!(g.active_system, None, "back at 9 s");
    }

    #[test]
    fn the_interval_clock_keeps_running_through_the_outage() {
        let mut g = VoidOrange::default();
        run(&mut g, 26, 1.0, 0);
        // the next outage is 26 s after the last one fired, not 26 s after the system came back
        let fired = run(&mut g, 25, 1.0, 1);
        assert!(fired.is_empty(), "the next one is not due before 26 s from the last");
        let fired = run(&mut g, 1, 1.0, 1);
        assert_eq!(fired.len(), 1, "due at 26 s from the last");
        assert_eq!(fired[0].1.system, System::Climate);
    }

    #[test]
    fn a_missing_runner_holds_the_clock_and_the_outage_fires_on_return() {
        let mut g = VoidOrange::default();
        // 40 s with no runner: the timer runs past the interval but nothing fires
        for _ in 0..40 {
            assert!(g.tick(1.0, false, |_| panic!("no draw without a runner")).is_none());
        }
        // the runner comes back: the overdue outage fires on the first frame
        let o = g.tick(1.0, true, |_| 0).expect("fires as soon as there is a runner");
        assert_eq!(o.system, System::Flashlight);
    }

    #[test]
    fn network_state_round_trips_the_offline_system() {
        let mut g = VoidOrange::default();
        run(&mut g, 26, 1.0, 1);
        assert_eq!(g.network_state(), Some(System::Climate));
        let mut peer = VoidOrange::default();
        peer.apply_network_state(g.network_state());
        assert_eq!(peer.active_system, Some(System::Climate));
        peer.apply_network_state(None);
        assert_eq!(peer.active_system, None);
    }
}
