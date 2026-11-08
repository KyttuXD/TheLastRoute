//! Mixed Shift's constants (port of `mixed_data.gd`): the pool of threats, the five shifts, every tuning number, the
//! routes through the transit hub and the vent run, the chaos events and the music box. The numbers live in
//! `data/mixed/tuning.json` (embedded here); this file parses them into plain typed structs. Pure data, no drawing.
#![allow(dead_code)]

use serde_json::Value;
use std::sync::OnceLock;

pub const TUNING_JSON: &str = include_str!("../../data/mixed/tuning.json");

pub const HOUR_MS: f64 = 45_000.0;
pub const NIGHT_MS: f64 = 270_000.0;

// ------------------------------------------------------------------------------------------------ small enums

/// The six threat types.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Counter,
    Watched,
    Monitor,
    Path,
    Vent,
    Winder,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Counter, Kind::Watched, Kind::Monitor, Kind::Path, Kind::Vent, Kind::Winder];

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "counter" => Kind::Counter,
            "watched" => Kind::Watched,
            "monitor" => Kind::Monitor,
            "path" => Kind::Path,
            "vent" => Kind::Vent,
            "winder" => Kind::Winder,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Counter => "counter",
            Kind::Watched => "watched",
            Kind::Monitor => "monitor",
            Kind::Path => "path",
            Kind::Vent => "vent",
            Kind::Winder => "winder",
        }
    }

    pub fn idx(self) -> usize {
        self as usize
    }
}

/// A door side (L, R) or a vent (L, R, T).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    L,
    R,
    T,
}

impl Side {
    pub const ALL: [Side; 3] = [Side::L, Side::R, Side::T];

    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "L" => Some(Side::L),
            "R" => Some(Side::R),
            "T" => Some(Side::T),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Side::L => "L",
            Side::R => "R",
            Side::T => "T",
        }
    }

    pub fn idx(self) -> usize {
        self as usize
    }

    /// "left" / "right" / "top" (VENT_NAME).
    pub fn word(self) -> &'static str {
        match self {
            Side::L => "left",
            Side::R => "right",
            Side::T => "top",
        }
    }

    /// The office door of a side ("doorL" / "doorR"); None for the top vent.
    pub fn door(self) -> Option<&'static str> {
        match self {
            Side::L => Some("doorL"),
            Side::R => Some("doorR"),
            Side::T => None,
        }
    }

    pub fn vent_room(self) -> &'static str {
        match self {
            Side::L => "ventL",
            Side::R => "ventR",
            Side::T => "ventT",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Patient,
    Hungry,
    Shy,
    Reckless,
}

impl Mood {
    pub const ALL: [Mood; 4] = [Mood::Patient, Mood::Hungry, Mood::Shy, Mood::Reckless];

    pub fn name(self) -> &'static str {
        match self {
            Mood::Patient => "patient",
            Mood::Hungry => "hungry",
            Mood::Shy => "shy",
            Mood::Reckless => "reckless",
        }
    }

    pub fn idx(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Beat {
    Lull,
    Build,
    Spike,
    Breather,
}

impl Beat {
    pub fn name(self) -> &'static str {
        match self {
            Beat::Lull => "lull",
            Beat::Build => "build",
            Beat::Spike => "spike",
            Beat::Breather => "breather",
        }
    }

    pub fn idx(self) -> usize {
        self as usize
    }
}

// ------------------------------------------------------------------------------------------------ tuning structs

/// One shift of the board (QUIET, STEADY, RESTLESS, CHAOS, ENDLESS).
#[derive(Clone, Debug)]
pub struct Shift {
    pub id: String,
    pub name: String,
    pub speed: f64,
    pub tell_ms: f64,
    pub min_tell_ms: f64,
    pub start_ms: f64,
    pub roster: (i32, i32),
    pub spikes: (i32, i32),
    pub grace_ms: f64,
    pub new_max: i32,
    pub must: Vec<String>,
    pub min_types: i32,
    pub breather: Option<(f64, f64)>,
    pub chaos: bool,
    pub endless: bool,
}

