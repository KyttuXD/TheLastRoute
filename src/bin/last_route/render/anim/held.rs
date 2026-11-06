//! The first-person held item's motion, from game/player/held_item_poses.gd and player_controller_3d.gd:
//!
//! * placement (`HeldItemPoses.first_person_transform`): the item's view node sits at FIRST_PERSON.view in camera space,
//!   the model under it is turned Rx(pitch) * Ry(yaw), scaled, and shifted so its GRIP is the view's origin; the whole
//!   FirstPersonItems group is scaled by PlayerModels.SCALE (1.067) and slid back by the wall push.
//! * `_animate_tool_use` (3558): crowbar / axe swing = three tween steps on the view node (TRANS_QUAD): windup to
//!   rotation.x = arc x .70 (EASE_OUT) while the hand rises (.02, .05, .06), strike to -arc x 1.15 (EASE_IN) while it drops
//!   forward (-.03, -.05, -.12), recover to rest (EASE_OUT); a use without a swing twists rotation.z to -.72 and back.
//! * `_animate_taser_fire` (2772): TRANS_BACK kick to +(0, .015, .09) in .07 s and back in .18 s.
//! * `_animate_flashlight_toggle` (2747): TRANS_QUAD dip +(0, -.055, .035) in .09 s, back in .14 s (EASE_OUT).
//! * `_update_viewmodel_push` (3533): a wall closer than the item's REACH slides the hand back (and down a little).
//! Position tweeners without their own ease use the Tween default, EASE_IN_OUT.

use macroquad::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeldId {
    Flashlight,
    Taser,
    Crowbar,
    RescueAxe,
    Battery,
    StimPack,
    MusicBox,
}

pub struct Pose {
    pub view: Vec3,
    pub scale: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub grip: Vec3,
    pub reach: f32,
    /// (windup, strike, recover, arc degrees) for the swinging tools
    pub swing: Option<(f32, f32, f32, f32)>,
}

pub const PLAYER_MODELS_SCALE: f32 = 1.067;
pub const VIEWMODEL_PUSH_RANGE: f32 = 1.0;
pub const VIEWMODEL_MAX_PUSH: f32 = 0.65;

/// HeldItemPoses FIRST_PERSON / GRIP / REACH / SWINGS.
pub fn pose(id: HeldId) -> Pose {
    let p = |view: Vec3, scale: f32, yaw: f32, pitch: f32, grip: f32, reach: f32, swing| Pose {
        view,
        scale,
        yaw,
        pitch,
        grip: vec3(0.0, grip, 0.0),
        reach,
        swing,
    };
    match id {
        HeldId::Flashlight => p(vec3(0.27, -0.24, -0.52), 0.50, 0.0, -76.0, 0.20, 0.90, None),
        HeldId::Taser => p(
            vec3(-0.24, -0.24, -0.50),
            0.50,
            0.0,
            -72.0,
            0.16,
            0.85,
            None,
        ),
        HeldId::Crowbar => p(
            vec3(0.24, -0.28, -0.54),
            0.37,
            70.0,
            -60.0,
            0.34,
            0.95,
            Some((0.12, 0.09, 0.20, 62.0)),
        ),
        HeldId::RescueAxe => p(
            vec3(0.24, -0.28, -0.54),
            0.37,
            90.0,
            -60.0,
            0.14,
            0.95,
            Some((0.16, 0.08, 0.26, 78.0)),
        ),
        HeldId::Battery => p(
            vec3(0.25, -0.26, -0.52),
            0.62,
            25.0,
            -20.0,
            0.23,
            0.75,
            None,
        ),
        HeldId::StimPack => p(
            vec3(0.25, -0.25, -0.50),
            0.55,
            15.0,
            -64.0,
            0.18,
            0.80,
            None,
        ),
        // the box is carried flat and turned to show its front (no swing, like the other carried items)
        HeldId::MusicBox => p(
            vec3(0.25, -0.26, -0.52),
            0.46,
            20.0,
            -28.0,
            0.22,
            0.78,
            None,
        ),
    }
}

/// HeldItemPoses.first_person_transform: the model under its view node.
pub fn model_transform(id: HeldId) -> Mat4 {
    let p = pose(id);
    let basis = Mat4::from_rotation_x(p.pitch.to_radians())
        * Mat4::from_rotation_y(p.yaw.to_radians())
        * Mat4::from_scale(Vec3::splat(p.scale));
    Mat4::from_translation(-basis.transform_vector3(p.grip)) * basis
}

// Godot easing_equations.h, normalised (t in 0..1)
fn quad(t: f32, ease: Ease) -> f32 {
    match ease {
        Ease::In => t * t,
        Ease::Out => -t * (t - 2.0),
        Ease::InOut => {
            let t = t * 2.0;
            if t < 1.0 {
                0.5 * t * t
            } else {
                -0.5 * ((t - 1.0) * (t - 3.0) - 1.0)
            }
        }
    }
}

