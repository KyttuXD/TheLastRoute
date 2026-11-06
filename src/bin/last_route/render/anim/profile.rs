//! Per-hunter animation profiles (data/render/hunter_anims.json, `lr.hunter_anims/1`).
//!
//! The clips themselves are already one set per hunter: every hunter's glb carries its own idle / walk / chase / sprint /
//! stun clips shaped by the original's `tools/blocky_hunters/hunters/motion.py` personality profile (tempo, stride,
//! arm swing, bounce, lean, head tilt...), baked into assets/hunters/anim/<id>.json. The view's chase layer
//! (`chase_style`, data/hunters/<id>.json) is per hunter too. What this file adds is a RUNTIME layer on top of those
//! clips, so the port can tune a hunter's gait without re-exporting a model:
//!
//! * gait speed (`walk_speed`, `chase_speed`): multiplies the stride-matched playback speed of the walk / chase / sprint
//!   clips (1.0 = the legs match the ground exactly, as before),
//! * stride and arm swing (`stride`, `arm_swing`): how far the legs / arms swing from their rest while a travel clip plays,
//! * lean (`lean_deg`, `chase_lean_deg`): extra forward lean of the torso while walking / chasing,
//! * crouch (`crouch`, `chase_crouch`): knees bent, hips lowered so the feet stay on the floor,
//! * idle sway (`idle_sway`): how much the idle clip moves,
//! * head tilt (`head_roll_deg`),
//! * named STATES (`states`): a mechanic animation triggered by `Input::mechanic`, a clip of the hunter's rig and/or a
//!   bone pose laid over whatever the clips do, eased in and out. This is the slot for steal-battery, lunge, cast...
//!
//! The `default` row is the identity (today's look); a hunter's row overrides only the keys it names, and a state in a
//! hunter's row replaces the global state of that name (`null` removes it). See docs/HUNTER_ANIMATIONS.md.

use super::clip::Rig;
use super::hunter::Bones;
use crate::content::embedded;
use macroquad::prelude::*;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

/// Where the table lives under data/ (the embedded file's name).
pub const PATH: &str = "render/hunter_anims.json";
/// The full knee bend of `crouch` = 1, radians (thigh forward, calf back twice that, foot flat).
const CROUCH_MAX: f32 = 0.7;
/// How fast a layer's weight follows the clip it belongs to, per second.
pub const LAYER_RATE: f32 = 6.0;

/// A mechanic animation: a clip of the hunter's own rig and/or bone rotations laid over the pose.
#[derive(Clone, Debug, PartialEq)]
pub struct StateDef {
    /// A clip of the hunter's rig played while the state is on (looping clips loop). A rig without it plays its normal
    /// clip, so a state can name a clip only some hunters have.
    pub clip: Option<String>,
    /// Playback speed of `clip`.
    pub speed: f32,
    /// Seconds the clip cross-fades in.
    pub blend: f32,
    /// Bone name -> euler degrees (x pitch, y yaw, z roll, Godot's YXZ order) multiplied onto the bone's rotation at full
    /// weight, so a bone does not need a clip to move. A bone the rig lacks is skipped.
    pub pose: Vec<(String, [f32; 3])>,
    /// Seconds the pose eases in / out.
    pub blend_in: f32,
    pub blend_out: f32,
    /// An item model the hunter holds in her right hand while the state is on (a `net/held_wire.rs` wire id: `battery`,
    /// `music_box`...), posed by the `remote_held.json` table like a runner's held item. It follows the hand through every clip.
    pub prop: Option<String>,
    /// Seconds into the state after which the prop is put away (pocketed, crushed); None: it stays while the state is on.
    pub prop_until: Option<f32>,
    /// `pose` is a pose RELATIVE TO REST that replaces what the clips and the chase layer did to those bones (eased in by
    /// the weight), so the silhouette is the same whatever the walk was doing (`"mode": "set"`). False (`"mode": "add"`, the
    /// default): the rotations are multiplied onto the animated pose.
    pub set: bool,
    /// Knee bend while the state is on, 0..1 like the profile's `crouch` (the hips drop so the feet stay on the floor).
    pub crouch: f32,
    /// A second beat: from `after` seconds in, the pose (and crouch) move to this one over `over` seconds.
    pub then: Option<Then>,
}

/// The second beat of a mechanic state (`then`): a bone listed here moves to its pose here, a bone not listed keeps the first beat's.
#[derive(Clone, Debug, PartialEq)]
pub struct Then {
    pub after: f32,
    pub over: f32,
    pub pose: Vec<(String, [f32; 3])>,
    pub crouch: Option<f32>,
}

impl Default for StateDef {
    fn default() -> StateDef {
        StateDef { clip: None, speed: 1.0, blend: 0.15, pose: vec![], blend_in: 0.2, blend_out: 0.25, prop: None, prop_until: None, set: false, crouch: 0.0, then: None }
    }
}

/// One hunter's tuning. `Profile::default()` changes nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub walk_speed: f32,
    pub chase_speed: f32,
    pub stride: f32,
    pub arm_swing: f32,
    pub lean_deg: f32,
    pub chase_lean_deg: f32,
    pub crouch: f32,
    pub chase_crouch: f32,
    pub idle_sway: f32,
    pub head_roll_deg: f32,
    pub states: BTreeMap<String, StateDef>,
}

impl Default for Profile {
    fn default() -> Profile {
        Profile {
            walk_speed: 1.0,
            chase_speed: 1.0,
            stride: 1.0,
            arm_swing: 1.0,
            lean_deg: 0.0,
            chase_lean_deg: 0.0,
            crouch: 0.0,
            chase_crouch: 0.0,
            idle_sway: 1.0,
            head_roll_deg: 0.0,
            states: BTreeMap::new(),
        }
    }
}

impl Profile {
    /// True when no pose layer would move a bone (the playback speeds are not a pose).
    pub fn is_identity_pose(&self) -> bool {
        let d = Profile::default();
        self.stride == d.stride
            && self.arm_swing == d.arm_swing
            && self.lean_deg == 0.0
            && self.chase_lean_deg == 0.0
            && self.crouch == 0.0
            && self.chase_crouch == 0.0
            && self.idle_sway == d.idle_sway
            && self.head_roll_deg == 0.0
    }
}

/// The weights of the layers this frame, each 0..1.
#[derive(Clone, Copy, Debug, Default)]
pub struct Weights {
    /// a walk / chase / sprint clip is playing
    pub gait: f32,
    /// the idle clip is playing
    pub idle: f32,
    /// the chase layer's own blend (1 while chasing)
    pub chase: f32,
    /// she is up and about (not dormant, stunned, waking or held)
    pub live: f32,
}

const NUMBER_KEYS: [(&str, f32, f32); 10] = [
    ("walk_speed", 0.25, 4.0),
    ("chase_speed", 0.25, 4.0),
    ("stride", 0.0, 3.0),
    ("arm_swing", 0.0, 3.0),
    ("lean_deg", -30.0, 45.0),
    ("chase_lean_deg", -30.0, 45.0),
    ("crouch", 0.0, 1.0),
    ("chase_crouch", 0.0, 1.0),
    ("idle_sway", 0.0, 4.0),
    ("head_roll_deg", -30.0, 30.0),
];

