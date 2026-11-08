//! Navigation for living hunters (contract 9.1): a per-map graph, deterministic A*, and the `NavWalker` that carries a
//! hunter's sim position along a path over real time. Pure logic: no drawing, no GL, no window.
//!
//! # How a mode uses it
//!
//! ```ignore
//! // 1. Build the graph once per map. From JSON (nodes, edges, room rectangles), or in code with the builder calls.
//! let nav = Rc::new(NavMap::classic());                         // data/nav/classic.json
//! let nav2 = NavMap::from_json(include_str!("../data/nav/x.json"))?;   // any other map, same format
//! let mut b = NavMap::new("mymap");                              // or by hand:
//! b.add_room(RoomRect::new("hall", (0.0, 0.0), (6.0, 6.0)));
//! b.add_node(NavNode::new("C:hall", NavKind::Waypoint, "hall", vec3(3.0, 0.0, 3.0)));
//! b.link("C:hall", "P0:hall");                                   // undirected, weight = distance
//! b.link_jump("L:west", "DO:doorL");                             // a jump edge: the walker steps across it
//!
//! // 2. Put the hunters in a Crowd (behaviour.rs): one NavWalker + one idle-behaviour brain per hunter.
//! let mut crowd = Crowd::new(nav.clone(), BehaviourData::embedded(), session.seed_value);
//! crowd.add("dovelle", "stage", 0);                              // stands at the room's stand slot
//!
//! // 3. The sim decides WHEN and WHERE (the Godot rule is unchanged); the crowd only decides HOW she gets there.
//! crowd.send("dovelle", "dining", 0, Pace::Budget(budget_for(level)));   // A* from where she stands now
//! crowd.send_to("dovelle", "doorL", pos, look, Pace::Walk);      // a computed stand spot (the office door creeps closer)
//!
//! // 4. Tick every frame with real dt (seconds). Idle behaviour (stare / task / wait) runs between sim moves.
//! let events = crowd.tick(dt_s);                                 // Vec<BehaviourEvent>: FirstStare, TaskStart, ...
//!
//! // 5. Read back for rendering and for camera visibility.
//! let a = crowd.agent("dovelle").unwrap();
//! a.walker.pos; a.walker.yaw; a.walker.room();                   // sim position, facing, the room she is in right now
//! a.walker.moving; a.walker.running;                             // walk / sprint clip
//! a.brain.view().pose;                                           // Some("stare_camera") etc. when idle behaviour owns the pose
//! crowd.visible_in("stage");                                     // hunters on the stage camera's feed (by position, so a
//!                                                                // hunter crossing the lane shows up in the next feed)
//! crowd.stare_pulse("stage");                                    // 0..1 soft static pulse for that room's feed
//! // drive an Actor with `behaviour::ActorDriver::drive(&agent, &mut actor)` while the sim has no pose of its own for her.
//! ```
//!
//! Collision and sight checks use `walker.pos` (the sim position). `NavWalker::room()` is the room she is standing in
//! right now (it flips when she reaches a doorway node into the next room); `NavMap::room_at(pos)` is by rectangle.
//!
//! # Map JSON (`nw.nav/1`)
//!
//! ```text
//! { "schema": "nw.nav/1", "id": "classic",
//!   "rooms": { "stage": {"min": [x, z], "max": [x, z], "y": [lo, hi]?} },     // visible area of the room's camera
//!   "nodes": [ {"id": "P0:stage", "kind": "stand", "room": "stage", "p": [x, y, z], "look": [x, y, z]?,
//!               "task": "sit"?, "pace_to": [x, y, z]?, "between": ["a", "b"]?, "cam": 1?} ],
//!   "edges": [ ["a", "b"], ["a", "b", "jump"] ] }
//! ```
//!
//! Kinds: `waypoint`, `stand`, `doorway` (room "" and `between`), `vent`, `task`, `camera` (never linked; a look target),
//! `door` (the office-door stand), `outer` (down the hall from it).

#![allow(dead_code)]

use crate::core::graph::{walked_length, Graph, Leg};
use macroquad::prelude::{vec3, Vec3};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, VecDeque};

/// The classic building's graph (night_watch_building.gd + night_watch_paths.gd), office at the origin.
pub const CLASSIC_NAV_JSON: &str = include_str!("../data/nav/classic.json");

// ------------------------------------------------------------------------------------------------ nodes

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavKind {
    Waypoint,
    Stand,
    Doorway,
    Vent,
    Task,
    Camera,
    Door,
    Outer,
}

impl NavKind {
    pub fn parse(s: &str) -> Option<NavKind> {
        Some(match s {
            "waypoint" => NavKind::Waypoint,
            "stand" => NavKind::Stand,
            "doorway" => NavKind::Doorway,
            "vent" => NavKind::Vent,
            "task" => NavKind::Task,
            "camera" => NavKind::Camera,
            "door" => NavKind::Door,
            "outer" => NavKind::Outer,
            _ => return None,
        })
    }

    /// Cameras are look targets, not places to walk.
    pub fn walkable(self) -> bool {
        self != NavKind::Camera
    }
}

#[derive(Clone, Debug)]
pub struct NavNode {
    pub key: String,
    pub kind: NavKind,
    /// The room the node is in ("" for doorways, which are between two rooms).
    pub room: String,
    pub between: Option<(String, String)>,
    pub p: Vec3,
    /// Where a hunter standing here looks (stand spots: the room camera; task spots: the thing she works on).
    pub look: Option<Vec3>,
    /// Task id for task spots.
    pub task: Option<String>,
    /// A second end for the `pace` task.
    pub pace_to: Option<Vec3>,
    /// Camera number for camera nodes (0 otherwise).
    pub cam: i32,
}

impl NavNode {
    pub fn new(key: &str, kind: NavKind, room: &str, p: Vec3) -> NavNode {
        NavNode {
            key: key.to_string(),
            kind,
            room: room.to_string(),
            between: None,
            p,
            look: None,
            task: None,
            pace_to: None,
            cam: 0,
        }
    }
    pub fn with_look(mut self, look: Vec3) -> NavNode {
        self.look = Some(look);
        self
    }
    pub fn with_task(mut self, task: &str) -> NavNode {
        self.task = Some(task.to_string());
        self
    }
    pub fn with_pace_to(mut self, p: Vec3) -> NavNode {
        self.pace_to = Some(p);
        self
    }
    pub fn with_between(mut self, a: &str, b: &str) -> NavNode {
        self.between = Some((a.to_string(), b.to_string()));
        self
    }
}

