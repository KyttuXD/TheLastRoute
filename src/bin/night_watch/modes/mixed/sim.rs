//! Mixed Shift's rules (port of `mixed_logic.gd`, itself a port of createMixedGame in the 2D game): a drawn roster of
//! threats of different types (counter, watched, monitor, path, vent, winder) run by a small pacing director. Every
//! threat gives a tell before it can kill and has an answer. No drawing, no window, no real clock: the shift calls the
//! setters (`set_door`, `set_vent`, `light_vent`, `set_wind`, `set_monitor`, `set_room`) and `step(dt_ms)`, and draws the
//! events. Three seeded streams (director, threats, feints) from `Mulberry32`; the draw ORDER follows the GDScript, so a
//! seed gives the same night, event for event.
#![allow(dead_code)]

use super::data::*;
use crate::core::{Fnv, Headless, Mulberry32, Outcome};

// ------------------------------------------------------------------------------------------------ events

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EvType {
    Step,
    Turned,
    Gone,
    Answered,
    Arrive,
    Tell,
    Pulse,
    Kill,
    Stop,
    Echo,
    Feint,
    Mood,
    MoodTell,
    Beat,
    Spike,
    Strain,
    DoorShut,
    VentShut,
    VentOpen,
    VentStrain,
    Hum,
    Wound,
    Chaos,
    Hour,
    Rotate,
    Win,
}

impl EvType {
    pub fn name(self) -> &'static str {
        match self {
            EvType::Step => "step",
            EvType::Turned => "turned",
            EvType::Gone => "gone",
            EvType::Answered => "answered",
            EvType::Arrive => "arrive",
            EvType::Tell => "tell",
            EvType::Pulse => "pulse",
            EvType::Kill => "kill",
            EvType::Stop => "stop",
            EvType::Echo => "echo",
            EvType::Feint => "feint",
            EvType::Mood => "mood",
            EvType::MoodTell => "moodtell",
            EvType::Beat => "beat",
            EvType::Spike => "spike",
            EvType::Strain => "strain",
            EvType::DoorShut => "doorshut",
            EvType::VentShut => "ventshut",
            EvType::VentOpen => "ventopen",
            EvType::VentStrain => "ventstrain",
            EvType::Hum => "hum",
            EvType::Wound => "wound",
            EvType::Chaos => "chaos",
            EvType::Hour => "hour",
            EvType::Rotate => "rotate",
            EvType::Win => "win",
        }
    }
}

