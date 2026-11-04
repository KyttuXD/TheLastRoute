//! Salvage Run, the day-shop and rack night of Night Watch (Godot `game/night_watch/dayshift`).
//! The rules and scoring of `dayshift_logic.gd`, with the data of `dayshift_data.gd`, number for number. The shell
//! builds a `SalvageRun` for the week, feeds `step` the doors, the lights and the rack each frame, and reads back the
//! events. The screens, the controls and the save are the driver in `modes_play/salvage_run.rs`.

use crate::data::{self, int, list, num, text};
use crate::framework::{jitter, pick_index, spread_ms, NightWatchStream};
use serde_json::Value;
use std::sync::OnceLock;

const KIND_IDS: [&str; 3] = ["scrap", "parts", "prize"];
const UPGRADE_IDS: [&str; 5] = ["hinges", "cell", "soles", "gloves", "bench"];
const HUNTER_IDS: [&str; 3] = ["ravelle", "ceryth", "ysolde"];

/// Every tunable of the mode, read once from `data/rules/salvage_run.json` (not on the hot path).
struct Tuning {
    night_ms: f64,
    hour_ms: f64,
    hours: i32,
    combo_level: i32,
    combo_ms: f64,
    at_door_by_week_ms: [f64; 5],
    at_door_ms: f64,
    leave_ms: f64,
    start_dist: i32,
    lead_steps: i32,
    repelled_dist: i32,
    wait_base_ms: f64,
    wait_per_level_ms: f64,
    wait_min_ms: f64,
    wait_spread_ms: f64,
    noise_per_sec: f64,
    noise_decay_per_sec: f64,
    noise_max: f64,
    loud: f64,
    uneasy: f64,
    loud_bonus: i32,
    uneasy_bonus: i32,
    bump_hour_1: i32,
    bump_1: i32,
    bump_hour_2: i32,
    bump_2: i32,
    power_start: f64,
    power_base: f64,
    power_per_device: f64,
    out_attack_ms: f64,
    slots: usize,
    spawn_ms: f64,
    spawn_spread: f64,
    first_spawn_ms: f64,
    first_spawn_spread: f64,
    expire_ms: f64,
    kind_value: [u32; 3],
    kind_hold_ms: [f64; 3],
    decks: Vec<DeckSpec>,
    upgrade_cost: [u32; 5],
    upgrade_name: [&'static str; 5],
    hinges_factor: f64,
    cell_factor: f64,
    soles_levels: i32,
    gloves_factor: f64,
    bench_factor: f64,
    window_gap_ms: f64,
    window_gap_per_level_ms: f64,
    window_gap_min_ms: f64,
    window_gap_spread: f64,
    knock_min_ms: f64,
    knock_max_ms: f64,
    present_ms: f64,
    present_spread: f64,
    expose_ms: f64,
    carry_cap: u32,
    budget: [u32; 5],
    quota: [u32; 5],
    max_level: i32,
    week_levels: [[i32; 5]; 3],
}

fn week_row<T>(r: &Value, path: &str, read: impl Fn(&Value) -> T) -> [T; 5] {
    let rows = list(r, path);
    assert!(rows.len() == 5, "night_watch data for mode 'salvage_run': '{path}' needs one entry per week");
    std::array::from_fn(|i| read(&rows[i]))
}

fn number(v: &Value) -> f64 {
    v.as_f64().expect("night_watch data for mode 'salvage_run': a week entry is not a number")
}

fn whole(v: &Value) -> u32 {
    v.as_u64().and_then(|n| u32::try_from(n).ok()).expect("night_watch data for mode 'salvage_run': a week entry is not a whole number")
}

fn level_of(v: &Value) -> i32 {
    v.as_i64().and_then(|n| i32::try_from(n).ok()).expect("night_watch data for mode 'salvage_run': a level is not an integer")
}

impl Tuning {
    fn load() -> Tuning {
        let r = data::rules("salvage_run");
        let night_ms = num(r, "night_ms");
        let hours = int(r, "hours") as i32;
        // Power was tuned for a longer night (Classic's eight minutes), so the draw is scaled to this one.
        let time_scale = night_ms / num(r, "power_scale_night_ms");
        let kind = |k: &str, field: &str| num(r, &format!("rack.kinds.{k}.{field}"));
        let upgrade = |u: &str, field: &str| format!("upgrades.{u}.{field}");
        Tuning {
            night_ms,
            hour_ms: night_ms / f64::from(hours),
            hours,
            combo_level: int(r, "doors.combo_level") as i32,
            combo_ms: num(r, "doors.combo_ms"),
            at_door_by_week_ms: week_row(r, "doors.at_door_by_week_ms", number),
            at_door_ms: num(r, "doors.at_door_ms"),
            leave_ms: num(r, "doors.leave_ms"),
            start_dist: int(r, "doors.start_dist") as i32,
            lead_steps: int(r, "doors.lead_steps") as i32,
            repelled_dist: int(r, "doors.repelled_dist") as i32,
            wait_base_ms: num(r, "pacer.wait_base_ms"),
            wait_per_level_ms: num(r, "pacer.wait_per_level_ms"),
            wait_min_ms: num(r, "pacer.wait_min_ms"),
            wait_spread_ms: num(r, "pacer.wait_spread_ms"),
            noise_per_sec: num(r, "noise.per_sec"),
            noise_decay_per_sec: num(r, "noise.decay_per_sec"),
            noise_max: num(r, "noise.max"),
            loud: num(r, "noise.loud"),
            uneasy: num(r, "noise.uneasy"),
            loud_bonus: int(r, "noise.loud_bonus") as i32,
            uneasy_bonus: int(r, "noise.uneasy_bonus") as i32,
            bump_hour_1: int(r, "ai_bump.hour_1") as i32,
            bump_1: int(r, "ai_bump.bump_1") as i32,
            bump_hour_2: int(r, "ai_bump.hour_2") as i32,
            bump_2: int(r, "ai_bump.bump_2") as i32,
            power_start: num(r, "power.start"),
            power_base: num(r, "power.base") / time_scale,
            power_per_device: num(r, "power.per_device") / time_scale,
            out_attack_ms: num(r, "power.out_attack_ms"),
            slots: int(r, "rack.slots") as usize,
            spawn_ms: num(r, "rack.spawn_ms"),
            spawn_spread: num(r, "rack.spawn_spread"),
            first_spawn_ms: num(r, "rack.first_spawn_ms"),
            first_spawn_spread: num(r, "rack.first_spawn_spread"),
            expire_ms: num(r, "rack.expire_ms"),
            kind_value: KIND_IDS.map(|k| kind(k, "value") as u32),
            kind_hold_ms: KIND_IDS.map(|k| kind(k, "hold_ms")),
            decks: list(r, "rack.decks")
                .iter()
                .map(|d| DeckSpec {
                    weight: whole(&d["weight"]),
                    counts: KIND_IDS.map(|k| d[k].as_u64().expect("night_watch data for mode 'salvage_run': a deck count is not a whole number") as usize),
                })
                .collect(),
            upgrade_cost: UPGRADE_IDS.map(|u| int(r, &upgrade(u, "cost")) as u32),
            upgrade_name: UPGRADE_IDS.map(|u| text(r, &upgrade(u, "name"))),
            hinges_factor: num(r, "upgrades.hinges.factor"),
            cell_factor: num(r, "upgrades.cell.factor"),
            soles_levels: int(r, "upgrades.soles.levels") as i32,
            gloves_factor: num(r, "upgrades.gloves.factor"),
            bench_factor: num(r, "upgrades.bench.factor"),
            window_gap_ms: num(r, "window.gap_ms"),
            window_gap_per_level_ms: num(r, "window.gap_per_level_ms"),
            window_gap_min_ms: num(r, "window.gap_min_ms"),
            window_gap_spread: num(r, "window.gap_spread"),
            knock_min_ms: num(r, "window.knock_min_ms"),
            knock_max_ms: num(r, "window.knock_max_ms"),
            present_ms: num(r, "window.present_ms"),
            present_spread: num(r, "window.present_spread"),
            expose_ms: num(r, "window.expose_ms"),
            carry_cap: int(r, "carry_cap") as u32,
            budget: week_row(r, "budget", whole),
            quota: week_row(r, "quota", whole),
            max_level: int(r, "max_level") as i32,
            week_levels: HUNTER_IDS.map(|h| week_row(r, &format!("hunters.{h}.week_levels"), level_of)),
        }
    }
}

fn tuning() -> &'static Tuning {
    static TUNING: OnceLock<Tuning> = OnceLock::new();
    TUNING.get_or_init(Tuning::load)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upgrade {
    Hinges,
    Cell,
    Soles,
    Gloves,
    Bench,
}

impl Upgrade {
    pub const ALL: [Upgrade; 5] = [Upgrade::Hinges, Upgrade::Cell, Upgrade::Soles, Upgrade::Gloves, Upgrade::Bench];

