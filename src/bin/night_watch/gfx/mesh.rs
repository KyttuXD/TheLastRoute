#![allow(dead_code)]
//! Shaded cubes, groups, the world builder and the baked world. Port of `NightWatchArt.box_mesh` / `box`.
//! Geometry and batching are pure (`cube_geometry`, `build_batches`); only `WorldBuilder::bake` and drawing touch GL.

use super::art::{self, Textures, HAZ_PERIOD_M, HAZ_TILE, TEX, TILE};
use super::shader::{LightSet, LitShader, EMISSIVE_MODE, LIT_MODE};
use macroquad::prelude::*;
use std::collections::{BTreeMap, HashMap};

/// Batches are cut at this many vertices / indices.
pub const MAX_BATCH_VERTS: usize = 10_000;
pub const MAX_BATCH_INDICES: usize = 30_000;

#[derive(Clone, Debug)]
pub struct Cube {
    pub name: String,
    /// Material kind; empty means "from the name" (resolved by `Group::add`).
    pub kind: String,
    pub tint: [f32; 3],
    /// Centre, local to the group.
    pub pos: Vec3,
    /// Full extents.
    pub size: Vec3,
    pub emissive: bool,
    /// Axis letters ("xyz" subset) that carry hazard tape, or "".
    pub hazard: String,
    pub ao: f32,
    pub shade: f32,
    pub solid: bool,
    pub rot_y: f32,
}

impl Cube {
    pub fn new(name: &str, size: Vec3, pos: Vec3) -> Cube {
        Cube {
            name: name.to_string(),
            kind: String::new(),
            tint: [1.0, 1.0, 1.0],
            pos,
            size,
            emissive: false,
            hazard: String::new(),
            ao: 0.72,
            shade: 1.0,
            solid: false,
            rot_y: 0.0,
        }
    }
    pub fn kind(mut self, k: &str) -> Cube {
        self.kind = k.to_string();
        self
    }
    pub fn tint(mut self, t: [f32; 3]) -> Cube {
        self.tint = t;
        self
    }
    pub fn solid(mut self) -> Cube {
        self.solid = true;
        self
    }
    pub fn emissive(mut self) -> Cube {
        self.emissive = true;
        self
    }
    pub fn hazard(mut self, axes: &str) -> Cube {
        self.hazard = axes.to_string();
        self
    }
    pub fn ao(mut self, v: f32) -> Cube {
        self.ao = v;
        self
    }
    pub fn rot_y(mut self, r: f32) -> Cube {
        self.rot_y = r;
        self
    }

    // In-place twins of the builder methods, so `group.bx(..)` (which returns `&mut Cube`) can chain.
    pub fn set_kind(&mut self, k: &str) -> &mut Cube {
        self.kind = k.to_string();
        self
    }
    pub fn set_tint(&mut self, t: [f32; 3]) -> &mut Cube {
        self.tint = t;
        self
    }
    pub fn set_solid(&mut self) -> &mut Cube {
        self.solid = true;
        self
    }
    pub fn set_emissive(&mut self) -> &mut Cube {
        self.emissive = true;
        self
    }
    pub fn set_hazard(&mut self, axes: &str) -> &mut Cube {
        self.hazard = axes.to_string();
        self
    }
    pub fn set_ao(&mut self, v: f32) -> &mut Cube {
        self.ao = v;
        self
    }
    pub fn set_shade(&mut self, v: f32) -> &mut Cube {
        self.shade = v;
        self
    }
    pub fn set_rot_y(&mut self, r: f32) -> &mut Cube {
        self.rot_y = r;
        self
    }
}

#[derive(Clone)]
pub struct Group {
    pub name: String,
    pub origin: Vec3,
    pub cubes: Vec<Cube>,
}

impl Group {
    pub fn new(name: &str, origin: Vec3) -> Group {
        Group { name: name.to_string(), origin, cubes: Vec::new() }
    }

    /// Kind falls back to `kind_for(name)`, emissive to `is_emissive_name(name)`.
    pub fn add(&mut self, mut c: Cube) -> &mut Group {
        if c.kind.is_empty() {
            c.kind = art::kind_for(&c.name).to_string();
        }
        if !c.emissive && art::is_emissive_name(&c.name) {
            c.emissive = true;
        }
        self.cubes.push(c);
        self
    }

