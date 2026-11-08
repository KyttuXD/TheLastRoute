//! One flight between two floors, defined once, in absolute y (docs/STAIRS_CONTINUOUS_DESIGN.md section 2.3).
//!
//! A level's floor is at `level * STOREY`. A [`Flight`] joins the floor of `upper_level` to the floor below it and gives
//! the walking height ([`flight_surface_y`]) and the cells both levels must open ([`well_footprint`]) from one record,
//! so the top and the bottom cannot disagree. All three shapes (switchback, straight, spiral) and the ladder and the
//! drop are defined here. Every number is in `data/flights.json` ([`flights`]); this file holds behaviour only.
//!
//! The switchback and the spiral are Godot's (stairwells.gd, streamed_facility_view_3d.gd:2486-2823) scaled to the 5 m
//! storey: a switchback is two flights `lane_width` cells wide side by side, a solid divider between them and a mid
//! landing across the whole width (the 5 x 7 interior of the 7 x 9 box); a spiral is a smooth helicoid from a centre
//! post out to the wall of a 5 x 5 well, 270 degrees from one doorway-side corner to the other. Both surfaces are smooth
//! ramps (no tread blocks). The doorway landing is floor on both levels; the rest of the interior is the well.
//!
//! Frame. The flight's local frame has `u` metres along `run` from the far edge of the landing cell (`u = 0` there;
//! cell `k` of the run covers `u` in `k-1..k`) and `v` metres across, towards the +x / +y side. The anchor cell is the
//! landing A at `u` in `-1..0`, `v` in `0..1`. A switchback's arriving (top) flight is on the +v side of the anchor
//! (Godot's flight B, columns 3-4), the divider is just on the -v side and the leaving (foot) flight beyond it
//! (columns 0-1), so the anchor is the first cell of the top flight. A spiral's anchor is the doorway-side landing cell
//! the arc starts at, its foot the landing row's other end cell (`B = A + side * 2 * dir`).
//!
//! Two layers. [`flight_surface_y`] is the whole drawn surface, including the floor slab that fills the wedge of the
//! upper floor between a spiral's two end radials. [`flight_walk_y_with`] is what a body walks on, held by one level:
//! the lower level sees the same wedge as plain floor (the slab is overhead), the upper level sees the slab.

use super::facility::Cell;
use super::vertical::{STEPS_PER_CELL, STOREY};
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
    /// Down one wide flight to a mid landing, back up the other (Godot's 5 x 7 well).
    Switchback,
    /// One longer flight.
    Straight,
    /// Around a centre post.
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
    /// A and `margin_cells` cells behind it, and the landing floor of the well, which must stay open floor on the upper level.
    pub upper_clear: Vec<Cell>,
    /// B and `margin_cells` cells beyond it, and the landing floor of the well, which must stay open floor on the lower level.
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

/// An inclusive range a tunable must stay inside (`limits` in data/flights.json).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limit {
    pub min: f32,
    pub max: f32,
}

impl Limit {
    fn holds(&self, n: f32) -> bool {
        n >= self.min && n <= self.max
    }
}

/// The bounds `validate` holds the tunables to (the `limits` block of data/flights.json).
#[derive(Clone, Debug, PartialEq)]
pub struct FlightLimits {
    /// Slab thickness in metres.
    pub slab: Limit,
    pub max_riser: Limit,
    pub margin_cells: Limit,
    pub sb_length: Limit,
    pub sb_lane_width: Limit,
    pub sb_divider_width: Limit,
    pub sb_landing_depth: Limit,
    pub st_length: Limit,
    pub st_width: Limit,
    pub sp_half: Limit,
    pub sp_landing_cells: Limit,
    pub sp_inner: Limit,
    pub sp_post: Limit,
    pub sp_walk: Limit,
    pub sp_steps: Limit,
    /// Degrees the spiral's arc starts away from the doorway axis.
    pub sp_start_offset: Limit,
    pub tiles_per_cell: Limit,
}

/// The stairwell fit-out (Godot `_add_stairwell_signs_and_light`, `_add_handrails_switchback`, `_add_landing_guards`).
/// Distances Godot gives in its 4 m storey are stored raw (`godot_*`) and scaled by [`Dressing::scale`]; heights a
/// person's size (rails, guards) are not scaled.
#[derive(Clone, Debug, PartialEq)]
pub struct Dressing {
    /// Godot's storey height (4.0): the scale of a `godot_*` distance is `storey / godot_storey`.
    pub godot_storey: f32,
    /// The "LEVEL n" sign: colour, height over the floor (Godot units), height of the lettering (Godot units) and how
    /// far it stands off the wall (metres).
    pub sign_colour: [f32; 3],
    pub sign_godot_height: f32,
    pub sign_godot_text_height: f32,
    pub sign_inset: f32,
    /// The stairwell light: colour, Godot's energy and the factor that makes it read the same under this renderer's
    /// linear falloff (the prop lights double it), range and height over mid storey (Godot units).
    pub lamp_colour: [f32; 3],
    pub lamp_energy: f32,
    pub lamp_energy_scale: f32,
    pub lamp_godot_range: f32,
    pub lamp_godot_height: f32,
    /// Handrail: height over the tread, bar thickness, distance in from the flight's edge, and the offset of the spiral's
    /// rail out from the inner radius (metres).
    pub rail_height: f32,
    pub rail_thickness: f32,
    pub rail_inset: f32,
    pub rail_inner_offset: f32,
    /// Landing guard: height and thickness of the solid rail.
    pub guard_height: f32,
    pub guard_thickness: f32,
    /// The iron of the rails, the post and the guards: Godot's colour and the brightening that lets it read here.
    pub iron_colour: [f32; 3],
    pub iron_lift: f32,
}

impl Dressing {
    /// `storey / godot_storey`: what a Godot distance is worth here.
    pub fn scale(&self) -> f32 {
        STOREY / self.godot_storey
    }
    /// The sign's height over its floor.
    pub fn sign_y(&self) -> f32 {
        self.sign_godot_height * self.scale()
    }
    /// The height of the sign's letters.
    pub fn sign_text_height(&self) -> f32 {
        self.sign_godot_text_height * self.scale()
    }
    /// The lamp's reach.
    pub fn lamp_range(&self) -> f32 {
        self.lamp_godot_range * self.scale()
    }
    /// The lamp's energy in this renderer's units.
    pub fn lamp_energy(&self) -> f32 {
        self.lamp_energy * self.lamp_energy_scale
    }
    /// The lamp's height over the lower floor: mid storey plus Godot's height over it, scaled.
    pub fn lamp_y(&self) -> f32 {
        STOREY * 0.5 + self.lamp_godot_height * self.scale()
    }
}

