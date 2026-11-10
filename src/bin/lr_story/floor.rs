//! The floor, drawn as 1 m blocks (2x2 per map tile) instead of one stretched picture.
//!
//! A map's `floor` style picks the look: `blocks` (the 1 m bevelled blocks), `asphalt` (dark, fine noise, no bevel; lane
//! paint comes from the art tint), `paving` (one bevelled slab per tile) or `tiles` (small square tiles, 0.25 m), `cobble` and `dirt` (painted textures, one repeat per metre).
//!
//! Each walkable tile becomes four blocks. A block is a quad with a small repeating bevelled texture, tinted by the
//! average colour of the art picture under it and nudged a few percent in brightness by a per-block hash so the grid
//! reads. Meshes are built once per map and cached; each holds at most `BLOCKS_PER_MESH` blocks (4 vertices each),
//! well under macroquad's vertex limit.

use crate::body::TILE_M;
use crate::model::{Kind, MapDef};
use macroquad::prelude::{draw_mesh, Color, FilterMode, Image, Mesh, Texture2D, Vertex};
use std::collections::HashMap;

/// Blocks along one side of a tile (TILE_M is 2.0 m, blocks are 1 m).
pub const BLOCKS_PER_SIDE: i32 = 2;
/// One block's edge in metres.
pub const BLOCK_M: f32 = TILE_M / BLOCKS_PER_SIDE as f32;
/// 800 quads = 3200 vertices and 4800 indices: under macroquad's default draw call (10000 vertices, 5000 indices).
const BLOCKS_PER_MESH: usize = 800;
const FLOOR_Y: f32 = 0.0;

#[derive(Default)]
pub struct FloorCache {
    maps: HashMap<String, Vec<Mesh>>,
    /// One texture per style name, painted for the flat setting in `flat`.
    textures: HashMap<String, Texture2D>,
    flat: Option<bool>,
}

/// How a floor style is cut and painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloorLook {
    /// Blocks along the side of one quad: 1 (a block) or 2 (a whole tile).
    pub cell_blocks: i32,
    /// Texture repeats across one quad.
    pub uv_repeat: f32,
    /// Brightness spread per quad (plus or minus).
    pub spread: f32,
}

/// The look of a floor style. An unknown name gets the `blocks` look (the loader rejects unknown names).
pub fn floor_look(style: &str) -> FloorLook {
    match style {
        "asphalt" => FloorLook { cell_blocks: 1, uv_repeat: 2.0, spread: 0.03 },
        "paving" => FloorLook { cell_blocks: 2, uv_repeat: 1.0, spread: 0.08 },
        "tiles" => FloorLook { cell_blocks: 1, uv_repeat: 4.0, spread: 0.04 },
        // medieval: the painted texture repeats once per metre (one block), no bevel grid, little brightness spread
        "cobble" => FloorLook { cell_blocks: 1, uv_repeat: 1.0, spread: 0.03 },
        "dirt" => FloorLook { cell_blocks: 1, uv_repeat: 1.0, spread: 0.02 },
        _ => FloorLook { cell_blocks: 1, uv_repeat: 1.0, spread: 0.06 },
    }
}

/// The quads of a map's floor as their lower corner in block coordinates, `cell_blocks` blocks to a side. A tile is
/// whole in a quad, so `cell_blocks` 2 gives one quad per walkable tile.
pub fn floor_cells(map: &MapDef, cell_blocks: i32) -> Vec<(i32, i32)> {
    if cell_blocks <= 1 {
        return block_cells(map);
    }
    let mut out = Vec::new();
    for ty in 0..map.grid.h as i32 {
        for tx in 0..map.grid.w as i32 {
            if matches!(map.grid.at(tx, ty), Some(Kind::Wall { .. }) | None) {
                continue;
            }
            out.push((tx * BLOCKS_PER_SIDE, ty * BLOCKS_PER_SIDE));
        }
    }
    out
}

/// Block cells (bx, bz) of a map: every block of every non-wall tile, row by row.
pub fn block_cells(map: &MapDef) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for ty in 0..map.grid.h as i32 {
        for tx in 0..map.grid.w as i32 {
            if matches!(map.grid.at(tx, ty), Some(Kind::Wall { .. }) | None) {
                continue;
            }
            for dz in 0..BLOCKS_PER_SIDE {
                for dx in 0..BLOCKS_PER_SIDE {
                    out.push((tx * BLOCKS_PER_SIDE + dx, ty * BLOCKS_PER_SIDE + dz));
                }
            }
        }
    }
    out
}

