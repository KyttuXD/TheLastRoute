//! Level -3 caverns (port of game/world/regions/inferno_caverns.gd, docs/INFERNO_CAVERNS.md): every chunk
//! gets one cavern 16-22 cells across (a second 14-16 when it fits), a rough ellipse with a clear 7x7 middle,
//! a 5-wide hall from each cavern to the chunk edge, and up to 3 one-cell crevices cut into the rim.
//!
//! Heights: the rim and the cavern edge are an upper ledge (0.0, the maze floor), the middle is a lower
//! basin (`-BASIN_DEPTH`), with a smoothstep slope between. Walking stays 2D; the renderer can read
//! [`Inferno::height`] to draw the ledges and basin later.

use super::infinite::{ChunkKey, CHUNK, EDGE_OFFSETS};
use super::rng::Rng;
use super::Cell;

/// How far the basin floor sits below the ledge, in metres.
pub const BASIN_DEPTH: f32 = 1.2;
const MARGIN: i32 = 3;
const HALL_WIDTH: i32 = 5;
const N: usize = CHUNK as usize;

/// Which elevation band a cell is in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    Ledge,
    Slope,
    Basin,
}

/// A rectangle of chunk-local cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn touches_edge(&self) -> bool {
        self.x <= 0 || self.y <= 0 || self.x + self.w >= CHUNK || self.y + self.h >= CHUNK
    }
}

#[derive(Clone, Debug)]
pub struct Cavern {
    pub rect: Rect,
    pub center: Cell,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug)]
pub struct Inferno {
    pub caverns: Vec<Cavern>,
    pub halls: Vec<Rect>,
    pub crevices: Vec<Cell>,
    open: Vec<bool>,
    height: Vec<f32>,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The seam offsets of one chunk border (same draw as infinite::boundary_offsets), so a hall meets the
/// neighbour's seam.
fn seam_offsets(seed: &str, key: &str) -> Vec<i32> {
    let mut rng = Rng::new(&format!("{seed}:SEAM:{key}"));
    let mut pool = EDGE_OFFSETS.to_vec();
    let count = 1 + (rng.next_float() < 0.5) as usize;
    let mut out = vec![];
    for _ in 0..count {
        out.push(pool.remove(rng.next_index(pool.len())));
    }
    out
}

impl Inferno {
    /// Cell height in metres for a chunk-local cell: 0.0 on a ledge, hall or plain maze cell,
    /// down to `-BASIN_DEPTH` in a basin. Carves the chunk each call; keep an `Inferno` to ask often.
    pub fn height(seed: &str, key: ChunkKey, cell: Cell) -> f32 {
        Inferno::carve(seed, key).height_at(cell)
    }

    pub fn height_at(&self, c: Cell) -> f32 {
        if c.x < 0 || c.y < 0 || c.x >= CHUNK || c.y >= CHUNK {
            return 0.0;
        }
        self.height[c.y as usize * N + c.x as usize]
    }

    pub fn tier_at(&self, c: Cell) -> Tier {
        let h = self.height_at(c);
        if h > -0.1 * BASIN_DEPTH {
            Tier::Ledge
        } else if h < -0.9 * BASIN_DEPTH {
            Tier::Basin
        } else {
            Tier::Slope
        }
    }

    /// Whether the carve opens this cell (cavern, hall or crevice).
    pub fn is_open(&self, c: Cell) -> bool {
        c.x >= 0 && c.y >= 0 && c.x < CHUNK && c.y < CHUNK && self.open[c.y as usize * N + c.x as usize]
    }

    /// Every cell the carve opens.
    pub fn open_cells(&self) -> Vec<Cell> {
        (0..N * N)
            .filter(|i| self.open[*i])
            .map(|i| Cell::new((i % N) as i32, (i / N) as i32))
            .collect()
    }