fn number(v: &Value, key: &str, lo: f32, hi: f32) -> Result<f32, String> {
    let n = v.as_f64().ok_or_else(|| format!("{key}: must be a number"))? as f32;
    if !(n.is_finite() && (lo..=hi).contains(&n)) {
        return Err(format!("{key}: {n} is outside {lo}..{hi}"));
    }
    Ok(n)
}

fn parse_pose(name: &str, x: &Value) -> Result<Vec<(String, [f32; 3])>, String> {
    let p = x.as_object().ok_or_else(|| format!("state {name}: pose must be an object"))?;
    let mut out = vec![];
    for (bone, deg) in p {
        let a = deg.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("state {name}.pose.{bone}: want [x, y, z] degrees"))?;
        let mut d = [0.0f32; 3];
        for (n, c) in a.iter().enumerate() {
            d[n] = number(c, &format!("state {name}.pose.{bone}"), -180.0, 180.0)?;
        }
        out.push((bone.clone(), d));
    }
    Ok(out)
}

fn parse_state(name: &str, v: &Value) -> Result<StateDef, String> {
    let o = v.as_object().ok_or_else(|| format!("state {name}: must be an object"))?;
    let mut def = StateDef::default();
    for (k, x) in o {
        match k.as_str() {
            "clip" => def.clip = Some(x.as_str().ok_or_else(|| format!("state {name}: clip must be a string"))?.to_string()),
            "speed" => def.speed = number(x, &format!("state {name}.speed"), 0.1, 4.0)?,
            "blend" => def.blend = number(x, &format!("state {name}.blend"), 0.0, 2.0)?,
            "blend_in" => def.blend_in = number(x, &format!("state {name}.blend_in"), 0.0, 5.0)?,
            "blend_out" => def.blend_out = number(x, &format!("state {name}.blend_out"), 0.0, 5.0)?,
            "pose" => def.pose = parse_pose(name, x)?,
            "mode" => {
                def.set = match x.as_str() {
                    Some("set") => true,
                    Some("add") => false,
                    _ => return Err(format!("state {name}: mode must be \"add\" or \"set\"")),
                }
            }
            "crouch" => def.crouch = number(x, &format!("state {name}.crouch"), 0.0, 1.0)?,
            "prop_until" => def.prop_until = Some(number(x, &format!("state {name}.prop_until"), 0.0, 30.0)?),
            "then" => {
                let o = x.as_object().ok_or_else(|| format!("state {name}: then must be an object"))?;
                let mut th = Then { after: 0.5, over: 0.2, pose: vec![], crouch: None };
                for (k2, y) in o {
                    match k2.as_str() {
                        "after" => th.after = number(y, &format!("state {name}.then.after"), 0.0, 30.0)?,
                        "over" => th.over = number(y, &format!("state {name}.then.over"), 0.0, 5.0)?,
                        "pose" => th.pose = parse_pose(name, y)?,
                        "crouch" => th.crouch = Some(number(y, &format!("state {name}.then.crouch"), 0.0, 1.0)?),
                        "_about" => {}
                        other => return Err(format!("state {name}.then: unknown key {other}")),
                    }
                }
                def.then = Some(th);
            }
            "prop" => {
                let id = x.as_str().ok_or_else(|| format!("state {name}: prop must be a string"))?;
                if crate::net::held_wire::from_wire(id).is_none() {
                    return Err(format!("state {name}: prop {id} is not an item (net/held_wire.rs wire ids)"));
                }
                def.prop = Some(id.to_string());
            }
            "_about" => {}
            other => return Err(format!("state {name}: unknown key {other}")),
        }
    }
    if def.clip.is_none() && def.pose.is_empty() && def.prop.is_none() && def.crouch == 0.0 {
        return Err(format!("state {name}: needs a clip, a pose, a crouch or a prop"));
    }
    Ok(def)
}

/// `base` with the keys of the object `v` laid over it. An unknown key is an error (a typo would otherwise be silent).
pub fn overlay(base: &Profile, v: &Value) -> Result<Profile, String> {
    let o = v.as_object().ok_or("a profile must be an object")?;
    let mut p = base.clone();
    for (k, x) in o {
        if k == "states" {
            let s = x.as_object().ok_or("states must be an object")?;
            for (name, def) in s {
                if name.starts_with('_') {
                    continue; // a note
                }
                if def.is_null() {
                    p.states.remove(name);
                } else {
                    p.states.insert(name.clone(), parse_state(name, def)?);
                }
            }
            continue;
        }
        if k == "_about" {
            continue;
        }
        let Some(&(_, lo, hi)) = NUMBER_KEYS.iter().find(|row| row.0 == k.as_str()) else {
            return Err(format!("unknown key {k}"));
        };
        let n = number(x, k, lo, hi)?;
        match k.as_str() {
            "walk_speed" => p.walk_speed = n,
            "chase_speed" => p.chase_speed = n,
            "stride" => p.stride = n,
            "arm_swing" => p.arm_swing = n,
            "lean_deg" => p.lean_deg = n,
            "chase_lean_deg" => p.chase_lean_deg = n,
            "crouch" => p.crouch = n,
            "chase_crouch" => p.chase_crouch = n,
            "idle_sway" => p.idle_sway = n,
            _ => p.head_roll_deg = n,
        }
    }
    Ok(p)
}

/// The whole table: the default row, the global states and each named hunter's merged row.
#[derive(Debug, Default)]
pub struct Table {
    pub default: Profile,
    pub hunters: HashMap<String, Profile>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Table, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        match v.get("schema").and_then(Value::as_str) {
            Some("lr.hunter_anims/1") => {}
            other => return Err(format!("schema must be lr.hunter_anims/1, got {other:?}")),
        }
        let mut default = Profile::default();
        if let Some(d) = v.get("default") {
            default = overlay(&default, d).map_err(|e| format!("default: {e}"))?;
        }
        if let Some(s) = v.get("states") {
            default = overlay(&default, &serde_json::json!({ "states": s })).map_err(|e| format!("states: {e}"))?;
        }
        let mut hunters = HashMap::new();
        for (id, row) in v.get("hunters").and_then(Value::as_object).into_iter().flatten() {
            hunters.insert(id.clone(), overlay(&default, row).map_err(|e| format!("{id}: {e}"))?);
        }
        Ok(Table { default, hunters })
    }

    /// The hunter's profile: its own row, else the default (a pack hunter without a row).
    pub fn profile(&self, id: &str) -> &Profile {
        self.hunters.get(id).unwrap_or(&self.default)
    }
}

/// The table compiled into the executable, read once; a bad table is reported on stderr and the identity is used.
pub fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("");
        Table::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            Table::default()
        })
    })
}

/// The profile of the hunter drawn from the model `id`.
pub fn profile(id: &str) -> &'static Profile {
    table().profile(id)
}

