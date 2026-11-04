//! One flight between two floors, defined once, in absolute y (docs/STAIRS_CONTINUOUS_DESIGN.md section 2.3).
//!
//! A level's floor is at `level * STOREY`. A [`Flight`] joins the floor of `upper_level` to the floor below it and gives
//! the walking height ([`flight_surface_y`]) and the cells both levels must open ([`well_footprint`]) from one record,
//! so the top and the bottom cannot disagree. All three shapes (switchback, straight, spiral) and the ladder and the
//! drop are defined here. Every number is in `data/flights.json` ([`flights`]); this file holds behaviour only.
//!
//! Nothing in the running game calls this yet (phase 0): only the tests do.
//!
//! Frame. The flight's local frame has `u` metres along `run` from the far edge of the landing cell (`u = 0` there;
//! cell `k` of the run covers `u` in `k-1..k`) and `v` metres across, towards the +x / +y side (the second lane is
//! always on that side, as in `facility::stair_well_cells`). The anchor cell is the landing A at `u` in `-1..0`.

use super::facility::Cell;
use super::vertical::STOREY;
use std::f32::consts::{PI, TAU};
use std::sync::OnceLock;

const FILE: &str = include_str!("../data/flights.json");

/// What joins the two floors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlightKind {
    /// A walked stair; the shape says how it is laid out.
    Stairs,
    /// A one-cell shaft climbed by hand. `anchor_cell` is the hatch; `run` points to the one clear step cell beside it.
    Ladder,
    /// A free fall (waterfall or hole). `anchor_cell` is the hole; `run` is the flow direction; the landing A is the cell upstream.
    Drop,
}

/// How a stair is laid out (KT decision D1: all three).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlightShape {
    /// Down one lane to a mid landing, back up the other lane (the 2x4 well).
    Switchback,
    /// One longer flight.
    Straight,
    /// Around a centre column.
    Spiral,
}

/// One flight. `upper_level` is the level whose floor the top reaches; the bottom is on `upper_level - 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flight {
    pub kind: FlightKind,
    pub shape: FlightShape,
    pub upper_level: i32,
    /// Stairs: the landing cell A on the upper level. Ladder: the hatch cell. Drop: the hole cell.
    pub anchor_cell: Cell,
    /// Unit step along the flight (see `FlightKind` for ladders and drops).
    pub run: (i32, i32),
    /// Spiral only: which way it turns (the mirror image when true).
    pub clockwise: bool,
}

impl Flight {
    pub fn new(kind: FlightKind, shape: FlightShape, upper_level: i32, anchor_cell: Cell, run: (i32, i32)) -> Flight {
        Flight { kind, shape, upper_level, anchor_cell, run, clockwise: false }
    }
    pub fn with_clockwise(mut self, clockwise: bool) -> Flight {
        self.clockwise = clockwise;
        self
    }
    /// Absolute y of the lower floor, `(upper_level - 1) * STOREY`.
    pub fn lower_y(&self) -> f32 {
        (self.upper_level - 1) as f32 * STOREY
    }
    /// Absolute y of the upper floor, `upper_level * STOREY`.
    pub fn upper_y(&self) -> f32 {
        self.upper_level as f32 * STOREY
    }
    /// The same flight seen from its foot: the cell B on `lower_level` (the level below the upper one).
    pub fn from_foot_with(
        rules: &FlightRules,
        kind: FlightKind,
        shape: FlightShape,
        lower_level: i32,
        foot: Cell,
        run: (i32, i32),
        clockwise: bool,
    ) -> Flight {
        let mut f = Flight { kind, shape, upper_level: lower_level + 1, anchor_cell: foot, run, clockwise };
        if kind == FlightKind::Stairs {
            let (k, j) = foot_offset(rules, &f);
            let s = side(run);
            f.anchor_cell = Cell::new(foot.x - run.0 * k - s.0 * j, foot.y - run.1 * k - s.1 * j);
        }
        f
    }
}

