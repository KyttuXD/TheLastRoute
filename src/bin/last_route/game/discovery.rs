//! The cells of level 0 the runner has seen, for the minimap and the full map: a compact bitmap per chunk, keyed by the chunk's
//! absolute coordinates (so it outlives the 3x3 window that moves with the runner, a chunk unloading and loading again, and a
//! walk across regions). A sewer or Inferno storey keeps its discovery as a set of absolute cells in `InfState::seen`; level 0
//! has far more floor than they do, so it keeps one bit a cell (45 x 45 = 2025 bits, 256 bytes a chunk) instead.
//!
//! Marking is a shift and an or, and nothing here allocates once a chunk has its bitmap, so the runner's reveal (five times a
//! second) and the window refill on a recentre cost no more than the sewer's.

use crate::world::infinite::CHUNK;
use std::collections::HashMap;

/// The 64-bit words one chunk's bitmap needs.
const WORDS: usize = ((CHUNK * CHUNK) as usize).div_ceil(64);

/// One chunk's seen cells, bit `z * CHUNK + x` for the chunk-local cell (x, z).
#[derive(Clone, Debug)]
pub struct Bits {
    words: [u64; WORDS],
    /// How many bits are set, kept so "any" and "count" are free.
    count: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits { words: [0; WORDS], count: 0 }
    }

    /// Whether chunk-local cell (`x`, `z`) is seen.
    pub fn get(&self, x: i32, z: i32) -> bool {
        let i = (z * CHUNK + x) as usize;
        self.words[i / 64] >> (i % 64) & 1 == 1
    }

    /// Sets the bit; true when it was clear.
    fn set(&mut self, x: i32, z: i32) -> bool {
        let i = (z * CHUNK + x) as usize;
        let m = 1u64 << (i % 64);
        let w = &mut self.words[i / 64];
        if *w & m != 0 {
            return false;
        }
        *w |= m;
        self.count += 1;
        true
    }
}

/// Level 0's seen cells by chunk (cx, cy).
#[derive(Clone, Debug, Default)]
pub struct SeenCells {
    chunks: HashMap<(i32, i32), Bits>,
    /// The seen cells in all chunks; a change to it tells a reader the picture changed.
    total: usize,
}

/// The chunk of an absolute cell and the cell inside it.
fn split(ax: i32, ay: i32) -> ((i32, i32), (i32, i32)) {
    ((ax.div_euclid(CHUNK), ay.div_euclid(CHUNK)), (ax.rem_euclid(CHUNK), ay.rem_euclid(CHUNK)))
}

impl SeenCells {
    /// Whether absolute cell (`ax`, `ay`) has been seen.
    pub fn is_seen(&self, ax: i32, ay: i32) -> bool {
        let (k, (x, z)) = split(ax, ay);
        self.chunks.get(&k).is_some_and(|b| b.get(x, z))
    }

    /// Marks absolute cell (`ax`, `ay`) seen; true when it was new.
    pub fn mark(&mut self, ax: i32, ay: i32) -> bool {
        let (k, (x, z)) = split(ax, ay);
        let new = self.chunks.entry(k).or_insert_with(Bits::new).set(x, z);
        self.total += new as usize;
        new
    }

    /// Marks every cell of chunk `k` seen (the admin map reveal).
    pub fn mark_chunk(&mut self, k: (i32, i32)) {
        let b = self.chunks.entry(k).or_insert_with(Bits::new);
        if b.count as i32 == CHUNK * CHUNK {
            return;
        }
        self.total -= b.count as usize;
        for z in 0..CHUNK {
            for x in 0..CHUNK {
                b.set(x, z);
            }
        }
        self.total += b.count as usize;
    }

    /// The bitmap of chunk `k`, for a reader that tests many cells of it (the full map).
    pub fn chunk(&self, k: (i32, i32)) -> Option<&Bits> {
        self.chunks.get(&k)
    }

    /// Whether chunk `k` has any seen cell.
    pub fn chunk_has_any(&self, k: (i32, i32)) -> bool {
        self.chunks.get(&k).is_some_and(|b| b.count > 0)
    }

