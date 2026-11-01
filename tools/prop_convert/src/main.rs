//! prop_convert: blocky prop GLBs (Godot project) -> Blockbench `.bbmodel` files.
//!
//! Usage: prop_convert <godot_project_dir> <out_dir>
//!
//! Reads `game/config/blocky_prop_models.gd`, takes the first GLB of every type
//! (variant `_0` or the single file), and writes `<type>.bbmodel`, the shared atlas
//! (`blocky_atlas.png`) and `manifest.json` into `out_dir`.
//!
//! Geometry becomes Blockbench "mesh" elements (one per glTF primitive), in prop space
//! (node transforms applied), 16 Blockbench units per metre, UVs in atlas pixels.
//! Triangle pairs that came from one quad are merged back into quads.

use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const UNITS_PER_METRE: f64 = 16.0;
const ATLAS_FILE: &str = "blocky_atlas.png";

type Mat4 = [[f32; 4]; 4]; // column-major, as glTF

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: prop_convert <godot_project_dir> <out_dir>");
        std::process::exit(2);
    }
    let project = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let table = std::fs::read_to_string(project.join("game/config/blocky_prop_models.gd"))
        .expect("read blocky_prop_models.gd");
    let types = parse_model_table(&table);

    let mut manifest = Map::new();
    let mut shared_atlas: Option<Vec<u8>> = None;
    let mut failures = Vec::new();
    let mut notes = Vec::new();

    for (type_name, res_path) in &types {
        let rel = res_path.trim_start_matches("res://");
        match convert(&project.join(rel), type_name) {
            Ok(prop) => {
                if prop.image_count != 1 {
                    notes.push(format!("{type_name}: {} images in GLB", prop.image_count));
                }
                // One atlas file for everything if every GLB embeds the same PNG;
                // otherwise the odd one out gets its own `<type>_atlas.png`.
                let atlas_name = match &shared_atlas {
                    None => {
                        shared_atlas = Some(prop.png.clone());
                        ATLAS_FILE.to_string()
                    }
                    Some(a) if *a == prop.png => ATLAS_FILE.to_string(),
                    Some(_) => {
                        notes.push(format!("{type_name}: atlas differs from the shared one"));
                        let name = format!("{type_name}_atlas.png");
                        std::fs::write(out_dir.join(&name), &prop.png).unwrap();
                        name
                    }
                };
                let file = format!("{type_name}.bbmodel");
                let model = prop.to_bbmodel(type_name, &atlas_name);
                std::fs::write(out_dir.join(&file), serde_json::to_string(&model).unwrap())
                    .unwrap();
                manifest.insert(
                    type_name.clone(),
                    json!({ "file": file, "source": rel, "min": prop.min, "max": prop.max }),
                );
                println!(
                    "{type_name:32} {:4} verts {:4} faces  <- {rel}",
                    prop.vertex_count(),
                    prop.face_count()
                );
            }
            Err(e) => failures.push(format!("{type_name} ({rel}): {e}")),
        }
    }

    if let Some(png) = &shared_atlas {
        std::fs::write(out_dir.join(ATLAS_FILE), png).unwrap();
    }
    std::fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_string_pretty(&Value::Object(manifest)).unwrap(),
    )
    .unwrap();

    println!("\nconverted {} of {} types", types.len() - failures.len(), types.len());
    for n in &notes {
        println!("note: {n}");
    }
    for f in &failures {
        println!("FAILED: {f}");
    }
}

/// `"type": ["res://a_0.glb", ...],` lines -> (type, first path), in file order.
fn parse_model_table(src: &str) -> Vec<(String, String)> {
    src.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            let name = quoted.next()?;
            let first = quoted.next()?;
            first.starts_with("res://").then(|| (name.to_string(), first.to_string()))
        })
        .collect()
}

/// One converted face: 3 or 4 vertex indices (into `Element::positions`), CCW, with UVs.
struct Face {
    verts: Vec<usize>,
    uvs: Vec<[f64; 2]>,
}

