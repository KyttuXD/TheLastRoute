//! Level -3 caverns (port of game/world/regions/inferno_caverns.gd, docs/INFERNO_CAVERNS.md): every chunk
//! gets one cavern 16-22 cells across (a second 14-16 when it fits), a rough ellipse with a clear 7x7 middle,
//! a 5-wide hall from each cavern to the chunk edge, and up to 3 one-cell crevices cut into the rim.
//!
//! Heights: the rim and the cavern edge are an upper ledge (0.0, the maze floor), the middle is a lower
//! basin (`-basin_depth()`), with a smoothstep slope between. Walking stays 2D; the renderer can read
//! [`Inferno::height`] to draw the ledges and basin later.

use super::infinite::{ChunkKey, CHUNK, EDGE_OFFSETS};
use super::rng::Rng;
use super::Cell;

const N: usize = CHUNK as usize;
const FILE: &str = include_str!("../data/inferno.json");

/// Inclusive integer range from a two-number JSON array.
fn span(v: &serde_json::Value, k: &str) -> (i32, i32) {
    let a = &v[k];
    (a[0].as_i64().expect("inferno.json span") as i32, a[1].as_i64().expect("inferno.json span") as i32)
}

fn draw(rng: &mut Rng, s: (i32, i32)) -> i32 {
    s.0 + rng.next_index((s.1 - s.0 + 1).max(1) as usize) as i32
}

/// Every level -3 tunable (data/inferno.json).
#[derive(Clone, Debug)]
pub struct InfernoRules {
    pub basin_depth: f32,
    pub margin: i32,
    pub hall_width: i32,
    pub caves_enabled: bool,
    pub cavern_w: (i32, i32),
    pub cavern_h: (i32, i32),
    pub clear_half: i32,
    pub rim_min: f32,
    pub rim_spread: f32,
    pub slope_from: f32,
    pub slope_to: f32,
    pub second_w: (i32, i32),
    pub second_h: (i32, i32),
    pub second_gap: i32,
    pub second_attempts: i32,
    pub crevices_max: usize,
    pub tier_ledge_frac: f32,
    pub tier_basin_frac: f32,
    /// The noisy floor and ceiling (`inferno_terrain`).
    pub terrain: super::inferno_terrain::TerrainRules,
}

impl InfernoRules {
    pub fn parse(text: &str) -> InfernoRules {
        let v: serde_json::Value = serde_json::from_str(text).expect("inferno.json parses");
        let f = |x: &serde_json::Value, k: &str| x[k].as_f64().unwrap_or_else(|| panic!("inferno.json: {k} missing"));
        let i = |x: &serde_json::Value, k: &str| f(x, k) as i32;
        let (c, s) = (&v["cavern"], &v["second_cavern"]);
        InfernoRules {
            basin_depth: f(&v, "basin_depth") as f32,
            margin: i(&v, "margin"),
            hall_width: i(&v, "hall_width").max(1),
            caves_enabled: v["caves_enabled"].as_bool().unwrap_or(false),
            cavern_w: span(c, "width"),
            cavern_h: span(c, "height"),
            clear_half: i(c, "clear_half"),
            rim_min: f(c, "rim_min") as f32,
            rim_spread: f(c, "rim_spread") as f32,
            slope_from: f(c, "slope_from") as f32,
            slope_to: f(c, "slope_to") as f32,
            second_w: span(s, "width"),
            second_h: span(s, "height"),
            second_gap: i(s, "gap"),
            second_attempts: i(s, "attempts"),
            crevices_max: i(&v, "crevices_max").max(0) as usize,
            tier_ledge_frac: f(&v, "tier_ledge_frac") as f32,
            tier_basin_frac: f(&v, "tier_basin_frac") as f32,
            terrain: super::inferno_terrain::TerrainRules::parse(&v["terrain"]).unwrap_or_else(|e| panic!("{e}")),
        }
    }

    /// The embedded rules.
    pub fn builtin() -> &'static InfernoRules {
        static R: std::sync::OnceLock<InfernoRules> = std::sync::OnceLock::new();
        R.get_or_init(|| InfernoRules::parse(FILE))
    }
}

