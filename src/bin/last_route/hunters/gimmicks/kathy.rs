//! Kathy: search beam. A narrow cone along her facing (11.5 cells, 0.24 rad) spots the runner
//! whatever the light (kathy_gimmick.gd).

use super::Ctx;
use crate::hunters::brain::{Brain, Target};

/// Her search beam's reach (metres) and half angle (radians): GameConfig.KATHY_BEAM_RANGE / KATHY_BEAM_HALF_ANGLE. The
/// drawn beam (render/beam.rs) uses the same numbers.
pub const BEAM_RANGE: f32 = 11.5;
pub const BEAM_HALF_ANGLE: f32 = 0.24;

pub fn before(b: &mut Brain, c: &Ctx, t: &Target) {
    if t.hidden || t.in_safe_zone {
        return;
    }
    if Brain::beam_reaches(c.fac, b.pos, b.facing, t.pos, BEAM_RANGE, BEAM_HALF_ANGLE) {
        b.chase(t.pos);
        b.direct_visible = true;
        b.ability_direct_sight = true;
    }
}
