//! Shaping the floor's meshes for the draw loop: splitting a big mesh into spatial tiles, so a culling box around each
//! piece is tight enough to drop whole pieces the camera does not see, and ordering meshes so that macroquad's batcher
//! (which joins consecutive `draw_mesh` calls with the same texture into one draw call) joins as many as it can.
//! Both are pure functions of the meshes; nothing here draws.

use macroquad::prelude::*;
use std::collections::BTreeMap;

/// Metres on a side of one culling tile. A window chunk is 45 cells wide (`world::infinite::CHUNK`), so 15 m cuts a chunk
/// into 3 x 3 tiles that line up with its edges.
pub const TILE_M: f32 = 15.0;

/// The tile size the floor's meshes are cut to: `TILE_M`, or 0 (no cut) when `LR_NO_MESH_TILES` is set in the environment of a
/// native build, which lets the performance panel's counters be compared with and without the cut. The browser build has no
/// environment and always cuts.
pub fn tile_size() -> f32 {
    static SIZE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| if std::env::var_os("LR_NO_MESH_TILES").is_some() { 0.0 } else { TILE_M })
}

/// Splits `mesh` into one mesh per `tile`-metre square (on the ground plane) that holds the centre of at least one of its
/// triangles. Every triangle lands in exactly one piece with its three vertices unchanged, so the pieces together draw
/// the original; a triangle that straddles a tile edge goes whole to the tile of its centre, and so its piece's box grows
/// past the tile by its overhang. The pieces come in tile order, and a mesh that sits in one tile comes back as it was.
pub fn split_into_tiles(mesh: Mesh, tile: f32) -> Vec<Mesh> {
    if mesh.indices.len() < 3 || !(tile > 0.0) || !tile.is_finite() {
        return vec![mesh];
    }
    let key_of = |t: &[u16]| -> (i32, i32) {
        let p = |k: usize| mesh.vertices[t[k] as usize].position;
        let c = (p(0) + p(1) + p(2)) / 3.0;
        ((c.x / tile).floor() as i32, (c.z / tile).floor() as i32)
    };
    // the tiles in order, each with its running piece
    let mut tiles: BTreeMap<(i32, i32), usize> = BTreeMap::new();
    let mut keys: Vec<(i32, i32)> = Vec::with_capacity(mesh.indices.len() / 3);
    for t in mesh.indices.chunks_exact(3) {
        let k = key_of(t);
        keys.push(k);
        tiles.entry(k).or_insert(0);
    }
    if tiles.len() <= 1 {
        return vec![mesh];
    }
    for (n, slot) in tiles.values_mut().enumerate() {
        *slot = n;
    }
    struct Piece {
        vertices: Vec<Vertex>,
        indices: Vec<u16>,
    }
    let mut pieces: Vec<Piece> = (0..tiles.len()).map(|_| Piece { vertices: vec![], indices: vec![] }).collect();
    // for each source vertex: the piece it was last copied into and its index there
    let mut copied: Vec<(u32, u16)> = vec![(u32::MAX, 0); mesh.vertices.len()];
    for (t, k) in mesh.indices.chunks_exact(3).zip(&keys) {
        let n = tiles[k];
        let piece = &mut pieces[n];
        for &i in t {
            let slot = &mut copied[i as usize];
            if slot.0 != n as u32 {
                *slot = (n as u32, piece.vertices.len() as u16);
                piece.vertices.push(mesh.vertices[i as usize]);
            }
            piece.indices.push(slot.1);
        }
    }
    pieces
        .into_iter()
        .map(|mut p| {
            // the pieces live as long as the chunk: give back the spare capacity their growth left (about a quarter)
            p.vertices.shrink_to_fit();
            p.indices.shrink_to_fit();
            Mesh { vertices: p.vertices, indices: p.indices, texture: mesh.texture.clone() }
        })
        .collect()
}

/// Whether a mesh draws with blending: any vertex in a mode past the unlit one (the shader's mode 3 and the ghost mode 4
/// write their alpha). Every other mode writes alpha 1, so its draw order against other opaque meshes never shows.
pub fn is_blended(mesh: &Mesh) -> bool {
    mesh.vertices.iter().any(|v| v.normal.w > 2.5)
}

