//! A hunter's rig and clips (assets/hunters/anim/<id>.json, extracted from the original's GLBs by extract.py), evaluated
//! the way Godot does it:
//!
//! * import: Godot's glTF importer (gltf_document.cpp `_import_animation`, animation/fps = 30) BAKES every TRS track
//!   onto a 1/30 s grid from 0 to the clip's last key (the last sample exactly at the end), each sample interpolated
//!   from the source keys (`_interpolate_track`: linear, quaternions `slerp().normalized()`); tracks holding only the
//!   node's rest value were dropped (remove_immutable_tracks) and the node keeps its rest.
//! * playback: `Animation::position_track_interpolate` / `rotation_track_interpolate` with INTERPOLATION_LINEAR on the
//!   baked keys (vectors lerp, quaternions `Quaternion::slerp`), loop wrap on.
//!
//! Units: translations in bbmodel units (the glb's metres x `unit`), so the rig sits directly on the .bbmodel geometry.

use crate::content::embedded;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Godot's `Quaternion::slerp` (core/math/quaternion.cpp): shortest way, plain lerp when nearly equal (no normalize).
pub fn gd_slerp(a: Quat, b: Quat, w: f32) -> Quat {
    let mut cosom = a.dot(b);
    let to1 = if cosom < 0.0 {
        cosom = -cosom;
        -b
    } else {
        b
    };
    let (s0, s1) = if 1.0 - cosom > 0.00001 {
        let omega = cosom.acos();
        let sinom = omega.sin();
        (((1.0 - w) * omega).sin() / sinom, (w * omega).sin() / sinom)
    } else {
        (1.0 - w, w)
    };
    Quat::from_xyzw(
        s0 * a.x + s1 * to1.x,
        s0 * a.y + s1 * to1.y,
        s0 * a.z + s1 * to1.z,
        s0 * a.w + s1 * to1.w,
    )
}

/// Godot `Quaternion.from_euler` (YXZ: yaw, then pitch, then roll) from degrees, as NightWatchPoses.key_quaternion.
pub fn key_quat(deg: Vec3) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        deg.y.to_radians(),
        deg.x.to_radians(),
        deg.z.to_radians(),
    )
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Pos,
    Rot,
}

pub struct Track {
    pub bone: usize,
    pub kind: Kind,
    /// Baked keys (Vec4: xyz for positions, xyzw for rotations), one per `Clip::times`.
    pub keys: Vec<Vec4>,
}

pub struct Clip {
    pub length: f32,
    pub looping: bool,
    pub times: Vec<f32>,
    pub tracks: Vec<Track>,
}

pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub rest_t: Vec3,
    pub rest_r: Quat,
}

pub struct Rig {
    pub unit: f32,
    /// A creature rig (the secret wolf spiders, extract_creature.py): the metres one bbmodel unit is drawn at. Without
    /// it a rig is a humanoid hunter, drawn at `MODEL_SCALE / unit`.
    pub metres: Option<f32>,
    pub bones: Vec<Bone>,
    pub clips: HashMap<String, Clip>,
    /// HunterStride table entries: clip -> (reference speed at authored size, reverse)
    pub stride: HashMap<String, (f32, bool)>,
}

const BAKE_FPS: f64 = 30.0;

/// gltf_document.cpp `_interpolate_track` (LINEAR) on the source keys.
fn source_sample(times: &[f32], vals: &[Vec4], t: f64, kind: Kind) -> Vec4 {
    let mut idx: isize = -1;
    for &k in times {
        if k as f64 > t {
            break;
        }
        idx += 1;
    }
    if idx < 0 {
        return vals[0];
    }
    let i = idx as usize;
    if i >= times.len() - 1 {
        return vals[times.len() - 1];
    }
    let c = ((t - times[i] as f64) / (times[i + 1] as f64 - times[i] as f64)) as f32;
    match kind {
        Kind::Pos => vals[i].lerp(vals[i + 1], c),
        Kind::Rot => {
            let q = gd_slerp(Quat::from_vec4(vals[i]), Quat::from_vec4(vals[i + 1]), c).normalize();
            Vec4::from(q)
        }
    }
}

fn floats(v: &serde_json::Value) -> Vec<f32> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect())
        .unwrap_or_default()
}

fn v3(v: &serde_json::Value) -> Vec3 {
    let f = floats(v);
    vec3(
        *f.first().unwrap_or(&0.0),
        *f.get(1).unwrap_or(&0.0),
        *f.get(2).unwrap_or(&0.0),
    )
}

