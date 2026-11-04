//! Open Air night type (Godot `game/night_watch/openair`, mode id `extra`): three outdoor watches, each with its own
//! hunter and its own way of keeping the dark off. Ported from `openair_logic.gd` (the 2D `createWatchGame`): rules,
//! weather, pacing, the Custom Night chain and the score. The walkable sets, art, sound and admin hooks are not ported.
//! The shell runs one `OpenAir` per watch: `step` with what is held each frame, `act` for the lamp, a canister or a log.

use crate::data::{self, CallsData};
use crate::framework::{jitter, pick_index, CallScript, NightWatchStream};
use serde_json::Value;
use std::sync::OnceLock;

pub const MODE_ID: &str = "extra";

/// A pacer's three-second window: `base_ms` minus `per_level_ms` a level, never under `min_ms`.
#[derive(Debug)]
struct PacerRule {
    base_ms: f64,
    per_level_ms: f64,
    min_ms: f64,
    window_ms: f64,
}

/// One weather's timing and how hard it bites.
#[derive(Debug)]
struct WeatherRule {
    every_ms: f64,
    lasts_ms: f64,
    mult: f64,
    first_ms: f64,
}

/// Every tunable of Open Air, read once from data/rules/open_air.json and data/cast/open_air.json (see docs/NIGHT_WATCH_DATA.md).
#[derive(Debug)]
struct Rules {
    watch_ms: f64,
    hour_ms: f64,
    last_hour: f64,
    meter_max: f64,
    start_oil: f64,
    repel_dist: i32,
    bus_recover_mult: f64,
    chill_level_div: f64,
    chill_factor_min: f64,
    mood_bold_share: f64,
    weather_gap_spread: f64,
    ai_steps: Vec<(i32, i32)>,
    pacer: PacerRule,
    snow: WeatherRule,
    rain: WeatherRule,
    names: [&'static str; 4],
    labels: [&'static str; 3],
    watch_hunters: [Hunter; 3],
    verrine_side: Side,
    quenelle_side: Side,
    points_per_level: i32,
    max_ai: i32,
    bus_ms: f64,
    start_dist: i32,
    at_ms: f64,
    leave_ms: f64,
    side_switch: f64,
    arrive_spread_ms: f64,
    dark_bonus: i32,
    gust_ms: f64,
    gust_jitter_ms: f64,
    gust_first_spread: f64,
    relight_ms: f64,
    torch_drain: f64,
    torch_regen: f64,
    oil_base: f64,
    oil_per_bar: f64,
    relight_oil: f64,
    cold_drain: f64,
    lamp_warm: f64,
    lamp_fuel: f64,
    chill_line: f64,
    chill_per_point: f64,
    chill_recover: f64,
    cold_spread: f64,
    cold_every_ms: f64,
    cold_every_spread: f64,
    can_first_ms: f64,
    can_first_spread: f64,
    can_every_ms: f64,
    can_spread: f64,
    can_fuel: f64,
    max_cans: usize,
    weather_last_spread: f64,
    camp_drain: f64,
    log_fuel: f64,
    start_logs: u32,
    gather_ms: f64,
    out_grace_ms: f64,
    fuel_steal: f64,
    camp_at_ms: f64,
    push_ms: f64,
    push_back: i32,
    lost_ms: f64,
    lost_back: i32,
    tent_pace: f64,
    back_pace: f64,
    low_fire: f64,
    low_fire_bonus: i32,
    mood_every_ms: f64,
    mood_every_spread: f64,
    mood_pace: f64,
    mood_calm: f64,
}

fn weather_rule(g: &'static Value, kind: &str) -> WeatherRule {
    WeatherRule {
        every_ms: data::num(g, &format!("weather.{kind}.every_ms")),
        lasts_ms: data::num(g, &format!("weather.{kind}.lasts_ms")),
        mult: data::num(g, &format!("weather.{kind}.mult")),
        first_ms: data::num(g, &format!("weather.{kind}.first_ms")),
    }
}

fn side_of(text: &str) -> Side {
    match text {
        "left" => Side::Left,
        "right" => Side::Right,
        _ => panic!("night_watch data for mode 'open_air' names an unknown side '{text}'"),
    }
}

impl Rules {
    fn load() -> Rules {
        use data::{int, list, num, text};
        let g = data::rules("open_air");
        let c = data::cast("open_air");
        let watch_ms = num(g, "watch_ms");
        let hours = int(g, "hours");
        let hunter_names = |i: usize| Hunter::ALL[i].key();
        let watch_at = |i: usize, key: &str| text(c, &format!("watches.{i}.{key}"));
        Rules {
            watch_ms,
            hour_ms: watch_ms / hours as f64,
            last_hour: (hours - 1) as f64,
            meter_max: num(g, "meter_max"),
            start_oil: num(g, "rooftop.start_oil"),
            repel_dist: int(g, "stalker.repel_dist") as i32,
            bus_recover_mult: num(g, "bus_stop.bus_recover_mult"),
            chill_level_div: num(g, "bus_stop.chill_level_div"),
            chill_factor_min: num(g, "bus_stop.chill_factor_min"),
            mood_bold_share: num(g, "campfire.mood_bold_share"),
            weather_gap_spread: num(g, "weather.gap_spread"),
            ai_steps: list(g, "ai_steps").iter().map(|s| (int(s, "from_hour") as i32, int(s, "bump") as i32)).collect(),
            pacer: PacerRule {
                base_ms: num(g, "pacer.base_ms"),
                per_level_ms: num(g, "pacer.per_level_ms"),
                min_ms: num(g, "pacer.min_ms"),
                window_ms: num(g, "pacer.window_ms"),
            },
            snow: weather_rule(g, "snow"),
            rain: weather_rule(g, "rain"),
            names: std::array::from_fn(|i| text(c, &format!("hunters.{}.name", hunter_names(i)))),
            labels: std::array::from_fn(|i| watch_at(i, "label")),
            watch_hunters: std::array::from_fn(|i| Hunter::from_key(watch_at(i, "hunter"))),
            verrine_side: side_of(text(c, "campfire_sides.verrine")),
            quenelle_side: side_of(text(c, "campfire_sides.quenelle")),
        points_per_level: int(g, "points_per_level") as i32,
        max_ai: int(g, "max_ai") as i32,
        bus_ms: num(g, "bus_ms"),
        start_dist: int(g, "stalker.start_dist") as i32,
        at_ms: num(g, "stalker.at_ms"),
        leave_ms: num(g, "stalker.leave_ms"),
        side_switch: num(g, "stalker.side_switch"),
        arrive_spread_ms: num(g, "stalker.arrive_spread_ms"),
        dark_bonus: int(g, "stalker.dark_bonus") as i32,
        gust_ms: num(g, "rooftop.gust_ms"),
        gust_jitter_ms: num(g, "rooftop.gust_jitter_ms"),
        gust_first_spread: num(g, "rooftop.gust_first_spread"),
        relight_ms: num(g, "rooftop.relight_ms"),
        torch_drain: num(g, "torch.drain"),
        torch_regen: num(g, "torch.regen"),
        oil_base: num(g, "rooftop.oil_base"),
        oil_per_bar: num(g, "rooftop.oil_per_bar"),
        relight_oil: num(g, "rooftop.relight_oil"),
        cold_drain: num(g, "bus_stop.cold_drain"),
        lamp_warm: num(g, "bus_stop.lamp_warm"),
        lamp_fuel: num(g, "bus_stop.lamp_fuel"),
        chill_line: num(g, "bus_stop.chill_line"),
        chill_per_point: num(g, "bus_stop.chill_per_point"),
        chill_recover: num(g, "bus_stop.chill_recover"),
        cold_spread: num(g, "bus_stop.cold_spread"),
        cold_every_ms: num(g, "bus_stop.cold_every_ms"),
        cold_every_spread: num(g, "bus_stop.cold_every_spread"),
        can_first_ms: num(g, "bus_stop.can_first_ms"),
        can_first_spread: num(g, "bus_stop.can_first_spread"),
        can_every_ms: num(g, "bus_stop.can_every_ms"),
        can_spread: num(g, "bus_stop.can_spread"),
        can_fuel: num(g, "bus_stop.can_fuel"),
        max_cans: int(g, "bus_stop.max_cans") as usize,
        weather_last_spread: num(g, "weather.last_spread"),
        camp_drain: num(g, "campfire.drain"),
        log_fuel: num(g, "campfire.log_fuel"),
        start_logs: int(g, "campfire.start_logs") as u32,
        gather_ms: num(g, "campfire.gather_ms"),
        out_grace_ms: num(g, "campfire.out_grace_ms"),
        fuel_steal: num(g, "campfire.fuel_steal"),
        camp_at_ms: num(g, "campfire.at_ms"),
        push_ms: num(g, "campfire.push_ms"),
        push_back: int(g, "campfire.push_back") as i32,
        lost_ms: num(g, "campfire.lost_ms"),
        lost_back: int(g, "campfire.lost_back") as i32,
        tent_pace: num(g, "campfire.tent_pace"),
        back_pace: num(g, "campfire.back_pace"),
        low_fire: num(g, "campfire.low_fire"),
        low_fire_bonus: int(g, "campfire.low_fire_bonus") as i32,
        mood_every_ms: num(g, "campfire.mood_every_ms"),
        mood_every_spread: num(g, "campfire.mood_every_spread"),
        mood_pace: num(g, "campfire.mood_pace"),
        mood_calm: num(g, "campfire.mood_calm"),
        }
    }
}

/// The loaded rules. Read on `OpenAir::new` and by the small helpers; the file is parsed once (see `data::rules`).
fn tune() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(Rules::load)
}





#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hunter {
    Verrine,
    Wynelle,
    Ysolde,
    Quenelle,
}