#[derive(Clone, Debug)]
pub struct PoolDef {
    pub id: String,
    pub kind: Kind,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct TypeText {
    pub label: String,
    pub does: String,
    pub answer: String,
}

/// A route through the building or the vent run; the last room before the gate is the choke.
#[derive(Clone, Debug)]
pub struct Route {
    pub id: String,
    pub side: Side,
    pub rooms: Vec<String>,
    /// Vent routes: the spot in the vent run of each step.
    pub spots: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Timing {
    pub step_ms: f64,
    pub rest_ms: f64,
    pub grace_ms: f64,
    pub span_ms: f64,
    pub arm_at: f64,
    pub cue_at: f64,
    pub calm_decay: f64,
    pub arrive_ms: f64,
    pub idle_leave_ms: f64,
    pub steps_to_arm: i32,
    pub decay_ms: f64,
    pub cranky_ms: f64,
}

#[derive(Clone, Debug)]
pub struct ChaosDef {
    pub id: String,
    pub weight: f64,
    pub ms: f64,
    pub needs: Option<Kind>,
    pub hold_ms: f64,
    pub regen: f64,
    pub factor: f64,
}

#[derive(Clone, Debug)]
pub struct Ramp {
    pub pace_from: i32,
    pub pace_step: f64,
    pub pace_cap: f64,
    pub tell_from: i32,
    pub tell_step: f64,
    pub tell_min: f64,
    pub gap_start: f64,
    pub gap_step: f64,
    pub gap_min: f64,
    pub rotate_ms: f64,
    pub roster_by_gen: Vec<(i32, i32)>,
    pub floor_from: f64,
    pub floor_max: f64,
    pub floor_span: f64,
    pub budget_from: i32,
    pub budget_step: f64,
    pub budget_min: f64,
    pub event_from: i32,
    pub event_p: f64,
}

#[derive(Clone, Debug)]
pub struct Tuning {
    // fairness
    pub tell_floor_ms: f64,
    pub cooldown_ms: f64,
    pub tell_jitter: f64,
    pub jitter: f64,
    pub give_up_ms: f64,
    pub choke_ms: f64,
    pub end_clear_ms: f64,
    pub min_dwell_ms: f64,
    pub cue_foot_ms: f64,
    // doors
    pub seal_max: f64,
    pub seal_regen: f64,
    pub seal_min_to_shut: f64,
    pub seal_reserve: f64,
    pub door_hold_ms: f64,
    pub pop_cool_ms: f64,
    // vents
    pub vent_seal_max: f64,
    pub vent_seal_regen: f64,
    pub vent_min_to_seal: f64,
    pub vent_reserve: f64,
    pub vent_hold_ms: f64,
    pub vent_linger_ms: f64,
    pub vent_pop_cool_ms: f64,
    pub vent_light_ms: f64,
    pub vent_light_cool_ms: f64,
    // moods
    pub mood_min_ms: f64,
    pub mood_max_ms: f64,
    pub mood_speed: [f64; 4],
    pub mood_tell: [f64; 4],
    pub mood_cue_ms: (f64, f64),
    // the director
    pub spike_len_ms: (f64, f64),
    pub spike_warn_ms: f64,
    pub breather_ms: (f64, f64),
    pub spike_first_ms: f64,
    pub beat_rate: [f64; 4],
    pub lull_len: (f64, f64),
    pub build_len: (f64, f64),
    pub lull_feint: (f64, f64),
    pub build_feint: (f64, f64),
    pub tension_floor_from: f64,
    pub tension_floor: (f64, f64),
    pub max_walkers: usize,
    pub chaos_breather_floor_ms: f64,
    pub shifts: Vec<Shift>,
    // endless
    pub endless_seal_max: f64,
    pub endless_seal_regen: f64,
    pub endless_budget: f64,
    pub ramp: Ramp,
    pub endless_max_ms: f64,
    // chaos
    pub chaos_events: Vec<ChaosDef>,
    pub decoy_quiet_ms: f64,
    pub decoy_types: Vec<Kind>,
    pub chaos_hours: (i32, i32),
    // threats
    pub type_order: Vec<Kind>,
    pub types: [TypeText; 6],
    pub pool: Vec<PoolDef>,
    pub walker_types: Vec<Kind>,
    pub new_types: Vec<Kind>,
    pub timing: [Timing; 6],
    /// Per threat type: the pulse kind name and its (start, end) period in ms.
    pub tell_kind: [String; 6],
    pub pulse: [(f64, f64); 6],
    pub tune_span_ms: f64,
    pub wind_per_sec: f64,
    pub restart_at: f64,
    pub slow_at: f64,
    pub grace_floor: f64,
    pub hum_ms: (f64, f64),
    pub box_rooms: Vec<String>,
    pub cam_rooms: Vec<String>,
    pub routes: Vec<Route>,
    pub vent_routes: Vec<Route>,
    /// Vent spots near each vent (index by `Side::idx`).
    pub vent_near: [Vec<usize>; 3],
}

fn num(v: &Value, path: &[&str]) -> Result<f64, String> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p).ok_or_else(|| format!("mixed tuning: missing {}", path.join(".")))?;
    }
    cur.as_f64().ok_or_else(|| format!("mixed tuning: {} is not a number", path.join(".")))
}

