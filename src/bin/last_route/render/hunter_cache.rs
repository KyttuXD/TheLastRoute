//! Hunter mesh cache: a hunter's posed mesh is rebuilt only when its model, pose or tint changes
//! (quantised by the steps in data/render/hunters.json), instead of into fresh Vecs every frame.
//! Hooked from render/scene.rs, hunters loop.

use crate::content::embedded;
use macroquad::prelude::*;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const TABLE_PATH: &str = "render/hunters.json";

#[derive(Clone, Debug, PartialEq)]
pub struct CacheTuning {
    /// Quantisation of rotation/scale matrix elements and bone matrices.
    pub pose_step: f32,
    /// Quantisation of translation components.
    pub position_step: f32,
    /// Quantisation of tint channels.
    pub tint_step: f32,
    /// Entries kept before the cache is cleared.
    pub max_entries: usize,
    /// Animation LOD bands as (distance in metres, frames between pose updates), nearest first. A hunter nearer
    /// than the first band updates every frame; beyond the last band it uses the last interval.
    pub lod_bands: Vec<(f32, u32)>,
    /// Beyond this distance (the fog end) a hunter's pose is not rebuilt at all; 0 turns it off.
    pub skip_beyond: f32,
}

impl Default for CacheTuning {
    fn default() -> Self {
        CacheTuning {
            pose_step: 0.002,
            position_step: 0.004,
            tint_step: 0.01,
            max_entries: 64,
            lod_bands: vec![(12.0, 1), (24.0, 2), (40.0, 4)],
            skip_beyond: 0.0,
        }
    }
}

impl CacheTuning {
    pub fn parse(text: &str) -> Result<CacheTuning, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let o = v.as_object().ok_or("expected a JSON object")?;
        let mut t = CacheTuning::default();
        for (k, val) in o {
            match k.as_str() {
                "pose_step" | "position_step" | "tint_step" => {
                    let n = val
                        .as_f64()
                        .filter(|n| *n > 0.0 && n.is_finite())
                        .ok_or_else(|| format!("{k} must be a positive number"))? as f32;
                    match k.as_str() {
                        "pose_step" => t.pose_step = n,
                        "position_step" => t.position_step = n,
                        _ => t.tint_step = n,
                    }
                }
                "max_entries" => {
                    t.max_entries = val.as_u64().filter(|n| *n > 0).ok_or("max_entries must be a positive integer")? as usize;
                }
                "skip_beyond" => {
                    t.skip_beyond = val.as_f64().filter(|n| *n >= 0.0 && n.is_finite()).ok_or("skip_beyond must be >= 0")? as f32;
                }
                "lod_bands" => {
                    let arr = val.as_array().ok_or("lod_bands must be an array")?;
                    let mut bands = vec![];
                    for b in arr {
                        let d = b[0].as_f64().filter(|d| *d >= 0.0 && d.is_finite()).ok_or("lod_bands: bad distance")?;
                        let n = b[1].as_u64().filter(|n| *n >= 1).ok_or("lod_bands: interval must be >= 1")?;
                        bands.push((d as f32, n as u32));
                    }
                    bands.sort_by(|a, b| a.0.total_cmp(&b.0));
                    t.lod_bands = bands;
                }
                _ => {}
            }
        }
        Ok(t)
    }
}

/// The tunables from data/render/hunters.json.
pub fn tuning() -> &'static CacheTuning {
    static T: OnceLock<CacheTuning> = OnceLock::new();
    T.get_or_init(|| {
        let text = embedded::data_under(TABLE_PATH).next().map(|(_, t)| t).unwrap_or("{}");
        CacheTuning::parse(text).unwrap_or_else(|e| {
            eprintln!("{TABLE_PATH}: {e}");
            CacheTuning::default()
        })
    })
}

/// Frames between pose updates for a hunter `dist` metres from the camera: 1 (every frame) for near hunters.
pub fn pose_interval(tuning: &CacheTuning, dist: f32) -> u32 {
    let mut interval = 1;
    for (d, n) in &tuning.lod_bands {
        if dist >= *d {
            interval = *n;
        }
    }
    interval
}

