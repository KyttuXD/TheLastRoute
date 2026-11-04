//! Catwalks above level 0: a walkway grid over the maze with railings, gaps and landings.
//! Layout rules live in `data/regions/catwalk.json`; this file holds behaviour only. Pure: seed text in, grid out.
//! Every walkable cell is reachable from a landing (cells cut off by gaps are pruned back to void).

use std::collections::VecDeque;
use std::sync::OnceLock;

use serde_json::Value;

use crate::world::rng::Rng;

pub const VOID: u8 = 0;
pub const WALK: u8 = 1;
pub const LANDING: u8 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct CatwalkParams {
    pub size: usize,
    pub spacing: usize,
    pub margin: usize,
    pub width: usize,
    pub gap_chance: f64,
    pub gap_length: usize,
    pub landing_size: usize,
    pub min_landings: usize,
    pub height: f32,
    pub rail_height: f32,
}

fn num(v: &Value, k: &str) -> Result<f64, String> {
    v.get(k).and_then(Value::as_f64).ok_or_else(|| format!("catwalk.json: \"{k}\" must be a number"))
}

impl CatwalkParams {
    pub fn parse(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("catwalk.json: {e}"))?;
        let u = |k: &str| num(&v, k).map(|n| n.max(0.0) as usize);
        let p = CatwalkParams {
            size: u("size")?,
            spacing: u("spacing")?,
            margin: u("margin")?,
            width: u("width")?,
            gap_chance: num(&v, "gap_chance")?.clamp(0.0, 1.0),
            gap_length: u("gap_length")?,
            landing_size: u("landing_size")?,
            min_landings: u("min_landings")?,
            height: num(&v, "height")? as f32,
            rail_height: num(&v, "rail_height")? as f32,
        };
        if p.size < 8 || p.spacing < 3 || p.width == 0 || p.landing_size < p.width || p.margin * 2 + p.landing_size >= p.size {
            return Err("catwalk.json: size, spacing, width, landing_size or margin out of range".into());
        }
        Ok(p)
    }
}

/// The embedded rules.
pub fn params() -> &'static CatwalkParams {
    static P: OnceLock<CatwalkParams> = OnceLock::new();
    P.get_or_init(|| CatwalkParams::parse(include_str!("../../data/regions/catwalk.json")).expect("catwalk.json"))
}

#[derive(Clone, Debug)]
pub struct Catwalk {
    pub size: usize,
    /// `cells[z][x]`: VOID, WALK or LANDING.
    pub cells: Vec<Vec<u8>>,
}

impl Catwalk {
    pub fn at(&self, x: i32, z: i32) -> u8 {
        if x < 0 || z < 0 || x as usize >= self.size || z as usize >= self.size { VOID } else { self.cells[z as usize][x as usize] }
    }
    pub fn walkable(&self, x: i32, z: i32) -> bool { self.at(x, z) != VOID }
    /// A railing stands on each open side of a walkable cell (north, east, south, west = -z, +x, +z, -x).
    pub fn rails(&self, x: i32, z: i32) -> [bool; 4] {
        if !self.walkable(x, z) { return [false; 4]; }
        [(0, -1), (1, 0), (0, 1), (-1, 0)].map(|(dx, dz)| !self.walkable(x + dx, z + dz))
    }
    pub fn landings(&self) -> Vec<(usize, usize)> {
        let mut out = vec![];
        for z in 0..self.size { for x in 0..self.size { if self.cells[z][x] == LANDING { out.push((x, z)); } } }
        out
    }
}

