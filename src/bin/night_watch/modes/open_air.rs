//! Open Air night type (Godot `game/night_watch/openair`, mode id `extra`): three outdoor watches, each with its own
//! hunter and its own way of keeping the dark off. Ported from `openair_logic.gd` (the 2D `createWatchGame`): rules,
//! weather, pacing, the Custom Night chain and the score. The walkable sets, art, sound and admin hooks are not ported.
//! The shell runs one `OpenAir` per watch: `step` with what is held each frame, `act` for the lamp, a canister or a log.

use crate::data::CallsData;
use crate::framework::{jitter, pick_index, CallScript, NightWatchStream};

pub const MODE_ID: &str = "extra";
const POINTS_PER_LEVEL: i32 = 10;
const MAX_AI: i32 = 20;
const WATCH_MS: f64 = 270_000.0;
const HOUR_MS: f64 = WATCH_MS / 6.0;
const BUS_MS: f64 = 18_000.0;

const START_DIST: i32 = 3;
const AT_MS: f64 = 5_200.0;
const LEAVE_MS: f64 = 450.0;
const SIDE_SWITCH: f64 = 0.65;
const ARRIVE_SPREAD_MS: f64 = 4_000.0;
const DARK_BONUS: i32 = 4;

const GUST_MS: f64 = 22_000.0;
const GUST_JITTER_MS: f64 = 8_000.0;
const GUST_FIRST_SPREAD: f64 = 0.25;
const RELIGHT_MS: f64 = 1_600.0;
const TORCH_DRAIN: f64 = 9.0;
const TORCH_REGEN: f64 = 4.0;
const OIL_BASE: f64 = 0.12;
const OIL_PER_BAR: f64 = 0.45;
const RELIGHT_OIL: f64 = 4.0;

const COLD_DRAIN: f64 = 1.0;
const LAMP_WARM: f64 = 2.8;
const LAMP_FUEL: f64 = 1.6;
const CHILL_LINE: f64 = 40.0;
const CHILL_PER_POINT: f64 = 0.12;
const CHILL_RECOVER: f64 = 3.0;
const COLD_SPREAD: f64 = 0.15;
const COLD_EVERY_MS: f64 = 30_000.0;
const COLD_EVERY_SPREAD: f64 = 0.33;
const CAN_FIRST_MS: f64 = 4_000.0;
const CAN_FIRST_SPREAD: f64 = 0.5;
const CAN_EVERY_MS: f64 = 17_000.0;
const CAN_SPREAD: f64 = 0.25;
const CAN_FUEL: f64 = 35.0;
const MAX_CANS: usize = 3;
const WEATHER_LAST_SPREAD: f64 = 0.25;

const CAMP_DRAIN: f64 = 1.8;
const LOG_FUEL: f64 = 28.0;
const START_LOGS: u32 = 6;
const GATHER_MS: f64 = 2_200.0;
const OUT_GRACE_MS: f64 = 10_000.0;
const FUEL_STEAL: f64 = 15.0;
const CAMP_AT_MS: f64 = 5_200.0;
const PUSH_MS: f64 = 1_500.0;
const PUSH_BACK: i32 = 2;
const LOST_MS: f64 = 2_000.0;
const LOST_BACK: i32 = 1;
const TENT_PACE: f64 = 3.25;
const BACK_PACE: f64 = 1.0;
const LOW_FIRE: f64 = 25.0;
const LOW_FIRE_BONUS: i32 = 3;
const MOOD_EVERY_MS: f64 = 45_000.0;
const MOOD_EVERY_SPREAD: f64 = 0.33;
const MOOD_PACE: f64 = 1.3;
const MOOD_CALM: f64 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hunter {
    Verrine,
    Wynelle,
    Ysolde,
    Quenelle,
}

impl Hunter {
    pub fn name(self) -> &'static str {
        match self {
            Hunter::Verrine => "Verrine",
            Hunter::Wynelle => "Wynelle",
            Hunter::Ysolde => "Ysolde",
            Hunter::Quenelle => "Quenelle",
        }
    }
}

