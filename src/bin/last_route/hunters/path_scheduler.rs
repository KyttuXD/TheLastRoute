//! Hunter route planning under a per-frame budget (data/hunters/pathing.json).
//!
//! Every hunter that needs a grid route (a repath timer running out, a hearing check, a search point) used to run a full
//! A* on the spot, so a crowd of hunters made a crowd of searches in one frame. This module puts those requests behind:
//!
//! * a frame budget: `begin_frame` opens a frame and at most `searches_per_frame` searches run in it. A route cut from a
//!   shared goal field costs one unit, and so does building the field. A request past the budget is queued, answered
//!   first thing next frame (lowest priority number first, then first asked), and the brain keeps walking the route it
//!   has until then;
//! * an LRU cache of finished routes, keyed by everything the plan reads (the facility, the start and goal cells, the
//!   cells a step was just blocked by, the gate cells, the water field version and the hunter's water rules). A cached
//!   route is fresh for `cache_ttl_frames` frames;
//! * a shared goal distance field: when `field_min_sharers` or more queued requests aim at one goal on one walk surface,
//!   one reverse Dijkstra from the goal answers all of them by walking downhill, with the same step, corner and water
//!   costs as the A* in `brain::plan_route_wet` (so the same cost; the tie between equal routes may differ).
//!
//! Without an open frame (a unit test, or any caller that never runs `begin_frame`) nothing is budgeted or cached: every
//! request is the plain search, exactly as before.
//!
//! Determinism: queued requests are ordered by an integer priority and then by the sequence they were first asked in,
//! neighbours are visited in a fixed order, and no HashMap is iterated for output. The same requests in the same frames
//! give the same routes.

use super::brain::{cell_of, plan_route_wet, walk_open, Standable};
use super::water::{HunterWater, WaterField, WaterPlan};
use crate::world::{Cell, Facility};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BinaryHeap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

/// data/hunters/pathing.json: the budget, the cache and the shared fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathingRules {
    /// Units (searches, field builds) a frame may spend.
    pub searches_per_frame: usize,
    /// Routes the LRU cache keeps.
    pub cache_entries: usize,
    /// Frames a cached route stays fresh.
    pub cache_ttl_frames: u64,
    /// Queued requests on one goal and surface needed before a shared goal field is built for them.
    pub field_min_sharers: usize,
    /// Goal fields kept at once.
    pub field_cache_entries: usize,
    /// Frames a queued request is kept after it was last asked for (a brain that no longer needs it stops asking).
    pub queue_keep_frames: u64,
    /// Units a frame keeps for requests from resident (not active) floors, spent even when the active floor used the whole budget.
    pub resident_searches_per_frame: usize,
}

impl PathingRules {
    pub fn parse(text: &str) -> Result<Self, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let whole = |key: &str| -> Result<u64, String> {
            v.get(key).and_then(|x| x.as_u64()).ok_or_else(|| format!("{key} must be a whole number"))
        };
        let rules = PathingRules {
            searches_per_frame: whole("searches_per_frame")? as usize,
            cache_entries: whole("cache_entries")? as usize,
            cache_ttl_frames: whole("cache_ttl_frames")?,
            field_min_sharers: whole("field_min_sharers")? as usize,
            field_cache_entries: whole("field_cache_entries")? as usize,
            queue_keep_frames: whole("queue_keep_frames")?,
            resident_searches_per_frame: whole("resident_searches_per_frame")? as usize,
        };
        if rules.searches_per_frame == 0 || rules.cache_entries == 0 || rules.field_cache_entries == 0 {
            return Err("searches_per_frame, cache_entries and field_cache_entries must be at least 1".into());
        }
        if rules.field_min_sharers < 2 {
            return Err("field_min_sharers must be at least 2".into());
        }
        Ok(rules)
    }

    pub fn get() -> &'static PathingRules {
        static RULES: OnceLock<PathingRules> = OnceLock::new();
        RULES.get_or_init(|| {
            PathingRules::parse(include_str!("../data/hunters/pathing.json"))
                .unwrap_or_else(|e| panic!("data/hunters/pathing.json: {e}"))
        })
    }
}

/// One route request: what `plan_route_wet` reads, borrowed from the brain.
pub struct PlanRequest<'a> {
    pub from: (f32, f32),
    pub to: Cell,
    pub avoid: &'a [(Cell, f32)],
    pub gates: &'a [Cell],
    /// The water field, the hunter's water rules and whether it is chasing; None for the dry plan.
    pub water: Option<(&'a Arc<WaterField>, &'a HunterWater, bool)>,
    /// Lower goes first when the request has to wait for the budget.
    pub priority: u64,
}