/// Every flight tunable (data/flights.json).
#[derive(Clone, Debug, PartialEq)]
pub struct FlightRules {
    pub limits: FlightLimits,
    pub storey: f32,
    pub slab: f32,
    pub rise_total: f32,
    pub max_riser: f32,
    pub steps_per_cell: f32,
    pub margin_cells: i32,
    /// Rows of the switchback well beyond the landing row: the ramp and the mid landing.
    pub sb_length: i32,
    /// Cells across each switchback flight.
    pub sb_lane_width: i32,
    /// Cells (metres) of solid wall between the two flights.
    pub sb_divider_width: i32,
    /// Rows of floor at the doorway end on both levels (the landing A row and the margin row behind it).
    pub sb_landing_depth: i32,
    pub sb_mid_rise: f32,
    /// Rows deep of the mid landing.
    pub sb_mid_len: i32,
    pub st_length: i32,
    pub st_width: i32,
    /// Half the side of the spiral's square well, in cells (the square is `2 * half + 1` across, the post in its middle).
    pub sp_half_cells: i32,
    /// Cells across the doorway-wide landing row of a spiral (odd).
    pub sp_landing_cells: i32,
    pub sp_inner: f32,
    pub sp_post: f32,
    pub sp_walk: f32,
    /// How far the helicoid stops short of the well's wall (metres).
    pub sp_clear: f32,
    pub sp_turns: f32,
    /// Degrees from the doorway axis (the direction back to the landing) to where the arc starts; the arc ends at the
    /// same angle on the other side when `start_offset_deg * 2 + turns * 360 = 360` (Godot: 45 + 270 + 45).
    pub sp_start_offset_deg: f32,
    pub sp_steps: i32,
    /// Tessellation of the smooth ramp: tiles along a well cell's side, per shape.
    pub tiles_switchback: i32,
    pub tiles_straight: i32,
    pub tiles_spiral: i32,
    pub dressing: Dressing,
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

/// A colour written as six hex digits (`"e8c25a"`), as 0..1 channels.
fn jcolour(v: &Json, path: &[&str]) -> Result<[f32; 3], String> {
    let text = jget(v, path)?
        .as_str()
        .ok_or_else(|| format!("flights.json: \"{}\" must be a colour string", path.join(".")))?;
    let text = text.trim_start_matches('#');
    if text.len() != 6 || !text.is_ascii() {
        return Err(format!("flights.json: \"{}\" must be six hex digits, got \"{text}\"", path.join(".")));
    }
    let mut out = [0.0f32; 3];
    for (i, c) in out.iter_mut().enumerate() {
        let byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("flights.json: \"{}\" must be six hex digits, got \"{text}\"", path.join(".")))?;
        *c = byte as f32 / 255.0;
    }
    Ok(out)
}