/// How far the basin floor sits below the ledge, in metres (data).
pub fn basin_depth() -> f32 {
    InfernoRules::builtin().basin_depth
}

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
    rules: InfernoRules,
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
    /// down to `-basin_depth()` in a basin. Carves the chunk each call; keep an `Inferno` to ask often.
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
        let d = self.rules.basin_depth;
        if h > -self.rules.tier_ledge_frac * d {
            Tier::Ledge
        } else if h < -self.rules.tier_basin_frac * d {
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
        Inferno::carve_with(seed, k, InfernoRules::builtin())
    }

    pub fn carve_with(seed: &str, k: ChunkKey, rules: &InfernoRules) -> Inferno {
        let margin = rules.margin;
        let text = format!("{seed}:L{}:C{},{}", k.level, k.cx, k.cy);
        let mut rng = Rng::new(&format!("{text}:INFERNO"));
        let mut inf = Inferno {
            caverns: vec![],
            halls: vec![],
            crevices: vec![],
            open: vec![false; N * N],
            height: vec![0.0; N * N],
            rules: rules.clone(),
        };
        // seam keys per side: 0 left, 1 right, 2 top, 3 bottom
        let sides = [
            format!("V:{}:{}:{}", k.level, k.cx, k.cy),
            format!("V:{}:{}:{}", k.level, k.cx + 1, k.cy),
            format!("H:{}:{}:{}", k.level, k.cx, k.cy),
            format!("H:{}:{}:{}", k.level, k.cx, k.cy + 1),
        ];
        let first_side = rng.next_index(4);
        let w = draw(&mut rng, rules.cavern_w);
        let h = draw(&mut rng, rules.cavern_h);
        let r1 = Rect {
            x: margin + rng.next_index((CHUNK - 2 * margin - w + 1).max(1) as usize) as i32,
            y: margin + rng.next_index((CHUNK - 2 * margin - h + 1).max(1) as usize) as i32,
            w,
            h,
        };
        inf.add_cavern(&mut rng, r1);
        inf.add_hall(&mut rng, seed, &sides, first_side, 0);
        // a second, smaller cavern when it fits (a 2-cell gap to the first)
        let (w2, h2) = (draw(&mut rng, rules.second_w), draw(&mut rng, rules.second_h));
        let gap = rules.second_gap;
        for _ in 0..rules.second_attempts {
            let r2 = Rect {
                x: margin + rng.next_index((CHUNK - 2 * margin - w2 + 1).max(1) as usize) as i32,
                y: margin + rng.next_index((CHUNK - 2 * margin - h2 + 1).max(1) as usize) as i32,
                w: w2,
                h: h2,
            };
            let apart = r2.x + r2.w + gap <= r1.x
                || r1.x + r1.w + gap <= r2.x
                || r2.y + r2.h + gap <= r1.y
                || r1.y + r1.h + gap <= r2.y;
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
        let (rim_min, rim_spread, clear, depth) =
            (self.rules.rim_min, self.rules.rim_spread, self.rules.clear_half, self.rules.basin_depth);
        let (s_from, s_to) = (self.rules.slope_from, self.rules.slope_to);
        let bins: Vec<f32> = (0..12).map(|_| rim_min + rng.next_f32() * rim_spread).collect();
        let mut cells = vec![];
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                let (nx, ny) = ((x as f32 + 0.5 - fx) / rx, (y as f32 + 0.5 - fy) / ry);
                let r = (nx * nx + ny * ny).sqrt();
                let a = (ny.atan2(nx) + std::f32::consts::PI) / (2.0 * std::f32::consts::PI) * 12.0;
                let (i, t) = (a.floor() as usize % 12, a.fract());
                let edge = bins[i] * (1.0 - t) + bins[(i + 1) % 12] * t;
                let middle = (x - center.x).abs() <= clear && (y - center.y).abs() <= clear;
                if r <= edge || middle {
                    cells.push(Cell::new(x, y));
                    let i = y as usize * N + x as usize;
                    self.open[i] = true;
                    // basin in the middle, ledge at the rim, smoothstep slope between
                    self.height[i] = -depth * (1.0 - smoothstep(s_from, s_to, r));
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
        let hall_width = self.rules.hall_width;
        let half = hall_width / 2;
        let mut rects = vec![];
        if side < 2 {
            // a column at c.x joins the cavern to row o, which runs to the left/right edge
            let (y0, y1) = (c.y.min(o) - half, c.y.max(o) + half);
            rects.push(Rect { x: c.x - half, y: y0, w: hall_width, h: y1 - y0 + 1 });
            let (x0, x1) = if side == 0 { (0, c.x + half) } else { (c.x - half, CHUNK - 1) };
            rects.push(Rect { x: x0, y: o - half, w: x1 - x0 + 1, h: hall_width });
        } else {
            let (x0, x1) = (c.x.min(o) - half, c.x.max(o) + half);
            rects.push(Rect { x: x0, y: c.y - half, w: x1 - x0 + 1, h: hall_width });
            let (y0, y1) = if side == 2 { (0, c.y + half) } else { (c.y - half, CHUNK - 1) };
            rects.push(Rect { x: o - half, y: y0, w: hall_width, h: y1 - y0 + 1 });
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
        let want = rng.next_index(self.rules.crevices_max + 1);
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
                    assert_eq!(Inferno::height(world, k, big.center), -basin_depth());
                }
            }
        }
    }

    #[test]
    fn changed_data_changes_the_carve() {
        let base = InfernoRules::builtin().clone();
        let mut deep = base.clone();
        deep.basin_depth = 2.4;
        let mut narrow = base.clone();
        narrow.hall_width = 3;
        let k = ChunkKey { cx: 0, cy: 0, level: INFERNO_LEVEL };
        let (a, b, c) = (
            Inferno::carve_with(WORLDS[0], k, &base),
            Inferno::carve_with(WORLDS[0], k, &deep),
            Inferno::carve_with(WORLDS[0], k, &narrow),
        );
        let mid = a.caverns[0].center;
        assert!(b.height_at(mid) < a.height_at(mid), "a deeper basin must lower the middle");
        assert!(c.open_cells().len() < a.open_cells().len(), "narrower halls must open fewer cells");
        // Standard values match the game
        assert_eq!(base.basin_depth, 1.2);
        assert_eq!((base.margin, base.hall_width), (3, 5));
    }

    #[test]
    fn data_switch_picks_caves_or_the_old_carve_and_lava_stays_closed() {
        let k = ChunkKey { cx: 1, cy: 0, level: INFERNO_LEVEL };
        let mut on = InfernoRules::builtin().clone();
        on.caves_enabled = true;
        let mut off = on.clone();
        off.caves_enabled = false;
        for world in WORLDS {
            let a = InfernoChunk::carve_with(world, k, &on);
            let caves = a.caves.as_ref().expect("caves on");
            assert_eq!(a.open_cells(), caves.walkable_cells());
            for l in a.lava_cells() {
                assert!(!a.open_cells().contains(&l) && a.is_lava(l));
                assert_eq!(a.tier_at(l), Tier::Basin);
            }
            let b = InfernoChunk::carve_with(world, k, &off);
            assert!(b.caves.is_none() && b.lava_cells().is_empty());
            assert_eq!(b.open_cells(), Inferno::carve_with(world, k, &off).open_cells());
        }
    }

    #[test]
    fn the_inferno_chunk_keeps_stairs_and_exits_reachable() {
        let cat = Catalog::load();
        let loot = LootTables::default();
        for world in WORLDS {
            for (cx, cy) in [(0, 0), (1, -1), (-2, 2)] {
                let k = ChunkKey { cx, cy, level: INFERNO_LEVEL };
                let (f, _) = chunk(world, k, &cat, &loot);
                let mut seen = std::collections::HashSet::new();
                let mut stack = vec![f.start];
                seen.insert((f.start.x, f.start.y));
                while let Some(c) = stack.pop() {
                    for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (x, y) = (c.x + dx, c.y + dy);
                        if x >= 0 && y >= 0 && x < CHUNK && y < CHUNK && !f.is_wall_cell(x, y) && seen.insert((x, y)) {
                            stack.push(Cell::new(x, y));
                        }
                    }
                }
                for s in &f.stairs {
                    assert!(seen.contains(&(s.cell.x, s.cell.y)), "{world} {cx},{cy}: stair {:?} unreachable", s.cell);
                }
                let ic = InfernoChunk::carve(world, k);
                if let Some(caves) = ic.caves.as_ref().filter(|c| !c.spawns.is_empty()) {
                    for e in &caves.exits {
                        assert!(caves.reachable(caves.spawns[0])[e.y as usize * CHUNK as usize + e.x as usize], "exit cut off");
                    }
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
                // the build opens what InfernoChunk carves (caves, or the old carve when the data disables them)
                let inf = InfernoChunk::carve(world, k);
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

/// One level -3 chunk as the build and the renderer see it: the cavern carve (the fallback, and the data
/// switch `caves_enabled` picks between them) or the chamber/passage/lava caves of `inferno_caves`.
#[derive(Clone, Debug)]
pub struct InfernoChunk {
    pub inferno: Inferno,
    pub caves: Option<super::inferno_caves::Caves>,
}

impl InfernoChunk {
    pub fn carve(seed: &str, k: ChunkKey) -> InfernoChunk {
        InfernoChunk::carve_with(seed, k, InfernoRules::builtin())
    }

    pub fn carve_with(seed: &str, k: ChunkKey, rules: &InfernoRules) -> InfernoChunk {
        InfernoChunk {
            inferno: Inferno::carve_with(seed, k, rules),
            caves: rules.caves_enabled.then(|| super::inferno_caves::Caves::carve(seed, k)),
        }
    }

    /// Cells the build opens (walkable only: lava stays closed).
    pub fn open_cells(&self) -> Vec<Cell> {
        match &self.caves {
            Some(c) => c.walkable_cells(),
            None => self.inferno.open_cells(),
        }
    }

    /// Rectangles whose props the build clears.
    pub fn clear_rects(&self) -> Vec<Rect> {
        match &self.caves {
            Some(c) => c.chambers.iter().map(|m| Rect { x: m.x, y: m.y, w: m.w, h: m.h }).collect(),
            None => self.inferno.caverns.iter().map(|c| c.rect).chain(self.inferno.halls.iter().copied()).collect(),
        }
    }

    /// Lava cells (empty on the cavern carve). A hazard, never walkable.
    pub fn lava_cells(&self) -> Vec<Cell> {
        self.caves.as_ref().map(|c| c.lava_cells()).unwrap_or_default()
    }

    pub fn is_lava(&self, c: Cell) -> bool {
        self.caves.as_ref().is_some_and(|v| v.is_lava(c))
    }

    /// Elevation band of a chunk-local cell. On caves: lava and a chamber's middle are basin, its outer
    /// ring is ledge, the band between is slope; passages are ledge.
    pub fn tier_at(&self, c: Cell) -> Tier {
        let Some(caves) = &self.caves else { return self.inferno.tier_at(c) };
        if caves.is_lava(c) {
            return Tier::Basin;
        }
        for m in &caves.chambers {
            if c.x >= m.x && c.x < m.x + m.w && c.y >= m.y && c.y < m.y + m.h {
                let half = (m.w.max(m.h) as f32 / 2.0).max(1.0);
                let d = (c.x - m.center.x).abs().max((c.y - m.center.y).abs()) as f32 / half;
                return if d < 0.5 {
                    Tier::Basin
                } else if d < 0.85 {
                    Tier::Slope
                } else {
                    Tier::Ledge
                };
            }
        }
        Tier::Ledge
    }

    /// Floor height in metres for the cell's tier.
    pub fn height_at(&self, c: Cell) -> f32 {
        if self.caves.is_none() {
            return self.inferno.height_at(c);
        }
        let d = self.inferno.rules.basin_depth;
        match self.tier_at(c) {
            Tier::Ledge => 0.0,
            Tier::Slope => -d * 0.5,
            Tier::Basin => -d,
        }
    }
}
