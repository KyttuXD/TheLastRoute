//! Player emotes as a procedural pose on top of the mannequin rig. `data/player/emotes.json` holds everything tunable:
//! the hold key, the blend times, the wheel's size, and for each emote its id, name, duration and keyframes. A keyframe
//! names bones (the rig's own names, like "R-Arm" or "Head") and gives each an extra rotation in degrees (x, y, z),
//! applied on top of the clip's pose. A bone a key does not name is at rest in that key. The pose is blended in over
//! `blend_in` seconds and out over `blend_out` seconds, so it never pops.
//!
//! This file is only the data and the pose maths: `game/emote.rs` holds the state (wheel, active emote, cancel rules),
//! `ui/emote_wheel.rs` the wheel, and `net/runner_figure.rs` applies a pose to a body.

use super::clip::Rig;
use super::player::Local;
use crate::content::embedded;
use macroquad::prelude::{EulerRot, Quat};
use std::sync::OnceLock;

/// The data file (under src/bin/last_route/data).
pub const FILE: &str = "player/emotes.json";

/// The most emotes the wheel holds.
pub const MAX_EMOTES: usize = 8;

/// One keyframe: at `t` seconds, these bones are rotated by these degrees (x, y, z).
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub t: f32,
    pub bones: Vec<(String, [f32; 3])>,
}

/// One emote.
#[derive(Clone, Debug, PartialEq)]
pub struct Emote {
    pub id: String,
    /// The wheel label (upper case).
    pub name: String,
    /// How long it plays, seconds.
    pub duration: f32,
    /// A looping emote repeats its keys every `period` seconds until `duration` ends.
    pub looping: bool,
    pub period: f32,
    pub keys: Vec<Key>,
}

/// The wheel's look, as fractions of the shorter screen side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelSpec {
    pub radius: f32,
    /// The inner dead zone, as a fraction of `radius`: a pointer inside it picks nothing.
    pub inner_fraction: f32,
    /// A stick pushed less than this far (0..1) picks nothing.
    pub stick_dead_zone: f32,
    pub text_scale: f32,
}

/// Everything in emotes.json.
#[derive(Clone, Debug, PartialEq)]
pub struct EmoteSet {
    /// The default key to hold (a macroquad key name, like "G").
    pub key: String,
    pub blend_in: f32,
    pub blend_out: f32,
    pub cancel_on_move: bool,
    pub cancel_on_sprint: bool,
    pub wheel: WheelSpec,
    pub emotes: Vec<Emote>,
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    let n = v[key].as_f64().ok_or_else(|| format!("{key} is missing or not a number"))? as f32;
    if n.is_finite() {
        Ok(n)
    } else {
        Err(format!("{key} is not finite"))
    }
}

fn flag(v: &serde_json::Value, key: &str) -> Result<bool, String> {
    v[key].as_bool().ok_or_else(|| format!("{key} is missing or not true/false"))
}

fn text<'a>(v: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    v[key].as_str().ok_or_else(|| format!("{key} is missing or not a string"))
}

fn parse_emote(v: &serde_json::Value) -> Result<Emote, String> {
    let id = text(v, "id")?.to_string();
    let err = |e: String| format!("emote {id}: {e}");
    let duration = num(v, "duration").map_err(err)?;
    if duration <= 0.0 {
        return Err(err("duration must be above 0".into()));
    }
    let looping = v["loop"].as_bool().unwrap_or(false);
    let period = if looping { num(v, "period").map_err(err)? } else { duration };
    if period <= 0.0 {
        return Err(err("period must be above 0".into()));
    }
    let mut keys = vec![];
    for k in v["keys"].as_array().ok_or_else(|| err("keys is missing or not a list".into()))? {
        let t = num(k, "t").map_err(err)?;
        let mut bones = vec![];
        for (name, a) in k["bones"].as_object().ok_or_else(|| err("a key has no bones object".into()))? {
            let a = a.as_array().filter(|a| a.len() == 3).ok_or_else(|| err(format!("{name} is not [x, y, z]")))?;
            let mut r = [0.0f32; 3];
            for (o, n) in r.iter_mut().zip(a) {
                *o = n.as_f64().filter(|n| n.is_finite()).ok_or_else(|| err(format!("{name} holds a bad number")))? as f32;
            }
            bones.push((name.clone(), r));
        }
        keys.push(Key { t, bones });
    }
    if keys.is_empty() || keys.windows(2).any(|w| w[1].t <= w[0].t) {
        return Err(err("keys must be one or more, with rising times".into()));
    }
    Ok(Emote { name: text(v, "name").map_err(err)?.to_string(), id, duration, looping, period, keys })
}

