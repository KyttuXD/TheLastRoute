//! Hunter vent routing: the duct graph between a floor's grates, the route a chaser takes through it, and the crawl
//! movement for a body in the ducts (Godot vent_nav.gd, vent_crawl.gd; the hatch and thump events of hunter_brain.gd
//! `_vent_step`). Verrine's duct patrol (gimmicks/verrine.rs) uses the same graph and crawl.
//!
//! The Rust floor has no duct geometry of its own, only the grates (`fix.vents`), so the ducts are runs between grates:
//! the ring (grate i to i+1) plus any extra links in data/hunters/vents.json, each an L along x then y. Every number is
//! data (`rules()`); the code is behaviour only. Nothing here touches a Brain: the brain hook feeds `Trip::step` the
//! hunter's position and applies what it returns.

use std::collections::VecDeque;
use std::sync::OnceLock;

use serde_json::Value;

use crate::content::embedded;
use crate::world::Cell;

/// The data file's path under the embedded data root (content/mod.rs skips it as a hunter definition).
pub const VENTS_PATH: &str = "hunters/vents.json";

#[derive(Clone, Debug, PartialEq)]
pub struct VerrineRules {
    pub initial_scale: f32,
    pub per_second: f32,
    pub max_scale: f32,
    pub lurk_radius: f32,
    pub return_seconds: f32,
    pub patrol_reach: f32,
    pub exit_step: f32,
    pub exit_radius: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub difficulties: Vec<String>,
    pub shorter_ratio: f32,
    pub grate_search: f32,
    pub entries_tried: usize,
    pub enter_seconds: f32,
    pub exit_seconds: f32,
    pub crawl_speed: f32,
    pub climb_reach: f32,
    pub ring: bool,
    pub links: Vec<(usize, usize)>,
    pub thump_every: f32,
    pub thump_loudness: f32,
    pub verrine: VerrineRules,
}

fn num(v: &Value, key: &str, positive: bool) -> Result<f32, String> {
    v[key]
        .as_f64()
        .filter(|x| x.is_finite() && if positive { *x > 0.0 } else { *x >= 0.0 })
        .map(|x| x as f32)
        .ok_or_else(|| format!("vents.json: `{key}` must be a {} number", if positive { "positive" } else { "non-negative" }))
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("vents.json: {e}"))?;
        if v["schema"].as_str() != Some("lr.vents/1") {
            return Err("vents.json: schema must be lr.vents/1".into());
        }
        let (r, d, t, w) = (&v["route"], &v["ducts"], &v["thump"], &v["verrine"]);
        let difficulties = r["difficulties"]
            .as_array()
            .ok_or("vents.json: route.difficulties must be a list")?
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
        let shorter_ratio = num(r, "shorter_ratio", true)?;
        if shorter_ratio > 1.0 {
            return Err("vents.json: shorter_ratio must be at most 1".into());
        }
        let mut links = vec![];
        for l in d["links"].as_array().ok_or("vents.json: ducts.links must be a list")? {
            let pair = l.as_array().filter(|a| a.len() == 2).ok_or("vents.json: a link is [grate, grate]")?;
            let idx = |i: usize| pair[i].as_u64().map(|x| x as usize).ok_or("vents.json: a link holds grate indexes");
            links.push((idx(0)?, idx(1)?));
        }
        Ok(Rules {
            difficulties,
            shorter_ratio,
            grate_search: num(r, "grate_search", true)?,
            entries_tried: (num(r, "entries_tried", true)? as usize).max(1),
            enter_seconds: num(r, "enter_seconds", true)?,
            exit_seconds: num(r, "exit_seconds", true)?,
            crawl_speed: num(r, "crawl_speed", true)?,
            climb_reach: num(r, "climb_reach", true)?,
            ring: d["ring"].as_bool().ok_or("vents.json: ducts.ring must be true or false")?,
            links,
            thump_every: num(t, "every", true)?,
            thump_loudness: num(t, "loudness", true)?,
            verrine: VerrineRules {
                initial_scale: num(w, "initial_scale", false)?,
                per_second: num(w, "per_second", false)?,
                max_scale: num(w, "max_scale", true)?,
                lurk_radius: num(w, "lurk_radius", true)?,
                return_seconds: num(w, "return_seconds", true)?,
                patrol_reach: num(w, "patrol_reach", true)?,
                exit_step: num(w, "exit_step", false)?,
                exit_radius: num(w, "exit_radius", true)?,
            },
        })
    }
}

