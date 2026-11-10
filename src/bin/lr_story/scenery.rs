//! Above and around the world: the ceiling of an indoor map, the sky of an outdoor one, and the texture lookups that
//! pick the wall and ceiling look from a map's `walls` style.
//!
//! * Indoor map (`outdoor == false`): one quad at `ceiling_height` over every non-wall tile, textured with the
//!   ceiling of the wall style and tinted by `ceiling_color`. Meshes are built once per map and cached, at most
//!   `CEILING_TILES_PER_MESH` tiles (4 vertices each) per mesh. It is not drawn while the camera is above it.
//! * Outdoor map: no ceiling. A dome of vertex colours (`sky.top` at the zenith, `sky.bottom` at the horizon) centred on
//!   the camera, drawn before the world, with a few faint stars when the sky is dark. The dome is far behind every
//!   tile, so the depth test lets the world draw over it.

use crate::body::TILE_M;
use crate::model::{Kind, MapDef, Rgb};
use macroquad::prelude::{draw_mesh, vec3, Color, FilterMode, Mesh, Texture2D, Vec3, Vertex};
use std::collections::{BTreeMap, HashMap};

#[cfg(test)]
use crate::model::{FLOOR_STYLES, WALL_STYLES};
/// The ceiling height when a map has no wall in its legend and no `"ceiling"` height.
pub const DEFAULT_CEILING_M: f32 = 3.0;
/// The dusk sky of an outdoor map that names none: (top, bottom).
pub const DUSK_SKY: (Rgb, Rgb) = ([34, 40, 84], [214, 120, 92]);

/// 800 quads = 3200 vertices and 4800 indices: under macroquad's default draw call (10000 vertices, 5000 indices).
const CEILING_TILES_PER_MESH: usize = 800;
/// Radius of the sky dome in metres (the camera's far plane is 300 m).
const SKY_RADIUS: f32 = 250.0;
/// Rings of the dome from below the horizon to the zenith, in degrees of latitude; segments round it.
const SKY_RINGS_DEG: [f32; 9] = [-25.0, 0.0, 8.0, 20.0, 35.0, 50.0, 65.0, 80.0, 90.0];
const SKY_SEGMENTS: usize = 16;
const STAR_COUNT: u32 = 40;

/// Whether `style` is a known wall style.
#[cfg(test)]
pub fn is_wall_style(style: &str) -> bool {
    WALL_STYLES.contains(&style)
}

/// Whether `style` is a known floor style.
#[cfg(test)]
pub fn is_floor_style(style: &str) -> bool {
    FLOOR_STYLES.contains(&style)
}

/// The (group, kind) the textures painter paints for a wall style's walls. An unknown style gets the lab wall.
pub fn wall_texture_kind(style: &str) -> (&'static str, &'static str) {
    match style {
        "brick" => ("wall", "brick0"),
        "concrete" => ("wall", "stone0"),
        "plaster" => ("wall", "whitewash0"),
        "timber" => ("wall", "timber0"),
        "stone" => ("wall", "ashlar0"),
        _ => ("wall", "lab0"),
    }
}

/// The (group, kind) of the ceiling over a wall style: lab -> ceil0, brick -> ceil1, concrete and plaster -> vault.
pub fn ceiling_texture_kind(style: &str) -> (&'static str, &'static str) {
    match style {
        "brick" => ("ceiling", "ceil1"),
        "concrete" | "plaster" => ("ceiling", "vault"),
        // the medieval looks: dark planks with heavy cross beams
        "timber" | "stone" => ("ceiling", "beams0"),
        _ => ("ceiling", "ceil0"),
    }
}

/// A repeating nearest-filter texture painted by the textures painter. The lab look (and `--flat-textures`) is painted
/// flat; the other styles keep the painted look. None when the painter does not know the kind.
pub fn painted_texture(kind: (&str, &str), style: &str) -> Option<Texture2D> {
    let look = if style == "lab" || crate::textures::flat_textures() {
        crate::textures::Style::Flat
    } else {
        crate::textures::Style::Painted
    };
    let (w, h, px) = crate::textures::paint_styled(kind.0, kind.1, 0, look)?;
    let tex = Texture2D::from_rgba8(w, h, &px);
    tex.set_filter(FilterMode::Nearest);
    crate::repeat_wrap(&tex);
    Some(tex)
}

