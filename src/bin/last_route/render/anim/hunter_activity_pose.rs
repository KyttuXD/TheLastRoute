//! Hunter activity poses and presentation extras, layered on the humanoid rigs.
//!
//! * `data/render/hunter_activity_poses.json` holds, per activity (locker inspection lean, sniff, look-around sweep, door
//!   push, door knock, listen, corner peek left / right, vent crawl, attack lunge), keyframed bone rotations in degrees,
//!   blended in and out like the player emote poses (`emote_pose.rs`, whose `Emote` / `offsets` this reuses). The view
//!   (`hunter.rs`) eases a weight toward the activity the brain names and calls [`apply`].
//! * The same file says how Kathy's scythe is carried (clip swaps, the swing one-shot, where the prop rides).
//! * [`ActivityRead`] is how the scene reads the activity from a brain; [`Extras`] is what it hands the view next to the
//!   view's `Input` (the activity name and the hunter emote index).
//! * Hunter emote voice cues are queued here ([`push_cue`] / [`drain_cues`]) for audio to drain.

use super::clip::{key_quat, Rig};
use super::emote_pose::{offsets, Emote, Key};
use super::player::Local;
use crate::content::embedded;
use crate::hunters::brain::Brain;
use crate::hunters::gimmicks::Gimmick;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The data file (under src/bin/last_route/data).
pub const FILE: &str = "render/hunter_activity_poses.json";
pub const SCHEMA: &str = "lr.hunter_activity_poses/1";

/// Every activity name the brain may raise (a name with no data row does nothing).
pub const ACTIVITIES: [&str; 10] = [
    "locker_inspect",
    "sniff",
    "look_around",
    "door_push",
    "door_knock",
    "listen",
    "corner_peek_left",
    "corner_peek_right",
    "vent_crawl",
    "attack_lunge",
];

/// One activity pose.
#[derive(Clone, Debug, PartialEq)]
pub struct Def {
    pub name: String,
    pub blend_in: f32,
    pub blend_out: f32,
    pub pose: Emote,
}

/// Kathy's scythe.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Scythe {
    pub hunters: Vec<String>,
    pub bone: String,
    pub fallback_bone: String,
    pub model: String,
    /// plain clip name -> the scythe clip that replaces it
    pub clips: HashMap<String, String>,
    pub swing_clip: String,
    /// The activity that starts the swing one-shot.
    pub swing_on: String,
    pub offset_m: [f32; 3],
    pub rot_deg: [f32; 3],
    pub scale: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Table {
    pub activities: Vec<Def>,
    /// The body clip a peeking hunter plays instead of the walk clip.
    pub peek_clip: String,
    pub scythe: Scythe,
    /// A playing hunter emote's face replaces the state's face.
    pub emote_face_override: bool,
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    let n = v[key].as_f64().ok_or_else(|| format!("{key} is missing or not a number"))? as f32;
    if n.is_finite() {
        Ok(n)
    } else {
        Err(format!("{key} is not finite"))
    }
}

fn text(v: &serde_json::Value, key: &str) -> Result<String, String> {
    v[key].as_str().map(str::to_string).ok_or_else(|| format!("{key} is missing or not a string"))
}

fn triple(v: &serde_json::Value, key: &str) -> Result<[f32; 3], String> {
    let a = v[key].as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("{key} is not [x, y, z]"))?;
    let mut r = [0.0f32; 3];
    for (o, n) in r.iter_mut().zip(a) {
        *o = n.as_f64().filter(|n| n.is_finite()).ok_or_else(|| format!("{key} holds a bad number"))? as f32;
    }
    Ok(r)
}