    /// The seen cells of chunk `k`.
    pub fn chunk_count(&self, k: (i32, i32)) -> usize {
        self.chunks.get(&k).map_or(0, |b| b.count as usize)
    }

    /// The seen cells in all chunks.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Writes the seen state of the window whose top-left absolute cell is `origin` into `grid` (rows of `grid[y][x]`, any
    /// size): every cell is set or cleared, so a window refilled after a recentre shows exactly what was seen.
    pub fn fill_window(&self, origin: (i32, i32), grid: &mut [Vec<bool>]) {
        for (wy, row) in grid.iter_mut().enumerate() {
            let ay = origin.1 + wy as i32;
            let cy = ay.div_euclid(CHUNK);
            let z = ay.rem_euclid(CHUNK);
            let mut wx = 0;
            while wx < row.len() {
                let ax = origin.0 + wx as i32;
                let cx = ax.div_euclid(CHUNK);
                let x0 = ax.rem_euclid(CHUNK);
                // the run of this row inside one chunk
                let run = ((CHUNK - x0) as usize).min(row.len() - wx);
                match self.chunks.get(&(cx, cy)) {
                    Some(b) => {
                        for i in 0..run {
                            row[wx + i] = b.get(x0 + i as i32, z);
                        }
                    }
                    None => row[wx..wx + run].iter_mut().for_each(|d| *d = false),
                }
                wx += run;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_is_keyed_by_its_absolute_coordinates_including_negative_ones() {
        let mut s = SeenCells::default();
        assert!(s.mark(0, 0));
        assert!(!s.mark(0, 0), "a second mark is not new");
        assert!(s.mark(-1, -1));
        assert!(s.mark(CHUNK, -CHUNK - 1));
        assert!(s.is_seen(0, 0) && s.is_seen(-1, -1) && s.is_seen(CHUNK, -CHUNK - 1));
        assert!(!s.is_seen(1, 0) && !s.is_seen(-2, -1) && !s.is_seen(CHUNK - 1, -CHUNK - 1));
        assert_eq!(s.total(), 3);
        // (-1, -1) is the last cell of chunk (-1, -1); (0, 0) the first of chunk (0, 0)
        assert_eq!(s.chunk_count((-1, -1)), 1);
        assert_eq!(s.chunk_count((0, 0)), 1);
        assert!(s.chunk_has_any((1, -2)) && !s.chunk_has_any((1, 0)));
    }

    #[test]
    fn the_last_cell_of_a_chunk_fits_the_bitmap() {
        let mut s = SeenCells::default();
        assert!(s.mark(CHUNK - 1, CHUNK - 1));
        assert!(s.is_seen(CHUNK - 1, CHUNK - 1) && !s.is_seen(CHUNK - 2, CHUNK - 1));
    }

    #[test]
    fn a_window_is_refilled_from_the_chunks_it_covers() {
        let mut s = SeenCells::default();
        // a cell in the chunk west of the window and one inside it
        s.mark(-1, 5);
        s.mark(10, 7);
        let mut grid = vec![vec![true; 30]; 12];
        // the window starts at absolute (-3, 0): column 2 is absolute -1, column 13 is absolute 10
        s.fill_window((-3, 0), &mut grid);
        let seen: Vec<(usize, usize)> = (0..12)
            .flat_map(|y| (0..30).map(move |x| (x, y)))
            .filter(|&(x, y)| grid[y][x])
            .collect();
        assert_eq!(seen, vec![(2, 5), (13, 7)], "stale cells are cleared and only the seen ones are set");
    }

    #[test]
    fn a_whole_chunk_marks_once_and_counts_right() {
        let mut s = SeenCells::default();
        s.mark(3, 3);
        s.mark_chunk((0, 0));
        assert_eq!(s.chunk_count((0, 0)), (CHUNK * CHUNK) as usize);
        assert_eq!(s.total(), (CHUNK * CHUNK) as usize);
        s.mark_chunk((0, 0));
        assert_eq!(s.total(), (CHUNK * CHUNK) as usize, "filling a full chunk again changes nothing");
    }

    #[test]
    fn a_chunk_costs_a_few_hundred_bytes() {
        assert!(std::mem::size_of::<Bits>() <= 288, "{}", std::mem::size_of::<Bits>());
    }
}
