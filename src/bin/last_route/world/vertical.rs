//! The vertical numbers of the facility in one place: the storey, which storey a body is on (with its hysteresis), the
//! height a body stands at on a flight and the ladder's timings. The flight itself is defined once in `world/flight.rs`
//! (absolute y, data in `data/flights.json`); this file only reads it for a stair record. Godot is the reference: its
//! storey is 4 m (`StreamedFacilityView3D.LEVEL_HEIGHT`, `game/world/streaming/streamed_facility_view_3d.gd:18`) and ours
//! is `WALL_HEIGHT` (5 m, `config/mod.rs`), so every Godot distance here is scaled by `GODOT_TO_OURS` = 5/4.

use crate::config::WALL_HEIGHT;
use crate::world::facility::{Cell, Stair};

/// Godot's storey height in metres (`streamed_facility_view_3d.gd:18`, LEVEL_HEIGHT).
pub const GODOT_STOREY: f32 = 4.0;

/// Our storey over Godot's: every Godot distance is multiplied by this (4 m to 5 m, so 1.25).
pub const GODOT_TO_OURS: f32 = WALL_HEIGHT / GODOT_STOREY;

/// Our storey height, the floor-to-floor distance a level change crosses.
pub const STOREY: f32 = WALL_HEIGHT;

// ---------------------------------------------------------------------------------------------------------------------
// Stair flights
// ---------------------------------------------------------------------------------------------------------------------

/// Treads per cell of run, so a cell's metre of run is three deep steps rather than one metre-high block (read by
/// `world/flight.rs`).
pub const STEPS_PER_CELL: i32 = 3;

// ---------------------------------------------------------------------------------------------------------------------
// Storeys: which level a body is on
// ---------------------------------------------------------------------------------------------------------------------

/// Godot's LEVEL_TRANSITION_MARGIN (`streamed_facility_view_3d.gd:60`): the hysteresis past the storey midpoint.
pub const GODOT_LEVEL_TRANSITION_MARGIN: f32 = 0.35;
pub const LEVEL_TRANSITION_MARGIN: f32 = GODOT_LEVEL_TRANSITION_MARGIN * GODOT_TO_OURS;

/// The storey midpoint, 2.5 m here (Godot's 2 m at its 4 m storey): a body must be this far from its floor, plus the
/// hysteresis margin, before its level label changes (nothing swaps; absolute height stays continuous).
pub const LEVEL_MIDPOINT: f32 = STOREY * 0.5;

// ---------------------------------------------------------------------------------------------------------------------
// Ladders (`game/player/player_controller_3d.gd`)
// ---------------------------------------------------------------------------------------------------------------------

/// Ladder climb speed in m/s (`player_controller_3d.gd:377`, 1.7): a speed is a distance over time, so it scales with
/// the storey like every distance does (2.125 m/s on our 5 m storey).
pub const LADDER_CLIMB_SPEED: f32 = 1.7 * GODOT_TO_OURS;

/// How far the body stands from the rails, in metres (`player_controller_3d.gd:1492`, LADDER_RUNG_REACH .4). It is a
/// reach to the rungs, not a storey distance, so it is not scaled.
pub const LADDER_RAIL_REACH: f32 = 0.4;

/// The smooth mount onto a ladder, in seconds (`player_controller_3d.gd:379`).
pub const LADDER_MOUNT_SECONDS: f32 = 0.3;

/// The smooth dismount off a ladder, in seconds (`player_controller_3d.gd:380`).
pub const LADDER_DISMOUNT_SECONDS: f32 = 0.45;

/// Under this height on the ladder, jump lets go (`player_controller_3d.gd:378`: 1.3 times the player scale, 1.387 m). It
/// is a reach of the body, not a share of the storey, so it is not scaled: the same 1.387 m as Godot.
pub const LADDER_LET_GO_HEIGHT: f32 = 1.3 * 1.067;

/// The small lift over the hatch lip on the way off a ladder at the top, in metres (`player_controller_3d.gd:1348`, a
/// `sin(k * PI)` hop of .25 m during the dismount). It is a body motion over a lip, not a storey distance, so it is
/// not scaled.
pub const LADDER_TOP_HOP: f32 = 0.25;

// ---------------------------------------------------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------------------------------------------------

/// The height of the walking surface at `pos` (x, z in grid cells) above the floor of `level`, the level of the facility
/// holding `stair`, or `None` off the stair (see `Stair::surface_rel`). A ladder has no stair surface.
pub fn stair_surface_y(stair: Stair, level: i32, pos: (f32, f32)) -> Option<f32> {
    stair.surface_rel(level, pos.0, pos.1)
}

