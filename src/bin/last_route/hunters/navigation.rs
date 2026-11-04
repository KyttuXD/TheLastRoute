//! The walkable surface of a level-0 hunter's world: the footprint rules of Godot's `StreamedHunterNavigation`
//! (game/hunters/ai/streamed_hunter_navigation.gd) that the live window's `Facility` grid is built from.
//!
//! The grid is `Facility::hunter_open` over the runner's 3x3 window. `world::route::live::from_layout` sets its walls
//! and its solid props, and takes each prop's footprint from the rules here. The footprint is the prop's collider
//! (`collider_size`, else `blocker_size`: `_collider_rects` and `StreamedHunterNavigation._collect_solid_prop_cells`
//! read the collider the same way), and a prop no taller than `WALK_ON_HEIGHT` is floor to the runner and to the
//! hunters (`blocks_nothing`), so it is not solid.

/// `StreamedHunterNavigation.WALK_ON_HEIGHT` (streamed_hunter_navigation.gd): a solid prop whose collider is no taller
/// than this is walked over.
pub const WALK_ON_HEIGHT: f32 = 0.25;

/// `StreamedHunterNavigation.NAV_RADIUS` (the capsule radius a cell's centre is tested with): `Facility::DEFAULT_AGENT_RADIUS`.
pub const NAV_RADIUS: f32 = 0.23;

/// The collider a prop's footprint is read from: `collider_size`, else the blocker
/// (`prop.get("collider_size", prop.get("blocker_size", Vector3.ONE))`).
pub fn footprint(collider: Option<[f32; 3]>, blocker: [f32; 3]) -> [f32; 3] {
    collider.unwrap_or(blocker)
}

/// `StreamedHunterNavigation.blocks_nothing`: true for a solid prop the runner and the hunters walk over. The raised
/// collider rule (`collider_y0`) is not on the Rust prop, so it counts as zero.
pub fn walk_over(size: [f32; 3]) -> bool {
    size[1] <= WALK_ON_HEIGHT
}

/// The half extents of a prop's footprint on the plan, for a rotation in quarter turns: `Facility::can_occupy` reads one
/// axis-aligned box per prop, so x and z swap on an odd turn.
pub fn half_extents(size: [f32; 3], quarter_turns: i32) -> (f32, f32) {
    let [bx, _, bz] = size;
    if quarter_turns.rem_euclid(4) % 2 == 1 {
        (bz / 2.0, bx / 2.0)
    } else {
        (bx / 2.0, bz / 2.0)
    }
}

/// The one footprint rule for a body on a flight at absolute height `y` (`Facility::hunter_fits_at_y`): the brain's
/// edge walk and the gimmicks share it, so a hunter standing on a flight cell off an edge is legal exactly when it is
/// legal on one.
pub fn on_flight(fac: &crate::world::Facility, x: f32, z: f32, y: f32) -> bool {
    fac.hunter_fits_at_y(x, z, y, super::flight_edges::rules().edge_fit_radius)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_collider_is_the_footprint_when_there_is_one() {
        assert_eq!(footprint(Some([2.0, 1.0, 0.5]), [1.0, 1.0, 1.0]), [2.0, 1.0, 0.5]);
        assert_eq!(footprint(None, [1.0, 1.0, 1.0]), [1.0, 1.0, 1.0]);
    }

    #[test]
    fn a_prop_no_taller_than_the_walk_on_height_is_floor() {
        assert!(walk_over([1.0, WALK_ON_HEIGHT, 1.0]));
        assert!(walk_over([1.0, 0.1, 1.0]));
        assert!(!walk_over([1.0, 0.26, 1.0]));
    }

    #[test]
    fn half_extents_swap_on_an_odd_turn() {
        assert_eq!(half_extents([2.0, 1.0, 4.0], 0), (1.0, 2.0));
        assert_eq!(half_extents([2.0, 1.0, 4.0], 1), (2.0, 1.0));
        assert_eq!(half_extents([2.0, 1.0, 4.0], 2), (1.0, 2.0));
        assert_eq!(half_extents([2.0, 1.0, 4.0], -1), (2.0, 1.0));
    }
}
