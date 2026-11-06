//! Flights, ladders and drops as edges of one navigation graph over the resident levels (docs/STAIRS_CONTINUOUS_DESIGN.md,
//! phase 8). A hunter, a bot and a remote player use the same flights as the runner, so the walkable space is the union of
//! each level's `hunter_open` grid plus these edges.
//!
//! * A [`StackNav`] holds one [`LevelGrid`] per resident level and every [`Edge`] between them. A stair of an upper
//!   level becomes a two-way `Flight` edge (landing A to foot B, cost = the walked route length); a ladder becomes a
//!   two-way `Ladder` edge (cost by climb time); a drop becomes a one-way `Drop` edge, and only when the data allows it.
//! * [`StackNav::find_path`] is A* over that union graph. [`StackNav::next_hop`] names the first edge on the route.
//! * An [`EdgeWalk`] is a self-contained walk along one edge's polyline (x, z, absolute y), so a nav rebuild or a level
//!   relabel never invalidates a walk in progress.
//! * [`sight_clear_3d`] is the line of sight across storeys: through an open well, never through a slab.
//!
//! Every tunable is in `data/hunters/flights.json` ([`rules`]). Containers are sorted (`BTreeMap`), so two builds of the
//! same levels are equal. Coordinates of a path point are `[x, z, y_abs]` (grid metres, then height in the vertical frame).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::f32::consts::SQRT_2;
use std::sync::{Arc, OnceLock};

use serde_json::Value;

use crate::game::ladder;
use crate::game::level_stack::{owning_level, LevelStack};
use crate::world::facility::{Cell, Stair};
use crate::world::flight::{flight_surface_y, walk_path, well_footprint};
use crate::world::vertical::{LADDER_CLIMB_SPEED, STOREY};
use crate::world::Facility;

pub const SCHEMA: &str = "lr.hunter_flights/1";
/// The embedded path; content/mod.rs skips it when it reads hunter definitions.
pub const FLIGHTS_PATH: &str = "hunters/flights.json";

/// Metres between the points of a walked flight route (`walk_path`).
const WALK_STEP: f32 = 0.25;

// ---------------------------------------------------------------------------------------------------------------------
// Rules (data/hunters/flights.json)
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct FlightEdgeRules {
    /// Multiplies `LADDER_CLIMB_SPEED` for a hunter or bot on a ladder.
    pub ladder_speed_scale: f32,
    /// Whether a hunter may jump down a drop (one way). Off by default.
    pub drop_allowed: bool,
    /// Metres per second of a fall down a drop.
    pub drop_fall_speed: f32,
    /// Multiplies the path cost of every edge (at least 1 so the A* heuristic stays admissible).
    pub edge_cost_scale: f32,
    /// The hunter walk speed (m/s) that turns a ladder climb time into a path cost.
    pub reference_speed: f32,
    /// Factor on the radius of a sound one storey away.
    pub hear_adjacent_scale: f32,
    /// Whether a target one storey away can be seen through an open well.
    pub see_adjacent: bool,
    /// Largest height difference (m) at which a hunter captures.
    pub capture_height: f32,
    /// Largest height difference (m) at which two hunters count as standing at the same height (the director's pairing).
    pub same_layer_eps: f32,
    /// How close (m) to the entrance of an edge a walker must be to start it.
    pub edge_enter_radius: f32,
    /// Body radius (m) of the footprint test while a walker follows an edge. The routes of the flights are validated for
    /// a runner's body, so this is the runner's radius: a wider hunter body would be refused at the tight turn of a mid
    /// landing and stall there.
    pub edge_fit_radius: f32,
    /// Seconds without progress on an edge before the walker replans.
    pub edge_stuck_seconds: f32,
    /// Simulated hunters per resident floor beside the active one.
    pub resident_hunter_cap: u32,
}

fn num(v: &Value, key: &str, lo: f32, hi: f32) -> Result<f32, String> {
    let x = v
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("flights.json: `{key}` is missing or not a number"))? as f32;
    if x.is_finite() && x >= lo && x <= hi {
        Ok(x)
    } else {
        Err(format!("flights.json: `{key}` = {x} is outside {lo}..={hi}"))
    }
}

fn flag(v: &Value, key: &str) -> Result<bool, String> {
    v.get(key).and_then(Value::as_bool).ok_or_else(|| format!("flights.json: `{key}` is missing or not true/false"))
}

impl FlightEdgeRules {
    /// Reads and checks a flights file: every value must be present and inside its limits, or the file is refused.
    pub fn parse(text: &str) -> Result<FlightEdgeRules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("flights.json: {e}"))?;
        if v.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
            return Err(format!("flights.json: `schema` must be \"{SCHEMA}\""));
        }
        let cap = num(&v, "resident_hunter_cap", 0.0, 64.0)?;
        if cap.fract() != 0.0 {
            return Err(format!("flights.json: `resident_hunter_cap` = {cap} must be a whole number"));
        }
        Ok(FlightEdgeRules {
            ladder_speed_scale: num(&v, "ladder_speed_scale", 0.1, 5.0)?,
            drop_allowed: flag(&v, "drop_allowed")?,
            drop_fall_speed: num(&v, "drop_fall_speed", 1.0, 30.0)?,
            edge_cost_scale: num(&v, "edge_cost_scale", 1.0, 20.0)?,
            reference_speed: num(&v, "reference_speed", 0.1, 20.0)?,
            hear_adjacent_scale: num(&v, "hear_adjacent_scale", 0.0, 1.0)?,
            see_adjacent: flag(&v, "see_adjacent")?,
            capture_height: num(&v, "capture_height", 0.1, 3.0)?,
            same_layer_eps: num(&v, "same_layer_eps", 0.1, 2.0)?,
            edge_enter_radius: num(&v, "edge_enter_radius", 0.05, 2.0)?,
            edge_fit_radius: num(&v, "edge_fit_radius", 0.05, 0.6)?,
            edge_stuck_seconds: num(&v, "edge_stuck_seconds", 0.5, 60.0)?,
            resident_hunter_cap: cap as u32,
        })
    }

    /// Metres per second up or down a ladder for a hunter or bot.
    pub fn ladder_speed(&self) -> f32 {
        LADDER_CLIMB_SPEED * self.ladder_speed_scale
    }
}

