//! Bedtime's rules (bedroom_logic.gd, same numbers, same draws in the same order) plus the living house around them:
//! `Sim` pairs the rules with a `Crowd` so Javelle really walks the hall on the nav graph while the rules decide WHEN.
//! Pure: no drawing, no GL, no window, no real clock. `BedroomHeadless` is the scripted autoplay used by `--check` and tests.
//!
//! You lie in bed with one free hand. Three things come for you, each from its own side, each turned away a different way:
//!   Loxelle   breathes behind the closet door (left). Hold the closet door shut until she goes.
//!   Javelle   waits in the hall doorway (right). A little light on the hall makes her leave.
//!   Haelith   creeps toward the bed, but only while the room is dark. Light on the bed pushes her back.
#![allow(dead_code)]

use super::map::{map, BedroomMap};
use crate::core::{
    effective_level, hour_index, Fnv, Headless, Mulberry32, Outcome, Pacer, Session, Stream, NIGHT_MS, TIME_SCALE,
};
use crate::audio::cue_names::office;
use crate::core::Cue;
use crate::hunters::{BehaviourData, BehaviourEvent, Crowd, NavMap, Pace};
use macroquad::prelude::Vec3;
use serde_json::Value;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::OnceLock;

pub const RULES_JSON: &str = include_str!("../../data/bedroom/rules.json");
pub const NAV_JSON: &str = include_str!("../../data/nav/bedroom.json");

// ------------------------------------------------------------------------------------------------ small enums

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hunter {
    Loxelle,
    Javelle,
    Haelith,
}

impl Hunter {
    pub const ALL: [Hunter; 3] = [Hunter::Loxelle, Hunter::Javelle, Hunter::Haelith];
    pub fn id(self) -> &'static str {
        match self {
            Hunter::Loxelle => "loxelle",
            Hunter::Javelle => "javelle",
            Hunter::Haelith => "haelith",
        }
    }
    pub fn from_id(id: &str) -> Option<Hunter> {
        match id {
            "loxelle" => Some(Hunter::Loxelle),
            "javelle" => Some(Hunter::Javelle),
            "haelith" => Some(Hunter::Haelith),
            _ => None,
        }
    }
    pub fn idx(self) -> usize {
        match self {
            Hunter::Loxelle => 0,
            Hunter::Javelle => 1,
            Hunter::Haelith => 2,
        }
    }
}

/// How far one of the two door threats has come.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Away,
    Breath,
    Near,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Away => "away",
            Stage::Breath => "breath",
            Stage::Near => "near",
        }
    }
    pub fn from_name(s: &str) -> Option<Stage> {
        match s {
            "away" => Some(Stage::Away),
            "breath" => Some(Stage::Breath),
            "near" => Some(Stage::Near),
            _ => None,
        }
    }
}

/// Which flashlight is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Light {
    Dark,
    Hall,
    Bed,
}

impl Light {
    pub fn name(self) -> &'static str {
        match self {
            Light::Dark => "",
            Light::Hall => "hall",
            Light::Bed => "bed",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    Running,
    Win,
    Caught,
}

/// What the one free hand is doing this step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SimInput {
    pub hold: bool,
    pub light: Light,
}

impl SimInput {
    pub const NONE: SimInput = SimInput { hold: false, light: Light::Dark };
}

#[derive(Clone, PartialEq, Debug)]
pub enum SimEvent {
    Breath(Hunter),
    Near(Hunter),
    Leave(Hunter),
    Stir(f64),
    Restless,
    Caught(Hunter),
    Win,
    /// Idle behaviour of a hunter (hunter id, what she is doing).
    Task(String, String),
}

impl SimEvent {
    pub fn name(&self) -> String {
        match self {
            SimEvent::Breath(h) => format!("breath:{}", h.id()),
            SimEvent::Near(h) => format!("near:{}", h.id()),
            SimEvent::Leave(h) => format!("leave:{}", h.id()),
            SimEvent::Stir(p) => format!("stir:{}", p.round() as i32),
            SimEvent::Restless => "restless".to_string(),
            SimEvent::Caught(h) => format!("caught:{}", h.id()),
            SimEvent::Win => "win".to_string(),
            SimEvent::Task(id, what) => format!("task:{id}:{what}"),
        }
    }
}

// ------------------------------------------------------------------------------------------------ rules data

#[derive(Clone, Debug)]
pub struct NightRow {
    pub ai: [i32; 3],
    pub breath_ms: f64,
    pub near_ms: f64,
    /// First wait before a door threat can come: [loxelle, javelle].
    pub first: [f64; 2],
    /// Rest after she was turned away: [loxelle, javelle].
    pub rest: [f64; 2],
}

#[derive(Clone, Debug)]
pub struct Tunables {
    pub wait_spread: f64,
    pub door_tempo: f64,
    pub haelith_tempo: f64,
    pub swap_first: f64,
    pub stage_jitter: f64,
    pub hold_need_ms: f64,
    pub light_need_ms: f64,
    pub afk_ms: f64,
    pub afk_bonus: i32,
    pub afk_speed: f64,
    pub bed_gain: f64,
    pub bed_gain_per_level: f64,
    pub bed_lit_drain_per_sec: f64,
    pub bed_visible_at: f64,
    pub bed_stir_at: f64,
}

#[derive(Clone, Debug)]
pub struct CueTuning {
    pub breath_gap_near_ms: f64,
    pub breath_gap_ms: f64,
    pub haelith_gap_ms: f64,
    pub haelith_visible_p: f32,
    pub breath_near: f32,
    pub breath_far: f32,
}

#[derive(Clone, Debug)]
pub struct LookTuning {
    pub yaw: f32,
    pub pitch_up: f32,
    pub pitch_down_factor: f32,
    pub sense: f32,
    pub key_yaw_rate: f32,
    pub key_pitch_rate: f32,
    pub fov: f32,
}

#[derive(Clone, Debug)]
pub struct Rules {
    pub tun: Tunables,
    pub nights: Vec<NightRow>,
    pub custom_row: NightRow,
    pub custom_night: i32,
    pub score_max: i32,
    /// Base flash seconds before the time scale.
    pub flash_base: Vec<f64>,
    pub names: [String; 3],
    pub pan: [f32; 3],
    pub javelle_budget_s: f32,
    pub cues: CueTuning,
    pub look: LookTuning,
    pub react_ms: (f64, f64),
    pub bed_at_p: f64,
}

fn num(v: &Value, d: f64) -> f64 {
    v.as_f64().unwrap_or(d)
}

fn row_of(v: &Value) -> NightRow {
    let ai = |id: &str| v["ai"][id].as_i64().unwrap_or(0) as i32;
    NightRow {
        ai: [ai("loxelle"), ai("javelle"), ai("haelith")],
        breath_ms: num(&v["breathMs"], 3200.0),
        near_ms: num(&v["nearMs"], 4200.0),
        first: [num(&v["first"]["loxelle"], 3000.0), num(&v["first"]["javelle"], 3000.0)],
        rest: [num(&v["rest"]["loxelle"], 7000.0), num(&v["rest"]["javelle"], 7000.0)],
    }
}

