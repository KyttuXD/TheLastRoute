//! The Facility's rules (mode `story`), ported from `story_logic.gd` and `story_data.gd`: same numbers, same draws in the
//! same order. Pure logic: no drawing, no sound, no window, no real time. Tunables and the task lists are read from
//! `data/story/rules.json`.
//!
//! You walk from room to room and do each task at its room's own station. Moving loudly makes noise; crawling is slow and
//! quiet; every action is a little noisy. Three hunters answer to different things, and a hunter you are shining the beacon
//! on holds still:
//!   Opalie    lives in the gallery and closes in on noise. Stay quiet, or leave.
//!   Porcelle  lives in the workshop and the auditorium and closes in while it is dark. The beacon holds her and pushes her back.
//!   Quenelle  rides the lift. A bell rings, then it opens at Control. Be hidden, or be elsewhere.
//! The night ends when the last task is done. Hunters get keener as tasks are completed and with time on the clock.
//! `step(dt_ms, input)` with `SimInput { work, light, hide }` returns events; `move_dir(dir)` and `interact(..)` are the
//! player's actions.
#![allow(dead_code)]

use crate::core::{Fnv, Headless, Mulberry32, Outcome, Pacer, Session, Stream};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const RULES_JSON: &str = include_str!("../../data/story/rules.json");

// ------------------------------------------------------------------------------------------------ who

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Who {
    Opalie,
    Porcelle,
    Quenelle,
}

impl Who {
    /// Godot's CAST order.
    pub const ALL: [Who; 3] = [Who::Opalie, Who::Porcelle, Who::Quenelle];

    pub fn id(self) -> &'static str {
        match self {
            Who::Opalie => "opalie",
            Who::Porcelle => "porcelle",
            Who::Quenelle => "quenelle",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Who::Opalie => "Opalie",
            Who::Porcelle => "Porcelle",
            Who::Quenelle => "Quenelle",
        }
    }

    /// Where her sounds come from (StoryData.PAN).
    pub fn pan(self) -> f32 {
        match self {
            Who::Opalie => -0.8,
            Who::Porcelle => 0.8,
            Who::Quenelle => 0.0,
        }
    }

    pub fn from_id(s: &str) -> Option<Who> {
        Who::ALL.iter().copied().find(|w| w.id() == s)
    }
}

// ------------------------------------------------------------------------------------------------ data

fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
}

fn nums(v: &Value, key: &str) -> Vec<f64> {
    v.get(key).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default()
}

#[derive(Clone, Debug, Default)]
pub struct Variety {
    pub step_tempo: f64,
    pub start_nearer: f64,
    pub start_farther: f64,
    pub lift_gap_spread: f64,
}

/// StoryLogic.T
#[derive(Clone, Debug, Default)]
pub struct Tuning {
    pub start_dist: i32,
    pub strike_ms: f64,
    pub recover_ms: f64,
    pub walk_ms: f64,
    pub crawl_ms: f64,
    pub walk_noise: f64,
    pub crawl_noise: f64,
    pub work_noise_per_sec: f64,
    pub noise_decay_per_sec: f64,
    pub hear_noise: f64,
    pub door_ms: f64,
    pub combo_level: i32,
    pub combo_ms: f64,
    pub dial_steps: i32,
    pub step_noise: Vec<(String, f64)>,
    pub wrong_noise: f64,
    pub beacon_drain_per_sec: f64,
    pub beacon_regen_per_sec: f64,
    pub beacon_push_ms: f64,
    pub lift_base_ms: f64,
    pub lift_per_level_ms: f64,
    pub lift_min_ms: f64,
    pub bell_ms: f64,
    pub open_ms: f64,
    pub expose_ms: f64,
}

impl Tuning {
    pub fn step_noise_of(&self, kind: TaskKind) -> f64 {
        let name = kind.name();
        self.step_noise.iter().find(|(k, _)| k == name).map(|(_, v)| *v).unwrap_or(0.0)
    }
}

/// One night's column of StoryLogic.NIGHTS.
#[derive(Clone, Debug, Default)]
pub struct Tune {
    pub start_dist: i32,
    pub pace_x: f64,
    pub hear: f64,
    pub quiet_x: f64,
    pub recover_x: f64,
    pub away_pace_x: f64,
    pub strike_ms: f64,
    pub drain_x: f64,
    pub keen_ms: f64,
    pub keen_cap: i32,
    pub lift_gap_x: f64,
    pub open_x: f64,
    pub lift_sees: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskKind {
    Code,
    Dials,
    Collect,
    Crank,
    Switches,
    Hold,
}

impl TaskKind {
    pub fn name(self) -> &'static str {
        match self {
            TaskKind::Code => "code",
            TaskKind::Dials => "dials",
            TaskKind::Collect => "collect",
            TaskKind::Crank => "crank",
            TaskKind::Switches => "switches",
            TaskKind::Hold => "hold",
        }
    }

