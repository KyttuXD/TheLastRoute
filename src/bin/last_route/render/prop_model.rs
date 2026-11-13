//! Prop, fixture and hand-tool models, read from Blockbench `.bbmodel` files (props/<set>/*.bbmodel and
//! interactables/*.bbmodel; the converter that made them from the original's Blender exports is
//! tools/props/glb_to_bbmodel.mjs). A model has one texture, so it is one part. A room door's hinge leaves and a hiding
//! spot's door leaf are outliner groups (`DoorHinge`, `DoorHingeRight`, `LockerDoor`), kept apart from the body so they
//! can swing; the door frame's pieces (`DoorFrameTop`, `DoorFrameL`, `DoorFrameR`) are groups too, so door.rs can
//! measure them. Mod packs may still bring a `.glb` prop; `PropModel::parse_glb` is the one place that reads one.

use super::bbmodel::{self, Face, Model};
use super::builder::Builder;
use super::door::{self, DoorMeasure};
use macroquad::prelude::*;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_KEY: AtomicUsize = AtomicUsize::new(1);

pub struct Part {
    /// Model-space vertices; `normal.w` is the shader mode (1 textured).
    pub verts: Vec<Vertex>,
    pub idx: Vec<u32>,
    pub tex: Texture2D,
    /// Identifies the texture for batching.
    pub key: usize,
}

pub struct PropModel {
    pub parts: Vec<Part>,
    pub min: Vec3,
    pub max: Vec3,
    /// The door measures, for a model with door hinges (door.rs); `None` for every other model.
    pub door: Option<door::DoorDims>,
    /// A room door's hinge leaves, kept out of `parts` (the static batch bakes the door shut) so each can be turned
    /// by the door's open amount (`draw_hinges`).
    pub hinges: Vec<Hinge>,
    /// A hiding spot's door leaf, kept out of `parts` so it can swing (`draw_locker_door`); `None` for any other model.
    pub locker_door: Option<LockerDoor>,
}

/// One door hinge node: its leaf geometry is in the hinge's own frame, with the authored (open) rotation.
pub struct Hinge {
    pub name: String,
    /// The hinge's parent transform in model space, and the hinge's authored local transform.
    pub base: Mat4,
    pub local: Mat4,
    /// The closed leaf's span along the model's X (low, high), before any fit: what `door::leaf_fit` stretches.
    pub closed_x: (f32, f32),
    pub parts: Vec<Part>,
}

/// The door of a hiding spot: its geometry is relative to `pivot`, the hinge line on the model's left edge.
pub struct LockerDoor {
    pub pivot: Vec3,
    pub parts: Vec<Part>,
}

/// The outliner group of a hiding spot model that swings with its door (the hinge blocks stay on the body).
pub const LOCKER_DOOR_GROUP: &str = "LockerDoor";

/// Textures already made, by the folder and file name they were found under: every model that names the same atlas file
/// (the blocky props all share `blocky_atlas.png`) shares one texture and so one batch.
#[derive(Default)]
pub struct AtlasCache(HashMap<String, (Texture2D, usize)>);

/// Vertices and triangles of one destination (the body, a hinge's leaf, the locker door).
#[derive(Default)]
struct Geometry {
    verts: Vec<Vertex>,
    idx: Vec<u32>,
}

impl Geometry {
    fn into_part(mut self, tex: &Texture2D, key: usize) -> Vec<Part> {
        if self.verts.is_empty() {
            return vec![];
        }
        // a model stays loaded for the session: drop the spare capacity its growth left
        self.verts.shrink_to_fit();
        self.idx.shrink_to_fit();
        vec![Part {
            verts: self.verts,
            idx: self.idx,
            tex: tex.clone(),
            key,
        }]
    }
}

/// Whether a prop texture of this size may take the REPEAT wrap. The browser build draws on WebGL 1, which samples a
/// texture whose width or height is not a power of two as opaque black once it repeats, so there such a texture keeps
/// the default clamp (its UVs stay in 0..1). Native GL repeats every size, so native keeps REPEAT for every texture.
fn may_repeat(width: u32, height: u32, webgl1: bool) -> bool {
    !webgl1 || (width.is_power_of_two() && height.is_power_of_two())
}

fn repeat(t: &Texture2D) {
    use macroquad::miniquad::TextureWrap;
    if !may_repeat(t.width() as u32, t.height() as u32, cfg!(target_arch = "wasm32")) {
        return;
    }
    let id = t.raw_miniquad_id();
    unsafe {
        get_internal_gl().quad_context.texture_set_wrap(
            id,
            TextureWrap::Repeat,
            TextureWrap::Repeat,
        );
    }
}

fn new_texture(bytes: &[u8]) -> Texture2D {
    // a picture that does not decode draws white instead of panicking
    let t = Image::from_file_with_format(bytes, None)
        .map(|img| Texture2D::from_image(&img))
        .unwrap_or_else(|_| super::builder::white_texture());
    t.set_filter(FilterMode::Nearest);
    repeat(&t);
    t
}

/// Blends each vertex normal with the mean normal of the vertices that share its place (the blocky props' rounded look).
fn soften(verts: &mut [Vertex]) {
    let key = |p: Vec3| {
        (
            (p.x * 64.0).round() as i32,
            (p.y * 64.0).round() as i32,
            (p.z * 64.0).round() as i32,
        )
    };
    let mut sum: HashMap<(i32, i32, i32), Vec3> = HashMap::new();
    for v in verts.iter() {
        *sum.entry(key(v.position)).or_insert(Vec3::ZERO) += v.normal.truncate();
    }
    for v in verts.iter_mut() {
        let face = v.normal.truncate();
        let avg = sum[&key(v.position)].normalize_or_zero();
        let n = (face * 0.45 + avg * 0.55).normalize_or_zero();
        v.normal = vec4(n.x, n.y, n.z, v.normal.w);
    }
}

/// Stretches model-space vertices by `s` per axis, and turns the normals the way a stretched surface faces (by the
/// inverse factors, then unit length); `normal.w`, the shader mode, is kept.
fn stretch_verts(verts: &mut [Vertex], s: Vec3) {
    for v in verts.iter_mut() {
        v.position *= s;
        let n = (v.normal.truncate() / s).normalize_or_zero();
        v.normal = vec4(n.x, n.y, n.z, v.normal.w);
    }
}

fn white_vertex(position: Vec3, uv: Vec2, normal: Vec3) -> Vertex {
    Vertex {
        position,
        uv,
        color: [255; 4],
        normal: vec4(normal.x, normal.y, normal.z, 1.0),
    }
}

/// A byte range of buffer `idx`, or `None` when the buffer or the range does not exist (malformed GLB view).
fn checked_view(buffers: &[Vec<u8>], idx: usize, offset: usize, length: usize) -> Option<&[u8]> {
    let end = offset.checked_add(length)?;
    buffers.get(idx)?.get(offset..end)
}

/// True when the optional vertex attributes all have one entry per position.
fn attr_counts_match(pos: usize, normals: usize, uvs: usize, cols: usize) -> bool {
    normals == pos && uvs == pos && cols == pos
}