fn opt_num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0)
}

fn pair(v: &Value, path: &[&str]) -> Result<(f64, f64), String> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p).ok_or_else(|| format!("mixed tuning: missing {}", path.join(".")))?;
    }
    let a = cur.as_array().ok_or_else(|| format!("mixed tuning: {} is not a pair", path.join(".")))?;
    if a.len() < 2 {
        return Err(format!("mixed tuning: {} needs two numbers", path.join(".")));
    }
    Ok((a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)))
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

fn kinds(v: &Value, key: &str) -> Result<Vec<Kind>, String> {
    strings(v, key)
        .iter()
        .map(|s| Kind::parse(s).ok_or_else(|| format!("mixed tuning: unknown type '{s}' in {key}")))
        .collect()
}

fn routes(v: &Value, key: &str) -> Result<Vec<Route>, String> {
    let list = v.get(key).and_then(|r| r.as_array()).ok_or_else(|| format!("mixed tuning: no {key}"))?;
    let mut out = Vec::new();
    for r in list {
        let id = r.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let side = Side::parse(r.get("side").and_then(|x| x.as_str()).unwrap_or("")).ok_or_else(|| format!("mixed tuning: route {id} has no side"))?;
        let rooms = strings(r, "rooms");
        if rooms.len() < 3 {
            return Err(format!("mixed tuning: route {id} is too short"));
        }
        let spots = r.get("spots").and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as usize).collect()).unwrap_or_default();
        out.push(Route { id, side, rooms, spots });
    }
    Ok(out)
}

