//! The one place that turns a connection into a [`Flight`] (docs/STAIRS_CONTINUOUS_DESIGN.md, phase 1).
//!
//! Both levels of a stairwell call the same pure function here, so the two footprints cannot disagree:
//!
//! - [`level0_well_flight`]: a maze (level 0) stairwell. Godot's layout box is kept as it is (the golden parity run
//!   compares only the stairwell boxes), so a `stairs` well is Godot's switchback (the 5 x 7 interior: two landing rows,
//!   a 5 x 5 well) and a `spiral` well is Godot's spiral (landing and foot are the two end cells of the landing row just
//!   inside the doorway, the 5 x 5 interior round the post), with no fallback and no rock carved.
//! - [`sewer_well_flights`]: the same flights seen from level -1, computed through [`level0_well_flight`].
//! - [`bay_flight`]: the stair bays of levels -1 -> -2 and -2 -> -3 (`infinite::stair_link`). The first candidate that
//!   fits wins, in a seeded fixed order, so the upper and the lower chunk always pick the same one.
//!
//! [`upper_stair`] and [`lower_stair`] make the `Stair` record each level stores from a flight. Every number about the
//! flight itself (shapes, lengths, weights) is data in `data/flights.json` (`world::flight`); the only constants here
//! are chunk geometry.

use super::facility::{Cell, Stair};
use super::flight::{ladder_dir_order, shape_for, spiral_clockwise_for, well_footprint, Flight, FlightKind, FlightShape};
use super::infinite::{self, CHUNK};
use super::route::layout::{self, Coord};
use super::route::sewer::{self, core as score};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The `Stair` the UPPER level stores: dir -1, the cell is the landing A, `run` is the flight's own run.
pub fn upper_stair(f: &Flight) -> Stair {
    Stair::stairs(well_footprint(f).landing, -1, f.run, f.shape, f.clockwise)
}

/// The `Stair` the LOWER level stores: dir +1, the cell is the foot B, `run` is the flight's own run.
pub fn lower_stair(f: &Flight) -> Stair {
    Stair::stairs(well_footprint(f).foot, 1, f.run, f.shape, f.clockwise)
}

/// The world seed text as the number `shape_for` keys on: the decimal seed itself, or a hash of any other text.
pub(crate) fn seed_key(seed: &str) -> u64 {
    seed.parse::<u64>().unwrap_or_else(|_| {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in seed.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// The sewer chunks the choice looks at, memoized
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Clone)]
struct WellInfo {
    kind: &'static str,
    door: (i32, i32, i32),
    north: bool,
}

/// A sewer ladder of the chunk: the maze hallway cell it hangs from, the sewer cell it ends at, and its cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LadderCells {
    pub from: Cell,
    pub to: Cell,
    pub grate: bool,
}

struct SewerInfo {
    grid: score::Grid,
    wells: Vec<WellInfo>,
    ladders: Vec<LadderCells>,
}

type SewerKey = (String, i32, i32, i32);

/// The seed as a cache key, tagged with the level-0 region scope (`layout::with_godot_regions`): a chunk laid out from
/// Godot's region draw and one laid out from the concept map differ, so a parity test must not leave its chunks in
/// the cache for a test of the played layout (or the other way round).
fn scoped(seed: &str) -> String {
    format!("{seed}\u{0}{}", layout::godot_regions())
}

/// What the flight choice needs of the sewer chunk at (cx, level, cy): its grid and its wells. Memoized (a sewer chunk
/// is costly to build), keyed (seed, cx, cy, level). `None` where the sewer has no chunk.
fn sewer_info(seed: &str, cx: i32, level: i32, cy: i32) -> Arc<Option<SewerInfo>> {
    static CACHE: OnceLock<Mutex<HashMap<SewerKey, Arc<Option<SewerInfo>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key: SewerKey = (scoped(seed), cx, cy, level);
    if let Some(v) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return v.clone();
    }
    // built outside the lock: a duplicate build in a parallel test is harmless
    let built = sewer::chunk(Coord::new(cx, level, cy), &sewer::Inputs { world: seed }).map(|sc| SewerInfo {
        grid: sc.sewer_grid,
        wells: sc
            .wells
            .iter()
            .map(|w| WellInfo { kind: w.kind, door: (w.door.x, w.door.y, w.door.w), north: w.north })
            .collect(),
        ladders: sc
            .links
            .iter()
            .map(|l| LadderCells {
                from: Cell::new(l.from_cell.x, l.from_cell.y),
                to: Cell::new(l.to_cell.x, l.to_cell.y),
                grate: l.cover == "grate",
            })
            .collect(),
    });
    let v = Arc::new(built);
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() > 2048 {
        g.clear();
    }
    g.insert(key, v.clone());
    v
}