#[cfg(test)]
mod glb_guard_tests {
    use super::*;

    #[test]
    fn a_view_past_the_buffer_is_refused() {
        let bufs = vec![vec![1u8, 2, 3, 4]];
        assert_eq!(checked_view(&bufs, 0, 1, 2), Some(&[2u8, 3][..]));
        assert!(checked_view(&bufs, 0, 3, 5).is_none());
        assert!(checked_view(&bufs, 0, usize::MAX, 2).is_none());
        assert!(checked_view(&bufs, 1, 0, 1).is_none());
    }

    #[test]
    fn short_attribute_accessors_are_refused() {
        assert!(attr_counts_match(3, 3, 3, 3));
        assert!(!attr_counts_match(3, 2, 3, 3));
        assert!(!attr_counts_match(3, 3, 1, 3));
        assert!(!attr_counts_match(3, 3, 3, 0));
    }
}

/// One hinge's leaf as `split_faces` found it: in the hinge's own frame (the authored, open pose).
struct SplitHinge {
    name: String,
    pivot: Vec3,
    geo: Geometry,
    closed_x: (f32, f32),
}

/// A model's faces sorted into the body, each door hinge's leaf and the locker door, with the model's bounds (the door
/// leaves taken closed) and its door measures. Reads no texture, so the door build can be tested without a graphics
/// context.
struct Split {
    body: Geometry,
    hinges: Vec<SplitHinge>,
    locker: Option<(Vec3, Geometry)>,
    min: Vec3,
    max: Vec3,
    door: Option<door::DoorDims>,
}

/// Sorts `faces` (each with the name and origin, in model units, of the outliner group it hangs under) by group, `s`
/// metres per model unit. `None` when no face lands in the body.
fn split_faces<'a>(faces: impl Iterator<Item = (Option<(&'a str, Vec3)>, &'a Face)>, s: f32) -> Option<Split> {
    let mut body = Geometry::default();
    let mut leaf = Geometry::default();
    let mut hinges: Vec<SplitHinge> = vec![];
    let mut locker_pivot = None;
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let mut measure = DoorMeasure::default();
    for (group, face) in faces {
        let name = group.map(|g| g.0).unwrap_or("");
        let pivot = group.map(|g| g.1 * s).unwrap_or(Vec3::ZERO);
        let hinge = door::closed_hinge_deg(name).and_then(|deg| {
            if !hinges.iter().any(|h| h.name == name) {
                measure.hinge(name);
                hinges.push(SplitHinge {
                    name: name.to_string(),
                    pivot,
                    geo: Geometry::default(),
                    closed_x: (f32::MAX, f32::MIN),
                });
            }
            hinges.iter().position(|h| h.name == name).map(|i| (i, deg))
        });
        let is_locker = name == LOCKER_DOOR_GROUP;
        if is_locker {
            locker_pivot = Some(pivot);
        }
        let geo = match hinge {
            Some((i, _)) => &mut hinges[i].geo,
            None if is_locker => &mut leaf,
            None => &mut body,
        };
        let mut x_hits: Vec<(usize, f32)> = Vec::new();
        let base = geo.verts.len() as u32;
        for (c, uv) in face.corners.iter().zip(&face.uv) {
            // the model as authored (a door's leaves open); the hinge's closed pose is what is measured and bounded
            let w = *c * s;
            let (p, store) = match hinge {
                Some((i, deg)) => {
                    let store = w - pivot;
                    let p = door::closed_hinge_local(Mat4::from_translation(pivot), deg).transform_point3(store);
                    measure.leaf_vertex(p.y);
                    x_hits.push((i, p.x));
                    (p, store)
                }
                None if is_locker => (w, w - pivot),
                None => (w, w),
            };
            min = min.min(p);
            max = max.max(p);
            match name {
                "DoorFrameTop" => measure.header_vertex(p.y),
                "DoorFrameL" | "DoorFrameR" => measure.jamb_vertex(p.x),
                _ => {}
            }
            geo.verts.push(white_vertex(store, *uv, face.normal));
        }
        for k in 1..(face.corners.len() as u32).saturating_sub(1) {
            geo.idx.extend_from_slice(&[base, base + k, base + k + 1]);
        }
        for (i, px) in x_hits {
            let x = &mut hinges[i].closed_x;
            *x = (x.0.min(px), x.1.max(px));
        }
    }
    if body.verts.is_empty() {
        return None;
    }
    let door = measure.finish(max.y);
    Some(Split {
        body,
        hinges,
        locker: locker_pivot.map(|pivot| (pivot, leaf)),
        min,
        max,
        door,
    })
}

impl PropModel {
    /// A model from its bbmodel text. `find` reads a file the model names next to itself (a shared atlas), `scope` is the
    /// folder the model sits in (the key `cache` shares a found atlas under), and `soft` rounds the normals the way the
    /// blocky props are drawn.
    pub fn parse(
        text: &str,
        find: &dyn Fn(&str) -> Option<Vec<u8>>,
        scope: &str,
        cache: &mut AtlasCache,
        soft: bool,
    ) -> Option<PropModel> {
        let j: serde_json::Value = serde_json::from_str(text).ok()?;
        let (file, bytes) = bbmodel::texture_bytes(&j, find)?;
        // A named atlas file is shared by its path. A picture embedded in the model is shared by its content: 1047
        // of the built-in models embed a picture and only 207 of those differ, so equal ones become one texture
        // (less memory, and one batch for every model that uses it).
        let shared = Some(match file.as_ref() {
            Some(f) => format!("{scope}/{f}"),
            None => {
                use sha2::{Digest, Sha256};
                let digest = Sha256::digest(&bytes);
                format!("sha256:{}", digest.iter().map(|b| format!("{b:02x}")).collect::<String>())
            }
        });
        let (tex, key) = match shared.as_ref().and_then(|k| cache.0.get(k)) {
            Some((t, k)) => (t.clone(), *k),
            None => {
                let t = new_texture(&bytes);
                let k = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
                if let Some(k2) = shared {
                    cache.0.insert(k2, (t.clone(), k));
                }
                (t, k)
            }
        };
        let m = Model::from_json(&j, Some(tex.clone()), find, None)?;
        let split = split_faces(
            m.faces().map(|(gi, face)| (gi.map(|i| (m.groups[i].name.as_str(), m.groups[i].origin)), face)),
            m.scale,
        )?;
        let hinges: Vec<Hinge> = split
            .hinges
            .into_iter()
            .map(|h| Hinge {
                name: h.name,
                base: Mat4::IDENTITY,
                local: Mat4::from_translation(h.pivot),
                closed_x: h.closed_x,
                parts: h.geo.into_part(&tex, key),
            })
            .collect();
        let locker_door = split.locker.map(|(pivot, geo)| LockerDoor {
            pivot,
            parts: geo.into_part(&tex, key),
        });
        let mut body = split.body;
        if soft {
            soften(&mut body.verts);
        }
        // a type with a draw-only stretch per axis (data/render/prop_axis_scale.json): baked in here, once per model
        let (mut min, mut max) = (split.min, split.max);
        let plain = split.door.is_none() && hinges.is_empty() && locker_door.is_none();
        let stretch = j["name"]
            .as_str()
            .filter(|_| plain)
            .and_then(|name| super::prop_scale::model_axis_scale(&format!("{scope}/{name}.bbmodel")));
        if let Some(s) = stretch {
            stretch_verts(&mut body.verts, Vec3::from(s));
            min *= Vec3::from(s);
            max *= Vec3::from(s);
        }
        Some(PropModel {
            parts: body.into_part(&tex, key),
            min,
            max,
            door: split.door,
            hinges,
            locker_door,
        })
    }

