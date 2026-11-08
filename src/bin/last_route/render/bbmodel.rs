//! Blockbench (.bbmodel, "free" format) reader for the hunters and the room props: cube
//! elements with per-face atlas UVs (hunters) and mesh elements (props rebuilt from the
//! original's blocky GLBs). Hunters are posed by their rig (outliner groups = bones, see render/anim:
//! `build_posed`); `build` keeps the old whole-limb swing for a model without one.

use crate::content::embedded::base64_decode;
use macroquad::prelude::*;

pub struct Face {
    /// Corners in model units (TL, TR, BR, BL for cubes; 3 or 4 for mesh faces) and UVs (0..1).
    pub corners: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub normal: Vec3,
}

#[derive(Clone, Copy, PartialEq)]
enum Limb {
    None,
    Arm(f32, Vec3),
    Leg(f32, Vec3),
}

struct Part {
    faces: Vec<Face>,
    limb: Limb,
    /// The outliner group (bone) the element hangs under.
    group: Option<usize>,
    /// The BlockyFace expression quad (element "face"): its UVs shift by the expression cell.
    face_quad: bool,
}

/// An outliner group: a bone of the rig, pivoting at its origin (bbmodel units).
pub struct Group {
    pub name: String,
    pub parent: Option<usize>,
    pub origin: Vec3,
}

pub struct Model {
    parts: Vec<Part>,
    /// The outliner's groups, parents before children.
    pub groups: Vec<Group>,
    /// Atlas resolution the UVs were authored against (pixels).
    pub res: Vec2,
    /// Model units -> metres.
    pub scale: f32,
    pub texture: Texture2D,
    /// Bounds in model units.
    pub min: Vec3,
    pub max: Vec3,
}

fn v3(v: &serde_json::Value) -> Vec3 {
    let a = v.as_array();
    let g = |i: usize| {
        a.and_then(|a| a.get(i))
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0) as f32
    };
    vec3(g(0), g(1), g(2))
}

fn rot_of(e: &serde_json::Value) -> Mat3 {
    let r = v3(&e["rotation"]);
    Mat3::from_euler(
        EulerRot::ZYX,
        r.z.to_radians(),
        r.y.to_radians(),
        r.x.to_radians(),
    )
}

