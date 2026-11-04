//! Purple: radar corrupter (void_purple_gimmick.gd, voids/purple.js). Every INTERVAL seconds she corrupts the
//! radar into static for STATIC_SECONDS; each corruption has a BLACKOUT_CHANCE to black the radar out
//! completely for BLACKOUT_SECONDS instead. The runner's radar overlay is the player side (main_3d's
//! `radar_corrupt` handling); this struct only keeps the timers and reports each corruption.
//!
//! Not wired yet: the integrator adds `pub mod void;` to hunters/gimmicks/mod.rs, a `Gimmick::VoidPurple`
//! variant, `"void_purple" => Gimmick::VoidPurple(Default::default())` in `for_id`, and calls
//! `before` from `before_perception` (see the report for the exact signature).

use crate::hunters::brain::Brain;

/// void_purple_gimmick.gd:8: seconds between corruption rolls (the clock runs only while a runner exists).
pub const INTERVAL: f32 = 21.0;
/// void_purple_gimmick.gd:9: seconds of static after a corruption that is not a blackout.
pub const STATIC_SECONDS: f32 = 8.0;
/// void_purple_gimmick.gd:10: chance that a corruption is a full blackout (a roll strictly below it).
pub const BLACKOUT_CHANCE: f32 = 0.35;
/// void_purple_gimmick.gd:11: seconds of total blackout.
pub const BLACKOUT_SECONDS: f32 = 11.0;

/// The radar state the gimmick holds (void_purple_gimmick.gd `mode`: "static" or "blackout", "" when the radar works).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadarMode {
    Static,
    Blackout,
}

/// One corruption as the game receives it. Godot `emit_world_event({what: "radar_corrupt", mode, seconds, sound, message})`
/// (void_purple_gimmick.gd:29-30, 34-35) passes through `HunterBrain.emit_world_event` with ANNOUNCE_HUNTER_EVENTS off
/// (hunter_brain.gd:88-94), which strips the "message" and the "alert" sound. So only `mode` and `seconds` reach the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadarCorrupt {
    pub mode: RadarMode,
    pub seconds: f32,
}

#[derive(Default)]
pub struct VoidPurple {
    /// The radar state now (None: the radar works). Godot `mode`.
    pub mode: Option<RadarMode>,
    /// Counts up to INTERVAL (void_purple_gimmick.gd `_timer`). Reset to 0 on a roll, not carried over.
    timer: f32,
    /// Seconds left in the current corruption (void_purple_gimmick.gd `_left`).
    left: f32,
}

impl VoidPurple {
    /// void_purple_gimmick.gd before_perception. `has_target` is Godot's `not target.is_empty()`: with no runner the
    /// clock keeps running but no roll is made, so the first roll comes on the first tick with a runner after INTERVAL.
    /// Called only while the hunter is active (Godot hunter_brain.gd:272-273; Purple does not override runs_while_inactive).
    pub fn before(&mut self, b: &mut Brain, dt: f32, has_target: bool) -> Option<RadarCorrupt> {
        self.step(dt, has_target, || b.next_float())
    }

    /// The timing and the roll, with the random draw passed in so the rule can be tested without a Brain.
    /// The draw is taken only when a roll happens (Godot calls `brain.next_random_float()` only then).
    fn step(&mut self, dt: f32, has_target: bool, mut roll: impl FnMut() -> f32) -> Option<RadarCorrupt> {
        self.left = (self.left - dt).max(0.0);
        if self.left <= 0.0 {
            self.mode = None;
        }
        self.timer += dt;
        if self.timer < INTERVAL || !has_target {
            return None;
        }
        self.timer = 0.0;
        if roll() < BLACKOUT_CHANCE {
            self.mode = Some(RadarMode::Blackout);
            self.left = BLACKOUT_SECONDS;
            Some(RadarCorrupt { mode: RadarMode::Blackout, seconds: BLACKOUT_SECONDS })
        } else {
            self.mode = Some(RadarMode::Static);
            self.left = STATIC_SECONDS;
            Some(RadarCorrupt { mode: RadarMode::Static, seconds: STATIC_SECONDS })
        }
    }