struct Element {
    name: String,
    positions: Vec<[f64; 3]>, // metres, prop space, deduplicated
    faces: Vec<Face>,
}

struct Prop {
    elements: Vec<Element>,
    png: Vec<u8>,
    width: u32,
    height: u32,
    image_count: usize,
    min: [f64; 3],
    max: [f64; 3],
}

fn convert(glb: &Path, type_name: &str) -> Result<Prop, String> {
    let bytes = std::fs::read(glb).map_err(|e| e.to_string())?;
    let gltf = gltf::Gltf::from_slice(&bytes).map_err(|e| e.to_string())?;
    let buffers = gltf::import_buffers(&gltf.document, glb.parent(), gltf.blob.clone())
        .map_err(|e| e.to_string())?;
    let doc = &gltf.document;

    // The atlas: first image, embedded in a buffer view as PNG.
    let image = doc.images().next().ok_or("no image")?;
    let png = match image.source() {
        gltf::image::Source::View { view, mime_type } if mime_type == "image/png" => {
            let buf = &buffers[view.buffer().index()];
            buf[view.offset()..view.offset() + view.length()].to_vec()
        }
        _ => return Err("image is not an embedded PNG".into()),
    };
    let (width, height) = png_size(&png).ok_or("bad PNG header")?;

    let mut elements = Vec::new();
    let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or("no scene")?;
    for node in scene.nodes() {
        collect_node(&node, IDENTITY, &buffers, width, height, type_name, &mut elements)?;
    }
    if elements.is_empty() {
        return Err("no triangle geometry".into());
    }

    let mut min = [f64::MAX; 3];
    let mut max = [f64::MIN; 3];
    for p in elements.iter().flat_map(|e| e.positions.iter()) {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    Ok(Prop {
        elements,
        png,
        width,
        height,
        image_count: doc.images().count(),
        min: min.map(round4),
        max: max.map(round4),
    })
}

const IDENTITY: Mat4 = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.], [0., 0., 0., 1.]];

fn collect_node(
    node: &gltf::Node,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    w: u32,
    h: u32,
    type_name: &str,
    out: &mut Vec<Element>,
) -> Result<(), String> {
    let world = mat_mul(&parent, &node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        let base = node.name().or(mesh.name()).unwrap_or(type_name).to_string();
        let prim_count = mesh.primitives().count();
        for (pi, prim) in mesh.primitives().enumerate() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                return Err(format!("primitive mode {:?} not supported", prim.mode()));
            }
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            let pos: Vec<[f64; 3]> = reader
                .read_positions()
                .ok_or("primitive without POSITION")?
                .map(|p| transform_point(&world, p))
                .collect();
            let uv: Vec<[f64; 2]> = match reader.read_tex_coords(0) {
                Some(t) => t
                    .into_f32()
                    .map(|[u, v]| [round4(u as f64 * w as f64), round4(v as f64 * h as f64)])
                    .collect(),
                None => vec![[0.0, 0.0]; pos.len()],
            };
            let idx: Vec<usize> = match reader.read_indices() {
                Some(i) => i.into_u32().map(|i| i as usize).collect(),
                None => (0..pos.len()).collect(),
            };
            let flip = det3(&world) < 0.0; // mirrored transform reverses winding
            let tris: Vec<[usize; 3]> = idx
                .chunks_exact(3)
                .map(|t| if flip { [t[0], t[2], t[1]] } else { [t[0], t[1], t[2]] })
                .collect();
            // Sanity check: the winding (Blockbench derives face normals from it) must
            // agree with the GLB's own normals.
            if let Some(normals) = reader.read_normals() {
                let n: Vec<[f64; 3]> = normals.map(|v| transform_dir(&world, v)).collect();
                let bad = tris
                    .iter()
                    .filter(|t| dot(tri_normal(**t, &pos), n[t[0]]) < 0.0)
                    .count();
                if bad > 0 {
                    eprintln!("warning: {type_name}: {bad} triangles wound against their normals");
                }
            }
            let name = if prim_count > 1 { format!("{base}_{pi}") } else { base.clone() };
            let el = build_element(name, &pos, &uv, &tris);
            let kept: usize = el.faces.iter().map(|f| f.verts.len() - 2).sum();
            if kept != tris.len() {
                eprintln!("warning: {type_name}: dropped {} degenerate triangles", tris.len() - kept);
            }
            out.push(el);
        }
    }
    for child in node.children() {
        collect_node(&child, world, buffers, w, h, type_name, out)?;
    }
    Ok(())
}