/// The mechanic state the brain is in that the profile may animate (`Brain::mechanic`): a ladder climb, a one-shot cue raised
/// at a gameplay event, what the hunter's gimmick is doing, or (a guest) what the host's frame said. The names are
/// `hunters/mechanics.rs`; a name the hunter's profile has no state for does nothing.
pub fn mechanic_of(brain: &crate::hunters::brain::Brain) -> Option<&'static str> {
    brain.mechanic()
}

fn scaled(b: &mut Bones, bones: &[&str], scale: f32) {
    if (scale - 1.0).abs() < 1e-4 {
        return;
    }
    for &n in bones {
        let q = b.swing(n, scale);
        b.put(n, q);
    }
}

/// The profile's pose layers on the frame's pose: leg and arm swing, idle sway, lean, crouch, head tilt.
pub(super) fn apply(p: &Profile, b: &mut Bones, w: Weights) {
    scaled(b, &["L-Thigh", "R-Thigh", "L-Calf", "R-Calf", "L-Foot", "R-Foot"], 1.0 + (p.stride - 1.0) * w.gait);
    scaled(b, &["L-Arm", "R-Arm", "L-Forearm", "R-Forearm"], 1.0 + (p.arm_swing - 1.0) * w.gait);
    scaled(b, &["Chest", "Belly", "Head", "L-Arm", "R-Arm", "L-Forearm", "R-Forearm"], 1.0 + (p.idle_sway - 1.0) * w.idle);
    // lean: the torso pitches forward (negative X in this rig), the head keeps half of it back so the gaze stays level
    let lean = (p.lean_deg * w.gait + p.chase_lean_deg * w.chase).to_radians();
    if lean.abs() > 1e-5 {
        for (bone, share) in [("Belly", 0.4), ("Chest", 0.6), ("Head", -0.5)] {
            let q = Quat::from_rotation_x(-lean * share) * b.now(bone);
            b.put(bone, q);
        }
    }
    let crouch = ((p.crouch + p.chase_crouch * w.chase) * w.live).clamp(0.0, 1.0);
    if crouch > 1e-4 {
        crouch_legs(b, crouch * CROUCH_MAX);
    }
    if p.head_roll_deg != 0.0 {
        let q = Quat::from_rotation_z((p.head_roll_deg * w.live).to_radians()) * b.now("Head");
        b.put("Head", q);
    }
}

/// Where one ankle is relative to the pelvis, from the current local pose of its leg chain.
fn ankle(b: &Bones, side: &str) -> Option<Vec3> {
    let i = |bone: &str| b.rig.bone(&format!("{side}-{bone}"));
    let (t, c, f) = (i("Thigh")?, i("Calf")?, i("Foot")?);
    let p = &*b.pose;
    Some(p[t].t + p[t].r * (p[c].t + p[c].r * p[f].t))
}

/// Bends both legs by `a` radians: thigh forward `a`, calf back `2a`, foot flat again, and the pelvis moved by exactly what
/// the bend does to the ankles (lowered, and set back a little), so the feet stay where they were whatever the rig's leg offsets.
fn crouch_legs(b: &mut Bones, a: f32) {
    let Some(pelvis) = b.rig.bone("Pelvis") else { return };
    let before: Vec<Vec3> = ["L", "R"].iter().filter_map(|s| ankle(b, s)).collect();
    if before.is_empty() {
        return;
    }
    for side in ["L", "R"] {
        for (bone, angle) in [("Thigh", a), ("Calf", -2.0 * a), ("Foot", a)] {
            let name = format!("{side}-{bone}");
            let q = Quat::from_rotation_x(angle) * b.now(&name);
            b.put(&name, q);
        }
    }
    let after: Vec<Vec3> = ["L", "R"].iter().filter_map(|s| ankle(b, s)).collect();
    let n = before.len() as f32;
    let moved = before.iter().zip(&after).fold(Vec3::ZERO, |acc, (x, y)| acc + (*y - *x)) / n;
    let r = b.pose[pelvis].r;
    b.pose[pelvis].t -= r * moved;
}

/// A state's bone pose at weight `w`.
pub(super) fn apply_state(def: &StateDef, b: &mut Bones, w: f32, t: f32) {
    if w <= 0.0 {
        return;
    }
    // how far the second beat has moved the pose (0 before it, 1 after)
    let k = def.then.as_ref().map_or(0.0, |th| {
        let x = ((t - th.after) / th.over.max(1e-3)).clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    });
    let first = |bone: &str| def.pose.iter().find(|p| p.0 == bone).map(|p| p.1);
    let second = |bone: &str| def.then.as_ref().and_then(|th| th.pose.iter().find(|p| p.0 == bone).map(|p| p.1));
    let mut bones: Vec<&str> = def.pose.iter().map(|p| p.0.as_str()).collect();
    if let Some(th) = &def.then {
        for (bone, _) in &th.pose {
            if !bones.contains(&bone.as_str()) {
                bones.push(bone);
            }
        }
    }
    for bone in bones {
        if b.rig.bone(bone).is_none() {
            continue;
        }
        let a = first(bone).unwrap_or([0.0; 3]);
        let d = second(bone).map_or(a, |s| [a[0] + (s[0] - a[0]) * k, a[1] + (s[1] - a[1]) * k, a[2] + (s[2] - a[2]) * k]);
        let q = if def.set {
            let target = super::clip::key_quat(vec3(d[0], d[1], d[2])) * b.rest(bone);
            b.now(bone).slerp(target, w)
        } else {
            super::clip::key_quat(vec3(d[0], d[1], d[2]) * w) * b.now(bone)
        };
        b.put(bone, q);
    }
    let c1 = def.crouch;
    let c = def.then.as_ref().and_then(|th| th.crouch).map_or(c1, |c2| c1 + (c2 - c1) * k);
    if c * w > 1e-4 {
        crouch_legs(b, c * w * CROUCH_MAX);
    }
}