    /// A static model from a mod pack's `.glb` prop (the only GLB the game still reads): its meshes with their node
    /// transforms, base colour textures, factors and vertex colours, one part per texture. `sibling` reads a file the
    /// model names by URI; `white` stands in for a mesh without a texture.
    pub fn parse_glb(
        bytes: &[u8],
        sibling: &dyn Fn(&str) -> Option<Vec<u8>>,
        white: &Texture2D,
    ) -> Option<PropModel> {
        use crate::render::glb::{Glb, ImageSource};
        let g = Glb::parse(bytes)?;
        let buffers = g.buffers(sibling)?;
        let mut textures: Vec<Option<Texture2D>> = vec![];
        for img in g.images() {
            let data = match img {
                // A view that runs past its buffer leaves the picture out (white fallback), never panics.
                Some(ImageSource::View { buffer, offset, length }) => checked_view(&buffers, buffer, offset, length).map(|s| s.to_vec()),
                Some(ImageSource::Uri(uri)) => sibling(&uri),
                None => None,
            };
            textures.push(data.map(|d| new_texture(&d)));
        }
        let mut by_tex: HashMap<Option<usize>, Geometry> = HashMap::new();
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        for (matrix, mesh) in g.mesh_nodes()? {
            let m = Mat4::from_cols_array(&matrix);
            let nm = Mat3::from_mat4(m).inverse().transpose();
            for prim in g.primitives(mesh, &buffers) {
                let pos = prim.positions;
                let normals = prim.normals.unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; pos.len()]);
                let uvs = prim.uvs.unwrap_or_else(|| vec![[0.0, 0.0]; pos.len()]);
                let cols = prim.colors.unwrap_or_else(|| vec![[1.0; 4]; pos.len()]);
                if !attr_counts_match(pos.len(), normals.len(), uvs.len(), cols.len()) {
                    continue;
                }
                let factor = prim.base_color;
                let tex_index = prim.image.filter(|i| textures.get(*i).map(|t| t.is_some()).unwrap_or(false));
                let entry = by_tex.entry(tex_index).or_default();
                let base = entry.verts.len() as u32;
                for i in 0..pos.len() {
                    let p = m.transform_point3(Vec3::from(pos[i]));
                    min = min.min(p);
                    max = max.max(p);
                    let n = (nm * Vec3::from(normals[i])).normalize_or_zero();
                    let c = |k: usize| (factor[k] * cols[i][k] * 255.0).clamp(0.0, 255.0) as u8;
                    entry.verts.push(Vertex {
                        position: p,
                        uv: Vec2::from(uvs[i]),
                        color: [c(0), c(1), c(2), c(3)],
                        normal: vec4(n.x, n.y, n.z, 1.0),
                    });
                }
                match prim.indices {
                    // an index past the vertices would read outside the mesh: drop those triangles' indices
                    Some(ix) => entry.idx.extend(ix.into_iter().filter(|i| (*i as usize) < pos.len()).map(|i| i + base)),
                    None => entry.idx.extend((0..pos.len() as u32).map(|i| i + base)),
                }
            }
        }
        if by_tex.is_empty() {
            return None;
        }
        let parts = by_tex
            .into_iter()
            .map(|(t, geo)| Part {
                verts: geo.verts,
                idx: geo.idx,
                tex: t.and_then(|i| textures[i].clone()).unwrap_or_else(|| white.clone()),
                key: NEXT_KEY.fetch_add(1, Ordering::Relaxed),
            })
            .collect();
        Some(PropModel {
            parts,
            min,
            max,
            door: None,
            hinges: vec![],
            locker_door: None,
        })
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// Adds the model, placed by `xf`, to per-texture builders (static world batching).
    pub fn add_to(&self, builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color) {
        add_parts(&self.parts, builders, xf, tint, |p| p);
    }

    /// Adds a room door's frame (everything but the leaves), placed by `xf`, with the posts and sill slid out to the
    /// doorway when `fit` says the widen alone falls short (`door::DoorFit`).
    pub fn add_door_body(&self, builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color, fit: &door::DoorFit) {
        add_parts(&self.parts, builders, xf, tint, |p| vec3(fit.body_x(p.x), p.y, p.z));
    }

    /// Adds a room door's hinge leaves to `builders`, the door placed by `xf` and swung to `open` (0 shut, 1 open):
    /// each leaf turns about its hinge to `door::hinge_deg`, from the same hinge frame the static batch closes.
    pub fn add_hinges(&self, builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color, open: f32, fit: &door::DoorFit) {
        let Some(dims) = self.door else { return };
        for h in &self.hinges {
            let leaf = door::leaf_fit(dims, fit, &h.name, h.closed_x);
            add_parts(&h.parts, builders, xf * door::leaf_xf(h.base, h.local, &h.name, open, &leaf), tint, |p| p);
        }
    }

    /// Adds a hiding spot's door to `builders`, swung `angle` radians open about its hinge line (toward the front).
    pub fn add_locker_door(&self, builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color, angle: f32) {
        if let Some(d) = &self.locker_door {
            let swing = xf * Mat4::from_translation(d.pivot) * Mat4::from_rotation_y(-angle);
            add_parts(&d.parts, builders, swing, tint, |p| p);
        }
    }

    /// Draws the model right now (dynamic objects: pickups, fixtures, the held tool).
    pub fn draw(&self, xf: Mat4, tint: Color) {
        let mut builders = HashMap::new();
        self.add_to(&mut builders, xf, tint);
        draw_builders(builders);
    }

    /// [`Self::draw`] for a model that rides a runner: `tint.a` below 1 blends it the way the runner's body blends
    /// (`shader::GHOST_MODE`), so a faded runner does not leave an opaque item in the air. The runner's lighting and fog
    /// come from the material the caller set, as for the body.
    pub fn draw_faded(&self, xf: Mat4, tint: Color) {
        let mut builders = HashMap::new();
        self.add_to(&mut builders, xf, tint);
        if tint.a < 1.0 {
            for b in builders.values_mut() {
                fade_verts(&mut b.verts, tint.a);
            }
        }
        draw_builders(builders);
    }

    /// The meshes `add_to` builds for this placement, built the first frame they are asked for and kept by `cache` while
    /// the prop stays drawn (a static prop: its transform and tint do not move between frames).
    pub fn static_meshes<'c>(
        &self,
        cache: &'c mut StaticCache<Placement, Vec<Mesh>>,
        xf: Mat4,
        tint: Color,
    ) -> &'c Vec<Mesh> {
        cache.get_or_build(Placement::of(self, xf, tint), || {
            let mut builders = HashMap::new();
            self.add_to(&mut builders, xf, tint);
            builders.into_values().flat_map(|b| b.finish()).collect()
        })
    }
}