fn in_bounds(c: Cell) -> bool {
    c.x >= 1 && c.y >= 1 && c.x <= CHUNK - 2 && c.y <= CHUNK - 2
}

/// The cells a flight physically needs open on one level or the other: the hole, the landing and foot, and both clear
/// margins — but NOT the widened spiral apron (`vertical::spiral_keepout`). This is the FIT-and-EXCLUSION footprint:
/// whether a bay actually fits the chunk ([`bay_fits`]) and whether a sewer room must keep off it
/// (`route::sewer::core::stair_bay_cells`) both read this, never the apron. The apron is a decorative keep-out for
/// props, hiding, fixtures and vents only (see [`carve_cells`]); folding it into the fit would reject a spiral whose
/// core fits but whose apron pokes past the chunk border, and over-exclude sewer rooms.
pub fn carve_cells_core(f: &Flight) -> Vec<Cell> {
    let fp = well_footprint(f);
    let mut out: Vec<Cell> = Vec::new();
    out.extend(fp.upper_hole.iter().copied());
    out.extend(fp.upper_clear.iter().copied());
    out.extend(fp.lower_clear.iter().copied());
    out.push(fp.landing);
    out.push(fp.foot);
    out.sort();
    out.dedup();
    out
}

/// Every cell a flight keeps clear on one level or the other: the physical core ([`carve_cells_core`]) plus, for a spiral,
/// the widened decorative zone round both ends (props, hiding, fixtures, vents). Use [`carve_cells_core`] for any physical
/// fit or room-exclusion test; this wider set is for decoration and prop-clearing only.
pub fn carve_cells(f: &Flight) -> Vec<Cell> {
    let mut out = carve_cells_core(f);
    if f.shape == FlightShape::Spiral {
        // the widened zone round both ends; none in the Godot parity scope (vertical::spiral_keepout)
        let fp = well_footprint(f);
        out.extend(crate::world::vertical::spiral_keepout(&fp.upper_hole, fp.landing, fp.foot));
        out.sort();
        out.dedup();
    }
    out
}

/// The bay of a flight as a rectangle `(x, y, w, h)` of chunk cells: the bounding box of its carve cells grown by one cell
/// (the ring round the well and the landing) and kept inside the chunk border. The one box every level of the flight clears
/// of props (`infinite::carve_bay`) and the Inferno keeps free of lava and cave decoration (`inferno::InfernoChunk`).
pub fn bay_rect(f: &Flight) -> (i32, i32, i32, i32) {
    let cells = carve_cells(f);
    let x0 = (cells.iter().map(|c| c.x).min().unwrap_or(1) - 1).max(1);
    let x1 = (cells.iter().map(|c| c.x).max().unwrap_or(1) + 1).min(CHUNK - 2);
    let y0 = (cells.iter().map(|c| c.y).min().unwrap_or(1) - 1).max(1);
    let y1 = (cells.iter().map(|c| c.y).max().unwrap_or(1) + 1).min(CHUNK - 2);
    (x0, y0, x1 - x0 + 1, y1 - y0 + 1)
}

// ---------------------------------------------------------------------------------------------------------------------
// Level 0 wells
// ---------------------------------------------------------------------------------------------------------------------