/// The embedded rules (`data/hunters/vents.json`).
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let (_, text) = embedded::data_under(VENTS_PATH).next().expect("data/hunters/vents.json is embedded");
        Rules::parse(text).expect("data/hunters/vents.json parses")
    })
}

/// VentNav.uses_vents for the chasers: only on a listed difficulty, and never Verrine (the ducts are her whole hunt, so
/// her gimmick moves her; she is not a chaser that takes a shortcut).
pub fn uses_vents(r: &Rules, hunter_id: &str, difficulty_id: &str) -> bool {
    hunter_id != "verrine" && r.difficulties.iter().any(|d| d == difficulty_id)
}

pub fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// One duct run between two grates, listed from `a` to `b`.
#[derive(Clone, Debug, PartialEq)]
pub struct Duct {
    pub a: usize,
    pub b: usize,
    pub points: Vec<(f32, f32)>,
    pub length: f32,
}

impl Duct {
    fn between(a: usize, b: usize, pa: (f32, f32), pb: (f32, f32)) -> Duct {
        let mut points = vec![pa];
        if (pa.0 - pb.0).abs() > 1e-4 && (pa.1 - pb.1).abs() > 1e-4 {
            points.push((pb.0, pa.1));
        }
        points.push(pb);
        let length = points.windows(2).map(|w| dist(w[0], w[1])).sum();
        Duct { a, b, points, length }
    }
}

/// A point on a crawl path; `node` is the grate index when the point is a grate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    pub pos: (f32, f32),
    pub node: Option<usize>,
}

/// The vent trip a chaser takes: the grate it climbs in at, the one it climbs out at and the grates between.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub entry: usize,
    pub exit: usize,
    pub nodes: Vec<usize>,
    /// Walk to the entry, climbs, crawl and walk on from the exit, in cells.
    pub length: f32,
}

/// The duct network of one floor: grates and the runs between them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VentGraph {
    pub grates: Vec<(f32, f32)>,
    pub ducts: Vec<Duct>,
}

