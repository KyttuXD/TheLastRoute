//! The vertical numbers of the facility in one place: the stair treads (drawn by `render/stairs.rs`, which reads
//! them from here), the height a body stands at on a flight, which storey a body is on (with its hysteresis) and the
//! ladder's timings. Godot is the reference: its storey is 4 m (`StreamedFacilityView3D.LEVEL_HEIGHT`,
//! `game/world/streaming/streamed_facility_view_3d.gd:18`) and ours is `WALL_HEIGHT` (5 m, `config/mod.rs`), so every
//! Godot distance here is scaled by `GODOT_TO_OURS` = 5/4. Slopes and ratios carry over unchanged.

use crate::config::WALL_HEIGHT;
use crate::world::facility::{stair_well_cells, Stair, STAIR_FLIGHT};

/// Godot's storey height in metres (`streamed_facility_view_3d.gd:18`, LEVEL_HEIGHT).
pub const GODOT_STOREY: f32 = 4.0;

/// Our storey over Godot's: every Godot distance is multiplied by this (4 m to 5 m, so 1.25).
pub const GODOT_TO_OURS: f32 = WALL_HEIGHT / GODOT_STOREY;

/// Our storey height, the floor-to-floor distance a level change crosses.
pub const STOREY: f32 = WALL_HEIGHT;

// ---------------------------------------------------------------------------------------------------------------------
// Stair flights (the numbers render/stairs.rs draws)
// ---------------------------------------------------------------------------------------------------------------------

/// Treads per cell of run, so a cell's metre of run is three deep steps rather than one metre-high block.
pub const STEPS_PER_CELL: i32 = 3;

/// The flat treads at the foot of a flight down (its last cell): a landing to stand on, so the flight ends on a floor
/// and not against the pit's wall.
pub const LANDING: i32 = STEPS_PER_CELL;

/// Treads in one whole flight: every cell of run times the treads per cell.
pub const STAIR_STEPS: i32 = STAIR_FLIGHT * STEPS_PER_CELL;

/// How far a flight up rises per metre of run: about 33 degrees (`FLIGHT_SLOPE_DEG` gives the Godot slope it follows).
pub const RISE_UP: f32 = 0.65;

/// How far a flight down sinks per metre of run, over the metres before its landing: about 14 degrees. Kept shallower
/// than Godot's flights so the treads still show past the near lip of the pit from a standing eye.
pub const RISE_DOWN: f32 = 0.24;

/// The height of one tread's riser on a flight up (`RISE_UP` spread over its treads per cell).
pub const STAIR_RISE: f32 = RISE_UP / STEPS_PER_CELL as f32;

/// Godot's switchback half flight runs this far (metres, `game/world/generation/stairwells.gd`: rows 2..last-1 of the
/// interior, a 3 m run) while it rises half a storey to the mid landing.
pub const GODOT_SWITCHBACK_RUN: f32 = 3.0;

/// The Godot flight slope in degrees: half a storey (2 m) over the 3 m run is atan(2/3), 33.7 degrees.
pub const FLIGHT_SLOPE_DEG: f32 = 33.7;

/// The same slope as rise per metre of run (tan 33.7 degrees is 2/3). Our `RISE_UP` is 0.65, a hair under it.
pub const FLIGHT_RISE_PER_RUN: f32 = 2.0 / 3.0;

/// A half flight in our metres: its run and, to the mid landing, its rise (half of `STOREY`).
pub const FLIGHT_RUN: f32 = GODOT_SWITCHBACK_RUN * GODOT_TO_OURS;
pub const MID_LANDING_RISE: f32 = STOREY * 0.5;

/// Godot's FLOOR_SNAP (`player_controller_3d.gd:45`): .28 of the player's model scale (`player_models.gd` SCALE 1.067),
/// how far a grounded body is pulled down onto a ramp. Scaled to our storey like every distance.
pub const GODOT_FLOOR_SNAP: f32 = 0.28 * 1.067;
pub const FLOOR_SNAP: f32 = GODOT_FLOOR_SNAP * GODOT_TO_OURS;

// ---------------------------------------------------------------------------------------------------------------------
// Storeys: which level a body is on
// ---------------------------------------------------------------------------------------------------------------------

