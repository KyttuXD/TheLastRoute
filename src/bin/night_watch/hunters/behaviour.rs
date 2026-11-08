//! Idle behaviour for living hunters (contract 9.1). Between sim moves a hunter stares at the nearest camera of her room,
//! goes and does a small task at a task spot, or just waits in her Godot pose. The choice is seeded and deterministic, the
//! durations and weights come from `data/behaviour.json`, and none of it reaches the sim:
//!
//! * a brain owns its own `Mulberry32` (seed ^ hash(hunter id)); it never draws from a mode's stream,
//! * a brain has no access to attack, door, power or pacer state, and no way to change them,
//! * the walker positions are the sim positions, so a hunter at a task spot is where the sim says she is; the mode keeps
//!   deciding WHEN she moves and WHERE (`Crowd::send`), and a sim move always wins over whatever she was doing.
//!
//! `Crowd` ties a `NavMap` (nav.rs) to one `NavWalker` and one `HunterBrain` per hunter. See the top of nav.rs for the
//! calling sequence. `ActorDriver` applies an agent to an `Actor` while the mode has no pose of its own for her.

#![allow(dead_code)]

use super::nav::{NavMap, NavWalker, Pace};
use crate::core::rng::Mulberry32;
use macroquad::prelude::Vec3;
use serde_json::Value;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::OnceLock;

pub const BEHAVIOUR_JSON: &str = include_str!("../data/behaviour.json");

// ------------------------------------------------------------------------------------------------ data

#[derive(Clone, Debug)]
pub struct StareDef {
    pub pose: String,
    pub speed: f32,
    pub pulse_hz: f32,
    pub pulse_strength: f32,
    pub subtitle: String,
}

#[derive(Clone, Debug)]
pub struct TaskDef {
    pub id: String,
    /// The pose clip to play at the spot (an id from night_watch_poses.json); None = the rig's own idle (or walk, for pacing).
    pub pose: Option<String>,
    pub speed: f32,
    pub label: String,
    /// Walk between the spot and its `pace_to` end while the task lasts.
    pub pace: bool,
}

/// One hunter's tunables (the default, with her overrides applied).
#[derive(Clone, Debug)]
pub struct Params {
    pub first_delay_s: (f32, f32),
    pub w_stare: f32,
    pub w_task: f32,
    pub w_wait: f32,
    pub stare_s: (f32, f32),
    pub task_s: (f32, f32),
    pub wait_s: (f32, f32),
    /// How many doorways away from her room a task spot may be (0 = her own room only).
    pub roam_hops: u32,
    /// task id -> weight, sorted by id
    pub tasks: Vec<(String, f32)>,
}

impl Params {
    fn fallback() -> Params {
        Params {
            first_delay_s: (2.0, 6.0),
            w_stare: 2.0,
            w_task: 3.0,
            w_wait: 3.0,
            stare_s: (3.0, 6.0),
            task_s: (5.0, 10.0),
            wait_s: (3.0, 8.0),
            roam_hops: 0,
            tasks: Vec::new(),
        }
    }

    fn apply(&mut self, v: &Value) {
        if let Some(p) = pair(v, "first_delay_s") {
            self.first_delay_s = p;
        }
        if let Some(p) = pair(v, "stare_s") {
            self.stare_s = p;
        }
        if let Some(p) = pair(v, "task_s") {
            self.task_s = p;
        }
        if let Some(p) = pair(v, "wait_s") {
            self.wait_s = p;
        }
        if let Some(h) = v.get("roam_hops").and_then(|h| h.as_u64()) {
            self.roam_hops = h as u32;
        }
        if let Some(w) = v.get("weights") {
            if let Some(x) = w.get("stare").and_then(|x| x.as_f64()) {
                self.w_stare = x as f32;
            }
            if let Some(x) = w.get("task").and_then(|x| x.as_f64()) {
                self.w_task = x as f32;
            }
            if let Some(x) = w.get("wait").and_then(|x| x.as_f64()) {
                self.w_wait = x as f32;
            }
        }
        if let Some(t) = v.get("tasks").and_then(|t| t.as_object()) {
            for (k, x) in t {
                let w = x.as_f64().unwrap_or(0.0) as f32;
                match self.tasks.iter_mut().find(|e| e.0 == *k) {
                    Some(e) => e.1 = w,
                    None => self.tasks.push((k.clone(), w)),
                }
            }
            self.tasks.sort_by(|a, b| a.0.cmp(&b.0));
        }
    }
}

fn pair(v: &Value, key: &str) -> Option<(f32, f32)> {
    let a = v.get(key)?.as_array()?;
    if a.len() < 2 {
        return None;
    }
    let (x, y) = (a[0].as_f64()? as f32, a[1].as_f64()? as f32);
    Some((x.min(y), x.max(y)))
}

#[derive(Clone, Debug)]
pub struct BehaviourData {
    pub stare: StareDef,
    pub tasks: BTreeMap<String, TaskDef>,
    pub default: Params,
    pub hunters: BTreeMap<String, Params>,
}

impl BehaviourData {
    pub fn parse(src: &str) -> Result<BehaviourData, String> {
        let root: Value = serde_json::from_str(src).map_err(|e| format!("behaviour json: {e}"))?;
        let s = root.get("stare").ok_or_else(|| "behaviour json: no stare".to_string())?;
        let stare = StareDef {
            pose: s.get("pose").and_then(|x| x.as_str()).unwrap_or("stare_camera").to_string(),
            speed: s.get("speed").and_then(|x| x.as_f64()).unwrap_or(1.0) as f32,
            pulse_hz: s.get("pulse_hz").and_then(|x| x.as_f64()).unwrap_or(0.7) as f32,
            pulse_strength: s.get("pulse_strength").and_then(|x| x.as_f64()).unwrap_or(0.35) as f32,
            subtitle: s.get("subtitle").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        };
        let mut tasks = BTreeMap::new();
        if let Some(t) = root.get("tasks").and_then(|t| t.as_object()) {
            for (id, d) in t {
                tasks.insert(
                    id.clone(),
                    TaskDef {
                        id: id.clone(),
                        pose: d.get("pose").and_then(|x| x.as_str()).map(|x| x.to_string()),
                        speed: d.get("speed").and_then(|x| x.as_f64()).unwrap_or(1.0) as f32,
                        label: d.get("label").and_then(|x| x.as_str()).unwrap_or(id).to_string(),
                        pace: d.get("pace").and_then(|x| x.as_bool()).unwrap_or(false),
                    },
                );
            }
        }
        let mut default = Params::fallback();
        if let Some(d) = root.get("default") {
            default.apply(d);
        }
        let mut hunters = BTreeMap::new();
        if let Some(h) = root.get("hunters").and_then(|h| h.as_object()) {
            for (id, v) in h {
                let mut p = default.clone();
                p.apply(v);
                hunters.insert(id.clone(), p);
            }
        }
        Ok(BehaviourData { stare, tasks, default, hunters })
    }