/// The cells a flight needs, from one source: both levels read the same lists.
#[derive(Clone, Debug, PartialEq)]
pub struct Footprint {
    /// Cells where the upper level has no floor (sorted).
    pub upper_hole: Vec<Cell>,
    /// Cells where the lower level has no ceiling: always the same cells as `upper_hole`.
    pub lower_open: Vec<Cell>,
    /// A: the open floor cell on the upper level where the walker steps on (a ladder's step cell, a drop's bank).
    pub landing: Cell,
    /// B: the open floor cell on the lower level where the walker steps off (a ladder's and a drop's hole cell).
    pub foot: Cell,
    /// A and `margin_cells` cells behind it, which must stay open floor on the upper level.
    pub upper_clear: Vec<Cell>,
    /// B and `margin_cells` cells beyond it, which must stay open floor on the lower level.
    pub lower_clear: Vec<Cell>,
    /// A ladder's one clear cell beside the hatch (D4); `None` for stairs and drops.
    pub step_cell: Option<Cell>,
    /// Slab thickness between the two floors (data).
    pub slab: f32,
}

// ---------------------------------------------------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------------------------------------------------

/// Relative weights of the three shapes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeWeights {
    pub switchback: f32,
    pub straight: f32,
    pub spiral: f32,
}

impl ShapeWeights {
    fn total(&self) -> f32 {
        self.switchback + self.straight + self.spiral
    }
}

/// Every flight tunable (data/flights.json).
#[derive(Clone, Debug, PartialEq)]
pub struct FlightRules {
    pub storey: f32,
    pub slab: f32,
    pub rise_total: f32,
    pub max_riser: f32,
    pub steps_per_cell: f32,
    pub margin_cells: i32,
    pub sb_length: i32,
    pub sb_lane_width: i32,
    pub sb_mid_rise: f32,
    pub sb_mid_len: f32,
    pub st_length: i32,
    pub st_width: i32,
    pub sp_radius: f32,
    pub sp_lane_width: i32,
    pub sp_turns: f32,
    pub weights: ShapeWeights,
    /// (level, weights) that replace `weights` on that level.
    pub region_weights: Vec<(i32, ShapeWeights)>,
}

type Json = serde_json::Value;

fn jget<'a>(v: &'a Json, path: &[&str]) -> Result<&'a Json, String> {
    let mut cur = v;
    for key in path {
        cur = cur.get(*key).ok_or_else(|| format!("flights.json: \"{}\" is missing", path.join(".")))?;
    }
    Ok(cur)
}

fn jnum(v: &Json, path: &[&str]) -> Result<f32, String> {
    jget(v, path)?
        .as_f64()
        .map(|n| n as f32)
        .ok_or_else(|| format!("flights.json: \"{}\" must be a number", path.join(".")))
}

fn jint(v: &Json, path: &[&str]) -> Result<i32, String> {
    let n = jnum(v, path)?;
    if n.fract() != 0.0 {
        return Err(format!("flights.json: \"{}\" must be a whole number, got {n}", path.join(".")));
    }
    Ok(n as i32)
}

fn jweights(v: &Json, label: &str) -> Result<ShapeWeights, String> {
    let get = |k: &str| -> Result<f32, String> {
        let n = v
            .get(k)
            .and_then(Json::as_f64)
            .ok_or_else(|| format!("flights.json: {label}.{k} must be a number"))? as f32;
        if n < 0.0 || !n.is_finite() {
            return Err(format!("flights.json: {label}.{k} must not be negative, got {n}"));
        }
        Ok(n)
    };
    let w = ShapeWeights { switchback: get("switchback")?, straight: get("straight")?, spiral: get("spiral")? };
    if w.total() <= 0.0 {
        return Err(format!("flights.json: {label} must have at least one weight above zero"));
    }
    Ok(w)
}