/// Godot's LEVEL_TRANSITION_MARGIN (`streamed_facility_view_3d.gd:60`): the hysteresis past the storey midpoint.
pub const GODOT_LEVEL_TRANSITION_MARGIN: f32 = 0.35;
pub const LEVEL_TRANSITION_MARGIN: f32 = GODOT_LEVEL_TRANSITION_MARGIN * GODOT_TO_OURS;

/// The storey midpoint, 2.5 m here (Godot's 2 m at its 4 m storey): a body must be this far from its floor, plus the
/// hysteresis margin, before the level changes.
pub const LEVEL_MIDPOINT: f32 = STOREY * 0.5;

// ---------------------------------------------------------------------------------------------------------------------
// Ladders (`game/player/player_controller_3d.gd`)
// ---------------------------------------------------------------------------------------------------------------------

/// Ladder climb speed in m/s (`player_controller_3d.gd:377`). Taken as Godot's value; see the report note on scaling.
pub const LADDER_CLIMB_SPEED: f32 = 1.7;

/// The smooth mount onto a ladder, in seconds (`player_controller_3d.gd:379`).
pub const LADDER_MOUNT_SECONDS: f32 = 0.3;

/// The smooth dismount off a ladder, in seconds (`player_controller_3d.gd:380`).
pub const LADDER_DISMOUNT_SECONDS: f32 = 0.45;

/// Under this height on the ladder, jump lets go (`player_controller_3d.gd:378`: 1.3 times the player scale), scaled to
/// our storey.
pub const LADDER_LET_GO_HEIGHT: f32 = 1.3 * 1.067 * GODOT_TO_OURS;

// ---------------------------------------------------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------------------------------------------------

/// The top of tread `j` (1-based, 1..=STAIR_STEPS) of a flight: `dir` -1 down, 1 up. Each tread is one rise further
/// than the last; a flight down stops sinking at its foot, so its last `LANDING` treads are flat.
pub fn tread_top(dir: i32, j: i32) -> f32 {
    let per = STEPS_PER_CELL as f32;
    let rise = (if dir < 0 { RISE_DOWN } else { RISE_UP }) / per;
    let sloped = if dir < 0 { STAIR_STEPS - LANDING } else { STAIR_STEPS };
    dir.signum() as f32 * j.min(sloped) as f32 * rise
}

