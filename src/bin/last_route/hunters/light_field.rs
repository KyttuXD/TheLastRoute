//! LightField (game/look/light_field.gd, docs/LIGHT_SIGHT.md): how lit a spot is, 0..1, from the lamps that drive the
//! rendering. The hunters read it for the mutual light rule: a runner standing where the building lights her is seen
//! from further off whatever her flashlight does, and a hunter standing in light the runner can see her by sees the
//! runner at her full range (`Brain::sight_limit`). A pure function of lamp data, so the same rule runs in the native
//! and the web build and in the tests.

/// How far a lamp counts (cells), whatever its own range.
pub const LAMP_REACH: f32 = 25.0;
/// The exposure floor in a facility with no light at all (Godot's near-black ambient).
pub const AMBIENT: f32 = 0.02;
/// Exposure the player's eye needs to make out a hunter; mutually, the exposure at which a hunter "stands in light".
pub const VISIBLE_EXPOSURE: f32 = 0.36;
/// Sight range scale by the runner's own exposure when her flashlight is off: pitch dark, and a full lamp pool.
pub const DARK_SIGHT_SCALE: f32 = 0.25;
pub const LIT_SIGHT_SCALE: f32 = 1.6;
/// How a lamp's light falls off over its range, as Godot's light_field.gd has it (calibrated against captures of the
/// lamp pools): `energy * FALLOFF_GAIN * (1 - d / range)^FALLOFF_POWER`. The world shader's own falloff is the
/// square of the same ramp times a surface term, a little steeper; this one reads a body's whole front.
pub const FALLOFF_GAIN: f32 = 0.9;
pub const FALLOFF_POWER: f32 = 1.6;
/// A lamp adding less than this to a spot is left out.
pub const MIN_CONTRIBUTION: f32 = 0.02;
/// Height above the floor (cells) a body is lit at: its chest, where the camera sees a body.
pub const BODY_HEIGHT: f32 = 1.2;

/// A light as the field reads it: where (x, y up, z), its reach, and its energy with the flicker of this moment
/// already applied (a lamp that is dark in its flicker lights nothing).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub pos: [f32; 3],
    pub range: f32,
    pub energy: f32,
}

/// Sight range scale for a runner of `exposure` with her flashlight off: `DARK_SIGHT_SCALE` in the dark up to
/// `LIT_SIGHT_SCALE` in a lamp pool (LightField.unlit_scale).
pub fn unlit_scale(exposure: f32) -> f32 {
    let t = exposure.clamp(0.0, 1.0);
    DARK_SIGHT_SCALE + (LIT_SIGHT_SCALE - DARK_SIGHT_SCALE) * t
}

/// The exposure at `point` (x, y up, z): `ambient` plus every source within reach, falling off over its range
/// (`FALLOFF_GAIN * energy * (1 - d / range)^FALLOFF_POWER`), unless `line_clear(source xz, point xz)` says a wall is between. Clamped
/// to 0..1.
pub fn exposure_at(
    point: [f32; 3],
    ambient: f32,
    sources: &[Source],
    line_clear: impl Fn((f32, f32), (f32, f32)) -> bool,
) -> f32 {
    let mut total = ambient.max(0.0);
    for s in sources {
        let (dx, dy, dz) = (s.pos[0] - point[0], s.pos[1] - point[1], s.pos[2] - point[2]);
        let d = (dx * dx + dy * dy + dz * dz).sqrt();
        if s.range <= 0.0 || d >= s.range.min(LAMP_REACH) {
            continue;
        }
        let contribution = s.energy * FALLOFF_GAIN * (1.0 - d / s.range).powf(FALLOFF_POWER);
        if contribution < MIN_CONTRIBUTION {
            continue;
        }
        if !line_clear((s.pos[0], s.pos[2]), (point[0], point[2])) {
            continue;
        }
        total += contribution;
    }
    total.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lamp(x: f32, energy: f32) -> Source {
        Source { pos: [x, 2.6, 0.0], range: 7.0, energy }
    }

    const OPEN: fn((f32, f32), (f32, f32)) -> bool = |_, _| true;

    #[test]
    fn a_spot_with_no_lamp_is_the_ambient() {
        assert_eq!(exposure_at([0.0, 1.2, 0.0], 0.09, &[], OPEN), 0.09);
        assert_eq!(exposure_at([0.0, 1.2, 0.0], 0.09, &[lamp(100.0, 2.0)], OPEN), 0.09, "a lamp out of reach adds nothing");
    }

    #[test]
    fn light_falls_off_with_the_distance_from_the_lamp() {
        let l = [lamp(0.0, 1.5)];
        let near = exposure_at([0.0, 1.2, 0.0], 0.0, &l, OPEN);
        let mid = exposure_at([3.0, 1.2, 0.0], 0.0, &l, OPEN);
        let far = exposure_at([6.0, 1.2, 0.0], 0.0, &l, OPEN);
        assert!(near > mid && mid > far, "{near} {mid} {far}");
        // the field's falloff with d the 3D distance
        let d = (3.0f32 * 3.0 + 1.4 * 1.4).sqrt();
        assert!((mid - 1.5 * 0.9 * (1.0 - d / 7.0).powf(1.6)).abs() < 1e-5);
        assert_eq!(exposure_at([20.0, 1.2, 0.0], 0.0, &l, OPEN), 0.0, "past the range there is nothing");
    }

    #[test]
    fn a_wall_between_the_lamp_and_the_spot_blocks_it() {
        let l = [lamp(0.0, 1.5)];
        let wall = |_: (f32, f32), _: (f32, f32)| false;
        assert_eq!(exposure_at([1.0, 1.2, 0.0], 0.05, &l, wall), 0.05);
        assert!(exposure_at([1.0, 1.2, 0.0], 0.05, &l, OPEN) > 0.5);
    }

    #[test]
    fn a_lamp_dark_in_its_flicker_lights_nothing_and_exposure_is_clamped() {
        assert_eq!(exposure_at([0.0, 1.2, 0.0], 0.0, &[lamp(0.0, 0.0)], OPEN), 0.0);
        assert_eq!(exposure_at([0.0, 1.2, 0.0], 0.5, &[lamp(0.0, 3.0), lamp(0.5, 3.0)], OPEN), 1.0);
    }

    #[test]
    fn several_lamps_add_up() {
        let one = exposure_at([2.0, 1.2, 0.0], 0.0, &[lamp(0.0, 0.6)], OPEN);
        let two = exposure_at([2.0, 1.2, 0.0], 0.0, &[lamp(0.0, 0.6), lamp(4.0, 0.6)], OPEN);
        assert!(two > one * 1.5, "{one} {two}");
    }

    #[test]
    fn the_unlit_scale_runs_from_dark_to_a_lamp_pool() {
        assert_eq!(unlit_scale(0.0), DARK_SIGHT_SCALE);
        assert_eq!(unlit_scale(1.0), LIT_SIGHT_SCALE);
        assert_eq!(unlit_scale(-3.0), DARK_SIGHT_SCALE);
        assert_eq!(unlit_scale(9.0), LIT_SIGHT_SCALE);
        assert!(unlit_scale(0.5) > unlit_scale(0.2));
    }
}
