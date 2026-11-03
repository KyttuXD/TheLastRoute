//! Salvage Run, the day-shop and rack night of Night Watch (Godot `game/night_watch/dayshift`).
//! The rules and scoring of `dayshift_logic.gd`, with the data of `dayshift_data.gd`, number for number. The shell
//! builds a `SalvageRun` for the week, feeds `step` the doors, the lights and the rack each frame, and reads back the
//! events. Art, sound, the shop screen and the window view are left to the shell.

use crate::framework::{jitter, pick_index, spread_ms, NightWatchStream};

const NIGHT_MS: f64 = 270_000.0;
const HOUR_MS: f64 = NIGHT_MS / 6.0;
const TIME_SCALE: f64 = NIGHT_MS / 480_000.0;

const COMBO_LEVEL: i32 = 6;
const COMBO_MS: f64 = 2200.0;
const AT_DOOR_MS: f64 = 5200.0;
const AT_DOOR_BY_WEEK_MS: [f64; 5] = [8000.0, 7000.0, 6200.0, 5600.0, 5200.0];
const LEAVE_MS: f64 = 450.0;
const START_DIST: i32 = 3;
const LEAD_STEPS: i32 = 1;
const REPELLED_DIST: i32 = 2;

const NOISE_PER_SEC: f64 = 14.0;
const NOISE_DECAY_PER_SEC: f64 = 10.0;
const NOISE_MAX: f64 = 100.0;
const LOUD: f64 = 40.0;
const UNEASY: f64 = 20.0;

const POWER_START: f64 = 100.0;
const POWER_BASE: f64 = 0.10 / TIME_SCALE;
const POWER_PER_DEVICE: f64 = 0.07 / TIME_SCALE;
const OUT_ATTACK_MS: f64 = 9000.0;

const SLOTS: usize = 4;
const SPAWN_MS: f64 = 11000.0;
const SPAWN_SPREAD: f64 = 0.2;
const FIRST_SPAWN_MS: f64 = 2000.0;
const FIRST_SPAWN_SPREAD: f64 = 0.5;
const EXPIRE_MS: f64 = 30000.0;
const GLOVES_FACTOR: f64 = 0.7;
const BENCH_FACTOR: f64 = 0.5;
const HINGES_FACTOR: f64 = 0.5;
const CELL_FACTOR: f64 = 0.75;
const SOLES_LEVELS: i32 = 2;

const WINDOW_GAP_MS: f64 = 42000.0;
const WINDOW_GAP_PER_LEVEL_MS: f64 = 1800.0;
const WINDOW_GAP_MIN_MS: f64 = 14000.0;
const WINDOW_GAP_SPREAD: f64 = 0.3;
const KNOCK_MIN_MS: f64 = 3000.0;
const KNOCK_MAX_MS: f64 = 4200.0;
const PRESENT_MS: f64 = 6000.0;
const PRESENT_SPREAD: f64 = 0.2;
const EXPOSE_MS: f64 = 1500.0;

const CARRY_CAP: u32 = 40;
const BUDGET: [u32; 5] = [90, 100, 110, 120, 130];
const QUOTA: [u32; 5] = [140, 210, 280, 340, 380];
const MAX_LEVEL: i32 = 20;

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
        match self {
            Upgrade::Hinges => 30,
            Upgrade::Cell => 40,
            Upgrade::Soles => 35,
            Upgrade::Gloves => 45,
            Upgrade::Bench => 40,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Upgrade::Hinges => "Sturdy hinges",
            Upgrade::Cell => "Extra cell",
            Upgrade::Soles => "Soft soles",
            Upgrade::Gloves => "Salvage gloves",
            Upgrade::Bench => "Padded bench",
        }
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
        match self {
            Hunter::Ravelle => [4, 7, 10, 13, 15],
            Hunter::Ceryth => [0, 5, 8, 12, 14],
            Hunter::Ysolde => [0, 0, 6, 10, 13],
        }
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
        match self {
            Kind::Scrap => 10,
            Kind::Parts => 25,
            Kind::Prize => 60,
        }
    }

    fn hold_ms(self) -> f64 {
        match self {
            Kind::Scrap => 2000.0,
            Kind::Parts => 3500.0,
            Kind::Prize => 5500.0,
        }
    }
}