/// The height of the tallest wall in a legend, else `DEFAULT_CEILING_M`. The default ceiling height of a map.
pub fn default_ceiling_height(legend: &BTreeMap<String, Kind>) -> f32 {
    legend
        .values()
        .filter_map(|k| match k {
            Kind::Wall { height, .. } => Some(*height),
            _ => None,
        })
        .fold(None, |best: Option<f32>, h| Some(best.map_or(h, |b| b.max(h))))
        .filter(|h| *h > 0.0)
        .unwrap_or(DEFAULT_CEILING_M)
}

/// The ceiling height to draw: the map's own, or the default when it is not positive.
pub fn ceiling_height_of(map: &MapDef) -> f32 {
    if map.ceiling_height > 0.0 {
        map.ceiling_height
    } else {
        default_ceiling_height(&map.legend)
    }
}

/// The sky colours of a map: its own, else the dusk gradient.
pub fn sky_of(map: &MapDef) -> (Rgb, Rgb) {
    map.sky.unwrap_or(DUSK_SKY)
}

/// The tiles under a ceiling: every non-wall tile of an indoor map, row by row. An outdoor map has none.
pub fn ceiling_cells(map: &MapDef) -> Vec<(i32, i32)> {
    if map.outdoor {
        return Vec::new();
    }
    let mut out = Vec::new();
    for ty in 0..map.grid.h as i32 {
        for tx in 0..map.grid.w as i32 {
            if matches!(map.grid.at(tx, ty), Some(Kind::Wall { .. }) | None) {
                continue;
            }
            out.push((tx, ty));
        }
    }
    out
}

fn rgb_color(c: Rgb) -> Color {
    Color::new(c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0)
}

/// The ceiling meshes: one downward quad per tile at `height`, u and v one repeat per metre.
fn build_ceiling(cells: &[(i32, i32)], height: f32, tint: Color, tex: &Texture2D) -> Vec<Mesh> {
    cells
        .chunks(CEILING_TILES_PER_MESH)
        .map(|chunk| {
            let mut vertices = Vec::with_capacity(chunk.len() * 4);
            let mut indices: Vec<u16> = Vec::with_capacity(chunk.len() * 6);
            for &(tx, tz) in chunk {
                let (x0, z0) = (tx as f32 * TILE_M, tz as f32 * TILE_M);
                let (x1, z1) = (x0 + TILE_M, z0 + TILE_M);
                let base = vertices.len() as u16;
                vertices.push(Vertex::new(x0, height, z0, x0, z0, tint));
                vertices.push(Vertex::new(x1, height, z0, x1, z0, tint));
                vertices.push(Vertex::new(x1, height, z1, x1, z1, tint));
                vertices.push(Vertex::new(x0, height, z1, x0, z1, tint));
                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            Mesh { vertices, indices, texture: Some(tex.clone()) }
        })
        .collect()
}

/// The cached ceiling meshes and textures of the maps drawn so far.
#[derive(Default)]
pub struct SceneryCache {
    ceilings: HashMap<String, Vec<Mesh>>,
    textures: HashMap<String, Texture2D>,
    flat: Option<bool>,
}

/// Draws the ceiling of an indoor map inside the 3D camera (after the walls), building and caching it on first use.
/// Nothing is drawn for an outdoor map, or while the camera (`eye_y`) is at or above the ceiling (a third-person view
/// over the roof).
pub fn draw_ceiling(cache: &mut SceneryCache, map_id: &str, map: &MapDef, eye_y: f32) {
    if map.outdoor {
        return;
    }
    let height = ceiling_height_of(map);
    if eye_y >= height - 0.05 {
        return;
    }
    let flat = crate::textures::flat_textures();
    if cache.flat != Some(flat) {
        cache.flat = Some(flat);
        cache.ceilings.clear();
        cache.textures.clear();
    }
    if !cache.ceilings.contains_key(map_id) {
        if !cache.textures.contains_key(&map.wall_style) {
            if let Some(tex) = painted_texture(ceiling_texture_kind(&map.wall_style), &map.wall_style) {
                cache.textures.insert(map.wall_style.clone(), tex);
            }
        }
        let Some(tex) = cache.textures.get(&map.wall_style) else { return };
        let tint = map.ceiling_color.map_or(Color::new(1.0, 1.0, 1.0, 1.0), rgb_color);
        let meshes = build_ceiling(&ceiling_cells(map), height, tint, tex);
        cache.ceilings.insert(map_id.to_string(), meshes);
    }
    if let Some(meshes) = cache.ceilings.get(map_id) {
        for m in meshes {
            draw_mesh(m);
        }
    }
}

/// Draws the sky of an outdoor map round the camera at `eye`; call it first, before the floor and walls. Nothing for an
/// indoor map.
pub fn draw_sky(map: &MapDef, eye: Vec3) {
    if !map.outdoor {
        return;
    }
    let (top, bottom) = sky_of(map);
    let (vertices, indices) = sky_mesh(top, bottom, eye);
    draw_mesh(&Mesh { vertices, indices, texture: None });
}

fn lerp_rgb(a: Rgb, b: Rgb, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let ch = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) / 255.0;
    Color::new(ch(0), ch(1), ch(2), 1.0)
}