/// The height of the walking surface at `pos` (x, z in grid cells, a cell `(x, y)` spanning `x..x+1`), or `None` off
/// the stair. The landing cell is flat at 0; each flight cell is the height of the tread under `pos`, so the surface
/// is flat on a landing and steps up or down along a flight. A ladder has no stair surface.
pub fn stair_surface_y(stair: Stair, pos: (f32, f32)) -> Option<f32> {
    if stair.is_ladder() {
        return None;
    }
    let (x, z) = pos;
    let here = (x.floor() as i32, z.floor() as i32);
    if here == (stair.cell.x, stair.cell.y) {
        return Some(0.0);
    }
    // metres along the run from the landing cell's far edge (0 there, 1 at the end of the first cell)
    let edge = (stair.cell.x + (stair.run.0 > 0) as i32, stair.cell.y + (stair.run.1 > 0) as i32);
    let along = stair.run.0 as f32 * (x - edge.0 as f32) + stair.run.1 as f32 * (z - edge.1 as f32);
    for k in 1..=STAIR_FLIGHT {
        if stair_well_cells(stair, k).iter().any(|c| (c.x, c.y) == here) {
            let j = ((along * STEPS_PER_CELL as f32).floor() as i32).clamp(0, STAIR_STEPS - 1);
            return Some(tread_top(stair.dir, j + 1));
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::stairs::treads;
    use crate::world::facility::{Cell, LinkKind};

    fn flight(dir: i32, run: (i32, i32)) -> Stair {
        Stair { cell: Cell::new(20, 20), dir, run, kind: LinkKind::Stairs }
    }

    /// A point `along` metres from the landing's far edge, on the first lane of the flight.
    fn at(s: Stair, along: f32) -> (f32, f32) {
        let edge = (s.cell.x + (s.run.0 > 0) as i32, s.cell.y + (s.run.1 > 0) as i32);
        let side = (s.run.1.abs() as f32, s.run.0.abs() as f32);
        (
            edge.0 as f32 + s.run.0 as f32 * along + side.0 * 0.5,
            edge.1 as f32 + s.run.1 as f32 * along + side.1 * 0.5,
        )
    }

    const RUNS: [(i32, i32); 4] = [(0, -1), (0, 1), (1, 0), (-1, 0)];

    #[test]
    fn the_numbers_are_godot_scaled_to_our_storey() {
        assert!((GODOT_TO_OURS - 1.25).abs() < 1e-6);
        assert!((LEVEL_MIDPOINT - 2.5).abs() < 1e-6, "2 m at Godot's 4 m storey is 2.5 m here");
        assert!((LEVEL_TRANSITION_MARGIN - 0.4375).abs() < 1e-6);
        assert!((FLIGHT_RISE_PER_RUN.atan().to_degrees() - FLIGHT_SLOPE_DEG).abs() < 0.05);
        assert!((MID_LANDING_RISE - LEVEL_MIDPOINT).abs() < 1e-6);
        assert!((FLOOR_SNAP - 0.28 * 1.067 * 1.25).abs() < 1e-6);
        // the drawn flight up is within a degree of Godot's slope
        assert!((RISE_UP.atan().to_degrees() - FLIGHT_SLOPE_DEG).abs() < 1.0);
    }

    #[test]
    fn the_surface_is_flat_on_the_landings() {
        for dir in [-1, 1] {
            let s = flight(dir, (0, -1));
            // the landing cell the runner steps on is level with the floor
            for along in [-0.9, -0.5, -0.1] {
                assert_eq!(stair_surface_y(s, at(s, along)), Some(0.0), "dir {dir} at {along}");
            }
        }
        // a flight down ends in LANDING flat treads: the last cell is one level, at the foot's height
        for run in RUNS {
            let s = flight(-1, run);
            let foot = tread_top(-1, STAIR_STEPS);
            for j in 0..LANDING {
                let along = (STAIR_STEPS - LANDING + j) as f32 / STEPS_PER_CELL as f32 + 0.1;
                assert!((stair_surface_y(s, at(s, along)).unwrap() - foot).abs() < 1e-6, "run {run:?} tread {j}");
            }
            assert!((tread_top(-1, STAIR_STEPS - LANDING) - foot).abs() < 1e-6, "the sloped part meets the foot");
        }
    }

    #[test]
    fn the_surface_is_monotonic_on_the_flight() {
        for run in RUNS {
            for dir in [-1, 1] {
                let s = flight(dir, run);
                let mut prev = stair_surface_y(s, at(s, 0.0)).unwrap();
                // 0.01 m steps over the whole flight, from just past the landing to just short of its last metre
                for i in 1..(STAIR_FLIGHT * 100) {
                    let along = i as f32 * 0.01;
                    let y = stair_surface_y(s, at(s, along)).unwrap();
                    assert!((y - prev) * dir as f32 >= -1e-6, "dir {dir} run {run:?} at {along}: {prev} -> {y}");
                    prev = y;
                }
                // the flight goes the way it is meant to: up ends above the floor, down ends below it
                assert!(prev * dir as f32 >= 0.0);
            }
        }
    }

    #[test]
    fn the_surface_matches_the_mesh_step_tops() {
        for dir in [-1, 1] {
            let s = flight(dir, (0, 1));
            let mesh = treads(dir);
            assert_eq!(mesh.len(), STAIR_STEPS as usize);
            for (i, t) in mesh.iter().enumerate() {
                let mid = (t.from + t.to) * 0.5;
                let y = stair_surface_y(s, at(s, mid)).unwrap();
                assert!((y - t.top).abs() < 1e-6, "dir {dir} tread {i}: surface {y} mesh {}", t.top);
            }
        }
    }

    #[test]
    fn the_surface_is_none_off_the_stair() {
        let s = flight(-1, (0, -1));
        assert_eq!(stair_surface_y(s, (0.5, 0.5)), None, "far from the stair");
        assert_eq!(stair_surface_y(s, at(s, STAIR_FLIGHT as f32 + 0.5)), None, "past the last cell");
        let ladder = Stair { kind: LinkKind::Ladder(crate::world::facility::Cover::Grate), ..s };
        assert_eq!(stair_surface_y(ladder, at(s, 0.5)), None, "a ladder has no stair surface");
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
