//! The concept map: KT's region image (`data/regions/map_concept.png`, 333x260) read as a region grid, and the
//! rules that turn it into the Infinite route's level 0 layout.
//!
//! The PNG is converted once by `tools/concept_map.mjs` into `data/concept/map_concept.json` (`lr.concept_map/1`,
//! one key character per pixel). It lives in `data/concept/`, not `data/regions/`, because the region catalogue
//! reads every file in `regions/` as a `lr.region/1` row. Every number that tunes the layout is in
//! `data/concept/concept_settings.json` (`lr.concept_settings/1`): `scale` (cells per pixel), `spacing` (cells of
//! wall kept between different regions), `tunnel_width`, the pixel that sits under the spawn chunk, what lies
//! outside the image (`mirror` or `tile`), whether a seed turns the map, and which generator region each colour uses.
//!
//! How it is read: a world cell maps to a pixel (`scale`, the seed's turn, the spawn pixel). A chunk is the region
//! that most of its sampled cells belong to (`classify_chunk`), or hollow (solid rock) when too little of it is
//! region. The mineshaft tunnels (key `T`) are carved into every chunk they pass through, `tunnel_width` cells wide
//! whatever the scale (`is_tunnel_cell`), and they are what joins the regions. Chunk (0, 0) is always the maze, and
//! chunk (1, 0), where the two primary hunters always start (`hunter_spawns`), is never hollow: when its vote would
//! make it rock it is the maze too.
//! Only level 0 uses the map. The result depends on the world seed only through `orient_of`.

use crate::content::embedded;
use crate::world::rng::Rng;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const MAP_SCHEMA: &str = "lr.concept_map/1";
pub const SETTINGS_SCHEMA: &str = "lr.concept_settings/1";
/// Cells per chunk side (layout::CHUNK) and the cell that sits on the spawn pixel (CHUNK / 2).
const CHUNK: i32 = 45;
const ORIGIN: i32 = CHUNK / 2;
/// Sample points per chunk side: cells 2, 7, ... 42.
const SAMPLES: i32 = 9;
pub const WALL: u8 = b'#';
pub const TUNNEL: u8 = b'T';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outside {
    Mirror,
    Tile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Legend {
    /// The generator region the colour uses (a name in `regions::NAMES`).
    pub region: &'static str,
    /// Player-facing name, upper case.
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// False lays level 0 out as before the concept map (Godot's region draw: no rock, no tunnels). Missing means true.
    pub enabled: bool,
    pub scale: i32,
    pub spacing: i32,
    pub tunnel_width: i32,
    pub spawn_pixel: (i32, i32),
    pub outside: Outside,
    pub seed_orient: bool,
    pub min_region_share: f64,
    pub legend: HashMap<u8, Legend>,
}

/// A turn of the map about the spawn, drawn from the world seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Orient {
    pub swap: bool,
    pub flip_x: bool,
    pub flip_z: bool,
}

/// What a chunk is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkClass {
    /// A district laid out with this generator region.
    Region(&'static str),
    /// Solid rock, with the tunnels cut through it.
    Hollow,
}

#[derive(Clone, Debug)]
pub struct Concept {
    pub width: i32,
    pub height: i32,
    rows: Vec<Vec<u8>>,
    pub settings: Settings,
    /// Cell-resolution mineshaft mask over the base tile `[0, width*scale) x [0, height*scale)`, row-major with
    /// stride `mask_w`. It is the pixel-`T` network's centreline (a Zhang-Suen skeleton) dilated back to
    /// `tunnel_width` across and clipped to the `T` cells, so the carved shaft stays 4-connected: a fixed band
    /// around a connected centreline never pinches the way the old per-cell narrowing did. Built once in
    /// `build_tunnel_mask` and read by `is_tunnel_cell`.
    tunnel_mask: Vec<bool>,
    mask_w: i32,
    mask_h: i32,
}

