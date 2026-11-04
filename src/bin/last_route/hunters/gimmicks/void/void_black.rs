//! Black Void (rearStalker): frozen while the runner is looking at her (within 11 cells, 0.85 rad, with a clear
//! line), and 1.5x faster whenever she is not watched (void_black_gimmick.gd). The watch is re-read every tick in
//! `before`, so the speed scale used in the same tick's movement always matches it. There is no timer.

use super::super::Ctx;
use crate::hunters::brain::{Brain, Target};
use crate::world::facility::Facility;

/// void_black_gimmick.gd:7 (GAZE_RANGE): the runner must be within 11 cells of her.
const GAZE_RANGE: f32 = 11.0;
/// void_black_gimmick.gd:8 (GAZE_HALF_ANGLE): the runner's facing must be within 0.85 rad of the direction to her.
const GAZE_HALF_ANGLE: f32 = 0.85;
/// void_black_gimmick.gd:9 (HIDDEN_SPEED_SCALE): her speed multiplier while she is not watched.
const HIDDEN_SPEED_SCALE: f32 = 1.5;

/// The looked-at rule: VoidGimmick.player_watching (void_gimmick.gd:9-20), called from void_black_gimmick.gd:15.
/// A hidden runner, or one in a safe zone, is never watching (void_gimmick.gd:10). Otherwise the runner's facing
/// must lie within `GAZE_HALF_ANGLE` of the direction to her, she must be within `GAZE_RANGE`, and the line between
/// them must be clear. `Brain::beam_reaches` (brain.rs) is the same cone, range and line-of-sight test, with the
/// runner as the origin.
pub fn watched_by(t: &Target, from: (f32, f32), fac: &Facility) -> bool {
    if t.hidden || t.in_safe_zone {
        return false;
    }
    Brain::beam_reaches(fac, t.pos, t.facing, from, GAZE_RANGE, GAZE_HALF_ANGLE)
}

/// void_black_gimmick.gd:11 (`var watched := false`): she starts unwatched.
#[derive(Default)]
pub struct VoidBlack {
    pub watched: bool,
}

impl VoidBlack {
    /// void_black_gimmick.gd:14-15 (before_perception): re-read the watch from the runner's current state.
    pub fn before(&mut self, b: &Brain, c: &Ctx, t: &Target) {
        self.watched = watched_by(t, b.pos, c.fac);
    }

    /// void_black_gimmick.gd:18-19 (speed_scale): frozen while watched, 1.5x otherwise.
    pub fn speed_scale(&self) -> f32 {
        if self.watched {
            0.0
        } else {
            HIDDEN_SPEED_SCALE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;
    use crate::world::Cell;

    fn facility() -> Facility {
        let cat = Catalog::load();
        Facility::generate("VOID-BLACK-1", &Options::default(), &cat)
    }

    /// Unit vector from `a` toward `b`.
    fn toward(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = (dx * dx + dy * dy).sqrt();
        (dx / l, dy / l)
    }

    /// `d` turned by `angle` radians.
    fn turned(d: (f32, f32), angle: f32) -> (f32, f32) {
        let (s, c) = angle.sin_cos();
        (d.0 * c - d.1 * s, d.0 * s + d.1 * c)
    }

    /// Two floor-cell centres `lo..=hi` apart, in clear line when `clear` is true, or with a wall between when false.
    fn pair(fac: &Facility, lo: f32, hi: f32, clear: bool) -> Option<((f32, f32), (f32, f32))> {
        let cells: Vec<Cell> = fac.floor_cells();
        for a in cells.iter().step_by(3) {
            for b in cells.iter().step_by(7) {
                let (pa, pb) = (a.center(), b.center());
                let d = crate::hunters::brain::dist(pa, pb);
                if d >= lo && d <= hi && fac.has_line_of_sight(pa, pb) == clear {
                    return Some((pa, pb));
                }
            }
        }
        None
    }

    /// A runner standing at `pos`, looking along `facing`, not hidden and not in a safe zone.
    fn runner(pos: (f32, f32), facing: (f32, f32)) -> Target {
        Target { pos, facing, ..Default::default() }
    }

    #[test]
    fn starts_unwatched_and_runs_fast() {
        // void_black_gimmick.gd:11 (watched := false) and :19 (1.5 while not watched)
        let g = VoidBlack::default();
        assert!(!g.watched);
        assert_eq!(g.speed_scale(), HIDDEN_SPEED_SCALE);
    }

    #[test]
    fn speed_follows_the_same_tick_watch() {
        // void_black_gimmick.gd:18-19: watched freezes her (0.0); otherwise 1.5. The flip is immediate, no timer.
        let mut g = VoidBlack { watched: true };
        assert_eq!(g.speed_scale(), 0.0);
        g.watched = false;
        assert_eq!(g.speed_scale(), 1.5);
    }

    #[test]
    fn gaze_constants_match_godot() {
        // void_black_gimmick.gd:7-9
        assert_eq!(GAZE_RANGE, 11.0);
        assert_eq!(GAZE_HALF_ANGLE, 0.85);
        assert_eq!(HIDDEN_SPEED_SCALE, 1.5);
    }

    #[test]
    fn looking_straight_at_her_in_a_clear_line_watches_her() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        assert!(watched_by(&runner(p, toward(p, h)), h, &fac));
    }

    #[test]
    fn facing_away_does_not_watch_her() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        assert!(!watched_by(&runner(p, toward(h, p)), h, &fac));
    }

    #[test]
    fn the_half_angle_is_0_85_radians() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        let d = toward(p, h);
        assert!(watched_by(&runner(p, turned(d, 0.80)), h, &fac), "0.80 rad is inside the cone");
        assert!(!watched_by(&runner(p, turned(d, 0.90)), h, &fac), "0.90 rad is outside the cone");
    }

    #[test]
    fn range_is_11_cells() {
        let fac = facility();
        let (p, h) = pair(&fac, 11.5, 13.0, true).expect("a clear pair 11.5 to 13 cells apart");
        assert!(!watched_by(&runner(p, toward(p, h)), h, &fac), "beyond 11 cells never watches");
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        assert!(watched_by(&runner(p, toward(p, h)), h, &fac), "inside 11 cells does");
    }

    #[test]
    fn a_wall_between_them_breaks_the_watch() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 9.0, false).expect("a pair with a wall between them");
        assert!(!watched_by(&runner(p, toward(p, h)), h, &fac));
    }

    #[test]
    fn hidden_or_in_a_safe_zone_never_watches() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        let facing = toward(p, h);
        let hidden = Target { hidden: true, ..runner(p, facing) };
        let safe = Target { in_safe_zone: true, ..runner(p, facing) };
        assert!(!watched_by(&hidden, h, &fac), "a hidden runner does not watch");
        assert!(!watched_by(&safe, h, &fac), "a runner in a safe zone does not watch");
    }

    #[test]
    fn a_zero_facing_never_watches() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        assert!(!watched_by(&runner(p, (0.0, 0.0)), h, &fac));
    }
}