impl Rig {
    pub fn parse(id: &str, text: &str) -> Option<Rig> {
        let j: serde_json::Value = serde_json::from_str(text).ok()?;
        let unit = j["unit"].as_f64().unwrap_or(65.0) as f32;
        let metres = j["metres"].as_f64().map(|m| m as f32).filter(|m| *m > 0.0);
        let mut bones = vec![];
        for b in j["bones"].as_array()? {
            let r = floats(&b[3]);
            let p = b[1].as_i64().unwrap_or(-1);
            bones.push(Bone {
                name: b[0].as_str().unwrap_or("").to_string(),
                parent: if p >= 0 { Some(p as usize) } else { None },
                rest_t: v3(&b[2]),
                rest_r: if r.len() == 4 {
                    Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize()
                } else {
                    Quat::IDENTITY
                },
            });
        }
        let mut clips = HashMap::new();
        for (name, c) in j["clips"].as_object()? {
            let end = c["len"].as_f64().unwrap_or(0.0);
            // the bake grid: time += 1/fps until it reaches the end, then one last key exactly at the end
            let mut times = vec![];
            let mut t = 0.0f64;
            loop {
                times.push(t);
                if t >= end {
                    break;
                }
                t += 1.0 / BAKE_FPS;
                if t >= end {
                    t = end;
                }
            }
            let mut tracks = vec![];
            for tr in c["tracks"].as_array().into_iter().flatten() {
                let bone = tr[0].as_u64().unwrap_or(0) as usize;
                let kind = if tr[1].as_str() == Some("t") {
                    Kind::Pos
                } else {
                    Kind::Rot
                };
                let st = floats(&tr[2]);
                let raw = floats(&tr[3]);
                let n = if kind == Kind::Pos { 3 } else { 4 };
                if st.is_empty() || raw.len() != st.len() * n || bone >= bones.len() {
                    continue;
                }
                let vals: Vec<Vec4> = raw
                    .chunks(n)
                    .map(|v| {
                        if n == 3 {
                            vec4(v[0], v[1], v[2], 0.0)
                        } else {
                            Vec4::from(Quat::from_xyzw(v[0], v[1], v[2], v[3]).normalize())
                        }
                    })
                    .collect();
                let keys = times
                    .iter()
                    .map(|&t| source_sample(&st, &vals, t, kind))
                    .collect();
                tracks.push(Track { bone, kind, keys });
            }
            clips.insert(
                name.clone(),
                Clip {
                    length: end as f32,
                    looping: c["loop"].as_bool().unwrap_or(false),
                    times: times.iter().map(|t| *t as f32).collect(),
                    tracks,
                },
            );
        }
        let mut stride = HashMap::new();
        if let Some(s) = j["stride"].as_object() {
            for (k, v) in s {
                stride.insert(
                    k.clone(),
                    (
                        v["ref"].as_f64().unwrap_or(1.5) as f32,
                        v["reverse"].as_bool().unwrap_or(false),
                    ),
                );
            }
        }
        let _ = id;
        Some(Rig {
            unit,
            metres,
            bones,
            clips,
            stride,
        })
    }

    /// Metres per bbmodel unit when the rig is drawn: a creature's own, else the humanoid scale (HunterRoster.MODEL_SCALE) over the glb's unit.
    pub fn draw_scale(&self) -> f32 {
        self.metres.unwrap_or(super::hunter::MODEL_SCALE / self.unit)
    }

    /// A creature rig: not the humanoid bone set, so the humanoid layers (head look, chase style, grab arms) find nothing to move.
    pub fn is_creature(&self) -> bool {
        self.metres.is_some()
    }

    pub fn bone(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    pub fn has_clip(&self, name: &str) -> bool {
        self.clips.contains_key(name)
    }

    pub fn length(&self, name: &str) -> f32 {
        self.clips.get(name).map(|c| c.length).unwrap_or(0.0)
    }
}

impl Clip {
    /// The baked key pair around `t` and the weight between them (Animation::_find + linear, loop wrap on).
    fn locate(&self, t: f32) -> (usize, usize, f32) {
        let n = self.times.len();
        if n <= 1 {
            return (0, 0, 0.0);
        }
        // keys are on a uniform 1/30 s grid (the last one may be closer), so index directly and fix up
        let mut i = ((t as f64 * BAKE_FPS).floor() as isize).clamp(0, n as isize - 1) as usize;
        while i > 0 && self.times[i] > t {
            i -= 1;
        }
        while i + 1 < n && self.times[i + 1] <= t {
            i += 1;
        }
        if t < self.times[0] {
            return (0, 0, 0.0);
        }
        if i + 1 >= n {
            if self.looping && self.length > self.times[n - 1] {
                // wrap from the last key to the first over the remaining length
                let span = self.length - self.times[n - 1] + self.times[0];
                return (
                    n - 1,
                    0,
                    ((t - self.times[n - 1]) / span.max(1e-6)).clamp(0.0, 1.0),
                );
            }
            return (n - 1, n - 1, 0.0);
        }
        let span = self.times[i + 1] - self.times[i];
        (
            i,
            i + 1,
            if span > 0.0 {
                (t - self.times[i]) / span
            } else {
                0.0
            },
        )
    }

    /// Samples every track at `t`: `f(track, value)` with rotations as quaternions in xyzw.
    pub fn sample(&self, t: f32, mut f: impl FnMut(&Track, Vec4)) {
        let (a, b, c) = self.locate(t);
        for tr in &self.tracks {
            let v = match tr.kind {
                Kind::Pos => tr.keys[a].lerp(tr.keys[b], c),
                Kind::Rot => Vec4::from(gd_slerp(
                    Quat::from_vec4(tr.keys[a]),
                    Quat::from_vec4(tr.keys[b]),
                    c,
                )),
            };
            f(tr, v);
        }
    }
}

thread_local! {
    static RIGS: RefCell<HashMap<String, Option<Rc<Rig>>>> = RefCell::new(HashMap::new());
}

/// The rig of a hunter model (assets/hunters/anim/<id>.json), loaded once.
pub fn rig(id: &str) -> Option<Rc<Rig>> {
    RIGS.with(|r| {
        r.borrow_mut()
            .entry(id.to_string())
            .or_insert_with(|| {
                let bytes = embedded::asset(&format!("hunters/anim/{id}.json")).or_else(|| embedded::asset(&format!("player/anim/{id}.json")))?;
                Rig::parse(id, std::str::from_utf8(bytes).ok()?).map(Rc::new)
            })
            .clone()
    })
}