impl Hunter {
    const ALL: [Hunter; 4] = [Hunter::Verrine, Hunter::Wynelle, Hunter::Ysolde, Hunter::Quenelle];

    /// The key the data files use for this hunter.
    fn key(self) -> &'static str {
        match self {
            Hunter::Verrine => "verrine",
            Hunter::Wynelle => "wynelle",
            Hunter::Ysolde => "ysolde",
            Hunter::Quenelle => "quenelle",
        }
    }

    pub fn name(self) -> &'static str {
        tune().names[self as usize]
    }

    /// The hunter a data file calls `key` ("verrine"). A key that names nobody is a content bug.
    fn from_key(key: &str) -> Hunter {
        match key {
            "verrine" => Hunter::Verrine,
            "wynelle" => Hunter::Wynelle,
            "ysolde" => Hunter::Ysolde,
            "quenelle" => Hunter::Quenelle,
            _ => panic!("night_watch data for mode 'open_air' names an unknown hunter '{key}'"),
        }
    }
}

/// AI levels for the four hunters of this mode. `Levels::listed()` is the mode's own table; a Custom Night sets its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Levels {
    pub verrine: i32,
    pub wynelle: i32,
    pub ysolde: i32,
    pub quenelle: i32,
}

impl Levels {
    /// The mode's own table, read from `levels.listed` in data/rules/open_air.json (the only place it is written).
    pub fn listed() -> Levels {
        let g = data::rules("open_air");
        let at = |hunter: Hunter| data::int(g, &format!("levels.listed.{}", hunter.key())) as i32;
        Levels {
            verrine: at(Hunter::Verrine),
            wynelle: at(Hunter::Wynelle),
            ysolde: at(Hunter::Ysolde),
            quenelle: at(Hunter::Quenelle),
        }
    }