/// The embedded rules (`data/hunters/flights.json`).
pub fn rules() -> &'static FlightEdgeRules {
    static RULES: OnceLock<FlightEdgeRules> = OnceLock::new();
    RULES.get_or_init(|| {
        FlightEdgeRules::parse(include_str!("../data/hunters/flights.json")).expect("data/hunters/flights.json parses")
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Graph types
// ---------------------------------------------------------------------------------------------------------------------

/// One grid cell of one level.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Node {
    pub level: i32,
    pub cell: Cell,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum EdgeKind {
    Flight,
    Ladder,
    Drop,
}

/// Names an edge across nav rebuilds: the upper level, the cell on it (a stair's landing A, a ladder's step cell, a drop's
/// hole cell) and the kind.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct EdgeKey {
    pub upper_level: i32,
    pub cell: Cell,
    pub kind: EdgeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub key: EdgeKey,
    pub from: Node,
    pub to: Node,
    /// Path cost in cells; never less than the octile distance between `from` and `to`.
    pub cost: f32,
    /// `[x, z, y_abs]` polyline from `from` to `to`.
    pub path: Arc<[[f32; 3]]>,
    /// Length of `path` in metres (3D).
    pub len: f32,
    pub one_way: bool,
}

/// One level's walkable cells for hunters (`Facility::hunter_open`).
#[derive(Clone, Debug, PartialEq)]
pub struct LevelGrid {
    pub level: i32,
    pub cols: i32,
    pub rows: i32,
    /// Open ground (`Facility::hunter_open`).
    pub open: Vec<bool>,
}

impl LevelGrid {
    pub fn from_facility(fac: &Facility) -> LevelGrid {
        let cap = (fac.cols * fac.rows).max(0) as usize;
        let mut open = Vec::with_capacity(cap);
        for y in 0..fac.rows {
            for x in 0..fac.cols {
                let c = Cell::new(x, y);
                open.push(fac.hunter_open(c));
            }
        }
        LevelGrid { level: fac.level, cols: fac.cols, rows: fac.rows, open }
    }

    fn index(&self, c: Cell) -> Option<usize> {
        (c.x >= 0 && c.y >= 0 && c.x < self.cols && c.y < self.rows).then(|| (c.y * self.cols + c.x) as usize)
    }

    /// Open ground (`Facility::hunter_open`).
    pub fn is_open(&self, c: Cell) -> bool {
        self.index(c).is_some_and(|i| self.open[i])
    }
}

/// A step of a route: the node reached and the edge used to reach it (`None` for a step on the same level's grid, and for
/// the start).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RouteStep {
    pub node: Node,
    pub via: Option<EdgeKey>,
}

/// The first edge on a cross-level route.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Hop {
    pub edge: EdgeKey,
    /// The cell on the walker's level where the edge begins.
    pub entrance: Cell,
    /// Grid steps from the start to the entrance.
    pub steps: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackNav {
    grids: BTreeMap<i32, LevelGrid>,
    out: BTreeMap<Node, Vec<Edge>>,
    by_key: BTreeMap<EdgeKey, Edge>,
    version: u64,
    /// The rules the graph was built with (`None` for an empty graph: the embedded rules apply).
    rules: Option<FlightEdgeRules>,
}

fn octile(a: Cell, b: Cell) -> f32 {
    let (dx, dy) = ((a.x - b.x).abs() as f32, (a.y - b.y).abs() as f32);
    dx.max(dy) + (SQRT_2 - 1.0) * dx.min(dy)
}

fn path_len_3d(p: &[[f32; 3]]) -> f32 {
    p.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt()).sum()
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn put(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

const NEIGHBOURS: [(i32, i32); 8] = [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)];

impl StackNav {
    pub fn empty() -> StackNav {
        StackNav::default()
    }

    /// Grids from every resident level's `hunter_open`, edges from every resident facility's stairs (and, when the data
    /// allows, its drops).
    pub fn build(active: &Facility, stack: &LevelStack, rules: &FlightEdgeRules) -> StackNav {
        let mut facs: BTreeMap<i32, &Facility> = BTreeMap::new();
        for r in &stack.residents {
            facs.insert(r.level, &r.fac);
        }
        facs.insert(active.level, active);
        let mut nav = StackNav::empty();
        nav.rules = Some(rules.clone());
        for (level, fac) in &facs {
            nav.grids.insert(*level, LevelGrid::from_facility(fac));
        }
        for (level, fac) in &facs {
            if !nav.grids.contains_key(&(level - 1)) {
                continue;
            }
            for s in fac.stairs.iter().filter(|s| s.dir < 0) {
                if let Some(e) = stair_edge(fac, s, rules) {
                    nav.add(e);
                }
            }
            if rules.drop_allowed {
                for c in fac.drop_cells() {
                    if let Some(e) = nav.drop_edge(fac, *c, rules) {
                        nav.add(e);
                    }
                }
            }
        }
        nav.version = nav.hash();
        nav
    }

    fn add(&mut self, e: Edge) {
        if !self.grids.contains_key(&e.from.level) || !self.grids.contains_key(&e.to.level) {
            return;
        }
        if !e.one_way {
            let mut rev = e.clone();
            rev.from = e.to;
            rev.to = e.from;
            let mut p: Vec<[f32; 3]> = e.path.to_vec();
            p.reverse();
            rev.path = p.into();
            self.out.entry(rev.from).or_default().push(rev);
        }
        self.out.entry(e.from).or_default().push(e.clone());
        self.by_key.insert(e.key, e);
    }