/// Builds the cell-resolution mineshaft mask for `is_tunnel_cell`. The pixel-`T` network is blown up to cell
/// resolution (each `T` pixel is `scale` cells), thinned to a one-cell centreline with Zhang-Suen, then that
/// centreline is dilated by a Manhattan ball of radius `(tunnel_width - 1) / 2` and clipped back to the `T` cells.
/// A fixed band around a connected centreline is itself 4-connected, so no shaft pinches shut; a straight tunnel
/// keeps its full `tunnel_width`, and a wide blob is narrowed to a `tunnel_width`-wide corridor down its middle.
/// Border cells of the tile are left unthinned (Zhang-Suen needs a one-cell margin); that only keeps the band a
/// touch wider at the tile edge, where the mirror/tile fold joins it to its reflection.
fn build_tunnel_mask(rows: &[Vec<u8>], scale: i32, tunnel_width: i32) -> (Vec<bool>, i32, i32) {
    let s = scale.max(1) as usize;
    let wu = rows[0].len() * s;
    let hu = rows.len() * s;
    let idx = |x: usize, y: usize| y * wu + x;
    // the T network blown up to cell resolution
    let mut t = vec![false; wu * hu];
    for y in 0..hu {
        for x in 0..wu {
            t[idx(x, y)] = rows[y / s][x / s] == TUNNEL;
        }
    }
    // Zhang-Suen thinning to a one-cell skeleton (the one-cell border is left as-is)
    let mut img = t.clone();
    let at = |img: &[bool], x: usize, y: usize| u8::from(img[idx(x, y)]);
    let mut changed = true;
    while changed {
        changed = false;
        for step in 0..2 {
            let mut clear: Vec<usize> = Vec::new();
            for y in 1..hu.saturating_sub(1) {
                for x in 1..wu.saturating_sub(1) {
                    if !img[idx(x, y)] {
                        continue;
                    }
                    let p2 = at(&img, x, y - 1);
                    let p3 = at(&img, x + 1, y - 1);
                    let p4 = at(&img, x + 1, y);
                    let p5 = at(&img, x + 1, y + 1);
                    let p6 = at(&img, x, y + 1);
                    let p7 = at(&img, x - 1, y + 1);
                    let p8 = at(&img, x - 1, y);
                    let p9 = at(&img, x - 1, y - 1);
                    let b = p2 + p3 + p4 + p5 + p6 + p7 + p8 + p9;
                    if !(2..=6).contains(&b) {
                        continue;
                    }
                    let seq = [p2, p3, p4, p5, p6, p7, p8, p9, p2];
                    let a = (0..8).filter(|&i| seq[i] == 0 && seq[i + 1] == 1).count();
                    if a != 1 {
                        continue;
                    }
                    let keep = if step == 0 {
                        p2 * p4 * p6 == 0 && p4 * p6 * p8 == 0
                    } else {
                        p2 * p4 * p8 == 0 && p2 * p6 * p8 == 0
                    };
                    if keep {
                        clear.push(idx(x, y));
                    }
                }
            }
            if !clear.is_empty() {
                changed = true;
                for i in clear {
                    img[i] = false;
                }
            }
        }
    }
    // dilate the skeleton by a Manhattan ball of radius r, clipped to the T cells
    let r = (tunnel_width.max(1) - 1) / 2;
    let (w, h) = (wu as i32, hu as i32);
    let mut mask = vec![false; wu * hu];
    for y in 0..hu {
        for x in 0..wu {
            if !img[idx(x, y)] {
                continue;
            }
            for dy in -r..=r {
                let span = r - dy.abs();
                for dx in -span..=span {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= 0 && nx < w && ny >= 0 && ny < h && t[idx(nx as usize, ny as usize)] {
                        mask[idx(nx as usize, ny as usize)] = true;
                    }
                }
            }
        }
    }
    (mask, w, h)
}

fn int(v: &Value, file: &str, key: &str, min: i64) -> Result<i32, String> {
    match v.get(key).and_then(Value::as_i64) {
        Some(n) if n >= min && n < 100_000 => Ok(n as i32),
        _ => Err(format!("{file}: \"{key}\" must be a whole number of at least {min}")),
    }
}

pub fn parse_settings(file: &str, text: &str) -> Result<Settings, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
    if v.get("schema").and_then(Value::as_str) != Some(SETTINGS_SCHEMA) {
        return Err(format!("{file}: \"schema\" must be \"{SETTINGS_SCHEMA}\""));
    }
    let enabled = match v.get("enabled") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(format!("{file}: \"enabled\" must be true or false")),
    };
    let spawn = v
        .get("spawn_pixel")
        .and_then(Value::as_array)
        .and_then(|a| Some((a.first()?.as_i64()? as i32, a.get(1)?.as_i64()? as i32)))
        .ok_or_else(|| format!("{file}: \"spawn_pixel\" must be [x, y]"))?;
    let outside = match v.get("outside").and_then(Value::as_str) {
        Some("mirror") => Outside::Mirror,
        Some("tile") => Outside::Tile,
        _ => return Err(format!("{file}: \"outside\" must be \"mirror\" or \"tile\"")),
    };
    let share = v
        .get("min_region_share")
        .and_then(Value::as_f64)
        .filter(|s| (0.0..=1.0).contains(s))
        .ok_or_else(|| format!("{file}: \"min_region_share\" must be a number from 0 to 1"))?;
    let mut legend = HashMap::new();
    let entries = v
        .get("legend")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{file}: \"legend\" must be an object"))?;
    for (key, e) in entries {
        let ch = match key.as_bytes() {
            [b] => *b,
            _ => return Err(format!("{file}: legend key \"{key}\" must be one character")),
        };
        let name = e
            .get("region")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{file}: legend \"{key}\" needs a \"region\""))?;
        let region = super::regions::NAMES
            .iter()
            .copied()
            .find(|n| *n == name)
            .ok_or_else(|| format!("{file}: legend \"{key}\" names unknown region \"{name}\""))?;
        let label = e.get("label").and_then(Value::as_str).unwrap_or(name).to_string();
        legend.insert(ch, Legend { region, label });
    }
    Ok(Settings {
        enabled,
        scale: int(&v, file, "scale", 1)?,
        spacing: int(&v, file, "spacing", 0)?,
        tunnel_width: int(&v, file, "tunnel_width", 1)?,
        spawn_pixel: spawn,
        outside,
        seed_orient: v.get("seed_orient").and_then(Value::as_bool).unwrap_or(false),
        min_region_share: share,
        legend,
    })
}

