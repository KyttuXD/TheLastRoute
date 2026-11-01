//! Kathy: search beam. A narrow cone along her facing (11.5 cells, 0.24 rad) spots the runner
//! whatever the light (kathy_gimmick.gd).

use super::Ctx;
use crate::hunters::brain::{Brain, Target};

pub fn before(b: &mut Brain, c: &Ctx, t: &Target) {
    if t.hidden || t.in_safe_zone {
        return;
    }
    if Brain::beam_reaches(c.fac, b.pos, b.facing, t.pos, 11.5, 0.24) {
        b.chase(t.pos);
        b.direct_visible = true;
        b.ability_direct_sight = true;
    }
}
