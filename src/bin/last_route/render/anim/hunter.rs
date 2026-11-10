//! A hunter's presentation, ported from the original's view layer (all paths under game/hunters/view/ unless noted):
//!
//! * clip choice: hunter_view_3d.gd `_update_animation` (1708-1829) -- dormant hold (stun at 0.15), the one-shot
//!   stun_recover after a stun and on waking, flight / charge, sprint while she sees the runner and has breath, chase,
//!   walk for wander/investigate/search, idle; the gait hysteresis (GAIT_ENTER_SPEED / STILL_CLIP_AFTER); travel clips
//!   played by `_play_travel` (0.15 s blend, stride phase kept) at the stride-matched speed of hunter_stride.gd `choose`.
//! * motion sampling: `sync_from_brain` (1361-1418) -- smoothed move speed, teleport guard, still time.
//! * body yaw and head look: hunter_facing.gd (track / steer / update_head) via `_update_body_yaw` (1142) and
//!   `_update_head_look` (1006) / `_head_mode` (1042).
//! * hunter_chase_style.gd: the per-hunter upper-body chase layer (reach2 reach1 pump lope glide skitter stalk stopstart).
//! * `_update_heavy_stun_pose` (1835): the crowbar/axe reel on the model root.
//! * faces: `_desired_expression` / `_update_expression` (1859-1910) on player/blocky_face.gd's cells; stars (1914).

use super::clip::{key_quat, Rig};
use super::hunter_activity_pose::{self as act, Extras};
use super::player::{AnimPlayer, Local};
use super::profile::{self, Profile, StateDef, Weights};
use crate::hunters::brain::State;
use crate::hunters::gimmicks::wolf_spider::Phase;
use crate::hunters::secret_creatures::Rules as SecretRules;
use macroquad::prelude::*;
use std::borrow::Cow;
use std::f32::consts::{PI, TAU};
use std::rc::Rc;

/// HunterRoster.MODEL_SCALE: humanoid hunters are drawn 1.5x their authored glb size.
pub const MODEL_SCALE: f32 = 1.5;
const STILL_CLIP_AFTER: f32 = 0.35;
const STILL_SPEED: f32 = 0.15;
const TELEPORT_DISTANCE: f32 = 4.0;
const GAIT_ENTER_SPEED: f32 = 0.35;
const TRAVEL_BLEND: f32 = 0.15;
const DORMANT_SPEED: f32 = 0.15;
const SCREAM_DISTANCE: f32 = 2.6;
const FACE_HOLD: f32 = 0.45;
const HEAVY_STUN_LEAN: f32 = 0.32;
const HEAVY_STUN_SWAY: f32 = 0.09;
const HEAVY_STUN_SWAY_RATE: f32 = 0.0042;
const HEAVY_STUN_EASE: f32 = 9.0;
/// The corner-peek lean: the figure pitches 0.38 rad toward the edge and shifts 0.1 m with it, so the head (about
/// 1.5 m up) ends 0.7 m ahead of the feet, the distance from her stand to the leaned eye (`hunters::peek`).
const PEEK_LEAN_ANGLE: f32 = 0.38;
const PEEK_LEAN_SHIFT: f32 = 0.1;
const PEEK_LEAN_EASE: f32 = 10.0;

// hunter_stride.gd
const MIN_RATIO: f32 = 0.55;
const MAX_RATIO: f32 = 1.9;
const HARD_MIN: f32 = 0.2;
const HARD_MAX: f32 = 2.6;
const TRAVEL_CLIPS: [&str; 3] = ["walk", "chase", "sprint"];
/// Every clip that is a gait for the profile's stride / arm swing / lean layers (Kathy's scythe walk included).
const TRAVEL_CLIP_NAMES: [&str; 4] = ["walk", "chase", "sprint", "scythe_walk"];

/// What the view reads from the brain each frame (all replicated / public state in the original too).
pub struct Input<'a> {
    pub pos: (f32, f32),
    pub brain_facing: (f32, f32),
    pub active: bool,
    pub stun: f32,
    pub heavy_stun: bool,
    pub state: State,
    pub direct_visible: bool,
    pub winded: bool,
    /// Xivvy airborne
    pub airborne: bool,
    /// Xivvy's height above the floor, metres: her flight altitude plus the float bob (0 on foot)
    pub lift: f32,
    /// Xivvy's float sway, a world offset across her heading (zero on foot)
    pub sway: (f32, f32),
    /// Loxelle's charge (windup + charge)
    pub charging: bool,
    /// brain.presentation_look_target(): last_known while chasing/investigating, the search point while searching
    pub look_target: Option<(f32, f32)>,
    /// brain.path_look_ahead(d), when the brain exposes its route (else the travel direction is used, as on guests)
    pub path_ahead: Option<&'a dyn Fn(f32) -> Option<(f32, f32)>>,
    pub chase_style: &'a str,
    /// The runner, for the scream face (look_target_position in the original is the same last-known point).
    pub camera: Vec3,
    /// A grab has hold of her this frame (chase layer and head look off, pinned).
    pub grabbing: bool,
    /// Rust-side lean (the original plays a Night Watch pose, "corner_peek_left/right"): the way her head leans past a
    /// corner edge while she peeks out (`Brain::peeking_out`), None otherwise.
    pub peek_lean: Option<(f32, f32)>,
    /// The wolf spiders' gimmick state (a creature rig reads it instead of the humanoid clip choice), None for every other hunter.
    pub creature: Option<CreatureInput>,
    /// A named mechanic state the hunter is in (`profile::mechanic_of`: "climb", later "steal_battery", "lunge"...). The
    /// hunter's profile (data/render/hunter_anims.json `states`) says what it looks like; a name it has no row for does nothing.
    pub mechanic: Option<&'a str>,
}

/// What a creature rig's presentation reads from its gimmick (creature_view.gd): the phase picks the clip, a new bark
/// number starts the bark clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CreatureInput {
    pub phase: Phase,
    pub bark_seq: u32,
    /// The runner is within the lunge range of it while it hunts (the attack clip).
    pub target_in_reach: bool,
}

/// The posed model for this frame.
pub struct Output {
    /// Model-space (bbmodel units) transform of every rig bone.
    pub bones: Vec<Mat4>,
    /// World from model space: view (position, yaw) x model (heavy-stun lean, MODEL_SCALE / unit).
    pub root: Mat4,
    /// BlockyFace cell on the face quad.
    pub face_cell: u32,
    pub yaw: f32,
}

/// godot String::hash (djb2) as GDScript's hash() returns it.
pub fn gd_hash(s: &str) -> u32 {
    let mut h: u32 = 5381;
    for c in s.chars() {
        h = h.wrapping_mul(33).wrapping_add(c as u32);
    }
    h
}

fn wrapf(v: f32, lo: f32, hi: f32) -> f32 {
    let r = hi - lo;
    lo + (v - lo).rem_euclid(r)
}

fn move_toward(a: f32, b: f32, d: f32) -> f32 {
    if (b - a).abs() <= d {
        b
    } else {
        a + (b - a).signum() * d
    }
}

fn smoothstep01(w: f32) -> f32 {
    w * w * (3.0 - 2.0 * w)
}

fn v2len(v: (f32, f32)) -> f32 {
    (v.0 * v.0 + v.1 * v.1).sqrt()
}

// ------------------------------------------------------------------------------------------------- HunterFacing

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    None,
    Ahead,
    Scan,
    Lock,
    Sweep,
}

#[derive(Default)]
struct Facing {
    move_dir: (f32, f32),
    speed: f32,
    head_yaw: f32,
    head_pitch: f32,
    weight: f32,
    phase: f32,
    last: (f32, f32),
    has_last: bool,
    still_time: f32,
}

