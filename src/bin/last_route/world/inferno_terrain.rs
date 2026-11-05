//! The noisy floor and ceiling of the Inferno caverns (level -3). Owner decision, overriding Godot parity (where the
//! cavern floor is flat): the caves get smooth noisy floor and ceiling heights.
//!
//! Both surfaces are a pure function of the world seed text and the world grid corner, so they join across chunk
//! seams and across windows without a seam. A corner's height is smooth value noise sampled at its integer world
//! position; between corners the floor is the very surface the renderer draws (each cell is two triangles split along
//! the (x, z) to (x + 1, z + 1) diagonal, the way `Builder::quad` cuts a quad), so a body that stands at `floor_at`
//! stands on the visible ground.
//!
//! The rules (all data, `terrain` block of data/inferno.json):
//! * the floor never rises or dips more than `floor.amplitude`, and two neighbouring corners never differ by more than
//!   `max_step_per_cell`: the noise and the stairwell blend are Lipschitz with a known constant, and the loader refuses
//!   numbers whose constant is higher (`TerrainRules::parse`), so the slope is limited by construction;
//! * the ceiling hangs between `floor + min_headroom` and the slab line (`STOREY - slab`), never lower or higher;
//! * around a stairwell (the flight that comes up from this level) the terrain eases to the flat floor (0) and the full
//!   ceiling (the slab line) over `pad_blend` cells, so the flight foot, the landing and the shaft keep their flat rules.
//!
//! Lava cells are closed (`InfernoChunk::open_cells`): they are never walkable, so the floor under them is only ever
//! seen as the decoration height; the terrain is defined everywhere.

use super::flight::well_footprint;
use super::flight_links::bay_flight;
use super::infinite::CHUNK;
use super::rng::Rng;
use super::vertical::STOREY;
use serde_json::Value;
use std::sync::Arc;

/// One noise layer of the terrain (data).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// The largest deviation of the layer, in metres.
    pub amplitude: f32,
    /// Cells between the lattice points of the first octave.
    pub scale: f32,
    /// How many octaves are summed; each halves the scale and keeps `persistence` of the amplitude.
    pub octaves: u32,
    pub persistence: f32,
}

impl Field {
    /// Amplitude of octave `k` (the octaves sum to `amplitude`).
    fn octave_amplitude(&self, k: u32) -> f64 {
        let norm: f64 = (0..self.octaves).map(|i| (self.persistence as f64).powi(i as i32)).sum();
        self.amplitude as f64 * (self.persistence as f64).powi(k as i32) / norm
    }

    /// Cells between lattice points of octave `k`.
    fn octave_scale(&self, k: u32) -> f64 {
        self.scale as f64 / 2f64.powi(k as i32)
    }

    /// The largest change of the layer over one cell along an axis: smoothstep rises at most 1.5 times the mean slope,
    /// and lattice values differ by at most 2 (noise is in [-1, 1]).
    pub fn max_slope(&self) -> f64 {
        (0..self.octaves).map(|k| self.octave_amplitude(k) * 3.0 / self.octave_scale(k)).sum()
    }

    fn parse(v: &Value, what: &str) -> Result<Field, String> {
        let num = |k: &str| -> Result<f32, String> {
            v[k].as_f64()
                .map(|x| x as f32)
                .filter(|x| x.is_finite())
                .ok_or_else(|| format!("inferno.json terrain.{what}.{k} must be a number"))
        };
        let f = Field {
            amplitude: num("amplitude")?,
            scale: num("scale")?,
            octaves: num("octaves")? as u32,
            persistence: num("persistence")?,
        };
        if f.amplitude < 0.0 {
            return Err(format!("inferno.json terrain.{what}.amplitude must be zero or more"));
        }
        if f.scale < 1.0 {
            return Err(format!("inferno.json terrain.{what}.scale must be at least 1 cell"));
        }
        if !(1..=6).contains(&f.octaves) {
            return Err(format!("inferno.json terrain.{what}.octaves must be 1 to 6"));
        }
        if !(0.0..=1.0).contains(&f.persistence) {
            return Err(format!("inferno.json terrain.{what}.persistence must be 0 to 1"));
        }
        Ok(f)
    }
}