    fn drop_edge(&self, fac: &Facility, c: Cell, rules: &FlightEdgeRules) -> Option<Edge> {
        let lower = self.grids.get(&(fac.level - 1))?;
        let bank = [(0, -1), (1, 0), (0, 1), (-1, 0)]
            .iter()
            .map(|d| Cell::new(c.x + d.0, c.y + d.1))
            .find(|n| fac.hunter_open(*n))?;
        let land = nearest_open(lower, c, 8)?;
        let (uy, ly) = (fac.level as f32 * STOREY, (fac.level - 1) as f32 * STOREY);
        let (bc, cc, lc) = (bank.center(), c.center(), land.center());
        let path: Arc<[[f32; 3]]> = vec![[bc.0, bc.1, uy], [cc.0, cc.1, uy], [cc.0, cc.1, ly], [lc.0, lc.1, ly]].into();
        let from = Node { level: fac.level, cell: bank };
        let to = Node { level: fac.level - 1, cell: land };
        let len = path_len_3d(&path);
        Some(Edge {
            key: EdgeKey { upper_level: fac.level, cell: c, kind: EdgeKind::Drop },
            from,
            to,
            cost: (len * rules.edge_cost_scale).max(octile(from.cell, to.cell)),
            path,
            len,
            one_way: true,
        })
    }

    fn hash(&self) -> u64 {
        let mut h = Fnv::new();
        for (l, g) in &self.grids {
            h.put(*l as i64 as u64);
            h.put(g.cols as u64);
            h.put(g.rows as u64);
            for chunk in g.open.chunks(64) {
                h.put(chunk.iter().enumerate().fold(0u64, |a, (i, b)| a | ((*b as u64) << i)));
            }
        }
        for (k, e) in &self.by_key {
            h.put(k.upper_level as i64 as u64);
            h.put(k.cell.x as i64 as u64);
            h.put(k.cell.y as i64 as u64);
            h.put(k.kind as u64);
            h.put(e.from.level as i64 as u64);
            h.put(e.to.level as i64 as u64);
            h.put(e.cost.to_bits() as u64);
            h.put(e.path.len() as u64);
        }
        h.0
    }

    /// The rules this graph was built with, so a walk on its edges uses the same data the costs were made from (the
    /// embedded rules for an empty graph).
    pub fn rules(&self) -> &FlightEdgeRules {
        self.rules.as_ref().unwrap_or_else(|| rules())
    }

    /// Levels with a grid, ascending.
    pub fn levels(&self) -> Vec<i32> {
        self.grids.keys().copied().collect()
    }

    pub fn has_level(&self, level: i32) -> bool {
        self.grids.contains_key(&level)
    }

    pub fn grid(&self, level: i32) -> Option<&LevelGrid> {
        self.grids.get(&level)
    }

    /// A hash of the grids and edges: equal for two identical builds, different once a cell opens or an edge changes.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Open ground.
    pub fn is_open(&self, n: Node) -> bool {
        self.grids.get(&n.level).is_some_and(|g| g.is_open(n.cell))
    }

    /// The edges that start at `n` (a two-way edge appears at both ends, each with its own direction).
    pub fn edges_from(&self, n: Node) -> impl Iterator<Item = &Edge> {
        self.out.get(&n).into_iter().flatten()
    }

    /// The edge `key` names, in its canonical direction (upper to lower).
    pub fn edge(&self, key: EdgeKey) -> Option<&Edge> {
        self.by_key.get(&key)
    }

    /// The edge `key` names as walked from `from` (a two-way edge has one record per direction; this picks the one that
    /// starts at `from`). `None` when no such edge starts there, e.g. after a nav rebuild dropped it.
    pub fn edge_from(&self, from: Node, key: EdgeKey) -> Option<&Edge> {
        self.edges_from(from).find(|e| e.key == key)
    }

    /// An [`EdgeWalk`] along the edge a [`Hop`] names, starting at the hop's entrance on `level`.
    pub fn start_walk(&self, level: i32, hop: &Hop) -> Option<EdgeWalk> {
        self.edge_from(Node { level, cell: hop.entrance }, hop.edge).map(EdgeWalk::start)
    }

