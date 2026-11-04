//! Pink Void (`void_pink`), the falseSignal gimmick: she never attacks and cannot be caught. Every 11 s she has a 60 %
//! chance to make the runner hear a few false footsteps somewhere close. The sound is audio only: no text, no HUD, no
//! alert stinger. Port of game/hunters/gimmicks/void_pink_gimmick.gd. The Godot chart (void_hunter_gimmick_chart.gd,
//! `VoidHunterGimmickChart.ENTRIES["void_pink"]`) repeats the same numbers.
//!
//! The Godot gimmick queues one mechanic event, `fake_footsteps`, through `brain.emit_world_event`. This file only makes the
//! payload (`FakeFootsteps`). The integrator adds `Event::FakeFootsteps(FakeFootsteps)` to gimmicks/mod.rs and pushes it
//! from the `after_perception` dispatch (see the report). Every draw comes from the brain's own stream (`b.next_float()`),
//! in the Godot order: chance, steps, pan, distance, approach, gap.

use crate::hunters::brain::Brain;

/// void_pink_gimmick.gd:14 `INTERVAL := 11.0`: the roll clock, seconds.
pub const INTERVAL: f32 = 11.0;
/// void_pink_gimmick.gd:15 `CHANCE := 0.6`: a roll succeeds when the draw is below this.
pub const CHANCE: f32 = 0.6;
/// void_pink_gimmick.gd:16 `STEPS_MIN := 4`.
pub const STEPS_MIN: u32 = 4;
/// void_pink_gimmick.gd:17 `STEPS_SPAN := 4` (steps = 4..7).
pub const STEPS_SPAN: u32 = 4;
/// void_pink_gimmick.gd:18 `PAN_MAX := 0.8` (pan = -0.8..0.8).
pub const PAN_MAX: f32 = 0.8;
/// void_pink_gimmick.gd:19 `GAP := 0.42` (seconds between steps, centre).
pub const GAP: f32 = 0.42;
/// void_pink_gimmick.gd:20 `GAP_JITTER := 0.08` (gap = 0.38..0.46 s).
pub const GAP_JITTER: f32 = 0.08;
/// void_pink_gimmick.gd:21 `DISTANCE_MIN := 4.0` (metres).
pub const DISTANCE_MIN: f32 = 4.0;
/// void_pink_gimmick.gd:22 `DISTANCE_SPAN := 5.0` (distance = 4..9 m).
pub const DISTANCE_SPAN: f32 = 5.0;
/// void_pink_gimmick.gd:41 `brain.next_random_float() < 0.5`: the footsteps close in (approach).
pub const APPROACH_CHANCE: f32 = 0.5;

/// The `fake_footsteps` payload (void_pink_gimmick.gd:36-43). The Godot `peer_id` and `target_depth`
/// (void_pink_gimmick.gd:44-47) are added by the host to route the event to the targeted player; the Rust `Target`
/// does not carry them yet.
#[derive(Clone, Debug, PartialEq)]
pub struct FakeFootsteps {
    /// 4..=7 footfalls.
    pub steps: u32,
    /// -0.8..=0.8 stereo pan.
    pub pan: f32,
    /// 0.38..0.46 s between steps.
    pub gap: f32,
    /// 4..9 m away.
    pub distance: f32,
    /// True when the steps close in on the runner.
    pub approach: bool,
}

/// Pink's gimmick state: only the roll clock (void_pink_gimmick.gd `_cooldown`, which starts at INTERVAL).
pub struct VoidPink {
    cooldown: f32,
}

impl Default for VoidPink {
    fn default() -> Self {
        VoidPink { cooldown: INTERVAL }
    }
}

impl VoidPink {
    /// `after_perception` (void_pink_gimmick.gd:27-48) on the brain's own stream. The Rust `Target` is always present, so
    /// the Godot `target.is_empty()` guard is always passed here and this always rolls.
    pub fn after(&mut self, b: &mut Brain, dt: f32) -> Option<FakeFootsteps> {
        self.roll(dt, true, &mut || b.next_float())
    }

    /// The roll itself, with the draws passed in so tests can script them. `has_target` is Godot's `not target.is_empty()`.
    /// The clock runs first and resets on every roll, even one that fails (void_pink_gimmick.gd:28-31). When there is no
    /// target, or the chance draw fails, nothing further is drawn (void_pink_gimmick.gd:32-33).
    pub fn roll(&mut self, dt: f32, has_target: bool, draw: &mut dyn FnMut() -> f32) -> Option<FakeFootsteps> {
        self.cooldown -= dt;
        if self.cooldown > 0.0 {
            return None;
        }
        self.cooldown = INTERVAL;
        if !has_target || draw() >= CHANCE {
            return None;
        }
        // Draw order matches the Godot literal: steps (line 35), pan, distance, approach, gap (lines 38-42).
        let steps = (STEPS_MIN + (draw() * STEPS_SPAN as f32) as u32).clamp(STEPS_MIN, STEPS_MIN + STEPS_SPAN - 1);
        let pan = (draw() * 2.0 - 1.0) * PAN_MAX;
        let distance = DISTANCE_MIN + draw() * DISTANCE_SPAN;
        let approach = draw() < APPROACH_CHANCE;
        let gap = GAP - GAP_JITTER * 0.5 + draw() * GAP_JITTER;
        Some(FakeFootsteps { steps, pan, gap, distance, approach })
    }

