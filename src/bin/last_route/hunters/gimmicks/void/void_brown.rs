//! Brown Void (`void_brown`): spotJammer. Every 18 s, up to two hiding spots within 9 cells of
//! the runner are jammed shut for 20 s. The spot the runner occupies is never picked, and a spot
//! already jammed is not picked again (void_brown_gimmick.gd).
//!
//! A jam is `Event::SpotDisabled(index, true)`, applied to `hiding_disabled` by the game
//! (game/hunting.rs), and the expiry is `Event::SpotDisabled(index, false)`. The Godot cue
//! message is `Event::Message`; `silence_callouts` drops it, as Godot's `emit_world_event` does
//! with the announce flag off (ai/hunter_brain.gd:89-91).

use crate::hunters::brain::{dist, Brain, Target};
use crate::hunters::gimmicks::{Ctx, Event};

/// void_brown_gimmick.gd:7.
const INTERVAL: f32 = 18.0;
/// void_brown_gimmick.gd:8.
const JAM_SECONDS: f32 = 20.0;
/// void_brown_gimmick.gd:9. Measured from the hiding spot's cell centre, inclusive.
const RANGE: f32 = 9.0;
/// void_brown_gimmick.gd:10.
const COUNT: usize = 2;

/// Brown Void's spot jammer. `jammed` is Godot's `_jammed`, kept in the same order.
pub struct VoidBrown {
    /// Godot `_cooldown`, starts at INTERVAL (void_brown_gimmick.gd:12).
    cooldown: f32,
    /// (hiding spot index, seconds left) for each spot currently jammed shut.
    jammed: Vec<(usize, f32)>,
}

impl Default for VoidBrown {
    fn default() -> Self {
        Self {
            cooldown: INTERVAL,
            jammed: Vec::new(),
        }
    }
}

impl VoidBrown {
    /// Godot `after_perception` (void_brown_gimmick.gd:16-43): expire jams, then on each due
    /// cooldown jam up to COUNT spots near the runner.
    pub fn after(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        self.expire(dt, ev);
        if !self.tick_cooldown(dt) {
            return;
        }
        let spots = c
            .fac
            .hiding
            .iter()
            .enumerate()
            .map(|(i, h)| (i, h.cell.center()));
        let near = self.candidates(spots, t.pos, t.hiding_spot);
        self.jam(near, |n| b.next_index(n), ev);
    }

    /// Godot lines 16-21: every jam's `left` drops by dt; an expired one is removed and reported
    /// with `disabled: false`.
    fn expire(&mut self, dt: f32, ev: &mut Vec<Event>) {
        self.jammed.retain_mut(|(spot, left)| {
            *left -= dt;
            if *left <= 0.0 {
                ev.push(Event::SpotDisabled(*spot, false));
                false
            } else {
                true
            }
        });
    }

    /// Godot lines 22-25: the cooldown runs down every frame. It fires once it reaches zero and
    /// then restarts at INTERVAL (line 25).
    fn tick_cooldown(&mut self, dt: f32) -> bool {
        self.cooldown -= dt;
        if self.cooldown > 0.0 {
            return false;
        }
        self.cooldown = INTERVAL;
        true
    }

