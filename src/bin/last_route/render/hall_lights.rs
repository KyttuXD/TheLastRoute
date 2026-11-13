//! Hall lights: fairy-light strings hung across halls from wall to wall, and small wall lamps (sconces) on hall walls.
//! Sizes, colours and how often they appear are data/render/hall_lights.json (`lr.hall_lights/1`, docs/HALL_LIGHTS.md).
//!
//! Godot hangs its fairy lights in `CeilingDecor` (game/world/streaming/ceiling_decor.gd), placed by
//! `streamed_facility_view_3d.gd` in the maze and the carnival only: a wire 0.7 to 1.2 m long with three to five
//! bulbs. Here a string runs the whole width of its hall, wall face to wall face, and a strung stretch of hall gets
//! several of them. The bulb colours are Godot's five (`BULB_COLORS`). The wall lamps are not in Godot.
//!
//! Placement is a hash of the run's seed, the chunk and the cell ([`seed`], [`hash`]): the route generator's random
//! numbers are not touched, so the prop placement that is checked against the Godot goldens does not move. Like the
//! hall arches (render/hall_arch.rs) everything is in the chunk's own frame and reads no cell outside the chunk, so a
//! chunk's lights are the same in every window that holds it. Nothing here is solid.
//!
//! This module is pure: no GL, no assets, no `Facility`. The caller hands it a [`Site`] and turns the quads and boxes
//! into mesh (`render::world_mesh::add_hall_lights`) and the lights into lamps (`render::world_mesh::lamps`).

use super::hall_arch::{ChunkView, DIRS};
use crate::content::embedded;
use macroquad::prelude::{vec3, Vec3};
use serde_json::Value;
use std::sync::OnceLock;

pub const PATH: &str = "render/hall_lights.json";
const SCHEMA: &str = "lr.hall_lights/1";

/// How often something appears, by region id: a share from 0 to 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Chance {
    default: f32,
    regions: Vec<(String, f32)>,
}

impl Chance {
    /// The share for `region`: its own row, else the default.
    pub fn of(&self, region: &str) -> f32 {
        self.regions.iter().find(|(r, _)| r == region).map_or(self.default, |(_, c)| *c)
    }

    fn parse(v: Option<&Value>, at: &str) -> Result<Chance, String> {
        let rows = v.and_then(Value::as_object).ok_or_else(|| format!("{at}.chance: expected an object"))?;
        let mut out = Chance { default: 0.0, regions: vec![] };
        for (key, n) in rows {
            let n = n.as_f64().filter(|n| (0.0..=1.0).contains(n)).ok_or_else(|| format!("{at}.chance.{key}: expected a number from 0 to 1"))? as f32;
            if key == "default" {
                out.default = n;
            } else {
                out.regions.push((key.clone(), n));
            }
        }
        Ok(out)
    }
}

/// The light a string or a wall lamp gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightCfg {
    pub color: [f32; 3],
    pub energy: f32,
    pub range: f32,
    pub flicker_hz: f32,
    pub flicker_depth: f32,
}

/// The `fairy` block.
#[derive(Clone, Debug, PartialEq)]
pub struct FairyCfg {
    pub chance: Chance,
    pub stretch_cells: i32,
    pub string_every_cells: i32,
    pub string_phase: i32,
    pub max_width_cells: i32,
    pub drop_m: f32,
    pub sag_per_m: f32,
    pub sag_max_m: f32,
    pub wire_thickness_m: f32,
    pub wire_segments: i32,
    pub wire_color: [f32; 3],
    pub bulb_spacing_m: f32,
    pub bulb_size_m: f32,
    pub bulb_colors: Vec<[f32; 3]>,
    pub light_every_strings: i32,
    pub light: LightCfg,
}

/// The `sconce` block.
#[derive(Clone, Debug, PartialEq)]
pub struct SconceCfg {
    pub chance: Chance,
    pub every_cells: i32,
    pub phase: i32,
    pub height_m: f32,
    pub plate_m: [f32; 3],
    pub shade_m: [f32; 3],
    pub plate_color: [f32; 3],
    pub shade_color: [f32; 3],
    pub light_out_m: f32,
    pub light: LightCfg,
}

/// The parsed `lr.hall_lights/1`.
#[derive(Clone, Debug, PartialEq)]
pub struct Cfg {
    pub fairy: FairyCfg,
    pub sconce: SconceCfg,
}

fn num(v: &Value, at: &str, key: &str) -> Result<f32, String> {
    v.get(key).and_then(Value::as_f64).map(|x| x as f32).ok_or_else(|| format!("{at}.{key}: expected a number"))
}