/// Average colour of the art under block (bx, bz); opaque. Pixels per block side is art width / (map width * 2),
/// at least 1. Blocks outside the picture read as mid grey.
pub fn floor_tint_at(art: &Image, map: &MapDef, bx: i32, bz: i32) -> Color {
    let grey = Color::new(0.5, 0.5, 0.5, 1.0);
    let (aw, ah) = (art.width as i32, art.height as i32);
    let (bw, bh) = (map.grid.w as i32 * BLOCKS_PER_SIDE, map.grid.h as i32 * BLOCKS_PER_SIDE);
    if aw == 0 || ah == 0 || bw == 0 || bh == 0 || bx < 0 || bz < 0 || bx >= bw || bz >= bh {
        return grey;
    }
    let x0 = bx * aw / bw;
    let x1 = ((bx + 1) * aw / bw).max(x0 + 1).min(aw);
    let z0 = bz * ah / bh;
    let z1 = ((bz + 1) * ah / bh).max(z0 + 1).min(ah);
    let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
    for y in z0..z1 {
        for x in x0..x1 {
            let i = (y as usize * aw as usize + x as usize) * 4;
            if let Some(px) = art.bytes.get(i..i + 4) {
                r += px[0] as u32;
                g += px[1] as u32;
                b += px[2] as u32;
                n += 1;
            }
        }
    }
    if n == 0 {
        return grey;
    }
    let f = n as f32 * 255.0;
    Color::new(r as f32 / f, g as f32 / f, b as f32 / f, 1.0)
}

/// Deterministic brightness factor in [0.94, 1.06] for a block.
fn variation(bx: i32, bz: i32) -> f32 {
    let mut h = (bx as u32).wrapping_mul(0x9E37_79B1) ^ (bz as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 13;
    0.94 + (h % 1000) as f32 / 1000.0 * 0.12
}

/// The 32x32 block texture: near-white face, light top-left bevel, dark bottom-right bevel, dim grout line.
/// `flat` skips the face speckle.
fn block_image(flat: bool) -> Image {
    const N: u32 = 32;
    let mut img = Image::gen_image_color(N as u16, N as u16, Color::new(0.92, 0.92, 0.92, 1.0));
    for y in 0..N {
        for x in 0..N {
            let edge = x.min(y).min(N - 1 - x).min(N - 1 - y);
            let mut v = 0.92f32;
            if edge == 0 {
                v = 0.55;
            } else if edge <= 2 {
                v = if x + y < N { 1.0 } else { 0.78 };
            } else if !flat {
                let h = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663);
                v -= ((h >> 7) % 8) as f32 / 255.0;
            }
            img.set_pixel(x, y, Color::new(v, v, v, 1.0));
        }
    }
    img
}

/// Asphalt: a 32x32 grey with fine grain, no bevel (the art tint supplies the colour and the lane paint).
fn asphalt_image(flat: bool) -> Image {
    const N: u32 = 32;
    let mut img = Image::gen_image_color(N as u16, N as u16, Color::new(0.75, 0.75, 0.75, 1.0));
    for y in 0..N {
        for x in 0..N {
            let mut v = 0.74f32;
            if !flat {
                let h = x.wrapping_mul(2654435761).wrapping_add(y.wrapping_mul(40503)) ^ (x * y).wrapping_mul(97);
                v += (((h >> 9) % 16) as f32 - 8.0) / 100.0;
            }
            img.set_pixel(x, y, Color::new(v, v, v, 1.0));
        }
    }
    img
}

/// A paving slab: a wide grout line, a broad bevel and a faintly speckled face.
fn slab_image(flat: bool) -> Image {
    const N: u32 = 32;
    let mut img = Image::gen_image_color(N as u16, N as u16, Color::new(0.88, 0.88, 0.88, 1.0));
    for y in 0..N {
        for x in 0..N {
            let edge = x.min(y).min(N - 1 - x).min(N - 1 - y);
            let mut v = 0.88f32;
            if edge <= 1 {
                v = 0.45;
            } else if edge <= 4 {
                v = if x + y < N { 0.98 } else { 0.72 };
            } else if !flat {
                let h = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663);
                v -= ((h >> 6) % 10) as f32 / 160.0;
            }
            img.set_pixel(x, y, Color::new(v, v, v, 1.0));
        }
    }
    img
}