    pub fn of(self, hunter: Hunter) -> i32 {
        match hunter {
            Hunter::Verrine => self.verrine,
            Hunter::Wynelle => self.wynelle,
            Hunter::Ysolde => self.ysolde,
            Hunter::Quenelle => self.quenelle,
        }
    }

    fn sum_clamped(self) -> i32 {
        Hunter::ALL
            .iter()
            .map(|&h| self.of(h).clamp(0, tune().max_ai))
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
        tune().labels[self as usize]
    }

    /// The watch's own hunter. The campfire's own hunter is Ysolde; Verrine and Quenelle come from the trees too.
    fn hunter(self) -> Hunter {
        tune().watch_hunters[self as usize]
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
    let p = &tune().pacer;
    let lo = (p.base_ms - f64::from(level) * p.per_level_ms).max(p.min_ms);
    lo + stream.next() * p.window_ms
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
    (stream.next() * 2.0 - 1.0) * tune().arrive_spread_ms
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
            dist: tune().start_dist,
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

    /// The rooftop climber. Returns true once she has waited out tune().at_ms at the door.
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
            if self.leave >= tune().leave_ms {
                self.dist = tune().repel_dist;
                self.leave = 0.0;
                self.pacer.reset();
                if stream.next() < tune().side_switch {
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
        self.at_ms >= tune().at_ms
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
                (Hunter::Verrine, Mood::Bold) => tune().mood_pace,
                (Hunter::Verrine, Mood::Shy) => 1.0 / tune().mood_pace,
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
            if self.push >= tune().push_ms {
                self.send_back(tune().push_back, ev);
            }
            return false;
        }
        if self.hunter == Hunter::Quenelle && hidden {
            self.lost += dt_ms;
            if self.lost >= tune().lost_ms {
                self.send_back(tune().lost_back, ev);
            }
            return false;
        }
        if !held {
            self.at_ms += dt_ms;
        }
        self.at_ms >= tune().camp_at_ms
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
    if u < tune().mood_calm {
        Mood::Calm
    } else if (u - tune().mood_calm) / (1.0 - tune().mood_calm) < tune().mood_bold_share {
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
        let rule = match kind {
            WeatherKind::Snow => &tune().snow,
            WeatherKind::Rain => &tune().rain,
        };
        let next = jitter(rule.first_ms, tune().weather_gap_spread, stream);
        let lasts = jitter(rule.lasts_ms, tune().weather_last_spread, stream);
        Self { kind, on: false, t: 0.0, next, lasts, mult: rule.mult, every_ms: rule.every_ms, lasts_ms: rule.lasts_ms }
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
    tune().ai_steps.iter().rev().find(|&&(from, _)| hour >= from).map_or(0, |&(_, bump)| bump)
}

/// A hunter's level after the night's step-ups; a hunter at 0 stays at 0.
fn keen(level: i32, hour: i32) -> i32 {
    if level <= 0 {
        0
    } else {
        (level + ai_bump(hour)).min(tune().max_ai)
    }
}

fn hour_index(elapsed_ms: f64) -> i32 {
    (elapsed_ms / tune().hour_ms).floor().min(tune().last_hour) as i32
}

fn low_fire_bonus(fire: f64) -> i32 {
    if fire < tune().low_fire {
        tune().low_fire_bonus
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
        r.oil = (r.oil - (tune().oil_base + tune().oil_per_bar * strain) * dt).max(0.0);
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
        r.relight = if t.input.relight && r.oil >= tune().relight_oil {
            r.relight + t.dt_ms
        } else {
            (r.relight - t.dt_ms).max(0.0)
        };
        if r.relight >= tune().relight_ms {
            r.lantern = true;
            r.relight = 0.0;
            r.oil -= tune().relight_oil;
            r.gust = tune().gust_ms + stream.next() * tune().gust_jitter_ms;
            ev.push(Event::Relit);
        }
    }
    let barred = match r.climber.side {
        Side::Left => held_left,
        Side::Right => held_right,
    };
    let base = t.levels.verrine + if r.lantern { 0 } else { tune().dark_bonus };
    let arrived = r.climber.climb(t.dt_ms, keen(base, t.hour), t.torch, barred, stream, ev);
    arrived.then_some((Hunter::Verrine, Cause::Edge))
}

fn step_bus(b: &mut BusStop, t: &Tick, stream: &mut NightWatchStream, ev: &mut Vec<Event>) -> Caught {
    let dt = t.dt_ms / 1000.0;
    let lamp_on = b.lamp && b.fuel > 0.0;
    b.cold_left -= t.dt_ms;
    if b.cold_left <= 0.0 {
        b.cold_mult = jitter(1.0, tune().cold_spread, stream);
        b.cold_left = jitter(tune().cold_every_ms, tune().cold_every_spread, stream);
    }
    let heat = if lamp_on { tune().lamp_warm } else { 0.0 };
    b.warm = (b.warm + (heat - tune().cold_drain * t.wx_mult * b.cold_mult) * dt).clamp(0.0, tune().meter_max);
    if lamp_on {
        b.fuel = (b.fuel - tune().lamp_fuel * dt).max(0.0);
        if b.fuel <= 0.0 {
            b.lamp = false;
            ev.push(Event::Dry);
        }
    }
    b.can_clock += t.dt_ms;
    if b.can_clock >= b.can_at && b.cans.len() < tune().max_cans {
        b.can_clock = 0.0;
        b.can_at = jitter(tune().can_every_ms, tune().can_spread, stream);
        let free: Vec<u8> = (0..tune().max_cans as u8).filter(|i| !b.cans.contains(i)).collect();
        b.cans.push(free[pick_index(stream, free.len())]);
        ev.push(Event::Can);
    }
    let factor = (f64::from(t.levels.wynelle) / tune().chill_level_div).max(tune().chill_factor_min);
    if b.bus > 0.0 {
        b.chill = (b.chill - tune().chill_recover * tune().bus_recover_mult * dt).max(0.0);
    } else if b.warm < tune().chill_line {
        b.chill = (b.chill + (tune().chill_line - b.warm) * tune().chill_per_point * factor * dt).min(tune().meter_max);
    } else {
        b.chill = (b.chill - tune().chill_recover * dt).max(0.0);
    }
    (b.chill >= tune().meter_max).then_some((Hunter::Wynelle, Cause::Cold))
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
        c.mood_left = jitter(tune().mood_every_ms, tune().mood_every_spread, stream);
    }
    c.fire = (c.fire - tune().camp_drain * t.wx_mult * dt).max(0.0);
    if t.input.gather && !c.hidden {
        c.gather += t.dt_ms;
        if c.gather >= tune().gather_ms {
            c.gather = 0.0;
            c.logs += 1;
            ev.push(Event::Gathered);
        }
    } else {
        c.gather = 0.0;
    }
    if c.fire <= 0.0 {
        c.out += t.dt_ms;
        if c.out >= tune().out_grace_ms {
            return Some((dark_by(c, t.levels.ysolde), Cause::Fire));
        }
    } else {
        c.out = 0.0;
    }

    let y_level = keen(t.levels.ysolde, t.hour);
    if c.hidden {
        if c.ysolde.fwd.step(t.dt_ms * tune().tent_pace, y_level, stream) {
            c.ysolde.dist -= 1;
            if c.ysolde.dist == 1 {
                ev.push(Event::Creep);
            } else if c.ysolde.dist == 0 {
                if c.logs > 0 {
                    c.logs -= 1;
                } else {
                    c.fire = (c.fire - tune().fuel_steal).max(0.0);
                }
                ev.push(Event::Stolen);
            } else {
                return Some((Hunter::Ysolde, Cause::Tent));
            }
        }
    } else if c.ysolde.dist < 2 && c.ysolde.back.step(t.dt_ms * tune().back_pace, y_level, stream) {
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
                let gust = jitter(tune().gust_ms, tune().gust_first_spread, &mut stream);
                let side = if stream.next() < 0.5 { Side::Left } else { Side::Right };
                let climber = Stalker::new(Hunter::Verrine, side, levels.verrine, &mut stream);
                Scene::Rooftop(Rooftop { lantern: true, gust, relight: 0.0, oil: tune().start_oil, climber })
            }
            Watch::BusStop => {
                let can_at = jitter(tune().can_first_ms, tune().can_first_spread, &mut stream);
                let cold_mult = jitter(1.0, tune().cold_spread, &mut stream);
                let cold_left = jitter(tune().cold_every_ms, tune().cold_every_spread, &mut stream);
                Scene::BusStop(BusStop {
                    warm: tune().meter_max,
                    lamp: false,
                    fuel: tune().meter_max,
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
                let mood_left = jitter(tune().mood_every_ms, tune().mood_every_spread, &mut stream);
                let verrine = Stalker::new(Hunter::Verrine, tune().verrine_side, levels.verrine, &mut stream);
                let quenelle = Stalker::new(Hunter::Quenelle, tune().quenelle_side, levels.quenelle, &mut stream);
                Scene::Campfire(Campfire {
                    fire: tune().meter_max,
                    logs: tune().start_logs,
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
            torch: tune().meter_max,
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
            self.torch = (self.torch - tune().torch_drain * dt).max(0.0);
        } else {
            self.torch = (self.torch + tune().torch_regen * dt).min(tune().meter_max);
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
        } else if self.elapsed >= tune().watch_ms {
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
                    b.fuel = (b.fuel + tune().can_fuel).min(tune().meter_max);
                    true
                }
                None => false,
            },
            (Action::Log, Scene::Campfire(c)) if c.logs > 0 && !c.hidden => {
                c.logs -= 1;
                c.fire = (c.fire + tune().log_fuel).min(tune().meter_max);
                true
            }
            _ => false,
        }
    }

    /// The weather's multiplier this step. On the bus stop's last seconds the weather ends and the bus comes.
    fn step_weather(&mut self, dt_ms: f64, ev: &mut Vec<Event>) -> f64 {
        let bus_due = self.elapsed >= tune().watch_ms - tune().bus_ms;
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
                b.bus = ((elapsed - (tune().watch_ms - tune().bus_ms)) / tune().bus_ms).min(1.0);
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
            w.next = jitter(w.every_ms, tune().weather_gap_spread, stream);
            w.lasts = jitter(w.lasts_ms, tune().weather_last_spread, stream);
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
    levels.sum_clamped() * tune().points_per_level
}

/// The call script for a watch (Zellia). A Custom Night has no calls.
pub fn call_script(calls: &CallsData, watch: Watch, custom: bool) -> CallScript {
    if custom {
        CallScript::new(Vec::new(), 0.0)
    } else {
        CallScript::for_level(calls, MODE_ID, watch.level())
    }
}

/// One hunter as the picture shows her: where she is on her approach (`dist` 0 is at the door, the fire or the bus).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glimpse {
    pub hunter: Hunter,
    pub side: Option<Side>,
    pub dist: i32,
}

/// The watch's own meters and fixtures, as the picture reads them.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneSeen {
    Rooftop { lantern: bool, oil: f64, relight: f64 },
    BusStop { warm: f64, lamp: bool, fuel: f64, cans: Vec<u8>, chill: f64 },
    Campfire { fire: f64, logs: u32, gather: f64, hidden: bool },
}

/// A read-only picture of a watch: the rules state copied out, nothing changed. The driver draws from it and picks
/// its scripted input from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub watch: Watch,
    pub hour: i32,
    pub torch: f64,
    pub torch_side: Option<Side>,
    pub weather: Option<WeatherKind>,
    pub hunters: Vec<Glimpse>,
    pub scene: SceneSeen,
}

impl OpenAir {
    /// The picture of this watch right now.
    pub fn seen(&self) -> Seen {
        let hunters = match &self.scene {
            Scene::Rooftop(r) => {
                if r.climber.on {
                    vec![Glimpse { hunter: r.climber.hunter, side: Some(r.climber.side), dist: r.climber.dist }]
                } else {
                    Vec::new()
                }
            }
            Scene::BusStop(_) => Vec::new(),
            Scene::Campfire(c) => {
                let mut out = Vec::new();
                for s in [&c.verrine, &c.quenelle] {
                    if s.on {
                        out.push(Glimpse { hunter: s.hunter, side: Some(s.side), dist: s.dist });
                    }
                }
                out.push(Glimpse { hunter: Hunter::Ysolde, side: None, dist: c.ysolde.dist });
                out
            }
        };
        let scene = match &self.scene {
            Scene::Rooftop(r) => SceneSeen::Rooftop { lantern: r.lantern, oil: r.oil, relight: r.relight },
            Scene::BusStop(b) => {
                SceneSeen::BusStop { warm: b.warm, lamp: b.lamp, fuel: b.fuel, cans: b.cans.clone(), chill: b.chill }
            }
            Scene::Campfire(c) => SceneSeen::Campfire { fire: c.fire, logs: c.logs, gather: c.gather, hidden: c.hidden },
        };
        Seen {
            watch: self.watch,
            hour: hour_index(self.elapsed),
            torch: self.torch,
            torch_side: self.torch_side,
            weather: self.weather.as_ref().filter(|w| w.on).map(|w| w.kind),
            hunters,
            scene,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_torch_on_the_climbers_side_holds_her_at_the_door() {
        let mut watch = OpenAir::new(Watch::Rooftop, 7, Levels::listed(), true);
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
            Scene::Rooftop(r) => assert_eq!(r.climber.dist, tune().start_dist),
            _ => unreachable!("the rooftop watch has a climber"),
        }
    }

    #[test]
    fn the_json_carries_every_key_the_mode_reads() {
        let rules = Rules::load();
        assert_eq!(rules.ai_steps.len(), 2);
        assert_eq!(rules.names.len(), 4);
        assert_eq!(rules.labels.len(), 3);
    }

    #[test]
    fn the_loaded_values_equal_the_old_constants() {
        let t = tune();
        assert_eq!((t.watch_ms, t.hour_ms, t.bus_ms), (270_000.0, 45_000.0, 18_000.0));
        assert_eq!((t.start_dist, t.at_ms, t.leave_ms, t.dark_bonus), (3, 5_200.0, 450.0, 4));
        assert_eq!((t.max_ai, t.points_per_level, t.max_cans, t.start_logs), (20, 10, 3, 6));
        assert_eq!((t.camp_at_ms, t.tent_pace, t.low_fire, t.low_fire_bonus), (5_200.0, 3.25, 25.0, 3));
        assert_eq!((t.snow.mult, t.snow.every_ms, t.snow.lasts_ms, t.snow.first_ms), (1.8, 34_000.0, 9_000.0, 20_000.0));
        assert_eq!((t.rain.mult, t.rain.every_ms, t.rain.lasts_ms, t.rain.first_ms), (2.2, 40_000.0, 10_000.0, 25_000.0));
        assert_eq!((ai_bump(2), ai_bump(3), ai_bump(4), ai_bump(5)), (0, 1, 2, 2));
        assert_eq!(hour_index(269_999.0), 5);
        assert_eq!(hour_index(1.0e9), 5);
        assert_eq!(Hunter::Quenelle.name(), "Quenelle");
        assert_eq!(Watch::BusStop.label(), "BUS STOP");
        assert_eq!(Watch::Campfire.hunter(), Hunter::Ysolde);
        assert_eq!((t.verrine_side, t.quenelle_side), (Side::Left, Side::Right));
    }

    #[test]
    fn the_pacer_window_slides_with_the_level() {
        let mut stream = NightWatchStream::seeded(1);
        assert!((12_500.0..15_500.0).contains(&wait_ms(1, &mut stream)));
        assert!((3_000.0..6_000.0).contains(&wait_ms(20, &mut stream)));
        assert!(wait_ms(40, &mut stream) >= 2_500.0);
    }

    #[test]
    fn the_listed_levels_come_from_the_json_in_godots_order() {
        let g = data::rules("open_air");
        let listed = Levels::listed();
        // Godot lists the hunters Verrine, Wynelle, Ysolde, Quenelle; the JSON keeps that order (preserve_order).
        let table = g["levels"]["listed"].as_object().expect("levels.listed is an object");
        assert!(!table.is_empty());
        let keys: Vec<&str> = table.keys().map(String::as_str).collect();
        let expected: Vec<&str> = Hunter::ALL.iter().map(|h| h.key()).collect();
        assert_eq!(keys, expected);
        assert!(Hunter::ALL.iter().all(|&h| listed.of(h) > 0));
        assert_eq!(custom_score(listed), 380);
    }
}