/// The rig's lengths a state needs checked at load: every pose bone and clip a state names (for the data tests).
pub fn missing_in_rig(def: &StateDef, rig: &Rig) -> Vec<String> {
    let mut out = vec![];
    if let Some(c) = &def.clip {
        if !rig.has_clip(c) {
            out.push(format!("clip {c}"));
        }
    }
    for (bone, _) in &def.pose {
        if rig.bone(bone).is_none() {
            out.push(format!("bone {bone}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::player::Local;
    use super::*;
    use serde_json::json;

    #[test]
    fn the_default_row_is_the_identity() {
        let p = Profile::default();
        assert!(p.is_identity_pose());
        assert_eq!((p.walk_speed, p.chase_speed), (1.0, 1.0));
        let t = Table::parse(r#"{"schema":"lr.hunter_anims/1"}"#).unwrap();
        assert_eq!(t.default, Profile::default());
        assert_eq!(t.profile("anyone"), &Profile::default());
    }

    #[test]
    fn a_hunter_row_overrides_only_the_keys_it_names() {
        let t = Table::parse(
            r#"{"schema":"lr.hunter_anims/1","default":{"stride":1.1},
                "hunters":{"aki":{"crouch":0.2},"zellia":{"stride":0.9,"states":{"x":{"pose":{"R-Arm":[10,0,0]}}}}}}"#,
        )
        .unwrap();
        assert_eq!(t.profile("aki").stride, 1.1, "keeps the default's stride");
        assert_eq!(t.profile("aki").crouch, 0.2);
        assert_eq!(t.profile("zellia").stride, 0.9);
        assert_eq!(t.profile("zellia").crouch, 0.0);
        assert!(t.profile("zellia").states.contains_key("x"));
        assert!(!t.profile("aki").states.contains_key("x"), "a state of one hunter is not another's");
    }

    #[test]
    fn a_hunters_state_replaces_the_global_one_and_null_removes_it() {
        let t = Table::parse(
            r#"{"schema":"lr.hunter_anims/1","states":{"s":{"clip":"idle"}},
                "hunters":{"a":{"states":{"s":{"clip":"walk","speed":2}}},"b":{"states":{"s":null}}}}"#,
        )
        .unwrap();
        assert_eq!(t.profile("c").states["s"].clip.as_deref(), Some("idle"));
        assert_eq!(t.profile("a").states["s"].clip.as_deref(), Some("walk"));
        assert_eq!(t.profile("a").states["s"].speed, 2.0);
        assert!(t.profile("b").states.is_empty());
    }

    #[test]
    fn bad_data_is_refused_with_the_key_named() {
        for (doc, needle) in [
            (json!({"schema":"lr.hunter_anims/2"}), "schema"),
            (json!({"schema":"lr.hunter_anims/1","default":{"strde":1.0}}), "strde"),
            (json!({"schema":"lr.hunter_anims/1","default":{"stride":"big"}}), "stride"),
            (json!({"schema":"lr.hunter_anims/1","default":{"crouch":2.0}}), "crouch"),
            (json!({"schema":"lr.hunter_anims/1","hunters":{"aki":{"walk_speed":0.0}}}), "aki"),
            (json!({"schema":"lr.hunter_anims/1","states":{"s":{}}}), "needs a clip, a pose, a crouch or a prop"),
            (json!({"schema":"lr.hunter_anims/1","states":{"s":{"clip":"x","zoom":1}}}), "zoom"),
            (json!({"schema":"lr.hunter_anims/1","states":{"s":{"pose":{"R-Arm":[1,2]}}}}), "x, y, z"),
        ] {
            let e = Table::parse(&doc.to_string()).unwrap_err();
            assert!(e.contains(needle), "{doc}: {e}");
        }
    }

    /// A rig at its rest pose, and a pose-layer test bench over it.
    fn rest_pose(rig: &Rig) -> Vec<Local> {
        rig.bones.iter().map(|b| Local { t: b.rest_t, r: b.rest_r }).collect()
    }

    fn world_of(rig: &Rig, pose: &[Local], bone: &str) -> Vec3 {
        let m = super::super::hunter::model_space_stretched(rig, pose, &[]);
        m[rig.bone(bone).unwrap()].w_axis.truncate()
    }

    fn rig(id: &str) -> std::rc::Rc<Rig> {
        super::super::clip::rig(id).unwrap()
    }

    #[test]
    fn the_identity_profile_leaves_the_pose_alone() {
        let r = rig("aki");
        let mut pose = rest_pose(&r);
        let before: Vec<Quat> = pose.iter().map(|l| l.r).collect();
        let w = Weights { gait: 1.0, idle: 1.0, chase: 1.0, live: 1.0 };
        apply(&Profile::default(), &mut Bones::new(&r, &mut pose), w);
        for (k, l) in pose.iter().enumerate() {
            assert!(l.r.abs_diff_eq(before[k], 1e-6), "{}", r.bones[k].name);
        }
    }

    #[test]
    fn crouch_lowers_the_hips_and_keeps_the_feet_on_the_floor() {
        for id in ["aki", "xivvy", "void_red", "void_black", "verrine", "wynelle", "loxelle", "kathy"] {
            let r = rig(id);
            let mut pose = rest_pose(&r);
            let (foot0, hip0) = (world_of(&r, &pose, "L-Foot"), world_of(&r, &pose, "Pelvis"));
            let p = Profile { crouch: 0.6, ..Profile::default() };
            apply(&p, &mut Bones::new(&r, &mut pose), Weights { live: 1.0, ..Default::default() });
            let (foot1, hip1) = (world_of(&r, &pose, "L-Foot"), world_of(&r, &pose, "Pelvis"));
            assert!(hip1.y < hip0.y - 1.0, "{id}: the hips drop ({} -> {})", hip0.y, hip1.y);
            assert!((foot1.y - foot0.y).abs() < 0.05, "{id}: the ankle stays at {} (is {})", foot0.y, foot1.y);
            assert!((foot1.z - foot0.z).abs() < 0.3, "{id}: and under her ({} -> {})", foot0.z, foot1.z);
        }
    }

    #[test]
    fn stride_and_arm_swing_scale_how_far_a_limb_swings_from_its_rest() {
        let r = rig("aki");
        let walk = &r.clips["walk"];
        // a mid-stride pose from the clip itself
        let mut player = super::super::player::AnimPlayer::default();
        player.play(&r, "walk", -1.0);
        player.advance(&r, walk.length * 0.25);
        let base = player.pose(&r);
        let swing = |pose: &[Local], bone: &str| {
            let i = r.bone(bone).unwrap();
            (pose[i].r * r.bones[i].rest_r.inverse()).w.clamp(-1.0, 1.0).abs().acos() * 2.0
        };
        let (t0, a0) = (swing(&base, "L-Thigh"), swing(&base, "L-Arm"));
        assert!(t0 > 0.1 && a0 > 0.02, "the clip swings: {t0} {a0}");
        let mut pose = base.clone();
        let p = Profile { stride: 1.5, arm_swing: 0.5, ..Profile::default() };
        apply(&p, &mut Bones::new(&r, &mut pose), Weights { gait: 1.0, ..Default::default() });
        assert!((swing(&pose, "L-Thigh") / t0 - 1.5).abs() < 0.05, "legs swing half again as far");
        assert!((swing(&pose, "L-Arm") / a0 - 0.5).abs() < 0.05, "arms swing half as far");
        // the layer follows the gait weight: at 0 nothing moves
        let mut idle = base.clone();
        apply(&p, &mut Bones::new(&r, &mut idle), Weights::default());
        assert!((swing(&idle, "L-Thigh") - t0).abs() < 1e-4);
    }

    #[test]
    fn lean_pitches_the_torso_forward_and_keeps_the_head_up() {
        let r = rig("aki");
        let mut pose = rest_pose(&r);
        let (chest0, head0) = (world_of(&r, &pose, "Chest"), world_of(&r, &pose, "Head"));
        let p = Profile { lean_deg: 20.0, ..Profile::default() };
        apply(&p, &mut Bones::new(&r, &mut pose), Weights { gait: 1.0, ..Default::default() });
        let head1 = world_of(&r, &pose, "Head");
        assert!(head1.z < head0.z - 1.0, "the head moves forward (-Z): {} -> {}", head0.z, head1.z);
        assert!(head1.y < head0.y, "and a little down");
        assert!(world_of(&r, &pose, "Chest").z < chest0.z, "the chest too");
    }

    #[test]
    fn a_state_pose_eases_with_its_weight() {
        let r = rig("ysolde");
        let def = parse_state("steal", &json!({"pose": {"R-Arm": [80.0, 0.0, 0.0], "Nowhere": [10.0, 0.0, 0.0]}})).unwrap();
        let hand = |w: f32| {
            let mut pose = rest_pose(&r);
            apply_state(&def, &mut Bones::new(&r, &mut pose), w, 0.0);
            world_of(&r, &pose, "R-Hand")
        };
        let (rest, half, full) = (hand(0.0), hand(0.5), hand(1.0));
        assert!(full.z < rest.z - 10.0, "the hand reaches forward: {rest:?} -> {full:?}");
        assert!(half.z < rest.z && half.z > full.z, "halfway at half weight");
        assert_eq!(missing_in_rig(&def, &r), vec!["bone Nowhere".to_string()]);
    }

    // ---- the view: the profile through HunterView ----

    use super::super::hunter::{HunterView, Input, Output};
    use crate::hunters::brain::State;

    fn leak(p: Profile) -> &'static Profile {
        Box::leak(Box::new(p))
    }

    fn input(x: f32, state: State, mechanic: Option<&'static str>) -> Input<'static> {
        Input {
            pos: (x, 5.0),
            brain_facing: (1.0, 0.0),
            active: true,
            stun: 0.0,
            heavy_stun: false,
            state,
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
            mechanic,
        }
    }

    /// `n` frames at 30 fps with the body moving `speed` m/s along +X; the last frame's output.
    fn run(view: &mut HunterView, n: usize, speed: f32, state: State, mechanic: Option<&'static str>) -> Output {
        let mut out = None;
        let mut x = 0.0;
        for k in 0..n {
            x += speed / 30.0;
            out = Some(view.update(&input(x, state, mechanic), 1.0 / 30.0, k as f32 / 30.0).1);
        }
        out.unwrap()
    }

    fn bone_at(view: &HunterView, out: &Output, bone: &str) -> Vec3 {
        out.bones[view.rig().bone(bone).unwrap()].w_axis.truncate()
    }

    fn state_table(src: &str) -> &'static Profile {
        leak(Table::parse(src).unwrap().hunters["t"].clone())
    }

    #[test]
    fn a_mechanic_state_with_a_clip_plays_it_and_the_clip_follows_the_state() {
        let p = state_table(r#"{"schema":"lr.hunter_anims/1","hunters":{"t":{"states":{"climb":{"clip":"climb"},"nope":{"clip":"no_such_clip"}}}}}"#);
        let mut v = HunterView::new("aki", rig("aki")).with_profile(p);
        run(&mut v, 30, 0.0, State::Idle, None);
        assert_eq!(v.clip(), "idle");
        run(&mut v, 10, 0.0, State::Idle, Some("climb"));
        assert_eq!(v.clip(), "climb", "standing still on a ladder is the climb, not the idle");
        run(&mut v, 10, 0.0, State::Idle, Some("unknown_state"));
        assert_eq!(v.clip(), "idle", "a name the profile has no row for does nothing");
        run(&mut v, 10, 0.0, State::Idle, Some("nope"));
        assert_eq!(v.clip(), "idle", "a clip the rig lacks falls back to its normal clip");
        run(&mut v, 10, 0.0, State::Idle, Some("climb"));
        assert_eq!(v.clip(), "climb");
        run(&mut v, 10, 0.0, State::Idle, None);
        assert_eq!(v.clip(), "idle", "and back when the state ends");
    }

    #[test]
    fn a_stun_beats_a_mechanic_state() {
        let p = state_table(r#"{"schema":"lr.hunter_anims/1","hunters":{"t":{"states":{"climb":{"clip":"climb"}}}}}"#);
        let mut v = HunterView::new("aki", rig("aki")).with_profile(p);
        run(&mut v, 10, 0.0, State::Idle, Some("climb"));
        assert_eq!(v.clip(), "climb");
        let mut i = input(0.0, State::Idle, Some("climb"));
        i.stun = 1.0;
        v.update(&i, 1.0 / 30.0, 1.0);
        assert_eq!(v.clip(), "stun");
    }

    #[test]
    fn the_shipped_climb_state_gives_every_hunter_the_climb_clip_on_a_ladder() {
        for id in ["aki", "kathy", "xivvy", "void_gray", "zellia"] {
            let mut v = HunterView::new(id, rig(id));
            run(&mut v, 10, 0.0, State::Chase, Some("climb"));
            assert_eq!(v.clip(), "climb", "{id}");
            run(&mut v, 10, 0.0, State::Chase, None);
            assert_ne!(v.clip(), "climb", "{id}");
        }
    }

    #[test]
    fn a_state_pose_eases_in_while_on_and_out_after() {
        let p = state_table(r#"{"schema":"lr.hunter_anims/1","hunters":{"t":{"states":{"steal":{"pose":{"R-Arm":[80,0,0]},"blend_in":0.3,"blend_out":0.3}}}}}"#);
        let mut v = HunterView::new("ysolde", rig("ysolde")).with_profile(p);
        let hand_z = |v: &HunterView, o: &Output| bone_at(v, o, "R-Hand").z;
        let base = run(&mut v, 20, 0.0, State::Idle, None);
        let z0 = hand_z(&v, &base);
        let early = run(&mut v, 3, 0.0, State::Idle, Some("steal"));
        let on = run(&mut v, 20, 0.0, State::Idle, Some("steal"));
        let (ze, zo) = (hand_z(&v, &early), hand_z(&v, &on));
        assert!(zo < z0 - 10.0, "the arm reaches forward ({z0} -> {zo})");
        assert!(ze < z0 && ze > zo, "and gets there over blend_in ({ze})");
        let off = run(&mut v, 3, 0.0, State::Idle, None);
        let zf = hand_z(&v, &off);
        assert!(zf < z0 - 1.0 && zf > zo, "it eases back out, not at once ({zf})");
        let gone = run(&mut v, 30, 0.0, State::Idle, None);
        assert!((hand_z(&v, &gone) - z0).abs() < 4.0, "and ends where the idle puts it");
    }

    #[test]
    fn the_gait_speed_multiplies_the_stride_matched_playback() {
        let slow = state_table(r#"{"schema":"lr.hunter_anims/1","hunters":{"t":{"chase_speed":1.0}}}"#);
        let fast = state_table(r#"{"schema":"lr.hunter_anims/1","hunters":{"t":{"chase_speed":1.5,"walk_speed":0.5}}}"#);
        let play = |p: &'static Profile, state: State, speed: f32| {
            let mut v = HunterView::new("aki", rig("aki")).with_profile(p);
            run(&mut v, 40, speed, state, None);
            (v.clip().to_string(), v.playback_speed())
        };
        let (c0, s0) = play(slow, State::Chase, 3.0);
        let (c1, s1) = play(fast, State::Chase, 3.0);
        assert_eq!(c0, c1);
        assert!((s1 / s0 - 1.5).abs() < 0.01, "chase clip {c0}: {s0} -> {s1}");
        let (w0, a0) = play(slow, State::Wander, 1.2);
        let (w1, a1) = play(fast, State::Wander, 1.2);
        assert_eq!((w0.as_str(), w1.as_str()), ("walk", "walk"));
        assert!((a1 / a0 - 0.5).abs() < 0.01, "the walk is not scaled by the chase speed: {a0} -> {a1}");
    }

    #[test]
    fn chase_lean_and_crouch_apply_while_chasing_and_not_while_idle() {
        let plain = leak(Profile::default());
        let tuned = leak(Profile { chase_lean_deg: 15.0, chase_crouch: 0.3, ..Profile::default() });
        let pose_of = |p: &'static Profile, state: State, speed: f32| {
            let mut v = HunterView::new("aki", rig("aki")).with_profile(p);
            let out = run(&mut v, 60, speed, state, None);
            (bone_at(&v, &out, "Head"), bone_at(&v, &out, "Pelvis"))
        };
        let (h0, p0) = pose_of(plain, State::Chase, 3.0);
        let (h1, p1) = pose_of(tuned, State::Chase, 3.0);
        assert!(h1.z < h0.z - 1.0, "head forward in the chase: {} -> {}", h0.z, h1.z);
        assert!(p1.y < p0.y - 0.5, "hips lower in the chase: {} -> {}", p0.y, p1.y);
        let (hi0, pi0) = pose_of(plain, State::Idle, 0.0);
        let (hi1, pi1) = pose_of(tuned, State::Idle, 0.0);
        assert!((hi1 - hi0).length() < 0.2 && (pi1 - pi0).length() < 0.2, "standing still looks the same");
    }

    #[test]
    fn a_hunter_without_a_row_is_drawn_exactly_as_with_the_identity_profile() {
        for id in ["dovelle", "kathy", "mavrine", "aki"] {
            assert!(profile(id).is_identity_pose(), "{id} has no pose row");
            assert_eq!((profile(id).walk_speed, profile(id).chase_speed), (1.0, 1.0), "{id}");
        }
        let a = &mut HunterView::new("dovelle", rig("dovelle"));
        let b = &mut HunterView::new("dovelle", rig("dovelle")).with_profile(leak(Profile::default()));
        for (state, speed) in [(State::Chase, 3.4), (State::Wander, 1.2), (State::Idle, 0.0)] {
            let (oa, ob) = (run(a, 45, speed, state, None), run(b, 45, speed, state, None));
            for (m, n) in oa.bones.iter().zip(&ob.bones) {
                assert!(m.abs_diff_eq(*n, 1e-6));
            }
        }
    }

    #[test]
    fn every_hunter_with_a_row_moves_differently_from_the_default() {
        for (id, p) in &table().hunters {
            assert!(
                p.chase_speed != 1.0 || p.walk_speed != 1.0 || !p.is_identity_pose() || p.states != table().default.states,
                "{id}: a row that changes nothing"
            );
        }
    }

    #[test]
    fn the_shipped_table_parses_and_every_row_is_a_known_hunter_with_real_bones_and_clips() {
        let t = Table::parse(embedded::data_under(PATH).next().expect("hunter_anims.json is embedded").1).expect("parses");
        assert_eq!(t.default, {
            let mut d = Profile::default();
            d.states = t.default.states.clone();
            d
        }, "the default row is the identity: today's look for every hunter without a row");
        for (id, p) in &t.hunters {
            let r = super::super::clip::rig(id).unwrap_or_else(|| panic!("{id}: no rig"));
            assert!(
                embedded::data_under("hunters/").any(|(path, _)| path == format!("hunters/{id}.json")),
                "{id}: a row for a hunter that has no data/hunters/{id}.json"
            );
            for (name, s) in &p.states {
                let missing = missing_in_rig(s, &r);
                // a state may name a clip only some hunters have, but a bone every humanoid rig has must exist
                assert!(missing.iter().all(|m| m.starts_with("clip ")), "{id}/{name}: {missing:?}");
            }
        }
        for (name, s) in &t.default.states {
            for id in ["aki", "kathy", "xivvy", "void_red"] {
                let r = super::super::clip::rig(id).unwrap();
                assert!(missing_in_rig(s, &r).is_empty(), "global state {name} on {id}: {:?}", missing_in_rig(s, &r));
            }
        }
    }

    // ---- mechanic states: set mode, the second beat, the prop, and the shipped table ----

    use crate::net::held_wire;

    fn shipped_state(id: &str, name: &str) -> &'static StateDef {
        profile(id).states.get(name).unwrap_or_else(|| panic!("{id} has no state {name}"))
    }

    /// The rest pose with a shipped state applied at weight `w`, `t` seconds in.
    fn posed(id: &str, name: &str, w: f32, t: f32) -> (std::rc::Rc<Rig>, Vec<Local>) {
        let r = rig(id);
        let mut pose = rest_pose(&r);
        apply_state(shipped_state(id, name), &mut Bones::new(&r, &mut pose), w, t);
        (r, pose)
    }

    fn moved(id: &str, name: &str, bone: &str, t: f32) -> Vec3 {
        let (r, rest) = (rig(id), rest_pose(&rig(id)));
        let (_, pose) = posed(id, name, 1.0, t);
        world_of(&r, &pose, bone) - world_of(&r, &rest, bone)
    }

    #[test]
    fn set_mode_gives_the_same_silhouette_whatever_the_walk_was_doing() {
        let r = rig("ysolde");
        let set = parse_state("s", &json!({"mode": "set", "pose": {"R-Arm": [80.0, 0.0, 0.0]}})).unwrap();
        let add = parse_state("a", &json!({"pose": {"R-Arm": [80.0, 0.0, 0.0]}})).unwrap();
        // a mid-stride pose, where the arm is swung away from rest
        let mut player = super::super::player::AnimPlayer::default();
        player.play(&r, "walk", -1.0);
        player.advance(&r, r.clips["walk"].length * 0.25);
        let walking = player.pose(&r);
        // the shoulder's own rotation (the torso above it still walks, so the hand in the world would differ)
        let arm = r.bone("R-Arm").unwrap();
        let rot = |def: &StateDef, base: &[Local], w: f32| {
            let mut pose = base.to_vec();
            apply_state(def, &mut Bones::new(&r, &mut pose), w, 0.0);
            pose[arm].r
        };
        let at_rest = rest_pose(&r);
        assert!(rot(&set, &walking, 1.0).abs_diff_eq(rot(&set, &at_rest, 1.0), 1e-4), "set: the walk does not show in the arm");
        assert!(!rot(&add, &walking, 1.0).abs_diff_eq(rot(&add, &at_rest, 1.0), 1e-2), "add: the swing is still under it");
        // eased by the weight, from whatever the clip had
        let (from, half, to) = (rot(&set, &walking, 0.0), rot(&set, &walking, 0.5), rot(&set, &walking, 1.0));
        assert!(!half.abs_diff_eq(from, 1e-3) && !half.abs_diff_eq(to, 1e-3), "halfway at half weight");
        assert!(from.abs_diff_eq(walking[arm].r, 1e-5), "and at weight 0 the clip is untouched");
    }

    #[test]
    fn the_second_beat_moves_the_pose_after_its_time_and_not_before() {
        let r = rig("ysolde");
        let def = parse_state(
            "s",
            &json!({"mode": "set", "pose": {"R-Arm": [50.0, 0.0, 0.0]}, "crouch": 0.0,
                    "then": {"after": 0.4, "over": 0.2, "pose": {"R-Arm": [10.0, 0.0, 0.0], "Head": [0.0, 30.0, 0.0]}, "crouch": 0.4}}),
        )
        .unwrap();
        let at = |t: f32| {
            let mut pose = rest_pose(&r);
            apply_state(&def, &mut Bones::new(&r, &mut pose), 1.0, t);
            (world_of(&r, &pose, "R-Hand"), world_of(&r, &pose, "Pelvis"))
        };
        let (a, b, c) = (at(0.0), at(0.39), at(0.6));
        assert!((a.0 - b.0).length() < 0.01 && (a.1 - b.1).length() < 0.01, "nothing moves before the second beat");
        assert!(c.0.z > a.0.z + 5.0, "the arm comes back: {:?} -> {:?}", a.0, c.0);
        assert!(c.1.y < a.1.y - 1.0, "and the knees bend");
        let mid = at(0.5);
        assert!(mid.0.z > a.0.z && mid.0.z < c.0.z, "eased between the beats");
    }

    #[test]
    fn a_state_crouch_lowers_the_hips_and_keeps_the_feet_down() {
        let r = rig("ysolde");
        let def = parse_state("s", &json!({"crouch": 0.5})).unwrap();
        let mut pose = rest_pose(&r);
        let (foot0, hip0) = (world_of(&r, &pose, "L-Foot"), world_of(&r, &pose, "Pelvis"));
        apply_state(&def, &mut Bones::new(&r, &mut pose), 1.0, 0.0);
        let (foot1, hip1) = (world_of(&r, &pose, "L-Foot"), world_of(&r, &pose, "Pelvis"));
        assert!(hip1.y < hip0.y - 1.0);
        assert!((foot1.y - foot0.y).abs() < 0.05, "the ankle stays on the floor");
        let mut half = rest_pose(&r);
        apply_state(&def, &mut Bones::new(&r, &mut half), 0.5, 0.0);
        assert!(world_of(&r, &half, "Pelvis").y > hip1.y, "and it eases with the weight");
    }

    #[test]
    fn the_new_state_keys_are_checked() {
        for (doc, needle) in [
            (json!({"schema":"lr.hunter_anims/1","states":{"s":{"pose":{"R-Arm":[1,2,3]},"mode":"fast"}}}), "mode"),
            (json!({"schema":"lr.hunter_anims/1","states":{"s":{"pose":{"R-Arm":[1,2,3]},"prop":"sword"}}}), "prop"),
            (json!({"schema":"lr.hunter_anims/1","states":{"s":{"pose":{"R-Arm":[1,2,3]},"crouch":3}}}), "crouch"),
            (json!({"schema":"lr.hunter_anims/1","states":{"s":{"pose":{"R-Arm":[1,2,3]},"then":{"pose":{"R-Arm":[1,2,3]},"soon":1}}}}), "soon"),
            (json!({"schema":"lr.hunter_anims/1","states":{"s":{"pose":{"R-Arm":[1,2,3]},"prop_until":-1}}}), "prop_until"),
        ] {
            let e = Table::parse(&doc.to_string()).unwrap_err();
            assert!(e.contains(needle), "{doc}: {e}");
        }
        let ok = Table::parse(r#"{"schema":"lr.hunter_anims/1","states":{"s":{"prop":"battery"}}}"#).unwrap();
        assert_eq!(ok.default.states["s"].prop.as_deref(), Some("battery"), "a prop alone is a state");
    }

    #[test]
    fn a_state_prop_is_in_the_hand_while_the_state_is_on_until_prop_until() {
        let p = state_table(
            r#"{"schema":"lr.hunter_anims/1","hunters":{"t":{"states":{
            "grab":{"pose":{"R-Arm":[60,0,0]},"prop":"battery","prop_until":0.5,"blend_in":0.1},
            "bare":{"pose":{"R-Arm":[60,0,0]}}}}}}"#,
        );
        let mut v = HunterView::new("ysolde", rig("ysolde")).with_profile(p);
        let out = run(&mut v, 20, 0.0, State::Idle, None);
        assert!(v.held_prop(&out).is_none(), "nothing in her hand with the state off");
        let out = run(&mut v, 6, 0.0, State::Idle, Some("grab"));
        let (item, _) = v.held_prop(&out).expect("the battery is in her hand");
        assert_eq!(item, crate::game::Item::Battery);
        let out = run(&mut v, 15, 0.0, State::Idle, Some("grab"));
        assert!(v.held_prop(&out).is_none(), "put away after prop_until while the pose carries on");
        let out = run(&mut v, 20, 0.0, State::Idle, None);
        assert!(v.held_prop(&out).is_none());
        let out = run(&mut v, 10, 0.0, State::Idle, Some("bare"));
        assert!(v.held_prop(&out).is_none(), "a state with no prop holds nothing");
        // the state comes on again: the timeline starts over and the battery is back
        run(&mut v, 20, 0.0, State::Idle, None);
        let out = run(&mut v, 6, 0.0, State::Idle, Some("grab"));
        assert!(v.held_prop(&out).is_some(), "the second time too");
    }

    #[test]
    fn a_stun_takes_the_prop_out_of_her_hand() {
        let p = state_table(r#"{"schema":"lr.hunter_anims/1","hunters":{"t":{"states":{"grab":{"pose":{"R-Arm":[60,0,0]},"prop":"battery"}}}}}"#);
        let mut v = HunterView::new("ysolde", rig("ysolde")).with_profile(p);
        let out = run(&mut v, 10, 0.0, State::Idle, Some("grab"));
        assert!(v.held_prop(&out).is_some());
        let mut i = input(0.0, State::Idle, Some("grab"));
        i.stun = 1.0;
        let mut out = None;
        for k in 0..20 {
            out = Some(v.update(&i, 1.0 / 30.0, 1.0 + k as f32 / 30.0).1);
        }
        assert!(v.held_prop(&out.unwrap()).is_none(), "stunned: the pose eased out, the battery is gone");
    }

    #[test]
    fn every_prop_sits_in_the_hand_and_points_forward_on_the_real_rigs() {
        let d = held_wire::data().expect("the pose table loads");
        for (id, name) in [("ysolde", "pocket_battery"), ("solyth", "burn_battery"), ("glyra", "music_box_hold"), ("glyra", "music_box_startle")] {
            let mut v = HunterView::new(id, rig(id));
            let out = run(&mut v, 12, 0.0, State::Idle, Some(name));
            let (item, hand) = v.held_prop(&out).unwrap_or_else(|| panic!("{id}/{name}: no prop"));
            let r = rig(id);
            let xf = held_wire::item_transform(d, item, out.root, hand, r.draw_scale());
            let origin = xf.transform_point3(Vec3::ZERO);
            let pivot = out.root.transform_point3(hand.transform_point3(Vec3::ZERO));
            assert!((origin - pivot).length() < 0.45, "{id}: the item is at the hand ({origin:?} vs {pivot:?})");
            // a hand hanging at rest holds it pointing the way she faces (the posed hand turns with the arm)
            let root = Mat4::from_scale(Vec3::splat(r.draw_scale()));
            let mats = super::super::hunter::model_space_stretched(&r, &rest_pose(&r), &[]);
            let rest_xf = held_wire::item_transform(d, item, root, mats[r.bone("R-Hand").unwrap()], r.draw_scale());
            let o = rest_xf.transform_point3(Vec3::ZERO);
            assert!(rest_xf.transform_point3(Vec3::Y * 0.5).z < o.z - 0.05, "{id}: at rest the item points forward (-Z)");
            let head = out.root.transform_point3(out.bones[r.bone("Head").unwrap()].transform_point3(Vec3::ZERO));
            assert!(origin.y > 0.5 && origin.y < head.y + 0.3, "{id}: and at a sensible height, {origin:?} (head {head:?})");
        }
    }

    #[test]
    fn the_shipped_states_follow_this_rigs_axes() {
        // Ysolde reaching down for a battery: bent forward, the right hand out in front and low, the head forward
        let hand = moved("ysolde", "steal_battery", "R-Hand", 0.0);
        let head = moved("ysolde", "steal_battery", "Head", 0.0);
        assert!(hand.z < -10.0 && hand.y < 5.0, "reaching out and down: {hand:?}");
        assert!(head.z < -8.0 && head.y < 0.0, "head forward and down: {head:?}");
        // the pocketing: the hand comes up to the chest, then goes to her side
        let lifted = moved("ysolde", "pocket_battery", "R-Hand", 0.0);
        let pocketed = moved("ysolde", "pocket_battery", "R-Hand", 0.9);
        assert!(lifted.y > 20.0 && lifted.z < -10.0, "the battery is lifted in front: {lifted:?}");
        assert!(pocketed.y < lifted.y - 10.0, "then the hand drops to the pocket: {pocketed:?}");
        // Glyra's box: both hands in front of her chest
        let (r, l) = (moved("glyra", "music_box_hold", "R-Hand", 0.0), moved("glyra", "music_box_hold", "L-Hand", 0.0));
        assert!(r.z < -10.0 && l.z < -10.0 && r.y > 15.0 && l.y > 15.0, "{r:?} {l:?}");
        // Aki's burst: the arms fly out to the sides (right hand to +X, left to -X)
        let (r, l) = (moved("aki", "fuse_burst", "R-Hand", 0.0), moved("aki", "fuse_burst", "L-Hand", 0.0));
        assert!(r.x > 15.0 && l.x < -15.0, "arms wide: {r:?} {l:?}");
        // Loxelle's windup: low and bowed forward
        let (rig_l, rest) = (rig("loxelle"), rest_pose(&rig("loxelle")));
        let (_, pose) = posed("loxelle", "charge_windup", 1.0, 0.0);
        assert!(world_of(&rig_l, &pose, "Pelvis").y < world_of(&rig_l, &rest, "Pelvis").y - 2.0, "hips down");
        assert!(world_of(&rig_l, &pose, "Head").z < world_of(&rig_l, &rest, "Head").z - 8.0, "head forward");
        // Nyxie / Solyth in the air: both arms stretched ahead and up
        for id in ["nyxie", "solyth"] {
            let (r, l) = (moved(id, "pounce", "R-Hand", 0.0), moved(id, "pounce", "L-Hand", 0.0));
            assert!(r.z < -20.0 && l.z < -20.0 && r.y > 15.0, "{id}: {r:?} {l:?}");
        }
    }

    #[test]
    fn the_shipped_table_names_exactly_the_animations_the_code_raises() {
        use crate::hunters::mechanics::{CUES, HELD};
        let t = Table::parse(embedded::data_under(PATH).next().unwrap().1).unwrap();
        let known: Vec<&str> = CUES.iter().chain(HELD.iter()).copied().collect();
        let mut defined: Vec<&str> = t.default.states.keys().map(String::as_str).collect();
        for (id, p) in &t.hunters {
            for name in p.states.keys() {
                assert!(known.contains(&name.as_str()), "{id}: state {name} is raised by nothing (hunters/mechanics.rs)");
                defined.push(name);
            }
        }
        for name in &known {
            assert!(defined.contains(name), "{name} is raised by the code but no hunter has a state for it");
        }
    }

    #[test]
    fn a_cues_second_beat_and_prop_fit_inside_the_cue() {
        use crate::hunters::mechanics::{cues, CUES};
        let t = Table::parse(embedded::data_under(PATH).next().unwrap().1).unwrap();
        let all = t
            .hunters
            .iter()
            .flat_map(|(id, p)| p.states.iter().map(move |(n, s)| (id.as_str(), n.as_str(), s)))
            .chain(t.default.states.iter().map(|(n, s)| ("default", n.as_str(), s)));
        for (id, name, s) in all {
            if !CUES.contains(&name) {
                continue;
            }
            let secs = cues().seconds(name);
            if let Some(th) = &s.then {
                assert!(th.after + th.over <= secs + 1e-3, "{id}/{name}: the second beat ({} + {}) outlasts the {secs} s cue", th.after, th.over);
            }
            if let Some(u) = s.prop_until {
                assert!(u <= secs, "{id}/{name}: the prop is put away at {u} s of a {secs} s cue");
            }
            assert!(s.set, "{id}/{name}: a mechanic pose should be set mode so it reads the same mid-stride");
        }
    }

    #[test]
    fn ysolde_end_to_end_reaches_pockets_and_the_battery_shows_in_her_hand_then_goes() {
        use crate::hunters::gimmicks::bench::{Bench, DT};
        let mut x = Bench::new("ysolde");
        for i in 1..x.batteries_taken.len() {
            x.batteries_taken[i] = true;
        }
        let bat = x.fac.batteries[0].center();
        let from = [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]
            .iter()
            .map(|(dx, dy)| (bat.0 + dx * 1.4, bat.1 + dy * 1.4))
            .find(|q| x.fac.can_occupy(q.0 as f64, q.1 as f64, 0.33) && x.fac.has_line_of_sight(bat, *q))
            .unwrap();
        x.place(from);
        let mut v = HunterView::new("ysolde", rig("ysolde"));
        let (mut reached, mut held, mut cleared_after) = (false, false, false);
        for k in 0..(4.0 / DT) as usize {
            x.frame();
            let m = x.mech();
            let out = v.update(&input(x.b.pos.0, State::Wander, m), DT, k as f32 * DT).1;
            reached |= m == Some("steal_battery");
            let has = v.held_prop(&out).is_some();
            held |= has;
            cleared_after |= held && m.is_none();
            assert!(!has || matches!(m, Some("pocket_battery")), "the battery is only in her hand while she pockets it");
        }
        assert!(reached && held, "she reached, then held the battery");
        assert!(cleared_after, "and the pose cleared by itself afterwards");
    }
}
