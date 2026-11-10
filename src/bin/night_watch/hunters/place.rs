//! Where and how big a hunter stands: yaw helpers, the hunter scale (NightWatchHunterScale) and the shared stand
//! slot offsets. Pure maths, no GL.
#![allow(dead_code)]

use super::anim::clip::Rig;
use super::anim::MODEL_SCALE;
use macroquad::prelude::{vec3, Vec3};

/// The humanoid hunters' height in metres (1.5x scale, about 2.58 m).
pub const HUNTER_HEIGHT_M: f32 = 2.58;

/// The yaw that turns a hunter standing at `from` to face `to` on the ground plane. Yaw 0 faces -z and the forward
/// vector is (-sin yaw, -cos yaw), the same convention as the mannequin and the player.
pub fn yaw_toward(from: Vec3, to: Vec3) -> f32 {
    (from.x - to.x).atan2(from.z - to.z)
}

/// `face()` of the contract: the yaw to look along a direction (not a target point). Zero length keeps `keep`.
pub fn yaw_of_dir(dir: Vec3, keep: f32) -> f32 {
    if dir.x * dir.x + dir.z * dir.z < 1e-6 {
        keep
    } else {
        (-dir.x).atan2(-dir.z)
    }
}

/// The yaw to face `to` from `from`, keeping `keep` when they coincide on the ground.
pub fn face(from: Vec3, to: Vec3, keep: f32) -> f32 {
    yaw_of_dir(to - from, keep)
}

/// Metres per bbmodel unit a rig is drawn at (HunterRoster.scale_for over the glb's unit): the humanoids at 1.5x,
/// creature rigs at their own scale.
pub fn hunter_scale(rig: &Rig) -> f32 {
    rig.draw_scale()
}

/// The factor over the authored size (1.5 for the humanoids). Creature rigs report their own ratio to 1.5/unit.
pub fn scale_factor(rig: &Rig) -> f32 {
    let base = 1.0 / rig.unit.max(1e-6);
    rig.draw_scale() / base
}

/// The shared stand spot offsets inside a room (NightWatchBuilding stand slots): slot 0 and slot 1; further slots
/// step along x.
pub fn slot_offset(slot: usize) -> Vec3 {
    match slot {
        0 => vec3(-0.6, 0.0, 0.4),
        1 => vec3(1.0, 0.0, -0.6),
        n => vec3(1.0 + 1.3 * (n as f32 - 1.0), 0.0, -0.6),
    }
}

/// A stand spot: room centre plus the slot offset, with an optional platform lift (the stage adds 0.45 up, 1.4 back).
pub fn stand_spot(centre: Vec3, slot: usize, platform: bool) -> Vec3 {
    let mut p = centre + slot_offset(slot);
    if platform {
        p += vec3(0.0, 0.45, -1.4);
    }
    p
}

/// The default rig scale when no rig is at hand.
pub const DEFAULT_MODEL_SCALE: f32 = MODEL_SCALE;

#[cfg(test)]
mod tests {
    use super::*;

    fn forward(yaw: f32) -> Vec3 {
        vec3(-yaw.sin(), 0.0, -yaw.cos())
    }

    #[test]
    fn yaw_toward_points_the_forward_vector_at_the_target() {
        let from = vec3(0.0, 0.0, 0.0);
        for to in [vec3(0.0, 0.0, -5.0), vec3(5.0, 0.0, 0.0), vec3(0.0, 0.0, 5.0), vec3(-5.0, 0.0, 0.0), vec3(3.0, 0.0, -4.0)] {
            let f = forward(yaw_toward(from, to));
            let d = (to - from).normalize();
            assert!((f - d).length() < 1e-4, "{to:?} -> {f:?}");
        }
    }

    #[test]
    fn yaw_toward_quadrants() {
        let o = Vec3::ZERO;
        assert!(yaw_toward(o, vec3(0.0, 0.0, -1.0)).abs() < 1e-6);
        assert!((yaw_toward(o, vec3(1.0, 0.0, 0.0)) + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((yaw_toward(o, vec3(-1.0, 0.0, 0.0)) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((yaw_toward(o, vec3(0.0, 0.0, 1.0)).abs() - std::f32::consts::PI).abs() < 1e-5);
    }

    #[test]
    fn face_agrees_with_yaw_toward_and_keeps_yaw_when_on_the_spot() {
        let a = vec3(1.0, 0.0, 2.0);
        let b = vec3(-3.0, 1.0, 0.5);
        assert!((face(a, b, 0.0) - yaw_toward(a, b)).abs() < 1e-5);
        assert_eq!(face(a, a + vec3(0.0, 2.0, 0.0), 1.25), 1.25);
    }

    #[test]
    fn stand_spots() {
        assert_eq!(slot_offset(0), vec3(-0.6, 0.0, 0.4));
        assert_eq!(slot_offset(1), vec3(1.0, 0.0, -0.6));
        let s = stand_spot(vec3(0.0, 0.0, -40.0), 0, true);
        assert!((s - vec3(-0.6, 0.45, -41.0)).length() < 1e-5);
    }
}