/// Brightness of a colour, 0 to 1.
fn luminance(c: Rgb) -> f32 {
    (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0
}

/// A small deterministic hash to [0, 1).
fn unit(seed: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9E37_79B1) ^ 0x85EB_CA77;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 13;
    (h % 10_000) as f32 / 10_000.0
}

/// The sky dome (a ring-by-ring sphere of vertex colours, plus faint star quads when the top is dark) centred on `eye`.
pub fn sky_mesh(top: Rgb, bottom: Rgb, eye: Vec3) -> (Vec<Vertex>, Vec<u16>) {
    let mut vertices = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    for &lat in &SKY_RINGS_DEG {
        // 0 at the horizon and below, 1 at the zenith; eased so the bottom colour holds near the horizon
        let t = (lat.max(0.0) / 90.0).powf(0.6);
        let c = lerp_rgb(bottom, top, t);
        let (y, r) = (lat.to_radians().sin(), lat.to_radians().cos());
        for s in 0..=SKY_SEGMENTS {
            let a = s as f32 / SKY_SEGMENTS as f32 * std::f32::consts::TAU;
            let p = eye + vec3(a.cos() * r, y, a.sin() * r) * SKY_RADIUS;
            vertices.push(Vertex::new(p.x, p.y, p.z, 0.0, 0.0, c));
        }
    }
    let stride = (SKY_SEGMENTS + 1) as u16;
    for ring in 0..(SKY_RINGS_DEG.len() as u16 - 1) {
        for s in 0..SKY_SEGMENTS as u16 {
            let a = ring * stride + s;
            let b = a + stride;
            indices.extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
        }
    }
    // Stars: only where the sky is dark, and faint.
    let alpha = (1.0 - luminance(top) * 2.0).clamp(0.0, 1.0) * 0.6;
    if alpha > 0.02 {
        for i in 0..STAR_COUNT {
            let az = unit(i * 3 + 1) * std::f32::consts::TAU;
            let el = (12.0 + unit(i * 3 + 2) * 76.0).to_radians();
            let dir = vec3(az.cos() * el.cos(), el.sin(), az.sin() * el.cos());
            let right = dir.cross(vec3(0.0, 1.0, 0.0)).normalize_or_zero();
            let up = right.cross(dir).normalize_or_zero();
            let size = SKY_RADIUS * (0.0025 + unit(i * 3 + 3) * 0.003);
            let centre = eye + dir * (SKY_RADIUS * 0.98);
            let c = Color::new(1.0, 1.0, 0.95, alpha * (0.5 + unit(i + 99) * 0.5));
            let base = vertices.len() as u16;
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = centre + right * (dx * size) + up * (dy * size);
                vertices.push(Vertex::new(p.x, p.y, p.z, 0.0, 0.0, c));
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    (vertices, indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Grid;

    fn map(cells: Vec<Kind>, w: usize, h: usize, outdoor: bool) -> MapDef {
        MapDef {
            id: "t".into(),
            name: "t".into(),
            art: "t.png".into(),
            legend: BTreeMap::new(),
            grid: Grid { w, h, cells },
            spawns: BTreeMap::new(),
            spawn_facing: BTreeMap::new(),
            outdoor,
            ceiling_height: 3.0,
            ceiling_color: None,
            sky: None,
            wall_style: "lab".into(),
            floor_style: "blocks".into(),
            weather: None,
        }
    }

    fn wall(height: f32) -> Kind {
        Kind::Wall { height, color: [0, 0, 0] }
    }

    #[test]
    fn the_ceiling_covers_every_non_wall_tile_of_an_indoor_map() {
        let m = map(vec![Kind::Floor, wall(3.0), Kind::Floor, Kind::Floor], 2, 2, false);
        assert_eq!(ceiling_cells(&m), vec![(0, 0), (0, 1), (1, 1)]);
    }

    #[test]
    fn an_outdoor_map_has_no_ceiling() {
        let m = map(vec![Kind::Floor; 4], 2, 2, true);
        assert!(ceiling_cells(&m).is_empty());
    }

    #[test]
    fn the_default_ceiling_is_the_tallest_wall_else_three_metres() {
        let mut legend = BTreeMap::new();
        assert_eq!(default_ceiling_height(&legend), DEFAULT_CEILING_M);
        legend.insert("111111".to_string(), wall(2.5));
        legend.insert("222222".to_string(), wall(4.0));
        legend.insert("333333".to_string(), Kind::Floor);
        assert_eq!(default_ceiling_height(&legend), 4.0);
        let mut m = map(vec![Kind::Floor], 1, 1, false);
        m.legend = legend;
        m.ceiling_height = 0.0;
        assert_eq!(ceiling_height_of(&m), 4.0, "an unset height falls back to the tallest wall");
        m.ceiling_height = 2.2;
        assert_eq!(ceiling_height_of(&m), 2.2);
    }

    #[test]
    fn every_style_names_a_texture_the_painter_has() {
        for style in WALL_STYLES {
            let (g, k) = wall_texture_kind(style);
            assert!(crate::textures::kinds(g).contains(&k), "wall {style}: {k}");
            let (g, k) = ceiling_texture_kind(style);
            assert!(crate::textures::kinds(g).contains(&k), "ceiling {style}: {k}");
            assert!(is_wall_style(style));
        }
        assert_eq!(wall_texture_kind("brick").1, "brick0");
        assert_eq!(wall_texture_kind("timber"), ("wall", "timber0"));
        assert_eq!(wall_texture_kind("stone"), ("wall", "ashlar0"));
        assert_eq!(wall_texture_kind("nonsense").1, "lab0", "an unknown style paints the lab wall");
        assert!(!is_wall_style("nonsense"));
        assert!(is_floor_style("asphalt") && !is_floor_style("lava"));
    }

    #[test]
    fn medieval_styles_are_listed_and_timber_and_stone_get_a_beamed_ceiling() {
        for w in ["timber", "stone"] {
            assert!(is_wall_style(w), "{w}");
            assert_eq!(ceiling_texture_kind(w), ("ceiling", "beams0"), "{w}");
        }
        for f in ["cobble", "dirt"] {
            assert!(is_floor_style(f), "{f}");
        }
        assert_eq!(ceiling_texture_kind("brick").1, "ceil1", "the old styles keep their ceilings");
        assert_eq!(ceiling_texture_kind("lab").1, "ceil0");
    }

    #[test]
    fn the_sky_defaults_to_dusk_and_its_mesh_is_small() {
        let m = map(vec![Kind::Floor], 1, 1, true);
        assert_eq!(sky_of(&m), DUSK_SKY);
        let mut own = m.clone();
        own.sky = Some(([1, 2, 3], [4, 5, 6]));
        assert_eq!(sky_of(&own), ([1, 2, 3], [4, 5, 6]));
        let (v, i) = sky_mesh([0, 0, 20], [40, 40, 60], vec3(1.0, 2.0, 3.0));
        assert!(v.len() < 8000 && i.iter().all(|&ix| (ix as usize) < v.len()));
        let (plain, _) = sky_mesh([250, 250, 250], [255, 255, 255], Vec3::ZERO);
        assert_eq!(plain.len(), SKY_RINGS_DEG.len() * (SKY_SEGMENTS + 1), "a bright sky has no stars");
        assert!(v.len() > plain.len(), "a dark sky has stars");
    }

    #[test]
    fn ceiling_meshes_stay_under_the_vertex_limit() {
        assert!(CEILING_TILES_PER_MESH * 4 <= 10000 && CEILING_TILES_PER_MESH * 6 <= 5000);
    }
}
