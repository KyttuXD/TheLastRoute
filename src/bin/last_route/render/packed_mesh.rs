//! The floor's meshes as they are kept between frames: 18 bytes a vertex in place of macroquad's 40. A window holds
//! about two million of these vertices (docs/RAM_PASS.md) and draws a fraction of them in any one frame.
//!
//! Unpacking costs more than twice a plain copy (1.6 ns a vertex against 0.7), so a mesh that is being drawn keeps its
//! unpacked vertices too (`Hot`) and is drawn from them exactly as before; a mesh the cull has dropped for
//! `HOT_SECONDS` gives them back. The memory saved is on everything out of sight: the tiles behind the fog and the
//! camera, the kept parts of other windows, the neighbour levels.
//!
//! What is stored, and what it costs the picture:
//! - position: 16 bits an axis on a fixed grid of 1/2048 m (under half a millimetre; at most a quarter of one off).
//!   The grid is the same for every mesh, so a corner shared by two meshes still lands on one point and no gap opens.
//!   A mesh wider than 32 m takes a coarser grid (none is built today: tiles are 15 m, the widest about 17 m).
//! - UV: 16 bits each on a power-of-two step sized to the mesh's UV span (1/32768 for spans up to 2).
//! - colour: as it was.
//! - normal: 9 bits an axis (under a quarter of a degree off; exact for axis-aligned faces); shader mode: 5 bits.
//!
//! Moving a mesh (a window recentre, a level's height offset) changes its `origin` only.

use macroquad::prelude::*;