/// One rack deck: how often it is the one dealt, and how many of each kind it holds (scrap, parts, prize).
struct DeckSpec {
    weight: u32,
    counts: [usize; 3],
}

const DECKS: [DeckSpec; 3] = [
    DeckSpec { weight: 2, counts: [5, 3, 1] },
    DeckSpec { weight: 1, counts: [7, 0, 2] },
    DeckSpec { weight: 1, counts: [3, 6, 0] },
];

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub slot: usize,
    pub kind: Kind,
    pub value: u32,
    need_ms: f64,
    progress_ms: f64,
    age_ms: f64,
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
    let lo = (13000.0 - f64::from(level) * 500.0).max(2500.0);
    lo + rng.next() * 3000.0
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
        Self { state: WindowState::Away, t: 0.0, clock: 0.0, gap: None, expose: 0.0, stay: PRESENT_MS, knock: KNOCK_MIN_MS }
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
    lead: Option<Hunter>,
    outcome: Option<Outcome>,
}

impl SalvageRun {
    pub fn new(week: u32, seed: i64, options: Options) -> Self {
        let week = week_index(week);
        let quota = options.quota.unwrap_or(QUOTA[week]);
        let door_ms = if options.custom { AT_DOOR_MS } else { AT_DOOR_BY_WEEK_MS[week] };
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
            power: POWER_START,
            out: false,
            out_ms: 0.0,
            noise: 0.0,
            money: 0,
            items: Vec::new(),
            deck: Deck { spec: 0, left: Vec::new() },
            ravelle: Visitor::new(START_DIST),
            ceryth: Visitor::new(START_DIST),
            window: Window::new(),
            lead: None,
            outcome: None,
        };
        run.deck.spec = pick_deck(&mut run.rng);
        let lead_draw = run.rng.next();
        if run.base_level(Hunter::Ravelle) > 0 && run.base_level(Hunter::Ceryth) > 0 {
            let (lead, other) = if lead_draw < 0.5 { (Hunter::Ravelle, Hunter::Ceryth) } else { (Hunter::Ceryth, Hunter::Ravelle) };
            run.visitor_mut(lead).dist = START_DIST - LEAD_STEPS;
            run.visitor_mut(other).dist = START_DIST + LEAD_STEPS;
            run.lead = Some(lead);
        }
        run.spawn_at_ms = jitter(FIRST_SPAWN_MS, FIRST_SPAWN_SPREAD, &mut run.rng);
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

    pub fn lead(&self) -> Option<Hunter> {
        self.lead
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
            if self.out_ms >= OUT_ATTACK_MS {
                return self.finish(events, Hunter::Ravelle);
            }
        } else {
            let doors = u32::from(dev.door_l) + u32::from(dev.door_r);
            let lights = u32::from(dev.light_l) + u32::from(dev.light_r);
            let door_factor = if self.has(Upgrade::Hinges) { HINGES_FACTOR } else { 1.0 };
            let mut draw = POWER_BASE + POWER_PER_DEVICE * (f64::from(doors) * door_factor + f64::from(lights));
            if self.has(Upgrade::Cell) {
                draw *= CELL_FACTOR;
            }
            if !self.infinite_power {
                self.power = (self.power - draw * dt).max(0.0);
            }
            if self.power <= 0.0 {
                self.out = true;
                events.push(Event::Out);
            }
        }

        self.noise = (self.noise - NOISE_DECAY_PER_SEC * dt).max(0.0);
        self.spawn_timer_ms += dt_ms;
        if self.spawn_timer_ms >= self.spawn_at_ms {
            self.spawn_timer_ms = 0.0;
            if let Some(slot) = self.spawn_item() {
                events.push(Event::Item { slot });
            }
            self.spawn_at_ms = jitter(SPAWN_MS, SPAWN_SPREAD, &mut self.rng);
        }
        self.expire_items(dt_ms, &mut events);

