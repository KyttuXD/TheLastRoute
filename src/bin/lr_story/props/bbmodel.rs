//! Blockbench (.bbmodel, "free" format) reader for the hunters and the room props: cube
//! elements with per-face atlas UVs (hunters) and mesh elements (props rebuilt from the
//! original's blocky GLBs). Hunters are posed by their rig (outliner groups = bones, see render/anim:
//! `build_posed`); `build` keeps the old whole-limb swing for a model without one.

use super::LIT_MODE;
use macroquad::prelude::*;

/// A PNG as a nearest-filtered texture, or None when it does not decode (the copy of `shared::assets::try_tex`).
pub fn try_tex(bytes: &[u8]) -> Option<Texture2D> {
    let img = Image::from_file_with_format(bytes, Some(ImageFormat::Png)).ok()?;
    let t = Texture2D::from_image(&img);
    t.set_filter(FilterMode::Nearest);
    Some(t)
}

/// A small standard base64 decoder for textures embedded in .bbmodel files (the copy of `content::embedded`'s).
pub fn base64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' | b'-' => Some(62),
            b'/' | b'_' => Some(63),
            _ => None,
        }
    };
    let mut out = vec![];
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let Some(v) = val(c) else { continue };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    out
}

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
    try_tex(&bytes)
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
            .or_else(|| j["resolution"][res_key].as_f64().filter(|v| *v > 0.0))
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
                        (fj["rotation"].as_f64().unwrap_or(0.0) / 90.0).round().rem_euclid(4.0) as usize % 4;
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
        self.build_posed_mode(out, idx, root, bone_of, face_uv, tint, LIT_MODE);
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

    // The hunter golden digests and the void-model tests read the game's embedded hunters, which are not in lr_story's
    // assets, so they stay with the game (render/bbmodel.rs in last_route). The tests kept and added here read the
    // prop files shipped with lr_story and need no graphics context.

    /// The shipped prop folder of this crate (lr_story's assets/props).
    fn props_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/lr_story/assets/props")
    }

    /// A shipped prop read the way the loader reads it: its geometry from the JSON, its UV space from the embedded
    /// texture's picture (decoded on the CPU, so no GL context is needed).
    fn shipped_shape(rel: &str) -> Shape {
        let path = props_dir().join(rel);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let j: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let no_file = |_: &str| None::<Vec<u8>>;
        let (_, bytes) = texture_bytes(&j, &no_file).unwrap_or_else(|| panic!("{rel}: no texture"));
        let img = Image::from_file_with_format(&bytes, Some(ImageFormat::Png))
            .ok()
            .unwrap_or_else(|| panic!("{rel}: its texture does not decode"));
        let (w, h) = uv_space(&j, img.width() as f32, img.height() as f32);
        read_shape(&j, w, h).unwrap_or_else(|| panic!("{rel}: no drawable element"))
    }

    /// A blocky lab prop (labStool) and a maze office prop (officeDesk, shipped as office_desk.bbmodel) each read into
    /// drawable cubes with faces.
    #[test]
    fn shipped_lab_stool_and_office_desk_read_into_cubes() {
        for rel in ["blocky/labStool_0.bbmodel", "maze/office_desk.bbmodel"] {
            let shape = shipped_shape(rel);
            let faces: usize = shape.parts.iter().map(|p| p.faces.len()).sum();
            assert!(!shape.parts.is_empty(), "{rel}: no cube or mesh element");
            assert!(faces > 0, "{rel}: no faces");
        }
    }

    /// Build order 7: one lab prop (assets/props/blocky) and one maze prop (assets/props/maze) parse into the bones and
    /// elements their files hold. Both are flat props (no outliner groups), so they have no bones. The labStool is four
    /// cubes and one mesh; the officeDesk is twelve cubes.
    #[test]
    fn a_lab_prop_and_a_maze_prop_parse_to_their_bone_and_cube_counts() {
        for (rel, bones, cubes, others) in [("blocky/labStool_0.bbmodel", 0usize, 4usize, 1usize), ("maze/office_desk.bbmodel", 0, 12, 0)] {
            let text = std::fs::read_to_string(props_dir().join(rel)).unwrap();
            let j: serde_json::Value = serde_json::from_str(&text).unwrap();
            let kind = |t: &str| j["elements"].as_array().unwrap().iter().filter(|e| e["type"].as_str() == Some(t)).count();
            assert_eq!((kind("cube"), kind("mesh")), (cubes, others), "{rel}: the file's own element counts changed");
            let shape = shipped_shape(rel);
            assert_eq!(shape.groups.len(), bones, "{rel}: bones");
            assert_eq!(shape.parts.len(), cubes + others, "{rel}: one part per element");
            let cube_parts = shape.parts.iter().filter(|p| !p.faces.is_empty()).count();
            assert!(cube_parts >= cubes, "{rel}: {cube_parts} drawable parts for {cubes} cubes");
            assert!(shape.min.is_finite() && shape.max.is_finite() && shape.min.cmplt(shape.max).all(), "{rel}: bounds");
        }
    }

    /// Prop placement (build order 8): the shipped blocky labStool at the draw scale the copied table gives it (1.5).
    /// PropModel::parse makes a texture, which needs a graphics context, so this checks the CPU side: the corners of the
    /// parsed faces, scaled the way parse scales them (1/16 metre per unit for a prop) and then by the draw scale. Every
    /// vertex must be finite, the box non-empty on each axis, and under 5 m on each axis.
    #[test]
    fn lab_stool_at_its_draw_scale_is_finite_and_under_five_metres() {
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/lr_story/assets");
        let table = crate::props::prop_scale::table(&assets);
        let draw = crate::props::prop_scale::draw_scale(&table, "labStool");
        assert!((draw - 1.5).abs() < 1e-6, "labStool draws at {draw}, not 1.5");
        let shape = shipped_shape("blocky/labStool_0.bbmodel");
        let metres = (1.0 / 16.0) * draw;
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        let mut vertices = 0usize;
        for part in &shape.parts {
            for face in &part.faces {
                for corner in &face.corners {
                    let p = *corner * metres;
                    assert!(p.is_finite(), "labStool vertex {p:?} is not finite");
                    lo = lo.min(p);
                    hi = hi.max(p);
                    vertices += 1;
                }
            }
        }
        assert!(vertices > 0, "labStool has no vertices");
        let size = hi - lo;
        for (axis, extent) in ["x", "y", "z"].iter().zip(size.to_array()) {
            assert!(extent > 0.0, "labStool box is empty along {axis}: {extent}");
            assert!(extent < 5.0, "labStool is {extent} m along {axis} after scaling");
        }
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

    #[test]
    fn base64_decodes_a_data_uri_picture_to_its_png_bytes() {
        // "iVBORw0K" is the PNG signature's base64 start
        let bytes = base64_decode("iVBORw0KGgo=");
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    }
}