/// The order to draw meshes in, as indexes into the input. `texture[i]` identifies mesh i's texture and `blended[i]`
/// says whether it blends. Opaque meshes come first, grouped by texture so the batcher joins each group into one draw
/// call (groups in order of first appearance, input order kept inside a group); blended meshes follow in their input
/// order, so their blending order is the one they were built in.
pub fn draw_order(texture: &[usize], blended: &[bool]) -> Vec<usize> {
    debug_assert_eq!(texture.len(), blended.len());
    let mut first_seen: Vec<usize> = vec![];
    let group: Vec<usize> = texture
        .iter()
        .map(|t| match first_seen.iter().position(|s| s == t) {
            Some(g) => g,
            None => {
                first_seen.push(*t);
                first_seen.len() - 1
            }
        })
        .collect();
    let mut order: Vec<usize> = (0..texture.len()).collect();
    // a stable sort: blended ones last, opaque ones by group, everything else in input order
    order.sort_by_key(|&i| if blended[i] { (1, 0) } else { (0, group[i]) });
    order
}

/// `items` (a mesh each, with whatever travels with it) in `draw_order`.
pub fn group_for_batching<K>(items: Vec<(K, Mesh)>) -> Vec<(K, Mesh)> {
    let mut textures: Vec<Option<Texture2D>> = vec![];
    let ids: Vec<usize> = items
        .iter()
        .map(|(_, m)| match textures.iter().position(|t| *t == m.texture) {
            Some(i) => i,
            None => {
                textures.push(m.texture.clone());
                textures.len() - 1
            }
        })
        .collect();
    let blended: Vec<bool> = items.iter().map(|(_, m)| is_blended(m)).collect();
    let order = draw_order(&ids, &blended);
    let mut slots: Vec<Option<(K, Mesh)>> = items.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| slots[i].take()).collect()
}

/// The most vertices one merged mesh holds: the 16-bit index limit (indices 0..=65534).
pub const MAX_MERGED_VERTICES: usize = 65535;

/// Joins `meshes` that sit next to each other in the list, share a texture and the same blending, into one mesh each,
/// starting a new mesh where the vertex count would pass `max_vertices` (so indices stay in 16 bits). Vertices and
/// triangles keep their order, so the drawn image is the same; only the number of meshes (and so of draw calls the
/// caller makes) drops. A mesh that is alone or already past the limit comes through unchanged.
pub fn merge_adjacent(meshes: Vec<Mesh>, max_vertices: usize) -> Vec<Mesh> {
    let max_vertices = max_vertices.min(MAX_MERGED_VERTICES);
    let mut out: Vec<Mesh> = Vec::with_capacity(meshes.len());
    let mut last_blended = false;
    for m in meshes {
        if m.indices.is_empty() {
            continue;
        }
        let blended = is_blended(&m);
        if let Some(cur) = out.last_mut() {
            if cur.texture == m.texture
                && last_blended == blended
                && cur.vertices.len() + m.vertices.len() <= max_vertices
            {
                let base = cur.vertices.len() as u16;
                cur.vertices.extend_from_slice(&m.vertices);
                cur.indices.extend(m.indices.iter().map(|&i| i + base));
                continue;
            }
        }
        last_blended = blended;
        out.push(m);
    }
    out
}

/// `group_for_batching` and then `merge_adjacent`: opaque meshes grouped by texture and joined, blended ones joined only
/// where they were already neighbours of one texture, so the blending order is kept.
pub fn merge_for_draw(meshes: Vec<Mesh>) -> Vec<Mesh> {
    let items: Vec<((), Mesh)> = meshes.into_iter().map(|m| ((), m)).collect();
    let grouped: Vec<Mesh> = group_for_batching(items).into_iter().map(|(_, m)| m).collect();
    merge_adjacent(grouped, MAX_MERGED_VERTICES)
}

/// A model of macroquad's batcher (`QuadGl::geometry`), which joins consecutive `draw_mesh` calls into one draw call:
/// a new call starts where the texture changes, where the running call would pass `max_vertices` or `max_indices`, or
/// after a flush (`reset`; a camera change flushes). Everything else the real batcher compares (the pipeline, the model
/// matrix, the clip) is the same for the floor's meshes. The draw-call counter of the performance overlay runs on it.
pub struct BatchModel<T> {
    last: Option<T>,
    vertices: usize,
    indices: usize,
    max_vertices: usize,
    max_indices: usize,
    /// Draw calls started since the model was made or `clear`ed.
    pub calls: usize,
}

impl<T: PartialEq + Clone> BatchModel<T> {
    pub fn new(max_vertices: usize, max_indices: usize) -> Self {
        BatchModel { last: None, vertices: 0, indices: 0, max_vertices, max_indices, calls: 0 }
    }