/// One thing that happened in a step. Unused fields stay at their defaults.
#[derive(Clone, Debug)]
pub struct Ev {
    pub ty: EvType,
    /// Elapsed ms when it happened.
    pub t: f64,
    /// The action number (`act` events only) and the beat it happened in.
    pub a: i32,
    pub beat: Beat,
    pub id: String,
    /// A threat type name (`Kind::name`), a pulse kind (pulse events), a chaos kind, or "step" / "glow" (feints).
    pub kind: String,
    pub room: String,
    pub from: String,
    pub to: String,
    pub side: Option<Side>,
    /// How a threat was answered ("door", "seal", "light", "wind", "switch", "monitor", "steady"), a mood name, a spike phase.
    pub how: String,
    pub spot: usize,
    /// Tell lead (tell events) or kill lead (kill events), ms.
    pub lead: f64,
    pub level: f64,
    pub phase: String,
    /// rearm (tell), spawn (step), armed (pulse), soft (feint), quiet (arrive), was (wound).
    pub flag: bool,
    pub gen: i32,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

impl Ev {
    pub fn new(ty: EvType, t: f64) -> Ev {
        Ev {
            ty,
            t,
            a: 0,
            beat: Beat::Lull,
            id: String::new(),
            kind: String::new(),
            room: String::new(),
            from: String::new(),
            to: String::new(),
            side: None,
            how: String::new(),
            spot: 0,
            lead: 0.0,
            level: 0.0,
            phase: String::new(),
            flag: false,
            gen: 0,
            added: Vec::new(),
            removed: Vec::new(),
        }
    }
    fn id(mut self, v: &str) -> Ev {
        self.id = v.to_string();
        self
    }
    fn kind(mut self, v: &str) -> Ev {
        self.kind = v.to_string();
        self
    }
    fn room(mut self, v: &str) -> Ev {
        self.room = v.to_string();
        self
    }
    fn from(mut self, v: &str) -> Ev {
        self.from = v.to_string();
        self
    }
    fn to(mut self, v: &str) -> Ev {
        self.to = v.to_string();
        self
    }
    fn side(mut self, v: Option<Side>) -> Ev {
        self.side = v;
        self
    }
    fn how(mut self, v: &str) -> Ev {
        self.how = v.to_string();
        self
    }
    fn spot(mut self, v: usize) -> Ev {
        self.spot = v;
        self
    }
    fn lead(mut self, v: f64) -> Ev {
        self.lead = v;
        self
    }
    fn level(mut self, v: f64) -> Ev {
        self.level = v;
        self
    }
    fn phase(mut self, v: &str) -> Ev {
        self.phase = v.to_string();
        self
    }
    fn flag(mut self, v: bool) -> Ev {
        self.flag = v;
        self
    }
}

// ------------------------------------------------------------------------------------------------ state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    None,
    Win,
    Caught,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DoorS {
    pub shut: bool,
    pub want: bool,
    pub held: f64,
    pub cool: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct VentS {
    pub shut: bool,
    pub want: bool,
    pub held: f64,
    pub linger: f64,
    pub cool: f64,
    pub flash: f64,
}

#[derive(Clone, Debug)]
pub struct Mon {
    pub up: bool,
    pub room: String,
    pub stay: f64,
    pub toggles: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct Spike {
    pub at: f64,
    pub len: f64,
}

#[derive(Clone, Debug)]
pub struct ChaosS {
    pub hr: i32,
    pub at: f64,
    pub kind: String,
    pub until: f64,
    pub strain: f64,
    pub flicker: f64,
    pub boxfast: f64,
    pub room: String,
    pub n: i32,
}

#[derive(Clone, Debug)]
pub struct RunS {
    pub gen: i32,
    pub rotate_at: f64,
    pub budget: f64,
}

/// A threat's whole state as plain data.
#[derive(Clone, Debug)]
pub struct Threat {
    pub id: String,
    pub kind: Kind,
    pub mood: Mood,
    pub mood_left: f64,
    pub mood_cue: f64,
    pub active: bool,
    pub armed: bool,
    pub armed_ms: f64,
    pub armed_at: f64,
    pub tell_on: bool,
    pub tell_need: f64,
    pub side: Option<Side>,
    pub pulse: f64,
    pub give_ms: f64,
    pub wait: f64,
    /// How often she went to each side (L, R, T): the less, the likelier next.
    pub mem: [i32; 3],
    pub route: Vec<String>,
    pub route_id: String,
    pub start: usize,
    pub i: usize,
    pub seen: f64,
    pub c: f64,
    pub hot: bool,
    pub grace: f64,
    pub span: f64,
    pub present: bool,
    pub steps: i32,
    pub pend: i32,
    pub idle: f64,
    pub idle_max: f64,
    pub decay: f64,
    pub seen_toggles: i32,
    pub dwell: f64,
    pub flinch: bool,
    pub rewound: bool,
    pub tune: f64,
    pub wind_ms: f64,
    pub stopped: bool,
    pub box_room: String,
    pub hum: f64,
}

impl Threat {
    /// The room she stands in on her route, if she is walking one.
    pub fn room(&self) -> Option<&str> {
        if self.active && !self.route.is_empty() {
            self.route.get(self.i).map(|s| s.as_str())
        } else {
            None
        }
    }
}

// ------------------------------------------------------------------------------------------------ seeded draws

pub fn jitter(ms: f64, rng: &mut Mulberry32) -> f64 {
    let k = tun().jitter;
    ms * (1.0 - k + 2.0 * k * rng.next())
}

pub fn jitter_spread(base: f64, spread: f64, rng: &mut Mulberry32) -> f64 {
    let k = spread.clamp(0.0, 0.9);
    base * (1.0 - k + 2.0 * k * rng.next())
}

pub fn between(r: (f64, f64), u: f64) -> f64 {
    r.0 + (r.1 - r.0) * u
}

/// The tell a threat gives before it can kill: the shift's, scaled by its mood, never below the floor (nor the preset's minimum).
pub fn tell_for(tell_ms: f64, min_tell_ms: f64, mood: Mood, u: f64) -> f64 {
    let t = tun();
    let raw = (tell_ms * t.mood_tell[mood.idx()] * (1.0 + t.tell_jitter * (2.0 * u - 1.0))).round();
    t.tell_floor_ms.max(min_tell_ms.max(raw))
}

pub fn tension_floor_at(t: f64) -> f64 {
    let d = tun();
    if t < d.tension_floor_from {
        return 0.0;
    }
    let k = ((t - d.tension_floor_from) / (NIGHT_MS - d.tension_floor_from)).clamp(0.0, 1.0);
    d.tension_floor.0 + (d.tension_floor.1 - d.tension_floor.0) * k
}

/// The night's roster, drawn with one seeded generator; always at least `min_types` types, never more than `new_max`
/// newcomers or MAX_WALKERS walkers.
pub fn draw_roster(seed: u32, d: &Shift, gen: i32) -> Vec<String> {
    let t = tun();
    let mixed = ((seed as u64) ^ 0x524f_5354).wrapping_add((gen as i64 as u64).wrapping_mul(0x9e37_79b1)) & 0xFFFF_FFFF;
    let mut r = Mulberry32::new(mixed as u32);
    let range = if d.endless { t.ramp.roster_by_gen[(gen.max(0) as usize).min(t.ramp.roster_by_gen.len() - 1)] } else { d.roster };
    let must: Vec<String> = d.must.clone();
    let n = must.len().max(range.0 as usize + (r.next() * (range.1 - range.0 + 1) as f64).floor() as usize);
    let mut bag: Vec<String> = t.pool.iter().map(|p| p.id.clone()).collect();
    let mut i = bag.len() - 1;
    while i > 0 {
        let j = ((r.next() * (i + 1) as f64).floor() as usize).min(i);
        bag.swap(i, j);
        i -= 1;
    }
    let kind = |id: &str| -> Kind { t.kind_of(id).unwrap_or(Kind::Path) };
    let types_of = |ids: &[String]| -> Vec<Kind> {
        let mut out: Vec<Kind> = Vec::new();
        for id in ids {
            let k = kind(id);
            if !out.contains(&k) {
                out.push(k);
            }
        }
        out
    };
    let new_max = d.new_max as usize;
    let allowed = |have: &[String], id: &str| -> bool {
        let k = kind(id);
        let count = |set: &[Kind]| have.iter().filter(|x| set.contains(&kind(x))).count();
        !(t.new_types.contains(&k) && count(&t.new_types) >= new_max) && !(t.walker_types.contains(&k) && count(&t.walker_types) >= t.max_walkers)
    };
    let mut pick: Vec<String> = must.clone();
    for id in &bag {
        if pick.len() >= n {
            break;
        }
        if !pick.contains(id) && allowed(&pick, id) {
            pick.push(id.clone());
        }
    }
    let mut guard = 0;
    while types_of(&pick).len() < d.min_types as usize && guard < 8 {
        guard += 1;
        let have = types_of(&pick);
        let mut at = pick.len() - 1;
        let mut k = pick.len() as i64 - 1;
        while k >= 0 {
            let same = pick.iter().filter(|x| kind(x) == kind(&pick[k as usize])).count();
            if same > 1 {
                at = k as usize;
                break;
            }
            k -= 1;
        }
        let rest: Vec<String> = pick.iter().enumerate().filter(|(idx, _)| *idx != at).map(|(_, s)| s.clone()).collect();
        let add = bag.iter().find(|id| !have.contains(&kind(id)) && allowed(&rest, id));
        match add {
            Some(a) => pick[at] = a.clone(),
            None => break,
        }
    }
    t.pool.iter().filter(|p| pick.contains(&p.id)).map(|p| p.id.clone()).collect()
}

/// A threat's whole state. `k` is her place in the roster (it staggers the first moves).
pub fn new_threat(def: &PoolDef, rng: &mut Mulberry32, d: &Shift, toggles: i32, k: usize) -> Threat {
    let t = tun();
    let kind = def.kind;
    let mood = Mood::ALL[((rng.next() * Mood::ALL.len() as f64).floor() as usize).min(3)];
    let mut th = Threat {
        id: def.id.clone(),
        kind,
        mood,
        mood_left: 0.0,
        mood_cue: 0.0,
        active: false,
        armed: false,
        armed_ms: 0.0,
        armed_at: -1.0,
        tell_on: false,
        tell_need: 0.0,
        side: None,
        pulse: 0.0,
        give_ms: 0.0,
        wait: 0.0,
        mem: [0; 3],
        route: Vec::new(),
        route_id: String::new(),
        start: 0,
        i: 0,
        seen: 0.0,
        c: 0.0,
        hot: false,
        grace: 0.0,
        span: 0.0,
        present: false,
        steps: 0,
        pend: 0,
        idle: 0.0,
        idle_max: 0.0,
        decay: 0.0,
        seen_toggles: toggles,
        dwell: 0.0,
        flinch: false,
        rewound: false,
        tune: 1.0,
        wind_ms: 0.0,
        stopped: false,
        box_room: String::new(),
        hum: 0.0,
    };
    th.mood_left = between((t.mood_min_ms, t.mood_max_ms), rng.next());
    th.mood_cue = between(t.mood_cue_ms, rng.next());
    match kind {
        Kind::Counter => {
            th.grace = jitter(t.timing_of(Kind::Counter).grace_ms, rng);
            th.span = jitter(t.timing_of(Kind::Counter).span_ms, rng);
            th.active = true;
        }
        Kind::Monitor => {
            th.wait = jitter(t.timing_of(Kind::Monitor).arrive_ms, rng) + k as f64 * 1500.0;
        }
        Kind::Winder => {
            let n = t.box_rooms.len();
            th.box_room = t.box_rooms[((rng.next() * n as f64).floor() as usize).min(n - 1)].clone();
            th.span = jitter(t.tune_span_ms, rng);
            th.hum = t.hum_ms.0;
        }
        _ => {
            th.wait = jitter(d.start_ms, rng) + k as f64 * 2500.0;
        }
    }
    th
}

/// The spikes of a night: each at a jittered time inside its own slot, never in the first hour, ending END_CLEAR_MS before dawn.
pub fn plan_spikes(rng: &mut Mulberry32, d: &Shift) -> Vec<Spike> {
    let t = tun();
    let count = d.spikes.0 + (rng.next() * (d.spikes.1 - d.spikes.0 + 1) as f64).floor() as i32;
    let start = t.spike_first_ms + 8000.0;
    let end_ms = NIGHT_MS - t.end_clear_ms - t.spike_len_ms.1 - 6000.0;
    let width = (end_ms - start) / count as f64;
    let breather = d.breather.unwrap_or(t.breather_ms);
    let mut out = Vec::new();
    for k in 0..count {
        let length = between(t.spike_len_ms, rng.next()).round();
        let room = width - length - breather.1 - 2000.0;
        out.push(Spike { at: (start + k as f64 * width + rng.next() * room).round(), len: length });
    }
    out
}

pub fn first_endless_spike(rng: &mut Mulberry32) -> Spike {
    let t = tun();
    let at = (t.spike_first_ms + 8000.0 + rng.next() * 0.8 * HOUR_MS).round();
    Spike { at, len: between(t.spike_len_ms, rng.next()).round() }
}

// ------------------------------------------------------------------------------------------------ the core

/// Everything but the threats.
#[derive(Clone, Debug)]
pub struct Core {
    pub seed: u32,
    pub diff: Shift,
    pub endless: bool,
    pub chaos: bool,
    pub breather: (f64, f64),
    pub dir_rng: Mulberry32,
    pub th_rng: Mulberry32,
    pub fx_rng: Mulberry32,
    pub seal_max: f64,
    /// Admin: a multiplier on how fast the threats act (1.0 = the 2D game's numbers exactly).
    pub hunter_speed: f64,
    pub elapsed: f64,
    pub over: Over,
    pub by: String,
    pub cool: f64,
    /// The id of the threat whose deadly tell is running ("" = none).
    pub lock: String,
    pub act_seq: i32,
    pub since_act: f64,
    pub beat: Beat,
    pub beat_left: f64,
    pub tension: f64,
    pub plan: Vec<Spike>,
    pub spike_idx: usize,
    pub spike_end: f64,
    pub warned: bool,
    pub feint_left: f64,
    pub seal: f64,
    pub doors: [DoorS; 2],
    pub vent_seal: f64,
    pub vents: [VentS; 3],
    pub lamp_cool: f64,
    pub wind: bool,
    pub mon: Mon,
    pub chaos_s: Option<ChaosS>,
    pub run: Option<RunS>,
}

/// What `light_vent` answers.
#[derive(Clone, Debug, PartialEq)]
pub struct LightRes {
    pub ok: bool,
    pub why: &'static str,
    pub seen: Vec<String>,
}

fn side_weights(th: &Threat) -> [f64; 3] {
    let mut out = [0.0; 3];
    let n = if th.kind == Kind::Vent { 3 } else { 2 };
    for k in 0..n {
        out[k] = 0.2f64.max(1.0 / (1.0 + 0.8 * th.mem[k] as f64));
    }
    out
}

fn remember(th: &mut Threat, side: Option<Side>) {
    if let Some(s) = side {
        th.mem[s.idx()] = (th.mem[s.idx()] + 1).min(6);
    }
}

fn clear_gate_state(th: &mut Threat) {
    th.armed = false;
    th.tell_on = false;
    th.armed_ms = 0.0;
    th.give_ms = 0.0;
    th.pulse = 0.0;
}

impl Core {
    pub fn hour_of(&self) -> i32 {
        (self.elapsed / HOUR_MS).floor() as i32
    }

    /// The pace in force now: the shift's own, except in Endless where it ramps up by the hour.
    pub fn live_speed(&self) -> f64 {
        if self.endless {
            tun().endless_pace(self.hour_of())
        } else {
            self.diff.speed
        }
    }

    /// The tell in force now (Endless ramps it down by the hour).
    pub fn live_tell(&self) -> f64 {
        if self.endless {
            tun().endless_tell(self.hour_of())
        } else {
            self.diff.tell_ms
        }
    }

    fn quiet(&self) -> bool {
        self.beat == Beat::Breather
    }

    fn can_act(&self) -> bool {
        self.cool <= 0.0 && !self.quiet()
    }

    fn time_runs(&self, th: &Threat) -> bool {
        !self.quiet() && (self.lock.is_empty() || self.lock == th.id)
    }

    fn speed_of(&self, th: &Threat) -> f64 {
        let t = tun();
        t.beat_rate[self.beat.idx()] * (0.8 + 0.5 * self.tension) * t.mood_speed[th.mood.idx()] * self.live_speed() * self.hunter_speed
    }

    fn grace_ok(&self) -> bool {
        self.elapsed >= self.diff.grace_ms
    }

    fn observed(&self, room: &str) -> bool {
        self.mon.up && self.mon.room == room
    }

    fn act(&mut self, ev: &mut Vec<Ev>, events: Vec<Ev>) {
        self.cool = tun().cooldown_ms;
        self.act_seq += 1;
        self.since_act = 0.0;
        for mut e in events {
            e.t = self.elapsed;
            e.a = self.act_seq;
            e.beat = self.beat;
            ev.push(e);
        }
    }

    fn scare(&mut self, d: f64) {
        self.tension = (self.tension + d).clamp(0.0, 1.0);
    }

    fn mk(&self, ty: EvType) -> Ev {
        Ev::new(ty, self.elapsed)
    }

    fn choose_route(&mut self, th: &mut Threat) {
        let t = tun();
        let table: Vec<&Route> = match th.kind {
            Kind::Vent => t.vent_routes.iter().collect(),
            Kind::Winder => t.routes.iter().filter(|r| r.rooms.contains(&th.box_room)).collect(),
            _ => t.routes.iter().collect(),
        };
        let table: Vec<&Route> = if table.is_empty() { t.routes.iter().collect() } else { table };
        let w = side_weights(th);
        let weights: Vec<f64> = table.iter().map(|r| w[r.side.idx()]).collect();
        let total: f64 = weights.iter().sum();
        let mut x = self.th_rng.next() * total;
        let mut k = 0;
        while k < table.len() - 1 && x >= weights[k] {
            x -= weights[k];
            k += 1;
        }
        let r2 = table[k];
        let at = r2.rooms.iter().position(|r| *r == th.box_room);
        th.start = match at {
            Some(a) if th.kind == Kind::Winder && a + 3 <= r2.rooms.len() => a,
            _ => 0,
        };
        th.route_id = r2.id.clone();
        th.route = r2.rooms[th.start..].to_vec();
        th.i = 0;
        th.seen = 0.0;
        th.flinch = false;
    }

    /// Which spot of the vent run a vent crawler stands at.
    pub fn vent_spot_of(th: &Threat) -> usize {
        for r in tun().vent_routes.iter() {
            if r.id == th.route_id {
                return r.spots.get(th.i).copied().unwrap_or(0);
            }
        }
        0
    }

    fn gate_shut(&self, th: &Threat) -> bool {
        match th.side {
            Some(s) if th.kind == Kind::Vent => self.vents[s.idx()].shut,
            Some(s) if s != Side::T => self.doors[s.idx()].shut,
            _ => false,
        }
    }

    fn arm(&mut self, th: &mut Threat, side: Option<Side>) {
        let t = tun();
        self.lock = th.id.clone();
        th.armed = true;
        th.armed_ms = 0.0;
        th.armed_at = -1.0;
        th.tell_on = false;
        th.give_ms = 0.0;
        th.side = side;
        th.pulse = 0.0;
        th.tell_need = tell_for(self.live_tell(), self.diff.min_tell_ms, th.mood, self.th_rng.next());
        if th.kind == Kind::Vent {
            if self.vent_seal < t.vent_reserve {
                self.vent_seal = t.vent_reserve;
            }
        } else if side.is_some() && self.seal < t.seal_reserve {
            if self.endless {
                let budget = self.run.as_ref().map(|r| r.budget).unwrap_or(0.0);
                let g = (t.seal_reserve - self.seal).min(budget);
                self.seal += g;
                if let Some(r) = self.run.as_mut() {
                    r.budget -= g;
                }
            } else {
                self.seal = t.seal_reserve;
            }
        }
        self.scare(-0.15);
    }

    fn start_tell(&mut self, th: &mut Threat, ev: &mut Vec<Ev>, rearm: bool) {
        let t = tun();
        th.tell_on = true;
        th.armed_ms = 0.0;
        th.armed_at = self.elapsed;
        th.pulse = t.pulse[th.kind.idx()].0;
        ev.push(self.mk(EvType::Tell).id(&th.id).kind(th.kind.name()).side(th.side).lead(th.tell_need).flag(rearm));
    }

    fn pulses(&mut self, th: &mut Threat, ev: &mut Vec<Ev>, dt: f64, level: f64) {
        let t = tun();
        let p = t.pulse[th.kind.idx()];
        let period = p.0 + (p.1 - p.0) * level.clamp(0.0, 1.0);
        th.pulse += dt;
        if th.pulse >= period {
            th.pulse -= period;
            ev.push(self.mk(EvType::Pulse).id(&th.id).kind(&t.tell_kind[th.kind.idx()]).side(th.side).level(level).flag(th.armed));
        }
    }

    fn kill(&mut self, th: &mut Threat, ev: &mut Vec<Ev>) -> bool {
        let lead = self.elapsed - th.armed_at;
        if lead < tun().tell_floor_ms {
            return false;
        }
        let e = self.mk(EvType::Kill).id(&th.id).kind(th.kind.name()).side(th.side).lead(lead);
        self.act(ev, vec![e]);
        self.over = Over::Caught;
        self.by = th.id.clone();
        true
    }

    fn send_away(&mut self, th: &mut Threat, ev: &mut Vec<Ev>, how: &str) {
        let side = th.side;
        clear_gate_state(th);
        if self.lock == th.id {
            self.lock.clear();
        }
        remember(th, side);
        th.active = false;
        th.present = false;
        th.steps = 0;
        th.pend = 0;
        th.i = 0;
        th.seen = 0.0;
        th.dwell = 0.0;
        th.side = None;
        th.flinch = false;
        th.rewound = false;
        if th.kind == Kind::Winder && how == "wind" {
            th.stopped = false;
            th.wait = 0.0;
        } else {
            th.wait = jitter(tun().timing_of(th.kind).rest_ms, &mut self.th_rng);
        }
        self.scare(-0.25);
        let answered = self.mk(EvType::Answered).id(&th.id).kind(th.kind.name()).how(how).side(side);
        let gone = self.mk(EvType::Gone).id(&th.id);
        self.act(ev, vec![answered, gone]);
    }

    // ---- doors and vents

    fn step_doors(&mut self, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        let mut open = 0;
        let strained = self.chaos_s.as_ref().map(|c| c.strain > 0.0).unwrap_or(false);
        let strain_def = t.chaos_def("strain");
        let hold_ms = if strained { strain_def.map(|d| d.hold_ms).unwrap_or(t.door_hold_ms) } else { t.door_hold_ms };
        for (i, side) in [Side::L, Side::R].iter().enumerate() {
            let mut d = self.doors[i];
            if d.cool > 0.0 {
                d.cool = (d.cool - dt).max(0.0);
            }
            if d.shut {
                d.held += dt;
                self.seal = (self.seal - dt).max(0.0);
                if d.held >= hold_ms || self.seal <= 0.0 {
                    d.shut = false;
                    d.want = false;
                    d.held = 0.0;
                    d.cool = t.pop_cool_ms;
                    ev.push(self.mk(EvType::Strain).side(Some(*side)));
                }
            } else if d.want && d.cool <= 0.0 && self.seal >= t.seal_min_to_shut {
                d.shut = true;
                d.held = 0.0;
                ev.push(self.mk(EvType::DoorShut).side(Some(*side)));
            }
            if !d.shut {
                open += 1;
            }
            self.doors[i] = d;
        }
        if open > 0 {
            let base = if self.endless { t.endless_seal_regen } else { t.seal_regen };
            let rate = base * if strained { strain_def.map(|d| d.regen).unwrap_or(1.0) } else { 1.0 };
            let mut gain = rate * dt * (open as f64 / 2.0);
            if self.endless {
                if let Some(r) = self.run.as_mut() {
                    gain = gain.min(r.budget.min((self.seal_max - self.seal).max(0.0)));
                    r.budget -= gain;
                }
            }
            self.seal = self.seal_max.min(self.seal + gain);
        }
    }

    fn step_vents(&mut self, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        self.lamp_cool = (self.lamp_cool - dt).max(0.0);
        let mut open = 0;
        for side in Side::ALL {
            let i = side.idx();
            let mut d = self.vents[i];
            if d.cool > 0.0 {
                d.cool = (d.cool - dt).max(0.0);
            }
            if d.flash > 0.0 {
                d.flash = (d.flash - dt).max(0.0);
            }
            if d.shut {
                self.vent_seal = (self.vent_seal - dt).max(0.0);
                if d.want {
                    d.held += dt;
                } else {
                    d.linger = (d.linger - dt).max(0.0);
                }
                if d.held >= t.vent_hold_ms || self.vent_seal <= 0.0 {
                    d.shut = false;
                    d.want = false;
                    d.held = 0.0;
                    d.linger = 0.0;
                    d.cool = t.vent_pop_cool_ms;
                    ev.push(self.mk(EvType::VentStrain).side(Some(side)));
                } else if !d.want && d.linger <= 0.0 {
                    d.shut = false;
                    d.held = 0.0;
                    ev.push(self.mk(EvType::VentOpen).side(Some(side)));
                }
            } else if d.want && d.cool <= 0.0 && self.vent_seal >= t.vent_min_to_seal {
                d.shut = true;
                d.held = 0.0;
                d.linger = 0.0;
                ev.push(self.mk(EvType::VentShut).side(Some(side)));
            }
            if !d.shut {
                open += 1;
            }
            self.vents[i] = d;
        }
        if open > 0 {
            self.vent_seal = t.vent_seal_max.min(self.vent_seal + t.vent_seal_regen * dt * (open as f64 / 3.0));
        }
    }

    /// Hold a vent's control (want = true) to seal it; let go and it stays shut VENT_LINGER_MS more. Returns whether it is shut.
    pub fn set_vent(&mut self, side: Side, want_in: bool) -> bool {
        let t = tun();
        if self.over != Over::None {
            return false;
        }
        let i = side.idx();
        let mut d = self.vents[i];
        if want_in == d.want {
            return d.shut;
        }
        d.want = want_in;
        if want_in {
            if d.shut {
                d.linger = 0.0;
            } else if d.cool <= 0.0 && self.vent_seal >= t.vent_min_to_seal {
                d.shut = true;
                d.held = 0.0;
                d.linger = 0.0;
            }
        } else if d.shut {
            d.linger = t.vent_linger_ms;
        }
        self.vents[i] = d;
        d.shut
    }

    /// The vent light: a short pale flash down a vent (one lamp for the three). Shows the crawler if she is near that vent
    /// or at it, and she backs off.
    pub fn light_vent(&mut self, side: Side, threats: &mut [Threat]) -> LightRes {
        let t = tun();
        if self.over != Over::None {
            return LightRes { ok: false, why: "over", seen: Vec::new() };
        }
        let i = side.idx();
        if self.vents[i].shut {
            return LightRes { ok: false, why: "shut", seen: Vec::new() };
        }
        if self.lamp_cool > 0.0 {
            return LightRes { ok: false, why: "cool", seen: Vec::new() };
        }
        self.lamp_cool = t.vent_light_cool_ms;
        self.vents[i].flash = t.vent_light_ms;
        let mut seen = Vec::new();
        for th in threats.iter_mut() {
            if th.kind != Kind::Vent {
                continue;
            }
            let at_it = th.armed && th.side == Some(side);
            let near = th.active && !th.armed && t.vent_near[i].contains(&Core::vent_spot_of(th));
            if at_it || near {
                th.flinch = true;
                seen.push(th.id.clone());
            }
        }
        LightRes { ok: true, why: "", seen }
    }

    fn wind_jump(&mut self, threats: &[Threat]) {
        for t in threats {
            if t.kind == Kind::Winder {
                if self.wind && self.mon.up && self.mon.room != t.box_room {
                    let room = t.box_room.clone();
                    self.set_room(&room);
                }
                return;
            }
        }
    }

    pub fn set_wind(&mut self, on: bool, threats: &[Threat]) {
        if self.over == Over::None {
            self.wind = on;
            self.wind_jump(threats);
        }
    }

    pub fn set_door(&mut self, side: Side, want_in: bool) -> bool {
        let t = tun();
        if side == Side::T || self.over != Over::None {
            return false;
        }
        let d = &mut self.doors[side.idx()];
        d.want = want_in;
        if !want_in {
            d.shut = false;
            d.held = 0.0;
        } else if !d.shut && d.cool <= 0.0 && self.seal >= t.seal_min_to_shut {
            d.shut = true;
            d.held = 0.0;
        }
        d.shut
    }

    pub fn set_monitor(&mut self, up: bool, threats: &[Threat]) {
        if self.over != Over::None || up == self.mon.up {
            return;
        }
        self.mon.up = up;
        self.mon.stay = 0.0;
        self.mon.toggles += 1;
        self.wind_jump(threats);
    }

    pub fn set_room(&mut self, room: &str) {
        if self.over != Over::None || !tun().cam_rooms.iter().any(|r| r == room) || room == self.mon.room {
            return;
        }
        self.mon.room = room.to_string();
        self.mon.stay = 0.0;
    }

    // ---- the director

    fn next_idx(&self) -> usize {
        if self.endless {
            0
        } else {
            self.spike_idx
        }
    }

    fn start_beat(&mut self, beat: Beat, ev: &mut Vec<Ev>) {
        let t = tun();
        self.beat = beat;
        self.beat_left = match beat {
            Beat::Breather => between(self.breather, self.dir_rng.next()),
            Beat::Spike => self.plan.get(self.next_idx()).map(|s| s.len).unwrap_or(11_000.0),
            Beat::Lull => between(t.lull_len, self.dir_rng.next()),
            Beat::Build => between(t.build_len, self.dir_rng.next()),
        };
        let mut e = self.mk(EvType::Beat);
        e.how = beat.name().to_string();
        e.level = self.tension;
        ev.push(e);
    }

    fn end_spike(&mut self, from_ms: Option<f64>) {
        let t = tun();
        let from = from_ms.unwrap_or(self.elapsed);
        if self.endless {
            if !self.plan.is_empty() {
                self.plan.remove(0);
            }
            let gap = jitter_spread(t.endless_gap(self.hour_of()), 0.25, &mut self.dir_rng);
            let at = (from + t.breather_ms.1 + gap).round();
            let len = between(t.spike_len_ms, self.dir_rng.next()).round();
            self.plan.push(Spike { at, len });
        }
        self.spike_idx += 1;
    }

    fn step_director(&mut self, dt: f64, threats: &mut [Threat], ev: &mut Vec<Ev>) {
        let t = tun();
        self.beat_left -= dt;
        let nxt: Option<Spike> = self.plan.get(self.next_idx()).copied();
        let beat = self.beat;
        match beat {
            Beat::Spike => {
                if self.elapsed >= self.spike_end && self.lock.is_empty() {
                    ev.push(self.mk(EvType::Spike).phase("end"));
                    self.end_spike(None);
                    self.start_beat(Beat::Breather, ev);
                }
            }
            Beat::Breather => {
                if self.beat_left <= 0.0 {
                    self.start_beat(Beat::Lull, ev);
                }
            }
            _ => {
                if let Some(n) = nxt {
                    if !self.warned && self.elapsed >= n.at - t.spike_warn_ms {
                        self.warned = true;
                        let mut e = self.mk(EvType::Spike).phase("warn");
                        e.level = n.at;
                        ev.push(e);
                    }
                }
                let go = matches!(nxt, Some(n) if self.elapsed >= n.at) && self.lock.is_empty();
                if go {
                    let n = nxt.unwrap_or(Spike { at: 0.0, len: 11_000.0 });
                    self.warned = false;
                    self.spike_end = self.elapsed + n.len;
                    self.start_beat(Beat::Spike, ev);
                    ev.push(self.mk(EvType::Spike).phase("start"));
                    for th in threats.iter_mut() {
                        if th.active && !th.armed && th.kind != Kind::Counter {
                            th.wait = th.wait.min(jitter(2500.0, &mut self.dir_rng));
                        }
                    }
                } else if self.beat_left <= 0.0 {
                    let last = self.elapsed >= t.tension_floor_from;
                    let mut again = false;
                    if beat == Beat::Build && self.tension > 0.5 {
                        again = self.dir_rng.next() < if last { 0.8 } else { 0.35 };
                    }
                    self.start_beat(if beat == Beat::Lull || again { Beat::Build } else { Beat::Lull }, ev);
                }
            }
        }
    }

    fn step_feints(&mut self, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        let beat = self.beat;
        if beat != Beat::Lull && beat != Beat::Build {
            return;
        }
        self.feint_left -= dt;
        if self.feint_left > 0.0 {
            return;
        }
        if !self.lock.is_empty() || self.since_act < 2500.0 || self.cool > 0.0 {
            self.feint_left = 1000.0;
            return;
        }
        let step = self.fx_rng.next() < 0.5;
        if step {
            let n = t.cam_rooms.len();
            let room = t.cam_rooms[((self.fx_rng.next() * n as f64).floor() as usize).min(n - 1)].clone();
            ev.push(self.mk(EvType::Feint).kind("step").room(&room).flag(true));
        } else {
            let side = if self.fx_rng.next() < 0.5 { Side::L } else { Side::R };
            ev.push(self.mk(EvType::Feint).kind("glow").side(Some(side)).flag(true));
        }
        let range = if beat == Beat::Lull { t.lull_feint } else { t.build_feint };
        self.feint_left = between(range, self.fx_rng.next());
    }

    // ---- one threat

    fn step_mood(&mut self, th: &mut Threat, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        th.mood_left -= dt;
        if th.mood_left <= 0.0 {
            let others: Vec<Mood> = Mood::ALL.iter().copied().filter(|m| *m != th.mood).collect();
            th.mood = others[((self.th_rng.next() * others.len() as f64).floor() as usize).min(others.len() - 1)];
            th.mood_left = between((t.mood_min_ms, t.mood_max_ms), self.th_rng.next());
            ev.push(self.mk(EvType::Mood).id(&th.id).how(th.mood.name()));
        }
        th.mood_cue -= dt;
        if th.mood_cue <= 0.0 {
            th.mood_cue = between(t.mood_cue_ms, self.th_rng.next());
            if th.active || th.present {
                ev.push(self.mk(EvType::MoodTell).id(&th.id).how(th.mood.name()));
            }
        }
    }

    fn step_walker(&mut self, th: &mut Threat, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        let kind = th.kind;
        let watched = kind == Kind::Watched;
        if th.armed {
            self.step_armed_at_gate(th, dt, ev);
            return;
        }
        if !th.active {
            if !self.time_runs(th) {
                return;
            }
            th.wait -= dt * self.speed_of(th);
            if th.wait > 0.0 || !self.can_act() {
                return;
            }
            self.choose_route(th);
            th.active = true;
            th.dwell = 0.0;
            th.wait = jitter(t.timing_of(kind).step_ms, &mut self.th_rng);
            let mut e = self.mk(EvType::Step).id(&th.id).kind(kind.name()).room(&th.route[0]).flag(true);
            e.level = th.wait;
            if kind == Kind::Vent {
                e = e.spot(Core::vent_spot_of(th));
            }
            self.act(ev, vec![e]);
            return;
        }
        if kind == Kind::Winder && th.rewound {
            if self.can_act() {
                self.send_away(th, ev, "wind");
            }
            return;
        }
        let route = th.route.clone();
        let room = route[th.i.min(route.len() - 1)].clone();
        let hall = route[route.len() - 2].clone();
        let seen_now = self.observed(&room);
        th.dwell += dt;
        if kind == Kind::Vent && th.flinch && th.dwell >= t.min_dwell_ms && self.can_act() {
            th.flinch = false;
            th.i = 0;
            th.dwell = 0.0;
            th.wait = jitter(t.timing_of(Kind::Vent).step_ms, &mut self.th_rng);
            for r in t.vent_routes.iter() {
                if r.id == th.route_id {
                    remember(th, Some(r.side));
                }
            }
            self.scare(-0.1);
            let e = self.mk(EvType::Turned).id(&th.id).kind("vent").room(&room).to(&room).how("light").spot(0);
            self.act(ev, vec![e]);
            return;
        }
        if room == hall && kind == Kind::Path && self.time_runs(th) {
            if seen_now {
                th.seen += dt;
                if th.seen >= t.choke_ms && self.can_act() {
                    let to = th.i.saturating_sub(2);
                    th.i = to;
                    th.seen = 0.0;
                    th.dwell = 0.0;
                    th.wait = jitter(t.timing_of(Kind::Path).step_ms, &mut self.th_rng);
                    remember(th, door_side(route.last().map(|s| s.as_str()).unwrap_or("")));
                    self.scare(-0.1);
                    let e = self.mk(EvType::Turned).id(&th.id).kind(kind.name()).room(&room).to(&route[to]);
                    self.act(ev, vec![e]);
                    return;
                }
            } else {
                th.seen = 0.0;
            }
        }
        if seen_now && (watched || th.mood == Mood::Shy) {
            return;
        }
        if !self.time_runs(th) {
            return;
        }
        th.wait -= dt * self.speed_of(th);
        if th.wait > 0.0 || th.dwell < t.min_dwell_ms || !self.can_act() {
            return;
        }
        let nxt = match route.get(th.i + 1) {
            Some(n) => n.clone(),
            None => return,
        };
        let side = gate_side(&nxt);
        if side.is_some() {
            if !self.lock.is_empty() || !self.grace_ok() {
                return;
            }
            th.i += 1;
            self.arm(th, side);
            let e = self.mk(EvType::Arrive).id(&th.id).kind(kind.name()).room(&nxt).from(&room).side(side);
            self.act(ev, vec![e]);
            if !self.gate_shut(th) {
                self.start_tell(th, ev, false);
            }
            return;
        }
        th.i += 1;
        th.dwell = 0.0;
        th.wait = jitter(t.timing_of(kind).step_ms, &mut self.th_rng);
        let mut e2 = self.mk(EvType::Step).id(&th.id).kind(kind.name()).room(&nxt).from(&room);
        e2.level = th.wait;
        if kind == Kind::Vent {
            e2 = e2.spot(Core::vent_spot_of(th));
        }
        self.act(ev, vec![e2]);
    }

    fn step_armed_at_gate(&mut self, th: &mut Threat, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        let kind = th.kind;
        if kind == Kind::Vent && th.flinch && self.can_act() {
            self.send_away(th, ev, "light");
            return;
        }
        if kind == Kind::Winder && th.rewound && self.can_act() {
            self.send_away(th, ev, "wind");
            return;
        }
        if self.gate_shut(th) {
            th.tell_on = false;
            th.armed_ms = 0.0;
            th.give_ms += dt;
            if th.give_ms >= t.give_up_ms && self.can_act() {
                self.send_away(th, ev, if kind == Kind::Vent { "seal" } else { "door" });
            }
            return;
        }
        th.give_ms = 0.0;
        if !th.tell_on {
            self.start_tell(th, ev, true);
            return;
        }
        th.armed_ms += dt;
        let level = th.armed_ms / th.tell_need;
        self.pulses(th, ev, dt, level);
        if th.armed_ms >= th.tell_need && self.can_act() {
            self.kill(th, ev);
        }
    }

    fn step_winder(&mut self, th: &mut Threat, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        let winding = self.wind && self.mon.up && self.mon.room == th.box_room && self.over == Over::None;
        th.wind_ms = if winding { th.wind_ms + dt } else { 0.0 };
        let was = th.stopped;
        let before = th.tune;
        if !was && self.time_runs(th) {
            let boxfast = match &self.chaos_s {
                Some(c) if c.boxfast > 0.0 => t.chaos_def("boxfast").map(|d| d.factor).unwrap_or(1.0),
                _ => 1.0,
            };
            th.tune = (th.tune - dt * self.speed_of(th) * boxfast / th.span).max(0.0);
        }
        if winding {
            th.tune = (th.tune + t.wind_per_sec * dt / 1000.0).min(1.0);
            if was && th.tune >= t.restart_at {
                th.stopped = false;
                th.hum = 0.0;
                th.span = jitter(t.tune_span_ms, &mut self.th_rng);
                ev.push(self.mk(EvType::Wound).id(&th.id).room(&th.box_room).flag(was));
                if th.active || th.armed {
                    th.rewound = true;
                } else {
                    th.wait = 0.0;
                }
            } else if !was && before < 1.0 && th.tune >= 1.0 {
                th.span = jitter(t.tune_span_ms, &mut self.th_rng);
            }
        }
        if !th.stopped {
            if self.elapsed < self.diff.grace_ms {
                th.tune = th.tune.max(t.grace_floor);
            }
            if th.tune <= 0.0005 && th.wind_ms == 0.0 {
                if self.can_act() && self.grace_ok() && self.time_runs(th) {
                    th.tune = 0.0;
                    th.stopped = true;
                    th.wait = jitter(t.timing_of(Kind::Winder).cranky_ms, &mut self.th_rng);
                    th.hum = 0.0;
                    let mut e = self.mk(EvType::Stop).id(&th.id).kind("winder").room(&th.box_room);
                    e.level = th.wait;
                    self.act(ev, vec![e]);
                } else {
                    th.tune = 0.0006;
                }
            }
            th.hum -= dt;
            if th.hum <= 0.0 && !th.stopped {
                let h = t.hum_ms;
                th.hum = h.0 + (h.1 - h.0) * (1.0 - th.tune).clamp(0.0, 1.0);
                let mut e = self.mk(EvType::Hum).id(&th.id).room(&th.box_room);
                e.level = th.tune;
                ev.push(e);
            }
        }
        if th.stopped || th.active || th.armed {
            self.step_walker(th, dt, ev);
        }
    }

    fn step_counter(&mut self, th: &mut Threat, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        let c = t.timing_of(Kind::Counter);
        let holding = self.mon.up && self.mon.stay >= th.grace;
        let sp = self.speed_of(th);
        let span_eff = th.span / 0.3f64.max(if sp != 0.0 { sp } else { 0.3 });
        if holding && !self.quiet() {
            th.c = (th.c + dt / span_eff).min(1.0);
        } else if !holding {
            th.c = (th.c - c.calm_decay * dt / th.span).max(0.0);
        }
        if th.c > 0.0 {
            th.hot = true;
        } else if th.hot {
            th.hot = false;
            th.grace = jitter(c.grace_ms, &mut self.th_rng);
            th.span = jitter(c.span_ms, &mut self.th_rng);
        }
        if !th.armed {
            if th.c >= c.arm_at && holding {
                if self.lock.is_empty() && self.can_act() && self.grace_ok() {
                    self.arm(th, None);
                    let e = self.mk(EvType::Arrive).id(&th.id).kind("counter").room(&self.mon.room.clone());
                    self.act(ev, vec![e]);
                    self.start_tell(th, ev, false);
                } else {
                    th.c = th.c.min(c.arm_at);
                }
            } else if th.c >= c.cue_at {
                let level = th.c;
                self.pulses(th, ev, dt, level);
            }
            return;
        }
        if !holding {
            if !self.can_act() {
                return;
            }
            th.armed = false;
            th.tell_on = false;
            th.armed_ms = 0.0;
            if self.lock == th.id {
                self.lock.clear();
            }
            th.grace = jitter(c.grace_ms, &mut self.th_rng);
            th.span = jitter(c.span_ms, &mut self.th_rng);
            self.scare(-0.25);
            let how = if self.mon.up { "switch" } else { "monitor" };
            let e = self.mk(EvType::Answered).id(&th.id).kind("counter").how(how);
            self.act(ev, vec![e]);
            return;
        }
        th.armed_ms += dt;
        let level = th.c.max(th.armed_ms / th.tell_need);
        self.pulses(th, ev, dt, level);
        if th.c >= 1.0 && th.armed_ms >= th.tell_need && self.can_act() {
            self.kill(th, ev);
        }
    }

    fn step_monitor_gated(&mut self, th: &mut Threat, dt: f64, ev: &mut Vec<Ev>) {
        let t = tun();
        let m = t.timing_of(Kind::Monitor);
        let fresh = self.mon.toggles - th.seen_toggles;
        th.seen_toggles = self.mon.toggles;
        if !th.present {
            if !self.time_runs(th) {
                return;
            }
            th.wait -= dt * self.speed_of(th);
            if th.wait > 0.0 || !self.can_act() || !self.lock.is_empty() || !self.grace_ok() {
                return;
            }
            let w = side_weights(th);
            let side = if self.th_rng.next() * (w[0] + w[1]) < w[0] { Side::L } else { Side::R };
            th.present = true;
            th.active = true;
            th.side = Some(side);
            th.steps = 0;
            th.pend = 0;
            th.idle = 0.0;
            th.decay = 0.0;
            th.give_ms = 0.0;
            th.idle_max = jitter(m.idle_leave_ms, &mut self.th_rng);
            let mut e = self.mk(EvType::Arrive).id(&th.id).kind("monitor").room("office").side(Some(side));
            e.flag = true;
            self.act(ev, vec![e]);
            return;
        }
        if th.armed {
            self.step_armed_at_gate(th, dt, ev);
            return;
        }
        if self.quiet() {
            th.pend = 0;
            return;
        }
        th.pend = (th.pend + fresh).min(1);
        let side = th.side.unwrap_or(Side::L);
        if self.doors[side.idx()].shut {
            th.give_ms += dt;
            if th.give_ms >= t.give_up_ms && self.can_act() {
                self.send_away(th, ev, "door");
                return;
            }
        } else {
            th.give_ms = 0.0;
        }
        if !self.time_runs(th) {
            return;
        }
        th.idle += dt;
        th.decay += dt;
        if th.pend > 0 && self.can_act() {
            th.pend = 0;
            th.idle = 0.0;
            th.decay = 0.0;
            if th.steps < m.steps_to_arm {
                th.steps += 1;
                let e = self.mk(EvType::Echo).id(&th.id).kind("monitor").level(th.steps as f64).side(th.side);
                self.act(ev, vec![e]);
            } else if self.lock.is_empty() && self.grace_ok() {
                self.arm(th, Some(side));
                let e = self.mk(EvType::Arrive).id(&th.id).kind("monitor").room("office").from("office").side(Some(side));
                self.act(ev, vec![e]);
                if !self.doors[side.idx()].shut {
                    self.start_tell(th, ev, false);
                }
            } else {
                th.pend = 1;
            }
            return;
        }
        if th.decay >= m.decay_ms && th.steps > 0 {
            th.steps -= 1;
            th.decay = 0.0;
        }
        if th.idle >= th.idle_max && self.can_act() {
            self.send_away(th, ev, "steady");
        }
    }

    // ---- CHAOS and ENDLESS

    fn apply_floor(&mut self) {
        let t = tun();
        if self.endless {
            self.tension = self.tension.max(t.endless_floor_at(self.elapsed));
        } else if self.elapsed >= t.tension_floor_from {
            self.tension = self.tension.max(tension_floor_at(self.elapsed));
        }
    }

    fn step_hours(&mut self, threats: &[Threat], ev: &mut Vec<Ev>) {
        let t = tun();
        loop {
            let hr = match &self.chaos_s {
                Some(c) => c.hr,
                None => return,
            };
            if hr >= self.hour_of() {
                return;
            }
            let hr = hr + 1;
            if let Some(c) = self.chaos_s.as_mut() {
                c.hr = hr;
            }
            if self.endless {
                if let Some(r) = self.run.as_mut() {
                    r.budget = t.endless_budget_at(hr);
                }
                let mut e = self.mk(EvType::Hour);
                e.level = hr as f64;
                e.gen = hr;
                ev.push(e);
            }
            let roll = if self.chaos {
                hr >= t.chaos_hours.0 && hr <= t.chaos_hours.1
            } else {
                self.endless && hr >= t.ramp.event_from && self.dir_rng.next() < t.ramp.event_p
            };
            let at = self.chaos_s.as_ref().map(|c| c.at).unwrap_or(-1.0);
            if roll && at < 0.0 {
                self.schedule_event(hr as f64 * HOUR_MS, threats);
            }
        }
    }

    fn schedule_event(&mut self, hour_start: f64, threats: &[Threat]) {
        let t = tun();
        let have: Vec<Kind> = threats.iter().map(|x| x.kind).collect();
        let kinds: Vec<&ChaosDef> = t.chaos_events.iter().filter(|e| e.needs.map(|n| have.contains(&n)).unwrap_or(true)).collect();
        let total: f64 = kinds.iter().map(|k| k.weight).sum();
        let mut x = self.dir_rng.next() * total;
        let mut kind = kinds[0].id.clone();
        for k in &kinds {
            if x < k.weight {
                kind = k.id.clone();
                break;
            }
            x -= k.weight;
        }
        let at = (hour_start + 5000.0 + self.dir_rng.next() * (HOUR_MS - 15_000.0)).max(self.diff.grace_ms);
        if let Some(c) = self.chaos_s.as_mut() {
            c.kind = kind;
            c.until = hour_start + HOUR_MS;
            c.at = at;
            if c.at >= c.until {
                c.kind.clear();
                c.at = -1.0;
            }
        }
    }

    fn fx_running(&self) -> bool {
        match &self.chaos_s {
            Some(c) => c.strain > 0.0 || c.flicker > 0.0 || c.boxfast > 0.0,
            None => false,
        }
    }

    fn step_chaos(&mut self, dt: f64, threats: &mut [Threat], ev: &mut Vec<Ev>) {
        let t = tun();
        let mut ended: Vec<&'static str> = Vec::new();
        if let Some(c) = self.chaos_s.as_mut() {
            for (name, field) in [("strain", &mut c.strain), ("flicker", &mut c.flicker), ("boxfast", &mut c.boxfast)] {
                if *field > 0.0 {
                    *field = (*field - dt).max(0.0);
                    if *field == 0.0 {
                        ended.push(name);
                    }
                }
            }
        }
        for k in ended {
            ev.push(self.mk(EvType::Chaos).kind(k).phase("end"));
        }
        let (at, until, kind) = match &self.chaos_s {
            Some(c) => (c.at, c.until, c.kind.clone()),
            None => return,
        };
        if at < 0.0 || self.elapsed < at {
            return;
        }
        if self.elapsed >= until {
            if let Some(c) = self.chaos_s.as_mut() {
                c.at = -1.0;
                c.kind.clear();
            }
            return;
        }
        if !self.lock.is_empty() || self.fx_running() || self.over != Over::None {
            return;
        }
        if kind == "decoy" && (self.since_act < t.decoy_quiet_ms || self.cool > 0.0 || (self.beat != Beat::Lull && self.beat != Beat::Build)) {
            return;
        }
        self.fire_event(&kind, threats, ev);
        if let Some(c) = self.chaos_s.as_mut() {
            c.at = -1.0;
            c.kind.clear();
            c.n += 1;
        }
    }

    /// Start a chaos event now. Returns true when it produced an event.
    fn fire_event(&mut self, kind: &str, threats: &mut [Threat], ev: &mut Vec<Ev>) -> bool {
        let t = tun();
        let mut e = self.mk(EvType::Chaos).kind(kind).phase("start");
        match kind {
            "mood" => {
                if threats.is_empty() {
                    return false;
                }
                let idx = ((self.dir_rng.next() * threats.len() as f64).floor() as usize).min(threats.len() - 1);
                let th = &mut threats[idx];
                let others: Vec<Mood> = Mood::ALL.iter().copied().filter(|m| *m != th.mood).collect();
                th.mood = others[((self.dir_rng.next() * others.len() as f64).floor() as usize).min(others.len() - 1)];
                th.mood_left = between((t.mood_min_ms, t.mood_max_ms), self.dir_rng.next());
                e.id = th.id.clone();
                e.how = th.mood.name().to_string();
            }
            "wake" => {
                let found = threats.iter_mut().find(|x| x.kind == Kind::Vent && !x.active && !x.armed);
                match found {
                    Some(f) => {
                        f.wait = f.wait.min(jitter(2500.0, &mut self.dir_rng));
                        e.id = f.id.clone();
                    }
                    None => return false,
                }
            }
            "decoy" => {
                let mut types: Vec<Kind> = Vec::new();
                for dt in &t.decoy_types {
                    if threats.iter().any(|x| x.kind == *dt) {
                        types.push(*dt);
                    }
                }
                let mut tkind = Kind::Path;
                if !types.is_empty() {
                    tkind = types[((self.dir_rng.next() * types.len() as f64).floor() as usize).min(types.len() - 1)];
                }
                let side = if tkind == Kind::Vent {
                    let n = Side::ALL.len();
                    Side::ALL[((self.dir_rng.next() * n as f64).floor() as usize).min(n - 1)]
                } else if self.dir_rng.next() < 0.5 {
                    Side::L
                } else {
                    Side::R
                };
                e.to = tkind.name().to_string();
                e.side = Some(side);
                self.cool = self.cool.max(t.decoy_quiet_ms);
            }
            "flicker" => {
                let n = t.cam_rooms.len();
                let room = t.cam_rooms[((self.dir_rng.next() * n as f64).floor() as usize).min(n - 1)].clone();
                let ms = t.chaos_def("flicker").map(|d| d.ms).unwrap_or(0.0);
                if let Some(c) = self.chaos_s.as_mut() {
                    c.room = room.clone();
                    c.flicker = ms;
                }
                e.room = room;
            }
            "strain" => {
                let ms = t.chaos_def("strain").map(|d| d.ms).unwrap_or(0.0);
                if let Some(c) = self.chaos_s.as_mut() {
                    c.strain = ms;
                }
            }
            "boxfast" => {
                let ms = t.chaos_def("boxfast").map(|d| d.ms).unwrap_or(0.0);
                if let Some(c) = self.chaos_s.as_mut() {
                    c.boxfast = ms;
                }
            }
            _ => return false,
        }
        ev.push(e);
        true
    }
}

// ------------------------------------------------------------------------------------------------ the sim

/// The whole shift: the core state and the roster's threats (in pool order).
#[derive(Clone, Debug)]
pub struct MixedSim {
    pub c: Core,
    pub threats: Vec<Threat>,
}

impl MixedSim {
    /// `level_index` 0..=4: QUIET, STEADY, RESTLESS, CHAOS, ENDLESS.
    pub fn new(seed: u32, level_index: i32) -> MixedSim {
        let t = tun();
        let diff = t.shift(level_index).clone();
        let endless = diff.endless;
        let chaos = diff.chaos;
        let breather = diff.breather.unwrap_or(t.breather_ms);
        let mut dir_rng = Mulberry32::new(seed ^ 0x4449_5243);
        let th_rng_seed = seed ^ 0x5448_5254;
        let fx_seed = seed ^ 0x4658_5f31;
        let mut th_rng = Mulberry32::new(th_rng_seed);
        let mut fx_rng = Mulberry32::new(fx_seed);
        let roster = draw_roster(seed, &diff, 0);
        let plan = if endless { vec![first_endless_spike(&mut dir_rng)] } else { plan_spikes(&mut dir_rng, &diff) };
        let seal_max = if endless { t.endless_seal_max } else { t.seal_max };
        let beat_left = between(t.lull_len, dir_rng.next());
        let feint_left = between(t.lull_feint, fx_rng.next());
        let mut threats = Vec::new();
        for (k, id) in roster.iter().enumerate() {
            if let Some(def) = t.pool_entry(id) {
                threats.push(new_threat(def, &mut th_rng, &diff, 0, k));
            }
        }
        let c = Core {
            seed,
            diff: diff.clone(),
            endless,
            chaos,
            breather,
            dir_rng,
            th_rng,
            fx_rng,
            seal_max,
            hunter_speed: 1.0,
            elapsed: 0.0,
            over: Over::None,
            by: String::new(),
            cool: 0.0,
            lock: String::new(),
            act_seq: 0,
            since_act: 0.0,
            beat: Beat::Lull,
            beat_left,
            tension: 0.3,
            plan,
            spike_idx: 0,
            spike_end: 0.0,
            warned: false,
            feint_left,
            seal: seal_max,
            doors: [DoorS::default(); 2],
            vent_seal: t.vent_seal_max,
            vents: [VentS::default(); 3],
            lamp_cool: 0.0,
            wind: false,
            mon: Mon { up: false, room: t.cam_rooms[0].clone(), stay: 0.0, toggles: 0 },
            chaos_s: if chaos || endless {
                Some(ChaosS { hr: 0, at: -1.0, kind: String::new(), until: 0.0, strain: 0.0, flicker: 0.0, boxfast: 0.0, room: String::new(), n: 0 })
            } else {
                None
            },
            run: if endless { Some(RunS { gen: 0, rotate_at: t.ramp.rotate_ms, budget: t.endless_budget }) } else { None },
        };
        MixedSim { c, threats }
    }

    pub fn roster(&self) -> Vec<String> {
        self.threats.iter().map(|t| t.id.clone()).collect()
    }

    pub fn threat(&self, id: &str) -> Option<&Threat> {
        self.threats.iter().find(|t| t.id == id)
    }

    pub fn threat_mut(&mut self, id: &str) -> Option<&mut Threat> {
        self.threats.iter_mut().find(|t| t.id == id)
    }

    pub fn hour_of(&self) -> i32 {
        self.c.hour_of()
    }

    pub fn hour_reached(&self) -> i32 {
        if self.c.endless {
            self.c.hour_of()
        } else {
            self.c.hour_of().min(6)
        }
    }

    pub fn over(&self) -> Over {
        self.c.over
    }

    pub fn elapsed(&self) -> f64 {
        self.c.elapsed
    }

    pub fn next_spike(&self) -> Option<Spike> {
        self.c.plan.get(self.c.next_idx()).copied()
    }

    // ---- the setters

    pub fn set_door(&mut self, side: Side, want: bool) -> bool {
        self.c.set_door(side, want)
    }

    pub fn set_vent(&mut self, side: Side, want: bool) -> bool {
        self.c.set_vent(side, want)
    }

    pub fn light_vent(&mut self, side: Side) -> LightRes {
        self.c.light_vent(side, &mut self.threats)
    }

    pub fn set_wind(&mut self, on: bool) {
        self.c.set_wind(on, &self.threats);
    }

    pub fn set_monitor(&mut self, up: bool) {
        self.c.set_monitor(up, &self.threats);
    }

    pub fn set_room(&mut self, room: &str) {
        self.c.set_room(room);
    }

    // ---- the step

    pub fn step(&mut self, dt: f64) -> Vec<Ev> {
        let t = tun();
        let MixedSim { c, threats } = self;
        if c.over != Over::None {
            return Vec::new();
        }
        let mut ev: Vec<Ev> = Vec::new();
        c.elapsed += dt;
        c.since_act += dt;
        c.cool = (c.cool - dt).max(0.0);
        if c.elapsed >= NIGHT_MS && !c.endless {
            c.over = Over::Win;
            ev.push(c.mk(EvType::Win));
            return ev;
        }
        if c.chaos_s.is_some() {
            c.step_hours(threats, &mut ev);
        }
        c.apply_floor();
        if c.mon.up {
            c.mon.stay += dt;
        }
        c.step_doors(dt, &mut ev);
        c.step_vents(dt, &mut ev);
        c.step_director(dt, threats, &mut ev);
        c.step_feints(dt, &mut ev);
        if c.chaos_s.is_some() {
            c.step_chaos(dt, threats, &mut ev);
        }
        if c.endless {
            MixedSim::step_rotation(c, threats, &mut ev);
        }
        for i in 0..threats.len() {
            if c.over != Over::None {
                break;
            }
            let th = &mut threats[i];
            c.step_mood(th, dt, &mut ev);
            match th.kind {
                Kind::Counter => c.step_counter(th, dt, &mut ev),
                Kind::Monitor => c.step_monitor_gated(th, dt, &mut ev),
                Kind::Winder => c.step_winder(th, dt, &mut ev),
                _ => c.step_walker(th, dt, &mut ev),
            }
        }
        if c.lock.is_empty() && c.since_act > 6000.0 {
            c.scare(dt / 60_000.0);
        }
        c.apply_floor();
        let _ = t;
        ev
    }

    /// Endless: every three hours the roster is drawn again, at a moment no deadly tell is running. Whoever stays keeps her state.
    fn step_rotation(c: &mut Core, threats: &mut Vec<Threat>, ev: &mut Vec<Ev>) {
        let t = tun();
        let (rotate_at, gen_now) = match &c.run {
            Some(r) => (r.rotate_at, r.gen),
            None => return,
        };
        if c.elapsed < rotate_at || !c.lock.is_empty() {
            return;
        }
        if threats.iter().any(|x| x.armed) {
            return;
        }
        let gen = gen_now + 1;
        if let Some(r) = c.run.as_mut() {
            r.gen = gen;
            r.rotate_at += t.ramp.rotate_ms;
        }
        let nxt = draw_roster(c.seed, &c.diff, gen);
        let old_ids: Vec<String> = threats.iter().map(|x| x.id.clone()).collect();
        let removed: Vec<String> = old_ids.iter().filter(|id| !nxt.contains(id)).cloned().collect();
        let added: Vec<String> = nxt.iter().filter(|id| !old_ids.contains(id)).cloned().collect();
        let mut old: Vec<Threat> = std::mem::take(threats);
        for (k, id) in added.iter().enumerate() {
            if let Some(def) = t.pool_entry(id) {
                old.push(new_threat(def, &mut c.th_rng, &c.diff, c.mon.toggles, k));
            }
        }
        for id in &nxt {
            if let Some(pos) = old.iter().position(|x| &x.id == id) {
                threats.push(old.swap_remove(pos));
            }
        }
        let mut e = c.mk(EvType::Rotate);
        e.gen = gen;
        e.added = added;
        e.removed = removed;
        ev.push(e);
    }

    // ---- what the scene draws

    /// Who is standing in each camera room (walkers on their route, not yet at a door or vent; the vent crawler is in the vent run).
    pub fn occupancy(&self) -> Vec<(String, Vec<String>)> {
        let t = tun();
        let mut out: Vec<(String, Vec<String>)> = t.cam_rooms.iter().map(|r| (r.clone(), Vec::new())).collect();
        for th in &self.threats {
            let walks = t.walker_types.contains(&th.kind) || th.kind == Kind::Vent;
            if walks && th.active && !th.armed && !th.route.is_empty() {
                if let Some(room) = th.route.get(th.i) {
                    if let Some(slot) = out.iter_mut().find(|(r, _)| r == room) {
                        slot.1.push(th.id.clone());
                    }
                }
            }
        }
        out
    }

    // ---- admin (public, deterministic given the seed; none of it runs in a normal shift)

    pub fn admin_clear_tells(&mut self) {
        let ids: Vec<String> = self.threats.iter().filter(|t| t.armed).map(|t| t.id.clone()).collect();
        for id in ids {
            self.admin_send_home(&id);
        }
        self.c.lock.clear();
    }

    /// Send a threat home (as an answer does, without the cooldown).
    pub fn admin_send_home(&mut self, id: &str) {
        let MixedSim { c, threats } = self;
        let Some(th) = threats.iter_mut().find(|t| t.id == id) else { return };
        if th.armed && c.lock == th.id {
            c.lock.clear();
        }
        th.armed = false;
        th.tell_on = false;
        th.armed_ms = 0.0;
        th.give_ms = 0.0;
        th.pulse = 0.0;
        th.active = false;
        th.present = false;
        th.steps = 0;
        th.pend = 0;
        th.i = 0;
        th.seen = 0.0;
        th.dwell = 0.0;
        th.side = None;
        th.flinch = false;
        th.rewound = false;
        th.wait = 1500.0;
        if th.kind == Kind::Counter {
            th.active = true;
            th.c = 0.0;
        }
    }

    /// Put a threat in a place: a camera room (a walker on her route there), a door or a vent opening (at it, her tell begins).
    pub fn admin_place(&mut self, id: &str, place: &str) -> bool {
        if self.threat(id).is_none() {
            return false;
        }
        self.admin_clear_tells();
        let t = tun();
        let MixedSim { c, threats } = self;
        let Some(th) = threats.iter_mut().find(|x| x.id == id) else { return false };
        let kind = th.kind;
        let gate = gate_side(place);
        if kind == Kind::Vent {
            let want = if gate.is_some() && place.starts_with("vent") { gate } else { Some(Side::L) };
            for r in t.vent_routes.iter() {
                if Some(r.side) == want {
                    th.route_id = r.id.clone();
                    th.route = r.rooms.clone();
                    th.start = 0;
                    th.active = true;
                    th.seen = 0.0;
                    th.dwell = 0.0;
                    th.flinch = false;
                    th.wait = jitter(t.timing_of(Kind::Vent).step_ms, &mut c.th_rng);
                    th.i = if place.starts_with("vent") { r.rooms.len() - 2 } else { 1 };
                }
            }
            if place.starts_with("vent") && gate.is_some() {
                th.i += 1;
                c.arm(th, gate);
            }
            return true;
        }
        if kind == Kind::Counter {
            return false;
        }
        if kind == Kind::Monitor {
            let side = if place.starts_with("door") { door_side(place) } else { Some(Side::L) };
            th.present = true;
            th.active = true;
            th.side = side;
            th.steps = 0;
            th.pend = 0;
            th.idle = 0.0;
            th.decay = 0.0;
            th.give_ms = 0.0;
            if place.starts_with("door") {
                c.arm(th, side);
            }
            return true;
        }
        for r in t.routes.iter() {
            let has_place = r.rooms.iter().any(|x| x == place);
            if has_place && (kind != Kind::Winder || r.rooms.contains(&th.box_room)) {
                th.route_id = r.id.clone();
                th.route = r.rooms.clone();
                th.start = 0;
                th.active = true;
                th.seen = 0.0;
                th.dwell = 0.0;
                th.flinch = false;
                th.wait = jitter(t.timing_of(kind).step_ms, &mut c.th_rng);
                th.i = r.rooms.iter().position(|x| x == place).unwrap_or(0);
                if gate.is_some() {
                    c.arm(th, gate);
                } else {
                    th.dwell = 0.0;
                }
                return true;
            }
        }
        false
    }

    /// Add a threat of the pool to tonight's roster (the roster keeps pool order).
    pub fn admin_add_threat(&mut self, id: &str) -> bool {
        let t = tun();
        if self.threats.iter().any(|x| x.id == id) {
            return false;
        }
        let Some(def) = t.pool_entry(id) else { return false };
        let fresh = new_threat(def, &mut self.c.th_rng, &self.c.diff, self.c.mon.toggles, self.threats.len());
        self.threats.push(fresh);
        let order: Vec<&str> = t.pool.iter().map(|p| p.id.as_str()).collect();
        self.threats.sort_by_key(|x| order.iter().position(|o| *o == x.id).unwrap_or(99));
        true
    }

    /// Force a chaos event now (CHAOS and ENDLESS run the director's; in any other shift the state is made on the spot).
    pub fn admin_force_chaos(&mut self, kind: &str) -> bool {
        if tun().chaos_def(kind).is_none() {
            return false;
        }
        if self.c.chaos_s.is_none() {
            self.c.chaos_s = Some(ChaosS { hr: self.c.hour_of(), at: -1.0, kind: String::new(), until: 0.0, strain: 0.0, flicker: 0.0, boxfast: 0.0, room: String::new(), n: 0 });
        }
        let mut ev = Vec::new();
        let MixedSim { c, threats } = self;
        c.fire_event(kind, threats, &mut ev)
    }

    /// The next spike starts at the next step (the director's own rules still apply: never while a deadly tell runs).
    pub fn admin_force_spike(&mut self) -> bool {
        let idx = self.c.next_idx();
        let now = self.c.elapsed;
        if idx >= self.c.plan.len() {
            self.c.plan.push(Spike { at: now, len: 11_000.0 });
            return true;
        }
        self.c.plan[idx].at = now;
        true
    }

    pub fn admin_set_hour(&mut self, hour: i32) {
        self.c.elapsed = hour as f64 * HOUR_MS;
        if let Some(ch) = self.c.chaos_s.as_mut() {
            ch.hr = hour;
        }
        if self.c.endless {
            let t = tun();
            if let Some(r) = self.c.run.as_mut() {
                r.budget = t.endless_budget_at(hour);
                r.rotate_at = ((self.c.elapsed / t.ramp.rotate_ms).floor() + 1.0) * t.ramp.rotate_ms;
            }
        }
    }

    /// Endless: draw the roster again at the next step (when no deadly tell is running).
    pub fn admin_redraw_roster(&mut self) -> bool {
        if !self.c.endless {
            return false;
        }
        let now = self.c.elapsed;
        if let Some(r) = self.c.run.as_mut() {
            r.rotate_at = now;
        }
        true
    }

    /// Whole-sim digest for determinism checks.
    pub fn digest(&self) -> u64 {
        let c = &self.c;
        let mut h = Fnv::new();
        h.f64(c.elapsed).i32(c.over as i32).f64(c.seal).f64(c.vent_seal).f64(c.tension).i32(c.beat.idx() as i32).i32(c.act_seq).str(&c.lock);
        for th in &self.threats {
            h.str(&th.id).i32(th.active as i32).i32(th.armed as i32).i32(th.i as i32).f64(th.tune).f64(th.c).f64(th.wait).i32(th.mood.idx() as i32);
        }
        h.finish()
    }
}

// ------------------------------------------------------------------------------------------------ autoplay

/// A seeded stand-in for a player: answers whatever is armed, glances at the cameras, winds the music box. Used by the
/// headless run behind `--check` and by tests. Deterministic: same seed and same sim give the same inputs.
#[derive(Clone, Debug)]
pub struct Auto {
    rng: Mulberry32,
    since: f64,
}

impl Auto {
    pub fn new(seed: u32) -> Auto {
        Auto { rng: Mulberry32::new(seed ^ 0x4155_5431), since: 0.0 }
    }

    pub fn drive(&mut self, sim: &mut MixedSim, dt: f64) {
        self.since += dt;
        if self.since < 250.0 {
            return;
        }
        self.since -= 250.0;
        let armed: Option<(Kind, Option<Side>)> = sim.threats.iter().find(|t| t.armed).map(|t| (t.kind, t.side));
        let mut want_door = [false; 2];
        let mut want_vent = [false; 3];
        let mut counter_armed = false;
        if let Some((kind, side)) = armed {
            match (kind, side) {
                (Kind::Vent, Some(s)) => want_vent[s.idx()] = true,
                (Kind::Counter, _) => counter_armed = true,
                (_, Some(s)) if s != Side::T => want_door[s.idx()] = true,
                _ => {}
            }
        }
        if counter_armed && sim.c.mon.up {
            sim.set_monitor(false);
        }
        sim.set_door(Side::L, want_door[0]);
        sim.set_door(Side::R, want_door[1]);
        for s in Side::ALL {
            sim.set_vent(s, want_vent[s.idx()]);
        }
        let r = self.rng.next();
        let r2 = self.rng.next();
        // a glance at the cameras now and then (never while a counter is the danger)
        if armed.is_none() {
            let winder: Option<(String, bool)> = sim.threats.iter().find(|t| t.kind == Kind::Winder).map(|t| (t.box_room.clone(), t.tune < 0.5 || t.stopped));
            match winder {
                Some((room, true)) => {
                    sim.set_monitor(true);
                    sim.set_room(&room);
                    sim.set_wind(true);
                }
                _ => {
                    sim.set_wind(false);
                    if sim.c.mon.up && r < 0.35 {
                        sim.set_monitor(false);
                    } else if !sim.c.mon.up && r < 0.15 {
                        sim.set_monitor(true);
                    }
                    if sim.c.mon.up && r2 < 0.5 {
                        let rooms = &tun().cam_rooms;
                        let pick = rooms[((self.rng.next() * rooms.len() as f64).floor() as usize).min(rooms.len() - 1)].clone();
                        sim.set_room(&pick);
                    }
                }
            }
            // flash the vent light when the crawler is on her way
            if sim.threats.iter().any(|t| t.kind == Kind::Vent && t.active && !t.armed) && r2 > 0.8 {
                let side = Side::ALL[((r * 3.0).floor() as usize).min(2)];
                sim.light_vent(side);
            }
        }
    }
}

/// The window-less half of the mode: the sim fed by the seeded autoplayer.
pub struct MixedHeadless {
    pub sim: MixedSim,
    auto: Auto,
}

impl MixedHeadless {
    pub fn new(seed: u32, level: i32) -> MixedHeadless {
        MixedHeadless { sim: MixedSim::new(seed, level - 1), auto: Auto::new(seed) }
    }
}

impl Headless for MixedHeadless {
    fn id(&self) -> &'static str {
        "mixed"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        self.auto.drive(&mut self.sim, dt_ms);
        self.sim.step(dt_ms).iter().map(|e| if e.id.is_empty() { e.ty.name().to_string() } else { format!("{}:{}", e.ty.name(), e.id) }).collect()
    }

    fn outcome(&self) -> Outcome {
        match self.sim.c.over {
            Over::None => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    fn elapsed_ms(&self) -> f64 {
        self.sim.c.elapsed
    }

    fn digest(&self) -> u64 {
        self.sim.digest()
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn run_names(seed: u32, level_index: i32, ms: f64) -> Vec<String> {
        let mut h = MixedHeadless::new(seed, level_index + 1);
        let mut out = Vec::new();
        let mut t = 0.0;
        while t < ms && h.outcome() == Outcome::Running {
            out.extend(h.step_ms(100.0));
            t += 100.0;
        }
        out
    }

    /// A sim with one chosen threat only, past the grace period, no player input.
    fn solo(seed: u32, level_index: i32, id: &str) -> MixedSim {
        let mut sim = MixedSim::new(seed, level_index);
        sim.threats.clear();
        assert!(sim.admin_add_threat(id));
        sim.admin_set_hour(3);
        sim
    }

    #[test]
    fn rosters_are_deterministic_and_follow_the_type_rules() {
        let t = tun();
        for seed in 0..60u32 {
            for lvl in 0..5 {
                let d = t.shift(lvl);
                let a = draw_roster(seed, d, 0);
                let b = draw_roster(seed, d, 0);
                assert_eq!(a, b);
                assert!(a.len() >= d.must.len().max(1) && a.len() <= 7, "seed {seed} lvl {lvl} {a:?}");
                for m in &d.must {
                    assert!(a.contains(m), "{m} must be in {a:?}");
                }
                let kinds: Vec<Kind> = a.iter().map(|x| t.kind_of(x).unwrap()).collect();
                let newcomers = kinds.iter().filter(|k| t.new_types.contains(k)).count();
                let walkers = kinds.iter().filter(|k| t.walker_types.contains(k)).count();
                if d.must.is_empty() {
                    assert!(newcomers <= d.new_max as usize, "seed {seed} lvl {lvl} {a:?}");
                }
                assert!(walkers <= t.max_walkers + d.must.len(), "{a:?}");
                // pool order
                let order: Vec<usize> = a.iter().map(|x| t.pool.iter().position(|p| &p.id == x).unwrap()).collect();
                let mut sorted = order.clone();
                sorted.sort();
                assert_eq!(order, sorted);
            }
        }
    }

    #[test]
    fn chaos_draws_the_biggest_roster_with_four_types() {
        let t = tun();
        for seed in 0..40u32 {
            let a = draw_roster(seed, t.shift(3), 0);
            assert!(a.len() >= 6, "{a:?}");
            let mut kinds: Vec<Kind> = a.iter().map(|x| t.kind_of(x).unwrap()).collect();
            kinds.sort_by_key(|k| k.idx());
            kinds.dedup();
            assert!(kinds.len() >= 4, "{a:?}");
        }
    }

    #[test]
    fn endless_rosters_change_with_the_generation() {
        let t = tun();
        let a = draw_roster(7, t.shift(4), 0);
        let b = draw_roster(7, t.shift(4), 3);
        assert!(!a.is_empty() && !b.is_empty());
        assert_eq!(draw_roster(7, t.shift(4), 3), b);
    }

    #[test]
    fn tell_is_never_below_the_floor_or_the_preset_minimum() {
        for m in Mood::ALL {
            for u in [0.0, 0.5, 1.0] {
                assert!(tell_for(1650.0, 0.0, m, u) >= 1500.0);
                assert!(tell_for(3600.0, 3000.0, m, u) >= 3000.0);
            }
        }
        assert_eq!(tell_for(1900.0, 0.0, Mood::Patient, 0.5), 1900.0);
    }

    #[test]
    fn spikes_are_planned_inside_the_night() {
        let t = tun();
        for seed in 0..30u32 {
            for lvl in 0..4 {
                let mut r = Mulberry32::new(seed);
                let plan = plan_spikes(&mut r, t.shift(lvl));
                assert!(plan.len() as i32 >= t.shift(lvl).spikes.0 && plan.len() as i32 <= t.shift(lvl).spikes.1);
                let mut last = 0.0;
                for s in &plan {
                    assert!(s.at >= t.spike_first_ms + 8000.0, "never in the first hour");
                    assert!(s.at + s.len <= NIGHT_MS - t.end_clear_ms + 1.0, "ends before dawn: {s:?}");
                    assert!(s.at > last);
                    last = s.at + s.len;
                }
            }
        }
    }

    #[test]
    fn the_door_seal_drains_while_shut_and_comes_back_open() {
        let mut sim = MixedSim::new(3, 1);
        sim.threats.clear();
        let full = sim.c.seal;
        assert!(sim.set_door(Side::L, true));
        for _ in 0..20 {
            sim.step(100.0);
        }
        assert!(sim.c.seal < full - 1600.0, "2 s of a shut door (net of the open door regen) drained {}", full - sim.c.seal);
        let low = sim.c.seal;
        sim.set_door(Side::L, false);
        for _ in 0..20 {
            sim.step(100.0);
        }
        assert!(sim.c.seal > low, "two open doors recover the seal");
        assert!(sim.c.seal <= sim.c.seal_max);
    }

    #[test]
    fn a_door_pops_after_the_hold_time() {
        let mut sim = MixedSim::new(3, 1);
        sim.threats.clear();
        sim.set_door(Side::R, true);
        let mut popped = false;
        for _ in 0..90 {
            let ev = sim.step(100.0);
            if ev.iter().any(|e| e.ty == EvType::Strain) {
                popped = true;
                break;
            }
        }
        assert!(popped, "8 s of holding shut pops the door");
        assert!(!sim.c.doors[1].shut);
        assert!(!sim.c.doors[1].want);
    }

    #[test]
    fn a_vent_seal_holds_then_lingers_then_opens() {
        let mut sim = MixedSim::new(3, 1);
        sim.threats.clear();
        assert!(sim.set_vent(Side::T, true));
        sim.step(100.0);
        assert!(sim.c.vents[2].shut);
        assert!(sim.c.vent_seal < tun().vent_seal_max);
        sim.set_vent(Side::T, false);
        assert!(sim.c.vents[2].shut, "stays shut while it lingers");
        let mut opened = false;
        for _ in 0..40 {
            if sim.step(100.0).iter().any(|e| e.ty == EvType::VentOpen) {
                opened = true;
                break;
            }
        }
        assert!(opened);
        assert!(!sim.c.vents[2].shut);
    }

    #[test]
    fn no_kill_before_the_tell_floor_even_with_a_tiny_tell() {
        for seed in [1u32, 2, 3, 4] {
            let mut sim = solo(seed, 1, "dovelle");
            assert!(sim.admin_place("dovelle", "doorL"));
            sim.threats[0].tell_need = 100.0;
            let mut tell_at = None;
            let mut killed_at = None;
            for _ in 0..200 {
                let ev = sim.step(100.0);
                for e in &ev {
                    if e.ty == EvType::Tell && tell_at.is_none() {
                        tell_at = Some(e.t);
                    }
                    if e.ty == EvType::Kill {
                        killed_at = Some(e.t);
                    }
                }
                if killed_at.is_some() {
                    break;
                }
            }
            let (a, k) = (tell_at.expect("a tell"), killed_at.expect("a kill"));
            assert!(k - a >= 1500.0, "seed {seed}: the kill came {} ms after the tell", k - a);
            assert_eq!(sim.c.over, Over::Caught);
            assert_eq!(sim.c.by, "dovelle");
        }
    }

    #[test]
    fn a_shut_door_pauses_the_tell_and_sends_her_away() {
        let mut sim = solo(5, 1, "ravelle");
        assert!(sim.admin_place("ravelle", "doorR"));
        assert!(sim.set_door(Side::R, true));
        let mut answered = false;
        for _ in 0..60 {
            let ev = sim.step(100.0);
            assert!(sim.threats[0].armed_ms == 0.0 || !sim.threats[0].armed, "the tell does not build at a shut door");
            if ev.iter().any(|e| e.ty == EvType::Answered && e.how == "door") {
                answered = true;
                break;
            }
        }
        assert!(answered, "she gives up after 1.2 s of shut door");
        assert!(!sim.threats[0].armed && !sim.threats[0].active);
        assert!(sim.c.lock.is_empty());
        assert_eq!(sim.threats[0].mem[Side::R.idx()], 1, "she remembers the side");
    }

    #[test]
    fn a_watched_one_waits_while_a_camera_is_on_her() {
        let mut sim = solo(8, 1, "ilyra");
        // she is on her route, in her first room, with a long wait
        let room = {
            let th = &mut sim.threats[0];
            th.route = tun().routes[0].rooms.clone();
            th.route_id = tun().routes[0].id.clone();
            th.active = true;
            th.i = 0;
            th.wait = 100.0;
            th.dwell = 5000.0;
            th.route[0].clone()
        };
        sim.set_monitor(true);
        sim.set_room(&room);
        for _ in 0..50 {
            sim.step(100.0);
        }
        assert_eq!(sim.threats[0].i, 0, "she does not move while watched");
        sim.set_monitor(false);
        let mut moved = false;
        for _ in 0..400 {
            sim.step(100.0);
            if sim.threats[0].i > 0 || sim.threats[0].armed {
                moved = true;
                break;
            }
        }
        assert!(moved, "she moves when nobody looks");
    }

    #[test]
    fn the_monitor_gated_one_steps_closer_with_each_toggle() {
        let mut sim = solo(9, 1, "opalie");
        assert!(sim.admin_place("opalie", "office"));
        sim.threats[0].idle_max = 16_000.0; // admin_place leaves idleMax at 0 (as in Godot), which would send her away at once
        for i in 0..3 {
            sim.set_monitor(true);
            sim.step(100.0);
            sim.step(1300.0);
            sim.set_monitor(false);
            sim.step(100.0);
            sim.step(1300.0);
            assert!(sim.threats[0].steps >= 1, "toggle {i}: steps {}", sim.threats[0].steps);
            sim.c.cool = 0.0;
        }
        sim.c.cool = 0.0;
        sim.set_monitor(true);
        let mut armed = false;
        for _ in 0..40 {
            sim.c.cool = 0.0;
            sim.step(100.0);
            if sim.threats[0].armed {
                armed = true;
                break;
            }
        }
        assert!(armed || sim.threats[0].steps >= 2, "enough touches arm her (steps {})", sim.threats[0].steps);
    }

    #[test]
    fn the_counter_builds_while_you_stay_and_kills_if_you_do_not_leave() {
        let mut sim = solo(11, 1, "haelith");
        sim.set_monitor(true);
        let mut armed_at = None;
        let mut over = false;
        for _ in 0..3000 {
            let ev = sim.step(100.0);
            for e in &ev {
                if e.ty == EvType::Arrive && armed_at.is_none() {
                    armed_at = Some(e.t);
                }
            }
            if sim.c.over == Over::Caught {
                over = true;
                break;
            }
        }
        assert!(armed_at.is_some(), "she arms");
        assert!(over, "staying on one camera forever is fatal");
        assert_eq!(sim.c.by, "haelith");
    }

    #[test]
    fn the_counter_is_answered_by_lowering_the_monitor() {
        let mut sim = solo(11, 1, "haelith");
        sim.set_monitor(true);
        let mut answered = false;
        for _ in 0..3000 {
            let ev = sim.step(100.0);
            if sim.threats[0].armed {
                sim.set_monitor(false);
            }
            if ev.iter().any(|e| e.ty == EvType::Answered) {
                answered = true;
                break;
            }
        }
        assert!(answered);
        assert!(!sim.threats[0].armed);
    }

    #[test]
    fn the_music_box_runs_down_without_winding_and_recovers_with_it() {
        let mut sim = solo(13, 1, "glyra");
        sim.admin_set_hour(0);
        sim.c.elapsed = 0.0;
        let before = sim.threats[0].tune;
        for _ in 0..100 {
            sim.step(100.0);
        }
        let after = sim.threats[0].tune;
        assert!(after < before, "{before} -> {after}");
        let room = sim.threats[0].box_room.clone();
        sim.set_monitor(true);
        sim.set_room(&room);
        sim.set_wind(true);
        for _ in 0..100 {
            sim.step(100.0);
        }
        assert!(sim.threats[0].tune > after, "winding raises the tune");
    }

    #[test]
    fn winding_jumps_the_monitor_to_the_boxs_camera() {
        let mut sim = solo(13, 1, "glyra");
        let room = sim.threats[0].box_room.clone();
        sim.set_monitor(true);
        let other = tun().cam_rooms.iter().find(|r| **r != room).unwrap().clone();
        sim.set_room(&other);
        sim.set_wind(true);
        assert_eq!(sim.c.mon.room, room);
    }

    #[test]
    fn the_vent_light_makes_a_crawler_near_the_vent_back_off() {
        let mut sim = solo(17, 1, "verrine");
        assert!(sim.admin_place("verrine", "ventL"));
        assert!(sim.threats[0].armed);
        let res = sim.light_vent(Side::L);
        assert!(res.ok);
        assert_eq!(res.seen, vec!["verrine".to_string()]);
        sim.c.cool = 0.0;
        sim.c.beat = Beat::Lull;
        let ev = sim.step(100.0);
        assert!(ev.iter().any(|e| e.ty == EvType::Answered && e.how == "light"));
        let again = sim.light_vent(Side::L);
        assert!(!again.ok && again.why == "cool");
    }

    #[test]
    fn a_sealed_vent_refuses_the_light() {
        let mut sim = solo(17, 1, "verrine");
        sim.set_vent(Side::R, true);
        assert_eq!(sim.light_vent(Side::R).why, "shut");
    }

    #[test]
    fn the_same_seed_gives_the_same_events() {
        let a = run_names(42, 1, 120_000.0);
        let b = run_names(42, 1, 120_000.0);
        assert!(!a.is_empty());
        assert_eq!(a, b);
        let c = run_names(43, 1, 120_000.0);
        assert_ne!(a, c, "another seed plays another night");
    }

    #[test]
    fn a_seeded_night_reaches_an_outcome_for_the_four_scored_shifts() {
        for lvl in 1..=4 {
            let mut h = MixedHeadless::new(12345, lvl);
            let mut t = 0.0;
            while t < 271_000.0 && h.outcome() == Outcome::Running {
                h.step_ms(100.0);
                t += 100.0;
            }
            assert_ne!(h.outcome(), Outcome::Running, "level {lvl} ended at {t}");
            let mut g = MixedHeadless::new(12345, lvl);
            let mut u = 0.0;
            while u < 271_000.0 && g.outcome() == Outcome::Running {
                g.step_ms(100.0);
                u += 100.0;
            }
            assert_eq!(h.digest(), g.digest(), "level {lvl} is deterministic");
            assert_eq!(h.outcome(), g.outcome());
        }
    }

    #[test]
    fn endless_has_no_dawn_and_rotates_its_roster() {
        let mut h = MixedHeadless::new(12345, 5);
        let mut rotated = false;
        let mut t = 0.0;
        while t < 400_000.0 && h.outcome() == Outcome::Running {
            for e in h.step_ms(100.0) {
                if e.starts_with("rotate") {
                    rotated = true;
                }
            }
            t += 100.0;
        }
        if h.outcome() == Outcome::Running {
            assert!(t >= 400_000.0);
            assert!(rotated, "after 135 s the roster is drawn again");
            assert!(h.sim.hour_reached() >= 8);
        }
    }

    #[test]
    fn an_empty_quiet_shift_is_won_at_dawn() {
        let mut sim = MixedSim::new(9, 0);
        sim.threats.clear();
        let mut t = 0.0;
        while t < NIGHT_MS + 2_000.0 && sim.c.over == Over::None {
            sim.step(500.0);
            t += 500.0;
        }
        assert_eq!(sim.c.over, Over::Win, "an empty roster survives to dawn");
    }

    #[test]
    fn quiet_has_no_threat_before_its_grace() {
        let mut sim = MixedSim::new(5, 0);
        for _ in 0..800 {
            let ev = sim.step(100.0);
            assert!(!ev.iter().any(|e| e.ty == EvType::Arrive || e.ty == EvType::Kill), "t {}", sim.c.elapsed);
        }
        assert!(sim.c.elapsed < sim.c.diff.grace_ms);
    }

    #[test]
    fn admin_places_and_clears_threats() {
        let mut sim = solo(21, 1, "dovelle");
        assert!(sim.admin_place("dovelle", "westhall"));
        assert_eq!(sim.threats[0].room(), Some("westhall"));
        assert!(!sim.threats[0].armed);
        assert!(sim.admin_place("dovelle", "doorR"));
        assert!(sim.threats[0].armed && sim.c.lock == "dovelle");
        sim.admin_clear_tells();
        assert!(!sim.threats[0].armed && sim.c.lock.is_empty());
        assert!(!sim.admin_place("nobody", "doorL"));
        assert!(sim.admin_force_spike());
        assert!(sim.admin_force_chaos("strain"));
        assert!(sim.c.chaos_s.as_ref().unwrap().strain > 0.0);
    }

    #[test]
    fn the_occupancy_lists_walkers_by_room() {
        let mut sim = solo(21, 1, "aki");
        assert!(sim.admin_place("aki", "dining"));
        let occ = sim.occupancy();
        let dining = occ.iter().find(|(r, _)| r == "dining").unwrap();
        assert_eq!(dining.1, vec!["aki".to_string()]);
        assert_eq!(occ.len(), 9);
    }

    #[test]
    fn the_director_starts_a_spike_when_forced_and_ends_it() {
        let mut sim = solo(23, 1, "ilyra");
        sim.threats[0].wait = 1.0e9;
        assert!(sim.admin_force_spike());
        let mut started = false;
        let mut ended = false;
        for _ in 0..400 {
            for e in sim.step(100.0) {
                if e.ty == EvType::Spike && e.phase == "start" {
                    started = true;
                }
                if e.ty == EvType::Spike && e.phase == "end" {
                    ended = true;
                }
            }
        }
        assert!(started && ended);
    }

    #[test]
    fn chaos_fires_an_event_in_its_hours() {
        let mut sim = MixedSim::new(31, 3);
        for t in sim.threats.iter_mut() {
            t.wait = 1.0e9;
        }
        let mut fired = false;
        for _ in 0..2500 {
            for e in sim.step(100.0) {
                if e.ty == EvType::Chaos && e.phase == "start" {
                    fired = true;
                }
            }
            if sim.c.over != Over::None || fired {
                break;
            }
        }
        assert!(fired || sim.c.over != Over::None, "CHAOS schedules an event in hours 1 to 5");
    }
}