    /// The shipped file. If it ever fails to parse, hunters just wait (no stare pose, no tasks).
    pub fn embedded() -> Rc<BehaviourData> {
        static DATA: OnceLock<BehaviourData> = OnceLock::new();
        let d = DATA.get_or_init(|| {
            BehaviourData::parse(BEHAVIOUR_JSON).unwrap_or_else(|_| {
                let mut p = Params::fallback();
                p.w_stare = 0.0;
                p.w_task = 0.0;
                BehaviourData {
                    stare: StareDef { pose: "stare_camera".to_string(), speed: 1.0, pulse_hz: 0.7, pulse_strength: 0.35, subtitle: String::new() },
                    tasks: BTreeMap::new(),
                    default: p,
                    hunters: BTreeMap::new(),
                }
            })
        });
        Rc::new(d.clone())
    }

    pub fn params_for(&self, hunter: &str) -> &Params {
        self.hunters.get(hunter).unwrap_or(&self.default)
    }

    /// Problems in the data (empty = fine). `pose_ids` are the ids of night_watch_poses.json (`poses::pose_ids`).
    pub fn validate(&self, pose_ids: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        if !pose_ids.iter().any(|p| *p == self.stare.pose) {
            out.push(format!("stare pose {} is not a pose", self.stare.pose));
        }
        for t in self.tasks.values() {
            if let Some(p) = &t.pose {
                if !pose_ids.iter().any(|x| x == p) {
                    out.push(format!("task {} uses unknown pose {}", t.id, p));
                }
            }
        }
        let mut check = |who: &str, p: &Params| {
            for (name, r) in [("first_delay_s", p.first_delay_s), ("stare_s", p.stare_s), ("task_s", p.task_s), ("wait_s", p.wait_s)] {
                if r.0 < 0.0 || r.1 < r.0 {
                    out.push(format!("{who}: bad range {name}"));
                }
            }
            if p.w_stare < 0.0 || p.w_task < 0.0 || p.w_wait < 0.0 {
                out.push(format!("{who}: negative weight"));
            }
            for (t, w) in &p.tasks {
                if *w < 0.0 {
                    out.push(format!("{who}: negative weight for task {t}"));
                }
                if !self.tasks.contains_key(t) {
                    out.push(format!("{who}: unknown task {t}"));
                }
            }
        };
        check("default", &self.default);
        for (id, p) in &self.hunters {
            check(id, p);
        }
        out
    }
}

// ------------------------------------------------------------------------------------------------ events

#[derive(Clone, Debug, PartialEq)]
pub enum BehaviourEvent {
    /// The first stare of the night (once per night, across all hunters): the HUD shows `subtitle`.
    FirstStare { hunter: String, room: String, cam: String, subtitle: String },
    StareStart { hunter: String, room: String, cam: String },
    StareEnd { hunter: String, room: String },
    TaskStart { hunter: String, task: String, label: String, room: String },
    TaskEnd { hunter: String, task: String, room: String },
}

// ------------------------------------------------------------------------------------------------ one brain

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Activity {
    /// The sim owns her (she is on a sim move, the brain is off, or she has not settled yet).
    Held,
    /// Standing at her stand spot (the "Wait" behaviour; also the pause between behaviours).
    Waiting,
    Staring,
    GoingToTask,
    Working,
    Returning,
}

/// What the brain wants shown right now.
#[derive(Clone, Copy, Debug)]
pub struct BrainView<'a> {
    /// A pose clip id to play (stare, task); None = the rig's own idle / walk / sprint.
    pub pose: Option<&'a str>,
    pub pose_speed: f32,
    pub activity: Activity,
    /// A short description of the task ("wipes the counter"), while working.
    pub label: Option<&'a str>,
}

#[derive(Clone, Debug)]
struct Home {
    pos: Vec3,
    room: String,
    look: Option<Vec3>,
}