/// The least headroom the data may ask for: a standing body with its head and a margin.
pub const MIN_HEADROOM_FLOOR: f32 = 2.4;

/// The terrain tunables (data/inferno.json, `terrain`).
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainRules {
    pub floor: Field,
    pub ceiling: Field,
    pub min_headroom: f32,
    pub pad_blend: f32,
    pub max_step_per_cell: f32,
    pub ground_snap: f32,
}

impl TerrainRules {
    /// The ceiling line: the underside of the slab between this storey and the one above, less the ceiling inset, so
    /// no cavern ceiling shares a plane with the slab (the same line as `render::seams::ceiling_y(STOREY)`).
    pub fn slab_line() -> f32 {
        crate::render::seams::ceiling_y(STOREY)
    }

    /// The steepest floor change over one cell the noise and the stairwell blend can make.
    pub fn floor_slope_bound(&self) -> f32 {
        (self.floor.max_slope() + 1.5 * self.floor.amplitude as f64 / self.pad_blend as f64) as f32
    }

    pub fn parse(v: &Value) -> Result<TerrainRules, String> {
        let num = |k: &str| -> Result<f32, String> {
            v[k].as_f64()
                .map(|x| x as f32)
                .filter(|x| x.is_finite() && *x >= 0.0)
                .ok_or_else(|| format!("inferno.json terrain.{k} must be a number of zero or more"))
        };
        let (floor, ceiling) = (Field::parse(&v["floor"], "floor")?, Field::parse(&v["ceiling"], "ceiling")?);
        let min_headroom = ceiling_num(&v["ceiling"], "min_headroom")?;
        let r = TerrainRules {
            floor,
            ceiling,
            min_headroom,
            pad_blend: num("pad_blend")?,
            max_step_per_cell: num("max_step_per_cell")?,
            ground_snap: num("ground_snap")?,
        };
        if r.pad_blend < 1.0 {
            return Err("inferno.json terrain.pad_blend must be at least 1 cell".to_string());
        }
        if r.min_headroom < MIN_HEADROOM_FLOOR {
            return Err(format!("inferno.json terrain.ceiling.min_headroom must be at least {MIN_HEADROOM_FLOOR} m"));
        }
        // the lowest ceiling the clamp can ask for must still fit under the slab line
        if r.floor.amplitude + r.min_headroom > Self::slab_line() {
            return Err("inferno.json terrain: floor.amplitude + ceiling.min_headroom does not fit under the slab line".to_string());
        }
        if r.max_step_per_cell > crate::game::level_stack::STEP_UP {
            return Err("inferno.json terrain.max_step_per_cell is above the step-up of a body".to_string());
        }
        if r.floor_slope_bound() > r.max_step_per_cell {
            return Err(format!(
                "inferno.json terrain: the floor can change {:.3} m over a cell, above max_step_per_cell {}",
                r.floor_slope_bound(),
                r.max_step_per_cell
            ));
        }
        Ok(r)
    }

    /// The embedded rules (a bad embedded file stops the game at first use with the reason).
    pub fn builtin() -> &'static TerrainRules {
        &super::inferno::InfernoRules::builtin().terrain
    }
}

fn ceiling_num(v: &Value, k: &str) -> Result<f32, String> {
    v[k].as_f64().map(|x| x as f32).filter(|x| x.is_finite()).ok_or_else(|| format!("inferno.json terrain.ceiling.{k} must be a number"))
}

// ---------------------------------------------------------------------------------------------------------------------
// Noise
// ---------------------------------------------------------------------------------------------------------------------