/// One small square tile: a one-pixel grout line and a glossy corner. Repeated 4x4 over a block.
fn tile_image(flat: bool) -> Image {
    const N: u32 = 16;
    let mut img = Image::gen_image_color(N as u16, N as u16, Color::new(0.93, 0.93, 0.93, 1.0));
    for y in 0..N {
        for x in 0..N {
            let edge = x.min(y).min(N - 1 - x).min(N - 1 - y);
            let mut v = 0.93f32;
            if edge == 0 {
                v = 0.5;
            } else if !flat && edge == 1 && x + y < N {
                v = 1.0;
            }
            img.set_pixel(x, y, Color::new(v, v, v, 1.0));
        }
    }
    img
}

/// The (group, kind) the textures painter paints for a painted floor style (cobble, dirt); None for the generated ones.
pub fn painted_floor_kind(style: &str) -> Option<(&'static str, &'static str)> {
    match style {
        "cobble" => Some(("floor", "cobble0")),
        "dirt" => Some(("floor", "dirt0")),
        _ => None,
    }
}

/// The texture picture of a floor style.
fn style_image(style: &str, flat: bool) -> Image {
    match style {
        "asphalt" => asphalt_image(flat),
        "paving" => slab_image(flat),
        "tiles" => tile_image(flat),
        _ => block_image(flat),
    }
}