/// The level a body at height `y` is on, given the level it was on last (`current`). It stays put until the body is
/// more than `LEVEL_MIDPOINT + LEVEL_TRANSITION_MARGIN` from its floor, so a ladder or ramp never flickers between two.
pub fn level_from_height(y: f32, current: i32) -> i32 {
    let floor_y = current as f32 * STOREY;
    if (y - floor_y).abs() <= LEVEL_MIDPOINT + LEVEL_TRANSITION_MARGIN {
        return current;
    }
    (y / STOREY).round() as i32
}

// ---------------------------------------------------------------------------------------------------------------------
// Spiral clearance (data/stairs_links.json, `spiral_clearance`)
// ---------------------------------------------------------------------------------------------------------------------

/// How wide the clear area round a spiral staircase is, in cells (all data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpiralClearance {
    /// Cells of clear floor round the well.
    pub ring_width: i32,
    /// Cells of clear floor round the landing (upper level) or foot (lower level).
    pub landing_apron: i32,
    /// Width of the approach lane leading away from the end cell.
    pub approach_lane_width: i32,
    /// Length of the approach lane beyond the end cell.
    pub approach_lane_length: i32,
}

/// Reads the clearance numbers from the JSON text of stairs_links.json.
pub fn parse_spiral_clearance(text: &str) -> Result<SpiralClearance, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let o = v.get("spiral_clearance").ok_or("spiral_clearance: missing")?;
    let get = |k: &str| -> Result<i32, String> {
        let n = o.get(k).and_then(|x| x.as_i64()).ok_or_else(|| format!("spiral_clearance.{k}: an integer is needed"))?;
        if !(0..=8).contains(&n) {
            return Err(format!("spiral_clearance.{k}: {n} is outside 0..=8"));
        }
        Ok(n as i32)
    };
    Ok(SpiralClearance {
        ring_width: get("ring_width")?,
        landing_apron: get("landing_apron")?,
        approach_lane_width: get("approach_lane_width")?,
        approach_lane_length: get("approach_lane_length")?,
    })
}

/// The spiral clearance numbers, loaded once from the embedded data/stairs_links.json.
pub fn spiral_clearance() -> SpiralClearance {
    static C: std::sync::OnceLock<SpiralClearance> = std::sync::OnceLock::new();
    *C.get_or_init(|| {
        let text = crate::content::embedded::data_under("")
            .find(|(p, _)| *p == "stairs_links.json")
            .map(|(_, t)| t)
            .expect("data/stairs_links.json is embedded by build.rs");
        parse_spiral_clearance(text).expect("data/stairs_links.json spiral_clearance is valid")
    })
}

/// The widened clear zone of a spiral on one level, from the well `hole`, the level's own `end` cell (landing above,
/// foot below) and the numbers `c`: the ring round the well, the apron round the end and the approach lane leading
/// away from the well. Sorted, no repeats, without the hole itself. Walls are not removed here; the caller drops them.
pub fn spiral_zone_with(c: SpiralClearance, hole: &[Cell], end: Cell) -> Vec<Cell> {
    let mut out: Vec<Cell> = vec![];
    for h in hole {
        for dy in -c.ring_width..=c.ring_width {
            for dx in -c.ring_width..=c.ring_width {
                out.push(Cell::new(h.x + dx, h.y + dy));
            }
        }
    }
    for dy in -c.landing_apron..=c.landing_apron {
        for dx in -c.landing_apron..=c.landing_apron {
            out.push(Cell::new(end.x + dx, end.y + dy));
        }
    }
    // the lane leaves the well through the end cell, along the axis from the well's centre to it
    let n = hole.len().max(1) as f32;
    let (cx, cy) = (hole.iter().map(|h| h.x as f32).sum::<f32>() / n, hole.iter().map(|h| h.y as f32).sum::<f32>() / n);
    let (ox, oy) = (end.x as f32 - cx, end.y as f32 - cy);
    let dir: (i32, i32) = if ox.abs() >= oy.abs() { (if ox >= 0.0 { 1 } else { -1 }, 0) } else { (0, if oy >= 0.0 { 1 } else { -1 }) };
    let side = (dir.1.abs(), dir.0.abs());
    let half = c.approach_lane_width / 2;
    for along in 0..=c.approach_lane_length {
        for w in -half..(c.approach_lane_width - half) {
            out.push(Cell::new(end.x + dir.0 * along + side.0 * w, end.y + dir.1 * along + side.1 * w));
        }
    }
    out.sort();
    out.dedup();
    out.retain(|x| !hole.contains(x));
    out
}