/// True when a hunter beyond the skip distance should not have its pose rebuilt (it is in the fog).
pub fn beyond_fog(tuning: &CacheTuning, dist: f32) -> bool {
    tuning.skip_beyond > 0.0 && dist > tuning.skip_beyond
}

/// Whether the hunter in `slot_index` is due a pose update on `frame`; slots are staggered so far hunters
/// do not all rebuild on the same frame.
pub fn pose_due(tuning: &CacheTuning, dist: f32, frame: u64, slot_index: usize) -> bool {
    let n = pose_interval(tuning, dist) as u64;
    n <= 1 || (frame + slot_index as u64) % n == 0
}

fn q(v: f32, step: f32) -> i32 {
    (v / step).round() as i32
}

/// Pose key: the root matrix and bone matrices quantised (translation with the position step).
pub fn pose_key(tuning: &CacheTuning, root: &Mat4, bones: &[Mat4], face_cell: i32) -> Vec<i32> {
    let mut key = Vec::with_capacity(16 * (1 + bones.len()) + 1);
    key.push(face_cell);
    for m in std::iter::once(root).chain(bones.iter()) {
        let a = m.to_cols_array();
        for (i, v) in a.iter().enumerate() {
            let step = if (12..15).contains(&i) { tuning.position_step } else { tuning.pose_step };
            key.push(q(*v, step));
        }
    }
    key
}

pub fn tint_key(tuning: &CacheTuning, tint: Color) -> [i32; 4] {
    [q(tint.r, tuning.tint_step), q(tint.g, tuning.tint_step), q(tint.b, tuning.tint_step), q(tint.a, tuning.tint_step)]
}

/// A cache slot: the absolute level of the list a hunter lives in, and its index there. A bare index means level 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotKey {
    pub level: i32,
    pub index: usize,
}

impl From<usize> for SlotKey {
    fn from(index: usize) -> SlotKey {
        SlotKey { level: 0, index }
    }
}

impl From<(i32, usize)> for SlotKey {
    fn from((level, index): (i32, usize)) -> SlotKey {
        SlotKey { level, index }
    }
}

struct Entry {
    model: String,
    pose: Vec<i32>,
    tint: [i32; 4],
    mesh: Mesh,
}

#[derive(Default)]
pub struct HunterMeshCache {
    entries: HashMap<SlotKey, Entry>,
    /// The run the entries were made for.
    run: u32,
    pub rebuilds: u64,
    pub hits: u64,
}

impl HunterMeshCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The hunter's mesh for `slot`; `build` runs only when model, pose or tint differ from the cached one.
    #[allow(clippy::too_many_arguments)]
    pub fn get_or_build(
        &mut self,
        tuning: &CacheTuning,
        slot: impl Into<SlotKey>,
        model: &str,
        pose: Vec<i32>,
        tint: [i32; 4],
        texture: Option<Texture2D>,
        build: impl FnOnce(&mut Vec<Vertex>, &mut Vec<u16>),
    ) -> &Mesh {
        let slot: SlotKey = slot.into();
        let fresh = self
            .entries
            .get(&slot)
            .map(|e| e.model == model && e.tint == tint && e.pose == pose)
            .unwrap_or(false);
        if fresh {
            self.hits += 1;
        } else {
            if !self.entries.contains_key(&slot) && self.entries.len() >= tuning.max_entries {
                self.entries.clear();
            }
            // reuse the slot's old buffers so a rebuild does not allocate
            let (mut vertices, mut indices) = match self.entries.remove(&slot) {
                Some(e) => (e.mesh.vertices, e.mesh.indices),
                None => (vec![], vec![]),
            };
            vertices.clear();
            indices.clear();
            build(&mut vertices, &mut indices);
            self.rebuilds += 1;
            self.entries.insert(slot, Entry { model: model.to_string(), pose, tint, mesh: Mesh { vertices, indices, texture } });
        }
        &self.entries[&slot].mesh
    }

    /// The last mesh built for `slot` if it was for `model`, whatever its pose: what a far hunter shows between
    /// its LOD updates (the caller skips the pose build when `pose_due` is false or `beyond_fog` is true).
    pub fn stale(&mut self, slot: impl Into<SlotKey>, model: &str) -> Option<&Mesh> {
        let e = self.entries.get(&slot.into()).filter(|e| e.model == model)?;
        self.hits += 1;
        Some(&e.mesh)
    }

    /// Keeps the slots of every list across a relabel or a hand-over: `lists` is (absolute level, hunter count) for
    /// each list. Entries of a vanished slot are dropped, and a new run clears all. True when anything was dropped.
    pub fn sync(&mut self, lists: &[(i32, usize)], run: u32) -> bool {
        if self.run != run {
            let had = !self.entries.is_empty();
            self.entries.clear();
            self.run = run;
            return had;
        }
        let before = self.entries.len();
        self.entries.retain(|k, _| lists.iter().any(|(l, n)| *l == k.level && k.index < *n));
        self.entries.len() != before
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Names a roster hunter drawn without a rig (the stiff fallback block) once on stderr, so a missing
/// assets/hunters/anim/<id>.json is noticed. Hooked from render/scene.rs where `anim::clip::rig(id)` is `None`.
pub fn warn_unrigged_once(id: &str) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(mut s) = seen.lock() {
        if s.insert(id.to_string()) {
            eprintln!("hunter {id}: no rig (assets/hunters/anim/{id}.json), drawing the stiff fallback");
        }
    }
}