    /// Carves a level -3 chunk, deterministic from the chunk's own seed text.
    pub fn carve(seed: &str, k: ChunkKey) -> Inferno {
        let text = format!("{seed}:L{}:C{},{}", k.level, k.cx, k.cy);
        let mut rng = Rng::new(&format!("{text}:INFERNO"));
        let mut inf = Inferno {
            caverns: vec![],
            halls: vec![],
            crevices: vec![],
            open: vec![false; N * N],
            height: vec![0.0; N * N],
        };
        // seam keys per side: 0 left, 1 right, 2 top, 3 bottom
        let sides = [
            format!("V:{}:{}:{}", k.level, k.cx, k.cy),
            format!("V:{}:{}:{}", k.level, k.cx + 1, k.cy),
            format!("H:{}:{}:{}", k.level, k.cx, k.cy),
            format!("H:{}:{}:{}", k.level, k.cx, k.cy + 1),
        ];
        let first_side = rng.next_index(4);
        let w = 16 + rng.next_index(7) as i32;
        let h = 16 + rng.next_index(7) as i32;
        let r1 = Rect {
            x: MARGIN + rng.next_index((CHUNK - 2 * MARGIN - w + 1) as usize) as i32,
            y: MARGIN + rng.next_index((CHUNK - 2 * MARGIN - h + 1) as usize) as i32,
            w,
            h,
        };
        inf.add_cavern(&mut rng, r1);
        inf.add_hall(&mut rng, seed, &sides, first_side, 0);
        // a second, smaller cavern when it fits (a 2-cell gap to the first)
        let (w2, h2) = (14 + rng.next_index(3) as i32, 14 + rng.next_index(3) as i32);
        for _ in 0..24 {
            let r2 = Rect {
                x: MARGIN + rng.next_index((CHUNK - 2 * MARGIN - w2 + 1) as usize) as i32,
                y: MARGIN + rng.next_index((CHUNK - 2 * MARGIN - h2 + 1) as usize) as i32,
                w: w2,
                h: h2,
            };
            let apart = r2.x + r2.w + 2 <= r1.x
                || r1.x + r1.w + 2 <= r2.x
                || r2.y + r2.h + 2 <= r1.y
                || r1.y + r1.h + 2 <= r2.y;
            if apart {
                inf.add_cavern(&mut rng, r2);
                let side = (first_side + 1 + rng.next_index(3)) % 4;
                inf.add_hall(&mut rng, seed, &sides, side, 1);
                break;
            }
        }
        inf.add_crevices(&mut rng);
        inf
    }