    /// Seconds until the next roll (for the view or tests).
    pub fn cooldown(&self) -> f32 {
        self.cooldown
    }
}

/// void_pink_gimmick.gd:51-52 `immune_to_capture() -> true`: she can never catch-lock the runner.
pub fn immune_to_capture() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::rng::Rng;

    #[test]
    fn the_first_roll_comes_on_the_eleventh_second() {
        let mut p = VoidPink::default();
        let mut draw = || 0.0;
        for _ in 0..21 {
            assert!(p.roll(0.5, true, &mut draw).is_none());
        }
        assert!((p.cooldown() - 0.5).abs() < 1e-6, "10.5 s in, half a second left");
        // the 22nd half second is the 11.0 s mark: the roll runs (a 0.0 chance draw passes), so an event comes out
        assert!(p.roll(0.5, true, &mut draw).is_some(), "the roll fires at 11 s");
    }

    #[test]
    fn the_clock_resets_to_the_full_interval_even_when_the_roll_fails() {
        let mut p = VoidPink::default();
        let mut draw = || 0.99;
        for _ in 0..21 {
            p.roll(0.5, true, &mut draw);
        }
        assert!(p.roll(0.5, true, &mut draw).is_none(), "a failed roll gives nothing");
        assert_eq!(p.cooldown(), INTERVAL, "the failed roll still restarts the 11 s clock");
        for _ in 0..21 {
            assert!(p.roll(0.5, true, &mut draw).is_none());
        }
        assert!(p.roll(0.5, true, &mut draw).is_none(), "the next roll is 11 s later, not sooner");
    }

    #[test]
    fn the_chance_is_below_sixty_percent() {
        let mut fired = false;
        let mut p = VoidPink::default();
        let mut draw = || CHANCE - 1e-4;
        for _ in 0..22 {
            fired |= p.roll(0.5, true, &mut draw).is_some();
        }
        assert!(fired, "a draw just under 0.6 passes");
        let mut fired = false;
        let mut p = VoidPink::default();
        let mut draw = || CHANCE;
        for _ in 0..22 {
            fired |= p.roll(0.5, true, &mut draw).is_some();
        }
        assert!(!fired, "a draw of exactly 0.6 fails (Godot's >= CHANCE)");
    }

    #[test]
    fn no_target_means_no_draws_and_no_event() {
        let mut p = VoidPink::default();
        let mut used = 0;
        let mut draw = || {
            used += 1;
            0.0
        };
        for _ in 0..22 {
            assert!(p.roll(0.5, false, &mut draw).is_none());
        }
        assert_eq!(used, 0, "an empty target spends no random draws (void_pink_gimmick.gd:32 short-circuit)");
    }

    #[test]
    fn the_payload_follows_the_godot_draw_order() {
        // chance 0.5 passes; steps 0.0 -> 4; pan 0.5 -> 0.0; distance 0.0 -> 4.0 m; approach 0.4 -> true; gap 0.5 -> 0.42 s
        let mut script = vec![0.5, 0.0, 0.5, 0.0, 0.4, 0.5].into_iter();
        let mut p = VoidPink { cooldown: 0.0 };
        let f = p.roll(0.0, true, &mut || script.next().unwrap()).expect("event");
        assert_eq!(f.steps, 4);
        assert!(f.pan.abs() < 1e-6);
        assert!((f.distance - 4.0).abs() < 1e-6);
        assert!(f.approach);
        assert!((f.gap - GAP).abs() < 1e-6);
        assert!(script.next().is_none(), "exactly six draws after the clock");
    }

    #[test]
    fn the_payload_ranges_hold_at_their_ends() {
        let mut p = VoidPink { cooldown: 0.0 };
        let mut script = vec![0.0, 0.999, 0.999, 0.999, 0.5, 0.0].into_iter();
        let hi = p.roll(0.0, true, &mut || script.next().unwrap()).expect("event");
        assert_eq!(hi.steps, 7, "top draw gives 7 steps");
        assert!((hi.pan - PAN_MAX).abs() < 1e-2);
        assert!((hi.distance - 9.0).abs() < 1e-2, "9 m at the top");
        assert!(!hi.approach, "a draw of 0.5 is not below 0.5");
        assert!((hi.gap - (GAP - GAP_JITTER * 0.5)).abs() < 1e-6, "gap at the bottom of its jitter: {}", hi.gap);
        let mut p = VoidPink { cooldown: 0.0 };
        let mut script = vec![0.0; 6].into_iter();
        let lo = p.roll(0.0, true, &mut || script.next().unwrap()).expect("event");
        assert_eq!(lo.steps, 4);
        assert!((lo.pan + PAN_MAX).abs() < 1e-6);
        assert!(lo.approach);
    }

    #[test]
    fn about_sixty_percent_of_rolls_fire_over_a_long_run() {
        let mut rng = Rng::new("void_pink_rate");
        let mut p = VoidPink::default();
        let mut fired = 0;
        // 8000 s at 1/60 s ticks: about 727 rolls on the 11 s clock
        for _ in 0..(8000 * 60) {
            if p.roll(1.0 / 60.0, true, &mut || rng.next_f32()).is_some() {
                fired += 1;
            }
        }
        let rolls = 8000 / 11;
        let share = fired as f32 / rolls as f32;
        assert!(share > 0.5 && share < 0.7, "fired {fired} of about {rolls}: {share}");
    }

    #[test]
    fn pink_is_immune_to_capture() {
        assert!(immune_to_capture());
    }
}
