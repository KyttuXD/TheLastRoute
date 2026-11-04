//! The player side of the void hunters' powers (game/player/void_player_effects.gd): a system outage (Void Orange),
//! radar static or blackout (Void Purple) and Void Pink's fake footsteps. The gimmicks only emit events
//! (hunters/gimmicks/void/*.rs); the timers and the HUD state live here.

use super::{Game, Heater};
use crate::audio::cues::rules::fake_step_volume;
use crate::audio::sound;
use crate::hunters::gimmicks::void::void_orange::{Outage, System};
use crate::hunters::gimmicks::void::void_pink::FakeFootsteps;
use crate::hunters::gimmicks::void::void_purple::{RadarCorrupt, RadarMode};

/// void_player_effects.gd:42 `FAKE_STEP_LEAD`: the first fake step lands this long after the event.
const FAKE_STEP_LEAD: f32 = 0.12;
/// void_player_effects.gd:43 `FAKE_STEPS_MAX`.
const FAKE_STEPS_MAX: u32 = 8;

/// One queued fake step (void_player_effects.gd `_fake_steps`): seconds until it plays, pan and volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingStep {
    pub left: f32,
    pub pan: f32,
    pub volume: f32,
}

/// The timed effects a player is under right now.
#[derive(Default)]
pub struct VoidEffects {
    /// The offline system and the seconds left (player.gd `outage_system` / `outage_left`).
    pub outage: Option<(System, f32)>,
    /// The radar state and the seconds left (hud.set_radar_state / tick_radar_state).
    pub radar: Option<(RadarMode, f32)>,
    /// The queued fake footsteps, one run at a time.
    pub steps: Vec<PendingStep>,
}