fn parse_def(name: &str, v: &serde_json::Value) -> Result<Def, String> {
    let err = |e: String| format!("activity {name}: {e}");
    let blend_in = num(v, "blend_in").map_err(err)?;
    let blend_out = num(v, "blend_out").map_err(err)?;
    if blend_in < 0.0 || blend_out < 0.0 {
        return Err(err("blend times must not be negative".into()));
    }
    let looping = v["loop"].as_bool().ok_or_else(|| err("loop is missing or not true/false".into()))?;
    let period = num(v, "period").map_err(err)?;
    if period <= 0.0 {
        return Err(err("period must be above 0".into()));
    }
    let mut keys = vec![];
    for k in v["keys"].as_array().ok_or_else(|| err("keys is missing or not a list".into()))? {
        let t = num(k, "t").map_err(err)?;
        let mut bones = vec![];
        for (bone, _) in k["bones"].as_object().ok_or_else(|| err("a key has no bones object".into()))? {
            let r = triple(&k["bones"], bone).map_err(err)?;
            bones.push((bone.clone(), r));
        }
        keys.push(Key { t, bones });
    }
    if keys.is_empty() || keys.windows(2).any(|w| w[1].t <= w[0].t) {
        return Err(err("keys must be one or more, with rising times".into()));
    }
    let pose = Emote { id: name.to_string(), name: name.to_uppercase(), duration: period, looping, period, keys };
    Ok(Def { name: name.to_string(), blend_in, blend_out, pose })
}

impl Table {
    /// Reads the file. Every value is required; a bad one is an error, never a silent default.
    pub fn parse(src: &str) -> Result<Table, String> {
        let v: serde_json::Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
        if v["schema"].as_str() != Some(SCHEMA) {
            return Err(format!("schema must be {SCHEMA}"));
        }
        let mut activities = vec![];
        for (name, def) in v["activities"].as_object().ok_or("activities is missing or not an object")? {
            if !ACTIVITIES.contains(&name.as_str()) {
                return Err(format!("activity {name} is not one of the known activities"));
            }
            activities.push(parse_def(name, def)?);
        }
        let s = &v["scythe"];
        let strings = |key: &str| -> Result<HashMap<String, String>, String> {
            s[key]
                .as_object()
                .ok_or_else(|| format!("scythe.{key} is missing or not an object"))?
                .iter()
                .map(|(k, c)| c.as_str().map(|c| (k.clone(), c.to_string())).ok_or_else(|| format!("scythe.{key}.{k} is not a string")))
                .collect()
        };
        let scale = num(s, "scale").map_err(|e| format!("scythe.{e}"))?;
        if scale <= 0.0 {
            return Err("scythe.scale must be above 0".into());
        }
        let scythe = Scythe {
            hunters: s["hunters"]
                .as_array()
                .ok_or("scythe.hunters is missing or not a list")?
                .iter()
                .filter_map(|h| h.as_str().map(str::to_string))
                .collect(),
            bone: text(s, "bone").map_err(|e| format!("scythe.{e}"))?,
            fallback_bone: text(s, "fallback_bone").map_err(|e| format!("scythe.{e}"))?,
            model: text(s, "model").map_err(|e| format!("scythe.{e}"))?,
            clips: strings("clips")?,
            swing_clip: text(s, "swing_clip").map_err(|e| format!("scythe.{e}"))?,
            swing_on: text(s, "swing_on").map_err(|e| format!("scythe.{e}"))?,
            offset_m: triple(s, "offset_m").map_err(|e| format!("scythe.{e}"))?,
            rot_deg: triple(s, "rot_deg").map_err(|e| format!("scythe.{e}"))?,
            scale,
        };
        Ok(Table {
            activities,
            peek_clip: text(&v, "peek_clip")?,
            scythe,
            emote_face_override: v["emote"]["face_override"].as_bool().ok_or("emote.face_override is missing or not true/false")?,
        })
    }

    pub fn find(&self, name: &str) -> Option<&Def> {
        self.activities.iter().find(|d| d.name == name)
    }

    /// Whether this hunter carries the scythe.
    pub fn carries_scythe(&self, hunter_id: &str) -> bool {
        self.scythe.hunters.iter().any(|h| h == hunter_id)
    }
}

/// The embedded table, read once. An invalid file prints its error once and gives an empty table: no poses then.
pub fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let Some((path, src)) = embedded::data_under(FILE).next() else {
            return Table::default();
        };
        Table::parse(src).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            Table::default()
        })
    })
}

