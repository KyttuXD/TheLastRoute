//! The Facility night type (Godot `game/night_watch/story/story_logic.gd`, tables from `story_data.gd`).
//! You walk the six rooms and do each night's tasks at their stations while Opalie, Porcelle and Quenelle close in.
//! Pure rules: the shell draws the building, feeds `Input` each frame and forwards doors and station presses.

use crate::framework::{pick_index, spread_ms, tempo, NightWatchStream};

pub const MODE_ID: &str = "story";

const SPREAD: f64 = 0.2;
const VARIETY_STEP_TEMPO: f64 = 0.1;
const VARIETY_START_NEARER: f64 = 0.1;
const VARIETY_START_FARTHER: f64 = 0.05;
const VARIETY_LIFT_GAP_SPREAD: f64 = 0.3;

const RECOVER_MS: f64 = 6000.0;
const WALK_MS: f64 = 1800.0;
const CRAWL_MS: f64 = 3600.0;
const DOOR_MS: f64 = 500.0;
const WALK_NOISE: f64 = 35.0;
const CRAWL_NOISE: f64 = 6.0;
const WORK_NOISE_PER_SEC: f64 = 6.0;
const NOISE_DECAY_PER_SEC: f64 = 12.0;
const WRONG_NOISE: f64 = 16.0;
const DIAL_STEPS: usize = 6;
const BEACON_DRAIN_PER_SEC: f64 = 18.0;
const BEACON_REGEN_PER_SEC: f64 = 3.0;
const BEACON_PUSH_MS: f64 = 1500.0;
const COMBO_LEVEL: i32 = 5;
const COMBO_MS: f64 = 1500.0;
const LIFT_BASE_MS: f64 = 30000.0;
const LIFT_PER_LEVEL_MS: f64 = 1000.0;
const LIFT_MIN_MS: f64 = 14000.0;
const BELL_MS: f64 = 3500.0;
const OPEN_MS: f64 = 5000.0;
const EXPOSE_MS: f64 = 1500.0;
const MAX_LEVEL: i32 = 20;
const MAX_SLIDER: u32 = 20;
const POINTS_PER_LEVEL: u32 = 10;

/// Each hunter's level per night 1 to 6 (Opalie, Porcelle, Quenelle).
const LEVELS: [[i32; 6]; 3] = [[0, 8, 10, 10, 11, 13], [0, 0, 10, 10, 10, 12], [0, 0, 0, 6, 9, 12]];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hunter {
    Opalie,
    Porcelle,
    Quenelle,
}

impl Hunter {
    pub const ALL: [Hunter; 3] = [Hunter::Opalie, Hunter::Porcelle, Hunter::Quenelle];

    pub fn name(self) -> &'static str {
        match self {
            Hunter::Opalie => "Opalie",
            Hunter::Porcelle => "Porcelle",
            Hunter::Quenelle => "Quenelle",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Room {
    Breaker,
    Gallery,
    Control,
    Workshop,
    Auditorium,
    Lift,
}

/// `Left` and `Right` are the side doors, `Lift` the lift door off Control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Door {
    Left,
    Right,
    Lift,
}

impl Room {
    pub fn label(self) -> &'static str {
        match self {
            Room::Breaker => "BREAKER ROOM",
            Room::Gallery => "GALLERY",
            Room::Control => "CONTROL",
            Room::Workshop => "WORKSHOP",
            Room::Auditorium => "AUDITORIUM",
            Room::Lift => "LIFT",
        }
    }