/// A `{ "min": a, "max": b }` limit; a missing or inverted one is refused.
fn jlimit(v: &Json, name: &str) -> Result<Limit, String> {
    let min = jnum(v, &["limits", name, "min"])?;
    let max = jnum(v, &["limits", name, "max"])?;
    if min > max {
        return Err(format!("flights.json: limits.{name} is inverted: min {min} is above max {max}"));
    }
    Ok(Limit { min, max })
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
        let limits = FlightLimits {
            slab: jlimit(&v, "slab")?,
            max_riser: jlimit(&v, "max_riser")?,
            margin_cells: jlimit(&v, "margin_cells")?,
            sb_length: jlimit(&v, "switchback_length")?,
            sb_lane_width: jlimit(&v, "switchback_lane_width")?,
            sb_divider_width: jlimit(&v, "switchback_divider_width")?,
            sb_landing_depth: jlimit(&v, "switchback_landing_depth")?,
            st_length: jlimit(&v, "straight_length")?,
            st_width: jlimit(&v, "straight_width")?,
            sp_half: jlimit(&v, "spiral_half")?,
            sp_landing_cells: jlimit(&v, "spiral_landing_cells")?,
            sp_inner: jlimit(&v, "spiral_inner_radius")?,
            sp_post: jlimit(&v, "spiral_post_radius")?,
            sp_walk: jlimit(&v, "spiral_walk_radius")?,
            sp_steps: jlimit(&v, "spiral_steps")?,
            sp_start_offset: jlimit(&v, "spiral_start_offset_deg")?,
            tiles_per_cell: jlimit(&v, "tiles_per_cell")?,
        };
        let dressing = Dressing {
            godot_storey: jnum(&v, &["dressing", "godot_storey"])?,
            sign_colour: jcolour(&v, &["dressing", "sign", "colour"])?,
            sign_godot_height: jnum(&v, &["dressing", "sign", "godot_height"])?,
            sign_godot_text_height: jnum(&v, &["dressing", "sign", "godot_text_height"])?,
            sign_inset: jnum(&v, &["dressing", "sign", "inset"])?,
            lamp_colour: jcolour(&v, &["dressing", "lamp", "colour"])?,
            lamp_energy: jnum(&v, &["dressing", "lamp", "energy"])?,
            lamp_energy_scale: jnum(&v, &["dressing", "lamp", "energy_scale"])?,
            lamp_godot_range: jnum(&v, &["dressing", "lamp", "godot_range"])?,
            lamp_godot_height: jnum(&v, &["dressing", "lamp", "godot_height_over_mid"])?,
            rail_height: jnum(&v, &["dressing", "handrail", "height"])?,
            rail_thickness: jnum(&v, &["dressing", "handrail", "thickness"])?,
            rail_inset: jnum(&v, &["dressing", "handrail", "inset"])?,
            rail_inner_offset: jnum(&v, &["dressing", "handrail", "inner_offset"])?,
            guard_height: jnum(&v, &["dressing", "guard", "height"])?,
            guard_thickness: jnum(&v, &["dressing", "guard", "thickness"])?,
            iron_colour: jcolour(&v, &["dressing", "iron", "colour"])?,
            iron_lift: jnum(&v, &["dressing", "iron", "lift"])?,
        };
        let rules = FlightRules {
            limits,
            storey: jnum(&v, &["storey"])?,
            slab: jnum(&v, &["slab"])?,
            rise_total: jnum(&v, &["rise_total"])?,
            max_riser: jnum(&v, &["max_riser"])?,
            steps_per_cell: jnum(&v, &["steps_per_cell"])?,
            margin_cells: jint(&v, &["margin_cells"])?,
            sb_length: jint(&v, &["switchback", "length"])?,
            sb_lane_width: jint(&v, &["switchback", "lane_width"])?,
            sb_divider_width: jint(&v, &["switchback", "divider_width"])?,
            sb_landing_depth: jint(&v, &["switchback", "landing_depth"])?,
            sb_mid_rise: jnum(&v, &["switchback", "mid_landing_rise"])?,
            sb_mid_len: jint(&v, &["switchback", "mid_landing_len"])?,
            st_length: jint(&v, &["straight", "length"])?,
            st_width: jint(&v, &["straight", "width"])?,
            sp_half_cells: jint(&v, &["spiral", "half"])?,
            sp_landing_cells: jint(&v, &["spiral", "landing_cells"])?,
            sp_inner: jnum(&v, &["spiral", "inner_radius"])?,
            sp_post: jnum(&v, &["spiral", "post_radius"])?,
            sp_walk: jnum(&v, &["spiral", "walk_radius"])?,
            sp_clear: jnum(&v, &["spiral", "wall_clearance"])?,
            sp_turns: jnum(&v, &["spiral", "turns"])?,
            sp_start_offset_deg: jnum(&v, &["spiral", "start_offset_deg"])?,
            sp_steps: jint(&v, &["spiral", "steps"])?,
            tiles_switchback: jint(&v, &["render", "tiles_per_cell", "switchback"])?,
            tiles_straight: jint(&v, &["render", "tiles_per_cell", "straight"])?,
            tiles_spiral: jint(&v, &["render", "tiles_per_cell", "spiral"])?,
            dressing,
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
        let l = &self.limits;
        let ranged = |label: &str, n: f32, r: &Limit| -> Result<(), String> {
            if r.holds(n) {
                Ok(())
            } else {
                Err(format!("flights.json: {label} {n} must be from {} to {} (limits)", r.min, r.max))
            }
        };
        ranged("slab", self.slab, &l.slab)?;
        ranged("max_riser", self.max_riser, &l.max_riser)?;
        if self.steps_per_cell < 1.0 || self.steps_per_cell.fract() != 0.0 {
            return Err(format!("flights.json: steps_per_cell {} must be a whole number of 1 or more", self.steps_per_cell));
        }
        if !near(self.steps_per_cell, STEPS_PER_CELL as f32) {
            return Err(format!(
                "flights.json: steps_per_cell {} must equal the game's STEPS_PER_CELL {STEPS_PER_CELL} (world/vertical.rs)",
                self.steps_per_cell
            ));
        }
        ranged("margin_cells", self.margin_cells as f32, &l.margin_cells)?;
        ranged("switchback.length", self.sb_length as f32, &l.sb_length)?;
        ranged("switchback.lane_width", self.sb_lane_width as f32, &l.sb_lane_width)?;
        ranged("switchback.divider_width", self.sb_divider_width as f32, &l.sb_divider_width)?;
        ranged("switchback.landing_depth", self.sb_landing_depth as f32, &l.sb_landing_depth)?;
        if self.sb_mid_rise <= 0.0 || self.sb_mid_rise >= self.rise_total {
            return Err(format!(
                "flights.json: switchback.mid_landing_rise {} must be above 0 and below rise_total {}",
                self.sb_mid_rise, self.rise_total
            ));
        }
        if self.sb_mid_len < 1 || self.sb_length - self.sb_mid_len < 1 {
            return Err(format!(
                "flights.json: switchback.mid_landing_len {} must be at least 1 and leave 1 row of ramp in length {}",
                self.sb_mid_len, self.sb_length
            ));
        }
        ranged("straight.length", self.st_length as f32, &l.st_length)?;
        ranged("straight.width", self.st_width as f32, &l.st_width)?;
        ranged("spiral.half", self.sp_half_cells as f32, &l.sp_half)?;
        ranged("spiral.landing_cells", self.sp_landing_cells as f32, &l.sp_landing_cells)?;
        if self.sp_landing_cells % 2 == 0 || self.sp_landing_cells > 2 * self.sp_half_cells + 1 {
            return Err(format!(
                "flights.json: spiral.landing_cells {} must be odd and fit the well's {} cells",
                self.sp_landing_cells,
                2 * self.sp_half_cells + 1
            ));
        }
        ranged("spiral.inner_radius", self.sp_inner, &l.sp_inner)?;
        ranged("spiral.post_radius", self.sp_post, &l.sp_post)?;
        ranged("spiral.walk_radius", self.sp_walk, &l.sp_walk)?;
        ranged("spiral.steps", self.sp_steps as f32, &l.sp_steps)?;
        if self.sp_clear < 0.0 || self.sp_clear >= 0.5 {
            return Err(format!("flights.json: spiral.wall_clearance {} must be from 0 to under half a cell", self.sp_clear));
        }
        if !(self.sp_post < self.sp_inner && self.sp_inner < self.sp_walk && self.sp_walk < self.sp_reach()) {
            return Err(format!(
                "flights.json: the spiral radii must grow: post_radius {} < inner_radius {} < walk_radius {} < the wall at {}",
                self.sp_post,
                self.sp_inner,
                self.sp_walk,
                self.sp_reach()
            ));
        }
        if ![0.25, 0.5, 0.75].iter().any(|t| near(self.sp_turns, *t)) {
            return Err(format!("flights.json: spiral.turns {} must be 0.25, 0.5 or 0.75", self.sp_turns));
        }
        ranged("spiral.start_offset_deg", self.sp_start_offset_deg, &l.sp_start_offset)?;
        // the arc must end before it meets its own start again on the far side of the doorway axis
        if self.sp_start_offset_deg * 2.0 + self.sp_turns * 360.0 > 360.0 + 1e-3 {
            return Err(format!(
                "flights.json: spiral.start_offset_deg {} twice plus {} turns overruns the circle: the arc would cross its own start",
                self.sp_start_offset_deg, self.sp_turns
            ));
        }
        // the slab rail runs along the end radial to the landing row, so that radial must not run parallel to it
        if (self.sp_start_offset_deg + self.sp_turns * 360.0).to_radians().cos().abs() < MIN_END_RADIAL_COS {
            return Err(format!(
                "flights.json: spiral.start_offset_deg {} with {} turns ends the arc along the landing row, so the slab rail would never reach it",
                self.sp_start_offset_deg, self.sp_turns
            ));
        }
        for (label, n) in [
            ("switchback", self.tiles_switchback),
            ("straight", self.tiles_straight),
            ("spiral", self.tiles_spiral),
        ] {
            ranged(&format!("render.tiles_per_cell.{label}"), n as f32, &l.tiles_per_cell)?;
        }
        let d = &self.dressing;
        if d.godot_storey <= 0.0 {
            return Err(format!("flights.json: dressing.godot_storey {} must be above 0", d.godot_storey));
        }
        for (label, n) in [
            ("sign.godot_height", d.sign_godot_height),
            ("sign.godot_text_height", d.sign_godot_text_height),
            ("lamp.energy", d.lamp_energy),
            ("lamp.energy_scale", d.lamp_energy_scale),
            ("lamp.godot_range", d.lamp_godot_range),
            ("handrail.height", d.rail_height),
            ("handrail.thickness", d.rail_thickness),
            ("guard.height", d.guard_height),
            ("guard.thickness", d.guard_thickness),
            ("iron.lift", d.iron_lift),
        ] {
            if !(n > 0.0 && n.is_finite()) {
                return Err(format!("flights.json: dressing.{label} {n} must be above 0"));
            }
        }
        for (label, n) in [("sign.inset", d.sign_inset), ("handrail.inset", d.rail_inset), ("handrail.inner_offset", d.rail_inner_offset)] {
            if !(n >= 0.0 && n.is_finite()) {
                return Err(format!("flights.json: dressing.{label} {n} must not be negative"));
            }
        }
        if d.sign_y() + d.sign_text_height() * 0.5 >= self.storey {
            return Err(format!("flights.json: dressing.sign stands at {} m, past the storey {}", d.sign_y(), self.storey));
        }
        if d.lamp_y() >= self.storey {
            return Err(format!("flights.json: dressing.lamp hangs at {} m, past the storey {}", d.lamp_y(), self.storey));
        }
        if d.rail_height >= d.guard_height || d.guard_height >= self.storey {
            return Err(format!(
                "flights.json: dressing heights must grow: handrail {} < guard {} < storey {}",
                d.rail_height, d.guard_height, self.storey
            ));
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

    /// Half the side of the spiral's square, in cells (the square is `2 * half + 1` across, the post in its middle).
    pub fn sp_half(&self) -> i32 {
        self.sp_half_cells
    }
    /// How far from the post centre the helicoid reaches along an axis (the wall, less the clearance).
    pub fn sp_reach(&self) -> f32 {
        self.sp_half_cells as f32 + 0.5 - self.sp_clear
    }
    fn sp_span(&self) -> f32 {
        TAU * self.sp_turns
    }
    /// The start offset in radians.
    fn sp_offset(&self) -> f32 {
        self.sp_start_offset_deg.to_radians()
    }
    /// The rows of a switchback's ramp.
    pub fn sb_ramp(&self) -> f32 {
        (self.sb_length - self.sb_mid_len) as f32
    }
    /// The first and last `v` of a switchback's well, across (the foot flight's far edge, the top flight's far edge).
    pub fn sb_v_range(&self) -> (f32, f32) {
        (-((self.sb_divider_width + self.sb_lane_width) as f32), self.sb_lane_width as f32)
    }
    /// How finely each well cell of `shape` is tessellated for the smooth ramp.
    pub fn tiles_for(&self, shape: FlightShape) -> i32 {
        match shape {
            FlightShape::Switchback => self.tiles_switchback,
            FlightShape::Straight => self.tiles_straight,
            FlightShape::Spiral => self.tiles_spiral,
        }
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

/// The tallest riser (metres per tread) a walker meets on a shape: the slope on the walking line, per 1/steps_per_cell
/// metre of run. The surfaces are smooth ramps, so this bounds the slope, not a block; the spiral's walking line is the
/// radius the walk path runs on (its inner edge is steeper still, but a body is kept off it by the post).
pub fn max_riser_of(rules: &FlightRules, shape: FlightShape) -> f32 {
    let slope = match shape {
        FlightShape::Straight => rules.rise_total / rules.st_length as f32,
        FlightShape::Switchback => {
            let ramp = rules.sb_ramp();
            (rules.sb_mid_rise / ramp).max((rules.rise_total - rules.sb_mid_rise) / ramp)
        }
        FlightShape::Spiral => rules.rise_total / (rules.sp_span() * rules.sp_walk),
    };
    slope / rules.steps_per_cell
}

// ---------------------------------------------------------------------------------------------------------------------
// Shape choice
// ---------------------------------------------------------------------------------------------------------------------

fn mix(seed: u64, upper_level: i32, cell: Cell, salt: u64) -> u64 {
    let mut z = seed ^ salt;
    z = z.wrapping_add((upper_level as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
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

/// The shape of the stairwell at `cell` of world `seed`: a pure function, weighted by the data (the region's weights
/// when `upper_level` has its own, else the default). Callers pass the UPPER level of the connection (the level whose
/// floor the top reaches, `Flight::upper_level`) and the landing cell A on it, so both sides of one stairwell, which
/// know the same flight, get the same shape. The lower side passes `lower_level + 1` and the anchor that
/// `Flight::from_foot_with` finds.
pub fn shape_for_with(rules: &FlightRules, seed: u64, upper_level: i32, cell: Cell) -> FlightShape {
    let w = rules.weights_for(upper_level);
    let x = unit(mix(seed, upper_level, cell, 0xF11_6847)) as f32 * w.total();
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

pub fn shape_for(seed: u64, upper_level: i32, cell: Cell) -> FlightShape {
    shape_for_with(flights(), seed, upper_level, cell)
}

/// Which way a spiral at `cell` turns: a pure function of the seed, the UPPER level and the anchor cell (see
/// [`shape_for_with`]), so both sides of one stairwell agree.
pub fn spiral_clockwise_for(seed: u64, upper_level: i32, cell: Cell) -> bool {
    mix(seed, upper_level, cell, 0x5B1_7A15) & 1 == 1
}

/// The four unit steps of a ladder's step cell in the order they are tried for the hatch `cell` of chunk column
/// `chunk` in world `seed` (the world seed as `flight_links::seed_key`): a rotation of north, east, south, west that
/// starts at a seeded point and turns one way or the other. A pure function, so both levels agree.
pub fn ladder_dir_order(seed: u64, chunk: (i32, i32), cell: Cell) -> [(i32, i32); 4] {
    const DIRS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
    let column = seed
        ^ (chunk.0 as i64 as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ (chunk.1 as i64 as u64).wrapping_mul(0xA076_1D64_78BD_642F);
    let h = mix(column, 0, cell, 0x1ADD_E4);
    let start = (h % 4) as usize;
    let backwards = (h >> 8) & 1 == 1;
    std::array::from_fn(|i| DIRS[if backwards { (start + 4 - i) % 4 } else { (start + i) % 4 }])
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

/// The spiral's centre in the local frame. The landing row is the anchor's row and the well is the `2 * half + 1` square
/// whose near side is that row, so the centre is `half - 0.5` along the run (the anchor row occupies `u` in `-1..0`);
/// across, the doorway's middle cell is `dir` cells across from the landing A, so the centre is over its middle.
fn spiral_centre(rules: &FlightRules, f: &Flight) -> (f32, f32) {
    (rules.sp_half() as f32 - 0.5, 0.5 + spiral_dir(rules, f))
}

/// The angle (in the local frame) of the point at `swept` radians along the spiral, counted from the doorway axis: the
/// doorway axis points back from the centre to the landing row (angle `PI`), and the spiral turns away from it `dir` way.
fn spiral_angle(rules: &FlightRules, f: &Flight, swept: f32) -> f32 {
    PI + spiral_dir(rules, f) * swept
}

/// The point of the spiral's walking line at `swept` radians from the doorway axis (local frame).
fn spiral_point(rules: &FlightRules, f: &Flight, swept: f32) -> (f32, f32) {
    spiral_point_at(rules, f, swept, rules.sp_walk)
}

/// The point at `swept` radians from the doorway axis and radius `r` from the post (local frame).
fn spiral_point_at(rules: &FlightRules, f: &Flight, swept: f32, r: f32) -> (f32, f32) {
    let (uc, vc) = spiral_centre(rules, f);
    let phi = spiral_angle(rules, f, swept);
    (uc + r * phi.cos(), vc + r * phi.sin())
}

/// The foot cell B as (cells along the run, cells across) from the anchor A. A switchback's foot is the cell beside the
/// divider on the leaving flight, level with A; a spiral's is the landing row's other end cell, `B = A + side * 2 * dir`
/// (Godot's top and bottom landings are the one landing row, stairwells.gd:116-134).
fn foot_offset(rules: &FlightRules, f: &Flight) -> (i32, i32) {
    match f.shape {
        FlightShape::Switchback => (0, -(rules.sb_divider_width + 1)),
        FlightShape::Straight => (rules.st_length + 1, 0),
        FlightShape::Spiral => (0, 2 * spiral_dir(rules, f) as i32),
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

/// The floor of the well's landing on both levels: a switchback's `landing_depth` rows across its whole width (rows `k`
/// = 0, -1, ...), a spiral's doorway-wide landing row (A, the cell between and B). Empty for the other flights.
fn landing_zone(rules: &FlightRules, f: &Flight) -> Vec<Cell> {
    let mut out = Vec::new();
    if f.kind != FlightKind::Stairs {
        return out;
    }
    match f.shape {
        FlightShape::Switchback => {
            let (v0, v1) = rules.sb_v_range();
            for r in 0..rules.sb_landing_depth {
                for j in v0 as i32..v1 as i32 {
                    out.push(cell_at(f, -r, j));
                }
            }
        }
        FlightShape::Straight => {}
        FlightShape::Spiral => {
            let mid = spiral_dir(rules, f) as i32;
            let door = rules.sp_landing_cells / 2;
            for j in mid - door..=mid + door {
                out.push(cell_at(f, 0, j));
            }
        }
    }
    out
}

/// The landing row: the cells of the well's landing at the anchor's own row (`k` = 0), A and B among them. A body stepping
/// onto the flight has its back corners over these, so they carry the flat floor of the level (see [`flight_walk_y_with`]).
pub fn landing_row_with(rules: &FlightRules, f: &Flight) -> Vec<Cell> {
    if f.kind != FlightKind::Stairs {
        return vec![];
    }
    match f.shape {
        FlightShape::Switchback => {
            let (v0, v1) = rules.sb_v_range();
            (v0 as i32..v1 as i32).map(|j| cell_at(f, 0, j)).collect()
        }
        FlightShape::Straight => vec![],
        FlightShape::Spiral => {
            let mid = spiral_dir(rules, f) as i32;
            let door = rules.sp_landing_cells / 2;
            (mid - door..=mid + door).map(|j| cell_at(f, 0, j)).collect()
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
                let (v0, v1) = rules.sb_v_range();
                for k in 1..=rules.sb_length {
                    for j in v0 as i32..v1 as i32 {
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
                // the square around the post, whose near row is the landing row: its doorway-wide middle is floor, the
                // rest (the two corner cells of that row and every row beyond) is the well
                let half = rules.sp_half();
                let mid = spiral_dir(rules, f) as i32;
                let door = rules.sp_landing_cells / 2;
                for k in 0..=2 * half {
                    for j in mid - half..=mid + half {
                        if k == 0 && (j - mid).abs() <= door {
                            continue;
                        }
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
        // both ends are landing-row cells: the clear cells lie behind the row, away from the well
        (FlightKind::Stairs, FlightShape::Spiral) => ((-f.run.0, -f.run.1), (-f.run.0, -f.run.1)),
    };
    let zone = landing_zone(rules, f);
    let mut upper_clear: Vec<Cell> = (0..=rules.margin_cells).map(|m| step(landing, up_dir, m)).collect();
    let mut lower_clear: Vec<Cell> = (0..=rules.margin_cells).map(|m| step(foot, low_dir, m)).collect();
    upper_clear.extend(zone.iter().copied());
    lower_clear.extend(zone.iter().copied());
    upper_clear.sort();
    upper_clear.dedup();
    lower_clear.sort();
    lower_clear.dedup();
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

/// Which surface of the spiral a point is on: the helicoid itself, or the slab of upper floor in the wedge between the
/// two end radials (Godot's top slab and corner slab).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpiralLayer {
    Helix,
    Slab,
}

/// The spiral's surface at the local point `(u, v)`: its absolute height and layer, or `None` off it (outside the
/// well, in the post, or in the open corner on the bottom end's side, which is a drop to the lower floor).
///
/// - The helicoid: the arc `start_offset .. start_offset + turns` swept from the doorway axis, from the inner radius out
///   to the well's wall, `y` falling from the upper floor at the start radial to the lower floor at the end radial, a
///   function of the angle only (so it is a true helicoid, steepest at the post).
/// - The slab: the wedge between the two radials that faces the doorway is upper floor. The triangle from the post to
///   the landing row's inner edge, and on the arc's start side the corner cell of the landing row beyond it. The bottom
///   end's side corner stays open.
fn spiral_surface(rules: &FlightRules, f: &Flight, u: f32, v: f32) -> Option<(f32, SpiralLayer)> {
    let (uc, vc) = spiral_centre(rules, f);
    let (du, dv) = (u - uc, v - vc);
    let reach = rules.sp_reach();
    if du.abs() > reach || dv.abs() > reach {
        return None;
    }
    if du.hypot(dv) < rules.sp_inner {
        return None;
    }
    let (span, off) = (rules.sp_span(), rules.sp_offset());
    let a = (spiral_dir(rules, f) * (dv.atan2(du) - PI)).rem_euclid(TAU);
    let hi = f.lower_y() + rules.rise_total;
    if a >= off && a <= off + span {
        return Some((hi - rules.rise_total * (a - off) / span, SpiralLayer::Helix));
    }
    // the wedge facing the doorway: upper floor beyond the landing row's inner edge (u >= 0), and on the start side the
    // landing row's corner cell, past the landing row's end (half the doorway out from the post)
    if u >= 0.0 || (a < off && dv.abs() >= rules.sp_landing_cells as f32 * 0.5) {
        return Some((hi, SpiralLayer::Slab));
    }
    None
}

/// The absolute y of the walking surface at `(x, z)` (grid cells), or `None` off the flight. Exactly the upper floor at
/// the landing cell A and exactly the lower floor at the foot cell B; between them a smooth monotone rise (flat on a
/// mid landing). A ladder or drop has only its two ends: the step cell / bank at the upper floor, the shaft or hole
/// cell at the lower floor (the ground under a climber or a falling body). This is the whole drawn surface; see
/// [`flight_walk_y_with`] for what a body of one level walks on.
pub fn flight_surface_y_with(rules: &FlightRules, f: &Flight, x: f32, z: f32) -> Option<f32> {
    surface_with(rules, f, x, z, false)
}

/// The walking surface as the level that holds the record sees it: `lower` is the lower level's record (dir > 0). Equal
/// to [`flight_surface_y_with`] except in a spiral's wedge between the end radials, where the slab is overhead for the
/// lower level (its floor is the plain floor, held flat at the lower floor, so the walker arriving at the foot of the
/// helicoid steps off onto it) and the open corner on the arc's end side is floor, not a drop.
pub fn flight_walk_y_with(rules: &FlightRules, f: &Flight, x: f32, z: f32, lower: bool) -> Option<f32> {
    surface_with(rules, f, x, z, lower)
}

fn surface_with(rules: &FlightRules, f: &Flight, x: f32, z: f32, lower_view: bool) -> Option<f32> {
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
    // the rest of the landing row is the floor of the level the walker is on (a body sinking onto the ramp still has its
    // back corners over it): the upper floor for the upper level's record, the lower floor for the lower level's
    if landing_row_with(rules, f).contains(&here) {
        return Some(if lower_view { lo } else { hi });
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
            let len = rules.sb_length as f32;
            let (v0, v1) = rules.sb_v_range();
            if !(0.0..=len).contains(&u) || !(v0..=v1).contains(&v) {
                return None;
            }
            let ramp = rules.sb_ramp();
            let mid = rules.sb_mid_rise;
            let dw = rules.sb_divider_width as f32;
            Some(if u >= ramp {
                lo + mid
            } else if v >= 0.0 {
                // the arriving flight, on the +v side: the upper floor at the landing row down to the mid landing
                lo + mid + (rise - mid) * (1.0 - u / ramp)
            } else if v > -dw {
                // the solid divider
                return None;
            } else {
                // the leaving flight: the lower floor at the landing row up to the mid landing
                lo + mid * u / ramp
            })
        }
        FlightShape::Spiral => match spiral_surface(rules, f, u, v) {
            // the slab is overhead for the lower level: its floor there is the plain floor
            Some((_, SpiralLayer::Slab)) if lower_view => Some(lo),
            Some((y, _)) => Some(y),
            // the open corner on the arc's end side: a drop above, floor below
            None => (lower_view && spiral_floor(rules, f, u, v)).then_some(lo),
        },
    }
}

/// Whether the local point is on the spiral's well floor outside the arc: inside the square, past the post, not on the
/// helicoid. The lower level's floor (the plain floor under the slab and in the open corner).
fn spiral_floor(rules: &FlightRules, f: &Flight, u: f32, v: f32) -> bool {
    let (uc, vc) = spiral_centre(rules, f);
    let (du, dv) = (u - uc, v - vc);
    let reach = rules.sp_reach();
    du.abs() <= reach && dv.abs() <= reach && du.hypot(dv) >= rules.sp_inner
}

pub fn flight_surface_y(f: &Flight, x: f32, z: f32) -> Option<f32> {
    flight_surface_y_with(flights(), f, x, z)
}

/// Whether `(x, z)` is on the slab of upper floor that fills a spiral's wedge between its end radials (the drawn surface
/// there is the upper floor; the lower level walks on the plain floor under it). False for every other flight.
pub fn on_slab(f: &Flight, x: f32, z: f32) -> bool {
    if f.kind != FlightKind::Stairs || f.shape != FlightShape::Spiral {
        return false;
    }
    let (u, v) = to_local(f, x, z);
    matches!(spiral_surface(flights(), f, u, v), Some((_, SpiralLayer::Slab)))
}

/// A line segment in grid coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: (f32, f32),
    pub b: (f32, f32),
}

/// The solid divider between the two flights of a switchback, as the centre line of a wall `divider_width` metres thick
/// whose two ends are flat: it runs the length of the ramp (from the landing row to the mid landing), from the floor to
/// the upper floor. Empty for the other shapes, ladders and drops. Grid coordinates, the same list whichever level's
/// record asks.
pub fn lane_dividers_with(rules: &FlightRules, f: &Flight) -> Vec<Segment> {
    if f.kind != FlightKind::Stairs || f.shape != FlightShape::Switchback || rules.sb_divider_width == 0 {
        return vec![];
    }
    let mid_v = -(rules.sb_divider_width as f32) * 0.5;
    vec![Segment { a: to_world(f, 0.0, mid_v), b: to_world(f, rules.sb_ramp(), mid_v) }]
}

pub fn lane_dividers(f: &Flight) -> Vec<Segment> {
    lane_dividers_with(flights(), f)
}

/// The width in metres of the wall a [`lane_dividers`] segment is the centre line of.
pub fn divider_width() -> f32 {
    flights().sb_divider_width as f32
}

/// The spiral's centre post: its centre in grid coordinates and its radius. `None` for the other flights.
pub fn spiral_post_with(rules: &FlightRules, f: &Flight) -> Option<((f32, f32), f32)> {
    if f.kind != FlightKind::Stairs || f.shape != FlightShape::Spiral {
        return None;
    }
    let (uc, vc) = spiral_centre(rules, f);
    Some((to_world(f, uc, vc), rules.sp_post))
}

pub fn spiral_post(f: &Flight) -> Option<((f32, f32), f32)> {
    spiral_post_with(flights(), f)
}

/// Points (x, z) along the walked route of a stair from the centre of A to the centre of B, about `step` metres apart.
/// A ladder or drop gives just the two cell centres. A switchback goes down the arriving flight's centre line, across the
/// mid landing and back up the leaving flight's; a spiral round the helicoid's walking line.
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
    match f.shape {
        FlightShape::Straight => line(&mut local, (-0.5, 0.5), (rules.st_length as f32 + 0.5, 0.5)),
        FlightShape::Switchback => {
            let (lw, dw) = (rules.sb_lane_width as f32, rules.sb_divider_width as f32);
            let (top_c, foot_c) = (lw * 0.5, -dw - lw * 0.5);
            let land = rules.sb_length as f32 - rules.sb_mid_len as f32 * 0.5;
            // A's centre to the arriving flight's centre line (still inside A's cell), down it to the mid landing, across
            // the mid landing, up the leaving flight's centre line, and into B's cell
            line(&mut local, (-0.5, 0.5), (0.0, top_c));
            line(&mut local, (0.0, top_c), (land, top_c));
            line(&mut local, (land, top_c), (land, foot_c));
            line(&mut local, (land, foot_c), (0.0, foot_c));
            line(&mut local, (0.0, foot_c), (-0.5, -dw - 0.5));
        }
        FlightShape::Spiral => {
            // landing A, in to where the arc starts, round the arc, out to foot B
            let (span, off, r) = (rules.sp_span(), rules.sp_offset(), rules.sp_walk);
            let dir = spiral_dir(rules, f);
            line(&mut local, (-0.5, 0.5), spiral_point(rules, f, off));
            let n = (span * r / step).ceil().max(1.0) as i32;
            for i in 1..=n {
                local.push(spiral_point(rules, f, off + span * i as f32 / n as f32));
            }
            line(&mut local, spiral_point(rules, f, off + span), (-0.5, 0.5 + 2.0 * dir));
        }
    }
    local.into_iter().map(|(u, v)| to_world(f, u, v)).collect()
}

pub fn walk_path(f: &Flight, step: f32) -> Vec<(f32, f32)> {
    walk_path_with(flights(), f, step)
}

// ---------------------------------------------------------------------------------------------------------------------
// Dressing: guards, handrails, signs, lamp (all pure geometry in grid coordinates; render/stairs.rs draws them)
// ---------------------------------------------------------------------------------------------------------------------

/// A guard rail along the rim of the well on the UPPER level: a solid rail `guard_height` high (the hole rule keeps a
/// walker out of the well already; this is what shows it) between the well and `outside`, the floor cell beside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guard {
    pub seg: Segment,
    /// The floor cell on the walker's side of the rail.
    pub outside: Cell,
}

/// The rim guards of the upper level, with a gap where the stairs arrive (Godot `_add_landing_guards`).
///
/// - A switchback: along the landing row's inner edge across the leaving flight and the divider (the arriving flight's
///   width is the gap), so the whole edge is guarded but the head of the stairs.
/// - A spiral: every stretch of the well's rim the walker cannot step over (see [`spiral_rim_guards`]): the whole outer
///   edge of the 5 x 5 well, which has no surface in its last 2 cm, and the open corner on the arc's end side; the
///   doorway edge and the corner slab on the start side are floored, so they are the gap. The slab's own rail is
///   [`spiral_slab_rail`].
/// - A straight flight: every rim edge of the well that borders an open cell, except the one at the landing.
pub fn landing_guards_with(rules: &FlightRules, f: &Flight) -> Vec<Guard> {
    if f.kind != FlightKind::Stairs {
        return vec![];
    }
    let seg = |a: (f32, f32), b: (f32, f32)| Segment { a: to_world(f, a.0, a.1), b: to_world(f, b.0, b.1) };
    match f.shape {
        FlightShape::Switchback => {
            // one guard per cell edge of the landing row's inner edge, from the leaving flight's far side to the divider's
            // near edge: each stands beside the one floor cell of the row it guards
            let (v0, _) = rules.sb_v_range();
            (v0 as i32..0)
                .map(|j| Guard { seg: seg((0.0, j as f32), (0.0, (j + 1) as f32)), outside: cell_at(f, 0, j) })
                .collect()
        }
        FlightShape::Spiral => spiral_rim_guards(rules, f),
        FlightShape::Straight => {
            let hole = well_footprint_with(rules, f).upper_hole;
            let (landing, _) = ends(rules, f);
            let mut out = Vec::new();
            for c in &hole {
                for d in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let n = Cell::new(c.x + d.0, c.y + d.1);
                    if hole.contains(&n) || n == landing {
                        continue;
                    }
                    // the edge of cell c towards n
                    let (x0, z0) = (c.x as f32, c.y as f32);
                    let (a, b) = match d {
                        (1, 0) => ((x0 + 1.0, z0), (x0 + 1.0, z0 + 1.0)),
                        (-1, 0) => ((x0, z0), (x0, z0 + 1.0)),
                        (0, 1) => ((x0, z0 + 1.0), (x0 + 1.0, z0 + 1.0)),
                        _ => ((x0, z0), (x0 + 1.0, z0)),
                    };
                    out.push(Guard { seg: Segment { a, b }, outside: n });
                }
            }
            out
        }
    }
}

/// How finely a spiral's rim is read for the guards: samples per cell edge (a metre).
const RIM_SAMPLES: usize = 20;
/// How far inside the well cell a rim sample is taken (m): the first thing a body's corner meets when it crosses the edge.
const RIM_INSET: f32 = 0.005;
/// An opening in a spiral's guard narrower than this many samples (0.45 m, a body is 0.46 m across) cannot be walked
/// through, so it is closed.
const RIM_MIN_GAP: usize = 9;

/// Whether a body standing on the upper floor can step over the rim at `(x, z)` (grid coordinates, just inside a well
/// cell): the collision rule of `Facility::body_fits_at_y`, which lets a body over a stair well only where the flight's
/// surface is within `FLIGHT_FIT_TOL` of its walking height (the upper floor).
fn spiral_rim_open(rules: &FlightRules, f: &Flight, x: f32, z: f32) -> bool {
    flight_walk_y_with(rules, f, x, z, false).is_some_and(|y| (y - f.upper_y()).abs() <= super::facility::FLIGHT_FIT_TOL)
}

/// The guards of a spiral's rim on the upper level: for every edge of the well that borders a cell outside it, the
/// stretches where a body is stopped (read from the same surface the collision reads, so the rail and the collision
/// agree). The well is a square whose outer 2 cm (`spiral.wall_clearance`) has no surface, so its whole outer edge is
/// closed to a body; the doorway edge is floored by the slab and is the gap, as is the corner cell on the arc's start
/// side; the corner on the arc's end side is a drop and is closed. Openings narrower than a body are closed.
fn spiral_rim_guards(rules: &FlightRules, f: &Flight) -> Vec<Guard> {
    let hole = well_footprint_with(rules, f).upper_hole;
    let mut out = Vec::new();
    for c in &hole {
        for d in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let n = Cell::new(c.x + d.0, c.y + d.1);
            if hole.contains(&n) {
                continue;
            }
            let (x0, z0) = (c.x as f32, c.y as f32);
            // the point `t` along the edge towards n
            let at = |t: f32| match d {
                (1, 0) => (x0 + 1.0, z0 + t),
                (-1, 0) => (x0, z0 + t),
                (0, 1) => (x0 + t, z0 + 1.0),
                _ => (x0 + t, z0),
            };
            let mut closed: Vec<bool> = (0..RIM_SAMPLES)
                .map(|i| {
                    let (x, z) = at((i as f32 + 0.5) / RIM_SAMPLES as f32);
                    !spiral_rim_open(rules, f, x - d.0 as f32 * RIM_INSET, z - d.1 as f32 * RIM_INSET)
                })
                .collect();
            // close the openings between two closed stretches that a body cannot pass
            let mut i = 0;
            while i < RIM_SAMPLES {
                if closed[i] {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < RIM_SAMPLES && !closed[j] {
                    j += 1;
                }
                if i > 0 && j < RIM_SAMPLES && j - i < RIM_MIN_GAP {
                    closed[i..j].iter_mut().for_each(|c| *c = true);
                }
                i = j;
            }
            let mut i = 0;
            while i < RIM_SAMPLES {
                if !closed[i] {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < RIM_SAMPLES && closed[j] {
                    j += 1;
                }
                out.push(Guard {
                    seg: Segment { a: at(i as f32 / RIM_SAMPLES as f32), b: at(j as f32 / RIM_SAMPLES as f32) },
                    outside: n,
                });
                i = j;
            }
        }
    }
    out
}

pub fn landing_guards(f: &Flight) -> Vec<Guard> {
    landing_guards_with(flights(), f)
}

/// Smallest |cos| of the spiral's end radial the slab rail accepts (about 0.06 degrees off horizontal).
const MIN_END_RADIAL_COS: f32 = 1e-3;

/// The rail along the spiral's end radial on the upper level (Godot: a 1.1 m collider and a 0.95 m rail from the post
/// to the landing row's inner edge, where the slab beside the helicoid's lowest step drops a storey): grid coordinates,
/// from `post_radius` out from the post to the landing row's edge. `None` for the other flights.
pub fn spiral_slab_rail_with(rules: &FlightRules, f: &Flight) -> Option<Segment> {
    if f.kind != FlightKind::Stairs || f.shape != FlightShape::Spiral {
        return None;
    }
    let (uc, vc) = spiral_centre(rules, f);
    let end = rules.sp_offset() + rules.sp_span();
    let phi = spiral_angle(rules, f, end);
    let (cu, sv) = (phi.cos(), phi.sin());
    // along the radial from the post to the line u = 0
    // a (near) horizontal end radial never reaches the line u = 0: no rail rather than one thousands of cells long
    if cu.abs() < MIN_END_RADIAL_COS {
        return None;
    }
    let t = uc / -cu;
    let from = (uc + cu * (rules.sp_post), vc + sv * rules.sp_post);
    let to = (0.0, vc + sv * t);
    Some(Segment { a: to_world(f, from.0, from.1), b: to_world(f, to.0, to.1) })
}

pub fn spiral_slab_rail(f: &Flight) -> Option<Segment> {
    spiral_slab_rail_with(flights(), f)
}

/// A handrail: a polyline of points `(x, y, z)` with `y` measured from the LOWER floor to the rail (so the polyline is
/// already `rail_height` over the tread).
#[derive(Clone, Debug, PartialEq)]
pub struct Handrail {
    pub points: Vec<[f32; 3]>,
}

/// The handrails of the lower level's fit-out (Godot `_add_handrails_switchback` and the spiral's helix rail): a
/// switchback has one on each edge of each flight (four), a straight flight one on each side, a spiral one winding
/// round the post at `inner_radius + inner_offset`.
pub fn handrails_with(rules: &FlightRules, f: &Flight) -> Vec<Handrail> {
    if f.kind != FlightKind::Stairs {
        return vec![];
    }
    let d = &rules.dressing;
    let lo = f.lower_y();
    let at = |u: f32, v: f32, y: f32| {
        let (x, z) = to_world(f, u, v);
        [x, y - lo + d.rail_height, z]
    };
    let rise = rules.rise_total;
    match f.shape {
        FlightShape::Switchback => {
            let (lw, dw) = (rules.sb_lane_width as f32, rules.sb_divider_width as f32);
            let (mid, ramp) = (rules.sb_mid_rise, rules.sb_ramp());
            let top = |u: f32| lo + mid + (rise - mid) * (1.0 - u / ramp);
            let foot = |u: f32| lo + mid * u / ramp;
            let ins = d.rail_inset;
            let mut out = Vec::new();
            for v in [ins, lw - ins] {
                out.push(Handrail { points: vec![at(0.0, v, top(0.0)), at(ramp, v, top(ramp))] });
            }
            for v in [-dw - lw + ins, -dw - ins] {
                out.push(Handrail { points: vec![at(0.0, v, foot(0.0)), at(ramp, v, foot(ramp))] });
            }
            out
        }
        FlightShape::Straight => {
            let len = rules.st_length as f32;
            let w = rules.st_width as f32;
            let y = |u: f32| lo + rise * (1.0 - u / len);
            [d.rail_inset, w - d.rail_inset]
                .into_iter()
                .map(|v| Handrail { points: vec![at(0.0, v, y(0.0)), at(len, v, y(len))] })
                .collect()
        }
        FlightShape::Spiral => {
            let (span, off) = (rules.sp_span(), rules.sp_offset());
            let r = rules.sp_inner + d.rail_inner_offset;
            let n = rules.sp_steps;
            let points = (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    let (u, v) = spiral_point_at(rules, f, off + span * t, r);
                    at(u, v, lo + rise * (1.0 - t))
                })
                .collect();
            vec![Handrail { points }]
        }
    }
}

pub fn handrails(f: &Flight) -> Vec<Handrail> {
    handrails_with(flights(), f)
}

/// A "LEVEL n" sign on the well's low-`v` wall at the landing, facing into the well.
#[derive(Clone, Debug, PartialEq)]
pub struct Sign {
    /// "LEVEL +1", "LEVEL 0", "LEVEL -1".
    pub text: String,
    /// The point on the wall plane (grid coordinates, already `inset` out from the wall) at the landing row's middle.
    pub at: (f32, f32),
    /// The unit horizontal direction the sign faces (into the well).
    pub normal: (f32, f32),
    /// The unit horizontal direction the text runs, left to right for a reader facing the sign.
    pub right: (f32, f32),
    /// The centre of the sign over the floor of its level (metres).
    pub y: f32,
    /// The level the sign names, and whether it stands on the upper level (else the lower).
    pub level: i32,
    pub upper: bool,
}

/// "LEVEL +1" / "LEVEL 0" / "LEVEL -1": the sign's words for `level` (Godot's `"LEVEL %+d"`, zero unsigned).
pub fn sign_text(level: i32) -> String {
    if level == 0 {
        "LEVEL 0".to_string()
    } else {
        format!("LEVEL {level:+}")
    }
}

/// The two signs of a stairwell: the upper level's and the lower level's, on the wall beside the landing. A spiral has
/// none (KT: no "LEVEL n" lettering on the spiral staircases); the other flights keep theirs.
pub fn signs_with(rules: &FlightRules, f: &Flight) -> Vec<Sign> {
    if f.kind != FlightKind::Stairs || f.shape == FlightShape::Spiral {
        return vec![];
    }
    let d = &rules.dressing;
    let (u, v_wall) = match f.shape {
        // the door-adjacent row of the landing, and the interior's low-v wall
        FlightShape::Switchback => (-(rules.sb_landing_depth as f32) + 0.5, rules.sb_v_range().0),
        FlightShape::Straight => (-0.5, 0.0),
        FlightShape::Spiral => {
            let (_, vc) = spiral_centre(rules, f);
            (-0.5, vc - (rules.sp_half() as f32 + 0.5))
        }
    };
    let at = to_world(f, u, v_wall + d.sign_inset);
    let s = side(f.run);
    let normal = (s.0 as f32, s.1 as f32);
    // a reader faces the wall: forward is -normal, right = forward x up = (n.z, -n.x)
    let right = (normal.1, -normal.0);
    [(f.upper_level, true), (f.upper_level - 1, false)]
        .into_iter()
        .map(|(level, upper)| Sign { text: sign_text(level), at, normal, right, y: d.sign_y(), level, upper })
        .collect()
}

pub fn signs(f: &Flight) -> Vec<Sign> {
    signs_with(flights(), f)
}

/// The stairwell light: (x, y over the LOWER floor, z), over the middle of the well (Godot: the interior's centre, 1.4
/// Godot metres over mid storey).
pub fn lamp_place_with(rules: &FlightRules, f: &Flight) -> (f32, f32, f32) {
    let d = &rules.dressing;
    let (u, v) = match f.shape {
        FlightShape::Switchback => {
            // the interior: landing_depth rows behind the hole's first row, then the hole
            let depth = rules.sb_landing_depth as f32 + rules.sb_length as f32;
            let (v0, v1) = rules.sb_v_range();
            (-(rules.sb_landing_depth as f32) + depth * 0.5, (v0 + v1) * 0.5)
        }
        FlightShape::Straight => (rules.st_length as f32 * 0.5, rules.st_width as f32 * 0.5),
        FlightShape::Spiral => spiral_centre(rules, f),
    };
    let (x, z) = to_world(f, u, v);
    (x, d.lamp_y(), z)
}

pub fn lamp_place(f: &Flight) -> (f32, f32, f32) {
    lamp_place_with(flights(), f)
}

#[cfg(test)]
mod end_radial_tests {
    use super::*;

    #[test]
    fn a_horizontal_end_radial_is_rejected_and_gives_no_runaway_rail() {
        let mut bad = flights().clone();
        bad.sp_start_offset_deg = 0.0;
        bad.sp_turns = 0.75;
        assert!(bad.validate().unwrap_err().contains("slab rail"));
        bad.sp_turns = 0.25;
        assert!(bad.validate().unwrap_err().contains("slab rail"));
        for cw in [false, true] {
            let f = Flight::new(FlightKind::Stairs, FlightShape::Spiral, 1, Cell::new(20, 20), (0, 1)).with_clockwise(cw);
            assert!(spiral_slab_rail_with(&bad, &f).is_none(), "cw {cw}: no rail along a horizontal end radial");
            let ok = spiral_slab_rail_with(flights(), &f).expect("the shipped spiral keeps its rail");
            for p in [ok.a, ok.b] {
                assert!(p.0.abs() < 1000.0 && p.1.abs() < 1000.0, "cw {cw}: bounded rail");
            }
        }
    }
}

#[cfg(test)]
#[path = "flight_tests.rs"]
mod flight_tests;
