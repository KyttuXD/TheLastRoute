//! White Void (`void_white`): lightFrozen. While the runner's lit torch is on her (within 10 cells, 0.42 rad, with a clear
//! line and no hiding or safe zone), she is frozen; otherwise she moves at 1.35x (void_white_gimmick.gd). The beam is
//! re-read every tick in `before`, so the speed scale used in the same tick's movement always matches it. There is no
//! timer and no event: the gimmick only changes her speed.
//!
//! The beam test is VoidGimmick.beam_on (void_gimmick.gd:24-30), which builds on player_watching (void_gimmick.gd:9-20).
//! The runner-as-origin cone, range and line test is `Brain::beam_reaches` (brain.rs), the same as void_black's gaze.

use super::super::Ctx;
use crate::hunters::brain::{Brain, Target};
use crate::world::facility::Facility;

/// void_white_gimmick.gd:6 (BEAM_RANGE): the torch reaches 10 cells.
const BEAM_RANGE: f32 = 10.0;
/// void_white_gimmick.gd:7 (BEAM_HALF_ANGLE): the torch must point within 0.42 rad of the direction to her.
const BEAM_HALF_ANGLE: f32 = 0.42;
/// void_white_gimmick.gd:8 (DARK_SPEED_SCALE): her speed multiplier while the beam is not on her.
const DARK_SPEED_SCALE: f32 = 1.35;

/// VoidGimmick.beam_on (void_gimmick.gd:24-30): the runner's lit torch hits `from` (the hunter's position).
/// A torch inside a hiding spot never counts (void_gimmick.gd:26-27); the torch must be on with a charged lamp
/// (void_gimmick.gd:28-29); the rest is player_watching, the cone, range and line test (void_gimmick.gd:9-20).
pub fn beam_on(t: &Target, from: (f32, f32), range: f32, half_angle: f32, fac: &Facility) -> bool {
    if t.hidden {
        return false;
    }
    if !t.flashlight_on || t.lamp_charge <= 0.0 {
        return false;
    }
    player_watching(t, from, range, half_angle, fac)
}

/// VoidGimmick.player_watching (void_gimmick.gd:9-20). A hidden runner or one in a safe zone is never watching
/// (void_gimmick.gd:10). Otherwise the runner's facing must lie within `half_angle` of the direction to `from`, `from`
/// must be within `range`, and the line between them must be clear. `Brain::beam_reaches` is that test with the
/// runner as the origin (void_gimmick.gd:14-20).
fn player_watching(t: &Target, from: (f32, f32), range: f32, half_angle: f32, fac: &Facility) -> bool {
    if t.hidden || t.in_safe_zone {
        return false;
    }
    Brain::beam_reaches(fac, t.pos, t.facing, from, range, half_angle)
}

/// void_white_gimmick.gd:11 (`var lit := false`): she starts unlit, so she moves at the dark speed until the beam reads.
#[derive(Default)]
pub struct VoidWhite {
    /// The runner's torch is on her this tick (void_white_gimmick.gd `lit`).
    pub lit: bool,
}

impl VoidWhite {
    /// void_white_gimmick.gd:13-14 (before_perception): re-read the beam from her current position.
    pub fn before(&mut self, b: &Brain, c: &Ctx, t: &Target) {
        self.refresh(b.pos, c.fac, t);
    }

    /// The per-tick read used by `before` (void_white_gimmick.gd:14): `lit` is recomputed from the runner's current state
    /// and her position `from`, with no memory of earlier ticks.
    pub fn refresh(&mut self, from: (f32, f32), fac: &Facility, t: &Target) {
        self.lit = beam_on(t, from, BEAM_RANGE, BEAM_HALF_ANGLE, fac);
    }

    /// void_white_gimmick.gd:17-18 (speed_scale): frozen while lit, 1.35x otherwise.
    pub fn speed_scale(&self) -> f32 {
        if self.lit {
            0.0
        } else {
            DARK_SPEED_SCALE
        }
    }

    /// void_white_gimmick.gd:21-22 (network_state): the one bit the networked state carries.
    pub fn network_state(&self) -> bool {
        self.lit
    }

