//! A hunter's hold on a caught runner (the escape struggle), from game/config/grab_styles.gd and
//! game/hunters/view/hunter_grab.gd:
//!
//! * style: GrabStyles.pick -- the hunter's weighted pool (assets/hunters/grabs/<id>.json = the original's data/grabs),
//!   the styles the room rules out dropped (ceiling, clear floor behind the runner / the hunter, a wall to pin against),
//!   shoulder_grab when nothing fits; the roll is SeededRandom("<seed>:GRAB:<peer>:<catch>").
//! * timeline: GrabStyles.pose(style, t) -- the runner's body (feet offset, lift, tilt, facing), which body points her hands
//!   go to, and her own [forward, drop, lean].
//! * her body: pose_hunter_body (pinned where the grab began, turned square to the runner over the reach, leaned, stepped,
//!   crouched by _crouch's squat + waist bend), reach_correction (3 passes), solve_arm (two-bone IK with the elbow pole and
//!   the upper-arm twist), rest_free_hands, head_look at the camera (Aki's look-away delay from the data).
//! * the first-person view: at the grabbed body's eyes, looking at her face, rolled within the limits, eased in from the
//!   view at the catch (CATCH_EASE).
//! * release: ease_out_grab -- her model and bones ease back to what her clips do over GRAB_EASE_SECONDS (smoothstep).
//!
//! Not ported: the arm stretch (arm_stretch 1.6, a non-uniform bone scale), the camera's wall/collider clearance and the
//! FOV widen, the carry's walk clip, the runner's own first-person body and hands.

use super::clip::{gd_slerp, Rig};
use super::hunter::{model_space_stretched, HunterView, Output};

/// HunterGrab.arm_stretch and STRETCH_STYLES: the reaching styles lengthen her arms up to 1.6x, eased in with the reach.
const ARM_STRETCH: f32 = 1.6;
const STRETCH_STYLES: [&str; 9] = [
    "neck_lift",
    "shoulder_grab",
    "wrist_yank",
    "wall_pin",
    "wrap",
    "collar_drag",
    "leg_grab",
    "upside_down",
    "tackle",
];

pub fn stretch_for(style: &str, t: f32) -> f32 {
    if STRETCH_STYLES.contains(&style) {
        1.0 + (ARM_STRETCH - 1.0) * smooth(t / REACH_SECONDS)
    } else {
        1.0
    }
}
use super::player::Local;
use crate::content::embedded;
use crate::world::rng::Rng;
use crate::world::Facility;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::f32::consts::PI;

pub const REACH_SECONDS: f32 = 0.35;
pub const ACT_SECONDS: f32 = 0.7;
pub const HOLD_SECONDS: f32 = 0.85;
pub const TOTAL_SECONDS: f32 = REACH_SECONDS + ACT_SECONDS + HOLD_SECONDS;
pub const CARRY_METRES: f32 = 2.4;
pub const CARRY_SECONDS: f32 = 3.2;
const BODY_EYE: f32 = 1.612;
const GRAB_EASE_SECONDS: f32 = 0.45;
const CATCH_EASE: f32 = 0.35;
const MAX_ROLL: f32 = 0.26;
const FLIP_SWAY: f32 = 0.17;
const WAIST_BEND: f32 = 1.2;
const SQUAT_BEND: f32 = 1.45;
const PLAYER_MODELS_SCALE: f32 = 1.067;

/// (style, ceiling, back, hunter_back, wall) -- GrabStyles.STYLES needs (99 = none)
const STYLES: [(&str, f32, f32, f32, f32); 10] = [
    ("neck_lift", 2.5, 0.0, 0.0, 0.0),
    ("shoulder_grab", 0.0, 0.0, 0.0, 0.0),
    ("tackle", 0.0, 1.7, 0.0, 0.0),
    ("wrist_yank", 0.0, 0.0, 0.0, 0.0),
    ("wrap", 0.0, 0.0, 0.0, 0.0),
    ("leg_grab", 0.0, 1.8, 0.0, 0.0),
    ("upside_down", 2.4, 1.5, 0.0, 0.0),
    ("shoulder_carry", 2.75, 2.8, 0.0, 0.0),
    ("collar_drag", 0.0, 0.0, 1.6, 0.0),
    ("wall_pin", 0.0, 0.0, 0.0, 1.3),
];

/// GrabStyles.BODY_POINTS: on the runner body, facing -Z, +X its right, feet at 0.
fn body_point(name: &str) -> Vec3 {
    match name {
        "l_shoulder" => vec3(-0.356, 1.506, -0.036),
        "r_shoulder" => vec3(0.356, 1.506, -0.036),
        "l_wrist" => vec3(-0.414, 1.312, -0.5),
        "back_l" => vec3(-0.285, 1.245, 0.095),
        "back_r" => vec3(0.285, 1.245, 0.095),
        "r_ankle" => vec3(0.119, 0.296, 0.119),
        "knees" => vec3(0.0, 0.948, -0.32),
        _ => vec3(0.0, 1.28, -0.202), // neck
    }
}