        let working = match input.target {
            Some(slot) if input.work && !self.out => self.items.iter().position(|it| it.slot == slot),
            _ => None,
        };
        if let Some(idx) = working {
            self.items[idx].progress_ms += dt_ms;
            let bench = if self.has(Upgrade::Bench) { BENCH_FACTOR } else { 1.0 };
            self.noise = (self.noise + NOISE_PER_SEC * bench * dt + NOISE_DECAY_PER_SEC * dt).min(NOISE_MAX);
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
            if self.visitor(other).dist > 0 && self.level(Hunter::Ravelle) >= COMBO_LEVEL && self.level(Hunter::Ceryth) >= COMBO_LEVEL {
                self.visitor_mut(other).pacer.skip(COMBO_MS);
                events.push(Event::Warned { id: other });
            }
        }

        let ysolde_level = self.level(Hunter::Ysolde);
        if ysolde_level > 0 && self.window_step(ysolde_level, dt_ms, dev, working.is_some(), &mut events) {
            return self.finish(events, Hunter::Ysolde);
        }

        if self.elapsed_ms >= NIGHT_MS {
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
        let bonus = if self.noise >= LOUD {
            3
        } else if self.noise >= UNEASY {
            1
        } else {
            0
        };
        let soles = if self.has(Upgrade::Soles) { SOLES_LEVELS } else { 0 };
        (base + ai_bump(self.elapsed_ms) + bonus - soles).clamp(1, MAX_LEVEL)
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
            let counts = DECKS[self.deck.spec].counts;
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
        let free: Vec<usize> = (0..SLOTS).filter(|slot| self.items.iter().all(|it| it.slot != *slot)).collect();
        if free.is_empty() {
            return None;
        }
        let kind = self.deal();
        let slot = free[pick_index(&mut self.rng, free.len())];
        let gloves = if self.has(Upgrade::Gloves) { GLOVES_FACTOR } else { 1.0 };
        self.items.push(Item { slot, kind, value: kind.value(), need_ms: kind.hold_ms() * gloves, progress_ms: 0.0, age_ms: 0.0 });
        Some(slot)
    }

    fn expire_items(&mut self, dt_ms: f64, events: &mut Vec<Event>) {
        let mut i = 0;
        while i < self.items.len() {
            self.items[i].age_ms += dt_ms;
            if self.items[i].age_ms >= EXPIRE_MS {
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
                        let base = (WINDOW_GAP_MS - f64::from(level) * WINDOW_GAP_PER_LEVEL_MS).max(WINDOW_GAP_MIN_MS);
                        let gap = jitter(base, WINDOW_GAP_SPREAD, &mut self.rng);
                        self.window.gap = Some(gap);
                        gap
                    }
                };
                if self.window.clock >= gap {
                    let (lo, hi) = (KNOCK_MIN_MS, KNOCK_MAX_MS);
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
                    self.window.stay = spread_ms(PRESENT_MS, &mut self.rng, PRESENT_SPREAD);
                    events.push(Event::Window);
                }
            }
            WindowState::Here => {
                self.window.t += dt_ms;
                let hears = dev.light_l || dev.light_r || working;
                self.window.expose = if hears { self.window.expose + dt_ms } else { (self.window.expose - dt_ms).max(0.0) };
                if self.window.expose >= EXPOSE_MS {
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
        if v.leave_ms >= LEAVE_MS {
            v.dist = REPELLED_DIST;
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
    let total: u32 = DECKS.iter().map(|d| d.weight).sum();
    let mut r = rng.next() * f64::from(total);
    for (i, deck) in DECKS.iter().enumerate() {
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
    let hour = ((elapsed_ms / HOUR_MS).floor() as i32).min(5);
    if hour >= 4 {
        2
    } else if hour >= 3 {
        1
    } else {
        0
    }
}

pub fn total_cost(upgrades: &[Upgrade]) -> u32 {
    upgrades.iter().map(|u| u.cost()).sum()
}

/// The week's budget plus the carried surplus (capped).
pub fn budget_for(week: u32, carry: u32) -> u32 {
    BUDGET[week_index(week)] + carry.min(CARRY_CAP)
}

pub fn quota_for(week: u32) -> u32 {
    QUOTA[week_index(week)]
}

/// Last week's surplus brought forward, capped.
pub fn carry_from(money: u32, quota: u32) -> u32 {
    money.saturating_sub(quota).min(CARRY_CAP)
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
}