/// The position grid, metres: 2^-11.
pub const POS_STEP: f32 = 1.0 / 2048.0;
/// The finest UV step: 2^-15.
const UV_STEP_MIN: f32 = 1.0 / 32768.0;
/// Seconds a mesh keeps its unpacked vertices after it was last drawn. Long enough that looking round a room does not
/// unpack the same tiles again and again; short enough that what is left behind is given back within a few steps.
pub const HOT_SECONDS: f64 = 1.5;
/// The largest stored offset. One below the 16-bit limit, since the origin is rounded down to the grid.
const SPAN_MAX: f32 = 65534.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
struct PackedVertex {
    pos: [u16; 3],
    uv: [u16; 2],
    color: [u8; 4],
    /// Bits 0-8, 9-17, 18-26: the normal's x, y, z as (value * 255 + 255); bits 27-31: the shader mode. Little-endian.
    normal_mode: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct PackedMesh {
    /// Where stored position (0, 0, 0) stands. A multiple of `step` when packed; moves add to it.
    pub origin: Vec3,
    step: f32,
    uv_origin: Vec2,
    uv_step: f32,
    vertices: Vec<PackedVertex>,
    pub indices: Vec<u16>,
    pub texture: Option<Texture2D>,
    /// Whether any vertex is in a blending shader mode (`mesh_batch::is_blended`).
    pub blended: bool,
    /// The unpacked vertices while the mesh is being drawn.
    hot: std::cell::RefCell<Option<Hot>>,
}

#[derive(Clone, Debug)]
struct Hot {
    vertices: Vec<Vertex>,
    /// The mesh's origin the vertices were unpacked for; a moved mesh is unpacked again.
    origin: Vec3,
    /// The draw clock at the last draw.
    drawn: f64,
}

/// The smallest power-of-two multiple of `min_step` that fits `span` into 16 bits.
fn step_for(span: f32, min_step: f32) -> f32 {
    let mut step = min_step;
    while span / step > SPAN_MAX && step < 1.0e6 {
        step *= 2.0;
    }
    step
}

fn pack_normal_mode(n: Vec4) -> [u8; 4] {
    let mut v = vec3(n.x, n.y, n.z);
    if !v.is_finite() {
        v = Vec3::ZERO;
    }
    let big = v.abs().max_element();
    if big > 1.0 {
        v /= big;
    }
    let q = |c: f32| ((c * 255.0).round() as i32 + 255).clamp(0, 510) as u32;
    let mode = if n.w.is_finite() { (n.w.round() as i32).clamp(0, 31) as u32 } else { 0 };
    (q(v.x) | q(v.y) << 9 | q(v.z) << 18 | mode << 27).to_le_bytes()
}

fn unpack_normal_mode(b: [u8; 4]) -> Vec4 {
    let w = u32::from_le_bytes(b);
    // a division, not a table: the compiler turns the loop over a mesh into vector code, which a table lookup stops
    // (measured: 1.6 ns a vertex against 2.0)
    let c = |shift: u32| (((w >> shift) & 511) as i32 - 255) as f32 / 255.0;
    vec4(c(0), c(9), c(18), (w >> 27) as f32)
}

impl PackedMesh {
    pub fn pack(mesh: Mesh) -> PackedMesh {
        let Mesh { vertices, indices, texture } = mesh;
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let (mut uv_lo, mut uv_hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        let mut blended = false;
        for v in &vertices {
            lo = lo.min(v.position);
            hi = hi.max(v.position);
            uv_lo = uv_lo.min(v.uv);
            uv_hi = uv_hi.max(v.uv);
            blended |= v.normal.w > 2.5;
        }
        if vertices.is_empty() || !lo.is_finite() || !hi.is_finite() || !uv_lo.is_finite() || !uv_hi.is_finite() {
            lo = Vec3::ZERO;
            hi = Vec3::ZERO;
            uv_lo = Vec2::ZERO;
            uv_hi = Vec2::ZERO;
        }
        let step = step_for((hi - lo).max_element(), POS_STEP);
        let uv_step = step_for((uv_hi - uv_lo).max_element(), UV_STEP_MIN);
        let origin = (lo / step).floor() * step;
        let uv_origin = (uv_lo / uv_step).floor() * uv_step;
        let q = |value: f32, from: f32, step: f32| ((value - from) / step).round().clamp(0.0, 65535.0) as u16;
        let packed = vertices
            .iter()
            .map(|v| PackedVertex {
                pos: [q(v.position.x, origin.x, step), q(v.position.y, origin.y, step), q(v.position.z, origin.z, step)],
                uv: [q(v.uv.x, uv_origin.x, uv_step), q(v.uv.y, uv_origin.y, uv_step)],
                color: v.color,
                normal_mode: pack_normal_mode(v.normal),
            })
            .collect();
        PackedMesh { origin, step, uv_origin, uv_step, vertices: packed, indices, texture, blended, hot: Default::default() }
    }

    pub fn pack_all(meshes: Vec<Mesh>) -> Vec<PackedMesh> {
        meshes.into_iter().map(PackedMesh::pack).collect()
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    fn unpack(&self, v: &PackedVertex) -> Vertex {
        Vertex {
            position: self.origin + vec3(v.pos[0] as f32, v.pos[1] as f32, v.pos[2] as f32) * self.step,
            uv: self.uv_origin + vec2(v.uv[0] as f32, v.uv[1] as f32) * self.uv_step,
            color: v.color,
            normal: unpack_normal_mode(v.normal_mode),
        }
    }

    /// Vertex `i` as macroquad draws it.
    pub fn vertex(&self, i: usize) -> Vertex {
        self.unpack(&self.vertices[i])
    }

    pub fn vertices(&self) -> impl Iterator<Item = Vertex> + '_ {
        self.vertices.iter().map(|v| self.unpack(v))
    }

    /// The box over the vertices (min, max); an empty mesh gives an inverted box, as a fold over no vertices does.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        if self.vertices.is_empty() {
            return (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        }
        let (mut lo, mut hi) = ([u16::MAX; 3], [0u16; 3]);
        for v in &self.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v.pos[k]);
                hi[k] = hi[k].max(v.pos[k]);
            }
        }
        let at = |p: [u16; 3]| self.origin + vec3(p[0] as f32, p[1] as f32, p[2] as f32) * self.step;
        (at(lo), at(hi))
    }

    /// Drops the unpacked copy if the mesh has not been drawn for `HOT_SECONDS` (`now` in seconds, the draw clock).
    pub fn cool(&self, now: f64) {
        let mut hot = self.hot.borrow_mut();
        if hot.as_ref().is_some_and(|h| now - h.drawn > HOT_SECONDS) {
            *hot = None;
        }
    }

    /// Drops the unpacked copy now (a mesh that left the drawn set).
    pub fn chill(&mut self) {
        *self.hot.get_mut() = None;
    }

    /// Whether the mesh holds an unpacked copy.
    pub fn is_hot(&self) -> bool {
        self.hot.borrow().is_some()
    }

    /// The unpacked vertices as they are drawn at `now`: the kept copy, unpacked first when there is none or the
    /// mesh has moved since.
    fn with_hot<R>(&self, now: f64, f: impl FnOnce(&[Vertex]) -> R) -> R {
        let mut hot = self.hot.borrow_mut();
        match hot.as_mut() {
            Some(h) if h.origin == self.origin => h.drawn = now,
            _ => *hot = Some(Hot { vertices: self.vertices().collect(), origin: self.origin, drawn: now }),
        }
        f(&hot.as_ref().expect("set above").vertices)
    }

    /// The mesh as macroquad's own type (tests, tools).
    #[cfg(test)]
    pub fn to_mesh(&self) -> Mesh {
        Mesh { vertices: self.vertices().collect(), indices: self.indices.clone(), texture: self.texture.clone() }
    }
}