impl EmoteSet {
    /// Reads emotes.json. Every value is required; a bad one is an error, never a silent default.
    pub fn parse(src: &str) -> Result<EmoteSet, String> {
        let v: serde_json::Value = serde_json::from_str(src).map_err(|e| e.to_string())?;
        let w = &v["wheel"];
        let emotes = v["emotes"]
            .as_array()
            .ok_or("emotes is missing or not a list")?
            .iter()
            .map(parse_emote)
            .collect::<Result<Vec<_>, _>>()?;
        if emotes.is_empty() || emotes.len() > MAX_EMOTES {
            return Err(format!("emotes must hold 1 to {MAX_EMOTES} entries"));
        }
        for (i, e) in emotes.iter().enumerate() {
            if e.id.is_empty() || emotes[..i].iter().any(|o| o.id == e.id) {
                return Err(format!("emote id '{}' is empty or repeated", e.id));
            }
        }
        Ok(EmoteSet {
            key: text(&v, "key")?.to_string(),
            blend_in: num(&v, "blend_in")?,
            blend_out: num(&v, "blend_out")?,
            cancel_on_move: flag(&v, "cancel_on_move")?,
            cancel_on_sprint: flag(&v, "cancel_on_sprint")?,
            wheel: WheelSpec {
                radius: num(w, "radius").map_err(|e| format!("wheel.{e}"))?,
                inner_fraction: num(w, "inner_fraction").map_err(|e| format!("wheel.{e}"))?,
                stick_dead_zone: num(w, "stick_dead_zone").map_err(|e| format!("wheel.{e}"))?,
                text_scale: num(w, "text_scale").map_err(|e| format!("wheel.{e}"))?,
            },
            emotes,
        })
    }

    /// The emote with this id; None for an id this build does not know (a newer peer's).
    pub fn find(&self, id: &str) -> Option<&Emote> {
        self.emotes.iter().find(|e| e.id == id)
    }

    /// The blend weight (0..1) of `e` at `t` seconds after it began: eased in, eased out, 0 outside its length.
    pub fn weight(&self, e: &Emote, t: f32) -> f32 {
        if !(t >= 0.0 && t < e.duration) {
            return 0.0;
        }
        let ramp = |dist: f32, len: f32| if len > 1e-4 { (dist / len).clamp(0.0, 1.0) } else { 1.0 };
        let x = ramp(t, self.blend_in).min(ramp(e.duration - t, self.blend_out));
        x * x * (3.0 - 2.0 * x)
    }
}

/// The embedded set, read once. None when the file is missing or invalid (the error is printed once): no emotes then.
pub fn set() -> Option<&'static EmoteSet> {
    static SET: OnceLock<Option<EmoteSet>> = OnceLock::new();
    SET.get_or_init(|| {
        let (path, src) = embedded::data_under(FILE).next()?;
        match EmoteSet::parse(src) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("{path}: {e}");
                None
            }
        }
    })
    .as_ref()
}

/// Each animated bone's rotation (degrees x, y, z) at `t` seconds into `e`, before blending. A looping emote wraps `t`
/// by its period. A bone absent from a key is at rest there.
pub fn offsets(e: &Emote, t: f32) -> Vec<(&str, [f32; 3])> {
    let t = if e.looping { t.rem_euclid(e.period) } else { t };
    let next = e.keys.iter().position(|k| k.t >= t).unwrap_or(e.keys.len() - 1);
    let (a, b) = if next == 0 { (&e.keys[0], &e.keys[0]) } else { (&e.keys[next - 1], &e.keys[next]) };
    let f = if b.t > a.t { ((t - a.t) / (b.t - a.t)).clamp(0.0, 1.0) } else { 1.0 };
    let at = |k: &Key, name: &str| k.bones.iter().find(|(n, _)| n == name).map(|(_, r)| *r).unwrap_or([0.0; 3]);
    let mut out: Vec<(&str, [f32; 3])> = vec![];
    for (name, _) in a.bones.iter().chain(b.bones.iter()) {
        if out.iter().any(|(n, _)| n == name) {
            continue;
        }
        let (ra, rb) = (at(a, name), at(b, name));
        out.push((name.as_str(), [0, 1, 2].map(|i| ra[i] + (rb[i] - ra[i]) * f)));
    }
    out
}

