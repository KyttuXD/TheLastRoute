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
use std::cell::RefCell;
use std::collections::{BTreeMap, BinaryHeap};
use std::f32::consts::SQRT_2;
use crate::shared::atomic64::AtomicU64;
use std::sync::atomic::Ordering as AtomicOrdering;
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

/// Metres of one grid cell (`Cell::center` is cell + 0.5): the unit `EdgeWalk::shift` moves by.
const CELL_M: f32 = 1.0;

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
    /// Seconds without progress on an edge before the walker replans: a hunter whose next step on a flight is refused
    /// for this long turns round and walks back along the edge.
    pub edge_stuck_seconds: f32,
    /// Simulated hunters per resident floor beside the active one.
    pub resident_hunter_cap: u32,
    /// The most cells one `StackNav::find_path` may expand before it gives up and returns an empty route.
    pub search_expansions: u32,
    /// Cells a goal on a closed cell (a stair tread) may move to reach the nearest open one (`StackNav::open_goal`).
    pub goal_snap_radius: f32,
    /// Seconds between two re-plans of a walker that is crossing levels.
    pub cross_repath_seconds: f32,
    /// Seconds a hunter keeps following a target that left its level before it gives up.
    pub follow_cross_seconds: f32,
    /// How far (cells) from the near end of the flight the runner took a hunter that was not after the runner still
    /// follows it to the other level.
    pub follow_cross_cells: f32,
    /// Seconds after the runner walked onto a level during which a hunter spawned or lined up there starts awake and
    /// wandering instead of investigating where the runner stands.
    pub arrival_grace_seconds: f32,
    /// Chance (0 to 1) that a wandering hunter or bot picks a goal on another level.
    pub wander_cross_chance: f32,
    /// Chance (0 to 1) that a searching hunter picks a goal on another level.
    pub search_cross_chance: f32,
    /// Largest offset (m) from the centre of a landing cell a hunter is set down at (clamped to keep the body in the cell).
    pub land_spread: f32,
    /// How far (cells) from a hunter a stair is looked for when it picks a goal on another level.
    pub cross_wander_radius: f32,
    /// The longest gap (m) between two consecutive points of an edge's polyline. A ladder's shaft and a drop's fall are one
    /// long segment in the route; the points inserted on them lie on the same line, so the length, the cost and the walk
    /// speed are unchanged, but the polyline is continuous for a consumer that samples its points.
    pub max_point_gap: f32,
    /// Hard cap (m per frame) on the overlap push of `director::separate`.
    pub separation_max_step: f32,
    /// Extra metres past the grab distance within which a capture needs no clear line (brain.rs).
    pub capture_los_slack: f32,
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

/// `num` for a key an older file may lack: absent takes `default`; present, it must still be a number inside the limits.
fn opt_num(v: &Value, key: &str, default: f32, lo: f32, hi: f32) -> Result<f32, String> {
    if v.get(key).is_none() {
        return Ok(default);
    }
    num(v, key, lo, hi)
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
        let expansions = num(&v, "search_expansions", 1.0, 1_000_000.0)?;
        if expansions.fract() != 0.0 {
            return Err(format!("flights.json: `search_expansions` = {expansions} must be a whole number"));
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
            search_expansions: expansions as u32,
            goal_snap_radius: num(&v, "goal_snap_radius", 0.0, 16.0)?,
            cross_repath_seconds: num(&v, "cross_repath_seconds", 0.05, 30.0)?,
            follow_cross_seconds: num(&v, "follow_cross_seconds", 0.1, 120.0)?,
            follow_cross_cells: opt_num(&v, "follow_cross_cells", 40.0, 0.0, 512.0)?,
            arrival_grace_seconds: opt_num(&v, "arrival_grace_seconds", 6.0, 0.0, 120.0)?,
            wander_cross_chance: num(&v, "wander_cross_chance", 0.0, 1.0)?,
            search_cross_chance: num(&v, "search_cross_chance", 0.0, 1.0)?,
            land_spread: num(&v, "land_spread", 0.0, 0.4)?,
            cross_wander_radius: num(&v, "cross_wander_radius", 1.0, 256.0)?,
            max_point_gap: opt_num(&v, "max_point_gap", 0.5, 0.05, 4.0)?,
            separation_max_step: opt_num(&v, "separation_max_step", 0.15, 0.01, 1.0)?,
            capture_los_slack: opt_num(&v, "capture_los_slack", 0.15, 0.0, 1.0)?,
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
    /// Connected component of each cell (4-way over the open cells): 0 for a closed cell, else 1 up to `comps`. More
    /// than `u16::MAX` components share the last id, which only ever makes `StackNav::reachable` say yes.
    pub comp: Vec<u16>,
    /// How many component ids `comp` uses.
    pub comps: u32,
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
        let (comp, comps) = flood_components(&open, fac.cols, fac.rows);
        LevelGrid { level: fac.level, cols: fac.cols, rows: fac.rows, open, comp, comps }
    }

    fn index(&self, c: Cell) -> Option<usize> {
        (c.x >= 0 && c.y >= 0 && c.x < self.cols && c.y < self.rows).then(|| (c.y * self.cols + c.x) as usize)
    }

    /// Open ground (`Facility::hunter_open`).
    pub fn is_open(&self, c: Cell) -> bool {
        self.index(c).is_some_and(|i| self.open[i])
    }
}

