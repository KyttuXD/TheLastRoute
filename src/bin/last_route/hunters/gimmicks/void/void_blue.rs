//! Blue Void (`void_blue`, silentStalker): nearly invisible and off the radar until the runner's
//! flashlight beam lands on her. A beam hit (12 cells, 0.48 rad) shoves her 6 cells straight away
//! from the runner and shows her for 3.5 s. Her chase and perception are the shared brain's.
//!
//! Godot source: game/hunters/gimmicks/void_blue_gimmick.gd (VoidBlueGimmick, extends VoidGimmick).
//! Player-facing text: game/hunters/gimmicks/void_hunter_gimmicks/void_hunter_gimmick_chart.gd:12-15.
//!
//! Integrator wiring (gimmicks/mod.rs is not edited here):
//! - `Gimmick::VoidBlue(void::void_blue::VoidBlue)`, and `"void_blue" => Gimmick::VoidBlue(Default::default())` in `for_id`.
//! - `before_perception`: `Gimmick::VoidBlue(g) => g.before(b, dt, c, t, ev)`. It runs only while she is active, like every
//!   ordinary gimmick (hunter_brain.gd:254-273).
//! - `hidden_from_view`: `Gimmick::VoidBlue(g) => g.hidden_from_view()`. She starts hidden (reveal 0.0).
//! - Events: `Event::Message("SOMETHING WAS STANDING THERE")` on a fresh reveal only. `silence_callouts` drops it, the same
//!   as Godot's `emit_world_event` with ANNOUNCE_HUNTER_EVENTS off (hunter_brain.gd:82, 88-95), so nothing is shown.
//! - No player-side effect: the beam is read from the runner's existing flashlight and lamp state.

use crate::config::HUNTER_RADIUS;
use crate::hunters::brain::{dist, Brain, Target};
use crate::hunters::gimmicks::{Ctx, Event};

/// void_blue_gimmick.gd:7 `REVEAL_SECONDS := 3.5`: how long a beam hit keeps her drawn.
const REVEAL_SECONDS: f32 = 3.5;
/// void_blue_gimmick.gd:8 `PUSHBACK := 6.0`: the distance, in cells, she is shoved away from the runner.
const PUSHBACK: f32 = 6.0;
/// void_blue_gimmick.gd:23 (`PUSHBACK + 1.5`): a push that lands within this slack of PUSHBACK is taken.
const PUSHBACK_SLACK: f32 = 1.5;
/// void_blue_gimmick.gd:9 `BEAM_RANGE := 12.0`.
const BEAM_RANGE: f32 = 12.0;
/// void_blue_gimmick.gd:10 `BEAM_HALF_ANGLE := 0.48`, radians.
const BEAM_HALF_ANGLE: f32 = 0.48;
/// void_blue_gimmick.gd:26: the callout on a fresh reveal (dropped by the shared silencer, see the header).
pub const REVEAL_MESSAGE: &str = "SOMETHING WAS STANDING THERE";

/// Blue Void's state. `reveal_remaining` is Godot's `reveal_remaining` (void_blue_gimmick.gd:12): above zero she is drawn.
#[derive(Default)]
pub struct VoidBlue {
    reveal_remaining: f32,
}

impl VoidBlue {
    /// Godot `before_perception` (void_blue_gimmick.gd:15-27): the reveal clock runs first, then a beam hit on her pushes
    /// her and (only if she was hidden) raises the callout, then the reveal restarts.
    pub fn before(&mut self, b: &mut Brain, dt: f32, c: &Ctx, t: &Target, ev: &mut Vec<Event>) {
        self.tick(dt);
        if !beam_on(t, b.pos, BEAM_RANGE, BEAM_HALF_ANGLE, |a, p| c.fac.has_line_of_sight(a, p)) {
            return;
        }
        // the shove stops at the first wall or railing on its line: the open spot is searched from there, so it never
        // lands her on the far side of a railing
        let from = b.pos;
        b.pos = pushed_position(from, t.pos, |x, y| {
            let (sx, sy) = c.fac.clear_toward(from, (x, y), HUNTER_RADIUS);
            c.fac.nearest_open(sx, sy, HUNTER_RADIUS)
        });
        if self.strike() {
            ev.push(Event::Message(REVEAL_MESSAGE.to_string()));
        }
    }