impl Facing {
    const WALK_TURN: f32 = 420.0 * PI / 180.0;
    const RUN_TURN: f32 = 600.0 * PI / 180.0;
    const TURN_GAIN: f32 = 12.0;
    const MIN_HEADING_SPEED: f32 = 0.45;
    const TELEPORT_STEP: f32 = 3.0;
    const MOVING_HOLD: f32 = 0.25;
    const YAW_LIMIT: f32 = 70.0 * PI / 180.0;
    const PITCH_LIMIT: f32 = 35.0 * PI / 180.0;
    const HEAD_DAMP: f32 = 9.0;
    const FADE_SPEED: f32 = 4.0;
    const AHEAD_SWAY: f32 = 5.0 * PI / 180.0;
    const SCAN_SWING: f32 = 9.0 * PI / 180.0;
    const SCAN_RATE: f32 = 0.45;
    const SWEEP_SWING: f32 = 14.0 * PI / 180.0;
    const NOD: f32 = 2.0 * PI / 180.0;

    fn moving(&self) -> bool {
        self.still_time < Self::MOVING_HOLD
            && (self.move_dir.0 * self.move_dir.0 + self.move_dir.1 * self.move_dir.1) > 0.0001
    }

    fn track(&mut self, p: (f32, f32), delta: f32) {
        let frame = delta.clamp(1.0 / 240.0, 0.1);
        if !self.has_last {
            self.has_last = true;
            self.last = p;
            return;
        }
        let step = (p.0 - self.last.0, p.1 - self.last.1);
        self.last = p;
        let d = v2len(step);
        if d > Self::TELEPORT_STEP {
            self.speed = 0.0;
            self.move_dir = (0.0, 0.0);
            self.still_time = Self::MOVING_HOLD;
            return;
        }
        self.speed += (d / frame - self.speed) * 0.3;
        if d / frame > Self::MIN_HEADING_SPEED {
            self.still_time = 0.0;
            let dir = (step.0 / d, step.1 / d);
            let k = (frame * 18.0).clamp(0.0, 1.0);
            self.move_dir = if v2len(self.move_dir).powi(2) < 0.01 {
                dir
            } else {
                (
                    self.move_dir.0 + (dir.0 - self.move_dir.0) * k,
                    self.move_dir.1 + (dir.1 - self.move_dir.1) * k,
                )
            };
        } else {
            self.still_time += frame;
        }
    }

    fn steer(&self, current: f32, dir: (f32, f32), running: bool, delta: f32) -> f32 {
        if dir.0 * dir.0 + dir.1 * dir.1 < 0.0001 {
            return current;
        }
        let frame = delta.clamp(1.0 / 240.0, 0.1);
        let limit = if running {
            Self::RUN_TURN
        } else {
            Self::WALK_TURN
        } * frame;
        let error = wrapf((-dir.0).atan2(-dir.1) - current, -PI, PI);
        let turn = (error * (frame * Self::TURN_GAIN).clamp(0.0, 1.0)).clamp(-limit, limit);
        current + turn
    }

    fn look_ahead_distance(&self) -> f32 {
        (2.0 + self.speed * 0.35).clamp(2.0, 4.0)
    }

    fn update_head(
        &mut self,
        delta: f32,
        mode: Mode,
        body_yaw: f32,
        origin: (f32, f32),
        target: (f32, f32),
        seconds: f32,
    ) {
        let frame = delta.clamp(1.0 / 240.0, 0.1);
        self.weight = move_toward(
            self.weight,
            if mode == Mode::None { 0.0 } else { 1.0 },
            frame * Self::FADE_SPEED,
        );
        let mut yaw = 0.0;
        let mut pitch = 0.0;
        if mode != Mode::None {
            if mode != Mode::Scan {
                let off = (target.0 - origin.0, target.1 - origin.1);
                if off.0 * off.0 + off.1 * off.1 > 0.04 {
                    yaw = wrapf((-off.0).atan2(-off.1) - body_yaw, -PI, PI);
                }
            }
            match mode {
                Mode::Ahead => yaw += (seconds * 0.6 + self.phase).sin() * Self::AHEAD_SWAY,
                Mode::Sweep => yaw += (seconds * 1.1 + self.phase).sin() * Self::SWEEP_SWING,
                Mode::Scan => {
                    yaw = (seconds * Self::SCAN_RATE + self.phase).sin() * Self::SCAN_SWING
                }
                _ => {}
            }
            if mode != Mode::Lock {
                pitch = (seconds * 0.5 + self.phase * 1.3).sin() * Self::NOD;
            }
        }
        yaw = yaw.clamp(-Self::YAW_LIMIT, Self::YAW_LIMIT);
        pitch = pitch.clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
        let blend = 1.0 - (-Self::HEAD_DAMP * frame).exp();
        self.head_yaw += (yaw - self.head_yaw) * blend;
        self.head_pitch += (pitch - self.head_pitch) * blend;
    }
}

// ------------------------------------------------------------------------------------------------- HunterChaseStyle

#[derive(Default)]
struct ChaseStyle {
    weight: f32,
    chase_time: f32,
    stride: f32,
}

const KEEP_ELBOW: f32 = 0.65;
const REACH_KEEP_SWING: f32 = 0.25;
const AIM_LIMIT: f32 = 35.0 * PI / 180.0;
const CHASE_BLEND_SPEED: f32 = 3.0;
const CHASE_STYLES: [&str; 8] = [
    "reach2",
    "reach1",
    "pump",
    "lope",
    "glide",
    "skitter",
    "stalk",
    "stopstart",
];

fn style_base(style: &str) -> &str {
    let name = style.split(':').next().unwrap_or("");
    if CHASE_STYLES.contains(&name) {
        name
    } else {
        "pump"
    }
}

/// Bone access by name on the frame's local pose, with the bind-time rest rotations.
pub(super) struct Bones<'a> {
    pub(super) rig: &'a Rig,
    pub(super) pose: &'a mut [Local],
}

impl<'a> Bones<'a> {
    pub(super) fn new(rig: &'a Rig, pose: &'a mut [Local]) -> Bones<'a> {
        Bones { rig, pose }
    }
}

impl Bones<'_> {
    pub(super) fn now(&self, b: &str) -> Quat {
        self.rig
            .bone(b)
            .map(|i| self.pose[i].r)
            .unwrap_or(Quat::IDENTITY)
    }
    pub(super) fn rest(&self, b: &str) -> Quat {
        self.rig
            .bone(b)
            .map(|i| self.rig.bones[i].rest_r)
            .unwrap_or(Quat::IDENTITY)
    }
    pub(super) fn put(&mut self, b: &str, q: Quat) {
        if let Some(i) = self.rig.bone(b) {
            self.pose[i].r = q;
        }
    }
    /// HunterChaseStyle._swing: scales how far the clip has swung a bone from its rest.
    pub(super) fn swing(&self, b: &str, scale: f32) -> Quat {
        let rest = self.rest(b);
        let now = self.now(b);
        let mut delta = now * rest.inverse();
        if delta.w < 0.0 {
            delta = -delta;
        }
        let angle = 2.0 * delta.w.clamp(-1.0, 1.0).acos();
        if angle < 0.0001 {
            return now;
        }
        // Quaternion.get_axis
        let axis = if delta.w.abs() > 1.0 - 0.00001 {
            vec3(delta.x, delta.y, delta.z)
        } else {
            vec3(delta.x, delta.y, delta.z) / (1.0 - delta.w * delta.w).sqrt()
        };
        Quat::from_axis_angle(axis.normalize(), angle * scale) * rest
    }
}