impl FlightRules {
    /// Parse and validate a flights table. A bad table is refused with a message naming the field.
    pub fn parse(text: &str) -> Result<FlightRules, String> {
        let v: Json = serde_json::from_str(text).map_err(|e| format!("flights.json: not valid JSON: {e}"))?;
        if v.get("schema").and_then(Json::as_str) != Some("lr.flights/1") {
            return Err("flights.json: schema must be \"lr.flights/1\"".to_string());
        }
        let mut region_weights = Vec::new();
        if let Some(map) = v.get("region_weights").and_then(Json::as_object) {
            for (key, w) in map {
                let level: i32 = key
                    .parse()
                    .map_err(|_| format!("flights.json: region_weights key \"{key}\" must be a level number"))?;
                region_weights.push((level, jweights(w, &format!("region_weights.{key}"))?));
            }
            region_weights.sort_by_key(|(l, _)| *l);
        }
        let rules = FlightRules {
            storey: jnum(&v, &["storey"])?,
            slab: jnum(&v, &["slab"])?,
            rise_total: jnum(&v, &["rise_total"])?,
            max_riser: jnum(&v, &["max_riser"])?,
            steps_per_cell: jnum(&v, &["steps_per_cell"])?,
            margin_cells: jint(&v, &["margin_cells"])?,
            sb_length: jint(&v, &["switchback", "length"])?,
            sb_lane_width: jint(&v, &["switchback", "lane_width"])?,
            sb_mid_rise: jnum(&v, &["switchback", "mid_landing_rise"])?,
            sb_mid_len: jnum(&v, &["switchback", "mid_landing_len"])?,
            st_length: jint(&v, &["straight", "length"])?,
            st_width: jint(&v, &["straight", "width"])?,
            sp_radius: jnum(&v, &["spiral", "radius"])?,
            sp_lane_width: jint(&v, &["spiral", "lane_width"])?,
            sp_turns: jnum(&v, &["spiral", "turns"])?,
            weights: jweights(jget(&v, &["shape_weights"])?, "shape_weights")?,
            region_weights,
        };
        rules.validate()?;
        Ok(rules)
    }

    /// Reject a table that cannot make a sound flight.
    pub fn validate(&self) -> Result<(), String> {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        if !near(self.storey, STOREY) {
            return Err(format!("flights.json: storey {} must equal the game's STOREY {STOREY}", self.storey));
        }
        if !near(self.rise_total, self.storey) {
            return Err(format!(
                "flights.json: rise_total {} must equal storey {}: a flight joins the two floors exactly",
                self.rise_total, self.storey
            ));
        }
        if !(0.0..self.storey * 0.5).contains(&self.slab) {
            return Err(format!("flights.json: slab {} must be from 0 to half a storey", self.slab));
        }
        if !(0.05..=0.5).contains(&self.max_riser) {
            return Err(format!("flights.json: max_riser {} must be from 0.05 to 0.5", self.max_riser));
        }
        if self.steps_per_cell < 1.0 || self.steps_per_cell.fract() != 0.0 {
            return Err(format!("flights.json: steps_per_cell {} must be a whole number of 1 or more", self.steps_per_cell));
        }
        if !(0..=4).contains(&self.margin_cells) {
            return Err(format!("flights.json: margin_cells {} must be from 0 to 4", self.margin_cells));
        }
        if !(3..=8).contains(&self.sb_length) {
            return Err(format!("flights.json: switchback.length {} must be from 3 to 8", self.sb_length));
        }
        if !(1..=3).contains(&self.sb_lane_width) {
            return Err(format!("flights.json: switchback.lane_width {} must be from 1 to 3", self.sb_lane_width));
        }
        if self.sb_mid_rise <= 0.0 || self.sb_mid_rise >= self.rise_total {
            return Err(format!(
                "flights.json: switchback.mid_landing_rise {} must be above 0 and below rise_total {}",
                self.sb_mid_rise, self.rise_total
            ));
        }
        if self.sb_mid_len <= 0.0 || self.sb_length as f32 - self.sb_mid_len < 1.0 {
            return Err(format!(
                "flights.json: switchback.mid_landing_len {} must be above 0 and leave 1 cell of ramp in length {}",
                self.sb_mid_len, self.sb_length
            ));
        }
        if !(4..=16).contains(&self.st_length) {
            return Err(format!("flights.json: straight.length {} must be from 4 to 16", self.st_length));
        }
        if !(1..=4).contains(&self.st_width) {
            return Err(format!("flights.json: straight.width {} must be from 1 to 4", self.st_width));
        }
        if self.sp_lane_width % 2 == 0 || !(1..=3).contains(&self.sp_lane_width) {
            return Err(format!("flights.json: spiral.lane_width {} must be 1 or 3", self.sp_lane_width));
        }
        if self.sp_radius.fract() != 0.0 || self.sp_radius < 1.0 || self.sp_radius > 6.0 {
            return Err(format!(
                "flights.json: spiral.radius {} must be a whole number of cells from 1 to 6",
                self.sp_radius
            ));
        }
        if self.sp_radius - self.sp_lane_width as f32 * 0.5 < 0.5 {
            return Err("flights.json: spiral.radius leaves no centre column inside the lane".to_string());
        }
        if ![0.25, 0.5, 0.75].iter().any(|t| near(self.sp_turns, *t)) {
            return Err(format!("flights.json: spiral.turns {} must be 0.25, 0.5 or 0.75", self.sp_turns));
        }
        for shape in [FlightShape::Switchback, FlightShape::Straight, FlightShape::Spiral] {
            let riser = max_riser_of(self, shape);
            if riser > self.max_riser + 1e-4 {
                return Err(format!(
                    "flights.json: the {shape:?} flight has a {riser:.3} m riser, above max_riser {}",
                    self.max_riser
                ));
            }
        }
        Ok(())
    }