fn whole(v: &Value, at: &str, key: &str, lo: i64, hi: i64) -> Result<i32, String> {
    v.get(key)
        .and_then(Value::as_i64)
        .filter(|n| (lo..=hi).contains(n))
        .map(|n| n as i32)
        .ok_or_else(|| format!("{at}.{key}: expected a whole number from {lo} to {hi}"))
}

/// "#rrggbb" as 0..1 channels.
fn hex(v: Option<&Value>, at: &str) -> Result<[f32; 3], String> {
    let bad = || format!("{at}: expected a colour \"#rrggbb\"");
    let text = v.and_then(Value::as_str).and_then(|s| s.strip_prefix('#')).filter(|s| s.len() == 6 && s.is_ascii()).ok_or_else(bad)?;
    let mut out = [0.0; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|_| bad())? as f32 / 255.0;
    }
    Ok(out)
}

fn three(v: &Value, at: &str, key: &str) -> Result<[f32; 3], String> {
    v.get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() == 3)
        .and_then(|a| Some([a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32]))
        .filter(|s| s.iter().all(|x| *x > 0.0))
        .ok_or_else(|| format!("{at}.{key}: expected 3 numbers above 0"))
}

impl LightCfg {
    fn parse(v: Option<&Value>, at: &str) -> Result<LightCfg, String> {
        let v = v.ok_or_else(|| format!("{at}: expected an object"))?;
        let light = LightCfg {
            color: hex(v.get("color"), &format!("{at}.color"))?,
            energy: num(v, at, "energy")?,
            range: num(v, at, "range")?,
            flicker_hz: num(v, at, "flicker_hz")?,
            flicker_depth: num(v, at, "flicker_depth")?,
        };
        if light.energy < 0.0 || light.range <= 0.0 {
            return Err(format!("{at}: energy must not be negative and range must be above 0"));
        }
        Ok(light)
    }
}

impl Cfg {
    pub fn parse(text: &str) -> Result<Cfg, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
            return Err(format!("schema: expected {SCHEMA}"));
        }
        let f = v.get("fairy").ok_or("fairy: expected an object")?;
        let string_every_cells = whole(f, "fairy", "string_every_cells", 1, 45)?;
        let colors = f.get("bulb_colors").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or("fairy.bulb_colors: expected a list of colours")?;
        let mut bulb_colors = vec![];
        for c in colors {
            bulb_colors.push(hex(Some(c), "fairy.bulb_colors")?);
        }
        let fairy = FairyCfg {
            chance: Chance::parse(f.get("chance"), "fairy")?,
            stretch_cells: whole(f, "fairy", "stretch_cells", 1, 45)?,
            string_every_cells,
            string_phase: whole(f, "fairy", "string_phase", 0, string_every_cells as i64 - 1)?,
            max_width_cells: whole(f, "fairy", "max_width_cells", 1, 12)?,
            drop_m: num(f, "fairy", "drop_m")?,
            sag_per_m: num(f, "fairy", "sag_per_m")?,
            sag_max_m: num(f, "fairy", "sag_max_m")?,
            wire_thickness_m: num(f, "fairy", "wire_thickness_m")?,
            wire_segments: whole(f, "fairy", "wire_segments", 1, 12)?,
            wire_color: hex(f.get("wire_color"), "fairy.wire_color")?,
            bulb_spacing_m: num(f, "fairy", "bulb_spacing_m")?,
            bulb_size_m: num(f, "fairy", "bulb_size_m")?,
            bulb_colors,
            light_every_strings: whole(f, "fairy", "light_every_strings", 1, 45)?,
            light: LightCfg::parse(f.get("light"), "fairy.light")?,
        };
        if fairy.drop_m < 0.0 || fairy.sag_per_m < 0.0 || fairy.sag_max_m < 0.0 || fairy.wire_thickness_m <= 0.0 || fairy.bulb_size_m <= 0.0 {
            return Err("fairy: drop and sag must not be negative, the wire and the bulbs must have a size".into());
        }
        if fairy.bulb_spacing_m < 0.2 {
            return Err("fairy.bulb_spacing_m: at least 0.2 (every bulb is four triangles)".into());
        }
        let s = v.get("sconce").ok_or("sconce: expected an object")?;
        let every_cells = whole(s, "sconce", "every_cells", 1, 45)?;
        let sconce = SconceCfg {
            chance: Chance::parse(s.get("chance"), "sconce")?,
            every_cells,
            phase: whole(s, "sconce", "phase", 0, every_cells as i64 - 1)?,
            height_m: num(s, "sconce", "height_m")?,
            plate_m: three(s, "sconce", "plate_m")?,
            shade_m: three(s, "sconce", "shade_m")?,
            plate_color: hex(s.get("plate_color"), "sconce.plate_color")?,
            shade_color: hex(s.get("shade_color"), "sconce.shade_color")?,
            light_out_m: num(s, "sconce", "light_out_m")?,
            light: LightCfg::parse(s.get("light"), "sconce.light")?,
        };
        if sconce.height_m <= 0.0 || !(0.0..0.5).contains(&sconce.light_out_m) {
            return Err("sconce: height_m must be above 0 and light_out_m under half a cell".into());
        }
        Ok(Cfg { fairy, sconce })
    }
}