    pub fn exit(self, door: Door) -> Option<Room> {
        match (self, door) {
            (Room::Breaker, Door::Right) => Some(Room::Gallery),
            (Room::Gallery, Door::Left) => Some(Room::Breaker),
            (Room::Gallery, Door::Right) => Some(Room::Control),
            (Room::Control, Door::Left) => Some(Room::Gallery),
            (Room::Control, Door::Right) => Some(Room::Workshop),
            (Room::Control, Door::Lift) => Some(Room::Lift),
            (Room::Workshop, Door::Left) => Some(Room::Control),
            (Room::Workshop, Door::Right) => Some(Room::Auditorium),
            (Room::Auditorium, Door::Left) => Some(Room::Workshop),
            (Room::Lift, Door::Lift) => Some(Room::Control),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskKind {
    /// Key the code of this many digits on the note.
    Code(usize),
    /// Turn this many dials to their marks.
    Dials(usize),
    /// Take this many parts off the bench.
    Collect(usize),
    /// Turn the stage crank this many times.
    Crank(usize),
    /// Flip this many breakers in their numbered order.
    Switches(usize),
    /// Hold the lift's call button for this many milliseconds.
    Hold(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Code,
    Dials,
    Collect,
    Crank,
    Switches,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    pub room: Room,
    pub label: &'static str,
    pub kind: TaskKind,
}

fn task(room: Room, label: &'static str, kind: TaskKind) -> Task {
    Task { room, label, kind }
}

/// One night's full task list (the 1-based list number 6 is also the Custom Night's list).
fn night_tasks(list_night: usize) -> Vec<Task> {
    match list_night {
        1 => vec![
            task(Room::Control, "Log in", TaskKind::Code(3)),
            task(Room::Gallery, "Calibrate the lamps", TaskKind::Dials(3)),
            task(Room::Workshop, "Collect the parts", TaskKind::Collect(4)),
            task(Room::Control, "Sign off", TaskKind::Code(3)),
        ],
        2 => vec![
            task(Room::Control, "Log in", TaskKind::Code(3)),
            task(Room::Gallery, "Calibrate the lamps", TaskKind::Dials(3)),
            task(Room::Breaker, "Reset the breaker", TaskKind::Switches(4)),
            task(Room::Gallery, "Check the lamps", TaskKind::Dials(4)),
            task(Room::Control, "Sign off", TaskKind::Code(4)),
        ],
        3 => vec![
            task(Room::Control, "Log in", TaskKind::Code(3)),
            task(Room::Workshop, "Collect the parts", TaskKind::Collect(5)),
            task(Room::Auditorium, "Service the stage", TaskKind::Crank(14)),
            task(Room::Gallery, "Check the lamps", TaskKind::Dials(4)),
            task(Room::Control, "Sign off", TaskKind::Code(4)),
        ],
        4 => vec![
            task(Room::Control, "Log in", TaskKind::Code(4)),
            task(Room::Lift, "Check the floor above", TaskKind::Hold(4500)),
            task(Room::Breaker, "Reset the breaker", TaskKind::Switches(5)),
            task(Room::Gallery, "Check the lamps", TaskKind::Dials(4)),
            task(Room::Control, "Sign off", TaskKind::Code(4)),
        ],
        5 => vec![
            task(Room::Control, "Log in", TaskKind::Code(4)),
            task(Room::Auditorium, "Service the stage", TaskKind::Crank(14)),
            task(Room::Breaker, "Reset the breaker", TaskKind::Switches(5)),
            task(Room::Lift, "Check the floor above", TaskKind::Hold(4500)),
            task(Room::Gallery, "Calibrate the lamps", TaskKind::Dials(4)),
            task(Room::Control, "Sign off", TaskKind::Code(4)),
        ],
        _ => vec![
            task(Room::Control, "Log in", TaskKind::Code(5)),
            task(Room::Workshop, "Collect the parts", TaskKind::Collect(6)),
            task(Room::Auditorium, "Service the stage", TaskKind::Crank(16)),
            task(Room::Breaker, "Reset the breaker", TaskKind::Switches(6)),
            task(Room::Lift, "Check the floor above", TaskKind::Hold(5000)),
            task(Room::Gallery, "Calibrate the lamps", TaskKind::Dials(5)),
            task(Room::Control, "Sign off", TaskKind::Code(5)),
        ],
    }
}

/// The orders a night's list may be done in, as indexes into `night_tasks`. Log in stays first and Sign off last.
fn task_orders(list_night: usize) -> &'static [&'static [usize]] {
    match list_night {
        1 => &[&[0, 1, 2, 3]],
        2 => &[&[0, 1, 2, 3, 4], &[0, 1, 3, 2, 4], &[0, 2, 1, 3, 4]],
        3 => &[&[0, 1, 2, 3, 4], &[0, 2, 1, 3, 4]],
        4 => &[&[0, 1, 2, 3, 4]],
        5 => &[&[0, 1, 2, 3, 4, 5], &[0, 1, 4, 3, 2, 5]],
        _ => &[&[0, 1, 2, 3, 4, 5, 6], &[0, 1, 2, 5, 4, 3, 6]],
    }
}

/// Per-night tuning, one row per night 1 to 6; row 6 is also used for Custom Night.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tune {
    start_dist: i32,
    pace_x: f64,
    hear: f64,
    quiet_x: f64,
    recover_x: f64,
    away_pace_x: f64,
    strike_ms: f64,
    drain_x: f64,
    keen_ms: f64,
    keen_cap: i32,
    lift_gap_x: f64,
    open_x: f64,
    lift_sees: bool,
}

const NIGHTS: [Tune; 6] = [
    Tune { start_dist: 3, pace_x: 1.0, hear: 24.0, quiet_x: 0.0, recover_x: 1.0, away_pace_x: 0.0, strike_ms: 1200.0, drain_x: 1.0, keen_ms: 0.0, keen_cap: 0, lift_gap_x: 1.0, open_x: 1.0, lift_sees: false },
    Tune { start_dist: 2, pace_x: 2.2, hear: 15.0, quiet_x: 1.2, recover_x: 1.5, away_pace_x: 0.8, strike_ms: 1200.0, drain_x: 1.4, keen_ms: 30000.0, keen_cap: 2, lift_gap_x: 1.0, open_x: 1.0, lift_sees: false },
    Tune { start_dist: 2, pace_x: 2.7, hear: 12.0, quiet_x: 1.5, recover_x: 1.6, away_pace_x: 0.9, strike_ms: 1200.0, drain_x: 1.5, keen_ms: 25000.0, keen_cap: 3, lift_gap_x: 1.0, open_x: 1.0, lift_sees: false },
    Tune { start_dist: 2, pace_x: 2.75, hear: 11.5, quiet_x: 1.55, recover_x: 1.7, away_pace_x: 1.0, strike_ms: 1150.0, drain_x: 1.6, keen_ms: 22000.0, keen_cap: 3, lift_gap_x: 0.9, open_x: 1.0, lift_sees: true },
    Tune { start_dist: 2, pace_x: 2.8, hear: 11.0, quiet_x: 1.6, recover_x: 1.8, away_pace_x: 1.1, strike_ms: 1150.0, drain_x: 1.7, keen_ms: 20000.0, keen_cap: 4, lift_gap_x: 0.8, open_x: 1.15, lift_sees: true },
    Tune { start_dist: 2, pace_x: 3.2, hear: 9.0, quiet_x: 2.6, recover_x: 2.0, away_pace_x: 1.4, strike_ms: 1100.0, drain_x: 1.9, keen_ms: 18000.0, keen_cap: 4, lift_gap_x: 0.7, open_x: 1.3, lift_sees: true },
];

fn tune_for(night: u32, custom: bool) -> Tune {
    let index = if custom { 5 } else { (night.clamp(1, 6) - 1) as usize };
    NIGHTS[index]
}

fn step_noise(kind: StepKind) -> f64 {
    match kind {
        StepKind::Code => 4.0,
        StepKind::Dials => 8.0,
        StepKind::Collect => 10.0,
        StepKind::Crank => 12.0,
        StepKind::Switches => 7.0,
    }
}

/// How long a hunter waits between steps at a level: a window that slides down with the level (the 2D `move_window`).
fn move_window(level: i32) -> (f64, f64) {
    let lo = (13000.0 - f64::from(level) * 500.0).max(2500.0);
    (lo, lo + 3000.0)
}

fn move_draw(level: i32, rng: &mut NightWatchStream) -> f64 {
    let (lo, hi) = move_window(level);
    lo + rng.next() * (hi - lo)
}

/// How often a hunter moves (the 2D `createPacer`): a random wait from `move_window`, drawn from the night's main stream.
#[derive(Clone, Debug, Default)]
struct Pacer {
    t: f64,
    next: Option<f64>,
}

impl Pacer {
    fn step(&mut self, dt_ms: f64, level: i32, rng: &mut NightWatchStream) -> bool {
        if level <= 0 {
            self.t = 0.0;
            self.next = None;
            return false;
        }
        let due = match self.next {
            Some(due) => due,
            None => {
                let due = move_draw(level, rng);
                self.next = Some(due);
                due
            }
        };
        self.t += dt_ms;
        if self.t < due {
            return false;
        }
        self.t = 0.0;
        self.next = Some(move_draw(level, rng));
        true
    }

    fn skip(&mut self, ms: f64) {
        self.t += ms;
    }
}

#[derive(Clone, Debug)]
struct Creeper {
    dist: i32,
    pacer: Pacer,
    strike: f64,
    away: f64,
    recover: f64,
    push: f64,
    alert: f64,
    tempo: f64,
}

fn start_offset(night_index: usize, custom: bool, vr: &mut NightWatchStream) -> i32 {
    let u = vr.next();
    if vr.is_neutral() || (night_index == 0 && !custom) {
        return 0;
    }
    if u < VARIETY_START_NEARER {
        -1
    } else if u >= 1.0 - VARIETY_START_FARTHER {
        1
    } else {
        0
    }
}

fn recover_draw(tune: &Tune, rng: &mut NightWatchStream) -> f64 {
    spread_ms(RECOVER_MS * tune.recover_x, rng, SPREAD)
}

fn new_creeper(tune: &Tune, night_index: usize, custom: bool, rng: &mut NightWatchStream, vr: &mut NightWatchStream) -> Creeper {
    let dist = tune.start_dist + start_offset(night_index, custom, vr);
    let recover = recover_draw(tune, rng);
    Creeper {
        dist,
        pacer: Pacer::default(),
        strike: 0.0,
        away: 0.0,
        recover,
        push: 0.0,
        alert: 0.0,
        tempo: tempo(vr, VARIETY_STEP_TEMPO),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Puzzle {
    None,
    Code { code: Vec<usize>, entered: usize },
    Dials { target: Vec<usize>, pos: Vec<usize> },
    Collect { n: usize, taken: Vec<usize> },
    Crank { n: usize, done: usize },
    Switches { order: Vec<usize>, next: usize },
    Hold,
}

/// One action at the current task's station.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Digit(usize),
    Index(usize),
    Crank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseKind {
    Noticed,
    Quiet,
    Away,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Arrive(Room),
    Close { hunter: Hunter, dist: i32, kind: CloseKind },
    Back { hunter: Hunter, dist: i32 },
    Pushed(Hunter),
    Alert(Hunter),
    Bell(Hunter),
    Open(Hunter),
    Leave(Hunter),
    Station { kind: StepKind, ok: bool },
    Deny,
    TaskDone(usize),
    Caught(Hunter),
    Win,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Caught(Hunter),
}

/// The player's held controls this step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub work: bool,
    pub light: bool,
    pub hide: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walk {
    pub to: Room,
    pub left: f64,
    pub door: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiftPhase {
    Away,
    Bell,
    Open,
}

#[derive(Clone, Debug)]
struct Lift {
    phase: LiftPhase,
    t: f64,
    clock: f64,
    gap: Option<f64>,
    expose: f64,
    open_for: f64,
}

#[derive(Clone, Debug)]
pub struct FacilityOptions {
    pub night: u32,
    pub seed: i64,
    pub custom: bool,
    /// A hunter's level override per Opalie, Porcelle, Quenelle (a negative value keeps the night's level).
    pub levels: [Option<i32>; 3],
    pub infinite_beacon: bool,
    /// A task order index, for tests and replays; otherwise the night draws one.
    pub order: Option<usize>,
    pub variety: Option<NightWatchStream>,
}

impl FacilityOptions {
    pub fn new(night: u32, seed: i64) -> Self {
        Self { night, seed, custom: false, levels: [None; 3], infinite_beacon: false, order: None, variety: None }
    }
}

/// Custom Night score: each hunter's slider (0 to 20) summed, times 10 points per level.
pub fn custom_score(sliders: [u32; 3]) -> u32 {
    sliders.iter().map(|s| (*s).min(MAX_SLIDER)).sum::<u32>() * POINTS_PER_LEVEL
}

#[derive(Clone, Debug)]
pub struct Facility {
    pub elapsed: f64,
    pub over: Option<Outcome>,
    pub room: Room,
    pub moving: Option<Walk>,
    pub crawl: bool,
    pub hidden: bool,
    pub light: bool,
    pub working: bool,
    pub noise: f64,
    pub charge: f64,
    pub task_idx: usize,
    pub tasks: Vec<Task>,
    /// Admin: the hunter that would have caught you is sent back instead.
    pub god: bool,
    /// Admin: multiplies every hunter's step clock (1.0 is the 2D numbers).
    pub hunter_speed: f64,
    custom: bool,
    night_index: usize,
    infinite_beacon: bool,
    levels: [Option<i32>; 3],
    tune: Tune,
    rng: NightWatchStream,
    vr: NightWatchStream,
    task_ms: f64,
    puzzle: Puzzle,
    opalie: Creeper,
    porcelle: Creeper,
    lift: Lift,
}

impl Facility {
    pub fn start(options: FacilityOptions) -> Self {
        let night = options.night.clamp(1, 6);
        let night_index = (night - 1) as usize;
        let custom = options.custom;
        let mut rng = NightWatchStream::seeded(options.seed);
        let mut vr = NightWatchStream::variety_of(&rng, options.variety);
        let list_night = if custom { 6 } else { night_index + 1 };
        let orders = task_orders(list_night);
        let drawn = pick_index(&mut vr, orders.len());
        let order_idx = match options.order {
            Some(given) if given < orders.len() => given,
            _ => drawn,
        };
        let base = night_tasks(list_night);
        let tasks: Vec<Task> = orders[order_idx].iter().map(|&i| base[i]).collect();
        let tune = tune_for(night, custom);
        let opalie = new_creeper(&tune, night_index, custom, &mut rng, &mut vr);
        let porcelle = new_creeper(&tune, night_index, custom, &mut rng, &mut vr);
        let mut sim = Self {
            elapsed: 0.0,
            over: None,
            room: Room::Control,
            moving: None,
            crawl: false,
            hidden: false,
            light: false,
            working: false,
            noise: 0.0,
            charge: 100.0,
            task_idx: 0,
            tasks,
            god: false,
            hunter_speed: 1.0,
            custom,
            night_index,
            infinite_beacon: options.infinite_beacon,
            levels: options.levels,
            tune,
            rng,
            vr,
            task_ms: 0.0,
            puzzle: Puzzle::None,
            opalie,
            porcelle,
            lift: Lift { phase: LiftPhase::Away, t: 0.0, clock: 0.0, gap: None, expose: 0.0, open_for: OPEN_MS },
        };
        let first = sim.tasks[0];
        sim.puzzle = sim.make_puzzle(first);
        sim
    }

    pub fn current_task(&self) -> Option<Task> {
        self.tasks.get(self.task_idx).copied()
    }

    pub fn level(&self, hunter: Hunter) -> i32 {
        let base = self.base_level(hunter);
        if base <= 0 {
            return 0;
        }
        (base + (self.task_idx / 2).min(2) as i32 + self.keen()).min(MAX_LEVEL)
    }

    pub fn active_hunters(level_idx: usize) -> Vec<Hunter> {
        Hunter::ALL.into_iter().filter(|h| LEVELS[h.index()][level_idx.min(5)] > 0).collect()
    }

    fn base_level(&self, hunter: Hunter) -> i32 {
        match self.levels[hunter.index()] {
            Some(level) if level >= 0 => level,
            _ => LEVELS[hunter.index()][self.night_index],
        }
    }

    fn keen(&self) -> i32 {
        if self.custom || self.tune.keen_ms <= 0.0 {
            return 0;
        }
        let steps = (self.elapsed / self.tune.keen_ms).floor() as i32;
        steps.min(self.tune.keen_cap)
    }

    fn pick(&mut self, count: usize) -> usize {
        (self.rng.next() * count as f64).floor() as usize
    }

    fn make_puzzle(&mut self, task: Task) -> Puzzle {
        match task.kind {
            TaskKind::Code(n) => {
                let code = (0..n).map(|_| self.pick(10)).collect();
                Puzzle::Code { code, entered: 0 }
            }
            TaskKind::Dials(n) => {
                let target: Vec<usize> = (0..n).map(|_| self.pick(DIAL_STEPS)).collect();
                let pos = target.iter().map(|&x| (x + 1 + self.pick(DIAL_STEPS - 1)) % DIAL_STEPS).collect();
                Puzzle::Dials { target, pos }
            }
            TaskKind::Collect(n) => Puzzle::Collect { n, taken: Vec::new() },
            TaskKind::Crank(n) => Puzzle::Crank { n, done: 0 },
            TaskKind::Switches(n) => {
                let mut order: Vec<usize> = (0..n).collect();
                for i in (1..n).rev() {
                    let j = self.pick(i + 1);
                    order.swap(i, j);
                }
                Puzzle::Switches { order, next: 0 }
            }
            TaskKind::Hold(_) => Puzzle::Hold,
        }
    }

    /// Advances the shift by `dt_ms` with the player's held controls. Returns the events of this step.
    pub fn step(&mut self, dt_ms: f64, input: Input) -> Vec<Event> {
        let mut ev = Vec::new();
        if self.over.is_some() {
            return ev;
        }
        self.elapsed += dt_ms;
        let dts = dt_ms / 1000.0;

        let mut through = false;
        let mut arrived = None;
        if let Some(walk) = self.moving.as_mut() {
            if walk.door > 0.0 {
                walk.door -= dt_ms;
                through = walk.door <= 0.0;
            }
            walk.left -= dt_ms;
            if walk.left <= 0.0 {
                arrived = Some(walk.to);
            }
        }
        if let Some(room) = arrived {
            self.room = room;
            self.moving = None;
            ev.push(Event::Arrive(room));
        }

        let still = self.moving.is_none();
        let there = still || self.moving.as_ref().is_some_and(|walk| walk.door > 0.0);
        self.hidden = still && self.room == Room::Control && input.hide;
        self.light = input.light && still && self.charge > 0.0;
        if self.light {
            if !self.infinite_beacon {
                self.charge = (self.charge - BEACON_DRAIN_PER_SEC * self.tune.drain_x * dts).max(0.0);
            }
        } else {
            self.charge = (self.charge + BEACON_REGEN_PER_SEC * dts).min(100.0);
        }

        self.noise = (self.noise - NOISE_DECAY_PER_SEC * dts).max(0.0);
        let task = self.current_task();
        self.working = still
            && !self.hidden
            && input.work
            && task.is_some_and(|t| t.room == self.room && matches!(t.kind, TaskKind::Hold(_)));
        if self.working {
            self.task_ms += dt_ms;
            self.noise = (self.noise + WORK_NOISE_PER_SEC * dts).min(100.0);
            if let Some(Task { kind: TaskKind::Hold(ms), .. }) = task {
                if self.task_ms >= f64::from(ms) {
                    self.complete_task(&mut ev);
                }
            }
            if self.over.is_some() {
                return ev;
            }
        }

        let before = ev.len();
        let opalie_present = there && self.room == Room::Gallery;
        if self.creep(Hunter::Opalie, opalie_present, dt_ms, &mut ev) {
            return self.finish(ev, Hunter::Opalie);
        }
        let closed = ev[before..].iter().any(|e| matches!(e, Event::Close { hunter: Hunter::Opalie, .. }));
        if closed && self.level(Hunter::Opalie) >= COMBO_LEVEL && self.level(Hunter::Porcelle) >= COMBO_LEVEL {
            self.porcelle.alert = COMBO_MS;
            ev.push(Event::Alert(Hunter::Porcelle));
        }

        let porcelle_present = there && matches!(self.room, Room::Workshop | Room::Auditorium);
        if self.creep(Hunter::Porcelle, porcelle_present, dt_ms, &mut ev) {
            return self.finish(ev, Hunter::Porcelle);
        }
        if through {
            self.slip();
        }

        if self.step_lift(dt_ms, there, &mut ev) {
            return self.finish(ev, Hunter::Quenelle);
        }
        ev
    }

    /// Quenelle's bell, opening and leaving. True when she has caught you.
    fn step_lift(&mut self, dt_ms: f64, there: bool, ev: &mut Vec<Event>) -> bool {
        let q_level = self.level(Hunter::Quenelle);
        if q_level <= 0 {
            return false;
        }
        self.lift.clock += dt_ms;
        let gap = match self.lift.gap {
            Some(gap) => gap,
            None => {
                let base = (LIFT_BASE_MS - f64::from(q_level) * LIFT_PER_LEVEL_MS).max(LIFT_MIN_MS);
                let draw = 1.0 - VARIETY_LIFT_GAP_SPREAD + 2.0 * VARIETY_LIFT_GAP_SPREAD * self.rng.next();
                let gap = base * self.tune.lift_gap_x * draw;
                self.lift.gap = Some(gap);
                gap
            }
        };
        match self.lift.phase {
            LiftPhase::Away => {
                if self.lift.clock >= gap {
                    self.lift.phase = LiftPhase::Bell;
                    self.lift.t = 0.0;
                    self.lift.clock = 0.0;
                    self.lift.gap = None;
                    ev.push(Event::Bell(Hunter::Quenelle));
                }
            }
            LiftPhase::Bell => {
                self.lift.t += dt_ms;
                if self.lift.t >= BELL_MS {
                    self.lift.phase = LiftPhase::Open;
                    self.lift.t = 0.0;
                    self.lift.expose = 0.0;
                    self.lift.open_for = spread_ms(OPEN_MS * self.tune.open_x, &mut self.rng, SPREAD);
                    ev.push(Event::Open(Hunter::Quenelle));
                }
            }
            LiftPhase::Open => {
                self.lift.t += dt_ms;
                let seen = there
                    && ((self.room == Room::Control && !self.hidden) || (self.room == Room::Lift && self.tune.lift_sees));
                self.lift.expose = if seen { self.lift.expose + dt_ms } else { (self.lift.expose - dt_ms).max(0.0) };
                if self.lift.expose >= EXPOSE_MS {
                    return true;
                }
                if self.lift.t >= self.lift.open_for {
                    self.lift.phase = LiftPhase::Away;
                    self.lift.t = 0.0;
                    self.lift.expose = 0.0;
                    self.lift.clock = 0.0;
                    self.lift.gap = None;
                    ev.push(Event::Leave(Hunter::Quenelle));
                }
            }
        }
        false
    }

    /// Opalie and Porcelle close in a step at a time while you are in their rooms; once you leave they step back.
    /// True when she has reached you and struck.
    fn creep(&mut self, hunter: Hunter, present: bool, dt_ms: f64, ev: &mut Vec<Event>) -> bool {
        let level = self.level(hunter);
        let noise = self.noise;
        let light = self.light;
        let tune = self.tune;
        let speed = self.hunter_speed;
        let is_opalie = hunter == Hunter::Opalie;
        let noticed_by_noise = !is_opalie || noise > tune.hear;
        let c = if is_opalie { &mut self.opalie } else { &mut self.porcelle };

        if !present {
            c.strike = 0.0;
            c.push = 0.0;
            if tune.away_pace_x > 0.0 && c.dist > 1 {
                c.away = 0.0;
                if noticed_by_noise
                    && c.pacer.step(dt_ms * tune.away_pace_x * c.tempo * speed, level, &mut self.rng)
                {
                    c.dist -= 1;
                    c.tempo = tempo(&mut self.vr, VARIETY_STEP_TEMPO);
                    ev.push(Event::Close { hunter, dist: c.dist, kind: CloseKind::Away });
                }
                return false;
            }
            c.away += dt_ms;
            if c.away >= c.recover && c.dist < tune.start_dist {
                c.dist += 1;
                c.away = 0.0;
                c.recover = recover_draw(&tune, &mut self.rng);
                ev.push(Event::Back { hunter, dist: c.dist });
            }
            return false;
        }

        c.away = 0.0;
        if c.alert != 0.0 {
            c.pacer.skip(c.alert);
            c.alert = 0.0;
        }
        if light {
            if hunter == Hunter::Porcelle {
                c.push += dt_ms;
                if c.push >= BEACON_PUSH_MS {
                    c.push = 0.0;
                    if c.dist < tune.start_dist {
                        c.dist += 1;
                        ev.push(Event::Pushed(hunter));
                    }
                }
            }
        } else {
            c.push = 0.0;
            let pace = if noticed_by_noise { tune.pace_x } else { tune.quiet_x };
            if c.dist > 0 && pace > 0.0 && c.pacer.step(dt_ms * pace * c.tempo * speed, level, &mut self.rng) {
                c.dist -= 1;
                c.tempo = tempo(&mut self.vr, VARIETY_STEP_TEMPO);
                let kind = if noticed_by_noise { CloseKind::Noticed } else { CloseKind::Quiet };
                ev.push(Event::Close { hunter, dist: c.dist, kind });
            }
        }
        if c.dist <= 0 {
            c.strike += dt_ms;
            return c.strike >= tune.strike_ms;
        }
        c.strike = 0.0;
        false
    }

    /// The hunter reached you. With god mode on she is sent back and the night goes on.
    fn finish(&mut self, mut ev: Vec<Event>, who: Hunter) -> Vec<Event> {
        if self.god {
            match who {
                Hunter::Quenelle => {
                    self.lift.expose = 0.0;
                    self.lift.phase = LiftPhase::Away;
                    self.lift.t = 0.0;
                }
                _ => {
                    let c = if who == Hunter::Opalie { &mut self.opalie } else { &mut self.porcelle };
                    c.dist = 2;
                    c.strike = 0.0;
                }
            }
            return ev;
        }
        self.over = Some(Outcome::Caught(who));
        ev.push(Event::Caught(who));
        ev
    }

    /// Slipping out of a room with a hunter at your throat gives you a step of room.
    fn slip(&mut self) {
        if self.room == Room::Gallery && self.opalie.dist == 0 {
            self.opalie.dist = 1;
        }
        if matches!(self.room, Room::Workshop | Room::Auditorium) && self.porcelle.dist == 0 {
            self.porcelle.dist = 1;
        }
    }

    fn complete_task(&mut self, ev: &mut Vec<Event>) {
        self.task_ms = 0.0;
        self.task_idx += 1;
        ev.push(Event::TaskDone(self.task_idx));
        match self.current_task() {
            Some(next) => self.puzzle = self.make_puzzle(next),
            None => {
                self.over = Some(Outcome::Win);
                self.puzzle = Puzzle::None;
                ev.push(Event::Win);
            }
        }
    }

    /// One press at the current task's station. A wrong press starts the puzzle over and is loud.
    pub fn interact(&mut self, press: Press) -> Vec<Event> {
        if self.over.is_some() || self.moving.is_some() || self.hidden {
            return Vec::new();
        }
        let at_station = self.current_task().is_some_and(|t| t.room == self.room);
        if !at_station || matches!(self.puzzle, Puzzle::None | Puzzle::Hold) {
            return vec![Event::Deny];
        }
        let (kind, ok, done) = match (&mut self.puzzle, press) {
            (Puzzle::Code { code, entered }, Press::Digit(digit)) => {
                let ok = code[*entered] == digit;
                if ok {
                    *entered += 1;
                } else {
                    *entered = 0;
                }
                (StepKind::Code, ok, ok && *entered == code.len())
            }
            (Puzzle::Dials { target, pos }, Press::Index(i)) => {
                if i >= pos.len() {
                    return vec![Event::Deny];
                }
                pos[i] = (pos[i] + 1) % DIAL_STEPS;
                (StepKind::Dials, true, *pos == *target)
            }
            (Puzzle::Collect { n, taken }, Press::Index(i)) => {
                if i >= *n || taken.contains(&i) {
                    return vec![Event::Deny];
                }
                taken.push(i);
                (StepKind::Collect, true, taken.len() == *n)
            }
            (Puzzle::Crank { n, done }, Press::Crank) => {
                *done += 1;
                (StepKind::Crank, true, *done >= *n)
            }
            (Puzzle::Switches { order, next }, Press::Index(i)) => {
                if i >= order.len() {
                    return vec![Event::Deny];
                }
                let ok = i == order[*next];
                if ok {
                    *next += 1;
                } else {
                    *next = 0;
                }
                (StepKind::Switches, ok, ok && *next == order.len())
            }
            _ => return vec![Event::Deny],
        };
        let mut ev = Vec::new();
        let added = if ok { step_noise(kind) } else { WRONG_NOISE };
        self.noise = (self.noise + added).min(100.0);
        ev.push(Event::Station { kind, ok });
        if done {
            self.complete_task(&mut ev);
        }
        ev
    }

    /// Starts walking through a door. False when there is no door that way or you are already walking.
    pub fn walk_through(&mut self, door: Door) -> bool {
        if self.over.is_some() || self.moving.is_some() {
            return false;
        }
        let Some(to) = self.room.exit(door) else {
            return false;
        };
        self.hidden = false;
        let (noise, left) = if self.crawl { (CRAWL_NOISE, CRAWL_MS) } else { (WALK_NOISE, WALK_MS) };
        self.noise = (self.noise + noise).min(100.0);
        self.moving = Some(Walk { to, left, door: DOOR_MS });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_digit_restarts_the_code_and_is_loud() {
        let mut sim = Facility::start(FacilityOptions::new(1, 7));
        let Puzzle::Code { code, .. } = sim.puzzle.clone() else {
            panic!("night 1 opens on the Log in code");
        };
        let wrong = (code[0] + 1) % 10;
        let events = sim.interact(Press::Digit(wrong));
        assert_eq!(events, vec![Event::Station { kind: StepKind::Code, ok: false }]);
        assert_eq!(sim.noise, WRONG_NOISE);
        assert_eq!(sim.puzzle, Puzzle::Code { code, entered: 0 });
    }
}