    /// Half the side of the spiral's square, in cells (the square is `2 * half + 1` across, the centre column in its middle).
    pub fn sp_half(&self) -> i32 {
        (self.sp_radius + self.sp_lane_width as f32 * 0.5 - 0.5 - 1e-3).ceil() as i32
    }
    fn sp_span(&self) -> f32 {
        TAU * self.sp_turns
    }
    /// The flat stub past each end of the spiral's arc (an angle), half a cell of walkway.
    fn sp_ext(&self) -> f32 {
        0.5 / self.sp_radius
    }
    /// The weights in force on `level`.
    pub fn weights_for(&self, level: i32) -> ShapeWeights {
        self.region_weights.iter().find(|(l, _)| *l == level).map(|(_, w)| *w).unwrap_or(self.weights)
    }
}

/// The embedded flights table, parsed once. The shipped file is checked by a test; a bad embedded file stops the game
/// at first use with the reason.
pub fn flights() -> &'static FlightRules {
    static RULES: OnceLock<FlightRules> = OnceLock::new();
    RULES.get_or_init(|| FlightRules::parse(FILE).unwrap_or_else(|e| panic!("{e}")))
}

/// The tallest riser (metres per tread) a walker meets on a shape: the steepest slope, taken at the spiral's inner edge.
pub fn max_riser_of(rules: &FlightRules, shape: FlightShape) -> f32 {
    let slope = match shape {
        FlightShape::Straight => rules.rise_total / rules.st_length as f32,
        FlightShape::Switchback => {
            let ramp = rules.sb_length as f32 - rules.sb_mid_len;
            (rules.sb_mid_rise / ramp).max((rules.rise_total - rules.sb_mid_rise) / ramp)
        }
        FlightShape::Spiral => {
            let inner = rules.sp_radius - rules.sp_lane_width as f32 * 0.5;
            rules.rise_total / (rules.sp_span() * inner)
        }
    };
    slope / rules.steps_per_cell
}

// ---------------------------------------------------------------------------------------------------------------------
// Shape choice
// ---------------------------------------------------------------------------------------------------------------------