impl Tuning {
    pub fn parse(src: &str) -> Result<Tuning, String> {
        let v: Value = serde_json::from_str(src).map_err(|e| format!("mixed tuning: {e}"))?;
        let mut mood_speed = [1.0; 4];
        let mut mood_tell = [1.0; 4];
        for m in Mood::ALL {
            mood_speed[m.idx()] = num(&v, &["moods", "speed", m.name()])?;
            mood_tell[m.idx()] = num(&v, &["moods", "tell", m.name()])?;
        }
        let mut beat_rate = [0.0; 4];
        for b in [Beat::Lull, Beat::Build, Beat::Spike, Beat::Breather] {
            beat_rate[b.idx()] = num(&v, &["director", "beat_rate", b.name()])?;
        }
        let mut shifts = Vec::new();
        for s in v.get("shifts").and_then(|s| s.as_array()).ok_or("mixed tuning: no shifts")? {
            let roster = pair(s, &["roster"])?;
            let spikes = pair(s, &["spikes"])?;
            shifts.push(Shift {
                id: s.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                name: s.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                speed: num(s, &["speed"])?,
                tell_ms: num(s, &["tell_ms"])?,
                min_tell_ms: opt_num(s, "min_tell_ms"),
                start_ms: num(s, &["start_ms"])?,
                roster: (roster.0 as i32, roster.1 as i32),
                spikes: (spikes.0 as i32, spikes.1 as i32),
                grace_ms: num(s, &["grace_ms"])?,
                new_max: num(s, &["new_max"])? as i32,
                must: strings(s, "must"),
                min_types: s.get("min_types").and_then(|x| x.as_i64()).unwrap_or(3) as i32,
                breather: pair(s, &["breather_ms"]).ok(),
                chaos: s.get("chaos").and_then(|x| x.as_bool()).unwrap_or(false),
                endless: s.get("endless").and_then(|x| x.as_bool()).unwrap_or(false),
            });
        }
        if shifts.len() != 5 {
            return Err(format!("mixed tuning: expected 5 shifts, found {}", shifts.len()));
        }
        let mut roster_by_gen = Vec::new();
        for g in v["endless"]["ramp"]["roster_by_gen"].as_array().ok_or("mixed tuning: no roster_by_gen")? {
            let a = g.as_array().ok_or("mixed tuning: bad roster_by_gen")?;
            roster_by_gen.push((a[0].as_i64().unwrap_or(3) as i32, a[1].as_i64().unwrap_or(4) as i32));
        }
        let ramp = Ramp {
            pace_from: num(&v, &["endless", "ramp", "pace_from"])? as i32,
            pace_step: num(&v, &["endless", "ramp", "pace_step"])?,
            pace_cap: num(&v, &["endless", "ramp", "pace_cap"])?,
            tell_from: num(&v, &["endless", "ramp", "tell_from"])? as i32,
            tell_step: num(&v, &["endless", "ramp", "tell_step"])?,
            tell_min: num(&v, &["endless", "ramp", "tell_min"])?,
            gap_start: num(&v, &["endless", "ramp", "gap_start"])?,
            gap_step: num(&v, &["endless", "ramp", "gap_step"])?,
            gap_min: num(&v, &["endless", "ramp", "gap_min"])?,
            rotate_ms: num(&v, &["endless", "ramp", "rotate_ms"])?,
            roster_by_gen,
            floor_from: num(&v, &["endless", "ramp", "floor_from"])?,
            floor_max: num(&v, &["endless", "ramp", "floor_max"])?,
            floor_span: num(&v, &["endless", "ramp", "floor_span"])?,
            budget_from: num(&v, &["endless", "ramp", "budget_from"])? as i32,
            budget_step: num(&v, &["endless", "ramp", "budget_step"])?,
            budget_min: num(&v, &["endless", "ramp", "budget_min"])?,
            event_from: num(&v, &["endless", "ramp", "event_from"])? as i32,
            event_p: num(&v, &["endless", "ramp", "event_p"])?,
        };
        let mut chaos_events = Vec::new();
        for id in strings(&v, "chaos_order") {
            let e = v["chaos_events"].get(&id).ok_or_else(|| format!("mixed tuning: no chaos event {id}"))?;
            let needs = e.get("needs").and_then(|x| x.as_str()).unwrap_or("");
            chaos_events.push(ChaosDef {
                id: id.clone(),
                weight: num(e, &["weight"])?,
                ms: num(e, &["ms"])?,
                needs: if needs.is_empty() { None } else { Kind::parse(needs) },
                hold_ms: opt_num(e, "hold_ms"),
                regen: opt_num(e, "regen"),
                factor: opt_num(e, "factor"),
            });
        }
        let ch = pair(&v, &["chaos_hours"])?;
        let blank = || TypeText { label: String::new(), does: String::new(), answer: String::new() };
        let mut types = [blank(), blank(), blank(), blank(), blank(), blank()];
        for k in Kind::ALL {
            let t = v["types"].get(k.name()).ok_or_else(|| format!("mixed tuning: no text for {}", k.name()))?;
            types[k.idx()] = TypeText {
                label: t["label"].as_str().unwrap_or("").to_string(),
                does: t["does"].as_str().unwrap_or("").to_string(),
                answer: t["answer"].as_str().unwrap_or("").to_string(),
            };
        }
        let mut pool = Vec::new();
        for p in v.get("pool").and_then(|p| p.as_array()).ok_or("mixed tuning: no pool")? {
            let kind = Kind::parse(p["type"].as_str().unwrap_or("")).ok_or("mixed tuning: a pool entry has an unknown type")?;
            pool.push(PoolDef { id: p["id"].as_str().unwrap_or("").to_string(), kind, name: p["name"].as_str().unwrap_or("").to_string() });
        }
        let mut timing: [Timing; 6] = Default::default();
        for k in Kind::ALL {
            let t = v["timing"].get(k.name()).ok_or_else(|| format!("mixed tuning: no timing for {}", k.name()))?;
            timing[k.idx()] = Timing {
                step_ms: opt_num(t, "step_ms"),
                rest_ms: opt_num(t, "rest_ms"),
                grace_ms: opt_num(t, "grace_ms"),
                span_ms: opt_num(t, "span_ms"),
                arm_at: opt_num(t, "arm_at"),
                cue_at: opt_num(t, "cue_at"),
                calm_decay: opt_num(t, "calm_decay"),
                arrive_ms: opt_num(t, "arrive_ms"),
                idle_leave_ms: opt_num(t, "idle_leave_ms"),
                steps_to_arm: opt_num(t, "steps_to_arm") as i32,
                decay_ms: opt_num(t, "decay_ms"),
                cranky_ms: opt_num(t, "cranky_ms"),
            };
        }
        let mut tell_kind: [String; 6] = Default::default();
        let mut pulse = [(0.0, 0.0); 6];
        for k in Kind::ALL {
            let name = v["tell_kind"][k.name()].as_str().ok_or_else(|| format!("mixed tuning: no tell kind for {}", k.name()))?.to_string();
            pulse[k.idx()] = pair(&v, &["pulse_ms", name.as_str()])?;
            tell_kind[k.idx()] = name;
        }
        let mut vent_near: [Vec<usize>; 3] = Default::default();
        for s in Side::ALL {
            vent_near[s.idx()] = v["vent_near"][s.key()].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as usize).collect()).unwrap_or_default();
        }
        let t = Tuning {
            tell_floor_ms: num(&v, &["fair", "tell_floor_ms"])?,
            cooldown_ms: num(&v, &["fair", "cooldown_ms"])?,
            tell_jitter: num(&v, &["fair", "tell_jitter"])?,
            jitter: num(&v, &["fair", "jitter"])?,
            give_up_ms: num(&v, &["fair", "give_up_ms"])?,
            choke_ms: num(&v, &["fair", "choke_ms"])?,
            end_clear_ms: num(&v, &["fair", "end_clear_ms"])?,
            min_dwell_ms: num(&v, &["fair", "min_dwell_ms"])?,
            cue_foot_ms: num(&v, &["fair", "cue_foot_ms"])?,
            seal_max: num(&v, &["door", "seal_max"])?,
            seal_regen: num(&v, &["door", "seal_regen"])?,
            seal_min_to_shut: num(&v, &["door", "seal_min_to_shut"])?,
            seal_reserve: num(&v, &["door", "seal_reserve"])?,
            door_hold_ms: num(&v, &["door", "hold_ms"])?,
            pop_cool_ms: num(&v, &["door", "pop_cool_ms"])?,
            vent_seal_max: num(&v, &["vent", "seal_max"])?,
            vent_seal_regen: num(&v, &["vent", "seal_regen"])?,
            vent_min_to_seal: num(&v, &["vent", "min_to_seal"])?,
            vent_reserve: num(&v, &["vent", "reserve"])?,
            vent_hold_ms: num(&v, &["vent", "hold_ms"])?,
            vent_linger_ms: num(&v, &["vent", "linger_ms"])?,
            vent_pop_cool_ms: num(&v, &["vent", "pop_cool_ms"])?,
            vent_light_ms: num(&v, &["vent", "light_ms"])?,
            vent_light_cool_ms: num(&v, &["vent", "light_cool_ms"])?,
            mood_min_ms: num(&v, &["moods", "min_ms"])?,
            mood_max_ms: num(&v, &["moods", "max_ms"])?,
            mood_speed,
            mood_tell,
            mood_cue_ms: pair(&v, &["moods", "cue_ms"])?,
            spike_len_ms: pair(&v, &["director", "spike_len_ms"])?,
            spike_warn_ms: num(&v, &["director", "spike_warn_ms"])?,
            breather_ms: pair(&v, &["director", "breather_ms"])?,
            spike_first_ms: num(&v, &["director", "spike_first_ms"])?,
            beat_rate,
            lull_len: pair(&v, &["director", "beat_len", "lull"])?,
            build_len: pair(&v, &["director", "beat_len", "build"])?,
            lull_feint: pair(&v, &["director", "feint_ms", "lull"])?,
            build_feint: pair(&v, &["director", "feint_ms", "build"])?,
            tension_floor_from: num(&v, &["director", "tension_floor_from"])?,
            tension_floor: pair(&v, &["director", "tension_floor"])?,
            max_walkers: num(&v, &["max_walkers"])? as usize,
            chaos_breather_floor_ms: num(&v, &["chaos_breather_floor_ms"])?,
            shifts,
            endless_seal_max: num(&v, &["endless", "seal", "max"])?,
            endless_seal_regen: num(&v, &["endless", "seal", "regen"])?,
            endless_budget: num(&v, &["endless", "seal", "budget"])?,
            ramp,
            endless_max_ms: num(&v, &["endless", "max_ms"])?,
            chaos_events,
            decoy_quiet_ms: num(&v, &["decoy_quiet_ms"])?,
            decoy_types: kinds(&v, "decoy_types")?,
            chaos_hours: (ch.0 as i32, ch.1 as i32),
            type_order: kinds(&v, "type_order")?,
            types,
            pool,
            walker_types: kinds(&v, "walker_types")?,
            new_types: kinds(&v, "new_types")?,
            timing,
            tell_kind,
            pulse,
            tune_span_ms: num(&v, &["tune", "span_ms"])?,
            wind_per_sec: num(&v, &["tune", "wind_per_sec"])?,
            restart_at: num(&v, &["tune", "restart_at"])?,
            slow_at: num(&v, &["tune", "slow_at"])?,
            grace_floor: num(&v, &["tune", "grace_floor"])?,
            hum_ms: pair(&v, &["tune", "hum_ms"])?,
            box_rooms: strings(&v, "box_rooms"),
            cam_rooms: strings(&v, "cam_rooms"),
            routes: routes(&v, "routes")?,
            vent_routes: routes(&v, "vent_routes")?,
            vent_near,
        };
        if t.cam_rooms.len() != 9 || t.box_rooms.len() != 8 {
            return Err("mixed tuning: expected 9 camera rooms and 8 box rooms".to_string());
        }
        if t.pool.is_empty() {
            return Err("mixed tuning: the pool is empty".to_string());
        }
        Ok(t)
    }

    pub fn shift(&self, level_index: i32) -> &Shift {
        &self.shifts[(level_index.max(0) as usize).min(self.shifts.len() - 1)]
    }

    pub fn pool_entry(&self, id: &str) -> Option<&PoolDef> {
        self.pool.iter().find(|p| p.id == id)
    }

    pub fn kind_of(&self, id: &str) -> Option<Kind> {
        self.pool_entry(id).map(|p| p.kind)
    }

    pub fn name_of(&self, id: &str) -> String {
        match self.pool_entry(id) {
            Some(p) => p.name.clone(),
            None => crate::core::display_name(id),
        }
    }

    pub fn chaos_def(&self, id: &str) -> Option<&ChaosDef> {
        self.chaos_events.iter().find(|c| c.id == id)
    }

    pub fn text(&self, k: Kind) -> &TypeText {
        &self.types[k.idx()]
    }

    pub fn timing_of(&self, k: Kind) -> &Timing {
        &self.timing[k.idx()]
    }

    // ---- the Endless ramps

    pub fn endless_pace(&self, hr: i32) -> f64 {
        let r = &self.ramp;
        r.pace_cap.min(1.0 + r.pace_step * (hr - r.pace_from).max(0) as f64)
    }

    pub fn endless_tell(&self, hr: i32) -> f64 {
        let r = &self.ramp;
        let base = self.shifts[4].tell_ms;
        r.tell_min.max(base - r.tell_step * (hr - r.tell_from).max(0) as f64)
    }

    pub fn endless_budget_at(&self, hr: i32) -> f64 {
        let r = &self.ramp;
        r.budget_min.max(self.endless_budget - r.budget_step * (hr - r.budget_from).max(0) as f64)
    }

    pub fn endless_gap(&self, hr: i32) -> f64 {
        let r = &self.ramp;
        r.gap_min.max(r.gap_start - r.gap_step * hr as f64)
    }

    pub fn endless_floor_at(&self, t: f64) -> f64 {
        let r = &self.ramp;
        if t < r.floor_from {
            return 0.0;
        }
        r.floor_max * (1.0 - (-(t - r.floor_from) / HOUR_MS / r.floor_span).exp())
    }
}

