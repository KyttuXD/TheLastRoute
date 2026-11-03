//! The original's authored Blender models (.glb): props, fixtures and hand tools. Meshes keep
//! their node transforms, base colour textures (nearest-sampled, repeating), base colour
//! factors and vertex colours, grouped into one part per texture.

use super::builder::Builder;
use super::door::{self, DoorMeasure};
use macroquad::prelude::*;
use std::collections::HashMap;
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

pub struct GlbModel {
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
    pub parts: Vec<Part>,
}

/// The door of a hiding spot: its geometry is relative to `pivot`, the hinge line on the model's left edge.
pub struct LockerDoor {
    pub pivot: Vec3,
    pub parts: Vec<Part>,
}

/// The nodes of the hiding spot model that swing with its door (the hinge blocks stay on the body).
fn locker_door_part(name: &str) -> bool {
    matches!(
        name,
        "BlockLatch"
            | "DoorLowerMass"
            | "DoorRail_L"
            | "DoorRail_R"
            | "DoorTopMass"
            | "Handle"
            | "HandlePlate"
            | "LowerInset"
            | "LowerPanel"
            | "NumberPlate"
            | "SightCrackLeftRail"
            | "SightCrackRightRail"
            | "SightSlat_1"
            | "SightSlat_2"
            | "SightSlat_3"
    )
}

/// The node whose position is the hiding spot door's hinge line.
const LOCKER_HINGE_NODE: &str = "BlockHinge_1";

type Geometry = HashMap<Option<usize>, (Vec<Vertex>, Vec<u32>)>;

enum Dest {
    Body,
    Hinge(usize),
    Leaf,
}

fn repeat(t: &Texture2D) {
    use macroquad::miniquad::TextureWrap;
    let id = t.raw_miniquad_id();
    unsafe {
        get_internal_gl().quad_context.texture_set_wrap(
            id,
            TextureWrap::Repeat,
            TextureWrap::Repeat,
        );
    }
}