/// A lattice value in [-1, 1]: an integer hash of the seed key, the layer, the octave and the lattice point.
fn lattice(key: u32, layer: u32, octave: u32, ix: i64, iy: i64) -> f64 {
    let mut h = key ^ layer.wrapping_mul(0x9e37_79b9) ^ octave.wrapping_mul(0x85eb_ca6b);
    h ^= (ix as i32 as u32).wrapping_mul(0xc2b2_ae35);
    h = h.rotate_left(13).wrapping_mul(5).wrapping_add(0xe654_6b64);
    h ^= (iy as i32 as u32).wrapping_mul(0x27d4_eb2f);
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h as f64 / 4294967295.0 * 2.0 - 1.0
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Smooth value noise in [-1, 1] at world point (x, y), lattice spacing `scale` cells.
fn value_noise(key: u32, layer: u32, octave: u32, scale: f64, x: f64, y: f64) -> f64 {
    let (u, v) = (x / scale, y / scale);
    let (ix, iy) = (u.floor(), v.floor());
    let (fx, fy) = (smooth(u - ix), smooth(v - iy));
    let (ix, iy) = (ix as i64, iy as i64);
    let a = lattice(key, layer, octave, ix, iy);
    let b = lattice(key, layer, octave, ix + 1, iy);
    let c = lattice(key, layer, octave, ix, iy + 1);
    let d = lattice(key, layer, octave, ix + 1, iy + 1);
    (a + (b - a) * fx) + ((c + (d - c) * fx) - (a + (b - a) * fx)) * fy
}

/// Layered noise of `field` at (x, y): in [-amplitude, amplitude].
fn fractal(field: &Field, key: u32, layer: u32, x: f64, y: f64) -> f64 {
    (0..field.octaves)
        .map(|k| field.octave_amplitude(k) * value_noise(key, layer, k, field.octave_scale(k), x, y))
        .sum()
}

const LAYER_FLOOR: u32 = 1;
const LAYER_CEILING: u32 = 2;

// ---------------------------------------------------------------------------------------------------------------------
// Stairwell pads
// ---------------------------------------------------------------------------------------------------------------------

/// A rectangle of world cells, `x0..x1` by `y0..y1` (end exclusive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pad {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Pad {
    /// Distance in cells from the point (x, y) to the rectangle; 0 inside.
    fn distance(&self, x: f64, y: f64) -> f64 {
        let dx = (self.x0 as f64 - x).max(x - self.x1 as f64).max(0.0);
        let dy = (self.y0 as f64 - y).max(y - self.y1 as f64).max(0.0);
        dx.hypot(dy)
    }
}

/// The pad of the flight that comes up from the Inferno in chunk column (cx, cy): the box around its foot, its well and
/// the clear cells beyond them, in world cells. A pure function of the world text and the chunk.
pub fn pad_of_chunk(world: &str, cx: i32, cy: i32) -> Option<Pad> {
    let flight = bay_flight(world, cx, cy, super::infinite::INFERNO_LEVEL + 1)?;
    let fp = well_footprint(&flight);
    let cells = fp.lower_open.iter().chain(&fp.lower_clear).chain(std::iter::once(&fp.foot)).chain(std::iter::once(&fp.landing));
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for c in cells {
        x0 = x0.min(c.x);
        y0 = y0.min(c.y);
        x1 = x1.max(c.x + 1);
        y1 = y1.max(c.y + 1);
    }
    (x0 <= x1).then(|| Pad { x0: x0 + cx * CHUNK, y0: y0 + cy * CHUNK, x1: x1 + cx * CHUNK, y1: y1 + cy * CHUNK })
}

// ---------------------------------------------------------------------------------------------------------------------
// The terrain of a window
// ---------------------------------------------------------------------------------------------------------------------

/// The Inferno terrain as one facility sees it: its coordinates are the facility's grid cells, `origin` is the world
/// cell of the grid's (0, 0). Valid for every point inside the chunks it was made for (plus the margin of one chunk of
/// stairwell pads it looked up).
#[derive(Clone, Debug)]
pub struct Terrain {
    key: u32,
    origin: (i32, i32),
    pads: Arc<Vec<Pad>>,
    rules: &'static TerrainRules,
}