/// A far hunter's last posed mesh, moved by `by` (x, z) to where the hunter stands now. Between LOD updates the pose
/// stays frozen while the position follows the simulation, so the copy is drawn at the live spot. `shift` is a scratch
/// mesh whose buffers are reused, so the copy does not allocate once it is warm.
pub fn shift_stale(src: &Mesh, shift: &mut Mesh, by: (f32, f32)) {
    shift.vertices.clear();
    shift.vertices.extend(src.vertices.iter().map(|v| {
        let mut v = *v;
        v.position.x += by.0;
        v.position.z += by.1;
        v
    }));
    shift.indices.clear();
    shift.indices.extend_from_slice(&src.indices);
    shift.texture = src.texture.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut HunterMeshCache, t: &CacheTuning, slot: usize, model: &str, root: Mat4, tint: Color, builds: &mut u32) {
        let pose = pose_key(t, &root, &[], 0);
        c.get_or_build(t, slot, model, pose, tint_key(t, tint), None, |_, i| {
            *builds += 1;
            i.push(0);
        });
    }

    /// A far hunter's pose is frozen (no rebuild) while its position moves every frame: the drawn copy follows it.
    #[test]
    fn frozen_pose_still_walks() {
        let t = CacheTuning::default();
        let mut c = HunterMeshCache::new();
        let pose = pose_key(&t, &Mat4::IDENTITY, &[], 0);
        c.get_or_build(&t, 0usize, "a", pose, tint_key(&t, WHITE), None, |v, i| {
            v.push(Vertex::new(1.0, 0.0, 2.0, 0.0, 0.0, WHITE));
            i.push(0);
        });
        let mut shift = Mesh { vertices: Vec::new(), indices: Vec::new(), texture: None };
        for step in 1..=3 {
            // the hunter has walked (0.5 m, -0.25 m) per step on x and z since it was posed
            let by = (0.5 * step as f32, -0.25 * step as f32);
            let src = c.stale(0usize, "a").expect("a far hunter keeps its last mesh");
            shift_stale(src, &mut shift, by);
            assert_eq!(src.vertices[0].position, vec3(1.0, 0.0, 2.0), "the stored pose is untouched");
            assert_eq!(shift.vertices[0].position, vec3(1.0 + by.0, 0.0, 2.0 + by.1), "the drawn copy is at the live spot");
        }
        assert_eq!(c.rebuilds, 1, "the pose was never rebuilt while it was frozen");
    }

    #[test]
    fn standard_data_parses() {
        let t = tuning();
        assert!(t.pose_step > 0.0 && t.position_step > 0.0 && t.tint_step > 0.0 && t.max_entries >= 1);
    }

    #[test]
    fn unchanged_pose_does_not_rebuild() {
        let t = CacheTuning::default();
        let mut c = HunterMeshCache::new();
        let mut n = 0;
        for _ in 0..5 {
            run(&mut c, &t, 0, "dovelle", Mat4::IDENTITY, WHITE, &mut n);
        }
        assert_eq!(n, 1);
        assert_eq!(c.hits, 4);
    }

    #[test]
    fn change_below_step_is_a_hit_above_is_a_rebuild() {
        let t = CacheTuning::default();
        let mut c = HunterMeshCache::new();
        let mut n = 0;
        run(&mut c, &t, 0, "a", Mat4::from_translation(vec3(1.0, 0.0, 0.0)), WHITE, &mut n);
        run(&mut c, &t, 0, "a", Mat4::from_translation(vec3(1.0 + t.position_step * 0.1, 0.0, 0.0)), WHITE, &mut n);
        assert_eq!(n, 1);
        run(&mut c, &t, 0, "a", Mat4::from_translation(vec3(1.0 + t.position_step * 5.0, 0.0, 0.0)), WHITE, &mut n);
        assert_eq!(n, 2);
    }

    #[test]
    fn tint_or_model_change_rebuilds() {
        let t = CacheTuning::default();
        let mut c = HunterMeshCache::new();
        let mut n = 0;
        let stun = Color::new(0.7, 0.8, 1.0, 1.0);
        run(&mut c, &t, 1, "a", Mat4::IDENTITY, WHITE, &mut n);
        run(&mut c, &t, 1, "a", Mat4::IDENTITY, stun, &mut n);
        run(&mut c, &t, 1, "b", Mat4::IDENTITY, stun, &mut n);
        assert_eq!(n, 3);
    }

    #[test]
    fn slots_are_independent_and_bounded() {
        let t = CacheTuning { max_entries: 2, ..CacheTuning::default() };
        let mut c = HunterMeshCache::new();
        let mut n = 0;
        run(&mut c, &t, 0, "a", Mat4::IDENTITY, WHITE, &mut n);
        run(&mut c, &t, 1, "a", Mat4::IDENTITY, WHITE, &mut n);
        run(&mut c, &t, 0, "a", Mat4::IDENTITY, WHITE, &mut n);
        assert_eq!(n, 2);
        run(&mut c, &t, 2, "a", Mat4::IDENTITY, WHITE, &mut n);
        assert!(c.len() <= 2);
    }

    #[test]
    fn hunter_cache_keeps_slots_across_a_relabel() {
        let t = CacheTuning::default();
        let mut c = HunterMeshCache::new();
        let mut n = 0;
        for l in [0, 1] {
            let pose = pose_key(&t, &Mat4::IDENTITY, &[], 0);
            c.get_or_build(&t, (l, 0usize), "a", pose, tint_key(&t, WHITE), None, |_, ix| {
                n += 1;
                ix.push(0);
            });
        }
        assert_eq!(n, 2);
        // levels are absolute, so a relabel leaves the lists as they were: nothing dropped, nothing re-posed
        assert!(!c.sync(&[(0, 1), (1, 1)], 0));
        let pose = pose_key(&t, &Mat4::IDENTITY, &[], 0);
        c.get_or_build(&t, (1, 0usize), "a", pose, tint_key(&t, WHITE), None, |_, _| n += 1);
        assert_eq!(n, 2);
        // a vanished list drops only its slots; a new run drops all
        assert!(c.sync(&[(0, 1)], 0));
        assert_eq!(c.len(), 1);
        assert!(c.sync(&[(0, 1)], 7));
        assert!(c.is_empty());
    }

    #[test]
    fn lod_rate_follows_distance() {
        let t = CacheTuning::default();
        assert_eq!(pose_interval(&t, 0.0), 1);
        let mut last = 1;
        for d in [0.0, 5.0, 15.0, 30.0, 60.0, 500.0] {
            let n = pose_interval(&t, d);
            assert!(n >= last, "interval must not fall with distance");
            last = n;
        }
        assert!(last > 1);
        assert!((0..8).all(|f| pose_due(&t, 1.0, f, 3)));
        let due = (0..8).filter(|f| pose_due(&t, 100.0, *f, 3)).count();
        assert!(due >= 1 && due < 8);
        let slow = CacheTuning::parse(r#"{"lod_bands": [[0, 10]]}"#).unwrap();
        assert_eq!(pose_interval(&slow, 1.0), 10);
    }

    #[test]
    fn fog_skip_is_data_driven() {
        let mut t = CacheTuning::default();
        assert!(!beyond_fog(&t, 1000.0));
        t.skip_beyond = 50.0;
        assert!(beyond_fog(&t, 60.0) && !beyond_fog(&t, 40.0));
        assert!(CacheTuning::parse(r#"{"lod_bands": [[5, 0]]}"#).is_err());
    }

    #[test]
    fn cached_pose_equals_fresh_and_buffers_are_reused() {
        let t = CacheTuning::default();
        let mut c = HunterMeshCache::new();
        let key = |c: &mut HunterMeshCache, x: f32| {
            let pose = pose_key(&t, &Mat4::from_translation(vec3(x, 0.0, 0.0)), &[], 0);
            let m = c.get_or_build(&t, 0usize, "a", pose, tint_key(&t, WHITE), None, |v, i| {
                for k in 0..4u16 {
                    v.push(Vertex::new(k as f32, 0.0, 0.0, 0.0, 0.0, WHITE));
                    i.push(k);
                }
            });
            (m.vertices.len(), m.indices.clone())
        };
        let first = key(&mut c, 0.0);
        let cached = key(&mut c, 0.0);
        let rebuilt = key(&mut c, 5.0);
        assert_eq!(first, cached);
        assert_eq!(first, rebuilt, "a recycled buffer holds exactly one build");
        assert_eq!(c.stale(0usize, "a").unwrap().vertices.len(), 4);
        assert!(c.stale(0usize, "b").is_none() && c.stale(9usize, "a").is_none());
    }

    #[test]
    fn data_step_changes_behaviour() {
        let coarse = CacheTuning::parse(r#"{"position_step": 1.0}"#).unwrap();
        let mut c = HunterMeshCache::new();
        let mut n = 0;
        run(&mut c, &coarse, 0, "a", Mat4::from_translation(vec3(0.1, 0.0, 0.0)), WHITE, &mut n);
        run(&mut c, &coarse, 0, "a", Mat4::from_translation(vec3(0.2, 0.0, 0.0)), WHITE, &mut n);
        assert_eq!(n, 1);
    }

    #[test]
    fn rejects_bad_tables() {
        assert!(CacheTuning::parse(r#"{"pose_step": 0}"#).is_err());
        assert!(CacheTuning::parse(r#"{"max_entries": 0}"#).is_err());
        assert!(CacheTuning::parse("[]").is_err());
    }

    /// Every roster hunter id (data/hunters/*.json with the hunter schema).
    fn roster_ids() -> Vec<String> {
        embedded::data_under("hunters/")
            .filter_map(|(_, t)| serde_json::from_str::<serde_json::Value>(t).ok())
            .filter(|v| v["schema"].as_str().is_some_and(|s| s.starts_with("lr.hunter/")))
            .filter_map(|v| v["id"].as_str().map(str::to_string))
            .collect()
    }

    const RIGGED_LATE: [&str; 14] = [
        "nyxie", "solyth", "void_black", "void_blue", "void_brown", "void_cyan", "void_gray", "void_green",
        "void_orange", "void_pink", "void_purple", "void_red", "void_white", "void_yellow",
    ];

    fn rig_json(id: &str) -> serde_json::Value {
        let bytes = embedded::asset(&format!("hunters/anim/{id}.json")).unwrap_or_else(|| panic!("{id}: no rig file"));
        serde_json::from_slice(bytes).unwrap()
    }

    #[test]
    fn every_roster_hunter_with_a_model_has_a_rig() {
        let ids = roster_ids();
        // data/hunters holds 38 roster files; every late-rigged hunter must be among them
        assert!(ids.len() >= 38, "roster looks empty: {}", ids.len());
        for late in RIGGED_LATE {
            assert!(ids.iter().any(|i| i == late), "{late} is missing from the roster");
        }
        for id in ids {
            if embedded::asset(&format!("hunters/{id}.bbmodel")).is_some() {
                assert!(embedded::asset(&format!("hunters/anim/{id}.json")).is_some(), "{id} has a model but no rig (stiff fallback)");
            }
        }
    }

    #[test]
    fn late_rigs_have_idle_walk_and_chase() {
        for id in RIGGED_LATE {
            let j = rig_json(id);
            for clip in ["idle", "walk", "chase"] {
                let c = &j["clips"][clip];
                assert!(c.is_object(), "{id}: no {clip} clip");
                assert!(!c["tracks"].as_array().unwrap().is_empty(), "{id}: {clip} has no tracks");
            }
            assert!(j["clips"]["idle"]["loop"].as_bool().unwrap() && j["clips"]["walk"]["loop"].as_bool().unwrap(), "{id}");
            assert!(j["stride"]["walk"]["ref"].as_f64().unwrap() > 0.0, "{id}: no walk stride");
            assert!(j["clips"].as_object().unwrap().keys().all(|k| !k.starts_with("emote_")), "{id}: emotes are a later pass");
        }
    }

    #[test]
    fn late_rig_bones_exist_in_the_model() {
        // the rig drives the model's outliner groups by name, so Head and a leg must be among them
        for id in RIGGED_LATE {
            let j = rig_json(id);
            let names: Vec<&str> = j["bones"].as_array().unwrap().iter().map(|b| b[0].as_str().unwrap()).collect();
            for n in ["Pelvis", "Head", "L-Thigh", "R-Thigh"] {
                assert!(names.contains(&n), "{id}: no bone {n}");
            }
            for t in j["clips"].as_object().unwrap().values() {
                for tr in t["tracks"].as_array().unwrap() {
                    assert!((tr[0].as_u64().unwrap() as usize) < names.len(), "{id}: track on a missing bone");
                }
            }
        }
    }

    /// Model top in bbmodel units: the highest cube face.
    fn model_top(id: &str) -> f32 {
        let j: serde_json::Value = serde_json::from_slice(embedded::asset(&format!("hunters/{id}.bbmodel")).unwrap()).unwrap();
        j["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["visibility"].as_bool() != Some(false))
            .filter_map(|e| e["to"][1].as_f64())
            .fold(0.0, f64::max) as f32
    }

    #[test]
    fn rigged_hunters_are_drawn_at_model_scale() {
        // the drawn height is top / unit x MODEL_SCALE (hunter_roster.gd 185: every humanoid takes 1.5), and
        // the void models are authored against unit 65 like the others
        for id in RIGGED_LATE {
            let unit = rig_json(id)["unit"].as_f64().unwrap() as f32;
            assert!((unit - 65.0).abs() < 0.01, "{id}: unit {unit}");
            let metres = model_top(id) / unit * crate::render::anim::hunter::MODEL_SCALE;
            let want = if id.starts_with("void_") { model_top("dovelle") / 65.0 * 1.5 } else { metres };
            assert!((metres - want).abs() < 0.02, "{id}: {metres} m");
        }
        // Nyxie and Solyth reach the audit's heights
        assert!((model_top("nyxie") / 65.0 * 1.5 - 3.14).abs() < 0.1);
        assert!((model_top("solyth") / 65.0 * 1.5 - 3.19).abs() < 0.1);
    }

    #[test]
    fn extract_is_deterministic() {
        // needs the original's project and Python: set LR_GODOT_DIR to run it, otherwise it is skipped
        let Ok(godot) = std::env::var("LR_GODOT_DIR") else { return };
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/src/bin/last_route/render/anim/extract.py");
        let run = |tag: &str| -> Vec<u8> {
            let app = std::env::temp_dir().join(format!("lr_extract_{}_{tag}", crate::shared::task::process_id()));
            let hunters = app.join("assets").join("hunters");
            std::fs::create_dir_all(&hunters).unwrap();
            std::fs::copy(
                concat!(env!("CARGO_MANIFEST_DIR"), "/src/bin/last_route/assets/hunters/nyxie.bbmodel"),
                hunters.join("nyxie.bbmodel"),
            )
            .unwrap();
            let ok = std::process::Command::new("python").args(["-I", script]).arg(&godot).arg(&app).status().map(|s| s.success());
            assert_eq!(ok.ok(), Some(true), "extract.py failed");
            let bytes = std::fs::read(hunters.join("anim").join("nyxie.json")).unwrap();
            let _ = std::fs::remove_dir_all(&app);
            bytes
        };
        assert_eq!(run("a"), run("b"));
    }

    #[test]
    fn warning_is_safe_to_repeat() {
        warn_unrigged_once("test_hunter");
        warn_unrigged_once("test_hunter");
    }
}