/// `spiral_zone_with` with the data numbers.
pub fn spiral_zone(hole: &[Cell], end: Cell) -> Vec<Cell> {
    spiral_zone_with(spiral_clearance(), hole, end)
}

/// The widened keep-out of a spiral on both levels (the zone round its landing and round its foot), or nothing in the
/// Godot parity scope (`route::layout::godot_regions`, the `chunk_layout_godot` goldens), which keeps Godot's cells.
/// The one helper `Stair::carve_cells` and `flight_links::carve_cells` read, so props, hiding, fixtures, furnished
/// placement and vents all keep out of the same cells.
pub fn spiral_keepout(hole: &[Cell], landing: Cell, foot: Cell) -> Vec<Cell> {
    if crate::world::route::layout::godot_regions() {
        return vec![];
    }
    let mut v = spiral_zone(hole, landing);
    v.extend(spiral_zone(hole, foot));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::facility::{Cell, Cover};
    use crate::world::flight::{flight_surface_y, flights, max_riser_of, well_footprint, FlightKind, FlightShape};

    const RUNS: [(i32, i32); 4] = [(0, -1), (0, 1), (1, 0), (-1, 0)];
    const SHAPES: [FlightShape; 3] = [FlightShape::Switchback, FlightShape::Straight, FlightShape::Spiral];

    /// The stair on the upper level (level 0) and the stair on the lower level (-1) of the same flight.
    fn pair(shape: FlightShape, run: (i32, i32)) -> (Stair, Stair) {
        let upper = Stair::stairs(Cell::new(30, 30), -1, run, shape, false);
        let flight = upper.flight(0).unwrap();
        let lower = Stair::stairs(well_footprint(&flight).foot, 1, run, shape, false);
        (upper, lower)
    }

    fn cell_of(p: (f32, f32)) -> Cell {
        Cell::new(p.0.floor() as i32, p.1.floor() as i32)
    }

    #[test]
    fn the_numbers_are_godot_scaled_to_our_storey() {
        assert!((GODOT_TO_OURS - 1.25).abs() < 1e-6);
        assert!((LEVEL_MIDPOINT - 2.5).abs() < 1e-6, "2 m at Godot's 4 m storey is 2.5 m here");
        assert!((LEVEL_TRANSITION_MARGIN - 0.4375).abs() < 1e-6);
    }

    #[test]
    fn the_ladder_body_numbers_are_godots_unscaled() {
        // the let-go reach and the hop over the hatch lip are body motions, not storey distances
        assert!((LADDER_LET_GO_HEIGHT - 1.3871).abs() < 1e-4, "{LADDER_LET_GO_HEIGHT}");
        assert_eq!(LADDER_TOP_HOP, 0.25);
        // the climb speed does scale: the same 2.35 s over the whole shaft as Godot's 1.7 m/s over 4 m
        assert!((STOREY / LADDER_CLIMB_SPEED - GODOT_STOREY / 1.7).abs() < 1e-4);
    }

    #[test]
    fn a_flight_reaches_the_storey() {
        for shape in SHAPES {
            for run in RUNS {
                let (upper, lower) = pair(shape, run);
                let flight = upper.flight(0).unwrap();
                assert_eq!(lower.flight(-1), Some(flight), "{shape:?} {run:?}: both levels see one flight");
                let fp = well_footprint(&flight);
                assert_eq!(upper.well_cells(), lower.well_cells(), "{shape:?} {run:?}: one footprint");
                assert_eq!((upper.landing(), upper.foot()), (fp.landing, fp.foot));
                assert_eq!(upper.far_end(), fp.foot);
                assert_eq!(lower.far_end(), fp.landing);

                // the own end is the level's own floor, the other end is plain floor with no stair surface
                let (a, b) = (fp.landing.center(), fp.foot.center());
                assert_eq!(stair_surface_y(upper, 0, a), Some(0.0), "{shape:?} {run:?}: A is the upper floor");
                assert_eq!(stair_surface_y(lower, -1, b), Some(0.0), "{shape:?} {run:?}: B is the lower floor");
                assert_eq!(stair_surface_y(lower, -1, a), None, "{shape:?} {run:?}: no surface at A below");
                assert_eq!(stair_surface_y(upper, 0, b), None, "{shape:?} {run:?}: no surface at B above");

                let step = 0.05;
                let path = upper.walk_path(0, step);
                assert_eq!(path, flight_path_forward(&flight, step), "walk_path goes A to B");
                assert_eq!(cell_of(path[0]), fp.landing);
                assert_eq!(cell_of(*path.last().unwrap()), fp.foot);

                // A spiral's lead-out to the foot crosses the slab of upper floor beside the helicoid's end: the walker is on
                // the lower floor under it, so those samples are not on the way down
                let path: Vec<(f32, f32)> = path.into_iter().filter(|p| !crate::world::flight::on_slab(&flight, p.0, p.1)).collect();
                // absolute height along the walk: 0 at A down to -STOREY at B, never rising, small steps, one level flip
                let abs: Vec<f32> = path.iter().map(|p| flight_surface_y(&flight, p.0, p.1).unwrap()).collect();
                assert!((abs[0] - 0.0).abs() < 1e-4 && (abs[abs.len() - 1] + STOREY).abs() < 1e-4);
                assert!((abs[0] - abs[abs.len() - 1] - STOREY).abs() < 1e-4, "spans exactly one storey");
                let max_step = max_riser_of(flights(), shape) * STEPS_PER_CELL as f32 * step * 1.05 + 1e-4;
                let mut level = 0;
                let mut flips = 0;
                for w in abs.windows(2) {
                    assert!(w[1] <= w[0] + 1e-5, "{shape:?} {run:?}: rises on the way down {} -> {}", w[0], w[1]);
                    assert!(w[0] - w[1] <= max_step, "{shape:?} {run:?}: step {} over {max_step}", w[0] - w[1]);
                }
                for y in &abs {
                    let next = level_from_height(*y, level);
                    flips += (next != level) as i32;
                    level = next;
                }
                assert_eq!((flips, level), (1, -1), "{shape:?} {run:?}: the level flips exactly once");

                // the upper stair: relative height 0 down to -STOREY, equal to the absolute height on level 0
                let mut prev = 0.0f32;
                for p in path.iter().filter(|p| cell_of(**p) != fp.foot) {
                    let y = stair_surface_y(upper, 0, *p).expect("surface on the well and at A");
                    assert!(y <= prev + 1e-5 && y >= -STOREY - 1e-4, "{shape:?} {run:?}: upper {prev} -> {y}");
                    prev = y;
                }
                // the lower stair walked from B to A: relative height 0 up, never falling, the same surface + STOREY
                let mut prev = 0.0f32;
                let landing_row = crate::world::flight::landing_row_with(flights(), &flight);
                // (the rest of the landing row is the level's flat floor in each record, not the flight)
                for p in path.iter().rev().filter(|p| cell_of(**p) != fp.landing && (cell_of(**p) == fp.foot || !landing_row.contains(&cell_of(**p)))) {
                    let y = stair_surface_y(lower, -1, *p).expect("surface on the well and at B");
                    let want = flight_surface_y(&flight, p.0, p.1).unwrap() + STOREY;
                    assert!((y - want).abs() < 1e-4, "{shape:?} {run:?}: lower is the same surface");
                    assert!(y >= prev - 1e-5 && y <= STOREY + 1e-4, "{shape:?} {run:?}: lower {prev} -> {y}");
                    prev = y;
                }
                assert!(prev >= 3.0, "{shape:?} {run:?}: the lower stair climbs most of the storey, {prev}");
            }
        }
    }

    fn flight_path_forward(f: &crate::world::flight::Flight, step: f32) -> Vec<(f32, f32)> {
        crate::world::flight::walk_path(f, step)
    }

    #[test]
    fn the_surface_is_none_off_the_stair_and_for_ladders() {
        let (upper, _) = pair(FlightShape::Switchback, (0, -1));
        assert_eq!(stair_surface_y(upper, 0, (0.5, 0.5)), None, "far from the stair");
        for run in RUNS {
            let ladder = Stair::ladder_with_run(Cell::new(30, 30), -1, Cover::Grate, run);
            let step = Cell::new(30 + run.0, 30 + run.1);
            for at in [(30.5, 30.5), (step.x as f32 + 0.5, step.y as f32 + 0.5)] {
                assert_eq!(stair_surface_y(ladder, 0, at), None, "{run:?}: a ladder has no stair surface");
            }
            let flight = ladder.flight(0).expect("a ladder is a flight");
            assert_eq!(flight.kind, FlightKind::Ladder);
            assert_eq!((flight.anchor_cell, flight.run), (Cell::new(30, 30), run));
            assert_eq!(ladder.well_cells(), vec![Cell::new(30, 30)], "a ladder opens its hatch on the upper level");
            // the foot below is the same cell and the same flight: one hatch cell on both levels
            let foot = Stair::ladder_with_run(Cell::new(30, 30), 1, Cover::Grate, run);
            assert_eq!(foot.well_cells(), ladder.well_cells(), "{run:?}: one shaft cell from both records");
            assert_eq!(foot.flight(-1), Some(flight), "{run:?}: both levels are one flight");
            assert_eq!((ladder.step_cell(), foot.step_cell()), (step, step), "{run:?}: one step cell");
            assert_eq!((ladder.landing(), ladder.foot()), (step, Cell::new(30, 30)));
            assert_eq!(ladder.far_end(), Cell::new(30, 30));
            assert_eq!(foot.far_end(), step);
            assert_eq!(ladder.rails_side(), (-run.0, -run.1), "the rails stand opposite the step cell");
            assert_eq!(ladder.carve_cells(), foot.carve_cells());
            for c in [Cell::new(30, 30), step] {
                assert!(ladder.carve_cells().contains(&c), "{run:?}: the hatch and the step cell are carved");
            }
            assert!(ladder.dividers(0).is_empty());
        }
        let plain = Stair::ladder(Cell::new(30, 30), -1, Cover::Grate);
        assert_eq!(plain.run, (0, -1), "the default step is north");
        assert_eq!(plain.step_cell(), Cell::new(30, 29));
    }

    #[test]
    fn the_carve_cells_cover_the_well_both_ends_and_the_margins() {
        for shape in SHAPES {
            let (upper, lower) = pair(shape, (1, 0));
            let carve = upper.carve_cells();
            let mut sorted = carve.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(carve, sorted, "sorted without repeats");
            assert_eq!(carve, lower.carve_cells(), "both levels carve the same cells");
            for c in upper.well_cells().into_iter().chain([upper.landing(), upper.foot()]) {
                assert!(carve.contains(&c), "{shape:?}: {c:?}");
            }
        }
    }

    #[test]
    fn shifted_moves_only_the_cell() {
        let (upper, _) = pair(FlightShape::Spiral, (0, 1));
        let s = upper.shifted(3, -2);
        assert_eq!(s.cell, Cell::new(33, 28));
        assert_eq!((s.dir, s.run, s.shape, s.clockwise), (upper.dir, upper.run, upper.shape, upper.clockwise));
    }

    #[test]
    fn spiral_zone_follows_the_data_widths() {
        let hole: Vec<Cell> = (0..5).flat_map(|y| (0..5).map(move |x| Cell::new(40 + x, 40 + y))).collect();
        let end = Cell::new(42, 45);
        let c = SpiralClearance { ring_width: 2, landing_apron: 1, approach_lane_width: 3, approach_lane_length: 4 };
        let z = spiral_zone_with(c, &hole, end);
        assert!(!z.iter().any(|x| hole.contains(x)), "the well itself is not in the zone");
        assert!(z.contains(&Cell::new(38, 38)) && z.contains(&Cell::new(46, 46)), "the ring is ring_width wide");
        assert!(z.contains(&Cell::new(42, 49)), "the lane runs lane_length beyond the end");
        let wider = spiral_zone_with(SpiralClearance { ring_width: 3, ..c }, &hole, end);
        assert!(wider.len() > z.len(), "a larger data width gives a larger zone");
        let narrow = spiral_zone_with(SpiralClearance { ring_width: 1, ..c }, &hole, end);
        assert!(narrow.len() < z.len());
    }

    #[test]
    fn spiral_clearance_data_is_wider_than_one_cell() {
        let c = spiral_clearance();
        assert!(c.ring_width >= 2 && c.landing_apron >= 1 && c.approach_lane_width >= 3);
        assert!(parse_spiral_clearance("{}").is_err());
    }

    #[test]
    fn the_level_flips_only_past_the_midpoint_plus_hysteresis() {
        let limit = LEVEL_MIDPOINT + LEVEL_TRANSITION_MARGIN;
        // standing on level 0: stays through the midpoint and the hysteresis band
        assert_eq!(level_from_height(0.0, 0), 0);
        assert_eq!(level_from_height(LEVEL_MIDPOINT, 0), 0);
        assert_eq!(level_from_height(limit - 0.01, 0), 0);
        // and flips just past it
        assert_eq!(level_from_height(limit + 0.01, 0), 1);
        // coming back down from level 1 the same band holds it
        assert_eq!(level_from_height(STOREY - limit + 0.01, 1), 1);
        assert_eq!(level_from_height(STOREY - limit - 0.01, 1), 0);
        // a body already on the level below stays there while it is still in the band
        assert_eq!(level_from_height(-LEVEL_MIDPOINT, -1), -1);
        assert_eq!(level_from_height(-limit - 0.01, -1), -1);
        assert_eq!(level_from_height(-limit - 0.1, 0), -1, "a fall past the band changes level");
    }
}