/// Poses `locals` (the rig's bone pose, in rig bone order) for emote `id` at `t` seconds after it began. Returns false
/// and leaves `locals` alone for an unknown id, a finished emote, or a rig without the bones.
pub fn apply(locals: &mut [Local], rig: &Rig, set: &EmoteSet, id: &str, t: f32) -> bool {
    let Some(e) = set.find(id) else {
        return false;
    };
    let w = set.weight(e, t);
    if w <= 0.0 {
        return false;
    }
    let mut any = false;
    for (name, deg) in offsets(e, t) {
        let Some(l) = rig.bone(name).and_then(|i| locals.get_mut(i)) else {
            continue;
        };
        let q = Quat::from_euler(EulerRot::XYZ, deg[0].to_radians(), deg[1].to_radians(), deg[2].to_radians());
        l.r *= Quat::IDENTITY.slerp(q, w);
        any = true;
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> EmoteSet {
        let (_, src) = embedded::data_under(FILE).next().expect("player/emotes.json is embedded");
        EmoteSet::parse(src).expect("emotes.json parses")
    }

    #[test]
    fn shipped_data_parses_and_fits_the_wheel() {
        let s = shipped();
        assert!((6..=MAX_EMOTES).contains(&s.emotes.len()));
        assert_eq!(s.key, "B", "G is the taser");
        for e in &s.emotes {
            assert_eq!(e.name, e.name.to_uppercase());
            assert!(e.duration > 0.0 && e.keys.last().unwrap().t <= e.duration.max(e.period) + 1e-4);
        }
        assert!(s.find("wave").is_some() && s.find("nope").is_none());
    }

    #[test]
    fn bad_data_is_an_error() {
        assert!(EmoteSet::parse("{}").is_err());
        let src = include_str!("../../data/player/emotes.json").replace("\"duration\": 2.4", "\"duration\": 0");
        assert!(EmoteSet::parse(&src).is_err());
        let dup = include_str!("../../data/player/emotes.json").replace("\"id\": \"no\"", "\"id\": \"yes\"");
        assert!(EmoteSet::parse(&dup).is_err());
    }

    #[test]
    fn weight_blends_in_and_out() {
        let s = shipped();
        let e = s.find("wave").unwrap();
        assert_eq!(s.weight(e, -0.1), 0.0);
        assert_eq!(s.weight(e, 0.0), 0.0);
        let mid = s.weight(e, e.duration / 2.0);
        assert!((mid - 1.0).abs() < 1e-5);
        assert!(s.weight(e, s.blend_in / 2.0) < mid);
        assert!(s.weight(e, e.duration - s.blend_out / 2.0) < mid);
        assert_eq!(s.weight(e, e.duration), 0.0);
    }

    #[test]
    fn offsets_interpolate_between_keys() {
        let s = shipped();
        let e = s.find("no").unwrap();
        let head = |t: f32| offsets(e, t).iter().find(|(n, _)| *n == "Head").unwrap().1[1];
        assert!((head(0.2) - 35.0).abs() < 1e-3);
        assert!(head(0.35).abs() < 1.0 + 1e-3);
        assert!((head(0.5) + 35.0).abs() < 1e-3);
    }

    #[test]
    fn a_looping_emote_repeats_each_period() {
        let s = shipped();
        let e = s.find("dance").unwrap();
        assert!(e.looping);
        let a = offsets(e, 0.3);
        let b = offsets(e, 0.3 + e.period * 2.0);
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.0, y.0);
            for i in 0..3 {
                assert!((x.1[i] - y.1[i]).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn apply_poses_bones_and_ignores_unknown_ids() {
        let Some(rig) = super::super::runner::player_rig(false) else {
            return;
        };
        let s = shipped();
        let rest = super::super::player::AnimPlayer::default().pose(&rig);
        let mut posed = rest.clone();
        assert!(apply(&mut posed, &rig, &s, "wave", 0.9));
        let arm = rig.bone("R-Arm").unwrap();
        assert!(posed[arm].r.dot(rest[arm].r).abs() < 0.99);
        let mut other = rest.clone();
        assert!(!apply(&mut other, &rig, &s, "no-such-emote", 0.9));
        assert!(!apply(&mut other, &rig, &s, "wave", 99.0));
        assert!(other.iter().zip(&rest).all(|(a, b)| a.r == b.r));
    }
}
