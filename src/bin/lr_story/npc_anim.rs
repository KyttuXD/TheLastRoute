//! Procedural poses for NPCs, layered on the mannequin rig. Each `NpcAction` has a target pose: a rotation per major
//! bone (degrees, in the parent bone's space) plus a pelvis offset (bbmodel units). The walk uses the rig's own walk
//! clip, so its pose is zero here; every other action sits on top of the idle clip. A change of action blends from the
//! pose the body had to the new pose over `BLEND_SECS` (smoothstep), so nothing snaps.
//!
//! The pose maths needs no window, so it is tested directly. Conventions (the rig's): yaw 0 faces -z, a +x rotation
//! swings a hanging limb forward and tips an upright bone back, a +z rotation of the right arm raises it outward.

use crate::anim::clip::Rig;
use crate::anim::player::Local;
use crate::npc_ai::NpcAction;
use macroquad::prelude::*;

/// Seconds a pose takes to blend into the next one.
pub const BLEND_SECS: f32 = 0.3;

/// The bones a pose moves, in `BONES` order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Pelvis,
    Belly,
    Chest,
    Head,
    LArm,
    LFore,
    RArm,
    RFore,
    LThigh,
    LCalf,
    RThigh,
    RCalf,
}

pub const SLOTS: usize = 12;

/// The rig bone name of each slot.
pub const BONES: [&str; SLOTS] = [
    "Pelvis", "Belly", "Chest", "Head", "L-Arm", "L-Forearm", "R-Arm", "R-Forearm", "L-Thigh", "L-Calf", "R-Thigh", "R-Calf",
];

/// A pose as offsets from the base clip: Euler degrees per slot (XYZ) and the pelvis shift in bbmodel units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PoseDelta {
    pub deg: [Vec3; SLOTS],
    pub pelvis_t: Vec3,
}

impl PoseDelta {
    pub const ZERO: PoseDelta = PoseDelta { deg: [Vec3::ZERO; SLOTS], pelvis_t: Vec3::ZERO };

    fn set(mut self, s: Slot, x: f32, y: f32, z: f32) -> PoseDelta {
        self.deg[s as usize] = vec3(x, y, z);
        self
    }

    /// The straight mix of two poses; `w` 0 is `a`, 1 is `b`.
    pub fn lerp(a: &PoseDelta, b: &PoseDelta, w: f32) -> PoseDelta {
        let w = w.clamp(0.0, 1.0);
        let mut out = *a;
        for k in 0..SLOTS {
            out.deg[k] = a.deg[k].lerp(b.deg[k], w);
        }
        out.pelvis_t = a.pelvis_t.lerp(b.pelvis_t, w);
        out
    }
}