    /// Godot `hidden_from_view` (void_blue_gimmick.gd:30-31): drawn only while the reveal runs.
    pub fn hidden_from_view(&self) -> bool {
        self.reveal_remaining <= 0.0
    }

    /// The reveal clock (void_blue_gimmick.gd:16): `maxf(0.0, reveal_remaining - delta)`.
    fn tick(&mut self, dt: f32) {
        self.reveal_remaining = (self.reveal_remaining - dt).max(0.0);
    }

    /// Restarts the reveal (void_blue_gimmick.gd:27). Returns true when she was out of view, i.e. a fresh reveal
    /// that raises the callout (void_blue_gimmick.gd:25-26). A hit during the reveal extends it without a callout.
    fn strike(&mut self) -> bool {
        let fresh = self.reveal_remaining <= 0.0;
        self.reveal_remaining = REVEAL_SECONDS;
        fresh
    }
}

/// The runner's beam reaches `from` (VoidGimmick.beam_on, void_gimmick.gd:24-30). A torch that is off, a lamp with no
/// charge, or a runner hidden in a hiding spot does not count. Then `player_watching` (void_gimmick.gd:9-20): the runner
/// is not in a safe zone, `from` is within `range` (and not on top of her), inside `half_angle` of her facing, and the
/// line `los` is clear. `los` takes the runner's position and then `from`, as `has_line_of_sight(at, from)` does.
pub fn beam_on(
    t: &Target,
    from: (f32, f32),
    range: f32,
    half_angle: f32,
    los: impl Fn((f32, f32), (f32, f32)) -> bool,
) -> bool {
    if t.hidden || !t.flashlight_on || t.lamp_charge <= 0.0 {
        return false;
    }
    if t.in_safe_zone {
        return false;
    }
    let distance = dist(from, t.pos);
    let facing_len = (t.facing.0 * t.facing.0 + t.facing.1 * t.facing.1).sqrt();
    if distance <= 0.001 || distance > range || facing_len <= 0.001 {
        return false;
    }
    // offset = from - at, normalised, dotted with the runner's facing (void_gimmick.gd:14-18)
    let offset = ((from.0 - t.pos.0) / distance, (from.1 - t.pos.1) / distance);
    let dot = offset.0 * (t.facing.0 / facing_len) + offset.1 * (t.facing.1 / facing_len);
    if dot < half_angle.cos() {
        return false;
    }
    los(t.pos, from)
}

