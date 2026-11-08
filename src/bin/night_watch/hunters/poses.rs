//! NightWatchPoses: night_watch_poses.json compiled into clips "nw/<pose>" on a hunter rig. Pure maths on the rig
//! data (no GL, no macroquad drawing).
//!
//! The poses are per-bone Euler (degrees) rotation keys and position offsets, deltas from the rig's rest pose:
//! rotation key = `key_quat(x, y, z) * rest_q(bone)`, position key = `rest_t(bone) + (x, y, z) * (pelvis_rest_y /
//! 0.796875)` (the offsets are authored for a pelvis at 0.796875 m, in bbmodel units here). Looping poses use
//! Catmull-Rom on the Euler triple with wrap (Godot's cubic loop-wrap track), one-shots linear; a key transition
//! `ease` below 1 (flinch: 0.5) is Godot's `ease()`. Clips are baked on the rig's 1/30 s grid with the last sample
//! exactly at `length`, the way `Rig::parse` bakes the imported clips.
#![allow(dead_code)]

use super::anim::clip::{key_quat, Clip, Kind, Rig, Track};
use macroquad::prelude::*;
use std::collections::HashMap;

pub const LIBRARY: &str = "nw";
pub const PELVIS_REST_Y: f32 = 0.796875;
/// The rig's bones the poses drive, parents before children.
pub const BONES: [&str; 16] = [
    "Pelvis", "Belly", "Chest", "Head", "L-Arm", "L-Forearm", "L-Hand", "R-Arm", "R-Forearm", "R-Hand", "L-Thigh",
    "L-Calf", "L-Foot", "R-Thigh", "R-Calf", "R-Foot",
];
/// The blend (seconds) used when a pose names none.
pub const DEFAULT_BLEND: f32 = 0.3;

const BAKE_FPS: f64 = 30.0;

/// "nw/<pose>".
pub fn clip_name(pose: &str) -> String {
    format!("{LIBRARY}/{pose}")
}

/// Godot `ease(x, c)` for a positive curve.
pub fn ease(x: f32, c: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if c <= 0.0 || (c - 1.0).abs() < 1e-6 {
        x
    } else if c < 1.0 {
        1.0 - (1.0 - x).powf(1.0 / c)
    } else {
        x.powf(c)
    }
}

type Keys = Vec<(f32, Vec3)>;

