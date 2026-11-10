//! Pathfinding for the hunters: a waypoint graph searched with Dijkstra, and the Walker that follows a route at a
//! walking pace. Ported from night_watch_paths.gd (graph part) and night_watch_walker.gd. Pure logic.
#![allow(dead_code)]

use super::data::move_window;
use macroquad::prelude::{vec3, Vec3};
use std::collections::{HashMap, VecDeque};

/// One step of a route: where to go, whether it is a jump (the walker steps across it), and the waypoint key.
#[derive(Clone, Debug)]
pub struct Leg {
    pub p: Vec3,
    pub jump: bool,
    pub key: String,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    nodes: Vec<(String, Vec3)>,
    /// per node: (neighbour index, weight, is_jump)
    adj: Vec<Vec<(usize, f32, bool)>>,
    index: HashMap<String, usize>,
}

impl Graph {
    pub fn new() -> Graph {
        Graph::default()
    }

    /// Adds a waypoint; adding a key twice moves the first one.
    pub fn add_node(&mut self, key: &str, p: Vec3) {
        if let Some(&i) = self.index.get(key) {
            self.nodes[i].1 = p;
            return;
        }
        self.index.insert(key.to_string(), self.nodes.len());
        self.nodes.push((key.to_string(), p));
        self.adj.push(Vec::new());
    }

    fn connect(&mut self, a: &str, b: &str, jump: bool) {
        let (ia, ib) = match (self.index.get(a), self.index.get(b)) {
            (Some(&x), Some(&y)) => (x, y),
            _ => return,
        };
        if ia == ib {
            return;
        }
        let w = self.nodes[ia].1.distance(self.nodes[ib].1);
        if !self.adj[ia].iter().any(|e| e.0 == ib && e.2 == jump) {
            self.adj[ia].push((ib, w, jump));
        }
        if !self.adj[ib].iter().any(|e| e.0 == ia && e.2 == jump) {
            self.adj[ib].push((ia, w, jump));
        }
    }

    /// Undirected link, weight = distance.
    pub fn link(&mut self, a: &str, b: &str) {
        self.connect(a, b, false);
    }

    /// Undirected jump link: the leg into the far node is marked jump when traversed.
    pub fn link_jump(&mut self, a: &str, b: &str) {
        self.connect(a, b, true);
    }