impl Concept {
    /// Builds a concept from the map JSON text and parsed settings. Every non-wall key of the map must be in
    /// the legend.
    pub fn parse(file: &str, map_text: &str, settings: Settings) -> Result<Concept, String> {
        let v: Value = serde_json::from_str(map_text).map_err(|e| format!("{file}: {e}"))?;
        if v.get("schema").and_then(Value::as_str) != Some(MAP_SCHEMA) {
            return Err(format!("{file}: \"schema\" must be \"{MAP_SCHEMA}\""));
        }
        let rows: Vec<Vec<u8>> = v
            .get("rows")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{file}: \"rows\" must be an array"))?
            .iter()
            .map(|r| r.as_str().map(|s| s.as_bytes().to_vec()))
            .collect::<Option<_>>()
            .ok_or_else(|| format!("{file}: every row must be a string"))?;
        let width = rows.first().map_or(0, Vec::len);
        if rows.is_empty() || width == 0 || rows.iter().any(|r| r.len() != width) {
            return Err(format!("{file}: rows must be non-empty and the same length"));
        }
        for row in &rows {
            for ch in row {
                if *ch != WALL && !settings.legend.contains_key(ch) {
                    return Err(format!("{file}: key '{}' is not in the legend", *ch as char));
                }
            }
        }
        let (tunnel_mask, mask_w, mask_h) = build_tunnel_mask(&rows, settings.scale, settings.tunnel_width);
        Ok(Concept { width: width as i32, height: rows.len() as i32, rows, settings, tunnel_mask, mask_w, mask_h })
    }

    /// Folds a coordinate into `[0, n)` by mirror or tile, as the settings say.
    fn fold(&self, v: i32, n: i32) -> i32 {
        match self.settings.outside {
            Outside::Tile => v.rem_euclid(n),
            Outside::Mirror => {
                let m = v.rem_euclid(2 * n);
                if m >= n {
                    2 * n - 1 - m
                } else {
                    m
                }
            }
        }
    }

    /// The key at pixel (px, py); outside the image it mirrors or tiles by the settings.
    pub fn pixel(&self, px: i32, py: i32) -> u8 {
        self.rows[self.fold(py, self.height) as usize][self.fold(px, self.width) as usize]
    }

    /// A cell's place on the map: its offset from the spawn cell, turned by `o`.
    fn frame(&self, o: Orient, wx: i32, wz: i32) -> (i32, i32) {
        let (mut u, mut v) = (wx - ORIGIN, wz - ORIGIN);
        if o.swap {
            std::mem::swap(&mut u, &mut v);
        }
        if o.flip_x {
            u = -u - 1;
        }
        if o.flip_z {
            v = -v - 1;
        }
        (u, v)
    }

    fn pixel_in_frame(&self, u: i32, v: i32) -> u8 {
        let s = self.settings.scale;
        self.pixel(self.settings.spawn_pixel.0 + u.div_euclid(s), self.settings.spawn_pixel.1 + v.div_euclid(s))
    }

    /// The pixel key under a world cell (no spacing).
    pub fn pixel_of_cell(&self, o: Orient, wx: i32, wz: i32) -> u8 {
        let (u, v) = self.frame(o, wx, wz);
        self.pixel_in_frame(u, v)
    }

    /// The key of a world cell: the pixel key, except that a region cell within `spacing` cells (straight
    /// along an axis) of a different region or of wall is wall. Tunnels keep their key and make no gap.
    pub fn kind_at_cell(&self, o: Orient, wx: i32, wz: i32) -> u8 {
        let (u, v) = self.frame(o, wx, wz);
        let k = self.pixel_in_frame(u, v);
        let sp = self.settings.spacing;
        if k == WALL || k == TUNNEL || sp == 0 {
            return k;
        }
        for (du, dv) in [(sp, 0), (-sp, 0), (0, sp), (0, -sp)] {
            let n = self.pixel_in_frame(u + du, v + dv);
            if n != k && n != TUNNEL {
                return WALL;
            }
        }
        k
    }

    /// The generator region a key stands for.
    pub fn region_of_key(&self, key: u8) -> Option<&'static str> {
        self.settings.legend.get(&key).map(|l| l.region)
    }

    /// What the chunk (cx, cz) is: the region most of its samples belong to, or hollow. Chunk (0, 0) is always the
    /// maze. Chunk (1, 0) holds the two primary hunters (`hunter_spawns`, guaranteed there), so it takes the vote but
    /// is never hollow: a vote that would make it rock makes it the maze instead.
    pub fn classify_chunk(&self, o: Orient, cx: i32, cz: i32) -> ChunkClass {
        let maze = ChunkClass::Region(self.region_of_key(b'M').unwrap_or("maze"));
        if cx == 0 && cz == 0 {
            return maze;
        }
        match self.vote_chunk(o, cx, cz) {
            ChunkClass::Hollow if cx == 1 && cz == 0 => maze,
            class => class,
        }
    }

    /// The plain vote of `classify_chunk`: the region most of the chunk's sampled cells belong to, or hollow when it
    /// holds less than `min_region_share` of them.
    fn vote_chunk(&self, o: Orient, cx: i32, cz: i32) -> ChunkClass {
        let mut counts: HashMap<u8, i32> = HashMap::new();
        for i in 0..SAMPLES {
            for j in 0..SAMPLES {
                let k = self.kind_at_cell(o, cx.saturating_mul(CHUNK).saturating_add(2 + i * 5), cz.saturating_mul(CHUNK).saturating_add(2 + j * 5));
                if k != WALL && k != TUNNEL {
                    *counts.entry(k).or_default() += 1;
                }
            }
        }
        // most samples win; a tie goes to the lower key, so the answer never depends on hash order
        let best = counts.iter().map(|(k, n)| (*n, std::cmp::Reverse(*k))).max();
        match best {
            Some((n, std::cmp::Reverse(k))) if f64::from(n) / f64::from(SAMPLES * SAMPLES) >= self.settings.min_region_share => {
                self.region_of_key(k).map_or(ChunkClass::Hollow, ChunkClass::Region)
            }
            _ => ChunkClass::Hollow,
        }
    }

    /// Folds `base * scale + coord` (a cell offset from the spawn pixel's cell) into `[0, n)`, in i64 so a far chunk
    /// cannot overflow, the same mirror/tile way `fold` folds pixels.
    fn mask_fold(&self, base: i32, coord: i32, n: i32) -> i32 {
        let a = i64::from(base) * i64::from(self.settings.scale) + i64::from(coord);
        let n = i64::from(n);
        let f = match self.settings.outside {
            Outside::Tile => a.rem_euclid(n),
            Outside::Mirror => {
                let m = a.rem_euclid(2 * n);
                if m >= n {
                    2 * n - 1 - m
                } else {
                    m
                }
            }
        };
        f as i32
    }

    /// True when the world cell is inside a mineshaft tunnel. The shaft is the pixel-`T` network carved
    /// `tunnel_width` cells wide; this reads the precomputed connectivity-preserving mask (`build_tunnel_mask`: the
    /// network's centreline dilated to `tunnel_width`), so the tunnels never pinch shut the way the old per-cell
    /// narrowing did. The world cell is folded through `frame`/orient exactly as `pixel_of_cell`, keeping determinism
    /// (the seed's turn still applies) and `tunnel_width` the only width knob.
    pub fn is_tunnel_cell(&self, o: Orient, wx: i32, wz: i32) -> bool {
        let (u, v) = self.frame(o, wx, wz);
        let bx = self.mask_fold(self.settings.spawn_pixel.0, u, self.mask_w);
        let by = self.mask_fold(self.settings.spawn_pixel.1, v, self.mask_h);
        self.tunnel_mask[(by * self.mask_w + bx) as usize]
    }

    /// True when the chunk holds a carved tunnel cell. It reads the same mask `is_tunnel_cell` does (not the raw `T`
    /// pixels), so "has a tunnel" means the same thing to the carve gate, the dressing gate and the tests: a chunk
    /// that only grazes the rim of a wide blob — whose narrow centreline band runs through a neighbour — reports no
    /// tunnel, because none is carved there.
    pub fn chunk_has_tunnel(&self, o: Orient, cx: i32, cz: i32) -> bool {
        let (ox, oz) = (cx.saturating_mul(CHUNK), cz.saturating_mul(CHUNK));
        (0..CHUNK).any(|z| (0..CHUNK).any(|x| self.is_tunnel_cell(o, ox.saturating_add(x), oz.saturating_add(z))))
    }

    /// The seed's turn of the map. Without `seed_orient` the map is always as drawn.
    pub fn orient_of(&self, world: &str) -> Orient {
        if !self.settings.seed_orient {
            return Orient::default();
        }
        let mut rng = Rng::new(&format!("{world}:CONCEPT-ORIENT"));
        Orient { swap: rng.next_float() < 0.5, flip_x: rng.next_float() < 0.5, flip_z: rng.next_float() < 0.5 }
    }
}