    /// A* over the union of the level grids and the edges: 8-way, no corner cutting, an octile heuristic that stays
    /// admissible because an edge costs at least its horizontal run. `step_cost(level, from, to)` is a multiplier on the
    /// step's length (water; values under 1 are taken as 1) or `None` when that step is not allowed. The route includes
    /// `from` (via `None`); empty when `from` or `to` is not open or no route exists. A one-way edge is never walked backwards.
    pub fn find_path(&self, from: Node, to: Node, step_cost: &dyn Fn(i32, Cell, Cell) -> Option<f32>) -> Vec<RouteStep> {
        if !self.is_open(from) || !self.is_open(to) {
            return vec![];
        }
        if from == to {
            return vec![RouteStep { node: from, via: None }];
        }
        #[derive(PartialEq)]
        struct Item {
            f: f32,
            node: Node,
        }
        impl Eq for Item {}
        impl PartialOrd for Item {
            fn partial_cmp(&self, o: &Item) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Item {
            fn cmp(&self, o: &Item) -> Ordering {
                o.f.total_cmp(&self.f).then_with(|| o.node.cmp(&self.node))
            }
        }
        let h = |n: Node| octile(n.cell, to.cell);
        let mut g: BTreeMap<Node, f32> = BTreeMap::new();
        let mut came: BTreeMap<Node, (Node, Option<EdgeKey>)> = BTreeMap::new();
        let mut closed: BTreeSet<Node> = BTreeSet::new();
        let mut heap = BinaryHeap::new();
        g.insert(from, 0.0);
        heap.push(Item { f: h(from), node: from });
        let mut found = false;
        while let Some(Item { node: cur, .. }) = heap.pop() {
            if cur == to {
                found = true;
                break;
            }
            if !closed.insert(cur) {
                continue;
            }
            let gc = g[&cur];
            let grid = &self.grids[&cur.level];
            let mut relax = |n: Node, ng: f32, via: Option<EdgeKey>, heap: &mut BinaryHeap<Item>| {
                if g.get(&n).map_or(true, |old| ng < *old) {
                    g.insert(n, ng);
                    came.insert(n, (cur, via));
                    heap.push(Item { f: ng + h(n), node: n });
                }
            };
            for (dx, dy) in NEIGHBOURS {
                let nb = Cell::new(cur.cell.x + dx, cur.cell.y + dy);
                if !grid.is_open(nb) {
                    continue;
                }
                let diag = dx != 0 && dy != 0;
                if diag {
                    let (a, b) = (Cell::new(cur.cell.x + dx, cur.cell.y), Cell::new(cur.cell.x, cur.cell.y + dy));
                    if !grid.is_open(a) || !grid.is_open(b) {
                        continue;
                    }
                }
                let Some(factor) = step_cost(cur.level, cur.cell, nb) else { continue };
                let step = if diag { SQRT_2 } else { 1.0 } * factor.max(1.0);
                relax(Node { level: cur.level, cell: nb }, gc + step, None, &mut heap);
            }
            for e in self.edges_from(cur) {
                if self.is_open(e.to) {
                    relax(e.to, gc + e.cost, Some(e.key), &mut heap);
                }
            }
        }
        if !found {
            return vec![];
        }
        let mut route = vec![];
        let mut at = to;
        loop {
            let (prev, via) = match came.get(&at) {
                Some(p) => *p,
                None => (at, None),
            };
            route.push(RouteStep { node: at, via });
            if at == from {
                break;
            }
            at = prev;
        }
        route.reverse();
        if let Some(first) = route.first_mut() {
            first.via = None;
        }
        route
    }

    /// The first edge on the cheapest route from `from` to `to`, or `None` when the route needs no edge (same grid) or
    /// there is no route (the goal's level is not reachable).
    pub fn next_hop(&self, from: Node, to: Node, step_cost: &dyn Fn(i32, Cell, Cell) -> Option<f32>) -> Option<Hop> {
        let route = self.find_path(from, to, step_cost);
        let i = route.iter().position(|s| s.via.is_some())?;
        Some(Hop { edge: route[i].via?, entrance: route[i - 1].node.cell, steps: i - 1 })
    }
}

/// The open cell of `grid` nearest to `c` within `reach` cells (ties in scan order), or `None`.
fn nearest_open(grid: &LevelGrid, c: Cell, reach: i32) -> Option<Cell> {
    let mut best: Option<(i32, Cell)> = None;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let n = Cell::new(c.x + dx, c.y + dy);
            let d = dx * dx + dy * dy;
            if grid.is_open(n) && best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, n));
            }
        }
    }
    best.map(|(_, n)| n)
}

