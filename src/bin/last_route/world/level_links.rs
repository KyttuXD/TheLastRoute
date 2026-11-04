//! Level links other than the stair flights: the sewer ladders between the maze (level 0) and the first sewer (-1).
//! The stair flights themselves, including the sewer end of each maze stairwell, are `world::flight_links`.
//!
//! Godot places them in `SewerRouteGenerator._build_chunk` (sewer_route_generator.gd, see docs/SEWERS.md "Ladders"):
//! about 55 % of sewer chunks (the origin always) take up to two `LevelLink`s of kind `ladder`, with `cover` set to
//! `manhole` or `grate`. The top end is a clear maze hallway cell and the bottom the same cell in the sewer. Rust reads
//! them from the ported generator (`route::sewer::chunk`), so both levels agree on every ladder.
//! A ladder is climbed by hand (game/ladder.rs), not taken with a fade.

use super::facility::{Cell, Cover, Stair};
use super::infinite::ChunkKey;
use super::route::layout::Coord;
use super::route::sewer;

/// The ladders of chunk `k` as stair links (chunk-local cells, `run` (0, 0)): on level 0 each leads down from its
/// hallway cell, on -1 up from the same cell. Empty on every other level.
pub fn ladders(seed: &str, k: ChunkKey) -> Vec<Stair> {
    let dir = match k.level {
        0 => -1,
        sewer::core::LEVEL => 1,
        _ => return vec![],
    };
    let Some(chunk) = sewer::chunk(Coord::new(k.cx, sewer::core::LEVEL, k.cy), &sewer::Inputs { world: seed }) else {
        return vec![];
    };
    chunk
        .links
        .iter()
        .map(|l| {
            let c = if dir < 0 { l.from_cell } else { l.to_cell };
            Stair::ladder(Cell::new(c.x, c.y), dir, if l.cover == "grate" { Cover::Grate } else { Cover::Manhole })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
