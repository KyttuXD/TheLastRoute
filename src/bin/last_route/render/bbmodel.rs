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
    let tex = Texture2D::from_file_with_format(&bytes, Some(ImageFormat::Png));
    tex.set_filter(FilterMode::Nearest);
    Some(tex)
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
        let res_w = j["resolution"]["width"]
            .as_f64()
            .unwrap_or(texture.width() as f64) as f32;
        let res_h = j["resolution"]["height"]
            .as_f64()
            .unwrap_or(texture.height() as f64) as f32;
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
                        normal: vec4(n.x, n.y, n.z, 1.0),
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