impl Rules {
    pub fn parse(src: &str) -> Result<Rules, String> {
        let j: Value = serde_json::from_str(src).map_err(|e| format!("bedroom rules json: {e}"))?;
        let t = &j["t"];
        let v = &j["variety"];
        let nights: Vec<NightRow> = j["nights"].as_array().map(|a| a.iter().map(row_of).collect()).unwrap_or_default();
        if nights.len() != 6 {
            return Err(format!("bedroom rules json: {} nights, want 6", nights.len()));
        }
        let flash_base: Vec<f64> = j["flash_base_seconds"].as_array().map(|a| a.iter().map(|x| num(x, 60.0)).collect()).unwrap_or_default();
        if flash_base.len() != 6 {
            return Err("bedroom rules json: flash_base_seconds needs 6 entries".to_string());
        }
        let name = |id: &str| j["names"][id].as_str().unwrap_or(id).to_string();
        let pan = |id: &str| j["pan"][id].as_f64().unwrap_or(0.0) as f32;
        let c = &j["cues"];
        let l = &j["look"];
        let a = &j["autoplay"];
        let react = (num(&a["react_ms"][0], 250.0), num(&a["react_ms"][1], 700.0));
        Ok(Rules {
            tun: Tunables {
                wait_spread: num(&v["waitSpread"], 0.25),
                door_tempo: num(&v["doorTempo"], 0.1),
                haelith_tempo: num(&v["haelithTempo"], 0.2),
                swap_first: num(&v["swapFirst"], 0.3),
                stage_jitter: num(&t["stageJitter"], 0.4),
                hold_need_ms: num(&t["holdNeedMs"], 700.0),
                light_need_ms: num(&t["lightNeedMs"], 600.0),
                afk_ms: num(&t["afkMs"], 14000.0),
                afk_bonus: num(&t["afkBonus"], 4.0) as i32,
                afk_speed: num(&t["afkSpeed"], 1.5),
                bed_gain: num(&t["bedGain"], 8.0),
                bed_gain_per_level: num(&t["bedGainPerLevel"], 0.9),
                bed_lit_drain_per_sec: num(&t["bedLitDrainPerSec"], 35.0),
                bed_visible_at: num(&t["bedVisibleAt"], 80.0),
                bed_stir_at: num(&t["bedStirAt"], 45.0),
            },
            nights,
            custom_row: row_of(&j["custom_row"]),
            custom_night: num(&j["custom_night"], 5.0) as i32,
            score_max: num(&j["score_max"], 400.0) as i32,
            flash_base,
            names: [name("loxelle"), name("javelle"), name("haelith")],
            pan: [pan("loxelle"), pan("javelle"), pan("haelith")],
            javelle_budget_s: num(&j["javelle_budget_s"], 2.4) as f32,
            cues: CueTuning {
                breath_gap_near_ms: num(&c["breath_gap_near_ms"], 1100.0),
                breath_gap_ms: num(&c["breath_gap_ms"], 1700.0),
                haelith_gap_ms: num(&c["haelith_gap_ms"], 2400.0),
                haelith_visible_p: num(&c["haelith_visible_p"], 10.0) as f32,
                breath_near: num(&c["breath_near"], 0.9) as f32,
                breath_far: num(&c["breath_far"], 0.5) as f32,
            },
            look: LookTuning {
                yaw: num(&l["yaw"], 1.25) as f32,
                pitch_up: num(&l["pitch_up"], 0.7) as f32,
                pitch_down_factor: num(&l["pitch_down_factor"], 0.6) as f32,
                sense: num(&l["sense"], 0.0022) as f32,
                key_yaw_rate: num(&l["key_yaw_rate"], 1.4) as f32,
                key_pitch_rate: num(&l["key_pitch_rate"], 1.0) as f32,
                fov: num(&l["fov"], 75.0) as f32,
            },
            react_ms: react,
            bed_at_p: num(&a["bed_at_p"], 45.0),
        })
    }

    /// Flash seconds of night index `n` (0-based): `roundi(base * TIME_SCALE)`.
    pub fn flash_seconds(&self, n: usize) -> i64 {
        (self.flash_base[n.min(5)] * TIME_SCALE).round() as i64
    }

    pub fn name_of(&self, h: Hunter) -> &str {
        &self.names[h.idx()]
    }

    pub fn pan_of(&self, h: Hunter) -> f32 {
        self.pan[h.idx()]
    }

    /// Who is on the floor on a level (0-based): the hunters with a level above 0.
    pub fn active_hunters(&self, level_idx: usize) -> Vec<Hunter> {
        let row = &self.nights[level_idx.min(5)];
        Hunter::ALL.iter().copied().filter(|h| row.ai[h.idx()] > 0).collect()
    }
}

/// The embedded rules. A broken file is reported by the tests and `--check`; the fallback keeps every night at level 0.
pub fn rules() -> &'static Rules {
    static R: OnceLock<Rules> = OnceLock::new();
    R.get_or_init(|| {
        Rules::parse(RULES_JSON).unwrap_or_else(|_| {
            let row = NightRow { ai: [0; 3], breath_ms: 3200.0, near_ms: 4200.0, first: [3000.0; 2], rest: [7000.0; 2] };
            Rules {
                tun: Tunables {
                    wait_spread: 0.25,
                    door_tempo: 0.1,
                    haelith_tempo: 0.2,
                    swap_first: 0.3,
                    stage_jitter: 0.4,
                    hold_need_ms: 700.0,
                    light_need_ms: 600.0,
                    afk_ms: 14000.0,
                    afk_bonus: 4,
                    afk_speed: 1.5,
                    bed_gain: 8.0,
                    bed_gain_per_level: 0.9,
                    bed_lit_drain_per_sec: 35.0,
                    bed_visible_at: 80.0,
                    bed_stir_at: 45.0,
                },
                nights: vec![row.clone(); 6],
                custom_row: row,
                custom_night: 5,
                score_max: 400,
                flash_base: vec![60.0; 6],
                names: ["Loxelle".into(), "Javelle".into(), "Haelith".into()],
                pan: [-0.85, 0.85, 0.0],
                javelle_budget_s: 2.4,
                cues: CueTuning {
                    breath_gap_near_ms: 1100.0,
                    breath_gap_ms: 1700.0,
                    haelith_gap_ms: 2400.0,
                    haelith_visible_p: 10.0,
                    breath_near: 0.9,
                    breath_far: 0.5,
                },
                look: LookTuning {
                    yaw: 1.25,
                    pitch_up: 0.7,
                    pitch_down_factor: 0.6,
                    sense: 0.0022,
                    key_yaw_rate: 1.4,
                    key_pitch_rate: 1.0,
                    fov: 75.0,
                },
                react_ms: (250.0, 700.0),
                bed_at_p: 45.0,
            }
        })
    })
}

// ------------------------------------------------------------------------------------------------ the rules

/// One of the two door threats (Loxelle, Javelle).
#[derive(Clone, Debug)]
pub struct DoorThreat {
    pub state: Stage,
    pub t: f64,
    pub pacer: Pacer,
    pub cool: f64,
    pub need: f64,
    pub jitter: f64,
    pub tempo: f64,
}

/// Haelith's creep.
#[derive(Clone, Debug)]
pub struct Creeper {
    /// The sleep meter, 0..100.
    pub p: f64,
    pub pacer: Pacer,
    pub tempo: f64,
}

pub struct LogicOpts {
    /// 1..=6.
    pub night: i32,
    pub custom: bool,
    pub rng: Stream,
    pub var_rng: Option<Stream>,
    /// Hunter levels that override the night's row (custom nights); None or a negative level keeps the row.
    pub levels: [Option<i32>; 3],
    pub infinite_battery: bool,
}

impl LogicOpts {
    pub fn seeded(night: i32, seed: u32) -> LogicOpts {
        LogicOpts { night, custom: false, rng: Stream::seeded(seed), var_rng: None, levels: [None; 3], infinite_battery: false }
    }
}

/// bedroom_logic.gd: pure rules. `step(dt_ms, input)` returns events.
pub struct BedroomLogic {
    /// 0-based night.
    pub n: usize,
    pub row: NightRow,
    rng: Stream,
    vr: Stream,
    pub levels: [Option<i32>; 3],
    pub infinite_battery: bool,
    pub is_custom: bool,
    pub flash_per_sec: f64,
    pub swapped: bool,
    pub elapsed: f64,
    pub over: Over,
    pub by: Option<Hunter>,
    pub battery: f64,
    pub light: Light,
    pub hold: bool,
    /// [loxelle, javelle]
    pub doors: [DoorThreat; 2],
    pub haelith: Creeper,
    pub idle: f64,
    pub restless: bool,
    /// Admin: whoever would have got in is sent back.
    pub god: bool,
    /// Admin: the hunters' wait clocks run at this multiple.
    pub hunter_speed: f64,
}

fn spread(ms: f64, rng: &mut Stream, s: f64) -> f64 {
    ms * (1.0 - s + 2.0 * s * rng.next())
}

impl BedroomLogic {
    pub fn new(o: LogicOpts) -> BedroomLogic {
        let r = rules();
        let n = (o.night.clamp(1, 6) - 1) as usize;
        let row = if o.custom { r.custom_row.clone() } else { r.nights[n].clone() };
        let rng = o.rng;
        let vr = Stream::variety_of(&rng, o.var_rng);
        let flash_per_sec = 100.0 / r.flash_seconds(n) as f64;
        let mut me = BedroomLogic {
            n,
            row,
            rng,
            vr,
            levels: o.levels,
            infinite_battery: o.infinite_battery,
            is_custom: o.custom,
            flash_per_sec,
            swapped: false,
            elapsed: 0.0,
            over: Over::Running,
            by: None,
            battery: 100.0,
            light: Light::Dark,
            hold: false,
            doors: [
                DoorThreat { state: Stage::Away, t: 0.0, pacer: Pacer::new(), cool: 0.0, need: 0.0, jitter: 1.0, tempo: 1.0 },
                DoorThreat { state: Stage::Away, t: 0.0, pacer: Pacer::new(), cool: 0.0, need: 0.0, jitter: 1.0, tempo: 1.0 },
            ],
            haelith: Creeper { p: 0.0, pacer: Pacer::new(), tempo: 1.0 },
            idle: 0.0,
            restless: false,
            god: false,
            hunter_speed: 1.0,
        };
        // the draws, in the order of the GDScript _init
        let swap = me.vr.chance(r.tun.swap_first);
        me.swapped = swap && me.base_level(Hunter::Loxelle) > 0 && me.base_level(Hunter::Javelle) > 0;
        me.doors[0] = me.make_door(Hunter::Loxelle);
        me.doors[1] = me.make_door(Hunter::Javelle);
        let tempo = me.vr.tempo(r.tun.haelith_tempo);
        me.haelith = Creeper { p: 0.0, pacer: Pacer::new(), tempo };
        me
    }