static TUNING: OnceLock<Tuning> = OnceLock::new();

/// The embedded tuning. Panics only if the embedded file is broken (`check` and the tests catch that).
pub fn tun() -> &'static Tuning {
    TUNING.get_or_init(|| Tuning::parse(TUNING_JSON).unwrap_or_else(|e| panic!("{e}")))
}

/// The clock in Endless: 12 AM to 5 AM as ever, then "HOUR 6", "HOUR 7" ...
pub fn endless_clock(elapsed: f64) -> String {
    let h = (elapsed / HOUR_MS).floor() as i32;
    if h < 6 {
        format!("{} AM", if h == 0 { 12 } else { h })
    } else {
        format!("HOUR {h}")
    }
}

pub fn hours_text(n: i32) -> String {
    format!("{} hour{}", n, if n == 1 { "" } else { "s" })
}

pub fn door_side(node: &str) -> Option<Side> {
    match node {
        "doorL" => Some(Side::L),
        "doorR" => Some(Side::R),
        _ => None,
    }
}

pub fn vent_gate_side(node: &str) -> Option<Side> {
    match node {
        "ventL" => Some(Side::L),
        "ventR" => Some(Side::R),
        "ventT" => Some(Side::T),
        _ => None,
    }
}

pub fn gate_side(node: &str) -> Option<Side> {
    door_side(node).or_else(|| vent_gate_side(node))
}