    pub fn cost(self) -> u32 {
        tuning().upgrade_cost[self as usize]
    }

    pub fn name(self) -> &'static str {
        tuning().upgrade_name[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hunter {
    Ravelle,
    Ceryth,
    Ysolde,
}

impl Hunter {
    pub const ALL: [Hunter; 3] = [Hunter::Ravelle, Hunter::Ceryth, Hunter::Ysolde];

    fn index(self) -> usize {
        self as usize
    }

    fn week_levels(self) -> [i32; 5] {
        tuning().week_levels[self.index()]
    }

    fn other_door(self) -> Hunter {
        match self {
            Hunter::Ravelle => Hunter::Ceryth,
            _ => Hunter::Ravelle,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Scrap,
    Parts,
    Prize,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Scrap, Kind::Parts, Kind::Prize];

    pub fn value(self) -> u32 {
        tuning().kind_value[self as usize]
    }

    fn hold_ms(self) -> f64 {
        tuning().kind_hold_ms[self as usize]
    }
}

/// One rack deck: how often it is the one dealt, and how many of each kind it holds (scrap, parts, prize).
struct DeckSpec {
    weight: u32,
    counts: [usize; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub slot: usize,
    pub kind: Kind,
    pub value: u32,
    need_ms: f64,
    progress_ms: f64,
    age_ms: f64,
}

impl Item {
    /// The share of the hold done so far, 0 to 1.
    pub fn progress(&self) -> f64 {
        (self.progress_ms / self.need_ms.max(1.0)).clamp(0.0, 1.0)
    }

    /// The share of the time before the piece slips off the rack, 0 to 1.
    pub fn age(&self) -> f64 {
        (self.age_ms / tuning().expire_ms).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Devices {
    pub door_l: bool,
    pub door_r: bool,
    pub light_l: bool,
    pub light_r: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub work: bool,
    pub target: Option<usize>,
    pub devices: Devices,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Win,
    Short,
    Caught(Hunter),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Item { slot: usize },
    Expire { slot: usize },
    Salvage { slot: usize, value: u32 },
    Close { id: Hunter, dist: i32 },
    Repelled { id: Hunter },
    Warned { id: Hunter },
    Knock,
    Window,
    Gone,
    Out,
    Caught { by: Hunter },
    Win { money: u32, quota: u32 },
    Short { money: u32, quota: u32 },
}

/// The per-night options the shell passes in: the bought upgrades, optional hunter level overrides (`None` keeps the
/// week's table), the Custom Night switches and the quota (`None` is the week's quota).
#[derive(Clone, Debug)]
pub struct Options {
    pub upgrades: Vec<Upgrade>,
    pub levels: [Option<i32>; 3],
    pub infinite_power: bool,
    pub custom: bool,
    pub quota: Option<u32>,
    pub hunter_speed: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            upgrades: Vec::new(),
            levels: [None; 3],
            infinite_power: false,
            custom: false,
            quota: None,
            hunter_speed: 1.0,
        }
    }
}

/// A hunter's move timer (Godot `night_watch_pacer_mulberry.gd`): after each move she waits a random time from a window
/// that slides down with her level. Level 0 never moves.
#[derive(Clone, Debug, Default)]
struct Pacer {
    t: f64,
    next_ms: Option<f64>,
}

impl Pacer {
    fn step(&mut self, rng: &mut NightWatchStream, dt_ms: f64, level: i32) -> bool {
        if level <= 0 {
            self.reset();
            return false;
        }
        let wait = match self.next_ms {
            Some(ms) => ms,
            None => draw_wait(rng, level),
        };
        self.t += dt_ms;
        if self.t < wait {
            self.next_ms = Some(wait);
            return false;
        }
        self.t = 0.0;
        self.next_ms = Some(draw_wait(rng, level));
        true
    }

    fn reset(&mut self) {
        self.t = 0.0;
        self.next_ms = None;
    }

    fn skip(&mut self, ms: f64) {
        self.t += ms;
    }
}

fn draw_wait(rng: &mut NightWatchStream, level: i32) -> f64 {
    let t = tuning();
    let lo = (t.wait_base_ms - f64::from(level) * t.wait_per_level_ms).max(t.wait_min_ms);
    lo + rng.next() * t.wait_spread_ms
}

/// A hunter who comes to a side door: how far she is from the steel door, her move timer, and how long she has stood
/// at it (and how long she has been turned away).
#[derive(Clone, Debug)]
struct Visitor {
    dist: i32,
    pacer: Pacer,
    at_ms: f64,
    leave_ms: f64,
}

impl Visitor {
    fn new(dist: i32) -> Self {
        Self { dist, pacer: Pacer::default(), at_ms: 0.0, leave_ms: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowState {
    Away,
    Knock,
    Here,
}

/// Ysolde at the back window: away, knocking, or at the glass.
#[derive(Clone, Debug)]
struct Window {
    state: WindowState,
    t: f64,
    clock: f64,
    gap: Option<f64>,
    expose: f64,
    stay: f64,
    knock: f64,
}

impl Window {
    fn new() -> Self {
        Self { state: WindowState::Away, t: 0.0, clock: 0.0, gap: None, expose: 0.0, stay: tuning().present_ms, knock: tuning().knock_min_ms }
    }
}

#[derive(Clone, Debug)]
struct Deck {
    spec: usize,
    left: Vec<Kind>,
}

struct Visit {
    dt_ms: f64,
    speed: f64,
    level: i32,
    door_ms: f64,
    shut: bool,
    lit: bool,
}

/// One Salvage Run night. The week is 1 to 5; the seed is the shift's seed, so the draws match the 2D game.
pub struct SalvageRun {
    week: usize,
    rng: NightWatchStream,
    upgrades: Vec<Upgrade>,
    levels: [Option<i32>; 3],
    infinite_power: bool,
    hunter_speed: f64,
    quota: u32,
    door_ms: f64,
    elapsed_ms: f64,
    spawn_timer_ms: f64,
    spawn_at_ms: f64,
    power: f64,
    out: bool,
    out_ms: f64,
    noise: f64,
    money: u32,
    items: Vec<Item>,
    deck: Deck,
    ravelle: Visitor,
    ceryth: Visitor,
    window: Window,
    outcome: Option<Outcome>,
}

impl SalvageRun {
    pub fn new(week: u32, seed: i64, options: Options) -> Self {
        let week = week_index(week);
        let quota = options.quota.unwrap_or(tuning().quota[week]);
        let door_ms = if options.custom { tuning().at_door_ms } else { tuning().at_door_by_week_ms[week] };
        let mut run = Self {
            week,
            rng: NightWatchStream::seeded(seed),
            upgrades: options.upgrades,
            levels: options.levels,
            infinite_power: options.infinite_power,
            hunter_speed: options.hunter_speed,
            quota,
            door_ms,
            elapsed_ms: 0.0,
            spawn_timer_ms: 0.0,
            spawn_at_ms: 0.0,
            power: tuning().power_start,
            out: false,
            out_ms: 0.0,
            noise: 0.0,
            money: 0,
            items: Vec::new(),
            deck: Deck { spec: 0, left: Vec::new() },
            ravelle: Visitor::new(tuning().start_dist),
            ceryth: Visitor::new(tuning().start_dist),
            window: Window::new(),
            outcome: None,
        };
        run.deck.spec = pick_deck(&mut run.rng);
        let lead_draw = run.rng.next();
        if run.base_level(Hunter::Ravelle) > 0 && run.base_level(Hunter::Ceryth) > 0 {
            let (lead, other) = if lead_draw < 0.5 { (Hunter::Ravelle, Hunter::Ceryth) } else { (Hunter::Ceryth, Hunter::Ravelle) };
            run.visitor_mut(lead).dist = tuning().start_dist - tuning().lead_steps;
            run.visitor_mut(other).dist = tuning().start_dist + tuning().lead_steps;
        }
        run.spawn_at_ms = jitter(tuning().first_spawn_ms, tuning().first_spawn_spread, &mut run.rng);
        run
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    pub fn money(&self) -> u32 {
        self.money
    }

    pub fn quota(&self) -> u32 {
        self.quota
    }

    pub fn power(&self) -> f64 {
        self.power
    }

    pub fn noise(&self) -> f64 {
        self.noise
    }

    pub fn is_out(&self) -> bool {
        self.out
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Whole hours into the night (0 is 12 AM), capped at the last hour.
    pub fn hour(&self) -> u32 {
        let t = tuning();
        let hour = (self.elapsed_ms / t.hour_ms).floor().max(0.0) as u32;
        hour.min((t.hours - 1).max(0) as u32)
    }

    /// Steps left before a side-door visitor reaches her door (0 is at the door). Ravelle and Ceryth only.
    pub fn distance(&self, hunter: Hunter) -> i32 {
        self.visitor(hunter).dist
    }

    /// The share of a side-door visitor's wait at the door that is done, 0 to 1. Ravelle and Ceryth only.
    pub fn door_wait(&self, hunter: Hunter) -> f64 {
        (self.visitor(hunter).at_ms / self.door_ms.max(1.0)).clamp(0.0, 1.0)
    }

    /// Where Ysolde is: away, knocking, or at the glass.
    pub fn window(&self) -> WindowState {
        self.window.state
    }

    /// Advances the night by `dt_ms` with the shell's input and returns what happened this step.
    pub fn step(&mut self, dt_ms: f64, input: Input) -> Vec<Event> {
        let mut events = Vec::new();
        if self.outcome.is_some() {
            return events;
        }
        self.elapsed_ms += dt_ms;
        let dt = dt_ms / 1000.0;
        let dev = if self.out { Devices::default() } else { input.devices };

        if self.out {
            self.out_ms += dt_ms;
            if self.out_ms >= tuning().out_attack_ms {
                return self.finish(events, Hunter::Ravelle);
            }
        } else {
            let doors = u32::from(dev.door_l) + u32::from(dev.door_r);
            let lights = u32::from(dev.light_l) + u32::from(dev.light_r);
            let door_factor = if self.has(Upgrade::Hinges) { tuning().hinges_factor } else { 1.0 };
            let mut draw = tuning().power_base + tuning().power_per_device * (f64::from(doors) * door_factor + f64::from(lights));
            if self.has(Upgrade::Cell) {
                draw *= tuning().cell_factor;
            }
            if !self.infinite_power {
                self.power = (self.power - draw * dt).max(0.0);
            }
            if self.power <= 0.0 {
                self.out = true;
                events.push(Event::Out);
            }
        }

        self.noise = (self.noise - tuning().noise_decay_per_sec * dt).max(0.0);
        self.spawn_timer_ms += dt_ms;
        if self.spawn_timer_ms >= self.spawn_at_ms {
            self.spawn_timer_ms = 0.0;
            if let Some(slot) = self.spawn_item() {
                events.push(Event::Item { slot });
            }
            self.spawn_at_ms = jitter(tuning().spawn_ms, tuning().spawn_spread, &mut self.rng);
        }
        self.expire_items(dt_ms, &mut events);

        let working = match input.target {
            Some(slot) if input.work && !self.out => self.items.iter().position(|it| it.slot == slot),
            _ => None,
        };
        if let Some(idx) = working {
            self.items[idx].progress_ms += dt_ms;
            let bench = if self.has(Upgrade::Bench) { tuning().bench_factor } else { 1.0 };
            self.noise = (self.noise + tuning().noise_per_sec * bench * dt + tuning().noise_decay_per_sec * dt).min(tuning().noise_max);
            if self.items[idx].progress_ms >= self.items[idx].need_ms {
                let done = self.items.remove(idx);
                self.money += done.value;
                events.push(Event::Salvage { slot: done.slot, value: done.value });
            }
        }

        let before = events.len();
        let at = self.visit_at(Hunter::Ravelle, dev.door_l, dev.light_l, dt_ms);
        if visit(&mut self.ravelle, &mut self.rng, Hunter::Ravelle, at, &mut events) {
            return self.finish(events, Hunter::Ravelle);
        }
        let at = self.visit_at(Hunter::Ceryth, dev.door_r, dev.light_r, dt_ms);
        if visit(&mut self.ceryth, &mut self.rng, Hunter::Ceryth, at, &mut events) {
            return self.finish(events, Hunter::Ceryth);
        }
        let turned: Vec<Hunter> = events[before..]
            .iter()
            .filter_map(|e| match e {
                Event::Repelled { id } => Some(*id),
                _ => None,
            })
            .collect();
        for id in turned {
            let other = id.other_door();
            if self.visitor(other).dist > 0 && self.level(Hunter::Ravelle) >= tuning().combo_level && self.level(Hunter::Ceryth) >= tuning().combo_level {
                self.visitor_mut(other).pacer.skip(tuning().combo_ms);
                events.push(Event::Warned { id: other });
            }
        }

        let ysolde_level = self.level(Hunter::Ysolde);
        if ysolde_level > 0 && self.window_step(ysolde_level, dt_ms, dev, working.is_some(), &mut events) {
            return self.finish(events, Hunter::Ysolde);
        }

        if self.elapsed_ms >= tuning().night_ms {
            let (money, quota) = (self.money, self.quota);
            if money >= quota {
                self.outcome = Some(Outcome::Win);
                events.push(Event::Win { money, quota });
            } else {
                self.outcome = Some(Outcome::Short);
                events.push(Event::Short { money, quota });
            }
        }
        events
    }

    fn has(&self, upgrade: Upgrade) -> bool {
        self.upgrades.contains(&upgrade)
    }

    fn base_level(&self, hunter: Hunter) -> i32 {
        match self.levels[hunter.index()] {
            Some(level) if level >= 0 => level,
            _ => hunter.week_levels()[self.week],
        }
    }

    /// The hunter's level right now: her week's base, raised by the night's AI steps and by how loud the rack is.
    pub fn level(&self, hunter: Hunter) -> i32 {
        let base = self.base_level(hunter);
        if base <= 0 {
            return 0;
        }
        let bonus = if self.noise >= tuning().loud {
            tuning().loud_bonus
        } else if self.noise >= tuning().uneasy {
            tuning().uneasy_bonus
        } else {
            0
        };
        let soles = if self.has(Upgrade::Soles) { tuning().soles_levels } else { 0 };
        (base + ai_bump(self.elapsed_ms) + bonus - soles).clamp(1, tuning().max_level)
    }

    fn visitor(&self, hunter: Hunter) -> &Visitor {
        match hunter {
            Hunter::Ceryth => &self.ceryth,
            _ => &self.ravelle,
        }
    }

    fn visitor_mut(&mut self, hunter: Hunter) -> &mut Visitor {
        match hunter {
            Hunter::Ceryth => &mut self.ceryth,
            _ => &mut self.ravelle,
        }
    }

    fn visit_at(&self, hunter: Hunter, shut: bool, lit: bool, dt_ms: f64) -> Visit {
        Visit { dt_ms, speed: self.hunter_speed, level: self.level(hunter), door_ms: self.door_ms, shut, lit }
    }

    fn deal(&mut self) -> Kind {
        if self.deck.left.is_empty() {
            let counts = tuning().decks[self.deck.spec].counts;
            let mut bag = Vec::new();
            for kind in Kind::ALL {
                for _ in 0..counts[kind as usize] {
                    bag.push(kind);
                }
            }
            for i in (1..bag.len()).rev() {
                let j = (self.rng.next() * (i + 1) as f64).floor() as usize;
                bag.swap(i, j);
            }
            self.deck.left = bag;
        }
        self.deck.left.pop().unwrap_or(Kind::Scrap)
    }

    fn spawn_item(&mut self) -> Option<usize> {
        let free: Vec<usize> = (0..tuning().slots).filter(|slot| self.items.iter().all(|it| it.slot != *slot)).collect();
        if free.is_empty() {
            return None;
        }
        let kind = self.deal();
        let slot = free[pick_index(&mut self.rng, free.len())];
        let gloves = if self.has(Upgrade::Gloves) { tuning().gloves_factor } else { 1.0 };
        self.items.push(Item { slot, kind, value: kind.value(), need_ms: kind.hold_ms() * gloves, progress_ms: 0.0, age_ms: 0.0 });
        Some(slot)
    }

    fn expire_items(&mut self, dt_ms: f64, events: &mut Vec<Event>) {
        let mut i = 0;
        while i < self.items.len() {
            self.items[i].age_ms += dt_ms;
            if self.items[i].age_ms >= tuning().expire_ms {
                let gone = self.items.remove(i);
                events.push(Event::Expire { slot: gone.slot });
            } else {
                i += 1;
            }
        }
    }

    /// Ysolde's window: away, then a knock, then at the glass until she is seen long enough (`true` ends the night).
    fn window_step(&mut self, level: i32, dt_ms: f64, dev: Devices, working: bool, events: &mut Vec<Event>) -> bool {
        match self.window.state {
            WindowState::Away => {
                self.window.clock += dt_ms;
                let gap = match self.window.gap {
                    Some(gap) => gap,
                    None => {
                        let base = (tuning().window_gap_ms - f64::from(level) * tuning().window_gap_per_level_ms).max(tuning().window_gap_min_ms);
                        let gap = jitter(base, tuning().window_gap_spread, &mut self.rng);
                        self.window.gap = Some(gap);
                        gap
                    }
                };
                if self.window.clock >= gap {
                    let (lo, hi) = (tuning().knock_min_ms, tuning().knock_max_ms);
                    self.window.state = WindowState::Knock;
                    self.window.t = 0.0;
                    self.window.clock = 0.0;
                    self.window.gap = None;
                    self.window.knock = jitter((lo + hi) / 2.0, (hi - lo) / (lo + hi), &mut self.rng).max(lo);
                    events.push(Event::Knock);
                }
            }
            WindowState::Knock => {
                self.window.t += dt_ms;
                if self.window.t >= self.window.knock {
                    self.window.state = WindowState::Here;
                    self.window.t = 0.0;
                    self.window.expose = 0.0;
                    self.window.stay = spread_ms(tuning().present_ms, &mut self.rng, tuning().present_spread);
                    events.push(Event::Window);
                }
            }
            WindowState::Here => {
                self.window.t += dt_ms;
                let hears = dev.light_l || dev.light_r || working;
                self.window.expose = if hears { self.window.expose + dt_ms } else { (self.window.expose - dt_ms).max(0.0) };
                if self.window.expose >= tuning().expose_ms {
                    return true;
                }
                if self.window.t >= self.window.stay {
                    self.window.state = WindowState::Away;
                    self.window.t = 0.0;
                    self.window.expose = 0.0;
                    self.window.gap = None;
                    events.push(Event::Gone);
                }
            }
        }
        false
    }

    fn finish(&mut self, mut events: Vec<Event>, by: Hunter) -> Vec<Event> {
        self.outcome = Some(Outcome::Caught(by));
        events.push(Event::Caught { by });
        events
    }
}

/// One visitor's step (Godot `dayshift_logic.gd` `_visitor`). True when she gets in through the door.
fn visit(v: &mut Visitor, rng: &mut NightWatchStream, id: Hunter, at: Visit, events: &mut Vec<Event>) -> bool {
    if v.dist > 0 {
        if !at.lit && v.pacer.step(rng, at.dt_ms * at.speed, at.level) {
            v.dist -= 1;
            events.push(Event::Close { id, dist: v.dist });
        }
        return false;
    }
    if at.shut {
        v.at_ms = 0.0;
        v.leave_ms += at.dt_ms;
        if v.leave_ms >= tuning().leave_ms {
            v.dist = tuning().repelled_dist;
            v.leave_ms = 0.0;
            v.pacer.reset();
            events.push(Event::Repelled { id });
        }
        return false;
    }
    v.leave_ms = 0.0;
    if !at.lit {
        v.at_ms += at.dt_ms;
    }
    v.at_ms >= at.door_ms
}

fn pick_deck(rng: &mut NightWatchStream) -> usize {
    let total: u32 = tuning().decks.iter().map(|d| d.weight).sum();
    let mut r = rng.next() * f64::from(total);
    for (i, deck) in tuning().decks.iter().enumerate() {
        r -= f64::from(deck.weight);
        if r < 0.0 {
            return i;
        }
    }
    0
}

fn week_index(week: u32) -> usize {
    (week.clamp(1, 5) - 1) as usize
}

/// AI steps up with the hour: nothing before 3 AM, +1 at 3 AM, +2 from 4 AM (Godot `NightWatchData.ai_bump`).
fn ai_bump(elapsed_ms: f64) -> i32 {
    let t = tuning();
    let hour = ((elapsed_ms / t.hour_ms).floor() as i32).min(t.hours - 1);
    if hour >= t.bump_hour_2 {
        t.bump_2
    } else if hour >= t.bump_hour_1 {
        t.bump_1
    } else {
        0
    }
}

pub fn total_cost(upgrades: &[Upgrade]) -> u32 {
    upgrades.iter().map(|u| u.cost()).sum()
}

/// The week's budget plus the carried surplus (capped).
pub fn budget_for(week: u32, carry: u32) -> u32 {
    tuning().budget[week_index(week)] + carry.min(tuning().carry_cap)
}

pub fn quota_for(week: u32) -> u32 {
    tuning().quota[week_index(week)]
}

/// Last week's surplus brought forward, capped.
pub fn carry_from(money: u32, quota: u32) -> u32 {
    money.saturating_sub(quota).min(tuning().carry_cap)
}

/// Whether a basket of upgrades fits the budget, with no upgrade twice.
pub fn can_buy(upgrades: &[Upgrade], week: u32, carry: u32) -> bool {
    let mut seen: Vec<Upgrade> = Vec::new();
    for u in upgrades {
        if seen.contains(u) {
            return false;
        }
        seen.push(*u);
    }
    total_cost(upgrades) <= budget_for(week, carry)
}

/// One click in the shop: toggles `upgrade` in the basket. None when the new basket would go over the budget.
pub fn shop_toggle(owned: &[Upgrade], upgrade: Upgrade, week: u32, carry: u32) -> Option<Vec<Upgrade>> {
    let next: Vec<Upgrade> = if owned.contains(&upgrade) {
        owned.iter().copied().filter(|u| *u != upgrade).collect()
    } else {
        owned.iter().copied().chain(std::iter::once(upgrade)).collect()
    };
    can_buy(&next, week, carry).then_some(next)
}

/// The hunters who come this week (a level of 0 means she stays home).
pub fn active_hunters(week: u32) -> Vec<Hunter> {
    let w = week_index(week);
    Hunter::ALL.iter().copied().filter(|h| h.week_levels()[w] > 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piece_held_for_its_full_time_pays_its_value() {
        let mut run = SalvageRun::new(1, 7, Options::default());
        run.items.push(Item {
            slot: 0,
            kind: Kind::Scrap,
            value: Kind::Scrap.value(),
            need_ms: Kind::Scrap.hold_ms(),
            progress_ms: 0.0,
            age_ms: 0.0,
        });
        let input = Input { work: true, target: Some(0), devices: Devices::default() };
        let events = run.step(2000.0, input);
        assert!(events.contains(&Event::Salvage { slot: 0, value: 10 }));
        assert_eq!(run.money(), 10);
    }

    fn quiet() -> Options {
        Options { levels: [Some(0); 3], ..Options::default() }
    }

    fn drive(run: &mut SalvageRun, input: Input, limit_ms: f64) -> Vec<Event> {
        let mut all = Vec::new();
        let mut t = 0.0;
        while run.outcome().is_none() && t < limit_ms {
            all.extend(run.step(1000.0, input));
            t += 1000.0;
        }
        all
    }

    #[test]
    fn a_quiet_night_below_the_quota_ends_short_and_at_the_quota_ends_won() {
        let mut short = SalvageRun::new(1, 3, quiet());
        drive(&mut short, Input::default(), tuning().night_ms + 2000.0);
        assert_eq!(short.outcome(), Some(Outcome::Short));
        let mut won = SalvageRun::new(1, 3, Options { quota: Some(0), ..quiet() });
        drive(&mut won, Input::default(), tuning().night_ms + 2000.0);
        assert_eq!(won.outcome(), Some(Outcome::Win));
    }

    #[test]
    fn the_shop_refuses_a_basket_over_the_budget_and_a_repeat() {
        let all = Upgrade::ALL.to_vec();
        assert!(total_cost(&all) > budget_for(1, 0));
        assert!(shop_toggle(&[], Upgrade::Gloves, 1, 0).is_some());
        let mut basket = Vec::new();
        for u in Upgrade::ALL {
            match shop_toggle(&basket, u, 1, 0) {
                Some(next) => basket = next,
                None => assert!(total_cost(&basket) + u.cost() > budget_for(1, 0)),
            }
        }
        assert!(can_buy(&basket, 1, 0));
        assert!(!can_buy(&[Upgrade::Cell, Upgrade::Cell], 5, 40));
        // Taking an owned upgrade out of the basket always fits.
        assert!(shop_toggle(&basket, basket[0], 1, 0).is_some());
    }

    #[test]
    fn the_carry_is_the_surplus_and_it_is_capped() {
        assert_eq!(carry_from(100, 140), 0);
        assert_eq!(carry_from(150, 140), 10);
        assert_eq!(carry_from(10_000, 140), tuning().carry_cap);
        assert_eq!(budget_for(2, 10_000), tuning().budget[1] + tuning().carry_cap);
    }

    #[test]
    fn gloves_shorten_the_hold_and_soft_soles_lower_the_hunters_level() {
        let mut plain = SalvageRun::new(1, 9, quiet());
        let mut gloved = SalvageRun::new(1, 9, Options { upgrades: vec![Upgrade::Gloves], ..quiet() });
        plain.spawn_item();
        gloved.spawn_item();
        let (a, b) = (plain.items[0].need_ms, gloved.items[0].need_ms);
        assert!(b < a);
        assert!((b - a * tuning().gloves_factor).abs() < 1e-6);
        let live = Options { levels: [None; 3], ..Options::default() };
        let base = SalvageRun::new(3, 9, live.clone());
        let soft = SalvageRun::new(3, 9, Options { upgrades: vec![Upgrade::Soles], ..live });
        assert_eq!(base.level(Hunter::Ravelle) - soft.level(Hunter::Ravelle), tuning().soles_levels);
    }

    #[test]
    fn running_out_of_power_opens_the_doors_and_then_the_dark_gets_you() {
        let all_on = Devices { door_l: true, door_r: true, light_l: true, light_r: true };
        let mut run = SalvageRun::new(1, 5, quiet());
        let events = drive(&mut run, Input { devices: all_on, ..Input::default() }, tuning().night_ms);
        assert!(events.contains(&Event::Out));
        assert_eq!(run.outcome(), Some(Outcome::Caught(Hunter::Ravelle)));
    }

    #[test]
    fn infinite_power_never_drains() {
        let all_on = Devices { door_l: true, door_r: true, light_l: true, light_r: true };
        let mut run = SalvageRun::new(1, 5, Options { infinite_power: true, ..quiet() });
        drive(&mut run, Input { devices: all_on, ..Input::default() }, tuning().night_ms);
        assert_eq!(run.power(), tuning().power_start);
        assert!(!run.is_out());
    }

    #[test]
    fn an_unattended_piece_slips_off_the_rack_after_its_time() {
        let mut run = SalvageRun::new(1, 5, quiet());
        run.items.push(Item { slot: 0, kind: Kind::Prize, value: 60, need_ms: 5500.0, progress_ms: 0.0, age_ms: 0.0 });
        let events = drive(&mut run, Input::default(), tuning().expire_ms + 2000.0);
        assert!(events.contains(&Event::Expire { slot: 0 }));
        assert_eq!(run.money(), 0);
    }

    #[test]
    fn the_rules_file_carries_every_key_the_mode_reads() {
        // Loading panics naming the mode and key when one is missing.
        let t = Tuning::load();
        assert_eq!(t.decks.len(), 3);
        assert_eq!(t.week_levels.len(), HUNTER_IDS.len());
    }

    #[test]
    fn the_loaded_values_equal_the_old_constants() {
        let t = tuning();
        assert_eq!(t.night_ms, 270_000.0);
        assert_eq!(t.hour_ms, 45_000.0);
        assert_eq!(t.power_base, 0.10 / (270_000.0 / 480_000.0));
        assert_eq!(t.power_per_device, 0.07 / (270_000.0 / 480_000.0));
        assert_eq!(t.at_door_by_week_ms, [8000.0, 7000.0, 6200.0, 5600.0, 5200.0]);
        assert_eq!(t.budget, [90, 100, 110, 120, 130]);
        assert_eq!(t.quota, [140, 210, 280, 340, 380]);
        assert_eq!(Upgrade::Gloves.cost(), 45);
        assert_eq!(Upgrade::Soles.name(), "Soft soles");
        assert_eq!(Kind::Prize.hold_ms(), 5500.0);
        assert_eq!(Hunter::Ysolde.week_levels(), [0, 0, 6, 10, 13]);
        assert_eq!(t.decks[0].counts, [5, 3, 1]);
        assert_eq!(t.decks[2].weight, 1);
        assert_eq!((t.knock_min_ms, t.knock_max_ms), (3000.0, 4200.0));
        assert_eq!((t.carry_cap, t.max_level, t.slots), (40, 20, 4));
        assert_eq!((ai_bump(0.0), ai_bump(3.0 * 45_000.0), ai_bump(5.5 * 45_000.0)), (0, 1, 2));
    }
}
