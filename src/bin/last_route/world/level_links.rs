//! Level links other than the stair flights: the sewer ladders between the maze (level 0) and the first sewer (-1).
//! The stair flights themselves, including the sewer end of each maze stairwell, are `world::flight_links`.
//!
//! Godot places them in `SewerRouteGenerator._build_chunk` (sewer_route_generator.gd, see docs/SEWERS.md "Ladders"):
//! about 55 % of sewer chunks (the origin always) take up to two `LevelLink`s of kind `ladder`, with `cover` set to
//! `manhole` or `grate`. The top end is a clear maze hallway cell and the bottom the same cell in the sewer. Rust reads
//! them from the ported generator (`route::sewer::chunk`), so both levels agree on every ladder.
//! A ladder is climbed by hand (game/ladder.rs), the links are the residency radius plus one Flight each.
//!
//! Each ladder is one `Flight` (`flight_links::ladder_flight`) with the hatch as its anchor and one clear step cell
//! beside it (`run`): both levels build their record from that single flight, so the step cell is the same cell on
//! both. A link with no usable step cell is dropped on both levels (`flight_links::ladder_choice`).

use super::facility::{Cell, Cover, Stair};
use super::flight_links;
use super::infinite::ChunkKey;
use super::route::sewer;

/// The ladders of chunk `k` as stair links (chunk-local cells, `run` the step direction to the one clear step cell):
/// on level 0 each leads down from its hallway cell, on -1 up from the same cell. Empty on every other level.
pub fn ladders(seed: &str, k: ChunkKey) -> Vec<Stair> {
    let dir = match k.level {
        0 => -1,
        sewer::core::LEVEL => 1,
        _ => return vec![],
    };
    flight_links::sewer_ladder_cells(seed, k.cx, k.cy)
        .iter()
        .filter_map(|l| {
            let flight = flight_links::ladder_choice(seed, k.cx, k.cy, l.from)?.flight;
            let c = if dir < 0 { l.from } else { l.to };
            let cover = if l.grate { Cover::Grate } else { Cover::Manhole };
            Some(Stair::ladder_with_run(Cell::new(c.x, c.y), dir, cover, flight.run))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::route::layout::Coord;

    const WORLDS: [&str; 3] = ["1426541270", "4157433391", "2899547009"];

    #[test]
    fn the_origin_column_has_a_ladder_and_both_levels_agree() {
        for world in WORLDS {
            let k = ChunkKey { cx: 0, cy: 0, level: 0 };
            let top = ladders(world, k);
            let bottom = ladders(world, ChunkKey { level: -1, ..k });
            assert!(!top.is_empty(), "{world}: origin chunk has no ladder");
            assert_eq!(top.len(), bottom.len());
            for (t, b) in top.iter().zip(&bottom) {
                assert_eq!((t.cell, t.dir, b.dir), (b.cell, -1, 1));
                assert_eq!(t.kind, b.kind);
                assert!(t.is_ladder());
                assert_eq!(t.run, b.run, "{world}: both levels step off the same way");
                assert_eq!(t.flight(0), b.flight(-1), "{world}: both levels are one flight");
            }
        }
    }

    #[test]
    fn the_ladder_step_cell_is_a_unit_step_inside_the_chunk_and_dry_below() {
        use crate::world::route::sewer::core::{FALL, SOLID, WATER};
        let mut seen = 0;
        for world in WORLDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    let Some(sc) = sewer::chunk(Coord::new(cx, sewer::core::LEVEL, cy), &sewer::Inputs { world }) else {
                        continue;
                    };
                    for s in ladders(world, ChunkKey { cx, cy, level: 0 }) {
                        seen += 1;
                        assert_eq!(s.run.0.abs() + s.run.1.abs(), 1, "{world} {cx},{cy}: a unit step, got {:?}", s.run);
                        let step = s.step_cell();
                        assert_eq!(step, Cell::new(s.cell.x + s.run.0, s.cell.y + s.run.1));
                        assert!(step.x >= 1 && step.y >= 1 && step.x <= 43 && step.y <= 43, "{world} {cx},{cy}: step {step:?} at the border");
                        let code = sc.sewer_grid[step.y as usize][step.x as usize];
                        assert!(![SOLID, WATER, FALL].contains(&code), "{world} {cx},{cy}: step {step:?} is code {code}");
                    }
                    for l in &sc.links {
                        assert_eq!(l.from_cell, l.to_cell, "{world} {cx},{cy}: the hatch and the foot are one cell");
                    }
                }
            }
        }
        assert!(seen > 20, "only {seen} ladders in the sweep");
    }

    #[test]
    fn a_ladder_flight_is_the_one_producer_for_both_levels() {
        use crate::world::flight::{well_footprint, FlightKind};
        for world in WORLDS {
            let k = ChunkKey { cx: 0, cy: 0, level: 0 };
            for s in ladders(world, k) {
                let f = crate::world::flight_links::ladder_flight(world, 0, 0, s.cell);
                assert_eq!(f.kind, FlightKind::Ladder);
                assert_eq!((f.anchor_cell, f.run), (s.cell, s.run));
                assert_eq!(s.flight(0), Some(f));
                assert_eq!(well_footprint(&f).landing, s.step_cell());
                assert_eq!(s.well_cells(), vec![s.cell], "the hatch is the one hole cell");
            }
        }
    }

    #[test]
    fn ladders_only_link_the_maze_and_the_first_sewer() {
        let mut seen = 0;
        for world in WORLDS {
            for cx in -3..=3 {
                for cy in -3..=3 {
                    for level in [-2, -3] {
                        assert!(ladders(world, ChunkKey { cx, cy, level }).is_empty());
                    }
                    let top = ladders(world, ChunkKey { cx, cy, level: 0 });
                    assert!(top.len() <= 2);
                    seen += top.len();
                }
            }
        }
        // about 55 % of chunks have one (docs/SEWERS.md); 147 chunks should give plenty
        assert!(seen > 40, "only {seen} ladders");
    }

    #[test]
    fn the_maze_end_of_a_ladder_is_an_open_layout_cell() {
        for world in WORLDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    let Some(lay) = crate::world::route::layout::chunk_layout(world, Coord::new(cx, 0, cy)) else {
                        continue;
                    };
                    for s in ladders(world, ChunkKey { cx, cy, level: 0 }) {
                        assert_eq!(lay.grid[s.cell.y as usize][s.cell.x as usize], 0, "{world} {cx},{cy} {:?}", s.cell);
                    }
                }
            }
        }
    }
}