fn id_hash(id: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

fn pick_weighted(u: f64, weights: &[f32]) -> Option<usize> {
    let total: f32 = weights.iter().map(|w| w.max(0.0)).sum();
    if total <= 0.0 {
        return None;
    }
    let mut t = u as f32 * total;
    let mut last = None;
    for (i, w) in weights.iter().enumerate() {
        let w = w.max(0.0);
        if w <= 0.0 {
            continue;
        }
        last = Some(i);
        if t < w {
            return Some(i);
        }
        t -= w;
    }
    last
}

#[derive(Clone, Debug)]
pub struct HunterBrain {
    pub id: String,
    data: Rc<BehaviourData>,
    params: Params,
    rng: Mulberry32,
    act: Activity,
    left: f32,
    enabled: bool,
    home: Option<Home>,
    task: Option<String>,
    spot: Option<String>,
    pace_out: bool,
    pace_ends: (Vec3, Vec3),
    pose: Option<String>,
    pose_speed: f32,
    label: Option<String>,
    stare_t: f32,
    stare_len: f32,
    stare_cam: String,
    pending: Vec<BehaviourEvent>,
}

impl HunterBrain {
    pub fn new(id: &str, seed: u32, data: Rc<BehaviourData>) -> HunterBrain {
        let params = data.params_for(id).clone();
        HunterBrain {
            id: id.to_string(),
            data,
            params,
            rng: Mulberry32::new(seed ^ id_hash(id)),
            act: Activity::Held,
            left: 0.0,
            enabled: true,
            home: None,
            task: None,
            spot: None,
            pace_out: true,
            pace_ends: (Vec3::ZERO, Vec3::ZERO),
            pose: None,
            pose_speed: 1.0,
            label: None,
            stare_t: 0.0,
            stare_len: 0.0,
            stare_cam: String::new(),
            pending: Vec::new(),
        }
    }

    pub fn activity(&self) -> Activity {
        self.act
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn task(&self) -> Option<&str> {
        self.task.as_deref()
    }

    /// Where she stands when idle (set when she settles after a sim move).
    pub fn home_pos(&self) -> Option<Vec3> {
        self.home.as_ref().map(|h| h.pos)
    }

    pub fn view(&self) -> BrainView<'_> {
        BrainView { pose: self.pose.as_deref(), pose_speed: self.pose_speed, activity: self.act, label: self.label.as_deref() }
    }

    /// Switch idle behaviour off (the sim has her at a door, attacking, ...) or back on.
    pub fn set_enabled(&mut self, on: bool) {
        if on == self.enabled {
            return;
        }
        self.enabled = on;
        if !on {
            self.hold();
        }
    }

    /// The sim has moved her (or is about to): drop whatever she was doing. She settles again when she has arrived.
    pub fn moved(&mut self) {
        self.hold();
    }

    fn hold(&mut self) {
        self.end_activity();
        self.act = Activity::Held;
        self.pose = None;
        self.label = None;
        self.left = 0.0;
    }

    /// Emits the end event of a stare or a task that is being cut short.
    fn end_activity(&mut self) {
        let room = self.home.as_ref().map(|h| h.room.clone()).unwrap_or_default();
        match self.act {
            Activity::Staring => self.pending.push(BehaviourEvent::StareEnd { hunter: self.id.clone(), room }),
            Activity::Working | Activity::GoingToTask => {
                if let Some(t) = self.task.take() {
                    if self.act == Activity::Working {
                        self.pending.push(BehaviourEvent::TaskEnd { hunter: self.id.clone(), task: t, room });
                    }
                }
            }
            _ => {}
        }
        self.task = None;
        self.spot = None;
    }

    /// Soft static pulse strength for her room's feed while she stares: 0 when she is not staring.
    pub fn stare_pulse(&self) -> f32 {
        if self.act != Activity::Staring {
            return 0.0;
        }
        let fade_in = (self.stare_t / 0.6).clamp(0.0, 1.0);
        let fade_out = ((self.stare_len - self.stare_t) / 0.6).clamp(0.0, 1.0);
        let wave = 0.5 + 0.5 * (std::f32::consts::TAU * self.data.stare.pulse_hz * self.stare_t).sin();
        self.data.stare.pulse_strength * fade_in.min(fade_out) * wave
    }

    fn draw(&mut self, r: (f32, f32)) -> f32 {
        r.0 + (r.1 - r.0) * self.rng.next() as f32
    }

    fn settle(&mut self, w: &NavWalker) {
        self.home = Some(Home { pos: w.final_pos, room: w.room().to_string(), look: w.look });
        self.act = Activity::Waiting;
        self.pose = None;
        self.label = None;
        self.left = self.draw(self.params.first_delay_s);
    }

    /// Advances the brain by `dt` seconds. Call after `walker.step(dt)`. Returns the events of this tick.
    pub fn tick(&mut self, dt: f32, nav: &NavMap, w: &mut NavWalker) -> Vec<BehaviourEvent> {
        if !self.enabled {
            if self.act != Activity::Held {
                self.hold();
            }
            return std::mem::take(&mut self.pending);
        }
        match self.act {
            Activity::Held => {
                if !w.is_walking() && w.at_rest() {
                    self.settle(w);
                }
            }
            Activity::Waiting => {
                if w.is_walking() {
                    // someone sent her without telling the brain
                    self.hold();
                } else {
                    self.left -= dt;
                    if self.left <= 0.0 {
                        self.decide(nav, w);
                    }
                }
            }
            Activity::Staring => {
                self.stare_t += dt;
                self.left -= dt;
                if self.left <= 0.0 {
                    self.finish_stare(w);
                }
            }
            Activity::GoingToTask => {
                if !w.is_walking() && w.at_rest() {
                    self.start_work(nav, w);
                }
            }
            Activity::Working => {
                self.left -= dt;
                if self.left <= 0.0 {
                    self.finish_task(nav, w);
                } else if self.is_pacing() && !w.is_walking() && w.at_rest() {
                    let target = if self.pace_out { self.pace_ends.1 } else { self.pace_ends.0 };
                    self.pace_out = !self.pace_out;
                    let room = w.room().to_string();
                    let look = nav.node(self.spot.as_deref().unwrap_or("")).and_then(|n| n.look);
                    w.go_points(&[target], &room, target, look, Pace::Walk);
                }
            }
            Activity::Returning => {
                if !w.is_walking() && w.at_rest() {
                    self.act = Activity::Waiting;
                    self.left = self.draw(self.params.first_delay_s);
                }
            }
        }
        std::mem::take(&mut self.pending)
    }

    fn is_pacing(&self) -> bool {
        match &self.task {
            Some(t) => self.data.tasks.get(t).map_or(false, |d| d.pace),
            None => false,
        }
    }

    fn wait(&mut self) {
        self.act = Activity::Waiting;
        self.pose = None;
        self.left = self.draw(self.params.wait_s);
    }

    fn decide(&mut self, nav: &NavMap, w: &mut NavWalker) {
        let weights = [self.params.w_stare, self.params.w_task, self.params.w_wait];
        let u = self.rng.next();
        match pick_weighted(u, &weights) {
            Some(0) => self.start_stare(nav, w),
            Some(1) => self.start_task(nav, w),
            _ => self.wait(),
        }
    }

    fn start_stare(&mut self, nav: &NavMap, w: &mut NavWalker) {
        let room = w.room().to_string();
        let cam = match nav.nearest_camera(&room, w.pos) {
            Some(c) => (c.key.clone(), c.p),
            None => {
                self.wait();
                return;
            }
        };
        let len = self.draw(self.params.stare_s);
        self.act = Activity::Staring;
        self.left = len;
        self.stare_len = len;
        self.stare_t = 0.0;
        self.stare_cam = cam.0.clone();
        self.pose = Some(self.data.stare.pose.clone());
        self.pose_speed = self.data.stare.speed;
        w.look = Some(cam.1);
        self.pending.push(BehaviourEvent::StareStart { hunter: self.id.clone(), room, cam: cam.0 });
    }

    fn finish_stare(&mut self, w: &mut NavWalker) {
        let room = w.room().to_string();
        w.look = self.home.as_ref().and_then(|h| h.look);
        self.pose = None;
        self.pending.push(BehaviourEvent::StareEnd { hunter: self.id.clone(), room });
        self.wait();
    }

    fn start_task(&mut self, nav: &NavMap, w: &mut NavWalker) {
        let home_room = match &self.home {
            Some(h) => h.room.clone(),
            None => {
                self.wait();
                return;
            }
        };
        let rooms = nav.rooms_within(&home_room, self.params.roam_hops);
        // tasks she may do that have a spot in reach, in id order
        let mut cands: Vec<(String, f32)> = Vec::new();
        for (t, wt) in &self.params.tasks {
            if *wt > 0.0 && self.data.tasks.contains_key(t) && rooms.iter().any(|r| !nav.task_spots(r, Some(t)).is_empty()) {
                cands.push((t.clone(), *wt));
            }
        }
        let weights: Vec<f32> = cands.iter().map(|c| c.1).collect();
        let u = self.rng.next();
        let task = match pick_weighted(u, &weights) {
            Some(i) => cands[i].0.clone(),
            None => {
                self.wait();
                return;
            }
        };
        let mut spots: Vec<String> = Vec::new();
        for r in &rooms {
            for s in nav.task_spots(r, Some(&task)) {
                spots.push(s.key.clone());
            }
        }
        let pick = self.rng.pick(spots.len());
        let key = spots[pick].clone();
        let legs = nav.route_from(w.pos, &key);
        let node = match nav.node(&key) {
            Some(n) if !legs.is_empty() => n,
            _ => {
                self.wait();
                return;
            }
        };
        w.go(nav, &legs, &node.room, node.p, node.look, Pace::Walk);
        self.act = Activity::GoingToTask;
        self.task = Some(task);
        self.spot = Some(key);
        self.pose = None;
        self.label = None;
    }

    fn start_work(&mut self, nav: &NavMap, w: &mut NavWalker) {
        let task = self.task.clone().unwrap_or_default();
        let def = match self.data.tasks.get(&task) {
            Some(d) => d.clone(),
            None => {
                self.go_home(nav, w);
                return;
            }
        };
        let node = self.spot.as_deref().and_then(|k| nav.node(k));
        if def.pace {
            match node.and_then(|n| n.pace_to.map(|e| (n.p, e))) {
                Some(ends) => {
                    self.pace_ends = ends;
                    self.pace_out = true;
                }
                None => {
                    self.go_home(nav, w);
                    return;
                }
            }
        }
        self.act = Activity::Working;
        self.left = self.draw(self.params.task_s);
        self.pose = def.pose.clone();
        self.pose_speed = def.speed;
        self.label = Some(def.label.clone());
        self.pending.push(BehaviourEvent::TaskStart { hunter: self.id.clone(), task, label: def.label, room: w.room().to_string() });
    }

    fn finish_task(&mut self, nav: &NavMap, w: &mut NavWalker) {
        let task = self.task.take().unwrap_or_default();
        self.pending.push(BehaviourEvent::TaskEnd { hunter: self.id.clone(), task, room: w.room().to_string() });
        self.go_home(nav, w);
    }

    fn go_home(&mut self, nav: &NavMap, w: &mut NavWalker) {
        self.pose = None;
        self.label = None;
        self.task = None;
        self.spot = None;
        let home = match &self.home {
            Some(h) => h.clone(),
            None => {
                self.wait();
                return;
            }
        };
        let legs = match nav.nearest_walkable(home.pos) {
            Some(n) => nav.route_from(w.pos, &n.key.clone()),
            None => Vec::new(),
        };
        w.go(nav, &legs, &home.room, home.pos, home.look, Pace::Walk);
        self.act = Activity::Returning;
    }
}

// ------------------------------------------------------------------------------------------------ the crowd

/// One hunter: where she is (walker) and what she is up to (brain).
#[derive(Clone, Debug)]
pub struct Agent {
    pub id: String,
    pub walker: NavWalker,
    pub brain: HunterBrain,
}

impl Agent {
    /// What to show on her actor: a pose clip while the brain owns the pose and she stands still, else walk / sprint / idle.
    pub fn drive_command(&self) -> DriveCommand {
        let v = self.brain.view();
        match v.pose {
            Some(p) if !self.walker.moving => DriveCommand::Pose { pose: p.to_string(), speed: v.pose_speed },
            _ => DriveCommand::Loco { moving: self.walker.moving, running: self.walker.running },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DriveCommand {
    Pose { pose: String, speed: f32 },
    Loco { moving: bool, running: bool },
}

pub struct Crowd {
    nav: Rc<NavMap>,
    data: Rc<BehaviourData>,
    seed: u32,
    agents: Vec<Agent>,
    first_stare_done: bool,
    /// Hunters re-placed by `add_at` while already standing somewhere (a direct write, not a walk). Tests watch this.
    pub replaced: u32,
}

impl Crowd {
    pub fn new(nav: Rc<NavMap>, data: Rc<BehaviourData>, seed: u32) -> Crowd {
        Crowd { nav, data, seed, agents: Vec::new(), first_stare_done: false, replaced: 0 }
    }

    pub fn nav(&self) -> &NavMap {
        &self.nav
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn agent(&self, id: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == id)
    }

    pub fn agent_mut(&mut self, id: &str) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    /// Adds (or re-places) a hunter standing at a room's stand slot, facing that spot's look target.
    pub fn add(&mut self, id: &str, room: &str, slot: usize) -> bool {
        let (pos, look) = match self.nav.stand_node(room, slot) {
            Some(n) => (n.p, n.look),
            None => return false,
        };
        self.add_at(id, room, pos, look);
        true
    }

    /// Adds (or re-places) a hunter at an explicit spot.
    pub fn add_at(&mut self, id: &str, room: &str, pos: Vec3, look: Option<Vec3>) {
        if let Some(a) = self.agents.iter_mut().find(|a| a.id == id) {
            self.replaced += 1;
            a.walker.place(pos, room, look);
            a.brain.moved();
            return;
        }
        let mut walker = NavWalker::new();
        walker.place(pos, room, look);
        let brain = HunterBrain::new(id, self.seed, self.data.clone());
        self.agents.push(Agent { id: id.to_string(), walker, brain });
    }

    pub fn remove(&mut self, id: &str) {
        self.agents.retain(|a| a.id != id);
    }

    /// Puts her on a room's stand slot at once (manual clock, level restart).
    pub fn teleport(&mut self, id: &str, room: &str, slot: usize) -> bool {
        self.add(id, room, slot)
    }

    /// The sim moved her to `room`: A* from where she stands to that room's stand slot, walked at `pace`.
    /// Same room: she goes to the slot (the sim wants her there). False when the room has no stand node or id is unknown.
    pub fn send(&mut self, id: &str, room: &str, slot: usize, pace: Pace) -> bool {
        let (key, pos, look) = match self.nav.stand_node(room, slot) {
            Some(n) => (n.key.clone(), n.p, n.look),
            None => return false,
        };
        let nav = self.nav.clone();
        match self.agents.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                a.brain.moved();
                let legs = nav.route_from(a.walker.pos, &key);
                a.walker.go(&nav, &legs, room, pos, look, pace);
                true
            }
            None => false,
        }
    }

    /// Like `send`, with a computed final spot (the office door stand creeps closer): walks to the room's hub node, then
    /// to `final_pos`, then turns to `look`.
    pub fn send_to(&mut self, id: &str, room: &str, final_pos: Vec3, look: Option<Vec3>, pace: Pace) -> bool {
        let key = match self.nav.hub_key(room) {
            Some(k) => k,
            None => return false,
        };
        let nav = self.nav.clone();
        match self.agents.iter_mut().find(|a| a.id == id) {
            Some(a) => {
                a.brain.moved();
                let legs = nav.route_from(a.walker.pos, &key);
                a.walker.go(&nav, &legs, room, final_pos, look, pace);
                true
            }
            None => false,
        }
    }

    /// Moves her final spot and look target without a new trip (she shuffles there once she has arrived).
    pub fn set_final(&mut self, id: &str, pos: Vec3, look: Option<Vec3>) {
        if let Some(a) = self.agent_mut(id) {
            a.walker.final_pos = pos;
            a.walker.look = look;
        }
    }

    /// Switch a hunter's idle behaviour on or off (off while the sim has her at a door or attacking).
    pub fn set_enabled(&mut self, id: &str, on: bool) {
        if let Some(a) = self.agent_mut(id) {
            a.brain.set_enabled(on);
        }
    }

    /// A new night: the first-stare event can fire again and every brain starts over.
    pub fn new_night(&mut self) {
        self.first_stare_done = false;
        for a in &mut self.agents {
            a.brain.moved();
        }
    }

    /// Advance every hunter by `dt` seconds. Events come back in hunter order.
    pub fn tick(&mut self, dt: f32) -> Vec<BehaviourEvent> {
        let nav = self.nav.clone();
        let mut out = Vec::new();
        for a in &mut self.agents {
            a.walker.step(dt);
            let evs = a.brain.tick(dt, &nav, &mut a.walker);
            for e in evs {
                if let BehaviourEvent::StareStart { hunter, room, cam } = &e {
                    if !self.first_stare_done {
                        self.first_stare_done = true;
                        out.push(BehaviourEvent::FirstStare {
                            hunter: hunter.clone(),
                            room: room.clone(),
                            cam: cam.clone(),
                            subtitle: self.data.stare.subtitle.clone(),
                        });
                    }
                }
                out.push(e);
            }
        }
        out
    }

    /// Is this hunter in the view of `room`'s camera? By position when she is inside a room rectangle (the lane in front of
    /// the room counts), else by the room she is walking into.
    pub fn visible_in(&self, room: &str) -> Vec<&Agent> {
        self.agents.iter().filter(|a| self.agent_in(a, room)).collect()
    }

    fn agent_in(&self, a: &Agent, room: &str) -> bool {
        match self.nav.room_at(a.walker.pos) {
            Some(r) => r == room,
            None => a.walker.room() == room,
        }
    }

    /// 0..1 soft static pulse for `room`'s feed while a hunter in view stares into its camera.
    pub fn stare_pulse(&self, room: &str) -> f32 {
        let mut best = 0.0f32;
        for a in &self.agents {
            if a.brain.activity() == Activity::Staring && self.agent_in(a, room) {
                best = best.max(a.brain.stare_pulse());
            }
        }
        best
    }

    /// True when any hunter in view of `room`'s camera is staring into it.
    pub fn staring_at(&self, room: &str) -> bool {
        self.agents.iter().any(|a| a.brain.activity() == Activity::Staring && self.agent_in(a, room))
    }
}

// ------------------------------------------------------------------------------------------------ actor driving

/// Applies an agent to an `Actor` (position, yaw, pose or walk / idle clip), calling the actor only when something changed.
/// Use it while the sim has no pose of its own for the hunter; when it does (at a door, attacking), set the pose state on
/// the actor yourself and skip `drive`.
#[derive(Clone, Debug, Default)]
pub struct ActorDriver {
    last: Option<DriveCommand>,
}

impl ActorDriver {
    pub fn new() -> ActorDriver {
        ActorDriver::default()
    }

    /// Forget the last command (call after the mode used `set_pose_state` on the actor).
    pub fn reset(&mut self) {
        self.last = None;
    }

    pub fn drive(&mut self, agent: &Agent, actor: &mut super::rig::Actor) {
        actor.pos = agent.walker.pos;
        actor.yaw = agent.walker.yaw;
        let cmd = agent.drive_command();
        if self.last.as_ref() == Some(&cmd) {
            return;
        }
        match &cmd {
            DriveCommand::Pose { pose, speed } => {
                if !actor.play_pose(pose, *speed) {
                    actor.stop_pose();
                    actor.set_locomotion(false, false);
                }
            }
            DriveCommand::Loco { moving, running } => {
                if matches!(self.last, Some(DriveCommand::Pose { .. })) || self.last.is_none() {
                    actor.stop_pose();
                }
                actor.set_locomotion(*moving, *running);
            }
        }
        self.last = Some(cmd);
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::rng::Pacer;
    use macroquad::prelude::vec3;
    use crate::hunters::nav::angle_diff;
    use crate::hunters::place::yaw_toward;

    const POSES: &str = include_str!("../data/poses/night_watch_poses.json");

    fn crowd(seed: u32) -> Crowd {
        Crowd::new(Rc::new(NavMap::classic()), BehaviourData::embedded(), seed)
    }

    /// Ticks until `pred` is true or `max_s` seconds pass; returns the elapsed seconds, or None on timeout.
    fn until(c: &mut Crowd, max_s: f32, mut pred: impl FnMut(&mut Crowd, &[BehaviourEvent]) -> bool) -> Option<f32> {
        let dt = 0.05;
        let mut t = 0.0;
        while t < max_s {
            let ev = c.tick(dt);
            t += dt;
            if pred(c, &ev) {
                return Some(t);
            }
        }
        None
    }

    #[test]
    fn data_parses_merges_overrides_and_validates() {
        let d = BehaviourData::parse(BEHAVIOUR_JSON).unwrap_or_else(|e| panic!("{e}"));
        let ids = crate::hunters::poses::pose_ids(POSES);
        let problems = d.validate(&ids);
        assert!(problems.is_empty(), "{problems:?}");
        // every task in the brief exists
        for t in ["wipe_counter", "check_locker", "sort_boxes", "tap_terminal", "sit", "pace"] {
            assert!(d.tasks.contains_key(t), "{t}");
        }
        assert!(d.tasks["pace"].pace && d.tasks["pace"].pose.is_none());
        // nyxie overrides the weights but keeps the default wait range
        let n = d.params_for("nyxie");
        assert_eq!((n.w_stare, n.w_task, n.w_wait), (6.0, 0.0, 2.0));
        assert_eq!(n.wait_s, d.default.wait_s);
        assert_eq!(n.stare_s, (5.0, 10.0));
        // an unknown hunter gets the default
        assert_eq!(d.params_for("someone").w_task, d.default.w_task);
        // ravelle keeps the default tasks she did not override
        let r = d.params_for("ravelle");
        assert_eq!(r.tasks.iter().find(|t| t.0 == "pace").map(|t| t.1), Some(0.0));
        assert_eq!(r.tasks.iter().find(|t| t.0 == "wipe_counter").map(|t| t.1), Some(3.0));
        assert!(BehaviourData::parse("{").is_err());
        assert!(BehaviourData::parse("{}").is_err());
    }

    #[test]
    fn every_classic_task_spot_has_a_task_and_a_pose_in_the_pose_file() {
        let nav = NavMap::classic();
        let d = BehaviourData::embedded();
        let ids = crate::hunters::poses::pose_ids(POSES);
        for n in nav.nodes() {
            if let Some(t) = &n.task {
                let def = d.tasks.get(t).unwrap_or_else(|| panic!("task {t} at {}", n.key));
                if let Some(p) = &def.pose {
                    assert!(ids.contains(p), "{p}");
                }
                if def.pace {
                    assert!(n.pace_to.is_some(), "{} needs pace_to", n.key);
                }
            }
        }
        // the stare pose exists, and the stare targets exist in every classic room
        assert!(ids.contains(&d.stare.pose));
        for r in ["stage", "dining", "backstage", "westhall", "closet", "kitchen", "easthall", "corner"] {
            assert_eq!(nav.cameras(r).len(), 1);
        }
    }

    #[test]
    fn pick_weighted_follows_the_weights() {
        assert_eq!(pick_weighted(0.0, &[1.0, 1.0]), Some(0));
        assert_eq!(pick_weighted(0.49, &[1.0, 1.0]), Some(0));
        assert_eq!(pick_weighted(0.5, &[1.0, 1.0]), Some(1));
        assert_eq!(pick_weighted(0.99, &[0.0, 3.0, 0.0]), Some(1));
        assert_eq!(pick_weighted(0.0, &[0.0, 0.0]), None);
        assert_eq!(pick_weighted(0.9999999, &[1.0, 0.0]), Some(0));
    }

    fn run_digest(seed: u32) -> (Vec<String>, Vec<(i32, i32, i32)>) {
        let mut c = crowd(seed);
        c.add("dovelle", "stage", 0);
        c.add("ravelle", "kitchen", 0);
        c.add("aki", "dining", 0);
        let mut log = Vec::new();
        let mut path = Vec::new();
        for i in 0..2400 {
            let ev = c.tick(0.05);
            for e in ev {
                log.push(format!("{}:{:?}", i, e));
            }
            if i % 20 == 0 {
                for a in c.agents() {
                    path.push(((a.walker.pos.x * 100.0) as i32, (a.walker.pos.z * 100.0) as i32, (a.walker.yaw * 100.0) as i32));
                }
            }
        }
        (log, path)
    }

    #[test]
    fn behaviour_is_deterministic_for_a_seed() {
        let a = run_digest(1234);
        let b = run_digest(1234);
        assert_eq!(a, b);
        assert!(!a.0.is_empty(), "two minutes of idling produced no events");
        let other = run_digest(99);
        assert_ne!(a, other);
    }

    #[test]
    fn a_hunter_stares_at_the_nearest_camera_and_goes_back_to_her_look() {
        let mut c = crowd(7);
        let nav = c.nav.clone();
        let pos = nav.stand_node("stage", 0).map(|n| n.p).unwrap_or_default();
        let away = pos + vec3(0.0, 0.0, -6.0); // she is looking at the back wall
        c.add_at("nyxie", "stage", pos, Some(away));
        let cam = nav.cameras("stage")[0].p;
        let t = until(&mut c, 600.0, |c, _| c.agent("nyxie").map_or(false, |a| a.brain.activity() == Activity::Staring));
        assert!(t.is_some(), "never stared");
        // half a second later she faces the camera and wears the stare pose
        until(&mut c, 1.0, |_, _| false);
        let a = c.agent("nyxie").unwrap();
        assert_eq!(a.brain.activity(), Activity::Staring);
        assert_eq!(a.brain.view().pose, Some("stare_camera"));
        let want = yaw_toward(a.walker.pos, cam);
        assert!(angle_diff(a.walker.yaw, want).abs() < 1e-3, "{} vs {}", a.walker.yaw, want);
        assert!(c.staring_at("stage"));
        assert!(!c.staring_at("dining"));
        let pulse = c.stare_pulse("stage");
        assert!(pulse >= 0.0 && pulse <= BehaviourData::embedded().stare.pulse_strength + 1e-6);
        // when it ends she turns back to where she was looking and the pose is released
        let ended = until(&mut c, 30.0, |c, _| c.agent("nyxie").map_or(false, |a| a.brain.activity() != Activity::Staring));
        assert!(ended.is_some());
        until(&mut c, 1.0, |_, _| false);
        let a = c.agent("nyxie").unwrap();
        let back = yaw_toward(a.walker.pos, away);
        if a.brain.activity() == Activity::Waiting {
            assert!(angle_diff(a.walker.yaw, back).abs() < 1e-3 || a.brain.view().pose.is_some());
        }
        assert_eq!(a.walker.pos, pos); // she never left her spot
    }

    #[test]
    fn the_first_stare_of_the_night_fires_once_and_again_after_a_new_night() {
        let mut c = crowd(21);
        c.add("nyxie", "stage", 0);
        c.add("kathy", "dining", 0); // never stares
        let mut firsts = 0;
        let mut starts = 0;
        let mut subtitle = String::new();
        until(&mut c, 400.0, |_, ev| {
            for e in ev {
                match e {
                    BehaviourEvent::FirstStare { subtitle: s, .. } => {
                        firsts += 1;
                        subtitle = s.clone();
                    }
                    BehaviourEvent::StareStart { .. } => starts += 1,
                    _ => {}
                }
            }
            false
        });
        assert_eq!(firsts, 1);
        assert!(starts >= 2, "{starts}");
        assert!(!subtitle.is_empty());
        c.new_night();
        let mut again = 0;
        until(&mut c, 400.0, |_, ev| {
            again += ev.iter().filter(|e| matches!(e, BehaviourEvent::FirstStare { .. })).count();
            false
        });
        assert_eq!(again, 1);
    }

    #[test]
    fn a_hunter_walks_to_a_task_spot_does_the_task_and_comes_home() {
        let mut c = crowd(5);
        c.add("ravelle", "kitchen", 0);
        let nav = c.nav.clone();
        let home = c.agent("ravelle").unwrap().walker.pos;
        let mut started: Option<(String, String)> = None;
        let t = until(&mut c, 600.0, |_, ev| {
            for e in ev {
                if let BehaviourEvent::TaskStart { task, label, .. } = e {
                    started = Some((task.clone(), label.clone()));
                }
            }
            started.is_some()
        });
        assert!(t.is_some(), "no task in ten minutes");
        let (task, label) = started.unwrap_or_default();
        assert!(task == "wipe_counter" || task == "tap_terminal", "{task}");
        assert!(!label.is_empty());
        let a = c.agent("ravelle").unwrap();
        // she stands on a spot for that task in her own room, wearing the task's pose
        let spots = nav.task_spots("kitchen", Some(&task));
        assert!(spots.iter().any(|s| s.p.distance(a.walker.pos) < 0.05));
        assert_eq!(a.brain.activity(), Activity::Working);
        let want_pose = BehaviourData::embedded().tasks[&task].pose.clone();
        assert_eq!(a.brain.view().pose.map(|p| p.to_string()), want_pose);
        assert_eq!(a.walker.room(), "kitchen");
        // and later she is back at her stand spot, no pose
        let mut ended = false;
        until(&mut c, 120.0, |_, ev| {
            ended |= ev.iter().any(|e| matches!(e, BehaviourEvent::TaskEnd { .. }));
            ended
        });
        assert!(ended);
        until(&mut c, 60.0, |c, _| c.agent("ravelle").map_or(false, |a| a.brain.activity() == Activity::Waiting));
        let a = c.agent("ravelle").unwrap();
        assert!(a.walker.pos.distance(home) < 0.05);
        assert_eq!(a.brain.view().pose, None);
    }

    const PACE_ONLY: &str = r#"{
        "stare": {"pose": "stare_camera"},
        "tasks": {"pace": {"pose": null, "pace": true, "label": "paces"}},
        "default": {"first_delay_s": [0.5, 0.5], "weights": {"stare": 0, "task": 1, "wait": 0}, "task_s": [20, 20], "wait_s": [1, 1],
                    "tasks": {"pace": 1}}
    }"#;

    #[test]
    fn pacing_walks_between_the_two_ends_of_the_spot() {
        let d = Rc::new(BehaviourData::parse(PACE_ONLY).unwrap_or_else(|e| panic!("{e}")));
        let mut c = Crowd::new(Rc::new(NavMap::classic()), d, 3);
        c.add("ilyra", "stage", 0);
        let mut working = false;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut moving_seen = false;
        until(&mut c, 40.0, |c, _| {
            let a = c.agent("ilyra").unwrap();
            if a.brain.activity() == Activity::Working {
                working = true;
                lo = lo.min(a.walker.pos.x);
                hi = hi.max(a.walker.pos.x);
                moving_seen |= a.walker.moving;
                assert_eq!(a.brain.view().pose, None); // pacing uses the rig's walk clip
            }
            false
        });
        assert!(working);
        assert!(moving_seen);
        assert!(hi - lo > 2.5, "{lo} .. {hi}"); // the stage pace spot is 2.8 m wide
    }

    #[test]
    fn a_sim_move_cancels_whatever_she_was_doing_and_she_settles_in_the_new_room() {
        let mut c = crowd(5);
        c.add("ravelle", "kitchen", 0);
        until(&mut c, 600.0, |c, _| c.agent("ravelle").map_or(false, |a| a.brain.activity() == Activity::Working));
        assert!(c.send("ravelle", "dining", 0, Pace::Budget(3.0)));
        assert_eq!(c.agent("ravelle").unwrap().brain.activity(), Activity::Held);
        assert_eq!(c.agent("ravelle").unwrap().brain.view().pose, None);
        // the TaskEnd of the cut-short task is reported on the next tick
        let ev = c.tick(0.05);
        assert!(ev.iter().any(|e| matches!(e, BehaviourEvent::TaskEnd { .. })));
        let mut rooms_seen = Vec::new();
        until(&mut c, 60.0, |c, _| {
            let a = c.agent("ravelle").unwrap();
            if rooms_seen.last().map(|r: &String| r.as_str()) != Some(a.walker.room()) {
                rooms_seen.push(a.walker.room().to_string());
            }
            a.walker.arrived()
        });
        let a = c.agent("ravelle").unwrap();
        assert_eq!(a.walker.room(), "dining");
        assert_eq!(rooms_seen.last().map(|s| s.as_str()), Some("dining"));
        assert!(a.walker.pos.distance(c.nav().stand_node("dining", 0).map(|n| n.p).unwrap_or_default()) < 1e-3);
        until(&mut c, 20.0, |c, _| c.agent("ravelle").map_or(false, |a| a.brain.activity() == Activity::Waiting));
        let a = c.agent("ravelle").unwrap();
        assert_eq!(a.brain.activity(), Activity::Waiting);
        assert!(a.brain.home_pos().unwrap_or_default().distance(a.walker.pos) < 1e-3);
    }

    #[test]
    fn a_disabled_brain_does_nothing_and_shows_no_pose() {
        let mut c = crowd(21);
        c.add("nyxie", "stage", 0);
        c.set_enabled("nyxie", false);
        let mut any = 0;
        until(&mut c, 120.0, |_, ev| {
            any += ev.len();
            false
        });
        assert_eq!(any, 0);
        let a = c.agent("nyxie").unwrap();
        assert_eq!(a.brain.activity(), Activity::Held);
        assert_eq!(a.brain.view().pose, None);
        c.set_enabled("nyxie", true);
        let t = until(&mut c, 600.0, |c, _| c.agent("nyxie").map_or(false, |a| a.brain.activity() == Activity::Staring));
        assert!(t.is_some());
        // switching it off mid-stare ends the stare
        c.set_enabled("nyxie", false);
        let ev = c.tick(0.05);
        assert!(ev.iter().any(|e| matches!(e, BehaviourEvent::StareEnd { .. })));
        assert_eq!(c.agent("nyxie").unwrap().brain.view().pose, None);
    }

    #[test]
    fn a_walking_hunter_is_seen_leaving_one_feed_and_entering_the_next() {
        let mut c = crowd(1);
        c.add("dovelle", "stage", 0);
        c.set_enabled("dovelle", false);
        assert_eq!(c.visible_in("stage").len(), 1);
        assert!(c.visible_in("kitchen").is_empty());
        assert!(c.send("dovelle", "kitchen", 0, Pace::Walk));
        let (mut in_stage_after_start, mut in_kitchen) = (false, false);
        let mut both = false;
        until(&mut c, 60.0, |c, _| {
            let s = !c.visible_in("stage").is_empty();
            let k = !c.visible_in("kitchen").is_empty();
            in_stage_after_start |= s;
            in_kitchen |= k;
            both |= s && k;
            c.agent("dovelle").unwrap().walker.arrived()
        });
        assert!(in_stage_after_start && in_kitchen);
        assert!(!both, "rooms never overlap");
        assert_eq!(c.visible_in("kitchen").len(), 1);
        assert!(c.visible_in("stage").is_empty());
    }

    #[test]
    fn drive_command_is_a_pose_only_while_standing_in_one() {
        let mut c = crowd(5);
        c.add("ravelle", "kitchen", 0);
        assert!(matches!(c.agent("ravelle").unwrap().drive_command(), DriveCommand::Loco { moving: false, .. }));
        until(&mut c, 600.0, |c, _| c.agent("ravelle").map_or(false, |a| a.brain.activity() == Activity::GoingToTask));
        c.tick(0.05); // the walker moves on the frame after the brain sent her
        assert!(matches!(c.agent("ravelle").unwrap().drive_command(), DriveCommand::Loco { moving: true, .. }));
        until(&mut c, 60.0, |c, _| c.agent("ravelle").map_or(false, |a| a.brain.activity() == Activity::Working));
        c.tick(0.05); // she is still settling onto the spot on the frame she arrives
        assert!(matches!(c.agent("ravelle").unwrap().drive_command(), DriveCommand::Pose { .. }));
    }

    /// A stand-in for the sim's attack machinery: a hunter due at the door after a pacer wait, with her own attack timer.
    struct FakeSim {
        rng: Mulberry32,
        pacer: Pacer,
        at_door_ms: f64,
        attack_ms: f64,
        moves: Vec<u64>,
        t_ms: f64,
    }

    impl FakeSim {
        fn new(seed: u32) -> FakeSim {
            FakeSim { rng: Mulberry32::new(seed), pacer: Pacer::new(), at_door_ms: 0.0, attack_ms: 4000.0, moves: Vec::new(), t_ms: 0.0 }
        }
        fn step(&mut self, dt_ms: f64) {
            self.t_ms += dt_ms;
            let rng = &mut self.rng;
            if self.pacer.step(dt_ms, 8, &mut || rng.next()) {
                self.moves.push(self.t_ms as u64);
                self.at_door_ms = 0.0;
            }
            self.at_door_ms += dt_ms;
        }
    }

    #[test]
    fn idle_behaviour_never_changes_the_sims_attack_timing() {
        // The sim runs alone, and again next to a crowd that stares, works and walks. The sim's moves, its timers and its
        // random stream come out identical: a brain has no way to reach them, and its own stream is separate.
        let mut alone = FakeSim::new(77);
        let mut with = FakeSim::new(77);
        let mut c = crowd(77);
        c.add("dovelle", "stage", 0);
        c.add("nyxie", "dining", 0);
        c.add("ravelle", "kitchen", 0);
        for i in 0..6000 {
            alone.step(50.0);
            with.step(50.0);
            let ev = c.tick(0.05);
            let _ = ev;
            if i % 400 == 399 {
                // the sim moves hunters now and then; the crowd follows
                let room = ["dining", "kitchen", "stage", "westhall"][(i / 400) % 4];
                c.send("dovelle", room, 0, Pace::Budget(2.0));
            }
        }
        assert_eq!(alone.moves, with.moves);
        assert!(alone.moves.len() > 3);
        assert_eq!(alone.at_door_ms, with.at_door_ms);
        assert_eq!(alone.attack_ms, with.attack_ms);
        assert_eq!(alone.rng.state(), with.rng.state());
        assert_eq!(alone.pacer.t, with.pacer.t);
        assert_eq!(alone.pacer.next, with.pacer.next);
    }

    #[test]
    fn brains_draw_from_their_own_streams() {
        // two crowds with the same seed but different hunters present: a hunter behaves the same with or without company
        let mut a = crowd(9);
        a.add("nyxie", "stage", 0);
        let mut b = crowd(9);
        b.add("dovelle", "dining", 0);
        b.add("nyxie", "stage", 0);
        let (mut ea, mut eb) = (Vec::new(), Vec::new());
        for i in 0..1200 {
            for e in a.tick(0.05) {
                if matches!(&e, BehaviourEvent::StareStart { hunter, .. } if hunter == "nyxie") {
                    ea.push(i);
                }
            }
            for e in b.tick(0.05) {
                if matches!(&e, BehaviourEvent::StareStart { hunter, .. } if hunter == "nyxie") {
                    eb.push(i);
                }
            }
        }
        assert!(!ea.is_empty());
        assert_eq!(ea, eb);
    }
}
