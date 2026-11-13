//! Planted feet for a standing figure. The port's own: the original plays the idle clips as they are.
//!
//! The rigs' root bone is the pelvis, at the hips. The idle clips sway it (a turn of a degree or two and a small shift),
//! and since the legs hang from it, the whole figure turned about its middle and the feet slid over the floor. Here
//! the pelvis' turn is handed to the bones above it (the belly), so the upper body sways exactly as the clip says, and
//! each thigh is turned so its ankle stays on its spot: the feet are the pivot.
//!
//! [`Stance`] is the spot of each ankle for one clip (read once, when the clip starts to be used); [`Stance::plant`]
//! edits a frame's local pose in place, without allocating.

use super::clip::{gd_slerp, Kind, Rig};
use super::player::Local;
use macroquad::prelude::*;

const PELVIS: &str = "Pelvis";
const LEGS: [[&str; 3]; 2] = [["L-Thigh", "L-Calf", "L-Foot"], ["R-Thigh", "R-Calf", "R-Foot"]];

/// Where a clip stands: the rig's pelvis and leg bones, and each ankle's spot in model space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stance {
    pelvis: usize,
    /// (thigh, calf, foot) of the left and the right leg
    legs: [[usize; 3]; 2],
    feet: [Vec3; 2],
}

/// An ankle (the foot bone's origin) from its hip, in the pelvis' frame.
fn hip_to_ankle(pose: &[Local], leg: [usize; 3]) -> Vec3 {
    pose[leg[0]].r * (pose[leg[1]].t + pose[leg[1]].r * pose[leg[2]].t)
}

impl Stance {
    /// The stance of `clip`: its first frame with the pelvis' rest turn. None for a rig without the humanoid pelvis
    /// and legs (the creature rigs), a pelvis that is not the root, or a clip the rig lacks.
    pub fn of(rig: &Rig, clip: &str) -> Option<Stance> {
        let pelvis = rig.bone(PELVIS)?;
        if rig.bones[pelvis].parent.is_some() {
            return None;
        }
        let mut legs = [[0usize; 3]; 2];
        for (leg, names) in legs.iter_mut().zip(LEGS) {
            let mut parent = pelvis;
            for (slot, name) in leg.iter_mut().zip(names) {
                let bone = rig.bone(name)?;
                if rig.bones[bone].parent != Some(parent) {
                    return None;
                }
                *slot = bone;
                parent = bone;
            }
        }
        let mut pose: Vec<Local> = rig.bones.iter().map(|b| Local { t: b.rest_t, r: b.rest_r }).collect();
        rig.clips.get(clip)?.sample(0.0, |tr, v| match tr.kind {
            Kind::Pos => pose[tr.bone].t = v.truncate(),
            Kind::Rot => pose[tr.bone].r = Quat::from_vec4(v).normalize(),
        });
        let rest = rig.bones[pelvis].rest_r;
        let foot = |leg: [usize; 3]| pose[pelvis].t + rest * (pose[leg[0]].t + hip_to_ankle(&pose, leg));
        let feet = [foot(legs[0]), foot(legs[1])];
        feet.iter().all(|f| f.is_finite()).then_some(Stance { pelvis, legs, feet })
    }

    /// Plants the feet of `pose` (the rig's local pose, in bone order) at weight `w` (0 leaves the pose alone, 1 holds
    /// the ankles on their spots): the pelvis' turn from its rest goes to its other children, each thigh is turned so
    /// the ankle is back on its spot, the foot keeps the turn it had, and the pelvis takes up what the legs could not
    /// reach.
    pub fn plant(&self, rig: &Rig, pose: &mut [Local], w: f32) {
        let w = w.clamp(0.0, 1.0);
        if w <= 0.0 || pose.len() != rig.bones.len() || self.pelvis >= pose.len() {
            return;
        }
        // the pelvis' turn from its rest, in its own frame: taken off it and laid on its children that are not legs
        let turn = (rig.bones[self.pelvis].rest_r.inverse() * pose[self.pelvis].r).normalize();
        let turn = gd_slerp(Quat::IDENTITY, turn, w).normalize();
        pose[self.pelvis].r = (pose[self.pelvis].r * turn.inverse()).normalize();
        for (i, b) in rig.bones.iter().enumerate() {
            if b.parent == Some(self.pelvis) && i != self.legs[0][0] && i != self.legs[1][0] {
                pose[i].r = (turn * pose[i].r).normalize();
            }
        }
        for pass in 0..2 {
            let mut short = Vec3::ZERO;
            let (at, r) = (pose[self.pelvis].t, pose[self.pelvis].r);
            for (leg, spot) in self.legs.iter().zip(self.feet) {
                let [thigh, calf, foot] = *leg;
                let now = hip_to_ankle(pose, *leg);
                let wanted = r.inverse() * (spot - at) - pose[thigh].t;
                if now.length_squared() > 1e-8 && wanted.length_squared() > 1e-8 {
                    let aim = Quat::from_rotation_arc(now.normalize(), wanted.normalize());
                    let old = pose[thigh].r;
                    let new = (gd_slerp(Quat::IDENTITY, aim, w) * old).normalize();
                    pose[thigh].r = new;
                    pose[foot].r = ((new * pose[calf].r).inverse() * old * pose[calf].r * pose[foot].r).normalize();
                }
                short += spot - (at + r * (pose[thigh].t + hip_to_ankle(pose, *leg)));
            }
            if pass == 0 {
                // the legs are a fixed length: the hips move by what both were short of, then the thighs aim again
                pose[self.pelvis].t += short * (0.5 * w);
            }
        }
    }