    fn first_of(&self, h: Hunter) -> f64 {
        let k = if h == Hunter::Loxelle { 0 } else { 1 };
        if self.swapped {
            self.row.first[1 - k]
        } else {
            self.row.first[k]
        }
    }

    fn make_door(&mut self, h: Hunter) -> DoorThreat {
        let r = rules();
        let first = self.first_of(h);
        let cool = spread(first, &mut self.rng, r.tun.wait_spread);
        let tempo = self.vr.tempo(r.tun.door_tempo);
        DoorThreat { state: Stage::Away, t: 0.0, pacer: Pacer::new(), cool, need: 0.0, jitter: 1.0, tempo }
    }

    pub fn base_level(&self, h: Hunter) -> i32 {
        match self.levels[h.idx()] {
            Some(l) if l >= 0 => l,
            _ => self.row.ai[h.idx()],
        }
    }

    /// Steps up at 3 and 4 AM.
    pub fn level(&self, h: Hunter) -> i32 {
        effective_level(self.base_level(h), hour_index(self.elapsed))
    }

    pub fn door(&self, h: Hunter) -> Option<&DoorThreat> {
        match h {
            Hunter::Loxelle => Some(&self.doors[0]),
            Hunter::Javelle => Some(&self.doors[1]),
            Hunter::Haelith => None,
        }
    }

    pub fn door_mut(&mut self, h: Hunter) -> Option<&mut DoorThreat> {
        match h {
            Hunter::Loxelle => Some(&mut self.doors[0]),
            Hunter::Javelle => Some(&mut self.doors[1]),
            Hunter::Haelith => None,
        }
    }

    fn finish(&mut self, ev: &mut Vec<SimEvent>, who: Hunter) {
        if self.god {
            // admin: she is sent away
            match who {
                Hunter::Haelith => self.haelith.p = 0.0,
                _ => {
                    let rest = self.row.rest[who.idx().min(1)];
                    if let Some(th) = self.door_mut(who) {
                        th.state = Stage::Away;
                        th.t = 0.0;
                        th.need = 0.0;
                        th.cool = rest;
                        th.pacer.reset();
                    }
                }
            }
            return;
        }
        self.over = Over::Caught;
        self.by = Some(who);
        ev.push(SimEvent::Caught(who));
    }

    fn stage_jitter(&mut self) -> f64 {
        let j = rules().tun.stage_jitter;
        1.0 - j / 2.0 + self.rng.next() * j
    }

    /// One step. `input.hold` is the closet door, `input.light` the hall or the bed (ignored while holding).
    pub fn step(&mut self, dt_ms: f64, input: &SimInput) -> Vec<SimEvent> {
        let mut ev = Vec::new();
        if self.over != Over::Running {
            return ev;
        }
        let tun = rules().tun.clone();
        let dt = dt_ms / 1000.0;
        self.elapsed += dt_ms;
        // one hand: holding the door means no light, and only one light at a time; a flat battery means no light
        self.hold = input.hold;
        let mut lit = Light::Dark;
        if !self.hold {
            lit = input.light;
        }
        if self.battery <= 0.0 {
            lit = Light::Dark;
        }
        self.light = lit;
        if lit != Light::Dark && !self.infinite_battery {
            self.battery = (self.battery - self.flash_per_sec * dt).max(0.0);
        }
        // restless: nothing done for a while
        self.idle = if self.hold || lit != Light::Dark { 0.0 } else { self.idle + dt_ms };
        let is_restless = self.idle >= tun.afk_ms && (self.level(Hunter::Loxelle) > 0 || self.level(Hunter::Javelle) > 0);
        if is_restless && !self.restless {
            ev.push(SimEvent::Restless);
        }
        self.restless = is_restless;
        let speed = if self.restless { tun.afk_speed } else { 1.0 };
        // the two door threats
        for k in 0..2usize {
            let who = if k == 0 { Hunter::Loxelle } else { Hunter::Javelle };
            let answered = if k == 0 { self.hold } else { lit == Light::Hall };
            let need = if k == 0 { tun.hold_need_ms } else { tun.light_need_ms };
            if self.doors[k].state == Stage::Away {
                self.doors[k].cool = (self.doors[k].cool - dt_ms).max(0.0);
                let mut keen = self.level(who);
                if keen > 0 && self.restless {
                    keen = (keen + tun.afk_bonus).min(20);
                }
                if self.doors[k].cool <= 0.0 {
                    let step = dt_ms * self.doors[k].tempo * self.hunter_speed;
                    let due = {
                        let rng = &mut self.rng;
                        let th = &mut self.doors[k];
                        th.pacer.step(step, keen, &mut || rng.next())
                    };
                    if due {
                        let jitter = self.stage_jitter();
                        let tempo = self.vr.tempo(tun.door_tempo);
                        let th = &mut self.doors[k];
                        th.state = Stage::Breath;
                        th.t = 0.0;
                        th.need = 0.0;
                        th.jitter = jitter;
                        th.tempo = tempo;
                        ev.push(SimEvent::Breath(who));
                    }
                }
                continue;
            }
            if !(who == Hunter::Javelle && lit == Light::Hall) {
                self.doors[k].t += dt_ms * speed; // looked at: she holds her stage while the hall is lit
            }
            let cur = self.doors[k].need;
            self.doors[k].need = if answered { cur + dt_ms } else { (cur - dt_ms * 0.5).max(0.0) };
            if self.doors[k].need >= need {
                let rest = spread(self.row.rest[k], &mut self.rng, tun.wait_spread);
                let th = &mut self.doors[k];
                th.state = Stage::Away;
                th.cool = rest;
                th.pacer.reset();
                th.need = 0.0;
                ev.push(SimEvent::Leave(who));
            } else if self.doors[k].state == Stage::Breath && self.doors[k].t >= self.row.breath_ms * self.doors[k].jitter {
                let jitter = self.stage_jitter();
                let th = &mut self.doors[k];
                th.state = Stage::Near;
                th.t = 0.0;
                th.jitter = jitter;
                ev.push(SimEvent::Near(who));
            } else if self.doors[k].state == Stage::Near && self.doors[k].t >= self.row.near_ms * self.doors[k].jitter {
                self.finish(&mut ev, who);
                return ev;
            }
        }
        // Haelith: she only creeps while the room is dark; light on the bed pushes her back
        let lvl = self.level(Hunter::Haelith);
        if lit == Light::Dark {
            let step = dt_ms * self.haelith.tempo * self.hunter_speed;
            let due = {
                let rng = &mut self.rng;
                self.haelith.pacer.step(step, lvl, &mut || rng.next())
            };
            if due {
                self.haelith.p = (self.haelith.p + tun.bed_gain + tun.bed_gain_per_level * lvl as f64).min(100.0);
                self.haelith.tempo = self.vr.tempo(tun.haelith_tempo);
                ev.push(SimEvent::Stir(self.haelith.p));
            }
        }
        if lit == Light::Bed {
            self.haelith.p = (self.haelith.p - tun.bed_lit_drain_per_sec * dt).max(0.0);
        }
        if self.haelith.p >= 100.0 {
            self.finish(&mut ev, Hunter::Haelith);
            return ev;
        }
        if self.elapsed >= NIGHT_MS {
            self.over = Over::Win;
            ev.push(SimEvent::Win);
        }
        ev
    }
}

// ------------------------------------------------------------------------------------------------ the living house

/// Everything `Sim::new` needs from the session.
#[derive(Clone, Debug)]
pub struct SimOpts {
    pub night: i32,
    pub seed: u32,
    pub custom: bool,
    /// Custom Night sliders by hunter id.
    pub levels: BTreeMap<String, i32>,
    pub infinite_battery: bool,
    /// Manual clock (tests of presentation, `--shot`): hunters appear at their spot at once instead of walking.
    pub snap: bool,
}

impl SimOpts {
    pub fn from_session(s: &Session) -> SimOpts {
        let r = rules();
        let custom = s.is_custom;
        let night = if custom { r.custom_night } else { s.night.clamp(1, 6) };
        SimOpts { night, seed: s.seed_value, custom, levels: if custom { s.custom_levels.clone() } else { BTreeMap::new() }, infinite_battery: false, snap: false }
    }
}

