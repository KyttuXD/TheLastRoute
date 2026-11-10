//! Dead Air's rules (port of lure_logic.gd: same numbers, same draws in the same order). Pure: no drawing, no GL, no window.
//!
//! One real hunter (Xivvy) crawls through the building toward the booth's vent openings. The other three (Ferryn, Briselle,
//! Mavrine) are phantoms: harmless, but each knocks out a system now and then. Climate (Solyth and Wynelle): the building has
//! a temperature that drifts down while Wynelle is in (up when only Solyth is). Below 60 F Wynelle slowly freezes the booth;
//! from 95 F Solyth slowly cooks it. A heater and a fan fix the number, strain the air while they run, and Mavrine breaks them.
//!
//! * Lure: play a sound in a camera room and Xivvy heads there instead (needs Audio; a cooldown).
//! * Seals: close a vent opening to turn her away. Sealed vents strain the air, which builds stress.
//! * Reboot: a broken system takes six seconds to bring back, and is offline meanwhile.
//!
//! Stress at 100 and the air gives out. Xivvy in an open vent for 5.2 s and she gets in.
//! `Sim::step(dt_ms, watch)` returns the events. `watch` is the room the monitor shows ("" when it shows nothing).

#![allow(dead_code)]

use super::data::*;
use crate::core::{effective_level, hour_index, Fnv, Pacer, Session, Stream, NIGHT_MS};
use std::collections::{BTreeMap, HashMap};

/// What a step reports. The mode turns these into sounds, messages and hunter walks.
#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    /// Xivvy changed room.
    Moved { room: String, from: String },
    /// Xivvy reached a vent opening of the booth.
    AtOffice { room: String, from: String },
    /// A sealed vent turned her away.
    Retreat { side: char },
    /// A phantom shows on a camera.
    Phantom { id: String, room: String },
    /// A system went down (`id` is who took it; "strain" for the lure's strain on the speakers).
    Broken { system: String, id: String },
    Fixed { system: String },
    Caught { by: String },
    Win,
}