/// A room's floor area on the XZ plane (with an optional height band): what the room's camera sees.
#[derive(Clone, Debug)]
pub struct RoomRect {
    pub id: String,
    pub min: (f32, f32),
    pub max: (f32, f32),
    pub y: (f32, f32),
}

impl RoomRect {
    pub fn new(id: &str, min: (f32, f32), max: (f32, f32)) -> RoomRect {
        RoomRect { id: id.to_string(), min, max, y: (f32::NEG_INFINITY, f32::INFINITY) }
    }
    pub fn contains(&self, p: Vec3) -> bool {
        p.x >= self.min.0 && p.x <= self.max.0 && p.z >= self.min.1 && p.z <= self.max.1 && p.y >= self.y.0 && p.y <= self.y.1
    }
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    to: usize,
    w: f32,
    jump: bool,
}

/// A* open-list entry. Ordered so the BinaryHeap pops the lowest f, and among equal f the lowest node index: the tie-break
/// that makes every search deterministic.
struct Open {
    f: f32,
    idx: usize,
}

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f).then_with(|| other.idx.cmp(&self.idx))
    }
}

// ------------------------------------------------------------------------------------------------ the map

#[derive(Clone, Debug, Default)]
pub struct NavMap {
    pub id: String,
    nodes: Vec<NavNode>,
    adj: Vec<Vec<Edge>>,
    index: HashMap<String, usize>,
    rects: Vec<RoomRect>,
    /// room -> rooms joined to it by a walkable edge or a doorway (BTree: deterministic order)
    room_adj: BTreeMap<String, BTreeSet<String>>,
    /// links that named an unknown node (reported by `validate`)
    bad_links: Vec<String>,
}

fn arr3(v: &Value, key: &str) -> Option<Vec3> {
    let a = v.get(key)?.as_array()?;
    if a.len() < 3 {
        return None;
    }
    Some(vec3(a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32))
}