/// The flight of a maze stairwell. `door` is the layout door rectangle's (x, y, w) in chunk cells and `north` its side.
///
/// `stairs` wells are Godot's switchback (stairwells.gd:66-80, the 7 x 9 box): the landing is the two rows at the doorway
/// end, the arriving flight is the two columns on the +x side (columns 3 and 4 of the 5-wide interior), the divider the
/// column beside them and the leaving flight the two beyond, so the landing A is the first cell of the arriving flight,
/// in the row just inside the landing's door row (the row behind it, at the door, is its margin). `spiral` wells are a
/// spiral (stairwells.gd:116-134) whose landing A and foot B are the two end cells of the three-cell landing row (the
/// row just inside the doorway), its 5x5 well filling the box interior round the post, both ends open onto the landing
/// row and the corridor that already leaves the doorway on both levels. Every layout spiral therefore builds as a spiral;
/// no rock is carved for it and nothing falls back. Both levels call this with the same arguments, so they agree.
pub fn level0_well_flight(seed: &str, _cx: i32, _cy: i32, kind: &str, door: (i32, i32, i32), north: bool) -> Flight {
    if kind == "spiral" {
        return spiral_well_flight(seed, door, north);
    }
    let outward = if north { 1 } else { -1 };
    let mid = door.0 + door.2 / 2;
    Flight::new(FlightKind::Stairs, FlightShape::Switchback, 0, Cell::new(mid + 1, door.1 - 2 * outward), (0, -outward))
}

/// The spiral a `spiral` layout well asks for: the landing A is the landing-row cell the arc starts at, the foot B the cell
/// it ends at (`B = A + side * 2 * dir`), running into the box. The turn is Godot's: its helicoid is cut the same way round
/// in both box orientations (a rotation of the north layout is the south one), which in the one local frame the flight
/// uses, whose across axis is +x for both, is a mirror image: counter-clockwise (`clockwise` false) for a north well,
/// clockwise for a south one. The seed does not choose it.
pub fn spiral_well_flight(_seed: &str, door: (i32, i32, i32), north: bool) -> Flight {
    let outward = if north { 1 } else { -1 };
    let middle = Cell::new(door.0 + door.2 / 2, door.1 - outward);
    let clockwise = !north;
    let dir = if clockwise { -1 } else { 1 };
    Flight::new(FlightKind::Stairs, FlightShape::Spiral, 0, Cell::new(middle.x - dir, middle.y), (0, -outward))
        .with_clockwise(clockwise)
}

/// The flights of the level-0 wells of column (cx, cy), seen from the sewer (`k.level == -1`; empty otherwise). Each is
/// `level0_well_flight` of the sewer well's own kind, door and side, so both levels get the identical `Flight`.
pub fn sewer_well_flights(seed: &str, k: infinite::ChunkKey) -> Vec<Flight> {
    if k.level != score::LEVEL {
        return vec![];
    }
    let info = sewer_info(seed, k.cx, score::LEVEL, k.cy);
    let Some(info) = info.as_ref() else {
        return vec![];
    };
    info.wells.iter().map(|w| level0_well_flight(seed, k.cx, k.cy, w.kind, w.door, w.north)).collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// Sewer ladders (level 0 hatch over a level -1 foot)
// ---------------------------------------------------------------------------------------------------------------------

/// The sewer ladders of column (cx, cy): the cells the ported sewer generator gives them (memoized with the chunk).
pub fn sewer_ladder_cells(seed: &str, cx: i32, cy: i32) -> Vec<LadderCells> {
    sewer_info(seed, cx, score::LEVEL, cy).as_ref().as_ref().map(|i| i.ladders.clone()).unwrap_or_default()
}

/// The cells of the level -1 chunk of column (cx, cy) that are a waterfall void (`FALL`): the drop cells of level -1
/// and, in the same x/z, the air above the landing pool on level -2. Empty where the chunk has no falls.
pub fn fall_cells(seed: &str, cx: i32, cy: i32) -> Vec<Cell> {
    let info = sewer_info(seed, cx, score::LEVEL, cy);
    let Some(info) = info.as_ref() else { return vec![] };
    let mut out = vec![];
    for (y, row) in info.grid.iter().enumerate() {
        for (x, code) in row.iter().enumerate() {
            if *code == score::FALL {
                out.push(Cell::new(x as i32, y as i32));
            }
        }
    }
    out
}

/// The maze (level 0) side of a column as the ladder choice needs it: which cells the layout leaves open, and the
/// flights of its stairwells. Memoized; `None` where the layout does not lay the chunk out.
struct MazeInfo {
    open: Vec<bool>,
    flights: Vec<Flight>,
}

fn maze_info(seed: &str, cx: i32, cy: i32) -> Arc<Option<MazeInfo>> {
    static CACHE: OnceLock<Mutex<HashMap<SewerKey, Arc<Option<MazeInfo>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key: SewerKey = (scoped(seed), cx, cy, 0);
    if let Some(v) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return v.clone();
    }
    let built = layout::chunk_layout(seed, Coord::new(cx, 0, cy)).map(|lay| MazeInfo {
        open: (0..CHUNK as usize).flat_map(|z| (0..CHUNK as usize).map(move |x| (x, z))).map(|(x, z)| lay.grid[z][x] == 0).collect(),
        flights: lay
            .stairwells
            .iter()
            .map(|w| level0_well_flight(seed, cx, cy, w.kind, (w.door.x, w.door.y, w.door.w), w.north))
            .collect(),
    });
    let v = Arc::new(built);
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() > 2048 {
        g.clear();
    }
    g.insert(key, v.clone());
    v
}