/// The rules plus the crowd that walks the house. The rules decide WHEN a hunter moves; the crowd decides HOW she gets
/// there (A* on `data/nav/bedroom.json`, walked over real time).
pub struct Sim {
    pub logic: BedroomLogic,
    crowd: Crowd,
    nav: Rc<NavMap>,
    map: &'static BedroomMap,
    jav_stage: Stage,
    snap: bool,
    scared: bool,
}

impl Sim {
    pub fn new(o: &SimOpts) -> Sim {
        let map = map();
        let mut levels = [None; 3];
        for h in Hunter::ALL {
            levels[h.idx()] = o.levels.get(h.id()).copied();
        }
        let logic = BedroomLogic::new(LogicOpts {
            night: o.night,
            custom: o.custom,
            rng: Stream::seeded(o.seed),
            var_rng: None,
            levels,
            infinite_battery: o.infinite_battery,
        });
        let nav = Rc::new(NavMap::from_json(NAV_JSON).unwrap_or_else(|_| NavMap::new("bedroom")));
        let mut crowd = Crowd::new(nav.clone(), BehaviourData::embedded(), o.seed);
        let eye = Some(map.eye);
        crowd.add("javelle", "living_room", 0);
        crowd.add("loxelle", "wardrobe", 0);
        crowd.add_at("haelith", "bedroom", map.creep_position(0.0), eye);
        // the sim owns Loxelle (behind the closet door) and Haelith (the creep); Javelle works while she is away
        crowd.set_enabled("loxelle", false);
        crowd.set_enabled("haelith", false);
        Sim { logic, crowd, nav, map, jav_stage: Stage::Away, snap: o.snap, scared: false }
    }

    pub fn crowd(&self) -> &Crowd {
        &self.crowd
    }

    pub fn nav(&self) -> &NavMap {
        &self.nav
    }

    pub fn map(&self) -> &'static BedroomMap {
        self.map
    }

    pub fn set_snap(&mut self, on: bool) {
        self.snap = on;
    }

    pub fn pos_of(&self, h: Hunter) -> Vec3 {
        self.crowd.agent(h.id()).map(|a| a.walker.pos).unwrap_or(Vec3::ZERO)
    }

    /// Javelle's stage as the walker last got it.
    pub fn javelle_stage(&self) -> Stage {
        self.jav_stage
    }

    /// Step the rules, give the hunters their orders and walk the crowd for `dt_ms`.
    pub fn step(&mut self, dt_ms: f64, input: &SimInput) -> Vec<SimEvent> {
        let mut ev = self.logic.step(dt_ms, input);
        if let Some(who) = self.logic.by {
            if self.logic.over == Over::Caught && !self.scared {
                self.place_scare(who);
            }
        }
        if !self.scared {
            self.sync_javelle();
            self.sync_haelith();
        }
        for b in self.crowd.tick((dt_ms / 1000.0) as f32) {
            if let BehaviourEvent::TaskStart { hunter, label, .. } = b {
                ev.push(SimEvent::Task(hunter, label));
            }
        }
        ev
    }

    /// Keep the hunters walking after the outcome is decided (real seconds).
    pub fn tick_crowd(&mut self, dt_s: f32) {
        self.crowd.tick(dt_s);
    }

    /// Javelle: the stage of the rules picks where she stands; the crowd walks her there.
    pub fn sync_javelle(&mut self) {
        let want = self.logic.doors[1].state;
        if want == self.jav_stage {
            return;
        }
        self.jav_stage = want;
        let (room, slot) = match want {
            Stage::Away => ("living_room", 0),
            Stage::Breath => ("hall", 1),
            Stage::Near => ("hall", 0),
        };
        if self.snap {
            self.crowd.teleport("javelle", room, slot);
        } else {
            self.crowd.send("javelle", room, slot, Pace::Budget(rules().javelle_budget_s));
        }
        self.crowd.set_enabled("javelle", want == Stage::Away);
    }

    /// Haelith: across the floor as her number rises.
    pub fn sync_haelith(&mut self) {
        let spot = self.map.creep_position(self.logic.haelith.p as f32);
        let eye = Some(self.map.eye);
        if self.snap {
            self.crowd.add_at("haelith", "bedroom", spot, eye);
        } else {
            self.crowd.set_final("haelith", spot, eye);
        }
    }

    /// Admin: put a hunter at a stage (Haelith's creep goes to 0 / 50 / 90 percent for away / breath / near).
    pub fn admin_place(&mut self, h: Hunter, stage: Stage) {
        match h {
            Hunter::Haelith => {
                self.logic.haelith.p = match stage {
                    Stage::Away => 0.0,
                    Stage::Breath => 50.0,
                    Stage::Near => 90.0,
                };
            }
            _ => {
                if let Some(th) = self.logic.door_mut(h) {
                    th.state = stage;
                    th.t = 0.0;
                    th.need = 0.0;
                    th.jitter = 1.0;
                    if stage == Stage::Away {
                        th.cool = 1e9;
                    }
                }
            }
        }
    }

    /// The death scene: she stands where the Godot scare puts her, looking at the pillow.
    fn place_scare(&mut self, who: Hunter) {
        self.scared = true;
        let Some(sc) = self.map.scare(who.id()) else { return };
        let room = self.map.room_at(sc.pos.x, sc.pos.z).unwrap_or("bedroom").to_string();
        self.crowd.add_at(who.id(), &room, sc.pos, Some(self.map.eye));
        self.crowd.set_enabled(who.id(), false);
    }

    /// Make sure a catch (even one set by hand, like the admin lose) has put the hunter in her scene.
    pub fn settle_catch(&mut self) {
        if let Some(who) = self.logic.by {
            if self.logic.over == Over::Caught && !self.scared {
                self.place_scare(who);
            }
        }
    }

    /// Which light the death scene turns on.
    pub fn scare_light(&self, who: Hunter) -> Light {
        match self.map.scare(who.id()).map(|s| s.light.as_str()) {
            Some("hall") => Light::Hall,
            Some("bed") => Light::Bed,
            _ => Light::Dark,
        }
    }

    pub fn caught(&self) -> bool {
        self.logic.over == Over::Caught
    }

    pub fn outcome(&self) -> Outcome {
        match self.logic.over {
            Over::Running => Outcome::Running,
            Over::Win => Outcome::Won,
            Over::Caught => Outcome::Lost,
        }
    }

    /// FNV-1a of the key state: time, rules, hunters' stages and where all three stand.
    pub fn digest(&self, log: &[String]) -> u64 {
        let l = &self.logic;
        let mut h = Fnv::new();
        h.f64(l.elapsed).i32(l.over as i32 + 1).f64(l.battery).f64(l.haelith.p).i32(l.restless as i32);
        for d in &l.doors {
            h.str(d.state.name()).f64(d.t).f64(d.need).f64(d.cool);
        }
        for who in Hunter::ALL {
            let p = self.pos_of(who);
            h.f64(p.x as f64).f64(p.z as f64);
        }
        for e in log {
            h.str(e);
        }
        h.finish()
    }
}

// ------------------------------------------------------------------------------------------------ headless autoplay

/// What the scripted player does with the hand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hand {
    None,
    Hold,
    Hall,
    Bed,
}

impl Hand {
    pub fn input(self) -> SimInput {
        match self {
            Hand::None => SimInput::NONE,
            Hand::Hold => SimInput { hold: true, light: Light::Dark },
            Hand::Hall => SimInput { hold: false, light: Light::Hall },
            Hand::Bed => SimInput { hold: false, light: Light::Bed },
        }
    }
}

/// A seeded player: answers the nearest threat after a human reaction time, holds the closet when idle too long.
pub struct BedroomHeadless {
    pub sim: Sim,
    pol: Mulberry32,
    hand: Hand,
    want: Hand,
    due_ms: f64,
    log: Vec<String>,
    /// Count of events seen (for tests).
    pub seen: usize,
}

impl BedroomHeadless {
    pub fn new(session: &Session) -> BedroomHeadless {
        BedroomHeadless::from_opts(&SimOpts::from_session(session))
    }

    pub fn from_opts(o: &SimOpts) -> BedroomHeadless {
        BedroomHeadless {
            sim: Sim::new(o),
            pol: Mulberry32::new(o.seed ^ 0x6265_6430),
            hand: Hand::None,
            want: Hand::None,
            due_ms: 0.0,
            log: Vec::new(),
            seen: 0,
        }
    }

    /// The hand a careful player wants down right now.
    fn desired(&self) -> Hand {
        let l = &self.sim.logic;
        let tun = &rules().tun;
        let [lox, jav] = &l.doors;
        if lox.state == Stage::Near {
            Hand::Hold
        } else if jav.state == Stage::Near {
            Hand::Hall
        } else if lox.state == Stage::Breath {
            Hand::Hold
        } else if jav.state == Stage::Breath {
            Hand::Hall
        } else if l.haelith.p >= rules().bed_at_p && l.battery > 0.0 {
            Hand::Bed
        } else if l.idle >= tun.afk_ms * 0.7 {
            Hand::Hold // the closet door costs nothing to hold
        } else {
            Hand::None
        }
    }

