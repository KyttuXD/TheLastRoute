//! Open Air's rules: a port of `openair_logic.gd` (the 2D game's createWatchGame): three outdoor watches, each with its own
//! hunter and its own way of keeping the dark off. Pure state machine: no drawing, no sound, no window.
//!
//! ```text
//!   0 ROOFTOP   Verrine climbs in by the stair door (left) or the hatch (right), a step at a time. A torch held on her side
//!               keeps her still; a barred door or shut hatch sends her back at once. The lantern's oil runs the winches too:
//!               holding either shut burns it faster, relighting costs some, and with none left the lantern will not light and
//!               the barriers swing open. The wind blows the lantern out every so often, and in the dark she is keener.
//!   1 BUS STOP  Wynelle: it is cold. Warmth falls by itself; the heat lamp raises it and burns fuel, and canisters around the
//!               shelter refill it. Below 40 warmth her progress builds, and at the top the cold takes the stop.
//!   2 CAMPFIRE  Verrine always on the left, Quenelle always on the right, a step at a time on the random wait. The torch on a
//!               side holds her still. Held on Verrine at the edge of the light it sends her back; Quenelle does not mind the
//!               light, but if you are in the tent while she is at the edge she loses the trail and backs off. Ysolde works the
//!               tent: while you hide she creeps to the log pile, takes a log and in the end looks in; while you are out she
//!               backs away again, more slowly. The fire burns down: throw logs on it, fetch more.
//! ```
//!
//! Weather: the bus stop gets squalls of snow (the cold bites harder), the campfire showers of rain (the fire burns faster); in
//! the last eighteen seconds of the bus stop's watch the bus comes up the road and the cold lets go (`bus` runs 0 to 1).
//!
//! The draw order of the one seeded stream is kept as in GDScript (every `_r()` call is a `rng.next()` here, in the same
//! place), so a seed is deterministic within the port. The hunters' WALKING is not here: the rules say where a hunter is
//! (a distance on a side); `cast.rs` carries her there over real time.
#![allow(dead_code)]

use super::data::{active_hunters, default_level, tun};
use crate::core::{effective_level, hour_index, Fnv, Mulberry32, Outcome, Pacer, Stream, NIGHT_MS};