fn arr2(v: &Value, key: &str) -> Option<(f32, f32)> {
    let a = v.get(key)?.as_array()?;
    if a.len() < 2 {
        return None;
    }
    Some((a[0].as_f64()? as f32, a[1].as_f64()? as f32))
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

impl NavMap {
    pub fn new(id: &str) -> NavMap {
        NavMap { id: id.to_string(), ..NavMap::default() }
    }

    /// The classic building (embedded). Panics only if the embedded file is broken, which `--check` and the tests catch.
    pub fn classic() -> NavMap {
        NavMap::from_json(CLASSIC_NAV_JSON).unwrap_or_else(|_| NavMap::new("classic"))
    }

    /// Builds a map from an `nw.nav/1` document.
    pub fn from_json(src: &str) -> Result<NavMap, String> {
        let root: Value = serde_json::from_str(src).map_err(|e| format!("nav json: {e}"))?;
        let mut map = NavMap::new(&text(&root, "id"));
        if let Some(rooms) = root.get("rooms").and_then(|r| r.as_object()) {
            for (id, r) in rooms {
                let min = arr2(r, "min").ok_or_else(|| format!("nav json: room {id} has no min"))?;
                let max = arr2(r, "max").ok_or_else(|| format!("nav json: room {id} has no max"))?;
                let mut rect = RoomRect::new(id, min, max);
                if let Some(y) = arr2(r, "y") {
                    rect.y = y;
                }
                map.add_room(rect);
            }
        }
        let nodes = root.get("nodes").and_then(|n| n.as_array()).ok_or_else(|| "nav json: no nodes".to_string())?;
        for n in nodes {
            let key = text(n, "id");
            if key.is_empty() {
                return Err("nav json: a node has no id".to_string());
            }
            let kind_name = text(n, "kind");
            let kind = NavKind::parse(&kind_name).ok_or_else(|| format!("nav json: node {key} has unknown kind '{kind_name}'"))?;
            let p = arr3(n, "p").ok_or_else(|| format!("nav json: node {key} has no p"))?;
            let mut node = NavNode::new(&key, kind, &text(n, "room"), p);
            node.look = arr3(n, "look");
            node.pace_to = arr3(n, "pace_to");
            let task = text(n, "task");
            if !task.is_empty() {
                node.task = Some(task);
            }
            if let Some(b) = n.get("between").and_then(|b| b.as_array()) {
                if b.len() >= 2 {
                    node.between = Some((b[0].as_str().unwrap_or("").to_string(), b[1].as_str().unwrap_or("").to_string()));
                }
            }
            node.cam = n.get("cam").and_then(|c| c.as_f64()).map(|c| c as i32).unwrap_or(0);
            map.add_node(node);
        }
        if let Some(edges) = root.get("edges").and_then(|e| e.as_array()) {
            for e in edges {
                let a = match e.as_array() {
                    Some(a) if a.len() >= 2 => a,
                    _ => return Err("nav json: an edge needs two node ids".to_string()),
                };
                let (x, y) = (a[0].as_str().unwrap_or(""), a[1].as_str().unwrap_or(""));
                if a.len() >= 3 && a[2].as_str() == Some("jump") {
                    map.link_jump(x, y);
                } else {
                    map.link(x, y);
                }
            }
        }
        Ok(map)
    }

    // ---- building

    pub fn add_room(&mut self, rect: RoomRect) {
        if let Some(r) = self.rects.iter_mut().find(|r| r.id == rect.id) {
            *r = rect;
        } else {
            self.rects.push(rect);
        }
    }

    /// Adds a node; the same key again replaces the node and keeps its edges.
    pub fn add_node(&mut self, node: NavNode) {
        if let Some((a, b)) = &node.between {
            if !a.is_empty() && !b.is_empty() {
                self.room_adj.entry(a.clone()).or_default().insert(b.clone());
                self.room_adj.entry(b.clone()).or_default().insert(a.clone());
            }
        }
        if let Some(&i) = self.index.get(&node.key) {
            self.nodes[i] = node;
            return;
        }
        self.index.insert(node.key.clone(), self.nodes.len());
        self.nodes.push(node);
        self.adj.push(Vec::new());
    }

    fn connect(&mut self, a: &str, b: &str, jump: bool) {
        let (ia, ib) = match (self.index.get(a), self.index.get(b)) {
            (Some(&x), Some(&y)) => (x, y),
            _ => {
                self.bad_links.push(format!("{a} - {b}"));
                return;
            }
        };
        if ia == ib {
            return;
        }
        let w = self.nodes[ia].p.distance(self.nodes[ib].p);
        if !self.adj[ia].iter().any(|e| e.to == ib && e.jump == jump) {
            self.adj[ia].push(Edge { to: ib, w, jump });
        }
        if !self.adj[ib].iter().any(|e| e.to == ia && e.jump == jump) {
            self.adj[ib].push(Edge { to: ia, w, jump });
        }
        if !jump {
            let (ra, rb) = (self.nodes[ia].room.clone(), self.nodes[ib].room.clone());
            if !ra.is_empty() && !rb.is_empty() && ra != rb {
                self.room_adj.entry(ra.clone()).or_default().insert(rb.clone());
                self.room_adj.entry(rb).or_default().insert(ra);
            }
        }
    }

    /// Undirected link, weight = distance.
    pub fn link(&mut self, a: &str, b: &str) {
        self.connect(a, b, false);
    }

    /// Undirected jump link: the leg into the far node is marked `jump` when traversed.
    pub fn link_jump(&mut self, a: &str, b: &str) {
        self.connect(a, b, true);
    }

    // ---- lookups

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn nodes(&self) -> &[NavNode] {
        &self.nodes
    }
    pub fn rooms(&self) -> &[RoomRect] {
        &self.rects
    }
    pub fn node(&self, key: &str) -> Option<&NavNode> {
        self.index.get(key).map(|&i| &self.nodes[i])
    }
    pub fn pos(&self, key: &str) -> Option<Vec3> {
        self.node(key).map(|n| n.p)
    }

    /// The room whose rectangle holds `p` (the first in file order), if any.
    pub fn room_at(&self, p: Vec3) -> Option<&str> {
        self.rects.iter().find(|r| r.contains(p)).map(|r| r.id.as_str())
    }

    /// The key of the node a sim room hangs from: `C:<room>`, else `D:<room>` (office doors), else the room's first node.
    pub fn hub_key(&self, room: &str) -> Option<String> {
        for prefix in ["C:", "D:"] {
            let k = format!("{prefix}{room}");
            if self.index.contains_key(&k) {
                return Some(k);
            }
        }
        self.nodes.iter().find(|n| n.room == room && n.kind.walkable() && n.kind != NavKind::Doorway).map(|n| n.key.clone())
    }

    /// The stand node of a room slot: `P<slot>:<room>` (slots above 1 use slot 1), else `D:<room>`, else the hub.
    pub fn stand_key(&self, room: &str, slot: usize) -> Option<String> {
        let k = format!("P{}:{}", slot.min(1), room);
        if self.index.contains_key(&k) {
            return Some(k);
        }
        let d = format!("D:{room}");
        if self.index.contains_key(&d) {
            return Some(d);
        }
        self.hub_key(room)
    }

    pub fn stand_node(&self, room: &str, slot: usize) -> Option<&NavNode> {
        self.stand_key(room, slot).and_then(|k| self.node(&k))
    }

    /// The nearest node a hunter could walk from: any non-camera node that has an edge. Ties go to the earlier node.
    pub fn nearest_walkable(&self, p: Vec3) -> Option<&NavNode> {
        let mut best: Option<(usize, f32)> = None;
        for (i, n) in self.nodes.iter().enumerate() {
            if !n.kind.walkable() || self.adj[i].is_empty() {
                continue;
            }
            let d = n.p.distance_squared(p);
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| &self.nodes[i])
    }

    /// Camera nodes of a room, in file order.
    pub fn cameras(&self, room: &str) -> Vec<&NavNode> {
        self.nodes.iter().filter(|n| n.kind == NavKind::Camera && n.room == room).collect()
    }

    /// The camera of `room` nearest to `p` (the stare target).
    pub fn nearest_camera(&self, room: &str, p: Vec3) -> Option<&NavNode> {
        let mut best: Option<(&NavNode, f32)> = None;
        for c in self.cameras(room) {
            let d = c.p.distance_squared(p);
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((c, d));
            }
        }
        best.map(|(c, _)| c)
    }

    /// Task spots in `room`, optionally only for one task, in file order.
    pub fn task_spots(&self, room: &str, task: Option<&str>) -> Vec<&NavNode> {
        self.nodes
            .iter()
            .filter(|n| n.kind == NavKind::Task && n.room == room && task.map_or(true, |t| n.task.as_deref() == Some(t)))
            .collect()
    }

    /// Rooms joined to `room` by a walkable edge or a doorway.
    pub fn room_neighbours(&self, room: &str) -> Vec<String> {
        self.room_adj.get(room).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }

    /// `room` and every room within `hops` doorways of it (breadth first, deterministic order).
    pub fn rooms_within(&self, room: &str, hops: u32) -> Vec<String> {
        let mut seen: Vec<String> = vec![room.to_string()];
        let mut frontier: Vec<String> = vec![room.to_string()];
        for _ in 0..hops {
            let mut next = Vec::new();
            for r in &frontier {
                for n in self.room_neighbours(r) {
                    if !seen.contains(&n) {
                        seen.push(n.clone());
                        next.push(n);
                    }
                }
            }
            frontier = next;
        }
        seen
    }

    /// The same graph for code that uses `core::graph::Graph` (Dijkstra).
    pub fn to_graph(&self) -> Graph {
        let mut g = Graph::new();
        for n in &self.nodes {
            g.add_node(&n.key, n.p);
        }
        for (i, edges) in self.adj.iter().enumerate() {
            for e in edges {
                if e.jump {
                    g.link_jump(&self.nodes[i].key, &self.nodes[e.to].key);
                } else {
                    g.link(&self.nodes[i].key, &self.nodes[e.to].key);
                }
            }
        }
        g
    }

    // ---- A*

    fn astar(&self, start: usize, goal: usize) -> Option<Vec<(usize, bool)>> {
        let n = self.nodes.len();
        let goal_p = self.nodes[goal].p;
        let mut g = vec![f32::INFINITY; n];
        let mut prev: Vec<Option<(usize, bool)>> = vec![None; n];
        let mut closed = vec![false; n];
        let mut open = BinaryHeap::new();
        g[start] = 0.0;
        open.push(Open { f: self.nodes[start].p.distance(goal_p), idx: start });
        while let Some(Open { idx: u, .. }) = open.pop() {
            if closed[u] {
                continue;
            }
            if u == goal {
                break;
            }
            closed[u] = true;
            for e in &self.adj[u] {
                if closed[e.to] {
                    continue;
                }
                let ng = g[u] + e.w;
                if ng < g[e.to] {
                    g[e.to] = ng;
                    prev[e.to] = Some((u, e.jump));
                    open.push(Open { f: ng + self.nodes[e.to].p.distance(goal_p), idx: e.to });
                }
            }
        }
        if !g[goal].is_finite() {
            return None;
        }
        let mut out = Vec::new();
        let mut at = goal;
        loop {
            match prev[at] {
                Some((u, jump)) => {
                    out.push((at, jump));
                    at = u;
                }
                None => {
                    out.push((at, false));
                    break;
                }
            }
        }
        out.reverse();
        Some(out)
    }

    /// The shortest walk between two nodes. The first leg is the start itself and never a jump. Empty when a key is unknown,
    /// the nodes are not connected, or either is a camera.
    pub fn route(&self, from: &str, to: &str) -> Vec<Leg> {
        let (a, b) = match (self.index.get(from), self.index.get(to)) {
            (Some(&a), Some(&b)) => (a, b),
            _ => return Vec::new(),
        };
        match self.astar(a, b) {
            Some(path) => path
                .into_iter()
                .map(|(i, jump)| Leg { p: self.nodes[i].p, jump, key: self.nodes[i].key.clone() })
                .collect(),
            None => Vec::new(),
        }
    }

    /// The walk from wherever she stands (from the nearest node) to a node.
    pub fn route_from(&self, pos: Vec3, to: &str) -> Vec<Leg> {
        match self.nearest_walkable(pos) {
            Some(n) => self.route(&n.key.clone(), to),
            None => Vec::new(),
        }
    }

    /// The walk between two rooms' hub nodes (what `Building::legs_for` was).
    pub fn route_rooms(&self, from_room: &str, to_room: &str) -> Vec<Leg> {
        match (self.hub_key(from_room), self.hub_key(to_room)) {
            (Some(a), Some(b)) => self.route(&a, &b),
            _ => Vec::new(),
        }
    }

    /// Length of the walked part of the shortest route (jumps free); None when there is no route.
    pub fn distance(&self, from: &str, to: &str) -> Option<f32> {
        let legs = self.route(from, to);
        if legs.is_empty() {
            None
        } else {
            Some(walked_length(&legs))
        }
    }

    // ---- checks

    /// Problems in the map (empty = fine): unknown link ends, rooms without a node, cameras in unknown rooms, task spots
    /// without a task or a look target, and walkable nodes cut off from the rest.
    pub fn validate(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for b in &self.bad_links {
            out.push(format!("link names an unknown node: {b}"));
        }
        for r in &self.rects {
            if !self.nodes.iter().any(|n| n.room == r.id && n.kind.walkable()) {
                out.push(format!("room {} has no walkable node", r.id));
            }
            if r.min.0 > r.max.0 || r.min.1 > r.max.1 {
                out.push(format!("room {} has an inverted rectangle", r.id));
            }
        }
        for n in &self.nodes {
            if !n.room.is_empty() && !self.rects.iter().any(|r| r.id == n.room) {
                out.push(format!("node {} is in unknown room {}", n.key, n.room));
            }
            if n.kind == NavKind::Task && (n.task.is_none() || n.look.is_none()) {
                out.push(format!("task spot {} needs a task and a look target", n.key));
            }
            if n.kind == NavKind::Doorway && n.between.is_none() {
                out.push(format!("doorway {} needs a between pair", n.key));
            }
        }
        // connectivity over every edge, from the first walkable node that has an edge
        let first = (0..self.nodes.len()).find(|&i| self.nodes[i].kind.walkable() && !self.adj[i].is_empty());
        if let Some(s) = first {
            let mut seen = vec![false; self.nodes.len()];
            let mut stack = vec![s];
            seen[s] = true;
            while let Some(u) = stack.pop() {
                for e in &self.adj[u] {
                    if !seen[e.to] {
                        seen[e.to] = true;
                        stack.push(e.to);
                    }
                }
            }
            // a separate duct network (only vent nodes, no link to the building) is allowed: the vent run is its own space
            for i in 0..self.nodes.len() {
                if seen[i] || self.nodes[i].kind != NavKind::Vent {
                    continue;
                }
                let mut comp = vec![i];
                let mut mark = vec![false; self.nodes.len()];
                mark[i] = true;
                let mut st = vec![i];
                while let Some(u) = st.pop() {
                    for e in &self.adj[u] {
                        if !mark[e.to] && !seen[e.to] {
                            mark[e.to] = true;
                            comp.push(e.to);
                            st.push(e.to);
                        }
                    }
                }
                if comp.iter().all(|&c| self.nodes[c].kind == NavKind::Vent) {
                    for c in comp {
                        seen[c] = true;
                    }
                }
            }
            for (i, n) in self.nodes.iter().enumerate() {
                if n.kind.walkable() && !seen[i] {
                    out.push(format!("node {} is not connected to the rest", n.key));
                }
            }
        }
        out
    }
}