pub fn smooth(k: f32) -> f32 {
    let k = k.clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

pub fn duration(style: &str) -> f32 {
    if style == "shoulder_carry" {
        CARRY_SECONDS
    } else {
        TOTAL_SECONDS
    }
}

pub struct Room {
    pub ceiling: f32,
    pub back: f32,
    pub hunter_back: f32,
}

fn fits(style: &str, room: &Room) -> bool {
    let Some(&(_, ceiling, back, hunter_back, wall)) = STYLES.iter().find(|s| s.0 == style) else {
        return false;
    };
    if ceiling > 0.0 && room.ceiling < ceiling {
        return false;
    }
    if back > 0.0 && room.back < back {
        return false;
    }
    if hunter_back > 0.0 && room.hunter_back < hunter_back {
        return false;
    }
    if wall > 0.0 && (room.back > wall || room.back < 0.25) {
        return false;
    }
    true
}

pub struct Look {
    pub yaw: f32,
    // "chest" (the chest's share of the turn) is 0 for every port hunter's data, so only the head turns
    pub delay: f32,
}

fn data(id: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(embedded::asset(&format!("hunters/grabs/{id}.json"))?).ok()
}

/// GrabStyles.look_for (the data's "look" over LOOK_DEFAULTS)
pub fn look_for(id: &str) -> Look {
    let d = data(id);
    let l = d.as_ref().map(|d| &d["look"]);
    let g = |k: &str| l.and_then(|l| l[k].as_f64()).map(|x| x as f32);
    Look {
        yaw: g("yaw").map(f32::to_radians).unwrap_or(2.0),
        delay: g("delay").unwrap_or(0.0),
    }
}

/// A hunter's weighted pool from assets/hunters/grabs/<id>.json (style, weight), or shoulder_grab alone when the file is missing.
pub fn pool_of(id: &str) -> Vec<(String, f32)> {
    data(id)
        .and_then(|d| {
            d["pool"].as_object().map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.0) as f32))
                    .collect()
            })
        })
        .unwrap_or_else(|| vec![("shoulder_grab".into(), 1.0)])
}

/// GrabStyles.pick
pub fn pick(id: &str, roll: f32, room: &Room) -> String {
    pick_in(pool_of(id), roll, room)
}

/// GrabStyles.pick over a given pool: the styles the room rules out drop out, then a weighted pick with `roll`.
pub fn pick_in(mut pool: Vec<(String, f32)>, roll: f32, room: &Room) -> String {
    pool.sort_by(|a, b| a.0.cmp(&b.0));
    let fitting: Vec<&(String, f32)> = pool
        .iter()
        .filter(|(s, w)| *w > 0.0 && fits(s, room))
        .collect();
    if fitting.is_empty() {
        return "shoulder_grab".into();
    }
    let total: f32 = fitting.iter().map(|p| p.1).sum();
    let mut at = roll.clamp(0.0, 0.999999) * total;
    for p in &fitting {
        at -= p.1;
        if at < 0.0 {
            return p.0.clone();
        }
    }
    fitting.last().unwrap().0.clone()
}

/// The head's turn from where her face points (`facing`) toward `wanted`, limited to the neck (`limit`, radians):
/// the GrabStyles.head_look rotation, clamped to the neck limit by slerping back toward identity.
pub fn neck_turn(facing: Vec3, wanted: Vec3, limit: f32) -> Quat {
    let full = if facing.angle_between(wanted) > 0.001 {
        Quat::from_rotation_arc(facing, wanted)
    } else {
        Quat::IDENTITY
    };
    let angle = full.angle_between(Quat::IDENTITY);
    if angle > limit {
        gd_slerp(Quat::IDENTITY, full, limit / angle)
    } else {
        full
    }
}

/// GrabStyles.head_look's easing: the turn eases toward `full` by 1 - e^(-7 delta) every frame, the catch frame
/// included (Godot eases from identity there too; snapping to a full look-away in one frame reads as a broken neck).
pub fn ease_head_turn(current: Quat, full: Quat, delta: f32) -> Quat {
    gd_slerp(current, full, 1.0 - (-7.0 * delta).exp())
}

fn look_away(look: &Look, t: f32) -> f32 {
    if look.delay <= 0.0 {
        0.0
    } else {
        1.0 - smooth((t - look.delay) / 0.4)
    }
}

pub struct Ctx {
    pub eye: f32,
    pub shoulder_side: f32,
    pub shoulder_fwd: f32,
    pub shoulder_y: f32,
    pub arm: f32,
    pub back: f32,
}