impl VentGraph {
    pub fn build(grates: &[(f32, f32)], r: &Rules) -> VentGraph {
        let n = grates.len();
        let mut pairs: Vec<(usize, usize)> = vec![];
        if r.ring {
            for i in 0..n {
                let j = (i + 1) % n;
                if i != j {
                    pairs.push((i.min(j), i.max(j)));
                }
            }
        }
        for &(a, b) in &r.links {
            if a < n && b < n && a != b {
                pairs.push((a.min(b), a.max(b)));
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        let ducts = pairs.into_iter().map(|(a, b)| Duct::between(a, b, grates[a], grates[b])).collect();
        VentGraph { grates: grates.to_vec(), ducts }
    }

    pub fn from_cells(cells: &[Cell], r: &Rules) -> VentGraph {
        let grates: Vec<(f32, f32)> = cells.iter().map(|c| c.center()).collect();
        VentGraph::build(&grates, r)
    }

    /// The grates within `radius` of `p`, nearest first.
    pub fn grates_around(&self, p: (f32, f32), radius: f32) -> Vec<usize> {
        let mut found: Vec<(f32, usize)> =
            self.grates.iter().enumerate().map(|(i, g)| (dist(p, *g), i)).filter(|(d, _)| *d <= radius).collect();
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.into_iter().map(|(_, i)| i).collect()
    }

    /// The grate nearest `p` and how far it is.
    pub fn nearest(&self, p: (f32, f32)) -> Option<(usize, f32)> {
        self.grates
            .iter()
            .enumerate()
            .map(|(i, g)| (i, dist(p, *g)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    pub fn duct_between(&self, a: usize, b: usize) -> Option<&Duct> {
        self.ducts.iter().find(|d| (d.a == a && d.b == b) || (d.a == b && d.b == a))
    }

    /// The shortest crawl from grate `from` to grate `to`: its length and the grates passed (both ends included).
    pub fn shortest(&self, from: usize, to: usize) -> Option<(f32, Vec<usize>)> {
        let n = self.grates.len();
        if from >= n || to >= n {
            return None;
        }
        let mut best = vec![f32::INFINITY; n];
        let mut prev: Vec<Option<usize>> = vec![None; n];
        let mut done = vec![false; n];
        best[from] = 0.0;
        for _ in 0..n {
            let Some(u) = (0..n).filter(|i| !done[*i] && best[*i].is_finite()).min_by(|a, b| best[*a].total_cmp(&best[*b])) else {
                break;
            };
            done[u] = true;
            if u == to {
                break;
            }
            for d in self.ducts.iter().filter(|d| d.a == u || d.b == u) {
                let v = if d.a == u { d.b } else { d.a };
                if best[u] + d.length < best[v] {
                    best[v] = best[u] + d.length;
                    prev[v] = Some(u);
                }
            }
        }
        if !best[to].is_finite() {
            return None;
        }
        let mut nodes = vec![to];
        while let Some(p) = prev[*nodes.last().unwrap()] {
            nodes.push(p);
        }
        nodes.reverse();
        Some((best[to], nodes))
    }

    /// The crawl path through `nodes`: the first grate, then every duct point to the next grate, and so on. Only the
    /// grates carry a node.
    pub fn waypoints(&self, nodes: &[usize]) -> Vec<Waypoint> {
        let mut out = vec![];
        let Some(&first) = nodes.first() else { return out };
        out.push(Waypoint { pos: self.grates[first], node: Some(first) });
        for w in nodes.windows(2) {
            let Some(d) = self.duct_between(w[0], w[1]) else { continue };
            let pts: Vec<(f32, f32)> = if d.a == w[0] { d.points.clone() } else { d.points.iter().rev().copied().collect() };
            let last = pts.len() - 1;
            for (k, p) in pts.into_iter().enumerate().skip(1) {
                out.push(Waypoint { pos: p, node: (k == last).then_some(w[1]) });
            }
        }
        out
    }

    /// VentNav.route: the vent trip from `from` to `to`, or None when the floor is the better way. `floor_len` is the
    /// floor route's length in cells (infinity when there is none). The ducts are taken only when the whole trip is at
    /// most `shorter_ratio` of it.
    pub fn route(&self, r: &Rules, from: (f32, f32), to: (f32, f32), floor_len: f32, hunter_id: &str, difficulty_id: &str) -> Option<Route> {
        if !uses_vents(r, hunter_id, difficulty_id) {
            return None;
        }
        let entries = self.grates_around(from, r.grate_search);
        let exits = self.grates_around(to, r.grate_search);
        let climbs = (r.enter_seconds + r.exit_seconds) * r.crawl_speed;
        let mut best: Option<Route> = None;
        for &en in entries.iter().take(r.entries_tried) {
            for &ex in exits.iter().take(r.entries_tried) {
                if en == ex {
                    continue;
                }
                let Some((len, nodes)) = self.shortest(en, ex) else { continue };
                let total = dist(from, self.grates[en]) + len + climbs + dist(self.grates[ex], to);
                if best.as_ref().map_or(true, |b| total < b.length) {
                    best = Some(Route { entry: en, exit: ex, nodes, length: total });
                }
            }
        }
        let best = best?;
        (best.length <= floor_len * r.shorter_ratio).then_some(best)
    }
}

/// A graph kept for one floor's grate list; rebuilt when the list changes.
#[derive(Clone, Debug, Default)]
pub struct Ducts {
    pub graph: VentGraph,
    key: Vec<(i32, i32)>,
}

impl Ducts {
    /// Rebuilds for `cells` when they differ from the last call. Returns whether it rebuilt.
    pub fn sync(&mut self, cells: &[Cell]) -> bool {
        let key: Vec<(i32, i32)> = cells.iter().map(|c| (c.x, c.y)).collect();
        if key == self.key && !self.graph.grates.is_empty() {
            return false;
        }
        self.key = key;
        self.graph = VentGraph::from_cells(cells, rules());
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Out,
    ClimbIn,
    Duct,
    ClimbOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hatch {
    Enter,
    Exit,
}

/// What one crawl step did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    pub moved: f32,
    /// Sheet-metal thumps this step (one per `thump_every` cells crawled): a noise event each.
    pub thumps: u32,
    /// The hatch opened this step (climbing in or out began).
    pub hatch: Option<Hatch>,
    /// The climb in finished: now crawling.
    pub entered: bool,
    /// The climb out finished: back on the floor.
    pub landed: bool,
    /// The last grate the crawl passed.
    pub reached: Option<usize>,
}

/// A body moving through the ducts: climb in over `enter_seconds`, crawl the path at `crawl_speed`, climb out over
/// `exit_seconds`. Position changes continuously; nothing jumps from one grate to another.
#[derive(Clone, Debug, Default)]
pub struct Crawl {
    pub phase: Phase,
    pub pos: (f32, f32),
    /// The grate it last climbed in at or crawled to.
    pub grate: Option<usize>,
    from: (f32, f32),
    to: (f32, f32),
    t: f32,
    dur: f32,
    path: VecDeque<Waypoint>,
    since_thump: f32,
    hatch: Option<Hatch>,
}

impl Crawl {
    pub fn inside(&self) -> bool {
        self.phase != Phase::Out
    }

    pub fn path_empty(&self) -> bool {
        self.path.is_empty()
    }

    /// 0 standing on the floor, 1 in the duct (for the renderer's climb).
    pub fn height(&self) -> f32 {
        match self.phase {
            Phase::Out => 0.0,
            Phase::ClimbIn => self.t,
            Phase::Duct => 1.0,
            Phase::ClimbOut => 1.0 - self.t,
        }
    }

    /// Already in a duct at a grate (a hunter that lives there).
    pub fn place_in_duct(&mut self, grate: usize, pos: (f32, f32)) {
        self.phase = Phase::Duct;
        self.pos = pos;
        self.grate = Some(grate);
        self.path.clear();
    }

    /// Start climbing in at grate `grate` (at `grate_pos`) from `from`, then crawl `path`. False when `from` is out of
    /// reach of the grate or already inside.
    pub fn enter(&mut self, r: &Rules, grate: usize, grate_pos: (f32, f32), from: (f32, f32), path: Vec<Waypoint>) -> bool {
        if self.phase != Phase::Out || dist(from, grate_pos) > r.climb_reach {
            return false;
        }
        self.phase = Phase::ClimbIn;
        self.pos = from;
        self.from = from;
        self.to = grate_pos;
        self.t = 0.0;
        self.dur = r.enter_seconds.max(1e-3);
        self.grate = Some(grate);
        self.path = path.into();
        self.hatch = Some(Hatch::Enter);
        true
    }

    /// Start climbing out onto `onto` (a clear floor point beside the grate). Needs the path finished.
    pub fn exit_at(&mut self, r: &Rules, onto: (f32, f32)) -> bool {
        if self.phase != Phase::Duct || !self.path.is_empty() || dist(onto, self.pos) > r.climb_reach {
            return false;
        }
        self.phase = Phase::ClimbOut;
        self.from = self.pos;
        self.to = onto;
        self.t = 0.0;
        self.dur = r.exit_seconds.max(1e-3);
        self.hatch = Some(Hatch::Exit);
        true
    }

    /// Crawl `path` next (in a duct, nothing left to crawl).
    pub fn set_path(&mut self, path: Vec<Waypoint>) {
        self.path = path.into();
    }

    /// One frame. `scale` multiplies the crawl speed.
    pub fn step(&mut self, r: &Rules, dt: f32, scale: f32) -> Step {
        let mut s = Step { hatch: self.hatch.take(), ..Step::default() };
        match self.phase {
            Phase::Out => {}
            Phase::ClimbIn | Phase::ClimbOut => {
                self.t = (self.t + dt / self.dur).min(1.0);
                self.pos = (self.from.0 + (self.to.0 - self.from.0) * self.t, self.from.1 + (self.to.1 - self.from.1) * self.t);
                if self.t >= 1.0 {
                    if self.phase == Phase::ClimbIn {
                        self.phase = Phase::Duct;
                        s.entered = true;
                    } else {
                        self.phase = Phase::Out;
                        s.landed = true;
                    }
                }
            }
            Phase::Duct => {
                let mut budget = (r.crawl_speed * scale.max(0.0) * dt).max(0.0);
                while let Some(&next) = self.path.front() {
                    let d = dist(self.pos, next.pos);
                    if d <= budget {
                        self.pos = next.pos;
                        budget -= d;
                        s.moved += d;
                        if next.node.is_some() {
                            self.grate = next.node;
                            s.reached = next.node;
                        }
                        self.path.pop_front();
                    } else {
                        let k = budget / d;
                        self.pos = (self.pos.0 + (next.pos.0 - self.pos.0) * k, self.pos.1 + (next.pos.1 - self.pos.1) * k);
                        s.moved += budget;
                        break;
                    }
                }
                self.since_thump += s.moved;
                while self.since_thump >= r.thump_every {
                    self.since_thump -= r.thump_every;
                    s.thumps += 1;
                }
            }
        }
        s
    }
}

/// What a chaser's vent step asks of the brain this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TripStep {
    /// The entry grate to walk to instead of the ordinary goal (None while crawling or when no trip is planned).
    pub walk_to: Option<(f32, f32)>,
    pub hatch: Option<Hatch>,
    pub thumps: u32,
    /// The climb out landed: the trip is over, the brain walks on from `Trip::crawl.pos`.
    pub done: bool,
}

/// A chaser's vent shortcut: plan it, walk to the entry grate, climb in, crawl, climb out.
#[derive(Clone, Debug, Default)]
pub struct Trip {
    pub ducts: Ducts,
    pub crawl: Crawl,
    route: Option<Route>,
}

impl Trip {
    /// True while the hunter's body is in the ducts (the brain must not walk it).
    pub fn inside(&self) -> bool {
        self.crawl.inside()
    }

    pub fn planned(&self) -> bool {
        self.route.is_some()
    }

    /// Plans a trip from `pos` to `goal` when the ducts beat the floor route (`floor_len`, infinity when none). A trip
    /// already planned or under way is kept. Returns whether a trip is planned.
    pub fn plan(&mut self, cells: &[Cell], pos: (f32, f32), goal: (f32, f32), floor_len: f32, hunter_id: &str, difficulty_id: &str) -> bool {
        if self.route.is_some() || self.crawl.inside() {
            return self.route.is_some();
        }
        self.ducts.sync(cells);
        self.route = self.ducts.graph.route(rules(), pos, goal, floor_len, hunter_id, difficulty_id);
        self.route.is_some()
    }

    /// Drops a planned trip that has not started (the chase ended). A hunter already in the ducts finishes it.
    pub fn cancel(&mut self) {
        if !self.crawl.inside() {
            self.route = None;
        }
    }

    /// One frame. `pos` is the hunter's floor position (read while it is outside), `goal` the point it is heading for and
    /// `floor_near(grate, goal)` a clear floor point beside the exit grate.
    pub fn step(&mut self, dt: f32, pos: (f32, f32), goal: (f32, f32), floor_near: &dyn Fn((f32, f32), (f32, f32)) -> (f32, f32)) -> TripStep {
        let r = rules();
        let mut out = TripStep::default();
        let Some(route) = self.route.clone() else { return out };
        let g = &self.ducts.graph;
        if self.crawl.phase == Phase::Out {
            self.crawl.pos = pos;
            let entry = g.grates[route.entry];
            if dist(pos, entry) <= r.climb_reach * 0.5 {
                let path: Vec<Waypoint> = g.waypoints(&route.nodes).into_iter().skip(1).collect();
                self.crawl.enter(r, route.entry, entry, pos, path);
            } else {
                out.walk_to = Some(entry);
                return out;
            }
        }
        if self.crawl.phase == Phase::Duct && self.crawl.path_empty() {
            let exit = g.grates[route.exit];
            let onto = floor_near(exit, goal);
            self.crawl.exit_at(r, onto);
        }
        let s = self.crawl.step(r, dt, 1.0);
        out.hatch = s.hatch;
        out.thumps = s.thumps;
        if s.landed {
            self.route = None;
            out.done = true;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring() -> VentGraph {
        // four grates far apart: 0 and 2 are opposite corners
        VentGraph::build(&[(2.5, 2.5), (40.5, 2.5), (40.5, 40.5), (2.5, 40.5)], rules())
    }

    #[test]
    fn the_vent_graph_links_the_grates_in_a_ring() {
        let g = ring();
        assert_eq!(g.ducts.len(), 4);
        for i in 0..4 {
            assert!(g.duct_between(i, (i + 1) % 4).is_some(), "grate {i} joins the next");
        }
        assert!(g.duct_between(0, 2).is_none(), "opposite grates are not joined directly");
        let (len, nodes) = g.shortest(0, 2).expect("every grate reaches every other");
        assert_eq!(nodes.len(), 3, "two ducts across");
        assert!(len > 70.0 && len < 90.0);
        let wp = g.waypoints(&nodes);
        assert_eq!(wp.first().unwrap().node, Some(0));
        assert_eq!(wp.last().unwrap().node, Some(2));
        assert_eq!(wp.last().unwrap().pos, g.grates[2]);
        assert_eq!(wp.iter().filter(|w| w.node.is_some()).count(), 3);
    }

    #[test]
    fn a_duct_run_bends_once_when_the_grates_are_not_aligned() {
        let g = VentGraph::build(&[(2.5, 2.5), (10.5, 8.5), (2.5, 20.5)], rules());
        assert_eq!(g.duct_between(0, 1).unwrap().points.len(), 3);
        assert_eq!(g.duct_between(0, 2).unwrap().points.len(), 2);
        assert!((g.duct_between(0, 1).unwrap().length - 14.0).abs() < 1e-3, "an L: 8 along x and 6 along y");
    }

    #[test]
    fn the_rules_parse_and_a_bad_schema_is_refused() {
        assert!(rules().shorter_ratio > 0.0 && rules().shorter_ratio <= 1.0);
        assert!(Rules::parse("{\"schema\":\"nope\"}").is_err());
    }

    #[test]
    fn a_chaser_takes_the_vent_only_when_it_is_shorter_than_the_floor_by_the_margin() {
        let r = rules();
        let g = ring();
        let (from, to) = ((3.5, 3.5), (39.5, 39.5));
        let vent = g.route(r, from, to, f32::INFINITY, "kathy", "hard").expect("no floor route at all: take the ducts");
        // the floor way is far longer: the ducts win
        assert!(g.route(r, from, to, vent.length / r.shorter_ratio * 1.5, "kathy", "hard").is_some());
        assert!(g.route(r, from, to, vent.length / r.shorter_ratio * 1.01, "kathy", "nightmare").is_some());
        // the floor way is about as long as the vent trip: walk
        assert!(g.route(r, from, to, vent.length, "kathy", "hard").is_none());
        assert!(g.route(r, from, to, vent.length / r.shorter_ratio * 0.9, "kathy", "hard").is_none());
        // too far from any grate: no entry
        assert!(g.route(r, (20.5, 20.5), to, f32::INFINITY, "kathy", "hard").is_none());
    }

    #[test]
    fn only_the_listed_difficulties_and_never_verrine_take_the_vent() {
        let r = rules();
        let g = ring();
        let (from, to) = ((3.5, 3.5), (39.5, 39.5));
        assert!(g.route(r, from, to, f32::INFINITY, "kathy", "standard").is_none());
        assert!(g.route(r, from, to, f32::INFINITY, "kathy", "relaxed").is_none());
        assert!(g.route(r, from, to, f32::INFINITY, "verrine", "nightmare").is_none());
        assert!(g.route(r, from, to, f32::INFINITY, "kathy", "nightmare").is_some());
    }

    #[test]
    fn a_crawl_climbs_in_crawls_the_ducts_thumps_and_climbs_out() {
        let r = rules();
        let g = ring();
        let route = g.route(r, (3.5, 3.5), (39.5, 39.5), f32::INFINITY, "kathy", "hard").unwrap();
        let mut c = Crawl::default();
        let path: Vec<Waypoint> = g.waypoints(&route.nodes).into_iter().skip(1).collect();
        assert!(!c.enter(r, route.entry, (30.5, 30.5), (3.5, 3.5), vec![]), "out of reach of that grate");
        assert!(c.enter(r, route.entry, g.grates[route.entry], (3.5, 3.5), path));
        let s = c.step(r, 0.01, 1.0);
        assert_eq!(s.hatch, Some(Hatch::Enter), "the hatch opens as the climb starts");
        assert_eq!(c.phase, Phase::ClimbIn);
        let mut t = 0.0;
        let mut entered = false;
        while !entered && t < 10.0 {
            entered = c.step(r, 0.05, 1.0).entered;
            t += 0.05;
        }
        assert!(entered && (t - r.enter_seconds).abs() < 0.2, "the climb takes the data time");
        assert_eq!(c.phase, Phase::Duct);
        let mut thumps = 0;
        let mut moved = 0.0;
        for _ in 0..4000 {
            if c.path_empty() {
                break;
            }
            let s = c.step(r, 0.05, 1.0);
            thumps += s.thumps;
            moved += s.moved;
        }
        assert!(c.path_empty());
        assert!(thumps > 10 && thumps as f32 <= moved / r.thump_every + 1.0, "a thump every so many cells: {thumps}");
        assert!(dist(c.pos, g.grates[route.exit]) < 1e-3);
        assert!(!c.exit_at(r, (c.pos.0 + 10.0, c.pos.1)), "not past the climb reach");
        assert!(c.exit_at(r, (c.pos.0 - 1.0, c.pos.1)));
        let mut landed = false;
        for _ in 0..100 {
            if c.step(r, 0.05, 1.0).landed {
                landed = true;
                break;
            }
        }
        assert!(landed && c.phase == Phase::Out);
    }

    #[test]
    fn a_trip_walks_to_the_entry_goes_through_and_ends_on_the_floor() {
        let cells = [Cell::new(2, 2), Cell::new(40, 2), Cell::new(40, 40), Cell::new(2, 40)];
        let mut trip = Trip::default();
        let (mut pos, goal) = ((6.5, 3.5), (39.5, 38.5));
        assert!(!trip.plan(&cells, pos, goal, 10.0, "kathy", "hard"), "the floor way is short: no trip");
        assert!(trip.plan(&cells, pos, goal, f32::INFINITY, "kathy", "hard"));
        let near = |g: (f32, f32), _goal: (f32, f32)| (g.0, g.1 - 1.0);
        // far from the entry grate: it walks there
        let s = trip.step(0.05, pos, goal, &near);
        assert_eq!(s.walk_to, Some((2.5, 2.5)));
        assert!(!trip.inside());
        pos = (2.5, 2.9);
        let mut hatches = vec![];
        let mut thumps = 0;
        let mut done = false;
        for _ in 0..6000 {
            let s = trip.step(0.05, pos, goal, &near);
            hatches.extend(s.hatch);
            thumps += s.thumps;
            if s.done {
                done = true;
                break;
            }
        }
        assert!(done, "the trip ends");
        assert_eq!(hatches, vec![Hatch::Enter, Hatch::Exit]);
        assert!(thumps > 0);
        assert!(!trip.inside() && !trip.planned());
        assert!(dist(trip.crawl.pos, (40.5, 39.5)) < 1e-3, "it lands beside the exit grate");
    }
}