/// Poses `locals` (the rig's bone pose, in rig bone order) for `def` at `t` seconds after it began, at blend weight `w`
/// (0..1). The rotation is laid on the animated pose in the parent's frame, like the mechanic states' "add" mode. Returns
/// false and leaves `locals` alone for a zero weight or a rig without any of the bones.
pub fn apply(locals: &mut [Local], rig: &Rig, def: &Def, t: f32, w: f32) -> bool {
    if w <= 0.0 {
        return false;
    }
    let mut any = false;
    for (bone, deg) in offsets(&def.pose, t) {
        let Some(l) = rig.bone(bone).and_then(|i| locals.get_mut(i)) else {
            continue;
        };
        l.r = key_quat(vec3(deg[0], deg[1], deg[2]) * w) * l.r;
        any = true;
    }
    any
}

/// The way a corner peeker leans, as an activity name: right when the lean direction is on the right of her heading
/// (she faces -Z at yaw 0, right is +X), left otherwise.
pub fn peek_activity(yaw: f32, lean: (f32, f32)) -> &'static str {
    let fwd = (-yaw.sin(), -yaw.cos());
    if fwd.0 * lean.1 - fwd.1 * lean.0 > 0.0 {
        "corner_peek_right"
    } else {
        "corner_peek_left"
    }
}

/// Where the scythe model rides: the hand bone's matrix (model units) under the figure's root, with the data's offset,
/// rotation and scale in metres.
pub fn scythe_transform(s: &Scythe, root: Mat4, hand: Mat4, draw_scale: f32) -> Mat4 {
    root * hand
        * Mat4::from_scale(Vec3::splat(1.0 / draw_scale.max(1e-6)))
        * Mat4::from_translation(vec3(s.offset_m[0], s.offset_m[1], s.offset_m[2]))
        * Mat4::from_quat(key_quat(vec3(s.rot_deg[0], s.rot_deg[1], s.rot_deg[2])))
        * Mat4::from_scale(Vec3::splat(s.scale))
}

// ------------------------------------------------------------------------------------------------- brain side

/// How the scene reads what a brain is doing. Task 1 gives the brain an activity; until then this answers None. Hook: make
/// the body `self.activity.map(|a| a.name())` (a name from [`ACTIVITIES`]).
pub trait ActivityRead {
    fn activity_name(&self) -> Option<&'static str>;
}

impl ActivityRead for Brain {
    fn activity_name(&self) -> Option<&'static str> {
        use crate::hunters::activities::Activity as A;
        self.activity_view().map(|(a, _)| match a {
            A::InspectLocker => "locker_inspect",
            A::LookAround | A::CheckRoom | A::Fidget | A::Pace => "look_around",
            A::CornerPeek => "corner_peek_left",
            A::Sniff => "sniff",
            A::Listen => "listen",
            A::Knock => "door_knock",
            A::Push => "door_push",
        })
    }
}

/// Whether this gimmick is mid-leap (Nyxie's or Solyth's pounce): the view plays the charge clip, as the original does.
pub fn leaping(g: &Gimmick) -> bool {
    match g {
        Gimmick::Nyxie(n) => n.holds_position(),
        Gimmick::Solyth(s) => s.holds_position(),
        _ => false,
    }
}

/// What the scene hands a hunter's view each frame besides its `Input`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Extras {
    /// The brain's activity (see [`ACTIVITIES`]).
    pub activity: Option<&'static str>,
    /// The emote playing (index into the emote catalogue, the wire's `emote`).
    pub emote: Option<usize>,
}

/// The voice cue of a hunter emote that just began (`hunters/emotes.rs` `noise` is its audible radius, 0 = silent).
#[derive(Clone, Debug, PartialEq)]
pub struct EmoteCue {
    pub hunter: String,
    pub index: usize,
    pub id: String,
    pub noise: f32,
    pub pos: (f32, f32),
}

const MAX_CUES: usize = 32;

thread_local! {
    static CUES: RefCell<Vec<EmoteCue>> = const { RefCell::new(Vec::new()) };
}

pub fn push_cue(c: EmoteCue) {
    CUES.with(|q| {
        let mut q = q.borrow_mut();
        if q.len() >= MAX_CUES {
            q.remove(0);
        }
        q.push(c);
    });
}

