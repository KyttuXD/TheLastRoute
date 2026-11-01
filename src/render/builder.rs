//! Collects triangles and splits them into meshes under the 16-bit index limit; helpers for
//! textured quads and boxes.

use macroquad::prelude::*;

pub struct Builder {
    pub verts: Vec<Vertex>,
    pub idx: Vec<u16>,
    done: Vec<Mesh>,
    tex: Option<Texture2D>,
}

const UV: [Vec2; 4] = [vec2(0., 0.), vec2(1., 0.), vec2(1., 1.), vec2(0., 1.)];

impl Builder {
    pub fn new(tex: Option<Texture2D>) -> Builder {
        Builder {
            verts: vec![],
            idx: vec![],
            done: vec![],
            tex,
        }
    }

    /// Starts a new mesh once this one is close to the 16-bit index limit.
    pub fn reserve(&mut self, verts: usize) {
        if self.verts.len() + verts > 64000 {
            self.done.push(Mesh {
                vertices: std::mem::take(&mut self.verts),
                indices: std::mem::take(&mut self.idx),
                texture: self.tex.clone(),
            });
        }
    }

    /// A quad from four corners (TL, TR, BR, BL) with UVs and a shader mode (0 tiled, 1 atlas, 2 unlit).
    pub fn quad(&mut self, c: [Vec3; 4], uv: [Vec2; 4], n: Vec3, col: Color, mode: f32) {
        self.reserve(4);
        let base = self.verts.len() as u16;
        let colb: [u8; 4] = col.into();
        for k in 0..4 {
            self.verts.push(Vertex {
                position: c[k],
                uv: uv[k],
                color: colb,
                normal: vec4(n.x, n.y, n.z, mode),
            });
        }
        self.idx
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A box with an arbitrary orientation basis.
    pub fn cube_basis(&mut self, center: Vec3, size: Vec3, basis: Mat3, col: Color, mode: f32) {
        let h = size * 0.5;
        let p = |x: f32, y: f32, z: f32| center + basis * vec3(x * h.x, y * h.y, z * h.z);
        let faces: [([Vec3; 4], Vec3); 6] = [
            (
                [
                    p(-1., 1., 1.),
                    p(1., 1., 1.),
                    p(1., -1., 1.),
                    p(-1., -1., 1.),
                ],
                vec3(0., 0., 1.),
            ),
            (
                [
                    p(1., 1., -1.),
                    p(-1., 1., -1.),
                    p(-1., -1., -1.),
                    p(1., -1., -1.),
                ],
                vec3(0., 0., -1.),
            ),
            (
                [
                    p(1., 1., 1.),
                    p(1., 1., -1.),
                    p(1., -1., -1.),
                    p(1., -1., 1.),
                ],
                vec3(1., 0., 0.),
            ),
            (
                [
                    p(-1., 1., -1.),
                    p(-1., 1., 1.),
                    p(-1., -1., 1.),
                    p(-1., -1., -1.),
                ],
                vec3(-1., 0., 0.),
            ),
            (
                [
                    p(-1., 1., -1.),
                    p(1., 1., -1.),
                    p(1., 1., 1.),
                    p(-1., 1., 1.),
                ],
                vec3(0., 1., 0.),
            ),
            (
                [
                    p(-1., -1., 1.),
                    p(1., -1., 1.),
                    p(1., -1., -1.),
                    p(-1., -1., -1.),
                ],
                vec3(0., -1., 0.),
            ),
        ];
        for (c, n) in faces {
            self.quad(c, UV, basis * n, col, mode);
        }
    }

    /// A box turned by `yaw` around its vertical centre line.
    pub fn cube(&mut self, center: Vec3, size: Vec3, yaw: f32, col: Color, mode: f32) {
        self.cube_basis(center, size, Mat3::from_rotation_y(yaw), col, mode);
    }

    pub fn finish(mut self) -> Vec<Mesh> {
        if !self.verts.is_empty() {
            self.done.push(Mesh {
                vertices: self.verts,
                indices: self.idx,
                texture: self.tex,
            });
        }
        self.done
    }

    /// Draws what was collected and empties the builder (for per-frame batches).
    pub fn draw_and_clear(&mut self) {
        for m in std::mem::take(&mut self.done) {
            draw_mesh(&m);
        }
        if !self.verts.is_empty() {
            let m = Mesh {
                vertices: std::mem::take(&mut self.verts),
                indices: std::mem::take(&mut self.idx),
                texture: self.tex.clone(),
            };
            draw_mesh(&m);
        }
    }
}

pub fn white_texture() -> Texture2D {
    let t = Texture2D::from_rgba8(1, 1, &[255, 255, 255, 255]);
    t.set_filter(FilterMode::Nearest);
    t
}
