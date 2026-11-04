//! The one place that turns a connection into a [`Flight`] (docs/STAIRS_CONTINUOUS_DESIGN.md, phase 1).
//!
//! Both levels of a stairwell call the same pure function here, so the two footprints cannot disagree:
//!
//! - [`level0_well_flight`]: a maze (level 0) stairwell. Godot's layout box is kept as it is (the golden parity run
//!   compares only the stairwell boxes), so a `stairs` well is a switchback and a `spiral` well is a spiral, except
//!   that a spiral whose foot would land in rock or water on the sewer side falls back to a switchback.
//! - [`sewer_well_flights`]: the same flights seen from level -1, computed through [`level0_well_flight`].
//! - [`bay_flight`]: the stair bays of levels -1 -> -2 and -2 -> -3 (`infinite::stair_link`). The first candidate that
//!   fits wins, in a seeded fixed order, so the upper and the lower chunk always pick the same one.
//!
//! [`upper_stair`] and [`lower_stair`] make the `Stair` record each level stores from a flight. Every number about the
//! flight itself (shapes, lengths, weights) is data in `data/flights.json` (`world::flight`); the only constants here
//! are chunk geometry.

use super::facility::{Cell, Stair};
use super::flight::{shape_for, spiral_clockwise_for, well_footprint, Flight, FlightKind, FlightShape};
use super::infinite::{self, CHUNK};
use super::route::layout::Coord;
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

struct SewerInfo {
    grid: score::Grid,
    wells: Vec<WellInfo>,
}

type SewerKey = (String, i32, i32, i32);