impl Terrain {
    /// The terrain of world `world` for a grid whose cell (0, 0) is world cell `origin` and that spans `chunks` chunks
    /// each way.
    pub fn new(world: &str, origin: (i32, i32), chunks: i32) -> Terrain {
        let (cx0, cy0) = (origin.0.div_euclid(CHUNK), origin.1.div_euclid(CHUNK));
        let mut pads = vec![];
        for cy in cy0 - 1..cy0 + chunks + 1 {
            for cx in cx0 - 1..cx0 + chunks + 1 {
                pads.extend(pad_of_chunk(world, cx, cy));
            }
        }
        Terrain {
            key: Rng::new(&format!("{world}:INFERNO-TERRAIN")).next_u32(),
            origin,
            pads: Arc::new(pads),
            rules: TerrainRules::builtin(),
        }
    }

    pub fn rules(&self) -> &'static TerrainRules {
        self.rules
    }

    /// 0 inside a stairwell pad, rising smoothly to 1 `pad_blend` cells away.
    fn mask(&self, wx: f64, wy: f64) -> f64 {
        let d = self.pads.iter().map(|p| p.distance(wx, wy)).fold(f64::INFINITY, f64::min);
        if d.is_infinite() {
            return 1.0;
        }
        smooth((d / self.rules.pad_blend as f64).min(1.0))
    }

    /// The floor height at the integer grid corner (`i`, `j`), metres above the level's floor line.
    pub fn floor_corner(&self, i: i32, j: i32) -> f32 {
        let (wx, wy) = ((i + self.origin.0) as f64, (j + self.origin.1) as f64);
        (fractal(&self.rules.floor, self.key, LAYER_FLOOR, wx, wy) * self.mask(wx, wy)) as f32
    }

    /// The ceiling height at the integer grid corner (`i`, `j`): between the floor plus the minimum headroom and the slab
    /// line.
    pub fn ceiling_corner(&self, i: i32, j: i32) -> f32 {
        let (wx, wy) = ((i + self.origin.0) as f64, (j + self.origin.1) as f64);
        let top = TerrainRules::slab_line() as f64;
        let c = &self.rules.ceiling;
        // noise in [-a, a] mapped to a sag in [0, a] below the slab line, eased to the line over a pad
        let sag = (fractal(c, self.key, LAYER_CEILING, wx, wy) + c.amplitude as f64) * 0.5 * self.mask(wx, wy);
        let floor = self.floor_corner(i, j) as f64;
        (top - sag).max(floor + self.rules.min_headroom as f64).min(top) as f32
    }

    /// The floor height under the point (x, z) of the facility grid: linear on each of the two triangles of its cell,
    /// exactly the surface the renderer draws.
    pub fn floor_at(&self, x: f32, z: f32) -> f32 {
        self.surface_at(x, z, |i, j| self.floor_corner(i, j))
    }

    /// The ceiling height over the point (x, z), cut like the floor.
    pub fn ceiling_at(&self, x: f32, z: f32) -> f32 {
        self.surface_at(x, z, |i, j| self.ceiling_corner(i, j))
    }

    fn surface_at(&self, x: f32, z: f32, corner: impl Fn(i32, i32) -> f32) -> f32 {
        let (i, j) = (x.floor() as i32, z.floor() as i32);
        let (u, v) = (x - i as f32, z - j as f32);
        let (h00, h10, h11, h01) = (corner(i, j), corner(i + 1, j), corner(i + 1, j + 1), corner(i, j + 1));
        if u >= v {
            h00 + u * (h10 - h00) + v * (h11 - h10)
        } else {
            h00 + u * (h11 - h01) + v * (h01 - h00)
        }
    }

    /// The clear height at (x, z): ceiling minus floor.
    pub fn headroom_at(&self, x: f32, z: f32) -> f32 {
        self.ceiling_at(x, z) - self.floor_at(x, z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::infinite::{ChunkKey, INFERNO_LEVEL};
    use crate::world::inferno::InfernoChunk;

    const WORLDS: [&str; 3] = ["1426541270", "4157433391", "2899547009"];

    fn terrain(world: &str, chunks: i32) -> Terrain {
        Terrain::new(world, (0, 0), chunks)
    }

    #[test]
    fn the_shipped_terrain_rules_pass_their_own_checks() {
        let r = TerrainRules::builtin();
        assert!(r.min_headroom >= MIN_HEADROOM_FLOOR);
        assert!(r.floor_slope_bound() <= r.max_step_per_cell);
        assert!(r.max_step_per_cell <= crate::game::level_stack::STEP_UP);
        assert!(r.floor.amplitude > 0.0 && r.ceiling.amplitude > 0.0, "the caves must actually be noisy");
    }

    #[test]
    fn bad_terrain_data_is_refused() {
        let text = include_str!("../data/inferno.json");
        let good: Value = serde_json::from_str(text).unwrap();
        assert!(TerrainRules::parse(&good["terrain"]).is_ok());
        let mut shallow = good["terrain"].clone();
        shallow["ceiling"]["min_headroom"] = serde_json::json!(2.0);
        assert!(TerrainRules::parse(&shallow).is_err(), "less than 2.4 m of headroom must be refused");
        let cases: [(&str, Value); 7] = [
            ("max_step_per_cell", serde_json::json!(0.9)),
            ("max_step_per_cell", serde_json::json!(0.05)),
            ("pad_blend", serde_json::json!(0.2)),
            ("ground_snap", serde_json::json!(-1.0)),
            ("floor", serde_json::json!({ "amplitude": 0.5, "scale": 16.0, "octaves": 0, "persistence": 0.35 })),
            ("floor", serde_json::json!({ "amplitude": 0.5, "scale": 0.5, "octaves": 3, "persistence": 0.35 })),
            ("ceiling", serde_json::json!({ "amplitude": 3.0, "scale": 12.0, "octaves": 2, "persistence": 0.5, "min_headroom": 4.5 })),
        ];
        for (key, value) in cases {
            let mut t = good["terrain"].clone();
            t[key] = value;
            assert!(TerrainRules::parse(&t).is_err(), "{key} must be refused");
        }
        let mut missing = good["terrain"].clone();
        missing.as_object_mut().unwrap().remove("floor");
        assert!(TerrainRules::parse(&missing).is_err());
    }

    #[test]
    fn the_terrain_is_deterministic_and_depends_on_the_world() {
        let (a, b) = (terrain(WORLDS[0], 3), terrain(WORLDS[0], 3));
        let other = terrain(WORLDS[1], 3);
        let mut differs = false;
        for (i, j) in [(3, 4), (40, 77), (101, 12), (60, 60), (7, 120)] {
            assert_eq!(a.floor_corner(i, j), b.floor_corner(i, j));
            assert_eq!(a.ceiling_corner(i, j), b.ceiling_corner(i, j));
            differs |= a.floor_corner(i, j) != other.floor_corner(i, j);
        }
        assert!(differs, "another world must have another floor");
    }

    #[test]
    fn the_floor_is_noisy_and_inside_its_amplitude() {
        for world in WORLDS {
            let t = terrain(world, 3);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for j in 0..3 * CHUNK {
                for i in 0..3 * CHUNK {
                    let h = t.floor_corner(i, j);
                    assert!(h.abs() <= t.rules().floor.amplitude + 1e-4, "{world}: corner {i},{j} is {h}");
                    lo = lo.min(h);
                    hi = hi.max(h);
                }
            }
            assert!(hi - lo > 0.3, "{world}: the floor varies by only {} m", hi - lo);
        }
    }

    #[test]
    fn two_windows_agree_on_every_shared_corner_so_chunks_join_without_a_seam() {
        for world in WORLDS {
            // window A at the origin, window B one chunk to the east: they share two columns of chunks
            let a = Terrain::new(world, (0, 0), 3);
            let b = Terrain::new(world, (CHUNK, 0), 3);
            for j in 0..3 * CHUNK {
                for i in 0..2 * CHUNK {
                    // the world corner (i, j) is (i, j) in A and (i - CHUNK, j) in B
                    let (wi, wj) = (i + CHUNK, j);
                    assert_eq!(a.floor_corner(wi, wj), b.floor_corner(wi - CHUNK, wj), "{world}: floor ({wi},{wj})");
                    assert_eq!(a.ceiling_corner(wi, wj), b.ceiling_corner(wi - CHUNK, wj), "{world}: ceiling ({wi},{wj})");
                }
            }
        }
    }

    #[test]
    fn the_surface_is_continuous_across_chunk_borders() {
        for world in WORLDS {
            let t = terrain(world, 3);
            for border in [CHUNK, 2 * CHUNK] {
                for k in 0..3 * CHUNK {
                    for (x, z) in [(border as f32, k as f32 + 0.37), (k as f32 + 0.61, border as f32)] {
                        let e = 1e-3;
                        let (before, after) = if x == border as f32 {
                            (t.floor_at(x - e, z), t.floor_at(x + e, z))
                        } else {
                            (t.floor_at(x, z - e), t.floor_at(x, z + e))
                        };
                        assert!((before - after).abs() < 5e-3, "{world}: a jump of {} at ({x},{z})", before - after);
                    }
                }
            }
        }
    }

    #[test]
    fn neighbouring_corners_never_differ_by_more_than_the_data_step() {
        for world in WORLDS {
            let t = terrain(world, 3);
            let max = t.rules().max_step_per_cell;
            let mut worst = 0.0_f32;
            for j in 0..3 * CHUNK {
                for i in 0..3 * CHUNK {
                    let h = t.floor_corner(i, j);
                    for (di, dj) in [(1, 0), (0, 1), (1, 1), (1, -1)] {
                        let step = (t.floor_corner(i + di, j + dj) - h).abs();
                        worst = worst.max(step);
                        assert!(step <= max * (di.abs() + dj.abs()) as f32 + 1e-4, "{world}: step {step} at {i},{j} +({di},{dj})");
                    }
                }
            }
            assert!(worst <= max, "{world}: the steepest axis step is {worst}");
            assert!(worst > 0.02, "{world}: the floor is flat ({worst})");
        }
    }

    #[test]
    fn walking_in_a_straight_line_never_steps_up_more_than_the_step_up() {
        use crate::game::level_stack::STEP_UP;
        for world in WORLDS {
            let t = terrain(world, 3);
            // 0.05 m strides along rows, columns and diagonals
            for line in 0..40 {
                let base = 5.3 + line as f32 * 3.1;
                for (dx, dz) in [(1.0, 0.0), (0.0, 1.0), (0.7071, 0.7071)] {
                    let (mut x, mut z) = if dx == 1.0 { (2.0, base) } else if dz == 1.0 { (base, 2.0) } else { (base * 0.5, 2.0) };
                    let mut last = t.floor_at(x, z);
                    for _ in 0..2400 {
                        x += dx * 0.05;
                        z += dz * 0.05;
                        let h = t.floor_at(x, z);
                        assert!((h - last).abs() <= 0.05 * t.rules().max_step_per_cell * 1.5, "{world}: a {} m jump in 5 cm at {x},{z}", h - last);
                        assert!((h - last).abs() < STEP_UP);
                        last = h;
                    }
                }
            }
        }
    }

    #[test]
    fn the_ceiling_keeps_the_headroom_and_stays_under_the_slab_line() {
        for world in WORLDS {
            let t = terrain(world, 3);
            let top = TerrainRules::slab_line();
            assert!(top < STOREY, "the slab has a thickness");
            let (mut sags, mut lowest) = (false, f32::MAX);
            for j in 0..3 * CHUNK {
                for i in 0..3 * CHUNK {
                    let (f, c) = (t.floor_corner(i, j), t.ceiling_corner(i, j));
                    assert!(c <= top + 1e-4, "{world}: ceiling {c} above the slab line {top} at {i},{j}");
                    assert!(c - f >= t.rules().min_headroom - 1e-4, "{world}: headroom {} at {i},{j}", c - f);
                    sags |= c < top - 0.2;
                    lowest = lowest.min(c - f);
                }
            }
            assert!(sags, "{world}: the ceiling never dips");
            assert!(lowest >= MIN_HEADROOM_FLOOR, "{world}: the lowest clear height is {lowest}");
            // between corners too (both triangles are convex combinations of corners)
            for k in 0..2000 {
                let (x, z) = (3.0 + k as f32 * 0.0617, 5.0 + (k * 7 % 130) as f32 * 0.91);
                assert!(t.headroom_at(x, z) >= MIN_HEADROOM_FLOOR - 1e-3);
                assert!(t.ceiling_at(x, z) <= top + 1e-4);
            }
        }
    }

    #[test]
    fn the_clamp_holds_the_headroom_when_the_floor_is_pushed_up() {
        // a rule set with a tall floor and a deep sag: the clamp, not luck, keeps the headroom
        let mut v: Value = serde_json::from_str(include_str!("../data/inferno.json")).unwrap();
        v["terrain"]["floor"]["amplitude"] = serde_json::json!(1.0);
        v["terrain"]["floor"]["scale"] = serde_json::json!(40.0);
        v["terrain"]["ceiling"]["amplitude"] = serde_json::json!(2.0);
        v["terrain"]["max_step_per_cell"] = serde_json::json!(0.4);
        let rules: &'static TerrainRules = Box::leak(Box::new(TerrainRules::parse(&v["terrain"]).unwrap()));
        let mut t = terrain(WORLDS[0], 3);
        t.rules = rules;
        let mut clamped = 0;
        for j in 0..3 * CHUNK {
            for i in 0..3 * CHUNK {
                let (f, c) = (t.floor_corner(i, j), t.ceiling_corner(i, j));
                assert!(c - f >= rules.min_headroom - 1e-4 && c <= TerrainRules::slab_line() + 1e-4);
                clamped += (c - f < rules.min_headroom + 1e-3) as i32;
            }
        }
        assert!(clamped > 0, "the test must reach the clamp");
    }

    #[test]
    fn a_stairwell_has_a_flat_floor_and_a_full_ceiling() {
        let mut found = 0;
        for world in WORLDS {
            for cy in -2..=2 {
                for cx in -2..=2 {
                    let Some(pad) = pad_of_chunk(world, cx, cy) else { continue };
                    found += 1;
                    let t = Terrain::new(world, (cx * CHUNK, cy * CHUNK), 1);
                    for wy in pad.y0..pad.y1 {
                        for wx in pad.x0..pad.x1 {
                            let (i, j) = (wx - cx * CHUNK, wy - cy * CHUNK);
                            assert_eq!(t.floor_corner(i, j), 0.0, "{world}: pad floor at {wx},{wy}");
                            assert!((t.ceiling_corner(i, j) - TerrainRules::slab_line()).abs() < 1e-4);
                        }
                    }
                }
            }
        }
        assert!(found > 3, "the sample has stairwells ({found})");
    }

    #[test]
    fn the_foot_of_every_flight_up_from_the_inferno_stands_on_a_flat_floor() {
        use crate::world::catalog::Catalog;
        use crate::content::loot::LootTables;
        let cat = Catalog::load();
        let loot = LootTables::default();
        for world in WORLDS {
            let (f, _) = crate::world::infinite::chunk(world, ChunkKey { cx: 0, cy: 0, level: INFERNO_LEVEL }, &cat, &loot);
            let t = Terrain::new(world, (0, 0), 1);
            let foot = f.stairs.iter().find(|s| s.dir > 0).expect("a flight up from the Inferno").cell;
            for (dx, dy) in [(0.5, 0.5), (0.0, 0.0), (1.0, 1.0), (0.5, 1.0)] {
                assert_eq!(t.floor_at(foot.x as f32 + dx, foot.y as f32 + dy), 0.0, "{world}: the foot {foot:?}");
            }
        }
    }

    #[test]
    fn walkable_cells_of_the_caves_lie_on_the_surface_and_lava_is_not_walkable() {
        for world in WORLDS {
            let k = ChunkKey { cx: 0, cy: 0, level: INFERNO_LEVEL };
            let chunk = InfernoChunk::carve(world, k);
            let t = terrain(world, 1);
            assert!(!chunk.open_cells().is_empty());
            for c in chunk.open_cells() {
                assert!(!chunk.is_lava(c), "{world}: lava cell {c:?} is open");
                let h = t.floor_at(c.x as f32 + 0.5, c.y as f32 + 0.5);
                assert!(h.abs() <= t.rules().floor.amplitude + 1e-4);
                assert!(t.headroom_at(c.x as f32 + 0.5, c.y as f32 + 0.5) >= MIN_HEADROOM_FLOOR);
            }
        }
    }
}
