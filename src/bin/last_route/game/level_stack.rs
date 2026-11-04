//! Absolute vertical frame for bodies (docs/STAIRS_CONTINUOUS_DESIGN.md phase 2): the floor of level `n` sits at
//! `n * STOREY`, and `ground_y` reports the walkable surface under a point in that frame. One level at first; a later
//! phase adds the neighbour level behind the same functions.

use super::player::stair_height_at;
use crate::world::facility::Cell;
use crate::world::vertical::{level_from_height, STOREY};
use crate::world::Facility;

/// The absolute y of the floor of `level` (`level * STOREY`).
pub fn floor_y_of(level: i32) -> f32 {
    level as f32 * STOREY
}

/// The level a body at absolute height `y` is on, given the level it was on last (the hysteresis of
/// `vertical::level_from_height`).
pub fn level_at(y: f32, previous: i32) -> i32 {
    level_from_height(y, previous)
}

/// The walkable surface under a point: its absolute height and the level that owns it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    pub y: f32,
    pub level: i32,
}

/// The absolute surface under (x, z) of `fac`'s own level: a stair tread (`stair_height_at`), else the cell's walk
/// surface (a catwalk deck, `Facility::walk_surface_at`), else the plain floor. `y_hint` is the body's absolute feet
/// height; it is unused while only one level is resident and picks the floor once a neighbour level is added.
pub fn ground_y(fac: &Facility, x: f32, z: f32, _y_hint: f32) -> Ground {
    Ground { y: floor_y_of(fac.level) + local_ground(fac, x, z), level: fac.level }
}

/// The surface under (x, z) measured from its own level's floor: a stair tread, else the cell's walk surface, else 0.
/// Callers that work relative to the floor use this instead of subtracting the floor from `ground_y`, which loses
/// precision (a flight on level -1 would come out as 1.8e-7 instead of the tread height).
pub fn local_ground(fac: &Facility, x: f32, z: f32) -> f32 {
    stair_height_at(fac, (x, z)).unwrap_or_else(|| fac.height_at(Cell::new(x.floor() as i32, z.floor() as i32)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::{DeckKind, LinkKind, Options};

    #[test]
    fn floor_y_is_the_level_times_the_storey() {
        assert_eq!(floor_y_of(0), 0.0);
        assert_eq!(floor_y_of(-1), -STOREY);
        assert_eq!(floor_y_of(2), 2.0 * STOREY);
    }

    #[test]
    fn level_at_matches_level_from_height_including_its_hysteresis() {
        for previous in [-3, -1, 0, 1] {
            let mut y = -20.0_f32;
            while y < 10.0 {
                assert_eq!(level_at(y, previous), level_from_height(y, previous), "y {y} prev {previous}");
                y += 0.05;
            }
        }
        // it stays put near its own floor and moves only past the band
        assert_eq!(level_at(floor_y_of(-1) + 0.5, -1), -1);
        assert_eq!(level_at(floor_y_of(-1), 0), -1, "a body at a floor reads that level from far away");
    }

    #[test]
    fn ground_on_a_plain_floor_is_the_levels_floor() {
        let cat = Catalog::load();
        for level in [0, -1] {
            let mut f = Facility::generate("GROUND-1", &Options::default(), &cat);
            f.level = level;
            let c = f.start;
            let g = ground_y(&f, c.x as f32 + 0.5, c.y as f32 + 0.5, floor_y_of(level));
            assert_eq!(g, Ground { y: floor_y_of(level), level });
        }
    }

    #[test]
    fn ground_on_a_stair_flight_cell_is_the_tread() {
        // a real window of the Infinite route (the path the game uses), not the single-floor generator, which has no flights
        let content = crate::content::Content::load();
        let cat = Catalog::load();
        for n in 0..40 {
            let text = format!("ROOM-TEST-{n}");
            let g = crate::game::Game::new_infinite_sim(crate::config::Difficulty::Standard, 7, &text, -1, &content, &cat);
            let f = &g.fac;
            let Some(s) = f.stairs.iter().copied().find(|s| matches!(s.kind, LinkKind::Stairs)) else { continue };
            let path = s.walk_path(f.level, 0.25);
            let p = path[path.len() / 2];
            let tread = stair_height_at(f, p).expect("on the flight");
            assert!(tread != 0.0, "a flight cell is off the floor");
            let ground = ground_y(f, p.0, p.1, floor_y_of(f.level));
            assert_eq!(ground.y, floor_y_of(f.level) + tread);
            assert_eq!(ground.level, f.level);
            assert_eq!(local_ground(f, p.0, p.1), tread, "the local height is the tread exactly, with no floor round trip");
            return;
        }
        panic!("no test window has a stair flight");
    }

    #[test]
    fn ground_on_a_catwalk_deck_cell_is_the_deck_height() {
        let cat = Catalog::load();
        let mut f = Facility::generate("GROUND-DECK", &Options::default(), &cat);
        let c = f.start;
        f.set_walk_surface(c, DeckKind::Catwalk, 2.25);
        let g = ground_y(&f, c.x as f32 + 0.5, c.y as f32 + 0.5, 2.25);
        assert_eq!(g, Ground { y: 2.25, level: 0 });
    }
}