#[derive(Default)]
pub struct Pose {
    pub at: Vec3,
    pub face: f32,
    pub tilt: f32,
    pub view_roll: f32,
    pub flip: f32,
    pub arms: Vec<(char, &'static str, f32)>,
    pub hunter: [f32; 3],
    pub wide: bool,
    pub look_ahead: bool,
}

/// GrabStyles.pose
pub fn pose(style: &str, t: f32, c: &Ctx) -> Pose {
    let r = smooth(t / REACH_SECONDS);
    let a = smooth((t - REACH_SECONDS) / ACT_SECONDS);
    let hold = (t - REACH_SECONDS - ACT_SECONDS).max(0.0);
    let shake = (t * 9.0).sin() * a * 0.6;
    let sway = (t * 7.0).sin() * a;
    let lerp = |x: f32, y: f32, k: f32| x + (y - x) * k;
    let mut o = Pose {
        at: vec3(0.0, 0.0, 0.75),
        face: 1.0,
        ..Default::default()
    };
    match style {
        "neck_lift" => {
            let grip_y = c.shoulder_y - c.arm * 0.1;
            let lift = (grip_y - body_point("neck").y * c.eye / BODY_EYE).clamp(0.3, 1.2);
            o.at = vec3(
                c.shoulder_side,
                lift * a,
                c.shoulder_fwd + c.arm * 0.95 + 0.22,
            );
            o.view_roll = 0.07 * a + sway * 0.03;
            o.arms = vec![('R', "neck", r)];
        }
        "shoulder_grab" => {
            o.at = vec3(shake * 0.015, 0.08 * a, 1.25);
            o.tilt = shake * 0.06;
            o.view_roll = shake * 0.03;
            o.arms = vec![('R', "l_shoulder", r), ('L', "r_shoulder", r)];
        }
        "tackle" => {
            let flat = smooth((t - REACH_SECONDS) / (ACT_SECONDS + 0.35));
            o.at = vec3(0.0, 0.0, lerp(0.75, 1.25, flat));
            o.tilt = lerp(0.0, PI * 0.5 - 0.1, flat);
            o.arms = vec![('R', "l_shoulder", r), ('L', "r_shoulder", r)];
            let down = smooth(
                (t - REACH_SECONDS - ACT_SECONDS * 0.4) / (ACT_SECONDS * 0.6 + HOLD_SECONDS * 0.4),
            );
            o.hunter = [0.3 * down, 0.3 * down, 0.3 * down];
        }
        "wrist_yank" => {
            let yank = smooth((t - REACH_SECONDS) / (ACT_SECONDS * 0.45));
            o.at = vec3(0.0, 0.0, lerp(1.05, 0.88, yank));
            o.tilt = -0.06 * yank;
            o.view_roll = -0.08 * yank;
            o.arms = vec![('R', "l_wrist", r)];
            o.hunter = [0.0, 0.0, -0.18 * yank];
        }
        "wrap" => {
            o.at = vec3(0.0, 0.1 * a, 1.3);
            o.view_roll = sway * 0.04;
            o.arms = vec![('R', "back_l", r), ('L', "back_r", r)];
            o.wide = true;
        }
        "shoulder_carry" => {
            let hoist = smooth((t - REACH_SECONDS) / 0.9);
            let walk = smooth((t - REACH_SECONDS - 1.0) / 1.5);
            let tilt = -(PI - 0.35) * hoist;
            let pelvis = vec3(
                c.shoulder_side,
                c.shoulder_y + 0.05,
                c.shoulder_fwd + walk * CARRY_METRES,
            );
            let carried = pelvis - vec3(0.0, 0.6 * tilt.cos(), 0.6 * tilt.sin());
            let out_side = smooth((t - REACH_SECONDS * 0.5) / 0.35);
            let wide = 1.0 + 0.3 * (PI * hoist.clamp(0.0, 1.0)).sin();
            o.at = vec3(
                carried.x * out_side * wide,
                carried.y * hoist,
                lerp(0.75, carried.z, hoist),
            );
            o.tilt = tilt;
            o.arms = vec![('R', "knees", r)];
            o.hunter = [walk * CARRY_METRES, 0.0, 0.0];
            o.look_ahead = true;
        }
        "leg_grab" => {
            let fall = smooth((t - REACH_SECONDS) / (ACT_SECONDS + 0.25));
            let drag = smooth(
                (t - REACH_SECONDS - ACT_SECONDS * 0.35)
                    / (ACT_SECONDS * 0.65 + HOLD_SECONDS * 0.6),
            );
            o.face = -1.0;
            o.at = vec3(0.0, 0.0, lerp(1.25, 0.75, drag));
            o.tilt = lerp(0.25 * r, PI * 0.5 - 0.06, fall);
            let stoop = smooth((t - REACH_SECONDS - 0.2) / (ACT_SECONDS * 0.55));
            o.arms = vec![('R', "r_ankle", stoop.max(r * 0.3))];
            o.hunter = [0.0, 0.7 * stoop, 0.45 * stoop];
        }
        "upside_down" => {
            let hoist = smooth((t - REACH_SECONDS) / (ACT_SECONDS + 0.65));
            let flip = smooth((t - REACH_SECONDS) / 1.5);
            o.at = vec3(
                c.shoulder_side,
                lerp(0.0, c.shoulder_y - 0.15, hoist),
                lerp(0.85, c.shoulder_fwd + c.arm * 0.95, hoist),
            );
            o.tilt = PI * hoist;
            o.flip = flip;
            o.view_roll = PI * flip + sway * 0.05;
            o.arms = vec![('R', "r_ankle", r)];
        }
        "collar_drag" => {
            let pull = smooth(
                (t - REACH_SECONDS - ACT_SECONDS * 0.5) / (ACT_SECONDS * 0.5 + HOLD_SECONDS * 0.7),
            );
            o.at = vec3(c.shoulder_side * r, 0.0, lerp(0.9, 0.6, pull));
            o.tilt = -0.9 * a;
            o.arms = vec![('R', "neck", r)];
            o.hunter = [
                (lerp(0.0, -0.9, a) - 0.4 * pull) * PLAYER_MODELS_SCALE / 0.9,
                0.3 * a,
                0.35 * a,
            ];
        }
        "wall_pin" => {
            let to_wall = (c.back - 0.2).clamp(0.0, 1.1);
            let shove = smooth((t - REACH_SECONDS) / (ACT_SECONDS * 0.5));
            o.at = vec3(c.shoulder_side, 0.3 * shove, 0.75 + to_wall * shove);
            o.view_roll = ((t - REACH_SECONDS - ACT_SECONDS * 0.5).max(0.0) * 12.0).sin()
                * 0.04
                * (-hold * 4.0).exp()
                * shove;
            o.arms = vec![('R', "neck", r)];
        }
        _ => {}
    }
    o
}

/// GrabStyles.body_basis: turned to face (or face away from) the hunter, leaned by tilt about its own X.
pub fn body_basis(along: Vec3, face: f32, tilt: f32) -> Mat3 {
    let fwd = -along * face;
    let yaw = (-fwd.x).atan2(-fwd.z);
    Mat3::from_euler(EulerRot::YXZ, yaw, tilt * face, 0.0)
}

// ------------------------------------------------------------------------------------------------- the posed hunter

/// Her bones with a world root, for the world-space edits of hunter_grab.gd.
struct Body<'a> {
    rig: &'a Rig,
    pose: Vec<Local>,
    root: Mat4,
    /// root rotation and uniform scale (world metres per model unit)
    root_r: Quat,
    scale: f32,
    world: Vec<Mat4>,
    /// per-bone lengthening (stretch_arms)
    stretch: Vec<f32>,
}