/// Merges consecutive triangle pairs that share an edge and a plane (Blender's quad
/// split) into quads, and deduplicates positions so Blockbench sees a connected mesh.
fn build_element(name: String, pos: &[[f64; 3]], uv: &[[f64; 2]], tris: &[[usize; 3]]) -> Element {
    let mut faces_idx: Vec<Vec<usize>> = Vec::new();
    let mut i = 0;
    while i < tris.len() {
        if i + 1 < tris.len() {
            if let Some(quad) = merge_quad(tris[i], tris[i + 1], pos) {
                faces_idx.push(quad);
                i += 2;
                continue;
            }
        }
        faces_idx.push(tris[i].to_vec());
        i += 1;
    }

    // Dedupe by rounded position.
    let mut key_of: BTreeMap<[i64; 3], usize> = BTreeMap::new();
    let mut positions = Vec::new();
    let mut remap = vec![0usize; pos.len()];
    for (gi, p) in pos.iter().enumerate() {
        let k = p.map(|c| (c * 1e4).round() as i64);
        remap[gi] = *key_of.entry(k).or_insert_with(|| {
            positions.push(p.map(round4));
            positions.len() - 1
        });
    }
    let faces = faces_idx
        .into_iter()
        .filter(|f| f.iter().map(|&g| remap[g]).collect::<std::collections::BTreeSet<_>>().len() == f.len())
        .map(|f| Face { verts: f.iter().map(|&g| remap[g]).collect(), uvs: f.iter().map(|&g| uv[g]).collect() })
        .collect();
    Element { name, positions, faces }
}

/// Two triangles sharing an edge with the same normal -> quad loop (u, s1, w, s2).
fn merge_quad(a: [usize; 3], b: [usize; 3], pos: &[[f64; 3]]) -> Option<Vec<usize>> {
    let shared: Vec<usize> = a.iter().copied().filter(|v| b.contains(v)).collect();
    if shared.len() != 2 {
        return None;
    }
    let (na, nb) = (tri_normal(a, pos), tri_normal(b, pos));
    if dot(na, nb) < 0.9999 {
        return None;
    }
    // Rotate `a` so its unshared vertex comes first: (u, s1, s2); the other
    // triangle's unshared vertex goes between s1 and s2.
    let k = a.iter().position(|v| !b.contains(v))?;
    let (u, s1, s2) = (a[k], a[(k + 1) % 3], a[(k + 2) % 3]);
    let w = *b.iter().find(|v| !a.contains(v))?;
    Some(vec![u, s1, w, s2])
}

impl Prop {
    fn vertex_count(&self) -> usize {
        self.elements.iter().map(|e| e.positions.len()).sum()
    }
    fn face_count(&self) -> usize {
        self.elements.iter().map(|e| e.faces.len()).sum()
    }