/// The hall-light tunables compiled in. A bad file is reported on stderr and the built-in copy is used.
pub fn config() -> &'static Cfg {
    static CFG: OnceLock<Cfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        Cfg::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            Cfg::parse(include_str!("../data/render/hall_lights.json")).expect("hall_lights.json is valid")
        })
    })
}

// -- hashing ---------------------------------------------------------------------------------------------------------

/// The splitmix64 finish.
fn mix(mut h: u64) -> u64 {
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d049bb133111eb);
    h ^ (h >> 31)
}

/// A hash of `seed` and four whole numbers.
pub fn hash(seed: u64, parts: [i32; 4]) -> u64 {
    let mut h = seed;
    for p in parts {
        h = mix(h ^ (p as u32 as u64).wrapping_add(0x9e3779b97f4a7c15));
    }
    h
}

/// A hash as a number from 0 up to (not including) 1.
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// The seed of one chunk's hall lights: the floor's seed text (FNV-1a) and the chunk's place in the world.
pub fn seed(text: &str, chunk: (i32, i32)) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    hash(h, [chunk.0, chunk.1, 0x4c49, 0x4854])
}

// -- placement -------------------------------------------------------------------------------------------------------

/// What the placement reads of one chunk. Every closure takes a chunk-local cell (x, z) inside the chunk.
pub struct Site<'a> {
    /// The chunk's grid, as the hall arches read it. `ramp` marks every cell nothing may hang over (no ceiling), and
    /// `height` is the y of the ceiling over an open cell. A sewer view gets no hall lights.
    pub view: &'a ChunkView<'a>,
    /// The chunk's seed ([`seed`]).
    pub seed: u64,
    /// The share of hall stretches that are strung, at an open cell (its region's `fairy.chance`).
    pub fairy_chance: &'a dyn Fn(i32, i32) -> f32,
    /// The share of candidate wall faces that carry a wall lamp, at the open cell the face looks into.
    pub sconce_chance: &'a dyn Fn(i32, i32) -> f32,
    /// Whether a prop stands in the open cell: no wall lamp is hung over it.
    pub taken: &'a dyn Fn(i32, i32) -> bool,
}

/// One flat quad (TL, TR, BR, BL).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub c: [Vec3; 4],
    pub n: Vec3,
    pub color: [f32; 3],
    /// Drawn unlit at its own colour (a bulb), not lit by the lamps (the wire).
    pub unlit: bool,
}

/// One axis-aligned box; `size` is its full extent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightBox {
    pub centre: Vec3,
    pub size: Vec3,
    pub color: [f32; 3],
    pub unlit: bool,
}

/// One light for the floor's lamp list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HallLight {
    pub pos: Vec3,
    /// A string's glow (`fairy.light`), not a wall lamp's (`sconce.light`).
    pub fairy: bool,
}

/// One string, for the checks: its two ends on the wall faces, the open cell it starts in, its width in cells and
/// how many bulbs it carries.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HungString {
    pub from: Vec3,
    pub to: Vec3,
    pub cell: (i32, i32),
    pub width: i32,
    pub bulbs: i32,
}

/// What the placement produced. `strings` and `sconces` are the log the checks read (as the hall arches' `log`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Decor {
    pub quads: Vec<Quad>,
    pub boxes: Vec<LightBox>,
    pub lights: Vec<HallLight>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub strings: Vec<HungString>,
    /// The wall-face point (at the lamp's height) of every wall lamp.
    #[cfg_attr(not(test), allow(dead_code))]
    pub sconces: Vec<Vec3>,
}

impl Decor {
    /// How many triangles the quads and boxes are drawn with.
    #[cfg(test)]
    pub fn triangles(&self) -> usize {
        self.quads.len() * 2 + self.boxes.len() * 12
    }
}

/// The hall lights the open cell (x, z) owns: the strings that start at a wall on its low side (one across x, one
/// across z), and the wall lamps on the wall faces that look into it. Each is owned by exactly one cell, so a chunk
/// drawn in parts (or cell by cell) draws each once.
pub fn cell_lights(site: &Site, cfg: &Cfg, x: i32, z: i32, out: &mut Decor) {
    if site.view.sewer || !site.view.is_open(x, z) {
        return;
    }
    fairy_string(site, &cfg.fairy, x, z, 0, out);
    fairy_string(site, &cfg.fairy, x, z, 1, out);
    for (k, d) in DIRS.iter().enumerate() {
        sconce(site, &cfg.sconce, x, z, k as i32, *d, out);
    }
}