impl ChaseStyle {
    fn apply(
        &mut self,
        b: &mut Bones,
        style: &str,
        chasing: bool,
        delta: f32,
        aim: f32,
        clock: f32,
    ) {
        use super::clip::gd_slerp as slerp;
        self.weight = move_toward(
            self.weight,
            if chasing { 1.0 } else { 0.0 },
            delta * CHASE_BLEND_SPEED,
        );
        self.chase_time = if chasing {
            self.chase_time + delta
        } else {
            0.0
        };
        if self.weight <= 0.001 {
            return;
        }
        let w = smoothstep01(self.weight);
        let extra = if style.contains(':') {
            style.split(':').nth(1).unwrap_or("")
        } else {
            ""
        };
        let q = |x: f32, y: f32, z: f32| key_quat(vec3(x, y, z) * w);
        match style_base(style) {
            kind @ ("reach2" | "reach1") => {
                let arms: &[&str] = if kind == "reach2" {
                    &["L-Arm", "R-Arm"]
                } else {
                    &["R-Arm"]
                };
                for &arm in arms {
                    let phase = if arm == "L-Arm" { 0.0 } else { 1.7 };
                    let wobble = (clock * 7.0 + phase).sin() * 4.0 * self.stride;
                    let reach =
                        Quat::from_axis_angle(Vec3::Y, aim * w) * q(80.0 + wobble, 0.0, 0.0);
                    let v = reach
                        * slerp(
                            b.rest(arm),
                            b.now(arm),
                            1.0 - w * (1.0 - REACH_KEEP_SWING * self.stride),
                        );
                    b.put(arm, v);
                    let fore = arm.replace("Arm", "Forearm");
                    let clutch = 1.0 + (KEEP_ELBOW - 1.0) * w;
                    let v = b.swing(&fore, clutch);
                    b.put(&fore, v);
                }
            }
            "pump" => match extra {
                "fast" | "big" => {
                    let amount = if extra == "fast" { 1.35 } else { 1.55 };
                    for arm in ["L-Arm", "R-Arm", "L-Forearm", "R-Forearm"] {
                        let v = slerp(b.now(arm), b.swing(arm, amount), w);
                        b.put(arm, v);
                    }
                }
                "fists" => {
                    for fore in ["L-Forearm", "R-Forearm"] {
                        let v = q(38.0, 0.0, 0.0) * b.now(fore);
                        b.put(fore, v);
                    }
                }
                "wings" => {
                    for arm in ["L-Arm", "R-Arm"] {
                        let side = if arm == "L-Arm" { -1.0 } else { 1.0 };
                        let flap = (clock * 9.0).sin() * 8.0;
                        let v = q(10.0, 0.0, (70.0 + flap) * side)
                            * slerp(b.rest(arm), b.now(arm), 1.0 - w);
                        b.put(arm, v);
                    }
                }
                _ => {}
            },
            "lope" => {
                let v = q(-14.0, 0.0, 0.0) * b.now("Chest");
                b.put("Chest", v);
                let v = q(-10.0, 0.0, 0.0) * b.now("Belly");
                b.put("Belly", v);
                let v = q(18.0, 0.0, 0.0) * b.now("Head");
                b.put("Head", v);
                for arm in ["L-Arm", "R-Arm"] {
                    let v = q(15.0, 0.0, 0.0) * slerp(b.now(arm), b.rest(arm), 0.3 * w);
                    b.put(arm, v);
                }
            }
            "glide" => {
                for bone in ["Chest", "Belly"] {
                    let v = slerp(b.now(bone), b.rest(bone), 0.85 * w);
                    b.put(bone, v);
                }
                for arm in ["L-Arm", "R-Arm"] {
                    let v = slerp(b.now(arm), b.rest(arm), 0.55 * w);
                    b.put(arm, v);
                }
                for fore in ["L-Forearm", "R-Forearm"] {
                    let v = slerp(b.now(fore), b.rest(fore), (1.0 - KEEP_ELBOW) * w);
                    b.put(fore, v);
                }
            }
            "skitter" => {
                let v = q(-24.0, 0.0, 0.0) * b.now("Belly");
                b.put("Belly", v);
                let v = q(-18.0, 0.0, 0.0) * b.now("Chest");
                b.put("Chest", v);
                let v = q(32.0, 0.0, 0.0) * b.now("Head");
                b.put("Head", v);
                for arm in ["L-Arm", "R-Arm"] {
                    let fore = arm.replace("Arm", "Forearm");
                    let v = q(40.0, 0.0, 0.0) * slerp(b.now(arm), b.rest(arm), 0.4 * w);
                    b.put(arm, v);
                    let v = q(42.0, 0.0, 0.0) * b.now(&fore);
                    b.put(&fore, v);
                }
            }
            "stalk" => {
                let urge = (self.chase_time / 4.0).clamp(0.0, 1.0);
                let v = q(6.0, 0.0, 18.0 + (6.0 - 18.0) * urge) * b.now("Head");
                b.put("Head", v);
                let v = q(-4.0 + (-16.0 + 4.0) * urge, 0.0, 0.0) * b.now("Chest");
                b.put("Chest", v);
                for arm in ["L-Arm", "R-Arm"] {
                    let v = q(5.0 + 25.0 * urge, 0.0, 0.0)
                        * slerp(b.now(arm), b.rest(arm), (1.0 - urge) * 0.35 * w);
                    b.put(arm, v);
                }
            }
            "stopstart" => {
                let cycle = (clock + (gd_hash(style) % 100) as f32 * 0.01) % 2.1;
                if cycle < 0.5 {
                    let lock = (cycle / 0.5 * PI).sin();
                    for arm in ["L-Arm", "R-Arm"] {
                        let v = slerp(
                            b.now(arm),
                            key_quat(vec3(28.0, 0.0, 0.0)) * b.rest(arm),
                            lock * w,
                        );
                        b.put(arm, v);
                    }
                    let v = key_quat(
                        vec3(0.0, (clock * 47.0).sin() * 9.0, (clock * 31.0).sin() * 6.0)
                            * lock
                            * w,
                    ) * b.now("Head");
                    b.put("Head", v);
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------------------------------------- the view

/// player/blocky_face.gd CELLS and the derp loop.
fn face_cell(expression: &str, clock: f32) -> u32 {
    match expression {
        "blink" => 1,
        "stun" => 2,
        "chase" => 3,
        "scream" => 4,
        "happy" => 5,
        "blank" => 10,
        "derp" => [6, 7, 8, 9][((clock / 0.16).floor() as i64).rem_euclid(4) as usize],
        _ => 0,
    }
}

pub struct HunterView {
    pub shown_id: String,
    rig: Rc<Rig>,
    player: AnimPlayer,
    facing: Facing,
    chase: ChaseStyle,
    pub yaw: f32,
    heading_initialized: bool,
    move_speed: f32,
    still_time: f32,
    speed_primed: bool,
    last_pos: (f32, f32),
    gait_moving: bool,
    stride_scale: f32,
    was_dormant: bool,
    was_stunned: bool,
    recover_left: f32,
    heavy_lean: f32,
    /// 0..1 ease of the corner-peek lean, and the direction it leans in (kept while it eases back out).
    peek_amount: f32,
    peek_toward: (f32, f32),
    /// The face's name: a borrowed one from the data, or the emote's own face when it overrides (owned).
    expression: Cow<'static, str>,
    blink_timer: f32,
    blink_left: f32,
    face_hold: f32,
    scream_latched: bool,
    derp_started: f32,
    blink_rng: u32,
    /// Creature rigs: the bark number last seen, the one-shot clip playing (bark / attack) and its seconds left.
    bark_seen: Option<u32>,
    oneshot: &'static str,
    oneshot_left: f32,
    attack_cooldown: f32,
    /// This hunter's animation profile (data/render/hunter_anims.json).
    profile: &'static Profile,
    /// Layer weights: the gait / idle / live layers follow the clip playing, the state layer its mechanic.
    layers: Weights,
    /// The mechanic state last on (kept while its pose eases out) and how far it is in, 0..1.
    mech: Option<&'static StateDef>,
    mech_w: f32,
    /// Seconds since the mechanic state last came on (a state's `then` phase and `prop_until` read it), and whether it is on.
    mech_t: f32,
    mech_on: bool,
    /// What the scene hands over next to the Input (the brain's activity, the playing emote index).
    extras: Extras,
    /// The activity pose last on (kept while it eases out), its weight 0..1, seconds since it began, and the name now wanted.
    act: Option<&'static act::Def>,
    act_w: f32,
    act_t: f32,
    act_name: Option<&'static str>,
    /// The emote playing (index), its resolved body clip (None = face only), its face, and whether the clip may play on the move.
    emote_seen: Option<usize>,
    emote_clip: Option<String>,
    emote_face: String,
    emote_moving: bool,
    /// Kathy's scythe: this hunter carries it, and the seconds the swing one-shot has left.
    carries_scythe: bool,
    swing_left: f32,
}

impl HunterView {
    pub fn new(shown_id: &str, rig: Rc<Rig>) -> HunterView {
        let phase = (gd_hash(shown_id) % 628) as f32 / 100.0;
        HunterView {
            shown_id: shown_id.to_string(),
            rig,
            player: AnimPlayer::default(),
            facing: Facing {
                phase,
                ..Default::default()
            },
            chase: ChaseStyle {
                stride: 1.0,
                ..Default::default()
            },
            yaw: 0.0,
            heading_initialized: false,
            move_speed: 0.0,
            still_time: 0.0,
            speed_primed: false,
            last_pos: (0.0, 0.0),
            gait_moving: false,
            stride_scale: 1.0,
            was_dormant: false,
            was_stunned: false,
            recover_left: 0.0,
            heavy_lean: 0.0,
            peek_amount: 0.0,
            peek_toward: (0.0, 0.0),
            expression: Cow::Borrowed(""),
            blink_timer: 2.0,
            blink_left: 0.0,
            face_hold: 0.0,
            scream_latched: false,
            derp_started: 0.0,
            blink_rng: gd_hash(shown_id) | 1,
            bark_seen: None,
            oneshot: "",
            oneshot_left: 0.0,
            attack_cooldown: 0.0,
            profile: profile::profile(shown_id),
            layers: Weights::default(),
            mech: None,
            mech_w: 0.0,
            mech_t: 0.0,
            mech_on: false,
            extras: Extras::default(),
            act: None,
            act_w: 0.0,
            act_t: 0.0,
            act_name: None,
            emote_seen: None,
            emote_clip: None,
            emote_face: String::new(),
            emote_moving: false,
            carries_scythe: act::table().carries_scythe(shown_id),
            swing_left: 0.0,
        }
    }

    /// The activity and emote the brain is on this frame (call before `update`).
    pub fn set_extras(&mut self, e: Extras) {
        self.extras = e;
    }

    /// The scythe clip standing in for `clip` on a carrier whose rig has it, else `clip` itself.
    fn scythe_clip<'a>(&self, clip: &'a str) -> Cow<'a, str> {
        if self.carries_scythe {
            if let Some(c) = act::table().scythe.clips.get(clip).filter(|c| self.rig.has_clip(c)) {
                return Cow::Borrowed(c.as_str());
            }
        }
        Cow::Borrowed(clip)
    }

    /// The leg speed scale for `clip` at `speed` (HunterStride.choose's last step).
    fn stride_for(&self, clip: &str, speed: f32) -> f32 {
        let scale = (speed / self.ref_speed(clip)).clamp(HARD_MIN, HARD_MAX);
        if self.rig.stride.get(clip).map(|s| s.1).unwrap_or(false) {
            -scale
        } else {
            scale
        }
    }

    /// The activity, scythe swing and emote state for this frame, before the clip choice reads them.
    fn step_extras(&mut self, i: &Input, delta: f32) {
        let table = act::table();
        let live = i.active && i.stun <= 0.0 && !i.grabbing && !self.rig.is_creature();
        let peek = i.peek_lean.map(|d| act::peek_activity(self.yaw, d));
        let name = if live { self.extras.activity.or(peek) } else { None };
        // a carrier's lunge swings the scythe once
        self.swing_left = (self.swing_left - delta).max(0.0);
        if self.carries_scythe
            && name.is_some()
            && name != self.act_name
            && name == Some(table.scythe.swing_on.as_str())
            && self.rig.has_clip(&table.scythe.swing_clip)
        {
            self.swing_left = self.rig.length(&table.scythe.swing_clip);
        }
        self.act_name = name;
        // a new emote index resolves its clip and face once and queues its voice cue
        let want = if live { self.extras.emote } else { None };
        if want != self.emote_seen {
            self.emote_seen = want;
            self.emote_clip = None;
            self.emote_face.clear();
            if let Some(idx) = want {
                let cat = crate::hunters::emotes::Catalogue::get();
                if let Some(e) = cat.emote(idx) {
                    let rig = self.rig.clone();
                    self.emote_clip = cat.resolve_clip(&self.shown_id, idx, cat.clip_override(&self.shown_id, idx), |c| rig.has_clip(c));
                    self.emote_face = e.face.clone();
                    self.emote_moving = e.moving;
                    act::push_cue(act::EmoteCue { hunter: self.shown_id.clone(), index: idx, id: e.id.clone(), noise: e.noise, pos: i.pos });
                }
            }
        }
    }

    /// The body clip an emote or the scythe swing forces now, if any.
    fn forced_clip(&self) -> Option<String> {
        if self.swing_left > 0.0 {
            return Some(act::table().scythe.swing_clip.clone());
        }
        self.emote_clip.clone().filter(|_| self.emote_moving || self.move_speed < GAIT_ENTER_SPEED)
    }

    /// The activity pose, eased in and out, over the finished clip / head / chase layers.
    fn apply_activity(&mut self, rig: &Rig, pose: &mut [Local], delta: f32, sample: bool) {
        let want = self.act_name.and_then(|n| act::table().find(n));
        if let Some(d) = want {
            if !self.act.is_some_and(|p| std::ptr::eq(p, d)) {
                self.act_t = 0.0;
            }
            self.act = Some(d);
            self.act_t += delta;
        }
        let goal = if want.is_some() { 1.0 } else { 0.0 };
        let secs = self.act.map(|d| if goal > 0.0 { d.blend_in } else { d.blend_out }).unwrap_or(0.0);
        self.act_w = if secs <= 0.0 { goal } else { move_toward(self.act_w, goal, delta / secs) };
        if let Some(d) = self.act {
            if self.act_w <= 0.0 && want.is_none() {
                self.act = None;
            } else if sample {
                act::apply(pose, rig, d, self.act_t, smoothstep01(self.act_w));
            }
        }
    }

    /// Kathy's scythe: its model path and the matrix to draw it with, on the hand bone, for a carrier.
    pub fn scythe_prop(&self, out: &Output) -> Option<(&'static str, Mat4)> {
        if !self.carries_scythe {
            return None;
        }
        let s = &act::table().scythe;
        let hand = self.rig.bone(&s.bone).or_else(|| self.rig.bone(&s.fallback_bone))?;
        Some((s.model.as_str(), act::scythe_transform(s, out.root, *out.bones.get(hand)?, self.rig.draw_scale())))
    }

    pub fn rig(&self) -> &Rig {
        &self.rig
    }

    /// This view with another animation profile (tests; the shipped one comes from data/render/hunter_anims.json).
    #[cfg(test)]
    pub fn with_profile(mut self, p: &'static Profile) -> HunterView {
        self.profile = p;
        self
    }

    /// The speed scale the clip plays at now (tests read it).
    #[cfg(test)]
    pub fn playback_speed(&self) -> f32 {
        self.player.speed_scale
    }

    fn randf(&mut self) -> f32 {
        // xorshift: the blink interval is randf_range(2.2, 5.0) in the original (unseeded)
        let mut x = self.blink_rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.blink_rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// HunterStride.ref_speed: the measured reference at authored size, x MODEL_SCALE.
    fn ref_speed(&self, clip: &str) -> f32 {
        let fallback = match clip {
            "walk" | "scythe_walk" => 1.4,
            "chase" => 3.0,
            "sprint" => 4.2,
            _ => 1.5,
        };
        self.rig
            .stride
            .get(clip)
            .map(|s| s.0)
            .unwrap_or(fallback)
            .max(0.3)
            * if self.rig.is_creature() { 1.0 } else { MODEL_SCALE }
    }

    /// HunterStride.choose
    fn choose(&self, preferred: &str, speed: f32) -> (String, f32) {
        let mut clip = preferred.to_string();
        let ratio = speed / self.ref_speed(preferred);
        if (!(MIN_RATIO..=MAX_RATIO).contains(&ratio)) && TRAVEL_CLIPS.contains(&preferred) {
            let mut best = f32::INFINITY;
            for c in TRAVEL_CLIPS {
                if !self.rig.stride.contains_key(c) {
                    continue;
                }
                let gap = (speed.max(0.05) / self.ref_speed(c)).ln().abs();
                if gap < best {
                    best = gap;
                    clip = c.to_string();
                }
            }
        }
        let mut scale = (speed / self.ref_speed(&clip)).clamp(HARD_MIN, HARD_MAX);
        if self.rig.stride.get(&clip).map(|s| s.1).unwrap_or(false) {
            scale = -scale;
        }
        (clip, scale)
    }

    /// ModelSlot.play_if_available
    fn play_if_available(&mut self, name: &str) {
        if !self.rig.has_clip(name) {
            return;
        }
        if self.player.current_animation() != name || !self.player.playing {
            let rig = self.rig.clone();
            self.player.play(&rig, name, -1.0);
        }
    }

    /// `_play_travel`: blend 0.15 s and keep the stride phase.
    fn play_travel(&mut self, clip: &str) {
        if !self.rig.has_clip(clip) || (self.player.assigned == clip && self.player.playing) {
            return;
        }
        let mut phase = 0.0;
        if self.player.playing
            && ["walk", "chase", "sprint", "scythe_walk"].contains(&self.player.assigned.as_str())
        {
            phase = self.player.position() / self.rig.length(&self.player.assigned).max(0.01);
        }
        let rig = self.rig.clone();
        self.player.play(&rig, clip, TRAVEL_BLEND);
        if phase > 0.0 {
            self.player.seek(phase * self.rig.length(clip));
        }
    }

    fn update_animation(&mut self, i: &Input, delta: f32) {
        // the clip name: a borrowed one from the data (no per-frame string), owned only for a runtime pick (a scythe, an emote)
        let mut clip: Cow<'static, str> = Cow::Borrowed("idle");
        // a mechanic state with a clip of this rig takes the place of the locomotion clips (not of stun / waking)
        let mech = self.mechanic_def(i);
        let mech_clip = mech.and_then(|m| m.clip.as_deref()).filter(|c| self.rig.has_clip(c));
        if !i.active {
            self.was_dormant = true;
            let hold = if self.rig.has_clip("stun") {
                "stun"
            } else {
                "idle"
            };
            let hold = self.scythe_clip(hold);
            self.play_if_available(&hold);
            self.player.speed_scale = DORMANT_SPEED;
            return;
        }
        if self.was_dormant {
            self.was_dormant = false;
            if self.rig.has_clip("stun_recover") {
                self.recover_left = self.rig.length(&self.scythe_clip("stun_recover")) - 0.03;
            }
        }
        if i.stun > 0.0 {
            clip = "stun".into();
            self.was_stunned = true;
            self.recover_left = 0.0;
        } else if self.was_stunned {
            self.was_stunned = false;
            if self.rig.has_clip("stun_recover") {
                self.recover_left = self.rig.length(&self.scythe_clip("stun_recover")) - 0.03;
            }
        }
        if clip != "stun" && self.recover_left > 0.0 {
            self.recover_left -= delta;
            clip = "stun_recover".into();
        } else if i.stun > 0.0 {
        } else if let Some(c) = mech_clip {
            clip = Cow::Borrowed(c);
        } else if i.airborne {
            clip = "flight".into();
        } else if i.charging {
            clip = "charge".into();
        } else if let Some(f) = self.forced_clip() {
            clip = Cow::Owned(f);
        } else {
            match i.state {
                State::Chase => {
                    clip = if i.direct_visible && !i.winded {
                        "sprint"
                    } else {
                        "chase"
                    }
                    .into()
                }
                State::Wander | State::Investigate | State::Search => clip = "walk".into(),
                // a corner peek crouches in place with the lean pose (hunter_activity_poses.json), it does not walk
                State::Peek => {
                    let p = &act::table().peek_clip;
                    clip = if self.rig.has_clip(p) { Cow::Borrowed(p.as_str()) } else { "idle".into() };
                }
                State::Idle => {}
            }
        }
        if mech_clip.is_none() && TRAVEL_CLIPS.contains(&&*clip) {
            if self.gait_moving && self.still_time > STILL_CLIP_AFTER {
                self.gait_moving = false;
            } else if !self.gait_moving && self.move_speed > GAIT_ENTER_SPEED {
                self.gait_moving = true;
            }
            if !self.gait_moving {
                clip = "idle".into();
            }
        }
        if clip == "sprint" && !self.rig.has_clip("sprint") {
            clip = "chase".into();
        }
        let mut speed = 1.0;
        if clip == "flight" {
            speed = (0.85 + self.move_speed * 0.22).clamp(0.85, 2.3); // XivvyFlight.flight_clip_speed
        }
        let travelling = mech_clip.is_none() && TRAVEL_CLIPS.contains(&&*clip);
        if travelling {
            let (pick, mut scale) = self.choose(&clip, self.move_speed);
            if self.rig.has_clip(&pick) {
                clip = Cow::Owned(pick);
            }
            // Kathy's scythe walk has its own stride speed
            let carried = self.scythe_clip(&clip);
            if carried != clip {
                scale = self.stride_for(&carried, self.move_speed);
            }
            self.stride_scale += (scale - self.stride_scale) * (1.0 - (-10.0 * delta).exp());
            // the profile's gait speed: 1.0 keeps the legs matched to the ground
            let gait = if clip == "walk" { self.profile.walk_speed } else { self.profile.chase_speed };
            speed = self.stride_scale * gait;
        }
        let play = self.scythe_clip(&clip);
        if travelling {
            self.play_travel(&play);
        } else if let Some(m) = mech.filter(|_| mech_clip.is_some()) {
            if self.player.current_animation() != &*clip || !self.player.playing {
                let rig = self.rig.clone();
                self.player.play(&rig, &clip, m.blend);
            }
            speed = m.speed;
        } else {
            self.play_if_available(&play);
        }
        self.player.speed_scale = speed;
    }

    /// The profile state the input names, while the hunter is up and about (a stunned, dormant or held one shows none).
    fn mechanic_def(&self, i: &Input) -> Option<&'static StateDef> {
        let name = i.mechanic?;
        if !i.active || i.stun > 0.0 || i.grabbing {
            return None;
        }
        let p: &'static Profile = self.profile;
        p.states.get(name)
    }

    /// CreatureView.tick: the clip comes from the gimmick's phase (idle, walk, sprint on a hunt, a one-shot bark when a new
    /// bark number arrives, an attack lunge at a runner within reach), the legs play at the clip's measured stride speed.
    fn update_creature_animation(&mut self, i: &Input, delta: f32) {
        let view = &SecretRules::get().view;
        self.attack_cooldown = (self.attack_cooldown - delta).max(0.0);
        self.oneshot_left = (self.oneshot_left - delta).max(0.0);
        let Some(c) = i.creature.filter(|_| i.active) else {
            self.play_if_available("idle");
            self.player.speed_scale = 1.0;
            return;
        };
        // a bark that arrived since the last frame (the first sight of a spider already barking plays it too)
        let fresh = match self.bark_seen {
            Some(seen) => seen != c.bark_seq,
            None => c.phase == Phase::Bark,
        };
        self.bark_seen = Some(c.bark_seq);
        if fresh {
            self.start_oneshot("bark");
        }
        let moving = self.move_speed > view.moving_speed && self.still_time < STILL_CLIP_AFTER;
        let mut clip = "idle";
        match c.phase {
            Phase::Hunt => {
                if moving {
                    clip = "sprint";
                }
                if self.attack_cooldown <= 0.0 && c.target_in_reach && self.oneshot_left <= 0.0 {
                    self.start_oneshot("attack");
                    self.attack_cooldown = view.attack_cooldown;
                }
            }
            Phase::Lurk | Phase::Retreat => {
                if moving {
                    clip = "walk";
                }
            }
            Phase::Bark => {}
        }
        if self.oneshot_left > 0.0 {
            let rig = self.rig.clone();
            if self.player.current_animation() != self.oneshot || !self.player.playing {
                self.player.play(&rig, self.oneshot, -1.0);
            }
            self.player.speed_scale = 1.0;
            return;
        }
        let mut speed = 1.0;
        if clip == "walk" || clip == "sprint" {
            // legs at real ground speed over the clip's stride speed (HunterStride), so they do not skate
            let (pick, scale) = self.choose(clip, self.move_speed);
            let pick = if self.rig.has_clip(&pick) { pick } else { clip.to_string() };
            self.stride_scale += (scale - self.stride_scale) * (1.0 - (-10.0 * delta).exp());
            speed = self.stride_scale;
            self.play_travel(&pick);
        } else {
            self.play_if_available(clip);
        }
        self.player.speed_scale = speed;
    }

    fn start_oneshot(&mut self, clip: &'static str) {
        if !self.rig.has_clip(clip) {
            return;
        }
        self.oneshot = clip;
        self.oneshot_left = self.rig.length(clip);
        let rig = self.rig.clone();
        self.player.play(&rig, clip, -1.0);
    }

    /// The clip playing now (tests and the audio cues read it).
    pub fn clip(&self) -> &str {
        self.player.current_animation()
    }

    fn desired_expression(&mut self, i: &Input) -> Cow<'static, str> {
        if !i.active {
            return "neutral".into();
        }
        if i.stun > 0.0 {
            return "stun".into();
        }
        if self.recover_left > 0.0 {
            return "blank".into();
        }
        if self.emote_seen.is_some() && !self.emote_face.is_empty() && act::table().emote_face_override {
            return Cow::Owned(self.emote_face.clone());
        }
        match i.state {
            State::Chase => {
                let gap = i
                    .look_target
                    .map(|t| v2len((i.pos.0 - t.0, i.pos.1 - t.1)))
                    .unwrap_or(f32::INFINITY);
                self.scream_latched =
                    gap < SCREAM_DISTANCE || (self.scream_latched && gap < SCREAM_DISTANCE * 1.4);
                if self.scream_latched {
                    "scream"
                } else {
                    "chase"
                }
                .into()
            }
            State::Search => "derp".into(),
            _ => "neutral".into(),
        }
    }

    fn update_expression(&mut self, i: &Input, delta: f32, clock: f32) {
        let mut wanted = self.desired_expression(i);
        if wanted == "neutral" {
            self.blink_left = (self.blink_left - delta).max(0.0);
            self.blink_timer -= delta;
            if self.blink_timer <= 0.0 {
                self.blink_timer = 2.2 + self.randf() * 2.8;
                self.blink_left = 0.13;
            }
            if self.blink_left > 0.0 {
                wanted = "blink".into();
            }
        }
        self.face_hold = (self.face_hold - delta).max(0.0);
        if wanted == self.expression {
            return;
        }
        let quick = ["blink", "stun", "blank"].contains(&&*wanted)
            || ["blink", "stun", "blank", ""].contains(&&*self.expression);
        if !quick && self.face_hold > 0.0 {
            return;
        }
        self.face_hold = FACE_HOLD;
        if wanted == "derp" {
            self.derp_started = clock;
        }
        self.expression = wanted;
    }

    /// One frame: motion sampling, clip choice, the AnimationPlayer step, then the view's own layers (head look, chase
    /// style, heavy stun lean) in the original's order, and the model-space bone transforms.
    pub fn update(&mut self, i: &Input, delta: f32, clock: f32) -> (Vec<Local>, Output) {
        self.update_lod(i, delta, clock, true)
    }

    /// `update` with the pose sample optional. Every clock, blend and layer weight advances on every call (the state
    /// step); `sample` false skips the pose writes (the layers' bone pose, the activity and mechanic poses, the bone
    /// transforms) and returns an empty pose and output. The scene passes false on a far hunter's frame that shows its
    /// last mesh (data/render/hunters.json lod_bands, skip_beyond), so the state moves on without the pose work.
    pub fn update_lod(&mut self, i: &Input, delta: f32, clock: f32, sample: bool) -> (Vec<Local>, Output) {
        let frame = delta.max(1.0 / 120.0);
        // sync_from_brain: smoothed move speed, teleport guard, still time
        let step = v2len((i.pos.0 - self.last_pos.0, i.pos.1 - self.last_pos.1));
        if !self.speed_primed || step > TELEPORT_DISTANCE {
            self.move_speed = 0.0;
            if !self.speed_primed {
                self.still_time = STILL_CLIP_AFTER + 0.01;
            }
        } else {
            self.move_speed += (step / frame - self.move_speed) * 0.3;
        }
        self.speed_primed = true;
        self.last_pos = i.pos;
        self.still_time = if self.move_speed < STILL_SPEED {
            self.still_time + frame
        } else {
            0.0
        };
        if !i.grabbing {
            self.facing.track(i.pos, delta);
            self.update_body_yaw(i, delta);
        }
        self.step_extras(i, delta);
        if self.rig.is_creature() {
            self.update_creature_animation(i, delta);
        } else {
            self.update_animation(i, delta);
        }
        let rig = self.rig.clone();
        self.player.advance(&rig, delta);
        let mut pose = self.player.pose(&rig);

        // _process: head look, then the chase layer
        let live = i.active && i.stun <= 0.0 && self.recover_left <= 0.0 && !i.grabbing;
        let mut mode = Mode::None;
        let mut target = (0.0, 0.0);
        if live {
            mode = match i.state {
                State::Chase if i.look_target.is_some() => Mode::Lock,
                State::Investigate | State::Search if i.look_target.is_some() => Mode::Sweep,
                _ => {
                    if self.facing.moving() {
                        Mode::Ahead
                    } else {
                        Mode::Scan
                    }
                }
            };
            if mode == Mode::Lock || mode == Mode::Sweep {
                target = i.look_target.unwrap_or_default();
            } else if mode == Mode::Ahead {
                let d = self.facing.look_ahead_distance();
                let straight = i.airborne || i.charging;
                let ahead = if straight {
                    None
                } else {
                    i.path_ahead.and_then(|f| f(d))
                };
                match ahead {
                    Some(p) => target = p,
                    None if self.facing.moving() => {
                        let m = self.facing.move_dir;
                        let l = v2len(m).max(1e-6);
                        target = (i.pos.0 + m.0 / l * d, i.pos.1 + m.1 / l * d);
                    }
                    None => mode = Mode::Scan,
                }
            }
        }
        let body = vec3(i.pos.0, 0.0, i.pos.1);
        if (i.camera - body).length_squared() > 40.0 * 40.0 {
            self.facing.weight = 0.0;
        } else {
            self.facing
                .update_head(delta, mode, self.yaw, i.pos, target, clock);
            if let Some(h) = rig.bone("Head").filter(|_| sample) {
                let w = self.facing.weight;
                let look = Quat::from_axis_angle(Vec3::Y, self.facing.head_yaw * w)
                    * Quat::from_axis_angle(Vec3::X, self.facing.head_pitch * w);
                pose[h].r = look * pose[h].r;
            }
        }
        // the profile's pose layers (stride, arm swing, idle sway, lean, crouch, head tilt) under the chase layer, which
        // sets the arms of its own style afterwards
        if !self.rig.is_creature() {
            let k = 1.0 - (-profile::LAYER_RATE * delta).exp();
            let clip = self.player.current_animation();
            let on = |names: &[&str]| if self.player.playing && names.contains(&clip) { 1.0 } else { 0.0 };
            let (gait, idle) = (on(&TRAVEL_CLIP_NAMES), on(&["idle", "scythe_idle"]));
            let l = &mut self.layers;
            l.gait += (gait - l.gait) * k;
            l.idle += (idle - l.idle) * k;
            l.live += ((live as i32 as f32) - l.live) * k;
            l.chase = self.chase.weight;
            if sample && !self.profile.is_identity_pose() {
                profile::apply(self.profile, &mut Bones::new(&rig, &mut pose), self.layers);
            }
        }
        let chasing = i.active
            && i.state == State::Chase
            && i.stun <= 0.0
            && self.recover_left <= 0.0
            && !i.airborne
            && !i.grabbing;
        let mut aim = 0.0;
        if let Some(t) = i
            .look_target
            .filter(|_| matches!(style_base(i.chase_style), "reach2" | "reach1"))
        {
            let fwd = (-self.yaw.sin(), -self.yaw.cos());
            let to = (t.0 - i.pos.0, t.1 - i.pos.1);
            let l = v2len(to);
            if l > 0.1 {
                let to = (to.0 / l, to.1 / l);
                // Vector2.angle_to
                aim = (fwd.0 * to.1 - fwd.1 * to.0)
                    .atan2(fwd.0 * to.0 + fwd.1 * to.1)
                    .clamp(-AIM_LIMIT, AIM_LIMIT);
            }
        }
        self.chase.stride = move_toward(
            self.chase.stride,
            if self.gait_moving { 1.0 } else { 0.0 },
            delta * CHASE_BLEND_SPEED,
        );
        self.chase.apply(
            &mut Bones {
                rig: &rig,
                pose: &mut pose,
            },
            i.chase_style,
            chasing,
            delta,
            -aim,
            clock,
        );
        // the activity pose (lean, sniff, sweep, push, knock, listen, peek, crawl, lunge) over the clips and the chase layer
        if !self.rig.is_creature() {
            self.apply_activity(&rig, &mut pose, delta, sample);
        }
        // a mechanic state's bone pose over everything the clips and the chase layer did, eased in and out
        let want = self.mechanic_def(i);
        if let Some(m) = want {
            // a state that comes on (or one that replaces another) starts its timeline again
            if !self.mech_on || self.mech.map_or(true, |p| !std::ptr::eq(p, m)) {
                self.mech_t = 0.0;
            }
            self.mech = Some(m);
            self.mech_t += delta;
        }
        self.mech_on = want.is_some();
        let goal = if want.is_some() { 1.0 } else { 0.0 };
        let secs = self.mech.map(|m| if goal > 0.0 { m.blend_in } else { m.blend_out }).unwrap_or(0.0);
        self.mech_w = if secs <= 0.0 { goal } else { move_toward(self.mech_w, goal, delta / secs) };
        if let Some(m) = self.mech {
            if self.mech_w <= 0.0 && want.is_none() {
                self.mech = None;
            } else if sample {
                profile::apply_state(m, &mut Bones::new(&rig, &mut pose), smoothstep01(self.mech_w), self.mech_t);
            }
        }
        self.update_expression(i, delta, clock);

        // _update_heavy_stun_pose (model root pitch)
        let reeling = i.heavy_stun && i.stun > 0.0 && i.active && !i.airborne && !i.grabbing;
        let goal = if reeling {
            HEAVY_STUN_LEAN + (clock * 1000.0 * HEAVY_STUN_SWAY_RATE).sin() * HEAVY_STUN_SWAY
        } else {
            0.0
        };
        self.heavy_lean += (goal - self.heavy_lean) * (1.0 - (-HEAVY_STUN_EASE * delta).exp());
        if !reeling && self.heavy_lean.abs() <= 0.0005 {
            self.heavy_lean = 0.0;
        }
        // the corner-peek lean: tilt toward the edge, easing in and out
        let leaning = i.peek_lean.filter(|_| i.active && i.stun <= 0.0 && !i.grabbing);
        if let Some(d) = leaning {
            self.peek_toward = d;
        }
        let goal = if leaning.is_some() { 1.0 } else { 0.0 };
        self.peek_amount += (goal - self.peek_amount) * (1.0 - (-PEEK_LEAN_EASE * delta).exp());
        if leaning.is_none() && self.peek_amount <= 0.002 {
            self.peek_amount = 0.0;
        }
        let shift = PEEK_LEAN_SHIFT * self.peek_amount;
        let at = vec3(
            i.pos.0 + i.sway.0 + self.peek_toward.0 * shift,
            i.lift,
            i.pos.1 + i.sway.1 + self.peek_toward.1 * shift,
        );
        if !sample {
            // a far frame: the state above has moved on, the bone transforms are not needed (the scene shows its last mesh)
            return (Vec::new(), Output { bones: Vec::new(), root: Mat4::IDENTITY, face_cell: 0, yaw: self.yaw });
        }
        (pose.clone(), self.finish(pose, at, clock))
    }

    /// Model-space bones and the root for a (possibly further edited) local pose.
    pub fn finish(&self, pose: Vec<Local>, at: Vec3, clock: f32) -> Output {
        let toward = vec3(self.peek_toward.0, 0.0, self.peek_toward.1);
        let lean = if self.peek_amount > 0.0 && toward.length_squared() > 0.0 {
            // pitch the whole figure about her feet so the head swings out past the edge: Y x toward takes up to toward
            Mat4::from_axis_angle(Vec3::Y.cross(toward).normalize(), PEEK_LEAN_ANGLE * self.peek_amount)
        } else {
            Mat4::IDENTITY
        };
        let root = Mat4::from_translation(at)
            * lean
            * Mat4::from_rotation_y(self.yaw)
            * Mat4::from_rotation_x(self.heavy_lean)
            * Mat4::from_scale(Vec3::splat(self.rig.draw_scale()));
        self.finish_with_root(pose, root, clock, &[])
    }

    pub fn finish_with_root(
        &self,
        pose: Vec<Local>,
        root: Mat4,
        clock: f32,
        stretch: &[f32],
    ) -> Output {
        let bones = model_space_stretched(&self.rig, &pose, stretch);
        let face = face_cell(&self.expression, clock - self.derp_started);
        Output {
            bones,
            root,
            face_cell: face,
            yaw: self.yaw,
        }
    }

    /// `_update_body_yaw`: the way she travels, else (chasing, standing) toward the runner, else the brain's heading.
    fn update_body_yaw(&mut self, i: &Input, delta: f32) {
        let mut dir = (0.0, 0.0);
        if self.facing.moving() {
            dir = self.facing.move_dir;
        } else if let Some(t) = i
            .look_target
            .filter(|_| i.active && i.stun <= 0.0 && i.state == State::Chase)
        {
            dir = (t.0 - i.pos.0, t.1 - i.pos.1);
        }
        if dir.0 * dir.0 + dir.1 * dir.1 < 0.0001 {
            dir = i.brain_facing;
        }
        if dir.0 * dir.0 + dir.1 * dir.1 < 0.0001 {
            return;
        }
        if !self.heading_initialized {
            self.yaw = (-dir.0).atan2(-dir.1);
            self.heading_initialized = true;
            return;
        }
        let running = i.state == State::Chase || self.facing.speed > 4.5;
        self.yaw = self.facing.steer(self.yaw, dir, running, delta);
    }

    /// The three stun stars (`_update_stars`): world centre of each star and its rotation, while stunned.
    pub fn stars(&self, i: &Input, out: &Output, clock: f32) -> Vec<(Vec3, Mat3)> {
        if !(i.active && i.stun > 0.0) {
            return vec![];
        }
        let Some(h) = self.rig.bone("Head") else {
            return vec![];
        };
        let head = out
            .root
            .transform_point3(out.bones[h].transform_point3(Vec3::ZERO));
        let top = head.y + 0.5;
        let phase = clock * 1000.0 * 0.004;
        let yaw = Mat3::from_rotation_y(out.yaw);
        (0..3)
            .map(|k| {
                let a = phase + TAU * k as f32 / 3.0;
                let local = vec3(a.cos(), (a * 2.0).sin() * 0.04, a.sin()) * 0.34;
                let rot = yaw * Mat3::from_euler(EulerRot::YXZ, a * 1.3, a, 0.0);
                (vec3(i.pos.0, top, i.pos.1) + yaw * local, rot)
            })
            .collect()
    }

    /// The item a mechanic state puts in her right hand (`StateDef::prop`) and the matrix of the hand bone it rides, while
    /// the state is mostly in. The caller draws the item model with `net/held_wire.rs item_transform`.
    pub fn held_prop(&self, out: &Output) -> Option<(crate::game::Item, Mat4)> {
        let m = self.mech.filter(|m| self.mech_w > 0.35 && m.prop_until.map_or(true, |t| self.mech_t < t))?;
        let item = crate::net::held_wire::from_wire(m.prop.as_deref()?)?;
        let hand = self.rig.bone("R-Hand")?;
        Some((item, *out.bones.get(hand)?))
    }

    pub fn heavy(&self, i: &Input) -> bool {
        i.heavy_stun && i.stun > 0.0
    }
}

/// Model-space transform of every bone from local TRS (parents come first in the rig), with bones lengthened along their own Y (`stretch[bone]`, 1 = none): HunterGrab.stretch_arms. A bone
/// under a stretched parent that is not stretched itself (the hand) keeps its own size: the parent's lengthening is taken
/// off its basis (its position still moves out with the longer forearm).
pub fn model_space_stretched(rig: &Rig, pose: &[Local], stretch: &[f32]) -> Vec<Mat4> {
    let sy = |k: usize| stretch.get(k).copied().unwrap_or(1.0);
    let mut out: Vec<Mat4> = Vec::with_capacity(rig.bones.len());
    for (k, b) in rig.bones.iter().enumerate() {
        let own = sy(k);
        let parent_sy = b.parent.map(sy).unwrap_or(1.0);
        let mut local = Mat4::from_translation(pose[k].t);
        if own == 1.0 && parent_sy != 1.0 {
            local *= Mat4::from_scale(vec3(1.0, 1.0 / parent_sy, 1.0));
        }
        local *= Mat4::from_quat(pose[k].r);
        if own != 1.0 {
            local *= Mat4::from_scale(vec3(1.0, own, 1.0));
        }
        let m = match b.parent {
            Some(p) if p < out.len() => out[p] * local,
            _ => local,
        };
        out.push(m);
    }
    out
}

#[cfg(test)]
mod lod_tests {
    use super::super::clip::rig;
    use super::*;

    fn input(x: f32) -> Input<'static> {
        Input {
            pos: (x, 5.0),
            brain_facing: (1.0, 0.0),
            active: true,
            stun: 0.0,
            heavy_stun: false,
            state: State::Idle,
            direct_visible: false,
            winded: false,
            airborne: false,
            lift: 0.0,
            sway: (0.0, 0.0),
            charging: false,
            look_target: None,
            path_ahead: None,
            chase_style: "pump",
            camera: vec3(0.0, 1.6, 0.0),
            grabbing: false,
            peek_lean: None,
            creature: None,
            mechanic: None,
        }
    }

    /// A far frame skips the pose sample (no layer pose, no bone transforms) and still advances every clock: the two
    /// views, one sampled and one far on every frame, agree on the state frame for frame.
    #[test]
    fn a_far_frame_skips_the_pose_but_the_state_moves_on() {
        let r = rig("aki").expect("the aki rig is embedded");
        let mut sampled = HunterView::new("aki", r.clone());
        let mut far = HunterView::new("aki", r);
        let i = input(0.0);
        let dt = 1.0 / 30.0;
        for f in 0..40 {
            let t = f as f32 * dt;
            let (_, out) = sampled.update_lod(&i, dt, t, true);
            assert!(!out.bones.is_empty(), "a sampled frame builds the bone transforms");
            let (far_pose, far_out) = far.update_lod(&i, dt, t, false);
            assert!(far_pose.is_empty() && far_out.bones.is_empty(), "a far frame builds no pose");
        }
        assert_eq!(sampled.clip(), far.clip());
        assert_eq!(sampled.blink_timer, far.blink_timer, "the blink clock advances on a far frame");
        assert_eq!(sampled.still_time, far.still_time);
        assert_eq!(sampled.move_speed, far.move_speed);
        assert_eq!(sampled.stride_scale, far.stride_scale);
        assert_eq!(sampled.playback_speed(), far.playback_speed());
    }

    /// `update` is the sampled path: the same pose length and bone count as `update_lod` with sample on.
    #[test]
    fn update_is_the_sampled_path() {
        let r = rig("aki").expect("the aki rig is embedded");
        let mut a = HunterView::new("aki", r.clone());
        let mut b = HunterView::new("aki", r);
        let i = input(0.0);
        let (pa, oa) = a.update(&i, 1.0 / 30.0, 0.5);
        let (pb, ob) = b.update_lod(&i, 1.0 / 30.0, 0.5, true);
        assert_eq!(pa.len(), pb.len());
        assert_eq!(oa.bones.len(), ob.bones.len());
        assert_eq!(oa.face_cell, ob.face_cell);
    }
}