impl<'a> Body<'a> {
    fn new(rig: &'a Rig, pose: Vec<Local>, root: Mat4, arm_stretch: f32) -> Body<'a> {
        let (s, r, _) = root.to_scale_rotation_translation();
        let mut stretch = vec![1.0; rig.bones.len()];
        for n in ["R-Arm", "R-Forearm", "L-Arm", "L-Forearm"] {
            if let Some(b) = rig.bone(n) {
                stretch[b] = arm_stretch;
            }
        }
        let mut b = Body {
            rig,
            pose,
            root,
            root_r: r,
            scale: s.x,
            world: vec![],
            stretch,
        };
        b.refresh();
        b
    }
    fn refresh(&mut self) {
        self.world = model_space_stretched(self.rig, &self.pose, &self.stretch)
            .into_iter()
            .map(|m| self.root * m)
            .collect();
    }
    fn idx(&self, n: &str) -> Option<usize> {
        self.rig.bone(n)
    }
    fn pos(&self, b: usize) -> Vec3 {
        self.world[b].w_axis.truncate()
    }
    fn rot(&self, b: usize) -> Quat {
        let m = Mat3::from_mat4(self.world[b]);
        Quat::from_mat3(&Mat3::from_cols(
            m.x_axis.normalize(),
            m.y_axis.normalize(),
            m.z_axis.normalize(),
        ))
        .normalize()
    }
    fn parent_rot(&self, b: usize) -> Quat {
        match self.rig.bones[b].parent {
            Some(p) => self.rot(p),
            None => self.root_r,
        }
    }
    fn set_world_rot(&mut self, b: usize, q: Quat) {
        self.pose[b].r = (self.parent_rot(b).inverse() * q).normalize();
        self.refresh();
    }
    /// HunterGrab._turn
    fn turn(&mut self, b: usize, from: Vec3, to: Vec3) {
        if from.length_squared() < 1e-8 || to.length_squared() < 1e-8 {
            return;
        }
        let q = Quat::from_rotation_arc(from.normalize(), to.normalize()) * self.rot(b);
        self.set_world_rot(b, q);
    }
    fn grip(&self, side: char) -> Option<Vec3> {
        if let Some(g) = self.idx(&format!("{side}-HandProp")) {
            return Some(self.pos(g));
        }
        let hand = self.idx(&format!("{side}-Hand"))?;
        let fore = self.idx(&format!("{side}-Forearm"))?;
        let dir = (self.pos(hand) - self.pos(fore)).normalize_or_zero();
        Some(self.pos(hand) + dir * 0.1 * super::hunter::MODEL_SCALE)
    }
    fn reach(&self, side: char) -> f32 {
        let (Some(a), Some(f), Some(g)) = (
            self.idx(&format!("{side}-Arm")),
            self.idx(&format!("{side}-Forearm")),
            self.grip(side),
        ) else {
            return 0.0;
        };
        self.pos(a).distance(self.pos(f)) + self.pos(f).distance(g)
    }
    /// HunterGrab.solve_arm (without the per-frame bend-rate limit)
    fn solve_arm(&mut self, side: char, target: Vec3, amount: f32, pole: Vec3) {
        let (Some(arm), Some(fore)) = (
            self.idx(&format!("{side}-Arm")),
            self.idx(&format!("{side}-Forearm")),
        ) else {
            return;
        };
        let Some(grip) = self.grip(side) else { return };
        if amount <= 0.0 {
            return;
        }
        let shoulder = self.pos(arm);
        let elbow = self.pos(fore);
        let upper = shoulder.distance(elbow);
        let lower = elbow.distance(grip);
        if upper < 1e-4 || lower < 1e-4 {
            return;
        }
        let goal = grip.lerp(target, amount.clamp(0.0, 1.0));
        let span = goal - shoulder;
        let distance = span
            .length()
            .clamp((upper - lower).abs() + 0.002, upper + lower - 0.002);
        let dir = span.normalize_or_zero();
        let mut bend = pole - dir * pole.dot(dir);
        if bend.length_squared() < 1e-6 {
            bend = dir.cross(Vec3::Y).cross(dir);
        }
        let bend = bend.normalize_or_zero();
        let cos_a = ((upper * upper + distance * distance - lower * lower)
            / (2.0 * upper * distance))
            .clamp(-1.0, 1.0);
        let wanted = shoulder + dir * cos_a * upper + bend * (1.0 - cos_a * cos_a).sqrt() * upper;
        self.turn(arm, elbow - shoulder, wanted - shoulder);
        let axis = (wanted - shoulder).normalize_or_zero();
        let plane = axis.cross(goal - wanted);
        if plane.length_squared() > 1e-6 {
            let mut plane = plane.normalize();
            let x = (self.rot(arm) * Vec3::X).normalize();
            let hinge = x - axis * x.dot(axis);
            if hinge.length_squared() > 1e-6 {
                let hinge = hinge.normalize();
                if hinge.dot(plane) < 0.0 {
                    plane = -plane;
                }
                // Vector3.signed_angle_to
                let cr = hinge.cross(plane);
                let twist = cr.length().atan2(hinge.dot(plane))
                    * if cr.dot(axis) < 0.0 { -1.0 } else { 1.0 };
                let q = Quat::from_axis_angle(axis, twist) * self.rot(arm);
                self.set_world_rot(arm, q);
            }
        }
        let g = self.grip(side).unwrap_or(goal);
        let fp = self.pos(fore);
        self.turn(fore, g - fp, goal - fp);
    }
    /// HunterGrab._crouch on the local pose; returns what is left to lower the whole model by.
    fn crouch(&mut self, drop: f32) -> f32 {
        let drop = drop.max(0.0);
        let soft = |d: f32| if d > 0.0 { d * d / (d + 0.05) } else { 0.0 };
        let names = [
            "Belly", "Pelvis", "R-Arm", "L-Thigh", "L-Calf", "L-Foot", "R-Thigh", "R-Calf",
            "R-Foot",
        ];
        let ids: Vec<Option<usize>> = names.iter().map(|n| self.idx(n)).collect();
        if ids.iter().any(|i| i.is_none()) {
            return drop;
        }
        let i = |k: usize| ids[k].unwrap();
        let leg = self.pos(i(3)).distance(self.pos(i(4))) + self.pos(i(4)).distance(self.pos(i(5)));
        let squat_drop = drop.min(leg * (1.0 - SQUAT_BEND.cos()));
        let squat = (1.0 - soft(squat_drop) / leg.max(0.01))
            .clamp(-1.0, 1.0)
            .acos();
        let torso = self.pos(i(2)).distance(self.pos(i(0)));
        let waist = (drop - squat_drop).min(torso * (1.0 - WAIST_BEND.cos()));
        let bend = (1.0 - soft(waist) / torso.max(0.01))
            .clamp(-1.0, 1.0)
            .acos();
        // _offset_bone: turned about X in the parent frame, lowered in the parent's units
        self.pose[i(1)].t.y -= squat_drop / self.scale.max(1e-6);
        let rx = |p: &mut Local, a: f32| p.r = Quat::from_rotation_x(a) * p.r;
        rx(&mut self.pose[i(0)], -bend);
        for (th, ca, fo) in [(3, 4, 5), (6, 7, 8)] {
            rx(&mut self.pose[i(th)], squat);
            rx(&mut self.pose[i(ca)], -2.0 * squat);
            rx(&mut self.pose[i(fo)], squat);
        }
        self.refresh();
        drop - waist - squat_drop
    }
}

/// Where an elbow points for a reach towards `along` (HunterGrab.elbow_pole).
fn elbow_pole(side: char, along: Vec3, wide: bool) -> Vec3 {
    let right = along.cross(Vec3::Y).normalize();
    let out = if wide { 1.6 } else { 0.6 } * if side == 'R' { 1.0 } else { -1.0 };
    Vec3::NEG_Y * if wide { 0.5 } else { 1.0 } + right * out - along * 0.35
}

// ------------------------------------------------------------------------------------------------- the running grab

pub struct Grab {
    pub slot: usize,
    pub hunter_id: String,
    pub style: String,
    pub start: f32,
    origin: Vec3,
    along: Vec3,
    side: Vec3,
    start_runner: Vec3,
    yaw0: f32,
    turn: f32,
    ctx: Ctx,
    look: Look,
    head_turn: Quat,
    pub camera_from: (Vec3, Vec3),
    /// last frame's camera eye and her face (the eye is looked at from, the face looked at)
    pub camera: Option<(Vec3, Vec3, Vec3)>,
    face_point: Option<Vec3>,
}

struct Release {
    slot: usize,
    start: f32,
    pose: Vec<Local>,
    root: (Vec3, Quat, Vec3),
    stretch: f32,
}

#[derive(Default)]
pub struct GrabTable {
    pub grab: Option<Grab>,
    release: Option<Release>,
    catches: u32,
}

thread_local! {
    pub static GRAB: RefCell<GrabTable> = RefCell::new(GrabTable::default());
}

/// HunterGrab.probe_room on the facility grid (walls only; the ceiling is the wall height).
pub fn probe_room(fac: &Facility, runner: Vec3, hunter: Vec3, along: Vec3, ceiling: f32) -> Room {
    let ray = |from: Vec3, dir: Vec3| -> f32 {
        let mut d = 0.0;
        while d < 3.0 {
            let p = from + dir * d;
            if fac.is_wall_cell(p.x.floor() as i32, p.z.floor() as i32) {
                return d;
            }
            d += 0.05;
        }
        3.0
    };
    Room {
        ceiling,
        back: ray(runner, along),
        hunter_back: ray(hunter, -along),
    }
}

impl Grab {
    /// Takes hold: the style, her anchor and the runner's frame (HunterGrab.play).
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        slot: usize,
        view: &HunterView,
        out: &Output,
        runner_feet: Vec3,
        eye: f32,
        fac: &Facility,
        ceiling: f32,
        roll: f32,
        camera: (Vec3, Vec3),
    ) -> Grab {
        let origin = vec3(out.root.w_axis.x, runner_feet.y, out.root.w_axis.z);
        let flat = vec3(runner_feet.x - origin.x, 0.0, runner_feet.z - origin.z);
        let along = if flat.length() > 0.01 {
            flat.normalize()
        } else {
            vec3(-view.yaw.sin(), 0.0, -view.yaw.cos())
        };
        let side = along.cross(Vec3::Y).normalize();
        let room = probe_room(fac, runner_feet, origin, along, ceiling);
        let id = view.shown_id.clone();
        // HunterGrab.play's force_style (admin_force_grab): LR_GRAB_STYLE=<style> forces one for testing
        let style = std::env::var("LR_GRAB_STYLE")
            .ok()
            .filter(|s| STYLES.iter().any(|x| x.0 == s))
            .unwrap_or_else(|| pick(&id, roll, &room));
        // HunterGrab.context: her right shoulder and arm, in the frame from her to the runner
        let rig = view.rig();
        let w = |n: &str| {
            rig.bone(n)
                .map(|b| out.root.transform_point3(out.bones[b].w_axis.truncate()))
        };
        let mut ctx = Ctx {
            eye,
            shoulder_side: -0.45,
            shoulder_fwd: 0.0,
            shoulder_y: 1.8,
            arm: 0.66,
            back: room.back,
        };
        if let (Some(arm), Some(hand)) = (w("R-Arm"), w("R-Hand")) {
            let off = arm - vec3(out.root.w_axis.x, out.root.w_axis.y, out.root.w_axis.z);
            ctx.shoulder_side = off.dot(side);
            ctx.shoulder_fwd = off.dot(along);
            ctx.shoulder_y = off.y;
            ctx.arm = (hand - arm).length().max(0.4);
        }
        if STRETCH_STYLES.contains(&style.as_str()) {
            ctx.arm *= ARM_STRETCH; // she holds the runner at her (stretched) arm's length
        }
        // turned square to the runner: from where her body faces to `along`
        let fwd = vec3(-view.yaw.sin(), 0.0, -view.yaw.cos());
        let cr = fwd.cross(along);
        let turn = cr.length().atan2(fwd.dot(along)) * if cr.y < 0.0 { -1.0 } else { 1.0 };
        Grab {
            slot,
            look: look_for(&id),
            hunter_id: id,
            style,
            start: 0.0,
            origin,
            along,
            side,
            start_runner: runner_feet,
            yaw0: view.yaw,
            turn,
            ctx,
            head_turn: Quat::IDENTITY,
            camera_from: camera,
            camera: None,
            face_point: None,
        }
    }