fn read_keys(v: &serde_json::Value) -> Keys {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|k| {
                    let k = k.as_array()?;
                    let g = |i: usize| k.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                    Some((g(0), vec3(g(1), g(2), g(3))))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn catmull(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, c: f32) -> Vec3 {
    let c2 = c * c;
    let c3 = c2 * c;
    0.5 * ((p1 * 2.0) + (p2 - p0) * c + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * c2 + (p1 * 3.0 - p0 - p2 * 3.0 + p3) * c3)
}

/// The Euler triple (degrees) of a rotation track at `t`. Looping tracks are cyclic over `length` (a last key at
/// `length` is the first key again and is dropped from the cycle).
pub fn sample_euler(keys: &[(f32, Vec3)], t: f32, length: f32, looping: bool, ease_c: f32) -> Vec3 {
    let mut n = keys.len();
    if n == 0 {
        return Vec3::ZERO;
    }
    if looping && n > 1 && keys[n - 1].0 >= length - 1e-4 {
        n -= 1;
    }
    if n == 1 {
        return keys[0].1;
    }
    let keys = &keys[..n];
    let first = keys[0].0;
    let mut tt = t;
    if !looping {
        if tt <= first {
            return keys[0].1;
        }
        if tt >= keys[n - 1].0 {
            return keys[n - 1].1;
        }
    } else if tt < first {
        tt += length;
    }
    let mut i = 0;
    for (k, key) in keys.iter().enumerate() {
        if key.0 <= tt {
            i = k;
        } else {
            break;
        }
    }
    let (j, tb) = if i + 1 < n {
        (i + 1, keys[i + 1].0)
    } else {
        (0, keys[0].0 + length)
    };
    let ta = keys[i].0;
    let span = (tb - ta).max(1e-6);
    let c = ease(((tt - ta) / span).clamp(0.0, 1.0), ease_c);
    if looping {
        catmull(keys[(i + n - 1) % n].1, keys[i].1, keys[j].1, keys[(j + 1) % n].1, c)
    } else {
        keys[i].1.lerp(keys[j].1, c)
    }
}

/// A position track at `t`: linear, held before the first and after the last key.
pub fn sample_offset(keys: &[(f32, Vec3)], t: f32) -> Vec3 {
    let n = keys.len();
    if n == 0 {
        return Vec3::ZERO;
    }
    if t <= keys[0].0 {
        return keys[0].1;
    }
    if t >= keys[n - 1].0 {
        return keys[n - 1].1;
    }
    let mut i = 0;
    for (k, key) in keys.iter().enumerate() {
        if key.0 <= t {
            i = k;
        } else {
            break;
        }
    }
    let span = (keys[i + 1].0 - keys[i].0).max(1e-6);
    keys[i].1.lerp(keys[i + 1].1, (t - keys[i].0) / span)
}

fn bake_times(end: f64) -> Vec<f64> {
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
    times
}

/// The pose ids in the document (file order).
pub fn pose_ids(json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|d| d["poses"].as_object().map(|o| o.keys().cloned().collect()))
        .unwrap_or_default()
}

/// Pose id -> its default blend time in seconds (`blend`, 0.3 when absent).
pub fn pose_blends(json: &str) -> HashMap<String, f32> {
    let mut out = HashMap::new();
    if let Some(d) = serde_json::from_str::<serde_json::Value>(json).ok() {
        if let Some(p) = d["poses"].as_object() {
            for (k, v) in p {
                out.insert(k.clone(), v["blend"].as_f64().map(|b| b as f32).unwrap_or(DEFAULT_BLEND));
            }
        }
    }
    out
}

/// Inserts clips "nw/<pose>" into `rig.clips` for every pose in `json`. Safe to call again (clips are replaced).
pub fn compile_nw_poses(rig: &mut Rig, json: &str) {
    compile_nw_poses_for(rig, json, "");
}

/// [`compile_nw_poses`] with the hunter's `hunter_offsets` entry applied (`scale_pos`, `bone_rot`). Returns the
/// number of clips made.
pub fn compile_nw_poses_for(rig: &mut Rig, json: &str, hunter_id: &str) -> usize {
    let Ok(data) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0;
    };
    let Some(poses) = data["poses"].as_object() else {
        return 0;
    };
    let mut bone_idx: HashMap<&'static str, usize> = HashMap::new();
    for b in BONES {
        if let Some(i) = rig.bone(b) {
            bone_idx.insert(b, i);
        }
    }
    // pelvis rest height in bbmodel units; the offsets were authored for 0.796875 m
    let pelvis_units = match bone_idx.get("Pelvis") {
        Some(&i) => rig.bones[i].rest_t.y,
        None => PELVIS_REST_Y * rig.unit,
    };
    let offsets = &data["hunter_offsets"][hunter_id];
    let pos_factor = pelvis_units / PELVIS_REST_Y * offsets["scale_pos"].as_f64().unwrap_or(1.0) as f32;
    let extra_rot = |bone: &str| -> Vec3 {
        let a = &offsets["bone_rot"][bone];
        let g = |i: usize| a[i].as_f64().unwrap_or(0.0) as f32;
        if a.is_array() {
            vec3(g(0), g(1), g(2))
        } else {
            Vec3::ZERO
        }
    };
    let mut made: Vec<(String, Clip)> = vec![];
    for (pose_id, spec) in poses {
        let length = spec["length"].as_f64().unwrap_or(0.0).max(0.0);
        let looping = spec["loop"].as_bool().unwrap_or(true);
        let ease_c = spec["ease"].as_f64().unwrap_or(1.0) as f32;
        let times = bake_times(length);
        let mut tracks: Vec<Track> = vec![];
        if let Some(per_bone) = spec["tracks"].as_object() {
            for (bone, tr) in per_bone {
                let Some((&name, &bi)) = bone_idx.get_key_value(bone.as_str()) else {
                    continue;
                };
                let rest = &rig.bones[bi];
                let rot = read_keys(&tr["rot"]);
                if !rot.is_empty() {
                    let add = extra_rot(name);
                    let mut keys: Vec<Vec4> = Vec::with_capacity(times.len());
                    let mut prev: Option<Quat> = None;
                    for &t in &times {
                        let e = sample_euler(&rot, t as f32, length as f32, looping, ease_c) + add;
                        let mut q = (key_quat(e) * rest.rest_r).normalize();
                        if let Some(p) = prev {
                            if p.dot(q) < 0.0 {
                                q = -q;
                            }
                        }
                        prev = Some(q);
                        keys.push(Vec4::from(q));
                    }
                    tracks.push(Track { bone: bi, kind: Kind::Rot, keys });
                }
                let pos = read_keys(&tr["pos"]);
                if !pos.is_empty() {
                    let keys: Vec<Vec4> = times
                        .iter()
                        .map(|&t| {
                            let p = rest.rest_t + sample_offset(&pos, t as f32) * pos_factor;
                            vec4(p.x, p.y, p.z, 0.0)
                        })
                        .collect();
                    tracks.push(Track { bone: bi, kind: Kind::Pos, keys });
                }
            }
        }
        made.push((
            clip_name(pose_id),
            Clip {
                length: length as f32,
                looping,
                times: times.iter().map(|t| *t as f32).collect(),
                tracks,
            },
        ));
    }
    let n = made.len();
    for (name, clip) in made {
        rig.clips.insert(name, clip);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::anim::player::AnimPlayer;

    const DOVELLE: &str = include_str!("../../last_route/assets/hunters/anim/dovelle.json");
    const VERRINE: &str = include_str!("../../last_route/assets/hunters/anim/verrine.json");

    fn rig_of(id: &str, text: &str) -> Rig {
        let mut rig = Rig::parse(id, text).expect("rig parses");
        compile_nw_poses(&mut rig, crate::assets::POSES_JSON);
        rig
    }

    #[test]
    fn all_26_poses_compile_with_finite_values() {
        let rig = rig_of("dovelle", DOVELLE);
        let nw: Vec<_> = rig.clips.keys().filter(|k| k.starts_with("nw/")).collect();
        assert_eq!(nw.len(), 26);
        assert_eq!(pose_ids(crate::assets::POSES_JSON).len(), 26);
        let mut bones_seen = std::collections::HashSet::new();
        for name in &nw {
            let clip = &rig.clips[*name];
            assert!(clip.length > 0.0, "{name}");
            assert!(!clip.tracks.is_empty(), "{name}");
            for tr in &clip.tracks {
                bones_seen.insert(tr.bone);
                assert_eq!(tr.keys.len(), clip.times.len(), "{name}");
                for k in &tr.keys {
                    assert!(k.to_array().iter().all(|v| v.is_finite()), "{name}");
                    if tr.kind == Kind::Rot {
                        assert!((k.length() - 1.0).abs() < 1e-3, "{name} quaternion not unit");
                    }
                }
            }
        }
        assert_eq!(bones_seen.len(), 16, "every driven bone has a track somewhere");
    }

    #[test]
    fn the_last_baked_sample_sits_exactly_at_the_clip_length() {
        let rig = rig_of("dovelle", DOVELLE);
        for (name, clip) in rig.clips.iter().filter(|(k, _)| k.starts_with("nw/")) {
            assert_eq!(*clip.times.last().unwrap(), clip.length, "{name}");
            assert_eq!(clip.times[0], 0.0);
        }
    }

    #[test]
    fn loop_poses_have_equal_first_and_last_key_and_flinch_is_a_one_shot() {
        let rig = rig_of("dovelle", DOVELLE);
        let doc: serde_json::Value = serde_json::from_str(crate::assets::POSES_JSON).unwrap();
        for (id, spec) in doc["poses"].as_object().unwrap() {
            let clip = &rig.clips[&clip_name(id)];
            let looping = spec["loop"].as_bool().unwrap_or(true);
            assert_eq!(clip.looping, looping, "{id}");
            if !looping {
                continue;
            }
            for tr in &clip.tracks {
                let (a, b) = (tr.keys[0], *tr.keys.last().unwrap());
                match tr.kind {
                    Kind::Rot => assert!(a.dot(b).abs() > 0.9999, "{id} rot first != last"),
                    Kind::Pos => assert!((a - b).length() < 1e-3, "{id} pos first != last"),
                }
            }
        }
        assert!(!rig.clips["nw/flinch"].looping);
        assert!(rig.clips["nw/door_lurk"].looping);
    }

    #[test]
    fn position_offsets_scale_with_the_pelvis_height() {
        let rig = rig_of("dovelle", DOVELLE);
        let pelvis = rig.bone("Pelvis").unwrap();
        let rest = rig.bones[pelvis].rest_t;
        let clip = &rig.clips["nw/sprint_lunge"];
        let tr = clip.tracks.iter().find(|t| t.bone == pelvis && t.kind == Kind::Pos).expect("pelvis pos track");
        // first key of the pose is (0, -0.12, -0.12)
        let f = rest.y / PELVIS_REST_Y;
        assert!((tr.keys[0].y - (rest.y - 0.12 * f)).abs() < 1e-3);
        assert!((tr.keys[0].z - (rest.z - 0.12 * f)).abs() < 1e-3);
    }

    #[test]
    fn ease_half_is_the_flinch_curve() {
        assert!((ease(0.5, 0.5) - 0.75).abs() < 1e-6);
        assert_eq!(ease(0.0, 0.5), 0.0);
        assert_eq!(ease(1.0, 0.5), 1.0);
        assert_eq!(ease(0.3, 1.0), 0.3);
        let keys = vec![(0.0, vec3(0.0, 0.0, 0.0)), (1.0, vec3(10.0, 0.0, 0.0))];
        let mid = sample_euler(&keys, 0.5, 1.0, false, 0.5);
        assert!((mid.x - 7.5).abs() < 1e-4);
    }

    #[test]
    fn loop_sampling_wraps_and_passes_through_keys() {
        let keys = vec![
            (0.0, vec3(0.0, 0.0, 0.0)),
            (1.0, vec3(10.0, 0.0, 0.0)),
            (2.0, vec3(0.0, 0.0, 0.0)),
            (3.0, vec3(-10.0, 0.0, 0.0)),
            (4.0, vec3(0.0, 0.0, 0.0)),
        ];
        assert!((sample_euler(&keys, 1.0, 4.0, true, 1.0).x - 10.0).abs() < 1e-4);
        assert!((sample_euler(&keys, 3.0, 4.0, true, 1.0).x + 10.0).abs() < 1e-4);
        assert!(sample_euler(&keys, 0.0, 4.0, true, 1.0).x.abs() < 1e-4);
        assert!(sample_euler(&keys, 4.0, 4.0, true, 1.0).x.abs() < 1e-3);
        // smooth across the wrap: values just before and after the loop point are close
        let a = sample_euler(&keys, 3.99, 4.0, true, 1.0).x;
        let b = sample_euler(&keys, 0.01, 4.0, true, 1.0).x;
        assert!((a - b).abs() < 1.0);
    }

    #[test]
    fn a_compiled_pose_plays_and_moves_the_bones() {
        let rig = rig_of("verrine", VERRINE);
        let mut p = AnimPlayer::default();
        p.play(&rig, "nw/door_lurk", 0.0);
        p.seek(0.8);
        let pose = p.pose(&rig);
        assert_eq!(pose.len(), rig.bones.len());
        let mut moved = false;
        for (b, l) in rig.bones.iter().zip(&pose) {
            assert!(l.r.to_array().iter().all(|v| v.is_finite()));
            assert!(l.t.to_array().iter().all(|v| v.is_finite()));
            if l.r.angle_between(b.rest_r) > 1e-3 {
                moved = true;
            }
        }
        assert!(moved);
    }

    #[test]
    fn verrine_vent_poses_exist_on_her_rig() {
        let rig = rig_of("verrine", VERRINE);
        for p in ["vent_crawl_verrine", "vent_drop_verrine", "vent_rise_verrine", "vent_crawl"] {
            assert!(rig.has_clip(&clip_name(p)), "{p}");
        }
    }

    #[test]
    fn blends_default_to_point_three() {
        let b = pose_blends(crate::assets::POSES_JSON);
        assert!((b["door_lurk"] - 0.45).abs() < 1e-6);
        assert!((b["flinch"] - 0.08).abs() < 1e-6);
        assert_eq!(b.len(), 26);
    }
}