/// What the flight choice needs of the sewer chunk at (cx, level, cy): its grid and its wells. Memoized (a sewer chunk
/// is costly to build), keyed (seed, cx, cy, level). `None` where the sewer has no chunk.
fn sewer_info(seed: &str, cx: i32, level: i32, cy: i32) -> Arc<Option<SewerInfo>> {
    static CACHE: OnceLock<Mutex<HashMap<SewerKey, Arc<Option<SewerInfo>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key: SewerKey = (seed.to_string(), cx, cy, level);
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

/// Every cell a flight needs open on one level or the other: the hole, the landing and foot, and both clear margins.
pub fn carve_cells(f: &Flight) -> Vec<Cell> {
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

// ---------------------------------------------------------------------------------------------------------------------
// Level 0 wells
// ---------------------------------------------------------------------------------------------------------------------

/// The flight of a maze stairwell. `door` is the layout door rectangle's (x, y, w) in chunk cells and `north` its side.
///
/// `stairs` wells are a switchback anchored on the landing cell just inside the doorway, running into the box (the old
/// layout's landing and run). `spiral` wells are a spiral anchored on the doorway cell, so its 5x5 hole fills the box
/// interior. The spiral's foot lies in the box wall column, so it is kept only when the sewer below is open there; else
/// the well is a switchback. Both levels call this with the same arguments, so they agree.
pub fn level0_well_flight(seed: &str, _cx: i32, _cy: i32, kind: &str, door: (i32, i32, i32), north: bool) -> Flight {
    if kind == "spiral" {
        let spiral = spiral_well_flight(seed, door, north);
        if spiral_carvable(&spiral) {
            return spiral;
        }
    }
    let outward = if north { 1 } else { -1 };
    let mid = door.0 + door.2 / 2;
    Flight::new(FlightKind::Stairs, FlightShape::Switchback, 0, Cell::new(mid, door.1 - outward), (0, -outward))
}

/// The spiral a `spiral` layout well asks for: anchored on the doorway cell, running into the box.
pub fn spiral_well_flight(seed: &str, door: (i32, i32, i32), north: bool) -> Flight {
    let outward = if north { 1 } else { -1 };
    let a = Cell::new(door.0 + door.2 / 2, door.1);
    Flight::new(FlightKind::Stairs, FlightShape::Spiral, 0, a, (0, -outward))
        .with_clockwise(spiral_clockwise_for(seed_key(seed), 0, a))
}

/// The cells the sewer below must hold open for a spiral well beyond its box: the foot B, its clear margin and the
/// cell past the margin (the foot has somewhere to go). The sewer chunk carves exactly these.
pub fn spiral_open_cells(f: &Flight) -> Vec<Cell> {
    let fp = well_footprint(f);
    let mut out: Vec<Cell> = fp.lower_clear.clone();
    out.push(fp.foot);
    let far = fp.lower_clear.iter().copied().max_by_key(|c| (c.x - fp.foot.x).abs() + (c.y - fp.foot.y).abs());
    if let Some(c) = far.filter(|c| *c != fp.foot) {
        out.push(Cell::new(c.x + (c.x - fp.foot.x).signum(), c.y + (c.y - fp.foot.y).signum()));
    }
    out.sort();
    out.dedup();
    out
}

/// A spiral well is possible unless its cells would leave the chunk interior (the only genuine impossibility: the
/// sewer makes room for everything else). Pure geometry, so both levels decide alike.
pub fn spiral_carvable(f: &Flight) -> bool {
    well_footprint(f).upper_hole.iter().chain(spiral_open_cells(f).iter()).all(|c| in_bounds(*c))
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
// Bay flights (levels -1 -> -2 and -2 -> -3)
// ---------------------------------------------------------------------------------------------------------------------

/// The flight of the stair link between `upper` and `upper - 1` in column (cx, cy), for `upper` in {-1, -2}. `None`
/// exactly when `infinite::stair_link` is `None` (and for any other `upper`). A pure function of (seed, cx, cy, upper).
pub fn bay_flight(seed: &str, cx: i32, cy: i32, upper: i32) -> Option<Flight> {
    static CACHE: OnceLock<Mutex<HashMap<SewerKey, Option<Flight>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key: SewerKey = (seed.to_string(), cx, cy, upper);
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
    let cells = carve_cells(f);
    if !cells.iter().all(|c| in_bounds(*c)) {
        return false;
    }
    let fp = well_footprint(f);
    let mut wet_check: Vec<Cell> = fp.upper_hole.clone();
    wet_check.push(fp.landing);
    wet_check.push(fp.foot);
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
    fn every_layout_spiral_well_is_a_spiral_and_the_sewer_makes_room_for_it() {
        let (mut spirals, mut fallbacks) = (0, 0);
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
                        if f.shape != FlightShape::Spiral {
                            fallbacks += 1;
                            let s = spiral_well_flight(&seed, door, w.north);
                            assert!(spiral_open_cells(&s).iter().any(|c| !in_bounds(*c)), "{seed} {cx},{cy}: a spiral that fits the chunk fell back");
                            continue;
                        }
                        assert!(flights.contains(&f), "{seed} {cx},{cy}: the sewer lacks the spiral");
                        assert_eq!(upper_stair(&f).flight(0), lower_stair(&f).flight(-1));
                        let fp = well_footprint(&f);
                        let g = |c: Cell| info.grid[c.y as usize][c.x as usize];
                        let open = |c: Cell| !matches!(g(c), score::SOLID | score::WATER | score::FALL);
                        for c in fp.upper_hole.iter().chain(spiral_open_cells(&f).iter()) {
                            assert!(open(*c), "{seed} {cx},{cy}: {c:?} is not open in the sewer");
                        }
                        // reachability from the foot over open cells to a cell outside the footprint
                        let own: BTreeSet<Cell> = fp.upper_hole.iter().chain(spiral_open_cells(&f).iter()).copied().collect();
                        let mut seen: BTreeSet<Cell> = BTreeSet::new();
                        let mut q = vec![fp.foot];
                        seen.insert(fp.foot);
                        while let Some(c) = q.pop() {
                            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                                let n = Cell::new(c.x + dx, c.y + dy);
                                if in_bounds(n) && open(n) && seen.insert(n) {
                                    q.push(n);
                                }
                            }
                        }
                        assert!(seen.iter().any(|c| !own.contains(c)), "{seed} {cx},{cy}: the foot {:?} is sealed in", fp.foot);
                        assert!(seen.len() > 60, "{seed} {cx},{cy}: the foot reaches only {} cells", seen.len());
                    }
                }
            }
        }
        assert!(spirals > 20, "only {spirals} spiral wells in the sweep");
        // the only fallback is the chunk edge: a corner box whose foot margin would leave the interior (asserted per well above)
        assert!(spirals - fallbacks > 10, "{fallbacks} of {spirals} spiral wells fell back");
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
        // a stairs well: landing just inside the doorway, running into the box, the old layout's cells
        for north in [true, false] {
            let outward = if north { 1 } else { -1 };
            let f = level0_well_flight("1426541270", 0, 0, "stairs", (20, 30, 3), north);
            assert_eq!(f.shape, FlightShape::Switchback);
            assert_eq!(f.anchor_cell, Cell::new(21, 30 - outward));
            assert_eq!(f.run, (0, -outward));
            assert_eq!(f.upper_level, 0);
            assert_eq!(well_footprint(&f).upper_hole.len(), 8, "the 2x4 hole");
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
