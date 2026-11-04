//! The Facility night type (Godot `game/night_watch/story/story_logic.gd`, tables from `story_data.gd`).
//! You walk the six rooms and do each night's tasks at their stations while Opalie, Porcelle and Quenelle close in.
//! Pure rules: the shell draws the building, feeds `Input` each frame and forwards doors and station presses.

use crate::data;
use crate::framework::{pick_index, spread_ms, tempo, NightWatchStream};
use std::sync::OnceLock;

pub const MODE_ID: &str = "story";
/// The id of this mode's files under `data/` (the mode id above is the Godot one).
const MODE_NAME: &str = "facility";

/// Every tunable of the mode, read once from `data/{rules,cast,buildings}/facility.json` (Godot `story_data.gd` / `story_logic.gd`).
struct Rules {
    spread: f64,
    step_tempo: f64,
    start_nearer: f64,
    start_farther: f64,
    lift_gap_spread: f64,
    recover_ms: f64,
    walk_ms: f64,
    crawl_ms: f64,
    door_ms: f64,
    beacon_push_ms: f64,
    combo_ms: f64,
    noise_max: f64,
    walk_noise: f64,
    crawl_noise: f64,
    work_noise_per_sec: f64,
    noise_decay_per_sec: f64,
    wrong_noise: f64,
    /// Indexed by `StepKind` (Code, Dials, Collect, Crank, Switches).
    step_noise: [f64; 5],
    charge_max: f64,
    beacon_drain_per_sec: f64,
    beacon_regen_per_sec: f64,
    dial_steps: usize,
    code_digits: usize,
    shock_enabled: bool,
    shock_cost: f64,
    shock_push: i32,
    shock_noise: f64,
    shock_cooldown_ms: f64,
    combo_level: i32,
    max_level: i32,
    custom_list_night: usize,
    #[cfg(test)]
    max_slider: u32,
    points_per_level: u32,
    god_reset_dist: i32,
    slip_dist: i32,
    task_level_every: usize,
    task_level_cap: usize,
    window_base_ms: f64,
    window_per_level_ms: f64,
    window_min_ms: f64,
    window_width_ms: f64,
    lift_base_ms: f64,
    lift_per_level_ms: f64,
    lift_min_ms: f64,
    bell_ms: f64,
    open_ms: f64,
    expose_ms: f64,
    /// Each hunter's level per night 1 to 6 (Opalie, Porcelle, Quenelle).
    levels: [Vec<i32>; 3],
    nights: Vec<Tune>,
    tasks: Vec<Vec<Task>>,
    orders: Vec<Vec<Vec<usize>>>,
    names: [&'static str; 3],
    room_labels: [&'static str; 6],
    /// Per room, the room behind its Left, Right and Lift door.
    exits: [[Option<Room>; 3]; 6],
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(Rules::load)
}

impl Rules {
    fn load() -> Rules {
        let r = data::rules(MODE_NAME);
        let n = |path: &str| data::num(r, path);
        let nights_col = |key: &str| -> Vec<f64> { (0..data::list(r, &format!("nights.{key}")).len()).map(|k| n(&format!("nights.{key}.{k}"))).collect() };
        let start_dist = nights_col("start_dist");
        let pace_x = nights_col("pace_x");
        let hear = nights_col("hear");
        let quiet_x = nights_col("quiet_x");
        let recover_x = nights_col("recover_x");
        let away_pace_x = nights_col("away_pace_x");
        let strike_ms = nights_col("strike_ms");
        let drain_x = nights_col("drain_x");
        let keen_ms = nights_col("keen_ms");
        let keen_cap = nights_col("keen_cap");
        let lift_gap_x = nights_col("lift_gap_x");
        let open_x = nights_col("open_x");
        let lift_sees = data::list(r, "nights.lift_sees");
        let nights: Vec<Tune> = (0..start_dist.len())
            .map(|k| Tune {
                start_dist: start_dist[k] as i32,
                pace_x: pace_x[k],
                hear: hear[k],
                quiet_x: quiet_x[k],
                recover_x: recover_x[k],
                away_pace_x: away_pace_x[k],
                strike_ms: strike_ms[k],
                drain_x: drain_x[k],
                keen_ms: keen_ms[k],
                keen_cap: keen_cap[k] as i32,
                lift_gap_x: lift_gap_x[k],
                open_x: open_x[k],
                lift_sees: lift_sees[k].as_bool().expect("night_watch data for mode 'facility': nights.lift_sees holds booleans"),
            })
            .collect();
        let level_row = |h: usize| -> Vec<i32> { (0..nights.len()).map(|k| data::int(r, &format!("levels.{h}.{k}")) as i32).collect() };
        let tasks = (0..data::list(r, "tasks").len())
            .map(|l| {
                data::list(r, &format!("tasks.{l}"))
                    .iter()
                    .map(|t| {
                        let kind = match data::text(t, "kind") {
                            "code" => TaskKind::Code(data::int(t, "n") as usize),
                            "dials" => TaskKind::Dials(data::int(t, "n") as usize),
                            "collect" => TaskKind::Collect(data::int(t, "n") as usize),
                            "crank" => TaskKind::Crank(data::int(t, "n") as usize),
                            "switches" => TaskKind::Switches(data::int(t, "n") as usize),
                            "hold" => TaskKind::Hold(data::int(t, "ms") as u32),
                            other => panic!("night_watch data for mode 'facility': unknown task kind '{other}'"),
                        };
                        task(Room::from_key(data::text(t, "room")), data::text(t, "label"), kind)
                    })
                    .collect()
            })
            .collect();
        let orders = (0..data::list(r, "task_orders").len())
            .map(|l| {
                data::list(r, &format!("task_orders.{l}"))
                    .iter()
                    .map(|order| order.as_array().expect("night_watch data for mode 'facility': a task order is a list").iter().map(|i| i.as_u64().expect("task order index") as usize).collect())
                    .collect()
            })
            .collect();
        let cast = data::cast(MODE_NAME);
        let mut names = [""; 3];
        for hunter in Hunter::ALL {
            let entry = &data::list(cast, "hunters")[hunter.index()];
            names[hunter.index()] = data::text(entry, "name");
        }
        let mut room_labels = [""; 6];
        let mut exits = [[None; 3]; 6];
        for room in data::list(data::building(MODE_NAME), "rooms") {
            let at = Room::from_key(data::text(room, "id")) as usize;
            room_labels[at] = data::text(room, "label");
            for (door, key) in ["left", "right", "lift"].into_iter().enumerate() {
                let to = data::text(room, &format!("exits.{key}"));
                if !to.is_empty() {
                    exits[at][door] = Some(Room::from_key(to));
                }
            }
        }
        Rules {
            spread: n("spread"),
            step_tempo: n("variety.step_tempo"),
            start_nearer: n("variety.start_nearer"),
            start_farther: n("variety.start_farther"),
            lift_gap_spread: n("variety.lift_gap_spread"),
            recover_ms: n("timing_ms.recover"),
            walk_ms: n("timing_ms.walk"),
            crawl_ms: n("timing_ms.crawl"),
            door_ms: n("timing_ms.door"),
            beacon_push_ms: n("timing_ms.beacon_push"),
            combo_ms: n("timing_ms.combo"),
            noise_max: n("noise.max"),
            walk_noise: n("noise.walk"),
            crawl_noise: n("noise.crawl"),
            work_noise_per_sec: n("noise.work_per_sec"),
            noise_decay_per_sec: n("noise.decay_per_sec"),
            wrong_noise: n("noise.wrong"),
            step_noise: ["code", "dials", "collect", "crank", "switches"].map(|k| n(&format!("noise.step.{k}"))),
            charge_max: n("beacon.max"),
            beacon_drain_per_sec: n("beacon.drain_per_sec"),
            beacon_regen_per_sec: n("beacon.regen_per_sec"),
            dial_steps: data::int(r, "puzzle.dial_steps") as usize,
            code_digits: data::int(r, "puzzle.code_digits") as usize,
            shock_enabled: n("shock.enabled") > 0.0,
            shock_cost: n("shock.cost"),
            shock_push: data::int(r, "shock.push_steps") as i32,
            shock_noise: n("shock.noise"),
            shock_cooldown_ms: n("shock.cooldown_ms"),
            combo_level: data::int(r, "combo_level") as i32,
            max_level: data::int(r, "max_level") as i32,
            custom_list_night: data::int(r, "custom.list_night") as usize,
            #[cfg(test)]
            max_slider: data::int(r, "custom.max_slider") as u32,
            points_per_level: data::int(r, "custom.points_per_level") as u32,
            god_reset_dist: data::int(r, "god_reset_dist") as i32,
            slip_dist: data::int(r, "slip_dist") as i32,
            task_level_every: data::int(r, "task_level.every_tasks") as usize,
            task_level_cap: data::int(r, "task_level.cap") as usize,
            window_base_ms: n("move_window.base_ms"),
            window_per_level_ms: n("move_window.per_level_ms"),
            window_min_ms: n("move_window.min_ms"),
            window_width_ms: n("move_window.width_ms"),
            lift_base_ms: n("lift.base_ms"),
            lift_per_level_ms: n("lift.per_level_ms"),
            lift_min_ms: n("lift.min_ms"),
            bell_ms: n("lift.bell_ms"),
            open_ms: n("lift.open_ms"),
            expose_ms: n("lift.expose_ms"),
            levels: [level_row(0), level_row(1), level_row(2)],
            nights,
            tasks,
            orders,
            names,
            room_labels,
            exits,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hunter {
    Opalie,
    Porcelle,
    Quenelle,
}

impl Hunter {
    pub const ALL: [Hunter; 3] = [Hunter::Opalie, Hunter::Porcelle, Hunter::Quenelle];

    pub fn name(self) -> &'static str {
        rules().names[self.index()]
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
    /// The room ids of `data/buildings/facility.json`.
    fn from_key(key: &str) -> Room {
        match key {
            "breaker" => Room::Breaker,
            "gallery" => Room::Gallery,
            "control" => Room::Control,
            "workshop" => Room::Workshop,
            "auditorium" => Room::Auditorium,
            "lift" => Room::Lift,
            other => panic!("night_watch data for mode 'facility': unknown room '{other}'"),
        }
    }

    pub fn label(self) -> &'static str {
        rules().room_labels[self as usize]
    }

    pub fn exit(self, door: Door) -> Option<Room> {
        rules().exits[self as usize][door as usize]
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
    let tasks = &rules().tasks;
    tasks[list_night.clamp(1, tasks.len()) - 1].clone()
}

/// The orders a night's list may be done in, as indexes into `night_tasks`. Log in stays first and Sign off last.
fn task_orders(list_night: usize) -> &'static [Vec<usize>] {
    let orders = &rules().orders;
    &orders[list_night.clamp(1, orders.len()) - 1]
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

fn tune_for(night: u32, custom: bool) -> Tune {
    let nights = &rules().nights;
    let index = if custom { rules().custom_list_night - 1 } else { (night.clamp(1, nights.len() as u32) - 1) as usize };
    nights[index]
}

fn step_noise(kind: StepKind) -> f64 {
    rules().step_noise[kind as usize]
}

/// How long a hunter waits between steps at a level: a window that slides down with the level (the 2D `move_window`).
fn move_window(level: i32) -> (f64, f64) {
    let k = rules();
    let lo = (k.window_base_ms - f64::from(level) * k.window_per_level_ms).max(k.window_min_ms);
    (lo, lo + k.window_width_ms)
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
    if u < rules().start_nearer {
        -1
    } else if u >= 1.0 - rules().start_farther {
        1
    } else {
        0
    }
}

fn recover_draw(tune: &Tune, rng: &mut NightWatchStream) -> f64 {
    spread_ms(rules().recover_ms * tune.recover_x, rng, rules().spread)
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
        tempo: tempo(vr, rules().step_tempo),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Puzzle {
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
pub enum LiftPhase {
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

#[cfg(test)]
/// Custom Night score: each hunter's slider (0 to 20) summed, times 10 points per level.
pub fn custom_score(sliders: [u32; 3]) -> u32 {
    sliders.iter().map(|s| (*s).min(rules().max_slider)).sum::<u32>() * rules().points_per_level
}

/// How many nights the Facility has in its data.
pub fn night_count() -> u32 {
    rules().nights.len() as u32
}

/// The top of the noise meter (the noise a shift can build to).
pub fn noise_max() -> f64 {
    rules().noise_max
}

/// The top of the beacon's charge.
pub fn charge_max() -> f64 {
    rules().charge_max
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
    /// Milliseconds until the shock button can fire again.
    pub shock_cd: f64,
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
        let k = rules();
        let night = options.night.clamp(1, k.nights.len() as u32);
        let night_index = (night - 1) as usize;
        let custom = options.custom;
        let mut rng = NightWatchStream::seeded(options.seed);
        let mut vr = NightWatchStream::variety_of(&rng, options.variety);
        let list_night = if custom { k.custom_list_night } else { night_index + 1 };
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
            charge: k.charge_max,
            shock_cd: 0.0,
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
            lift: Lift { phase: LiftPhase::Away, t: 0.0, clock: 0.0, gap: None, expose: 0.0, open_for: k.open_ms },
        };
        let first = sim.tasks[0];
        sim.puzzle = sim.make_puzzle(first);
        sim
    }

    pub fn current_task(&self) -> Option<Task> {
        self.tasks.get(self.task_idx).copied()
    }

    /// The station puzzle of the current task, for the driver to show and press.
    pub fn puzzle(&self) -> &Puzzle {
        &self.puzzle
    }

    /// A hunter's distance in steps from the player; None for Quenelle, who works the lift.
    pub fn distance(&self, hunter: Hunter) -> Option<i32> {
        match hunter {
            Hunter::Opalie => Some(self.opalie.dist),
            Hunter::Porcelle => Some(self.porcelle.dist),
            Hunter::Quenelle => None,
        }
    }

    /// The noise at which Opalie hears you on this night.
    pub fn hear(&self) -> f64 {
        self.tune.hear
    }

    /// How many steps a creeping hunter starts from you on this night.
    pub fn start_distance(&self) -> i32 {
        self.tune.start_dist
    }

    pub fn lift_phase(&self) -> LiftPhase {
        self.lift.phase
    }

    /// How long Quenelle has seen you at the open lift, 0 to 1 (you are taken at 1).
    pub fn lift_exposure(&self) -> f64 {
        (self.lift.expose / rules().expose_ms).clamp(0.0, 1.0)
    }

    /// Milliseconds the current hold task has been held so far.
    pub fn hold_ms(&self) -> f64 {
        self.task_ms
    }

    /// The score of a shift: 10 points per task on the list (read it on a win).
    pub fn score(&self) -> u32 {
        self.tasks.len() as u32 * rules().points_per_level
    }

    pub fn level(&self, hunter: Hunter) -> i32 {
        let base = self.base_level(hunter);
        if base <= 0 {
            return 0;
        }
        (base + (self.task_idx / rules().task_level_every).min(rules().task_level_cap) as i32 + self.keen()).min(rules().max_level)
    }

    #[cfg(test)]
    pub fn active_hunters(level_idx: usize) -> Vec<Hunter> {
        let levels = &rules().levels;
        Hunter::ALL.into_iter().filter(|h| levels[h.index()][level_idx.min(levels[h.index()].len() - 1)] > 0).collect()
    }

    fn base_level(&self, hunter: Hunter) -> i32 {
        match self.levels[hunter.index()] {
            Some(level) if level >= 0 => level,
            _ => rules().levels[hunter.index()][self.night_index],
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
                let code = (0..n).map(|_| self.pick(rules().code_digits)).collect();
                Puzzle::Code { code, entered: 0 }
            }
            TaskKind::Dials(n) => {
                let steps = rules().dial_steps;
                let target: Vec<usize> = (0..n).map(|_| self.pick(steps)).collect();
                let pos = target.iter().map(|&x| (x + 1 + self.pick(steps - 1)) % steps).collect();
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
        let k = rules();

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
                self.charge = (self.charge - k.beacon_drain_per_sec * self.tune.drain_x * dts).max(0.0);
            }
        } else {
            self.charge = (self.charge + k.beacon_regen_per_sec * dts).min(k.charge_max);
        }

        self.noise = (self.noise - k.noise_decay_per_sec * dts).max(0.0);
        self.shock_cd = (self.shock_cd - dt_ms).max(0.0);
        let task = self.current_task();
        self.working = still
            && !self.hidden
            && input.work
            && task.is_some_and(|t| t.room == self.room && matches!(t.kind, TaskKind::Hold(_)));
        if self.working {
            self.task_ms += dt_ms;
            self.noise = (self.noise + k.work_noise_per_sec * dts).min(k.noise_max);
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
        if closed && self.level(Hunter::Opalie) >= k.combo_level && self.level(Hunter::Porcelle) >= k.combo_level {
            self.porcelle.alert = k.combo_ms;
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
        let k = rules();
        self.lift.clock += dt_ms;
        let gap = match self.lift.gap {
            Some(gap) => gap,
            None => {
                let base = (k.lift_base_ms - f64::from(q_level) * k.lift_per_level_ms).max(k.lift_min_ms);
                let draw = 1.0 - k.lift_gap_spread + 2.0 * k.lift_gap_spread * self.rng.next();
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
                if self.lift.t >= k.bell_ms {
                    self.lift.phase = LiftPhase::Open;
                    self.lift.t = 0.0;
                    self.lift.expose = 0.0;
                    self.lift.open_for = spread_ms(k.open_ms * self.tune.open_x, &mut self.rng, k.spread);
                    ev.push(Event::Open(Hunter::Quenelle));
                }
            }
            LiftPhase::Open => {
                self.lift.t += dt_ms;
                let seen = there
                    && ((self.room == Room::Control && !self.hidden) || (self.room == Room::Lift && self.tune.lift_sees));
                self.lift.expose = if seen { self.lift.expose + dt_ms } else { (self.lift.expose - dt_ms).max(0.0) };
                if self.lift.expose >= k.expose_ms {
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
        let (step_tempo, push_ms) = (rules().step_tempo, rules().beacon_push_ms);
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
                    c.tempo = tempo(&mut self.vr, step_tempo);
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
                if c.push >= push_ms {
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
                c.tempo = tempo(&mut self.vr, step_tempo);
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
                    c.dist = rules().god_reset_dist;
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
            self.opalie.dist = rules().slip_dist;
        }
        if matches!(self.room, Room::Workshop | Room::Auditorium) && self.porcelle.dist == 0 {
            self.porcelle.dist = rules().slip_dist;
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
                pos[i] = (pos[i] + 1) % rules().dial_steps;
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
        let added = if ok { step_noise(kind) } else { rules().wrong_noise };
        self.noise = (self.noise + added).min(rules().noise_max);
        ev.push(Event::Station { kind, ok });
        if done {
            self.complete_task(&mut ev);
        }
        ev
    }

    /// The shock button: spends beacon charge and makes noise to knock back the hunter in your room.
    /// Denied when off, cooling down, moving, too weak, or nobody is here to shock.
    pub fn shock(&mut self) -> Vec<Event> {
        self.shock_with(rules())
    }

    fn shock_with(&mut self, k: &Rules) -> Vec<Event> {
        if self.over.is_some() || !k.shock_enabled || self.shock_cd > 0.0 || self.moving.is_some() || self.charge < k.shock_cost {
            return vec![Event::Deny];
        }
        let max = self.tune.start_dist;
        let target = match self.room {
            Room::Gallery => Some((Hunter::Opalie, &mut self.opalie)),
            Room::Workshop | Room::Auditorium => Some((Hunter::Porcelle, &mut self.porcelle)),
            _ => None,
        };
        let Some((hunter, c)) = target else {
            return vec![Event::Deny];
        };
        if c.dist >= max {
            return vec![Event::Deny];
        }
        c.dist = (c.dist + k.shock_push).min(max);
        c.strike = 0.0;
        if !self.infinite_beacon {
            self.charge -= k.shock_cost;
        }
        self.shock_cd = k.shock_cooldown_ms;
        self.noise = (self.noise + k.shock_noise).min(k.noise_max);
        vec![Event::Pushed(hunter)]
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
        let k = rules();
        let (noise, left) = if self.crawl { (k.crawl_noise, k.crawl_ms) } else { (k.walk_noise, k.walk_ms) };
        self.noise = (self.noise + noise).min(k.noise_max);
        self.moving = Some(Walk { to, left, door: k.door_ms });
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
        assert_eq!(sim.noise, rules().wrong_noise);
        assert_eq!(sim.puzzle, Puzzle::Code { code, entered: 0 });
    }

    #[test]
    fn shock_pushes_back_the_hunter_here_and_costs_charge() {
        let mut sim = Facility::start(FacilityOptions::new(2, 3));
        sim.room = Room::Gallery;
        sim.opalie.dist = 0;
        let (charge, noise) = (sim.charge, sim.noise);
        let mut on = Rules::load();
        on.shock_enabled = true;
        assert_eq!(sim.shock_with(&on), vec![Event::Pushed(Hunter::Opalie)]);
        assert!(sim.opalie.dist > 0 && sim.opalie.dist <= sim.tune.start_dist);
        assert!(sim.charge < charge && sim.noise > noise);
        assert!(sim.shock_cd > 0.0);
        assert_eq!(sim.shock_with(&on), vec![Event::Deny], "cooling down");
        sim.step(rules().shock_cooldown_ms + 1.0, Input::default());
        assert_eq!(sim.shock_cd, 0.0);
    }

    #[test]
    fn shock_is_on_in_the_facility_rules_and_shipped_from_data() {
        // The shock button belongs to The Facility (website src/modes/story); the switch and its numbers are data.
        let r = rules();
        assert!(r.shock_enabled, "the Facility ships the shock button; data/rules/facility.json shock.enabled");
        let mut off = Rules::load();
        off.shock_enabled = false;
        let mut sim = Facility::start(FacilityOptions::new(2, 3));
        sim.room = Room::Gallery;
        sim.opalie.dist = 0;
        let (charge, noise) = (sim.charge, sim.noise);
        assert_eq!(sim.shock_with(&off), vec![Event::Deny], "turning the data switch off denies the shock");
        assert_eq!((sim.charge, sim.noise, sim.opalie.dist), (charge, noise, 0));
    }

    #[test]
    fn shock_is_denied_with_nobody_here_or_no_charge() {
        let mut on = Rules::load();
        on.shock_enabled = true;
        let mut sim = Facility::start(FacilityOptions::new(2, 3));
        sim.room = Room::Control;
        assert_eq!(sim.shock_with(&on), vec![Event::Deny]);
        sim.room = Room::Gallery;
        sim.opalie.dist = 0;
        sim.charge = 0.0;
        assert_eq!(sim.shock_with(&on), vec![Event::Deny]);
    }

    #[test]
    fn the_json_carries_every_key_the_mode_reads() {
        // Loading panics naming the key when one is missing.
        let k = rules();
        assert_eq!(k.nights.len(), 6);
        assert_eq!(k.tasks.len(), 6);
        assert_eq!(k.orders.len(), 6);
        assert!(k.tasks.iter().zip(&k.orders).all(|(tasks, orders)| orders.iter().all(|o| o.iter().all(|&i| i < tasks.len()))));
        assert!(k.room_labels.iter().all(|l| !l.is_empty()));
    }

    #[test]
    fn loaded_values_equal_the_old_constants() {
        let k = rules();
        assert_eq!((k.recover_ms, k.walk_ms, k.crawl_ms, k.door_ms), (6000.0, 1800.0, 3600.0, 500.0));
        assert_eq!((k.walk_noise, k.crawl_noise, k.wrong_noise, k.dial_steps), (35.0, 6.0, 16.0, 6));
        assert_eq!((k.lift_base_ms, k.lift_min_ms, k.bell_ms, k.open_ms, k.expose_ms), (30000.0, 14000.0, 3500.0, 5000.0, 1500.0));
        assert_eq!((k.max_level, k.max_slider, k.points_per_level, k.combo_level), (20, 20, 10, 5));
        assert_eq!(k.levels[2], vec![0, 0, 0, 6, 9, 12]);
        assert_eq!(k.step_noise, [4.0, 8.0, 10.0, 12.0, 7.0]);
        let last = tune_for(6, false);
        assert_eq!((last.pace_x, last.hear, last.strike_ms, last.keen_cap, last.lift_sees), (3.2, 9.0, 1100.0, 4, true));
        assert_eq!(tune_for(1, true), last);
        assert_eq!(move_window(0), (13000.0, 16000.0));
        assert_eq!(move_window(40), (2500.0, 5500.0));
        assert_eq!(custom_score([20, 5, 100]), 450);
        assert_eq!(night_tasks(4)[1].kind, TaskKind::Hold(4500));
        assert_eq!(night_tasks(6).len(), 7);
        assert_eq!(Room::Control.exit(Door::Lift), Some(Room::Lift));
        assert_eq!(Room::Lift.exit(Door::Left), None);
        assert_eq!(Room::Breaker.label(), "BREAKER ROOM");
        assert_eq!(Hunter::Quenelle.name(), "Quenelle");
        assert_eq!(Facility::active_hunters(3), Hunter::ALL.to_vec());
        assert!(Facility::active_hunters(0).is_empty());
    }
}