/// AI levels for the four hunters of this mode. `LISTED` is the mode's own table; a Custom Night sets its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Levels {
    pub verrine: i32,
    pub wynelle: i32,
    pub ysolde: i32,
    pub quenelle: i32,
}

impl Levels {
    pub const LISTED: Levels = Levels { verrine: 9, wynelle: 8, ysolde: 8, quenelle: 13 };

    pub fn of(self, hunter: Hunter) -> i32 {
        match hunter {
            Hunter::Verrine => self.verrine,
            Hunter::Wynelle => self.wynelle,
            Hunter::Ysolde => self.ysolde,
            Hunter::Quenelle => self.quenelle,
        }
    }

    fn sum_clamped(self) -> i32 {
        [Hunter::Verrine, Hunter::Wynelle, Hunter::Ysolde, Hunter::Quenelle]
            .iter()
            .map(|&h| self.of(h).clamp(0, MAX_AI))
            .sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    Rooftop,
    BusStop,
    Campfire,
}

impl Watch {
    pub const ALL: [Watch; 3] = [Watch::Rooftop, Watch::BusStop, Watch::Campfire];

    /// Level 1 is the rooftop, 2 the bus stop, 3 the campfire; out-of-range levels clamp.
    pub fn from_level(level: i32) -> Watch {
        Self::ALL[(level - 1).clamp(0, 2) as usize]
    }

    pub fn level(self) -> i32 {
        self as i32 + 1
    }

    pub fn label(self) -> &'static str {
        match self {
            Watch::Rooftop => "ROOFTOP",
            Watch::BusStop => "BUS STOP",
            Watch::Campfire => "CAMPFIRE",
        }
    }

    /// The watch's own hunter. The campfire's own hunter is Ysolde; Verrine and Quenelle come from the trees too.
    fn hunter(self) -> Hunter {
        match self {
            Watch::Rooftop => Hunter::Verrine,
            Watch::BusStop => Hunter::Wynelle,
            Watch::Campfire => Hunter::Ysolde,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Edge,
    Cold,
    Fire,
    Tent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeatherKind {
    Snow,
    Rain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Playing,
    Won,
    Caught { by: Hunter, cause: Cause },
}

/// What happened during a step. Ysolde is the only hunter behind Backoff, Creep and Stolen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Close { hunter: Hunter, dist: i32, side: Side },
    Repelled { hunter: Hunter, side: Side },
    Gust,
    Relit,
    Dry,
    Weather(WeatherKind),
    WeatherEnd(WeatherKind),
    Bus,
    Can,
    Gathered,
    TentIn,
    TentOut,
    Creep,
    Stolen,
    Backoff { dist: i32 },
    Caught { by: Hunter, cause: Cause },
    Win,
}

/// What the player holds this step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub torch_left: bool,
    pub torch_right: bool,
    pub bar_left: bool,
    pub bar_right: bool,
    pub relight: bool,
    pub gather: bool,
    pub tent: bool,
}

/// A one-off press: the bus stop's lamp or a canister, the campfire's log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Lamp,
    Can(u8),
    Log,
}

/// The shift's pacer (2D `createPacer`): a wait drawn from a three-second window that slides down with the level.
#[derive(Clone, Debug)]
struct Pacer {
    t: f64,
    next: f64,
}

fn wait_ms(level: i32, stream: &mut NightWatchStream) -> f64 {
    let lo = (13_000.0 - f64::from(level) * 500.0).max(2_500.0);
    lo + stream.next() * 3_000.0
}

impl Pacer {
    fn new() -> Self {
        Self { t: 0.0, next: -1.0 }
    }