    /// The runner's body for this moment: (feet with the lift, basis, view roll).
    fn body(&self, p: &Pose, t: f32) -> (Vec3, Mat3, f32) {
        let r = smooth(t / REACH_SECONDS);
        let feet = self.origin + self.side * p.at.x + self.along * p.at.z;
        let runner = self.start_runner.lerp(feet, r);
        let basis = body_basis(self.along, p.face, p.tilt);
        let roll = if self.style == "upside_down" {
            PI * p.flip + (p.view_roll - PI * p.flip).clamp(-FLIP_SWAY, FLIP_SWAY)
        } else {
            p.view_roll.clamp(-MAX_ROLL, MAX_ROLL)
        };
        (runner + Vec3::Y * p.at.y, basis, roll)
    }

    /// Poses her on the runner at `t`: the model placed, crouched and stepped in, arms on the body points, head to the camera.
    pub fn apply(
        &mut self,
        view: &HunterView,
        pose: Vec<Local>,
        t: f32,
        delta: f32,
        clock: f32,
    ) -> (Output, Vec<Local>, (Vec3, Quat, Vec3)) {
        let rig = view.rig();
        let t = t.min(duration(&self.style));
        let p = pose_at(&self.style, t, &self.ctx);
        let (body_feet, basis, roll) = self.body(&p, t);
        let points: Vec<(char, Vec3, f32)> = p
            .arms
            .iter()
            .map(|(s, n, k)| (*s, body_feet + basis * body_point(n), *k))
            .collect();
        let scale = super::hunter::MODEL_SCALE / rig.unit;
        let turn_k = smooth(t / REACH_SECONDS);
        let yaw = self.yaw0 + self.turn * turn_k;
        let mut offsets = p.hunter;
        let stretch = stretch_for(&self.style, t);
        let place = |offsets: [f32; 3], pose: Vec<Local>| -> (Body, (Vec3, Quat, Vec3)) {
            let axis = Vec3::Y.cross(self.along).normalize();
            let rot = Quat::from_axis_angle(axis, offsets[2]) * Quat::from_rotation_y(yaw);
            let at = self.origin + self.along * offsets[0];
            let mut body = Body::new(
                rig,
                pose,
                Mat4::from_scale_rotation_translation(Vec3::splat(scale), rot, at),
                stretch,
            );
            let rest = body.crouch(offsets[1]);
            let mut at = at;
            if rest > 0.0 {
                at.y -= rest;
                body.root = Mat4::from_scale_rotation_translation(Vec3::splat(scale), rot, at);
                body.refresh();
            }
            (body, (at, rot, Vec3::splat(scale)))
        };
        let (mut body, mut root) = place(offsets, pose.clone());
        // reach_correction: step in / crouch until every hand reaches its contact
        for _ in 0..3 {
            let (mut step, mut crouch) = (0.0f32, 0.0f32);
            for (s, pt, k) in &points {
                let Some(arm) = body.idx(&format!("{s}-Arm")) else {
                    continue;
                };
                let reach = body.reach(*s);
                let far = reach * 0.96;
                let k = k.clamp(0.0, 1.0);
                let mut span = *pt - body.pos(arm);
                let below = -span.y;
                if below > far * 0.8 {
                    crouch = crouch.max((below - far * 0.8) * k);
                    span.y += below - far * 0.8;
                }
                let level = span.dot(self.along);
                let across = span.length_squared() - level * level;
                if span.length() > far {
                    step = step.max(
                        if far * far > across {
                            level - (far * far - across).sqrt()
                        } else {
                            level
                        } * k,
                    );
                } else if span.length() < reach * 0.4 {
                    step = step.min((span.length() - reach * 0.4) * k);
                }
            }
            if step.abs() < 0.005 && crouch < 0.005 {
                break;
            }
            offsets[0] += step;
            offsets[1] += crouch;
            (body, root) = place(offsets, pose.clone());
        }
        for (s, pt, k) in &points {
            body.solve_arm(*s, *pt, *k, elbow_pole(*s, self.along, p.wide));
        }
        // rest_free_hands: a free hand on her knee as she crouches
        let amount = smooth(offsets[1] / 0.7);
        if amount > 0.0 {
            for s in ['L', 'R'] {
                if points.iter().any(|p| p.0 == s) {
                    continue;
                }
                if let Some(calf) = body.idx(&format!("{s}-Calf")) {
                    let target = body.pos(calf) + Vec3::Y * 0.12 + self.along * 0.05;
                    body.solve_arm(s, target, amount, elbow_pole(s, self.along, false));
                }
            }
        }
        // head_look: at the camera (lifted toward her head), or ahead for the carry; Aki looks away first
        if let Some(head) = body.idx("Head") {
            let hp = body.pos(head);
            let mut target = match self.camera {
                Some((eye, _, _)) => vec3(eye.x, eye.y.max(hp.y - 0.12), eye.z),
                None => self.camera_from.0,
            };
            if p.look_ahead {
                target = vec3(root.0.x, self.origin.y, root.0.z)
                    + self.along * (4.0 + offsets[0])
                    + Vec3::Y * 1.6;
            }
            let away = look_away(&self.look, t);
            if away > 0.0 {
                let off = hp + (self.side * 1.5 + Vec3::NEG_Y * 0.8 - self.along * 0.3);
                target = target.lerp(off, away);
            }
            let eyes = (body.idx("L-Eye-Attachment"), body.idx("R-Eye-Attachment"));
            let facing = match eyes {
                (Some(l), Some(r)) => (body.pos(l) + body.pos(r)) * 0.5 - hp,
                _ => self.along,
            }
            .normalize_or_zero();
            let wanted = (target - hp).normalize_or_zero();
            let full = neck_turn(facing, wanted, self.look.yaw);
            self.head_turn = ease_head_turn(self.head_turn, full, delta);
            let q = self.head_turn * body.rot(head);
            body.set_world_rot(head, q);
            self.face_point = Some(match eyes {
                (Some(l), Some(r)) => (body.pos(l) + body.pos(r)) * 0.5,
                _ => body.pos(head),
            });
        }
        // the first-person view at the grabbed body's eyes, looking at her face, eased in from the catch
        let mut eye = body_feet + basis * vec3(0.0, self.ctx.eye, 0.0);
        eye.y = eye.y.max(self.start_runner.y + 0.1);
        let face = self.face_point.unwrap_or(eye + self.along);
        let e = smooth(t / CATCH_EASE);
        let eye = self.camera_from.0.lerp(eye, e);
        let dir =
            (self.camera_from.1.lerp((face - eye).normalize_or_zero(), e)).normalize_or_zero();
        let up = Quat::from_axis_angle(dir, roll * e) * Vec3::Y;
        self.camera = Some((eye, eye + dir, up));
        let locals = body.pose.clone();
        let stretched = body.stretch.clone();
        let out = view.finish_with_root(body.pose, body.root, clock, &stretched);
        (out, locals, root)
    }
}