/// The hall lights of a whole chunk, rows of z, x across each row.
#[cfg(test)]
pub fn chunk_lights(site: &Site, cfg: &Cfg) -> Decor {
    let mut out = Decor::default();
    for z in 0..site.view.size {
        for x in 0..site.view.size {
            cell_lights(site, cfg, x, z, &mut out);
        }
    }
    out
}

/// One string across the hall from the open cell (x, z), along +x (`axis` 0) or +z (`axis` 1), if the cell is the
/// first of the hall's width (a wall on its low side), the hall's stretch is strung and the far wall is in reach.
fn fairy_string(site: &Site, cfg: &FairyCfg, x: i32, z: i32, axis: i32, out: &mut Decor) {
    let view = site.view;
    let step = if axis == 0 { (1, 0) } else { (0, 1) };
    let (across, along) = if axis == 0 { (x, z) } else { (z, x) };
    // the cheap tests first: most cells stop here
    if along.rem_euclid(cfg.string_every_cells) != cfg.string_phase {
        return;
    }
    let (px, pz) = (x - step.0, z - step.1);
    if !view.inside(px, pz) || view.is_open(px, pz) || !(view.brick)(px, pz) {
        return;
    }
    let chance = (site.fairy_chance)(x, z);
    let stretch = along.div_euclid(cfg.stretch_cells);
    if chance <= 0.0 || unit(hash(site.seed, [axis, across, stretch, 0])) >= chance {
        return;
    }
    // the hall's width here: open cells up to the far wall, which must be real wall of this chunk
    let mut width = 1;
    while width <= cfg.max_width_cells && view.is_open(x + step.0 * width, z + step.1 * width) {
        width += 1;
    }
    let far = (x + step.0 * width, z + step.1 * width);
    if width > cfg.max_width_cells || !view.inside(far.0, far.1) || !(view.brick)(far.0, far.1) {
        return;
    }
    // a hall runs on past the string, on one side at least (not a one-cell nook)
    let side = (step.1, step.0);
    if !view.is_open(x + side.0, z + side.1) && !view.is_open(x - side.0, z - side.1) {
        return;
    }
    // nothing in a room, nothing where there is no ceiling; the string hangs from the lowest ceiling of its span
    let mut top = f32::INFINITY;
    for k in 0..width {
        let (cx, cz) = (x + step.0 * k, z + step.1 * k);
        if (view.room)(cx, cz) || (view.ramp)(cx, cz) {
            return;
        }
        top = top.min((view.height)(cx, cz));
    }
    let dir = vec3(step.0 as f32, 0.0, step.1 as f32);
    let run = vec3(side.0 as f32, 0.0, side.1 as f32);
    // the wall face on the low side of the first cell, in the middle of the cell along the hall
    let from = vec3(x as f32 + 0.5 * side.0 as f32, top - cfg.drop_m, z as f32 + 0.5 * side.1 as f32);
    let span = width as f32;
    let to = from + dir * span;
    let sag = (cfg.sag_per_m * span).min(cfg.sag_max_m);
    // a parabola: the ends on the walls, the middle `sag` lower
    let point = |t: f32| from + dir * (span * t) - Vec3::Y * (sag * 4.0 * t * (1.0 - t));
    let half = Vec3::Y * (cfg.wire_thickness_m * 0.5);
    for i in 0..cfg.wire_segments {
        let a = if i == 0 { from } else { point(i as f32 / cfg.wire_segments as f32) };
        let b = if i + 1 == cfg.wire_segments { to } else { point((i + 1) as f32 / cfg.wire_segments as f32) };
        out.quads.push(Quad { c: [a + half, b + half, b - half, a - half], n: run, color: cfg.wire_color, unlit: false });
    }
    let bulbs = ((span / cfg.bulb_spacing_m).round() as i32).clamp(2, 60);
    let r = cfg.bulb_size_m * 0.5;
    for i in 0..bulbs {
        let c = point((i as f32 + 0.5) / bulbs as f32) - Vec3::Y * r;
        let pick = hash(site.seed, [axis, x, z, 1 + i]) % cfg.bulb_colors.len() as u64;
        let color = cfg.bulb_colors[pick as usize];
        // two diamonds: one facing along the hall, one facing the floor, so the bulb shows from every side a runner
        // sees it from
        out.quads.push(Quad { c: [c + Vec3::Y * r, c + dir * r, c - Vec3::Y * r, c - dir * r], n: run, color, unlit: true });
        out.quads.push(Quad { c: [c + run * r, c + dir * r, c - run * r, c - dir * r], n: -Vec3::Y, color, unlit: true });
    }
    if along.div_euclid(cfg.string_every_cells).rem_euclid(cfg.light_every_strings) == 0 {
        out.lights.push(HallLight { pos: point(0.5) - Vec3::Y * (cfg.bulb_size_m + 0.05), fairy: true });
    }
    out.strings.push(HungString { from, to, cell: (x, z), width, bulbs });
}