fn back(t: f32, ease: Ease) -> f32 {
    let s = 1.70158;
    match ease {
        Ease::In => t * t * ((s + 1.0) * t - s),
        Ease::Out => {
            let t = t - 1.0;
            t * t * ((s + 1.0) * t + s) + 1.0
        }
        Ease::InOut => {
            let s = s * 1.525;
            let t = t * 2.0;
            if t < 1.0 {
                0.5 * (t * t * ((s + 1.0) * t - s))
            } else {
                let t = t - 2.0;
                0.5 * (t * t * ((s + 1.0) * t + s) + 2.0)
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Ease {
    In,
    Out,
    InOut,
}

/// A chain of tween steps on one value: (target, duration, easing); returns the value `t` seconds after the start.
fn chain<
    T: Copy + std::ops::Add<Output = T> + std::ops::Sub<Output = T> + std::ops::Mul<f32, Output = T>,
>(
    start: T,
    steps: &[(T, f32, &dyn Fn(f32) -> f32)],
    t: f32,
) -> T {
    let mut from = start;
    let mut at = 0.0;
    for (to, d, f) in steps {
        if t < at + d {
            let k = if *d > 0.0 {
                ((t - at) / d).clamp(0.0, 1.0)
            } else {
                1.0
            };
            return from + (*to - from) * f(k);
        }
        from = *to;
        at += d;
    }
    from
}

/// One running tween on the held item's view node.
#[derive(Clone, Copy, PartialEq)]
pub enum Motion {
    Swing,
    Use,
    TaserKick,
    TorchClick,
}

/// The view node's local offset from its rest position and its rotation (x, z) `t` seconds into `motion`.
pub fn view_offset(id: HeldId, motion: Motion, t: f32) -> (Vec3, f32, f32) {
    let p = pose(id);
    let qio: &dyn Fn(f32) -> f32 = &|k| quad(k, Ease::InOut);
    let qin: &dyn Fn(f32) -> f32 = &|k| quad(k, Ease::In);
    let qout: &dyn Fn(f32) -> f32 = &|k| quad(k, Ease::Out);
    match (motion, p.swing) {
        (Motion::Swing, Some((w, s, r, arc))) => {
            let arc = arc.to_radians();
            let rx = chain(
                0.0,
                &[(arc * 0.70, w, qout), (-arc * 1.15, s, qin), (0.0, r, qout)],
                t,
            );
            let pos = chain(
                Vec3::ZERO,
                &[
                    (vec3(0.02, 0.05, 0.06), w, qio),
                    (vec3(-0.03, -0.05, -0.12), s, qio),
                    (Vec3::ZERO, r, qio),
                ],
                t,
            );
            (pos, rx, 0.0)
        }
        (Motion::Swing, None) | (Motion::Use, _) => {
            let rz = chain(0.0, &[(-0.72, 0.09, qio), (0.0, 0.16, qio)], t);
            let pos = chain(
                Vec3::ZERO,
                &[
                    (vec3(-0.10, 0.04, -0.06), 0.09, qio),
                    (Vec3::ZERO, 0.16, qio),
                ],
                t,
            );
            (pos, 0.0, rz)
        }
        (Motion::TaserKick, _) => {
            let bio: &dyn Fn(f32) -> f32 = &|k| back(k, Ease::InOut);
            let pos = chain(
                Vec3::ZERO,
                &[(vec3(0.0, 0.015, 0.09), 0.07, bio), (Vec3::ZERO, 0.18, bio)],
                t,
            );
            (pos, 0.0, 0.0)
        }
        (Motion::TorchClick, _) => {
            let pos = chain(
                Vec3::ZERO,
                &[
                    (vec3(0.0, -0.055, 0.035), 0.09, qio),
                    (Vec3::ZERO, 0.14, qout),
                ],
                t,
            );
            (pos, 0.0, 0.0)
        }
    }
}

/// How long `motion` runs.
pub fn duration(id: HeldId, motion: Motion) -> f32 {
    match (motion, pose(id).swing) {
        (Motion::Swing, Some((w, s, r, _))) => w + s + r,
        (Motion::Swing, None) | (Motion::Use, _) => 0.25,
        (Motion::TaserKick, _) => 0.25 + 0.28,
        (Motion::TorchClick, _) => 0.23,
    }
}

/// PlayerController3D.viewmodel_push_for / viewmodel_offset
pub fn push_for(wall_distance: f32, reach: f32) -> f32 {
    (reach - wall_distance + 0.04).clamp(0.0, VIEWMODEL_MAX_PUSH)
}

pub fn push_offset(push: f32) -> Vec3 {
    vec3(0.0, -push * 0.25, push)
}

/// The held item's state between frames: the running tween and the eased wall push.
#[derive(Default)]
pub struct HeldState {
    pub motion: Option<(HeldId, Motion, f32)>,
    pub push: f32,
    pub last_swing_t: f32,
    pub last_taser_fx: f32,
    pub last_torch: Option<bool>,
    pub last_time: f32,
}