/// The per-bone lengthening for arms stretched `f` times (both upper arms and forearms).
fn arm_stretch_vec(rig: &Rig, f: f32) -> Vec<f32> {
    let mut v = vec![1.0; rig.bones.len()];
    for n in ["R-Arm", "R-Forearm", "L-Arm", "L-Forearm"] {
        if let Some(b) = rig.bone(n) {
            v[b] = f;
        }
    }
    v
}

fn pose_at(style: &str, t: f32, c: &Ctx) -> Pose {
    pose(style, t, c)
}

/// Starts, runs and lets go of the grab for the hunter in `slot`. `held` = the escape struggle holds the runner with her.
/// Returns her posed output when the grab (or its release ease) owns her this frame.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    slot: usize,
    held: bool,
    view: &HunterView,
    pose: Vec<Local>,
    normal: Output,
    now: f32,
    delta: f32,
    seed: u32,
    runner_feet: Vec3,
    eye_height: f32,
    camera: (Vec3, Vec3),
    fac: &Facility,
    ceiling: f32,
) -> Output {
    GRAB.with(|g| {
        let mut g = g.borrow_mut();
        let g = &mut *g;
        if held {
            if g.grab
                .as_ref()
                .map(|x| x.slot != slot || x.hunter_id != view.shown_id)
                .unwrap_or(true)
            {
                g.catches += 1;
                let roll = Rng::new(&format!("{seed}:GRAB:1:{}", g.catches)).next_float() as f32;
                let mut grab = Grab::begin(
                    slot,
                    view,
                    &normal,
                    runner_feet,
                    eye_height,
                    fac,
                    ceiling,
                    roll,
                    camera,
                );
                grab.start = now;
                g.grab = Some(grab);
                g.release = None;
            }
            let grab = g.grab.as_mut().unwrap();
            let t = now - grab.start;
            let stretch = stretch_for(&grab.style, t.min(duration(&grab.style)));
            let (out, locals, root) = grab.apply(view, pose, t, delta, now);
            g.release = Some(Release {
                slot,
                start: f32::NAN,
                pose: locals,
                root,
                stretch,
            });
            return out;
        }
        if g.grab.as_ref().map(|x| x.slot == slot).unwrap_or(false) {
            g.grab = None;
            if let Some(r) = &mut g.release {
                r.start = now;
            }
        }
        // ease_out_grab: from the last held pose back to her clips over GRAB_EASE_SECONDS
        if let Some(r) = g
            .release
            .as_ref()
            .filter(|r| r.slot == slot && !r.start.is_nan())
        {
            let w = 1.0 - smooth((now - r.start) / GRAB_EASE_SECONDS);
            if w < 0.01 {
                g.release = None;
                return normal;
            }
            let rig = view.rig();
            let eased: Vec<Local> = pose
                .iter()
                .zip(&r.pose)
                .map(|(a, b)| Local {
                    t: a.t.lerp(b.t, w),
                    r: gd_slerp(a.r, b.r, w),
                })
                .collect();
            let (ns, nr, nt) = normal.root.to_scale_rotation_translation();
            let root = Mat4::from_scale_rotation_translation(
                ns.lerp(r.root.2, w),
                gd_slerp(nr, r.root.1, w),
                nt.lerp(r.root.0, w),
            );
            let stretch = arm_stretch_vec(rig, 1.0 + (r.stretch - 1.0) * w);
            return view.finish_with_root(eased, root, now, &stretch);
        }
        normal
    })
}