fn mix(seed: u64, level: i32, cell: Cell, salt: u64) -> u64 {
    let mut z = seed ^ salt;
    z = z.wrapping_add((level as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    z ^= (cell.x as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z ^= (cell.y as i64 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// The shape of the stairwell at `cell` on `level` of world `seed`: a pure function, weighted by the data (the
/// region's weights when `level` has its own, else the default).
pub fn shape_for_with(rules: &FlightRules, seed: u64, level: i32, cell: Cell) -> FlightShape {
    let w = rules.weights_for(level);
    let x = unit(mix(seed, level, cell, 0xF11_6847)) as f32 * w.total();
    if x < w.switchback {
        FlightShape::Switchback
    } else if x < w.switchback + w.straight || w.spiral <= 0.0 {
        if w.straight > 0.0 {
            FlightShape::Straight
        } else {
            FlightShape::Switchback
        }
    } else {
        FlightShape::Spiral
    }
}

pub fn shape_for(seed: u64, level: i32, cell: Cell) -> FlightShape {
    shape_for_with(flights(), seed, level, cell)
}

/// Which way a spiral at `cell` turns: a pure function of the seed, level and cell.
pub fn spiral_clockwise_for(seed: u64, level: i32, cell: Cell) -> bool {
    mix(seed, level, cell, 0x5B1_7A15) & 1 == 1
}

// ---------------------------------------------------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------------------------------------------------

fn side(run: (i32, i32)) -> (i32, i32) {
    (run.1.abs(), run.0.abs())
}

/// The cell `k` along the run and `j` across it from the anchor.
fn cell_at(f: &Flight, k: i32, j: i32) -> Cell {
    let s = side(f.run);
    Cell::new(f.anchor_cell.x + f.run.0 * k + s.0 * j, f.anchor_cell.y + f.run.1 * k + s.1 * j)
}

fn step(c: Cell, dir: (i32, i32), n: i32) -> Cell {
    Cell::new(c.x + dir.0 * n, c.y + dir.1 * n)
}

/// The far edge of the landing cell, the local origin.
fn edge(f: &Flight) -> (f32, f32) {
    (f.anchor_cell.x as f32 + (f.run.0 > 0) as i32 as f32, f.anchor_cell.y as f32 + (f.run.1 > 0) as i32 as f32)
}

fn to_local(f: &Flight, x: f32, z: f32) -> (f32, f32) {
    let e = edge(f);
    let s = side(f.run);
    (
        f.run.0 as f32 * (x - e.0) + f.run.1 as f32 * (z - e.1),
        s.0 as f32 * (x - e.0) + s.1 as f32 * (z - e.1),
    )
}

fn to_world(f: &Flight, u: f32, v: f32) -> (f32, f32) {
    let e = edge(f);
    let s = side(f.run);
    (e.0 + f.run.0 as f32 * u + s.0 as f32 * v, e.1 + f.run.1 as f32 * u + s.1 as f32 * v)
}

fn spiral_dir(rules: &FlightRules, f: &Flight) -> f32 {
    let _ = rules;
    if f.clockwise {
        -1.0
    } else {
        1.0
    }
}

/// The spiral's exit direction in the local frame (an axis, as turns are quarters): outwards from the centre at the arc's end.
fn spiral_exit(rules: &FlightRules, f: &Flight) -> (i32, i32) {
    let psi = PI + spiral_dir(rules, f) * rules.sp_span();
    (psi.cos().round() as i32, psi.sin().round() as i32)
}

/// The foot cell B as (cells along the run, cells across) from the anchor A.
fn foot_offset(rules: &FlightRules, f: &Flight) -> (i32, i32) {
    match f.shape {
        FlightShape::Switchback => (0, rules.sb_lane_width),
        FlightShape::Straight => (rules.st_length + 1, 0),
        FlightShape::Spiral => {
            let half = rules.sp_half();
            let (du, dv) = spiral_exit(rules, f);
            (half + 1 + (half + 1) * du, (half + 1) * dv)
        }
    }
}

/// (landing A, foot B).
fn ends(rules: &FlightRules, f: &Flight) -> (Cell, Cell) {
    match f.kind {
        FlightKind::Ladder => (step(f.anchor_cell, f.run, 1), f.anchor_cell),
        FlightKind::Drop => (step(f.anchor_cell, f.run, -1), f.anchor_cell),
        FlightKind::Stairs => {
            let (k, j) = foot_offset(rules, f);
            (f.anchor_cell, cell_at(f, k, j))
        }
    }
}

/// The cells of the opening, in the flight's own frame.
pub fn well_footprint_with(rules: &FlightRules, f: &Flight) -> Footprint {
    let (landing, foot) = ends(rules, f);
    let mut hole = Vec::new();
    match f.kind {
        FlightKind::Ladder | FlightKind::Drop => hole.push(f.anchor_cell),
        FlightKind::Stairs => match f.shape {
            FlightShape::Switchback => {
                for k in 1..=rules.sb_length {
                    for j in 0..2 * rules.sb_lane_width {
                        hole.push(cell_at(f, k, j));
                    }
                }
            }
            FlightShape::Straight => {
                for k in 1..=rules.st_length {
                    for j in 0..rules.st_width {
                        hole.push(cell_at(f, k, j));
                    }
                }
            }
            FlightShape::Spiral => {
                let half = rules.sp_half();
                for k in 1..=2 * half + 1 {
                    for j in -half..=half {
                        hole.push(cell_at(f, k, j));
                    }
                }
            }
        },
    }
    hole.sort();
    hole.dedup();
    // the directions the clear margins run: behind the landing, beyond the foot
    let (up_dir, low_dir) = match (f.kind, f.shape) {
        (FlightKind::Ladder, _) => (f.run, f.run),
        (FlightKind::Drop, _) => ((-f.run.0, -f.run.1), f.run),
        (FlightKind::Stairs, FlightShape::Switchback) => ((-f.run.0, -f.run.1), (-f.run.0, -f.run.1)),
        (FlightKind::Stairs, FlightShape::Straight) => ((-f.run.0, -f.run.1), f.run),
        (FlightKind::Stairs, FlightShape::Spiral) => {
            let (du, dv) = spiral_exit(rules, f);
            let s = side(f.run);
            ((-f.run.0, -f.run.1), (f.run.0 * du + s.0 * dv, f.run.1 * du + s.1 * dv))
        }
    };
    let mut upper_clear: Vec<Cell> = (0..=rules.margin_cells).map(|m| step(landing, up_dir, m)).collect();
    let mut lower_clear: Vec<Cell> = (0..=rules.margin_cells).map(|m| step(foot, low_dir, m)).collect();
    upper_clear.sort();
    lower_clear.sort();
    Footprint {
        lower_open: hole.clone(),
        upper_hole: hole,
        landing,
        foot,
        upper_clear,
        lower_clear,
        step_cell: (f.kind == FlightKind::Ladder).then_some(landing),
        slab: rules.slab,
    }
}

pub fn well_footprint(f: &Flight) -> Footprint {
    well_footprint_with(flights(), f)
}

/// The progress along the spiral (0 at the top, 1 at the foot) of a point, or `None` off the walkway. The arc has a
/// flat stub half a cell long past each end so the cells beside the landing and the foot are covered.
fn spiral_progress(rules: &FlightRules, f: &Flight, u: f32, v: f32) -> Option<f32> {
    let uc = rules.sp_half() as f32 + 0.5;
    let (du, dv) = (u - uc, v - 0.5);
    let d = (du * du + dv * dv).sqrt();
    if (d - rules.sp_radius).abs() > rules.sp_lane_width as f32 * 0.5 + 1e-4 {
        return None;
    }
    let (span, ext) = (rules.sp_span(), rules.sp_ext());
    let a = spiral_dir(rules, f) * (dv.atan2(du) - PI);
    let mid = span * 0.5;
    let mut w = (a - mid).rem_euclid(TAU);
    if w > PI {
        w -= TAU;
    }
    let a = mid + w;
    if a < -ext || a > span + ext {
        return None;
    }
    Some((a / span).clamp(0.0, 1.0))
}

/// The absolute y of the walking surface at `(x, z)` (grid cells), or `None` off the flight. Exactly the upper floor at
/// the landing cell A and exactly the lower floor at the foot cell B; between them a smooth monotone rise (flat on a
/// mid landing). A ladder or drop has only its two ends: the step cell / bank at the upper floor, the shaft or hole
/// cell at the lower floor (the ground under a climber or a falling body).
pub fn flight_surface_y_with(rules: &FlightRules, f: &Flight, x: f32, z: f32) -> Option<f32> {
    let (lo, hi) = (f.lower_y(), f.lower_y() + rules.rise_total);
    let here = Cell::new(x.floor() as i32, z.floor() as i32);
    let (landing, foot) = ends(rules, f);
    if here == landing {
        return Some(hi);
    }
    if here == foot {
        return Some(lo);
    }
    if f.kind != FlightKind::Stairs {
        return None;
    }
    let (u, v) = to_local(f, x, z);
    let rise = rules.rise_total;
    match f.shape {
        FlightShape::Straight => {
            let len = rules.st_length as f32;
            if (0.0..=len).contains(&u) && (0.0..rules.st_width as f32).contains(&v) {
                return Some(lo + rise * (1.0 - u / len));
            }
            None
        }
        FlightShape::Switchback => {
            let (len, lw) = (rules.sb_length as f32, rules.sb_lane_width as f32);
            if !(0.0..=len).contains(&u) || !(0.0..2.0 * lw).contains(&v) {
                return None;
            }
            let ramp = len - rules.sb_mid_len;
            let mid = rules.sb_mid_rise;
            Some(if u >= ramp {
                lo + mid
            } else if v < lw {
                lo + mid + (rise - mid) * (1.0 - u / ramp)
            } else {
                lo + mid * u / ramp
            })
        }
        FlightShape::Spiral => spiral_progress(rules, f, u, v).map(|t| lo + rise * (1.0 - t)),
    }
}

pub fn flight_surface_y(f: &Flight, x: f32, z: f32) -> Option<f32> {
    flight_surface_y_with(flights(), f, x, z)
}

/// Points (x, z) along the walked route of a stair from the centre of A to the centre of B, about `step` metres apart.
/// A ladder or drop gives just the two cell centres.
pub fn walk_path_with(rules: &FlightRules, f: &Flight, step: f32) -> Vec<(f32, f32)> {
    let (landing, foot) = ends(rules, f);
    if f.kind != FlightKind::Stairs {
        return vec![landing.center(), foot.center()];
    }
    let mut local: Vec<(f32, f32)> = Vec::new();
    let line = |out: &mut Vec<(f32, f32)>, a: (f32, f32), b: (f32, f32)| {
        let n = (((b.0 - a.0).hypot(b.1 - a.1)) / step).ceil().max(1.0) as i32;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
        }
    };
    let lw = rules.sb_lane_width as f32;
    match f.shape {
        FlightShape::Straight => line(&mut local, (-0.5, 0.5), (rules.st_length as f32 + 0.5, 0.5)),
        FlightShape::Switchback => {
            let land = rules.sb_length as f32 - rules.sb_mid_len * 0.5;
            line(&mut local, (-0.5, 0.5), (land, 0.5));
            line(&mut local, (land, 0.5), (land, lw + 0.5));
            line(&mut local, (land, lw + 0.5), (-0.5, lw + 0.5));
        }
        FlightShape::Spiral => {
            let (half, r) = (rules.sp_half() as f32, rules.sp_radius);
            let (uc, vc) = (half + 0.5, 0.5);
            line(&mut local, (-0.5, 0.5), (uc - r, vc));
            let (s, span) = (spiral_dir(rules, f), rules.sp_span());
            let n = (span * r / step).ceil().max(1.0) as i32;
            for i in 1..=n {
                let phi = PI + s * span * i as f32 / n as f32;
                local.push((uc + r * phi.cos(), vc + r * phi.sin()));
            }
            let (du, dv) = spiral_exit(rules, f);
            let end = *local.last().expect("arc has points");
            line(&mut local, end, (uc + (half + 1.0) * du as f32, vc + (half + 1.0) * dv as f32));
        }
    }
    local.into_iter().map(|(u, v)| to_world(f, u, v)).collect()
}

pub fn walk_path(f: &Flight, step: f32) -> Vec<(f32, f32)> {
    walk_path_with(flights(), f, step)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNS: [(i32, i32); 4] = [(0, -1), (0, 1), (1, 0), (-1, 0)];
    const SHAPES: [FlightShape; 3] = [FlightShape::Switchback, FlightShape::Straight, FlightShape::Spiral];

    fn stairs(shape: FlightShape, run: (i32, i32), cw: bool) -> Flight {
        Flight::new(FlightKind::Stairs, shape, 1, Cell::new(20, 20), run).with_clockwise(cw)
    }

    #[test]
    fn the_shipped_table_is_valid_and_matches_the_game() {
        let r = FlightRules::parse(FILE).expect("data/flights.json is valid");
        assert!((r.storey - STOREY).abs() < 1e-6);
        assert_eq!(&r, flights());
    }

    #[test]
    fn the_surface_is_the_lower_floor_at_b_and_the_upper_floor_at_a() {
        let rules = flights();
        for level in [1, 0, -2, 3] {
            for shape in SHAPES {
                for run in RUNS {
                    for cw in [false, true] {
                        let f = Flight { upper_level: level, ..stairs(shape, run, cw) };
                        let fp = well_footprint_with(rules, &f);
                        let (a, b) = (fp.landing.center(), fp.foot.center());
                        let top = flight_surface_y(&f, a.0, a.1).expect("A has a surface");
                        let bottom = flight_surface_y(&f, b.0, b.1).expect("B has a surface");
                        assert!((top - level as f32 * STOREY).abs() < 1e-4, "{shape:?} {run:?}: A {top}");
                        assert!((bottom - (level - 1) as f32 * STOREY).abs() < 1e-4, "{shape:?} {run:?}: B {bottom}");
                    }
                }
            }
        }
        for kind in [FlightKind::Ladder, FlightKind::Drop] {
            for run in RUNS {
                let f = Flight::new(kind, FlightShape::Switchback, 0, Cell::new(5, 5), run);
                let fp = well_footprint(&f);
                let (a, b) = (fp.landing.center(), fp.foot.center());
                assert_eq!(flight_surface_y(&f, a.0, a.1), Some(0.0), "{kind:?} A");
                assert_eq!(flight_surface_y(&f, b.0, b.1), Some(-STOREY), "{kind:?} B");
            }
        }
    }

    #[test]
    fn the_walked_route_is_all_surface_and_only_goes_down() {
        let rules = flights();
        for shape in SHAPES {
            for run in RUNS {
                for cw in [false, true] {
                    let f = stairs(shape, run, cw);
                    let path = walk_path_with(rules, &f, 0.05);
                    let mut prev = f32::MAX;
                    for (x, z) in &path {
                        let y = flight_surface_y(&f, *x, *z).unwrap_or_else(|| panic!("{shape:?} {run:?}: no surface at {x},{z}"));
                        assert!(y <= prev + 1e-4, "{shape:?} {run:?} cw {cw}: rises from {prev} to {y} at {x},{z}");
                        prev = y;
                    }
                    assert!((flight_surface_y(&f, path[0].0, path[0].1).unwrap() - STOREY).abs() < 1e-4);
                    assert!(prev.abs() < 1e-4, "{shape:?}: ends at {prev}");
                }
            }
        }
    }

    #[test]
    fn no_riser_is_above_the_data_max() {
        let rules = flights();
        let tread = 1.0 / rules.steps_per_cell;
        for shape in SHAPES {
            assert!(max_riser_of(rules, shape) <= rules.max_riser + 1e-4, "{shape:?}");
            let f = stairs(shape, (0, 1), false);
            let path = walk_path_with(rules, &f, tread * 0.25);
            // heights one tread apart along the route
            let ys: Vec<f32> = path.iter().map(|(x, z)| flight_surface_y(&f, *x, *z).unwrap()).collect();
            for i in 0..ys.len().saturating_sub(4) {
                assert!((ys[i] - ys[i + 4]).abs() <= rules.max_riser + 1e-3, "{shape:?} at {i}: {} -> {}", ys[i], ys[i + 4]);
            }
        }
    }

    #[test]
    fn the_switchback_has_its_mid_landing_at_the_data_rise() {
        let rules = flights();
        let f = stairs(FlightShape::Switchback, (0, 1), false);
        let (u, v) = (rules.sb_length as f32 - rules.sb_mid_len * 0.5, 0.5);
        let (x, z) = to_world(&f, u, v);
        assert!((flight_surface_y(&f, x, z).unwrap() - 2.5).abs() < 1e-4);
        assert_eq!(well_footprint(&f).upper_hole.len(), 8, "the 2x4 well");
    }
}

#[cfg(test)]
#[path = "flight_tests.rs"]
mod flight_tests;