    /// Each ankle's place in model space for a local pose (what `plant` holds still).
    #[cfg(test)]
    pub fn ankles(&self, pose: &[Local]) -> [Vec3; 2] {
        let (at, r) = (pose[self.pelvis].t, pose[self.pelvis].r);
        [0, 1].map(|k| at + r * (pose[self.legs[k][0]].t + hip_to_ankle(pose, self.legs[k])))
    }

    /// The ankles' spots.
    #[cfg(test)]
    pub fn feet(&self) -> [Vec3; 2] {
        self.feet
    }
}

#[cfg(test)]
mod tests {
    use super::super::clip::rig;
    use super::super::player::AnimPlayer;
    use super::*;

    /// The idle clip's own pose at `t` seconds.
    fn idle_at(r: &Rig, t: f32) -> Vec<Local> {
        let mut p = AnimPlayer::default();
        p.play(r, "idle", 0.0);
        p.seek(t);
        p.pose(r)
    }

    /// A bone's rotation in model space.
    fn model_rot(r: &Rig, pose: &[Local], bone: usize) -> Quat {
        let mut q = pose[bone].r;
        let mut at = r.bones[bone].parent;
        while let Some(p) = at {
            q = pose[p].r * q;
            at = r.bones[p].parent;
        }
        q.normalize()
    }

    /// Over a whole idle loop the clip alone slides the feet (the figure turns about its hips); planted, the ankles
    /// stay on their spots, and the chest and the head keep the turn the clip gave them: the sway is still there.
    #[test]
    fn planted_feet_stay_put_through_the_idle_loop_and_the_body_still_sways() {
        for id in ["aki", "ferryn", "kathy", "xivvy", "player_mannequin", "player_mannequin_slim"] {
            let r = rig(id).unwrap_or_else(|| panic!("{id}: no rig"));
            let stance = Stance::of(&r, "idle").unwrap_or_else(|| panic!("{id}: no stance"));
            let (chest, head) = (r.bone("Chest").unwrap(), r.bone("Head").unwrap());
            let len = r.length("idle");
            let (mut raw, mut planted) = (0.0f32, 0.0f32);
            for k in 0..=48 {
                let mut pose = idle_at(&r, len * k as f32 / 48.0);
                let before = stance.ankles(&pose);
                let (chest0, head0) = (model_rot(&r, &pose, chest), model_rot(&r, &pose, head));
                stance.plant(&r, &mut pose, 1.0);
                assert!(pose.iter().all(|l| l.t.is_finite() && l.r.is_finite()), "{id}: frame {k}");
                let after = stance.ankles(&pose);
                for f in 0..2 {
                    raw = raw.max(before[f].distance(stance.feet()[f]));
                    planted = planted.max(after[f].distance(stance.feet()[f]));
                }
                assert!(model_rot(&r, &pose, chest).dot(chest0).abs() > 0.999_99, "{id}: frame {k} chest turned");
                assert!(model_rot(&r, &pose, head).dot(head0).abs() > 0.999_99, "{id}: frame {k} head turned");
            }
            // bbmodel units: 0.05 is about a millimetre on a hunter
            assert!(planted < 0.05, "{id}: a planted foot moved {planted} units");
            assert!(raw > planted * 4.0, "{id}: the clip alone moved a foot {raw} units, planted {planted}");
        }
    }

    /// Weight 0 leaves the pose alone and a part weight moves the feet part of the way.
    #[test]
    fn the_weight_blends_the_planting_in() {
        let r = rig("ferryn").unwrap();
        let stance = Stance::of(&r, "idle").unwrap();
        let far = |pose: &Vec<Local>| {
            let a = stance.ankles(pose);
            a[0].distance(stance.feet()[0]) + a[1].distance(stance.feet()[1])
        };
        // the frame of the loop where the clip has the feet furthest from their spots
        let len = r.length("idle");
        let worst = (0..=48).map(|k| len * k as f32 / 48.0).fold(0.0, |best, t| if far(&idle_at(&r, t)) > far(&idle_at(&r, best)) { t } else { best });
        let clip = idle_at(&r, worst);
        let mut none = clip.clone();
        stance.plant(&r, &mut none, 0.0);
        assert!(none.iter().zip(&clip).all(|(a, b)| a.t == b.t && a.r == b.r));
        let (mut half, mut full) = (clip.clone(), clip.clone());
        stance.plant(&r, &mut half, 0.5);
        stance.plant(&r, &mut full, 1.0);
        assert!(far(&clip) > 0.1, "the clip slides the feet {}", far(&clip));
        assert!(far(&half) < far(&clip) && far(&full) < far(&half), "{} {} {}", far(&clip), far(&half), far(&full));
    }

    /// A rig without the humanoid legs, or a clip the rig does not have, has no stance.
    #[test]
    fn a_creature_rig_or_a_missing_clip_has_no_stance() {
        assert!(Stance::of(&rig("white_wolf_spider").unwrap(), "idle").is_none());
        assert!(Stance::of(&rig("aki").unwrap(), "no_such_clip").is_none());
        assert!(Stance::of(&rig("kathy").unwrap(), "scythe_idle").is_some());
    }
}