/// Smoothstep of the blend progress: 0 at the start, 1 once `BLEND_SECS` have passed.
pub fn blend_weight(elapsed: f32) -> f32 {
    let x = (elapsed / BLEND_SECS).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Whether the walk clip drives the legs for this action (the pose layer is then zero).
pub fn uses_walk_clip(action: NpcAction, moving: bool) -> bool {
    moving && matches!(action, NpcAction::Walk | NpcAction::Flee | NpcAction::Shelter | NpcAction::Wander)
}

/// The target pose of an action `t` seconds into it (gestures swing with `t`).
pub fn pose_for(action: NpcAction, t: f32) -> PoseDelta {
    use NpcAction as A;
    use Slot::*;
    let z = PoseDelta::ZERO;
    match action {
        A::Idle | A::Walk | A::Flee | A::Shelter | A::Wander => z,
        A::Sit => {
            let mut p = z
                .set(LThigh, 88.0, 0.0, 0.0)
                .set(RThigh, 88.0, 0.0, 0.0)
                .set(LCalf, -88.0, 0.0, 0.0)
                .set(RCalf, -88.0, 0.0, 0.0)
                .set(LArm, 25.0, 0.0, 0.0)
                .set(RArm, 25.0, 0.0, 0.0)
                .set(LFore, 35.0, 0.0, 0.0)
                .set(RFore, 35.0, 0.0, 0.0);
            p.pelvis_t = vec3(0.0, -14.5, 0.0);
            p
        }
        A::Lean => {
            let mut p = z
                .set(Pelvis, 12.0, 0.0, 0.0)
                .set(Head, -8.0, 0.0, 0.0)
                .set(LArm, 35.0, 0.0, -8.0)
                .set(RArm, 35.0, 0.0, 8.0)
                .set(LFore, 95.0, 0.0, 0.0)
                .set(RFore, 95.0, 0.0, 0.0)
                .set(RThigh, -8.0, 0.0, 0.0)
                .set(RCalf, 8.0, 0.0, 0.0);
            p.pelvis_t = vec3(0.0, -1.0, 5.0);
            p
        }
        A::Work => {
            let swing = (t * 6.0).sin();
            z.set(Belly, -10.0, 0.0, 0.0)
                .set(RArm, 70.0 + 55.0 * swing, 0.0, 0.0)
                .set(RFore, 60.0 + 25.0 * swing, 0.0, 0.0)
                .set(LArm, 40.0, 0.0, 0.0)
                .set(LFore, 55.0, 0.0, 0.0)
        }
        A::DrawWater => {
            let s = (t * 3.0).sin();
            z.set(Belly, -15.0 - 5.0 * s, 0.0, 0.0)
                .set(LArm, 70.0 + 35.0 * s, 0.0, 0.0)
                .set(RArm, 70.0 - 35.0 * s, 0.0, 0.0)
                .set(LFore, 40.0, 0.0, 0.0)
                .set(RFore, 40.0, 0.0, 0.0)
        }
        A::Read => z
            .set(Head, -25.0, 0.0, 0.0)
            .set(LArm, 75.0, 0.0, 0.0)
            .set(RArm, 75.0, 0.0, 0.0)
            .set(LFore, 65.0, 0.0, 0.0)
            .set(RFore, 65.0, 0.0, 0.0),
        A::Sleep => {
            let mut p = z.set(Pelvis, 90.0, 0.0, 0.0).set(Chest, 2.0 * (t * 1.5).sin(), 0.0, 0.0);
            p.pelvis_t = vec3(0.0, -33.0, 0.0);
            p
        }
        A::WarmHands => {
            let tremble = 3.0 * (t * 9.0).sin();
            z.set(Head, -8.0, 0.0, 0.0)
                .set(LArm, 75.0 + tremble, 0.0, 0.0)
                .set(RArm, 75.0 - tremble, 0.0, 0.0)
                .set(LFore, 30.0, 0.0, 0.0)
                .set(RFore, 30.0, 0.0, 0.0)
        }
        A::Look => z.set(Head, 0.0, 55.0 * (t * 0.8).sin(), 0.0).set(Chest, 0.0, 10.0 * (t * 0.8).sin(), 0.0),
        A::Wave => z
            .set(RArm, 0.0, 0.0, 150.0)
            .set(RFore, 0.0, 0.0, 25.0 * (t * 9.0).sin())
            .set(Head, 0.0, 0.0, -4.0),
        A::Chat => z
            .set(RArm, 40.0 + 25.0 * (t * 4.0).sin(), 0.0, 0.0)
            .set(RFore, 50.0 + 20.0 * (t * 4.0 + 1.0).sin(), 0.0, 0.0)
            .set(LArm, 20.0 + 10.0 * (t * 3.1).sin(), 0.0, 0.0)
            .set(Head, -4.0 * (t * 5.0).sin(), 12.0 * (t * 1.7).sin(), 0.0),
        A::Cower => {
            let mut p = z
                .set(Belly, -35.0, 0.0, 0.0)
                .set(Head, -15.0, 0.0, 0.0)
                .set(LArm, 150.0, 0.0, 0.0)
                .set(RArm, 150.0, 0.0, 0.0)
                .set(LFore, 40.0, 0.0, 0.0)
                .set(RFore, 40.0, 0.0, 0.0)
                .set(LThigh, 70.0, 0.0, 0.0)
                .set(RThigh, 70.0, 0.0, 0.0)
                .set(LCalf, -110.0, 0.0, 0.0)
                .set(RCalf, -110.0, 0.0, 0.0);
            p.pelvis_t = vec3(0.0, -12.0, 0.0);
            p
        }
    }
}

/// One body's pose blend: the pose it was in when the action last changed, the action it is heading to, and the time.
pub struct PoseBlend {
    action: NpcAction,
    from: PoseDelta,
    /// Seconds since the action changed (drives the blend), and the gesture clock (restarts with the action).
    elapsed: f32,
    clock: f32,
    started: bool,
}

impl Default for PoseBlend {
    fn default() -> Self {
        PoseBlend { action: NpcAction::Idle, from: PoseDelta::ZERO, elapsed: BLEND_SECS, clock: 0.0, started: false }
    }
}

impl PoseBlend {
    /// The pose now: the old pose mixed into the action's pose. Advances time by `dt`. A change of `action` (or of the
    /// walk clip taking over) restarts the blend from whatever was showing.
    pub fn step(&mut self, action: NpcAction, moving: bool, dt: f32) -> PoseDelta {
        let want = if uses_walk_clip(action, moving) { NpcAction::Walk } else { action };
        if !self.started {
            self.started = true;
            self.action = want;
        }
        if want != self.action {
            self.from = self.current();
            self.action = want;
            self.elapsed = 0.0;
            self.clock = 0.0;
        }
        self.elapsed += dt.max(0.0);
        self.clock += dt.max(0.0);
        self.current()
    }

    fn current(&self) -> PoseDelta {
        let target = pose_for(self.action, self.clock);
        PoseDelta::lerp(&self.from, &target, blend_weight(self.elapsed))
    }
}

/// Layers `delta` on the base pose `locals` (rig bone order): each moved bone's rotation is turned by the delta in its
/// parent's space, and the pelvis is shifted. Bones the rig does not have are skipped.
pub fn apply(rig: &Rig, locals: &mut [Local], delta: &PoseDelta) {
    for (slot, name) in BONES.iter().enumerate() {
        let Some(b) = rig.bone(name) else { continue };
        let Some(l) = locals.get_mut(b) else { continue };
        let d = delta.deg[slot];
        if d != Vec3::ZERO {
            l.r = Quat::from_euler(EulerRot::XYZ, d.x.to_radians(), d.y.to_radians(), d.z.to_radians()) * l.r;
        }
        if slot == Slot::Pelvis as usize {
            l.t += delta.pelvis_t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [NpcAction; 16] = [
        NpcAction::Idle,
        NpcAction::Sit,
        NpcAction::Lean,
        NpcAction::Work,
        NpcAction::DrawWater,
        NpcAction::Read,
        NpcAction::Sleep,
        NpcAction::WarmHands,
        NpcAction::Look,
        NpcAction::Wave,
        NpcAction::Chat,
        NpcAction::Wander,
        NpcAction::Walk,
        NpcAction::Flee,
        NpcAction::Shelter,
        NpcAction::Cower,
    ];

    #[test]
    fn blend_weight_runs_zero_to_one_over_the_blend_time() {
        assert_eq!(blend_weight(0.0), 0.0);
        assert_eq!(blend_weight(BLEND_SECS), 1.0);
        assert_eq!(blend_weight(5.0), 1.0);
        assert_eq!(blend_weight(-1.0), 0.0);
        let mid = blend_weight(BLEND_SECS / 2.0);
        assert!((mid - 0.5).abs() < 1e-5);
        assert!(blend_weight(0.1) < blend_weight(0.2));
    }

    #[test]
    fn every_action_has_a_finite_pose() {
        for a in ALL {
            for t in [0.0, 0.7, 3.3] {
                let p = pose_for(a, t);
                assert!(p.deg.iter().all(|v| v.is_finite()) && p.pelvis_t.is_finite(), "{a:?}");
            }
        }
    }

    #[test]
    fn sit_drops_the_hips_and_bends_the_knees_while_idle_is_untouched() {
        assert_eq!(pose_for(NpcAction::Idle, 1.0), PoseDelta::ZERO);
        let s = pose_for(NpcAction::Sit, 0.0);
        assert!(s.pelvis_t.y < -10.0);
        assert!(s.deg[Slot::LThigh as usize].x > 60.0 && s.deg[Slot::LCalf as usize].x < -60.0);
        let sleep = pose_for(NpcAction::Sleep, 0.0);
        assert!(sleep.deg[Slot::Pelvis as usize].x > 80.0, "lying on the back");
    }

    #[test]
    fn work_swings_and_wave_raises_one_arm() {
        let a = pose_for(NpcAction::Work, 0.1).deg[Slot::RArm as usize].x;
        let b = pose_for(NpcAction::Work, 0.35).deg[Slot::RArm as usize].x;
        assert!((a - b).abs() > 5.0, "the hammer arm moves");
        let w = pose_for(NpcAction::Wave, 0.0);
        assert!(w.deg[Slot::RArm as usize].z > 120.0);
        assert_eq!(w.deg[Slot::LArm as usize], Vec3::ZERO);
    }

    #[test]
    fn a_change_of_action_blends_instead_of_snapping() {
        let mut b = PoseBlend::default();
        let idle = b.step(NpcAction::Idle, false, 0.016);
        assert_eq!(idle, PoseDelta::ZERO);
        let first = b.step(NpcAction::Sit, false, 0.016);
        let sit = pose_for(NpcAction::Sit, 0.0);
        assert!(first.pelvis_t.y < 0.0 && first.pelvis_t.y > sit.pelvis_t.y * 0.2, "barely started");
        let mid = b.step(NpcAction::Sit, false, BLEND_SECS / 2.0);
        assert!(mid.pelvis_t.y < first.pelvis_t.y && mid.pelvis_t.y > sit.pelvis_t.y);
        let done = b.step(NpcAction::Sit, false, 1.0);
        assert!((done.pelvis_t.y - sit.pelvis_t.y).abs() < 1e-4);
    }

    #[test]
    fn walking_uses_the_clip_so_the_layer_is_zero_and_stopped_walkers_stand() {
        assert!(uses_walk_clip(NpcAction::Walk, true));
        assert!(!uses_walk_clip(NpcAction::Walk, false));
        assert!(!uses_walk_clip(NpcAction::Sit, true));
        let mut b = PoseBlend::default();
        let p = b.step(NpcAction::Walk, true, 1.0);
        assert_eq!(p, PoseDelta::ZERO);
    }

    #[test]
    fn apply_moves_the_named_bones_of_the_real_rig() {
        let rig = crate::anim::runner::player_rig(false).expect("player rig asset");
        let mut locals: Vec<Local> = rig.bones.iter().map(|b| Local { t: b.rest_t, r: b.rest_r }).collect();
        let before = locals.clone();
        apply(&rig, &mut locals, &pose_for(NpcAction::Sit, 0.0));
        let pelvis = rig.bone("Pelvis").unwrap();
        assert!(locals[pelvis].t.y < before[pelvis].t.y);
        let thigh = rig.bone("L-Thigh").unwrap();
        assert!(locals[thigh].r.dot(before[thigh].r).abs() < 0.99, "the thigh turned");
        assert!(locals.iter().all(|l| l.r.is_finite() && l.t.is_finite()));
    }
}