/// 4-way flood fill of the open cells: the component id of each cell (0 closed) and how many ids were used. Build time only.
fn flood_components(open: &[bool], cols: i32, rows: i32) -> (Vec<u16>, u32) {
    let mut comp = vec![0u16; open.len()];
    let mut next = 0u32;
    let mut stack: Vec<u32> = Vec::new();
    for start in 0..open.len() {
        if !open[start] || comp[start] != 0 {
            continue;
        }
        next = (next + 1).min(u16::MAX as u32);
        let id = next as u16;
        comp[start] = id;
        stack.push(start as u32);
        while let Some(i) = stack.pop() {
            let (x, y) = (i as i32 % cols, i as i32 / cols);
            for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= cols || ny >= rows {
                    continue;
                }
                let j = (ny * cols + nx) as usize;
                if open[j] && comp[j] == 0 {
                    comp[j] = id;
                    stack.push(j as u32);
                }
            }
        }
    }
    (comp, next)
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

/// Where one level's cells sit in the dense search arrays (`offset + y * cols + x`) and its first global component id.
#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    level: i32,
    offset: usize,
    cols: i32,
    rows: i32,
    comp_base: u32,
}

#[derive(Clone, Debug, Default)]
pub struct StackNav {
    grids: BTreeMap<i32, LevelGrid>,
    out: BTreeMap<Node, Vec<Edge>>,
    by_key: BTreeMap<EdgeKey, Edge>,
    /// Unique per `build` (0 for an empty graph): a consumer that caches a hop against it drops the cache on a rebuild.
    version: u64,
    /// The rules the graph was built with (`None` for an empty graph: the embedded rules apply).
    rules: Option<FlightEdgeRules>,
    /// The levels in ascending order with their array offsets (derived from `grids`).
    slots: Vec<Slot>,
    total_cells: usize,
    /// Directed links between global component ids, one list per component (derived from `out`).
    comp_adj: Vec<Vec<u32>>,
}

/// Two graphs are equal when they hold the same grids, edges and rules; `version` (a build id) and the derived search
/// tables are not part of that.
impl PartialEq for StackNav {
    fn eq(&self, o: &StackNav) -> bool {
        self.grids == o.grids && self.out == o.out && self.by_key == o.by_key && self.rules == o.rules
    }
}

static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

/// One open-list entry of the A*: ordered so the lowest `f` pops first, ties by node (deterministic).
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

/// The reused arrays of one thread's searches. A cell's entry is valid only when its stamp equals `gen`, so a search
/// starts without clearing anything and allocates nothing once the arrays have grown to the nav's size.
#[derive(Default)]
struct Scratch {
    gen: u32,
    seen: Vec<u32>,
    closed: Vec<u32>,
    g: Vec<f32>,
    came: Vec<(Node, Option<EdgeKey>)>,
    heap: BinaryHeap<Item>,
    // component search
    comp_seen: Vec<u32>,
    comp_gen: u32,
    queue: Vec<u32>,
}

impl Scratch {
    fn begin(&mut self, cells: usize) {
        if self.seen.len() < cells {
            self.seen.resize(cells, 0);
            self.closed.resize(cells, 0);
            self.g.resize(cells, 0.0);
            self.came.resize(cells, (Node { level: 0, cell: Cell::new(0, 0) }, None));
        }
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            self.seen.fill(0);
            self.closed.fill(0);
            self.gen = 1;
        }
        self.heap.clear();
    }

    fn begin_comps(&mut self, comps: usize) {
        if self.comp_seen.len() < comps {
            self.comp_seen.resize(comps, 0);
        }
        self.comp_gen = self.comp_gen.wrapping_add(1);
        if self.comp_gen == 0 {
            self.comp_seen.fill(0);
            self.comp_gen = 1;
        }
        self.queue.clear();
    }

    #[cfg(test)]
    fn capacity(&self) -> usize {
        self.seen.capacity() + self.closed.capacity() + self.g.capacity() + self.came.capacity() + self.heap.capacity()
            + self.comp_seen.capacity() + self.queue.capacity()
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::default());
}

/// The summed capacity of this thread's search arrays: a second identical search must not grow it.
#[cfg(test)]
pub(crate) fn scratch_capacity() -> usize {
    SCRATCH.with(|s| s.borrow().capacity())
}

/// Splits every segment of `p` longer than `max_gap` into equal pieces on the same line.
fn densify(p: Vec<[f32; 3]>, max_gap: f32) -> Vec<[f32; 3]> {
    let mut out: Vec<[f32; 3]> = Vec::with_capacity(p.len());
    for (i, b) in p.iter().enumerate() {
        if i > 0 {
            let a = p[i - 1];
            let d = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
            let n = (d / max_gap).ceil() as usize;
            for k in 1..n {
                let t = k as f32 / n as f32;
                out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]);
            }
        }
        out.push(*b);
    }
    out
}

fn octile(a: Cell, b: Cell) -> f32 {
    let (dx, dy) = ((a.x - b.x).abs() as f32, (a.y - b.y).abs() as f32);
    dx.max(dy) + (SQRT_2 - 1.0) * dx.min(dy)
}