fn build(map: &MapDef, art: &Image, tex: &Texture2D) -> Vec<Mesh> {
    let look = floor_look(&map.floor_style);
    let cb = look.cell_blocks;
    floor_cells(map, cb)
        .chunks(BLOCKS_PER_MESH)
        .map(|chunk| {
            let mut vertices = Vec::with_capacity(chunk.len() * 4);
            let mut indices: Vec<u16> = Vec::with_capacity(chunk.len() * 6);
            for &(bx, bz) in chunk {
                // the average tint of the blocks under the quad
                let (mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32);
                for dz in 0..cb {
                    for dx in 0..cb {
                        let t = floor_tint_at(art, map, bx + dx, bz + dz);
                        r += t.r;
                        g += t.g;
                        b += t.b;
                    }
                }
                let n = (cb * cb) as f32;
                // variation() is within 0.94..=1.06; its spread is scaled to the style's
                let k = 1.0 + (variation(bx, bz) - 1.0) * look.spread / 0.06;
                let c = Color::new((r / n * k).min(1.0), (g / n * k).min(1.0), (b / n * k).min(1.0), 1.0);
                let (x0, z0) = (bx as f32 * BLOCK_M, bz as f32 * BLOCK_M);
                let (x1, z1) = (x0 + BLOCK_M * cb as f32, z0 + BLOCK_M * cb as f32);
                let uv = look.uv_repeat;
                let base = vertices.len() as u16;
                vertices.push(Vertex::new(x0, FLOOR_Y, z0, 0.0, 0.0, c));
                vertices.push(Vertex::new(x1, FLOOR_Y, z0, uv, 0.0, c));
                vertices.push(Vertex::new(x1, FLOOR_Y, z1, uv, uv, c));
                vertices.push(Vertex::new(x0, FLOOR_Y, z1, 0.0, uv, c));
                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            Mesh { vertices, indices, texture: Some(tex.clone()) }
        })
        .collect()
}

/// Draws the floor of `map_id` inside the 3D camera, building and caching its meshes on first use.
/// `art` is the decoded art picture of that map.
pub fn draw_floor(cache: &mut FloorCache, map_id: &str, map: &MapDef, art: &Image) {
    let flat = crate::textures::flat_textures();
    if cache.flat != Some(flat) {
        cache.flat = Some(flat);
        cache.textures.clear();
        cache.maps.clear();
    }
    let style = map.floor_style.as_str();
    if !cache.textures.contains_key(style) {
        // cobble and dirt come from the textures painter (nearest filter; flat mode is honoured there)
        let painted = painted_floor_kind(style).and_then(|k| crate::scenery::painted_texture(k, style));
        let tex = painted.unwrap_or_else(|| {
            let tex = Texture2D::from_image(&style_image(style, flat));
            tex.set_filter(FilterMode::Linear);
            // repeat wrap, so the tiles style can run its uv past 1
            crate::repeat_wrap(&tex);
            tex
        });
        cache.textures.insert(style.to_string(), tex);
    }
    let Some(tex) = cache.textures.get(style).cloned() else { return };
    let meshes = cache.maps.entry(map_id.to_string()).or_insert_with(|| build(map, art, &tex));
    for m in meshes.iter() {
        draw_mesh(m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Grid;
    use std::collections::BTreeMap;

    fn map(cells: Vec<Kind>, w: usize, h: usize) -> MapDef {
        MapDef {
            id: "t".into(),
            name: "t".into(),
            art: "t.png".into(),
            legend: BTreeMap::new(),
            grid: Grid { w, h, cells },
            spawns: BTreeMap::new(),
            spawn_facing: BTreeMap::new(),
            outdoor: false,
            ceiling_height: 3.0,
            ceiling_color: None,
            sky: None,
            wall_style: "lab".into(),
            weather: None,
            floor_style: "blocks".into(),
        }
    }

    #[test]
    fn four_blocks_per_floor_tile_none_for_walls() {
        let wall = Kind::Wall { height: 1.0, color: [0, 0, 0] };
        let m = map(vec![Kind::Floor, wall, Kind::Floor, Kind::Floor], 2, 2);
        assert_eq!(block_cells(&m).len(), 3 * 4);
        assert!(!block_cells(&m).iter().any(|&(bx, bz)| bx / 2 == 1 && bz / 2 == 0));
    }

    #[test]
    fn tint_averages_the_pixels_under_a_block() {
        // 1x1 tile map, 4x4 art: 2x2 blocks of 2x2 pixels. Left half red, right half blue.
        let mut art = Image::gen_image_color(4, 4, Color::new(1.0, 0.0, 0.0, 1.0));
        for y in 0..4 {
            for x in 2..4 {
                art.set_pixel(x, y, Color::new(0.0, 0.0, 1.0, 1.0));
            }
        }
        let m = map(vec![Kind::Floor], 1, 1);
        let a = floor_tint_at(&art, &m, 0, 1);
        let b = floor_tint_at(&art, &m, 1, 0);
        assert!(a.r > 0.99 && a.b < 0.01);
        assert!(b.b > 0.99 && b.r < 0.01);
        assert_eq!(floor_tint_at(&art, &m, 5, 5).r, 0.5);
    }

    #[test]
    fn variation_is_stable_and_small() {
        assert_eq!(variation(3, 4), variation(3, 4));
        for i in 0..50 {
            assert!((0.94..=1.06).contains(&variation(i, 50 - i)));
        }
    }

    #[test]
    fn style_lookup_cuts_paving_per_tile_and_the_rest_per_block() {
        assert_eq!(floor_look("paving").cell_blocks, 2);
        for s in ["blocks", "asphalt", "tiles", "unknown"] {
            assert_eq!(floor_look(s).cell_blocks, 1, "{s}");
        }
        assert_eq!(floor_look("tiles").uv_repeat, 4.0);
        assert_eq!(floor_look("unknown"), floor_look("blocks"));
        let wall = Kind::Wall { height: 1.0, color: [0, 0, 0] };
        let m = map(vec![Kind::Floor, wall, Kind::Floor, Kind::Floor], 2, 2);
        assert_eq!(floor_cells(&m, 2).len(), 3, "one slab per walkable tile");
        assert_eq!(floor_cells(&m, 2)[1], (0, 2));
        assert_eq!(floor_cells(&m, 1).len(), 12);
    }

    #[test]
    fn cobble_and_dirt_repeat_per_metre_on_single_blocks_with_painted_textures() {
        for s in ["cobble", "dirt"] {
            let l = floor_look(s);
            assert_eq!((l.cell_blocks, l.uv_repeat), (1, 1.0), "{s}: one repeat per 1 m block");
            assert!(l.spread < floor_look("blocks").spread, "{s}: no visible block grid");
        }
        assert_eq!(painted_floor_kind("cobble"), Some(("floor", "cobble0")));
        assert_eq!(painted_floor_kind("dirt"), Some(("floor", "dirt0")));
        assert_eq!(painted_floor_kind("blocks"), None);
    }

    #[test]
    fn meshes_stay_under_the_vertex_limit() {
        assert!(BLOCKS_PER_MESH * 4 <= 10000 && BLOCKS_PER_MESH * 6 <= 5000);
    }
}