/// The answer to a request.
pub enum Route {
    /// The route as `plan_route_wet` plans it (empty when there is none).
    Ready(Vec<Cell>),
    /// The frame's budget is spent. Keep the route you have; the answer is queued and ready from the next frame.
    Deferred,
}

/// Counters since `reset_thread`, for the bench and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// A* searches run (`plan_route_wet`).
    pub searches: u64,
    /// Shared goal fields built.
    pub field_builds: u64,
    /// Routes read off a goal field.
    pub field_routes: u64,
    /// Requests answered from the cache.
    pub cache_hits: u64,
    /// Requests deferred to the next frame.
    pub deferred: u64,
}

type WaterKey = (u64, bool, u64);

/// Everything a route depends on, as hashable values.
#[derive(Clone, PartialEq, Eq, Hash)]
struct PathKey {
    fac: u64,
    from: Cell,
    to: Cell,
    avoid: Vec<Cell>,
    gates: Vec<Cell>,
    water: Option<WaterKey>,
    /// The door state of the facility (`door_signature`): a door that opens or shuts makes a new key.
    doors: u64,
}

/// Everything a goal field depends on: the goal, the gates and the water.
#[derive(Clone, PartialEq, Eq, Hash)]
struct FieldKey {
    fac: u64,
    to: Cell,
    gates: Vec<Cell>,
    water: Option<WaterKey>,
    doors: u64,
}

/// The facility a route was planned on: its address, seed, size and window origin.
fn fac_key(fac: &Facility) -> u64 {
    let mut h = DefaultHasher::new();
    (fac as *const Facility as usize).hash(&mut h);
    fac.seed_text.hash(&mut h);
    (fac.cols, fac.rows, fac.origin).hash(&mut h);
    h.finish()
}

/// A hash of every room door's state (wanted open or shut, and whether its swing has settled fully open or fully shut),
/// so a route or goal field planned with a door in one state is never reused once the door changes. It changes when a
/// door is toggled and when its swing ends, not on every frame of the swing: a per-frame key would drop every queued
/// request and goal field while any door in the window animates.
fn door_signature(fac: &Facility) -> u64 {
    let mut h = DefaultHasher::new();
    for p in fac.props.iter().filter(|p| p.door.is_some()) {
        let open = fac.door_open_amount(p.cell);
        (p.cell, fac.door_is_open(p.cell), open >= 0.999, open <= 0.001).hash(&mut h);
    }
    h.finish()
}

fn water_key(water: Option<(&Arc<WaterField>, &HunterWater, bool)>) -> Option<WaterKey> {
    water.map(|(field, rules, chasing)| {
        let mut h = DefaultHasher::new();
        format!("{rules:?}").hash(&mut h);
        (field.version(), chasing, h.finish())
    })
}

fn path_key(
    fac: u64,
    doors: u64,
    from: (f32, f32),
    to: Cell,
    avoid: &[(Cell, f32)],
    gates: &[Cell],
    water: Option<WaterKey>,
) -> PathKey {
    PathKey {
        fac,
        from: cell_of(from),
        to,
        avoid: avoid.iter().map(|(c, _)| *c).collect(),
        gates: gates.to_vec(),
        water,
        doors,
    }
}

fn field_key(fac: u64, doors: u64, to: Cell, gates: &[Cell], water: Option<WaterKey>) -> FieldKey {
    FieldKey { fac, to, gates: gates.to_vec(), water, doors }
}

/// What one search reads, borrowed (a queued request lends its own copies).
struct Job<'a> {
    from: (f32, f32),
    to: Cell,
    avoid: &'a [(Cell, f32)],
    gates: &'a [Cell],
    water: Option<(&'a WaterField, &'a HunterWater, bool)>,
}

/// A request waiting for the budget. Owned, so it can be answered in a later frame.
struct Queued {
    fkey: FieldKey,
    from: (f32, f32),
    to: Cell,
    avoid: Vec<(Cell, f32)>,
    gates: Vec<Cell>,
    water: Option<(Arc<WaterField>, HunterWater, bool)>,
    priority: u64,
    seq: u64,
    asked: u64,
}

impl Queued {
    fn job(&self) -> Job<'_> {
        Job {
            from: self.from,
            to: self.to,
            avoid: &self.avoid,
            gates: &self.gates,
            water: self.water.as_ref().map(|(field, rules, chasing)| (&**field, rules, *chasing)),
        }
    }
}