// ------------------------------------------------------------------------------------------------ small types

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    L,
    R,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::L => Side::R,
            Side::R => Side::L,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Side::L => "L",
            Side::R => "R",
        }
    }
    pub fn index(self) -> usize {
        match self {
            Side::L => 0,
            Side::R => 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Calm,
    Bold,
    Shy,
}

impl Mood {
    pub fn as_str(self) -> &'static str {
        match self {
            Mood::Calm => "calm",
            Mood::Bold => "bold",
            Mood::Shy => "shy",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    Running,
    Win,
    Caught,
}

/// The rooftop's climber: the side she comes up (stair door L / hatch R), her distance 3..0, her pacer, how long she has waited
/// at the door and been sent off.
#[derive(Clone, Debug)]
pub struct Climber {
    pub side: Side,
    pub dist: i32,
    pub pacer: Pacer,
    pub at_ms: f64,
    pub leave: f64,
}

/// One of the campfire's two at the edge of the light.
#[derive(Clone, Debug)]
pub struct Edge {
    pub id: &'static str,
    pub side: Side,
    pub on: bool,
    pub lvl: i32,
    pub dist: i32,
    pub pacer: Pacer,
    pub at_ms: f64,
    pub push: f64,
    pub lost: f64,
}

impl Edge {
    fn blank(id: &'static str) -> Edge {
        Edge {
            id,
            side: if id == "quenelle" { Side::R } else { Side::L },
            on: false,
            lvl: 0,
            dist: tun().n.start_dist as i32,
            pacer: Pacer::new(),
            at_ms: 0.0,
            push: 0.0,
            lost: 0.0,
        }
    }
}

/// Ysolde at the campfire: dist 2 in the trees, 1 out of them, 0 at the log pile; two pacers so going in and out resets neither.
#[derive(Clone, Debug)]
pub struct Ysolde {
    pub dist: i32,
    pub fwd: Pacer,
    pub back: Pacer,
}

/// {on, t, next, lasts} of the bus stop and the campfire.
#[derive(Clone, Debug, Default)]
pub struct WeatherState {
    pub has: bool,
    pub on: bool,
    pub t: f64,
    pub next: f64,
    pub lasts: f64,
}

/// What is held this step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SimInput {
    pub torch_l: bool,
    pub torch_r: bool,
    pub bar_l: bool,
    pub bar_r: bool,
    pub relight: bool,
    pub gather: bool,
    pub tent: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    Close { id: &'static str, dist: i32, side: Side },
    Repelled { id: &'static str, side: Side, dist: Option<i32> },
    Gust,
    Relit,
    Dry,
    Weather { kind: String },
    WeatherEnd { kind: String },
    Bus,
    Can,
    Creep,
    Stolen,
    Backoff { dist: i32 },
    TentIn,
    TentOut,
    Gathered,
    Caught { by: &'static str, cause: &'static str },
    GodSave { by: &'static str, cause: &'static str },
    Win,
}

impl SimEvent {
    /// A short stable name (the events of a headless run, and what the digest hashes).
    pub fn label(&self) -> String {
        match self {
            SimEvent::Close { id, dist, side } => format!("close:{id}:{}:{dist}", side.as_str()),
            SimEvent::Repelled { id, side, dist } => format!("repelled:{id}:{}:{}", side.as_str(), dist.unwrap_or(-1)),
            SimEvent::Gust => "gust".to_string(),
            SimEvent::Relit => "relit".to_string(),
            SimEvent::Dry => "dry".to_string(),
            SimEvent::Weather { kind } => format!("weather:{kind}"),
            SimEvent::WeatherEnd { kind } => format!("weather_end:{kind}"),
            SimEvent::Bus => "bus".to_string(),
            SimEvent::Can => "can".to_string(),
            SimEvent::Creep => "creep".to_string(),
            SimEvent::Stolen => "stolen".to_string(),
            SimEvent::Backoff { dist } => format!("backoff:{dist}"),
            SimEvent::TentIn => "tent_in".to_string(),
            SimEvent::TentOut => "tent_out".to_string(),
            SimEvent::Gathered => "gathered".to_string(),
            SimEvent::Caught { by, cause } => format!("caught:{by}:{cause}"),
            SimEvent::GodSave { by, cause } => format!("godsave:{by}:{cause}"),
            SimEvent::Win => "win".to_string(),
        }
    }
}

/// The night's AI step-ups (a hunter at 0 stays at 0), at most 20.
pub fn keen(lvl: i32, elapsed_ms: f64) -> i32 {
    effective_level(lvl, hour_index(elapsed_ms)).min(20)
}

// ------------------------------------------------------------------------------------------------ the sim

#[derive(Clone, Debug)]
pub struct OpenAirSim {
    pub watch: usize,
    pub elapsed: f64,
    pub over: Over,
    pub by: &'static str,
    pub cause: &'static str,
    pub torch: f64,
    pub torch_side: Option<Side>,
    pub bus: f64,
    pub infinite_light: bool,
    /// Admin: god mode sends whoever would catch you back instead; speed scales how often the hunters step.
    pub god: bool,
    pub speed: f64,
    pub weather: WeatherState,

    // rooftop
    pub lantern: bool,
    pub gust: f64,
    pub relight: f64,
    pub oil: f64,
    pub bars: [bool; 2],
    pub climber: Climber,
    // bus stop
    pub warm: f64,
    pub lamp: bool,
    pub fuel: f64,
    pub cans: Vec<usize>,
    pub can_clock: f64,
    pub can_at: f64,
    pub chill: f64,
    pub cold_mult: f64,
    pub cold_left: f64,
    // campfire
    pub fire: f64,
    pub logs: i32,
    pub gather: f64,
    pub out: f64,
    pub hidden: bool,
    pub tent_ms: f64,
    pub y: Ysolde,
    pub mood: Mood,
    pub mood_left: f64,
    /// Verrine (left) and Quenelle (right) at the campfire.
    pub v: Edge,
    pub q: Edge,

    base: i32,
    verrine_at: i32,
    quenelle_at: i32,
    rng: Stream,
}

impl OpenAirSim {
    /// `seed` feeds the watch's one stream. `level_override` (>= 0) replaces the watch's hunter level; `verrine_level` and
    /// `quenelle_level` (>= 0) set the campfire's two (default: their listed levels).
    pub fn new(watch: usize, seed: u32, level_override: i32, verrine_level: i32, quenelle_level: i32, infinite: bool) -> OpenAirSim {
        OpenAirSim::with_rng(watch, Stream::seeded(seed), level_override, verrine_level, quenelle_level, infinite)
    }

    /// Like `new` with the stream given (tests pass `Stream::fixed(0.5)`).
    pub fn with_rng(watch: usize, rng: Stream, level_override: i32, verrine_level: i32, quenelle_level: i32, infinite: bool) -> OpenAirSim {
        let watch = watch.min(2);
        let t = tun();
        let n = &t.n;
        let hunter = super::data::watch_hunter(watch);
        let base = if level_override >= 0 { level_override } else { default_level(hunter) };
        let mut s = OpenAirSim {
            watch,
            elapsed: 0.0,
            over: Over::Running,
            by: "",
            cause: "",
            torch: 100.0,
            torch_side: None,
            bus: 0.0,
            infinite_light: infinite,
            god: false,
            speed: 1.0,
            weather: WeatherState::default(),
            lantern: true,
            gust: 0.0,
            relight: 0.0,
            oil: 100.0,
            bars: [false, false],
            climber: Climber { side: Side::L, dist: n.start_dist as i32, pacer: Pacer::new(), at_ms: 0.0, leave: 0.0 },
            warm: 100.0,
            lamp: false,
            fuel: 100.0,
            cans: Vec::new(),
            can_clock: 0.0,
            can_at: 0.0,
            chill: 0.0,
            cold_mult: 1.0,
            cold_left: 0.0,
            fire: 100.0,
            logs: 0,
            gather: 0.0,
            out: 0.0,
            hidden: false,
            tent_ms: 0.0,
            y: Ysolde { dist: 2, fwd: Pacer::new(), back: Pacer::new() },
            mood: Mood::Calm,
            mood_left: 0.0,
            v: Edge::blank("verrine"),
            q: Edge::blank("quenelle"),
            base,
            verrine_at: if verrine_level >= 0 { verrine_level } else { default_level("verrine") },
            quenelle_at: if quenelle_level >= 0 { quenelle_level } else { default_level("quenelle") },
            rng,
        };
        if let Some(wx) = &t.weather[watch] {
            let next = wx.first * (0.75 + s.r() * 0.5);
            let lasts = s.jitter(wx.lasts_ms, n.weather_last_spread);
            s.weather = WeatherState { has: true, on: false, t: 0.0, next, lasts };
        }
        match watch {
            0 => {
                s.gust = s.jitter(n.gust_ms, n.gust_first_spread);
                s.climber = s.new_climber();
            }
            1 => {
                s.can_at = s.jitter(n.can_first_ms, n.can_first_spread);
                s.cold_mult = s.jitter(1.0, n.cold_spread);
                s.cold_left = s.jitter(n.cold_every_ms, n.cold_every_spread);
            }
            _ => {
                s.logs = n.start_logs as i32;
                s.mood = s.draw_mood();
                s.mood_left = s.jitter(n.mood_every_ms, n.mood_every_spread);
                s.v = s.new_edge("verrine", s.verrine_at);
                s.q = s.new_edge("quenelle", s.quenelle_at);
            }
        }
        s
    }

    fn r(&mut self) -> f64 {
        self.rng.next()
    }

    fn jitter(&mut self, base: f64, spread: f64) -> f64 {
        let k = spread.clamp(0.0, 0.9);
        base * (1.0 - k + 2.0 * k * self.r())
    }

    /// The watch's own hunter's level.
    pub fn level(&self) -> i32 {
        self.base
    }

    /// The watch's own hunter is at `base`; at the campfire Verrine (left) and Quenelle (right) are at their own levels.
    pub fn level_of(&self, id: &str) -> i32 {
        if self.watch == 2 && id == "verrine" {
            return self.verrine_at;
        }
        if self.watch == 2 && id == "quenelle" {
            return self.quenelle_at;
        }
        self.base
    }

    pub fn keen_of(&self, lvl: i32) -> i32 {
        keen(lvl, self.elapsed)
    }

    pub fn hunter_ids(&self) -> Vec<&'static str> {
        active_hunters(self.watch)
    }

    /// Is this hunter on the floor (level above 0)?
    pub fn hunter_on(&self, id: &str) -> bool {
        match id {
            "verrine" => {
                if self.watch == 0 {
                    self.base > 0
                } else {
                    self.v.on
                }
            }
            "quenelle" => self.q.on,
            "ysolde" | "wynelle" => self.base > 0,
            _ => false,
        }
    }

    pub fn outcome(&self) -> Outcome {
        match self.over {
            Over::Running => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn draw_mood(&mut self) -> Mood {
        let calm = tun().n.mood_calm;
        let u = self.r();
        if u < calm {
            return Mood::Calm;
        }
        if (u - calm) / (1.0 - calm) < 0.5 {
            Mood::Bold
        } else {
            Mood::Shy
        }
    }

    /// A climber's or an edge hunter's first wait starts up to 4 s in credit or in debt, drawn whether or not she is on.
    fn arrival_offset(&mut self) -> f64 {
        (self.r() * 2.0 - 1.0) * tun().n.arrive_spread_ms
    }

    fn new_climber(&mut self) -> Climber {
        let side = if self.r() < 0.5 { Side::L } else { Side::R };
        let mut pacer = Pacer::new();
        let off = self.arrival_offset();
        pacer.skip(off);
        Climber { side, dist: tun().n.start_dist as i32, pacer, at_ms: 0.0, leave: 0.0 }
    }

    fn new_edge(&mut self, id: &'static str, lvl: i32) -> Edge {
        let mut pacer = Pacer::new();
        let off = self.arrival_offset();
        pacer.skip(off);
        Edge {
            id,
            side: if id == "quenelle" { Side::R } else { Side::L },
            on: lvl > 0,
            lvl,
            dist: tun().n.start_dist as i32,
            pacer,
            at_ms: 0.0,
            push: 0.0,
            lost: 0.0,
        }
    }

    fn finish(&mut self, mut ev: Vec<SimEvent>, who: &'static str, why: &'static str) -> Vec<SimEvent> {
        if self.god {
            self.god_save(who, why);
            ev.push(SimEvent::GodSave { by: who, cause: why });
            return ev;
        }
        self.over = Over::Caught;
        self.by = who;
        self.cause = why;
        ev.push(SimEvent::Caught { by: who, cause: why });
        ev
    }

    /// God mode: the catch is undone (she is sent back, the cold, the dark and the tent let go) and the watch goes on.
    fn god_save(&mut self, who: &str, why: &str) {
        match why {
            "cold" => self.chill = 40.0,
            "fire" => {
                self.out = 0.0;
                self.fire = self.fire.max(30.0);
            }
            "tent" => {
                self.y.dist = 2;
                self.y.fwd.reset();
            }
            _ => {
                if self.watch == 0 {
                    self.climber.dist = 2;
                    self.climber.at_ms = 0.0;
                    self.climber.leave = 0.0;
                    self.climber.pacer.reset();
                } else {
                    let h = if who == "verrine" { &mut self.v } else { &mut self.q };
                    if h.id == who {
                        let mut sink = Vec::new();
                        OpenAirSim::send_back(h, 2, &mut sink);
                    }
                }
            }
        }
    }

    /// A hunter who comes a step at a time toward a spot, held still by the torch on her side. True once she has waited out
    /// the time.
    #[allow(clippy::too_many_arguments)]
    fn approach(
        h: &mut Climber,
        rng: &mut Stream,
        torch_side: Option<Side>,
        speed: f64,
        elapsed: f64,
        dt_ms: f64,
        lvl: i32,
        barred: bool,
        ev: &mut Vec<SimEvent>,
        id: &'static str,
    ) -> bool {
        let n = &tun().n;
        if h.dist > 0 {
            if torch_side != Some(h.side) && h.pacer.step(dt_ms * speed, keen(lvl, elapsed), &mut || rng.next()) {
                h.dist -= 1;
                ev.push(SimEvent::Close { id, dist: h.dist, side: h.side });
            }
            return false;
        }
        if barred {
            h.at_ms = 0.0;
            h.leave += dt_ms;
            if h.leave >= n.leave_ms {
                h.dist = 2;
                h.leave = 0.0;
                h.pacer.reset();
                let switch = rng.next() < n.side_switch;
                if switch {
                    h.side = h.side.other();
                }
                ev.push(SimEvent::Repelled { id, side: h.side, dist: None });
            }
            return false;
        }
        h.leave = 0.0;
        if torch_side != Some(h.side) {
            h.at_ms += dt_ms;
        }
        h.at_ms >= n.at_ms
    }

    fn send_back(h: &mut Edge, to: i32, ev: &mut Vec<SimEvent>) {
        h.dist = to;
        h.at_ms = 0.0;
        h.push = 0.0;
        h.lost = 0.0;
        h.pacer.reset();
        ev.push(SimEvent::Repelled { id: h.id, side: h.side, dist: Some(to) });
    }

    /// The campfire's two: a step at a time toward the edge of the light on her own side. At the edge she waits out
    /// `camp_at_ms` and then she is at the fire. Verrine: the torch held on her there for `push_ms` sends her back.
    /// Quenelle: the tent while she is there pauses her, and after `lost_ms` she loses the trail and backs off a little.
    #[allow(clippy::too_many_arguments)]
    fn edge_step(
        h: &mut Edge,
        rng: &mut Stream,
        hidden: bool,
        torch_side: Option<Side>,
        speed: f64,
        elapsed: f64,
        fire: f64,
        mood: Mood,
        dt_ms: f64,
        ev: &mut Vec<SimEvent>,
    ) -> bool {
        let n = &tun().n;
        if !h.on {
            return false;
        }
        let held = torch_side == Some(h.side);
        let lvl = keen(h.lvl, elapsed) + if fire < n.low_fire { n.low_fire_bonus as i32 } else { 0 };
        if h.dist > 0 {
            let mut pace = 1.0;
            if h.id == "verrine" && mood != Mood::Calm {
                pace = if mood == Mood::Bold { n.mood_pace } else { 1.0 / n.mood_pace };
            }
            if !held && h.pacer.step(dt_ms * pace * speed, lvl, &mut || rng.next()) {
                h.dist -= 1;
                ev.push(SimEvent::Close { id: h.id, dist: h.dist, side: h.side });
            }
            return false;
        }
        if h.id == "verrine" && held {
            h.push += dt_ms;
            if h.push >= n.push_ms {
                OpenAirSim::send_back(h, n.push_back as i32, ev);
            }
            return false;
        }
        if h.id == "quenelle" && hidden {
            h.lost += dt_ms;
            if h.lost >= n.lost_ms {
                OpenAirSim::send_back(h, n.lost_back as i32, ev);
            }
            return false;
        }
        if !held {
            h.at_ms += dt_ms;
        }
        h.at_ms >= n.camp_at_ms
    }

    /// Who the dark lets in when the fire has been out too long: Ysolde, unless she is off; then the nearer of the two who are on.
    fn dark_by(&self) -> &'static str {
        if self.base > 0 {
            return "ysolde";
        }
        let mut best: Option<&Edge> = None;
        for h in [&self.q, &self.v] {
            if h.on && best.map_or(true, |b| h.dist < b.dist) {
                best = Some(h);
            }
        }
        best.map(|h| h.id).unwrap_or("ysolde")
    }

    /// Advance by `dt_ms` with what is held; returns what happened.
    pub fn step(&mut self, dt_ms: f64, input: &SimInput) -> Vec<SimEvent> {
        if self.over != Over::Running {
            return Vec::new();
        }
        let t = tun();
        let n = &t.n;
        let mut ev: Vec<SimEvent> = Vec::new();
        let dt = dt_ms / 1000.0;
        self.elapsed += dt_ms;

        // Weather comes and goes on its own clock; the bus stop's last seconds are the bus.
        let mut wx_mult = 1.0;
        if let Some(wx) = &t.weather[self.watch] {
            if self.watch == 1 && self.elapsed >= NIGHT_MS - n.bus_ms {
                if self.weather.on {
                    self.weather.on = false;
                    ev.push(SimEvent::WeatherEnd { kind: wx.kind.clone() });
                }
                if self.bus == 0.0 {
                    ev.push(SimEvent::Bus);
                }
                self.bus = ((self.elapsed - (NIGHT_MS - n.bus_ms)) / n.bus_ms).min(1.0);
            } else {
                self.weather.t += dt_ms;
                if !self.weather.on && self.weather.t >= self.weather.next {
                    self.weather.on = true;
                    self.weather.t = 0.0;
                    ev.push(SimEvent::Weather { kind: wx.kind.clone() });
                } else if self.weather.on && self.weather.t >= self.weather.lasts {
                    self.weather.on = false;
                    self.weather.t = 0.0;
                    self.weather.next = wx.every_ms * (0.75 + self.r() * 0.5);
                    self.weather.lasts = self.jitter(wx.lasts_ms, n.weather_last_spread);
                    ev.push(SimEvent::WeatherEnd { kind: wx.kind.clone() });
                }
            }
            if self.weather.on {
                wx_mult = wx.mult;
            }
        }

        // The torch: held on a side, it drains; let go and it recovers. Hands busy gathering, or hiding in the tent, means no torch.
        if self.watch == 2 {
            let hide = input.tent;
            if hide != self.hidden {
                self.hidden = hide;
                self.tent_ms = 0.0;
                self.gather = 0.0;
                ev.push(if hide { SimEvent::TentIn } else { SimEvent::TentOut });
            }
            if self.hidden {
                self.tent_ms += dt_ms;
            }
        }
        let mut want: Option<Side> = None;
        if !((input.gather || self.hidden) && self.watch == 2) {
            if input.torch_l {
                want = Some(Side::L);
            } else if input.torch_r {
                want = Some(Side::R);
            }
        }
        self.torch_side = if want.is_some() && self.torch > 0.0 { want } else { None };
        if self.torch_side.is_some() && !self.infinite_light {
            self.torch = (self.torch - n.torch_drain * dt).max(0.0);
        } else {
            self.torch = (self.torch + n.torch_regen * dt).min(100.0);
        }

        if self.watch == 0 {
            let held = [input.bar_l && self.oil > 0.0, input.bar_r && self.oil > 0.0];
            self.bars = held;
            let strain = held[0] as i32 + held[1] as i32;
            if !self.infinite_light {
                self.oil = (self.oil - (n.oil_base + n.oil_per_bar * strain as f64) * dt).max(0.0);
            }
            if self.lantern && self.oil <= 0.0 {
                self.lantern = false;
                self.relight = 0.0;
                ev.push(SimEvent::Dry);
            }
            self.gust -= dt_ms;
            if self.lantern && self.gust <= 0.0 {
                self.lantern = false;
                self.relight = 0.0;
                ev.push(SimEvent::Gust);
            }
            if !self.lantern {
                self.relight = if input.relight && self.oil >= n.relight_oil { self.relight + dt_ms } else { (self.relight - dt_ms).max(0.0) };
                if self.relight >= n.relight_ms {
                    self.lantern = true;
                    self.relight = 0.0;
                    self.oil -= n.relight_oil;
                    let extra = self.r();
                    self.gust = n.gust_ms + extra * n.gust_jitter_ms;
                    ev.push(SimEvent::Relit);
                }
            }
            let barred = held[self.climber.side.index()];
            let lvl = self.base + if self.lantern { 0 } else { n.dark_bonus as i32 };
            let arrived = OpenAirSim::approach(
                &mut self.climber,
                &mut self.rng,
                self.torch_side,
                self.speed,
                self.elapsed,
                dt_ms,
                lvl,
                barred,
                &mut ev,
                "verrine",
            );
            if arrived {
                return self.finish(ev, "verrine", "edge");
            }
        }

        if self.watch == 1 {
            let lamp_on = self.lamp && self.fuel > 0.0;
            self.cold_left -= dt_ms;
            if self.cold_left <= 0.0 {
                self.cold_mult = self.jitter(1.0, n.cold_spread);
                self.cold_left = self.jitter(n.cold_every_ms, n.cold_every_spread);
            }
            let heat = if lamp_on { n.lamp_warm } else { 0.0 };
            self.warm = (self.warm + (heat - n.cold_drain * wx_mult * self.cold_mult) * dt).min(100.0).max(0.0);
            if lamp_on {
                self.fuel = (self.fuel - n.lamp_fuel * dt).max(0.0);
                if self.fuel <= 0.0 {
                    self.lamp = false;
                    ev.push(SimEvent::Dry);
                }
            }
            self.can_clock += dt_ms;
            if self.can_clock >= self.can_at && (self.cans.len() as f64) < n.max_cans {
                self.can_clock = 0.0;
                self.can_at = self.jitter(n.can_every_ms, n.can_spread);
                let free: Vec<usize> = (0..3usize).filter(|i| !self.cans.contains(i)).collect();
                if !free.is_empty() {
                    let pick = ((self.r() * free.len() as f64).floor() as usize).min(free.len() - 1);
                    self.cans.push(free[pick]);
                    ev.push(SimEvent::Can);
                }
            }
            let factor = (self.base as f64 / 10.0).max(0.4);
            if self.bus > 0.0 {
                self.chill = (self.chill - n.chill_recover * 2.0 * dt).max(0.0);
            } else if self.warm < n.chill_line {
                self.chill = (self.chill + (n.chill_line - self.warm) * n.chill_per_point * factor * dt).min(100.0);
            } else {
                self.chill = (self.chill - n.chill_recover * dt).max(0.0);
            }
            if self.chill >= 100.0 {
                return self.finish(ev, "wynelle", "cold");
            }
        }

        if self.watch == 2 {
            self.mood_left -= dt_ms;
            if self.mood_left <= 0.0 {
                self.mood = self.draw_mood();
                self.mood_left = self.jitter(n.mood_every_ms, n.mood_every_spread);
            }
            self.fire = (self.fire - n.camp_drain * wx_mult * dt).max(0.0);
            if input.gather && !self.hidden {
                self.gather += dt_ms;
                if self.gather >= n.gather_ms {
                    self.gather = 0.0;
                    self.logs += 1;
                    ev.push(SimEvent::Gathered);
                }
            } else {
                self.gather = 0.0;
            }
            if self.fire <= 0.0 {
                self.out += dt_ms;
                if self.out >= n.out_grace_ms {
                    let who = self.dark_by();
                    return self.finish(ev, who, "fire");
                }
            } else {
                self.out = 0.0;
            }
            // Ysolde works the tent: while you hide she creeps out (2 -> 1), takes a log from the pile (1 -> 0) and then looks in (0 -> caught).
            let ys_level = keen(self.base, self.elapsed);
            if self.hidden {
                let due = {
                    let rng = &mut self.rng;
                    self.y.fwd.step(dt_ms * n.tent_pace * self.speed, ys_level, &mut || rng.next())
                };
                if due {
                    self.y.dist -= 1;
                    if self.y.dist == 1 {
                        ev.push(SimEvent::Creep);
                    } else if self.y.dist == 0 {
                        if self.logs > 0 {
                            self.logs -= 1;
                        } else {
                            self.fire = (self.fire - n.fuel_steal).max(0.0);
                        }
                        ev.push(SimEvent::Stolen);
                    } else {
                        return self.finish(ev, "ysolde", "tent");
                    }
                }
            } else if self.y.dist < 2 {
                let due = {
                    let rng = &mut self.rng;
                    self.y.back.step(dt_ms * n.back_pace * self.speed, ys_level, &mut || rng.next())
                };
                if due {
                    self.y.dist += 1;
                    ev.push(SimEvent::Backoff { dist: self.y.dist });
                }
            }
            let (hidden, torch_side, speed, elapsed, fire, mood) = (self.hidden, self.torch_side, self.speed, self.elapsed, self.fire, self.mood);
            let v_arrived = OpenAirSim::edge_step(&mut self.v, &mut self.rng, hidden, torch_side, speed, elapsed, fire, mood, dt_ms, &mut ev);
            if v_arrived {
                return self.finish(ev, "verrine", "edge");
            }
            let q_arrived = OpenAirSim::edge_step(&mut self.q, &mut self.rng, hidden, torch_side, speed, elapsed, fire, mood, dt_ms, &mut ev);
            if q_arrived {
                return self.finish(ev, "quenelle", "edge");
            }
        }

        if self.elapsed >= NIGHT_MS {
            self.over = Over::Win;
            ev.push(SimEvent::Win);
        }
        ev
    }

    // -------------------------------------------------------------------------------------------- player actions

    /// The heat lamp: on or off. True when it did something.
    pub fn act_lamp(&mut self) -> bool {
        if self.over != Over::Running {
            return false;
        }
        if self.watch == 1 && (self.fuel > 0.0 || self.lamp) {
            self.lamp = !self.lamp;
            return true;
        }
        false
    }

    /// Pick up canister `i` of the bus stop.
    pub fn act_can(&mut self, i: usize) -> bool {
        if self.over != Over::Running {
            return false;
        }
        if self.watch == 1 {
            if let Some(at) = self.cans.iter().position(|c| *c == i) {
                self.cans.remove(at);
                self.fuel = (self.fuel + tun().n.can_fuel).min(100.0);
                return true;
            }
        }
        false
    }

    /// A log on the fire.
    pub fn act_log(&mut self) -> bool {
        if self.over != Over::Running {
            return false;
        }
        if self.watch == 2 && self.logs > 0 && !self.hidden {
            self.logs -= 1;
            self.fire = (self.fire + tun().n.log_fuel).min(100.0);
            return true;
        }
        false
    }

    // -------------------------------------------------------------------------------------------- admin

    pub fn admin_set_level(&mut self, id: &str, n: i32) {
        let n = n.clamp(0, 20);
        if self.watch == 2 && id == "verrine" {
            self.verrine_at = n;
            self.v.lvl = n;
            self.v.on = n > 0;
        } else if self.watch == 2 && id == "quenelle" {
            self.quenelle_at = n;
            self.q.lvl = n;
            self.q.on = n > 0;
        } else if id == super::data::watch_hunter(self.watch) || (self.watch == 2 && id == "ysolde") {
            self.base = n;
        }
    }

    /// Put a hunter at a step of her approach: "far" (3 steps out), "step 2", "step 1", "at the edge" (0). Ysolde: the trees (2),
    /// out of them (1), the log pile (0). Wynelle's place is how far the cold has got (her chill).
    pub fn admin_place(&mut self, id: &str, place: &str) {
        let dist: i32 = match place {
            "step 2" => 2,
            "step 1" => 1,
            "at the edge" => 0,
            _ => 3,
        };
        if id == "wynelle" && self.watch == 1 {
            self.chill = [95.0, 70.0, 40.0, 5.0][dist as usize];
        } else if id == "ysolde" && self.watch == 2 {
            self.y.dist = if dist < 3 { dist.clamp(0, 2) } else { 2 };
        } else if self.watch == 0 && id == "verrine" {
            self.climber.dist = dist;
            self.climber.at_ms = 0.0;
            self.climber.leave = 0.0;
        } else if self.watch == 2 && (id == "verrine" || id == "quenelle") {
            let h = if id == "verrine" { &mut self.v } else { &mut self.q };
            h.dist = dist;
            h.at_ms = 0.0;
            h.push = 0.0;
            h.lost = 0.0;
        }
    }

    /// The rooftop's door she is at.
    pub fn admin_set_side(&mut self, side: Side) {
        if self.watch == 0 {
            self.climber.side = side;
            self.climber.at_ms = 0.0;
            self.climber.leave = 0.0;
        }
    }

    /// Set one of the watch's meters or switches: oil, lantern (0/1), warm, fuel, chill, fire, logs, mood (0 calm, 1 bold, 2 shy),
    /// rain (force a squall / shower now: 1, end it: 0), can (a canister now), bus (jump to the bus's start). True when the key
    /// applies to this watch.
    pub fn admin_set(&mut self, key: &str, value: f64) -> bool {
        let n = &tun().n;
        match key {
            "oil" if self.watch == 0 => {
                self.oil = value.clamp(0.0, 100.0);
                true
            }
            "lantern" if self.watch == 0 => {
                self.lantern = value != 0.0;
                self.relight = 0.0;
                if self.lantern {
                    self.gust = n.gust_ms;
                }
                true
            }
            "warm" if self.watch == 1 => {
                self.warm = value.clamp(0.0, 100.0);
                true
            }
            "fuel" if self.watch == 1 => {
                self.fuel = value.clamp(0.0, 100.0);
                true
            }
            "chill" if self.watch == 1 => {
                self.chill = value.clamp(0.0, 99.0);
                true
            }
            "fire" if self.watch == 2 => {
                self.fire = value.clamp(0.0, 100.0);
                self.out = 0.0;
                true
            }
            "logs" if self.watch == 2 => {
                self.logs = (value as i32).max(0);
                true
            }
            "mood" if self.watch == 2 => {
                self.mood = match value as i32 {
                    1 => Mood::Bold,
                    2 => Mood::Shy,
                    _ => Mood::Calm,
                };
                self.mood_left = n.mood_every_ms;
                true
            }
            "rain" if self.weather.has => {
                self.weather.on = value != 0.0;
                self.weather.t = 0.0;
                true
            }
            "can" if self.watch == 1 => {
                if (self.cans.len() as f64) < n.max_cans {
                    if let Some(i) = (0..3usize).find(|i| !self.cans.contains(i)) {
                        self.cans.push(i);
                        return true;
                    }
                }
                false
            }
            "bus" if self.watch == 1 => {
                self.elapsed = self.elapsed.max(NIGHT_MS - n.bus_ms);
                true
            }
            _ => false,
        }
    }

    // -------------------------------------------------------------------------------------------- digest

    /// FNV-1a of the state that decides a watch (stable across runs).
    pub fn digest(&self) -> u64 {
        let mut h = Fnv::new();
        h.f64(self.elapsed).i32(self.over as i32).str(self.by).str(self.cause).i32(self.watch as i32);
        h.i32(self.torch.round() as i32).i32(self.torch_side.map(|s| s.index() as i32 + 1).unwrap_or(0));
        match self.watch {
            0 => {
                h.i32(self.oil.round() as i32).i32(self.lantern as i32).i32(self.climber.dist).i32(self.climber.side.index() as i32);
                h.i32(self.climber.at_ms.round() as i32);
            }
            1 => {
                h.i32(self.warm.round() as i32).i32(self.fuel.round() as i32).i32(self.chill.round() as i32).i32(self.cans.len() as i32);
                h.i32((self.bus * 1000.0).round() as i32);
            }
            _ => {
                h.i32(self.fire.round() as i32).i32(self.logs).i32(self.y.dist).i32(self.v.dist).i32(self.q.dist).i32(self.hidden as i32);
                h.str(self.mood.as_str());
            }
        }
        h.i32(self.weather.on as i32);
        h.finish()
    }
}

// ------------------------------------------------------------------------------------------------ autoplay

/// What a scripted player does besides holding things.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Lamp,
    Can(usize),
    Log,
}

/// A seeded player for the headless run and the tests: reacts to a threat after a random time (and now and then not at all),
/// keeps the lamp, the fire and the lantern going. It reads only the sim, so the same seed always plays the same watch.
#[derive(Clone, Debug)]
pub struct Autoplay {
    rng: Mulberry32,
    threat: i32,
    threat_ms: f64,
    react_ms: f64,
}

impl Autoplay {
    pub fn new(seed: u32) -> Autoplay {
        Autoplay { rng: Mulberry32::new(seed ^ 0x4f50_454e), threat: 0, threat_ms: 0.0, react_ms: 0.0 }
    }

    /// An id for the thing that must be answered right now (0 none).
    fn threat_of(sim: &OpenAirSim) -> i32 {
        match sim.watch {
            0 => {
                if sim.climber.dist == 0 {
                    1 + sim.climber.side.index() as i32
                } else {
                    0
                }
            }
            1 => 0,
            _ => {
                if sim.v.on && sim.v.dist == 0 {
                    10
                } else if sim.q.on && sim.q.dist == 0 {
                    11
                } else {
                    0
                }
            }
        }
    }

    /// The input and the one-off actions for the next `dt_ms` of the watch.
    pub fn plan(&mut self, sim: &OpenAirSim, dt_ms: f64) -> (SimInput, Vec<Act>) {
        let mut input = SimInput::default();
        let mut acts: Vec<Act> = Vec::new();
        let threat = Autoplay::threat_of(sim);
        if threat != self.threat {
            self.threat = threat;
            self.threat_ms = 0.0;
            // one answer in twenty-five never comes
            self.react_ms = if self.rng.next() < 0.04 { 1.0e9 } else { 300.0 + self.rng.next() * 2400.0 };
        } else {
            self.threat_ms += dt_ms;
        }
        let ready = threat != 0 && self.threat_ms >= self.react_ms;
        match sim.watch {
            0 => {
                if !sim.lantern && sim.oil >= tun().n.relight_oil {
                    input.relight = true;
                }
                let side = sim.climber.side;
                if ready {
                    if side == Side::L {
                        input.bar_l = true;
                    } else {
                        input.bar_r = true;
                    }
                } else if sim.climber.dist == 1 && sim.torch > 15.0 {
                    if side == Side::L {
                        input.torch_l = true;
                    } else {
                        input.torch_r = true;
                    }
                }
            }
            1 => {
                let want_on = sim.warm < 60.0 || sim.chill > 20.0;
                if want_on != sim.lamp && (sim.fuel > 0.0 || sim.lamp) {
                    acts.push(Act::Lamp);
                }
                if let Some(i) = sim.cans.first() {
                    acts.push(Act::Can(*i));
                }
            }
            _ => {
                if sim.fire < 45.0 && sim.logs > 0 && !sim.hidden {
                    acts.push(Act::Log);
                }
                let quiet = sim.v.dist >= 1 && sim.q.dist >= 1;
                if sim.logs < 3 && !sim.hidden && quiet {
                    input.gather = true;
                }
                if ready {
                    if threat == 10 {
                        input.torch_l = true;
                    } else if threat == 11 {
                        if sim.y.dist == 2 {
                            input.tent = true;
                        } else {
                            input.torch_r = true;
                        }
                    }
                }
            }
        }
        (input, acts)
    }

    /// Apply the one-off actions to the sim.
    pub fn apply(sim: &mut OpenAirSim, acts: &[Act]) {
        for a in acts {
            match a {
                Act::Lamp => {
                    sim.act_lamp();
                }
                Act::Can(i) => {
                    sim.act_can(*i);
                }
                Act::Log => {
                    sim.act_log();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(watch: usize, level: i32) -> OpenAirSim {
        OpenAirSim::with_rng(watch, Stream::fixed(0.5), level, -1, -1, false)
    }

    fn run(s: &mut OpenAirSim, input: &SimInput, ms: f64) -> Vec<SimEvent> {
        let mut all = Vec::new();
        let mut t = 0.0;
        while t < ms && s.over == Over::Running {
            all.extend(s.step(100.0, input));
            t += 100.0;
        }
        all
    }

    #[test]
    fn the_climber_closes_in_three_steps_and_catches_after_the_wait() {
        let mut s = fixed(0, -1);
        s.gust = 1.0e12;
        assert_eq!((s.climber.dist, s.climber.side), (3, Side::R), "a draw of exactly 0.5 is not under 0.5, so the hatch");
        let ev = run(&mut s, &SimInput::default(), 60_000.0);
        let dists: Vec<i32> = ev.iter().filter_map(|e| if let SimEvent::Close { dist, .. } = e { Some(*dist) } else { None }).collect();
        assert_eq!(dists, vec![2, 1, 0]);
        assert_eq!(s.over, Over::Caught);
        assert_eq!((s.by, s.cause), ("verrine", "edge"));
        assert!(s.elapsed >= 35_000.0 && s.elapsed <= 35_500.0, "moved at 10, 20 and 30 s, then 5.2 s at the door: {}", s.elapsed);
    }

    #[test]
    fn a_barred_door_sends_her_back_after_450_ms() {
        let mut s = fixed(0, -1);
        s.gust = 1.0e12;
        s.climber.dist = 0;
        s.climber.side = Side::R;
        let input = SimInput { bar_r: true, ..SimInput::default() };
        let mut found = None;
        for i in 0..10 {
            for e in s.step(100.0, &input) {
                if let SimEvent::Repelled { side, .. } = e {
                    found = Some((i, side));
                }
            }
            if found.is_some() {
                break;
            }
        }
        let (i, side) = found.expect("repelled");
        assert_eq!(i, 4, "five steps of 100 ms reach 450");
        assert_eq!(side, Side::L, "0.5 is under the 0.65 side switch");
        assert_eq!((s.climber.dist, s.climber.leave), (2, 0.0));
        assert!(s.oil < 100.0, "holding a bar burns oil");
    }

    #[test]
    fn the_torch_on_her_side_holds_her_still() {
        let mut held = fixed(0, -1);
        held.gust = 1.0e12;
        let side_r = SimInput { torch_r: true, ..SimInput::default() };
        let ev = run(&mut held, &side_r, 9_000.0);
        assert!(ev.iter().all(|e| !matches!(e, SimEvent::Close { .. })));
        assert_eq!(held.climber.dist, 3);
        assert!(held.torch < 100.0, "the torch drains while held");
    }

    #[test]
    fn the_wind_puts_the_lantern_out_and_holding_relight_lights_it_again() {
        let mut s = fixed(0, 0);
        s.gust = 500.0;
        let ev = run(&mut s, &SimInput::default(), 600.0);
        assert!(ev.contains(&SimEvent::Gust));
        assert!(!s.lantern);
        let relight = SimInput { relight: true, ..SimInput::default() };
        let ev = run(&mut s, &relight, 1_700.0);
        assert!(ev.contains(&SimEvent::Relit));
        assert!(s.lantern);
        assert!(s.oil < 100.0 - 0.1, "relighting costs oil");
        assert!(s.gust >= 22_000.0, "the next gust is at least 22 s away");
    }

    #[test]
    fn with_no_oil_the_lantern_dies_and_the_bars_open() {
        let mut s = fixed(0, 0);
        s.gust = 1.0e12;
        s.oil = 0.05;
        let both = SimInput { bar_l: true, bar_r: true, ..SimInput::default() };
        let ev = run(&mut s, &both, 300.0);
        assert!(ev.contains(&SimEvent::Dry));
        assert!(!s.lantern);
        s.step(100.0, &both);
        assert_eq!(s.bars, [false, false], "no oil, no winches");
    }

    #[test]
    fn the_cold_takes_the_stop_when_the_chill_tops_out() {
        let mut s = fixed(1, -1);
        s.warm = 0.0;
        s.chill = 99.9;
        let ev = s.step(100.0, &SimInput::default());
        assert!(ev.contains(&SimEvent::Caught { by: "wynelle", cause: "cold" }));
        assert_eq!(s.outcome(), Outcome::Lost);
        assert!(s.step(100.0, &SimInput::default()).is_empty(), "nothing happens after the catch");
    }

    #[test]
    fn the_heat_lamp_warms_and_burns_fuel() {
        let mut s = fixed(1, 0);
        s.warm = 50.0;
        assert!(s.act_lamp());
        run(&mut s, &SimInput::default(), 5_000.0);
        assert!(s.warm > 50.0, "2.8 up against 1.0 down");
        assert!(s.fuel < 100.0 && s.fuel > 90.0);
        s.fuel = 0.0;
        assert!(s.act_lamp(), "she can always switch a lit lamp off");
        assert!(!s.act_lamp(), "but a dry lamp will not light");
    }

    #[test]
    fn a_canister_refills_fuel_and_only_when_it_is_there() {
        let mut s = fixed(1, 0);
        s.fuel = 40.0;
        assert!(!s.act_can(1));
        assert!(s.admin_set("can", 1.0));
        let first = s.cans[0];
        assert!(s.act_can(first));
        assert_eq!(s.fuel, 75.0);
        assert!(s.cans.is_empty());
    }

    #[test]
    fn the_fire_out_grace_is_ten_seconds_then_ysolde_is_let_in() {
        let mut s = fixed(2, -1);
        s.fire = 0.0;
        let ev = run(&mut s, &SimInput::default(), 20_000.0);
        assert_eq!(s.over, Over::Caught);
        assert_eq!((s.by, s.cause), ("ysolde", "fire"));
        assert!(ev.contains(&SimEvent::Caught { by: "ysolde", cause: "fire" }));
        assert!((s.elapsed - 10_000.0).abs() < 1.0, "ten seconds of the dark: {}", s.elapsed);
    }

    #[test]
    fn a_log_lights_the_fire_again_and_resets_the_grace() {
        let mut s = fixed(2, -1);
        s.fire = 0.0;
        run(&mut s, &SimInput::default(), 5_000.0);
        assert!(s.out >= 4_900.0);
        assert!(s.act_log());
        assert_eq!(s.logs, 5);
        s.step(100.0, &SimInput::default());
        assert_eq!(s.out, 0.0);
        assert!(s.fire > 25.0);
    }

    #[test]
    fn hiding_in_the_tent_lets_ysolde_creep_steal_and_look_in() {
        let mut s = fixed(2, -1);
        let hide = SimInput { tent: true, ..SimInput::default() };
        let ev = run(&mut s, &hide, 20_000.0);
        assert!(ev.contains(&SimEvent::TentIn));
        assert!(ev.contains(&SimEvent::Creep));
        assert!(ev.contains(&SimEvent::Stolen));
        assert_eq!(s.over, Over::Caught);
        assert_eq!((s.by, s.cause), ("ysolde", "tent"));
        assert_eq!(s.logs, 5, "she took a log from the pile");
        assert!(s.elapsed < 12_000.0, "{}", s.elapsed);
    }

    #[test]
    fn out_of_the_tent_ysolde_backs_off_more_slowly() {
        let mut s = fixed(2, -1);
        s.y.dist = 0;
        let ev = run(&mut s, &SimInput::default(), 40_000.0);
        let backs: Vec<i32> = ev.iter().filter_map(|e| if let SimEvent::Backoff { dist } = e { Some(*dist) } else { None }).collect();
        assert_eq!(backs, vec![1, 2]);
        assert_eq!(s.y.dist, 2);
    }

    #[test]
    fn the_torch_sends_verrine_back_and_the_tent_makes_quenelle_lose_the_trail() {
        let mut s = fixed(2, -1);
        s.v.dist = 0;
        s.q.dist = 0;
        let left = SimInput { torch_l: true, ..SimInput::default() };
        let ev = run(&mut s, &left, 1_700.0);
        assert!(ev.contains(&SimEvent::Repelled { id: "verrine", side: Side::L, dist: Some(2) }));
        assert_eq!(s.v.dist, 2);
        let mut t = fixed(2, -1);
        t.q.dist = 0;
        let hide = SimInput { tent: true, ..SimInput::default() };
        let ev = run(&mut t, &hide, 2_100.0);
        assert!(ev.contains(&SimEvent::Repelled { id: "quenelle", side: Side::R, dist: Some(1) }));
        assert_eq!(t.q.dist, 1);
        assert!(t.over == Over::Running);
    }

    #[test]
    fn weather_comes_at_20_s_and_goes_9_s_later_at_the_bus_stop() {
        let mut s = fixed(1, 0);
        let mut on_at = None;
        let mut off_at = None;
        for _ in 0..400 {
            for e in s.step(100.0, &SimInput::default()) {
                match e {
                    SimEvent::Weather { .. } => on_at = Some(s.elapsed),
                    SimEvent::WeatherEnd { .. } => off_at = Some(s.elapsed),
                    _ => {}
                }
            }
        }
        assert_eq!(on_at, Some(20_000.0));
        assert_eq!(off_at, Some(29_000.0));
        assert_eq!(s.weather.next, 34_000.0);
    }

    #[test]
    fn the_campfire_gets_rain_that_feeds_on_the_fire() {
        let mut dry = fixed(2, 0);
        let mut wet = fixed(2, 0);
        wet.weather.on = true;
        wet.weather.t = 0.0;
        wet.weather.lasts = 1.0e9;
        run(&mut dry, &SimInput::default(), 2_000.0);
        run(&mut wet, &SimInput::default(), 2_000.0);
        assert!(wet.fire < dry.fire, "2.2 times the burn");
        assert!((100.0 - wet.fire) / (100.0 - dry.fire) > 2.0);
    }

    #[test]
    fn the_bus_comes_up_the_road_in_the_last_18_seconds_and_the_cold_lets_go() {
        let mut s = fixed(1, 0);
        s.elapsed = 251_000.0;
        s.chill = 50.0;
        let mut bus_events = 0;
        for _ in 0..100 {
            for e in s.step(100.0, &SimInput::default()) {
                if e == SimEvent::Bus {
                    bus_events += 1;
                }
            }
        }
        assert!(bus_events >= 1);
        assert!(s.bus > 0.0 && s.bus < 1.0);
        assert!(s.chill < 50.0, "chill recovers twice as fast while the bus is coming");
    }

    #[test]
    fn a_quiet_watch_is_won_at_270_seconds() {
        let mut s = fixed(0, 0);
        s.gust = 1.0e12;
        let ev = run(&mut s, &SimInput::default(), 300_000.0);
        assert_eq!(s.over, Over::Win);
        assert_eq!(s.outcome(), Outcome::Won);
        assert_eq!(ev.last(), Some(&SimEvent::Win));
        assert!((s.elapsed - 270_000.0).abs() < 1.0);
    }

    #[test]
    fn god_mode_undoes_a_catch() {
        let mut s = fixed(1, -1);
        s.god = true;
        s.warm = 0.0;
        s.chill = 99.9;
        let ev = s.step(100.0, &SimInput::default());
        assert!(ev.contains(&SimEvent::GodSave { by: "wynelle", cause: "cold" }));
        assert_eq!(s.over, Over::Running);
        assert_eq!(s.chill, 40.0);
    }

    #[test]
    fn level_zero_hunters_are_off_and_the_cast_follows_the_watch() {
        let s = fixed(2, 0);
        assert!(!s.hunter_on("ysolde"));
        assert!(s.hunter_on("verrine") && s.hunter_on("quenelle"));
        let off = OpenAirSim::with_rng(2, Stream::fixed(0.5), -1, 0, 5, false);
        assert!(!off.hunter_on("verrine") && off.hunter_on("quenelle"));
        assert_eq!(off.level_of("quenelle"), 5);
        assert_eq!(fixed(1, -1).level(), 8);
        assert_eq!(fixed(0, -1).level(), 9);
        assert_eq!(fixed(0, -1).hunter_ids(), vec!["verrine"]);
    }

    #[test]
    fn the_ai_steps_up_through_the_night() {
        assert_eq!(keen(9, 0.0), 9);
        assert_eq!(keen(9, 3.0 * 45_000.0), 10);
        assert_eq!(keen(9, 4.0 * 45_000.0), 11);
        assert_eq!(keen(0, 4.0 * 45_000.0), 0);
        assert_eq!(keen(20, 5.0 * 45_000.0), 20);
    }

    #[test]
    fn a_seed_replays_the_same_watch() {
        for watch in 0..3usize {
            let digest = |seed: u32| {
                let mut s = OpenAirSim::new(watch, seed, -1, -1, -1, false);
                let mut ap = Autoplay::new(seed);
                let mut labels = Vec::new();
                for _ in 0..3_000 {
                    if s.over != Over::Running {
                        break;
                    }
                    let (input, acts) = ap.plan(&s, 100.0);
                    Autoplay::apply(&mut s, &acts);
                    for e in s.step(100.0, &input) {
                        labels.push(e.label());
                    }
                }
                (s.digest(), labels, s.outcome())
            };
            let a = digest(12345);
            let b = digest(12345);
            assert_eq!(a, b, "watch {watch}");
            assert_ne!(a.2, Outcome::Running, "a watch of 270 s ends: watch {watch}");
        }
    }

    #[test]
    fn events_have_stable_labels() {
        assert_eq!(SimEvent::Close { id: "verrine", dist: 2, side: Side::L }.label(), "close:verrine:L:2");
        assert_eq!(SimEvent::Caught { by: "wynelle", cause: "cold" }.label(), "caught:wynelle:cold");
        assert_eq!(SimEvent::Win.label(), "win");
    }
}