/// A static prop's drawn placement: the model (by address, which the asset map keeps fixed), its transform and its tint,
/// as bits so the key compares exactly. A prop moved or re-tinted is a different key, so it is built again.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Placement {
    model: usize,
    xf: [u32; 16],
    tint: [u32; 4],
}

impl Placement {
    fn of(model: &PropModel, xf: Mat4, tint: Color) -> Placement {
        Placement {
            model: model as *const PropModel as usize,
            xf: xf.to_cols_array().map(f32::to_bits),
            tint: [tint.r, tint.g, tint.b, tint.a].map(f32::to_bits),
        }
    }
}

/// Values kept across frames for what is drawn with a fixed placement. A value is built the first frame it is asked for
/// and reused while it keeps being asked for; one not asked for during a whole frame is dropped at the next
/// `begin_frame`, so the cache follows what is on screen.
pub struct StaticCache<K, V> {
    prev: HashMap<K, V>,
    next: HashMap<K, V>,
}

impl<K, V> Default for StaticCache<K, V> {
    fn default() -> Self {
        StaticCache {
            prev: HashMap::new(),
            next: HashMap::new(),
        }
    }
}

impl<K: Hash + Eq + Clone, V> StaticCache<K, V> {
    /// Starts a frame: what was asked for in the last frame stays, and what was not is dropped.
    pub fn begin_frame(&mut self) {
        self.prev = std::mem::take(&mut self.next);
    }

    /// The value for `key`: kept from an earlier frame if there is one, else built now.
    pub fn get_or_build(&mut self, key: K, build: impl FnOnce() -> V) -> &V {
        if !self.next.contains_key(&key) {
            let v = match self.prev.remove(&key) {
                Some(v) => v,
                None => build(),
            };
            self.next.insert(key.clone(), v);
        }
        &self.next[&key]
    }
}

/// The variant of a prop type a placement draws: `stable_index(key)` into `models`, else the first one `loaded` accepts
/// (a variant whose file did not load). `None` when no variant is loaded. The key is the room's variant key plus the
/// prop id, so one instance always gets the same variant.
pub fn pick_variant(models: &[String], key: &str, loaded: impl Fn(&str) -> bool) -> Option<usize> {
    if models.is_empty() {
        return None;
    }
    let at = crate::content::props::stable_index(key, models.len());
    if loaded(&models[at]) {
        return Some(at);
    }
    models.iter().position(|m| loaded(m))
}

/// Models by (kind, variant): each variant is loaded once, the first time an instance asks for it, and every instance
/// of that variant shares it. Draw code asks with the variant `pick_variant` chose per instance, so two props of one
/// kind can show different models without one load per prop.
pub struct VariantCache<M> {
    models: HashMap<(String, usize), M>,
    loads: usize,
}

impl<M> Default for VariantCache<M> {
    fn default() -> Self {
        VariantCache { models: HashMap::new(), loads: 0 }
    }
}

impl<M> VariantCache<M> {
    pub fn get_or_load(&mut self, kind: &str, variant: usize, load: impl FnOnce() -> M) -> &M {
        let loads = &mut self.loads;
        self.models.entry((kind.to_string(), variant)).or_insert_with(|| {
            *loads += 1;
            load()
        })
    }

    /// How many models were loaded so far (one per distinct (kind, variant) asked for).
    pub fn loads(&self) -> usize {
        self.loads
    }
}

fn add_parts(parts: &[Part], builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color, pre: impl Fn(Vec3) -> Vec3) {
    let nm = Mat3::from_mat4(xf).inverse().transpose();
    for p in parts {
        let b = builders
            .entry(p.key)
            .or_insert_with(|| Builder::new(Some(p.tex.clone())));
        add_verts(b, &p.verts, &p.idx, xf, nm, tint, &pre);
    }
}

/// Appends triangles `idx` of `verts` to `b`, each vertex placed by `xf` (its normal by `nm`, the inverse transpose of
/// `xf`) after `pre`, and tinted.
fn add_verts(
    b: &mut Builder,
    verts: &[Vertex],
    idx: &[u32],
    xf: Mat4,
    nm: Mat3,
    tint: Color,
    pre: &impl Fn(Vec3) -> Vec3,
) {
    let place = |v: &Vertex| {
        let n = (nm * v.normal.truncate()).normalize_or_zero();
        let c = [
            (v.color[0] as f32 * tint.r) as u8,
            (v.color[1] as f32 * tint.g) as u8,
            (v.color[2] as f32 * tint.b) as u8,
            v.color[3],
        ];
        Vertex {
            position: xf.transform_point3(pre(v.position)),
            uv: v.uv,
            color: c,
            normal: vec4(n.x, n.y, n.z, v.normal.w),
        }
    };
    // The usual part: its vertices go in once and its triangles index them, so a quad costs 4 vertices and not 6. (A part
    // too big for the 16-bit index falls through to the per-triangle path below.)
    if verts.len() <= SHARED_PART_MAX_VERTS {
        b.reserve(verts.len());
        let base = b.verts.len();
        b.verts.extend(verts.iter().map(place));
        b.idx.extend(idx.iter().map(|&i| (base + i as usize) as u16));
        return;
    }
    for chunk in idx.chunks(3 * 6000) {
        b.reserve(chunk.len());
        let base = b.verts.len() as u16;
        for (k, &i) in chunk.iter().enumerate() {
            b.verts.push(place(&verts[i as usize]));
            b.idx.push(base + k as u16);
        }
    }
}

/// The most vertices a part may have to be added whole (see `add_verts`): well under the builder's 16-bit index limit.
const SHARED_PART_MAX_VERTS: usize = 60_000;

/// Scales the vertices' alpha by `alpha` and switches them to the blended lit mode.
fn fade_verts(verts: &mut [Vertex], alpha: f32) {
    for v in verts {
        v.color[3] = (v.color[3] as f32 * alpha.clamp(0.0, 1.0)) as u8;
        v.normal.w = crate::render::shader::GHOST_MODE;
    }
}