/// What `ladder_choice` decided for one ladder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderChoice {
    pub flight: Flight,
    /// The step cell is a wall in the maze layout: the level 0 builder must open it (`Facility::open_cells`).
    pub carve_maze: bool,
}

/// Whether a sewer cell can be stood on beside a ladder's foot: not rock, water, a fall void, a stair shaft or a ramp.
fn sewer_standable(code: u8) -> bool {
    !matches!(code, score::SOLID | score::WATER | score::FALL | score::SHAFT | score::RAMP)
}

/// Picks the step cell of the ladder whose hatch is `cell` in column (cx, cy), and so the one `Flight` both levels
/// build it from. The four directions are tried in a seeded rotation (`flight::ladder_dir_order`); the step cell must
/// lie inside the chunk border, outside every other flight's carve cells and other ladder's hatch, and be a standable
/// (dry) cell of the sewer. A step cell the maze layout leaves open wins; failing that the first such direction is
/// taken and the maze carves it (`carve_maze`). `None` when the foot is wet or rock or no direction has a dry sewer
/// cell: the link is dropped on both levels (`level_links::ladders`). A pure function of (seed, cx, cy, cell).
pub fn ladder_choice(seed: &str, cx: i32, cy: i32, cell: Cell) -> Option<LadderChoice> {
    let sewer = sewer_info(seed, cx, score::LEVEL, cy);
    let sewer = sewer.as_ref().as_ref()?;
    let at = |c: Cell| sewer.grid[c.y as usize][c.x as usize];
    let inside = |c: Cell| c.x >= 0 && c.y >= 0 && c.x < CHUNK && c.y < CHUNK;
    if !inside(cell) || matches!(at(cell), score::SOLID | score::WATER | score::FALL) {
        return None;
    }
    let maze = maze_info(seed, cx, cy);
    let mut taken: Vec<Cell> = sewer.ladders.iter().flat_map(|l| [l.from, l.to]).filter(|c| *c != cell).collect();
    if let Some(m) = maze.as_ref() {
        for f in &m.flights {
            taken.extend(carve_cells(f));
        }
    }
    if let Some(f) = bay_flight(seed, cx, cy, score::LEVEL) {
        taken.extend(carve_cells(&f));
    }
    // the hatch and foot cell itself must not sit in another flight's carve cells either (a bay well's hole is not a floor)
    if taken.contains(&cell) {
        return None;
    }
    let candidates: Vec<(Cell, (i32, i32))> = ladder_dir_order(seed_key(seed), (cx, cy), cell)
        .into_iter()
        .map(|d| (Cell::new(cell.x + d.0, cell.y + d.1), d))
        .filter(|(s, _)| in_bounds(*s) && !taken.contains(s) && sewer_standable(at(*s)))
        .collect();
    let maze_open = |c: Cell| maze.as_ref().as_ref().is_some_and(|m| m.open[(c.y * CHUNK + c.x) as usize]);
    let (step, carve) = match candidates.iter().find(|(s, _)| maze_open(*s)) {
        Some(c) => (*c, false),
        None => (*candidates.first()?, true),
    };
    let flight = Flight::new(FlightKind::Ladder, FlightShape::Switchback, 0, cell, step.1);
    Some(LadderChoice { flight, carve_maze: carve })
}

/// The ladder `Flight` of the hatch `cell` of column (cx, cy): the single producer both levels read, so their step
/// cells cannot disagree. A hatch with no usable step cell (`ladder_choice` is `None`) gets the default northward run;
/// `level_links::ladders` drops such a link on both levels, so no record is ever built from it.
pub fn ladder_flight(seed: &str, cx: i32, cy: i32, cell: Cell) -> Flight {
    ladder_choice(seed, cx, cy, cell)
        .map(|c| c.flight)
        .unwrap_or_else(|| Flight::new(FlightKind::Ladder, FlightShape::Switchback, 0, cell, (0, -1)))
}