    /// Godot lines 27-34: the spots within RANGE of the runner, skipping the occupied one and any
    /// that are already jammed, in hiding-spot order.
    fn candidates(
        &self,
        spots: impl Iterator<Item = (usize, (f32, f32))>,
        at: (f32, f32),
        occupied: Option<usize>,
    ) -> Vec<usize> {
        spots
            .filter(|(i, centre)| {
                Some(*i) != occupied && !self.is_jammed(*i) && dist(*centre, at) <= RANGE
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Godot `_is_jammed` (line 46).
    fn is_jammed(&self, spot: usize) -> bool {
        self.jammed.iter().any(|(s, _)| *s == spot)
    }

    /// Godot lines 36-43: pick up to COUNT spots at random (removing each from `near`, as
    /// `pop_at` does), jam them for JAM_SECONDS, then report the cue if any were jammed.
    /// `pick(n)` returns an index in `0..n`, as `brain.next_random_index(n)` does.
    fn jam(&mut self, mut near: Vec<usize>, mut pick: impl FnMut(usize) -> usize, ev: &mut Vec<Event>) {
        let mut jammed = 0;
        while jammed < COUNT && !near.is_empty() {
            let idx = pick(near.len());
            let spot = near.remove(idx);
            self.jammed.push((spot, JAM_SECONDS));
            ev.push(Event::SpotDisabled(spot, true));
            jammed += 1;
        }
        if jammed > 0 {
            // dropped by `silence_callouts` in the dispatcher, like every other callout
            ev.push(Event::Message(cue(jammed)));
        }
    }
}

/// void_brown_gimmick.gd:41-43: "%d HIDING SPOT%s JAMMED SHUT", "S" unless exactly one.
fn cue(jammed: usize) -> String {
    let plural = if jammed == 1 { "" } else { "S" };
    format!("{jammed} HIDING SPOT{plural} JAMMED SHUT")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::gimmicks::silence_callouts;

    fn fires_after(total: f32, step: f32) -> bool {
        let mut v = VoidBrown::default();
        let mut t = 0.0;
        while t < total {
            if v.tick_cooldown(step) {
                return true;
            }
            t += step;
        }
        false
    }

    #[test]
    fn first_jam_is_due_at_exactly_18_seconds() {
        let mut v = VoidBrown::default();
        assert!(!v.tick_cooldown(17.5), "not yet at 17.5 s");
        assert!(v.tick_cooldown(0.5), "due at 18 s");
        // restarts at INTERVAL: the next jam is another 18 s away
        assert!(!v.tick_cooldown(17.0));
        assert!(v.tick_cooldown(1.0), "due again at 18 s");
    }

    #[test]
    fn cooldown_is_18_seconds_in_small_steps_too() {
        assert!(fires_after(18.0, 0.25));
        assert!(!fires_after(17.75, 0.25));
    }

    #[test]
    fn a_jam_lasts_20_seconds_then_reports_reenable() {
        let mut v = VoidBrown::default();
        let mut ev = Vec::new();
        v.jam(vec![3], |_| 0, &mut ev);
        assert!(v.is_jammed(3));
        ev.clear();
        v.expire(19.0, &mut ev);
        assert!(ev.is_empty(), "still jammed at 19 s");
        assert!(v.is_jammed(3));
        v.expire(1.0, &mut ev);
        assert!(!v.is_jammed(3), "released at 20 s");
        assert!(matches!(ev.as_slice(), [Event::SpotDisabled(3, false)]));
    }

    #[test]
    fn jams_at_most_two_random_spots_and_reports_them() {
        let mut v = VoidBrown::default();
        let mut ev = Vec::new();
        // always pick the last candidate, so the order is observable
        v.jam(vec![1, 2, 3, 4], |n| n - 1, &mut ev);
        assert!(matches!(
            ev.as_slice(),
            [
                Event::SpotDisabled(4, true),
                Event::SpotDisabled(3, true),
                Event::Message(m),
            ] if m == "2 HIDING SPOTS JAMMED SHUT"
        ));
        assert_eq!(v.jammed.len(), 2);
        assert!(v.is_jammed(4) && v.is_jammed(3) && !v.is_jammed(1));
    }

    #[test]
    fn the_cue_is_singular_for_one_spot_and_plural_otherwise() {
        assert_eq!(cue(1), "1 HIDING SPOT JAMMED SHUT");
        assert_eq!(cue(2), "2 HIDING SPOTS JAMMED SHUT");
    }

    #[test]
    fn no_candidates_means_no_jam_and_no_cue() {
        let mut v = VoidBrown::default();
        let mut ev = Vec::new();
        v.jam(Vec::new(), |_| 0, &mut ev);
        assert!(ev.is_empty());
        assert!(v.jammed.is_empty());
    }

    #[test]
    fn range_is_nine_cells_from_the_cell_centre_and_excludes_occupied_and_jammed() {
        let mut v = VoidBrown::default();
        // runner at (9.5, 0.5)
        let at = (9.5, 0.5);
        let spots = vec![
            (0, (0.5, 0.5)),   // exactly 9 cells away: in range (inclusive)
            (1, (10.5, 0.5)),  // 1 cell away: in range
            (2, (19.5, 0.5)),  // 10 cells away: out of range
            (3, (0.5, 0.5)),   // in range, but the runner occupies it: excluded
            (4, (2.5, 0.5)),   // in range, but already jammed: excluded
        ];
        v.jammed.push((4, JAM_SECONDS));
        let near = v.candidates(spots.into_iter(), at, Some(3));
        assert_eq!(near, vec![0, 1]);
    }

    #[test]
    fn no_occupied_spot_means_every_in_range_spot_is_a_candidate() {
        let v = VoidBrown::default();
        let spots = vec![(0, (0.5, 0.5)), (1, (1.5, 0.5))];
        let near = v.candidates(spots.into_iter(), (0.5, 0.5), None);
        assert_eq!(near, vec![0, 1]);
    }

    #[test]
    fn the_cue_message_is_silenced_like_every_other_callout() {
        let mut v = VoidBrown::default();
        let mut ev = Vec::new();
        v.jam(vec![5], |_| 0, &mut ev);
        assert!(ev.iter().any(|e| matches!(e, Event::Message(_))), "the cue is emitted");
        silence_callouts(&mut ev);
        assert!(!ev.iter().any(|e| matches!(e, Event::Message(_))));
        assert!(matches!(ev.as_slice(), [Event::SpotDisabled(5, true)]));
    }
}