/// Takes the queued emote cues. Hook: the audio frame calls this and plays each hunter's emote voice at `pos`.
pub fn drain_cues() -> Vec<EmoteCue> {
    CUES.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

#[cfg(test)]
mod tests {
    use super::super::clip::rig;
    use super::super::hunter::{HunterView, Input};
    use super::super::player::AnimPlayer;
    use super::*;
    use crate::hunters::brain::State;

    fn input(x: f32, state: State) -> Input<'static> {
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
            mechanic: None,
        }
    }

    /// `n` frames at 30 fps; `at` is the frame counter, so the game clock carries on across calls.
    fn go(v: &mut HunterView, at: &mut usize, n: usize, extras: Extras, i: &Input) -> Vec<Local> {
        let mut pose = vec![];
        for _ in 0..n {
            v.set_extras(extras);
            pose = v.update(i, 1.0 / 30.0, *at as f32 / 30.0).0;
            *at += 1;
        }
        pose
    }

    fn run(v: &mut HunterView, n: usize, extras: Extras, i: &Input) -> Vec<Local> {
        go(v, &mut 0, n, extras, i)
    }

    fn differs(a: &[Local], b: &[Local], bone: usize) -> bool {
        a[bone].r.dot(b[bone].r).abs() < 0.9995
    }

    #[test]
    fn shipped_data_parses_and_covers_every_activity() {
        let (_, src) = embedded::data_under(FILE).next().expect("the poses file is embedded");
        let t = Table::parse(src).expect("it parses");
        for a in ACTIVITIES {
            let d = t.find(a).unwrap_or_else(|| panic!("{a} has no row"));
            assert!(d.blend_in > 0.0 && d.blend_out > 0.0, "{a} blends");
        }
        assert!(t.carries_scythe("kathy") && !t.carries_scythe("aki"));
        assert!(Table::parse("{}").is_err());
        assert!(Table::parse(&src.replace("\"sniff\"", "\"snuffle\"")).is_err(), "an unknown activity is refused");
        assert!(Table::parse(&src.replace("\"period\": 0.9", "\"period\": 0")).is_err());
    }

    #[test]
    fn every_pose_bone_exists_in_the_humanoid_rigs() {
        for id in ["aki", "kathy", "nyxie"] {
            let r = rig(id).unwrap();
            for d in &table().activities {
                for k in &d.pose.keys {
                    for (bone, _) in &k.bones {
                        assert!(r.bone(bone).is_some(), "{id}: {} names {bone}", d.name);
                    }
                }
            }
        }
    }

    #[test]
    fn each_activity_pose_changes_its_named_bones_and_zero_weight_is_rest() {
        let r = rig("aki").unwrap();
        let rest = AnimPlayer::default().pose(&r);
        for d in &table().activities {
            let mut posed = rest.clone();
            assert!(apply(&mut posed, &r, d, 0.4, 1.0), "{}", d.name);
            let named: Vec<usize> = d.pose.keys.iter().flat_map(|k| k.bones.iter()).filter_map(|(b, _)| r.bone(b)).collect();
            assert!(named.iter().any(|&b| differs(&posed, &rest, b)), "{} moves a named bone", d.name);
            for (i, (p, q)) in posed.iter().zip(&rest).enumerate() {
                if !named.contains(&i) {
                    assert_eq!(p.r, q.r, "{}: bone {i} is not named and stays put", d.name);
                }
            }
            let mut still = rest.clone();
            assert!(!apply(&mut still, &r, d, 0.4, 0.0));
            assert!(still.iter().zip(&rest).all(|(a, b)| a.r == b.r), "{}: weight 0 is rest", d.name);
        }
    }

    #[test]
    fn a_looping_pose_repeats_and_a_held_one_keeps_its_last_key() {
        let t = table();
        let sniff = t.find("sniff").unwrap();
        let (a, b) = (offsets(&sniff.pose, 0.3), offsets(&sniff.pose, 0.3 + sniff.pose.period));
        assert!(a.iter().zip(&b).all(|(x, y)| (0..3).all(|i| (x.1[i] - y.1[i]).abs() < 1e-3)));
        let lunge = t.find("attack_lunge").unwrap();
        let (a, b) = (offsets(&lunge.pose, 5.0), offsets(&lunge.pose, 50.0));
        assert!(a.iter().zip(&b).all(|(x, y)| x.1 == y.1), "held");
    }

    #[test]
    fn the_view_poses_the_activity_then_returns_to_rest() {
        let still = input(0.0, State::Idle);
        for name in ACTIVITIES.iter().filter(|n| !n.starts_with("corner_peek")) {
            let mut base = HunterView::new("aki", rig("aki").unwrap());
            let rest = run(&mut base, 140, Extras::default(), &still);
            let mut v = HunterView::new("aki", rig("aki").unwrap());
            let mut at = 0;
            go(&mut v, &mut at, 20, Extras::default(), &still);
            let posed = go(&mut v, &mut at, 30, Extras { activity: Some(name), emote: None }, &still);
            let named: Vec<usize> = table()
                .find(name)
                .unwrap()
                .pose
                .keys
                .iter()
                .flat_map(|k| k.bones.iter())
                .filter_map(|(b, _)| v.rig().bone(b))
                .collect();
            let reference = run(&mut HunterView::new("aki", rig("aki").unwrap()), 50, Extras::default(), &still);
            assert!(named.iter().any(|&b| differs(&posed, &reference, b)), "{name}: the view poses it");
            let after = go(&mut v, &mut at, 90, Extras::default(), &still);
            for b in 0..after.len() {
                assert!(!differs(&after, &rest, b), "{name}: bone {b} returned to rest");
            }
        }
    }

    #[test]
    fn an_activity_is_ignored_while_stunned_or_held() {
        let mut v = HunterView::new("aki", rig("aki").unwrap());
        let mut stunned = input(0.0, State::Idle);
        stunned.stun = 3.0;
        let mut w = HunterView::new("aki", rig("aki").unwrap());
        let a = run(&mut v, 40, Extras { activity: Some("attack_lunge"), emote: None }, &stunned);
        let b = run(&mut w, 40, Extras::default(), &stunned);
        for i in 0..a.len() {
            assert!(!differs(&a, &b, i), "stunned: bone {i}");
        }
    }

    #[test]
    fn a_peeking_hunter_leans_left_or_right_and_does_not_walk() {
        assert_eq!(peek_activity(0.0, (1.0, 0.0)), "corner_peek_right");
        assert_eq!(peek_activity(0.0, (-1.0, 0.0)), "corner_peek_left");
        let r = rig("aki").unwrap();
        let rest = AnimPlayer::default().pose(&r);
        for name in ["corner_peek_left", "corner_peek_right"] {
            let mut p = rest.clone();
            assert!(apply(&mut p, &r, table().find(name).unwrap(), 0.4, 1.0));
            assert!(differs(&p, &rest, r.bone("Chest").unwrap()), "{name} leans the chest");
        }
        let mut v = HunterView::new("aki", r);
        let mut i = input(0.0, State::Peek);
        i.peek_lean = Some((1.0, 0.0));
        let mut out = vec![];
        for k in 0..60 {
            out = v.update(&i, 1.0 / 30.0, k as f32 / 30.0).0;
        }
        assert_eq!(v.clip(), table().peek_clip, "the peek plays its own clip, not the walk");
        let mut plain = HunterView::new("aki", rig("aki").unwrap());
        let mut j = input(0.0, State::Peek);
        j.peek_lean = None;
        let mut flat = vec![];
        for k in 0..60 {
            flat = plain.update(&j, 1.0 / 30.0, k as f32 / 30.0).0;
        }
        assert!(differs(&out, &flat, v.rig().bone("Chest").unwrap()), "the peek pose is on while she peeks out");
    }

    #[test]
    fn kathy_picks_the_scythe_clips() {
        let mut k = HunterView::new("kathy", rig("kathy").unwrap());
        run(&mut k, 30, Extras::default(), &input(0.0, State::Idle));
        assert_eq!(k.clip(), "scythe_idle");
        let mut x = 0.0;
        for n in 0..60 {
            x += 1.4 / 30.0;
            k.update(&input(x, State::Wander), 1.0 / 30.0, n as f32 / 30.0);
        }
        assert_eq!(k.clip(), "scythe_walk");
        let mut s = HunterView::new("kathy", rig("kathy").unwrap());
        let mut stunned = input(0.0, State::Idle);
        stunned.stun = 2.0;
        run(&mut s, 10, Extras::default(), &stunned);
        assert_eq!(s.clip(), "scythe_stun");
        let mut up = HunterView::new("kathy", rig("kathy").unwrap());
        run(&mut up, 10, Extras::default(), &stunned);
        run(&mut up, 3, Extras::default(), &input(0.0, State::Idle));
        assert_eq!(up.clip(), "scythe_stun_recover");
        let mut a = HunterView::new("kathy", rig("kathy").unwrap());
        run(&mut a, 10, Extras::default(), &input(0.0, State::Idle));
        run(&mut a, 5, Extras { activity: Some("attack_lunge"), emote: None }, &input(0.0, State::Chase));
        assert_eq!(a.clip(), "scythe_swing", "the lunge swings the scythe");
        let mut other = HunterView::new("aki", rig("aki").unwrap());
        run(&mut other, 30, Extras::default(), &input(0.0, State::Idle));
        assert_eq!(other.clip(), "idle", "a hunter without the scythe keeps the plain clips");
    }

    #[test]
    fn the_scythe_rides_the_hand_only_on_its_carrier() {
        let mut k = HunterView::new("kathy", rig("kathy").unwrap());
        let (_, out) = k.update(&input(0.0, State::Idle), 1.0 / 30.0, 0.0);
        let (model, xf) = k.scythe_prop(&out).expect("Kathy carries it");
        assert_eq!(model, table().scythe.model);
        assert!(xf.is_finite());
        let mut a = HunterView::new("aki", rig("aki").unwrap());
        let (_, out) = a.update(&input(0.0, State::Idle), 1.0 / 30.0, 0.0);
        assert!(a.scythe_prop(&out).is_none());
    }

    #[test]
    fn a_leaping_hunter_uses_the_charge_clip() {
        for id in ["nyxie", "solyth"] {
            let Some(r) = rig(id) else { continue };
            if !r.has_clip("charge") {
                continue;
            }
            let mut v = HunterView::new(id, r);
            let mut x = 0.0;
            let mut i = input(0.0, State::Chase);
            i.charging = true;
            for n in 0..30 {
                x += 3.0 / 30.0;
                i.pos = (x, 5.0);
                v.update(&i, 1.0 / 30.0, n as f32 / 30.0);
            }
            assert_eq!(v.clip(), "charge", "{id}");
        }
        assert!(!leaping(&Gimmick::None));
    }

    #[test]
    fn an_emote_reaches_the_view_state() {
        let cat = crate::hunters::emotes::Catalogue::get();
        let idx = cat.index_of("wave").expect("wave is in the catalogue");
        let face = cat.emote(idx).unwrap().face.clone();
        assert_eq!(face, "happy");
        drain_cues();
        let mut v = HunterView::new("aki", rig("aki").unwrap());
        let i = input(0.0, State::Idle);
        let (_, none) = v.update(&i, 1.0 / 30.0, 0.0);
        assert_eq!(none.face_cell, 0);
        v.set_extras(Extras { activity: None, emote: Some(idx) });
        for n in 1..20 {
            v.set_extras(Extras { activity: None, emote: Some(idx) });
            v.update(&i, 1.0 / 30.0, n as f32 / 30.0);
        }
        let (_, out) = v.update(&i, 1.0 / 30.0, 1.0);
        assert_eq!(out.face_cell, 5, "the emote's happy face shows");
        let cues = drain_cues();
        assert_eq!(cues.len(), 1, "one voice cue per emote, not per frame");
        assert_eq!(cues[0].id, "wave");
        assert_eq!(cues[0].hunter, "aki");
        v.set_extras(Extras::default());
        for n in 20..80 {
            v.update(&i, 1.0 / 30.0, 1.0 + n as f32 / 30.0);
        }
        let (_, out) = v.update(&i, 1.0 / 30.0, 4.0);
        assert_ne!(out.face_cell, 5, "the face returns when the emote ends");
    }

    #[test]
    fn cues_are_bounded() {
        drain_cues();
        for n in 0..(MAX_CUES + 8) {
            push_cue(EmoteCue { hunter: "aki".into(), index: n, id: "wave".into(), noise: 0.0, pos: (0.0, 0.0) });
        }
        let q = drain_cues();
        assert_eq!(q.len(), MAX_CUES);
        assert_eq!(q.last().unwrap().index, MAX_CUES + 7);
        assert!(drain_cues().is_empty());
    }
}