    /// Adds a mesh of `nv` vertices and `ni` indices with texture `tex`; true when it starts a new draw call.
    pub fn push(&mut self, tex: &T, nv: usize, ni: usize) -> bool {
        let new_call = self.last.as_ref() != Some(tex)
            || self.vertices + nv >= self.max_vertices.max(nv + 1)
            || self.indices + ni >= self.max_indices.max(ni + 1);
        if new_call {
            self.calls += 1;
            if self.last.as_ref() != Some(tex) {
                self.last = Some(tex.clone());
            }
            self.vertices = 0;
            self.indices = 0;
        }
        self.vertices += nv;
        self.indices += ni;
        new_call
    }

    /// A flush ends the running call: the next mesh starts a new one whatever its texture.
    pub fn reset(&mut self) {
        self.last = None;
        self.vertices = 0;
        self.indices = 0;
    }

    /// Starts the count again (and the running call).
    pub fn clear(&mut self) {
        self.reset();
        self.calls = 0;
    }
}

/// How many draw calls macroquad's batcher makes of `meshes` drawn in order, each given as (texture id, vertices,
/// indices); `None` is no texture.
pub fn batches(meshes: &[(Option<usize>, usize, usize)], max_vertices: usize, max_indices: usize) -> usize {
    let mut model = BatchModel::new(max_vertices, max_indices);
    for &(tex, nv, ni) in meshes {
        model.push(&tex, nv, ni);
    }
    model.calls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vert(x: f32, z: f32, tag: u8) -> Vertex {
        Vertex { position: vec3(x, 1.0, z), uv: vec2(x, z), color: [tag, 0, 0, 255], normal: vec4(0.0, 1.0, 0.0, 1.0) }
    }

    /// One triangle at (x, z), three vertices tagged `tag`.
    fn tri(x: f32, z: f32, tag: u8) -> [Vertex; 3] {
        [vert(x, z, tag), vert(x + 1.0, z, tag), vert(x, z + 1.0, tag)]
    }

    fn mesh_of(tris: &[[Vertex; 3]]) -> Mesh {
        let mut m = Mesh { vertices: vec![], indices: vec![], texture: None };
        for t in tris {
            let base = m.vertices.len() as u16;
            m.vertices.extend_from_slice(t);
            m.indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
        m
    }

    /// A mesh's triangles as (corner positions, tag) in a sorted list, to compare meshes whatever their order.
    fn triangles(m: &Mesh) -> Vec<(Vec<[i32; 3]>, u8)> {
        let mut out: Vec<(Vec<[i32; 3]>, u8)> = m
            .indices
            .chunks_exact(3)
            .map(|t| {
                let corners: Vec<[i32; 3]> = t
                    .iter()
                    .map(|&i| {
                        let p = m.vertices[i as usize].position;
                        [(p.x * 100.0) as i32, (p.y * 100.0) as i32, (p.z * 100.0) as i32]
                    })
                    .collect();
                (corners, m.vertices[t[0] as usize].color[0])
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn a_mesh_inside_one_tile_comes_back_whole() {
        let m = mesh_of(&[tri(1.0, 1.0, 1), tri(3.0, 4.0, 2)]);
        let pieces = split_into_tiles(m, 15.0);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].indices.len(), 6);
    }

    #[test]
    fn a_wide_mesh_splits_into_tiles_and_keeps_every_triangle_once() {
        let tris = [tri(1.0, 1.0, 1), tri(20.0, 1.0, 2), tri(1.0, 40.0, 3), tri(21.0, 2.0, 4), tri(-10.0, -10.0, 5)];
        let whole = mesh_of(&tris);
        let want = triangles(&whole);
        let pieces = split_into_tiles(whole, 15.0);
        // tiles: (0,0), (1,0) holding two, (0,2) and (-1,-1): four pieces
        assert_eq!(pieces.len(), 4);
        let mut got: Vec<(Vec<[i32; 3]>, u8)> = pieces.iter().flat_map(triangles).collect();
        got.sort();
        assert_eq!(got, want, "the pieces together hold exactly the original triangles");
        for p in &pieces {
            assert!(p.indices.iter().all(|&i| (i as usize) < p.vertices.len()));
        }
    }

    #[test]
    fn each_piece_lies_in_its_tile_but_for_the_overhang_of_its_triangles() {
        // a triangle whose centre is in tile 0 but which reaches 3 m into tile 1
        let long = [vert(12.0, 1.0, 9), vert(18.0, 1.0, 9), vert(12.0, 4.0, 9)];
        let m = mesh_of(&[long, tri(30.5, 1.0, 1)]);
        let pieces = split_into_tiles(m, 15.0);
        assert_eq!(pieces.len(), 2);
        let max_x = |p: &Mesh| p.vertices.iter().map(|v| v.position.x).fold(f32::MIN, f32::max);
        assert_eq!(max_x(&pieces[0]), 18.0, "the straddling triangle stays whole in the tile of its centre");
        assert_eq!(max_x(&pieces[1]), 31.5);
    }

    #[test]
    fn shared_vertices_are_kept_shared_inside_a_piece() {
        // a quad as two triangles sharing an edge, plus a far quad
        let mut m = Mesh {
            vertices: vec![vert(0.0, 0.0, 1), vert(1.0, 0.0, 1), vert(1.0, 1.0, 1), vert(0.0, 1.0, 1)],
            indices: vec![0, 1, 2, 0, 2, 3],
            texture: None,
        };
        let far = [vert(40.0, 0.0, 2), vert(41.0, 0.0, 2), vert(41.0, 1.0, 2), vert(40.0, 1.0, 2)];
        m.vertices.extend_from_slice(&far);
        m.indices.extend_from_slice(&[4, 5, 6, 4, 6, 7]);
        let pieces = split_into_tiles(m, 15.0);
        assert_eq!(pieces.len(), 2);
        for p in &pieces {
            assert_eq!(p.vertices.len(), 4);
            assert_eq!(p.indices.len(), 6);
        }
    }

    #[test]
    fn a_bad_tile_size_or_an_empty_mesh_changes_nothing() {
        let m = mesh_of(&[tri(1.0, 1.0, 1), tri(50.0, 1.0, 2)]);
        assert_eq!(split_into_tiles(m, 0.0).len(), 1);
        let m = mesh_of(&[tri(1.0, 1.0, 1), tri(50.0, 1.0, 2)]);
        assert_eq!(split_into_tiles(m, f32::NAN).len(), 1);
        let empty = Mesh { vertices: vec![], indices: vec![], texture: None };
        assert_eq!(split_into_tiles(empty, 15.0).len(), 1);
    }

    #[test]
    fn opaque_meshes_group_by_texture_in_first_seen_order() {
        // textures 7, 3, 7, 5, 3, 7
        let tex = [7, 3, 7, 5, 3, 7];
        let order = draw_order(&tex, &[false; 6]);
        assert_eq!(order, vec![0, 2, 5, 1, 4, 3]);
        // grouped: every texture's meshes are side by side
        let drawn: Vec<usize> = order.iter().map(|&i| tex[i]).collect();
        assert_eq!(drawn, vec![7, 7, 7, 3, 3, 5]);
    }

    #[test]
    fn blended_meshes_stay_in_their_order_after_all_the_opaque_ones() {
        let tex = [1, 2, 1, 2, 1, 3];
        let blended = [false, true, false, true, false, true];
        let order = draw_order(&tex, &blended);
        // opaque (texture 1) first, then the blended in the order they were given, whatever their textures
        assert_eq!(order, vec![0, 2, 4, 1, 3, 5]);
    }

    #[test]
    fn the_order_is_a_permutation_and_does_not_move_the_input_order_within_a_group() {
        let tex: Vec<usize> = (0..50).map(|i| (i * 7) % 5).collect();
        let blended: Vec<bool> = (0..50).map(|i| i % 9 == 0).collect();
        let order = draw_order(&tex, &blended);
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, (0..50).collect::<Vec<_>>());
        for t in 0..5 {
            let of_group: Vec<usize> = order.iter().copied().filter(|&i| tex[i] == t && !blended[i]).collect();
            assert!(of_group.windows(2).all(|w| w[0] < w[1]), "texture {t} keeps input order");
        }
        let blended_order: Vec<usize> = order.iter().copied().filter(|&i| blended[i]).collect();
        assert!(blended_order.windows(2).all(|w| w[0] < w[1]));
        let first_blended = order.iter().position(|&i| blended[i]).unwrap();
        assert!(order[first_blended..].iter().all(|&i| blended[i]), "no opaque mesh after the first blended one");
    }

    #[test]
    fn grouping_cuts_the_draw_calls() {
        // twenty meshes alternating two textures: 20 calls as built, 2 once grouped
        let tex: Vec<usize> = (0..20).map(|i| i % 2).collect();
        let as_built: Vec<(Option<usize>, usize, usize)> = tex.iter().map(|&t| (Some(t), 100, 150)).collect();
        assert_eq!(batches(&as_built, 65536, 98304), 20);
        let order = draw_order(&tex, &[false; 20]);
        let grouped: Vec<(Option<usize>, usize, usize)> = order.iter().map(|&i| as_built[i]).collect();
        assert_eq!(batches(&grouped, 65536, 98304), 2);
    }

    #[test]
    fn a_call_also_ends_where_the_buffer_would_overflow() {
        let one_texture: Vec<(Option<usize>, usize, usize)> = (0..10).map(|_| (Some(0), 30000, 45000)).collect();
        // 65536 vertices hold two 30000-vertex meshes, so ten of them take five calls
        assert_eq!(batches(&one_texture, 65536, 98304), 5);
    }

    #[test]
    fn blended_is_read_from_the_vertex_mode() {
        let mut m = mesh_of(&[tri(0.0, 0.0, 1)]);
        assert!(!is_blended(&m));
        m.vertices[1].normal.w = 4.0;
        assert!(is_blended(&m));
        m.vertices[1].normal.w = 2.0;
        assert!(!is_blended(&m), "the unlit mode writes alpha 1");
    }

    #[test]
    fn group_for_batching_carries_the_keys_with_the_meshes() {
        let items = vec![(10, mesh_of(&[tri(0.0, 0.0, 1)])), (11, mesh_of(&[tri(0.0, 0.0, 2)]))];
        let out = group_for_batching(items);
        assert_eq!(out.iter().map(|(k, _)| *k).collect::<Vec<_>>(), vec![10, 11]);
    }

    fn quad_mesh(n: usize, tag: u8) -> Mesh {
        let mut m = Mesh { vertices: vec![], indices: vec![], texture: None };
        for k in 0..n {
            let base = m.vertices.len() as u16;
            let x = k as f32;
            m.vertices.extend_from_slice(&[vert(x, 0.0, tag), vert(x + 1.0, 0.0, tag), vert(x, 1.0, tag), vert(x + 1.0, 1.0, tag)]);
            m.indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
        }
        m
    }

    #[test]
    fn merging_gives_the_same_triangles_in_fewer_meshes() {
        let a = mesh_of(&[tri(0.0, 0.0, 1)]);
        let b = mesh_of(&[tri(5.0, 0.0, 2), tri(7.0, 0.0, 3)]);
        let mut want = triangles(&a);
        want.extend(triangles(&b));
        want.sort();
        let out = merge_adjacent(vec![a, b], MAX_MERGED_VERTICES);
        assert_eq!(out.len(), 1);
        assert_eq!(triangles(&out[0]), want);
        assert!(out[0].indices.iter().all(|&i| (i as usize) < out[0].vertices.len()));
    }

    #[test]
    fn blended_and_opaque_meshes_are_not_merged_together() {
        let a = mesh_of(&[tri(0.0, 0.0, 1)]);
        let mut b = mesh_of(&[tri(5.0, 0.0, 2)]);
        b.vertices[0].normal.w = 4.0;
        assert_eq!(merge_adjacent(vec![a, b], MAX_MERGED_VERTICES).len(), 2);
    }

    #[test]
    fn a_large_batch_splits_at_the_index_limit() {
        // 20 meshes of 4000 vertices = 80000: more than one 16-bit mesh
        let meshes: Vec<Mesh> = (0..20).map(|i| quad_mesh(1000, i as u8)).collect();
        let total_v: usize = meshes.iter().map(|m| m.vertices.len()).sum();
        let total_i: usize = meshes.iter().map(|m| m.indices.len()).sum();
        let out = merge_adjacent(meshes, MAX_MERGED_VERTICES);
        assert!(out.len() >= 2);
        assert!(out.iter().all(|m| m.vertices.len() <= MAX_MERGED_VERTICES));
        assert_eq!(out.iter().map(|m| m.vertices.len()).sum::<usize>(), total_v);
        assert_eq!(out.iter().map(|m| m.indices.len()).sum::<usize>(), total_i);
        for m in &out {
            assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
        }
    }

    #[test]
    fn merge_for_draw_never_drops_or_adds_triangles() {
        let meshes: Vec<Mesh> = (0..6).map(|i| mesh_of(&[tri(i as f32 * 3.0, 0.0, i as u8)])).collect();
        let mut want: Vec<_> = meshes.iter().flat_map(triangles).collect();
        want.sort();
        let out = merge_for_draw(meshes);
        let mut got: Vec<_> = out.iter().flat_map(triangles).collect();
        got.sort();
        assert_eq!(got, want);
    }
}