    fn step(&mut self, dt_ms: f64, level: i32, stream: &mut NightWatchStream) -> bool {
        if level <= 0 {
            self.reset();
            return false;
        }
        if self.next < 0.0 {
            self.next = wait_ms(level, stream);
        }
        self.t += dt_ms;
        if self.t < self.next {
            return false;
        }
        self.t = 0.0;
        self.next = wait_ms(level, stream);
        true
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn skip(&mut self, ms: f64) {
        self.t += ms;
    }
}

fn arrival_offset(stream: &mut NightWatchStream) -> f64 {
    (stream.next() * 2.0 - 1.0) * ARRIVE_SPREAD_MS
}

/// One hunter walking toward the player: the rooftop climber, or a campfire edge hunter.
#[derive(Clone, Debug)]
struct Stalker {
    hunter: Hunter,
    side: Side,
    on: bool,
    lvl: i32,
    dist: i32,
    pacer: Pacer,
    at_ms: f64,
    leave: f64,
    push: f64,
    lost: f64,
}

impl Stalker {
    fn new(hunter: Hunter, side: Side, lvl: i32, stream: &mut NightWatchStream) -> Self {
        let mut pacer = Pacer::new();
        pacer.skip(arrival_offset(stream));
        Self {
            hunter,
            side,
            on: lvl > 0,
            lvl,
            dist: START_DIST,
            pacer,
            at_ms: 0.0,
            leave: 0.0,
            push: 0.0,
            lost: 0.0,
        }
    }

    fn send_back(&mut self, to: i32, ev: &mut Vec<Event>) {
        self.dist = to;
        self.at_ms = 0.0;
        self.push = 0.0;
        self.lost = 0.0;
        self.pacer.reset();
        ev.push(Event::Repelled { hunter: self.hunter, side: self.side });
    }

    /// The rooftop climber. Returns true once she has waited out AT_MS at the door.
    fn climb(
        &mut self,
        dt_ms: f64,
        level: i32,
        torch: Option<Side>,
        barred: bool,
        stream: &mut NightWatchStream,
        ev: &mut Vec<Event>,
    ) -> bool {
        if self.dist > 0 {
            if torch != Some(self.side) && self.pacer.step(dt_ms, level, stream) {
                self.dist -= 1;
                ev.push(Event::Close { hunter: self.hunter, dist: self.dist, side: self.side });
            }
            return false;
        }
        if barred {
            self.at_ms = 0.0;
            self.leave += dt_ms;
            if self.leave >= LEAVE_MS {
                self.dist = 2;
                self.leave = 0.0;
                self.pacer.reset();
                if stream.next() < SIDE_SWITCH {
                    self.side = self.side.other();
                }
                ev.push(Event::Repelled { hunter: self.hunter, side: self.side });
            }
            return false;
        }
        self.leave = 0.0;
        if torch != Some(self.side) {
            self.at_ms += dt_ms;
        }
        self.at_ms >= AT_MS
    }

