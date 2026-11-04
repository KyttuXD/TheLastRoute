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
}

impl Default for CacheTuning {
    fn default() -> Self {
        CacheTuning { pose_step: 0.002, position_step: 0.004, tint_step: 0.01, max_entries: 64 }
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
            let mut vertices = vec![];
            let mut indices = vec![];
            build(&mut vertices, &mut indices);
            self.rebuilds += 1;
            self.entries.insert(slot, Entry { model: model.to_string(), pose, tint, mesh: Mesh { vertices, indices, texture } });
        }
        &self.entries[&slot].mesh
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
}