/// Draws the builders' triangles now, one call per texture.
pub fn draw_builders(builders: HashMap<usize, Builder>) {
    for (_, mut b) in builders {
        b.draw_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::embedded;
    use std::cell::Cell;

    /// A unit box standing on the floor, as bare vertices (a model's parts, without a texture).
    pub(super) fn box_geometry() -> (Vec<Vertex>, Vec<u32>) {
        let c = [
            vec3(-0.5, 0.0, -0.5),
            vec3(0.5, 0.0, -0.5),
            vec3(0.5, 1.0, -0.5),
            vec3(-0.5, 1.0, -0.5),
            vec3(-0.5, 0.0, 0.5),
            vec3(0.5, 0.0, 0.5),
            vec3(0.5, 1.0, 0.5),
            vec3(-0.5, 1.0, 0.5),
        ];
        let verts = c
            .iter()
            .map(|&p| white_vertex(p, vec2(p.x + 0.5, p.y), p.normalize_or_zero()))
            .collect();
        let idx = vec![
            0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6, 0, 4, 5, 0, 5, 1, 3, 2, 6, 3, 6, 7, 0, 3, 7, 0, 7, 4, 1, 5, 6, 1, 6, 2,
        ];
        (verts, idx)
    }

    /// The meshes a prop built from `box_geometry` draws with this placement (the same path `static_meshes` takes).
    fn build(xf: Mat4, tint: Color) -> Vec<Mesh> {
        let (verts, idx) = box_geometry();
        let mut b = Builder::new(None);
        let nm = Mat3::from_mat4(xf).inverse().transpose();
        add_verts(&mut b, &verts, &idx, xf, nm, tint, &|p| p);
        b.finish()
    }

    /// Positions, normals, colours and indices of each mesh, to compare two builds exactly.
    fn signature(ms: &[Mesh]) -> Vec<(Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[u8; 4]>, Vec<u16>)> {
        ms.iter()
            .map(|m| {
                (
                    m.vertices.iter().map(|v| v.position.to_array()).collect(),
                    m.vertices.iter().map(|v| [v.normal.x, v.normal.y, v.normal.z]).collect(),
                    m.vertices.iter().map(|v| v.color).collect(),
                    m.indices.clone(),
                )
            })
            .collect()
    }

    /// An opaque prop never takes the blended pass (which draws after the opaque meshes, in build order, and would show
    /// through its own far side), and the world material draws both sides of a triangle: a one-sided piece of a model
    /// is seen from behind instead of vanishing.
    #[test]
    fn a_prop_draws_opaque_and_two_sided() {
        let meshes = build(Mat4::from_translation(vec3(1.0, 0.0, 2.0)), WHITE);
        assert!(!meshes.is_empty());
        assert!(meshes.iter().all(|m| !crate::render::mesh_batch::is_blended(m)), "a prop mesh must not be blended");
        assert!(meshes.iter().flat_map(|m| &m.vertices).all(|v| v.normal.w == 1.0), "a prop vertex is the opaque textured mode");
        assert_eq!(
            macroquad::miniquad::PipelineParams::default().cull_face,
            macroquad::miniquad::CullFace::Nothing,
            "the world material takes the default pipeline: no back-face culling"
        );
    }

    #[test]
    fn cached_static_meshes_equal_a_fresh_build() {
        let placements = [
            (Mat4::from_translation(vec3(4.0, 0.0, 2.0)), WHITE),
            (
                Mat4::from_rotation_y(1.2) * Mat4::from_translation(vec3(-3.0, 0.0, 7.0)),
                Color::new(0.25, 0.2, 0.18, 1.0),
            ),
            (Mat4::from_translation(Vec3::ZERO), Color::new(0.7, 1.0, 0.85, 1.0)),
        ];
        let mut cache: StaticCache<usize, Vec<Mesh>> = StaticCache::default();
        let built = Cell::new(0);
        for _frame in 0..3 {
            cache.begin_frame();
            for (i, (xf, tint)) in placements.iter().enumerate() {
                let cached = cache.get_or_build(i, || {
                    built.set(built.get() + 1);
                    build(*xf, *tint)
                });
                assert!(signature(cached) == signature(&build(*xf, *tint)));
            }
        }
        // three props over three frames: each built once, then kept
        assert_eq!(built.get(), placements.len());
    }

    #[test]
    fn a_faded_item_blends_like_the_body() {
        use crate::render::shader::{GHOST_MODE, LIT_MODE};
        let (mut verts, _) = box_geometry();
        assert!(verts.iter().all(|v| v.normal.w == LIT_MODE && v.color[3] == 255));
        fade_verts(&mut verts, 0.5);
        assert!(verts.iter().all(|v| v.normal.w == GHOST_MODE && (126..=128).contains(&v.color[3])));
    }

    #[test]
    fn only_a_power_of_two_prop_texture_repeats_on_webgl1() {
        // WebGL 1 repeats a power-of-two atlas only; native GL repeats every size
        assert!(may_repeat(128, 64, true));
        assert!(may_repeat(32, 32, true));
        assert!(!may_repeat(96, 16, true));
        assert!(!may_repeat(112, 16, true));
        assert!(may_repeat(96, 16, false));
    }

    #[test]
    fn the_flashlight_atlas_decodes_lit_and_does_not_repeat_on_webgl1() {
        // a 96x16 atlas (not a power of two) decodes to real colours, and on WebGL 1 it must not repeat (it draws black)
        let j: serde_json::Value =
            serde_json::from_slice(embedded::asset("interactables/flashlight_item.bbmodel").expect("embedded")).expect("json");
        let (_, bytes) = bbmodel::texture_bytes(&j, &|_| None).expect("atlas bytes");
        let img = Image::from_file_with_format(&bytes, None).expect("decodes");
        assert_eq!((img.width, img.height), (96, 16));
        let texels = img.get_image_data();
        let lit = texels.iter().filter(|p| p[3] > 0 && (p[0] | p[1] | p[2]) > 0).count();
        assert!(lit > texels.len() / 4, "{lit} of {} texels are lit", texels.len());
        assert!(!may_repeat(img.width as u32, img.height as u32, true));
    }

    #[test]
    fn a_prop_not_asked_for_a_whole_frame_is_built_again() {
        let mut cache: StaticCache<usize, u32> = StaticCache::default();
        let built = Cell::new(0);
        let ask = |cache: &mut StaticCache<usize, u32>| {
            cache.get_or_build(0, || {
                built.set(built.get() + 1);
                7
            });
        };
        cache.begin_frame();
        ask(&mut cache);
        // a whole frame without it drops it, so the next ask builds it again
        cache.begin_frame();
        cache.begin_frame();
        cache.begin_frame();
        ask(&mut cache);
        assert_eq!(built.get(), 2);
    }

    /// A triangle list as corner positions (millimetres), sorted, whatever the vertex order.
    fn triangle_set(ms: &[Mesh]) -> Vec<Vec<[i32; 3]>> {
        let mut out: Vec<Vec<[i32; 3]>> = ms
            .iter()
            .flat_map(|m| {
                m.indices.chunks_exact(3).map(|t| {
                    t.iter()
                        .map(|&i| {
                            let p = m.vertices[i as usize].position;
                            [(p.x * 1000.0).round() as i32, (p.y * 1000.0).round() as i32, (p.z * 1000.0).round() as i32]
                        })
                        .collect()
                })
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_part_is_added_with_shared_vertices_and_the_same_triangles() {
        // two quads (4 vertices, 6 indices each) in a part: 8 vertices, not the 12 a per-triangle copy takes
        let quad = |x: f32| -> Vec<Vertex> {
            [vec3(x, 0.0, 0.0), vec3(x + 1.0, 0.0, 0.0), vec3(x + 1.0, 1.0, 0.0), vec3(x, 1.0, 0.0)]
                .iter()
                .map(|&p| white_vertex(p, vec2(p.x, p.y), vec3(0.0, 0.0, 1.0)))
                .collect()
        };
        let mut verts = quad(0.0);
        verts.extend(quad(3.0));
        let idx: Vec<u32> = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        let xf = Mat4::from_translation(vec3(2.0, 0.0, 5.0)) * Mat4::from_rotation_y(0.7);
        let nm = Mat3::from_mat4(xf).inverse().transpose();
        let mut shared = Builder::new(None);
        add_verts(&mut shared, &verts, &idx, xf, nm, WHITE, &|p| p);
        let shared = shared.finish();
        assert_eq!(shared.iter().map(|m| m.vertices.len()).sum::<usize>(), 8);
        assert_eq!(shared.iter().map(|m| m.indices.len()).sum::<usize>(), 12);
        // the expected triangles, placed one by one
        let mut want: Vec<Vec<[i32; 3]>> = idx
            .chunks_exact(3)
            .map(|t| {
                t.iter()
                    .map(|&i| {
                        let p = xf.transform_point3(verts[i as usize].position);
                        [(p.x * 1000.0).round() as i32, (p.y * 1000.0).round() as i32, (p.z * 1000.0).round() as i32]
                    })
                    .collect()
            })
            .collect();
        want.sort();
        assert_eq!(triangle_set(&shared), want);
        // the box test geometry keeps its triangle count and every index in range
        let boxed = build(Mat4::IDENTITY, WHITE);
        assert_eq!(boxed.iter().map(|m| m.indices.len()).sum::<usize>(), 36);
        assert!(boxed.iter().all(|m| m.indices.iter().all(|&i| (i as usize) < m.vertices.len())));
    }

    const LUXURY_SINGLE: &str = include_str!("../assets/props/mansion/luxury_room_door.bbmodel");
    const LUXURY_DOUBLE: &str = include_str!("../assets/props/mansion/luxury_double_room_door.bbmodel");

    pub(super) fn json_vec(v: &serde_json::Value) -> Vec3 {
        let n = |i: usize| v[i].as_f64().unwrap_or(0.0) as f32;
        vec3(n(0), n(1), n(2))
    }

    /// A door model's faces as `Model::faces` hands them to the loader (bbmodel units), read from the bbmodel text
    /// without a texture: each face with the outliner group it hangs under.
    fn model_faces(text: &str) -> (Vec<(String, Vec3)>, Vec<(Option<usize>, Face)>) {
        let j: serde_json::Value = serde_json::from_str(text).expect("a bbmodel");
        let mut groups: Vec<(String, Vec3)> = vec![];
        let mut owner: HashMap<String, usize> = HashMap::new();
        for n in j["outliner"].as_array().expect("an outliner") {
            if n.is_object() {
                groups.push((n["name"].as_str().unwrap_or("").to_string(), json_vec(&n["origin"])));
                for c in n["children"].as_array().into_iter().flatten().filter_map(|c| c.as_str()) {
                    owner.insert(c.to_string(), groups.len() - 1);
                }
            }
        }
        let mut faces = vec![];
        for e in j["elements"].as_array().expect("elements") {
            let group = e["uuid"].as_str().and_then(|u| owner.get(u).copied());
            if e["type"].as_str() == Some("mesh") {
                for fj in e["faces"].as_object().expect("mesh faces").values() {
                    let keys: Vec<&str> = fj["vertices"].as_array().expect("face vertices").iter().filter_map(|k| k.as_str()).collect();
                    let corners: Vec<Vec3> = keys.iter().map(|k| json_vec(&e["vertices"][*k]) + json_vec(&e["origin"])).collect();
                    let uv = keys
                        .iter()
                        .map(|k| vec2(fj["uv"][*k][0].as_f64().unwrap_or(0.0) as f32 / 128.0, fj["uv"][*k][1].as_f64().unwrap_or(0.0) as f32 / 16.0))
                        .collect();
                    let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]).normalize_or_zero();
                    faces.push((group, Face { corners, uv, normal }));
                }
            } else {
                let (f, t) = (json_vec(&e["from"]), json_vec(&e["to"]));
                let corner = |i: usize| vec3(if i & 1 == 0 { f.x } else { t.x }, if i & 2 == 0 { f.y } else { t.y }, if i & 4 == 0 { f.z } else { t.z });
                for q in [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]] {
                    let corners: Vec<Vec3> = q.iter().map(|&i| corner(i)).collect();
                    let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]).normalize_or_zero();
                    faces.push((group, Face { corners, uv: vec![vec2(0.0, 0.0), vec2(0.1, 0.0), vec2(0.1, 0.1), vec2(0.0, 0.1)], normal }));
                }
            }
        }
        (groups, faces)
    }

    /// The triangles (world positions) a geometry adds to a builder under `xf`, `pre` applied first, as the door build
    /// does (`add_door_body`, `add_hinges`).
    fn placed(geo: &Geometry, xf: Mat4, pre: impl Fn(Vec3) -> Vec3) -> Vec<[Vec3; 3]> {
        let mut b = Builder::new(None);
        let nm = Mat3::from_mat4(xf).inverse().transpose();
        add_verts(&mut b, &geo.verts, &geo.idx, xf, nm, WHITE, &pre);
        let mut out = vec![];
        for m in b.finish() {
            for t in m.indices.chunks_exact(3) {
                out.push([m.vertices[t[0] as usize].position, m.vertices[t[1] as usize].position, m.vertices[t[2] as usize].position]);
            }
        }
        out
    }

    fn area(tris: &[[Vec3; 3]]) -> f32 {
        tris.iter().map(|t| 0.5 * (t[1] - t[0]).cross(t[2] - t[0]).length()).sum()
    }

    fn bounds(tris: &[[Vec3; 3]]) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for p in tris.iter().flatten() {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        (lo, hi)
    }

    /// The mansion's room doors (TODO: "Mansion doors are invisible"), loaded the way the game loads them and built the
    /// way a mansion room builds them: the real `luxuryRoomDoor` and `luxuryDoubleRoomDoor` models are sorted into body
    /// and hinge leaves by `split_faces` (the loader's own step), fitted to their doorway by `DoorFit` and `leaf_fit`, and
    /// placed. The frame and every leaf must come out as finite triangles with area, the leaves standing on the floor
    /// and flat in the wall when shut, the crown above the bar, all in the doorway.
    #[test]
    fn a_mansion_door_builds_visible_triangles_in_its_doorway() {
        for (ptype, text, double, spans) in [
            ("luxuryRoomDoor", LUXURY_SINGLE, false, vec![1]),
            ("luxuryDoubleRoomDoor", LUXURY_DOUBLE, true, vec![2, 3]),
        ] {
            let (groups, faces) = model_faces(text);
            let split = split_faces(faces.iter().map(|(g, f)| (g.map(|i| (groups[i].0.as_str(), groups[i].1)), f)), 1.0 / 16.0)
                .unwrap_or_else(|| panic!("{ptype}: no body"));
            let dims = split.door.unwrap_or_else(|| panic!("{ptype}: not read as a door"));
            assert_eq!(dims.double, double, "{ptype}");
            assert_eq!(split.hinges.len(), if double { 2 } else { 1 }, "{ptype}: hinges");
            assert!(!split.body.idx.is_empty(), "{ptype}: empty body");
            for span in spans {
                let ctx = format!("{ptype} span {span}");
                let fit = door::DoorFit::new(dims, ptype, span);
                assert!(fit.sx.is_finite() && fit.sy.is_finite() && fit.sx > 0.0 && fit.sy >= 1.0, "{ctx}: fit {fit:?}");
                let xf = door::door_transform(Vec3::ZERO, 0.0, fit.sx, fit.sy, 1.0);
                let half = span as f32 / 2.0;
                // the frame: posts, bar, sill and crown
                let body = placed(&split.body, xf, |p| vec3(fit.body_x(p.x), p.y, p.z));
                assert!(body.len() >= 24 && area(&body) > 1.0, "{ctx}: frame {} triangles, area {}", body.len(), area(&body));
                let (lo, hi) = bounds(&body);
                assert!(lo.is_finite() && hi.is_finite(), "{ctx}: frame bounds");
                assert!(lo.x <= -half && hi.x >= half, "{ctx}: frame spans the doorway ({}..{})", lo.x, hi.x);
                assert!(hi.y > dims.bar_top * fit.sy + 0.05, "{ctx}: no crown over the bar ({})", hi.y);
                // the leaves, shut and open
                for open in [0.0, 1.0] {
                    for h in &split.hinges {
                        assert!(h.closed_x.1 - h.closed_x.0 > 0.3, "{ctx}: {} closes to a zero-width leaf {:?}", h.name, h.closed_x);
                        assert!(!h.geo.idx.is_empty(), "{ctx}: {} has no triangles", h.name);
                        let leaf = door::leaf_fit(dims, &fit, &h.name, h.closed_x);
                        assert!(leaf.widen.is_finite() && leaf.widen > 0.0 && leaf.ky.is_finite() && leaf.ky > 0.0, "{ctx}: {leaf:?}");
                        let m = xf * door::leaf_xf(Mat4::IDENTITY, Mat4::from_translation(h.pivot), &h.name, open, &leaf);
                        let tris = placed(&h.geo, m, |p| p);
                        assert!(tris.len() >= 12 && area(&tris) > 1.0, "{ctx}: {} open {open}: {} triangles, area {}", h.name, tris.len(), area(&tris));
                        let (lo, hi) = bounds(&tris);
                        assert!(lo.is_finite() && hi.is_finite(), "{ctx}: {} bounds", h.name);
                        assert!(hi.y - lo.y > 1.5 && lo.y >= -1e-3 && lo.y < 0.05, "{ctx}: {} stands on the floor ({}..{})", h.name, lo.y, hi.y);
                        assert!(lo.x >= -half - 0.2 && hi.x <= half + 0.2, "{ctx}: {} is in the doorway ({lo:?}..{hi:?})", h.name);
                        if open == 0.0 {
                            assert!(hi.z - lo.z < 0.3, "{ctx}: {} shut is flat in the wall ({})", h.name, hi.z - lo.z);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod variant_tests {
    use super::*;
    use super::tests::{box_geometry, json_vec};
    use crate::content::embedded;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("props/maze/locker_{i}.bbmodel")).collect()
    }

    #[test]
    fn each_instance_gets_the_variant_its_key_names_and_keeps_it() {
        let models = names(3);
        let all = |_: &str| true;
        let mut seen = std::collections::HashSet::new();
        for i in 0..60 {
            let key = format!("room{i}:prop{i}");
            let a = pick_variant(&models, &key, all).unwrap();
            assert_eq!(a, crate::content::props::stable_index(&key, 3));
            assert_eq!(Some(a), pick_variant(&models, &key, all), "stable for one instance");
            seen.insert(a);
        }
        assert_eq!(seen.len(), 3, "different instances spread over the variants");
    }

    /// The arcade and carnival types of the props pass: every variant their data lists is reached by some room key,
    /// and a room keeps the one it was given (the pick is the stable hash of the key, nothing random).
    #[test]
    fn every_listed_variant_of_the_arcade_and_carnival_props_is_picked_by_some_room() {
        let content = crate::content::Content::load();
        for (ptype, at_least) in [
            ("arcadeCabinet", 11),
            ("arcadeCabinetCyanMagenta", 3),
            ("arcadeTokenChanger", 2),
            ("prizeClawMachine", 3),
            ("prizeCounter", 2),
            ("carnivalTicketBooth", 2),
            ("popcornCart", 2),
            ("ringTossStall", 2),
            ("strengthTester", 2),
            ("circusPedestal", 2),
        ] {
            let models = &content.props.get(ptype).unwrap_or_else(|| panic!("{ptype} is a prop type")).models;
            assert!(models.len() >= at_least, "{ptype}: {} variants", models.len());
            let mut seen = std::collections::HashSet::new();
            for room in 0..400 {
                let key = format!("C:{}:0:{}:ROOM:{}", room % 7, room / 7, room % 5);
                let a = pick_variant(models, &key, |_| true).unwrap();
                assert_eq!(Some(a), pick_variant(models, &key, |_| true), "{ptype}: stable for one room");
                seen.insert(a);
            }
            assert_eq!(seen.len(), models.len(), "{ptype}: every variant is drawn somewhere");
        }
        // the three booths that shared a model each have their own now
        let model = |t: &str| content.props[t].models.clone();
        assert_ne!(model("shootingGallery"), model("whackAMole"));
        assert!(!model("shootingGallery").iter().chain(&model("whackAMole")).any(|m| model("ringTossStall").contains(m)));
        assert!(!model("duckPond").iter().any(|m| model("circusPedestal").contains(m)));
    }

    /// Every model the props pass added reads as a model with geometry (the game's reader, no graphics context), is
    /// built from the one shared palette picture (so they all batch together), stays within the triangle budget of
    /// the converted originals, and stands on the floor inside the box of the model it is a variant of.
    #[test]
    fn every_props_pass_model_parses_and_fits_the_model_it_stands_in_for() {
        // (new model, the model whose place it takes or shares, metres of slack on each side)
        let pairs = [
            ("props/maze/arcade_cabinet_lime.bbmodel", "props/maze/arcade_cabinet_default.bbmodel"),
            ("props/maze/arcade_cabinet_sunset.bbmodel", "props/maze/arcade_cabinet_default.bbmodel"),
            ("props/maze/arcade_cabinet_mono.bbmodel", "props/maze/arcade_cabinet_default.bbmodel"),
            ("props/maze/arcade_cabinet_crimson.bbmodel", "props/maze/arcade_cabinet_default.bbmodel"),
            ("props/maze/arcade_cabinet_racer.bbmodel", "props/maze/arcade_cabinet_default.bbmodel"),
            ("props/maze/arcade_cabinet_blaster.bbmodel", "props/maze/arcade_cabinet_default.bbmodel"),
            ("props/maze/arcade_cabinet_magenta_gold.bbmodel", "props/maze/arcade_cabinet_cyan_magenta.bbmodel"),
            ("props/maze/arcade_cabinet_cyan_lime.bbmodel", "props/maze/arcade_cabinet_cyan_magenta.bbmodel"),
            ("props/maze/prize_claw_machine_pink.bbmodel", "props/maze/prize_claw_machine.bbmodel"),
            ("props/maze/prize_claw_machine_blue.bbmodel", "props/maze/prize_claw_machine.bbmodel"),
            ("props/maze/arcade_ticket_redeemer.bbmodel", "props/maze/arcade_token_changer.bbmodel"),
            ("props/maze/prize_plush_shelf.bbmodel", "props/maze/prize_counter.bbmodel"),
            ("props/carnival/shooting_gallery.bbmodel", "props/carnival/ring_toss_stall.bbmodel"),
            ("props/carnival/whack_a_mole.bbmodel", "props/carnival/ring_toss_stall.bbmodel"),
            ("props/carnival/balloon_dart_stall.bbmodel", "props/carnival/ring_toss_stall.bbmodel"),
            ("props/carnival/duck_pond.bbmodel", "props/carnival/circus_pedestal.bbmodel"),
            ("props/carnival/circus_pedestal_blue.bbmodel", "props/carnival/circus_pedestal.bbmodel"),
            ("props/carnival/carnival_ticket_booth_blue.bbmodel", "props/carnival/carnival_ticket_booth.bbmodel"),
            ("props/carnival/cotton_candy_cart.bbmodel", "props/carnival/popcorn_cart.bbmodel"),
            ("props/carnival/strength_tester_blue.bbmodel", "props/carnival/strength_tester.bbmodel"),
        ];
        // a model's document, its bounds in metres (cube corners and mesh vertices) and its drawn triangles
        let read = |path: &str| -> (serde_json::Value, Vec3, Vec3, usize) {
            let text = std::str::from_utf8(embedded::asset(path).unwrap_or_else(|| panic!("{path} is embedded"))).unwrap();
            let j: serde_json::Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{path}: {e}"));
            let (mut lo, mut hi, mut tris) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), 0);
            for e in j["elements"].as_array().unwrap_or_else(|| panic!("{path}: elements")) {
                if e["type"].as_str() == Some("mesh") {
                    for v in e["vertices"].as_object().into_iter().flat_map(|o| o.values()) {
                        let p = (json_vec(v) + json_vec(&e["origin"])) / 16.0;
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                    tris += e["faces"].as_object().map_or(0, |f| f.values().map(|x| x["vertices"].as_array().map_or(0, |k| k.len().saturating_sub(2))).sum());
                } else {
                    let (f, t) = (json_vec(&e["from"]) / 16.0, json_vec(&e["to"]) / 16.0);
                    assert!(f.cmplt(t).all(), "{path}: {} has no volume", e["name"]);
                    lo = lo.min(f);
                    hi = hi.max(t);
                    tris += e["faces"].as_object().map_or(0, |faces| faces.values().filter(|x| !x["texture"].is_null()).count() * 2);
                }
            }
            (j, lo, hi, tris)
        };
        let mut palette: Option<String> = None;
        for (new, old) in pairs {
            let (j, lo, hi, tris) = read(new);
            let (_, old_lo, old_hi, _) = read(old);
            assert!(j["elements"].as_array().unwrap().iter().all(|e| e["type"].as_str() == Some("cube")), "{new}: cubes only");
            assert!((100..=520).contains(&tris), "{new}: {tris} triangles");
            assert!(lo.y.abs() < 1e-4, "{new}: stands on the floor, lowest point {}", lo.y);
            let slack = 0.02;
            assert!(lo.cmpge(old_lo - vec3(slack, 0.1, slack)).all() && hi.cmple(old_hi + Vec3::splat(slack)).all(), "{new}: {lo:?}..{hi:?} is outside {old}: {old_lo:?}..{old_hi:?}");
            assert!((hi.y - old_hi.y).abs() < 0.05, "{new}: as tall as {old}");
            // every drawn face samples inside the picture, and the picture is a PNG inside the file
            let (w, h) = bbmodel::uv_space(&j, 0.0, 0.0);
            assert!(w > 0.0 && h > 0.0, "{new}: uv space");
            for e in j["elements"].as_array().unwrap() {
                for f in e["faces"].as_object().unwrap().values().filter(|x| !x["texture"].is_null()) {
                    let uv: Vec<f32> = f["uv"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
                    assert!(uv.len() == 4 && uv[0] >= 0.0 && uv[2] <= w && uv[1] >= 0.0 && uv[3] <= h && uv[0] < uv[2] && uv[1] < uv[3], "{new}: {} uv {uv:?}", e["name"]);
                }
            }
            let (file, bytes) = bbmodel::texture_bytes(&j, &|_| None).unwrap_or_else(|| panic!("{new}: no picture"));
            assert!(file.is_none() && bytes.starts_with(&[0x89, b'P', b'N', b'G']), "{new}: its picture is a PNG inside the file");
            let source = j["textures"][0]["source"].as_str().unwrap().to_string();
            match &palette {
                Some(p) => assert_eq!(p, &source, "{new}: the shared palette picture"),
                None => palette = Some(source),
            }
        }
    }

    #[test]
    fn a_stretch_scales_each_axis_and_keeps_flat_faces_flat() {
        let (mut verts, _) = box_geometry();
        let before: Vec<Vec3> = verts.iter().map(|v| v.position).collect();
        stretch_verts(&mut verts, vec3(2.0, 0.5, 1.5));
        for (v, b) in verts.iter().zip(&before) {
            assert_eq!(v.position, *b * vec3(2.0, 0.5, 1.5));
            let n = v.normal.truncate();
            assert!(n.length() < 1e-6 || (n.length() - 1.0).abs() < 1e-5, "unit normals");
            assert_eq!(v.normal.w, 1.0, "the shader mode is kept");
        }
        let mut one = [white_vertex(vec3(1.0, 1.0, 1.0), vec2(0.0, 0.0), vec3(0.0, 1.0, 0.0))];
        stretch_verts(&mut one, vec3(3.0, 0.5, 1.0));
        assert!((one[0].normal.truncate() - vec3(0.0, 1.0, 0.0)).length() < 1e-6, "an axis face still faces its axis");
    }

    #[test]
    fn a_variant_that_did_not_load_falls_back_to_one_that_did() {
        let models = names(3);
        let only_last = |m: &str| m == models[2];
        for i in 0..20 {
            assert_eq!(pick_variant(&models, &format!("k{i}"), only_last), Some(2));
        }
        assert_eq!(pick_variant(&models, "k", |_| false), None);
        assert_eq!(pick_variant(&[], "k", |_| true), None);
    }

    #[test]
    fn the_cache_loads_each_kind_and_variant_once() {
        let mut cache: VariantCache<String> = VariantCache::default();
        for _ in 0..5 {
            assert_eq!(cache.get_or_load("locker", 0, || "a".into()), "a");
            assert_eq!(cache.get_or_load("locker", 1, || "b".into()), "b");
            assert_eq!(cache.get_or_load("lamp", 0, || "c".into()), "c");
        }
        assert_eq!(cache.loads(), 3);
    }
}