/// void_player_effects.gd `fake_footstep_plan`: the steps of one run as (seconds from the start, pan, volume). An approach
/// closes from `distance` to about two thirds of it and drifts toward the centre; otherwise the walker moves off and drifts
/// out to the side.
pub fn fake_footstep_plan(f: &FakeFootsteps) -> Vec<PendingStep> {
    let steps = f.steps.clamp(1, FAKE_STEPS_MAX);
    let gap = f.gap.clamp(0.25, 1.0);
    let pan0 = f.pan.clamp(-1.0, 1.0);
    let distance0 = f.distance.clamp(1.0, 12.0);
    let side = if pan0 < 0.0 { -1.0 } else { 1.0 };
    let last = (steps - 1).max(1) as f32;
    (0..steps)
        .map(|i| {
            let t = i as f32 / last;
            let distance = if f.approach {
                lerp(distance0, distance0 * 0.65, t)
            } else {
                lerp(distance0, distance0 * 1.3, t)
            };
            let pan = if f.approach {
                lerp(pan0, pan0 * 0.3, t)
            } else {
                lerp(pan0, side * 0.9, t * 0.6)
            };
            PendingStep {
                left: FAKE_STEP_LEAD + i as f32 * gap,
                pan: pan.clamp(-1.0, 1.0),
                volume: fake_step_volume(distance as f64) as f32,
            }
        })
        .collect()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl VoidEffects {
    /// True while `system` is offline (player.gd `system_offline`).
    pub fn offline(&self, system: System) -> bool {
        matches!(self.outage, Some((s, left)) if s == system && left > 0.0)
    }

    /// The radar is blanked: an offline radar, or a blackout from Void Purple.
    pub fn radar_dark(&self) -> bool {
        self.offline(System::Radar) || matches!(self.radar, Some((RadarMode::Blackout, left)) if left > 0.0)
    }

    /// Radar static is on screen (Void Purple).
    pub fn radar_static(&self) -> bool {
        matches!(self.radar, Some((RadarMode::Static, left)) if left > 0.0)
    }

    /// Advances the timers and returns the fake steps that fall due this frame (void_player_effects.gd `_tick_fake_footsteps`).
    pub fn tick(&mut self, dt: f32) -> Vec<PendingStep> {
        self.outage = self.outage.and_then(|(s, left)| (left - dt > 0.0).then_some((s, left - dt)));
        self.radar = self.radar.and_then(|(m, left)| (left - dt > 0.0).then_some((m, left - dt)));
        for s in &mut self.steps {
            s.left -= dt;
        }
        let mut due = vec![];
        while self.steps.first().is_some_and(|s| s.left <= 0.0) {
            due.push(self.steps.remove(0));
        }
        due
    }
}

impl Game {
    /// Runs the void effects' timers and plays the fake steps that fall due (called once per frame).
    pub(super) fn tick_void_effects(&mut self, dt: f32) {
        for step in self.void_fx.tick(dt) {
            sound::hunter_footstep(step.pan, step.volume);
        }
    }

    /// Godot player.begin_outage + the `system_outage` event (void_player_effects.gd:64-70). The alarm plays for every
    /// system; the flashlight is forced off and cannot be switched on again until the outage ends.
    pub(super) fn begin_system_outage(&mut self, o: Outage) {
        if o.seconds > 0.0 {
            self.void_fx.outage = Some((o.system, o.seconds));
        }
        if o.system == System::Flashlight {
            self.p.flashlight_on = false;
        }
        sound::system_offline();
    }

    /// The `radar_corrupt` event (void_player_effects.gd:72-79): static or blackout for `seconds`, with the radar static cue.
    pub(super) fn begin_radar_corrupt(&mut self, r: RadarCorrupt) {
        if r.seconds > 0.0 {
            self.void_fx.radar = Some((r.mode, r.seconds));
        }
        sound::radar_static();
    }

    /// The `fake_footsteps` event (void_player_effects.gd `_begin_fake_footsteps`): one run at a time.
    pub(super) fn queue_fake_footsteps(&mut self, f: &FakeFootsteps) {
        if self.void_fx.steps.is_empty() {
            self.void_fx.steps = fake_footstep_plan(f);
        }
    }

    /// Climate: the heater runs only while it is powered and the climate system is not offline (Void Orange).
    pub fn heater_running(&self) -> bool {
        self.heater == Heater::Powered && !self.void_fx.offline(System::Climate)
    }

    /// The minimap and the full map are blanked (Void Orange radar outage, Void Purple blackout).
    pub fn radar_dark(&self) -> bool {
        self.void_fx.radar_dark()
    }

    /// Radar static is shown over the maps (Void Purple).
    pub fn radar_static(&self) -> bool {
        self.void_fx.radar_static()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(steps: u32, approach: bool) -> FakeFootsteps {
        FakeFootsteps { steps, pan: 0.5, gap: 0.4, distance: 6.0, approach }
    }

    #[test]
    fn outage_counts_down_and_clears() {
        let mut v = VoidEffects::default();
        v.outage = Some((System::Climate, 1.0));
        assert!(v.offline(System::Climate));
        assert!(!v.offline(System::Radar));
        v.tick(0.6);
        assert!(v.offline(System::Climate));
        v.tick(0.6);
        assert!(v.outage.is_none());
    }

    #[test]
    fn radar_outage_and_blackout_darken_the_map_static_does_not() {
        let mut v = VoidEffects::default();
        v.radar = Some((RadarMode::Static, 2.0));
        assert!(v.radar_static() && !v.radar_dark());
        v.radar = Some((RadarMode::Blackout, 2.0));
        assert!(v.radar_dark() && !v.radar_static());
        v.radar = None;
        v.outage = Some((System::Radar, 3.0));
        assert!(v.radar_dark());
    }

    #[test]
    fn fake_steps_fall_due_in_order_and_one_run_at_a_time() {
        let mut v = VoidEffects::default();
        v.steps = fake_footstep_plan(&run(4, false));
        assert_eq!(v.steps.len(), 4);
        // first step at 0.12 s, then 0.4 s apart
        assert!(v.tick(0.1).is_empty());
        assert_eq!(v.tick(0.03).len(), 1);
        assert_eq!(v.tick(0.4).len(), 1);
        assert_eq!(v.steps.len(), 2);
    }

    #[test]
    fn approach_closes_in_and_plan_respects_godot_clamps() {
        let plan = fake_footstep_plan(&run(5, true));
        assert_eq!(plan.len(), 5);
        // closing in: the last step is louder than the first
        assert!(plan[4].volume > plan[0].volume);
        // steps clamp to 1..=8, gap to .25..1.0
        let many = fake_footstep_plan(&FakeFootsteps { steps: 99, gap: 9.0, ..run(4, true) });
        assert_eq!(many.len(), FAKE_STEPS_MAX as usize);
        assert!((many[1].left - many[0].left - 1.0).abs() < 1e-5);
        let none = fake_footstep_plan(&FakeFootsteps { steps: 0, ..run(4, true) });
        assert_eq!(none.len(), 1);
    }
}
