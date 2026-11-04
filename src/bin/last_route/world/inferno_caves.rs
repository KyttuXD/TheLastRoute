//! Inferno cave layout rules: winding passages, chambers and lava channels, read from
//! `data/regions/inferno_caves.json` (`layout`). Code holds only the behaviour; every count, size and
//! chance is data. Pure and deterministic from the chunk's seed text. Lava cells are open but not
//! walkable; bridges keep every chamber reachable. Spawn and loot cells come from the same file.
//!
//! Nothing calls this yet; the hooks are in the stage report.

use super::infinite::{ChunkKey, CHUNK, EDGE_OFFSETS};
use super::rng::Rng;
use super::Cell;
use serde_json::Value;

const N: usize = CHUNK as usize;
const FILE: &str = include_str!("../../../../data/regions/inferno_caves.json");

/// Inclusive integer range from a two-number JSON array.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span(pub i32, pub i32);

#[derive(Clone, Debug)]
pub struct CaveRules {
    pub margin: i32,
    pub chamber_count: Span,
    pub chamber_w: Span,
    pub chamber_h: Span,
    pub chamber_gap: i32,
    pub clear_half: i32,
    pub attempts: i32,
    pub passage_half: i32,
    pub passage_wander: f64,
    pub passage_steps: i32,
    pub exits: i32,
    pub exit_half: i32,
    pub lava_channels: Span,
    pub lava_half: i32,
    pub lava_wander: f64,
    pub bridge_every: i32,
    pub bridge_len: i32,
    pub lava_core_clear: i32,
    pub spawn_per_chamber: Span,
    pub spawn_spacing: i32,
    pub hunter_chance: f64,
    pub loot_table: String,
    pub loot_chamber_chance: f64,
    pub loot_passage_chance: f64,
    pub loot_passage_spacing: i32,
    pub loot_max: usize,
}

fn int(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap_or_else(|| panic!("inferno_caves.json: {k} missing")) as i32
}
fn num(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or_else(|| panic!("inferno_caves.json: {k} missing"))
}
fn span(v: &Value, k: &str) -> Span {
    let a = &v[k];
    Span(a[0].as_i64().expect("span") as i32, a[1].as_i64().expect("span") as i32)
}

impl CaveRules {
    /// The built-in rules (embedded file).
    pub fn builtin() -> CaveRules {
        CaveRules::parse(FILE)
    }