// ------------------------------------------------------------------------------------------------ the walker

/// How fast a trip is walked.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pace {
    /// 2.3 m/s.
    Walk,
    /// 5.5 m/s.
    Run,
    /// Arrive in about this many seconds: route length / budget, clamped to MIN_SPEED..MAX_SPEED (the Godot rule; see
    /// `core::budget_for`).
    Budget(f32),
}

/// What one `NavWalker::step` did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepInfo {
    pub moving: bool,
    /// She reached her final spot this step (once per trip).
    pub arrived: bool,
    /// She crossed into this room this step.
    pub entered: Option<String>,
}

#[derive(Clone, Debug)]
struct Step {
    p: Vec3,
    jump: bool,
    room: String,
}

/// One hunter's visible movement along a path. Advances the sim position at walk or run speed; faces her direction of
/// travel while moving and turns to `look` when she has stopped. It never reads or writes sim state.
#[derive(Clone, Debug)]
pub struct NavWalker {
    pub pos: Vec3,
    /// Yaw in the actor convention (0 faces -z, same as `place::yaw_toward`).
    pub yaw: f32,
    pub moving: bool,
    pub running: bool,
    pub final_pos: Vec3,
    pub look: Option<Vec3>,
    room: String,
    dest_room: String,
    queue: VecDeque<Step>,
    speed: f32,
    seg_from: Vec3,
    seg_len: f32,
    seg_started: bool,
    arrived: bool,
}

impl Default for NavWalker {
    fn default() -> Self {
        NavWalker::new()
    }
}

/// The yaw that faces along `d` on the ground plane (front is -z): atan2(d.x, d.z) + PI, or `keep` when `d` is tiny.
pub fn yaw_of(d: Vec3, keep: f32) -> f32 {
    if d.x * d.x + d.z * d.z > 0.01 * 0.01 {
        d.x.atan2(d.z) + std::f32::consts::PI
    } else {
        keep
    }
}