const NEIGHBOURS: [(i32, i32); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

fn step_len(dx: i32, dy: i32) -> f32 {
    if dx != 0 && dy != 0 {
        std::f32::consts::SQRT_2
    } else {
        1.0
    }
}

/// The cost factor of entering `c` (1 with no water plan; the planner only enters open cells).
fn enter_cost(water: Option<&WaterPlan>, c: Cell) -> f32 {
    water.map_or(1.0, |w| w.step_factor((c.x, c.y)).unwrap_or(1.0))
}

/// Index of `c` in the facility's grid (the layout of `Facility::hunter_open`), None off the grid.
fn index(fac: &Facility, c: Cell) -> Option<usize> {
    if c.x >= 0 && c.y >= 0 && c.x < fac.cols && c.y < fac.rows {
        Some((c.y * fac.cols + c.x) as usize)
    } else {
        None
    }
}

/// A frontier entry for the reverse Dijkstra: the smallest distance pops first, ties by cell.
#[derive(PartialEq)]
struct Front(f32, Cell);

impl Eq for Front {}

impl PartialOrd for Front {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Front {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then_with(|| (o.1.x, o.1.y).cmp(&(self.1.x, self.1.y)))
    }
}

/// The cost from every cell to `to` (infinite where no route reaches), over the cells `walk_open` allows,
/// with the same corner rule and entry costs as the forward search. One reverse Dijkstra serves every start.
fn build_field(fac: &Facility, to: Cell, gates: &[Cell], water: Option<&WaterPlan>) -> Vec<f32> {
    let cells = (fac.cols.max(0) * fac.rows.max(0)) as usize;
    let stand = Standable::new(fac);
    let mut dist = vec![f32::INFINITY; cells];
    let Some(at) = index(fac, to) else { return dist };
    dist[at] = 0.0;
    let mut frontier = BinaryHeap::from([Front(0.0, to)]);
    while let Some(Front(d, u)) = frontier.pop() {
        let Some(iu) = index(fac, u) else { continue };
        if d > dist[iu] {
            continue;
        }
        let entered = enter_cost(water, u);
        for (dx, dy) in NEIGHBOURS {
            // the forward step x -> u is (dx, dy)
            let x = Cell::new(u.x - dx, u.y - dy);
            let Some(ix) = index(fac, x) else { continue };
            if !walk_open(&stand, x, gates, water) {
                continue;
            }
            if dx != 0
                && dy != 0
                && (!walk_open(&stand, Cell::new(x.x + dx, x.y), gates, water)
                    || !walk_open(&stand, Cell::new(x.x, x.y + dy), gates, water))
            {
                continue;
            }
            let nd = d + step_len(dx, dy) * entered;
            if nd < dist[ix] {
                dist[ix] = nd;
                frontier.push(Front(nd, x));
            }
        }
    }
    dist
}

/// The route from `start` down the field to `to`: each step takes the open neighbour that is strictly closer to the goal
/// and costs least to reach it. None if the field does not lead there (the caller searches instead).
fn descend(fac: &Facility, dist: &[f32], start: Cell, to: Cell, gates: &[Cell], water: Option<&WaterPlan>) -> Option<Vec<Cell>> {
    let at = |c: Cell| index(fac, c).map_or(f32::INFINITY, |i| dist[i]);
    let stand = Standable::new(fac);
    let mut cur = start;
    let mut path = vec![start];
    while cur != to {
        if path.len() > dist.len() {
            return None;
        }
        let here = at(cur);
        if !here.is_finite() {
            return None;
        }
        let mut best: Option<(f32, Cell)> = None;
        for (dx, dy) in NEIGHBOURS {
            let n = Cell::new(cur.x + dx, cur.y + dy);
            let dn = at(n);
            if !(dn < here) {
                continue;
            }
            if dx != 0
                && dy != 0
                && (!walk_open(&stand, Cell::new(cur.x + dx, cur.y), gates, water)
                    || !walk_open(&stand, Cell::new(cur.x, cur.y + dy), gates, water))
            {
                continue;
            }
            let cand = dn + step_len(dx, dy) * enter_cost(water, n);
            if best.map_or(true, |(b, _)| cand < b) {
                best = Some((cand, n));
            }
        }
        cur = best?.1;
        path.push(cur);
    }
    Some(path)
}

/// An LRU of finished routes. `touch` makes a hit the most recently used; a route older than the TTL is a miss.
#[derive(Default)]
struct Cache {
    cap: usize,
    ttl: u64,
    tick: u64,
    map: HashMap<PathKey, Entry>,
    order: BTreeMap<u64, PathKey>,
}

struct Entry {
    path: Vec<Cell>,
    tick: u64,
    frame: u64,
}

impl Cache {
    fn configure(&mut self, cap: usize, ttl: u64) {
        self.cap = cap;
        self.ttl = ttl;
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    fn touch(&mut self, key: &PathKey, frame: u64) -> bool {
        let Some(entry) = self.map.get_mut(key) else { return false };
        if frame.saturating_sub(entry.frame) > self.ttl {
            let tick = entry.tick;
            self.map.remove(key);
            self.order.remove(&tick);
            return false;
        }
        self.order.remove(&entry.tick);
        self.tick += 1;
        entry.tick = self.tick;
        self.order.insert(self.tick, key.clone());
        true
    }

    fn get(&mut self, key: &PathKey, frame: u64) -> Option<Vec<Cell>> {
        if !self.touch(key, frame) {
            return None;
        }
        self.map.get(key).map(|e| e.path.clone())
    }

    fn put(&mut self, key: PathKey, path: Vec<Cell>, frame: u64) {
        if let Some(old) = self.map.remove(&key) {
            self.order.remove(&old.tick);
        }
        self.tick += 1;
        self.order.insert(self.tick, key.clone());
        self.map.insert(key, Entry { path, tick: self.tick, frame });
        while self.map.len() > self.cap {
            let Some((_, oldest)) = self.order.pop_first() else { break };
            self.map.remove(&oldest);
        }
    }
}

struct Field {
    dist: Vec<f32>,
    frame: u64,
}

#[derive(Default)]
struct State {
    /// Units spent this frame on resident floors' requests out of their own reserve.
    resident_used: usize,
    /// facility -> (frame, signature): the door signature, worked out once per frame and facility.
    door_sigs: HashMap<u64, (u64, u64)>,
    /// A frame is open: `begin_frame` has run on this thread. Only an open frame budgets and caches.
    open: bool,
    /// The facility the open frame belongs to (`fac_key`).
    fac: Option<u64>,
    frame: u64,
    /// Units spent this frame.
    used: usize,
    budget_override: Option<usize>,
    field_cap: usize,
    cache: Cache,
    fields: Vec<(FieldKey, Field)>,
    queue: HashMap<PathKey, Queued>,
    seq: u64,
    stats: Stats,
}

impl State {
    fn budget(&self) -> usize {
        self.budget_override.unwrap_or(PathingRules::get().searches_per_frame)
    }

    /// A goal field older than the cache TTL is stale (the floor may have changed under it): it counts as absent.
    fn fresh(&self, f: &Field) -> bool {
        self.frame.saturating_sub(f.frame) <= PathingRules::get().cache_ttl_frames
    }

    fn doors_of(&mut self, fac: &Facility, fk: u64) -> u64 {
        match self.door_sigs.get(&fk) {
            Some((frame, sig)) if *frame == self.frame => *sig,
            _ => {
                let sig = door_signature(fac);
                self.door_sigs.insert(fk, (self.frame, sig));
                sig
            }
        }
    }

    fn has_field(&self, key: &FieldKey) -> bool {
        self.fields.iter().any(|(k, f)| k == key && self.fresh(f))
    }

    fn field(&self, key: &FieldKey) -> Option<&Field> {
        self.fields.iter().find(|(k, f)| k == key && self.fresh(f)).map(|(_, f)| f)
    }

    fn insert_field(&mut self, key: FieldKey, dist: Vec<f32>) {
        self.fields.retain(|(k, _)| *k != key);
        if self.fields.len() >= self.field_cap.max(1) {
            let oldest = (0..self.fields.len()).min_by_key(|i| self.fields[*i].1.frame);
            if let Some(i) = oldest {
                self.fields.swap_remove(i);
            }
        }
        let frame = self.frame;
        self.fields.push((key, Field { dist, frame }));
    }

    /// Plans one route. Returns it with the units it cost: a goal field build (only when `build`), then either the
    /// descent on the field (when one exists for this goal) or the A* search.
    fn plan_job(&mut self, fac: &Facility, job: &Job, fkey: &FieldKey, build: bool) -> (Vec<Cell>, usize) {
        let stand = Standable::new(fac);
        let plan = job.water.map(|(field, rules, chasing)| WaterPlan { field, rules, chasing });
        let here = cell_of(job.from);
        let mut units = 0;
        if job.avoid.is_empty()
            && walk_open(&stand, here, job.gates, plan.as_ref())
            && walk_open(&stand, job.to, job.gates, plan.as_ref())
        {
            if build && !self.has_field(fkey) {
                let dist = build_field(fac, job.to, job.gates, plan.as_ref());
                self.insert_field(fkey.clone(), dist);
                self.stats.field_builds += 1;
                units += 1;
            }
            if let Some(field) = self.field(fkey) {
                if let Some(path) = descend(fac, &field.dist, here, job.to, job.gates, plan.as_ref()) {
                    self.stats.field_routes += 1;
                    return (path, units + 1);
                }
            }
        }
        self.stats.searches += 1;
        (plan_route_wet(fac, job.from, job.to, job.avoid, job.gates, plan.as_ref()), units + 1)
    }
}

thread_local! {
    static SCHED: RefCell<State> = RefCell::new(State::default());
}

/// Opens a frame for `fac`: the budget refills, the queued requests of the last frame are answered (by priority) up to
/// the budget, and only then do the brains ask. Call once per frame before the hunters update. A new facility (or the
/// same one with a moved window) drops everything computed for the old one.
pub fn begin_frame(fac: &Facility) {
    let rules = *PathingRules::get();
    let fk = fac_key(fac);
    SCHED.with(|cell| {
        let mut guard = cell.borrow_mut();
        let s = &mut *guard;
        if s.fac != Some(fk) {
            s.fac = Some(fk);
            s.fields.clear();
            s.queue.clear();
            s.cache.clear();
        }
        s.open = true;
        s.frame += 1;
        s.used = 0;
        s.resident_used = 0;
        s.field_cap = rules.field_cache_entries;
        s.cache.configure(rules.cache_entries, rules.cache_ttl_frames);
        let frame = s.frame;
        s.fields.retain(|(_, f)| frame.saturating_sub(f.frame) <= rules.cache_ttl_frames);
        let doors = s.doors_of(fac, fk);
        // a request planned against other door states is stale: its asker will ask again with the current ones
        s.queue.retain(|k, q| (k.fac != fk || k.doors == doors) && frame.saturating_sub(q.asked) <= rules.queue_keep_frames);
        // this facility's requests only: a resident floor's requests wait for their own facility
        let mut order: Vec<(u64, u64, PathKey)> = s
            .queue
            .iter()
            .filter(|(k, _)| k.fac == fk)
            .map(|(k, q)| (q.priority, q.seq, k.clone()))
            .collect();
        order.sort_by_key(|(priority, seq, _)| (*priority, *seq));
        let mut sharers: HashMap<FieldKey, usize> = HashMap::new();
        for (_, _, key) in &order {
            let q = &s.queue[key];
            if q.avoid.is_empty() {
                *sharers.entry(q.fkey.clone()).or_insert(0) += 1;
            }
        }
        let budget = s.budget();
        for (_, _, key) in order {
            if s.cache.touch(&key, frame) {
                s.queue.remove(&key);
                continue;
            }
            if s.used >= budget {
                break;
            }
            let Some(q) = s.queue.remove(&key) else { continue };
            let shared = q.avoid.is_empty() && sharers.get(&q.fkey).copied().unwrap_or(0) >= rules.field_min_sharers;
            let build = shared && budget - s.used >= 2 && !s.has_field(&q.fkey);
            let job = q.job();
            let (path, units) = s.plan_job(fac, &job, &q.fkey, build);
            s.used += units;
            s.cache.put(key, path, frame);
        }
    });
}

/// Answers one route request. Without an open frame this is the plain search. With one: a cached route is returned at
/// once; otherwise the search runs now if the frame's budget allows, and if not, the request is queued and `Deferred`.
pub fn plan(fac: &Facility, req: PlanRequest<'_>) -> Route {
    SCHED.with(|cell| {
        let mut guard = cell.borrow_mut();
        let s = &mut *guard;
        if !s.open {
            let plan = req.water.map(|(field, rules, chasing)| WaterPlan { field: &**field, rules, chasing });
            return Route::Ready(plan_route_wet(fac, req.from, req.to, req.avoid, req.gates, plan.as_ref()));
        }
        let fk = fac_key(fac);
        let wk = water_key(req.water);
        let doors = s.doors_of(fac, fk);
        let key = path_key(fk, doors, req.from, req.to, req.avoid, req.gates, wk);
        let frame = s.frame;
        if let Some(path) = s.cache.get(&key, frame) {
            s.stats.cache_hits += 1;
            return Route::Ready(path);
        }
        let fkey = field_key(fk, doors, req.to, req.gates, wk);
        // a resident floor keeps a small reserve, so a busy active floor cannot starve its hunters
        let resident = s.fac.is_some_and(|a| a != fk);
        let in_budget = s.used < s.budget();
        let from_reserve = !in_budget && resident && s.resident_used < PathingRules::get().resident_searches_per_frame;
        if in_budget || from_reserve {
            let job = Job {
                from: req.from,
                to: req.to,
                avoid: req.avoid,
                gates: req.gates,
                water: req.water.map(|(field, rules, chasing)| (&**field, rules, chasing)),
            };
            let (path, units) = s.plan_job(fac, &job, &fkey, false);
            if from_reserve {
                s.resident_used += units;
            } else {
                s.used += units;
            }
            s.cache.put(key, path.clone(), frame);
            return Route::Ready(path);
        }
        s.stats.deferred += 1;
        let seq = match s.queue.get(&key) {
            Some(q) => q.seq,
            None => {
                s.seq += 1;
                s.seq
            }
        };
        let queued = Queued {
            fkey,
            from: req.from,
            to: req.to,
            avoid: req.avoid.to_vec(),
            gates: req.gates.to_vec(),
            water: req.water.map(|(field, rules, chasing)| (Arc::clone(field), *rules, chasing)),
            priority: req.priority,
            seq,
            asked: frame,
        };
        s.queue.insert(key, queued);
        Route::Deferred
    })
}

/// Overrides the frame budget on this thread (the bench's knob); None goes back to data/hunters/pathing.json.
pub fn set_searches_per_frame(n: Option<usize>) {
    SCHED.with(|cell| cell.borrow_mut().budget_override = n);
}

/// Units spent in the open frame so far.
pub fn units_this_frame() -> usize {
    SCHED.with(|cell| cell.borrow().used)
}

/// The counters of this thread.
pub fn stats() -> Stats {
    SCHED.with(|cell| cell.borrow().stats)
}

/// Back to a closed frame with empty caches and counters (tests and the bench start from here).
pub fn reset_thread() {
    SCHED.with(|cell| *cell.borrow_mut() = State::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    fn facility() -> Facility {
        let cat = Catalog::load();
        Facility::generate("SCHED-1", &Options::default(), &cat)
    }

    /// Up to `take` pairs of floor cells that a route leads between, each at least `min_len` cells long.
    fn pairs(fac: &Facility, min_len: usize, take: usize) -> Vec<(Cell, Cell)> {
        let cells = fac.floor_cells();
        let mut out = vec![];
        for a in cells.iter().step_by(37) {
            for b in cells.iter().rev().step_by(41) {
                if a != b && fac.find_path(*a, *b).len() >= min_len {
                    out.push((*a, *b));
                    if out.len() == take {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// Up to `n` distinct floor cells with a route to `goal`.
    fn starts_for(fac: &Facility, goal: Cell, n: usize) -> Vec<Cell> {
        let mut out = vec![];
        for c in fac.floor_cells() {
            if c != goal && fac.find_path(c, goal).len() >= 4 {
                out.push(c);
                if out.len() == n {
                    break;
                }
            }
        }
        out
    }

    fn ask(fac: &Facility, from: Cell, to: Cell, priority: u64) -> Route {
        plan(fac, PlanRequest { from: from.center(), to, avoid: &[], gates: &[], water: None, priority })
    }

    fn route_of(r: Route) -> Option<Vec<Cell>> {
        match r {
            Route::Ready(p) => Some(p),
            Route::Deferred => None,
        }
    }

    /// The step cost a route pays, as the planner counts it (entered cell's factor, diagonals sqrt 2).
    fn cost(route: &[Cell], water: Option<&WaterPlan>) -> f32 {
        route
            .windows(2)
            .map(|w| step_len(w[1].x - w[0].x, w[1].y - w[0].y) * enter_cost(water, w[1]))
            .sum()
    }

    #[test]
    fn without_an_open_frame_the_request_is_the_plain_search() {
        reset_thread();
        let fac = facility();
        let (a, b) = pairs(&fac, 6, 1)[0];
        let got = route_of(ask(&fac, a, b, 0)).expect("nothing is budgeted without a frame");
        assert_eq!(got, plan_route_wet(&fac, a.center(), b, &[], &[], None));
        assert_eq!(stats(), Stats::default(), "no frame, no cache, no counting");
    }

    #[test]
    fn a_frame_runs_no_more_searches_than_its_budget() {
        reset_thread();
        set_searches_per_frame(Some(3));
        let fac = facility();
        let ps = pairs(&fac, 6, 10);
        assert_eq!(ps.len(), 10, "the test floor has ten routes");
        begin_frame(&fac);
        let answered = ps.iter().enumerate().filter(|(i, (a, b))| route_of(ask(&fac, *a, *b, *i as u64)).is_some()).count();
        assert_eq!(answered, 3, "three searches in the frame, the rest deferred");
        assert_eq!(units_this_frame(), 3);
        assert_eq!(stats().deferred, 7);
        // the next frame answers the queued routes by priority, and still no more than the budget
        begin_frame(&fac);
        assert_eq!(units_this_frame(), 3, "the queue takes the whole budget first");
        assert!(route_of(ask(&fac, ps[3].0, ps[3].1, 3)).is_some(), "the first queued route is cached");
        assert!(route_of(ask(&fac, ps[9].0, ps[9].1, 9)).is_none(), "the last one is still waiting");
        assert_eq!(units_this_frame(), 3, "a cache hit costs nothing and a deferred request costs nothing");
    }

    #[test]
    fn queued_routes_are_answered_most_urgent_first() {
        reset_thread();
        set_searches_per_frame(Some(1));
        let fac = facility();
        let ps = pairs(&fac, 6, 3);
        begin_frame(&fac);
        assert!(route_of(ask(&fac, ps[0].0, ps[0].1, 5)).is_some(), "the one search of the frame");
        assert!(route_of(ask(&fac, ps[1].0, ps[1].1, 9)).is_none(), "queued with a low priority");
        assert!(route_of(ask(&fac, ps[2].0, ps[2].1, 0)).is_none(), "queued with the top priority");
        begin_frame(&fac);
        assert!(route_of(ask(&fac, ps[2].0, ps[2].1, 0)).is_some(), "the urgent route is answered first");
        assert!(route_of(ask(&fac, ps[1].0, ps[1].1, 9)).is_none(), "the other one waits another frame");
        begin_frame(&fac);
        assert!(route_of(ask(&fac, ps[1].0, ps[1].1, 9)).is_some(), "and then it is answered");
    }

    /// Runs one request script over a few frames and returns every answer (None: deferred), in order.
    fn script(fac: &Facility, ps: &[(Cell, Cell)]) -> Vec<Option<Vec<Cell>>> {
        reset_thread();
        set_searches_per_frame(Some(2));
        let mut out = vec![];
        for frame in 0..4 {
            begin_frame(fac);
            for (i, (a, b)) in ps.iter().enumerate() {
                out.push(route_of(ask(fac, *a, *b, ((i + frame) % 3) as u64)));
            }
        }
        out
    }

    #[test]
    fn the_same_requests_in_the_same_frames_give_the_same_routes() {
        let fac = facility();
        let ps = pairs(&fac, 6, 8);
        let first = script(&fac, &ps);
        let second = script(&fac, &ps);
        assert_eq!(first, second);
        assert!(first.iter().any(|r| r.is_none()), "the budget deferred some of them");
        assert!(first.iter().flatten().any(|r| !r.is_empty()), "and answered others");
    }

    #[test]
    fn a_repeated_request_is_answered_from_the_cache_and_expires() {
        reset_thread();
        set_searches_per_frame(Some(4));
        let fac = facility();
        let (a, b) = pairs(&fac, 6, 1)[0];
        begin_frame(&fac);
        let first = route_of(ask(&fac, a, b, 0)).expect("the budget allows one search");
        let spent = units_this_frame();
        let again = route_of(ask(&fac, a, b, 0)).expect("a cached route is never deferred");
        assert_eq!(first, again);
        assert_eq!(units_this_frame(), spent, "a hit spends no budget");
        assert_eq!(stats().cache_hits, 1);
        assert_eq!(stats().searches, 1);
        for _ in 0..=PathingRules::get().cache_ttl_frames {
            begin_frame(&fac);
        }
        assert!(route_of(ask(&fac, a, b, 0)).is_some());
        assert_eq!(stats().searches, 2, "an expired route is searched again");
    }

    #[test]
    fn requests_that_share_a_goal_share_one_field() {
        reset_thread();
        set_searches_per_frame(Some(1));
        let fac = facility();
        // the one unit of frame 1 goes to an unrelated route (the first pair), so the five below all wait
        let (a, goal) = pairs(&fac, 6, 1)[0];
        let starts: Vec<Cell> = starts_for(&fac, goal, 6).into_iter().filter(|c| *c != a).take(5).collect();
        assert_eq!(starts.len(), 5, "the test floor has five routes to one goal");
        begin_frame(&fac);
        assert!(route_of(ask(&fac, a, goal, 0)).is_some());
        for s in &starts {
            assert!(route_of(ask(&fac, *s, goal, 1)).is_none(), "queued");
        }
        set_searches_per_frame(Some(6));
        begin_frame(&fac);
        let st = stats();
        assert_eq!(st.field_builds, 1, "one reverse search for the goal");
        assert_eq!(st.field_routes, 5, "all five routes read off it");
        for s in &starts {
            assert!(route_of(ask(&fac, *s, goal, 1)).is_some(), "answered from the cache");
        }
    }

    #[test]
    fn a_goal_field_gives_routes_of_the_searched_cost() {
        let fac = facility();
        let ps = pairs(&fac, 4, 40);
        assert!(ps.len() >= 20, "the test floor has routes to check");
        // a shallow-water band across part of the floor, so the entry costs differ from cell to cell
        let band: Vec<((i32, i32), f32)> = fac.floor_cells().iter().step_by(5).map(|c| ((c.x, c.y), 0.3)).collect();
        let field = WaterField::from_depths(&band, &[]);
        let rules = HunterWater::default();
        let wet = WaterPlan { field: &field, rules: &rules, chasing: true };
        for water in [None, Some(&wet)] {
            for (a, b) in &ps {
                let dist = build_field(&fac, *b, &[], water);
                let searched = plan_route_wet(&fac, a.center(), *b, &[], &[], water);
                let down = descend(&fac, &dist, *a, *b, &[], water).expect("the field leads from the start to the goal");
                assert_eq!((down.first(), down.last()), (Some(a), Some(b)));
                assert!(
                    (cost(&down, water) - cost(&searched, water)).abs() < 1e-3,
                    "{a:?} -> {b:?}: field {} vs search {}",
                    cost(&down, water),
                    cost(&searched, water)
                );
            }
        }
    }
    #[test]
    fn a_door_changing_state_makes_the_cached_route_stale() {
        reset_thread();
        set_searches_per_frame(Some(8));
        let mut fac = facility();
        let (a, b) = pairs(&fac, 6, 1)[0];
        let door = fac.floor_cells()[0];
        fac.props.push(crate::world::facility::Prop {
            id: "door".into(),
            cell: door,
            ptype: "normalRoomDoor".into(),
            room: 0,
            rotation: 0,
            mount_y: None,
            solid: false,
            half: (0.5, 0.1),
            offset: (0.0, 0.0),
            door: Some(crate::world::facility::DoorDraw { span: 1, offset: [0.0, 0.0] }),
            sewer: None,
            crawl_under: false,
        });
        fac.update_doors(&[], &[(1e6, 1e6)], 0.0);
        begin_frame(&fac);
        assert!(route_of(ask(&fac, a, b, 0)).is_some());
        begin_frame(&fac);
        assert!(route_of(ask(&fac, a, b, 0)).is_some());
        assert_eq!(stats().searches, 1, "the same door state reuses the cached route");
        assert!(fac.toggle_door(door, &[]).is_some());
        begin_frame(&fac);
        assert!(route_of(ask(&fac, a, b, 0)).is_some());
        assert_eq!(stats().searches, 2, "a door that opened plans the route again");
    }

    #[test]
    fn a_goal_field_older_than_the_ttl_counts_as_absent() {
        let ttl = PathingRules::get().cache_ttl_frames;
        let key = field_key(1, 0, Cell::new(3, 3), &[], None);
        let mut s = State { frame: 5, ..State::default() };
        s.insert_field(key.clone(), vec![0.0]);
        assert!(s.has_field(&key) && s.field(&key).is_some());
        s.frame = 5 + ttl;
        assert!(s.has_field(&key), "still fresh at the TTL");
        s.frame = 5 + ttl + 1;
        assert!(!s.has_field(&key) && s.field(&key).is_none(), "expired");
    }

    #[test]
    fn a_resident_floor_is_answered_when_the_active_floor_spent_the_budget() {
        reset_thread();
        set_searches_per_frame(Some(1));
        let active = facility();
        let resident = Facility::generate("SCHED-RES", &crate::world::facility::Options::default(), &Catalog::load());
        let reserve = PathingRules::get().resident_searches_per_frame;
        assert!(reserve >= 1, "the data keeps a reserve for resident floors");
        let ps = pairs(&resident, 6, reserve + 1);
        assert_eq!(ps.len(), reserve + 1);
        let (a, b) = pairs(&active, 6, 1)[0];
        begin_frame(&active);
        assert!(route_of(ask(&active, a, b, 0)).is_some(), "the active floor takes the whole budget");
        let answered = ps.iter().filter(|(x, y)| route_of(ask(&resident, *x, *y, 0)).is_some()).count();
        assert_eq!(answered, reserve, "the resident floor gets exactly its reserve");
    }
}