impl GlbModel {
    /// `sibling` reads a file referenced by URI next to the model (external images or buffers).
    pub fn parse(
        bytes: &[u8],
        sibling: &dyn Fn(&str) -> Option<Vec<u8>>,
        white: &Texture2D,
    ) -> Option<GlbModel> {
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
            textures.push(data.map(|d| {
                let t = Texture2D::from_file_with_format(&d, None);
                t.set_filter(FilterMode::Nearest);
                repeat(&t);
                t
            }));
        }
        let mut by_tex: Geometry = HashMap::new();
        let mut hinges: Vec<Hinge> = vec![];
        let mut hinge_geo: Vec<Geometry> = vec![];
        let mut leaf_geo: Geometry = HashMap::new();
        let locker_pivot = g
            .nodes()
            .find(|n| n.name() == Some(LOCKER_HINGE_NODE))
            .map(|n| Vec3::from(n.transform().decomposed().0));
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let scene = g.default_scene().or_else(|| g.scenes().next())?;
        let mut measure = DoorMeasure::default();
        // (node, parent transform, the hinge it hangs from and its transform in that hinge's frame)
        type Stacked<'a> = (gltf::Node<'a>, Mat4, Option<(usize, Mat4)>);
        let mut stack: Vec<Stacked> = scene.nodes().map(|n| (n, Mat4::IDENTITY, None)).collect();
        while let Some((node, parent, in_hinge)) = stack.pop() {
            let name = node.name().unwrap_or("");
            let authored = Mat4::from_cols_array_2d(&node.transform().matrix());
            let mut local = authored;
            // a door's hinges are drawn closed, as Godot leaves them by default (door.rs); the static batch holds
            // them shut, while the leaf geometry is kept in the hinge's frame so `draw_hinges` can turn it
            let mut rel = in_hinge.map(|(i, r)| (i, r * authored));
            if let (Some(deg), None) = (door::closed_hinge_deg(name), in_hinge) {
                local = door::closed_hinge_local(authored, deg);
                measure.hinge();
                hinges.push(Hinge {
                    name: name.to_string(),
                    base: parent,
                    local: authored,
                    parts: vec![],
                });
                hinge_geo.push(HashMap::new());
                rel = Some((hinges.len() - 1, Mat4::IDENTITY));
            }
            let m = parent * local;
            for child in node.children() {
                stack.push((child, m, rel));
            }
            let Some(mesh) = node.mesh() else { continue };
            let header = name == "DoorFrameTop";
            let jamb = name == "DoorFrameL" || name == "DoorFrameR";
            let under = rel.is_some();
            let (dest, store) = match (rel, locker_pivot) {
                (Some((i, r)), _) => (Dest::Hinge(i), r),
                (None, Some(_)) if locker_door_part(name) => (Dest::Leaf, m),
                _ => (Dest::Body, m),
            };
            let origin = match dest {
                Dest::Leaf => locker_pivot.unwrap_or(Vec3::ZERO),
                _ => Vec3::ZERO,
            };
            let nm = Mat3::from_mat4(store).inverse().transpose();
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
                let mat = prim.material();
                let pbr = mat.pbr_metallic_roughness();
                let factor = pbr.base_color_factor();
                let tex_index = pbr
                    .base_color_texture()
                    .map(|t| t.texture().source().index())
                    .filter(|i| textures.get(*i).map(|t| t.is_some()).unwrap_or(false));
                let geo = match dest {
                    Dest::Body => &mut by_tex,
                    Dest::Hinge(i) => &mut hinge_geo[i],
                    Dest::Leaf => &mut leaf_geo,
                };
                let entry = geo.entry(tex_index).or_default();
                let base = entry.0.len() as u32;
                for i in 0..pos.len() {
                    let p = m.transform_point3(Vec3::from(pos[i]));
                    min = min.min(p);
                    max = max.max(p);
                    if under {
                        measure.leaf_vertex(p.y);
                    }
                    if header {
                        measure.header_vertex(p.y);
                    }
                    if jamb {
                        measure.jamb_vertex(p.x);
                    }
                    let n = (nm * Vec3::from(normals[i])).normalize_or_zero();
                    let c = [
                        (factor[0] * cols[i][0] * 255.0).clamp(0.0, 255.0) as u8,
                        (factor[1] * cols[i][1] * 255.0).clamp(0.0, 255.0) as u8,
                        (factor[2] * cols[i][2] * 255.0).clamp(0.0, 255.0) as u8,
                        (factor[3] * cols[i][3] * 255.0).clamp(0.0, 255.0) as u8,
                    ];
                    entry.0.push(Vertex {
                        position: store.transform_point3(Vec3::from(pos[i])) - origin,
                        uv: Vec2::from(uvs[i]),
                        color: c,
                        normal: vec4(n.x, n.y, n.z, 1.0),
                    });
                }
                match reader.read_indices() {
                    Some(ix) => entry.1.extend(ix.into_u32().map(|i| i + base)),
                    None => entry.1.extend((0..pos.len() as u32).map(|i| i + base)),
                }
            }
        }
        if by_tex.is_empty() {
            return None;
        }
        let into_parts = |geo: Geometry| -> Vec<Part> {
            geo.into_iter()
                .map(|(t, (verts, idx))| Part {
                    verts,
                    idx,
                    tex: t
                        .and_then(|i| textures[i].clone())
                        .unwrap_or_else(|| white.clone()),
                    key: NEXT_KEY.fetch_add(1, Ordering::Relaxed),
                })
                .collect()
        };
        for (h, geo) in hinges.iter_mut().zip(hinge_geo) {
            h.parts = into_parts(geo);
        }
        let locker_door = locker_pivot.map(|pivot| LockerDoor {
            pivot,
            parts: into_parts(leaf_geo),
        });
        let parts = into_parts(by_tex);
        let door = measure.finish(max.y);
        Some(GlbModel {
            parts,
            min,
            max,
            door,
            hinges,
            locker_door,
        })
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// Adds the model, placed by `xf`, to per-texture builders (static world batching).
    pub fn add_to(&self, builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color) {
        add_parts(&self.parts, builders, xf, tint);
    }

    /// Adds a room door's hinge leaves to `builders`, the door placed by `xf` and swung to `open` (0 shut, 1 open):
    /// each leaf turns about its hinge to `door::hinge_deg`, from the same hinge frame the static batch closes.
    pub fn add_hinges(&self, builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color, open: f32) {
        for h in &self.hinges {
            let deg = door::hinge_deg(&h.name, open).unwrap_or(0.0);
            add_parts(&h.parts, builders, xf * h.base * door::closed_hinge_local(h.local, deg), tint);
        }
    }

    /// Adds a hiding spot's door to `builders`, swung `angle` radians open about its hinge line (toward the front).
    pub fn add_locker_door(&self, builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color, angle: f32) {
        if let Some(d) = &self.locker_door {
            let swing = xf * Mat4::from_translation(d.pivot) * Mat4::from_rotation_y(-angle);
            add_parts(&d.parts, builders, swing, tint);
        }
    }

    /// Draws the model right now (dynamic objects: pickups, fixtures, the held tool).
    pub fn draw(&self, xf: Mat4, tint: Color) {
        let mut builders = HashMap::new();
        self.add_to(&mut builders, xf, tint);
        draw_builders(builders);
    }
}

fn add_parts(parts: &[Part], builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color) {
    let nm = Mat3::from_mat4(xf).inverse().transpose();
    for p in parts {
        let b = builders
            .entry(p.key)
            .or_insert_with(|| Builder::new(Some(p.tex.clone())));
        for chunk in p.idx.chunks(3 * 6000) {
            b.reserve(chunk.len());
            let base = b.verts.len() as u16;
            for (k, &i) in chunk.iter().enumerate() {
                let v = p.verts[i as usize];
                let n = (nm * v.normal.truncate()).normalize_or_zero();
                let c = [
                    (v.color[0] as f32 * tint.r) as u8,
                    (v.color[1] as f32 * tint.g) as u8,
                    (v.color[2] as f32 * tint.b) as u8,
                    v.color[3],
                ];
                b.verts.push(Vertex {
                    position: xf.transform_point3(v.position),
                    uv: v.uv,
                    color: c,
                    normal: vec4(n.x, n.y, n.z, v.normal.w),
                });
                b.idx.push(base + k as u16);
            }
        }
    }
}

/// Draws the builders' triangles now, one call per texture.
pub fn draw_builders(builders: HashMap<usize, Builder>) {
    for (_, mut b) in builders {
        b.draw_and_clear();
    }
}