    /// void_purple_gimmick.gd network_state: {"radar": mode}, with "" for a working radar.
    pub fn network_state(&self) -> &'static str {
        match self.mode {
            None => "",
            Some(RadarMode::Static) => "static",
            Some(RadarMode::Blackout) => "blackout",
        }
    }

    /// void_purple_gimmick.gd apply_network_state: any unknown value reads as a working radar.
    pub fn apply_network_state(&mut self, radar: &str) {
        self.mode = match radar {
            "static" => Some(RadarMode::Static),
            "blackout" => Some(RadarMode::Blackout),
            _ => None,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `ticks` steps of `dt` with a runner present and a fixed roll, collecting every corruption.
    fn run(g: &mut VoidPurple, dt: f32, ticks: usize, roll_value: f32) -> Vec<(usize, RadarCorrupt)> {
        let mut out = vec![];
        for i in 0..ticks {
            if let Some(c) = g.step(dt, true, || roll_value) {
                out.push((i, c));
            }
        }
        out
    }

    #[test]
    fn the_first_roll_comes_at_the_interval_not_before() {
        let mut g = VoidPurple::default();
        // 0.5 s ticks are exact in f32: 41 ticks is 20.5 s (no roll), the 42nd is 21.0 s (a roll).
        let fired = run(&mut g, 0.5, 42, 0.99);
        assert_eq!(fired.len(), 1, "one roll in 21 s");
        assert_eq!(fired[0].0, 41, "the roll is on the 42nd half-second tick");
        assert_eq!(fired[0].1, RadarCorrupt { mode: RadarMode::Static, seconds: STATIC_SECONDS });
    }

    #[test]
    fn the_clock_restarts_from_zero_after_each_roll() {
        let mut g = VoidPurple::default();
        // one-second ticks: a roll at tick 21, then the next only 21 ticks later (tick 42), no overrun carried over
        let fired = run(&mut g, 1.0, 63, 0.99);
        let ticks: Vec<usize> = fired.iter().map(|(i, _)| *i).collect();
        assert_eq!(ticks, vec![20, 41, 62]);
    }

    #[test]
    fn a_roll_below_the_blackout_chance_blacks_the_radar_out_for_eleven_seconds() {
        assert_eq!(BLACKOUT_CHANCE, 0.35, "void_purple_gimmick.gd:10");
        let mut g = VoidPurple::default();
        let c = g.step(INTERVAL, true, || 0.349).expect("rolls at the interval");
        assert_eq!(c, RadarCorrupt { mode: RadarMode::Blackout, seconds: 11.0 });
        assert_eq!(g.mode, Some(RadarMode::Blackout));
    }

    #[test]
    fn a_roll_at_or_above_the_chance_is_only_static() {
        // the chance is a strict "<": a roll of exactly 0.35 is static
        for roll in [0.35_f32, 0.9] {
            let mut g = VoidPurple::default();
            let c = g.step(INTERVAL, true, || roll).expect("rolls at the interval");
            assert_eq!(c, RadarCorrupt { mode: RadarMode::Static, seconds: 8.0 }, "roll {roll}");
        }
    }

    #[test]
    fn the_radar_returns_on_its_own_when_the_corruption_runs_out() {
        let mut g = VoidPurple::default();
        g.step(INTERVAL, true, || 0.9);
        assert_eq!(g.mode, Some(RadarMode::Static));
        // 7.5 s of an 8 s static: still corrupted
        for _ in 0..15 {
            g.step(0.5, true, || 0.9);
        }
        assert_eq!(g.mode, Some(RadarMode::Static), "7.5 s in, still static");
        g.step(0.5, true, || 0.9);
        assert_eq!(g.mode, None, "8 s in, the radar is back");

        // a blackout lasts 11 s
        let mut g = VoidPurple::default();
        g.step(INTERVAL, true, || 0.0);
        for _ in 0..21 {
            g.step(0.5, true, || 0.9);
        }
        assert_eq!(g.mode, Some(RadarMode::Blackout), "10.5 s in, still black");
        g.step(0.5, true, || 0.9);
        assert_eq!(g.mode, None, "11 s in, the radar is back");
    }

    #[test]
    fn with_no_runner_the_clock_runs_but_no_roll_is_made() {
        let mut g = VoidPurple::default();
        let mut draws = 0;
        for _ in 0..30 {
            assert!(g.step(1.0, false, || {
                draws += 1;
                0.0
            })
            .is_none());
        }
        assert_eq!(draws, 0, "no runner, no draw");
        // the first tick with a runner is past the interval, so it rolls at once
        let c = g.step(1.0, true, || 0.9);
        assert_eq!(c, Some(RadarCorrupt { mode: RadarMode::Static, seconds: 8.0 }));
    }

    #[test]
    fn the_network_state_round_trips_the_radar_mode() {
        let mut g = VoidPurple::default();
        assert_eq!(g.network_state(), "");
        g.apply_network_state("blackout");
        assert_eq!(g.mode, Some(RadarMode::Blackout));
        assert_eq!(g.network_state(), "blackout");
        g.apply_network_state("static");
        assert_eq!(g.network_state(), "static");
        g.apply_network_state("");
        assert_eq!(g.mode, None);
        g.apply_network_state("garbage");
        assert_eq!(g.mode, None);
    }
}