/// Whether level 0 is laid out from the concept map (`"enabled"` in data/concept/concept_settings.json). Off, level 0
/// keeps Godot's region draw. Reads only the settings file, so it never builds the map while off.
pub fn enabled() -> bool {
    #[cfg(test)]
    if let Some(f) = FORCE.with(std::cell::Cell::get) {
        return f;
    }
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let (sf, st) = embedded::data_under("concept/")
            .find(|(p, _)| p.ends_with("concept_settings.json"))
            .unwrap_or_else(|| panic!("concept map: concept_settings.json is not embedded"));
        parse_settings(sf, st).unwrap_or_else(|e| panic!("concept map: {e}")).enabled
    })
}

#[cfg(test)]
thread_local! {
    static FORCE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Tests only: turns the concept map on for the rest of this test's thread (each test has its own).
#[cfg(test)]
pub fn concept_for_this_test() {
    FORCE.with(|f| f.set(Some(true)));
}

/// Tests only: runs `f` with the concept map forced on or off, then restores the earlier state.
#[cfg(test)]
pub fn with_concept<R>(on: bool, f: impl FnOnce() -> R) -> R {
    let old = FORCE.with(|c| c.replace(Some(on)));
    let r = f();
    FORCE.with(|c| c.set(old));
    r
}

/// The embedded concept. A broken built-in file is a build error, so this panics with the message.
pub fn concept() -> &'static Concept {
    static CELL: OnceLock<Concept> = OnceLock::new();
    CELL.get_or_init(|| {
        let find = |name: &str| {
            embedded::data_under("concept/")
                .find(|(p, _)| p.ends_with(name))
                .unwrap_or_else(|| panic!("concept map: {name} is not embedded"))
        };
        let (sf, st) = find("concept_settings.json");
        let settings = parse_settings(sf, st).unwrap_or_else(|e| panic!("concept map: {e}"));
        let (mf, mt) = find("map_concept.json");
        Concept::parse(mf, mt, settings).unwrap_or_else(|e| panic!("concept map: {e}"))
    })
}