/// One wall lamp on the wall face that looks along `d` into the open cell (x, z); `k` is `d`'s index in `DIRS`.
fn sconce(site: &Site, cfg: &SconceCfg, x: i32, z: i32, k: i32, d: (i32, i32), out: &mut Decor) {
    let view = site.view;
    // position along the wall run, as the pilasters count it
    let along = if d.1 != 0 { x } else { z };
    if along.rem_euclid(cfg.every_cells) != cfg.phase {
        return;
    }
    let (wx, wz) = (x - d.0, z - d.1);
    if !view.inside(wx, wz) || view.is_open(wx, wz) || !(view.brick)(wx, wz) {
        return;
    }
    let chance = (site.sconce_chance)(x, z);
    if chance <= 0.0 || unit(hash(site.seed, [wx, wz, k, 2])) >= chance {
        return;
    }
    // the wall and the hall carry on a cell either side: not on a corner, not beside an opening
    let side = (d.1.abs(), d.0.abs());
    for s in [-1, 1] {
        if view.is_open(wx + side.0 * s, wz + side.1 * s) || !view.is_open(x + side.0 * s, z + side.1 * s) {
            return;
        }
    }
    if (view.room)(x, z) || (view.ramp)(x, z) || (site.taken)(x, z) {
        return;
    }
    // under a low ceiling the lamp comes down with it
    let y = cfg.height_m.min((view.height)(x, z) - cfg.plate_m[1]);
    let n = vec3(d.0 as f32, 0.0, d.1 as f32);
    let base = vec3(wx as f32 + 0.5 + d.0 as f32 * 0.5, y, wz as f32 + 0.5 + d.1 as f32 * 0.5);
    // (width along the wall, height, depth out of it) as a box in the chunk's axes
    let size = |s: [f32; 3]| if d.1 != 0 { vec3(s[0], s[1], s[2]) } else { vec3(s[2], s[1], s[0]) };
    out.boxes.push(LightBox { centre: base + n * (cfg.plate_m[2] * 0.5), size: size(cfg.plate_m), color: cfg.plate_color, unlit: false });
    out.boxes.push(LightBox { centre: base + n * (cfg.plate_m[2] + cfg.shade_m[2] * 0.5), size: size(cfg.shade_m), color: cfg.shade_color, unlit: true });
    out.lights.push(HallLight { pos: base + n * cfg.light_out_m, fairy: false });
    out.sconces.push(base);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: i32 = 45;
    const CEILING: f32 = 3.2;

    fn shipped() -> Cfg {
        Cfg::parse(include_str!("../data/render/hall_lights.json")).expect("the shipped file parses")
    }

    /// A chunk of solid wall with the given rectangles (x, z, w, h) carved open.
    fn carved(rects: &[(i32, i32, i32, i32)]) -> Vec<Vec<bool>> {
        let mut open = vec![vec![false; SIZE as usize]; SIZE as usize];
        for &(x0, z0, w, h) in rects {
            for z in z0..z0 + h {
                for x in x0..x0 + w {
                    open[z as usize][x as usize] = true;
                }
            }
        }
        open
    }

    /// A hall `width` cells wide running the whole chunk along x, its near wall on row 9.
    fn hall_along_x(width: i32) -> Vec<Vec<bool>> {
        carved(&[(0, 10, SIZE, width)])
    }

    /// What the placement is asked for in a test.
    struct Ask<'a> {
        seed: u64,
        sewer: bool,
        fairy: f32,
        sconce: f32,
        room: &'a dyn Fn(i32, i32) -> bool,
        taken: &'a dyn Fn(i32, i32) -> bool,
    }

    fn nowhere(_: i32, _: i32) -> bool {
        false
    }

    fn ask(fairy: f32, sconce: f32) -> Ask<'static> {
        Ask { seed: 7, sewer: false, fairy, sconce, room: &nowhere, taken: &nowhere }
    }

    fn lights_of(open: &[Vec<bool>], a: &Ask) -> Decor {
        let is_open = |x: i32, z: i32| open[z as usize][x as usize];
        let view = ChunkView { size: SIZE, sewer: a.sewer, open: &is_open, brick: &|_, _| true, room: a.room, ramp: &|_, _| false, height: &|_, _| CEILING };
        let (fairy, sconce) = (a.fairy, a.sconce);
        let site = Site { view: &view, seed: a.seed, fairy_chance: &move |_, _| fairy, sconce_chance: &move |_, _| sconce, taken: a.taken };
        chunk_lights(&site, &shipped())
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1.0e-4
    }

    #[test]
    fn the_shipped_file_parses_and_strings_only_the_maze_and_the_carnival() {
        let cfg = shipped();
        assert_eq!(config(), &cfg);
        assert!(cfg.fairy.chance.of("maze") > 0.0 && cfg.fairy.chance.of("carnival") > 0.0);
        for region in ["hospital", "office", "mall", "sewer", "prison"] {
            assert_eq!(cfg.fairy.chance.of(region), 0.0, "{region}");
        }
        assert_eq!(cfg.fairy.bulb_colors.len(), 5);
        // every third cell at phase one never falls on a hall arch (cells 3, 15, 27, 39 of a run)
        for along in 0..SIZE {
            if along.rem_euclid(cfg.fairy.string_every_cells) == cfg.fairy.string_phase {
                assert_ne!(along % 12, 3);
            }
            if along.rem_euclid(cfg.sconce.every_cells) == cfg.sconce.phase {
                assert!(!crate::render::hall_arch::is_pilaster_column(along));
            }
        }
        assert!(Cfg::parse("{}").is_err());
        assert!(Cfg::parse(&include_str!("../data/render/hall_lights.json").replacen("\"maze\": 0.16", "\"maze\": 1.5", 1)).is_err());
    }

    #[test]
    fn strings_span_exactly_wall_to_wall_for_halls_one_to_four_cells_wide() {
        let cfg = shipped();
        for width in 1..=4 {
            let decor = lights_of(&hall_along_x(width), &ask(1.0, 0.0));
            // one string on every third cell of the hall's length, all across z
            assert_eq!(decor.strings.len(), 15, "width {width}");
            let (near, far) = (10.0, 10.0 + width as f32);
            for s in &decor.strings {
                assert_eq!((s.cell.1, s.width), (10, width), "{s:?}");
                assert_eq!(s.cell.0 % 3, 1, "{s:?}");
                assert!(close(s.from.z, near) && close(s.to.z, far), "width {width}: {s:?}");
                assert!(close(s.from.x, s.cell.0 as f32 + 0.5) && close(s.to.x, s.from.x), "{s:?}");
                assert!(close(s.from.y, CEILING - cfg.fairy.drop_m) && close(s.to.y, s.from.y), "{s:?}");
                assert!(close((s.to - s.from).length(), width as f32), "the string is as long as the hall is wide");
            }
            // the same from the geometry: each wire runs from the near wall's face to the far wall's without a gap
            let wires: Vec<&Quad> = decor.quads.iter().filter(|q| !q.unlit).collect();
            assert_eq!(wires.len(), 15 * cfg.fairy.wire_segments as usize, "width {width}");
            for wire in wires.chunks(cfg.fairy.wire_segments as usize) {
                assert!(close(wire[0].c[0].z, near), "width {width}: the wire starts at {}", wire[0].c[0].z);
                assert!(close(wire[wire.len() - 1].c[1].z, far), "width {width}: the wire ends at {}", wire[wire.len() - 1].c[1].z);
                for pair in wire.windows(2) {
                    assert!(close(pair[0].c[1].z, pair[1].c[0].z) && close(pair[0].c[1].y, pair[1].c[0].y), "a gap in the wire");
                }
            }
            // bulbs all the way across: one within a bulb spacing of each wall, none in the walls, none in headroom
            let bulbs: Vec<&Quad> = decor.quads.iter().filter(|q| q.unlit).collect();
            let per_string = bulbs.len() / 15 / 2;
            assert!(per_string >= 2 && bulbs.len() == per_string * 30, "width {width}");
            let zs = || bulbs.iter().flat_map(|q| q.c.iter().map(|p| p.z));
            let (lo, hi) = (zs().fold(f32::INFINITY, f32::min), zs().fold(f32::NEG_INFINITY, f32::max));
            assert!(lo >= near && lo < near + cfg.fairy.bulb_spacing_m, "width {width}: the first bulb is at {lo}");
            assert!(hi <= far && hi > far - cfg.fairy.bulb_spacing_m, "width {width}: the last bulb is at {hi}");
            for q in &decor.quads {
                for p in q.c {
                    assert!(p.y < CEILING && p.y > 2.2, "width {width}: {p} is in the ceiling or in the runner's way");
                }
            }
        }
    }

    #[test]
    fn a_string_hangs_across_a_hall_that_runs_the_other_way_too() {
        let decor = lights_of(&carved(&[(20, 0, 3, SIZE)]), &ask(1.0, 0.0));
        assert_eq!(decor.strings.len(), 15);
        for s in &decor.strings {
            assert!(close(s.from.x, 20.0) && close(s.to.x, 23.0) && close(s.from.z, s.cell.1 as f32 + 0.5), "{s:?}");
        }
    }

    #[test]
    fn a_string_sags_in_the_middle_and_more_in_a_wider_hall() {
        let cfg = shipped();
        let lowest = |width: i32| {
            let decor = lights_of(&hall_along_x(width), &ask(1.0, 0.0));
            decor.quads.iter().filter(|q| !q.unlit).flat_map(|q| q.c.iter().map(|p| p.y)).fold(f32::INFINITY, f32::min)
        };
        let ends = CEILING - cfg.fairy.drop_m;
        assert!(lowest(2) < ends - 0.1 && lowest(4) < lowest(2));
        assert!(lowest(6) >= ends - cfg.fairy.sag_max_m - cfg.fairy.wire_thickness_m);
    }

    #[test]
    fn placement_is_deterministic_and_changes_with_the_seed() {
        let open = hall_along_x(3);
        let mut strung = 0;
        let mut lamps = 0;
        for s in 0..8 {
            let a = Ask { seed: seed("run:L0", (s, -s)), ..ask(0.3, 0.3) };
            let once = lights_of(&open, &a);
            assert_eq!(once, lights_of(&open, &a), "the same seed gives the same lights");
            // a stretch is strung whole or not at all: its two strings come together
            let mut by_stretch = [0; 8];
            for st in &once.strings {
                by_stretch[(st.cell.0 / 6) as usize] += 1;
            }
            assert!(by_stretch.iter().all(|n| *n == 0 || *n == 2 || (*n == 1 && by_stretch[7] == 1)), "{by_stretch:?}");
            strung += once.strings.len();
            lamps += once.sconces.len();
        }
        assert!(strung > 0 && strung < 8 * 15, "some stretches are strung and some are not ({strung})");
        assert!(lamps > 0 && lamps < 8 * 14, "some wall faces carry a lamp and some do not ({lamps})");
        assert_ne!(lights_of(&open, &Ask { seed: seed("run:L0", (0, 0)), ..ask(0.3, 0.3) }), lights_of(&open, &Ask { seed: seed("other:L0", (0, 0)), ..ask(0.3, 0.3) }));
        assert_ne!(seed("run:L0", (0, 0)), seed("run:L0", (1, 0)));
    }

    #[test]
    fn nothing_hangs_in_a_sewer_window() {
        for width in 1..=4 {
            let decor = lights_of(&hall_along_x(width), &Ask { sewer: true, ..ask(1.0, 1.0) });
            assert_eq!(decor, Decor::default(), "width {width}");
        }
    }

    #[test]
    fn nothing_hangs_in_a_room_or_where_the_chance_is_zero() {
        let open = hall_along_x(3);
        let everywhere = |_: i32, _: i32| true;
        assert_eq!(lights_of(&open, &Ask { room: &everywhere, ..ask(1.0, 1.0) }), Decor::default());
        assert_eq!(lights_of(&open, &ask(0.0, 0.0)), Decor::default());
        // a room over one cell of the hall's width takes the strings that would cross it
        let far_row = |_: i32, z: i32| z == 12;
        let decor = lights_of(&open, &Ask { room: &far_row, ..ask(1.0, 0.0) });
        assert!(decor.strings.is_empty());
    }

    #[test]
    fn a_hall_too_wide_or_without_its_far_wall_in_the_chunk_gets_no_string() {
        let max = shipped().fairy.max_width_cells;
        assert_eq!(lights_of(&hall_along_x(max), &ask(1.0, 0.0)).strings.len(), 15);
        assert!(lights_of(&hall_along_x(max + 1), &ask(1.0, 0.0)).strings.is_empty());
        // the hall's far side is the chunk's edge: the wall is another chunk's
        assert!(lights_of(&carved(&[(0, 42, SIZE, 3)]), &ask(1.0, 0.0)).strings.is_empty());
        // a one-cell nook off nothing is no hall
        assert!(lights_of(&carved(&[(4, 4, 1, 1)]), &ask(1.0, 1.0)).strings.is_empty());
    }

    #[test]
    fn the_triangle_count_is_bounded() {
        for width in 1..=6 {
            let decor = lights_of(&hall_along_x(width), &ask(1.0, 0.0));
            let per_string = decor.triangles() / decor.strings.len();
            assert_eq!(decor.triangles() % decor.strings.len(), 0);
            // a wire of four quads and two quads a bulb: at most 72 triangles a string in the widest hall
            assert!(per_string <= if width <= 4 { 52 } else { 72 }, "width {width}: {per_string} triangles a string");
            // a stretch of twelve cells holds four strings
            assert!(per_string * 4 <= 288, "width {width}");
        }
        // a wall lamp is two boxes
        let decor = lights_of(&hall_along_x(3), &ask(0.0, 1.0));
        assert!(!decor.sconces.is_empty());
        assert_eq!(decor.triangles(), decor.sconces.len() * 24);
    }

    #[test]
    fn every_second_string_and_every_wall_lamp_gives_one_light() {
        let decor = lights_of(&hall_along_x(3), &ask(1.0, 0.0));
        // strings on cells 1, 4, 7, ..: the light is on every second one
        assert_eq!(decor.lights.len(), 8);
        for l in &decor.lights {
            assert!(l.fairy && close(l.pos.z, 11.5) && l.pos.y < CEILING - 0.3 && l.pos.y > 2.0, "{l:?}");
        }
        let decor = lights_of(&hall_along_x(3), &ask(0.0, 1.0));
        assert_eq!(decor.lights.len(), decor.sconces.len());
        assert!(decor.lights.iter().all(|l| !l.fairy));
    }

    #[test]
    fn wall_lamps_sit_on_hall_wall_faces_between_the_pilasters_and_not_over_a_prop() {
        let cfg = shipped();
        let open = hall_along_x(3);
        let decor = lights_of(&open, &ask(0.0, 1.0));
        // cells 6, 12, .. 42 of both walls (not cell 0: the run must carry on a cell either side)
        assert_eq!(decor.sconces.len(), 2 * 7);
        for (i, at) in decor.sconces.iter().enumerate() {
            assert!(close(at.z, 10.0) || close(at.z, 13.0), "on a wall face: {at}");
            assert!(close(at.y, cfg.sconce.height_m), "{at}");
            assert_eq!(((at.x - 0.5).round() as i32) % 6, 0, "{at}");
            // the plate lies against the wall and both boxes stand out into the hall, under 20 cm
            for b in &decor.boxes[i * 2..i * 2 + 2] {
                let (lo, hi) = (b.centre.z - b.size.z * 0.5, b.centre.z + b.size.z * 0.5);
                assert!(lo >= 10.0 - 1.0e-4 && hi <= 13.0 + 1.0e-4, "{b:?}");
                assert!((lo - 10.0).min(13.0 - hi) < 0.05 && hi - lo < 0.2, "{b:?}");
            }
            let light = decor.lights[i].pos;
            assert!(light.z > 10.0 && light.z < 13.0 && close(light.y, at.y), "{light}");
        }
        // a prop in the cell under the lamp takes it
        let near_row = |_: i32, z: i32| z == 10;
        let decor = lights_of(&open, &Ask { taken: &near_row, ..ask(0.0, 1.0) });
        assert_eq!(decor.sconces.len(), 7);
        assert!(decor.sconces.iter().all(|at| close(at.z, 13.0)));
        // a hall along z: the boxes turn with the wall
        let decor = lights_of(&carved(&[(20, 0, 3, SIZE)]), &ask(0.0, 1.0));
        assert_eq!(decor.sconces.len(), 14);
        assert!(decor.boxes.iter().all(|b| b.size.x < b.size.z));
    }

    #[test]
    fn a_cell_owns_its_lights_once_so_parts_of_a_chunk_never_draw_one_twice() {
        let open = hall_along_x(3);
        let whole = lights_of(&open, &ask(1.0, 1.0));
        let is_open = |x: i32, z: i32| open[z as usize][x as usize];
        let view = ChunkView { size: SIZE, sewer: false, open: &is_open, brick: &|_, _| true, room: &nowhere, ramp: &nowhere, height: &|_, _| CEILING };
        let site = Site { view: &view, seed: 7, fairy_chance: &|_, _| 1.0, sconce_chance: &|_, _| 1.0, taken: &nowhere };
        let cfg = shipped();
        let mut parts = Decor::default();
        // the chunk in two halves, cell by cell, in another order
        for z in (0..SIZE).rev() {
            for x in (0..SIZE).filter(|x| x % 2 == 0).chain((0..SIZE).filter(|x| x % 2 == 1)) {
                cell_lights(&site, &cfg, x, z, &mut parts);
            }
        }
        assert_eq!(parts.strings.len(), whole.strings.len());
        assert_eq!(parts.sconces.len(), whole.sconces.len());
        assert_eq!(parts.triangles(), whole.triangles());
        // a wall cell and a cell outside the chunk own nothing
        let mut none = Decor::default();
        cell_lights(&site, &cfg, 4, 9, &mut none);
        cell_lights(&site, &cfg, 4, -1, &mut none);
        assert_eq!(none, Decor::default());
    }
}