    pub fn events_seen(&self) -> &[String] {
        &self.log
    }
}

impl Headless for BedroomHeadless {
    fn id(&self) -> &'static str {
        "bedroom"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        let want = self.desired();
        if want != self.want {
            self.want = want;
            let (lo, hi) = rules().react_ms;
            self.due_ms = self.sim.logic.elapsed + self.pol.range(lo, hi);
        }
        if self.want == Hand::None || self.sim.logic.elapsed >= self.due_ms {
            self.hand = self.want;
        }
        let ev = self.sim.step(dt_ms, &self.hand.input());
        let names: Vec<String> = ev.iter().map(|e| e.name()).collect();
        self.seen += names.len();
        self.log.extend(names.iter().cloned());
        names
    }

    fn outcome(&self) -> Outcome {
        self.sim.outcome()
    }

    fn elapsed_ms(&self) -> f64 {
        self.sim.logic.elapsed
    }

    fn digest(&self) -> u64 {
        self.sim.digest(&self.log)
    }
}

// ------------------------------------------------------------------------------------------------ what is seen and heard

/// Who shows on the screen (bedroom_game.gd `_place_hunters`): Javelle once she is at a stage, walking, or in the hall; Haelith
/// once she is far enough across the floor; Loxelle never (she is behind the closet door) except in the admin reveal. The
/// hunter who caught you is always shown.
pub fn hunter_visible(sim: &Sim, h: Hunter, reveal: bool) -> bool {
    if reveal {
        return true;
    }
    let l = &sim.logic;
    if sim.caught() && l.by == Some(h) {
        return true;
    }
    let awake = l.base_level(h) > 0;
    match h {
        Hunter::Loxelle => false,
        Hunter::Javelle => {
            let walking = sim.crowd().agent("javelle").map(|a| a.walker.is_walking()).unwrap_or(false);
            let in_hall = sim.pos_of(Hunter::Javelle).x < sim.map().hall_end_x() + 0.2;
            awake && (l.doors[1].state != Stage::Away || walking || in_hall)
        }
        Hunter::Haelith => awake && l.haelith.p as f32 >= rules().cues.haelith_visible_p,
    }
}

/// The sounds of the night. Everything is heard, so every cue is panned to the side its hunter comes from.
#[derive(Clone, Debug)]
pub struct CueClock {
    last: [f64; 3],
}

impl Default for CueClock {
    fn default() -> Self {
        CueClock::new()
    }
}

impl CueClock {
    pub fn new() -> CueClock {
        CueClock { last: [0.0; 3] }
    }

    /// The cues of one sim event (`BedroomSounds`).
    pub fn on_event(&mut self, e: &SimEvent, elapsed: f64) -> Vec<Cue> {
        let r = rules();
        match e {
            SimEvent::Breath(h) => {
                self.last[h.idx()] = elapsed;
                office("footsteps", r.pan_of(*h), 0.5)
            }
            SimEvent::Near(h) => {
                self.last[h.idx()] = elapsed;
                office("breath", r.pan_of(*h), 1.0)
            }
            SimEvent::Leave(h) => office("scrape", r.pan_of(*h), 0.9),
            SimEvent::Stir(_) => office("presence", 0.0, 0.7),
            SimEvent::Restless => office("warning", 0.0, 0.8),
            _ => Vec::new(),
        }
    }

    /// They are heard, not seen: breathing on the side each one comes from, louder as she gets close (the 2D scene's cadence).
    pub fn breathing(&mut self, l: &BedroomLogic) -> Vec<Cue> {
        let r = rules();
        let mut out = Vec::new();
        for h in [Hunter::Loxelle, Hunter::Javelle] {
            let th = &l.doors[h.idx()];
            if th.state == Stage::Away {
                continue;
            }
            let near = th.state == Stage::Near;
            let gap = if near { r.cues.breath_gap_near_ms } else { r.cues.breath_gap_ms };
            if l.elapsed - self.last[h.idx()] > gap {
                self.last[h.idx()] = l.elapsed;
                out.extend(office("breath", r.pan_of(h), if near { r.cues.breath_near } else { r.cues.breath_far }));
            }
        }
        if l.haelith.p >= r.tun.bed_stir_at && l.elapsed - self.last[2] > r.cues.haelith_gap_ms {
            self.last[2] = l.elapsed;
            out.extend(office("breath", 0.0, 0.3 + l.haelith.p as f32 / 300.0));
        }
        out
    }
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::NavMap;

    const DARK: SimInput = SimInput::NONE;
    const HOLD: SimInput = SimInput { hold: true, light: Light::Dark };
    const HALL: SimInput = SimInput { hold: false, light: Light::Hall };
    const BED: SimInput = SimInput { hold: false, light: Light::Bed };

    /// A neutral stream answers 0.5 everywhere: every jitter is its base, every wait the middle of its window.
    fn neutral(night: i32, levels: [Option<i32>; 3], infinite: bool) -> BedroomLogic {
        BedroomLogic::new(LogicOpts {
            night,
            custom: false,
            rng: Stream::neutral(),
            var_rng: None,
            levels,
            infinite_battery: infinite,
        })
    }

    /// Runs 10 ms steps until `pred` or `max_ms`; returns the elapsed time it happened at.
    fn run_until(l: &mut BedroomLogic, input: &SimInput, max_ms: f64, mut pred: impl FnMut(&BedroomLogic, &[SimEvent]) -> bool) -> Option<f64> {
        while l.elapsed < max_ms {
            let ev = l.step(10.0, input);
            if pred(l, &ev) {
                return Some(l.elapsed);
            }
        }
        None
    }

    #[test]
    fn the_rules_file_parses_with_the_original_numbers() {
        let r = Rules::parse(RULES_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.nights.len(), 6);
        assert_eq!(r.nights[0].ai, [3, 0, 0]);
        assert_eq!(r.nights[5].ai, [20, 12, 9]);
        assert_eq!(r.nights[3].rest, [2500.0, 5000.0]);
        assert_eq!(r.nights[1].first, [3000.0, 4500.0]);
        assert_eq!(r.nights[0].breath_ms, 3200.0);
        assert_eq!(r.nights[1].breath_ms, 2560.0);
        assert_eq!(r.nights[1].near_ms, 3360.0);
        assert_eq!(r.custom_row.ai, [0, 0, 0]);
        assert_eq!(r.custom_row.first, r.nights[0].first);
        assert_eq!(r.tun.hold_need_ms, 700.0);
        assert_eq!(r.tun.light_need_ms, 600.0);
        assert_eq!(r.tun.afk_ms, 14000.0);
        // roundi(base * 0.5625)
        let secs: Vec<i64> = (0..6).map(|n| r.flash_seconds(n)).collect();
        assert_eq!(secs, vec![68, 59, 51, 42, 34, 28]);
        assert_eq!(r.active_hunters(0), vec![Hunter::Loxelle]);
        assert_eq!(r.active_hunters(2), Hunter::ALL.to_vec());
        assert_eq!(r.name_of(Hunter::Haelith), "Haelith");
        assert_eq!(r.pan_of(Hunter::Loxelle), -0.85);
    }

    #[test]
    fn breath_then_near_then_caught_on_the_night_two_row() {
        // night 2: Loxelle level 7 (window 9500..12500 ms, middle 11000), first wait 3000, breath 2560, near 3360
        let mut l = neutral(2, [None, Some(0), Some(0)], true);
        let mut at = Vec::new();
        run_until(&mut l, &SimInput { hold: false, light: Light::Bed }, 40000.0, |_, ev| {
            for e in ev {
                at.push(e.clone());
            }
            at.iter().any(|e| matches!(e, SimEvent::Caught(_)))
        });
        assert!(at.contains(&SimEvent::Breath(Hunter::Loxelle)));
        assert!(at.contains(&SimEvent::Near(Hunter::Loxelle)));
        assert_eq!(l.over, Over::Caught);
        assert_eq!(l.by, Some(Hunter::Loxelle));
        // 3000 cool + 11000 window, then 2560 breath, then 3360 near
        let total = 3000.0 + 11000.0 + 2560.0 + 3360.0;
        assert!((l.elapsed - total).abs() <= 30.0, "caught at {} want about {}", l.elapsed, total);
    }

    #[test]
    fn night_one_has_only_loxelle() {
        let mut l = neutral(1, [None; 3], true);
        assert_eq!(l.level(Hunter::Javelle), 0);
        assert_eq!(l.level(Hunter::Haelith), 0);
        for _ in 0..2000 {
            l.step(100.0, &BED);
        }
        assert_eq!(l.doors[1].state, Stage::Away);
        assert_eq!(l.haelith.p, 0.0);
    }