    /// void_white_gimmick.gd:25-26 (apply_network_state): sets the lit flag from the host.
    pub fn apply_network_state(&mut self, lit: bool) {
        self.lit = lit;
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
        Facility::generate("VOID-WHITE-1", &Options::default(), &cat)
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

    /// A runner standing at `pos`, facing `facing`, torch on with a charged lamp, not hidden and not in a safe zone.
    fn lit_runner(pos: (f32, f32), facing: (f32, f32)) -> Target {
        Target { pos, facing, flashlight_on: true, lamp_charge: 1.0, ..Default::default() }
    }

    #[test]
    fn starts_unlit_and_runs_at_the_dark_speed() {
        // void_white_gimmick.gd:11 (lit := false) and :18 (1.35 while not lit)
        let g = VoidWhite::default();
        assert!(!g.lit);
        assert_eq!(g.speed_scale(), DARK_SPEED_SCALE);
    }

    #[test]
    fn speed_follows_the_same_tick_beam() {
        // void_white_gimmick.gd:17-18: lit freezes her (0.0), otherwise 1.35. No timer, so the flip is immediate.
        let mut g = VoidWhite { lit: true };
        assert_eq!(g.speed_scale(), 0.0);
        g.lit = false;
        assert_eq!(g.speed_scale(), 1.35);
    }

    #[test]
    fn constants_match_godot() {
        // void_white_gimmick.gd:6-8
        assert_eq!(BEAM_RANGE, 10.0);
        assert_eq!(BEAM_HALF_ANGLE, 0.42);
        assert_eq!(DARK_SPEED_SCALE, 1.35);
    }

    #[test]
    fn a_lit_torch_straight_at_her_in_a_clear_line_freezes_her() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        assert!(beam_on(&lit_runner(p, toward(p, h)), h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac));
    }

    #[test]
    fn the_torch_must_be_on_and_the_lamp_charged() {
        // void_gimmick.gd:28-29
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        let facing = toward(p, h);
        let off = Target { flashlight_on: false, ..lit_runner(p, facing) };
        let dead = Target { lamp_charge: 0.0, ..lit_runner(p, facing) };
        assert!(!beam_on(&off, h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac), "torch off does not freeze her");
        assert!(!beam_on(&dead, h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac), "an empty lamp does not freeze her");
    }

    #[test]
    fn a_runner_in_a_hiding_spot_or_safe_zone_does_not_pin_her() {
        // void_gimmick.gd:26-27 (hidden) and :10 (in_safe_zone via player_watching)
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        let facing = toward(p, h);
        let hidden = Target { hidden: true, ..lit_runner(p, facing) };
        let safe = Target { in_safe_zone: true, ..lit_runner(p, facing) };
        assert!(!beam_on(&hidden, h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac), "a torch inside a hiding spot never counts");
        assert!(!beam_on(&safe, h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac), "a runner in a safe zone is never watching");
    }

    #[test]
    fn facing_away_does_not_freeze_her() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        assert!(!beam_on(&lit_runner(p, toward(h, p)), h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac));
    }

    #[test]
    fn the_beam_range_is_10_cells() {
        let fac = facility();
        let (p, h) = pair(&fac, 10.5, 13.0, true).expect("a clear pair 10.5 to 13 cells apart");
        assert!(!beam_on(&lit_runner(p, toward(p, h)), h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac), "beyond 10 cells never freezes");
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        assert!(beam_on(&lit_runner(p, toward(p, h)), h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac), "inside 10 cells does");
    }

    #[test]
    fn the_beam_half_angle_is_0_42_radians() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        let d = toward(p, h);
        assert!(beam_on(&lit_runner(p, turned(d, 0.38)), h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac), "0.38 rad is inside the beam");
        assert!(!beam_on(&lit_runner(p, turned(d, 0.46)), h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac), "0.46 rad is outside the beam");
    }

    #[test]
    fn a_wall_between_them_breaks_the_beam() {
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 9.0, false).expect("a pair with a wall between them");
        assert!(!beam_on(&lit_runner(p, toward(p, h)), h, BEAM_RANGE, BEAM_HALF_ANGLE, &fac));
    }

    #[test]
    fn the_lit_flag_is_re_read_every_tick_with_no_memory() {
        // void_white_gimmick.gd:13-14: the beam is recomputed each tick, so turning the torch away or switching it
        // off unfreezes her on the same tick, and switching it back on freezes her again at once.
        let fac = facility();
        let (p, h) = pair(&fac, 3.0, 8.0, true).expect("a clear pair of floor cells");
        let aimed = lit_runner(p, toward(p, h));
        let mut g = VoidWhite::default();
        g.refresh(h, &fac, &aimed);
        assert_eq!(g.speed_scale(), 0.0, "aimed: frozen on the first tick");
        let off = Target { flashlight_on: false, ..aimed.clone() };
        g.refresh(h, &fac, &off);
        assert_eq!(g.speed_scale(), DARK_SPEED_SCALE, "torch off: 1.35 on the same tick");
        g.refresh(h, &fac, &aimed);
        assert_eq!(g.speed_scale(), 0.0, "torch back on: frozen again on the same tick");
    }

    #[test]
    fn network_state_round_trips_the_lit_flag() {
        // void_white_gimmick.gd:21-26
        let mut g = VoidWhite { lit: true };
        let s = g.network_state();
        g.lit = false;
        g.apply_network_state(s);
        assert!(g.lit);
    }
}