/// Build the catwalk grid for a seed text with the given rules.
pub fn generate_with(seed: &str, p: &CatwalkParams) -> Catwalk {
    let n = p.size;
    let mut rng = Rng::new(&format!("{seed}:CATWALK"));
    let mut cells = vec![vec![VOID; n]; n];
    let lo = p.margin;
    let hi = n - p.margin;
    let lines: Vec<usize> = (lo..hi).step_by(p.spacing).collect();
    let on_line = |t: usize| lines.iter().any(|&m| t >= m && t < m + p.width);
    for &l in &lines {
        for t in lo..hi {
            for w in 0..p.width {
                if l + w < hi { cells[l + w][t] = WALK; cells[t][l + w] = WALK; }
            }
        }
    }
    // Gaps: cut a short run out of a line; the run never covers a crossing.
    for &l in &lines {
        let mut t = lo + p.landing_size;
        while t + p.gap_length < hi.saturating_sub(p.landing_size) {
            if rng.next_float() < p.gap_chance {
                for k in 0..p.gap_length {
                    if on_line(t + k) { continue; }
                    for w in 0..p.width {
                        if l + w < hi { cells[l + w][t + k] = VOID; cells[t + k][l + w] = VOID; }
                    }
                }
                t += p.gap_length + 1;
            } else { t += 1; }
        }
    }
    // Landings at lattice corners.
    let first = lines[0];
    let last = *lines.last().unwrap();
    let spots = [(first, first), (last, last), (first, last), (last, first)];
    let count = p.min_landings.clamp(1, spots.len());
    for &(x, z) in spots.iter().take(count) {
        for dz in 0..p.landing_size { for dx in 0..p.landing_size {
            let (cx, cz) = (x + dx, z + dz);
            if cx < hi && cz < hi { cells[cz][cx] = LANDING; }
        } }
    }
    // Prune whatever the landings cannot reach.
    let mut seen = vec![vec![false; n]; n];
    let mut q = VecDeque::new();
    for z in 0..n { for x in 0..n { if cells[z][x] == LANDING { seen[z][x] = true; q.push_back((x, z)); } } }
    while let Some((x, z)) = q.pop_front() {
        for (dx, dz) in [(0i32, -1i32), (1, 0), (0, 1), (-1, 0)] {
            let (nx, nz) = (x as i32 + dx, z as i32 + dz);
            if nx < 0 || nz < 0 || nx as usize >= n || nz as usize >= n { continue; }
            let (nx, nz) = (nx as usize, nz as usize);
            if cells[nz][nx] != VOID && !seen[nz][nx] { seen[nz][nx] = true; q.push_back((nx, nz)); }
        }
    }
    for z in 0..n { for x in 0..n { if !seen[z][x] { cells[z][x] = VOID; } } }
    Catwalk { size: n, cells }
}

pub fn generate(seed: &str) -> Catwalk { generate_with(seed, params()) }

#[cfg(test)]
mod tests {
    use super::*;

    fn reach(c: &Catwalk) -> usize {
        let n = c.size;
        let mut seen = vec![vec![false; n]; n];
        let mut q: VecDeque<(i32, i32)> = c.landings().iter().map(|&(x, z)| (x as i32, z as i32)).collect();
        for &(x, z) in &q { seen[z as usize][x as usize] = true; }
        let mut count = q.len();
        while let Some((x, z)) = q.pop_front() {
            for (dx, dz) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (nx, nz) = (x + dx, z + dz);
                if c.walkable(nx, nz) && !seen[nz as usize][nx as usize] { seen[nz as usize][nx as usize] = true; count += 1; q.push_back((nx, nz)); }
            }
        }
        count
    }

    fn count(c: &Catwalk) -> usize { c.cells.iter().flatten().filter(|&&v| v != VOID).count() }

    #[test]
    fn data_parses() { assert!(params().min_landings >= 1); }

    #[test]
    fn every_cell_reachable_from_a_landing() {
        for s in ["a", "ROOM-X-1", "seed-42", "zzz"] {
            let c = generate(s);
            assert!(c.landings().len() >= params().min_landings);
            assert_eq!(reach(&c), count(&c), "seed {s}");
        }
    }

    #[test]
    fn gap_chance_changes_layout_and_is_deterministic() {
        let mut p = params().clone();
        p.gap_chance = 0.0;
        let solid = generate_with("s", &p);
        p.gap_chance = 0.9;
        let holey = generate_with("s", &p);
        assert!(count(&holey) < count(&solid));
        assert_eq!(generate("s").cells, generate("s").cells);
    }

    #[test]
    fn open_edges_have_rails() {
        let c = generate("rails");
        let any = (0..c.size as i32).flat_map(|z| (0..c.size as i32).map(move |x| (x, z))).any(|(x, z)| c.rails(x, z).iter().any(|r| *r));
        assert!(any);
        assert_eq!(c.rails(0, 0), [false; 4]);
    }
}