    #[test]
    fn holding_the_door_turns_loxelle_away_after_700_ms() {
        let mut l = neutral(2, [None, Some(0), Some(0)], true);
        run_until(&mut l, &DARK, 40000.0, |l, _| l.doors[0].state == Stage::Breath);
        assert_eq!(l.doors[0].state, Stage::Breath);
        let start = l.elapsed;
        let mut left = None;
        while l.elapsed < start + 3000.0 && left.is_none() {
            for e in l.step(10.0, &HOLD) {
                if e == SimEvent::Leave(Hunter::Loxelle) {
                    left = Some(l.elapsed - start);
                }
            }
        }
        let t = left.expect("she leaves");
        assert!((t - 700.0).abs() <= 20.0, "left after {t}");
        assert_eq!(l.doors[0].state, Stage::Away);
        assert!(l.doors[0].cool > 0.0);
    }

    #[test]
    fn an_unanswered_stage_clock_keeps_running_and_answering_less_than_the_need_decays() {
        let mut l = neutral(2, [None, Some(0), Some(0)], true);
        run_until(&mut l, &DARK, 40000.0, |l, _| l.doors[0].state == Stage::Breath);
        for _ in 0..30 {
            l.step(10.0, &HOLD); // 300 ms
        }
        assert!((l.doors[0].need - 300.0).abs() < 1.0);
        for _ in 0..20 {
            l.step(10.0, &DARK); // 200 ms of nothing: need drops at half speed
        }
        assert!((l.doors[0].need - 200.0).abs() < 1.0, "{}", l.doors[0].need);
    }

    #[test]
    fn the_hall_light_freezes_javelles_stage_clock_and_sends_her_off_after_600_ms() {
        let mut l = neutral(2, [Some(0), None, Some(0)], true);
        run_until(&mut l, &DARK, 40000.0, |l, _| l.doors[1].state == Stage::Breath);
        assert_eq!(l.doors[1].state, Stage::Breath);
        for _ in 0..40 {
            l.step(10.0, &DARK);
        }
        let t0 = l.doors[1].t;
        assert!(t0 > 300.0);
        for _ in 0..30 {
            l.step(10.0, &HALL); // 300 ms of light: the stage clock does not move
        }
        assert_eq!(l.doors[1].t, t0);
        let mut gone = false;
        for _ in 0..40 {
            if l.step(10.0, &HALL).contains(&SimEvent::Leave(Hunter::Javelle)) {
                gone = true;
            }
        }
        assert!(gone, "600 ms of hall light turns her away");
        assert_eq!(l.doors[1].state, Stage::Away);
    }

    #[test]
    fn any_light_pauses_haelith_and_the_bed_light_drains_her_at_35_a_second() {
        let mut l = neutral(3, [Some(0), Some(0), Some(20)], true);
        // dark: she creeps
        run_until(&mut l, &DARK, 60000.0, |l, _| l.haelith.p > 0.0);
        assert!(l.haelith.p > 0.0, "she stirs in the dark");
        let p = l.haelith.p;
        // the hall light pauses her: neither the pacer nor p moves
        let (t, next) = (l.haelith.pacer.t, l.haelith.pacer.next);
        for _ in 0..300 {
            l.step(10.0, &HALL);
        }
        assert_eq!(l.haelith.p, p);
        assert_eq!((l.haelith.pacer.t, l.haelith.pacer.next), (t, next));
        // the bed light drains her
        l.haelith.p = 80.0;
        for _ in 0..100 {
            l.step(10.0, &BED); // one second
        }
        assert!((l.haelith.p - 45.0).abs() < 0.5, "{}", l.haelith.p);
        for _ in 0..200 {
            l.step(10.0, &BED);
        }
        assert_eq!(l.haelith.p, 0.0);
    }

    #[test]
    fn haelith_gains_8_plus_09_per_level_and_catches_you_at_100() {
        let mut l = neutral(3, [Some(0), Some(0), Some(10)], true);
        l.haelith.p = 92.0;
        let mut ev = Vec::new();
        run_until(&mut l, &DARK, 120000.0, |l, e| {
            ev.extend_from_slice(e);
            l.over != Over::Running
        });
        assert_eq!(l.over, Over::Caught);
        assert_eq!(l.by, Some(Hunter::Haelith));
        assert!(ev.contains(&SimEvent::Stir(100.0)));
    }

    #[test]
    fn the_house_turns_restless_after_14_seconds_of_doing_nothing() {
        let mut l = neutral(2, [None, Some(0), Some(0)], true);
        l.doors[0].cool = 1e9; // keep her out of it
        let mut at = None;
        while l.elapsed < 20000.0 && at.is_none() {
            if l.step(10.0, &DARK).contains(&SimEvent::Restless) {
                at = Some(l.elapsed);
            }
        }
        let t = at.expect("restless");
        assert!((t - 14000.0).abs() <= 20.0, "{t}");
        assert!(l.restless);
        // a hand down clears it
        l.step(10.0, &HOLD);
        assert!(!l.restless);
        assert_eq!(l.idle, 0.0);
    }

    #[test]
    fn a_hand_down_keeps_the_house_calm_and_a_quiet_night_is_never_restless() {
        let mut l = neutral(1, [Some(0), Some(0), Some(0)], true);
        for _ in 0..3000 {
            l.step(10.0, &DARK);
        }
        assert!(!l.restless, "nobody to be restless with");
    }

    #[test]
    fn restless_makes_them_keener_and_faster() {
        let mut l = neutral(2, [None, Some(0), Some(0)], true);
        l.doors[0].cool = 0.0;
        l.idle = 20000.0;
        l.step(10.0, &DARK);
        assert!(l.restless);
        // keen = 7 + 4: the pacer's first wait comes from level 11's window (7500..10500, middle 9000)
        assert_eq!(l.doors[0].pacer.next, 9000.0);
        let mut c = neutral(2, [None, Some(0), Some(0)], true);
        c.doors[0].cool = 0.0;
        c.step(10.0, &HOLD);
        assert_eq!(c.doors[0].pacer.next, 11000.0);
    }

    #[test]
    fn the_ai_steps_up_at_3_and_4_am() {
        let mut l = neutral(2, [None; 3], true);
        assert_eq!(l.level(Hunter::Loxelle), 7);
        l.elapsed = 3.0 * 45000.0;
        assert_eq!(l.level(Hunter::Loxelle), 8);
        l.elapsed = 4.0 * 45000.0;
        assert_eq!(l.level(Hunter::Loxelle), 9);
        assert_eq!(l.level(Hunter::Haelith), 0, "level 0 stays 0");
    }

    #[test]
    fn a_flat_battery_means_no_light_and_the_battery_never_comes_back() {
        let mut l = neutral(2, [Some(0), Some(0), Some(0)], false);
        for _ in 0..8000 {
            l.step(10.0, &HALL);
            if l.battery <= 0.0 {
                break;
            }
        }
        assert_eq!(l.battery, 0.0);
        l.step(10.0, &HALL);
        assert_eq!(l.light, Light::Dark);
        for _ in 0..100 {
            l.step(10.0, &DARK);
        }
        assert_eq!(l.battery, 0.0);
        // night 2's battery lasts 59 s of light
        let mut m = neutral(2, [Some(0), Some(0), Some(0)], false);
        let mut ms = 0.0;
        while m.battery > 0.0 {
            m.step(10.0, &HALL);
            ms += 10.0;
        }
        assert!((ms - 59000.0_f64).abs() < 100.0, "{ms}");
    }

    #[test]
    fn holding_the_door_means_no_light_and_one_light_at_a_time() {
        let mut l = neutral(2, [Some(0), Some(0), Some(0)], false);
        l.step(10.0, &SimInput { hold: true, light: Light::Hall });
        assert_eq!(l.light, Light::Dark);
        assert_eq!(l.battery, 100.0);
        assert!(l.hold);
    }

    #[test]
    fn god_mode_sends_whoever_would_have_got_in_back() {
        let mut l = neutral(2, [None, Some(0), Some(0)], true);
        l.god = true;
        run_until(&mut l, &DARK, 60000.0, |l, _| l.over != Over::Running);
        assert_eq!(l.over, Over::Running);
        assert!(l.elapsed >= 60000.0);
    }

    #[test]
    fn the_night_is_won_at_270_seconds() {
        let mut l = neutral(1, [Some(0), Some(0), Some(0)], true);
        let mut won = false;
        while l.over == Over::Running {
            if l.step(100.0, &DARK).contains(&SimEvent::Win) {
                won = true;
            }
        }
        assert!(won);
        assert_eq!(l.over, Over::Win);
        assert_eq!(l.elapsed, NIGHT_MS);
        assert!(l.step(100.0, &DARK).is_empty(), "nothing happens after the end");
    }