/// Draws a packed mesh as `draw_mesh` draws its unpacked form, and counts it. `now` is the draw clock in seconds
/// (`get_time()`): the mesh keeps its unpacked vertices from this draw on, until `cool` finds it idle.
pub fn draw_counted(m: &PackedMesh, now: f64) {
    super::draw_stats::note(&m.texture, m.vertices.len(), m.indices.len());
    m.with_hot(now, |vertices| {
        let gl = unsafe { get_internal_gl() }.quad_gl;
        gl.texture(m.texture.as_ref());
        gl.draw_mode(DrawMode::Triangles);
        gl.geometry(vertices, &m.indices);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vert(p: Vec3, uv: Vec2, n: Vec4) -> Vertex {
        Vertex { position: p, uv, color: [10, 20, 30, 40], normal: n }
    }

    fn mesh(vertices: Vec<Vertex>) -> Mesh {
        let n = vertices.len() as u16;
        Mesh { vertices, indices: (0..n).collect(), texture: None }
    }

    #[test]
    fn the_vertex_is_eighteen_bytes() {
        assert_eq!(std::mem::size_of::<PackedVertex>(), 18);
        assert_eq!(std::mem::size_of::<Vertex>(), 40, "macroquad's vertex, which the packed one replaces");
    }

    #[test]
    fn grid_values_come_back_exactly() {
        let n = vec4(0.0, -1.0, 0.0, 1.0);
        let m = PackedMesh::pack(mesh(vec![
            vert(vec3(-3.5, 0.0, 120.25), vec2(0.0, 1.0), n),
            vert(vec3(11.5, 3.0, 135.0), vec2(0.5, 0.125), n),
            vert(vec3(4.0, -2.75, 127.0), vec2(2.0, 0.0), vec4(1.0, 0.0, 0.0, 4.0)),
        ]));
        assert_eq!(m.vertex(0).position, vec3(-3.5, 0.0, 120.25));
        assert_eq!(m.vertex(1).position, vec3(11.5, 3.0, 135.0));
        assert_eq!(m.vertex(2).position, vec3(4.0, -2.75, 127.0));
        assert_eq!(m.vertex(0).uv, vec2(0.0, 1.0));
        assert_eq!(m.vertex(1).uv, vec2(0.5, 0.125));
        assert_eq!(m.vertex(0).normal, n);
        assert_eq!(m.vertex(2).normal, vec4(1.0, 0.0, 0.0, 4.0));
        assert_eq!(m.vertex(1).color, [10, 20, 30, 40]);
        assert!(m.blended, "mode 4 blends");
        assert_eq!(m.bounds(), (vec3(-3.5, -2.75, 120.25), vec3(11.5, 3.0, 135.0)));
    }

    #[test]
    fn any_position_is_within_a_quarter_millimetre_and_any_normal_within_a_quarter_degree() {
        let mut vs = vec![];
        let mut s = 12345u32;
        let mut rnd = move || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as f32 / 16777216.0
        };
        for _ in 0..2000 {
            let p = vec3(rnd() * 17.0 + 90.0, rnd() * 6.5 - 8.0, rnd() * 17.0 - 60.0);
            let n = (vec3(rnd(), rnd(), rnd()) - 0.5).normalize();
            vs.push(vert(p, vec2(rnd() * 3.0 - 1.0, rnd()), vec4(n.x, n.y, n.z, 0.0)));
        }
        let m = PackedMesh::pack(mesh(vs.clone()));
        for (a, b) in vs.iter().zip(m.vertices()) {
            assert!((a.position - b.position).abs().max_element() <= POS_STEP * 0.5 + 1e-5, "{:?} {:?}", a.position, b.position);
            assert!((a.uv - b.uv).abs().max_element() <= 1.0 / 16384.0);
            let (na, nb) = (a.normal.truncate(), b.normal.truncate().normalize());
            assert!(na.dot(nb) > (0.25f32).to_radians().cos(), "{na:?} {nb:?}");
        }
    }

    #[test]
    fn a_point_shared_by_two_meshes_lands_on_one_point() {
        // the same off-grid corner in two meshes with different boxes
        let p = vec3(14.99913, 1.23457, 30.00071);
        let n = vec4(0.0, 1.0, 0.0, 0.0);
        let a = PackedMesh::pack(mesh(vec![vert(vec3(0.3, 0.0, 15.2), Vec2::ZERO, n), vert(p, Vec2::ZERO, n)]));
        let b = PackedMesh::pack(mesh(vec![vert(p, Vec2::ZERO, n), vert(vec3(29.7, 3.1, 44.9), Vec2::ZERO, n)]));
        assert_eq!(a.vertex(1).position, b.vertex(0).position);
    }

    #[test]
    fn moving_the_origin_moves_every_vertex() {
        let n = vec4(0.0, 1.0, 0.0, 0.0);
        let mut m = PackedMesh::pack(mesh(vec![vert(vec3(1.0, 0.0, 2.0), Vec2::ZERO, n), vert(vec3(4.0, 3.0, 2.0), Vec2::ZERO, n)]));
        m.origin += vec3(-45.0, 5.0, 90.0);
        assert_eq!(m.vertex(0).position, vec3(-44.0, 5.0, 92.0));
        assert_eq!(m.bounds(), (vec3(-44.0, 5.0, 92.0), vec3(-41.0, 8.0, 92.0)));
    }

    #[test]
    fn a_mesh_too_wide_for_the_grid_takes_a_coarser_one_and_a_wide_uv_span_a_coarser_step() {
        let n = vec4(0.0, 1.0, 0.0, 0.0);
        let m = PackedMesh::pack(mesh(vec![vert(vec3(-50.0, 0.0, 0.0), vec2(-3.0, 0.0), n), vert(vec3(50.0, 0.0, 0.0), vec2(187.0, 1.0), n)]));
        assert_eq!(m.vertex(0).position.x, -50.0);
        assert_eq!(m.vertex(1).position.x, 50.0);
        assert_eq!(m.vertex(0).uv, vec2(-3.0, 0.0));
        assert_eq!(m.vertex(1).uv, vec2(187.0, 1.0));
    }

    #[test]
    fn an_empty_mesh_packs_to_an_empty_mesh_with_an_inverted_box() {
        let m = PackedMesh::pack(Mesh { vertices: vec![], indices: vec![], texture: None });
        assert_eq!(m.vertex_count(), 0);
        let (lo, hi) = m.bounds();
        assert!(lo.x > hi.x);
    }

    #[test]
    fn odd_normals_and_modes_do_not_break_the_pack() {
        let m = PackedMesh::pack(mesh(vec![
            vert(Vec3::ZERO, Vec2::ZERO, vec4(3.0, 0.0, 0.0, 2.0)),
            vert(Vec3::ONE, Vec2::ONE, vec4(f32::NAN, 0.0, 0.0, f32::NAN)),
        ]));
        assert_eq!(m.vertex(0).normal, vec4(1.0, 0.0, 0.0, 2.0));
        assert_eq!(m.vertex(1).normal, vec4(0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn a_drawn_mesh_keeps_its_unpacked_vertices_until_it_has_been_idle() {
        let n = vec4(0.0, 1.0, 0.0, 0.0);
        let mut m = PackedMesh::pack(mesh(vec![vert(vec3(1.0, 0.0, 2.0), Vec2::ZERO, n), vert(vec3(4.0, 3.0, 2.0), Vec2::ZERO, n)]));
        assert!(!m.is_hot(), "a mesh that was never drawn holds the packed form only");
        assert_eq!(m.with_hot(10.0, |v| v[1].position), vec3(4.0, 3.0, 2.0));
        assert!(m.is_hot());
        m.cool(10.0 + HOT_SECONDS * 0.5);
        assert!(m.is_hot(), "culled for a moment: kept");
        m.with_hot(11.0, |_| ());
        m.cool(10.0 + HOT_SECONDS + 0.5);
        assert!(m.is_hot(), "drawn again since: the clock started over");
        m.cool(11.0 + HOT_SECONDS + 0.1);
        assert!(!m.is_hot(), "idle past the limit: given back");
        // a moved mesh is unpacked again for its new place
        m.with_hot(20.0, |_| ());
        m.origin += vec3(-45.0, 0.0, 0.0);
        assert_eq!(m.with_hot(20.1, |v| v[0].position), vec3(-44.0, 0.0, 2.0));
        m.chill();
        assert!(!m.is_hot());
    }
}