/// The model's first texture file: `find(relative_path or name)` first (the file name it was found under comes back
/// with the bytes, so models that name the same atlas can share one texture), else the embedded data URI.
pub fn texture_bytes(j: &serde_json::Value, find: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<(Option<String>, Vec<u8>)> {
    let t = j["textures"].get(0)?;
    for key in ["relative_path", "name", "path"] {
        if let Some(name) = t[key].as_str() {
            let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
            if let Some(b) = find(file) {
                return Some((Some(file.to_string()), b));
            }
        }
    }
    let src = t["source"].as_str()?;
    let i = src.find("base64,")?;
    Some((None, base64_decode(&src[i + 7..])))
}

/// The model's first texture, nearest-sampled.
fn texture_of(j: &serde_json::Value, find: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<Texture2D> {
    let (_, bytes) = texture_bytes(j, find)?;
    // a texture that does not decode is no texture (the model draws plain), never a panic
    crate::shared::assets::try_tex(&bytes)
}

/// The geometry of a model, read without its texture (so it can be read, and tested, with no graphics context).
struct Shape {
    parts: Vec<Part>,
    groups: Vec<Group>,
    min: Vec3,
    max: Vec3,
}

/// The pixel space the face UVs are written in. Blockbench maps a face's UV through the texture's own
/// `uv_width`/`uv_height`, not through the project `resolution` (the project value only seeds new textures), so
/// the texture's value wins; then the project resolution; then the picture's pixel size (`tex_w` x `tex_h`).
pub fn uv_space(j: &serde_json::Value, tex_w: f32, tex_h: f32) -> (f32, f32) {
    let pick = |key: &str, res_key: &str, px: f32| -> f32 {
        j["textures"][0][key]
            .as_f64()
            .filter(|v| *v > 0.0)
            .or_else(|| j["resolution"][res_key].as_f64())
            .unwrap_or(px as f64) as f32
    };
    (pick("uv_width", "width", tex_w), pick("uv_height", "height", tex_h))
}

/// The elements and outliner of a document, UVs divided by `res_w` x `res_h`; `None` with no drawable element.
fn read_shape(j: &serde_json::Value, res_w: f32, res_h: f32) -> Option<Shape> {
    let els = j["elements"].as_array()?;
    let pivot = |name: &str| -> Option<Vec3> {
        els.iter()
            .find(|e| e["name"].as_str() == Some(name))
            .map(|e| {
                let f = v3(&e["from"]);
                let t = v3(&e["to"]);
                vec3((f.x + t.x) * 0.5, t.y, (f.z + t.z) * 0.5)
            })
    };
    let (r_sh, l_sh, r_hip, l_hip) = (
        pivot("R-arm"),
        pivot("L-arm"),
        pivot("R-thigh"),
        pivot("L-thigh"),
    );
    // the outliner: groups (bones) and which group each element uuid hangs under
    let mut groups: Vec<Group> = vec![];
    let mut owner: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    fn walk(
        nodes: &serde_json::Value,
        gmap: &std::collections::HashMap<&str, &serde_json::Value>,
        parent: Option<usize>,
        groups: &mut Vec<Group>,
        owner: &mut std::collections::HashMap<String, usize>,
    ) {
        for n in nodes.as_array().into_iter().flatten() {
            if let Some(uuid) = n.as_str() {
                if let Some(p) = parent {
                    owner.insert(uuid.to_string(), p);
                }
            } else if n.is_object() {
                // Blockbench 5 outline nodes hold only {uuid, children}; the name and origin are in "groups"
                let src = n["uuid"].as_str().and_then(|u| gmap.get(u)).copied().unwrap_or(n);
                groups.push(Group {
                    name: src["name"].as_str().unwrap_or("").to_string(),
                    parent,
                    origin: v3(&src["origin"]),
                });
                let me = groups.len() - 1;
                walk(&n["children"], gmap, Some(me), groups, owner);
            }
        }
    }
    let gmap: std::collections::HashMap<&str, &serde_json::Value> = j["groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| g["uuid"].as_str().map(|u| (u, g)))
        .collect();
    walk(&j["outliner"], &gmap, None, &mut groups, &mut owner);
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let mut parts = vec![];
    for e in els {
        if e["visibility"].as_bool() == Some(false) || e["export"].as_bool() == Some(false) {
            continue;
        }
        let origin = v3(&e["origin"]);
        let rot = rot_of(e);
        let place = |v: Vec3| rot * (v - origin) + origin;
        let mut faces = vec![];
        match e["type"].as_str().unwrap_or("cube") {
            "cube" => {
                let f = v3(&e["from"]);
                let t = v3(&e["to"]);
                for (dir, normal) in [
                    ("north", vec3(0., 0., -1.)),
                    ("south", vec3(0., 0., 1.)),
                    ("east", vec3(1., 0., 0.)),
                    ("west", vec3(-1., 0., 0.)),
                    ("up", vec3(0., 1., 0.)),
                    ("down", vec3(0., -1., 0.)),
                ] {
                    let fj = &e["faces"][dir];
                    if fj.is_null() || fj["texture"].is_null() {
                        continue;
                    }
                    let uv = fj["uv"].as_array();
                    let g = |i: usize| {
                        uv.and_then(|a| a.get(i))
                            .and_then(|x| x.as_f64())
                            .unwrap_or(0.0) as f32
                    };
                    let (u1, v1, u2, v2) =
                        (g(0) / res_w, g(1) / res_h, g(2) / res_w, g(3) / res_h);
                    if (u1 - u2).abs() < 1e-6 && (v1 - v2).abs() < 1e-6 {
                        continue;
                    }
                    let c = match dir {
                        "north" => [
                            vec3(t.x, t.y, f.z),
                            vec3(f.x, t.y, f.z),
                            vec3(f.x, f.y, f.z),
                            vec3(t.x, f.y, f.z),
                        ],
                        "south" => [
                            vec3(f.x, t.y, t.z),
                            vec3(t.x, t.y, t.z),
                            vec3(t.x, f.y, t.z),
                            vec3(f.x, f.y, t.z),
                        ],
                        "east" => [
                            vec3(t.x, t.y, t.z),
                            vec3(t.x, t.y, f.z),
                            vec3(t.x, f.y, f.z),
                            vec3(t.x, f.y, t.z),
                        ],
                        "west" => [
                            vec3(f.x, t.y, f.z),
                            vec3(f.x, t.y, t.z),
                            vec3(f.x, f.y, t.z),
                            vec3(f.x, f.y, f.z),
                        ],
                        "up" => [
                            vec3(f.x, t.y, f.z),
                            vec3(t.x, t.y, f.z),
                            vec3(t.x, t.y, t.z),
                            vec3(f.x, t.y, t.z),
                        ],
                        _ => [
                            vec3(f.x, f.y, t.z),
                            vec3(t.x, f.y, t.z),
                            vec3(t.x, f.y, f.z),
                            vec3(f.x, f.y, f.z),
                        ],
                    };
                    let mut uvs = [vec2(u1, v1), vec2(u2, v1), vec2(u2, v2), vec2(u1, v2)];
                    let turns =
                        (fj["rotation"].as_f64().unwrap_or(0.0) / 90.0).round() as usize % 4;
                    uvs.rotate_right(turns);
                    faces.push(Face {
                        corners: c.iter().map(|p| place(*p)).collect(),
                        uv: uvs.to_vec(),
                        normal: rot * normal,
                    });
                }
            }
            "mesh" => {
                let verts = &e["vertices"];
                let Some(fs) = e["faces"].as_object() else {
                    continue;
                };
                for fj in fs.values() {
                    if fj["texture"].is_null() {
                        continue;
                    }
                    let Some(keys) = fj["vertices"].as_array() else {
                        continue;
                    };
                    let mut corners = vec![];
                    let mut uvs = vec![];
                    for k in keys {
                        let k = k.as_str().unwrap_or("");
                        corners.push(place(v3(&verts[k]) + origin));
                        let uv = &fj["uv"][k];
                        let u =
                            uv.get(0).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32 / res_w;
                        let v =
                            uv.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32 / res_h;
                        uvs.push(vec2(u, v));
                    }
                    if corners.len() < 3 {
                        continue;
                    }
                    let n = (corners[1] - corners[0])
                        .cross(corners[2] - corners[0])
                        .normalize_or_zero();
                    faces.push(Face {
                        corners,
                        uv: uvs,
                        normal: n,
                    });
                }
            }
            _ => continue,
        }
        for f in &faces {
            for c in &f.corners {
                min = min.min(*c);
                max = max.max(*c);
            }
        }
        let name = e["name"].as_str().unwrap_or("");
        let limb = if ["R-arm", "R-forearm", "R-hand"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            r_sh.map(|p| Limb::Arm(1.0, p)).unwrap_or(Limb::None)
        } else if ["L-arm", "L-forearm", "L-hand"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            l_sh.map(|p| Limb::Arm(-1.0, p)).unwrap_or(Limb::None)
        } else if ["R-thigh", "R-calf", "R-foot"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            r_hip.map(|p| Limb::Leg(-1.0, p)).unwrap_or(Limb::None)
        } else if ["L-thigh", "L-calf", "L-foot"]
            .iter()
            .any(|p| name.starts_with(p))
        {
            l_hip.map(|p| Limb::Leg(1.0, p)).unwrap_or(Limb::None)
        } else {
            Limb::None
        };
        let group = e["uuid"].as_str().and_then(|u| owner.get(u).copied());
        parts.push(Part {
            faces,
            limb,
            group,
            face_quad: name == "face",
        });
    }
    if parts.is_empty() {
        return None;
    }
    Some(Shape {
        parts,
        groups,
        min,
        max,
    })
}

impl Model {
    /// `height_m`: scale the model so its top sits at this height (hunters); `None` keeps
    /// Blockbench's 16 units per metre (props).
    pub fn parse(
        text: &str,
        texture: Option<Texture2D>,
        find: &dyn Fn(&str) -> Option<Vec<u8>>,
        height_m: Option<f32>,
    ) -> Option<Model> {
        let j: serde_json::Value = serde_json::from_str(text).ok()?;
        Model::from_json(&j, texture, find, height_m)
    }

    /// `parse` on a document already read (a caller that looks at the textures first reads it once).
    pub fn from_json(
        j: &serde_json::Value,
        texture: Option<Texture2D>,
        find: &dyn Fn(&str) -> Option<Vec<u8>>,
        height_m: Option<f32>,
    ) -> Option<Model> {
        let texture = match texture {
            Some(t) => t,
            None => texture_of(j, find)?,
        };
        let (res_w, res_h) = uv_space(j, texture.width(), texture.height());
        let Shape {
            parts,
            groups,
            min,
            max,
        } = read_shape(j, res_w, res_h)?;
        let scale = match height_m {
            Some(h) => h / max.y.max(1.0),
            None => 1.0 / 16.0,
        };
        Some(Model {
            parts,
            groups,
            res: vec2(res_w, res_h),
            scale,
            texture,
            min,
            max,
        })
    }

    /// Appends the model posed by a rig. `root` maps model space (bbmodel units) to the world; `bone_of(group)` gives a
    /// group's bone transform in model space (pivot at the bone origin), or None for a group the rig does not drive (it then
    /// rides its parent at its rest offset). Each element is placed relative to its group's origin, as the glb's per-bone
    /// meshes are (bb_gltf.py: vertex = corner - pivot). `face_uv` shifts the face quad's UVs (BlockyFace cell offset).
    pub fn build_posed(
        &self,
        out: &mut Vec<Vertex>,
        idx: &mut Vec<u16>,
        root: Mat4,
        bone_of: &dyn Fn(usize) -> Option<Mat4>,
        face_uv: Vec2,
        tint: Color,
    ) {
        self.build_posed_mode(out, idx, root, bone_of, face_uv, tint, crate::render::shader::LIT_MODE);
    }

    /// [`Self::build_posed`] with the shader mode written into every vertex (`shader::LIT_MODE`, or `shader::GHOST_MODE`
    /// for a body that fades by its tint alpha, like a crouching runner).
    #[allow(clippy::too_many_arguments)]
    pub fn build_posed_mode(
        &self,
        out: &mut Vec<Vertex>,
        idx: &mut Vec<u16>,
        root: Mat4,
        bone_of: &dyn Fn(usize) -> Option<Mat4>,
        face_uv: Vec2,
        tint: Color,
        mode: f32,
    ) {
        let col: [u8; 4] = tint.into();
        let mut group_m: Vec<Mat4> = Vec::with_capacity(self.groups.len());
        for (k, g) in self.groups.iter().enumerate() {
            let m = match bone_of(k) {
                Some(m) => m,
                None => match g.parent {
                    Some(p) if p < group_m.len() => {
                        group_m[p] * Mat4::from_translation(g.origin - self.groups[p].origin)
                    }
                    _ => Mat4::from_translation(g.origin),
                },
            };
            group_m.push(m);
        }
        for p in &self.parts {
            let (m, origin) = match p.group {
                Some(gi) => (root * group_m[gi], self.groups[gi].origin),
                None => (root, Vec3::ZERO),
            };
            let nm = Mat3::from_mat4(m);
            for f in &p.faces {
                if out.len() > 64000 {
                    return;
                }
                let n = (nm * f.normal).normalize_or_zero();
                let base = out.len() as u16;
                for (c, uv) in f.corners.iter().zip(&f.uv) {
                    let uv = if p.face_quad { *uv + face_uv } else { *uv };
                    out.push(Vertex {
                        position: m.transform_point3(*c - origin),
                        uv,
                        color: col,
                        normal: vec4(n.x, n.y, n.z, mode),
                    });
                }
                for k in 1..(f.corners.len() as u16 - 1) {
                    idx.extend_from_slice(&[base, base + k, base + k + 1]);
                }
            }
        }
    }

    /// Every face with the outliner group its element hangs under (an index into `groups`), in file order.
    pub fn faces(&self) -> impl Iterator<Item = (Option<usize>, &Face)> {
        self.parts.iter().flat_map(|p| p.faces.iter().map(move |f| (p.group, f)))
    }

    /// The BlockyFace cell's UV offset on this model's atlas (8x8 px cells, 3 columns).
    pub fn face_cell_uv(&self, cell: u32) -> Vec2 {
        vec2(
            (cell % 3) as f32 * 8.0 / self.res.x,
            (cell / 3) as f32 * 8.0 / self.res.y,
        )
    }

    /// Appends the posed model's triangles. `swing` is the stride phase in radians, `amp` its
    /// strength (0 when standing still); `mode` is the shader mode (1 lit atlas).
    pub fn build(
        &self,
        out: &mut Vec<Vertex>,
        idx: &mut Vec<u16>,
        pos: Vec3,
        yaw: f32,
        swing: f32,
        amp: f32,
        tint: Color,
    ) {
        self.build_scaled(
            out,
            idx,
            pos,
            yaw,
            Vec3::splat(self.scale),
            swing,
            amp,
            tint,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_scaled(
        &self,
        out: &mut Vec<Vertex>,
        idx: &mut Vec<u16>,
        pos: Vec3,
        yaw: f32,
        scale: Vec3,
        swing: f32,
        amp: f32,
        tint: Color,
    ) {
        let yaw_m = Mat3::from_rotation_y(yaw);
        let col: [u8; 4] = tint.into();
        for p in &self.parts {
            let limb = match p.limb {
                Limb::Arm(side, pv) => {
                    Some((Mat3::from_rotation_x(swing.sin() * amp * 0.9 * side), pv))
                }
                Limb::Leg(side, pv) => {
                    Some((Mat3::from_rotation_x(swing.sin() * amp * 0.75 * side), pv))
                }
                Limb::None => None,
            };
            for f in &p.faces {
                if out.len() > 64000 {
                    return;
                }
                let xf = |v: Vec3| -> Vec3 {
                    let mut q = v;
                    if let Some((m, pv)) = limb {
                        q = m * (q - pv) + pv;
                    }
                    pos + yaw_m * (q * scale)
                };
                let mut n = f.normal;
                if let Some((m, _)) = limb {
                    n = m * n;
                }
                n = yaw_m * n;
                let base = out.len() as u16;
                for (c, uv) in f.corners.iter().zip(&f.uv) {
                    out.push(Vertex {
                        position: xf(*c),
                        uv: *uv,
                        color: col,
                        normal: vec4(n.x, n.y, n.z, 1.0),
                    });
                }
                for k in 1..(f.corners.len() as u16 - 1) {
                    idx.extend_from_slice(&[base, base + k, base + k + 1]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::embedded;

    /// Every embedded bbmodel as (asset path, parsed document).
    fn all_models() -> Vec<(&'static str, serde_json::Value)> {
        embedded::assets_under("")
            .filter(|(p, _)| p.ends_with(".bbmodel"))
            .filter_map(|(p, b)| serde_json::from_slice(b).ok().map(|j| (p, j)))
            .collect()
    }

    /// The model's first texture as a CPU image, found the way the game finds it (a sibling file, else embedded).
    fn texture_image(path: &str, j: &serde_json::Value) -> Option<Image> {
        let dir = path.rsplit_once('/').map(|x| x.0).unwrap_or("");
        let find = |name: &str| embedded::asset(&format!("{dir}/{name}")).map(|b| b.to_vec());
        let (_, bytes) = texture_bytes(j, &find)?;
        Image::from_file_with_format(&bytes, Some(ImageFormat::Png)).ok()
    }

    /// The UV space exactly as the loader chose it before the 2026-11-05 void fix (the project `resolution`, else the
    /// picture's pixel size). The golden digests below were recorded with it.
    fn legacy_res(j: &serde_json::Value, w: f32, h: f32) -> (f32, f32) {
        (
            j["resolution"]["width"].as_f64().unwrap_or(w as f64) as f32,
            j["resolution"]["height"].as_f64().unwrap_or(h as f64) as f32,
        )
    }

    /// FNV-1a over everything a draw reads from the shape: groups, face corners, UVs, normals, owners.
    fn digest(s: &Shape) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut put = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        // Quantised to 1e-4, never raw bits: sin/cos in the Euler rotations differ in the last bit between the Windows CRT
        // and Linux libm, so raw f32 bits hashed differently per platform. 1e-4 is far below a pixel.
        let q = |f: f32| ((f as f64 * 10000.0).round() as i64).to_le_bytes();
        let v3b = |v: Vec3| [q(v.x), q(v.y), q(v.z)];
        for g in &s.groups {
            put(g.name.as_bytes());
            put(&g.parent.map(|p| p as u64 + 1).unwrap_or(0).to_le_bytes());
            for b in v3b(g.origin) {
                put(&b);
            }
        }
        for p in &s.parts {
            put(&p.group.map(|g| g as u64 + 1).unwrap_or(0).to_le_bytes());
            put(&[p.face_quad as u8]);
            for f in &p.faces {
                for c in &f.corners {
                    for b in v3b(*c) {
                        put(&b);
                    }
                }
                for uv in &f.uv {
                    put(&q(uv.x));
                    put(&q(uv.y));
                }
                for b in v3b(f.normal) {
                    put(&b);
                }
            }
        }
        for b in v3b(s.min).into_iter().chain(v3b(s.max)) {
            put(&b);
        }
        h
    }

    fn shape_with(j: &serde_json::Value, res: (f32, f32)) -> Option<Shape> {
        read_shape(j, res.0, res.1)
    }

    fn is_void(path: &str) -> bool {
        path.rsplit('/').next().is_some_and(|f| f.starts_with("void_"))
    }

    /// Recorded before the loader change, with the legacy rule, from the shipped files: every non-void hunter model.
    /// (re-recorded on 2026-11-08 with the 1e-4 quantised digest: the quantisation is deliberate, for cross-platform float
    /// stability (Windows CRT and Linux libm sin/cos differ in the last bit); the models themselves did not change.)
    const GOLDEN: [(&str, u64); 27] = [
        (("hunters/aki.bbmodel", 0xefb1163bbeb1d519)),
        (("hunters/briselle.bbmodel", 0xcae6c973b4694b95)),
        (("hunters/ceryth.bbmodel", 0xefb1163bbeb1d519)),
        (("hunters/dovelle.bbmodel", 0xf81accf6b9cabb9c)),
        (("hunters/elise.bbmodel", 0xd5b5ecd1cde876b8)),
        (("hunters/ferryn.bbmodel", 0xefb1163bbeb1d519)),
        (("hunters/glyra.bbmodel", 0x7ba0051c423dcedc)),
        (("hunters/haelith.bbmodel", 0xe3b3d4498207feec)),
        (("hunters/ilyra.bbmodel", 0xd5b5ecd1cde876b8)),
        (("hunters/javelle.bbmodel", 0x168303e50197bc2a)),
        (("hunters/kathy.bbmodel", 0x5965fd5adf0da5c8)),
        (("hunters/loxelle.bbmodel", 0x6f33379905be38a2)),
        (("hunters/mavrine.bbmodel", 0x5487ad3f4134dd25)),
        (("hunters/nyxie.bbmodel", 0x9e51a9cf6c81b08f)),
        (("hunters/opalie.bbmodel", 0x6f33379905be38a2)),
        (("hunters/porcelle.bbmodel", 0x8d3e3979e4ba6465)),
        (("hunters/quenelle.bbmodel", 0xcac658e0c04babc9)),
        (("hunters/ravelle.bbmodel", 0xefb1163bbeb1d519)),
        (("hunters/solyth.bbmodel", 0x559c71a52b00d147)),
        (("hunters/thessine.bbmodel", 0xb843b341c5412441)),
        (("hunters/umbriel.bbmodel", 0x6afb248092445560)),
        (("hunters/verrine.bbmodel", 0xcac658e0c04babc9)),
        (("hunters/white_wolf_spider.bbmodel", 0x8925666e14add7d4)),
        (("hunters/wynelle.bbmodel", 0xa1ffc8f0ec7c9580)),
        (("hunters/xivvy.bbmodel", 0xcad52b5942327f5e)),
        (("hunters/ysolde.bbmodel", 0xefb1163bbeb1d519)),
        (("hunters/zellia.bbmodel", 0xefb1163bbeb1d519)),
    ];

    /// One fold of the digests of every other embedded model that has a decodable texture (props, doors, the player
    /// mannequins, the A to Z hunters), recorded the same way, and how many went into it.
    /// (685 -> 684 on 2026-11-06: the retired heater station model left the embedded set, see archive/heater-stations.)
    /// (digest moved on 2026-11-06: the sewer outflow pipe, its broken stub and the sewer bridge were re-converted from
    /// the full Godot GLBs, their wall triangles restored; only those three models differ.)
    /// (digest moved on 2026-11-07: the six room door models' back faces are UV-mapped across the wood region, not one
    /// flat strip of the atlas, so the back of a door reads as wood. Doors only; no other model changed.)
    /// (684 -> 683 on 2026-11-07: the black wolf spider was removed from the game, so its model left the set; digest recomputed.)
    /// (digest moved on 2026-11-08: re-recorded with the deliberate 1e-4 quantisation, for cross-platform float stability.)
    const GOLDEN_ALL: (usize, u64) = (684, 0x53bf3aaefcdf9a54);

    /// The shape of a model read with the game's UV rule.
    fn shape_now(path: &str, j: &serde_json::Value) -> Option<Shape> {
        let img = texture_image(path, j)?;
        let (w, h) = uv_space(j, img.width() as f32, img.height() as f32);
        shape_with(j, (w, h))
    }

    /// Prints the golden table in the quantised digest. Run once after changing `digest`:
    /// `cargo test --bin last_route print_golden_digests -- --ignored --nocapture`, then paste into GOLDEN and GOLDEN_ALL.
    #[test]
    #[ignore]
    fn print_golden_digests() {
        let models = all_models();
        for (path, _) in GOLDEN {
            let (_, j) = models.iter().find(|(p, _)| *p == path).unwrap();
            println!("(\"{path}\", {:#018x}),", digest(&shape_now(path, j).unwrap()));
        }
        let (mut fold, mut n) = (0xcbf29ce484222325u64, 0);
        for (path, j) in &models {
            if is_void(path) {
                continue;
            }
            if let Some(s) = shape_now(path, j) {
                n += 1;
                fold = (fold ^ digest(&s)).wrapping_mul(0x100000001b3);
            }
        }
        println!("GOLDEN_ALL = ({n}, {fold:#018x})");
    }

    /// The digest ignores last-bit float noise (the cross-platform fix) but still sees a real move.
    #[test]
    fn digest_ignores_last_bit_noise_but_sees_real_moves() {
        let models = all_models();
        let (path, j) = models.iter().find(|(p, _)| *p == "hunters/aki.bbmodel").expect("aki embedded");
        let a = shape_now(path, j).unwrap();
        let mut b = shape_now(path, j).unwrap();
        b.min.x = f32::from_bits(b.min.x.to_bits() ^ 1);
        assert_eq!(digest(&a), digest(&b));
        b.min.x += 0.01;
        assert_ne!(digest(&a), digest(&b));
    }

    /// The 26 A to Z hunters, the spiders and every prop draw exactly as before the void fix: same corners, same UVs.
    #[test]
    fn hunters_and_props_other_than_void_keep_their_exact_vertices() {
        let models = all_models();
        for (path, want) in GOLDEN {
            let (_, j) = models.iter().find(|(p, _)| *p == path).unwrap_or_else(|| panic!("{path} is not embedded"));
            let got = digest(&shape_now(path, j).unwrap_or_else(|| panic!("{path} does not load")));
            assert_eq!(got, want, "{path}: its vertices/UVs changed ({got:#018x})");
        }
        for need in ["hunters/dovelle.bbmodel", "hunters/aki.bbmodel", "hunters/kathy.bbmodel"] {
            assert!(GOLDEN.iter().any(|(p, _)| *p == need), "{need} must stay in the golden table");
        }
        let (mut fold, mut n) = (0xcbf29ce484222325u64, 0);
        for (path, j) in &models {
            if is_void(path) {
                continue;
            }
            if let Some(s) = shape_now(path, j) {
                n += 1;
                fold = (fold ^ digest(&s)).wrapping_mul(0x100000001b3);
            }
        }
        assert_eq!((n, fold), GOLDEN_ALL, "a non-void model draws differently now");
    }

    /// The void models now use the A to Z rig and its 64x64 skins, so no model reads a UV space other than its own
    /// picture's and the project's (the old void 64x96 exception is gone).
    #[test]
    fn no_hunter_reads_a_uv_space_other_than_its_picture_and_the_project_resolution() {
        let mut moved: Vec<&str> = vec![];
        for (path, j) in all_models() {
            let Some(img) = texture_image(path, &j) else { continue };
            let (w, h) = (img.width() as f32, img.height() as f32);
            if uv_space(&j, w, h) != legacy_res(&j, w, h) {
                moved.push(path);
            }
        }
        assert!(moved.is_empty(), "{moved:?} read a different UV space");
    }

    #[test]
    fn uv_space_prefers_the_texture_then_the_project_then_the_picture() {
        let j = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap();
        // the texture's own UV size wins over the project resolution (the void models: 64x96 against 64x64)
        let both = j(r#"{"resolution":{"width":64,"height":64},"textures":[{"uv_width":64,"uv_height":96}]}"#);
        assert_eq!(uv_space(&both, 64.0, 96.0), (64.0, 96.0));
        // no texture size: the project resolution (the A to Z hunters)
        let res = j(r#"{"resolution":{"width":64,"height":128},"textures":[{}]}"#);
        assert_eq!(uv_space(&res, 64.0, 128.0), (64.0, 128.0));
        assert_eq!(uv_space(&res, 32.0, 32.0), (64.0, 128.0));
        // neither: the picture's pixels
        assert_eq!(uv_space(&j(r#"{"textures":[{}]}"#), 16.0, 8.0), (16.0, 8.0));
        // a zero texture size does not zero the UVs
        let zero = j(r#"{"resolution":{"width":32,"height":32},"textures":[{"uv_width":0,"uv_height":0}]}"#);
        assert_eq!(uv_space(&zero, 8.0, 8.0), (32.0, 32.0));
        // width and height are chosen independently
        let half = j(r#"{"resolution":{"width":64,"height":64},"textures":[{"uv_height":96}]}"#);
        assert_eq!(uv_space(&half, 64.0, 96.0), (64.0, 96.0));
    }

    fn void_models() -> Vec<(&'static str, serde_json::Value)> {
        let v: Vec<_> = all_models().into_iter().filter(|(p, _)| is_void(p)).collect();
        assert_eq!(v.len(), 12, "the twelve void hunters");
        v
    }

    /// The A to Z rig every void model is built on: its shape is the reference for the bones and the faces.
    fn a_to_z_shape() -> Shape {
        let (path, j) = all_models().into_iter().find(|(p, _)| *p == "hunters/dovelle.bbmodel").expect("the A to Z rig");
        shape_now(path, &j).expect("dovelle loads")
    }

    fn face_count(shape: &Shape) -> usize {
        shape.parts.iter().map(|p| p.faces.len()).sum()
    }

    #[test]
    fn every_void_model_loads_with_a_texture_that_matches_its_uv_space() {
        for (path, j) in void_models() {
            let img = texture_image(path, &j).unwrap_or_else(|| panic!("{path}: no decodable texture"));
            let (w, h) = uv_space(&j, img.width() as f32, img.height() as f32);
            assert_eq!(
                (w, h),
                (img.width() as f32, img.height() as f32),
                "{path}: UV space {w}x{h} is not the texture's {}x{}",
                img.width(),
                img.height()
            );
            let shape = shape_with(&j, (w, h)).unwrap_or_else(|| panic!("{path}: does not load"));
            assert_eq!(shape.groups.len(), a_to_z_shape().groups.len(), "{path}: the bones are the A to Z rig's");
            assert!(shape.parts.iter().map(|p| p.faces.len()).sum::<usize>() >= 100, "{path}: too few faces");
        }
    }

    #[test]
    fn every_void_face_uv_lies_inside_the_texture() {
        for (path, j) in void_models() {
            let img = texture_image(path, &j).unwrap();
            let shape = shape_now(path, &j).unwrap();
            let mut faces = 0;
            for p in &shape.parts {
                for f in &p.faces {
                    faces += 1;
                    for uv in &f.uv {
                        assert!(
                            (0.0..=1.0).contains(&uv.x) && (0.0..=1.0).contains(&uv.y),
                            "{path}: UV {uv} falls outside the atlas ({}x{})",
                            img.width(),
                            img.height()
                        );
                    }
                }
            }
            assert_eq!(faces, face_count(&a_to_z_shape()), "{path}: textured faces");
        }
    }

    /// Fraction of the texels under a face's UV rectangle that are not transparent.
    fn opaque_fraction(img: &Image, f: &Face) -> f32 {
        let (w, h) = (img.width() as f32, img.height() as f32);
        let (mut u0, mut v0, mut u1, mut v1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for uv in &f.uv {
            u0 = u0.min(uv.x);
            u1 = u1.max(uv.x);
            v0 = v0.min(uv.y);
            v1 = v1.max(uv.y);
        }
        let x0 = (u0 * w + 1e-3).floor() as i32;
        let x1 = ((u1 * w - 1e-3).ceil() as i32).max(x0 + 1);
        let y0 = (v0 * h + 1e-3).floor() as i32;
        let y1 = ((v1 * h - 1e-3).ceil() as i32).max(y0 + 1);
        let data = img.get_image_data();
        let (mut n, mut solid) = (0, 0);
        for y in y0..y1 {
            for x in x0..x1 {
                n += 1;
                let inside = x >= 0 && y >= 0 && (x as f32) < w && (y as f32) < h;
                if inside && data[y as usize * img.width() + x as usize][3] >= 8 {
                    solid += 1;
                }
            }
        }
        solid as f32 / n.max(1) as f32
    }

    /// The overlay layers of a Minecraft skin (hat, jacket, sleeves, pants), in skin pixels: x0, y0, x1, y1. A skin may
    /// leave them transparent, so their faces are not held to paint; every other face (the base body) must be painted.
    const SKIN_OVERLAYS: [(f32, f32, f32, f32); 6] = [
        (32.0, 0.0, 64.0, 16.0),  // hat
        (16.0, 32.0, 40.0, 48.0), // jacket
        (40.0, 32.0, 56.0, 48.0), // right sleeve
        (48.0, 48.0, 64.0, 64.0), // left sleeve
        (0.0, 32.0, 16.0, 48.0),  // right pants
        (0.0, 48.0, 16.0, 64.0),  // left pants
    ];

    /// Whether a face's UV centre lies in a skin overlay layer.
    fn in_skin_overlay(f: &Face) -> bool {
        let (sum_u, sum_v) = f.uv.iter().fold((0.0, 0.0), |(u, v), uv| (u + uv.x, v + uv.y));
        let n = f.uv.len().max(1) as f32;
        let (x, y) = (sum_u / n * 64.0, sum_v / n * 64.0);
        SKIN_OVERLAYS.iter().any(|&(x0, y0, x1, y1)| x > x0 && x < x1 && y > y0 && y < y1)
    }

    /// Every base-layer face has to land on paint.
    #[test]
    fn void_base_faces_land_on_painted_texels() {
        for (path, j) in void_models() {
            let img = texture_image(path, &j).unwrap();
            let shape = shape_now(path, &j).unwrap();
            let name_of = |g: Option<usize>| g.map(|g| shape.groups[g].name.as_str()).unwrap_or("");
            let mut faces = 0;
            for p in &shape.parts {
                for f in &p.faces {
                    faces += 1;
                    if !in_skin_overlay(f) {
                        let frac = opaque_fraction(&img, f);
                        assert!(frac >= 0.9, "{path}: a base face of {:?} samples only {:.0}% painted texels", name_of(p.group), frac * 100.0);
                    }
                }
            }
            assert_eq!(faces, face_count(&a_to_z_shape()));
        }
    }

    /// A texture that declares its own UV size declares its picture's size (a face past the bottom edge clamps to
    /// the last row). Props are exempt: their UVs are rebuilt from GLBs.
    #[test]
    fn no_hunter_texture_declares_a_uv_space_other_than_its_picture() {
        for (path, j) in all_models() {
            if !path.starts_with("hunters/") {
                continue;
            }
            let Some(img) = texture_image(path, &j) else { continue };
            let t = &j["textures"][0];
            if let (Some(w), Some(h)) = (t["uv_width"].as_f64(), t["uv_height"].as_f64()) {
                assert_eq!((w as usize, h as usize), (img.width(), img.height()), "{path}: uv_width/uv_height disagree with the picture");
            }
        }
    }
    // ---- prop mesh integrity: the "big sewer pipe is see-through" bug (2026-11-06) ----
    //
    // The sewer outflow pipe shipped with every tube segment missing about two thirds of its wall (a 500-triangle cap
    // had been cut into the source GLB, so a 20-sided barrel kept 23 of its 76 triangles): the pipe drew as stripes
    // with the room showing through. Nothing in the loader or the renderer was at fault (the world material does not
    // cull back faces, every prop vertex is the opaque textured mode, and a part stays far below the 16-bit index
    // limit), so the checks below read the shipped models themselves.

    /// One welded vertex: a model-space position rounded to 0.01 of a bbmodel unit (below the converter's 1e-4 m).
    type Weld = (i32, i32, i32);

    fn weld(p: Vec3) -> Weld {
        ((p.x * 100.0).round() as i32, (p.y * 100.0).round() as i32, (p.z * 100.0).round() as i32)
    }

    /// How many edges of `faces` (welded by position, each face a corner loop) are not shared by exactly two faces
    /// running in opposite directions: 0 means a closed surface with one consistent winding.
    fn broken_edges(faces: &[&Face]) -> usize {
        use std::collections::HashMap;
        // undirected edge -> (low to high count, high to low count)
        let mut edges: HashMap<(Weld, Weld), (u32, u32)> = HashMap::new();
        for f in faces {
            let w: Vec<Weld> = f.corners.iter().map(|c| weld(*c)).collect();
            for i in 0..w.len() {
                let (a, b) = (w[i], w[(i + 1) % w.len()]);
                if a == b {
                    continue;
                }
                let e = edges.entry(if a < b { (a, b) } else { (b, a) }).or_default();
                if a < b {
                    e.0 += 1;
                } else {
                    e.1 += 1;
                }
            }
        }
        edges.values().filter(|e| **e != (1, 1)).count()
    }

    /// The volume the face loops enclose with the origin, summed: positive when they wind counter-clockwise seen from
    /// outside, which is the way the loader derives a face normal (`(c1 - c0) x (c2 - c0)`), so a positive total means
    /// the lit normals of a closed part point outward.
    fn signed_volume(faces: &[&Face]) -> f32 {
        let mut v = 0.0;
        for f in faces {
            for k in 1..f.corners.len() - 1 {
                v += f.corners[0].dot(f.corners[k].cross(f.corners[k + 1])) / 6.0;
            }
        }
        v
    }

    /// The prop models of the sewer's big pipework: each drawable element (a tube segment, a flange ring, a strap, a bolt)
    /// must be a closed solid, or the camera sees through it from some side.
    const SEWER_PIPEWORK: [&str; 4] = [
        "props/sewer/sewer_outflow_pipe.bbmodel",
        "props/sewer/sewer_outflow_pipe_broken.bbmodel",
        "props/sewer/sewer_pipe_run.bbmodel",
        "props/sewer/sewer_bridge.bbmodel",
    ];

    #[test]
    fn every_element_of_the_sewer_pipework_is_a_closed_outward_solid() {
        let models = all_models();
        for path in SEWER_PIPEWORK {
            let (_, j) = models.iter().find(|(p, _)| *p == path).unwrap_or_else(|| panic!("{path} is not embedded"));
            let shape = shape_now(path, j).unwrap_or_else(|| panic!("{path} does not load"));
            assert!(shape.parts.len() >= 10, "{path}: {} elements", shape.parts.len());
            assert_eq!(shape.parts.len(), j["elements"].as_array().map_or(0, Vec::len), "{path}: a part per element, so the names line up");
            for (i, part) in shape.parts.iter().enumerate() {
                let faces: Vec<&Face> = part.faces.iter().collect();
                let name = j["elements"][i]["name"].as_str().unwrap_or("?");
                assert!(faces.len() >= 4, "{path}: element {name} has {} faces, a solid needs 4", faces.len());
                assert_eq!(broken_edges(&faces), 0, "{path}: element {name} is open or wound inconsistently (see-through)");
                // a cube carries its own face normals (its corner loops wind the other way); a mesh derives them from its winding
                if j["elements"][i]["type"].as_str() == Some("mesh") {
                    assert!(signed_volume(&faces) > 0.0, "{path}: element {name} is wound inward (its lit normals face the wrong way)");
                }
            }
        }
    }

    /// The old outflow pipe (a barrel that kept a third of its wall) must trip the check, so the check is known to bite.
    #[test]
    fn a_barrel_missing_most_of_its_wall_is_seen_as_open() {
        let ring = |z: f32, k: usize| {
            let a = k as f32 * std::f32::consts::TAU / 20.0;
            vec3(a.sin(), a.cos(), z)
        };
        let side = |k: usize| Face {
            corners: vec![ring(0.0, k), ring(0.0, k + 1), ring(1.0, k + 1), ring(1.0, k)],
            uv: vec![Vec2::ZERO; 4],
            normal: Vec3::Z,
        };
        // every third side of a 20-sided tube
        let few: Vec<Face> = (0..20).step_by(3).map(side).collect();
        assert!(broken_edges(&few.iter().collect::<Vec<_>>()) > 0);
        // all twenty sides still leave both ends open (20 edges at each): only a capped tube is a solid
        let all: Vec<Face> = (0..20).map(side).collect();
        assert_eq!(broken_edges(&all.iter().collect::<Vec<_>>()), 40);
    }

    /// The prop models the game draws: `props/<set>/<name>` and the interactables (the loose `props/<name>` files are
    /// older rebuilds nothing loads).
    fn drawn_prop_models() -> Vec<(&'static str, serde_json::Value)> {
        all_models()
            .into_iter()
            .filter(|(p, _)| p.starts_with("interactables/") || (p.starts_with("props/") && p.matches('/').count() >= 2))
            .collect()
    }

    /// Every face of every drawn prop reads inside its atlas. The browser build draws on WebGL 1, where a texture whose
    /// size is not a power of two keeps the clamp wrap (prop_model.rs `may_repeat`), so a UV outside 0..1 would smear the
    /// border texel across the face.
    #[test]
    fn every_prop_face_uv_stays_inside_its_atlas() {
        let models = drawn_prop_models();
        assert!(models.len() > 600, "only {} prop models found", models.len());
        for (path, j) in &models {
            let Some(shape) = shape_now(path, j) else { continue };
            for f in shape.parts.iter().flat_map(|p| &p.faces) {
                for uv in &f.uv {
                    let eps = 1e-3;
                    assert!(
                        uv.x.is_finite() && uv.y.is_finite() && uv.x >= -eps && uv.y >= -eps && uv.x <= 1.0 + eps && uv.y <= 1.0 + eps,
                        "{path}: UV {uv} lies outside the atlas"
                    );
                }
            }
        }
    }

    /// A prop is built into 16-bit-indexed meshes by `prop_model::add_verts`, which takes a part whole below 60,000
    /// vertices and splits it above; no model comes near, and every face is a triangle or a quad (the loader fans a
    /// longer loop, which slivers).
    #[test]
    fn no_prop_model_nears_the_16_bit_mesh_limit() {
        for (path, j) in drawn_prop_models() {
            let Some(shape) = shape_now(path, &j) else { continue };
            let faces: Vec<&Face> = shape.parts.iter().flat_map(|p| &p.faces).collect();
            let corners: usize = faces.iter().map(|f| f.corners.len()).sum();
            assert!(corners < 20_000, "{path}: {corners} vertices");
            assert!(faces.iter().all(|f| (3..=4).contains(&f.corners.len())), "{path}: a face is neither a triangle nor a quad");
        }
    }

}