    #[test]
    fn a_seed_can_swap_who_comes_first() {
        let mut swapped = None;
        for seed in 1..400u32 {
            let l = BedroomLogic::new(LogicOpts::seeded(2, seed));
            if l.swapped {
                swapped = Some(seed);
                break;
            }
        }
        let seed = swapped.expect("some seed swaps the first visitors");
        let l = BedroomLogic::new(LogicOpts::seeded(2, seed));
        // night 2: Loxelle's first wait is 3000, Javelle's 4500; swapped, Loxelle's is 4500 +/- 25%
        assert!(l.doors[0].cool >= 4500.0 * 0.75 - 1.0 && l.doors[0].cool <= 4500.0 * 1.25 + 1.0, "{}", l.doors[0].cool);
        // custom nights have no level in the row, so a swap needs both sliders above 0
        let c = BedroomLogic::new(LogicOpts { night: 5, custom: true, rng: Stream::seeded(seed), var_rng: None, levels: [Some(5), Some(0), Some(0)], infinite_battery: false });
        assert!(!c.swapped);
    }

    #[test]
    fn a_custom_night_uses_night_ones_timings_and_the_sliders() {
        let l = BedroomLogic::new(LogicOpts { night: 5, custom: true, rng: Stream::seeded(7), var_rng: None, levels: [Some(9), Some(0), Some(4)], infinite_battery: false });
        assert_eq!(l.row.breath_ms, 3200.0);
        assert_eq!(l.base_level(Hunter::Loxelle), 9);
        assert_eq!(l.base_level(Hunter::Javelle), 0);
        assert_eq!(l.base_level(Hunter::Haelith), 4);
        // night 5's battery (index 4): 60 s of base * 0.5625 = 34 s
        assert!((l.flash_per_sec - 100.0 / 34.0).abs() < 1e-9);
        // a slider of -1 keeps the row
        let k = BedroomLogic::new(LogicOpts { night: 5, custom: true, rng: Stream::seeded(7), var_rng: None, levels: [Some(-1), None, None], infinite_battery: false });
        assert_eq!(k.base_level(Hunter::Loxelle), 0);
    }

    // ---- the living house

    fn sim(night: i32, seed: u32) -> Sim {
        Sim::new(&SimOpts { night, seed, custom: false, levels: BTreeMap::new(), infinite_battery: true, snap: false })
    }

    #[test]
    fn the_nav_graph_is_valid_and_every_room_is_reachable_from_the_living_room() {
        let nav = NavMap::from_json(NAV_JSON).unwrap_or_else(|e| panic!("{e}"));
        let problems = nav.validate();
        assert!(problems.is_empty(), "{problems:?}");
        let map = map();
        for r in &map.rooms {
            let hub = nav.hub_key(&r.id).unwrap_or_else(|| panic!("no hub in {}", r.id));
            assert!(nav.distance("C:living_room", &hub).is_some(), "{} is cut off", r.id);
            assert!(nav.rooms().iter().any(|n| n.id == r.id), "{}", r.id);
        }
        // the three places Javelle stands exist and connect
        for (room, slot) in [("hall", 0usize), ("hall", 1), ("living_room", 0)] {
            assert!(nav.stand_node(room, slot).is_some(), "{room} {slot}");
        }
        let route = nav.route("C:living_room", "H:near");
        assert!(route.len() >= 4, "{route:?}");
        let keys: Vec<&str> = route.iter().map(|l| l.key.as_str()).collect();
        assert!(keys.contains(&"DW:hall|living_room") && keys.contains(&"H:far") && keys.contains(&"H:mid"), "{keys:?}");
        // the Godot doorway points are the nav doorway nodes
        for (a, b) in &map.edges {
            let d = map.door_between(a, b).expect("door");
            let n = nav.node(&format!("DW:{a}|{b}")).unwrap_or_else(|| panic!("no node DW:{a}|{b}"));
            assert!((n.p.x - d.point.0).abs() < 1e-3 && (n.p.z - d.point.1).abs() < 1e-3, "{a}|{b}");
        }
    }

    #[test]
    fn every_task_spot_connects_and_has_a_task_that_behaviour_data_knows() {
        let nav = NavMap::from_json(NAV_JSON).unwrap_or_else(|e| panic!("{e}"));
        let data = BehaviourData::embedded();
        let spots = nav.task_spots("living_room", None);
        assert!(spots.len() >= 4, "{}", spots.len());
        for s in spots {
            let t = s.task.as_deref().unwrap_or("");
            assert!(data.tasks.contains_key(t), "{t}");
            assert!(nav.distance("C:living_room", &s.key).is_some(), "{} is cut off", s.key);
            assert!(map().room_at(s.p.x, s.p.z) == Some("living_room"), "{}", s.key);
        }
    }

    #[test]
    fn javelle_walks_up_the_hall_instead_of_appearing_and_walks_home_again() {
        let mut s = sim(2, 11);
        let home = s.pos_of(Hunter::Javelle);
        assert!(home.x > 10.7, "she starts in the living room");
        s.logic.admin_place_for_test(Hunter::Javelle, Stage::Breath);
        let mut last = home;
        let mut max_step = 0.0f32;
        let mut seen_hall_mid = false;
        let mut was_in_door = false;
        for _ in 0..400 {
            // hold the breath stage (its timer would otherwise send her on to Near within 2.6 s)
            s.logic.admin_place_for_test(Hunter::Javelle, Stage::Breath);
            s.step(10.0, &SimInput::NONE);
            let p = s.pos_of(Hunter::Javelle);
            max_step = max_step.max(p.distance(last));
            last = p;
            if p.x < 9.0 && p.x > 5.0 && s.crowd().agent("javelle").map(|a| a.walker.room() == "hall").unwrap_or(false) {
                seen_hall_mid = true;
            }
            if (p.x - 10.7).abs() < 0.3 {
                was_in_door = true;
            }
            // a 10 ms step at the Godot clamp (6 m/s at most) never moves more than 6 cm
            assert!(max_step <= 0.0605, "she teleported: {max_step}");
        }
        assert!(was_in_door, "she crossed the doorway");
        assert!(seen_hall_mid);
        let mid = map().hall_nodes.iter().find(|(k, _)| k == "mid").map(|(_, x)| *x).unwrap_or(6.9);
        let p = s.pos_of(Hunter::Javelle);
        assert!((p.x - mid).abs() < 0.1, "she stands at the mid waypoint: {}", p.x);
        assert_eq!(s.crowd().agent("javelle").map(|a| a.walker.room().to_string()), Some("hall".to_string()));
        // near, then away: she walks home
        s.logic.admin_place_for_test(Hunter::Javelle, Stage::Near);
        for _ in 0..400 {
            s.logic.admin_place_for_test(Hunter::Javelle, Stage::Near);
            s.step(10.0, &SimInput::NONE);
        }
        assert!((s.pos_of(Hunter::Javelle).x - 3.4).abs() < 0.1);
        s.logic.admin_place_for_test(Hunter::Javelle, Stage::Away);
        for _ in 0..600 {
            s.step(10.0, &SimInput::NONE);
        }
        assert!(s.pos_of(Hunter::Javelle).x > 10.7, "home again: {}", s.pos_of(Hunter::Javelle).x);
    }

    #[test]
    fn javelle_may_work_in_the_living_room_while_she_is_away_but_never_while_she_breathes() {
        let quiet: BTreeMap<String, i32> = Hunter::ALL.iter().map(|h| (h.id().to_string(), 0)).collect();
        let mut s = Sim::new(&SimOpts { night: 5, seed: 5, custom: true, levels: quiet, infinite_battery: true, snap: false });
        let mut tasks = 0;
        for _ in 0..30000 {
            for e in s.step(20.0, &SimInput::NONE) {
                if let SimEvent::Task(id, _) = e {
                    assert_eq!(id, "javelle");
                    tasks += 1;
                }
            }
        }
        assert!(tasks > 0, "she does something in 10 simulated minutes");
        // once the rules put her at a stage, her brain is off
        s.logic.admin_place_for_test(Hunter::Javelle, Stage::Breath);
        s.step(10.0, &SimInput::NONE);
        assert!(!s.crowd().agent("javelle").map(|a| a.brain.enabled()).unwrap_or(true));
        s.logic.admin_place_for_test(Hunter::Javelle, Stage::Away);
        s.step(10.0, &SimInput::NONE);
        assert!(s.crowd().agent("javelle").map(|a| a.brain.enabled()).unwrap_or(false));
    }

