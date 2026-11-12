//! A small glTF 2.0 binary (`.glb`) reader: just what a static mod-pack prop needs (`PropModel::parse_glb`).
//! It replaces the `gltf` crate (about 25 crates) with serde_json, which the game already has.
//!
//! Read: the scene's node tree with matrix or TRS transforms, triangle primitives, POSITION / NORMAL / TEXCOORD_0 /
//! COLOR_0 and indices in every component type the format allows, the base colour factor and texture.
//! Not read: sparse accessors, skins, animations, morph targets, cameras, extensions. A file that needs one of those
//! for its geometry loads without that part or not at all; nothing here panics on a damaged file.

use serde_json::Value;

pub struct Glb {
    json: Value,
    bin: Option<Vec<u8>>,
}

/// One triangle primitive, read out of its accessors. Attributes the file leaves out are `None`.
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub indices: Option<Vec<u32>>,
    pub base_color: [f32; 4],
    /// Index into the file's images.
    pub image: Option<usize>,
}

/// Where a picture's bytes are.
pub enum ImageSource {
    View { buffer: usize, offset: usize, length: usize },
    Uri(String),
}

fn index(v: &Value) -> Option<usize> {
    v.as_u64().and_then(|n| usize::try_from(n).ok())
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

impl Glb {
    /// The container: a 12-byte header, a JSON chunk, then an optional BIN chunk.
    pub fn parse(bytes: &[u8]) -> Option<Glb> {
        if bytes.get(0..4)? != b"glTF" || le_u32(bytes, 4)? != 2 {
            return None;
        }
        let total = (le_u32(bytes, 8)? as usize).min(bytes.len());
        let mut at = 12;
        let mut json = None;
        let mut bin = None;
        while at + 8 <= total {
            let len = le_u32(bytes, at)? as usize;
            let kind = le_u32(bytes, at + 4)?;
            let data = bytes.get(at + 8..(at + 8).checked_add(len)?)?;
            match kind {
                0x4E4F_534A if json.is_none() => json = Some(serde_json::from_slice::<Value>(data).ok()?),
                0x004E_4942 if bin.is_none() => bin = Some(data.to_vec()),
                _ => {}
            }
            at += 8 + len;
            at += (4 - at % 4) % 4;
        }
        Some(Glb { json: json?, bin })
    }

    fn list(&self, key: &str) -> &[Value] {
        self.json[key].as_array().map(|a| a.as_slice()).unwrap_or(&[])
    }

    /// Every buffer's bytes, in order: the BIN chunk for a buffer with no URI, else `sibling(uri)`.
    pub fn buffers(&self, sibling: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<Vec<Vec<u8>>> {
        self.list("buffers")
            .iter()
            .map(|b| match b["uri"].as_str() {
                Some(uri) => sibling(uri),
                None => self.bin.clone(),
            })
            .collect()
    }

    pub fn images(&self) -> Vec<Option<ImageSource>> {
        self.list("images")
            .iter()
            .map(|img| {
                if let Some(uri) = img["uri"].as_str() {
                    return Some(ImageSource::Uri(uri.to_string()));
                }
                let view = self.list("bufferViews").get(index(&img["bufferView"])?)?;
                Some(ImageSource::View {
                    buffer: index(&view["buffer"])?,
                    offset: index(&view["byteOffset"]).unwrap_or(0),
                    length: index(&view["byteLength"])?,
                })
            })
            .collect()
    }

    /// The default scene's nodes that carry a mesh, each with its world matrix (column-major) and mesh index.
    pub fn mesh_nodes(&self) -> Option<Vec<([f32; 16], usize)>> {
        use macroquad::prelude::{Mat4, Quat, Vec3};
        let scenes = self.list("scenes");
        let scene = scenes.get(index(&self.json["scene"]).unwrap_or(0)).or_else(|| scenes.first())?;
        let nodes = self.list("nodes");
        let roots = scene["nodes"].as_array()?;
        let mut stack: Vec<(usize, Mat4)> = roots.iter().filter_map(index).map(|n| (n, Mat4::IDENTITY)).collect();
        let mut out = vec![];
        // a node tree has no cycles; a damaged file that has one stops at the node count
        let mut budget = nodes.len().saturating_mul(4) + 16;
        while let Some((n, parent)) = stack.pop() {
            if budget == 0 {
                return None;
            }
            budget -= 1;
            let Some(node) = nodes.get(n) else { continue };
            let floats = |key: &str, len: usize| -> Option<Vec<f32>> {
                let a = node[key].as_array()?;
                (a.len() == len).then(|| a.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
            };
            let local = match floats("matrix", 16) {
                Some(m) => Mat4::from_cols_slice(&m),
                None => {
                    let t = floats("translation", 3).map(|v| Vec3::from_slice(&v)).unwrap_or(Vec3::ZERO);
                    let r = floats("rotation", 4).map(|v| Quat::from_slice(&v)).unwrap_or(Quat::IDENTITY);
                    let s = floats("scale", 3).map(|v| Vec3::from_slice(&v)).unwrap_or(Vec3::ONE);
                    Mat4::from_scale_rotation_translation(s, r, t)
                }
            };
            let m = parent * local;
            if let Some(children) = node["children"].as_array() {
                stack.extend(children.iter().filter_map(index).map(|c| (c, m)));
            }
            if let Some(mesh) = index(&node["mesh"]) {
                out.push((m.to_cols_array(), mesh));
            }
        }
        Some(out)
    }

    /// The triangle primitives of one mesh. A primitive without readable positions is left out.
    pub fn primitives(&self, mesh: usize, buffers: &[Vec<u8>]) -> Vec<Primitive> {
        let Some(prims) = self.list("meshes").get(mesh).and_then(|m| m["primitives"].as_array()) else {
            return vec![];
        };
        prims
            .iter()
            .filter(|p| p["mode"].as_u64().unwrap_or(4) == 4)
            .filter_map(|p| {
                let attr = |name: &str| index(&p["attributes"][name]);
                let positions: Vec<[f32; 3]> = self.read(attr("POSITION")?, buffers, false)?;
                let material = index(&p["material"]).and_then(|i| self.list("materials").get(i));
                let pbr = material.map(|m| &m["pbrMetallicRoughness"]);
                let mut base_color = [1.0; 4];
                if let Some(f) = pbr.and_then(|p| p["baseColorFactor"].as_array()).filter(|f| f.len() == 4) {
                    for (k, v) in f.iter().enumerate() {
                        base_color[k] = v.as_f64().unwrap_or(1.0) as f32;
                    }
                }
                let image = pbr
                    .and_then(|p| index(&p["baseColorTexture"]["index"]))
                    .and_then(|t| self.list("textures").get(t))
                    .and_then(|t| index(&t["source"]));
                let colors = attr("COLOR_0").and_then(|a| {
                    // RGB or RGBA; RGB gets alpha 1
                    match self.json["accessors"][a]["type"].as_str()? {
                        "VEC4" => self.read::<4>(a, buffers, true),
                        "VEC3" => self.read::<3>(a, buffers, true).map(|v| v.into_iter().map(|c| [c[0], c[1], c[2], 1.0]).collect()),
                        _ => None,
                    }
                });
                Some(Primitive {
                    positions,
                    normals: attr("NORMAL").and_then(|a| self.read(a, buffers, false)),
                    uvs: attr("TEXCOORD_0").and_then(|a| self.read(a, buffers, true)),
                    colors,
                    indices: match index(&p["indices"]) {
                        Some(a) => Some(self.read_indices(a, buffers)?),
                        None => None,
                    },
                    base_color,
                    image,
                })
            })
            .collect()
    }

    /// An index accessor (unsigned 8, 16 or 32 bit scalars), read as integers so large indices stay exact.
    fn read_indices(&self, accessor: usize, buffers: &[Vec<u8>]) -> Option<Vec<u32>> {
        let acc = self.list("accessors").get(accessor)?;
        if !acc["sparse"].is_null() || acc["type"].as_str()? != "SCALAR" {
            return None;
        }
        let size = match acc["componentType"].as_u64()? {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            _ => return None,
        };
        let count = index(&acc["count"])?;
        let view = self.list("bufferViews").get(index(&acc["bufferView"])?)?;
        let buffer = buffers.get(index(&view["buffer"])?)?;
        let view_start = index(&view["byteOffset"]).unwrap_or(0);
        let view_bytes = buffer.get(view_start..view_start.checked_add(index(&view["byteLength"])?)?)?;
        let start = index(&acc["byteOffset"]).unwrap_or(0);
        let bytes = view_bytes.get(start..start.checked_add(count.checked_mul(size)?)?)?;
        Some(
            bytes
                .chunks_exact(size)
                .map(|b| match size {
                    1 => b[0] as u32,
                    2 => u16::from_le_bytes([b[0], b[1]]) as u32,
                    _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                })
                .collect(),
        )
    }

    /// An accessor as rows of `N` floats. Integer components become floats: scaled to 0..1 (or -1..1) when the
    /// accessor is normalized or `normalize_ints` asks for it (texture coordinates and colours), as they are otherwise.
    fn read<const N: usize>(&self, accessor: usize, buffers: &[Vec<u8>], normalize_ints: bool) -> Option<Vec<[f32; N]>> {
        let acc = self.list("accessors").get(accessor)?;
        if !acc["sparse"].is_null() {
            return None;
        }
        let width = match acc["type"].as_str()? {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => return None,
        };
        if width != N {
            return None;
        }
        let count = index(&acc["count"])?;
        let component = acc["componentType"].as_u64()?;
        let size = match component {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            5125 | 5126 => 4,
            _ => return None,
        };
        let view = self.list("bufferViews").get(index(&acc["bufferView"])?)?;
        let buffer = buffers.get(index(&view["buffer"])?)?;
        let view_start = index(&view["byteOffset"]).unwrap_or(0);
        let view_len = index(&view["byteLength"])?;
        let view_bytes = buffer.get(view_start..view_start.checked_add(view_len)?)?;
        let stride = index(&view["byteStride"]).filter(|s| *s > 0).unwrap_or(size * N);
        let start = index(&acc["byteOffset"]).unwrap_or(0);
        let normalized = normalize_ints || acc["normalized"].as_bool().unwrap_or(false);
        // the count comes from the file: check it against the bytes before reserving anything
        let last = start.checked_add(count.checked_sub(1)?.checked_mul(stride)?)?.checked_add(size * N)?;
        if count == 0 || last > view_bytes.len() {
            return None;
        }
        let mut rows = Vec::with_capacity(count);
        for i in 0..count {
            let mut row = [0.0f32; N];
            for (k, slot) in row.iter_mut().enumerate() {
                let at = start + i * stride + k * size;
                let b = &view_bytes[at..at + size];
                *slot = match component {
                    5120 if normalized => (b[0] as i8 as f32 / 127.0).max(-1.0),
                    5120 => b[0] as i8 as f32,
                    5121 if normalized => b[0] as f32 / 255.0,
                    5121 => b[0] as f32,
                    5122 if normalized => (i16::from_le_bytes([b[0], b[1]]) as f32 / 32767.0).max(-1.0),
                    5122 => i16::from_le_bytes([b[0], b[1]]) as f32,
                    5123 if normalized => u16::from_le_bytes([b[0], b[1]]) as f32 / 65535.0,
                    5123 => u16::from_le_bytes([b[0], b[1]]) as f32,
                    5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                    _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                };
            }
            rows.push(row);
        }
        Some(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GLB with one triangle: float positions, u16 indices, 8-bit normalized colours (RGB), a node moved by
    /// (10, 0, 0) under a parent scaled by 2, and a base colour factor.
    fn triangle() -> Vec<u8> {
        let mut bin = vec![];
        for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for c in v {
                bin.extend_from_slice(&c.to_le_bytes());
            }
        }
        for i in [0u16, 1, 2] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        bin.extend_from_slice(&[0, 0]);
        // three RGB rows, one padding byte each (byteStride 4)
        bin.extend_from_slice(&[255, 0, 0, 9, 0, 255, 0, 9, 0, 0, 255, 9]);
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"scale": [2.0, 2.0, 2.0], "children": [1]}, {"translation": [10.0, 0.0, 0.0], "mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "COLOR_0": 2}, "indices": 1, "material": 0}]}],
            "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [0.5, 1.0, 1.0, 1.0]}}],
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 6},
                {"buffer": 0, "byteOffset": 44, "byteLength": 12, "byteStride": 4}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"},
                {"bufferView": 2, "componentType": 5121, "normalized": true, "count": 3, "type": "VEC3"}
            ]
        });
        let mut text = serde_json::to_vec(&json).unwrap();
        while text.len() % 4 != 0 {
            text.push(b' ');
        }
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + 8 + text.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(text.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        out.extend_from_slice(&text);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(&bin);
        out
    }

    #[test]
    fn reads_a_triangle_with_its_node_transform_colours_and_material() {
        let g = Glb::parse(&triangle()).expect("parses");
        let buffers = g.buffers(&|_| None).expect("the BIN chunk");
        let nodes = g.mesh_nodes().expect("a scene");
        assert_eq!(nodes.len(), 1);
        let (m, mesh) = nodes[0];
        // parent scale 2, then the child's translation: column 3 is (20, 0, 0)
        assert_eq!((m[0], m[5], m[12]), (2.0, 2.0, 20.0));
        let prims = g.primitives(mesh, &buffers);
        assert_eq!(prims.len(), 1);
        let p = &prims[0];
        assert_eq!(p.positions, [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        assert_eq!(p.indices.as_deref(), Some(&[0u32, 1, 2][..]));
        assert_eq!(p.colors.as_deref(), Some(&[[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]][..]));
        assert_eq!(p.base_color, [0.5, 1.0, 1.0, 1.0]);
        assert!(p.normals.is_none() && p.uvs.is_none() && p.image.is_none());
    }

    /// The converter's source models (tools/props/source_glb) are real exporter output: each must read completely.
    #[test]
    fn reads_the_repo_source_models() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/props/source_glb/interactables");
        let mut read = 0;
        for entry in std::fs::read_dir(&dir).expect("the source models folder").flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("glb") {
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let g = Glb::parse(&bytes).unwrap_or_else(|| panic!("{} does not parse", path.display()));
            let buffers = g.buffers(&|_| None).unwrap_or_else(|| panic!("{} has no buffers", path.display()));
            let mut triangles = 0;
            for (_, mesh) in g.mesh_nodes().expect("a scene") {
                for p in g.primitives(mesh, &buffers) {
                    let n = p.positions.len();
                    assert!(n > 0, "{}", path.display());
                    for attr in [p.normals.as_ref().map(|v| v.len()), p.uvs.as_ref().map(|v| v.len()), p.colors.as_ref().map(|v| v.len())] {
                        assert!(attr.map_or(true, |len| len == n), "{}", path.display());
                    }
                    let ix = p.indices.unwrap_or_else(|| (0..n as u32).collect());
                    assert!(ix.iter().all(|i| (*i as usize) < n), "{}", path.display());
                    triangles += ix.len() / 3;
                }
            }
            assert!(triangles > 0, "{} has no triangles", path.display());
            read += 1;
        }
        assert!(read > 0);
    }

    #[test]
    fn damaged_files_are_refused_without_a_panic() {
        let good = triangle();
        assert!(Glb::parse(b"").is_none());
        assert!(Glb::parse(b"glTF\x01\0\0\0").is_none());
        // every truncation either fails to parse or yields no geometry
        for cut in (0..good.len()).step_by(7) {
            if let Some(g) = Glb::parse(&good[..cut]) {
                if let (Some(b), Some(nodes)) = (g.buffers(&|_| None), g.mesh_nodes()) {
                    for (_, mesh) in nodes {
                        let _ = g.primitives(mesh, &b);
                    }
                }
            }
        }
        // an accessor that claims more rows than the bytes hold
        let mut text = String::from_utf8_lossy(&good).into_owned();
        text = text.replacen("\"count\":3", "\"count\":9", 1);
        if let Some(g) = Glb::parse(text.as_bytes()) {
            let b = g.buffers(&|_| None).unwrap_or_default();
            for (_, mesh) in g.mesh_nodes().unwrap_or_default() {
                assert!(g.primitives(mesh, &b).iter().all(|p| p.positions.len() <= 3));
            }
        }
    }
}