    pub fn pos(&self, key: &str) -> Option<Vec3> {
        self.index.get(key).map(|&i| self.nodes[i].1)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The key of the waypoint closest to p.
    pub fn nearest(&self, p: Vec3) -> Option<String> {
        self.nodes
            .iter()
            .min_by(|a, b| a.1.distance_squared(p).partial_cmp(&b.1.distance_squared(p)).unwrap_or(std::cmp::Ordering::Equal))
            .map(|n| n.0.clone())
    }

    /// Shortest route (Dijkstra) from one waypoint to another; the first leg is the start itself and never a jump.
    /// Empty if either key is unknown or there is no path.
    pub fn route(&self, a: &str, b: &str) -> Vec<Leg> {
        let (start, goal) = match (self.index.get(a), self.index.get(b)) {
            (Some(&x), Some(&y)) => (x, y),
            _ => return Vec::new(),
        };
        let n = self.nodes.len();
        let mut dist = vec![f32::INFINITY; n];
        let mut prev: Vec<Option<(usize, bool)>> = vec![None; n];
        let mut done = vec![false; n];
        dist[start] = 0.0;
        loop {
            let mut best: Option<usize> = None;
            for i in 0..n {
                if !done[i] && dist[i].is_finite() && best.map_or(true, |b| dist[i] < dist[b]) {
                    best = Some(i);
                }
            }
            let u = match best {
                Some(u) => u,
                None => break,
            };
            if u == goal {
                break;
            }
            done[u] = true;
            for &(v, w, jump) in &self.adj[u] {
                if !done[v] && dist[u] + w < dist[v] {
                    dist[v] = dist[u] + w;
                    prev[v] = Some((u, jump));
                }
            }
        }
        if !dist[goal].is_finite() {
            return Vec::new();
        }
        let mut legs = Vec::new();
        let mut at = goal;
        loop {
            let (jump, back) = match prev[at] {
                Some((u, j)) => (j, Some(u)),
                None => (false, None),
            };
            legs.push(Leg { p: self.nodes[at].1, jump, key: self.nodes[at].0.clone() });
            match back {
                Some(u) => at = u,
                None => break,
            }
        }
        legs.reverse();
        legs
    }

    /// The route from wherever a hunter stands (from the nearest waypoint) to a waypoint.
    pub fn legs_from(&self, pos: Vec3, to_key: &str) -> Vec<Leg> {
        match self.nearest(pos) {
            Some(start) => self.route(&start, to_key),
            None => Vec::new(),
        }
    }
}

/// Length of a route's walked legs (jumps do not count; the first leg is the start).
pub fn walked_length(legs: &[Leg]) -> f32 {
    let mut total = 0.0;
    for i in 1..legs.len() {
        if !legs[i].jump {
            total += legs[i].p.distance(legs[i - 1].p);
        }
    }
    total
}

/// One hunter's visible movement: follows a route at a walking pace, faces where she goes, and when she has arrived
/// shuffles to `final_pos` and turns to face `look`. Nothing here touches the sim.
#[derive(Clone, Debug)]
pub struct Walker {
    pub pos: Vec3,
    pub yaw: f32,
    pub room: String,
    pub moving: bool,
    pub running: bool,
    pub final_pos: Vec3,
    pub look: Vec3,
    queue: VecDeque<Leg>,
    speed: f32,
}

impl Default for Walker {
    fn default() -> Self {
        Walker::new()
    }
}

impl Walker {
    pub const WALK_SPEED: f32 = 2.3;
    /// A step never covers more than this in the validator's bound.
    pub const RUN_SPEED: f32 = 5.5;
    /// The slowest pace a route is walked at (a short hop is not crawled).
    pub const MIN_SPEED: f32 = 1.2;
    /// The fastest (a long route on a short window is run, never flown).
    pub const MAX_SPEED: f32 = 6.0;

    pub fn new() -> Walker {
        Walker {
            pos: Vec3::ZERO,
            yaw: 0.0,
            room: String::new(),
            moving: false,
            running: false,
            final_pos: Vec3::ZERO,
            look: Vec3::ZERO,
            queue: VecDeque::new(),
            speed: Walker::WALK_SPEED,
        }
    }

    pub fn place(&mut self, p: Vec3, f: Vec3, look: Vec3, room: &str) {
        self.pos = p;
        self.final_pos = f;
        self.look = look;
        self.room = room.to_string();
        self.queue.clear();
        self.moving = false;
        self.face(look - p);
    }

    /// The pace for a route of `length` metres on a time `budget` (seconds).
    pub fn speed_for(length: f32, budget: f32) -> f32 {
        (length / budget.max(0.1)).clamp(Walker::MIN_SPEED, Walker::MAX_SPEED)
    }

    /// The sim moved her to `to_room`: take `legs` to it, arriving about `budget_s` seconds from now.
    pub fn go(&mut self, legs: Vec<Leg>, to_room: &str, f: Vec3, budget_s: f32) {
        self.room = to_room.to_string();
        self.final_pos = f;
        self.queue.clear();
        if legs.is_empty() {
            self.pos = f;
            return;
        }
        let first = if legs[0].jump { self.pos } else { legs[0].p };
        let length = walked_length(&legs) + self.pos.distance(first);
        self.queue.extend(legs);
        self.speed = Walker::speed_for(length, budget_s);
        self.running = self.speed > Walker::WALK_SPEED * 1.6;
    }

    pub fn is_walking(&self) -> bool {
        !self.queue.is_empty()
    }