/// The smallest signed angle from `a` to `b` in -PI..PI.
pub fn angle_diff(a: f32, b: f32) -> f32 {
    (b - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

impl NavWalker {
    pub const WALK_SPEED: f32 = 2.3;
    pub const RUN_SPEED: f32 = 5.5;
    pub const MIN_SPEED: f32 = 1.2;
    pub const MAX_SPEED: f32 = 6.0;
    /// How fast she turns to face something once she has stopped (radians per second).
    pub const TURN_RATE: f32 = 7.0;
    const ARRIVE_EPS: f32 = 0.02;

    pub fn new() -> NavWalker {
        NavWalker {
            pos: Vec3::ZERO,
            yaw: 0.0,
            moving: false,
            running: false,
            final_pos: Vec3::ZERO,
            look: None,
            room: String::new(),
            dest_room: String::new(),
            queue: VecDeque::new(),
            speed: NavWalker::WALK_SPEED,
            seg_from: Vec3::ZERO,
            seg_len: 0.0,
            seg_started: false,
            arrived: true,
        }
    }

    /// Stand somewhere, facing `look`, with no trip in progress.
    pub fn place(&mut self, pos: Vec3, room: &str, look: Option<Vec3>) {
        self.pos = pos;
        self.final_pos = pos;
        self.look = look;
        self.room = room.to_string();
        self.dest_room = room.to_string();
        self.queue.clear();
        self.seg_started = false;
        self.moving = false;
        self.running = false;
        self.arrived = true;
        if let Some(l) = look {
            self.yaw = yaw_of(l - pos, self.yaw);
        }
    }

    /// The room she is in right now. It changes when she reaches the next room's first node (a doorway counts as the room
    /// she is entering).
    pub fn room(&self) -> &str {
        &self.room
    }

    /// The room the current trip ends in (equal to `room()` when standing).
    pub fn dest_room(&self) -> &str {
        &self.dest_room
    }

    pub fn is_walking(&self) -> bool {
        !self.queue.is_empty()
    }

    /// True once the trip is over and she stands on her final spot.
    pub fn arrived(&self) -> bool {
        self.arrived
    }

    /// No trip left and (nearly) on the final spot.
    pub fn at_rest(&self) -> bool {
        let d = vec3(self.final_pos.x - self.pos.x, 0.0, self.final_pos.z - self.pos.z);
        self.queue.is_empty() && d.length() <= 0.05
    }

    /// The pace (m/s) the current trip is walked at.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Takes `legs` (from `NavMap::route*`) to `to_room`, ending on `final_pos`, then turning to `look`. An empty route
    /// puts her on the final spot at once (nowhere to walk). Rooms along the way come from the map's nodes.
    pub fn go(&mut self, nav: &NavMap, legs: &[Leg], to_room: &str, final_pos: Vec3, look: Option<Vec3>, pace: Pace) {
        self.dest_room = to_room.to_string();
        self.final_pos = final_pos;
        self.look = look;
        self.queue.clear();
        self.seg_started = false;
        self.arrived = false;
        if legs.is_empty() {
            self.pos = final_pos;
            self.room = to_room.to_string();
            self.moving = false;
            self.arrived = true;
            return;
        }
        let mut rooms: Vec<String> = legs.iter().map(|l| nav.node(&l.key).map(|n| n.room.clone()).unwrap_or_default()).collect();
        // doorways (room "") take the room of the next node along
        let mut next = to_room.to_string();
        for r in rooms.iter_mut().rev() {
            if r.is_empty() {
                *r = next.clone();
            } else {
                next = r.clone();
            }
        }
        let steps: Vec<Step> = legs.iter().zip(rooms).map(|(l, r)| Step { p: l.p, jump: l.jump, room: r }).collect();
        let first = if legs[0].jump { self.pos } else { legs[0].p };
        let length = walked_length(legs) + self.pos.distance(first);
        self.begin(steps, length, pace);
    }

    /// Walks straight through `points` (no graph), for short moves inside a room such as pacing.
    pub fn go_points(&mut self, points: &[Vec3], to_room: &str, final_pos: Vec3, look: Option<Vec3>, pace: Pace) {
        self.dest_room = to_room.to_string();
        self.final_pos = final_pos;
        self.look = look;
        self.queue.clear();
        self.seg_started = false;
        self.arrived = false;
        let mut length = 0.0;
        let mut at = self.pos;
        let mut steps = Vec::new();
        for p in points {
            length += at.distance(*p);
            at = *p;
            steps.push(Step { p: *p, jump: false, room: self.room.clone() });
        }
        self.begin(steps, length, pace);
    }

    fn begin(&mut self, steps: Vec<Step>, length: f32, pace: Pace) {
        self.speed = match pace {
            Pace::Walk => NavWalker::WALK_SPEED,
            Pace::Run => NavWalker::RUN_SPEED,
            Pace::Budget(b) => (length / b.max(0.1)).clamp(NavWalker::MIN_SPEED, NavWalker::MAX_SPEED),
        };
        self.running = self.speed > NavWalker::WALK_SPEED * 1.6;
        self.queue.extend(steps);
    }

    fn enter(&mut self, room: &str, info: &mut StepInfo) {
        if !room.is_empty() && room != self.room {
            self.room = room.to_string();
            info.entered = Some(room.to_string());
        }
    }

    fn turn_to(&mut self, goal: f32, dt: f32) {
        let diff = angle_diff(self.yaw, goal);
        let step = NavWalker::TURN_RATE * dt;
        if diff.abs() <= step {
            self.yaw = goal;
        } else {
            self.yaw += step * diff.signum();
        }
    }

    /// Advance by `dt` seconds. Mirrors night_watch_walker.gd: a jump leg is stepped across, walked legs are covered at
    /// `speed`, and on arrival she shuffles to the final spot (it moves at a door) and turns to `look`.
    pub fn step(&mut self, dt: f32) -> StepInfo {
        let mut info = StepInfo::default();
        let mut budget = dt;
        self.moving = false;
        while budget > 0.0 {
            let (target, jump, room) = match self.queue.front() {
                Some(s) => (s.p, s.jump, s.room.clone()),
                None => break,
            };
            if jump {
                self.pos = target;
                self.enter(&room, &mut info);
                self.queue.pop_front();
                self.seg_started = false;
                continue;
            }
            if !self.seg_started {
                self.seg_from = self.pos;
                self.seg_len = vec3(target.x - self.pos.x, 0.0, target.z - self.pos.z).length().max(1e-4);
                self.seg_started = true;
            }
            let mut d = target - self.pos;
            d.y = 0.0;
            let dist = d.length();
            let mv = self.speed * budget;
            self.moving = true;
            self.yaw = yaw_of(d, self.yaw);
            if mv >= dist {
                self.pos = target;
                budget -= dist / self.speed;
                self.enter(&room, &mut info);
                self.queue.pop_front();
                self.seg_started = false;
            } else {
                self.pos.x += d.x / dist * mv;
                self.pos.z += d.z / dist * mv;
                let remaining = dist - mv;
                let t = (1.0 - remaining / self.seg_len).clamp(0.0, 1.0);
                self.pos.y = self.seg_from.y + (target.y - self.seg_from.y) * t;
                budget = 0.0;
            }
        }
        if self.queue.is_empty() {
            let mut to_final = self.final_pos - self.pos;
            to_final.y = 0.0;
            let len = to_final.length();
            if len > NavWalker::ARRIVE_EPS {
                let m = len.min(NavWalker::WALK_SPEED * 0.5 * dt);
                self.pos.x += to_final.x / len * m;
                self.pos.z += to_final.z / len * m;
                self.moving = m > 0.001;
                // the lift (the stage platform) is taken at the same pace
                let dy = self.final_pos.y - self.pos.y;
                let my = dy.abs().min(NavWalker::WALK_SPEED * 0.5 * dt);
                self.pos.y += my * dy.signum();
            } else {
                self.pos = self.final_pos;
                if !self.arrived {
                    self.arrived = true;
                    let dest = self.dest_room.clone();
                    self.room = dest;
                    info.arrived = true;
                }
            }
            if !self.moving {
                if let Some(l) = self.look {
                    let goal = yaw_of(l - self.pos, self.yaw);
                    self.turn_to(goal, dt);
                }
            }
        }
        info.moving = self.moving;
        info
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn n(map: &mut NavMap, key: &str, x: f32, z: f32) {
        map.add_node(NavNode::new(key, NavKind::Waypoint, "r", vec3(x, 0.0, z)));
    }

    fn small() -> NavMap {
        let mut m = NavMap::new("small");
        m.add_room(RoomRect::new("r", (-10.0, -10.0), (10.0, 10.0)));
        n(&mut m, "S", 0.0, 0.0);
        n(&mut m, "A", 3.0, 1.0);
        n(&mut m, "B", 3.0, -1.0);
        n(&mut m, "G", 6.0, 0.0);
        n(&mut m, "far", 3.0, 30.0);
        n(&mut m, "lone", 50.0, 50.0);
        m.link("S", "A");
        m.link("S", "B");
        m.link("A", "G");
        m.link("B", "G");
        m.link("S", "far");
        m.link("far", "G");
        m
    }

    fn keys(legs: &[Leg]) -> Vec<String> {
        legs.iter().map(|l| l.key.clone()).collect()
    }

    #[test]
    fn a_star_finds_the_shortest_path_and_skips_the_long_way() {
        let mut m = NavMap::new("d");
        m.add_room(RoomRect::new("r", (-1.0, -1.0), (100.0, 100.0)));
        n(&mut m, "a", 0.0, 0.0);
        n(&mut m, "b", 10.0, 0.0);
        n(&mut m, "c", 5.0, 1.0);
        n(&mut m, "far", 5.0, 50.0);
        m.link("a", "b");
        m.link("a", "far");
        m.link("far", "b");
        m.link("a", "c");
        m.link("c", "b");
        // a-b direct is 10, via c is about 10.2
        assert_eq!(keys(&m.route("a", "b")), vec!["a", "b"]);
        let m2 = {
            let mut m = NavMap::new("e");
            n(&mut m, "a", 0.0, 0.0);
            n(&mut m, "c", 5.0, 1.0);
            n(&mut m, "b", 10.0, 0.0);
            n(&mut m, "far", 5.0, 50.0);
            m.link("a", "c");
            m.link("c", "b");
            m.link("a", "far");
            m.link("far", "b");
            m
        };
        assert_eq!(keys(&m2.route("a", "b")), vec!["a", "c", "b"]);
        let d = m2.distance("a", "b").unwrap_or(0.0);
        assert!((d - 2.0 * (26.0f32).sqrt()).abs() < 1e-3);
    }

    #[test]
    fn no_path_unknown_keys_and_same_node() {
        let m = small();
        assert!(m.route("S", "lone").is_empty());
        assert!(m.route("S", "nope").is_empty());
        assert!(m.distance("S", "lone").is_none());
        let same = m.route("A", "A");
        assert_eq!(same.len(), 1);
        assert!(!same[0].jump);
    }

    #[test]
    fn equal_cost_ties_resolve_the_same_way_every_time() {
        let m = small();
        let first = keys(&m.route("S", "G"));
        // A (added first) wins the tie against B
        assert_eq!(first, vec!["S", "A", "G"]);
        for _ in 0..50 {
            assert_eq!(keys(&m.route("S", "G")), first);
        }
        // and a rebuilt map gives the same answer
        assert_eq!(keys(&small().route("S", "G")), first);
    }

    #[test]
    fn jump_edges_are_marked_and_free_to_walk() {
        let mut m = NavMap::new("j");
        n(&mut m, "a", 0.0, 0.0);
        n(&mut m, "b", 5.0, 0.0);
        n(&mut m, "c", 5.0, -20.0);
        n(&mut m, "d", 5.0, -22.0);
        m.link("a", "b");
        m.link_jump("b", "c");
        m.link("c", "d");
        let r = m.route("a", "d");
        assert_eq!(keys(&r), vec!["a", "b", "c", "d"]);
        assert!(!r[0].jump && !r[1].jump && r[2].jump && !r[3].jump);
        assert!((walked_length(&r) - 7.0).abs() < 1e-4);
        let back = m.route("d", "a");
        assert!(back[2].jump);
    }

    #[test]
    fn classic_map_is_valid() {
        let m = NavMap::classic();
        let problems = m.validate();
        assert!(problems.is_empty(), "{problems:?}");
        assert!(m.len() > 50);
        assert_eq!(m.rooms().len(), 10);
    }

    #[test]
    fn classic_matches_the_building_paths() {
        use crate::gfx::building::Building;
        let m = NavMap::classic();
        let g = Building::paths(Vec3::ZERO);
        for room in ["stage", "dining", "backstage", "westhall", "closet", "kitchen", "easthall", "corner"] {
            for prefix in ["C", "S", "L"] {
                let key = format!("{prefix}:{room}");
                let a = m.pos(&key).unwrap_or(Vec3::splat(1e9));
                let b = g.pos(&key).unwrap_or(Vec3::splat(-1e9));
                assert!(a.distance(b) < 1e-3, "{key}: {a:?} vs {b:?}");
            }
            for slot in 0..2 {
                let a = m.stand_node(room, slot).map(|n| n.p).unwrap_or(Vec3::splat(1e9));
                assert!(a.distance(Building::stand_position(room, slot)) < 1e-3, "{room} slot {slot}");
            }
            // the camera node is the feed camera's mount
            let cam = m.cameras(room);
            assert_eq!(cam.len(), 1);
            assert!(cam[0].p.distance(Building::camera_transform(room).0) < 1e-3, "{room} camera");
        }
        for key in ["DO:doorL", "D:doorL", "DO:doorR", "D:doorR"] {
            let a = m.pos(key).unwrap_or(Vec3::splat(1e9));
            assert!(a.distance(g.pos(key).unwrap_or(Vec3::splat(-1e9))) < 1e-3, "{key}");
        }
    }

    #[test]
    fn a_star_agrees_with_dijkstra_on_every_classic_hub_pair() {
        let m = NavMap::classic();
        let g = m.to_graph();
        let hubs = ["stage", "dining", "backstage", "westhall", "closet", "kitchen", "easthall", "corner", "doorL", "doorR"];
        for a in hubs {
            for b in hubs {
                let ka = m.hub_key(a).unwrap_or_default();
                let kb = m.hub_key(b).unwrap_or_default();
                let la = walked_length(&m.route(&ka, &kb));
                let lb = walked_length(&g.route(&ka, &kb));
                assert!((la - lb).abs() < 1e-3, "{a} -> {b}: {la} vs {lb}");
                assert!(!m.route(&ka, &kb).is_empty(), "{a} -> {b}");
            }
        }
    }

    #[test]
    fn classic_routes_use_the_doorways_and_the_lane() {
        let m = NavMap::classic();
        // stage - backstage share a wall with a doorway: the route goes through it
        let r = m.route_rooms("stage", "backstage");
        assert_eq!(keys(&r), vec!["C:stage", "DW:backstage|stage", "C:backstage"]);
        // the kitchen has no west doorway into the stage: it goes round by the lane
        let k = keys(&m.route_rooms("kitchen", "stage"));
        assert!(k.contains(&"L:kitchen".to_string()) && k.contains(&"L:stage".to_string()), "{k:?}");
        // stage down to dining through dining's north doorway
        let d = keys(&m.route_rooms("stage", "dining"));
        assert!(d.contains(&"DW:N:dining".to_string()), "{d:?}");
        // the hall to the office door has exactly one jump
        let door = m.route_rooms("stage", "doorL");
        assert_eq!(door.iter().filter(|l| l.jump).count(), 1);
        assert_eq!(door.last().map(|l| l.key.as_str()), Some("D:doorL"));
    }

    #[test]
    fn rooms_cameras_and_tasks() {
        let m = NavMap::classic();
        assert_eq!(m.room_at(vec3(0.0, 0.0, -40.0)), Some("stage"));
        assert_eq!(m.room_at(vec3(0.0, 0.0, -36.0)), Some("stage")); // the lane in front of the stage
        assert_eq!(m.room_at(vec3(0.0, 0.0, -30.0)), Some("dining"));
        assert_eq!(m.room_at(vec3(-4.9, 0.0, 0.3)), Some("doorL"));
        assert_eq!(m.room_at(vec3(100.0, 0.0, 100.0)), None);
        let cam = m.nearest_camera("dining", vec3(0.0, 0.0, -32.0));
        assert_eq!(cam.map(|c| c.key.as_str()), Some("CAM:dining"));
        assert!(m.nearest_camera("doorL", Vec3::ZERO).is_none());
        assert_eq!(m.task_spots("kitchen", None).len(), 2);
        assert_eq!(m.task_spots("dining", Some("sit")).len(), 2);
        assert!(m.task_spots("stage", Some("sit")).is_empty());
        assert_eq!(m.hub_key("stage").as_deref(), Some("C:stage"));
        assert_eq!(m.hub_key("doorR").as_deref(), Some("D:doorR"));
        assert_eq!(m.stand_key("dining", 5).as_deref(), Some("P1:dining"));
        // rooms within one doorway of the westhall
        let near = m.rooms_within("westhall", 1);
        assert!(near.contains(&"westhall".to_string()) && near.contains(&"dining".to_string()) && near.contains(&"closet".to_string()));
        assert!(!near.contains(&"kitchen".to_string()));
        assert_eq!(m.rooms_within("stage", 0), vec!["stage".to_string()]);
    }

    #[test]
    fn json_errors_are_reported() {
        assert!(NavMap::from_json("{").is_err());
        assert!(NavMap::from_json(r#"{"nodes": [{"id": "a", "kind": "wat", "p": [0,0,0]}]}"#).is_err());
        assert!(NavMap::from_json(r#"{"nodes": [{"id": "a", "kind": "stand", "p": [0,0]}]}"#).is_err());
        assert!(NavMap::from_json(r#"{"nodes": [], "edges": [["a"]]}"#).is_err());
        let ok = NavMap::from_json(r#"{"id":"t","nodes":[{"id":"a","kind":"stand","room":"","p":[0,0,0]},{"id":"b","kind":"vent","room":"","p":[1,0,0]}],"edges":[["a","b"],["a","zzz"]]}"#);
        let m = ok.unwrap_or_default();
        assert_eq!(m.len(), 2);
        assert!(m.validate().iter().any(|p| p.contains("zzz")));
    }

    /// Runs the walker to the end of its trip; returns (steps, rooms entered, keys of nodes passed within one step (35 cm)).
    fn run(m: &NavMap, w: &mut NavWalker, watch: &[&str], max_s: f32) -> (usize, Vec<String>, Vec<String>) {
        let dt = 0.05;
        let mut rooms = Vec::new();
        let mut hit = Vec::new();
        let mut steps = 0;
        while steps < (max_s / dt) as usize && !(w.arrived() && !w.is_walking()) {
            let info = w.step(dt);
            if let Some(r) = info.entered {
                rooms.push(r);
            }
            for k in watch {
                if let Some(p) = m.pos(k) {
                    if w.pos.distance(p) < 0.35 && !hit.contains(&k.to_string()) {
                        hit.push(k.to_string());
                    }
                }
            }
            steps += 1;
        }
        (steps, rooms, hit)
    }

    #[test]
    fn walker_reaches_the_room_over_time_through_the_doorway() {
        let m = NavMap::classic();
        let mut w = NavWalker::new();
        let from = m.stand_node("stage", 0).map(|n| n.p).unwrap_or_default();
        w.place(from, "stage", None);
        let to = m.stand_node("backstage", 0).cloned().unwrap_or_else(|| NavNode::new("x", NavKind::Stand, "backstage", Vec3::ZERO));
        let legs = m.route_from(w.pos, &to.key);
        assert_eq!(legs.first().map(|l| l.key.as_str()), Some("P0:stage"));
        w.go(&m, &legs, "backstage", to.p, to.look, Pace::Walk);
        assert!(w.is_walking());
        assert_eq!(w.room(), "stage");
        let (steps, rooms, hit) = run(&m, &mut w, &["DW:backstage|stage", "C:stage", "C:backstage"], 60.0);
        assert!(hit.contains(&"DW:backstage|stage".to_string()), "{hit:?}");
        assert!(w.arrived() && !w.is_walking());
        assert_eq!(w.room(), "backstage");
        assert_eq!(rooms.last().map(|s| s.as_str()), Some("backstage"));
        assert!(w.pos.distance(to.p) < 1e-3);
        // about 10 m at 2.3 m/s takes seconds, not one frame
        assert!(steps > 40 && steps < 200, "{steps}");
        // the room flipped only at the doorway
        assert!(rooms.len() <= 2);
    }

    #[test]
    fn walker_crosses_the_lane_and_steps_across_the_office_jump() {
        let m = NavMap::classic();
        let mut w = NavWalker::new();
        let from = m.stand_node("stage", 0).map(|n| n.p).unwrap_or_default();
        w.place(from, "stage", None);
        let door = m.pos("D:doorL").unwrap_or_default();
        let legs = m.route_from(w.pos, "D:doorL");
        w.go(&m, &legs, "doorL", door, Some(Vec3::ZERO), Pace::Run);
        assert!(w.running);
        let mut longest = 0.0f32;
        let mut last = w.pos;
        let mut guard = 0;
        while !(w.arrived() && !w.is_walking()) && guard < 2000 {
            w.step(0.05);
            longest = longest.max(w.pos.distance(last));
            last = w.pos;
            guard += 1;
        }
        assert!(w.pos.distance(door) < 1e-3);
        assert_eq!(w.room(), "doorL");
        assert!(longest > 20.0, "the hall to the office is one jump: {longest}");
    }

    #[test]
    fn walker_speed_never_exceeds_max_and_pace_budget_is_clamped() {
        let mut m = NavMap::new("line");
        m.add_room(RoomRect::new("r", (-1.0, -1.0), (100.0, 100.0)));
        n(&mut m, "a", 0.0, 0.0);
        n(&mut m, "b", 30.0, 0.0);
        m.link("a", "b");
        let legs = m.route("a", "b");
        let mut w = NavWalker::new();
        w.place(Vec3::ZERO, "r", None);
        w.go(&m, &legs, "r", vec3(30.0, 0.0, 0.0), None, Pace::Budget(1.5));
        assert_eq!(w.speed(), NavWalker::MAX_SPEED);
        assert!(w.running);
        let mut last = w.pos;
        for _ in 0..200 {
            w.step(0.1);
            assert!(w.pos.distance(last) <= NavWalker::MAX_SPEED * 0.1 + 1e-3);
            last = w.pos;
        }
        assert!(w.pos.distance(vec3(30.0, 0.0, 0.0)) < 1e-3);
        w.place(Vec3::ZERO, "r", None);
        w.go(&m, &legs, "r", vec3(30.0, 0.0, 0.0), None, Pace::Budget(1000.0));
        assert_eq!(w.speed(), NavWalker::MIN_SPEED);
        assert!(!w.running);
        w.go(&m, &legs, "r", vec3(30.0, 0.0, 0.0), None, Pace::Walk);
        assert_eq!(w.speed(), NavWalker::WALK_SPEED);
    }

    #[test]
    fn walker_faces_travel_then_turns_to_look() {
        let mut m = NavMap::new("line");
        m.add_room(RoomRect::new("r", (-1.0, -1.0), (100.0, 100.0)));
        n(&mut m, "a", 0.0, 0.0);
        n(&mut m, "b", 10.0, 0.0);
        m.link("a", "b");
        let legs = m.route("a", "b");
        let mut w = NavWalker::new();
        w.place(Vec3::ZERO, "r", None);
        let look = vec3(10.0, 0.0, 10.0);
        w.go(&m, &legs, "r", vec3(10.0, 0.0, 0.0), Some(look), Pace::Walk);
        w.step(0.1);
        // moving +x: the front (-z) points along +x, yaw = atan2(1, 0) + PI
        assert!(angle_diff(w.yaw, std::f32::consts::FRAC_PI_2 + std::f32::consts::PI).abs() < 1e-4);
        for _ in 0..200 {
            w.step(0.05);
        }
        assert!(w.arrived());
        let want = crate::hunters::place::yaw_toward(w.pos, look);
        assert!(angle_diff(w.yaw, want).abs() < 1e-3, "{} vs {}", w.yaw, want);
    }

    #[test]
    fn walker_takes_the_stage_lift_and_an_empty_route_teleports() {
        let m = NavMap::classic();
        let mut w = NavWalker::new();
        let from = m.pos("C:stage").unwrap_or_default();
        w.place(from, "stage", None);
        let to = m.stand_node("stage", 1).cloned();
        let to = to.unwrap_or_else(|| NavNode::new("x", NavKind::Stand, "stage", Vec3::ZERO));
        let legs = m.route_from(w.pos, &to.key);
        w.go(&m, &legs, "stage", to.p, None, Pace::Walk);
        let mut seen_mid_lift = false;
        for _ in 0..400 {
            w.step(0.05);
            if w.pos.y > 0.05 && w.pos.y < to.p.y - 0.05 {
                seen_mid_lift = true;
            }
        }
        assert!(seen_mid_lift, "the platform is climbed, not popped onto");
        assert!((w.pos.y - 0.45).abs() < 1e-3);
        w.go(&m, &[], "dining", vec3(1.0, 0.0, 2.0), None, Pace::Walk);
        assert_eq!(w.pos, vec3(1.0, 0.0, 2.0));
        assert!(w.arrived() && w.room() == "dining");
    }

    #[test]
    fn go_points_walks_a_straight_line_in_the_room() {
        let mut w = NavWalker::new();
        w.place(Vec3::ZERO, "r", None);
        w.go_points(&[vec3(0.0, 0.0, 4.6)], "r", vec3(0.0, 0.0, 4.6), None, Pace::Walk);
        let mut t = 0.0_f32;
        while w.is_walking() && t < 10.0 {
            w.step(0.05);
            t += 0.05;
        }
        assert!((t - 2.0).abs() < 0.2, "{t}");
        assert!(w.at_rest());
    }
}