/// The shove (void_blue_gimmick.gd:19-24): she moves PUSHBACK cells straight away from the runner, to the nearest open
/// spot for her body. That spot is taken only if it lies within PUSHBACK + PUSHBACK_SLACK of where she was; otherwise
/// she stays put. `nearest_open(x, y)` is `nearest_open_position(x, y, radius)`, returning None for no open cell.
pub fn pushed_position(
    pos: (f32, f32),
    from: (f32, f32),
    nearest_open: impl Fn(f32, f32) -> Option<(f32, f32)>,
) -> (f32, f32) {
    let (dx, dy) = (pos.0 - from.0, pos.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    // Godot Vector2.normalized() on a zero vector is zero; a beam hit always has distance > 0.001, so this is a guard only.
    let away = if len > 0.0 { (dx / len, dy / len) } else { (0.0, 0.0) };
    let wanted = (pos.0 + away.0 * PUSHBACK, pos.1 + away.1 * PUSHBACK);
    match nearest_open(wanted.0, wanted.1) {
        Some(open) if dist(open, pos) <= PUSHBACK + PUSHBACK_SLACK => open,
        _ => pos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runner at the origin, facing +x, torch on, lamp charged.
    fn runner() -> Target {
        Target {
            pos: (0.0, 0.0),
            facing: (1.0, 0.0),
            flashlight_on: true,
            lamp_charge: 1.0,
            ..Default::default()
        }
    }

    fn clear(_: (f32, f32), _: (f32, f32)) -> bool {
        true
    }

    #[test]
    fn void_blue_starts_hidden_and_a_hit_reveals_for_3_5_seconds() {
        let mut g = VoidBlue::default();
        assert!(g.hidden_from_view());
        assert!(g.strike(), "the first hit is a fresh reveal (callout)");
        assert!(!g.hidden_from_view());
        g.tick(3.0);
        assert!(!g.hidden_from_view(), "0.5 s of the 3.5 s is still left");
        g.tick(0.5);
        assert!(g.hidden_from_view(), "hidden again once the 3.5 s is used up");
    }

    #[test]
    fn void_blue_hit_during_reveal_extends_it_without_a_second_callout() {
        let mut g = VoidBlue::default();
        assert!(g.strike());
        g.tick(2.0);
        assert!(!g.strike(), "already drawn: no fresh callout");
        g.tick(3.0);
        assert!(!g.hidden_from_view(), "the second hit restarted the 3.5 s clock at 2.0 s");
        g.tick(0.5);
        assert!(g.hidden_from_view());
        assert!(g.strike(), "after hiding again a hit is fresh again");
    }

    #[test]
    fn void_blue_beam_needs_a_lit_charged_torch_and_a_visible_runner() {
        let from = (5.0, 0.0);
        assert!(beam_on(&runner(), from, BEAM_RANGE, BEAM_HALF_ANGLE, clear));
        let mut t = runner();
        t.flashlight_on = false;
        assert!(!beam_on(&t, from, BEAM_RANGE, BEAM_HALF_ANGLE, clear), "torch off");
        let mut t = runner();
        t.lamp_charge = 0.0;
        assert!(!beam_on(&t, from, BEAM_RANGE, BEAM_HALF_ANGLE, clear), "lamp empty");
        let mut t = runner();
        t.hidden = true;
        assert!(!beam_on(&t, from, BEAM_RANGE, BEAM_HALF_ANGLE, clear), "runner hidden");
        let mut t = runner();
        t.in_safe_zone = true;
        assert!(!beam_on(&t, from, BEAM_RANGE, BEAM_HALF_ANGLE, clear), "runner in a safe zone");
    }

    #[test]
    fn void_blue_beam_range_and_cone_edges() {
        // exactly at range counts (`distance > range` is the miss)
        assert!(beam_on(&runner(), (12.0, 0.0), BEAM_RANGE, BEAM_HALF_ANGLE, clear));
        assert!(!beam_on(&runner(), (12.5, 0.0), BEAM_RANGE, BEAM_HALF_ANGLE, clear));
        // (5, 2) is about 0.38 rad off the facing: inside 0.48. (5, 3) is about 0.54 rad: outside.
        assert!(beam_on(&runner(), (5.0, 2.0), BEAM_RANGE, BEAM_HALF_ANGLE, clear));
        assert!(!beam_on(&runner(), (5.0, 3.0), BEAM_RANGE, BEAM_HALF_ANGLE, clear));
        // a hunter standing on the runner never counts
        assert!(!beam_on(&runner(), (0.0, 0.0), BEAM_RANGE, BEAM_HALF_ANGLE, clear));
    }

    #[test]
    fn void_blue_beam_needs_a_clear_line() {
        let blocked = |_: (f32, f32), _: (f32, f32)| false;
        assert!(!beam_on(&runner(), (5.0, 0.0), BEAM_RANGE, BEAM_HALF_ANGLE, blocked));
    }

    #[test]
    fn void_blue_push_moves_six_cells_straight_away_from_the_runner() {
        // hunter at (5, 5), runner at (4, 5): away is +x, the wanted point is (11, 5)
        let got = pushed_position((5.0, 5.0), (4.0, 5.0), |x, y| {
            assert!((x - 11.0).abs() < 1e-5 && (y - 5.0).abs() < 1e-5, "wanted point ({x}, {y})");
            Some((11.0, 5.0))
        });
        assert_eq!(got, (11.0, 5.0));
    }

    #[test]
    fn void_blue_push_takes_the_slack_but_not_a_longer_snap() {
        // 7.5 cells from where she was is still within PUSHBACK + 1.5 and is taken
        assert_eq!(pushed_position((5.0, 5.0), (4.0, 5.0), |_, _| Some((12.5, 5.0))), (12.5, 5.0));
        // 8 cells is not: she stays where she was
        assert_eq!(pushed_position((5.0, 5.0), (4.0, 5.0), |_, _| Some((13.0, 5.0))), (5.0, 5.0));
        // no open cell at all: she stays where she was
        assert_eq!(pushed_position((5.0, 5.0), (4.0, 5.0), |_, _| None), (5.0, 5.0));
    }
}