/// The class of the level 0 chunk (cx, cz) in the world with seed text `world`.
pub fn chunk_class(world: &str, cx: i32, cz: i32) -> ChunkClass {
    if !enabled() {
        return ChunkClass::Region(super::regions::godot_region_for(world, cx, cz, 0));
    }
    let c = concept();
    c.classify_chunk(c.orient_of(world), cx, cz)
}

/// The generator region of the level 0 chunk (cx, cz): a hollow chunk counts as "maze".
pub fn region_of_chunk(world: &str, cx: i32, cz: i32) -> &'static str {
    match chunk_class(world, cx, cz) {
        ChunkClass::Region(r) => r,
        ChunkClass::Hollow => "maze",
    }
}

/// The player-facing label of the concept area under a world cell (upper case), for a map or a region banner.
pub fn label_at_cell(world: &str, wx: i32, wz: i32) -> Option<String> {
    if !enabled() {
        return None;
    }
    let c = concept();
    let k = c.pixel_of_cell(c.orient_of(world), wx, wz);
    c.settings.legend.get(&k).map(|l| l.label.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_settings(scale: i32, spacing: i32) -> Settings {
        let text = format!(
            r#"{{"schema":"{SETTINGS_SCHEMA}","scale":{scale},"spacing":{spacing},"tunnel_width":3,"spawn_pixel":[2,2],
            "outside":"tile","seed_orient":false,"min_region_share":0.25,
            "legend":{{"M":{{"region":"maze","label":"A"}},"P":{{"region":"prison","label":"B"}},"T":{{"region":"maze","label":"C"}}}}}}"#
        );
        parse_settings("test", &text).unwrap()
    }

    /// A maze room (west), a 3 pixel wide tunnel band (columns 5 to 7) and a prison room (east); the spawn pixel
    /// (2, 2) is in the maze.
    fn small(scale: i32, spacing: i32) -> Concept {
        let rows = ["#####TTT####", "#MMMMTTTPPPP", "#MMMMTTTPPPP", "#MMMMTTTPPPP", "#####TTT####"];
        let text = format!(r#"{{"schema":"{MAP_SCHEMA}","rows":{}}}"#, serde_json::to_string(&rows).unwrap());
        Concept::parse("test", &text, test_settings(scale, spacing)).unwrap()
    }

    #[test]
    fn the_real_map_loads_and_every_key_is_in_the_legend() {
        let c = concept();
        assert_eq!((c.width, c.height), (333, 260));
        assert!(c.settings.scale >= 1 && c.settings.tunnel_width >= 1);
        // Concept::parse already refused any key outside the legend; a sample of pixels agrees
        for (x, y) in [(0, 0), (55, 128), (332, 259), (170, 100)] {
            let k = c.pixel(x, y);
            assert!(k == WALL || c.region_of_key(k).is_some());
        }
    }

    #[test]
    fn a_sampled_pixel_colour_maps_to_its_region() {
        let c = concept();
        // the maze lies under the spawn pixel; each colour of the key names its generator region
        assert_eq!(c.pixel(c.settings.spawn_pixel.0, c.settings.spawn_pixel.1), b'M');
        assert_eq!(c.region_of_key(b'M'), Some("maze"));
        assert_eq!(c.region_of_key(b'P'), Some("prison"));
        assert_eq!(c.region_of_key(b'L'), Some("lab"));
        assert_eq!(c.region_of_key(b'A'), Some("apartment"));
        assert_eq!(c.region_of_key(b'G'), Some("greenhouse"));
        // every legend colour appears somewhere in the picture
        for key in c.settings.legend.keys() {
            assert!(c.rows.iter().any(|r| r.contains(key)), "key {} is in the legend but not in the map", *key as char);
        }
        // and in a small map
        let s = small(1, 0);
        assert_eq!(s.pixel(1, 1), b'M');
        assert_eq!(s.region_of_key(s.pixel(9, 2)), Some("prison"));
    }

    #[test]
    fn the_shipped_map_is_switched_off_and_forcing_it_on_works() {
        let (sf, st) = embedded::data_under("concept/").find(|(p, _)| p.ends_with("concept_settings.json")).unwrap();
        assert!(!parse_settings(sf, st).unwrap().enabled);
        assert!(!enabled());
        with_concept(true, || assert!(enabled()));
        assert!(!enabled());
        concept_for_this_test();
        assert!(enabled());
    }

    #[test]
    fn the_enabled_key_is_a_bool_and_defaults_to_on() {
        assert!(test_settings(1, 0).enabled);
        let text = r#"{"schema":"lr.concept_settings/1","enabled":"no","scale":1,"spacing":0,"tunnel_width":3,"spawn_pixel":[2,2],
            "outside":"tile","min_region_share":0.25,"legend":{}}"#;
        assert!(parse_settings("t", text).unwrap_err().contains("enabled"));
    }

    #[test]
    fn switched_off_no_chunk_is_hollow_and_every_region_is_godots_draw() {
        assert!(!enabled());
        for i in 0..10 {
            let world = format!("{}", 500 + i * 13);
            for cx in -15..15 {
                for cz in -15..15 {
                    assert_eq!(
                        chunk_class(&world, cx, cz),
                        ChunkClass::Region(super::super::regions::godot_region_for(&world, cx, cz, 0)),
                        "{world} {cx},{cz}"
                    );
                }
            }
            assert_eq!(label_at_cell(&world, 10, 10), None);
        }
    }

    #[test]
    fn the_spawn_chunk_is_maze_for_every_seed() {
        concept_for_this_test();
        for i in 0..60 {
            let world = format!("{}", 1000 + i * 37);
            assert_eq!(chunk_class(&world, 0, 0), ChunkClass::Region("maze"));
            assert_eq!(region_of_chunk(&world, 0, 0), "maze");
        }
    }

    /// Every turn of the map: the primary hunter chunk (1, 0) is never hollow, and (0, 0) is the maze.
    #[test]
    fn the_primary_hunter_chunk_is_never_hollow() {
        let c = concept();
        for swap in [false, true] {
            for flip_x in [false, true] {
                for flip_z in [false, true] {
                    let o = Orient { swap, flip_x, flip_z };
                    assert_ne!(c.classify_chunk(o, 1, 0), ChunkClass::Hollow, "{o:?}");
                    assert_eq!(c.classify_chunk(o, 0, 0), ChunkClass::Region("maze"), "{o:?}");
                    // a region vote is kept: (1, 0) only changes where the vote was rock
                    if let ChunkClass::Region(r) = c.vote_chunk(o, 1, 0) {
                        assert_eq!(c.classify_chunk(o, 1, 0), ChunkClass::Region(r), "{o:?}");
                    }
                }
            }
        }
        // the rule holds on any map
        let s = small(1, 0);
        assert_ne!(s.classify_chunk(Orient::default(), 1, 0), ChunkClass::Hollow);
    }

    #[test]
    fn tunnels_connect_two_regions() {
        // in the small map the tunnel band touches both rooms
        let s = small(1, 0);
        assert_eq!(s.pixel(4, 2), b'M');
        assert_eq!(s.pixel(5, 2), TUNNEL);
        assert_eq!(s.pixel(8, 2), b'P');
        // in the real map the tunnels touch at least two kinds of region (within two pixels)
        let c = concept();
        let mut touching: std::collections::BTreeSet<u8> = Default::default();
        for y in 0..c.height {
            for x in 0..c.width {
                if c.pixel(x, y) != TUNNEL {
                    continue;
                }
                for (dx, dy) in [(-2, 0), (2, 0), (0, -2), (0, 2)] {
                    let n = c.pixel(x + dx, y + dy);
                    if n != WALL && n != TUNNEL {
                        touching.insert(n);
                    }
                }
            }
        }
        assert!(touching.len() >= 2, "tunnels touch only {touching:?}");
    }

    #[test]
    fn tunnel_cells_are_three_wide_at_any_scale() {
        let o = Orient::default();
        for scale in [1, 3, 5] {
            let s = small(scale, 0);
            // the band is pixels 5 to 7: offsets 3 to 5 from the spawn pixel, so cells ORIGIN + 3s .. ORIGIN + 6s
            let (x0, x1) = (ORIGIN + 3 * scale, ORIGIN + 6 * scale);
            // across the band, on the middle row, exactly tunnel_width cells are open
            let across = (x0..x1).filter(|x| s.is_tunnel_cell(o, *x, ORIGIN)).count();
            assert_eq!(across, 3, "scale {scale}");
            // along the band every row has an open cell
            for z in (ORIGIN - 2 * scale)..(ORIGIN + 3 * scale) {
                assert!((x0..x1).any(|x| s.is_tunnel_cell(o, x, z)), "scale {scale}: row {z} is closed");
            }
            // nothing is open in the rooms
            assert!(!s.is_tunnel_cell(o, ORIGIN, ORIGIN));
        }
    }

    #[test]
    fn the_scale_changes_the_layout() {
        let o = Orient::default();
        let (a, b) = (small(1, 0), small(5, 0));
        // 20 cells east of the spawn: pixel 22 (tiled to 10, prison) at scale 1, pixel 6 (tunnel) at scale 5
        assert_eq!(a.pixel_of_cell(o, ORIGIN + 20, ORIGIN), b'P');
        assert_eq!(b.pixel_of_cell(o, ORIGIN + 20, ORIGIN), TUNNEL);
    }

    #[test]
    fn the_spacing_turns_region_edge_cells_into_wall() {
        let o = Orient::default();
        let (open, spaced) = (small(3, 0), small(3, 4));
        assert_eq!(open.kind_at_cell(o, ORIGIN, ORIGIN), b'M');
        // the first cell of pixel column 1 (the west edge of the maze room)
        let edge_x = ORIGIN - 3;
        assert_eq!(open.kind_at_cell(o, edge_x, ORIGIN), b'M');
        assert_eq!(spaced.kind_at_cell(o, edge_x, ORIGIN), WALL);
        // spacing never closes a tunnel
        assert_eq!(spaced.kind_at_cell(o, ORIGIN + 4 * 3, ORIGIN), TUNNEL);
    }

    #[test]
    fn the_same_seed_gives_the_same_layout_and_seeds_differ() {
        let c = concept();
        assert!(c.settings.seed_orient);
        assert_eq!(c.orient_of("123456"), c.orient_of("123456"));
        let mut turns = std::collections::BTreeSet::new();
        for i in 0..40 {
            let o = c.orient_of(&format!("{}", 9000 + i));
            turns.insert((o.swap, o.flip_x, o.flip_z));
        }
        assert!(turns.len() > 1, "every seed turned the map the same way");
        for cx in -8..8 {
            for cz in -8..8 {
                assert_eq!(chunk_class("777", cx, cz), chunk_class("777", cx, cz));
            }
        }
    }

    #[test]
    fn the_map_has_regions_and_hollow_chunks_and_never_ends() {
        concept_for_this_test();
        let mut regions: std::collections::BTreeSet<&str> = Default::default();
        let mut hollow = 0;
        for cx in -40..40 {
            for cz in -40..40 {
                match chunk_class("4242", cx, cz) {
                    ChunkClass::Region(r) => {
                        regions.insert(r);
                    }
                    ChunkClass::Hollow => hollow += 1,
                }
            }
        }
        assert!(regions.len() >= 6, "only {regions:?}");
        assert!(hollow > 0);
    }

    #[test]
    fn outside_the_image_the_map_mirrors_or_tiles() {
        let mut c = small(1, 0);
        let w = c.width;
        c.settings.outside = Outside::Mirror;
        assert_eq!(c.pixel(-1, 1), c.pixel(0, 1));
        assert_eq!(c.pixel(w, 2), c.pixel(w - 1, 2));
        c.settings.outside = Outside::Tile;
        assert_eq!(c.pixel(w + 3, 2), c.pixel(3, 2));
        assert_eq!(c.pixel(-1, 2), c.pixel(w - 1, 2));
    }

    #[test]
    fn settings_reject_unknown_regions_and_bad_outside() {
        let bad = r#"{"schema":"lr.concept_settings/1","scale":1,"spacing":0,"tunnel_width":3,"spawn_pixel":[0,0],"outside":"tile","min_region_share":0.2,"legend":{"M":{"region":"narnia"}}}"#;
        assert!(parse_settings("t", bad).is_err());
        let bad = bad.replace("narnia", "maze").replace("tile", "wrap");
        assert!(parse_settings("t", &bad).is_err());
    }

    #[test]
    fn labels_are_upper_case() {
        for l in concept().settings.legend.values() {
            assert_eq!(l.label, l.label.to_uppercase());
        }
    }

    /// Every turn of the map, as `orient_of` draws it. A seed only ever changes the layout through this, so the eight
    /// turns stand in for every seed.
    fn all_orients() -> Vec<Orient> {
        let mut v = Vec::new();
        for swap in [false, true] {
            for flip_x in [false, true] {
                for flip_z in [false, true] {
                    v.push(Orient { swap, flip_x, flip_z });
                }
            }
        }
        v
    }

    /// The number of 4-connected components of the cells `pred` opens that touch the core window (chunks
    /// `-core..=core`), flooding through an extra `margin` chunks so a component that only dips out of the core near
    /// its edge is not miscounted as two. This makes the count immune to the hard window boundary that would
    /// otherwise split a tunnel (or a wide pixel-`T` blob) that merely grazes the edge.
    fn comps_touching_core(pred: &dyn Fn(i32, i32) -> bool, core: i32, margin: i32) -> usize {
        use std::collections::HashSet;
        let lo = -(core + margin) * CHUNK;
        let hi = (core + margin + 1) * CHUNK;
        let core_lo = -core * CHUNK;
        let core_hi = (core + 1) * CHUNK;
        let mut cells: HashSet<(i32, i32)> = HashSet::new();
        for z in lo..hi {
            for x in lo..hi {
                if pred(x, z) {
                    cells.insert((x, z));
                }
            }
        }
        let mut seen: HashSet<(i32, i32)> = HashSet::new();
        let mut comps = 0;
        for &start in &cells {
            if !seen.insert(start) {
                continue;
            }
            let mut touches = false;
            let mut stack = vec![start];
            while let Some((x, z)) = stack.pop() {
                if (core_lo..core_hi).contains(&x) && (core_lo..core_hi).contains(&z) {
                    touches = true;
                }
                for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let n = (x + dx, z + dz);
                    if cells.contains(&n) && seen.insert(n) {
                        stack.push(n);
                    }
                }
            }
            if touches {
                comps += 1;
            }
        }
        comps
    }

    /// The carved tunnel cells have the same 4-connected component count as the pixel-`T` network over the same
    /// window, for every turn of the map: the connectivity-preserving mask never splits a shaft. The old per-cell
    /// narrowing failed this badly (tens of components where the pixel network has one or a few), marooning whole
    /// districts; the mask carries it. A `tw = 99` escape hatch is not available here, but the mask is a subset of
    /// the pixel-`T` cells, so it can only ever match or (never) exceed the pixel count.
    #[test]
    fn carved_tunnels_match_the_pixel_network_connectivity() {
        let c = concept();
        for o in all_orients() {
            let tunnel = |x: i32, z: i32| c.is_tunnel_cell(o, x, z);
            let pixel_t = |x: i32, z: i32| {
                let (u, v) = c.frame(o, x, z);
                c.pixel(c.settings.spawn_pixel.0 + u.div_euclid(c.settings.scale), c.settings.spawn_pixel.1 + v.div_euclid(c.settings.scale)) == TUNNEL
            };
            // two window sizes: near spawn the pixel network is one component, a touch wider it is a few. Either way
            // the carved tunnels must agree exactly (the buggy narrowing gave dozens here).
            for (core, margin) in [(8, 3), (10, 3)] {
                let carved = comps_touching_core(&tunnel, core, margin);
                let pixels = comps_touching_core(&pixel_t, core, margin);
                assert_eq!(carved, pixels, "orient {o:?}, core {core}: {carved} carved vs {pixels} pixel components");
                assert!(carved >= 1, "orient {o:?}: the window holds no tunnel to check");
            }
        }
    }

    /// Every cell the mask opens is a pixel-`T` cell: the narrowing only ever removes `T` cells, never adds floor
    /// outside the drawn tunnels. This is what lets the connectivity test lean on "subset of pixel-`T`".
    #[test]
    fn the_tunnel_mask_is_a_subset_of_the_pixel_network() {
        let c = concept();
        for o in all_orients() {
            for cz in -8..8 {
                for cx in -8..8 {
                    for z in 0..CHUNK {
                        for x in 0..CHUNK {
                            let (wx, wz) = (cx * CHUNK + x, cz * CHUNK + z);
                            if c.is_tunnel_cell(o, wx, wz) {
                                assert_eq!(c.pixel_of_cell(o, wx, wz), TUNNEL, "mask cell ({wx}, {wz}) is not a tunnel pixel");
                            }
                        }
                    }
                }
            }
        }
    }
}