    fn to_bbmodel(&self, type_name: &str, atlas_name: &str) -> Value {
        let mut elements = Vec::new();
        let mut outliner = Vec::new();
        for (ei, el) in self.elements.iter().enumerate() {
            let uuid = stable_uuid(&format!("{type_name}/element/{ei}"));
            let vkey = |i: usize| format!("v{i}");
            let vertices: Map<String, Value> = el
                .positions
                .iter()
                .enumerate()
                .map(|(i, p)| (vkey(i), json!(p.map(|c| round4(c * UNITS_PER_METRE)))))
                .collect();
            let faces: Map<String, Value> = el
                .faces
                .iter()
                .enumerate()
                .map(|(fi, f)| {
                    let uv: Map<String, Value> =
                        f.verts.iter().zip(&f.uvs).map(|(&v, uv)| (vkey(v), json!(uv))).collect();
                    let verts: Vec<String> = f.verts.iter().map(|&v| vkey(v)).collect();
                    (format!("f{fi}"), json!({ "uv": uv, "vertices": verts, "texture": 0 }))
                })
                .collect();
            elements.push(json!({
                "name": el.name,
                "color": ei % 8,
                "origin": [0, 0, 0],
                "rotation": [0, 0, 0],
                "export": true,
                "visibility": true,
                "locked": false,
                "render_order": "default",
                "allow_mirror_modeling": true,
                "vertices": vertices,
                "faces": faces,
                "type": "mesh",
                "uuid": uuid,
            }));
            outliner.push(json!(uuid));
        }
        let source = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&self.png)
        );
        json!({
            "meta": { "format_version": "4.10", "model_format": "free", "box_uv": false },
            "name": type_name,
            "model_identifier": "",
            "visible_box": [1, 1, 0],
            "variable_placeholders": "",
            "variable_placeholder_buttons": [],
            "timeline_setups": [],
            "unhandled_root_fields": {},
            "resolution": { "width": self.width, "height": self.height },
            "elements": elements,
            "outliner": outliner,
            "textures": [{
                "path": "",
                "name": atlas_name,
                "folder": "",
                "namespace": "",
                "id": "0",
                "group": "",
                "width": self.width,
                "height": self.height,
                "uv_width": self.width,
                "uv_height": self.height,
                "particle": false,
                "use_as_default": false,
                "layers_enabled": false,
                "sync_to_project": "",
                "render_mode": "default",
                "render_sides": "auto",
                "frame_time": 1,
                "frame_order_type": "loop",
                "frame_order": "",
                "frame_interpolate": false,
                "visible": true,
                "internal": true,
                "saved": true,
                "uuid": stable_uuid(&format!("{type_name}/texture/{atlas_name}")),
                "relative_path": atlas_name,
                "source": source,
            }],
        })
    }
}

// --- small helpers ---------------------------------------------------------------

fn stable_uuid(name: &str) -> String {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, format!("lastroute-props:{name}").as_bytes())
        .to_string()
}

fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[1..4] != b"PNG" || &png[12..16] != b"IHDR" {
        return None;
    }
    let be = |o: usize| u32::from_be_bytes(png[o..o + 4].try_into().unwrap());
    Some((be(16), be(20)))
}

fn round4(x: f64) -> f64 {
    let r = (x * 1e4).round() / 1e4;
    if r == 0.0 { 0.0 } else { r } // no "-0.0"
}

fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut m = [[0f32; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            m[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    m
}

fn transform_point(m: &Mat4, p: [f32; 3]) -> [f64; 3] {
    let mut o = [0f64; 3];
    for r in 0..3 {
        o[r] = (m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]) as f64;
    }
    o
}

/// Direction through the linear part only (fine for rotation + uniform scale).
fn transform_dir(m: &Mat4, d: [f32; 3]) -> [f64; 3] {
    let mut o = [0f64; 3];
    for r in 0..3 {
        o[r] = (m[0][r] * d[0] + m[1][r] * d[1] + m[2][r] * d[2]) as f64;
    }
    o
}

fn det3(m: &Mat4) -> f32 {
    m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2]) - m[1][0] * (m[0][1] * m[2][2] - m[2][1] * m[0][2])
        + m[2][0] * (m[0][1] * m[1][2] - m[1][1] * m[0][2])
}

fn tri_normal(t: [usize; 3], pos: &[[f64; 3]]) -> [f64; 3] {
    let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let (e1, e2) = (sub(pos[t[1]], pos[t[0]]), sub(pos[t[2]], pos[t[0]]));
    let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    let len = dot(n, n).sqrt().max(1e-12);
    n.map(|c| c / len)
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