// ---------------------------------------------------------------------------------------------------------------------
// Bay flights (levels -1 -> -2 and -2 -> -3)
// ---------------------------------------------------------------------------------------------------------------------

/// The flight of the stair link between `upper` and `upper - 1` in column (cx, cy), for `upper` in {-1, -2}. `None`
/// exactly when `infinite::stair_link` is `None` (and for any other `upper`). A pure function of (seed, cx, cy, upper).
pub fn bay_flight(seed: &str, cx: i32, cy: i32, upper: i32) -> Option<Flight> {
    static CACHE: OnceLock<Mutex<HashMap<SewerKey, Option<Flight>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key: SewerKey = (scoped(seed), cx, cy, upper);
    if let Some(v) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return *v;
    }
    let v = compute_bay_flight(seed, cx, cy, upper);
    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
    if g.len() > 8192 {
        g.clear();
    }
    g.insert(key, v);
    v
}

/// `bay_flight` without the memo. Candidates in a fixed order: for each anchor (the link's own anchor, the other three of
/// its bay, then the four of the other bay of the same parity), the seeded shape first and then a switchback. The first
/// that fits is taken; if none does, the switchback at the link's own anchor.
pub(crate) fn compute_bay_flight(seed: &str, cx: i32, cy: i32, upper: i32) -> Option<Flight> {
    if upper != sewer::core::LEVEL && upper != sewer::core::LOWER_LEVEL {
        return None;
    }
    let candidates = infinite::bay_candidates(seed, cx, cy, upper);
    let (first_anchor, first_run) = *candidates.first()?;
    let key = seed_key(seed);
    let make = |anchor: (i32, i32), run: (i32, i32), shape: FlightShape| {
        let cell = Cell::new(anchor.0, anchor.1);
        let f = Flight::new(FlightKind::Stairs, shape, upper, cell, run);
        if shape == FlightShape::Spiral {
            f.with_clockwise(spiral_clockwise_for(key, upper, cell))
        } else {
            f
        }
    };
    for (anchor, run) in &candidates {
        let seeded = shape_for(key, upper, Cell::new(anchor.0, anchor.1));
        let mut shapes = vec![seeded];
        if seeded != FlightShape::Switchback {
            shapes.push(FlightShape::Switchback);
        }
        for shape in shapes {
            let f = make(*anchor, *run, shape);
            if bay_fits(seed, cx, cy, &f) {
                return Some(f);
            }
        }
    }
    Some(make(first_anchor, first_run, FlightShape::Switchback))
}