impl SimEvent {
    /// A short name for logs and the headless event list.
    pub fn name(&self) -> String {
        match self {
            SimEvent::Moved { room, .. } => format!("moved:{room}"),
            SimEvent::AtOffice { room, .. } => format!("atOffice:{room}"),
            SimEvent::Retreat { side } => format!("retreat:{side}"),
            SimEvent::Phantom { id, room } => format!("phantom:{id}:{room}"),
            SimEvent::Broken { system, id } => format!("broken:{system}:{id}"),
            SimEvent::Fixed { system } => format!("fixed:{system}"),
            SimEvent::Caught { by } => format!("caught:{by}"),
            SimEvent::Win => "win".to_string(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    None,
    Win,
    Caught,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SysState {
    Ok,
    Broken,
    Reboot,
}

#[derive(Clone, Copy, Debug)]
pub struct System {
    pub state: SysState,
    pub t: f64,
}

/// The air's rates for a game (per second).
#[derive(Clone, Copy, Debug)]
pub struct Air {
    pub seal: f64,
    pub broken: f64,
    pub appliance: f64,
    pub decay: f64,
}

/// The air's rates: the story nights' or the Custom Night's, with Mavrine's level for the slider.
pub fn air_rates(is_custom: bool, mavrine_level: i32) -> Air {
    let t = tuning();
    if is_custom {
        let c = &t.custom_air;
        Air {
            seal: c.seal_sec,
            broken: c.broken_sec + c.broken_per_level * (mavrine_level as f64).clamp(0.0, 20.0),
            appliance: c.appliance_stress,
            decay: c.decay_sec,
        }
    } else {
        Air { seal: t.t.stress_per_seal_sec, broken: t.t.stress_broken_sec, appliance: t.climate.appliance_stress, decay: t.t.stress_decay_sec }
    }
}

#[derive(Clone, Debug)]
pub struct Killer {
    pub room: String,
    pub pacer: Pacer,
    /// 'L' or 'R': the vent she is heading for.
    pub side: char,
    pub at_ms: f64,
    pub target: String,
    pub lure_left: f64,
    pub tempo: f64,
    pub dist: HashMap<String, i32>,
}

#[derive(Clone, Debug)]
pub struct PhantomAi {
    pub id: String,
    pub pacer: Pacer,
    pub cool: f64,
    pub tempo: f64,
}

/// A phantom drawn on a camera.
#[derive(Clone, Debug, PartialEq)]
pub struct Phantom {
    pub id: String,
    pub room: String,
    pub t: f64,
}

/// Options for a night (lure_logic.gd `_init(opts)`).
#[derive(Clone, Debug)]
pub struct SimOpts {
    /// 1..=6.
    pub night: i32,
    pub seed: u32,
    /// None = the night's level for Xivvy.
    pub level: Option<i32>,
    pub phantom_levels: Option<BTreeMap<String, i32>>,
    pub climate_levels: Option<BTreeMap<String, i32>>,
    pub phantoms: bool,
    pub focus: f64,
    pub infinite_stress: bool,
    pub custom: bool,
}

impl SimOpts {
    pub fn night(night: i32, seed: u32) -> SimOpts {
        SimOpts { night, seed, level: None, phantom_levels: None, climate_levels: None, phantoms: true, focus: 0.8, infinite_stress: false, custom: false }
    }

    /// A Custom Night plays at Night 5's tables with the sliders as the levels.
    pub fn from_session(s: &Session) -> SimOpts {
        let seed = s.seed_value;
        if s.is_custom {
            let c = s.custom_levels.clone();
            SimOpts {
                night: 5,
                seed,
                level: Some(c.get("xivvy").copied().unwrap_or(0)),
                phantom_levels: Some(c.clone()),
                climate_levels: Some(c),
                phantoms: true,
                focus: 0.8,
                infinite_stress: false,
                custom: true,
            }
        } else {
            SimOpts::night(s.night.clamp(1, 6), seed)
        }
    }
}

pub struct Sim {
    /// 0-based night.
    pub n: usize,
    pub rng: Stream,
    pub vr: Stream,
    pub lvl: i32,
    pub phantoms_on: bool,
    pub focus: f64,
    pub infinite_stress: bool,
    pub is_custom: bool,
    pub phantom_override: Option<BTreeMap<String, i32>>,
    pub solyth: i32,
    pub wynelle: i32,
    pub air: Air,
    pub elapsed: f64,
    pub over: Over,
    pub by: String,
    pub stress: f64,
    pub killer: Killer,
    pub systems: Vec<(String, System)>,
    pub temp: f64,
    pub drift_x: f64,
    pub heater_on: bool,
    pub fan_on: bool,
    pub cold: f64,
    pub heat: f64,
    /// [left, right]
    pub seals: [bool; 2],
    pub lure_cool: f64,
    pub lures: i32,
    pub pending: Vec<SimEvent>,
    pub phantom: Option<Phantom>,
    pub phantom_ai: Vec<PhantomAi>,
    pub has_climate: bool,
    pub has_fan: bool,
    /// Admin: whoever would have ended the night is sent back.
    pub god: bool,
    /// Admin: the hunters' wait clocks run at this multiple.
    pub hunter_speed: f64,
    dist_l: HashMap<String, i32>,
    dist_r: HashMap<String, i32>,
}

fn side_index(side: char) -> usize {
    if side == 'L' {
        0
    } else {
        1
    }
}

impl Sim {
    pub fn new(opts: SimOpts) -> Sim {
        let t = tuning();
        let n = (opts.night.clamp(1, 6) - 1) as usize;
        let mut rng = Stream::seeded(opts.seed);
        // the variety stream comes from the state BEFORE the night's first draw
        let mut vr = Stream::variety_of(&rng, None);
        let lvl = match opts.level {
            Some(l) if l >= 0 => l,
            _ => t.xivvy_level(n),
        };
        // the killer: her side is the first draw, her tempo the first variety draw
        let side = if rng.next() < 0.5 { 'L' } else { 'R' };
        let tempo = vr.tempo(t.variety.xivvy_tempo);
        let killer = Killer {
            room: "lobby".to_string(),
            pacer: Pacer::new(),
            side,
            at_ms: 0.0,
            target: String::new(),
            lure_left: 0.0,
            tempo,
            dist: HashMap::new(),
        };
        let systems: Vec<(String, System)> = SYSTEM_IDS.iter().map(|s| (s.to_string(), System { state: SysState::Ok, t: 0.0 })).collect();
        let mut phantom_ai = Vec::new();
        for id in PHANTOM_IDS {
            let cool = spread_with(&mut rng, t.t.phantom_first_ms, 0.2);
            let tempo = vr.tempo(t.variety.phantom_tempo);
            phantom_ai.push(PhantomAi { id: id.to_string(), pacer: Pacer::new(), cool, tempo });
        }
        let climate_of = |id: &str| -> i32 {
            match &opts.climate_levels {
                Some(m) if m.contains_key(id) => m[id],
                _ => t.climate_level(id, n),
            }
        };
        let solyth = climate_of("solyth");
        let wynelle = climate_of("wynelle");
        let has_climate = solyth > 0 || wynelle > 0;
        let has_fan = solyth > 0;
        let mut sim = Sim {
            n,
            rng,
            vr,
            lvl,
            phantoms_on: opts.phantoms,
            focus: opts.focus,
            infinite_stress: opts.infinite_stress,
            is_custom: opts.custom,
            phantom_override: opts.phantom_levels.clone(),
            solyth,
            wynelle,
            air: air_rates(opts.custom, 0),
            elapsed: 0.0,
            over: Over::None,
            by: String::new(),
            stress: 0.0,
            killer,
            systems,
            temp: t.climate.start,
            drift_x: 1.0,
            heater_on: false,
            fan_on: false,
            cold: 0.0,
            heat: 0.0,
            seals: [false, false],
            lure_cool: 0.0,
            lures: 0,
            pending: Vec::new(),
            phantom: None,
            phantom_ai,
            has_climate,
            has_fan,
            god: false,
            hunter_speed: 1.0,
            dist_l: distances_to("officeL"),
            dist_r: distances_to("officeR"),
        };
        // who wakes first: with two or more phantoms on the floor, shuffle which one makes her first visit earlier
        let awake: Vec<usize> = (0..PHANTOM_IDS.len()).filter(|i| sim.phantom_base(PHANTOM_IDS[*i]) > 0).collect();
        if sim.phantoms_on && awake.len() >= 2 && !sim.vr.neutral {
            let mut slots: Vec<f64> = (0..awake.len()).map(|i| (i as f64 - (awake.len() as f64 - 1.0) / 2.0) * t.variety.wake_step).collect();
            let mut i = slots.len();
            while i > 1 {
                i -= 1;
                let j = sim.vr.pick_index(i + 1);
                slots.swap(i, j);
            }
            for (k, idx) in awake.iter().enumerate() {
                sim.phantom_ai[*idx].pacer.skip(-slots[k] * t.t.phantom_pace);
            }
        }
        // the weather of the night
        if sim.has_climate {
            sim.temp = t.climate.start + (2.0 * sim.vr.next() - 1.0) * t.variety.start_temp_spread;
            sim.drift_x = sim.vr.tempo(t.variety.drift_spread);
        }
        sim.air = air_rates(sim.is_custom, sim.phantom_base("mavrine"));
        sim
    }

    pub fn phantom_base(&self, id: &str) -> i32 {
        if let Some(m) = &self.phantom_override {
            if let Some(v) = m.get(id) {
                return *v;
            }
        }
        tuning().phantom_level(id, self.n)
    }

    fn sys(&self, name: &str) -> Option<&System> {
        self.systems.iter().find(|(k, _)| k == name).map(|(_, s)| s)
    }

    fn sys_mut(&mut self, name: &str) -> Option<&mut System> {
        self.systems.iter_mut().find(|(k, _)| k == name).map(|(_, s)| s)
    }

    pub fn online(&self, system: &str) -> bool {
        self.sys(system).map(|s| s.state == SysState::Ok).unwrap_or(false)
    }

    pub fn state_of(&self, system: &str) -> SysState {
        self.sys(system).map(|s| s.state).unwrap_or(SysState::Ok)
    }

    pub fn reboot_t(&self, system: &str) -> f64 {
        self.sys(system).map(|s| s.t).unwrap_or(0.0)
    }

    fn goal(&self) -> &'static str {
        if self.killer.side == 'L' {
            "officeL"
        } else {
            "officeR"
        }
    }

    fn sealed(&self, side: char) -> bool {
        self.seals[side_index(side)]
    }

    /// Where she is heading: the lure while it lasts, otherwise her vent. Draw order as in Godot: the focus draw only when there
    /// is a closer room, then the pick.
    fn next_room(&mut self) -> String {
        let lured = self.killer.lure_left > 0.0 && !self.killer.target.is_empty();
        let target: String = if lured { self.killer.target.clone() } else { self.goal().to_string() };
        if lured && self.killer.dist.is_empty() {
            self.killer.dist = distances_to(&self.killer.target);
        }
        let room = self.killer.room.clone();
        let t = tuning();
        let mut around: Vec<String> = Vec::new();
        for r in t.neighbours(&room) {
            if r == "officeL" && (self.seals[0] || target != "officeL") {
                continue;
            }
            if r == "officeR" && (self.seals[1] || target != "officeR") {
                continue;
            }
            around.push(r);
        }
        let d: &HashMap<String, i32> = if lured {
            &self.killer.dist
        } else if self.killer.side == 'L' {
            &self.dist_l
        } else {
            &self.dist_r
        };
        let here = d.get(&room).copied().unwrap_or(999);
        let closer: Vec<String> = around.iter().filter(|r| d.get(*r).copied().unwrap_or(999) < here).cloned().collect();
        let mut pool = around;
        if !closer.is_empty() {
            let p = if self.killer.lure_left > 0.0 { 0.95 } else { self.focus };
            if self.rng.next() < p {
                pool = closer;
            }
        }
        if pool.is_empty() {
            return room;
        }
        let i = ((self.rng.next() * pool.len() as f64).floor() as usize).min(pool.len() - 1);
        pool[i].clone()
    }

    fn finish(&mut self, mut ev: Vec<SimEvent>, who: &str) -> Vec<SimEvent> {
        if self.god {
            match who {
                "xivvy" => {
                    self.killer.room = "storage".to_string();
                    self.killer.at_ms = 0.0;
                    self.killer.pacer.reset();
                }
                "mavrine" => self.stress = 40.0,
                "wynelle" => self.cold = 0.0,
                "solyth" => self.heat = 0.0,
                _ => {}
            }
            return ev;
        }
        self.over = Over::Caught;
        self.by = who.to_string();
        ev.push(SimEvent::Caught { by: who.to_string() });
        ev
    }

    fn break_system(&mut self, system: &str) {
        if let Some(s) = self.sys_mut(system) {
            *s = System { state: SysState::Broken, t: 0.0 };
        }
        if system == "heater" {
            self.heater_on = false;
        }
        if system == "fan" {
            self.fan_on = false;
        }
    }

    /// Advance by `dt_ms`. `watch` is the room the monitor shows (only when the cameras work; "" otherwise).
    pub fn step(&mut self, dt_ms: f64, watch: &str) -> Vec<SimEvent> {
        let mut ev: Vec<SimEvent> = Vec::new();
        if self.over != Over::None {
            return ev;
        }
        let t = tuning();
        let dt = dt_ms / 1000.0;
        self.elapsed += dt_ms;
        let hour = hour_index(self.elapsed);
        ev.append(&mut self.pending);
        self.lure_cool = (self.lure_cool - dt_ms).max(0.0);
        if self.killer.lure_left > 0.0 {
            self.killer.lure_left -= dt_ms;
            if self.killer.lure_left <= 0.0 {
                self.killer.target.clear();
                self.killer.dist.clear();
            }
        }

        // systems finish rebooting
        for (name, s) in self.systems.iter_mut() {
            if s.state == SysState::Reboot {
                s.t += dt_ms;
                if s.t >= t.t.reboot_ms {
                    s.state = SysState::Ok;
                    s.t = 0.0;
                    ev.push(SimEvent::Fixed { system: name.clone() });
                }
            }
        }

        // phantoms break things
        if self.phantoms_on {
            let expired = match &mut self.phantom {
                Some(p) => {
                    p.t += dt_ms;
                    p.t >= t.t.phantom_show_ms
                }
                None => false,
            };
            if expired {
                self.phantom = None;
            }
            for i in 0..self.phantom_ai.len() {
                let id = self.phantom_ai[i].id.clone();
                self.phantom_ai[i].cool = (self.phantom_ai[i].cool - dt_ms).max(0.0);
                if self.phantom_ai[i].cool > 0.0 {
                    continue;
                }
                let level = effective_level(self.phantom_base(&id), hour);
                let step = dt_ms * t.t.phantom_pace * self.phantom_ai[i].tempo * self.hunter_speed;
                let due = {
                    let rng = &mut self.rng;
                    self.phantom_ai[i].pacer.step(step, level, &mut || rng.next())
                };
                if !due {
                    continue;
                }
                let rest = (t.t.phantom_cool_ms - self.n as f64 * 800.0).max(5000.0);
                self.phantom_ai[i].cool = spread_with(&mut self.rng, rest, t.variety.phantom_rest_spread);
                self.phantom_ai[i].tempo = self.vr.tempo(t.variety.phantom_tempo);
                let system = t.system_of(&id).unwrap_or("cams").to_string();
                let room_i = ((self.rng.next() * ROOMS.len() as f64).floor() as usize).min(ROOMS.len() - 1);
                let room = ROOMS[room_i].to_string();
                self.phantom = Some(Phantom { id: id.clone(), room: room.clone(), t: 0.0 });
                let mut targets: Vec<String> = vec![system];
                if id == "mavrine" && self.has_climate {
                    if self.solyth > 0 && self.wynelle > 0 {
                        targets.push("heater".to_string());
                        targets.push("fan".to_string());
                    } else {
                        targets.push(if self.fan_on { "fan".to_string() } else { "heater".to_string() });
                    }
                }
                for (sys, chance) in t.extras(&id, self.n) {
                    let roll = self.rng.next();
                    if roll < chance && !targets.contains(&sys) {
                        targets.push(sys);
                    }
                }
                for tg in targets {
                    if self.online(&tg) {
                        self.break_system(&tg);
                        ev.push(SimEvent::Broken { system: tg, id: id.clone() });
                    }
                }
                ev.push(SimEvent::Phantom { id, room });
            }
        }

        // the climate
        if self.has_climate {
            let c = &t.climate;
            let dir = if self.wynelle > 0 { -1.0 } else { 1.0 };
            let drift = c.drift_per_sec * self.drift_x * (1.0 + hour as f64 * c.drift_per_hour) * dir;
            let mut push = 0.0;
            if self.heater_on && self.online("heater") {
                push += c.heater_per_sec;
            }
            if self.fan_on && self.online("fan") {
                push -= c.fan_per_sec;
            }
            self.temp = (self.temp + (drift + push) * dt).min(130.0).max(20.0);
            if self.wynelle > 0 && self.temp < c.cold {
                self.cold = (self.cold + (c.cold - self.temp) * c.cold_per_deg * (self.wynelle as f64 / 10.0).max(0.4) * dt).min(100.0);
            } else {
                self.cold = (self.cold - c.recover_per_sec * dt).max(0.0);
            }
            if self.solyth > 0 && self.temp >= c.hot - 5.0 {
                self.heat = (self.heat + (self.temp - (c.hot - 5.0)) * c.heat_per_deg * (self.solyth as f64 / 10.0).max(0.4) * dt).min(100.0);
            } else {
                self.heat = (self.heat - c.recover_per_sec * dt).max(0.0);
            }
            if self.cold >= 100.0 {
                return self.finish(ev, "wynelle");
            }
            if self.heat >= 100.0 {
                return self.finish(ev, "solyth");
            }
        }

        // the air
        let sealed = self.seals.iter().filter(|s| **s).count() as f64;
        let running = (if self.heater_on && self.online("heater") { 1.0 } else { 0.0 }) + (if self.fan_on && self.online("fan") { 1.0 } else { 0.0 });
        let bad = sealed * self.air.seal + (if self.online("vent") { 0.0 } else { self.air.broken }) + running * self.air.appliance;
        self.stress = (self.stress + (if bad > 0.0 { bad } else { -self.air.decay }) * dt).min(100.0).max(0.0);
        if self.stress >= 100.0 && !self.infinite_stress {
            return self.finish(ev, "mavrine");
        }

        // the hunter
        let room = self.killer.room.clone();
        if is_office(&room) {
            let side = if room == "officeL" { 'L' } else { 'R' };
            self.killer.at_ms += dt_ms;
            if self.sealed(side) {
                if self.killer.at_ms >= t.t.retreat_ms {
                    self.killer.room = "storage".to_string();
                    self.killer.at_ms = 0.0;
                    self.killer.pacer.reset();
                    self.killer.side = if self.vr.chance(t.variety.same_side_again) {
                        side
                    } else if side == 'L' {
                        'R'
                    } else {
                        'L'
                    };
                    self.killer.tempo = self.vr.tempo(t.variety.xivvy_tempo);
                    ev.push(SimEvent::Retreat { side });
                }
            } else if self.killer.at_ms >= t.t.office_wait_ms {
                return self.finish(ev, "xivvy");
            }
        } else if watch != room {
            let pace = if self.killer.lure_left > 0.0 { t.t.lure_pace } else { 1.0 };
            let level = effective_level(self.lvl, hour);
            let step = dt_ms * pace * self.killer.tempo * self.hunter_speed;
            let due = {
                let rng = &mut self.rng;
                self.killer.pacer.step(step, level, &mut || rng.next())
            };
            if due {
                self.killer.tempo = self.vr.tempo(t.variety.xivvy_tempo);
                let from = self.killer.room.clone();
                let to = self.next_room();
                self.killer.room = to.clone();
                if to != from {
                    let to_office = is_office(&to);
                    if to_office {
                        ev.push(SimEvent::AtOffice { room: to.clone(), from });
                        self.killer.at_ms = 0.0;
                    } else {
                        ev.push(SimEvent::Moved { room: to.clone(), from });
                    }
                    if self.killer.lure_left > 0.0 && to == self.killer.target {
                        self.killer.lure_left = 0.0;
                        self.killer.target.clear();
                        self.killer.dist.clear();
                    }
                }
            }
        }
        if self.elapsed >= NIGHT_MS {
            self.over = Over::Win;
            ev.push(SimEvent::Win);
        }
        ev
    }

    // --- player actions (each returns true when it did something) -----------------------------------------------------

    pub fn lure(&mut self, room: &str) -> bool {
        if self.over != Over::None || !self.online("audio") || self.lure_cool > 0.0 || !ROOMS.contains(&room) || room.starts_with("vent") {
            return false;
        }
        let t = tuning();
        self.killer.target = room.to_string();
        self.killer.dist.clear();
        self.killer.lure_left = t.t.lure_ms;
        self.lure_cool = t.t.lure_cool_ms;
        self.lures += 1;
        if t.t.lure_strain_every > 0 && self.lures % t.t.lure_strain_every == 0 {
            if let Some(s) = self.sys_mut("audio") {
                *s = System { state: SysState::Broken, t: 0.0 };
            }
            self.pending.push(SimEvent::Broken { system: "audio".to_string(), id: "strain".to_string() });
        }
        true
    }

    pub fn reboot(&mut self, system: &str) -> bool {
        if self.over != Over::None || self.state_of(system) != SysState::Broken || (system == "fan" && !self.has_fan) {
            return false;
        }
        if let Some(s) = self.sys_mut(system) {
            s.state = SysState::Reboot;
            s.t = 0.0;
            return true;
        }
        false
    }

    pub fn toggle(&mut self, name: &str) -> bool {
        if self.over != Over::None || !self.has_climate || !(name == "heater" || name == "fan") || (name == "fan" && !self.has_fan) || !self.online(name) {
            return false;
        }
        if name == "heater" {
            self.heater_on = !self.heater_on;
            if self.heater_on {
                self.fan_on = false;
            }
        } else {
            self.fan_on = !self.fan_on;
            if self.fan_on {
                self.heater_on = false;
            }
        }
        true
    }

    /// Toggle a seal (`on` None) or set it.
    pub fn seal(&mut self, side: char, on: Option<bool>) -> bool {
        if self.over != Over::None {
            return false;
        }
        let i = side_index(side);
        self.seals[i] = match on {
            Some(v) => v,
            None => !self.seals[i],
        };
        true
    }

    /// Everyone's AI level right now (with the night's step-ups), for the admin overlay.
    pub fn level_of(&self, id: &str) -> i32 {
        let hour = hour_index(self.elapsed);
        if id == "xivvy" {
            return effective_level(self.lvl, hour);
        }
        if id == "solyth" {
            return self.solyth;
        }
        if id == "wynelle" {
            return self.wynelle;
        }
        effective_level(self.phantom_base(id), hour)
    }

    /// Admin: set a hunter's AI level.
    pub fn set_level(&mut self, id: &str, value: i32) {
        if id == "xivvy" {
            self.lvl = value;
        } else if id == "solyth" || id == "wynelle" {
            if id == "solyth" {
                self.solyth = value;
            } else {
                self.wynelle = value;
            }
            self.has_climate = self.solyth > 0 || self.wynelle > 0;
            self.has_fan = self.solyth > 0;
        } else {
            self.phantom_override.get_or_insert_with(BTreeMap::new).insert(id.to_string(), value);
        }
    }

    /// Admin: Xivvy now (no cooldown, no audio needed).
    pub fn force_lure(&mut self, room: &str) {
        if ROOMS.contains(&room) && !room.starts_with("vent") {
            self.killer.target = room.to_string();
            self.killer.dist.clear();
            self.killer.lure_left = tuning().t.lure_ms;
        }
    }

    /// Admin: put Xivvy in a room (or at a vent opening).
    pub fn place_killer(&mut self, room: &str) {
        self.killer.room = room.to_string();
        self.killer.at_ms = 0.0;
        self.killer.pacer.reset();
    }

    /// Admin: a system down or back.
    pub fn set_system(&mut self, system: &str, broken: bool) {
        if self.sys(system).is_none() {
            return;
        }
        if let Some(s) = self.sys_mut(system) {
            *s = System { state: if broken { SysState::Broken } else { SysState::Ok }, t: 0.0 };
        }
        if broken && system == "heater" {
            self.heater_on = false;
        }
        if broken && system == "fan" {
            self.fan_on = false;
        }
    }

    pub fn set_appliance(&mut self, name: &str, on: bool) {
        if name == "heater" {
            self.heater_on = on;
            if on {
                self.fan_on = false;
            }
        } else if name == "fan" {
            self.fan_on = on;
            if on {
                self.heater_on = false;
            }
        }
    }

    pub fn clear_climate(&mut self) {
        self.cold = 0.0;
        self.heat = 0.0;
        self.temp = 70.0;
    }

    /// Air left, 0..100 (what the HUD shows).
    pub fn air_left(&self) -> i32 {
        (100.0 - self.stress).round() as i32
    }

    /// Who is in the booth at the last stage of the climate: (id, how far in 0..1), when over 85.
    pub fn climate_visitors(&self) -> Vec<(&'static str, f32)> {
        let mut out = Vec::new();
        for (id, amount) in [("wynelle", self.cold), ("solyth", self.heat)] {
            if amount >= 85.0 {
                out.push((id, (((amount - 85.0) / 15.0).clamp(0.0, 1.0)) as f32));
            }
        }
        out
    }

    /// FNV-1a of the key state (stable across runs).
    pub fn digest(&self) -> u64 {
        let mut h = Fnv::new();
        h.f64(self.elapsed).i32(self.over as i32).str(&self.by).f64(self.stress).f64(self.temp).str(&self.killer.room).f64(self.killer.at_ms);
        h.i32(self.lures).f64(self.cold).f64(self.heat).i32(self.seals[0] as i32 + 2 * self.seals[1] as i32);
        h.i32(self.heater_on as i32 + 2 * self.fan_on as i32).u64(self.rng.state() as u64).u64(self.vr.state() as u64);
        for (name, s) in &self.systems {
            h.str(name).i32(s.state as i32);
        }
        h.finish()
    }
}

/// ms +/- spread, evenly (the shared `spread_ms`, on a Stream).
pub fn spread_with(rng: &mut Stream, ms: f64, spread: f64) -> f64 {
    ms * (1.0 - spread + 2.0 * spread * rng.next())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet(night: i32) -> Sim {
        let mut o = SimOpts::night(night, 12345);
        o.level = Some(0);
        o.phantoms = false;
        Sim::new(o)
    }

    fn run(sim: &mut Sim, ms: f64, watch: &str) -> Vec<SimEvent> {
        let mut out = Vec::new();
        let mut left = ms;
        while left > 0.0 && sim.over == Over::None {
            out.extend(sim.step(100.0, watch));
            left -= 100.0;
        }
        out
    }

    #[test]
    fn night_one_is_a_tutorial_with_a_sleeping_hunter() {
        let s = Sim::new(SimOpts::night(1, 7));
        assert_eq!(s.lvl, 0);
        assert!(!s.has_climate);
        assert_eq!(s.phantom_base("ferryn"), 2);
        assert_eq!(s.phantom_base("briselle"), 0);
    }

    #[test]
    fn the_lure_has_a_cooldown_of_seven_seconds() {
        let mut s = quiet(2);
        assert!(s.lure("lobby"));
        assert_eq!(s.lure_cool, 7000.0);
        assert!(!s.lure("arcade"), "still cooling down");
        run(&mut s, 6900.0, "");
        assert!(!s.lure("arcade"));
        run(&mut s, 100.0, "");
        assert_eq!(s.lure_cool, 0.0);
        assert!(s.lure("arcade"));
    }

    #[test]
    fn a_lure_needs_audio_and_a_camera_room_but_not_a_vent() {
        let mut s = quiet(2);
        assert!(!s.lure("ventl"));
        assert!(!s.lure("officeL"));
        assert!(!s.lure("nowhere"));
        s.set_system("audio", true);
        assert!(!s.lure("lobby"));
        s.set_system("audio", false);
        assert!(s.lure("lobby"));
    }

    #[test]
    fn every_second_lure_breaks_the_audio() {
        let mut s = quiet(2);
        assert!(s.lure("lobby"));
        assert!(s.online("audio"));
        run(&mut s, 7000.0, "");
        assert!(s.lure("party"));
        assert!(!s.online("audio"), "the strain took it");
        let ev = s.step(100.0, "");
        assert!(ev.contains(&SimEvent::Broken { system: "audio".into(), id: "strain".into() }));
    }

    #[test]
    fn a_broken_system_takes_six_seconds_to_reboot() {
        let mut s = quiet(2);
        s.set_system("cams", true);
        assert!(s.reboot("cams"));
        assert_eq!(s.state_of("cams"), SysState::Reboot);
        assert!(!s.reboot("cams"), "only a broken system reboots");
        let mut fixed_at = None;
        for k in 1..=70 {
            let ev = s.step(100.0, "");
            if ev.contains(&SimEvent::Fixed { system: "cams".into() }) {
                fixed_at = Some(k);
                break;
            }
        }
        assert_eq!(fixed_at, Some(60));
        assert!(s.online("cams"));
    }

    #[test]
    fn she_gets_in_after_5200_ms_at_an_open_vent() {
        let mut s = quiet(2);
        s.killer.room = "officeL".into();
        let mut caught_at = None;
        for k in 1..=60 {
            let ev = s.step(100.0, "");
            if ev.iter().any(|e| matches!(e, SimEvent::Caught { .. })) {
                caught_at = Some(k);
                break;
            }
        }
        assert_eq!(caught_at, Some(52));
        assert_eq!(s.over, Over::Caught);
        assert_eq!(s.by, "xivvy");
    }

    #[test]
    fn a_sealed_vent_turns_her_away_after_two_seconds() {
        let mut s = quiet(2);
        s.killer.room = "officeR".into();
        assert!(s.seal('R', Some(true)));
        let mut at = None;
        for k in 1..=30 {
            let ev = s.step(100.0, "");
            if ev.contains(&SimEvent::Retreat { side: 'R' }) {
                at = Some(k);
                break;
            }
        }
        assert_eq!(at, Some(20));
        assert_eq!(s.killer.room, "storage");
        assert_eq!(s.over, Over::None);
        assert_eq!(s.killer.at_ms, 0.0);
    }

    #[test]
    fn sealed_vents_build_stress_and_the_air_gives_out_at_100() {
        let mut s = quiet(2);
        s.seal('L', Some(true));
        s.seal('R', Some(true));
        s.stress = 99.9;
        let ev = s.step(100.0, "");
        assert_eq!(s.stress, 100.0);
        assert_eq!(s.over, Over::Caught);
        assert_eq!(s.by, "mavrine");
        assert!(ev.contains(&SimEvent::Caught { by: "mavrine".into() }));
    }

    #[test]
    fn infinite_air_never_runs_out_and_god_sends_the_killer_back() {
        let mut s = quiet(2);
        s.infinite_stress = true;
        s.seal('L', Some(true));
        s.stress = 100.0;
        s.step(100.0, "");
        assert_eq!(s.over, Over::None);
        let mut g = quiet(2);
        g.god = true;
        g.killer.room = "officeL".into();
        run(&mut g, 6000.0, "");
        assert_eq!(g.over, Over::None);
        assert_eq!(g.killer.room, "storage");
    }

    #[test]
    fn stress_decays_when_nothing_is_wrong() {
        let mut s = quiet(2);
        s.stress = 10.0;
        s.step(1000.0, "");
        assert!((s.stress - 8.0).abs() < 1e-9, "decays 2 per second: {}", s.stress);
        s.set_system("vent", true);
        s.step(1000.0, "");
        assert!((s.stress - 11.0).abs() < 1e-9, "a broken vent adds 3 per second: {}", s.stress);
    }

    #[test]
    fn the_custom_night_uses_its_own_air() {
        let a = air_rates(true, 10);
        assert!((a.broken - 3.0).abs() < 1e-9);
        assert_eq!(a.seal, 1.2);
        assert_eq!(a.decay, 2.1);
        let b = air_rates(false, 10);
        assert_eq!(b.broken, 3.0);
        assert_eq!(b.decay, 2.0);
        let mut c = Session::new();
        let mut m = BTreeMap::new();
        m.insert("xivvy".to_string(), 9);
        m.insert("solyth".to_string(), 4);
        c.start_mode("lure", 1, m, Some(5));
        let s = Sim::new(SimOpts::from_session(&c));
        assert_eq!(s.n, 4, "a Custom Night plays at Night 5's tables");
        assert_eq!(s.lvl, 9);
        assert!(s.has_climate && s.has_fan);
        assert_eq!(s.phantom_base("ferryn"), 9, "a slider missing from the map falls back to Night 5's table");
    }

    #[test]
    fn the_climate_drifts_cold_with_wynelle_and_freezes_the_booth() {
        let mut o = SimOpts::night(4, 99);
        o.level = Some(0);
        o.phantoms = false;
        let mut s = Sim::new(o);
        assert!(s.has_climate && !s.has_fan);
        assert!(s.wynelle == 5 && s.solyth == 0);
        s.temp = 30.0;
        let mut dead_at = None;
        for k in 1..=2000 {
            s.step(100.0, "");
            if s.over != Over::None {
                dead_at = Some(k);
                break;
            }
        }
        assert!(dead_at.is_some());
        assert_eq!(s.by, "wynelle");
    }

    #[test]
    fn a_heater_pushes_the_temperature_up_and_strains_the_air() {
        let mut o = SimOpts::night(4, 99);
        o.level = Some(0);
        o.phantoms = false;
        let mut s = Sim::new(o);
        s.temp = 65.0;
        assert!(s.toggle("heater"));
        assert!(!s.toggle("fan"), "no fan without Solyth");
        let before = s.temp;
        s.step(1000.0, "");
        assert!(s.temp > before, "heater 0.9/s beats the drift");
        assert!(s.stress > 0.0, "the appliance strains the air");
        s.set_system("heater", true);
        assert!(!s.heater_on, "a broken heater switches off");
        assert!(!s.toggle("heater"));
    }

    #[test]
    fn xivvy_does_not_move_while_the_monitor_watches_her() {
        let mut o = SimOpts::night(6, 5);
        o.phantoms = false;
        let mut s = Sim::new(o);
        s.lvl = 20;
        let mut moved = false;
        for _ in 0..300 {
            let ev = s.step(100.0, "lobby");
            if ev.iter().any(|e| matches!(e, SimEvent::Moved { .. } | SimEvent::AtOffice { .. })) {
                moved = true;
            }
        }
        assert!(!moved);
        assert_eq!(s.killer.room, "lobby");
    }

    #[test]
    fn she_walks_the_graph_one_room_at_a_time_toward_her_vent() {
        let mut o = SimOpts::night(6, 5);
        o.phantoms = false;
        let mut s = Sim::new(o);
        s.lvl = 20;
        let graph = |a: &str, b: &str| tuning().neighbours(a).iter().any(|x| x == b);
        let mut last = s.killer.room.clone();
        let mut reached = false;
        for _ in 0..2000 {
            for e in s.step(100.0, "") {
                match e {
                    SimEvent::Moved { room, from } | SimEvent::AtOffice { room, from } => {
                        assert_eq!(from, last);
                        assert!(graph(&from, &room), "{from} -> {room}");
                        last = room.clone();
                        if is_office(&room) {
                            reached = true;
                        }
                    }
                    _ => {}
                }
            }
            if reached || s.over != Over::None {
                break;
            }
        }
        assert!(reached || s.over != Over::None);
    }

    #[test]
    fn the_lure_pulls_her_toward_the_room() {
        let mut o = SimOpts::night(3, 21);
        o.phantoms = false;
        let mut s = Sim::new(o);
        s.lvl = 20;
        s.killer.room = "storage".into();
        assert!(s.lure("lobby"));
        let mut rooms = vec![];
        for _ in 0..3000 {
            for e in s.step(100.0, "") {
                if let SimEvent::Moved { room, .. } = e {
                    rooms.push(room);
                }
            }
            if s.killer.lure_left <= 0.0 {
                break;
            }
        }
        assert!(!rooms.is_empty());
    }

    #[test]
    fn phantoms_break_their_system_and_show_on_a_camera() {
        let mut o = SimOpts::night(1, 3);
        o.level = Some(0);
        let mut s = Sim::new(o);
        s.hunter_speed = 8.0;
        let mut shown = false;
        let mut broken_cams = false;
        for _ in 0..3000 {
            for e in s.step(100.0, "") {
                match e {
                    SimEvent::Phantom { id, room } => {
                        assert_eq!(id, "ferryn");
                        assert!(ROOMS.contains(&room.as_str()));
                        shown = true;
                    }
                    SimEvent::Broken { system, id } => {
                        assert_eq!((system.as_str(), id.as_str()), ("cams", "ferryn"));
                        broken_cams = true;
                    }
                    _ => {}
                }
            }
            if shown {
                break;
            }
        }
        assert!(shown && broken_cams);
        assert!(s.phantom.is_some());
        run(&mut s, 4000.0, "");
        assert!(s.phantom.is_none(), "she shows for four seconds");
    }

    #[test]
    fn the_night_is_won_at_270_seconds() {
        let mut s = quiet(2);
        let mut ev = vec![];
        for _ in 0..2700 {
            ev.extend(s.step(100.0, ""));
        }
        assert_eq!(s.over, Over::Win);
        assert!(ev.contains(&SimEvent::Win));
        assert!(s.step(100.0, "").is_empty(), "nothing happens after the end");
    }

    #[test]
    fn the_same_seed_plays_the_same_shift() {
        let play = |seed: u32| {
            let mut o = SimOpts::night(5, seed);
            o.level = Some(12);
            let mut s = Sim::new(o);
            let mut names = vec![];
            for _ in 0..2700 {
                for e in s.step(100.0, "") {
                    names.push(e.name());
                }
                if s.over != Over::None {
                    break;
                }
            }
            (names, s.digest())
        };
        assert_eq!(play(777), play(777));
        assert_ne!(play(777).1, play(778).1);
    }

    #[test]
    fn the_levels_step_up_with_the_hour() {
        let mut s = quiet(5);
        s.lvl = 9;
        assert_eq!(s.level_of("xivvy"), 9);
        s.elapsed = 3.0 * 45_000.0;
        assert_eq!(s.level_of("xivvy"), 10);
        s.elapsed = 4.0 * 45_000.0;
        assert_eq!(s.level_of("xivvy"), 11);
        assert_eq!(s.level_of("wynelle"), 8, "the climate pair keep their level");
        s.set_level("ferryn", 4);
        assert_eq!(s.phantom_base("ferryn"), 4);
    }

    #[test]
    fn climate_visitors_show_from_85() {
        let mut s = quiet(4);
        assert!(s.climate_visitors().is_empty());
        s.cold = 85.0;
        assert_eq!(s.climate_visitors(), vec![("wynelle", 0.0)]);
        s.cold = 100.0;
        assert_eq!(s.climate_visitors()[0].1, 1.0);
    }
}