fn path_len_3d(p: &[[f32; 3]]) -> f32 {
    p.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2) + (w[1][2] - w[0][2]).powi(2)).sqrt()).sum()
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
        nav.index();
        nav.version = NEXT_VERSION.fetch_add(1, AtomicOrdering::Relaxed);
        nav
    }

    /// Fills the derived search tables (level slots and the component graph) once the grids and edges are in.
    fn index(&mut self) {
        let (mut offset, mut comp_base) = (0usize, 0u32);
        self.slots.clear();
        for (level, g) in &self.grids {
            self.slots.push(Slot { level: *level, offset, cols: g.cols, rows: g.rows, comp_base });
            offset += (g.cols * g.rows).max(0) as usize;
            comp_base += g.comps;
        }
        self.total_cells = offset;
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); comp_base as usize];
        for edges in self.out.values() {
            for e in edges {
                if let (Some(a), Some(b)) = (self.comp_of(e.from), self.comp_of(e.to)) {
                    if a != b && !adj[a as usize].contains(&b) {
                        adj[a as usize].push(b);
                    }
                }
            }
        }
        self.comp_adj = adj;
    }

    fn slot(&self, level: i32) -> Option<&Slot> {
        self.slots.binary_search_by_key(&level, |s| s.level).ok().map(|i| &self.slots[i])
    }

    /// The dense array index of a cell inside its level's grid.
    fn index_of(&self, n: Node) -> Option<usize> {
        let s = self.slot(n.level)?;
        (n.cell.x >= 0 && n.cell.y >= 0 && n.cell.x < s.cols && n.cell.y < s.rows)
            .then(|| s.offset + (n.cell.y * s.cols + n.cell.x) as usize)
    }

    /// The global component id of an open cell, or `None` for a closed or missing cell.
    fn comp_of(&self, n: Node) -> Option<u32> {
        let s = self.slot(n.level)?;
        let g = self.grids.get(&n.level)?;
        if n.cell.x < 0 || n.cell.y < 0 || n.cell.x >= s.cols || n.cell.y >= s.rows {
            return None;
        }
        match g.comp[(n.cell.y * s.cols + n.cell.x) as usize] {
            0 => None,
            c => Some(s.comp_base + c as u32 - 1),
        }
    }

    /// A cheap early-out for [`StackNav::find_path`]: `false` only when no route can exist, i.e. when both cells are
    /// open and the component of `to` cannot be reached from the component of `from` over the level grids and the
    /// edges. `true` otherwise, also when either cell is closed or missing (the exact search decides). With every step
    /// allowed it equals "a route exists". No allocation after the first call on a thread.
    pub fn reachable(&self, from: Node, to: Node) -> bool {
        let (Some(a), Some(b)) = (self.comp_of(from), self.comp_of(to)) else { return true };
        if a == b {
            return true;
        }
        SCRATCH.with(|cell| match cell.try_borrow_mut() {
            Ok(mut sc) => self.components_reach(&mut sc, a, b),
            Err(_) => self.components_reach(&mut Scratch::default(), a, b),
        })
    }

    fn components_reach(&self, sc: &mut Scratch, a: u32, b: u32) -> bool {
        sc.begin_comps(self.comp_adj.len());
        let gen = sc.comp_gen;
        sc.comp_seen[a as usize] = gen;
        sc.queue.push(a);
        let mut head = 0;
        while head < sc.queue.len() {
            let c = sc.queue[head];
            head += 1;
            if c == b {
                return true;
            }
            for &n in &self.comp_adj[c as usize] {
                if sc.comp_seen[n as usize] != gen {
                    sc.comp_seen[n as usize] = gen;
                    sc.queue.push(n);
                }
            }
        }
        false
    }

    /// `cell` when it is open on `level`, else the nearest open cell within the `goal_snap_radius` rule (a goal that
    /// stands on a stair tread: the hunter grid closes the well cells). `None` when the level has no grid or nothing
    /// is open that close.
    pub fn open_goal(&self, level: i32, cell: Cell) -> Option<Cell> {
        let grid = self.grids.get(&level)?;
        if grid.is_open(cell) {
            return Some(cell);
        }
        let radius = self.rules().goal_snap_radius;
        let near = nearest_open(grid, cell, radius.ceil() as i32)?;
        let (dx, dy) = ((near.x - cell.x) as f32, (near.y - cell.y) as f32);
        (dx * dx + dy * dy <= radius * radius + 1e-4).then_some(near)
    }

    /// The edges that start on `level` with their entrance within `radius` metres of `(x, z)`, in a stable order
    /// (by entrance cell, then the order the edges were added). A two-way edge appears on both of its levels, each time
    /// as the record that starts there. No allocation.
    pub fn edges_near(&self, level: i32, x: f32, z: f32, radius: f32) -> impl Iterator<Item = &Edge> {
        let lo = Node { level, cell: Cell::new(i32::MIN, i32::MIN) };
        let hi = Node { level, cell: Cell::new(i32::MAX, i32::MAX) };
        self.out.range(lo..=hi).flat_map(|(_, v)| v.iter()).filter(move |e| {
            let p = e.path.first().copied().unwrap_or([f32::INFINITY; 3]);
            (p[0] - x).hypot(p[1] - z) <= radius
        })
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
        let path: Arc<[[f32; 3]]> =
            densify(vec![[bc.0, bc.1, uy], [cc.0, cc.1, uy], [cc.0, cc.1, ly], [lc.0, lc.1, ly]], rules.max_point_gap).into();
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

    /// A number unique to this build (a counter, 0 for an empty graph): two builds never share one, even over identical
    /// levels, so a hop cached against it is dropped when the graph is rebuilt.
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
    /// `from` (via `None`); empty when `from` or `to` is not open, no route exists, [`StackNav::reachable`] says no, or
    /// the search spent the `search_expansions` rule. A one-way edge is never walked backwards. The search runs on dense
    /// arrays kept per thread (stamped, never cleared), so after warm-up a call allocates only the route it returns.
    pub fn find_path(&self, from: Node, to: Node, step_cost: &dyn Fn(i32, Cell, Cell) -> Option<f32>) -> Vec<RouteStep> {
        if !self.is_open(from) || !self.is_open(to) {
            return vec![];
        }
        if from == to {
            return vec![RouteStep { node: from, via: None }];
        }
        if !self.reachable(from, to) {
            return vec![];
        }
        SCRATCH.with(|cell| match cell.try_borrow_mut() {
            Ok(mut sc) => self.search(&mut sc, from, to, step_cost),
            Err(_) => self.search(&mut Scratch::default(), from, to, step_cost),
        })
    }

    fn search(&self, sc: &mut Scratch, from: Node, to: Node, step_cost: &dyn Fn(i32, Cell, Cell) -> Option<f32>) -> Vec<RouteStep> {
        let (Some(fi), Some(_)) = (self.index_of(from), self.index_of(to)) else { return vec![] };
        let cap = self.rules().search_expansions;
        sc.begin(self.total_cells);
        let gen = sc.gen;
        let h = |n: Node| octile(n.cell, to.cell);
        sc.g[fi] = 0.0;
        sc.seen[fi] = gen;
        sc.heap.push(Item { f: h(from), node: from });
        let mut expanded = 0u32;
        let mut found = false;
        while let Some(Item { node: cur, .. }) = sc.heap.pop() {
            if cur == to {
                found = true;
                break;
            }
            let Some(ci) = self.index_of(cur) else { continue };
            if sc.closed[ci] == gen {
                continue;
            }
            sc.closed[ci] = gen;
            expanded += 1;
            if expanded > cap {
                sc.heap.clear();
                return vec![];
            }
            let gc = sc.g[ci];
            let Some(grid) = self.grids.get(&cur.level) else { continue };
            let offset = self.slot(cur.level).map_or(0, |s| s.offset);
            let relax = |sc: &mut Scratch, i: usize, n: Node, ng: f32, via: Option<EdgeKey>| {
                if sc.seen[i] != gen || ng < sc.g[i] {
                    sc.seen[i] = gen;
                    sc.g[i] = ng;
                    sc.came[i] = (cur, via);
                    sc.heap.push(Item { f: ng + h(n), node: n });
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
                let i = offset + (nb.y * grid.cols + nb.x) as usize;
                relax(sc, i, Node { level: cur.level, cell: nb }, gc + step, None);
            }
            for e in self.edges_from(cur) {
                if self.is_open(e.to) {
                    if let Some(i) = self.index_of(e.to) {
                        relax(sc, i, e.to, gc + e.cost, Some(e.key));
                    }
                }
            }
        }
        sc.heap.clear();
        if !found {
            return vec![];
        }
        let mut route = vec![];
        let mut at = to;
        loop {
            if at == from {
                route.push(RouteStep { node: at, via: None });
                break;
            }
            let (prev, via) = match self.index_of(at) {
                Some(i) if sc.seen[i] == gen => sc.came[i],
                _ => (at, None),
            };
            route.push(RouteStep { node: at, via });
            if prev == at {
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
pub(crate) fn nearest_open(grid: &LevelGrid, c: Cell, reach: i32) -> Option<Cell> {
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
        path: densify(path, rules.max_point_gap).into(),
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

    /// Whether both walks follow the same edge in the same direction (same ends and key): the collision pass uses it to
    /// make a walker queue behind the one ahead.
    pub fn shares_edge(&self, other: &EdgeWalk) -> bool {
        self.from == other.from && self.to == other.to && self.key == other.key
    }

    /// Moves the whole walk by `dx_cells` and `dz_cells` whole cells (1 m each): the polyline, the end cells and the key
    /// cell. For a window recentre, so a walker in the middle of an edge keeps walking instead of snapping. The new
    /// polyline is a fresh `Arc`: this is the only allocation, and it happens on this call alone.
    pub fn shift(&mut self, dx_cells: i32, dz_cells: i32) {
        if dx_cells == 0 && dz_cells == 0 {
            return;
        }
        let (dx, dz) = (dx_cells as f32 * CELL_M, dz_cells as f32 * CELL_M);
        let moved: Vec<[f32; 3]> = self.path.iter().map(|p| [p[0] + dx, p[1] + dz, p[2]]).collect();
        self.path = moved.into();
        for c in [&mut self.from.cell, &mut self.to.cell, &mut self.key.cell] {
            c.x += dx_cells;
            c.y += dz_cells;
        }
    }

    /// The same walk turned round at the point it has reached: the polyline reversed, the ends swapped and the distance
    /// measured from the other end, so the pose (x, z and y) is the one it had and the walk goes back to where it began.
    /// For a walker whose way ahead stays refused. A drop is one way and is never turned (nothing refuses a fall).
    pub fn reversed(&self) -> EdgeWalk {
        let mut path: Vec<[f32; 3]> = self.path.to_vec();
        path.reverse();
        EdgeWalk {
            key: self.key,
            kind: self.kind,
            path: path.into(),
            s: (self.len - self.s).clamp(0.0, self.len),
            len: self.len,
            from: self.to,
            to: self.from,
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
        assert_eq!((r.search_expansions, r.goal_snap_radius), (6000, 4.0));
        assert!(approx(r.cross_repath_seconds, 0.9) && approx(r.follow_cross_seconds, 25.0));
        assert!(approx(r.follow_cross_cells, 40.0) && approx(r.arrival_grace_seconds, 6.0));
        assert!(approx(r.wander_cross_chance, 0.15) && approx(r.search_cross_chance, 0.25));
        let bad = [
            ("\"capture_height\": 1.2", "\"capture_height\": 99.0"),
            ("\"same_layer_eps\": 0.5", "\"same_layer_eps\": 5.0"),
            ("\"edge_cost_scale\": 1.0", "\"edge_cost_scale\": 0.5"),
            ("\"hear_adjacent_scale\": 0.5", "\"hear_adjacent_scale\": -0.1"),
            ("\"resident_hunter_cap\": 8", "\"resident_hunter_cap\": 2.5"),
            ("\"search_expansions\": 6000", "\"search_expansions\": 6000.5"),
            ("\"search_expansions\": 6000", "\"search_expansions\": 0"),
            ("\"wander_cross_chance\": 0.15", "\"wander_cross_chance\": 1.5"),
            ("\"search_cross_chance\": 0.25", "\"search_cross_chance\": -0.5"),
            ("\"cross_repath_seconds\": 0.9", "\"cross_repath_seconds\": 0.0"),
            ("\"goal_snap_radius\": 4,", ""),
            ("\"drop_allowed\": false", "\"drop_allowed\": 0"),
            ("\"ladder_speed_scale\": 1.0,", ""),
            ("lr.hunter_flights/1", "lr.hunter_flights/2"),
            ("\"max_point_gap\": 0.5", "\"max_point_gap\": 0.0"),
            ("\"separation_max_step\": 0.15", "\"separation_max_step\": 5.0"),
            ("\"capture_los_slack\": 0.15", "\"capture_los_slack\": -1.0"),
            ("\"follow_cross_cells\": 40.0", "\"follow_cross_cells\": -1.0"),
            ("\"arrival_grace_seconds\": 6.0", "\"arrival_grace_seconds\": 500.0"),
        ];
        for (from, to) in bad {
            assert!(text.contains(from), "test text `{from}` is in the file");
            assert!(FlightEdgeRules::parse(&text.replace(from, to)).is_err(), "`{to}` must be refused");
        }
        assert!(FlightEdgeRules::parse("not json").is_err());
        assert!(approx(r.max_point_gap, 0.5) && approx(r.separation_max_step, 0.15) && approx(r.capture_los_slack, 0.15));
        // an older file without the newer keys loads with today's values
        let old = text
            .replace("\n  \"follow_cross_cells\": 40.0,", "")
            .replace("\n  \"arrival_grace_seconds\": 6.0,", "")
            .replace(",
  \"max_point_gap\": 0.5", "")
            .replace(",
  \"separation_max_step\": 0.15", "")
            .replace(",
  \"capture_los_slack\": 0.15", "");
        assert!(!old.contains("max_point_gap\":") && !old.contains("capture_los_slack\":"));
        assert!(!old.contains("follow_cross_cells\":") && !old.contains("arrival_grace_seconds\":"));
        assert_eq!(FlightEdgeRules::parse(&old).expect("an old file loads"), r);
    }

    #[test]
    fn stacknav_build_is_deterministic() {
        for (shape, ladder) in [(FlightShape::Switchback, false), (FlightShape::Spiral, false), (FlightShape::Straight, true)] {
            let (a, upper, lower) = test_nav(shape, ladder);
            let b = test_nav_of(&upper, &lower, rules());
            assert_eq!(a, b, "equal content");
            assert_ne!(a.version(), b.version(), "but each build has its own id");
            assert_eq!(a.levels(), vec![-1, 0]);
            assert!(a.has_level(0) && !a.has_level(1));
        }
        // a changed cost gives a new version
        let (upper, lower) = Facility::test_levels(FlightShape::Switchback, false);
        let heavy = FlightEdgeRules { edge_cost_scale: 3.0, ..rules().clone() };
        assert_ne!(test_nav_of(&upper, &lower, &heavy), test_nav_of(&upper, &lower, rules()), "a changed cost is a changed graph");
        assert_eq!(StackNav::empty().levels(), Vec::<i32>::new());
    }

    // --- nav3d: the 3D layer other systems consume ---

    /// The three stair shapes of the test stages: a spiral, a switchback and a ladder, with the edge kind each makes.
    const STAGES: [(FlightShape, bool, EdgeKind); 3] = [
        (FlightShape::Spiral, false, EdgeKind::Flight),
        (FlightShape::Switchback, false, EdgeKind::Flight),
        (FlightShape::Switchback, true, EdgeKind::Ladder),
    ];

    fn gap3(a: &[f32; 3], b: &[f32; 3]) -> f32 {
        ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
    }

    #[test]
    fn nav3d_finds_a_path_down_a_spiral_a_switchback_and_a_ladder() {
        for (shape, ladder, kind) in STAGES {
            let (nav, _, _) = test_nav(shape, ladder);
            let from = Node { level: 0, cell: Cell::new(6, 6) };
            let to = Node { level: -1, cell: Cell::new(22, 22) };
            let route = nav.find_path(from, to, &free);
            assert!(!route.is_empty(), "{shape:?} ladder={ladder}: a route down");
            assert_eq!((route.first().unwrap().node, route.last().unwrap().node), (from, to));
            let vias: Vec<EdgeKey> = route.iter().filter_map(|s| s.via).collect();
            assert_eq!(vias.len(), 1, "{shape:?}: one edge");
            assert_eq!(vias[0].kind, kind, "{shape:?}: the edge kind");
            let e = nav.edge(vias[0]).expect("the edge is in the graph");
            assert!(e.path.len() >= 2);
            for w in e.path.windows(2) {
                assert!(w[1][2] <= w[0][2] + 1e-4, "{shape:?}: y rises going down: {:?} then {:?}", w[0], w[1]);
                assert!(gap3(&w[0], &w[1]) <= 0.6, "{shape:?}: a gap of {} m in the polyline", gap3(&w[0], &w[1]));
            }
            assert!(approx(e.path[0][2], 0.0) && approx(e.path.last().unwrap()[2], -STOREY), "{shape:?}: from floor to floor");
            // the walk and the route agree, and the way back up is a route too
            assert!(approx(e.len, e.path.windows(2).map(|w| gap3(&w[0], &w[1])).sum::<f32>()));
            assert!(!nav.find_path(to, from, &free).is_empty(), "{shape:?}: and back up");
        }
    }

    #[test]
    fn open_goal_moves_a_tread_goal_to_open_ground_within_the_radius() {
        let radius = rules().goal_snap_radius;
        for (shape, ladder, _) in STAGES {
            let (nav, upper, _) = test_nav(shape, ladder);
            let wells = upper.stairs[0].well_cells();
            let mut snapped = 0;
            for c in wells {
                let got = nav.open_goal(0, c).unwrap_or_else(|| panic!("{shape:?}: a goal for tread {c:?}"));
                assert!(nav.is_open(Node { level: 0, cell: got }), "{shape:?}: {got:?} is open");
                let d = (((got.x - c.x).pow(2) + (got.y - c.y).pow(2)) as f32).sqrt();
                assert!(d <= radius + 1e-3, "{shape:?}: {got:?} is {d} cells from {c:?}");
                if nav.is_open(Node { level: 0, cell: c }) {
                    assert_eq!(got, c, "an open cell is its own goal");
                } else {
                    snapped += 1;
                }
            }
            if !ladder {
                assert!(snapped > 0, "{shape:?}: some tread of the well is closed ground");
            }
        }
        let (nav, _, _) = test_nav(FlightShape::Switchback, false);
        assert_eq!(nav.open_goal(0, Cell::new(20, 20)), Some(Cell::new(20, 20)));
        assert_eq!(nav.open_goal(-7, Cell::new(20, 20)), None, "no grid on that level");
        assert_eq!(nav.open_goal(0, Cell::new(-100, -100)), None, "nothing open that close");
    }

    #[test]
    fn reachable_is_false_for_a_closed_pocket_and_matches_the_exact_search() {
        // a 3 x 3 room sealed by a ring of wall
        let (mut upper, lower) = Facility::test_levels(FlightShape::Switchback, false);
        for dz in -2i32..=2 {
            for dx in -2i32..=2 {
                if dx.abs() == 2 || dz.abs() == 2 {
                    upper.wall[(24 + dz) as usize][(24 + dx) as usize] = true;
                }
            }
        }
        upper.rebuild_grids();
        let nav = test_nav_of(&upper, &lower, rules());
        let pocket = Node { level: 0, cell: Cell::new(24, 24) };
        let outside = Node { level: 0, cell: Cell::new(6, 6) };
        assert!(nav.is_open(pocket) && nav.is_open(outside));
        assert!(!nav.reachable(outside, pocket) && !nav.reachable(pocket, outside));
        assert!(nav.find_path(outside, pocket, &free).is_empty());
        let sample = |nav: &StackNav| -> Vec<Node> {
            [(0, 6, 6), (0, 20, 20), (0, 24, 24), (-1, 22, 22), (-1, 6, 6), (-1, 16, 16), (0, 8, 8), (-1, 8, 9)]
                .iter()
                .map(|(l, x, y)| Node { level: *l, cell: Cell::new(*x, *y) })
                .filter(|n| nav.is_open(*n))
                .collect()
        };
        let mut checked = 0;
        for (nav, name) in [(&nav, "pocket")] {
            for a in sample(nav) {
                for b in sample(nav) {
                    let exact = !nav.find_path(a, b, &free).is_empty();
                    assert_eq!(nav.reachable(a, b), exact, "{name}: {a:?} to {b:?}");
                    checked += 1;
                }
            }
        }
        for (shape, ladder, _) in STAGES {
            let (nav, _, _) = test_nav(shape, ladder);
            for a in sample(&nav) {
                for b in sample(&nav) {
                    let exact = !nav.find_path(a, b, &free).is_empty();
                    assert_eq!(nav.reachable(a, b), exact, "{shape:?}: {a:?} to {b:?}");
                    assert!(exact, "{shape:?}: the open stage is one space");
                    checked += 1;
                }
            }
        }
        assert!(checked > 40);
        // a closed or missing cell is left to the exact search
        let wall = Node { level: 0, cell: Cell::new(22, 24) };
        assert!(!nav.is_open(wall) && nav.reachable(outside, wall) && nav.reachable(wall, outside));
        assert!(nav.reachable(outside, Node { level: -9, cell: Cell::new(1, 1) }));
        // a one-way drop is reachable down and not up
        let (upper, lower) = Facility::test_levels_drop();
        let on = test_nav_of(&upper, &lower, &FlightEdgeRules { drop_allowed: true, ..rules().clone() });
        let above = Node { level: 0, cell: Cell::new(17, 18) };
        let below = Node { level: -1, cell: Cell::new(12, 12) };
        assert!(on.reachable(above, below) && !on.reachable(below, above));
        let off = test_nav_of(&upper, &lower, &FlightEdgeRules { drop_allowed: false, ..rules().clone() });
        assert!(!off.reachable(above, below));
    }

    #[test]
    fn an_edge_walk_shifted_by_whole_cells_walks_the_same_edge_offset() {
        let r = rules();
        let (nav, upper, _) = test_nav(FlightShape::Switchback, false);
        let e = nav.edges_from(Node { level: 0, cell: upper.stairs[0].cell }).next().unwrap().clone();
        let mut plain = EdgeWalk::start(&e);
        let mut moved = plain.clone();
        assert!(plain.shares_edge(&moved));
        moved.shift(3, -2);
        assert!(!plain.shares_edge(&moved), "the shifted walk names other cells");
        let (a, b) = (plain.pose(), moved.pose());
        assert!(approx(b.x, a.x + 3.0) && approx(b.z, a.z - 2.0) && approx(b.y, a.y), "{a:?} vs {b:?}");
        assert_eq!((moved.from.cell.x, moved.from.cell.y), (plain.from.cell.x + 3, plain.from.cell.y - 2));
        assert_eq!((moved.to.cell.x, moved.to.cell.y), (plain.to.cell.x + 3, plain.to.cell.y - 2));
        assert_eq!((moved.key.cell.x, moved.key.cell.y), (plain.key.cell.x + 3, plain.key.cell.y - 2));
        assert_eq!(moved.key.kind, plain.key.kind);
        assert!(Arc::ptr_eq(&plain.path, &e.path), "the original keeps its polyline");
        let mut steps = 0;
        loop {
            let (pa, pb) = (plain.advance(2.0, 0.07, r), moved.advance(2.0, 0.07, r));
            assert!(approx(pb.x, pa.x + 3.0) && approx(pb.z, pa.z - 2.0) && approx(pb.y, pa.y), "step {steps}: {pa:?} vs {pb:?}");
            assert_eq!(pa.done, pb.done);
            assert!(approx(plain.fraction(), moved.fraction()));
            steps += 1;
            if pa.done || steps > 2000 {
                break;
            }
        }
        assert!(plain.pose().done);
        // a zero shift keeps the same polyline; the opposite direction is not the same edge
        let mut same = EdgeWalk::start(&e);
        let before = same.path.clone();
        same.shift(0, 0);
        assert!(Arc::ptr_eq(&before, &same.path));
        let back = nav.edges_from(e.to).find(|b| b.key == e.key).unwrap();
        assert!(!EdgeWalk::start(&e).shares_edge(&EdgeWalk::start(back)));
    }

    #[test]
    fn a_reversed_walk_keeps_its_pose_and_ends_where_it_began() {
        let r = rules();
        for (shape, ladder) in [(FlightShape::Straight, false), (FlightShape::Switchback, false), (FlightShape::Spiral, false), (FlightShape::Straight, true)] {
            let (nav, upper, _) = test_nav(shape, ladder);
            let e = nav.edges_near(upper.level, 0.0, 0.0, f32::INFINITY).next().expect("the stage has an edge").clone();
            let mut w = EdgeWalk::start(&e);
            while w.fraction() < 0.4 {
                w.advance(2.0, 0.05, r);
            }
            let (here, mut back) = (w.pose(), w.reversed());
            let turned = back.pose();
            assert!(gap3(&[here.x, here.z, here.y], &[turned.x, turned.z, turned.y]) < 1e-3, "{shape:?}: {here:?} vs {turned:?}");
            assert_eq!((back.from, back.to, back.key), (w.to, w.from, w.key), "{shape:?}: the ends are swapped, the edge is the same");
            assert!(!back.shares_edge(&w), "{shape:?}: it now walks the other way");
            let mut prev = [turned.x, turned.z, turned.y];
            let mut steps = 0;
            loop {
                let p = back.advance(2.0, 0.05, r);
                assert!(gap3(&prev, &[p.x, p.z, p.y]) <= 2.0 * 0.05 + 0.1 + r.ladder_speed() * 0.05, "{shape:?}: step {steps} is continuous");
                prev = [p.x, p.z, p.y];
                steps += 1;
                if p.done || steps > 4000 {
                    break;
                }
            }
            let start = w.start_point();
            assert!(back.pose().done && gap3(&prev, &start) < 1e-3, "{shape:?}: back at the end it began on ({prev:?} vs {start:?})");
        }
    }

    #[test]
    fn find_path_twice_is_identical_and_reuses_its_arrays() {
        for (shape, ladder, _) in STAGES {
            let (nav, _, _) = test_nav(shape, ladder);
            let from = Node { level: 0, cell: Cell::new(28, 5) };
            let to = Node { level: -1, cell: Cell::new(5, 28) };
            let first = nav.find_path(from, to, &free);
            assert!(!first.is_empty(), "{shape:?}");
            let cap = scratch_capacity();
            assert!(cap > 0);
            let second = nav.find_path(from, to, &free);
            assert_eq!(first, second, "{shape:?}: deterministic");
            assert_eq!(scratch_capacity(), cap, "{shape:?}: the second call allocated no search arrays");
            // a different route in the same nav, and the pair reversed, also stay inside the arrays
            let _ = nav.find_path(to, from, &free);
            let _ = nav.next_hop(from, to, &free);
            assert_eq!(scratch_capacity(), cap, "{shape:?}: nor did other searches");
        }
    }

    #[test]
    fn each_build_has_its_own_version() {
        let (upper, lower) = Facility::test_levels(FlightShape::Straight, false);
        let a = test_nav_of(&upper, &lower, rules());
        let b = test_nav_of(&upper, &lower, rules());
        assert_eq!(a, b);
        assert!(a.version() != 0 && b.version() != 0 && a.version() != b.version());
        assert_eq!(a.clone().version(), a.version(), "a clone is the same build");
        assert_eq!(StackNav::empty().version(), 0);
    }

    #[test]
    fn the_expansion_cap_ends_a_search_with_an_empty_route() {
        let (upper, lower) = Facility::test_levels(FlightShape::Switchback, false);
        let from = Node { level: 0, cell: Cell::new(28, 5) };
        let to = Node { level: -1, cell: Cell::new(5, 28) };
        let roomy = test_nav_of(&upper, &lower, rules());
        assert!(!roomy.find_path(from, to, &free).is_empty(), "the default cap is enough");
        let tiny = test_nav_of(&upper, &lower, &FlightEdgeRules { search_expansions: 5, ..rules().clone() });
        assert!(tiny.reachable(from, to));
        assert!(tiny.find_path(from, to, &free).is_empty(), "five expansions cannot cross two levels");
        assert!(tiny.next_hop(from, to, &free).is_none());
        // a goal far off the grid is no route at once, whatever the cap
        let huge = Node { level: -1, cell: Cell::new(10_000, 10_000) };
        assert!(roomy.find_path(from, huge, &free).is_empty());
        // a goal in a sealed pocket ends at the early-out
        let sealed = Node { level: 0, cell: Cell::new(24, 24) };
        let mut walled = upper.clone();
        for dz in -2i32..=2 {
            for dx in -2i32..=2 {
                if dx.abs() == 2 || dz.abs() == 2 {
                    walled.wall[(24 + dz) as usize][(24 + dx) as usize] = true;
                }
            }
        }
        walled.rebuild_grids();
        let nav = test_nav_of(&walled, &lower, rules());
        assert!(nav.is_open(sealed) && !nav.reachable(from, sealed));
        assert!(nav.find_path(from, sealed, &free).is_empty());
    }

    #[test]
    fn edges_near_lists_the_entrances_around_a_point_in_a_stable_order() {
        let (nav, upper, _) = test_nav(FlightShape::Switchback, false);
        let e = nav.edges_from(Node { level: 0, cell: upper.stairs[0].cell }).next().unwrap();
        let (x, z) = (e.path[0][0], e.path[0][1]);
        let near: Vec<&Edge> = nav.edges_near(0, x + 0.3, z - 0.2, 0.6).collect();
        assert_eq!(near.len(), 1);
        assert_eq!((near[0].key, near[0].from.level), (e.key, 0));
        assert_eq!(nav.edges_near(0, x + 5.0, z, 0.6).count(), 0, "out of reach");
        assert_eq!(nav.edges_near(7, x, z, 100.0).count(), 0, "no edge starts on a level without one");
        let low: Vec<&Edge> = nav.edges_near(-1, 0.0, 0.0, 1000.0).collect();
        assert_eq!(low.len(), 1);
        assert_eq!((low[0].from.level, low[0].to.level, low[0].key), (-1, 0, e.key), "the record that starts below");
        let again: Vec<&Edge> = nav.edges_near(0, x, z, 1000.0).collect();
        assert!(again.iter().zip(nav.edges_near(0, x, z, 1000.0)).all(|(a, b)| std::ptr::eq(*a, b)), "the same order twice");
    }
}