    pub fn parse(s: &str) -> Option<TaskKind> {
        Some(match s {
            "code" => TaskKind::Code,
            "dials" => TaskKind::Dials,
            "collect" => TaskKind::Collect,
            "crank" => TaskKind::Crank,
            "switches" => TaskKind::Switches,
            "hold" => TaskKind::Hold,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Task {
    pub room: String,
    pub label: String,
    pub kind: TaskKind,
    /// Digits, dials, parts, turns or breakers (0 for a hold).
    pub n: i32,
    /// Hold time in ms (0 for the others).
    pub ms: f64,
}

#[derive(Clone, Debug)]
pub struct RoomDef {
    pub id: String,
    pub label: String,
    /// (door letter, room): L = left door, R = right door, C = the lift door.
    pub exits: Vec<(char, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Rules {
    pub variety: Variety,
    pub t: Tuning,
    pub levels: Vec<(String, Vec<i32>)>,
    pub nights: Vec<(String, Vec<f64>)>,
    pub porcelle_rooms: Vec<String>,
    pub rooms: Vec<RoomDef>,
    tasks: Vec<(i32, Vec<Task>)>,
    orders: Vec<(i32, Vec<Vec<usize>>)>,
}

impl Rules {
    pub fn parse(src: &str) -> Result<Rules, String> {
        let root: Value = serde_json::from_str(src).map_err(|e| format!("story rules: {e}"))?;
        let var = root.get("variety").ok_or("story rules: no variety")?;
        let variety = Variety {
            step_tempo: num(var, "stepTempo"),
            start_nearer: num(var, "startNearer"),
            start_farther: num(var, "startFarther"),
            lift_gap_spread: num(var, "liftGapSpread"),
        };
        let t = root.get("t").ok_or("story rules: no t")?;
        let mut step_noise = Vec::new();
        if let Some(o) = t.get("stepNoise").and_then(|s| s.as_object()) {
            for (k, v) in o {
                step_noise.push((k.clone(), v.as_f64().unwrap_or(0.0)));
            }
        }
        let tuning = Tuning {
            start_dist: num(t, "startDist") as i32,
            strike_ms: num(t, "strikeMs"),
            recover_ms: num(t, "recoverMs"),
            walk_ms: num(t, "walkMs"),
            crawl_ms: num(t, "crawlMs"),
            walk_noise: num(t, "walkNoise"),
            crawl_noise: num(t, "crawlNoise"),
            work_noise_per_sec: num(t, "workNoisePerSec"),
            noise_decay_per_sec: num(t, "noiseDecayPerSec"),
            hear_noise: num(t, "hearNoise"),
            door_ms: num(t, "doorMs"),
            combo_level: num(t, "comboLevel") as i32,
            combo_ms: num(t, "comboMs"),
            dial_steps: num(t, "dialSteps") as i32,
            step_noise,
            wrong_noise: num(t, "wrongNoise"),
            beacon_drain_per_sec: num(t, "beaconDrainPerSec"),
            beacon_regen_per_sec: num(t, "beaconRegenPerSec"),
            beacon_push_ms: num(t, "beaconPushMs"),
            lift_base_ms: num(t, "liftBaseMs"),
            lift_per_level_ms: num(t, "liftPerLevelMs"),
            lift_min_ms: num(t, "liftMinMs"),
            bell_ms: num(t, "bellMs"),
            open_ms: num(t, "openMs"),
            expose_ms: num(t, "exposeMs"),
        };
        let mut levels = Vec::new();
        if let Some(o) = root.get("levels").and_then(|l| l.as_object()) {
            for (k, _) in o {
                levels.push((k.clone(), nums(&root["levels"], k).iter().map(|x| *x as i32).collect()));
            }
        }
        let mut nights = Vec::new();
        if let Some(o) = root.get("nights").and_then(|l| l.as_object()) {
            for (k, _) in o {
                nights.push((k.clone(), nums(&root["nights"], k)));
            }
        }
        let porcelle_rooms = root
            .get("porcelleRooms")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let mut rooms = Vec::new();
        if let Some(o) = root.get("rooms").and_then(|r| r.as_object()) {
            for (id, r) in o {
                let mut exits = Vec::new();
                if let Some(e) = r.get("exits").and_then(|e| e.as_object()) {
                    for (d, to) in e {
                        if let (Some(c), Some(to)) = (d.chars().next(), to.as_str()) {
                            exits.push((c, to.to_string()));
                        }
                    }
                }
                rooms.push(RoomDef { id: id.clone(), label: text(r, "label"), exits });
            }
        }
        let mut tasks: Vec<(i32, Vec<Task>)> = Vec::new();
        if let Some(o) = root.get("tasks").and_then(|r| r.as_object()) {
            for (night, list) in o {
                let n: i32 = night.parse().map_err(|_| format!("story rules: bad night '{night}'"))?;
                let mut out: Vec<Task> = Vec::new();
                for item in list.as_array().cloned().unwrap_or_default() {
                    let kind_name = text(&item, "kind");
                    let kind = TaskKind::parse(&kind_name).ok_or_else(|| format!("story rules: unknown task kind '{kind_name}'"))?;
                    out.push(Task { room: text(&item, "room"), label: text(&item, "label"), kind, n: num(&item, "n") as i32, ms: num(&item, "ms") });
                }
                tasks.push((n, out));
            }
        }
        let mut orders: Vec<(i32, Vec<Vec<usize>>)> = Vec::new();
        if let Some(o) = root.get("orders").and_then(|r| r.as_object()) {
            for (night, list) in o {
                let n: i32 = night.parse().map_err(|_| format!("story rules: bad night '{night}'"))?;
                let mut out: Vec<Vec<usize>> = Vec::new();
                for order in list.as_array().cloned().unwrap_or_default() {
                    out.push(order.as_array().map(|a| a.iter().filter_map(|x| x.as_u64().map(|v| v as usize)).collect()).unwrap_or_default());
                }
                orders.push((n, out));
            }
        }
        Ok(Rules { variety, t: tuning, levels, nights, porcelle_rooms, rooms, tasks, orders })
    }

    pub fn room(&self, id: &str) -> Option<&RoomDef> {
        self.rooms.iter().find(|r| r.id == id)
    }

    pub fn label(&self, id: &str) -> String {
        self.room(id).map(|r| r.label.clone()).unwrap_or_else(|| id.to_uppercase())
    }

    /// Where the door `dir` of `room` leads, if there is one.
    pub fn exit(&self, room: &str, dir: char) -> Option<&str> {
        self.room(room).and_then(|r| r.exits.iter().find(|(d, _)| *d == dir)).map(|(_, to)| to.as_str())
    }

    /// StoryData.tasks(night): nights 1-5 have their own list, anything else gets night 6's.
    pub fn tasks_for(&self, night: i32) -> Vec<Task> {
        let key = if (1..=5).contains(&night) { night } else { 6 };
        self.tasks.iter().find(|(n, _)| *n == key).map(|(_, t)| t.clone()).unwrap_or_default()
    }

    /// StoryData.TASK_ORDERS[night]
    pub fn orders_for(&self, night: i32) -> Vec<Vec<usize>> {
        self.orders.iter().find(|(n, _)| *n == night).map(|(_, o)| o.clone()).unwrap_or_default()
    }

    pub fn base_levels(&self, id: &str) -> Vec<i32> {
        self.levels.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    fn col(&self, key: &str, i: usize) -> f64 {
        self.nights.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.get(i.min(v.len().saturating_sub(1)))).copied().unwrap_or(0.0)
    }

    /// StoryLogic.tune_for: one column per night 1-6; a Custom Night plays night 6's column.
    pub fn tune_for(&self, night: i32, is_custom: bool) -> Tune {
        let i = if is_custom { 5 } else { (night.clamp(1, 6) - 1) as usize };
        Tune {
            start_dist: self.col("startDist", i) as i32,
            pace_x: self.col("paceX", i),
            hear: self.col("hear", i),
            quiet_x: self.col("quietX", i),
            recover_x: self.col("recoverX", i),
            away_pace_x: self.col("awayPaceX", i),
            strike_ms: self.col("strikeMs", i),
            drain_x: self.col("drainX", i),
            keen_ms: self.col("keenMs", i),
            keen_cap: self.col("keenCap", i) as i32,
            lift_gap_x: self.col("liftGapX", i),
            open_x: self.col("openX", i),
            lift_sees: self.col("liftSees", i) as i32,
        }
    }

    /// Who is on the floor on a level (0-based): hunters whose base level is above 0.
    pub fn active_hunters(&self, level_idx: usize) -> Vec<Who> {
        Who::ALL
            .iter()
            .copied()
            .filter(|w| self.base_levels(w.id()).get(level_idx.min(5)).copied().unwrap_or(0) > 0)
            .collect()
    }

    /// Problems in the data (empty = fine).
    pub fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.rooms.is_empty() {
            out.push("story rules: no rooms".to_string());
        }
        for n in 1..=6 {
            let tasks = self.tasks_for(n);
            if tasks.is_empty() {
                out.push(format!("story rules: night {n} has no tasks"));
            }
            for t in &tasks {
                if self.room(&t.room).is_none() {
                    out.push(format!("story rules: night {n} task '{}' is in unknown room '{}'", t.label, t.room));
                }
            }
            let orders = self.orders_for(n);
            if orders.is_empty() {
                out.push(format!("story rules: night {n} has no task order"));
            }
            for o in &orders {
                let mut seen = o.clone();
                seen.sort();
                let want: Vec<usize> = (0..tasks.len()).collect();
                if seen != want {
                    out.push(format!("story rules: night {n} order {o:?} is not a permutation of its {} tasks", tasks.len()));
                }
            }
        }
        for r in &self.rooms {
            for (d, to) in &r.exits {
                if self.room(to).is_none() {
                    out.push(format!("story rules: room {} door {d} leads to unknown room {to}", r.id));
                }
            }
        }
        for w in Who::ALL {
            if self.base_levels(w.id()).len() != 6 {
                out.push(format!("story rules: {} needs six levels", w.id()));
            }
        }
        for key in ["startDist", "paceX", "hear", "quietX", "recoverX", "awayPaceX", "strikeMs", "drainX", "keenMs", "keenCap", "liftGapX", "openX", "liftSees"] {
            if self.nights.iter().find(|(k, _)| k == key).map(|(_, v)| v.len()) != Some(6) {
                out.push(format!("story rules: nights.{key} needs six columns"));
            }
        }
        out
    }
}

/// The Facility's rules, parsed once. A broken embedded file gives an empty set (`--check` reports it).
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules::parse(RULES_JSON).unwrap_or_default())
}

/// The first door to take on the shortest walk from `from` to `to` (breadth first over the rooms' doors).
pub fn next_dir(from: &str, to: &str) -> Option<char> {
    if from == to {
        return None;
    }
    let r = rules();
    let mut seen: Vec<String> = vec![from.to_string()];
    let mut queue: Vec<(String, char)> = Vec::new();
    if let Some(room) = r.room(from) {
        for (d, next) in &room.exits {
            if !seen.contains(next) {
                seen.push(next.clone());
                queue.push((next.clone(), *d));
            }
        }
    }
    let mut i = 0;
    while i < queue.len() {
        let (room, first) = queue[i].clone();
        i += 1;
        if room == to {
            return Some(first);
        }
        if let Some(def) = r.room(&room) {
            for (_, next) in &def.exits {
                if !seen.contains(next) {
                    seen.push(next.clone());
                    queue.push((next.clone(), first));
                }
            }
        }
    }
    None
}

// ------------------------------------------------------------------------------------------------ the state

#[derive(Clone, Debug, PartialEq)]
pub enum Puzzle {
    None,
    /// Key the number on the note.
    Code { n: i32, code: String, entered: String },
    /// Turn each lamp's dial to its mark.
    Dials { n: i32, target: Vec<i32>, pos: Vec<i32> },
    /// Take every part off the bench.
    Collect { n: i32, taken: Vec<i32> },
    /// Turn the stage crank.
    Crank { n: i32, done: i32 },
    /// Flip the breakers in their numbered order.
    Switches { n: i32, order: Vec<i32>, next: i32 },
    /// Hold the lift's call button.
    Hold { ms: f64 },
}

impl Puzzle {
    pub fn kind(&self) -> Option<TaskKind> {
        match self {
            Puzzle::None => None,
            Puzzle::Code { .. } => Some(TaskKind::Code),
            Puzzle::Dials { .. } => Some(TaskKind::Dials),
            Puzzle::Collect { .. } => Some(TaskKind::Collect),
            Puzzle::Crank { .. } => Some(TaskKind::Crank),
            Puzzle::Switches { .. } => Some(TaskKind::Switches),
            Puzzle::Hold { .. } => Some(TaskKind::Hold),
        }
    }
}

/// Opalie's and Porcelle's closing in.
#[derive(Clone, Debug)]
pub struct Creeper {
    /// Steps from you (0 = at your throat).
    pub dist: i32,
    pub pacer: Pacer,
    pub strike: f64,
    pub away: f64,
    pub recover: f64,
    pub push: f64,
    pub alert: f64,
    pub tempo: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LiftState {
    Away,
    Bell,
    Open,
}

impl LiftState {
    pub fn name(self) -> &'static str {
        match self {
            LiftState::Away => "away",
            LiftState::Bell => "bell",
            LiftState::Open => "open",
        }
    }
}

/// Quenelle's lift.
#[derive(Clone, Debug)]
pub struct Lift {
    pub state: LiftState,
    pub t: f64,
    pub clock: f64,
    /// -1 until drawn.
    pub gap: f64,
    pub expose: f64,
    pub open_for: f64,
}

/// A walk through a door: until `door` runs out you are still in the room you are leaving.
#[derive(Clone, Debug)]
pub struct Moving {
    pub to: String,
    pub left: f64,
    pub door: f64,
    /// The whole walk, for the glide.
    pub dur: f64,
    pub dir: char,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    Running,
    Win,
    Caught,
}

/// Held keys of this step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SimInput {
    pub work: bool,
    pub light: bool,
    pub hide: bool,
}

/// One action at a station: a keypad digit, or an index (dial, part, breaker); neither for the crank.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Interact {
    pub digit: Option<char>,
    pub i: Option<i32>,
}

impl Interact {
    pub fn digit(d: char) -> Interact {
        Interact { digit: Some(d), i: None }
    }
    pub fn index(i: i32) -> Interact {
        Interact { digit: None, i: Some(i) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    Arrive(String),
    Close { id: Who, dist: i32, away: bool, quiet: bool },
    Back { id: Who, dist: i32 },
    Pushed(Who),
    Alert(Who),
    Bell,
    Open,
    Leave,
    Task(usize),
    Win,
    Caught(Who),
    Step(TaskKind),
    Wrong(TaskKind),
    Deny,
}

impl SimEvent {
    /// A short stable name (headless event lists, digests).
    pub fn name(&self) -> String {
        match self {
            SimEvent::Arrive(r) => format!("arrive:{r}"),
            SimEvent::Close { id, dist, away, quiet } => format!("close:{}:{}:{}{}", id.id(), dist, if *away { "a" } else { "" }, if *quiet { "q" } else { "" }),
            SimEvent::Back { id, dist } => format!("back:{}:{}", id.id(), dist),
            SimEvent::Pushed(w) => format!("pushed:{}", w.id()),
            SimEvent::Alert(w) => format!("alert:{}", w.id()),
            SimEvent::Bell => "bell".to_string(),
            SimEvent::Open => "open".to_string(),
            SimEvent::Leave => "leave".to_string(),
            SimEvent::Task(i) => format!("task:{i}"),
            SimEvent::Win => "win".to_string(),
            SimEvent::Caught(w) => format!("caught:{}", w.id()),
            SimEvent::Step(k) => format!("step:{}", k.name()),
            SimEvent::Wrong(k) => format!("wrong:{}", k.name()),
            SimEvent::Deny => "deny".to_string(),
        }
    }
}

/// StoryLogic's `opts`.
pub struct Options {
    pub night: i32,
    pub rng: Stream,
    pub var_rng: Option<Stream>,
    pub levels: BTreeMap<String, i32>,
    pub custom: bool,
    pub infinite_beacon: bool,
    /// A task order index (tests).
    pub order: Option<usize>,
}

impl Options {
    pub fn night(night: i32, seed: u32) -> Options {
        Options { night, rng: Stream::seeded(seed), var_rng: None, levels: BTreeMap::new(), custom: false, infinite_beacon: false, order: None }
    }

    /// The options of a session: the night (1-6), or a Custom Night (night 5's tables with the sliders' levels).
    pub fn from_session(session: &Session) -> Options {
        let seed = session.seed_or_fresh();
        if session.is_custom {
            Options {
                night: 5,
                rng: Stream::seeded(seed),
                var_rng: None,
                levels: session.custom_levels.clone(),
                custom: true,
                infinite_beacon: false,
                order: None,
            }
        } else {
            Options::night(session.night.clamp(1, 6), seed)
        }
    }
}

pub struct StoryLogic {
    /// 0-based night.
    pub n: usize,
    pub rng: Stream,
    pub vr: Stream,
    pub levels: BTreeMap<String, i32>,
    pub is_custom: bool,
    pub infinite_beacon: bool,
    pub tune: Tune,
    pub order_idx: usize,
    pub tasks: Vec<Task>,

    pub elapsed: f64,
    pub over: Over,
    pub by: Option<Who>,
    pub room: String,
    pub moving: Option<Moving>,
    pub crawl: bool,
    pub hidden: bool,
    pub light: bool,
    pub working: bool,
    pub noise: f64,
    pub charge: f64,
    pub task_idx: usize,
    pub task_ms: f64,
    pub puzzle: Puzzle,
    pub opalie: Creeper,
    pub porcelle: Creeper,
    pub quenelle: Lift,
    pub dt: f64,
    /// Admin: god mode sends whoever would have got you a step back.
    pub god: bool,
    /// Admin: the hunters' step clocks run at this multiple (1.0 = the 2D numbers).
    pub hunter_speed: f64,
}

impl StoryLogic {
    pub fn new(opts: Options) -> StoryLogic {
        let r = rules();
        let n = (opts.night.clamp(1, 6) - 1) as usize;
        let rng = opts.rng;
        let mut vr = Stream::variety_of(&rng, opts.var_rng);
        let list_night = if opts.custom { 6 } else { n as i32 + 1 };
        let orders = r.orders_for(list_night);
        let drawn = vr.pick_index(orders.len());
        let order_idx = match opts.order {
            Some(o) if o < orders.len() => o,
            _ => drawn,
        };
        let base = r.tasks_for(list_night);
        let mut tasks = Vec::new();
        if let Some(order) = orders.get(order_idx) {
            for i in order {
                if let Some(t) = base.get(*i) {
                    tasks.push(t.clone());
                }
            }
        }
        let tune = r.tune_for(opts.night, opts.custom);
        let mut sim = StoryLogic {
            n,
            rng,
            vr,
            levels: opts.levels,
            is_custom: opts.custom,
            infinite_beacon: opts.infinite_beacon,
            tune,
            order_idx,
            tasks,
            elapsed: 0.0,
            over: Over::Running,
            by: None,
            room: "control".to_string(),
            moving: None,
            crawl: false,
            hidden: false,
            light: false,
            working: false,
            noise: 0.0,
            charge: 100.0,
            task_idx: 0,
            task_ms: 0.0,
            puzzle: Puzzle::None,
            opalie: Creeper { dist: 0, pacer: Pacer::new(), strike: 0.0, away: 0.0, recover: 0.0, push: 0.0, alert: 0.0, tempo: 1.0 },
            porcelle: Creeper { dist: 0, pacer: Pacer::new(), strike: 0.0, away: 0.0, recover: 0.0, push: 0.0, alert: 0.0, tempo: 1.0 },
            quenelle: Lift { state: LiftState::Away, t: 0.0, clock: 0.0, gap: -1.0, expose: 0.0, open_for: r.t.open_ms },
            dt: 0.0,
            god: false,
            hunter_speed: 1.0,
        };
        // the draws, in Godot's order: opalie's creeper, porcelle's creeper, the first puzzle
        sim.opalie = sim.make_creeper();
        sim.porcelle = sim.make_creeper();
        sim.puzzle = sim.make_puzzle(0);
        sim
    }

    pub fn base_level(&self, who: Who) -> i32 {
        if let Some(v) = self.levels.get(who.id()) {
            if *v >= 0 {
                return *v;
            }
        }
        rules().base_levels(who.id()).get(self.n).copied().unwrap_or(0)
    }

    /// Time on the clock makes each hunter keener (not on a Custom Night).
    pub fn keen(&self) -> i32 {
        if !self.is_custom && self.tune.keen_ms > 0.0 {
            return self.tune.keen_cap.min((self.elapsed / self.tune.keen_ms).floor() as i32);
        }
        0
    }

    /// Steps up as tasks are done (+1 every two, up to +2), and with time on the clock.
    pub fn level(&self, who: Who) -> i32 {
        let b = self.base_level(who);
        if b <= 0 {
            return 0;
        }
        20i32.min(b + 2i32.min(self.task_idx as i32 / 2) + self.keen())
    }

    pub fn creeper(&self, who: Who) -> &Creeper {
        if who == Who::Porcelle {
            &self.porcelle
        } else {
            &self.opalie
        }
    }

    pub fn creeper_mut(&mut self, who: Who) -> &mut Creeper {
        if who == Who::Porcelle {
            &mut self.porcelle
        } else {
            &mut self.opalie
        }
    }

    pub fn task(&self) -> Option<&Task> {
        self.tasks.get(self.task_idx)
    }

    fn spread(&mut self, ms: f64, s: f64) -> f64 {
        ms * (1.0 - s + 2.0 * s * self.rng.next())
    }

    fn recover_draw(&mut self) -> f64 {
        let ms = rules().t.recover_ms * self.tune.recover_x;
        self.spread(ms, 0.2)
    }

    fn start_offset(&mut self) -> i32 {
        let u = self.vr.next();
        if self.vr.neutral || (self.n == 0 && !self.is_custom) {
            return 0;
        }
        let v = &rules().variety;
        if u < v.start_nearer {
            return -1;
        }
        if u >= 1.0 - v.start_farther {
            return 1;
        }
        0
    }

    fn make_creeper(&mut self) -> Creeper {
        let dist = self.tune.start_dist + self.start_offset();
        let recover = self.recover_draw();
        let tempo = self.vr.tempo(rules().variety.step_tempo);
        Creeper { dist, pacer: Pacer::new(), strike: 0.0, away: 0.0, recover, push: 0.0, alert: 0.0, tempo }
    }

    fn pick(&mut self, count: i32) -> i32 {
        (self.rng.next() * count as f64).floor() as i32
    }

    /// The puzzle for a task, made when it comes up (so the same seed gives the same night).
    pub fn make_puzzle(&mut self, idx: usize) -> Puzzle {
        let Some(t) = self.tasks.get(idx).cloned() else { return Puzzle::None };
        let steps = rules().t.dial_steps.max(2);
        match t.kind {
            TaskKind::Code => {
                let mut code = String::new();
                for _ in 0..t.n {
                    let d = self.pick(10);
                    code.push(char::from_digit(d as u32, 10).unwrap_or('0'));
                }
                Puzzle::Code { n: t.n, code, entered: String::new() }
            }
            TaskKind::Dials => {
                let mut target = Vec::new();
                for _ in 0..t.n {
                    target.push(self.pick(steps));
                }
                let mut pos = Vec::new();
                for x in &target {
                    pos.push((*x + 1 + self.pick(steps - 1)) % steps);
                }
                Puzzle::Dials { n: t.n, target, pos }
            }
            TaskKind::Collect => Puzzle::Collect { n: t.n, taken: Vec::new() },
            TaskKind::Crank => Puzzle::Crank { n: t.n, done: 0 },
            TaskKind::Switches => {
                let mut order: Vec<i32> = (0..t.n).collect();
                let mut i = order.len() as i32 - 1;
                while i > 0 {
                    let j = self.pick(i + 1);
                    order.swap(i as usize, j as usize);
                    i -= 1;
                }
                Puzzle::Switches { n: t.n, order, next: 0 }
            }
            TaskKind::Hold => Puzzle::Hold { ms: t.ms },
        }
    }

    /// Opalie and Porcelle close in a step at a time while you are in their rooms; leave and they slowly step back, or from
    /// night 2 hover near the door. True when the strike lands.
    fn creep(&mut self, who: Who, present: bool, ev: &mut Vec<SimEvent>) -> bool {
        let mut h = self.creeper(who).clone();
        let hit = self.creep_inner(who, &mut h, present, ev);
        *self.creeper_mut(who) = h;
        hit
    }

    fn creep_inner(&mut self, who: Who, h: &mut Creeper, present: bool, ev: &mut Vec<SimEvent>) -> bool {
        let t = &rules().t;
        let tempo_spread = rules().variety.step_tempo;
        let level = self.level(who);
        if !present {
            h.strike = 0.0;
            h.push = 0.0; // her step timer is kept: stepping out does not restart her wait
            let hears = if who == Who::Opalie { self.noise > self.tune.hear } else { true };
            if self.tune.away_pace_x > 0.0 && h.dist > 1 {
                h.away = 0.0;
                if hears {
                    let dtp = self.dt * self.tune.away_pace_x * h.tempo * self.hunter_speed;
                    let rng = &mut self.rng;
                    if h.pacer.step(dtp, level, &mut || rng.next()) {
                        h.dist -= 1;
                        h.tempo = self.vr.tempo(tempo_spread);
                        ev.push(SimEvent::Close { id: who, dist: h.dist, away: true, quiet: false });
                    }
                }
                return false;
            }
            h.away += self.dt;
            if h.away >= h.recover && h.dist < self.tune.start_dist {
                h.dist += 1;
                h.away = 0.0;
                h.recover = self.recover_draw();
                ev.push(SimEvent::Back { id: who, dist: h.dist });
            }
            return false;
        }
        h.away = 0.0;
        if h.alert != 0.0 {
            h.pacer.skip(h.alert);
            h.alert = 0.0;
        }
        if self.light {
            // looked at: she holds still, and Porcelle is pushed back
            if who == Who::Porcelle {
                h.push += self.dt;
                if h.push >= t.beacon_push_ms {
                    h.push = 0.0;
                    if h.dist < self.tune.start_dist {
                        h.dist += 1;
                        ev.push(SimEvent::Pushed(who));
                    }
                }
            }
        } else {
            h.push = 0.0;
            let noticed = if who == Who::Opalie { self.noise > self.tune.hear } else { true };
            let pace = if noticed { self.tune.pace_x } else { self.tune.quiet_x };
            if h.dist > 0 && pace > 0.0 {
                let dtp = self.dt * pace * h.tempo * self.hunter_speed;
                let rng = &mut self.rng;
                if h.pacer.step(dtp, level, &mut || rng.next()) {
                    h.dist -= 1;
                    h.tempo = self.vr.tempo(tempo_spread);
                    ev.push(SimEvent::Close { id: who, dist: h.dist, away: false, quiet: !noticed });
                }
            }
        }
        if h.dist <= 0 {
            h.strike += self.dt;
            return h.strike >= self.tune.strike_ms;
        }
        h.strike = 0.0;
        false
    }

    /// Someone got you: admin god mode sends her back instead.
    fn finish(&mut self, ev: &mut Vec<SimEvent>, who: Who) {
        if self.god {
            if who == Who::Quenelle {
                self.quenelle.expose = 0.0;
                self.quenelle.state = LiftState::Away;
                self.quenelle.t = 0.0;
            } else {
                let h = self.creeper_mut(who);
                h.dist = 2;
                h.strike = 0.0;
            }
            return;
        }
        self.over = Over::Caught;
        self.by = Some(who);
        ev.push(SimEvent::Caught(who));
    }

    /// Slipping out of a room with her at your throat gives you a step of room (once you are through the door).
    fn slip(&mut self) {
        if self.room == "gallery" && self.opalie.dist == 0 {
            self.opalie.dist = 1;
        }
        if rules().porcelle_rooms.contains(&self.room) && self.porcelle.dist == 0 {
            self.porcelle.dist = 1;
        }
    }

    /// Advance by `dt_ms` with the held keys of this step.
    pub fn step(&mut self, dt_ms: f64, input: SimInput) -> Vec<SimEvent> {
        let mut ev = Vec::new();
        if self.over != Over::Running {
            return ev;
        }
        let t = &rules().t;
        self.dt = dt_ms;
        self.elapsed += dt_ms;
        let dts = dt_ms / 1000.0;
        // walking between rooms takes time; getting through the door takes doorMs, and until then you are still in the room you are leaving
        let mut through = false;
        if let Some(mut m) = self.moving.take() {
            if m.door > 0.0 {
                m.door -= dt_ms;
                through = m.door <= 0.0;
            }
            m.left -= dt_ms;
            if m.left <= 0.0 {
                self.room = m.to.clone();
                ev.push(SimEvent::Arrive(m.to));
            } else {
                self.moving = Some(m);
            }
        }
        let still = self.moving.is_none();
        let there = still || self.moving.as_ref().map_or(false, |m| m.door > 0.0);
        self.hidden = still && self.room == "control" && input.hide;
        self.light = input.light && still && self.charge > 0.0;
        if self.light {
            if !self.infinite_beacon {
                self.charge = (self.charge - t.beacon_drain_per_sec * self.tune.drain_x * dts).max(0.0);
            }
        } else {
            self.charge = (self.charge + t.beacon_regen_per_sec * dts).min(100.0);
        }
        // the current task: hold work in the right room
        let task = self.tasks.get(self.task_idx).cloned();
        self.noise = (self.noise - t.noise_decay_per_sec * dts).max(0.0);
        self.working = still
            && !self.hidden
            && input.work
            && task.as_ref().map_or(false, |k| k.kind == TaskKind::Hold && k.room == self.room);
        if self.working {
            self.task_ms += dt_ms;
            self.noise = (self.noise + t.work_noise_per_sec * dts).min(100.0);
            let need = task.as_ref().map_or(0.0, |k| k.ms);
            if self.task_ms >= need {
                self.complete_task(&mut ev);
            }
            if self.over != Over::Running {
                return ev;
            }
        }
        let before = ev.len();
        if self.creep(Who::Opalie, there && self.room == "gallery", &mut ev) {
            self.finish(&mut ev, Who::Opalie);
            return ev;
        }
        // working together: when Opalie closes in on a noise she is heard, and Porcelle (once both are keen) will be 1.5 s nearer a step
        let closed = ev[before..].iter().any(|e| matches!(e, SimEvent::Close { id: Who::Opalie, .. }));
        if closed && self.level(Who::Opalie) >= t.combo_level && self.level(Who::Porcelle) >= t.combo_level {
            self.porcelle.alert = t.combo_ms;
            ev.push(SimEvent::Alert(Who::Porcelle));
        }
        let in_porcelle_room = rules().porcelle_rooms.contains(&self.room);
        if self.creep(Who::Porcelle, there && in_porcelle_room, &mut ev) {
            self.finish(&mut ev, Who::Porcelle);
            return ev;
        }
        if through {
            self.slip(); // through the door before her strike landed
        }
        // Quenelle: a bell, then the lift opens at Control for a while
        let q_level = self.level(Who::Quenelle);
        if q_level > 0 {
            self.quenelle.clock += dt_ms;
            if self.quenelle.gap < 0.0 {
                let base = t.lift_min_ms.max(t.lift_base_ms - q_level as f64 * t.lift_per_level_ms) * self.tune.lift_gap_x;
                let s = rules().variety.lift_gap_spread;
                self.quenelle.gap = base * (1.0 - s + 2.0 * s * self.rng.next());
            }
            let state = self.quenelle.state;
            if state == LiftState::Away && self.quenelle.clock >= self.quenelle.gap {
                self.quenelle.state = LiftState::Bell;
                self.quenelle.t = 0.0;
                self.quenelle.clock = 0.0;
                self.quenelle.gap = -1.0;
                ev.push(SimEvent::Bell);
            } else if state == LiftState::Bell {
                self.quenelle.t += dt_ms;
                if self.quenelle.t >= t.bell_ms {
                    self.quenelle.state = LiftState::Open;
                    self.quenelle.t = 0.0;
                    self.quenelle.expose = 0.0;
                    let ms = t.open_ms * self.tune.open_x;
                    self.quenelle.open_for = self.spread(ms, 0.2);
                    ev.push(SimEvent::Open);
                }
            } else if state == LiftState::Open {
                self.quenelle.t += dt_ms;
                let seen = there && ((self.room == "control" && !self.hidden) || (self.room == "lift" && self.tune.lift_sees != 0));
                if seen {
                    self.quenelle.expose += dt_ms;
                } else {
                    self.quenelle.expose = (self.quenelle.expose - dt_ms).max(0.0);
                }
                if self.quenelle.expose >= t.expose_ms {
                    self.finish(&mut ev, Who::Quenelle);
                    return ev;
                }
                if self.quenelle.t >= self.quenelle.open_for {
                    self.quenelle.state = LiftState::Away;
                    self.quenelle.t = 0.0;
                    self.quenelle.expose = 0.0;
                    self.quenelle.clock = 0.0;
                    self.quenelle.gap = -1.0;
                    ev.push(SimEvent::Leave);
                }
            }
        }
        ev
    }

    /// The current task is done: the next puzzle comes up, or the night is won.
    pub fn complete_task(&mut self, ev: &mut Vec<SimEvent>) {
        self.task_ms = 0.0;
        self.task_idx += 1;
        ev.push(SimEvent::Task(self.task_idx));
        if self.task_idx >= self.tasks.len() {
            self.over = Over::Win;
            self.puzzle = Puzzle::None;
            ev.push(SimEvent::Win);
            return;
        }
        self.puzzle = self.make_puzzle(self.task_idx);
    }

    /// One action at the current task's station. Wrong moves start the puzzle over and are loud.
    pub fn interact(&mut self, p: Interact) -> Vec<SimEvent> {
        if self.over != Over::Running || self.moving.is_some() || self.hidden {
            return Vec::new();
        }
        let Some(task) = self.tasks.get(self.task_idx).cloned() else { return vec![SimEvent::Deny] };
        if task.room != self.room || matches!(self.puzzle, Puzzle::None | Puzzle::Hold { .. }) {
            return vec![SimEvent::Deny];
        }
        let steps = rules().t.dial_steps.max(2);
        let (ok, done) = match &mut self.puzzle {
            Puzzle::Code { n, code, entered } => {
                let want = code.chars().nth(entered.chars().count());
                match (p.digit, want) {
                    (Some(d), Some(w)) if d == w => {
                        entered.push(d);
                        (true, entered.chars().count() as i32 == *n)
                    }
                    _ => {
                        entered.clear();
                        (false, false)
                    }
                }
            }
            Puzzle::Dials { n, target, pos } => {
                let i = p.i.unwrap_or(-1);
                if !(i >= 0 && i < *n) {
                    return vec![SimEvent::Deny];
                }
                let k = i as usize;
                pos[k] = (pos[k] + 1) % steps;
                (true, (0..*n as usize).all(|j| pos[j] == target[j]))
            }
            Puzzle::Collect { n, taken } => {
                let i = p.i.unwrap_or(-1);
                if !(i >= 0 && i < *n) || taken.contains(&i) {
                    return vec![SimEvent::Deny];
                }
                taken.push(i);
                (true, taken.len() as i32 == *n)
            }
            Puzzle::Crank { n, done } => {
                *done += 1;
                (true, *done >= *n)
            }
            Puzzle::Switches { n, order, next } => {
                let i = p.i.unwrap_or(-1);
                if !(i >= 0 && i < *n) {
                    return vec![SimEvent::Deny];
                }
                let want = order.get(*next as usize).copied().unwrap_or(-1);
                if i == want {
                    *next += 1;
                    (true, *next == *n)
                } else {
                    *next = 0;
                    (false, false)
                }
            }
            Puzzle::None | Puzzle::Hold { .. } => return vec![SimEvent::Deny],
        };
        let t = &rules().t;
        let add = if ok { t.step_noise_of(task.kind) } else { t.wrong_noise };
        self.noise = (self.noise + add).min(100.0);
        let mut ev = vec![if ok { SimEvent::Step(task.kind) } else { SimEvent::Wrong(task.kind) }];
        if done {
            self.complete_task(&mut ev);
        }
        ev
    }

    /// Start walking through a door (`L`, `R` or `C`). False when there is none or you are already walking.
    pub fn move_dir(&mut self, dir: char) -> bool {
        if self.over != Over::Running || self.moving.is_some() {
            return false;
        }
        let Some(to) = rules().exit(&self.room, dir).map(|s| s.to_string()) else { return false };
        let t = &rules().t;
        self.hidden = false;
        self.noise = (self.noise + if self.crawl { t.crawl_noise } else { t.walk_noise }).min(100.0);
        let left = if self.crawl { t.crawl_ms } else { t.walk_ms };
        self.moving = Some(Moving { to, left, door: t.door_ms, dur: left, dir });
        true
    }

    pub fn set_crawl(&mut self, on: bool) {
        self.crawl = on;
    }

    /// Admin: jump to task `index` with a fresh puzzle.
    pub fn jump_to_task(&mut self, index: usize) {
        if index >= self.tasks.len() {
            return;
        }
        self.task_idx = index;
        self.task_ms = 0.0;
        self.puzzle = self.make_puzzle(index);
    }

    /// FNV-1a of the key state (stable across runs).
    pub fn digest(&self) -> u64 {
        let mut h = Fnv::new();
        h.f64(self.elapsed).i32(self.over as i32).str(&self.room).i32(self.task_idx as i32).f64(self.noise).f64(self.charge);
        h.i32(self.opalie.dist).i32(self.porcelle.dist).i32(self.quenelle.state as i32).f64(self.quenelle.clock);
        h.u64(self.rng.state() as u64).u64(self.vr.state() as u64);
        h.finish()
    }
}

// ------------------------------------------------------------------------------------------------ headless

/// The scripted player of `--check` and the tests: it does the list in order, crawls from night 2, hides from the lift,
/// shines the beacon at whoever is at her throat. Seeded, so a seed always plays the same night.
struct Bot {
    cd: f64,
    r: Mulberry32,
}

impl Bot {
    fn new(seed: u32) -> Bot {
        Bot { cd: 600.0, r: Mulberry32::new(seed ^ 0x4641_4343) }
    }

    fn act(&mut self, s: &mut StoryLogic, dt_ms: f64, out: &mut Vec<SimEvent>) -> SimInput {
        let mut input = SimInput::default();
        if s.over != Over::Running {
            return input;
        }
        let lift_busy = s.level(Who::Quenelle) > 0 && s.quenelle.state != LiftState::Away;
        let still = s.moving.is_none();
        input.hide = still && s.room == "control" && lift_busy;
        let at_gallery = s.room == "gallery" && s.opalie.dist <= 1 && s.level(Who::Opalie) > 0;
        let at_porcelle = rules().porcelle_rooms.contains(&s.room) && s.porcelle.dist <= 1 && s.level(Who::Porcelle) > 0;
        input.light = still && (at_gallery || at_porcelle) && s.charge > 5.0;
        self.cd -= dt_ms;
        if !still || input.hide || self.cd > 0.0 {
            return input;
        }
        let Some(task) = s.task().cloned() else { return input };
        if s.room == "lift" && lift_busy {
            s.set_crawl(false);
            s.move_dir('C');
            self.cd = 200.0;
            return input;
        }
        if s.room != task.room {
            let Some(dir) = next_dir(&s.room, &task.room) else { return input };
            let into_lift = rules().exit(&s.room, dir) == Some("lift");
            if into_lift && lift_busy {
                self.cd = 250.0;
                return input;
            }
            s.set_crawl(s.n >= 1);
            s.move_dir(dir);
            self.cd = 250.0 + self.r.next() * 350.0;
            return input;
        }
        if task.kind == TaskKind::Hold {
            input.work = true;
            return input;
        }
        let act = match &s.puzzle {
            Puzzle::Code { code, entered, .. } => code.chars().nth(entered.chars().count()).map(Interact::digit),
            Puzzle::Dials { n, target, pos } => (0..*n as usize).find(|i| pos[*i] != target[*i]).map(|i| Interact::index(i as i32)),
            Puzzle::Collect { n, taken } => (0..*n).find(|i| !taken.contains(i)).map(Interact::index),
            Puzzle::Crank { .. } => Some(Interact::default()),
            Puzzle::Switches { order, next, .. } => order.get(*next as usize).map(|i| Interact::index(*i)),
            Puzzle::Hold { .. } | Puzzle::None => None,
        };
        if let Some(a) = act {
            out.extend(s.interact(a));
            self.cd = 350.0 + self.r.next() * 250.0;
        }
        input
    }
}

/// The window-less half: the rules plus the scripted player.
pub struct StoryHeadless {
    pub sim: StoryLogic,
    bot: Bot,
}

impl StoryHeadless {
    pub fn new(session: &Session) -> StoryHeadless {
        let seed = session.seed_or_fresh();
        StoryHeadless { sim: StoryLogic::new(Options::from_session(session)), bot: Bot::new(seed) }
    }

    pub fn with_options(opts: Options, seed: u32) -> StoryHeadless {
        StoryHeadless { sim: StoryLogic::new(opts), bot: Bot::new(seed) }
    }
}

impl Headless for StoryHeadless {
    fn id(&self) -> &'static str {
        "story"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        let mut events = Vec::new();
        let input = self.bot.act(&mut self.sim, dt_ms, &mut events);
        events.extend(self.sim.step(dt_ms, input));
        events.iter().map(|e| e.name()).collect()
    }

    fn outcome(&self) -> Outcome {
        match self.sim.over {
            Over::Running => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn elapsed_ms(&self) -> f64 {
        self.sim.elapsed
    }

    fn digest(&self) -> u64 {
        self.sim.digest()
    }
}

// ------------------------------------------------------------------------------------------------ tests

/// Why this mode cannot start, if so: Story cannot start: the rules, the map, the nav file or the behaviour file does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    Rules::parse(RULES_JSON).err().or_else(super::world::load_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(Rules::parse("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    fn sim(night: i32, seed: u32) -> StoryLogic {
        StoryLogic::new(Options::night(night, seed))
    }

    fn quiet() -> SimInput {
        SimInput::default()
    }

    #[test]
    fn the_embedded_rules_parse_and_validate() {
        let r = Rules::parse(RULES_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert!(r.validate().is_empty(), "{:?}", r.validate());
        assert_eq!(r.rooms.len(), 6);
        assert_eq!(r.tasks_for(1).len(), 4);
        assert_eq!(r.tasks_for(6).len(), 7);
        assert_eq!(r.tasks_for(9).len(), 7, "anything past night 5 gets night 6's list");
        assert_eq!(r.t.expose_ms, 1500.0);
        assert_eq!(r.base_levels("quenelle"), vec![0, 0, 0, 6, 9, 12]);
    }

    #[test]
    fn nights_tune_like_the_godot_table() {
        let r = rules();
        let n1 = r.tune_for(1, false);
        assert_eq!((n1.start_dist, n1.pace_x, n1.hear, n1.lift_sees), (3, 1.0, 24.0, 0));
        let n6 = r.tune_for(6, false);
        assert_eq!((n6.start_dist, n6.pace_x, n6.keen_cap, n6.lift_sees), (2, 3.2, 4, 1));
        // a Custom Night plays night 6's column
        assert_eq!(r.tune_for(5, true).pace_x, 3.2);
        assert_eq!(r.active_hunters(0), Vec::<Who>::new());
        assert_eq!(r.active_hunters(1), vec![Who::Opalie]);
        assert_eq!(r.active_hunters(5), vec![Who::Opalie, Who::Porcelle, Who::Quenelle]);
    }

    #[test]
    fn the_list_order_is_drawn_and_log_in_stays_first() {
        for seed in 1..60 {
            let s = sim(6, seed);
            assert_eq!(s.tasks.len(), 7);
            assert_eq!(s.tasks[0].label, "Log in");
            assert_eq!(s.tasks[6].label, "Sign off");
        }
        let a = StoryLogic::new(Options { order: Some(1), ..Options::night(5, 3) });
        assert_eq!(a.order_idx, 1);
        assert_eq!(a.tasks[3].kind, TaskKind::Hold, "night 5 order 2 does the lift fourth");
    }

    #[test]
    fn dial_puzzles_never_start_solved() {
        for seed in 0..300 {
            let mut s = StoryLogic::new(Options { order: Some(0), ..Options::night(2, seed) });
            let p = s.make_puzzle(1);
            match p {
                Puzzle::Dials { n, target, pos } => {
                    assert_eq!(n as usize, target.len());
                    for i in 0..target.len() {
                        assert_ne!(pos[i], target[i], "seed {seed} dial {i}");
                        assert!(pos[i] >= 0 && pos[i] < 6 && target[i] >= 0 && target[i] < 6);
                    }
                }
                other => panic!("night 2 task 2 is dials, got {other:?}"),
            }
        }
    }

    #[test]
    fn switch_orders_are_permutations_and_a_wrong_flip_resets() {
        let mut s = sim(2, 11);
        s.room = "breaker".to_string();
        s.task_idx = s.tasks.iter().position(|t| t.kind == TaskKind::Switches).unwrap_or(0);
        s.puzzle = s.make_puzzle(s.task_idx);
        let (order, n) = match &s.puzzle {
            Puzzle::Switches { order, n, .. } => (order.clone(), *n),
            other => panic!("expected switches, got {other:?}"),
        };
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, (0..n).collect::<Vec<_>>());
        // two right flips, then a wrong one
        let ev = s.interact(Interact::index(order[0]));
        assert_eq!(ev, vec![SimEvent::Step(TaskKind::Switches)]);
        s.interact(Interact::index(order[1]));
        if let Puzzle::Switches { next, .. } = &s.puzzle {
            assert_eq!(*next, 2);
        }
        let wrong = (0..n).find(|i| *i != order[2]).unwrap_or(0);
        let ev = s.interact(Interact::index(wrong));
        assert_eq!(ev, vec![SimEvent::Wrong(TaskKind::Switches)]);
        if let Puzzle::Switches { next, .. } = &s.puzzle {
            assert_eq!(*next, 0, "the order starts over");
        }
        assert!(s.noise >= rules().t.wrong_noise);
    }

    #[test]
    fn the_code_must_be_keyed_in_order() {
        let mut s = sim(1, 5);
        let code = match &s.puzzle {
            Puzzle::Code { code, .. } => code.clone(),
            other => panic!("night 1 starts with a code, got {other:?}"),
        };
        assert_eq!(code.len(), 3);
        let first = code.chars().next().unwrap_or('0');
        let wrong = if first == '0' { '1' } else { '0' };
        assert_eq!(s.interact(Interact::digit(wrong)), vec![SimEvent::Wrong(TaskKind::Code)]);
        for c in code.chars() {
            s.interact(Interact::digit(c));
        }
        assert_eq!(s.task_idx, 1, "the code opened the next task");
    }

    #[test]
    fn a_creeper_at_your_throat_strikes_after_strike_ms() {
        let mut s = sim(2, 9);
        s.room = "gallery".to_string();
        s.opalie.dist = 0;
        let mut t = 0.0;
        let mut caught = false;
        while t < 1500.0 && !caught {
            let ev = s.step(50.0, quiet());
            t += 50.0;
            caught = ev.iter().any(|e| matches!(e, SimEvent::Caught(Who::Opalie)));
        }
        assert!(caught);
        assert!(t >= rules().tune_for(2, false).strike_ms - 50.0 && t <= 1300.0, "struck at {t} ms");
        assert_eq!(s.over, Over::Caught);
        assert_eq!(s.by, Some(Who::Opalie));
    }

    #[test]
    fn slipping_out_through_the_door_buys_a_step() {
        let mut s = sim(2, 9);
        s.room = "gallery".to_string();
        s.opalie.dist = 0;
        assert!(s.move_dir('R'));
        let mut ev = Vec::new();
        for _ in 0..12 {
            ev.extend(s.step(50.0, quiet()));
        }
        assert_eq!(s.over, Over::Running);
        assert_eq!(s.opalie.dist, 1, "the door took 500 ms, and she was a step back once it did");
        assert!(s.moving.is_some());
    }

    #[test]
    fn the_beacon_holds_a_creeper_and_pushes_porcelle_back() {
        let mut s = sim(3, 4);
        s.room = "workshop".to_string();
        s.porcelle.dist = 1;
        let mut pushed = false;
        for _ in 0..40 {
            let ev = s.step(50.0, SimInput { light: true, ..SimInput::default() });
            pushed |= ev.iter().any(|e| matches!(e, SimEvent::Pushed(Who::Porcelle)));
        }
        assert!(pushed, "2 s of beacon is past beaconPushMs");
        assert_eq!(s.porcelle.dist, 2);
        assert!(s.charge < 100.0, "the beacon runs down");
    }

    #[test]
    fn the_lift_rings_then_opens_then_leaves() {
        let mut s = sim(4, 8);
        assert!(s.level(Who::Quenelle) > 0);
        s.room = "breaker".to_string(); // nowhere near it, and nobody lives here
        s.quenelle.clock = 1.0e9;
        s.quenelle.gap = 10.0;
        let ev = s.step(50.0, quiet());
        assert!(ev.contains(&SimEvent::Bell));
        assert_eq!(s.quenelle.state, LiftState::Bell);
        let mut opened = false;
        for _ in 0..80 {
            opened |= s.step(50.0, quiet()).contains(&SimEvent::Open);
        }
        assert!(opened, "the bell lasts bellMs, then it opens");
        assert_eq!(s.quenelle.state, LiftState::Open);
        let mut left = false;
        for _ in 0..200 {
            left |= s.step(50.0, quiet()).contains(&SimEvent::Leave);
        }
        assert!(left);
        assert_eq!(s.quenelle.state, LiftState::Away);
        assert_eq!(s.over, Over::Running, "she saw nobody");
    }

    #[test]
    fn quenelle_sees_you_in_control_unless_you_hide() {
        let mut seen = sim(4, 8);
        seen.quenelle.state = LiftState::Open;
        seen.quenelle.open_for = 5000.0;
        let mut caught = false;
        for _ in 0..60 {
            caught |= seen.step(50.0, quiet()).contains(&SimEvent::Caught(Who::Quenelle));
        }
        assert!(caught, "1.5 s in view of the open lift");

        let mut hidden = sim(4, 8);
        hidden.quenelle.state = LiftState::Open;
        hidden.quenelle.open_for = 5000.0;
        for _ in 0..60 {
            hidden.step(50.0, SimInput { hide: true, ..SimInput::default() });
        }
        assert_eq!(hidden.over, Over::Running);
        assert!(hidden.hidden);
    }

    #[test]
    fn god_mode_sends_her_back_instead() {
        let mut s = sim(2, 9);
        s.god = true;
        s.room = "gallery".to_string();
        s.opalie.dist = 0;
        for _ in 0..40 {
            s.step(50.0, quiet());
        }
        assert_eq!(s.over, Over::Running);
        assert!(s.opalie.dist >= 1);
    }

    #[test]
    fn the_hold_task_needs_the_button_held_in_the_lift() {
        let mut s = sim(4, 3);
        s.room = "lift".to_string();
        s.jump_to_task(1);
        assert!(matches!(s.puzzle, Puzzle::Hold { .. }));
        s.quenelle.clock = -1.0e9; // keep the lift away
        for _ in 0..40 {
            s.step(50.0, quiet());
        }
        assert_eq!(s.task_idx, 1, "not holding does nothing");
        let mut done = false;
        for _ in 0..120 {
            done |= s.step(50.0, SimInput { work: true, ..SimInput::default() }).iter().any(|e| matches!(e, SimEvent::Task(2)));
        }
        assert!(done, "4.5 s of holding finishes it");
    }

    #[test]
    fn moving_takes_time_and_arrives() {
        let mut s = sim(1, 2);
        assert!(s.move_dir('R'));
        assert!(!s.move_dir('L'), "already walking");
        assert!(sim(1, 2).move_dir('C'), "control has a lift door");
        let mut arrived = None;
        for i in 0..60 {
            for e in s.step(50.0, quiet()) {
                if let SimEvent::Arrive(r) = e {
                    arrived = Some((i, r));
                }
            }
        }
        let (i, r) = arrived.unwrap_or((0, String::new()));
        assert_eq!(r, "workshop");
        assert!((34..=37).contains(&i), "1.8 s walking, arrived at step {i}");
        let mut c = sim(1, 2);
        c.set_crawl(true);
        c.move_dir('R');
        assert_eq!(c.moving.as_ref().map(|m| m.left), Some(3600.0), "crawling takes twice as long");
        assert_eq!(c.noise, 6.0, "a crawl is quiet");
    }

    #[test]
    fn the_shortest_way_between_rooms() {
        assert_eq!(next_dir("control", "gallery"), Some('L'));
        assert_eq!(next_dir("control", "breaker"), Some('L'));
        assert_eq!(next_dir("control", "auditorium"), Some('R'));
        assert_eq!(next_dir("breaker", "lift"), Some('R'));
        assert_eq!(next_dir("lift", "workshop"), Some('C'));
        assert_eq!(next_dir("lift", "lift"), None);
    }

    #[test]
    fn a_night_is_won_when_the_last_task_is_done() {
        let mut s = sim(1, 21);
        let mut guard = 0;
        while s.over == Over::Running && guard < 500 {
            guard += 1;
            if let Some(task) = s.task().cloned() {
                s.room = task.room.clone();
            }
            let act = match &s.puzzle {
                Puzzle::Code { code, entered, .. } => code.chars().nth(entered.chars().count()).map(Interact::digit),
                Puzzle::Dials { n, target, pos } => (0..*n as usize).find(|i| pos[*i] != target[*i]).map(|i| Interact::index(i as i32)),
                Puzzle::Collect { n, taken } => (0..*n).find(|i| !taken.contains(i)).map(Interact::index),
                _ => None,
            };
            match act {
                Some(a) => {
                    s.interact(a);
                }
                None => break,
            }
        }
        assert_eq!(s.over, Over::Win);
        assert_eq!(s.task_idx, s.tasks.len());
        assert_eq!(s.puzzle, Puzzle::None);
    }

    #[test]
    fn the_same_seed_plays_the_same_night() {
        for night in 1..=6 {
            let mut a = Session::new();
            a.start_mode("story", night, BTreeMap::new(), Some(12345));
            let mut x = StoryHeadless::new(&a);
            let mut y = StoryHeadless::new(&a);
            let (mut ex, mut ey) = (Vec::new(), Vec::new());
            for _ in 0..3000 {
                ex.extend(x.step_ms(100.0));
                ey.extend(y.step_ms(100.0));
                if x.outcome() != Outcome::Running {
                    break;
                }
            }
            assert_eq!(ex, ey, "night {night}");
            assert_eq!(x.digest(), y.digest());
            assert_eq!(x.outcome(), y.outcome());
        }
    }

    #[test]
    fn a_seeded_night_reaches_an_outcome() {
        for night in 1..=6 {
            let mut a = Session::new();
            a.start_mode("story", night, BTreeMap::new(), Some(12345));
            let mut h = StoryHeadless::new(&a);
            let mut t = 0.0;
            while h.outcome() == Outcome::Running && t < 1_800_000.0 {
                h.step_ms(100.0);
                t += 100.0;
            }
            assert_ne!(h.outcome(), Outcome::Running, "night {night} still running after {t} ms");
        }
        // night 1 has nobody awake: the scripted player always wins it
        let mut a = Session::new();
        a.start_mode("story", 1, BTreeMap::new(), Some(7));
        let mut h = StoryHeadless::new(&a);
        for _ in 0..18000 {
            h.step_ms(100.0);
            if h.outcome() != Outcome::Running {
                break;
            }
        }
        assert_eq!(h.outcome(), Outcome::Won);
    }

    #[test]
    fn different_seeds_draw_different_codes() {
        let codes: Vec<String> = (1..30)
            .map(|seed| match sim(6, seed).puzzle {
                Puzzle::Code { code, .. } => code,
                _ => String::new(),
            })
            .collect();
        let mut unique = codes.clone();
        unique.sort();
        unique.dedup();
        assert!(unique.len() > 10);
    }

    #[test]
    fn custom_nights_use_the_sliders() {
        let mut sess = Session::new();
        let mut sliders = BTreeMap::new();
        sliders.insert("opalie".to_string(), 0);
        sliders.insert("porcelle".to_string(), 15);
        sliders.insert("quenelle".to_string(), 0);
        sess.start_mode("story", 3, sliders, Some(99));
        let s = StoryLogic::new(Options::from_session(&sess));
        assert!(s.is_custom);
        assert_eq!(s.base_level(Who::Opalie), 0);
        assert_eq!(s.base_level(Who::Porcelle), 15);
        assert_eq!(s.level(Who::Opalie), 0);
        assert_eq!(s.tasks.len(), 7, "a Custom Night does night 6's list");
        assert_eq!(s.keen(), 0, "no keenness on a Custom Night");
    }

    #[test]
    fn levels_step_up_with_tasks_and_time() {
        let mut s = sim(2, 1);
        let base = s.level(Who::Opalie);
        s.task_idx = 4;
        assert_eq!(s.level(Who::Opalie), base + 2);
        s.elapsed = 65_000.0;
        assert_eq!(s.level(Who::Opalie), base + 2 + 2, "night 2 is keener every 30 s, up to 2");
        assert_eq!(s.level(Who::Quenelle), 0, "a hunter who is off stays off");
    }
}