/// A bay candidate fits when every carve cell is inside the chunk (1..=CHUNK-2) and the hole, landing and foot hold no
/// water on the sewer levels it touches.
fn bay_fits(seed: &str, cx: i32, cy: i32, f: &Flight) -> bool {
    // the physical core only: a spiral's wider decorative apron is not a fit constraint (it may poke past the border
    // while the core sits inside), so it must not reject an otherwise-fitting bay. See `carve_cells_core`.
    let cells = carve_cells_core(f);
    if !cells.iter().all(|c| in_bounds(*c)) {
        return false;
    }
    let fp = well_footprint(f);
    let mut wet_check: Vec<Cell> = fp.upper_hole.clone();
    wet_check.push(fp.landing);
    wet_check.push(fp.foot);
    // a waterfall void (FALL on level -1, air above the pool on -2) is no floor to land, stand or margin on: no carve
    // cell of the flight may touch one, on either of the two levels
    if let Some(info) = sewer_info(seed, cx, score::LEVEL, cy).as_ref() {
        if cells.iter().any(|c| info.grid[c.y as usize][c.x as usize] == score::FALL) {
            return false;
        }
    }
    for level in [f.upper_level, f.upper_level - 1] {
        if level != score::LEVEL && level != score::LOWER_LEVEL {
            continue;
        }
        let info = sewer_info(seed, cx, level, cy);
        if let Some(info) = info.as_ref() {
            if wet_check.iter().any(|c| info.grid[c.y as usize][c.x as usize] == score::WATER) {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::infinite::ChunkKey;
    use std::collections::BTreeSet;

    const SEEDS: [&str; 12] = [
        "1426541270", "4157433391", "2899547009", "40490470", "7", "12345", "alpha", "beta", "gamma", "123456789",
        "987654321", "zz-top",
    ];

    fn bay_sweep() -> Vec<(&'static str, i32, i32, i32, Flight)> {
        let mut out = vec![];
        for seed in SEEDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    for upper in [-1, -2] {
                        if let Some(f) = bay_flight(seed, cx, cy, upper) {
                            out.push((seed, cx, cy, upper, f));
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_bay_flight_exists_exactly_where_the_stair_link_does() {
        for seed in SEEDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    for upper in [1, 0, -1, -2, -3] {
                        let want = infinite::stair_link(seed, cx, cy, upper).is_some() && (upper == -1 || upper == -2);
                        assert_eq!(bay_flight(seed, cx, cy, upper).is_some(), want, "{seed} {cx},{cy} upper {upper}");
                    }
                }
            }
        }
    }

    #[test]
    fn bay_and_well_flights_are_deterministic() {
        for seed in SEEDS {
            for cx in -1..=1 {
                for cy in -1..=1 {
                    for upper in [-1, -2] {
                        assert_eq!(
                            compute_bay_flight(seed, cx, cy, upper),
                            compute_bay_flight(seed, cx, cy, upper),
                            "{seed} {cx},{cy} upper {upper}"
                        );
                    }
                    for kind in ["stairs", "spiral"] {
                        let a = level0_well_flight(seed, cx, cy, kind, (20, 30, 3), true);
                        assert_eq!(a, level0_well_flight(seed, cx, cy, kind, (20, 30, 3), true), "{seed} {kind}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_upper_and_lower_stairs_of_a_bay_flight_agree() {
        let sweep = bay_sweep();
        assert!(sweep.len() > 20, "only {} bay flights in the sweep", sweep.len());
        for (seed, cx, cy, upper, f) in sweep {
            let (u, l) = (upper_stair(&f), lower_stair(&f));
            assert!(u.dir < 0 && l.dir > 0);
            assert_eq!(u.flight(upper), Some(f), "{seed} {cx},{cy} upper {upper}: the upper stair loses the flight");
            assert_eq!(l.flight(upper - 1), Some(f), "{seed} {cx},{cy} upper {upper}: the lower stair loses the flight");
            assert_eq!(u.flight(upper), l.flight(upper - 1), "{seed} {cx},{cy} upper {upper}");
        }
    }

    #[test]
    fn the_sewer_sees_the_same_flights_as_the_maze_wells() {
        let mut wells = 0;
        for seed in SEEDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    let from_maze: Vec<Flight> = crate::world::route::live::layout_stairwells(seed, ChunkKey { cx, cy, level: 0 })
                        .iter()
                        .map(|w| level0_well_flight(seed, cx, cy, w.kind, (w.door.x, w.door.y, w.door.w), w.north))
                        .collect();
                    let from_sewer = sewer_well_flights(seed, ChunkKey { cx, cy, level: -1 });
                    for f in &from_maze {
                        assert!(from_sewer.contains(f), "{seed} {cx},{cy}: the sewer lacks the maze flight {f:?}");
                    }
                    for f in &from_sewer {
                        assert!(from_maze.contains(f), "{seed} {cx},{cy}: the maze lacks the sewer flight {f:?}");
                    }
                    for f in from_maze {
                        wells += 1;
                        assert_eq!(upper_stair(&f).flight(0), lower_stair(&f).flight(-1));
                    }
                    assert!(sewer_well_flights(seed, ChunkKey { cx, cy, level: 0 }).is_empty());
                    assert!(sewer_well_flights(seed, ChunkKey { cx, cy, level: -2 }).is_empty());
                }
            }
        }
        assert!(wells > 0, "no level-0 well in the sweep");
    }


    #[test]
    fn every_layout_spiral_well_is_a_spiral_anchored_on_the_doorway() {
        crate::world::route::layout::with_godot_regions(|| {
        let mut spirals = 0;
        for n in 0..60u32 {
            let seed = format!("{}", 1000 + n * 7919);
            for cx in -1..=1 {
                for cy in -1..=1 {
                    let k = ChunkKey { cx, cy, level: -1 };
                    let wells = crate::world::route::live::layout_stairwells(&seed, ChunkKey { cx, cy, level: 0 });
                    let info = sewer_info(&seed, cx, score::LEVEL, cy);
                    let Some(info) = info.as_ref() else { continue };
                    let flights = sewer_well_flights(&seed, k);
                    assert_eq!(flights.len(), wells.len());
                    for w in wells.iter().filter(|w| w.kind == "spiral") {
                        spirals += 1;
                        let door = (w.door.x, w.door.y, w.door.w);
                        let f = level0_well_flight(&seed, cx, cy, "spiral", door, w.north);
                        assert_eq!(f.shape, FlightShape::Spiral, "{seed} {cx},{cy}: a layout spiral never falls back");
                        assert!(flights.contains(&f), "{seed} {cx},{cy}: the sewer lacks the spiral");
                        assert_eq!(upper_stair(&f).flight(0), lower_stair(&f).flight(-1));
                        let fp = well_footprint(&f);
                        // the landing and the foot are the two end cells of the landing row (the row just inside the doorway),
                        // two cells apart
                        let outward = if w.north { 1 } else { -1 };
                        let landing_row: Vec<Cell> = (door.0..door.0 + door.2).map(|x| Cell::new(x, door.1 - outward)).collect();
                        assert!(landing_row.contains(&fp.landing) && landing_row.contains(&fp.foot), "{seed} {cx},{cy}: A and B are landing row cells");
                        assert_eq!((fp.foot.x - fp.landing.x).abs(), 2);
                        assert_eq!(fp.foot.y, fp.landing.y);
                        // Godot's turn: counter-clockwise for a north well, clockwise for a south one, whatever the seed
                        assert_eq!(f.clockwise, !w.north, "{seed} {cx},{cy}");
                        // the doorway row stays open
                        let doorway: Vec<Cell> = (door.0..door.0 + door.2).map(|x| Cell::new(x, door.1)).collect();
                        // the well is the box interior less the landing row (floor on both levels), and the sewer already
                        // holds the doorway row open
                        let interior: BTreeSet<Cell> = (w.bbox.x + 1..w.bbox.x + w.bbox.w - 1)
                            .flat_map(|x| (w.bbox.y + 1..w.bbox.y + w.bbox.h - 1).map(move |y| Cell::new(x, y)))
                            .filter(|c| !landing_row.contains(c))
                            .collect();
                        assert_eq!(fp.upper_hole.iter().copied().collect::<BTreeSet<_>>(), interior, "{seed} {cx},{cy}: the hole is the 5x5 interior less the landing row");
                        let g = |c: Cell| info.grid[c.y as usize][c.x as usize];
                        let open = |c: Cell| !matches!(g(c), score::SOLID | score::WATER | score::FALL);
                        for c in doorway.iter().chain(landing_row.iter()).chain(fp.lower_clear.iter()) {
                            assert!(open(*c), "{seed} {cx},{cy}: {c:?} is not open in the sewer");
                        }
                        for c in fp.upper_hole.iter() {
                            assert!(!matches!(g(*c), score::SOLID | score::WATER), "{seed} {cx},{cy}: well cell {c:?} is rock or water");
                        }
                        // reachability from the foot over open cells, never through the well, to a cell far from the box
                        let mut seen: BTreeSet<Cell> = BTreeSet::new();
                        let mut q = vec![fp.foot];
                        seen.insert(fp.foot);
                        while let Some(c) = q.pop() {
                            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                                let n = Cell::new(c.x + dx, c.y + dy);
                                if in_bounds(n) && open(n) && !fp.upper_hole.contains(&n) && seen.insert(n) {
                                    q.push(n);
                                }
                            }
                        }
                        assert!(seen.len() > 60, "{seed} {cx},{cy}: the foot {:?} reaches only {} cells", fp.foot, seen.len());
                    }
                }
            }
        }
        assert!(spirals > 20, "only {spirals} spiral wells in the sweep");
            });
    }

    #[test]
    fn all_three_shapes_occur_among_the_bay_flights() {
        let shapes: BTreeSet<String> = bay_sweep().iter().map(|(_, _, _, _, f)| format!("{:?}", f.shape)).collect();
        for want in ["Straight", "Spiral", "Switchback"] {
            assert!(shapes.contains(want), "no {want} bay flight in the sweep: {shapes:?}");
        }
    }

    #[test]
    fn a_well_flight_keeps_the_layout_landing_and_run() {
        // a stairs well: landing in the second row inside the doorway, on the first column of the arriving flight (one right
        // of the doorway's middle), running into the box, Godot's 5 x 7 interior
        for north in [true, false] {
            let outward = if north { 1 } else { -1 };
            let f = level0_well_flight("1426541270", 0, 0, "stairs", (20, 30, 3), north);
            assert_eq!(f.shape, FlightShape::Switchback);
            assert_eq!(f.anchor_cell, Cell::new(22, 30 - 2 * outward));
            assert_eq!(f.run, (0, -outward));
            assert_eq!(f.upper_level, 0);
            assert_eq!(well_footprint(&f).upper_hole.len(), 25, "the 5 x 5 well beyond the two landing rows");
        }
    }

    #[test]
    fn no_carve_cell_is_outside_the_chunk_interior() {
        for (seed, cx, cy, upper, f) in bay_sweep() {
            for c in carve_cells(&f) {
                assert!(in_bounds(c), "{seed} {cx},{cy} upper {upper}: {:?} {c:?} is outside 1..=CHUNK-2", f.shape);
            }
        }
        for seed in SEEDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    for f in sewer_well_flights(seed, ChunkKey { cx, cy, level: -1 }) {
                        for c in carve_cells(&f) {
                            assert!(in_bounds(c), "{seed} {cx},{cy}: well {:?} {c:?} is outside 1..=CHUNK-2", f.shape);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_spiral_bay_fitting_its_core_is_not_rejected_by_the_wider_apron() {
        // a909987 folded the decorative spiral apron (vertical::spiral_keepout) into carve_cells, which also drove the
        // physical fit test (bay_fits) and the sewer room exclusion. A spiral whose core (hole, both ends, their margins)
        // sits inside the chunk but whose wider apron pokes past the border must still FIT: the fit test reads
        // carve_cells_core, decoration reads the widened carve_cells.
        let mut found = 0;
        for run in [(0, -1), (0, 1), (1, 0), (-1, 0)] {
            for x in 1..CHUNK - 1 {
                for y in 1..CHUNK - 1 {
                    let f = Flight::new(FlightKind::Stairs, FlightShape::Spiral, -1, Cell::new(x, y), run).with_clockwise(false);
                    let core = carve_cells_core(&f);
                    let wide = carve_cells(&f);
                    // the core is always a subset of the widened set (the apron only adds cells)
                    assert!(core.iter().all(|c| wide.contains(c)), "core {:?} not within widened at {x},{y} {run:?}", f.shape);
                    let core_fits = core.iter().all(|c| in_bounds(*c));
                    let wide_fits = wide.iter().all(|c| in_bounds(*c));
                    if core_fits && !wide_fits {
                        // exactly the regression case: the core fits, the apron pokes out. The fit gate is over the core,
                        // so such a bay is accepted (the old widened gate would have rejected it and fallen back to a switchback).
                        found += 1;
                        assert!(wide.len() > core.len(), "the apron must add cells beyond the core at {x},{y} {run:?}");
                    }
                }
            }
        }
        assert!(found > 0, "no spiral with a fitting core but an out-of-bounds apron was found near the chunk edge");
    }

    #[test]
    fn a_bay_flight_holds_no_water_on_the_sewer_levels() {
        for (seed, cx, cy, upper, f) in bay_sweep() {
            let fp = well_footprint(&f);
            for level in [upper, upper - 1] {
                if level != score::LEVEL && level != score::LOWER_LEVEL {
                    continue;
                }
                let info = sewer_info(seed, cx, level, cy);
                let Some(info) = info.as_ref() else { continue };
                let cells = fp.upper_hole.iter().chain([&fp.landing, &fp.foot]);
                for c in cells {
                    // a flight that had no dry candidate falls back to the link's own anchor; those are allowed
                    if info.grid[c.y as usize][c.x as usize] == score::WATER {
                        let own = infinite::bay_candidates(seed, cx, cy, upper)[0];
                        assert_eq!(
                            (f.anchor_cell, f.shape),
                            (Cell::new((own.0).0, (own.0).1), FlightShape::Switchback),
                            "{seed} {cx},{cy} upper {upper}: water at {c:?} but a different candidate was taken"
                        );
                    }
                }
            }
        }
    }
}