    fn add_cavern(&mut self, rng: &mut Rng, rect: Rect) {
        let (rx, ry) = (rect.w as f32 / 2.0, rect.h as f32 / 2.0);
        let (fx, fy) = (rect.x as f32 + rx, rect.y as f32 + ry);
        let center = Cell::new(fx.floor() as i32, fy.floor() as i32);
        // coherent rim roughness: 12 angle bins, interpolated
        let bins: Vec<f32> = (0..12).map(|_| 0.86 + rng.next_f32() * 0.22).collect();
        let mut cells = vec![];
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                let (nx, ny) = ((x as f32 + 0.5 - fx) / rx, (y as f32 + 0.5 - fy) / ry);
                let r = (nx * nx + ny * ny).sqrt();
                let a = (ny.atan2(nx) + std::f32::consts::PI) / (2.0 * std::f32::consts::PI) * 12.0;
                let (i, t) = (a.floor() as usize % 12, a.fract());
                let edge = bins[i] * (1.0 - t) + bins[(i + 1) % 12] * t;
                let middle = (x - center.x).abs() <= 3 && (y - center.y).abs() <= 3;
                if r <= edge || middle {
                    cells.push(Cell::new(x, y));
                    let i = y as usize * N + x as usize;
                    self.open[i] = true;
                    // basin in the middle, ledge at the rim, smoothstep slope between
                    self.height[i] = -BASIN_DEPTH * (1.0 - smoothstep(0.35, 0.8, r));
                }
            }
        }
        self.caverns.push(Cavern { rect, center, cells });
    }

    /// An L-shaped 5-wide hall from the cavern's centre to the chunk edge on `side`, along a seam offset.
    fn add_hall(&mut self, rng: &mut Rng, seed: &str, sides: &[String; 4], side: usize, which: usize) {
        let c = self.caverns[which].center;
        let offs = seam_offsets(seed, &sides[side]);
        let o = offs[rng.next_index(offs.len())];
        let half = HALL_WIDTH / 2;
        let mut rects = vec![];
        if side < 2 {
            // a column at c.x joins the cavern to row o, which runs to the left/right edge
            let (y0, y1) = (c.y.min(o) - half, c.y.max(o) + half);
            rects.push(Rect { x: c.x - half, y: y0, w: HALL_WIDTH, h: y1 - y0 + 1 });
            let (x0, x1) = if side == 0 { (0, c.x + half) } else { (c.x - half, CHUNK - 1) };
            rects.push(Rect { x: x0, y: o - half, w: x1 - x0 + 1, h: HALL_WIDTH });
        } else {
            let (x0, x1) = (c.x.min(o) - half, c.x.max(o) + half);
            rects.push(Rect { x: x0, y: c.y - half, w: x1 - x0 + 1, h: HALL_WIDTH });
            let (y0, y1) = if side == 2 { (0, c.y + half) } else { (c.y - half, CHUNK - 1) };
            rects.push(Rect { x: o - half, y: y0, w: HALL_WIDTH, h: y1 - y0 + 1 });
        }
        for r in rects {
            for y in r.y.max(0)..(r.y + r.h).min(CHUNK) {
                for x in r.x.max(0)..(r.x + r.w).min(CHUNK) {
                    // hall cells keep a cavern's height where they overlap one, else 0.0 (ledge)
                    self.open[y as usize * N + x as usize] = true;
                }
            }
            self.halls.push(r);
        }
    }

    /// Up to 3 one-cell dead ends cut outward from the rim.
    fn add_crevices(&mut self, rng: &mut Rng) {
        let want = rng.next_index(4);
        let mut tries = 0;
        while self.crevices.len() < want && tries < 40 {
            tries += 1;
            let ci = rng.next_index(self.caverns.len());
            let cells = &self.caverns[ci].cells;
            let c = cells[rng.next_index(cells.len())];
            let (dx, dy) = [(1, 0), (-1, 0), (0, 1), (0, -1)][rng.next_index(4)];
            let (nx, ny) = (c.x + dx, c.y + dy);
            if !(2..CHUNK - 2).contains(&nx) || !(2..CHUNK - 2).contains(&ny) || self.is_open(Cell::new(nx, ny)) {
                continue;
            }
            // a dead end: the only open neighbour of the new cell is the rim cell it leaves
            let open_around = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .filter(|(ax, ay)| self.is_open(Cell::new(nx + ax, ny + ay)))
                .count();
            if open_around != 1 {
                continue;
            }
            self.open[ny as usize * N + nx as usize] = true;
            self.crevices.push(Cell::new(nx, ny));
        }
    }

    /// Whether any hall reaches the chunk border.
    pub fn hall_reaches_edge(&self) -> bool {
        self.halls.iter().any(|r| r.touches_edge())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::loot::LootTables;
    use crate::world::catalog::Catalog;
    use crate::world::infinite::{chunk, INFERNO_LEVEL};

    const WORLDS: [&str; 3] = ["1426541270", "4157433391", "2899547009"];

    #[test]
    fn caverns_have_a_clear_middle_a_hall_to_the_edge_and_two_heights() {
        for world in WORLDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    let k = ChunkKey { cx, cy, level: INFERNO_LEVEL };
                    let inf = Inferno::carve(world, k);
                    let again = Inferno::carve(world, k);
                    assert_eq!(inf.open_cells(), again.open_cells(), "{world} {cx},{cy}: not deterministic");
                    assert!(!inf.caverns.is_empty() && inf.caverns.len() <= 2);
                    let big = &inf.caverns[0];
                    assert!((16..=22).contains(&big.rect.w) && (16..=22).contains(&big.rect.h));
                    for cv in &inf.caverns {
                        for dy in -3..=3 {
                            for dx in -3..=3 {
                                assert!(
                                    inf.is_open(Cell::new(cv.center.x + dx, cv.center.y + dy)),
                                    "{world} {cx},{cy}: 7x7 not clear"
                                );
                            }
                        }
                    }
                    assert!(inf.hall_reaches_edge(), "{world} {cx},{cy}: hall misses the edge");
                    assert!(inf.crevices.len() <= 3);
                    let tier = |t: Tier| big.cells.iter().filter(|c| inf.tier_at(**c) == t).count();
                    assert!(tier(Tier::Ledge) > 0 && tier(Tier::Basin) > 0, "{world} {cx},{cy}: needs ledge and basin");
                    assert_eq!(inf.tier_at(big.center), Tier::Basin);
                    assert_eq!(Inferno::height(world, k, big.center), -BASIN_DEPTH);
                }
            }
        }
    }

    #[test]
    fn the_inferno_floor_opens_the_caverns_and_keeps_the_up_stair() {
        let cat = Catalog::load();
        let loot = LootTables::default();
        for world in WORLDS {
            for (cx, cy) in [(0, 0), (1, -1), (-2, 2)] {
                let k = ChunkKey { cx, cy, level: INFERNO_LEVEL };
                let (f, _) = chunk(world, k, &cat, &loot);
                let inf = Inferno::carve(world, k);
                for c in inf.open_cells() {
                    assert!(!f.is_wall_cell(c.x, c.y), "{world} {cx},{cy}: carved cell {c:?} is a wall");
                }
                if (cx, cy) == (0, 0) {
                    assert!(f.stairs.iter().any(|s| s.dir > 0), "{world}: the up stair is gone");
                }
            }
        }
    }
}
