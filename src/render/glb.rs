//! The original's authored Blender models (.glb): props, fixtures and hand tools. Meshes keep
//! their node transforms, base colour textures (nearest-sampled, repeating), base colour
//! factors and vertex colours, grouped into one part per texture.

use super::builder::Builder;
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
        let mut by_tex: HashMap<Option<usize>, (Vec<Vertex>, Vec<u32>)> = HashMap::new();
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let scene = g.default_scene().or_else(|| g.scenes().next())?;
        let mut stack: Vec<(gltf::Node, Mat4)> =
            scene.nodes().map(|n| (n, Mat4::IDENTITY)).collect();
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
                let mat = prim.material();
                let pbr = mat.pbr_metallic_roughness();
                let factor = pbr.base_color_factor();
                let tex_index = pbr
                    .base_color_texture()
                    .map(|t| t.texture().source().index())
                    .filter(|i| textures.get(*i).map(|t| t.is_some()).unwrap_or(false));
                let entry = by_tex.entry(tex_index).or_default();
                let base = entry.0.len() as u32;
                for i in 0..pos.len() {
                    let p = m.transform_point3(Vec3::from(pos[i]));
                    min = min.min(p);
                    max = max.max(p);
                    let n = (nm * Vec3::from(normals[i])).normalize_or_zero();
                    let c = [
                        (factor[0] * cols[i][0] * 255.0).clamp(0.0, 255.0) as u8,
                        (factor[1] * cols[i][1] * 255.0).clamp(0.0, 255.0) as u8,
                        (factor[2] * cols[i][2] * 255.0).clamp(0.0, 255.0) as u8,
                        (factor[3] * cols[i][3] * 255.0).clamp(0.0, 255.0) as u8,
                    ];
                    entry.0.push(Vertex {
                        position: p,
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
        let parts = by_tex
            .into_iter()
            .map(|(t, (verts, idx))| Part {
                verts,
                idx,
                tex: t
                    .and_then(|i| textures[i].clone())
                    .unwrap_or_else(|| white.clone()),
                key: NEXT_KEY.fetch_add(1, Ordering::Relaxed),
            })
            .collect();
        Some(GlbModel { parts, min, max })
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    /// Adds the model, placed by `xf`, to per-texture builders (static world batching).
    pub fn add_to(&self, builders: &mut HashMap<usize, Builder>, xf: Mat4, tint: Color) {
        let nm = Mat3::from_mat4(xf).inverse().transpose();
        for p in &self.parts {
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

    /// Draws the model right now (dynamic objects: pickups, fixtures, the held tool).
    pub fn draw(&self, xf: Mat4, tint: Color) {
        let mut builders = HashMap::new();
        self.add_to(&mut builders, xf, tint);
        for (_, mut b) in builders {
            b.draw_and_clear();
        }
    }
}