    /// Shortcut: add a cube and hand it back for `set_tint` / `set_kind` chaining.
    pub fn bx(&mut self, name: &str, size: [f32; 3], pos: [f32; 3]) -> &mut Cube {
        let c = Cube::new(name, vec3(size[0], size[1], size[2]), vec3(pos[0], pos[1], pos[2]));
        self.add(c);
        self.cubes.last_mut().expect("cube just added")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub fn from_centre(c: Vec3, size: Vec3) -> Aabb {
        let h = size * 0.5;
        Aabb { min: c - h, max: c + h }
    }

    /// True when a disc of radius `r` at (x, z) overlaps the box footprint (square expansion).
    pub fn contains_xz(&self, x: f32, z: f32, r: f32) -> bool {
        x > self.min.x - r && x < self.max.x + r && z > self.min.z - r && z < self.max.z + r
    }

    /// Slab ray test: distance along `d` (in units of |d|) to the first hit, 0 when the origin is inside.
    pub fn ray(&self, o: Vec3, d: Vec3) -> Option<f32> {
        let mut t0 = 0.0f32;
        let mut t1 = f32::INFINITY;
        for a in 0..3 {
            let (oa, da, lo, hi) = (o[a], d[a], self.min[a], self.max[a]);
            if da.abs() < 1e-9 {
                if oa < lo || oa > hi {
                    return None;
                }
            } else {
                let mut ta = (lo - oa) / da;
                let mut tb = (hi - oa) / da;
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
                if t0 > t1 {
                    return None;
                }
            }
        }
        Some(t0)
    }
}

/// Step height and body height used when deciding whether a solid blocks someone standing at `pos` (feet).
pub const STEP_UP: f32 = 0.3;
pub const BODY_TOP: f32 = 1.6;

/// Push a disc (feet position `pos`, radius `radius`) out of every solid it overlaps; the result slides along walls.
/// `dir_hint` (the intended motion) only breaks ties when the centre is deep inside a box.
pub fn resolve_slide(solids: &[Aabb], pos: Vec3, radius: f32, dir_hint: Vec3) -> Vec3 {
    let mut p = pos;
    for _ in 0..4 {
        let mut moved = false;
        for s in solids {
            if s.max.y <= p.y + STEP_UP || s.min.y >= p.y + BODY_TOP {
                continue;
            }
            let qx = p.x.clamp(s.min.x, s.max.x);
            let qz = p.z.clamp(s.min.z, s.max.z);
            let dx = p.x - qx;
            let dz = p.z - qz;
            let d2 = dx * dx + dz * dz;
            if d2 > 1e-10 {
                if d2 < radius * radius {
                    let d = d2.sqrt();
                    let push = radius - d;
                    p.x += dx / d * push;
                    p.z += dz / d * push;
                    moved = true;
                }
            } else {
                // centre inside the footprint: leave by the shallowest side, preferring against the motion
                let pen = [
                    (p.x - s.min.x + radius, -1.0f32, 0usize),
                    (s.max.x - p.x + radius, 1.0, 0),
                    (p.z - s.min.z + radius, -1.0, 1),
                    (s.max.z - p.z + radius, 1.0, 1),
                ];
                let mut best = pen[0];
                for c in pen.iter() {
                    let better = c.0 < best.0 - 1e-6;
                    let tie = (c.0 - best.0).abs() <= 1e-6 && {
                        let dh = if c.2 == 0 { dir_hint.x } else { dir_hint.z };
                        dh * c.1 < 0.0
                    };
                    if better || tie {
                        best = *c;
                    }
                }
                if best.2 == 0 {
                    p.x += best.1 * best.0;
                } else {
                    p.z += best.1 * best.0;
                }
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    p
}

fn level_byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The 24 vertices and 36 indices of a cube, positions offset by `origin` (the group origin). `size` may differ
/// from `c.size` (runtime scale). Colours carry face level * bottom darkening * tint; `normal.w` is the mode.
pub fn cube_geometry(c: &Cube, size: Vec3, origin: Vec3) -> (Vec<Vertex>, Vec<u16>) {
    let h = size * 0.5;
    let (hx, hy, hz) = (h.x, h.y, h.z);
    // (normal, face level, corners ccw from outside, size along u, size along v, axis)
    let faces: [(Vec3, f32, [Vec3; 4], f32, f32, char); 6] = [
        (vec3(0.0, 1.0, 0.0), 1.20, [vec3(-hx, hy, hz), vec3(hx, hy, hz), vec3(hx, hy, -hz), vec3(-hx, hy, -hz)], size.x, size.z, 'y'),
        (vec3(0.0, -1.0, 0.0), 0.50, [vec3(-hx, -hy, -hz), vec3(hx, -hy, -hz), vec3(hx, -hy, hz), vec3(-hx, -hy, hz)], size.x, size.z, 'y'),
        (vec3(0.0, 0.0, 1.0), 1.00, [vec3(-hx, -hy, hz), vec3(hx, -hy, hz), vec3(hx, hy, hz), vec3(-hx, hy, hz)], size.x, size.y, 'z'),
        (vec3(0.0, 0.0, -1.0), 0.70, [vec3(hx, -hy, -hz), vec3(-hx, -hy, -hz), vec3(-hx, hy, -hz), vec3(hx, hy, -hz)], size.x, size.y, 'z'),
        (vec3(1.0, 0.0, 0.0), 0.88, [vec3(hx, -hy, hz), vec3(hx, -hy, -hz), vec3(hx, hy, -hz), vec3(hx, hy, hz)], size.z, size.y, 'x'),
        (vec3(-1.0, 0.0, 0.0), 0.80, [vec3(-hx, -hy, -hz), vec3(-hx, -hy, hz), vec3(-hx, hy, hz), vec3(-hx, hy, -hz)], size.z, size.y, 'x'),
    ];
    let emissive_flag = c.emissive;
    let emissive = c.emissive && c.hazard.is_empty();
    let shade = if emissive_flag { 1.0 } else { c.shade };
    let mode = if emissive { EMISSIVE_MODE } else { LIT_MODE };
    let rot = Quat::from_rotation_y(c.rot_y);
    let centre = origin + c.pos;
    let mut verts = Vec::with_capacity(24);
    let mut idx = Vec::with_capacity(36);
    for (normal, level, corners, fu, fv, axis) in faces.iter() {
        let level = level * shade;
        let striped = c.hazard.contains(*axis);
        let (su, sv);
        if striped {
            su = ((fu / HAZ_PERIOD_M).round().max(1.0)) * 0.5;
            sv = fv / HAZ_TILE;
        } else {
            su = fu / TILE;
            sv = fv / TILE;
        }
        let mut uvs = [vec2(0.0, sv), vec2(su, sv), vec2(su, 0.0), vec2(0.0, 0.0)];
        if !c.hazard.is_empty() && !striped {
            let dark = vec2(0.5 / TEX as f32, 5.5 / TEX as f32);
            uvs = [dark, dark, dark, dark];
        }
        let base = verts.len() as u16;
        let n = rot * *normal;
        for (i, p) in corners.iter().enumerate() {
            let mut k = level;
            if normal.y.abs() < 0.5 {
                let t = ((p.y + h.y) / size.y.max(0.001) * 2.2).clamp(0.0, 1.0);
                k *= c.ao + (1.0 - c.ao) * t;
            }
            let world = centre + rot * *p;
            verts.push(Vertex {
                position: world,
                uv: uvs[i],
                color: [level_byte(k * c.tint[0]), level_byte(k * c.tint[1]), level_byte(k * c.tint[2]), 255],
                normal: vec4(n.x, n.y, n.z, mode),
            });
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (verts, idx)
}

/// One batch of geometry sharing a texture kind and lit/unlit mode.
pub struct BatchData {
    pub kind: String,
    pub emissive: bool,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
}

/// Group `cubes` into batches per (kind, emissive), cut at 10000 vertices / 30000 indices. `scales[i]` multiplies
/// cube i's size when present.
pub fn build_batches(cubes: &[Cube], scales: &[Vec3], origin: Vec3) -> Vec<BatchData> {
    let mut open: BTreeMap<(String, bool), usize> = BTreeMap::new();
    let mut out: Vec<BatchData> = Vec::new();
    for (i, c) in cubes.iter().enumerate() {
        let hazard = !c.hazard.is_empty();
        let kind = if hazard { "hazard".to_string() } else if c.kind.is_empty() { art::kind_for(&c.name).to_string() } else { c.kind.clone() };
        let emissive = c.emissive && !hazard;
        let size = c.size * scales.get(i).copied().unwrap_or(Vec3::ONE);
        let (v, ix) = cube_geometry(c, size, origin);
        let key = (kind.clone(), emissive);
        let need_new = match open.get(&key) {
            Some(&b) => out[b].vertices.len() + v.len() > MAX_BATCH_VERTS || out[b].indices.len() + ix.len() > MAX_BATCH_INDICES,
            None => true,
        };
        if need_new {
            out.push(BatchData { kind, emissive, vertices: Vec::new(), indices: Vec::new() });
            open.insert(key.clone(), out.len() - 1);
        }
        let b = &mut out[open[&key]];
        let base = b.vertices.len() as u16;
        b.vertices.extend(v);
        b.indices.extend(ix.iter().map(|k| k + base));
    }
    out
}

/// World builder: collects groups, extra static solids and named markers.
pub struct WorldBuilder {
    pub groups: Vec<Group>,
    pub solids: Vec<Aabb>,
    pub markers: HashMap<String, Vec3>,
}

impl WorldBuilder {
    pub fn new() -> WorldBuilder {
        WorldBuilder { groups: Vec::new(), solids: Vec::new(), markers: HashMap::new() }
    }

    /// The group of that name, created at `origin` when missing.
    pub fn group(&mut self, name: &str, origin: Vec3) -> &mut Group {
        if let Some(i) = self.groups.iter().position(|g| g.name == name) {
            return &mut self.groups[i];
        }
        self.groups.push(Group::new(name, origin));
        self.groups.last_mut().expect("group just added")
    }

    /// Merge a prebuilt group (replaces cubes of a same-named group by appending).
    pub fn add_group(&mut self, g: Group) {
        if let Some(i) = self.groups.iter().position(|x| x.name == g.name) {
            self.groups[i].cubes.extend(g.cubes);
        } else {
            self.groups.push(g);
        }
    }

    pub fn marker(&mut self, name: &str, p: Vec3) {
        self.markers.insert(name.to_string(), p);
    }

    /// Needs GL (textures). Solid cubes become AABBs (a rotated cube uses the box around its footprint).
    pub fn bake(self, tex: &Textures) -> World {
        let handles = tex.handles();
        let mut solids = self.solids;
        let mut groups = Vec::new();
        for g in self.groups {
            for c in g.cubes.iter().filter(|c| c.solid) {
                solids.push(cube_aabb(c, g.origin));
            }
            let mut wg = WGroup {
                scales: vec![Vec3::ONE; g.cubes.len()],
                name: g.name,
                origin: g.origin,
                cubes: g.cubes,
                visible: true,
                offset: Vec3::ZERO,
                meshes: Vec::new(),
            };
            wg.rebuild(&handles);
            groups.push(wg);
        }
        World { groups, solids, markers: self.markers, handles }
    }
}

/// The AABB of a solid cube (rotation about Y widens the footprint).
pub fn cube_aabb(c: &Cube, origin: Vec3) -> Aabb {
    let (s, co) = (c.rot_y.sin().abs(), c.rot_y.cos().abs());
    let size = vec3(c.size.x * co + c.size.z * s, c.size.y, c.size.x * s + c.size.z * co);
    Aabb::from_centre(origin + c.pos, size)
}

struct WGroup {
    name: String,
    origin: Vec3,
    cubes: Vec<Cube>,
    scales: Vec<Vec3>,
    visible: bool,
    offset: Vec3,
    meshes: Vec<Mesh>,
}

impl WGroup {
    fn rebuild(&mut self, handles: &HashMap<String, Texture2D>) {
        self.meshes = build_batches(&self.cubes, &self.scales, self.origin)
            .into_iter()
            .map(|b| Mesh {
                vertices: b.vertices,
                indices: b.indices,
                texture: handles.get(&b.kind).or_else(|| handles.get("concrete")).cloned(),
            })
            .collect();
    }
}

/// Baked world: one Mesh per (kind, emissive) batch per group (split at 10000 vertices).
pub struct World {
    groups: Vec<WGroup>,
    pub solids: Vec<Aabb>,
    pub markers: HashMap<String, Vec3>,
    handles: HashMap<String, Texture2D>,
}

impl World {
    /// Bind the shader, draw every visible group, unbind.
    pub fn draw(&self, shader: &LitShader, lights: &LightSet, cam_pos: Vec3) {
        shader.begin(lights, cam_pos);
        self.draw_groups(&|_| true);
        shader.end();
    }

    /// Like `draw` but only the groups whose name starts with one of `prefixes`.
    pub fn draw_filtered(&self, shader: &LitShader, lights: &LightSet, cam_pos: Vec3, prefixes: &[&str]) {
        shader.begin(lights, cam_pos);
        self.cull_draw(prefixes);
        shader.end();
    }

    /// Draw only the listed groups (by name prefix). The caller has already bound the lit shader.
    pub fn cull_draw(&self, group_prefixes: &[&str]) {
        self.draw_groups(&|n| group_prefixes.iter().any(|p| n.starts_with(p)));
    }

    fn draw_groups(&self, want: &dyn Fn(&str) -> bool) {
        for g in self.groups.iter().filter(|g| g.visible && want(&g.name)) {
            let moved = g.offset != Vec3::ZERO;
            if moved {
                unsafe {
                    get_internal_gl().quad_gl.push_model_matrix(Mat4::from_translation(g.offset));
                }
            }
            for m in &g.meshes {
                draw_mesh(m);
            }
            if moved {
                unsafe {
                    get_internal_gl().quad_gl.pop_model_matrix();
                }
            }
        }
    }

    fn find(&mut self, group: &str) -> Option<usize> {
        self.groups.iter().position(|g| g.name == group)
    }

    pub fn has_group(&self, group: &str) -> bool {
        self.groups.iter().any(|g| g.name == group)
    }

    pub fn group_names(&self) -> Vec<String> {
        self.groups.iter().map(|g| g.name.clone()).collect()
    }

    pub fn set_visible(&mut self, group: &str, v: bool) {
        if let Some(i) = self.find(group) {
            self.groups[i].visible = v;
        }
    }

    pub fn set_offset(&mut self, group: &str, off: Vec3) {
        if let Some(i) = self.find(group) {
            self.groups[i].offset = off;
        }
    }

    pub fn offset_of(&self, group: &str) -> Vec3 {
        self.groups.iter().find(|g| g.name == group).map(|g| g.offset).unwrap_or(Vec3::ZERO)
    }

    /// Re-skin every cube called `cube` in `group` (lamp shade, panel buttons). Rebuilds only that group's meshes.
    pub fn set_material(&mut self, group: &str, cube: &str, kind: &str, tint: [f32; 3], emissive: bool) {
        let Some(i) = self.find(group) else { return };
        let mut changed = false;
        for c in self.groups[i].cubes.iter_mut().filter(|c| c.name == cube) {
            if c.kind != kind || c.tint != tint || c.emissive != emissive {
                c.kind = kind.to_string();
                c.tint = tint;
                c.emissive = emissive;
                changed = true;
            }
        }
        if changed {
            let h = self.handles.clone();
            self.groups[i].rebuild(&h);
        }
    }

    /// Scale every cube called `cube` in `group` about its centre (progress bars).
    pub fn set_cube_scale(&mut self, group: &str, cube: &str, s: Vec3) {
        let Some(i) = self.find(group) else { return };
        let mut changed = false;
        let g = &mut self.groups[i];
        for (k, c) in g.cubes.iter().enumerate() {
            if c.name == cube && g.scales[k] != s {
                g.scales[k] = s;
                changed = true;
            }
        }
        if changed {
            let h = self.handles.clone();
            self.groups[i].rebuild(&h);
        }
    }

    /// Slide a disc (feet position) out of the world's solids.
    pub fn resolve(&self, pos: Vec3, radius: f32, dir_hint: Vec3) -> Vec3 {
        resolve_slide(&self.solids, pos, radius, dir_hint)
    }

    /// Distance to the nearest solid along a ray (None for a clear ray), only hits nearer than `max`.
    pub fn raycast(&self, o: Vec3, d: Vec3, max: f32) -> Option<f32> {
        self.solids.iter().filter_map(|s| s.ray(o, d)).filter(|t| *t <= max).fold(None, |a: Option<f32>, t| Some(a.map_or(t, |x| x.min(t))))
    }

    pub fn triangle_count(&self) -> usize {
        self.groups.iter().filter(|g| g.visible).map(|g| g.meshes.iter().map(|m| m.indices.len() / 3).sum::<usize>()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_has_24_vertices_and_36_indices() {
        let c = Cube::new("Box", vec3(1.0, 1.0, 1.0), Vec3::ZERO);
        let (v, i) = cube_geometry(&c, c.size, Vec3::ZERO);
        assert_eq!(v.len(), 24);
        assert_eq!(i.len(), 36);
        assert!(i.iter().all(|k| (*k as usize) < 24));
    }

    #[test]
    fn side_faces_darken_toward_the_floor() {
        let c = Cube::new("Wall", vec3(1.0, 1.0, 1.0), Vec3::ZERO);
        let (v, _) = cube_geometry(&c, c.size, Vec3::ZERO);
        // +Z face: vertices 8,9 are the bottom edge, 10,11 the top edge; face level 1.0
        assert_eq!(v[8].color[0], level_byte(0.72));
        assert_eq!(v[10].color[0], 255);
        // top face is level 1.2, clamped to full
        assert_eq!(v[0].color[0], 255);
        // bottom face is 0.5
        assert_eq!(v[4].color[0], level_byte(0.5));
        // -X face level 0.8: bottom is 0.8 * 0.72
        assert_eq!(v[20].color[0], level_byte(0.8 * 0.72));
        assert_eq!(v[0].normal.w, LIT_MODE);
    }

    #[test]
    fn tint_multiplies_vertex_colour_and_emissive_sets_mode_zero() {
        let c = Cube::new("Screen", vec3(1.0, 1.0, 1.0), Vec3::ZERO).tint([1.0, 0.5, 0.0]).emissive();
        let (v, _) = cube_geometry(&c, c.size, Vec3::ZERO);
        assert!(v.iter().all(|x| x.normal.w == EMISSIVE_MODE));
        // +Y level 1.2 times the tint: red clamps to full, green is 0.6
        assert_eq!(v[0].color, [255, 153, 0, 255]);
    }

    #[test]
    fn uv_tiles_are_half_a_metre() {
        let c = Cube::new("Wall", vec3(2.0, 1.0, 0.2), Vec3::ZERO);
        let (v, _) = cube_geometry(&c, c.size, Vec3::ZERO);
        // +Z face u runs 0..4 (2 m / 0.5), v 0..2
        assert_eq!(v[9].uv, vec2(4.0, 2.0));
        assert_eq!(v[11].uv, vec2(0.0, 0.0));
    }

    #[test]
    fn hazard_faces_count_whole_periods_and_other_faces_use_one_dark_texel() {
        let c = Cube::new("Hazard", vec3(1.0, 0.09, 0.09), Vec3::ZERO).hazard("y");
        let (v, _) = cube_geometry(&c, c.size, Vec3::ZERO);
        // top face striped: u = round(1.0 / 0.125) * 0.5 = 4, v = 0.09 / 0.25
        assert_eq!(v[1].uv.x, 4.0);
        assert!((v[1].uv.y - 0.36).abs() < 1e-5);
        // a +Z end face is one texel
        assert_eq!(v[8].uv, v[10].uv);
    }

    #[test]
    fn rotation_turns_positions_and_normals() {
        let c = Cube::new("Box", vec3(2.0, 1.0, 1.0), vec3(5.0, 0.0, 0.0)).rot_y(std::f32::consts::FRAC_PI_2);
        let (v, _) = cube_geometry(&c, c.size, Vec3::ZERO);
        let max_z = v.iter().map(|x| x.position.z).fold(f32::MIN, f32::max);
        assert!((max_z - 1.0).abs() < 1e-4, "the 2 m side now runs along z");
        // +Z face normal rotated a quarter turn about Y points to +X
        assert!((v[8].normal.x - 1.0).abs() < 1e-4);
    }

    #[test]
    fn group_add_resolves_kind_and_emissive_from_the_name() {
        let mut g = Group::new("G", Vec3::ZERO);
        g.bx("DeskTop", [1.0, 0.1, 1.0], [0.0, 0.0, 0.0]);
        g.bx("LedAlarm", [0.1, 0.1, 0.1], [0.0, 0.0, 0.0]).set_tint([1.0, 0.0, 0.0]);
        assert_eq!(g.cubes[0].kind, "wood");
        assert!(!g.cubes[0].emissive);
        assert_eq!(g.cubes[1].kind, "green");
        assert!(g.cubes[1].emissive);
        assert_eq!(g.cubes[1].tint, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn batches_split_at_10000_vertices_and_group_by_kind() {
        let mut cubes = Vec::new();
        for i in 0..500 {
            let mut c = Cube::new("Wall", Vec3::ONE, vec3(i as f32, 0.0, 0.0));
            c.kind = "wall".to_string();
            cubes.push(c);
        }
        let mut f = Cube::new("Floor", Vec3::ONE, Vec3::ZERO);
        f.kind = "floor".to_string();
        cubes.push(f);
        let b = build_batches(&cubes, &[], Vec3::ZERO);
        let walls: Vec<_> = b.iter().filter(|x| x.kind == "wall").collect();
        assert_eq!(walls.len(), 2, "500 cubes = 12000 vertices need two batches");
        assert_eq!(walls[0].vertices.len(), 9984, "416 cubes fit under 10000 vertices");
        assert_eq!(walls.iter().map(|x| x.vertices.len()).sum::<usize>(), 12000);
        assert_eq!(b.iter().filter(|x| x.kind == "floor").count(), 1);
        for x in &b {
            assert!(x.vertices.len() <= MAX_BATCH_VERTS && x.indices.len() <= MAX_BATCH_INDICES);
            assert!(x.indices.iter().all(|i| (*i as usize) < x.vertices.len()));
        }
    }

    #[test]
    fn scale_changes_the_extent_about_the_centre() {
        let c = Cube::new("Bar", vec3(1.0, 1.0, 1.0), Vec3::ZERO);
        let b = build_batches(&[c], &[vec3(0.5, 1.0, 1.0)], Vec3::ZERO);
        let max_x = b[0].vertices.iter().map(|v| v.position.x).fold(f32::MIN, f32::max);
        assert!((max_x - 0.25).abs() < 1e-5);
    }

    #[test]
    fn aabb_ray_hits_and_misses() {
        let b = Aabb::from_centre(vec3(0.0, 1.0, -5.0), vec3(2.0, 2.0, 2.0));
        let t = b.ray(vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, -1.0)).expect("hit");
        assert!((t - 4.0).abs() < 1e-5);
        assert!(b.ray(vec3(5.0, 1.0, 0.0), vec3(0.0, 0.0, -1.0)).is_none());
        assert!(b.ray(vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)).is_none(), "behind the origin");
        assert_eq!(b.ray(vec3(0.0, 1.0, -5.0), vec3(0.0, 0.0, 1.0)), Some(0.0), "inside");
        assert!(b.contains_xz(1.1, -5.0, 0.2) && !b.contains_xz(1.5, -5.0, 0.2));
    }

    #[test]
    fn resolve_slides_along_a_wall() {
        let wall = Aabb { min: vec3(1.0, 0.0, -5.0), max: vec3(1.2, 3.0, 5.0) };
        let p = resolve_slide(&[wall], vec3(0.95, 0.0, 2.0), 0.24, Vec3::X);
        assert!((p.x - 0.76).abs() < 1e-4, "pushed back to the wall minus the radius");
        assert!((p.z - 2.0).abs() < 1e-4, "motion along the wall is kept");
        // far away: untouched
        assert_eq!(resolve_slide(&[wall], vec3(0.0, 0.0, 0.0), 0.24, Vec3::ZERO), vec3(0.0, 0.0, 0.0));
        // a floor slab is not an obstacle
        let floor = Aabb { min: vec3(-9.0, -0.2, -9.0), max: vec3(9.0, 0.0, 9.0) };
        assert_eq!(resolve_slide(&[floor], vec3(0.0, 0.0, 0.0), 0.24, Vec3::ZERO), vec3(0.0, 0.0, 0.0));
        // inside the box: leaves by the shallowest side
        let q = resolve_slide(&[wall], vec3(1.05, 0.0, 0.0), 0.24, Vec3::ZERO);
        assert!(q.x < 1.0 || q.x > 1.2);
    }

    #[test]
    fn solid_cube_aabb_widens_when_rotated() {
        let c = Cube::new("Box", vec3(2.0, 1.0, 1.0), vec3(0.0, 0.5, 0.0)).rot_y(std::f32::consts::FRAC_PI_2);
        let a = cube_aabb(&c, Vec3::ZERO);
        assert!((a.max.z - 1.0).abs() < 1e-4 && (a.max.x - 0.5).abs() < 1e-4);
    }
}