    /// Advance by `dt_s` seconds; true while she is on the move.
    pub fn step(&mut self, dt_s: f32) -> bool {
        let mut budget = dt_s;
        self.moving = false;
        while budget > 0.0 {
            let (target, jump) = match self.queue.front() {
                Some(l) => (l.p, l.jump),
                None => break,
            };
            if jump {
                self.pos = target;
                self.queue.pop_front();
                continue;
            }
            let mut d = target - self.pos;
            d.y = 0.0;
            let dist = d.length();
            let mv = self.speed * budget;
            self.moving = true;
            self.face(d);
            if mv >= dist {
                self.pos = vec3(target.x, self.pos.y, target.z);
                budget -= dist / self.speed;
                self.queue.pop_front();
            } else {
                self.pos += d / dist * mv;
                budget = 0.0;
            }
        }
        if self.queue.is_empty() {
            // arrived: shuffle to the stand spot (it moves at a door) and face what she looks at
            let mut to_final = self.final_pos - self.pos;
            to_final.y = 0.0;
            if to_final.length() > 0.02 {
                let move2 = to_final.length().min(Walker::WALK_SPEED * 0.5 * dt_s);
                self.pos += to_final.normalize() * move2;
                self.moving = move2 > 0.001;
            } else {
                self.pos = self.final_pos;
            }
            self.pos.y = self.final_pos.y;
            if !self.moving {
                let d = self.look - self.pos;
                self.face(d);
            }
        }
        self.moving
    }

