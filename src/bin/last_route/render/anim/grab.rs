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

/// The style table and the arm stretch, from assets/hunters/grabs/_styles.json (HunterGrab.arm_stretch, STRETCH_STYLES
/// and GrabStyles.STYLES). The reaching styles lengthen her arms up to `arm_stretch`, eased in with the reach.
const STYLES_ID: &str = "_styles";

/// One row of GrabStyles.STYLES: the room needs before the style fits (0 = the style needs nothing of that).
#[derive(Clone, Debug, PartialEq)]
pub struct StyleRule {
    pub id: String,
    pub ceiling: f32,
    pub back: f32,
    pub hunter_back: f32,
    pub wall: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleTable {
    /// GrabStyles.FALLBACK: picked when no style of a pool fits the room, and the pool of a hunter with no grab file.
    pub fallback: String,
    pub arm_stretch: f32,
    pub stretch_styles: Vec<String>,
    pub styles: Vec<StyleRule>,
}

/// Reads `lr.grab_styles/1`; missing numbers read as 0 (no stretch, no room needed) and a missing row list as no styles.
pub fn parse_styles(v: &serde_json::Value) -> StyleTable {
    let num = |x: &serde_json::Value, k: &str| x[k].as_f64().map(|n| n as f32).unwrap_or(0.0);
    StyleTable {
        fallback: v["fallback"].as_str().unwrap_or_default().to_string(),
        arm_stretch: num(v, "arm_stretch"),
        stretch_styles: v["stretch_styles"]
            .as_array()
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        styles: v["styles"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| {
                        Some(StyleRule {
                            id: s["id"].as_str()?.to_string(),
                            ceiling: num(s, "ceiling"),
                            back: num(s, "back"),
                            hunter_back: num(s, "hunter_back"),
                            wall: num(s, "wall"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// The shipped style table (assets/hunters/grabs/_styles.json), read once.
pub fn style_table() -> &'static StyleTable {
    static TABLE: std::sync::OnceLock<StyleTable> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let v = data(STYLES_ID).unwrap_or(serde_json::Value::Null);
        parse_styles(&v)
    })
}

/// True when `style` is one the data's style table knows.
pub fn is_style(style: &str) -> bool {
    style_table().styles.iter().any(|s| s.id == style)
}

/// True when the style's arms stretch (the data's stretch_styles).
pub fn stretches(style: &str) -> bool {
    style_table().stretch_styles.iter().any(|s| s == style)
}

pub fn stretch_for(style: &str, t: f32) -> f32 {
    if stretches(style) {
        1.0 + (style_table().arm_stretch - 1.0) * smooth(t / grab_timing().reach_seconds)
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

/// Where the timing table lives under data/ (the embedded file's name).
pub const GRAB_TIMING_PATH: &str = "render/grab.json";

/// The grab animation's timings and limits (data/render/grab.json): Godot's HunterGrab values. Seconds are the
/// phases of a grab, `carry_*` the shoulder carry, `ease_seconds` the release ease, `catch_ease` the view's catch
/// ease, the bends and roll limits radians. `head_turn_rate` is k of the head turn's 1 - e^(-k delta) ease.
#[derive(Clone, Debug, PartialEq)]
pub struct GrabTiming {
    pub reach_seconds: f32,
    pub act_seconds: f32,
    pub hold_seconds: f32,
    pub carry_metres: f32,
    pub carry_seconds: f32,
    pub ease_seconds: f32,
    pub catch_ease: f32,
    pub max_roll: f32,
    pub flip_sway: f32,
    pub waist_bend: f32,
    pub squat_bend: f32,
    pub head_turn_rate: f32,
    pub look_away_seconds: f32,
}

impl Default for GrabTiming {
    /// The Standard (Godot) values; also the fallback when the table fails to parse.
    fn default() -> GrabTiming {
        GrabTiming {
            reach_seconds: 0.35,
            act_seconds: 0.7,
            hold_seconds: 0.85,
            carry_metres: 2.4,
            carry_seconds: 3.2,
            ease_seconds: 0.45,
            catch_ease: 0.35,
            max_roll: 0.26,
            flip_sway: 0.17,
            waist_bend: 1.2,
            squat_bend: 1.45,
            head_turn_rate: 7.0,
            look_away_seconds: 0.4,
        }
    }
}

impl GrabTiming {
    /// Parses the table; every number must be present and positive.
    pub fn parse(text: &str) -> Result<GrabTiming, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let num = |k: &str| -> Result<f32, String> {
            v.get(k)
                .and_then(|x| x.as_f64())
                .filter(|x| *x > 0.0)
                .map(|x| x as f32)
                .ok_or_else(|| format!("{k}: must be a positive number"))
        };
        Ok(GrabTiming {
            reach_seconds: num("reach_seconds")?,
            act_seconds: num("act_seconds")?,
            hold_seconds: num("hold_seconds")?,
            carry_metres: num("carry_metres")?,
            carry_seconds: num("carry_seconds")?,
            ease_seconds: num("ease_seconds")?,
            catch_ease: num("catch_ease")?,
            max_roll: num("max_roll")?,
            flip_sway: num("flip_sway")?,
            waist_bend: num("waist_bend")?,
            squat_bend: num("squat_bend")?,
            head_turn_rate: num("head_turn_rate")?,
            look_away_seconds: num("look_away_seconds")?,
        })
    }

    /// A normal grab's whole length: reach, act and hold.
    pub fn total_seconds(&self) -> f32 {
        self.reach_seconds + self.act_seconds + self.hold_seconds
    }
}

/// The table compiled into the executable, read once; a bad table is reported on stderr and Standard is used.
pub fn grab_timing() -> &'static GrabTiming {
    static TABLE: std::sync::OnceLock<GrabTiming> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let text = embedded::data_under(GRAB_TIMING_PATH).next().map(|(_, t)| t).unwrap_or("{}");
        GrabTiming::parse(text).unwrap_or_else(|e| {
            eprintln!("{GRAB_TIMING_PATH}: {e}");
            GrabTiming::default()
        })
    })
}

const BODY_EYE: f32 = 1.612;
const PLAYER_MODELS_SCALE: f32 = 1.067;

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
        grab_timing().carry_seconds
    } else {
        grab_timing().total_seconds()
    }
}

pub struct Room {
    pub ceiling: f32,
    pub back: f32,
    pub hunter_back: f32,
}

fn fits(table: &StyleTable, style: &str, room: &Room) -> bool {
    let Some(rule) = table.styles.iter().find(|s| s.id == style) else {
        return false;
    };
    if rule.ceiling > 0.0 && room.ceiling < rule.ceiling {
        return false;
    }
    if rule.back > 0.0 && room.back < rule.back {
        return false;
    }
    if rule.hunter_back > 0.0 && room.hunter_back < rule.hunter_back {
        return false;
    }
    if rule.wall > 0.0 && (room.back > rule.wall || room.back < 0.25) {
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
        .unwrap_or_else(|| vec![(style_table().fallback.clone(), 1.0)])
}

/// GrabStyles.pick. A creature rig whose pool no longer fits the room falls back to the pounce, not the shoulder grab
/// (GrabStyles.CREATURE_FALLBACK).
pub fn pick(id: &str, roll: f32, room: &Room) -> String {
    let picked = pick_in(pool_of(id), roll, room);
    if picked == style_table().fallback && super::clip::rig(id).is_some_and(|r| r.is_creature()) {
        return CREATURE_FALLBACK.to_string();
    }
    picked
}

/// The style a creature rig falls back to.
pub const CREATURE_FALLBACK: &str = "pounce_pin";

/// GrabStyles.pick over a given pool: the styles the room rules out drop out, then a weighted pick with `roll`.
pub fn pick_in(pool: Vec<(String, f32)>, roll: f32, room: &Room) -> String {
    pick_with(style_table(), pool, roll, room)
}

/// The same pick against a given style table (the shipped one in play; a changed table in tests).
pub fn pick_with(table: &StyleTable, mut pool: Vec<(String, f32)>, roll: f32, room: &Room) -> String {
    pool.sort_by(|a, b| a.0.cmp(&b.0));
    let fitting: Vec<&(String, f32)> = pool
        .iter()
        .filter(|(s, w)| *w > 0.0 && fits(table, s, room))
        .collect();
    if fitting.is_empty() {
        return table.fallback.clone();
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

/// GrabStyles.head_look's easing: the turn eases toward `full` by 1 - e^(-head_turn_rate delta), Standard 7 every frame, the catch frame
/// included (Godot eases from identity there too; snapping to a full look-away in one frame reads as a broken neck).
pub fn ease_head_turn(current: Quat, full: Quat, delta: f32) -> Quat {
    gd_slerp(current, full, 1.0 - (-grab_timing().head_turn_rate * delta).exp())
}

fn look_away(look: &Look, t: f32) -> f32 {
    if look.delay <= 0.0 {
        0.0
    } else {
        1.0 - smooth((t - look.delay) / grab_timing().look_away_seconds)
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
    let r = smooth(t / grab_timing().reach_seconds);
    let a = smooth((t - grab_timing().reach_seconds) / grab_timing().act_seconds);
    let hold = (t - grab_timing().reach_seconds - grab_timing().act_seconds).max(0.0);
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
            let flat = smooth((t - grab_timing().reach_seconds) / (grab_timing().act_seconds + 0.35));
            o.at = vec3(0.0, 0.0, lerp(0.75, 1.25, flat));
            o.tilt = lerp(0.0, PI * 0.5 - 0.1, flat);
            o.arms = vec![('R', "l_shoulder", r), ('L', "r_shoulder", r)];
            let down = smooth(
                (t - grab_timing().reach_seconds - grab_timing().act_seconds * 0.4) / (grab_timing().act_seconds * 0.6 + grab_timing().hold_seconds * 0.4),
            );
            o.hunter = [0.3 * down, 0.3 * down, 0.3 * down];
        }
        "wrist_yank" => {
            let yank = smooth((t - grab_timing().reach_seconds) / (grab_timing().act_seconds * 0.45));
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
            let hoist = smooth((t - grab_timing().reach_seconds) / 0.9);
            let walk = smooth((t - grab_timing().reach_seconds - 1.0) / 1.5);
            let tilt = -(PI - 0.35) * hoist;
            let pelvis = vec3(
                c.shoulder_side,
                c.shoulder_y + 0.05,
                c.shoulder_fwd + walk * grab_timing().carry_metres,
            );
            let carried = pelvis - vec3(0.0, 0.6 * tilt.cos(), 0.6 * tilt.sin());
            let out_side = smooth((t - grab_timing().reach_seconds * 0.5) / 0.35);
            let wide = 1.0 + 0.3 * (PI * hoist.clamp(0.0, 1.0)).sin();
            o.at = vec3(
                carried.x * out_side * wide,
                carried.y * hoist,
                lerp(0.75, carried.z, hoist),
            );
            o.tilt = tilt;
            o.arms = vec![('R', "knees", r)];
            o.hunter = [walk * grab_timing().carry_metres, 0.0, 0.0];
            o.look_ahead = true;
        }
        "leg_grab" => {
            let fall = smooth((t - grab_timing().reach_seconds) / (grab_timing().act_seconds + 0.25));
            let drag = smooth(
                (t - grab_timing().reach_seconds - grab_timing().act_seconds * 0.35)
                    / (grab_timing().act_seconds * 0.65 + grab_timing().hold_seconds * 0.6),
            );
            o.face = -1.0;
            o.at = vec3(0.0, 0.0, lerp(1.25, 0.75, drag));
            o.tilt = lerp(0.25 * r, PI * 0.5 - 0.06, fall);
            let stoop = smooth((t - grab_timing().reach_seconds - 0.2) / (grab_timing().act_seconds * 0.55));
            o.arms = vec![('R', "r_ankle", stoop.max(r * 0.3))];
            o.hunter = [0.0, 0.7 * stoop, 0.45 * stoop];
        }
        "upside_down" => {
            let hoist = smooth((t - grab_timing().reach_seconds) / (grab_timing().act_seconds + 0.65));
            let flip = smooth((t - grab_timing().reach_seconds) / 1.5);
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
                (t - grab_timing().reach_seconds - grab_timing().act_seconds * 0.5) / (grab_timing().act_seconds * 0.5 + grab_timing().hold_seconds * 0.7),
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
        "pounce_pin" => {
            // creature rigs: knocked flat on the back, not snapped flat, the creature leaning over the runner
            let down = smooth((t - grab_timing().reach_seconds) / (grab_timing().act_seconds + 0.4));
            o.at = vec3(0.0, 0.0, lerp(0.9, 1.2, down));
            o.tilt = lerp(0.0, PI * 0.5 - 0.1, down);
            o.hunter = [lerp(0.0, 0.3, a), 0.0, 0.15 * a];
        }
        "wall_pin" => {
            let to_wall = (c.back - 0.2).clamp(0.0, 1.1);
            let shove = smooth((t - grab_timing().reach_seconds) / (grab_timing().act_seconds * 0.5));
            o.at = vec3(c.shoulder_side, 0.3 * shove, 0.75 + to_wall * shove);
            o.view_roll = ((t - grab_timing().reach_seconds - grab_timing().act_seconds * 0.5).max(0.0) * 12.0).sin()
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

/// The shoulder-to-goal distance kept inside what two arm segments can reach, or `None` for non-finite input or
/// segments too short to give an ordered range (so `clamp` can never see min > max).
fn arm_reach(len: f32, upper: f32, lower: f32) -> Option<f32> {
    let lo = (upper - lower).abs() + 0.002;
    let hi = upper + lower - 0.002;
    if !(len.is_finite() && lo.is_finite() && hi.is_finite() && lo <= hi) {
        return None;
    }
    Some(len.clamp(lo, hi))
}

#[cfg(test)]
mod arm_reach_tests {
    use super::arm_reach;

    #[test]
    fn crossed_or_non_finite_bounds_are_refused() {
        assert!(arm_reach(0.5, 0.001, 0.3).is_none());
        assert!(arm_reach(0.5, 0.3, 0.001).is_none());
        assert!(arm_reach(f32::NAN, 0.3, 0.3).is_none());
        assert!(arm_reach(0.5, f32::NAN, 0.3).is_none());
    }

    #[test]
    fn normal_arms_clamp_into_reach() {
        let d = arm_reach(5.0, 0.3, 0.3).unwrap();
        assert!(d <= 0.6 && d > 0.5);
        assert!(arm_reach(0.0, 0.3, 0.3).unwrap() > 0.0);
    }
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
        if !(upper >= 1e-4 && lower >= 1e-4) {
            return;
        }
        let goal = grip.lerp(target, amount.clamp(0.0, 1.0));
        let span = goal - shoulder;
        let Some(distance) = arm_reach(span.length(), upper, lower) else {
            return;
        };
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
        let squat_drop = drop.min(leg * (1.0 - grab_timing().squat_bend.cos()));
        let squat = (1.0 - soft(squat_drop) / leg.max(0.01))
            .clamp(-1.0, 1.0)
            .acos();
        let torso = self.pos(i(2)).distance(self.pos(i(0)));
        let waist = (drop - squat_drop).min(torso * (1.0 - grab_timing().waist_bend.cos()));
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
        let side = along.cross(Vec3::Y).normalize_or_zero();
        let room = probe_room(fac, runner_feet, origin, along, ceiling);
        let id = view.shown_id.clone();
        // HunterGrab.play's force_style (admin_force_grab): LR_GRAB_STYLE=<style> forces one for testing
        let style = std::env::var("LR_GRAB_STYLE")
            .ok()
            .filter(|s| is_style(s))
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
        if stretches(&style) {
            ctx.arm *= style_table().arm_stretch; // she holds the runner at her (stretched) arm's length
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
        let r = smooth(t / grab_timing().reach_seconds);
        let feet = self.origin + self.side * p.at.x + self.along * p.at.z;
        let runner = self.start_runner.lerp(feet, r);
        let basis = body_basis(self.along, p.face, p.tilt);
        let roll = if self.style == "upside_down" {
            PI * p.flip + (p.view_roll - PI * p.flip).clamp(-grab_timing().flip_sway, grab_timing().flip_sway)
        } else {
            p.view_roll.clamp(-grab_timing().max_roll, grab_timing().max_roll)
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
        let scale = rig.draw_scale();
        let turn_k = smooth(t / grab_timing().reach_seconds);
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
        if let Some(head) = body.idx("Head").or_else(|| body.idx("head")) {
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
        let e = smooth(t / grab_timing().catch_ease);
        let eye = self.camera_from.0.lerp(eye, e);
        let dir =
            (self.camera_from.1.lerp((face - eye).normalize_or_zero(), e)).normalize_or_zero();
        // a zero or NaN look (the face on the eye, a bad blend) would give a degenerate view: look along her instead
        let dir = if dir.is_finite() && dir.length_squared() > 0.5 { dir } else { self.along };
        let up = Quat::from_axis_angle(dir, roll * e) * Vec3::Y;
        self.camera = (eye.is_finite() && dir.is_finite() && up.is_finite()).then_some((eye, eye + dir, up));
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
            let w = 1.0 - smooth((now - r.start) / grab_timing().ease_seconds);
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

/// The grab's first-person camera (eye, target, up), while a grab holds the runner. None when no grab runs or its
/// camera is not usable (see `usable_camera`): the caller then keeps the runner's own view.
pub fn camera() -> Option<(Vec3, Vec3, Vec3)> {
    GRAB.with(|g| g.borrow().grab.as_ref().and_then(|x| x.camera)).filter(|c| usable_camera(*c))
}

/// A camera (eye, target, up) a view matrix can be built from: every part finite, a real look distance, an up that is
/// not along the look.
pub fn usable_camera((eye, target, up): (Vec3, Vec3, Vec3)) -> bool {
    let look = target - eye;
    eye.is_finite()
        && target.is_finite()
        && up.is_finite()
        && look.length_squared() > 1.0e-6
        && up.cross(look).length_squared() > 1.0e-8
}

/// Drops the running grab and its release ease: the held hunter was not drawn this frame (culled, hidden or without a
/// model), so its camera would be stale. The grab starts again when she is drawn.
pub fn abandon() {
    GRAB.with(|g| {
        let mut g = g.borrow_mut();
        g.grab = None;
        g.release = None;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_degenerate_grab_camera_is_not_usable() {
        let ok = (vec3(0.0, 1.6, 0.0), vec3(0.0, 1.6, -1.0), Vec3::Y);
        assert!(usable_camera(ok));
        assert!(!usable_camera((ok.0, ok.0, Vec3::Y)), "no look");
        assert!(!usable_camera((vec3(f32::NAN, 1.0, 0.0), ok.1, Vec3::Y)), "NaN eye");
        assert!(!usable_camera((ok.0, ok.1, Vec3::ZERO)), "no up");
        assert!(!usable_camera((ok.0, vec3(0.0, 2.6, 0.0), Vec3::Y)), "up along the look");
    }

    #[test]
    fn grab_timing_data_matches_standard_and_drives_behaviour() {
        let text = include_str!("../../data/render/grab.json");
        let t = GrabTiming::parse(text).expect("grab.json parses");
        assert_eq!(t, GrabTiming::default());
        assert_eq!(grab_timing(), &t);
        assert!((t.total_seconds() - 1.9).abs() < 1e-6);
        // a changed value changes behaviour
        let slow = GrabTiming::parse(&text.replace("\"reach_seconds\": 0.35", "\"reach_seconds\": 0.5")).unwrap();
        assert!((slow.total_seconds() - t.total_seconds() - 0.15).abs() < 1e-5);
        assert!(GrabTiming::parse("{}").is_err());
        assert!(GrabTiming::parse(&text.replace("\"max_roll\": 0.26", "\"max_roll\": 0")).is_err());
    }

    #[test]
    fn ease_head_turn_follows_the_data_rate() {
        let full = Quat::from_rotation_y(1.0);
        let k = 1.0 - (-grab_timing().head_turn_rate * 0.1).exp();
        let want = gd_slerp(Quat::IDENTITY, full, k);
        // compare components: Quat::angle_between is acos of a dot near 1, which only resolves ~5e-4 rad in f32
        let got = ease_head_turn(Quat::IDENTITY, full, 0.1);
        assert!((got.dot(want).abs() - 1.0).abs() < 1e-6, "got {got:?} want {want:?}");
        assert!(got.angle_between(want) < 1e-3);
        assert!(ease_head_turn(Quat::IDENTITY, full, 0.0).angle_between(Quat::IDENTITY) < 1e-6);
        assert!(ease_head_turn(Quat::IDENTITY, full, 10.0).angle_between(full) < 1e-3);
    }

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
        let side = along.cross(Vec3::Y).normalize_or_zero();
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

    /// The 15 grab pools ported from Godot's data/grabs/<id>.json (the ids without a Rust copy before) hold the Standard
    /// weights of the originals: each row is the Godot file's "pool", read as (style, weight).
    #[test]
    fn ported_grab_pools_match_godot_standard() {
        let expected: [(&str, &[(&str, f32)]); 15] = [
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

    /// Every hunter the game ships (data/hunters/<id>.json with the lr.hunter/1 schema) has its own grab file.
    #[test]
    fn every_shipped_hunter_resolves_a_grab_file() {
        let mut ids = vec![];
        for (path, text) in embedded::data_under("hunters/") {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            if v["schema"].as_str() != Some("lr.hunter/1") {
                continue;
            }
            let id = v["id"].as_str().unwrap_or_else(|| panic!("{path} has no id"));
            assert!(
                embedded::asset(&format!("hunters/grabs/{id}.json")).is_some(),
                "shipped hunter {id} has no grab file assets/hunters/grabs/{id}.json"
            );
            ids.push(id.to_string());
        }
        assert!(ids.len() >= 30, "found only {} shipped hunters", ids.len());
    }

    /// Every style a shipped grab pool names is a row of the data's style table, and the fallback is one too.
    #[test]
    fn every_shipped_grab_pool_style_is_in_the_style_table() {
        let table = style_table();
        assert!(is_style(&table.fallback), "fallback {} is not a style", table.fallback);
        for (path, _) in embedded::assets_under("hunters/grabs/") {
            let id = path.trim_start_matches("hunters/grabs/").trim_end_matches(".json");
            if id.starts_with('_') {
                continue;
            }
            for (style, _) in pool_of(id) {
                assert!(is_style(&style), "grab pool for {id} names unknown style {style}");
            }
        }
    }

    /// A hunter with no grab file (or an unreadable one) draws the data's fallback pool and picks the fallback style.
    #[test]
    fn missing_grab_file_falls_back_to_the_default_pool() {
        let fallback = style_table().fallback.clone();
        assert_eq!(pool_of("no_such_hunter"), vec![(fallback.clone(), 1.0)]);
        assert_eq!(pick("no_such_hunter", 0.3, &open_room()), fallback);
        assert_eq!(look_for("no_such_hunter").delay, 0.0);
    }

    /// The shipped _styles.json holds the Standard values the code used before the data move.
    #[test]
    fn standard_style_table_matches_the_original() {
        let table = style_table();
        assert_eq!(table.fallback, "shoulder_grab");
        assert_eq!(table.arm_stretch, 1.6);
        assert_eq!(table.styles.len(), 11, "the ten humanoid rows plus the creature pounce_pin");
        assert_eq!(table.stretch_styles.len(), 9);
        assert!(stretches("wrap") && !stretches("shoulder_carry"));
        let rule = |id: &str| table.styles.iter().find(|s| s.id == id).cloned().unwrap();
        assert_eq!(rule("neck_lift").ceiling, 2.5);
        assert_eq!(rule("upside_down").back, 1.5);
        assert_eq!(rule("shoulder_carry").ceiling, 2.75);
        assert_eq!(rule("shoulder_carry").back, 2.8);
        assert_eq!(rule("collar_drag").hunter_back, 1.6);
        assert_eq!(rule("wall_pin").wall, 1.3);
    }

    /// Changing a value in the style data changes the parsed table and the pick it drives.
    #[test]
    fn style_data_changes_the_pick() {
        let doc = serde_json::json!({
            "fallback": "shoulder_grab",
            "arm_stretch": 2.0,
            "stretch_styles": ["wrap"],
            "styles": [
                { "id": "wrap", "ceiling": 0.0, "back": 0.0, "hunter_back": 0.0, "wall": 0.0 },
                { "id": "tackle", "ceiling": 0.0, "back": 3.0, "hunter_back": 0.0, "wall": 0.0 }
            ]
        });
        let table = parse_styles(&doc);
        assert_eq!(table.arm_stretch, 2.0);
        assert_eq!(table.stretch_styles, vec!["wrap".to_string()]);
        let pool = vec![("wrap".to_string(), 1.0), ("tackle".to_string(), 1.0)];
        // with 2.5 m behind the runner tackle needs 3.0, so only wrap fits
        let room = Room { ceiling: 99.0, back: 2.5, hunter_back: 99.0 };
        assert_eq!(pick_with(&table, pool.clone(), 0.9, &room), "wrap");
        // with 3.5 m both fit and the roll decides (sorted: tackle, then wrap, each weight 1)
        let room = Room { ceiling: 99.0, back: 3.5, hunter_back: 99.0 };
        assert_eq!(pick_with(&table, pool.clone(), 0.3, &room), "tackle");
        assert_eq!(pick_with(&table, pool, 0.9, &room), "wrap");
    }
}