/// The edge of one upper stair record (a flight or a ladder), in its canonical direction: landing A down to foot B.
fn stair_edge(fac: &Facility, s: &Stair, rules: &FlightEdgeRules) -> Option<Edge> {
    let flight = s.flight(fac.level)?;
    let fp = well_footprint(&flight);
    let (uy, ly) = (flight.upper_y(), flight.lower_y());
    let (kind, path, cost): (EdgeKind, Vec<[f32; 3]>, f32) = if s.is_ladder() {
        let run = ladder::run_of(s);
        let step = ladder::step_center(s.cell, run);
        let stand = ladder::stand(s.cell, s.rails_side());
        let hatch = s.cell.center();
        let path = vec![[step.0, step.1, uy], [stand.0, stand.1, uy], [stand.0, stand.1, ly], [hatch.0, hatch.1, ly]];
        let cost = path_len_3d(&path) * rules.reference_speed / rules.ladder_speed();
        (EdgeKind::Ladder, path, cost)
    } else {
        let pts = walk_path(&flight, WALK_STEP);
        if pts.len() < 2 {
            return None;
        }
        let mut y = uy;
        let last = pts.len() - 1;
        let path: Vec<[f32; 3]> = pts
            .iter()
            .enumerate()
            .map(|(i, (x, z))| {
                y = if i == 0 {
                    uy
                } else if i == last {
                    ly
                } else {
                    flight_surface_y(&flight, *x, *z).map_or(y, |sy| sy.min(y)).max(ly)
                };
                [*x, *z, y]
            })
            .collect();
        // the walk is along the slope: the cost is the 3D length (what `EdgeWalk::advance` walks), not its ground shadow
        let cost = path_len_3d(&path) * rules.edge_cost_scale;
        (EdgeKind::Flight, path, cost)
    };
    let from = Node { level: fac.level, cell: fp.landing };
    let to = Node { level: fac.level - 1, cell: fp.foot };
    let len = path_len_3d(&path);
    let cost = if kind == EdgeKind::Ladder { cost * rules.edge_cost_scale } else { cost };
    Some(Edge {
        key: EdgeKey { upper_level: fac.level, cell: fp.landing, kind },
        from,
        to,
        cost: cost.max(octile(from.cell, to.cell)),
        path: path.into(),
        len,
        one_way: false,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Walking an edge
// ---------------------------------------------------------------------------------------------------------------------

/// A walk in progress along one edge. It carries its own polyline, so a nav rebuild never invalidates it.
#[derive(Clone, Debug, PartialEq)]
pub struct EdgeWalk {
    pub key: EdgeKey,
    pub kind: EdgeKind,
    pub path: Arc<[[f32; 3]]>,
    /// Metres walked along `path`.
    pub s: f32,
    pub len: f32,
    pub from: Node,
    pub to: Node,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgePose {
    pub x: f32,
    pub z: f32,
    pub y: f32,
    pub done: bool,
}

impl EdgeWalk {
    pub fn start(e: &Edge) -> EdgeWalk {
        EdgeWalk { key: e.key, kind: e.key.kind, path: e.path.clone(), s: 0.0, len: e.len, from: e.from, to: e.to }
    }

    pub fn is_ladder(&self) -> bool {
        self.kind == EdgeKind::Ladder
    }

    /// The share of the edge walked so far, 0 to 1.
    pub fn fraction(&self) -> f32 {
        if self.len > 1e-6 { (self.s / self.len).clamp(0.0, 1.0) } else { 1.0 }
    }

    /// The speed (m/s) a walker of `hunter_speed` moves along this edge on its flat parts: the hunter's own speed on a
    /// flight, the data ladder speed on a ladder. Used for the "held on the edge too long" limit.
    pub fn speed(&self, hunter_speed: f32, rules: &FlightEdgeRules) -> f32 {
        match self.kind {
            EdgeKind::Ladder => rules.ladder_speed(),
            _ => hunter_speed,
        }
    }

    /// `[x, z, y_abs]` of the edge's first point.
    pub fn start_point(&self) -> [f32; 3] {
        self.path.first().copied().unwrap_or([0.0; 3])
    }

    /// `[x, z, y_abs]` of the edge's last point.
    pub fn end_point(&self) -> [f32; 3] {
        self.path.last().copied().unwrap_or([0.0; 3])
    }

    /// The speed along the segment `a` to `b`: a flight at the hunter's speed, a ladder at the climb speed, and on a
    /// drop the vertical part at the fall speed.
    fn segment_speed(&self, a: &[f32; 3], b: &[f32; 3], hunter_speed: f32, rules: &FlightEdgeRules) -> f32 {
        match self.kind {
            EdgeKind::Flight => hunter_speed,
            EdgeKind::Ladder => rules.ladder_speed(),
            EdgeKind::Drop => {
                if (b[0] - a[0]).hypot(b[1] - a[1]) < 1e-4 {
                    rules.drop_fall_speed
                } else {
                    hunter_speed
                }
            }
        }
    }

    /// Moves along the edge for `dt` seconds and returns the pose. `done` once the end is reached.
    pub fn advance(&mut self, hunter_speed: f32, dt: f32, rules: &FlightEdgeRules) -> EdgePose {
        let mut left = dt.max(0.0);
        let mut guard = 0;
        while left > 0.0 && self.s < self.len && guard < 256 {
            guard += 1;
            let mut start = 0.0f32;
            let mut moved = false;
            for w in self.path.windows(2) {
                let seg = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt();
                let end = start + seg;
                if seg > 1e-6 && self.s < end {
                    let speed = self.segment_speed(&w[0], &w[1], hunter_speed, rules).max(1e-4);
                    let need = (end - self.s) / speed;
                    if need <= left {
                        self.s = end;
                        left -= need;
                    } else {
                        self.s += speed * left;
                        left = 0.0;
                    }
                    moved = true;
                    break;
                }
                start = end;
            }
            if !moved {
                self.s = self.len;
            }
        }
        if self.s >= self.len - 1e-5 {
            self.s = self.len;
        }
        self.pose()
    }

    /// The pose at the current distance.
    pub fn pose(&self) -> EdgePose {
        let done = self.s >= self.len - 1e-5;
        let mut start = 0.0f32;
        for w in self.path.windows(2) {
            let seg = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt();
            if seg > 1e-6 && self.s <= start + seg {
                let t = ((self.s - start) / seg).clamp(0.0, 1.0);
                return EdgePose {
                    x: w[0][0] + (w[1][0] - w[0][0]) * t,
                    z: w[0][1] + (w[1][1] - w[0][1]) * t,
                    y: w[0][2] + (w[1][2] - w[0][2]) * t,
                    done,
                };
            }
            start += seg;
        }
        let end = self.path.last().copied().unwrap_or([0.0; 3]);
        EdgePose { x: end[0], z: end[1], y: end[2], done: true }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Line of sight across storeys
// ---------------------------------------------------------------------------------------------------------------------

/// The facilities a sight line may cross: the active level and the resident stack.
#[derive(Clone, Copy)]
pub struct Floors<'a> {
    pub active: &'a Facility,
    pub stack: &'a LevelStack,
}

impl<'a> Floors<'a> {
    pub fn fac(&self, level: i32) -> Option<&'a Facility> {
        self.stack.fac_for(self.active, level)
    }
}

/// Whether `a` sees `b` (both `[x, z, y_abs]`). The 3D segment is sampled every 1/6 m. Each sample's level is
/// `owning_level(y)`; a level that is not resident blocks; each sample must be `sight_open`. Where two samples are on
/// different levels the crossing point must be an opening: the upper level has no floor there (`open_below`) or the lower
/// has no ceiling (`open_above`), so a sight line passes an open well and never a slab. On one level this is exactly
/// `Facility::has_line_of_sight`.
pub fn sight_clear_3d(floors: &Floors, a: [f32; 3], b: [f32; 3]) -> bool {
    let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    let steps = ((d * 6.0).ceil() as i32).max(1);
    let mut prev: Option<i32> = None;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let (x, z, y) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t);
        let level = owning_level(y);
        let Some(fac) = floors.fac(level) else { return false };
        if !fac.sight_open(x, z) {
            return false;
        }
        if let Some(pl) = prev.filter(|pl| *pl != level) {
            let hi = pl.max(level);
            if hi - pl.min(level) != 1 {
                return false;
            }
            let dy = b[2] - a[2];
            let tc = if dy.abs() < 1e-6 { t } else { ((hi as f32 * STOREY - a[2]) / dy).clamp(0.0, 1.0) };
            let (cx, cz) = (a[0] + (b[0] - a[0]) * tc, a[1] + (b[1] - a[1]) * tc);
            let cell = Cell::new(cx.floor() as i32, cz.floor() as i32);
            let open = floors.fac(hi).is_some_and(|u| u.open_below(cell)) || floors.fac(hi - 1).is_some_and(|l| l.open_above(cell));
            if !open {
                return false;
            }
        }
        prev = Some(level);
    }
    true
}

// ---------------------------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------------------------

#[cfg(test)]
use crate::game::level_stack::Resident;
#[cfg(test)]
use crate::world::flight::FlightShape;
#[cfg(test)]
use crate::world::fixtures::Fixtures;

/// The nav of the two test levels joined by a flight of `shape` (or a ladder), with the levels themselves.
#[cfg(test)]
pub fn test_nav(shape: FlightShape, ladder: bool) -> (StackNav, Facility, Facility) {
    let (upper, lower) = Facility::test_levels(shape, ladder);
    let nav = test_nav_of(&upper, &lower, rules());
    (nav, upper, lower)
}

/// Which pair of test levels a [`TestStage`] joins.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TestShape {
    Switchback,
    Straight,
    Spiral,
    Ladder,
    Drop,
}

/// Two open levels (upper 0, lower -1) joined by the edge of a [`TestShape`], with a floor point on each level clear of
/// the flight (and of the drop's hole). For the brain and bot tests.
#[cfg(test)]
pub(crate) struct TestStage {
    pub upper: Facility,
    pub lower: Facility,
    pub nav: Arc<StackNav>,
    /// A floor point on the lower level and one on the upper level, away from the flight.
    pub low: (f32, f32),
    pub high: (f32, f32),
    /// The edge's length in metres (0 for a drop stage whose rules keep drops off).
    pub len: f32,
}

#[cfg(test)]
pub(crate) const TEST_LOW_LEVEL: i32 = -1;
#[cfg(test)]
pub(crate) const TEST_HIGH_LEVEL: i32 = 0;

#[cfg(test)]
impl TestStage {
    /// The facility of one of the two levels.
    pub fn fac_of(&self, level: i32) -> &Facility {
        if level == TEST_HIGH_LEVEL { &self.upper } else { &self.lower }
    }
}

#[cfg(test)]
pub(crate) fn test_stage(shape: TestShape, rules: &FlightEdgeRules) -> TestStage {
    let (upper, lower) = match shape {
        TestShape::Switchback => Facility::test_levels(FlightShape::Switchback, false),
        TestShape::Straight => Facility::test_levels(FlightShape::Straight, false),
        TestShape::Spiral => Facility::test_levels(FlightShape::Spiral, false),
        TestShape::Ladder => Facility::test_levels(FlightShape::Switchback, true),
        TestShape::Drop => Facility::test_levels_drop(),
    };
    let nav = test_nav_of(&upper, &lower, rules);
    let len = nav.by_key.values().next().map_or(0.0, |e| e.len);
    TestStage { upper, lower, nav: Arc::new(nav), low: (24.5, 24.5), high: (24.5, 24.5), len }
}

/// How far (m) a body at `pos` with feet at absolute `y` is from the stage's flight surface; 0 off the flight's cells.
#[cfg(test)]
pub(crate) fn surface_error(stage: &TestStage, pos: (f32, f32), y: f32) -> f32 {
    let Some(stair) = stage.upper.stairs.first() else { return 0.0 };
    let Some(flight) = stair.flight(stage.upper.level) else { return 0.0 };
    // the surface a body walks on: the upper level's view or the lower level's (a spiral's lead-out runs under the slab)
    let rules = crate::world::flight::flights();
    [false, true]
        .into_iter()
        .filter_map(|lower| crate::world::flight::flight_walk_y_with(rules, &flight, pos.0, pos.1, lower).map(|sy| (sy - y).abs()))
        .fold(None, |best: Option<f32>, e| Some(best.map_or(e, |b| b.min(e))))
        .unwrap_or(0.0)
}

#[cfg(test)]
pub fn test_nav_of(upper: &Facility, lower: &Facility, rules: &FlightEdgeRules) -> StackNav {
    let mut stack = LevelStack::default();
    stack.put(Resident { level: lower.level, center: (0, 0), fac: lower.clone(), fix: Fixtures::default() });
    StackNav::build(upper, &stack, rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::flight::walk_path;

    const SHAPES: [FlightShape; 3] = [FlightShape::Switchback, FlightShape::Straight, FlightShape::Spiral];

    fn free(_: i32, _: Cell, _: Cell) -> Option<f32> {
        Some(1.0)
    }

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn a_flight_adds_an_edge_per_shape() {
        for shape in SHAPES {
            let (nav, upper, lower) = test_nav(shape, false);
            let s = upper.stairs[0];
            let (a, b) = (s.cell, s.foot());
            assert_eq!(lower.stairs[0].cell, b, "{shape:?}: the lower record is the foot");
            let from = Node { level: 0, cell: a };
            let to = Node { level: -1, cell: b };
            let e = nav.edges_from(from).find(|e| e.key.kind == EdgeKind::Flight).expect("a flight edge at the landing");
            assert_eq!((e.from, e.to), (from, to), "{shape:?}");
            assert!(!e.one_way);
            let flight = s.flight(0).unwrap();
            let pts = walk_path(&flight, WALK_STEP);
            let flat: f32 = pts.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum();
            // the cost is the length of the walk along the slope (what `EdgeWalk` walks), not its ground shadow
            let expect: f32 = e.path.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt()).sum();
            assert!(expect > flat + 1.0, "{shape:?}: the slope adds length: {expect} vs {flat}");
            assert!(approx(e.cost, expect * rules().edge_cost_scale), "{shape:?}: cost {} vs 3D walk path {expect}", e.cost);
            assert!(approx(e.len, expect), "{shape:?}: the walked length is the same 3D length");
            assert!(approx(e.path[0][2], flight.upper_y()) && approx(e.path.last().unwrap()[2], flight.lower_y()), "{shape:?}");
            // the same edge walked backwards starts at the foot and ends at the landing
            let back = nav.edges_from(to).find(|e| e.key.kind == EdgeKind::Flight).expect("two-way");
            assert_eq!((back.from, back.to), (to, from));
            assert!(approx(back.path[0][2], flight.lower_y()));
            assert_eq!(back.key, e.key);
            assert_eq!(nav.edge(e.key).map(|x| x.from), Some(from));
        }
    }

    #[test]
    fn a_ladder_edge_costs_by_climb_speed_and_data_changes_it() {
        let (upper, lower) = Facility::test_levels(FlightShape::Switchback, true);
        let key_of = |nav: &StackNav| {
            let a = Node { level: 0, cell: upper.stairs[0].landing() };
            nav.edges_from(a).find(|e| e.key.kind == EdgeKind::Ladder).cloned().expect("a ladder edge")
        };
        let base = rules().clone();
        let slow = key_of(&test_nav_of(&upper, &lower, &base));
        let expect = slow.len * base.reference_speed / (LADDER_CLIMB_SPEED * base.ladder_speed_scale) * base.edge_cost_scale;
        assert!(approx(slow.cost, expect), "{} vs {expect}", slow.cost);
        assert_eq!(slow.to.cell, upper.stairs[0].cell, "a ladder ends on the hatch cell below");
        let fast_rules = FlightEdgeRules { ladder_speed_scale: base.ladder_speed_scale * 2.0, ..base.clone() };
        let fast = key_of(&test_nav_of(&upper, &lower, &fast_rules));
        assert!(fast.cost < slow.cost && approx(fast.cost * 2.0, slow.cost), "{} vs {}", fast.cost, slow.cost);
    }

    #[test]
    fn a_drop_is_one_way_and_absent_by_default() {
        let (upper, lower) = Facility::test_levels_drop();
        let off = test_nav_of(&upper, &lower, &FlightEdgeRules { drop_allowed: false, ..rules().clone() });
        let above = Node { level: 0, cell: Cell::new(17, 18) };
        let below = Node { level: -1, cell: Cell::new(12, 12) };
        assert!(off.find_path(below, above, &free).is_empty());
        assert!(off.find_path(above, below, &free).is_empty(), "no drop edge by default");
        let on = test_nav_of(&upper, &lower, &FlightEdgeRules { drop_allowed: true, ..rules().clone() });
        let drops: Vec<&Edge> = (16..22)
            .flat_map(|y| (16..22).map(move |x| Node { level: 0, cell: Cell::new(x, y) }))
            .flat_map(|n| on.edges_from(n))
            .collect();
        assert!(!drops.is_empty(), "drop edges exist once the data allows them");
        assert!(drops.iter().all(|e| e.key.kind == EdgeKind::Drop && e.one_way && e.to.level == -1));
        assert!(!on.find_path(above, below, &free).is_empty(), "down a drop is a route");
        assert!(on.find_path(below, above, &free).is_empty(), "a drop is never walked back up");
        assert!(on.edges_from(below).next().is_none());
    }

    #[test]
    fn find_path_crosses_levels_over_the_union_graph() {
        for shape in SHAPES {
            let (nav, upper, _lower) = test_nav(shape, false);
            let a = upper.stairs[0].cell;
            let from = Node { level: 0, cell: Cell::new(6, 6) };
            let to = Node { level: -1, cell: Cell::new(22, 22) };
            let route = nav.find_path(from, to, &free);
            assert!(!route.is_empty(), "{shape:?}: a route down");
            assert_eq!(route.first().unwrap().node, from);
            assert_eq!(route.last().unwrap().node, to);
            let vias: Vec<_> = route.iter().filter_map(|s| s.via).collect();
            assert_eq!(vias.len(), 1, "{shape:?}: exactly one edge");
            assert_eq!(vias[0].kind, EdgeKind::Flight);
            let hop = nav.next_hop(from, to, &free).expect("a hop");
            assert_eq!((hop.edge, hop.entrance), (vias[0], a), "{shape:?}");
            assert_eq!(hop.steps + 1, route.iter().position(|s| s.via.is_some()).unwrap());
            // and back up
            let up = nav.find_path(to, from, &free);
            assert!(!up.is_empty() && up.iter().filter(|s| s.via.is_some()).count() == 1, "{shape:?}: a route up");
            // same-level routes need no hop
            let near = Node { level: 0, cell: Cell::new(12, 6) };
            assert!(!nav.find_path(from, near, &free).is_empty());
            assert!(nav.next_hop(from, near, &free).is_none());
        }
        let (nav, _, _) = test_nav(FlightShape::Straight, false);
        let nowhere = Node { level: -2, cell: Cell::new(10, 10) };
        assert!(nav.find_path(Node { level: 0, cell: Cell::new(6, 6) }, nowhere, &free).is_empty());
        assert!(nav.next_hop(Node { level: 0, cell: Cell::new(6, 6) }, nowhere, &free).is_none());
        // a blocked step is respected
        let wall = |_: i32, _: Cell, b: Cell| if b.x == 9 { None } else { Some(1.0) };
        let from = Node { level: 0, cell: Cell::new(6, 6) };
        let route = nav.find_path(from, Node { level: 0, cell: Cell::new(12, 6) }, &wall);
        assert!(route.is_empty() || route.iter().all(|s| s.node.cell.x != 9));
    }

    #[test]
    fn an_edge_walk_follows_flight_surface_y_at_hunter_speed() {
        let r = rules();
        for shape in SHAPES {
            let (nav, upper, _) = test_nav(shape, false);
            let e = nav.edges_from(Node { level: 0, cell: upper.stairs[0].cell }).next().unwrap().clone();
            let (speed, dt) = (2.0f32, 0.05f32);
            let mut w = EdgeWalk::start(&e);
            let mut last_y = e.path[0][2];
            let mut t = 0.0;
            let mut done = false;
            while t < 60.0 {
                let p = w.advance(speed, dt, r);
                assert!(p.y <= last_y + 1e-4, "{shape:?}: y rises going down");
                last_y = p.y;
                t += dt;
                if p.done {
                    done = true;
                    break;
                }
            }
            assert!(done, "{shape:?}");
            assert!((t - e.len / speed).abs() <= dt * 1.5, "{shape:?}: {t} s for {} m at {speed}", e.len);
            assert!(approx(last_y, e.path.last().unwrap()[2]) && approx(w.s, w.len));
            // the pose in the middle lies between the ends
            let mut mid = EdgeWalk::start(&e);
            let p = mid.advance(speed, e.len / speed * 0.5, r);
            assert!(!p.done && p.y < e.path[0][2] && p.y > e.path.last().unwrap()[2], "{shape:?}: mid y {}", p.y);
        }
        // a ladder climbs at the climb speed, whatever the hunter's walk speed
        let (nav, upper, _) = test_nav(FlightShape::Switchback, true);
        let e = nav.edges_from(Node { level: 0, cell: upper.stairs[0].landing() }).next().unwrap().clone();
        let mut w = EdgeWalk::start(&e);
        let p = w.advance(9.0, 1.0, r);
        assert!(approx(w.s, r.ladder_speed().min(e.len)), "{} m in a second", w.s);
        assert!(!p.done || e.len <= r.ladder_speed());
    }

    #[test]
    fn a_drop_walk_falls_at_the_fall_speed() {
        let (upper, lower) = Facility::test_levels_drop();
        let r = FlightEdgeRules { drop_allowed: true, ..rules().clone() };
        let nav = test_nav_of(&upper, &lower, &r);
        // the bank cell of a drop is the first open neighbour of a hole cell, so find the edge by its kind
        let e = (16..22)
            .flat_map(|y| (16..22).map(move |x| Node { level: 0, cell: Cell::new(x, y) }))
            .flat_map(|n| nav.edges_from(n))
            .find(|e| e.key.kind == EdgeKind::Drop)
            .expect("a drop edge")
            .clone();
        let mut w = EdgeWalk::start(&e);
        let mut ys = vec![];
        for _ in 0..400 {
            let p = w.advance(2.0, 0.02, &r);
            ys.push(p.y);
            if p.done {
                break;
            }
        }
        assert!(w.s >= w.len && ys.windows(2).all(|p| p[1] <= p[0] + 1e-4));
        assert!(approx(*ys.last().unwrap(), -STOREY));
    }

    #[test]
    fn sight_passes_an_open_well_and_stops_at_a_slab() {
        let (upper, lower) = Facility::test_levels(FlightShape::Switchback, false);
        let mut stack = LevelStack::default();
        stack.put(Resident { level: -1, center: (0, 0), fac: lower.clone(), fix: Fixtures::default() });
        let floors = Floors { active: &upper, stack: &stack };
        let well = upper.stairs[0].well_cells()[0].center();
        assert!(sight_clear_3d(&floors, [well.0, well.1, 1.6], [well.0, well.1, -3.4]), "down the open well");
        assert!(sight_clear_3d(&floors, [well.0, well.1, -3.4], [well.0, well.1, 1.6]), "and back up");
        // the same drop in height over solid floor is a slab
        assert!(!sight_clear_3d(&floors, [20.5, 20.5, 1.6], [20.5, 20.5, -3.4]), "a slab blocks");
        // an unresident level blocks
        let empty = LevelStack::default();
        let alone = Floors { active: &upper, stack: &empty };
        assert!(!sight_clear_3d(&alone, [well.0, well.1, 1.6], [well.0, well.1, -3.4]));
        // on one level it is has_line_of_sight
        for (a, b) in [((10.5, 10.5), (20.5, 12.5)), ((10.5, 10.5), (2.5, 2.5)), ((6.5, 6.5), (28.5, 28.5)), ((10.5, 10.5), (1.5, 20.5))] {
            assert_eq!(
                sight_clear_3d(&floors, [a.0, a.1, 1.6], [b.0, b.1, 1.6]),
                upper.has_line_of_sight(a, b),
                "{a:?} to {b:?}"
            );
        }
    }

    #[test]
    fn the_embedded_flights_json_parses_and_bad_data_is_refused() {
        let text = include_str!("../data/hunters/flights.json");
        let r = FlightEdgeRules::parse(text).expect("embedded file parses");
        assert_eq!(&r, rules());
        assert!(!r.drop_allowed && r.edge_cost_scale >= 1.0 && r.hear_adjacent_scale > 0.0 && r.hear_adjacent_scale <= 1.0);
        assert!(r.capture_height > 0.0 && r.edge_stuck_seconds > 0.0 && r.resident_hunter_cap > 0);
        let bad = [
            ("\"capture_height\": 1.2", "\"capture_height\": 99.0"),
            ("\"same_layer_eps\": 0.5", "\"same_layer_eps\": 5.0"),
            ("\"edge_cost_scale\": 1.0", "\"edge_cost_scale\": 0.5"),
            ("\"hear_adjacent_scale\": 0.5", "\"hear_adjacent_scale\": -0.1"),
            ("\"resident_hunter_cap\": 8", "\"resident_hunter_cap\": 2.5"),
            ("\"drop_allowed\": false", "\"drop_allowed\": 0"),
            ("\"ladder_speed_scale\": 1.0,", ""),
            ("lr.hunter_flights/1", "lr.hunter_flights/2"),
        ];
        for (from, to) in bad {
            assert!(text.contains(from), "test text `{from}` is in the file");
            assert!(FlightEdgeRules::parse(&text.replace(from, to)).is_err(), "`{to}` must be refused");
        }
        assert!(FlightEdgeRules::parse("not json").is_err());
    }

    #[test]
    fn stacknav_build_is_deterministic() {
        for (shape, ladder) in [(FlightShape::Switchback, false), (FlightShape::Spiral, false), (FlightShape::Straight, true)] {
            let (a, upper, lower) = test_nav(shape, ladder);
            let b = test_nav_of(&upper, &lower, rules());
            assert_eq!(a, b);
            assert_eq!(a.version(), b.version());
            assert_eq!(a.levels(), vec![-1, 0]);
            assert!(a.has_level(0) && !a.has_level(1));
        }
        // a changed cost gives a new version
        let (upper, lower) = Facility::test_levels(FlightShape::Switchback, false);
        let heavy = FlightEdgeRules { edge_cost_scale: 3.0, ..rules().clone() };
        assert_ne!(test_nav_of(&upper, &lower, &heavy).version(), test_nav_of(&upper, &lower, rules()).version());
        assert_eq!(StackNav::empty().levels(), Vec::<i32>::new());
    }
}