    pub fn parse(text: &str) -> CaveRules {
        let root: Value = serde_json::from_str(text).expect("inferno_caves.json parses");
        let l = &root["layout"];
        let (c, p, la, s, lo) = (&l["chambers"], &l["passages"], &l["lava"], &l["spawn"], &l["loot"]);
        CaveRules {
            margin: int(l, "margin"),
            chamber_count: span(c, "count"),
            chamber_w: span(c, "width"),
            chamber_h: span(c, "height"),
            chamber_gap: int(c, "gap"),
            clear_half: int(c, "clear_half"),
            attempts: int(c, "attempts"),
            passage_half: int(p, "half_width"),
            passage_wander: num(p, "wander_chance"),
            passage_steps: int(p, "max_steps"),
            exits: int(p, "exits"),
            exit_half: int(p, "exit_half_width"),
            lava_channels: span(la, "channels"),
            lava_half: int(la, "half_width"),
            lava_wander: num(la, "wander_chance"),
            bridge_every: int(la, "bridge_every").max(2),
            bridge_len: int(la, "bridge_len").max(1),
            lava_core_clear: int(la, "keep_clear_of_chamber_core"),
            spawn_per_chamber: span(s, "per_chamber"),
            spawn_spacing: int(s, "min_spacing"),
            hunter_chance: num(s, "hunter_chance"),
            loot_table: lo["table"].as_str().unwrap_or("route").to_string(),
            loot_chamber_chance: num(lo, "per_chamber_chance"),
            loot_passage_chance: num(lo, "per_passage_chance"),
            loot_passage_spacing: int(lo, "passage_spacing").max(1),
            loot_max: int(lo, "max_per_chunk").max(0) as usize,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chamber {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub center: Cell,
}

#[derive(Clone, Debug)]
pub struct Caves {
    pub chambers: Vec<Chamber>,
    /// Cells where a player or hunter may start, apart from each other.
    pub spawns: Vec<Cell>,
    /// Subset of `spawns` that may hold a hunter (rolled from `hunter_chance`).
    pub hunter_spawns: Vec<Cell>,
    pub loot: Vec<Cell>,
    /// Cells where a passage meets the chunk edge.
    pub exits: Vec<Cell>,
    open: Vec<bool>,
    lava: Vec<bool>,
    passage: Vec<bool>,
}

fn idx(c: Cell) -> usize {
    c.y as usize * N + c.x as usize
}
fn inside(c: Cell) -> bool {
    c.x >= 0 && c.y >= 0 && c.x < CHUNK && c.y < CHUNK
}

/// Same seam draw as inferno.rs, so an exit lines up with the neighbour's.
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

fn draw(rng: &mut Rng, s: Span) -> i32 {
    s.0 + rng.next_index((s.1 - s.0 + 1).max(1) as usize) as i32
}

impl Caves {
    /// Carves with the built-in rules.
    pub fn carve(seed: &str, k: ChunkKey) -> Caves {
        Caves::carve_with(seed, k, &CaveRules::builtin())
    }

    pub fn is_open(&self, c: Cell) -> bool {
        inside(c) && self.open[idx(c)]
    }
    pub fn is_lava(&self, c: Cell) -> bool {
        inside(c) && self.lava[idx(c)]
    }
    /// Open and not lava.
    pub fn is_walkable(&self, c: Cell) -> bool {
        self.is_open(c) && !self.lava[idx(c)]
    }
    pub fn is_passage(&self, c: Cell) -> bool {
        inside(c) && self.passage[idx(c)]
    }
    pub fn walkable_cells(&self) -> Vec<Cell> {
        (0..N * N)
            .filter(|i| self.open[*i] && !self.lava[*i])
            .map(|i| Cell::new((i % N) as i32, (i / N) as i32))
            .collect()
    }
    pub fn lava_cells(&self) -> Vec<Cell> {
        (0..N * N).filter(|i| self.lava[*i]).map(|i| Cell::new((i % N) as i32, (i / N) as i32)).collect()
    }

    /// Cells reachable on foot (4-neighbour) from `from`.
    pub fn reachable(&self, from: Cell) -> Vec<bool> {
        let mut seen = vec![false; N * N];
        if !self.is_walkable(from) {
            return seen;
        }
        let mut stack = vec![from];
        seen[idx(from)] = true;
        while let Some(c) = stack.pop() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = Cell::new(c.x + dx, c.y + dy);
                if self.is_walkable(n) && !seen[idx(n)] {
                    seen[idx(n)] = true;
                    stack.push(n);
                }
            }
        }
        seen
    }

    pub fn carve_with(seed: &str, k: ChunkKey, r: &CaveRules) -> Caves {
        let text = format!("{seed}:L{}:C{},{}", k.level, k.cx, k.cy);
        let mut rng = Rng::new(&format!("{text}:CAVES"));
        let mut cv = Caves {
            chambers: vec![],
            spawns: vec![],
            hunter_spawns: vec![],
            loot: vec![],
            exits: vec![],
            open: vec![false; N * N],
            lava: vec![false; N * N],
            passage: vec![false; N * N],
        };
        cv.place_chambers(&mut rng, r);
        cv.link_chambers(&mut rng, r);
        cv.add_exits(&mut rng, seed, k, r);
        cv.add_lava(&mut rng, r);
        cv.pick_spawns(&mut rng, r);
        cv.pick_loot(&mut rng, r);
        cv
    }

    fn open_disc(&mut self, c: Cell, half: i32, passage: bool) {
        for dy in -half..=half {
            for dx in -half..=half {
                let p = Cell::new(c.x + dx, c.y + dy);
                // keep a one-cell rock border so the chunk edge stays a wall except at exits
                if p.x >= 1 && p.y >= 1 && p.x < CHUNK - 1 && p.y < CHUNK - 1 {
                    self.open[idx(p)] = true;
                    if passage {
                        self.passage[idx(p)] = true;
                    }
                }
            }
        }
    }

    fn place_chambers(&mut self, rng: &mut Rng, r: &CaveRules) {
        let want = draw(rng, r.chamber_count);
        for _ in 0..r.attempts {
            if self.chambers.len() as i32 >= want {
                break;
            }
            let w = draw(rng, r.chamber_w).min(CHUNK - 2 * r.margin);
            let h = draw(rng, r.chamber_h).min(CHUNK - 2 * r.margin);
            let x = r.margin + rng.next_index((CHUNK - 2 * r.margin - w + 1).max(1) as usize) as i32;
            let y = r.margin + rng.next_index((CHUNK - 2 * r.margin - h + 1).max(1) as usize) as i32;
            let g = r.chamber_gap;
            let clash = self
                .chambers
                .iter()
                .any(|o| !(x + w + g <= o.x || o.x + o.w + g <= x || y + h + g <= o.y || o.y + o.h + g <= y));
            if clash {
                continue;
            }
            let center = Cell::new(x + w / 2, y + h / 2);
            let (rx, ry) = (w as f32 / 2.0, h as f32 / 2.0);
            for cy in y..y + h {
                for cx in x..x + w {
                    let (nx, ny) = ((cx as f32 + 0.5 - x as f32 - rx) / rx, (cy as f32 + 0.5 - y as f32 - ry) / ry);
                    let near = (cx - center.x).abs() <= r.clear_half && (cy - center.y).abs() <= r.clear_half;
                    if nx * nx + ny * ny <= 1.0 || near {
                        self.open[idx(Cell::new(cx, cy))] = true;
                    }
                }
            }
            self.chambers.push(Chamber { x, y, w, h, center });
        }
        if self.chambers.is_empty() {
            // every attempt clashed (cannot with a sane file): fall back to one middle chamber
            let c = Cell::new(CHUNK / 2, CHUNK / 2);
            self.open_disc(c, r.clear_half.max(2), false);
            self.chambers.push(Chamber { x: c.x - 2, y: c.y - 2, w: 5, h: 5, center: c });
        }
    }

    /// A winding walk from `a` toward `b`: mostly straight toward it, sometimes a sidestep.
    fn wind(&mut self, rng: &mut Rng, a: Cell, b: Cell, half: i32, wander: f64, max_steps: i32) {
        let mut c = a;
        for _ in 0..max_steps {
            self.open_disc(c, half, true);
            if c == b {
                break;
            }
            let (dx, dy) = ((b.x - c.x).signum(), (b.y - c.y).signum());
            let along_x = if dx == 0 {
                false
            } else if dy == 0 {
                true
            } else {
                rng.next_float() < 0.5
            };
            let (mut sx, mut sy) = if along_x { (dx, 0) } else { (0, dy) };
            if rng.next_float() < wander {
                // sidestep perpendicular, either way
                let sign = if rng.next_float() < 0.5 { 1 } else { -1 };
                (sx, sy) = if sx != 0 { (0, sign) } else { (sign, 0) };
            }
            c = Cell::new((c.x + sx).clamp(2, CHUNK - 3), (c.y + sy).clamp(2, CHUNK - 3));
        }
        self.open_disc(b, half, true);
    }

    fn link_chambers(&mut self, rng: &mut Rng, r: &CaveRules) {
        let cs: Vec<Cell> = self.chambers.iter().map(|c| c.center).collect();
        for i in 1..cs.len() {
            self.wind(rng, cs[i - 1], cs[i], r.passage_half, r.passage_wander, r.passage_steps);
        }
        // one extra loop link when there are three or more chambers
        if cs.len() >= 3 {
            self.wind(rng, cs[cs.len() - 1], cs[0], r.passage_half, r.passage_wander, r.passage_steps);
        }
    }

    fn add_exits(&mut self, rng: &mut Rng, seed: &str, k: ChunkKey, r: &CaveRules) {
        let sides = [
            format!("V:{}:{}:{}", k.level, k.cx, k.cy),
            format!("V:{}:{}:{}", k.level, k.cx + 1, k.cy),
            format!("H:{}:{}:{}", k.level, k.cx, k.cy),
            format!("H:{}:{}:{}", k.level, k.cx, k.cy + 1),
        ];
        let mut used: Vec<usize> = vec![];
        for _ in 0..r.exits.clamp(0, 4) {
            let mut side = rng.next_index(4);
            while used.contains(&side) {
                side = (side + 1) % 4;
            }
            used.push(side);
            let offs = seam_offsets(seed, &sides[side]);
            let o = offs[rng.next_index(offs.len())];
            let from = self.chambers[rng.next_index(self.chambers.len())].center;
            let (inner, edge) = match side {
                0 => (Cell::new(2, o), Cell::new(0, o)),
                1 => (Cell::new(CHUNK - 3, o), Cell::new(CHUNK - 1, o)),
                2 => (Cell::new(o, 2), Cell::new(o, 0)),
                _ => (Cell::new(o, CHUNK - 3), Cell::new(o, CHUNK - 1)),
            };
            self.wind(rng, from, inner, r.passage_half, r.passage_wander, r.passage_steps);
            // straight run through the border to the edge, exit_half wide each side
            let dir = Cell::new((edge.x - inner.x).signum(), (edge.y - inner.y).signum());
            for s in 0..=2 {
                let c = Cell::new(inner.x + dir.x * s, inner.y + dir.y * s);
                for w in -r.exit_half..=r.exit_half {
                    let p = if dir.x != 0 { Cell::new(c.x, c.y + w) } else { Cell::new(c.x + w, c.y) };
                    if inside(p) {
                        self.open[idx(p)] = true;
                        self.passage[idx(p)] = true;
                    }
                }
            }
            self.exits.push(edge);
        }
    }

    fn add_lava(&mut self, rng: &mut Rng, r: &CaveRules) {
        let want = draw(rng, r.lava_channels);
        let mut painted: Vec<Cell> = vec![];
        for _ in 0..want {
            // a channel crosses the chunk along one axis, drifting sideways, with bridges
            let vertical = rng.next_float() < 0.5;
            let mut pos = 6 + rng.next_index((CHUNK - 12) as usize) as i32;
            for t in 1..CHUNK - 1 {
                if rng.next_float() < r.lava_wander {
                    pos = (pos + if rng.next_float() < 0.5 { 1 } else { -1 }).clamp(2, CHUNK - 3);
                }
                // a bridge: bridge_len cells of floor every bridge_every cells
                if t % r.bridge_every < r.bridge_len {
                    continue;
                }
                let c = if vertical { Cell::new(pos, t) } else { Cell::new(t, pos) };
                for w in -r.lava_half..=r.lava_half {
                    let p = if vertical { Cell::new(c.x + w, c.y) } else { Cell::new(c.x, c.y + w) };
                    if inside(p) && self.open[idx(p)] {
                        painted.push(p);
                    }
                }
            }
        }
        for p in painted {
            let near_core = self
                .chambers
                .iter()
                .any(|c| (p.x - c.center.x).abs() <= r.lava_core_clear && (p.y - c.center.y).abs() <= r.lava_core_clear);
            let on_exit = self.exits.iter().any(|e| (p.x - e.x).abs() <= 4 && (p.y - e.y).abs() <= 4);
            if !near_core && !on_exit {
                self.lava[idx(p)] = true;
            }
        }
        // every chamber must stay reachable from the first; if lava cut one off, lay the lava aside
        let seen = self.reachable(self.chambers[0].center);
        let cut = self.chambers.iter().any(|c| !seen[idx(c.center)])
            || self.exits.iter().any(|e| !self.edge_reached(*e, &seen));
        if cut {
            self.lava.iter_mut().for_each(|l| *l = false);
        }
    }

    /// Whether the walkable cell nearest an exit is on the reached set.
    fn edge_reached(&self, e: Cell, seen: &[bool]) -> bool {
        for d in 0..=4 {
            for dy in -d..=d {
                for dx in -d..=d {
                    let p = Cell::new(e.x + dx, e.y + dy);
                    if self.is_walkable(p) {
                        return seen[idx(p)];
                    }
                }
            }
        }
        true
    }

    fn pick_spawns(&mut self, rng: &mut Rng, r: &CaveRules) {
        let chambers = self.chambers.clone();
        for ch in chambers {
            let want = draw(rng, r.spawn_per_chamber);
            let mut got = 0;
            for _ in 0..60 {
                if got >= want {
                    break;
                }
                let c = Cell::new(ch.x + rng.next_index(ch.w as usize) as i32, ch.y + rng.next_index(ch.h as usize) as i32);
                let far = self.spawns.iter().all(|s| (s.x - c.x).abs().max((s.y - c.y).abs()) >= r.spawn_spacing);
                if self.is_walkable(c) && far {
                    self.spawns.push(c);
                    if rng.next_float() < r.hunter_chance {
                        self.hunter_spawns.push(c);
                    }
                    got += 1;
                }
            }
        }
    }

    fn pick_loot(&mut self, rng: &mut Rng, r: &CaveRules) {
        let chambers = self.chambers.clone();
        for ch in chambers {
            if self.loot.len() >= r.loot_max {
                break;
            }
            if rng.next_float() >= r.loot_chamber_chance {
                continue;
            }
            for _ in 0..30 {
                let c = Cell::new(ch.x + rng.next_index(ch.w as usize) as i32, ch.y + rng.next_index(ch.h as usize) as i32);
                if self.is_walkable(c) && !self.loot.contains(&c) {
                    self.loot.push(c);
                    break;
                }
            }
        }
        let cells: Vec<Cell> = (0..N * N)
            .filter(|i| self.passage[*i] && self.open[*i] && !self.lava[*i])
            .map(|i| Cell::new((i % N) as i32, (i / N) as i32))
            .filter(|c| (c.x + c.y * 3) % r.loot_passage_spacing == 0)
            .collect();
        for c in cells {
            if self.loot.len() >= r.loot_max {
                break;
            }
            if rng.next_float() < r.loot_passage_chance && !self.loot.contains(&c) {
                self.loot.push(c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::infinite::INFERNO_LEVEL;

    const WORLDS: [&str; 3] = ["1426541270", "4157433391", "2899547009"];

    fn key(cx: i32, cy: i32) -> ChunkKey {
        ChunkKey { cx, cy, level: INFERNO_LEVEL }
    }

    #[test]
    fn rules_read_from_the_data_file() {
        let r = CaveRules::builtin();
        assert!(r.chamber_count.0 >= 1 && r.chamber_count.0 <= r.chamber_count.1);
        assert_eq!(r.loot_table, "route");
        assert!(r.passage_wander > 0.0 && r.passage_wander < 1.0);
    }

    #[test]
    fn carving_is_deterministic_and_chambers_fit_the_data_ranges() {
        let r = CaveRules::builtin();
        for w in WORLDS {
            for (cx, cy) in [(0, 0), (1, -1), (-2, 3)] {
                let a = Caves::carve(w, key(cx, cy));
                let b = Caves::carve(w, key(cx, cy));
                assert_eq!(a.walkable_cells(), b.walkable_cells());
                assert_eq!(a.lava_cells(), b.lava_cells());
                assert!(!a.chambers.is_empty());
                assert!(a.chambers.len() as i32 <= r.chamber_count.1);
                for c in &a.chambers {
                    assert!(c.w >= r.chamber_w.0 && c.w <= r.chamber_w.1);
                    assert!(a.is_walkable(c.center), "chamber centre must be dry floor");
                }
            }
        }
    }

    #[test]
    fn all_chambers_and_exits_are_reachable_around_the_lava() {
        let exits = CaveRules::builtin().exits;
        for w in WORLDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    let a = Caves::carve(w, key(cx, cy));
                    let seen = a.reachable(a.chambers[0].center);
                    for c in &a.chambers {
                        assert!(seen[idx(c.center)], "{w} {cx},{cy}");
                    }
                    for s in &a.spawns {
                        assert!(seen[idx(*s)]);
                    }
                    assert_eq!(a.exits.len() as i32, exits);
                    for e in &a.exits {
                        assert!(a.is_open(*e), "exit {e:?} must be open to the edge");
                    }
                }
            }
        }
    }

    #[test]
    fn lava_exists_somewhere_and_is_never_a_spawn_or_loot_cell() {
        let mut lava_seen = false;
        let max = CaveRules::builtin().loot_max;
        for w in WORLDS {
            for cx in -2..=2 {
                for cy in -2..=2 {
                    let a = Caves::carve(w, key(cx, cy));
                    lava_seen |= !a.lava_cells().is_empty();
                    for c in a.spawns.iter().chain(a.loot.iter()) {
                        assert!(!a.is_lava(*c) && a.is_open(*c));
                    }
                    assert!(a.hunter_spawns.iter().all(|h| a.spawns.contains(h)));
                    assert!(a.loot.len() <= max);
                }
            }
        }
        assert!(lava_seen);
    }

    #[test]
    fn spawns_keep_their_spacing() {
        let r = CaveRules::builtin();
        for w in WORLDS {
            let a = Caves::carve(w, key(0, 0));
            for (i, p) in a.spawns.iter().enumerate() {
                for q in &a.spawns[i + 1..] {
                    assert!((p.x - q.x).abs().max((p.y - q.y).abs()) >= r.spawn_spacing);
                }
            }
        }
    }

    #[test]
    fn changing_the_data_changes_the_layout() {
        let mut r = CaveRules::builtin();
        r.chamber_count = Span(1, 1);
        r.lava_channels = Span(0, 0);
        let a = Caves::carve_with(WORLDS[0], key(0, 0), &r);
        assert_eq!(a.chambers.len(), 1);
        assert!(a.lava_cells().is_empty());
        r.loot_max = 0;
        assert!(Caves::carve_with(WORLDS[0], key(0, 0), &r).loot.is_empty());
    }

    #[test]
    fn region_file_is_valid_lr_region_1() {
        let v: Value = serde_json::from_str(FILE).unwrap();
        assert_eq!(v["schema"], "lr.region/1");
        assert_eq!(v["id"], "inferno_caves");
    }
}