/// The grab's first-person camera (eye, target, up), while a grab holds the runner.
pub fn camera() -> Option<(Vec3, Vec3, Vec3)> {
    GRAB.with(|g| g.borrow().grab.as_ref().and_then(|x| x.camera))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A look-away grab (the head turned to the side and down, away from the runner) never turns the head past
    /// Aki's neck limit, and the limit is Godot's default look yaw (2.0 radians, about 115 degrees).
    #[test]
    fn grab_head_look_stays_within_neck_limit() {
        let look = look_for("aki");
        let limit_deg = look.yaw.to_degrees();
        assert!((look.yaw - 2.0).abs() < 1e-6, "aki neck limit {limit_deg}");
        let facing = vec3(0.0, 0.0, -1.0);
        // the look-away aim: far to one side and down
        for side in [1.0_f32, -1.0] {
            for (x, y, z) in [(1.5 * side, -0.8, 0.0), (3.0 * side, -2.0, 0.5), (1.5 * side, 0.0, 0.0)] {
                let wanted = vec3(x, y, z).normalize();
                let q = neck_turn(facing, wanted, look.yaw);
                let turned = q.angle_between(Quat::IDENTITY);
                assert!(
                    turned <= look.yaw + 1e-4,
                    "head turned {}deg, limit {limit_deg}deg",
                    turned.to_degrees()
                );
            }
        }
        // a small aim is not clamped
        let small = neck_turn(facing, vec3(0.1, 0.0, -1.0).normalize(), look.yaw);
        assert!(small.angle_between(Quat::IDENTITY) < 0.2);
    }

    /// The look-away turn (Aki's delay: she looks away from the catch) eases in from the catch frame like Godot's, never
    /// passes the neck limit on any frame, and reaches its aim.
    #[test]
    fn grab_head_turn_eases_within_neck_limit() {
        let look = look_for("aki");
        let along = vec3(0.0, 0.0, -1.0);
        let facing = along;
        // the look-away aim from grab.rs head_look: off to her side and down, a step back from the runner
        let side = along.cross(Vec3::Y).normalize();
        let off = side * 1.5 + Vec3::NEG_Y * 0.8 - along * 0.3;
        let full = neck_turn(facing, off.normalize(), look.yaw);
        let full_angle = full.angle_between(Quat::IDENTITY);
        assert!(full_angle <= look.yaw + 1e-4);
        let dt = 1.0 / 60.0;
        let mut turn = Quat::IDENTITY;
        let mut first = 0.0;
        for frame in 0..120 {
            turn = ease_head_turn(turn, full, dt);
            let a = turn.angle_between(Quat::IDENTITY);
            if frame == 0 {
                first = a;
            }
            assert!(
                a <= look.yaw + 1e-4,
                "frame {frame}: head turned {}deg, limit {}deg",
                a.to_degrees(),
                look.yaw.to_degrees()
            );
        }
        assert!(
            first < full_angle * 0.5,
            "catch frame turned {}deg of {}deg: the neck snaps",
            first.to_degrees(),
            full_angle.to_degrees()
        );
        assert!(turn.angle_between(full) < 1e-3, "head did not reach the look-away aim");
    }

    fn open_room() -> Room {
        Room {
            ceiling: 99.0,
            back: 99.0,
            hunter_back: 99.0,
        }
    }

    /// The 16 grab pools ported from Godot's data/grabs/<id>.json (the ids without a Rust copy before) hold the Standard
    /// weights of the originals: each row is the Godot file's "pool", read as (style, weight).
    #[test]
    fn ported_grab_pools_match_godot_standard() {
        let expected: [(&str, &[(&str, f32)]); 16] = [
            ("black_wolf_spider", &[("pounce_pin", 6.0)]),
            ("nyxie", &[("wrap", 6.0), ("leg_grab", 2.0), ("upside_down", 2.0)]),
            ("solyth", &[("wrist_yank", 6.0), ("shoulder_grab", 2.0), ("collar_drag", 2.0)]),
            ("white_wolf_spider", &[("pounce_pin", 6.0)]),
            ("void_black", &[("wrap", 6.0), ("upside_down", 2.0), ("shoulder_carry", 2.0)]),
            ("void_blue", &[("shoulder_grab", 6.0), ("wall_pin", 3.0), ("shoulder_carry", 2.0)]),
            ("void_brown", &[("tackle", 6.0), ("shoulder_grab", 2.0)]),
            ("void_cyan", &[("wrist_yank", 6.0), ("tackle", 2.0)]),
            ("void_gray", &[("shoulder_grab", 6.0), ("wall_pin", 2.0), ("collar_drag", 2.0)]),
            ("void_green", &[("leg_grab", 6.0), ("collar_drag", 2.0), ("upside_down", 2.0)]),
            ("void_orange", &[("tackle", 6.0), ("wrist_yank", 2.0)]),
            ("void_pink", &[("wrist_yank", 6.0), ("wrap", 2.0)]),
            ("void_purple", &[("wrap", 6.0), ("shoulder_carry", 3.0)]),
            ("void_red", &[("tackle", 6.0), ("wall_pin", 2.0), ("shoulder_grab", 2.0)]),
            ("void_white", &[("wrist_yank", 6.0), ("upside_down", 3.0)]),
            ("void_yellow", &[("shoulder_grab", 6.0), ("shoulder_carry", 2.0)]),
        ];
        for (id, want) in expected {
            let mut got = pool_of(id);
            got.sort_by(|a, b| a.0.cmp(&b.0));
            let mut want: Vec<(String, f32)> = want.iter().map(|(s, w)| (s.to_string(), *w)).collect();
            want.sort_by(|a, b| a.0.cmp(&b.0));
            assert_eq!(got, want, "grab pool for {id} differs from data/grabs/{id}.json");
        }
    }

    /// Changing a weight in the data changes the pick: the same roll lands on a different style when the weights swap.
    #[test]
    fn grab_pool_weights_change_the_pick() {
        let room = open_room();
        let heavy_wrap = vec![("wrap".to_string(), 6.0), ("tackle".to_string(), 2.0)];
        let heavy_tackle = vec![("wrap".to_string(), 2.0), ("tackle".to_string(), 6.0)];
        assert_eq!(pick_in(heavy_wrap, 0.5, &room), "wrap");
        assert_eq!(pick_in(heavy_tackle, 0.5, &room), "tackle");
    }

    /// The room drops styles the data allows: void_gray's wall pin needs a wall within 1.3 m, so an open room picks
    /// only from shoulder_grab and collar_drag, and the roll decides between them (weights 6 and 2, sorted by style).
    #[test]
    fn grab_room_rules_out_wall_pin_for_void_gray() {
        let room = open_room();
        assert_eq!(pick("void_gray", 0.0, &room), "collar_drag");
        assert_eq!(pick("void_gray", 0.9, &room), "shoulder_grab");
        let wall = Room {
            ceiling: 99.0,
            back: 0.5,
            hunter_back: 99.0,
        };
        // with a wall behind the runner, wall_pin (weight 2) joins the pool; sorted: collar_drag, shoulder_grab, wall_pin
        assert_eq!(pick("void_gray", 0.99, &wall), "wall_pin");
    }

    /// An empty fitting pool falls back to shoulder_grab, the same as GrabStyles.FALLBACK.
    #[test]
    fn grab_no_fitting_style_falls_back_to_shoulder_grab() {
        let cramped = Room {
            ceiling: 0.0,
            back: 0.0,
            hunter_back: 0.0,
        };
        // tackle needs 1.7 m clear behind the runner; a pool of only tackle has nothing that fits
        assert_eq!(pick_in(vec![("tackle".to_string(), 1.0)], 0.5, &cramped), "shoulder_grab");
        // and the shipped void_brown pool (tackle 6, shoulder_grab 2) still picks shoulder_grab in the cramped spot
        assert_eq!(pick("void_brown", 0.5, &cramped), "shoulder_grab");
    }
}