/// Per-type check lists used by the tests and `--check`: every route room is a camera room or a gate.
pub fn route_problems(t: &Tuning) -> Vec<String> {
    let mut out = Vec::new();
    for r in t.routes.iter().chain(t.vent_routes.iter()) {
        let last = r.rooms.last().map(|s| s.as_str()).unwrap_or("");
        if gate_side(last).is_none() {
            out.push(format!("route {} does not end at a gate", last));
        }
        for room in &r.rooms[..r.rooms.len() - 1] {
            if !t.cam_rooms.iter().any(|c| c == room) {
                out.push(format!("route {} passes {} which has no camera", r.id, room));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_tuning_parses() {
        let t = Tuning::parse(TUNING_JSON).expect("tuning");
        assert_eq!(t.shifts.len(), 5);
        assert_eq!(t.pool.len(), 8);
        assert_eq!(t.cam_rooms.len(), 9);
        assert_eq!(t.routes.len(), 6);
        assert_eq!(t.vent_routes.len(), 3);
        assert!(route_problems(&t).is_empty(), "{:?}", route_problems(&t));
    }

    #[test]
    fn the_shifts_match_the_godot_board() {
        let t = tun();
        let names: Vec<&str> = t.shifts.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["QUIET", "STEADY", "RESTLESS", "CHAOS", "ENDLESS"]);
        assert_eq!(t.shift(0).min_tell_ms, 3000.0);
        assert_eq!(t.shift(3).must, vec!["verrine".to_string(), "glyra".to_string()]);
        assert_eq!(t.shift(3).min_types, 4);
        assert!(t.shift(4).endless && t.shift(3).chaos);
        assert_eq!(t.shift(99).id, "endless");
    }

    #[test]
    fn endless_ramps_follow_the_formulas() {
        let t = tun();
        assert_eq!(t.endless_pace(0), 1.0);
        assert!((t.endless_pace(7) - 1.2).abs() < 1e-9);
        assert_eq!(t.endless_pace(500), 2.0);
        assert_eq!(t.endless_tell(0), 1900.0);
        assert_eq!(t.endless_tell(8), 1900.0 - 45.0 * 5.0);
        assert_eq!(t.endless_tell(1000), 1600.0);
        assert_eq!(t.endless_budget_at(0), 2400.0);
        assert_eq!(t.endless_budget_at(1000), 1400.0);
        assert_eq!(t.endless_gap(0), 46000.0);
        assert_eq!(t.endless_gap(100), 10000.0);
        assert_eq!(t.endless_floor_at(1000.0), 0.0);
        assert!(t.endless_floor_at(HOUR_MS * 6.0) > 0.0 && t.endless_floor_at(HOUR_MS * 1000.0) < 0.85 + 1e-9);
    }

    #[test]
    fn the_endless_clock_reads_hours() {
        assert_eq!(endless_clock(0.0), "12 AM");
        assert_eq!(endless_clock(HOUR_MS * 3.0 + 5.0), "3 AM");
        assert_eq!(endless_clock(HOUR_MS * 6.0), "HOUR 6");
        assert_eq!(endless_clock(HOUR_MS * 12.5), "HOUR 12");
        assert_eq!(hours_text(1), "1 hour");
        assert_eq!(hours_text(4), "4 hours");
    }

    #[test]
    fn gates_are_found_by_name() {
        assert_eq!(gate_side("doorL"), Some(Side::L));
        assert_eq!(gate_side("ventT"), Some(Side::T));
        assert_eq!(gate_side("stage"), None);
        assert_eq!(door_side("ventL"), None);
        assert_eq!(Side::R.word(), "right");
    }

    #[test]
    fn the_pulse_table_follows_the_tell_kind() {
        let t = tun();
        assert_eq!(t.tell_kind[Kind::Path.idx()], "knock");
        assert_eq!(t.pulse[Kind::Path.idx()], (750.0, 300.0));
        assert_eq!(t.tell_kind[Kind::Counter.idx()], "fog");
        assert_eq!(t.timing_of(Kind::Monitor).steps_to_arm, 3);
        assert_eq!(t.timing_of(Kind::Counter).arm_at, 0.65);
    }
}
