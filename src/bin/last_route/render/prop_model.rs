//! Prop, fixture and hand-tool models, read from Blockbench `.bbmodel` files (props/<set>/*.bbmodel and
//! interactables/*.bbmodel; the converter that made them from the original's Blender exports is
//! tools/props/glb_to_bbmodel.mjs). A model has one texture, so it is one part. A room door's hinge leaves and a hiding
//! spot's door leaf are outliner groups (`DoorHinge`, `DoorHingeRight`, `LockerDoor`), kept apart from the body so they
//! can swing; the door frame's pieces (`DoorFrameTop`, `DoorFrameL`, `DoorFrameR`) are groups too, so door.rs can
//! measure them. Mod packs may still bring a `.glb` prop; `PropModel::parse_glb` is the one place that reads one.

use super::bbmodel::{self, Model};
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
    fn into_part(self, tex: &Texture2D, key: usize) -> Vec<Part> {
        if self.verts.is_empty() {
            return vec![];
        }
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

fn white_vertex(position: Vec3, uv: Vec2, normal: Vec3) -> Vertex {
    Vertex {
        position,
        uv,
        color: [255; 4],
        normal: vec4(normal.x, normal.y, normal.z, 1.0),
    }
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
        let shared = file.as_ref().map(|f| format!("{scope}/{f}"));
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
        let s = m.scale;
        let mut body = Geometry::default();
        let mut leaf = Geometry::default();
        // (hinge name, pivot, its leaf in the hinge's frame)
        let mut hinge_geo: Vec<(String, Vec3, Geometry)> = vec![];
        let mut leaf_x: Vec<(f32, f32)> = vec![];
        let mut locker_pivot = None;
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let mut measure = DoorMeasure::default();
        for (gi, face) in m.faces() {
            let group = gi.map(|i| &m.groups[i]);
            let name = group.map(|g| g.name.as_str()).unwrap_or("");
            let pivot = group.map(|g| g.origin * s).unwrap_or(Vec3::ZERO);
            let hinge = door::closed_hinge_deg(name).and_then(|deg| {
                if !hinge_geo.iter().any(|h| h.0 == name) {
                    measure.hinge(name);
                    hinge_geo.push((name.to_string(), pivot, Geometry::default()));
                    leaf_x.push((f32::MAX, f32::MIN));
                }
                hinge_geo.iter().position(|h| h.0 == name).map(|i| (i, deg))
            });
            let is_locker = name == LOCKER_DOOR_GROUP;
            if is_locker {
                locker_pivot = Some(pivot);
            }
            let geo = match hinge {
                Some((i, _)) => &mut hinge_geo[i].2,
                None if is_locker => &mut leaf,
                None => &mut body,
            };
            let base = geo.verts.len() as u32;
            for (c, uv) in face.corners.iter().zip(&face.uv) {
                // the model as authored (a door's leaves open); the hinge's closed pose is what is measured and bounded
                let w = *c * s;
                let (p, store) = match hinge {
                    Some((i, deg)) => {
                        let store = w - pivot;
                        let p = door::closed_hinge_local(Mat4::from_translation(pivot), deg).transform_point3(store);
                        measure.leaf_vertex(p.y);
                        leaf_x[i] = (leaf_x[i].0.min(p.x), leaf_x[i].1.max(p.x));
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
        }
        if body.verts.is_empty() {
            return None;
        }
        if soft {
            soften(&mut body.verts);
        }
        let hinges = hinge_geo
            .into_iter()
            .zip(leaf_x)
            .map(|((name, pivot, geo), closed_x)| Hinge {
                name,
                base: Mat4::IDENTITY,
                local: Mat4::from_translation(pivot),
                closed_x,
                parts: geo.into_part(&tex, key),
            })
            .collect();
        let locker_door = locker_pivot.map(|pivot| LockerDoor {
            pivot,
            parts: leaf.into_part(&tex, key),
        });
        let door = measure.finish(max.y);
        Some(PropModel {
            parts: body.into_part(&tex, key),
            min,
            max,
            door,
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
        let g = gltf::Gltf::from_slice(bytes).ok()?;
        let mut buffers: Vec<Vec<u8>> = vec![];
        for b in g.buffers() {
            let data = match b.source() {
                gltf::buffer::Source::Bin => g.blob.clone()?,
                gltf::buffer::Source::Uri(u) => sibling(u)?,
            };
            buffers.push(data);
        }
        let mut textures: Vec<Option<Texture2D>> = vec![];
        for img in g.images() {
            let data = match img.source() {
                gltf::image::Source::View { view, .. } => {
                    let b = &buffers[view.buffer().index()];
                    Some(b[view.offset()..view.offset() + view.length()].to_vec())
                }
                gltf::image::Source::Uri { uri, .. } => sibling(uri),
            };
            textures.push(data.map(|d| new_texture(&d)));
        }
        let mut by_tex: HashMap<Option<usize>, Geometry> = HashMap::new();
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let scene = g.default_scene().or_else(|| g.scenes().next())?;
        let mut stack: Vec<(gltf::Node, Mat4)> = scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
        while let Some((node, parent)) = stack.pop() {
            let m = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
            for child in node.children() {
                stack.push((child, m));
            }
            let Some(mesh) = node.mesh() else { continue };
            let nm = Mat3::from_mat4(m).inverse().transpose();
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = prim.reader(|b| buffers.get(b.index()).map(|v| v.as_slice()));
                let Some(pos) = reader.read_positions() else {
                    continue;
                };
                let pos: Vec<[f32; 3]> = pos.collect();
                let normals: Vec<[f32; 3]> = reader
                    .read_normals()
                    .map(|n| n.collect())
                    .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; pos.len()]);
                let uvs: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|t| t.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.0, 0.0]; pos.len()]);
                let cols: Vec<[f32; 4]> = reader
                    .read_colors(0)
                    .map(|c| c.into_rgba_f32().collect())
                    .unwrap_or_else(|| vec![[1.0; 4]; pos.len()]);
                let factor = prim.material().pbr_metallic_roughness().base_color_factor();
                let tex_index = prim
                    .material()
                    .pbr_metallic_roughness()
                    .base_color_texture()
                    .map(|t| t.texture().source().index())
                    .filter(|i| textures.get(*i).map(|t| t.is_some()).unwrap_or(false));
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
                match reader.read_indices() {
                    Some(ix) => entry.idx.extend(ix.into_u32().map(|i| i + base)),
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
    for chunk in idx.chunks(3 * 6000) {
        b.reserve(chunk.len());
        let base = b.verts.len() as u16;
        for (k, &i) in chunk.iter().enumerate() {
            let v = verts[i as usize];
            let n = (nm * v.normal.truncate()).normalize_or_zero();
            let c = [
                (v.color[0] as f32 * tint.r) as u8,
                (v.color[1] as f32 * tint.g) as u8,
                (v.color[2] as f32 * tint.b) as u8,
                v.color[3],
            ];
            b.verts.push(Vertex {
                position: xf.transform_point3(pre(v.position)),
                uv: v.uv,
                color: c,
                normal: vec4(n.x, n.y, n.z, v.normal.w),
            });
            b.idx.push(base + k as u16);
        }
    }
}

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
    fn box_geometry() -> (Vec<Vertex>, Vec<u32>) {
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
}