    /// Face along d (the blocky models' front is -Z): yaw = atan2(d.x, d.z) + PI when |d.xz| > 0.01.
    pub fn face(&mut self, d: Vec3) {
        let flat = vec3(d.x, 0.0, d.z);
        if flat.length() > 0.01 {
            self.yaw = flat.x.atan2(flat.z) + std::f32::consts::PI;
        }
    }
}

/// The hop's time budget: a quarter of the move window's lower bound, between 1.5 and 4 seconds.
pub fn budget_for(level: i32) -> f32 {
    ((move_window(level).0 / 1000.0 * 0.25) as f32).clamp(1.5, 4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line() -> Graph {
        let mut g = Graph::new();
        g.add_node("A", vec3(0.0, 0.0, 0.0));
        g.add_node("B", vec3(5.0, 0.0, 0.0));
        g.add_node("C", vec3(5.0, 0.0, -20.0));
        g.add_node("D", vec3(5.0, 0.0, -22.0));
        g.add_node("E", vec3(0.0, 0.0, -30.0)); // unlinked
        g.link("A", "B");
        g.link_jump("B", "C");
        g.link("C", "D");
        g
    }

    #[test]
    fn dijkstra_finds_the_short_way_and_marks_the_jump() {
        let g = line();
        let r = g.route("A", "D");
        let keys: Vec<&str> = r.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(keys, vec!["A", "B", "C", "D"]);
        assert!(!r[0].jump && !r[1].jump && r[2].jump && !r[3].jump);
        assert!((walked_length(&r) - 7.0).abs() < 1e-4); // 5 + 2, the jump is free
        let back = g.route("D", "A");
        assert_eq!(back.len(), 4);
        assert_eq!(back.iter().filter(|l| l.jump).count(), 1);
        assert!(back[2].jump); // C to B
    }

    #[test]
    fn dijkstra_prefers_the_shorter_of_two_routes() {
        let mut g = Graph::new();
        g.add_node("a", vec3(0.0, 0.0, 0.0));
        g.add_node("b", vec3(10.0, 0.0, 0.0));
        g.add_node("c", vec3(5.0, 0.0, 1.0));
        g.add_node("far", vec3(5.0, 0.0, 50.0));
        g.link("a", "b");
        g.link("a", "far");
        g.link("far", "b");
        g.link("a", "c");
        g.link("c", "b");
        let keys: Vec<String> = g.route("a", "b").iter().map(|l| l.key.clone()).collect();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[test]
    fn no_path_and_unknown_keys_give_nothing() {
        let g = line();
        assert!(g.route("A", "E").is_empty());
        assert!(g.route("A", "nope").is_empty());
        let one = g.route("A", "A");
        assert_eq!(one.len(), 1);
        assert_eq!(g.nearest(vec3(5.2, 0.0, -21.5)).as_deref(), Some("D"));
        assert_eq!(g.pos("B"), Some(vec3(5.0, 0.0, 0.0)));
        let from = g.legs_from(vec3(4.0, 0.0, 0.5), "D");
        assert_eq!(from[0].key, "B");
    }

    #[test]
    fn budget_follows_the_move_window() {
        assert_eq!(budget_for(1), 3.125);
        assert_eq!(budget_for(20), 1.5);
        assert!(budget_for(0) <= 4.0);
    }

    #[test]
    fn walker_arrives_and_never_exceeds_max_speed() {
        let mut g = Graph::new();
        g.add_node("a", vec3(0.0, 0.0, 0.0));
        g.add_node("b", vec3(30.0, 0.0, 0.0));
        g.add_node("c", vec3(30.0, 0.0, 30.0));
        g.link("a", "b");
        g.link("b", "c");
        let legs = g.route("a", "c");
        let mut w = Walker::new();
        w.place(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), "a");
        let fin = vec3(30.0, 0.0, 30.0);
        w.go(legs, "c", fin, 1.5); // 60 m in 1.5 s would be 40 m/s: clamped to MAX_SPEED
        assert!(w.running);
        assert!(w.is_walking());
        let mut last = w.pos;
        let mut steps = 0;
        while w.is_walking() && steps < 2000 {
            w.step(0.1);
            assert!(w.pos.distance(last) <= Walker::MAX_SPEED * 0.1 + 1e-3);
            last = w.pos;
            steps += 1;
        }
        // 60 m at 6 m/s is 10 s
        assert!(steps >= 99 && steps <= 102, "{steps}");
        for _ in 0..50 {
            w.step(0.1);
        }
        assert!(w.pos.distance(fin) < 1e-3);
        assert!(!w.moving);
        assert_eq!(w.room, "c");
    }

    #[test]
    fn short_hops_are_not_crawled() {
        assert_eq!(Walker::speed_for(0.1, 4.0), Walker::MIN_SPEED);
        assert_eq!(Walker::speed_for(1000.0, 1.5), Walker::MAX_SPEED);
        assert_eq!(Walker::speed_for(6.0, 3.0), 2.0);
    }

    #[test]
    fn walker_steps_across_a_jump() {
        let g = line();
        let legs = g.route("A", "D");
        let mut w = Walker::new();
        w.place(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), "A");
        w.go(legs, "D", vec3(5.0, 0.0, -22.0), 3.0);
        let mut max_step = 0.0f32;
        let mut last = w.pos;
        for _ in 0..200 {
            w.step(0.05);
            max_step = max_step.max(w.pos.distance(last));
            last = w.pos;
        }
        assert!(w.pos.distance(vec3(5.0, 0.0, -22.0)) < 1e-3);
        assert!(max_step > 15.0, "the 20 m jump happened in one step");
    }

    #[test]
    fn empty_route_puts_her_at_the_stand_spot() {
        let mut w = Walker::new();
        w.go(Vec::new(), "x", vec3(2.0, 0.0, 3.0), 2.0);
        assert_eq!(w.pos, vec3(2.0, 0.0, 3.0));
        assert!(!w.is_walking());
    }

    #[test]
    fn face_uses_the_minus_z_front() {
        let mut w = Walker::new();
        w.face(vec3(0.0, 0.0, 1.0));
        assert!((w.yaw - std::f32::consts::PI).abs() < 1e-5);
        let y = w.yaw;
        w.face(vec3(0.001, 5.0, 0.0)); // too small on the ground plane: unchanged
        assert_eq!(w.yaw, y);
        w.face(vec3(1.0, 0.0, 0.0));
        assert!((w.yaw - (std::f32::consts::FRAC_PI_2 + std::f32::consts::PI)).abs() < 1e-5);
    }

    #[test]
    fn arrived_walker_shuffles_to_a_moved_stand_spot() {
        let mut w = Walker::new();
        w.place(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), "r");
        w.final_pos = vec3(1.0, 0.0, 0.0);
        let moving = w.step(0.1);
        assert!(moving);
        assert!((w.pos.x - 0.115).abs() < 1e-4); // WALK_SPEED * 0.5 * dt
        for _ in 0..30 {
            w.step(0.1);
        }
        assert!(w.pos.distance(vec3(1.0, 0.0, 0.0)) < 1e-3);
    }
}