    #[test]
    fn snap_puts_her_at_the_stand_at_once() {
        let mut s = Sim::new(&SimOpts { night: 2, seed: 3, custom: false, levels: BTreeMap::new(), infinite_battery: true, snap: true });
        s.logic.admin_place_for_test(Hunter::Javelle, Stage::Near);
        s.step(10.0, &SimInput::NONE);
        let p = s.pos_of(Hunter::Javelle);
        assert!((p.x - 3.4).abs() < 1e-3 && (p.z + 0.9).abs() < 1e-3, "{p:?}");
        s.logic.haelith.p = 50.0;
        s.step(10.0, &SimInput::NONE);
        let h = s.pos_of(Hunter::Haelith);
        let want = map().creep_position(50.0);
        assert!(h.distance(want) < 1e-3, "{h:?} vs {want:?}");
    }

    #[test]
    fn haelith_shuffles_along_the_creep_line_as_her_number_rises() {
        let mut s = sim(3, 9);
        s.logic.haelith.p = 60.0;
        for _ in 0..600 {
            s.step(10.0, &SimInput::NONE);
        }
        let want = map().creep_position(s.logic.haelith.p as f32);
        let h = s.pos_of(Hunter::Haelith);
        assert!(h.distance(want) < 0.3, "{h:?} vs {want:?}");
        // she starts in the north-west corner
        let s2 = sim(3, 9);
        assert!(s2.pos_of(Hunter::Haelith).distance(map().creep_from) < 1e-4);
    }

    #[test]
    fn a_catch_puts_the_hunter_where_the_scare_goes() {
        let mut s = sim(2, 21);
        s.logic.over = Over::Caught;
        s.logic.by = Some(Hunter::Loxelle);
        s.step(10.0, &SimInput::NONE);
        let p = s.pos_of(Hunter::Loxelle);
        assert!((p.x + 3.0).abs() < 0.05 && (p.z + 0.9).abs() < 0.05, "{p:?}");
        assert_eq!(s.scare_light(Hunter::Loxelle), Light::Bed);
        assert_eq!(s.scare_light(Hunter::Javelle), Light::Hall);
        assert_eq!(s.outcome(), Outcome::Lost);
    }

    // ---- headless

    fn headless(night: i32, seed: u32) -> BedroomHeadless {
        let mut sess = Session::new();
        sess.start_mode("bedroom", night, BTreeMap::new(), Some(seed));
        BedroomHeadless::new(&sess)
    }

    fn run_night(night: i32, seed: u32) -> (Outcome, u64, usize) {
        let mut h = headless(night, seed);
        let mut steps = 0;
        while h.outcome() == Outcome::Running && steps < 4000 {
            h.step_ms(100.0);
            steps += 1;
        }
        (h.outcome(), h.digest(), h.seen)
    }

    #[test]
    fn a_seeded_night_reaches_an_outcome_on_every_night() {
        for night in 1..=6 {
            let (o, _, _) = run_night(night, 12345);
            assert_ne!(o, Outcome::Running, "night {night}");
        }
    }

    #[test]
    fn the_scripted_player_wins_night_one() {
        let (o, _, seen) = run_night(1, 12345);
        assert_eq!(o, Outcome::Won);
        assert!(seen > 0, "something happens in a night");
    }

    #[test]
    fn same_seed_same_night_different_seed_different_night() {
        let a = run_night(3, 12345);
        let b = run_night(3, 12345);
        assert_eq!(a, b, "deterministic");
        let mut differs = false;
        for seed in [1u32, 2, 3, 4, 5] {
            if run_night(3, seed).1 != a.1 {
                differs = true;
            }
        }
        assert!(differs, "the seed matters");
    }

    #[test]
    fn headless_reports_its_identity_and_clock() {
        let mut h = headless(2, 1);
        assert_eq!(h.id(), "bedroom");
        assert_eq!(h.elapsed_ms(), 0.0);
        h.step_ms(100.0);
        assert_eq!(h.elapsed_ms(), 100.0);
        assert_eq!(h.outcome(), Outcome::Running);
    }

    #[test]
    fn cues_follow_the_side_each_hunter_comes_from() {
        let mut c = CueClock::new();
        let e = c.on_event(&SimEvent::Breath(Hunter::Loxelle), 1000.0);
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].name, e[0].pan, e[0].intensity), ("step", -0.85, 0.5));
        let e = c.on_event(&SimEvent::Near(Hunter::Javelle), 2000.0);
        assert_eq!((e[0].name, e[0].pan), ("breath", 0.85));
        assert!((e[0].intensity - 0.7).abs() < 1e-6, "breath is 0.7 of the asked volume");
        let e = c.on_event(&SimEvent::Leave(Hunter::Javelle), 3000.0);
        assert_eq!(e[0].name, "step");
        assert!((e[0].intensity - 0.45).abs() < 1e-6);
        assert_eq!(c.on_event(&SimEvent::Stir(10.0), 0.0)[0].name, "clank");
        assert_eq!(c.on_event(&SimEvent::Restless, 0.0)[0].name, "crackle");
        assert!(c.on_event(&SimEvent::Win, 0.0).is_empty());
        assert!(c.on_event(&SimEvent::Task("javelle".into(), "sits".into()), 0.0).is_empty());
    }

    #[test]
    fn breathing_repeats_on_the_original_cadence() {
        let mut l = neutral(2, [None, Some(0), Some(0)], true);
        let mut c = CueClock::new();
        assert!(c.breathing(&l).is_empty(), "nothing while she is away");
        l.doors[0].state = Stage::Breath;
        l.elapsed = 5000.0;
        assert_eq!(c.breathing(&l).len(), 1);
        l.elapsed = 6000.0;
        assert!(c.breathing(&l).is_empty(), "1.7 s between breaths");
        l.elapsed = 6800.0;
        assert_eq!(c.breathing(&l).len(), 1);
        l.doors[0].state = Stage::Near;
        l.elapsed = 7850.0;
        assert!(c.breathing(&l).is_empty());
        l.elapsed = 8000.0;
        let near = c.breathing(&l);
        assert_eq!(near.len(), 1, "1.1 s between breaths when she is near");
        assert!((near[0].intensity - 0.63).abs() < 1e-5);
        // Haelith breathes once she is past 45 percent, every 2.4 s, louder as she comes
        l.doors[0].state = Stage::Away;
        l.haelith.p = 30.0;
        l.elapsed = 20000.0;
        assert!(c.breathing(&l).is_empty());
        l.haelith.p = 60.0;
        let b = c.breathing(&l);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].pan, 0.0);
        l.elapsed = 21000.0;
        assert!(c.breathing(&l).is_empty());
        l.elapsed = 22500.0;
        assert_eq!(c.breathing(&l).len(), 1);
    }

    #[test]
    fn what_the_pillow_can_see() {
        let mut s = sim(3, 4);
        // Loxelle is behind the closet door, Javelle in the living room, Haelith has not stirred
        for h in Hunter::ALL {
            assert!(!hunter_visible(&s, h, false), "{h:?}");
            assert!(hunter_visible(&s, h, true), "{h:?} shows under the admin reveal");
        }
        s.logic.haelith.p = 9.0;
        assert!(!hunter_visible(&s, Hunter::Haelith, false));
        s.logic.haelith.p = 10.0;
        assert!(hunter_visible(&s, Hunter::Haelith, false));
        // Javelle shows once she is at a stage
        s.logic.admin_place_for_test(Hunter::Javelle, Stage::Breath);
        s.step(10.0, &SimInput::NONE);
        assert!(hunter_visible(&s, Hunter::Javelle, false));
        // a hunter who is not on the floor is never shown
        let q = Sim::new(&SimOpts { night: 1, seed: 4, custom: false, levels: BTreeMap::new(), infinite_battery: true, snap: false });
        assert!(!hunter_visible(&q, Hunter::Haelith, false));
        let mut q = q;
        q.logic.haelith.p = 50.0;
        assert!(!hunter_visible(&q, Hunter::Haelith, false), "level 0 on night 1");
        // the hunter who caught you is shown
        let mut w = sim(2, 4);
        w.logic.over = Over::Caught;
        w.logic.by = Some(Hunter::Loxelle);
        w.settle_catch();
        assert!(hunter_visible(&w, Hunter::Loxelle, false));
    }

    #[test]
    fn the_active_hunter_table_matches_the_nights() {
        let r = rules();
        let counts: Vec<usize> = (0..6).map(|n| r.active_hunters(n).len()).collect();
        assert_eq!(counts, vec![1, 2, 3, 3, 3, 3]);
        assert!(r.bed_at_p > 0.0);
    }
}

impl BedroomLogic {
    /// Test helper: put a door threat at a stage without the rules' own transitions.
    #[cfg(test)]
    pub fn admin_place_for_test(&mut self, h: Hunter, stage: Stage) {
        if let Some(th) = self.door_mut(h) {
            th.state = stage;
            th.t = 0.0;
            th.need = 0.0;
            th.jitter = 1.0;
            if stage == Stage::Away {
                th.cool = 1e9;
            }
        }
    }
}