    /// A campfire edge hunter. The torch on her side sends Verrine back and holds Quenelle still; a tent hides you
    /// from Quenelle, who loses the trail after a while. Returns true when she reaches the fire.
    fn edge(
        &mut self,
        dt_ms: f64,
        level: i32,
        torch: Option<Side>,
        hidden: bool,
        mood: Mood,
        stream: &mut NightWatchStream,
        ev: &mut Vec<Event>,
    ) -> bool {
        if !self.on {
            return false;
        }
        let held = torch == Some(self.side);
        if self.dist > 0 {
            let pace = match (self.hunter, mood) {
                (Hunter::Verrine, Mood::Bold) => MOOD_PACE,
                (Hunter::Verrine, Mood::Shy) => 1.0 / MOOD_PACE,
                _ => 1.0,
            };
            if !held && self.pacer.step(dt_ms * pace, level, stream) {
                self.dist -= 1;
                ev.push(Event::Close { hunter: self.hunter, dist: self.dist, side: self.side });
            }
            return false;
        }
        if self.hunter == Hunter::Verrine && held {
            self.push += dt_ms;
            if self.push >= PUSH_MS {
                self.send_back(PUSH_BACK, ev);
            }
            return false;
        }
        if self.hunter == Hunter::Quenelle && hidden {
            self.lost += dt_ms;
            if self.lost >= LOST_MS {
                self.send_back(LOST_BACK, ev);
            }
            return false;
        }
        if !held {
            self.at_ms += dt_ms;
        }
        self.at_ms >= CAMP_AT_MS
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mood {
    Calm,
    Bold,
    Shy,
}

fn draw_mood(stream: &mut NightWatchStream) -> Mood {
    let u = stream.next();
    if u < MOOD_CALM {
        Mood::Calm
    } else if (u - MOOD_CALM) / (1.0 - MOOD_CALM) < 0.5 {
        Mood::Bold
    } else {
        Mood::Shy
    }
}

/// Ysolde at the campfire: dist 2 in the trees, 1 out of them, 0 at the log pile.
#[derive(Clone, Debug)]
struct Ysolde {
    dist: i32,
    fwd: Pacer,
    back: Pacer,
}

#[derive(Clone, Debug)]
struct Weather {
    kind: WeatherKind,
    on: bool,
    t: f64,
    next: f64,
    lasts: f64,
    mult: f64,
    every_ms: f64,
    lasts_ms: f64,
}

impl Weather {
    fn new(kind: WeatherKind, stream: &mut NightWatchStream) -> Self {
        let (mult, every_ms, lasts_ms, first) = match kind {
            WeatherKind::Snow => (1.8, 34_000.0, 9_000.0, 20_000.0),
            WeatherKind::Rain => (2.2, 40_000.0, 10_000.0, 25_000.0),
        };
        let next = first * (0.75 + stream.next() * 0.5);
        let lasts = jitter(lasts_ms, WEATHER_LAST_SPREAD, stream);
        Self { kind, on: false, t: 0.0, next, lasts, mult, every_ms, lasts_ms }
    }
}

#[derive(Clone, Debug)]
struct Rooftop {
    lantern: bool,
    gust: f64,
    relight: f64,
    oil: f64,
    climber: Stalker,
}

#[derive(Clone, Debug)]
struct BusStop {
    warm: f64,
    lamp: bool,
    fuel: f64,
    cans: Vec<u8>,
    can_clock: f64,
    can_at: f64,
    chill: f64,
    cold_mult: f64,
    cold_left: f64,
    bus: f64,
}

#[derive(Clone, Debug)]
struct Campfire {
    fire: f64,
    logs: u32,
    gather: f64,
    out: f64,
    hidden: bool,
    ysolde: Ysolde,
    mood: Mood,
    mood_left: f64,
    verrine: Stalker,
    quenelle: Stalker,
}

#[derive(Clone, Debug)]
enum Scene {
    Rooftop(Rooftop),
    BusStop(BusStop),
    Campfire(Campfire),
}

/// The per-step values every scene reads.
#[derive(Clone, Copy, Debug)]
struct Tick {
    dt_ms: f64,
    hour: i32,
    infinite_light: bool,
    torch: Option<Side>,
    wx_mult: f64,
    input: Input,
    levels: Levels,
}

type Caught = Option<(Hunter, Cause)>;

fn ai_bump(hour: i32) -> i32 {
    if hour >= 4 {
        2
    } else if hour >= 3 {
        1
    } else {
        0
    }
}

/// A hunter's level after the night's step-ups; a hunter at 0 stays at 0.
fn keen(level: i32, hour: i32) -> i32 {
    if level <= 0 {
        0
    } else {
        (level + ai_bump(hour)).min(MAX_AI)
    }
}

fn hour_index(elapsed_ms: f64) -> i32 {
    (elapsed_ms / HOUR_MS).floor().min(5.0) as i32
}

fn low_fire_bonus(fire: f64) -> i32 {
    if fire < LOW_FIRE {
        LOW_FIRE_BONUS
    } else {
        0
    }
}

fn step_rooftop(r: &mut Rooftop, t: &Tick, stream: &mut NightWatchStream, ev: &mut Vec<Event>) -> Caught {
    let dt = t.dt_ms / 1000.0;
    let held_left = t.input.bar_left && r.oil > 0.0;
    let held_right = t.input.bar_right && r.oil > 0.0;
    let strain = f64::from(i32::from(held_left) + i32::from(held_right));
    if !t.infinite_light {
        r.oil = (r.oil - (OIL_BASE + OIL_PER_BAR * strain) * dt).max(0.0);
    }
    if r.lantern && r.oil <= 0.0 {
        r.lantern = false;
        r.relight = 0.0;
        ev.push(Event::Dry);
    }
    r.gust -= t.dt_ms;
    if r.lantern && r.gust <= 0.0 {
        r.lantern = false;
        r.relight = 0.0;
        ev.push(Event::Gust);
    }
    if !r.lantern {
        r.relight = if t.input.relight && r.oil >= RELIGHT_OIL {
            r.relight + t.dt_ms
        } else {
            (r.relight - t.dt_ms).max(0.0)
        };
        if r.relight >= RELIGHT_MS {
            r.lantern = true;
            r.relight = 0.0;
            r.oil -= RELIGHT_OIL;
            r.gust = GUST_MS + stream.next() * GUST_JITTER_MS;
            ev.push(Event::Relit);
        }
    }
    let barred = match r.climber.side {
        Side::Left => held_left,
        Side::Right => held_right,
    };
    let base = t.levels.verrine + if r.lantern { 0 } else { DARK_BONUS };
    let arrived = r.climber.climb(t.dt_ms, keen(base, t.hour), t.torch, barred, stream, ev);
    arrived.then_some((Hunter::Verrine, Cause::Edge))
}

fn step_bus(b: &mut BusStop, t: &Tick, stream: &mut NightWatchStream, ev: &mut Vec<Event>) -> Caught {
    let dt = t.dt_ms / 1000.0;
    let lamp_on = b.lamp && b.fuel > 0.0;
    b.cold_left -= t.dt_ms;
    if b.cold_left <= 0.0 {
        b.cold_mult = jitter(1.0, COLD_SPREAD, stream);
        b.cold_left = jitter(COLD_EVERY_MS, COLD_EVERY_SPREAD, stream);
    }
    let heat = if lamp_on { LAMP_WARM } else { 0.0 };
    b.warm = (b.warm + (heat - COLD_DRAIN * t.wx_mult * b.cold_mult) * dt).clamp(0.0, 100.0);
    if lamp_on {
        b.fuel = (b.fuel - LAMP_FUEL * dt).max(0.0);
        if b.fuel <= 0.0 {
            b.lamp = false;
            ev.push(Event::Dry);
        }
    }
    b.can_clock += t.dt_ms;
    if b.can_clock >= b.can_at && b.cans.len() < MAX_CANS {
        b.can_clock = 0.0;
        b.can_at = jitter(CAN_EVERY_MS, CAN_SPREAD, stream);
        let free: Vec<u8> = (0..MAX_CANS as u8).filter(|i| !b.cans.contains(i)).collect();
        b.cans.push(free[pick_index(stream, free.len())]);
        ev.push(Event::Can);
    }
    let factor = (f64::from(t.levels.wynelle) / 10.0).max(0.4);
    if b.bus > 0.0 {
        b.chill = (b.chill - CHILL_RECOVER * 2.0 * dt).max(0.0);
    } else if b.warm < CHILL_LINE {
        b.chill = (b.chill + (CHILL_LINE - b.warm) * CHILL_PER_POINT * factor * dt).min(100.0);
    } else {
        b.chill = (b.chill - CHILL_RECOVER * dt).max(0.0);
    }
    (b.chill >= 100.0).then_some((Hunter::Wynelle, Cause::Cold))
}

fn dark_by(c: &Campfire, ysolde_level: i32) -> Hunter {
    if ysolde_level > 0 {
        return Hunter::Ysolde;
    }
    let mut best: Option<&Stalker> = None;
    for h in [&c.quenelle, &c.verrine] {
        if h.on && best.map_or(true, |b| h.dist < b.dist) {
            best = Some(h);
        }
    }
    best.map_or(Hunter::Ysolde, |b| b.hunter)
}

fn step_campfire(c: &mut Campfire, t: &Tick, stream: &mut NightWatchStream, ev: &mut Vec<Event>) -> Caught {
    let dt = t.dt_ms / 1000.0;
    c.mood_left -= t.dt_ms;
    if c.mood_left <= 0.0 {
        c.mood = draw_mood(stream);
        c.mood_left = jitter(MOOD_EVERY_MS, MOOD_EVERY_SPREAD, stream);
    }
    c.fire = (c.fire - CAMP_DRAIN * t.wx_mult * dt).max(0.0);
    if t.input.gather && !c.hidden {
        c.gather += t.dt_ms;
        if c.gather >= GATHER_MS {
            c.gather = 0.0;
            c.logs += 1;
            ev.push(Event::Gathered);
        }
    } else {
        c.gather = 0.0;
    }
    if c.fire <= 0.0 {
        c.out += t.dt_ms;
        if c.out >= OUT_GRACE_MS {
            return Some((dark_by(c, t.levels.ysolde), Cause::Fire));
        }
    } else {
        c.out = 0.0;
    }

    let y_level = keen(t.levels.ysolde, t.hour);
    if c.hidden {
        if c.ysolde.fwd.step(t.dt_ms * TENT_PACE, y_level, stream) {
            c.ysolde.dist -= 1;
            if c.ysolde.dist == 1 {
                ev.push(Event::Creep);
            } else if c.ysolde.dist == 0 {
                if c.logs > 0 {
                    c.logs -= 1;
                } else {
                    c.fire = (c.fire - FUEL_STEAL).max(0.0);
                }
                ev.push(Event::Stolen);
            } else {
                return Some((Hunter::Ysolde, Cause::Tent));
            }
        }
    } else if c.ysolde.dist < 2 && c.ysolde.back.step(t.dt_ms * BACK_PACE, y_level, stream) {
        c.ysolde.dist += 1;
        ev.push(Event::Backoff { dist: c.ysolde.dist });
    }

    let low = low_fire_bonus(c.fire);
    if c.verrine.edge(t.dt_ms, keen(c.verrine.lvl, t.hour) + low, t.torch, c.hidden, c.mood, stream, ev) {
        return Some((Hunter::Verrine, Cause::Edge));
    }
    if c.quenelle.edge(t.dt_ms, keen(c.quenelle.lvl, t.hour) + low, t.torch, c.hidden, c.mood, stream, ev) {
        return Some((Hunter::Quenelle, Cause::Edge));
    }
    None
}

/// One watch of Open Air, in progress.
#[derive(Clone, Debug)]
pub struct OpenAir {
    watch: Watch,
    levels: Levels,
    stream: NightWatchStream,
    infinite_light: bool,
    elapsed: f64,
    outcome: Outcome,
    torch: f64,
    torch_side: Option<Side>,
    weather: Option<Weather>,
    scene: Scene,
}

impl OpenAir {
    /// Starts `watch` with a seeded stream (the same seed replays the same watch). `infinite_light` keeps the torch
    /// and the lantern's oil from running out, for tests and the admin tools.
    pub fn new(watch: Watch, seed: i64, levels: Levels, infinite_light: bool) -> Self {
        let mut stream = NightWatchStream::seeded(seed);
        let weather = match watch {
            Watch::Rooftop => None,
            Watch::BusStop => Some(Weather::new(WeatherKind::Snow, &mut stream)),
            Watch::Campfire => Some(Weather::new(WeatherKind::Rain, &mut stream)),
        };
        let scene = match watch {
            Watch::Rooftop => {
                let gust = jitter(GUST_MS, GUST_FIRST_SPREAD, &mut stream);
                let side = if stream.next() < 0.5 { Side::Left } else { Side::Right };
                let climber = Stalker::new(Hunter::Verrine, side, levels.verrine, &mut stream);
                Scene::Rooftop(Rooftop { lantern: true, gust, relight: 0.0, oil: 100.0, climber })
            }
            Watch::BusStop => {
                let can_at = jitter(CAN_FIRST_MS, CAN_FIRST_SPREAD, &mut stream);
                let cold_mult = jitter(1.0, COLD_SPREAD, &mut stream);
                let cold_left = jitter(COLD_EVERY_MS, COLD_EVERY_SPREAD, &mut stream);
                Scene::BusStop(BusStop {
                    warm: 100.0,
                    lamp: false,
                    fuel: 100.0,
                    cans: Vec::new(),
                    can_clock: 0.0,
                    can_at,
                    chill: 0.0,
                    cold_mult,
                    cold_left,
                    bus: 0.0,
                })
            }
            Watch::Campfire => {
                let mood = draw_mood(&mut stream);
                let mood_left = jitter(MOOD_EVERY_MS, MOOD_EVERY_SPREAD, &mut stream);
                let verrine = Stalker::new(Hunter::Verrine, Side::Left, levels.verrine, &mut stream);
                let quenelle = Stalker::new(Hunter::Quenelle, Side::Right, levels.quenelle, &mut stream);
                Scene::Campfire(Campfire {
                    fire: 100.0,
                    logs: START_LOGS,
                    gather: 0.0,
                    out: 0.0,
                    hidden: false,
                    ysolde: Ysolde { dist: 2, fwd: Pacer::new(), back: Pacer::new() },
                    mood,
                    mood_left,
                    verrine,
                    quenelle,
                })
            }
        };
        Self {
            watch,
            levels,
            stream,
            infinite_light,
            elapsed: 0.0,
            outcome: Outcome::Playing,
            torch: 100.0,
            torch_side: None,
            weather,
            scene,
        }
    }

    pub fn watch(&self) -> Watch {
        self.watch
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.elapsed
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// Advances the watch by `dt_ms` with what is held. Nothing happens once the watch is won or lost.
    pub fn step(&mut self, dt_ms: f64, input: Input) -> Vec<Event> {
        let mut ev = Vec::new();
        if self.outcome != Outcome::Playing {
            return ev;
        }
        self.elapsed += dt_ms;
        let wx_mult = self.step_weather(dt_ms, &mut ev);

        if let Scene::Campfire(c) = &mut self.scene {
            if input.tent != c.hidden {
                c.hidden = input.tent;
                c.gather = 0.0;
                ev.push(if c.hidden { Event::TentIn } else { Event::TentOut });
            }
        }
        let campfire = matches!(self.scene, Scene::Campfire(_));
        let hidden = matches!(&self.scene, Scene::Campfire(c) if c.hidden);
        let busy = campfire && (input.gather || hidden);
        let want = if busy {
            None
        } else if input.torch_left {
            Some(Side::Left)
        } else if input.torch_right {
            Some(Side::Right)
        } else {
            None
        };
        self.torch_side = if self.torch > 0.0 { want } else { None };
        let dt = dt_ms / 1000.0;
        if self.torch_side.is_some() && !self.infinite_light {
            self.torch = (self.torch - TORCH_DRAIN * dt).max(0.0);
        } else {
            self.torch = (self.torch + TORCH_REGEN * dt).min(100.0);
        }

        let tick = Tick {
            dt_ms,
            hour: hour_index(self.elapsed),
            infinite_light: self.infinite_light,
            torch: self.torch_side,
            wx_mult,
            input,
            levels: self.levels,
        };
        let caught = match &mut self.scene {
            Scene::Rooftop(r) => step_rooftop(r, &tick, &mut self.stream, &mut ev),
            Scene::BusStop(b) => step_bus(b, &tick, &mut self.stream, &mut ev),
            Scene::Campfire(c) => step_campfire(c, &tick, &mut self.stream, &mut ev),
        };
        if let Some((by, cause)) = caught {
            self.outcome = Outcome::Caught { by, cause };
            ev.push(Event::Caught { by, cause });
        } else if self.elapsed >= WATCH_MS {
            self.outcome = Outcome::Won;
            ev.push(Event::Win);
        }
        ev
    }

    /// The lamp, a canister or a log, as the player asks for them. True when it did something.
    pub fn act(&mut self, action: Action) -> bool {
        if self.outcome != Outcome::Playing {
            return false;
        }
        match (action, &mut self.scene) {
            (Action::Lamp, Scene::BusStop(b)) if b.fuel > 0.0 || b.lamp => {
                b.lamp = !b.lamp;
                true
            }
            (Action::Can(i), Scene::BusStop(b)) => match b.cans.iter().position(|&c| c == i) {
                Some(pos) => {
                    b.cans.remove(pos);
                    b.fuel = (b.fuel + CAN_FUEL).min(100.0);
                    true
                }
                None => false,
            },
            (Action::Log, Scene::Campfire(c)) if c.logs > 0 && !c.hidden => {
                c.logs -= 1;
                c.fire = (c.fire + LOG_FUEL).min(100.0);
                true
            }
            _ => false,
        }
    }

    /// The weather's multiplier this step. On the bus stop's last seconds the weather ends and the bus comes.
    fn step_weather(&mut self, dt_ms: f64, ev: &mut Vec<Event>) -> f64 {
        let bus_due = self.elapsed >= WATCH_MS - BUS_MS;
        let elapsed = self.elapsed;
        let stream = &mut self.stream;
        let Some(w) = self.weather.as_mut() else {
            return 1.0;
        };
        if bus_due && matches!(self.scene, Scene::BusStop(_)) {
            if w.on {
                w.on = false;
                ev.push(Event::WeatherEnd(w.kind));
            }
            if let Scene::BusStop(b) = &mut self.scene {
                if b.bus == 0.0 {
                    ev.push(Event::Bus);
                }
                b.bus = ((elapsed - (WATCH_MS - BUS_MS)) / BUS_MS).min(1.0);
            }
            return 1.0;
        }
        w.t += dt_ms;
        if !w.on && w.t >= w.next {
            w.on = true;
            w.t = 0.0;
            ev.push(Event::Weather(w.kind));
        } else if w.on && w.t >= w.lasts {
            w.on = false;
            w.t = 0.0;
            w.next = w.every_ms * (0.75 + stream.next() * 0.5);
            w.lasts = jitter(w.lasts_ms, WEATHER_LAST_SPREAD, stream);
            ev.push(Event::WeatherEnd(w.kind));
        }
        if w.on {
            w.mult
        } else {
            1.0
        }
    }
}

/// The Custom Night chain: the watches whose hunters are on, in order. An empty chain runs the rooftop.
pub fn custom_watches(levels: Levels) -> Vec<Watch> {
    let chain: Vec<Watch> = Watch::ALL
        .iter()
        .copied()
        .filter(|&w| match w {
            Watch::Campfire => levels.ysolde > 0 || levels.quenelle > 0,
            _ => levels.of(w.hunter()) > 0,
        })
        .collect();
    if chain.is_empty() {
        vec![Watch::Rooftop]
    } else {
        chain
    }
}

/// The score of a Custom Night: every slider (0 to 20) summed and times the points per level.
pub fn custom_score(levels: Levels) -> i32 {
    levels.sum_clamped() * POINTS_PER_LEVEL
}

/// The call script for a watch (Zellia). A Custom Night has no calls.
pub fn call_script(calls: &CallsData, watch: Watch, custom: bool) -> CallScript {
    if custom {
        CallScript::new(Vec::new(), 0.0)
    } else {
        CallScript::for_level(calls, MODE_ID, watch.level())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_torch_on_the_climbers_side_holds_her_at_the_door() {
        let mut watch = OpenAir::new(Watch::Rooftop, 7, Levels::LISTED, true);
        let side = match &watch.scene {
            Scene::Rooftop(r) => r.climber.side,
            _ => unreachable!("the rooftop watch has a climber"),
        };
        let held = Input {
            torch_left: side == Side::Left,
            torch_right: side == Side::Right,
            ..Input::default()
        };
        let mut events = Vec::new();
        for _ in 0..600 {
            events.extend(watch.step(100.0, held));
        }
        assert!(!events.iter().any(|e| matches!(e, Event::Close { .. })));
        assert_eq!(watch.outcome(), Outcome::Playing);
        match &watch.scene {
            Scene::Rooftop(r) => assert_eq!(r.climber.dist, START_DIST),
            _ => unreachable!("the rooftop watch has a climber"),
        }
    }
}
