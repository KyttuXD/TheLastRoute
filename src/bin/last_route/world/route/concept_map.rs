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
//! whatever the scale (`is_tunnel_cell`), and they are what joins the regions. Chunk (0, 0) is always the maze.
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
/// A tunnel run longer than this many pixels is counted as this long.
const RUN_CAP: i32 = 16;
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
        Ok(Concept { width: width as i32, height: rows.len() as i32, rows, settings })
    }

    /// The key at pixel (px, py); outside the image it mirrors or tiles by the settings.
    pub fn pixel(&self, px: i32, py: i32) -> u8 {
        let fold = |v: i32, n: i32| match self.settings.outside {
            Outside::Tile => v.rem_euclid(n),
            Outside::Mirror => {
                let m = v.rem_euclid(2 * n);
                if m >= n {
                    2 * n - 1 - m
                } else {
                    m
                }
            }
        };
        self.rows[fold(py, self.height) as usize][fold(px, self.width) as usize]
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

    /// What the chunk (cx, cz) is. Chunk (0, 0) is always the maze.
    pub fn classify_chunk(&self, o: Orient, cx: i32, cz: i32) -> ChunkClass {
        if cx == 0 && cz == 0 {
            return ChunkClass::Region(self.region_of_key(b'M').unwrap_or("maze"));
        }
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

    /// True when the world cell is inside a mineshaft tunnel: a tunnel pixel, narrowed to `tunnel_width` cells
    /// across the shorter way of its run of tunnel pixels (the length along the tunnel is not limited).
    pub fn is_tunnel_cell(&self, o: Orient, wx: i32, wz: i32) -> bool {
        let (u, v) = self.frame(o, wx, wz);
        let s = self.settings.scale;
        let (pu, pv) = (u.div_euclid(s), v.div_euclid(s));
        let at = |du: i32, dv: i32| {
            self.pixel(self.settings.spawn_pixel.0 + pu + du, self.settings.spawn_pixel.1 + pv + dv)
        };
        if at(0, 0) != TUNNEL {
            return false;
        }
        let run = |dx: i32, dz: i32| {
            let mut n = 0;
            while n < RUN_CAP && at(dx * (n + 1), dz * (n + 1)) == TUNNEL {
                n += 1;
            }
            n
        };
        let (l, r, up, down) = (run(-1, 0), run(1, 0), run(0, -1), run(0, 1));
        let horizontal = l + r <= up + down;
        let (p, pos, before, after) = if horizontal { (pu, u, l, r) } else { (pv, v, up, down) };
        let (start, end) = ((p - before) * s, (p + after + 1) * s);
        let tw = self.settings.tunnel_width;
        if end - start <= tw {
            return true;
        }
        let lo = (start + end) / 2 - tw / 2;
        pos >= lo && pos < lo + tw
    }

    /// True when any tunnel pixel lies under the chunk (a coarse test, one look per pixel).
    pub fn chunk_has_tunnel(&self, o: Orient, cx: i32, cz: i32) -> bool {
        let step = self.settings.scale.max(1) as usize;
        for x in (0..CHUNK).step_by(step).chain([CHUNK - 1]) {
            for z in (0..CHUNK).step_by(step).chain([CHUNK - 1]) {
                if self.pixel_of_cell(o, cx.saturating_mul(CHUNK).saturating_add(x), cz.saturating_mul(CHUNK).saturating_add(z)) == TUNNEL {
                    return true;
                }
            }
        }
        false
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
    fn the_spawn_chunk_is_maze_for_every_seed() {
        for i in 0..60 {
            let world = format!("{}", 1000 + i * 37);
            assert_eq!(chunk_class(&world, 0, 0), ChunkClass::Region("maze"));
            assert_eq!(region_of_chunk(&world, 0, 0), "maze");
        }
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
}
