//! Interactable sewer pipes: the valve wheels a runner holds to turn, and the water they start, stop or redirect.
//! Pure model, no `Game` access: a hook in the interact and world-sync code feeds it positions and the clock, and
//! draws the water from `PipeNetwork::depth_at_step`. Host-authoritative like doors: the host calls `turn`, publishes
//! the returned `TurnEvent`, and every machine (the host included) calls `apply` with it, so replays are harmless.
//!
//! Effects (data/pipes.json `effects`; Godot `_accept_valve` / `_apply_valve_event`):
//! - `flood`: shuts every running flow of the valve's chunk (the water drains in `shut_drain_seconds`); with none
//!   running the turn only counts (the chunk's pipe drips go off and on).
//! - `gate`: an odd turn count lowers the sluice gate into a crossing, an even one raises it.
//! - `flow`: an odd count opens the valve and starts a flow in its chunk, an even one closes it and the water drains.
//! - `redirect`: each turn sends the chunk's flows out of the next outlet (`flow.outlet_count`).
//! - `count`: counted only.
//!
//! The numbers (reach, hold, noise, depths, times, difficulty scales, prompts) are data/pipes.json, parsed once.
//! The turner's noise is the real runner's only: a bot or a chased fake player turning a valve calls `turn` with
//! `by_player = false` and the event carries no noise, so it never reaches the danger readout, heartbeat or chase cue.

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// The data file (under the embedded data root, see build.rs).
pub const PIPES_PATH: &str = "pipes.json";

/// A chunk coordinate: x, storey (y), z.
pub type Chunk = (i32, i32, i32);

#[derive(Clone, Debug, PartialEq)]
pub struct ValveRules {
    pub prop_type: String,
    /// How near the valve the turner must stand (cells).
    pub reach: f32,
    /// How long interact is held (seconds).
    pub hold_seconds: f32,
    /// How long the wheel spins after the hold (seconds); also how long the turner's noise lasts.
    pub turn_seconds: f32,
    /// The turner's noise radius.
    pub noise: f32,
    pub max_id_length: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowRules {
    pub rise_seconds: f32,
    pub max_depth: f32,
    pub deep_depth: f32,
    pub creep_seconds: f32,
    pub shut_drain_seconds: f32,
    pub min_depth: f32,
    pub start_reach: i32,
    pub full_reach: i32,
    pub max_cells: usize,
    pub outlet_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Flood,
    Gate,
    Flow,
    Redirect,
    Count,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectDef {
    pub kind: EffectKind,
    pub prompts: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DifficultyScale {
    pub creep: f32,
    pub rise: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub valve: ValveRules,
    pub flow: FlowRules,
    pub effects: BTreeMap<String, EffectDef>,
    pub difficulty: BTreeMap<String, DifficultyScale>,
    /// Axe strikes that burst a pipe, by pipe kind ("crawl", "outflow").
    pub durability: BTreeMap<String, u32>,
}

fn num(v: &Value, section: &str, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(Value::as_f64)
        .map(|x| x as f32)
        .filter(|x| x.is_finite())
        .ok_or_else(|| format!("{section}.{key}: a number is needed"))
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if root.get("schema").and_then(Value::as_str) != Some("lr.pipes/1") {
            return Err("schema must be \"lr.pipes/1\"".into());
        }
        let valve_v = root.get("valve").ok_or("valve: missing")?;
        let flow_v = root.get("flow").ok_or("flow: missing")?;
        let valve = ValveRules {
            prop_type: valve_v
                .get("prop_type")
                .and_then(Value::as_str)
                .ok_or("valve.prop_type: a string is needed")?
                .to_string(),
            reach: num(valve_v, "valve", "reach")?,
            hold_seconds: num(valve_v, "valve", "hold_seconds")?,
            turn_seconds: num(valve_v, "valve", "turn_seconds")?,
            noise: num(valve_v, "valve", "noise")?,
            max_id_length: num(valve_v, "valve", "max_id_length")? as usize,
        };
        let flow = FlowRules {
            rise_seconds: num(flow_v, "flow", "rise_seconds")?,
            max_depth: num(flow_v, "flow", "max_depth")?,
            deep_depth: num(flow_v, "flow", "deep_depth")?,
            creep_seconds: num(flow_v, "flow", "creep_seconds")?,
            shut_drain_seconds: num(flow_v, "flow", "shut_drain_seconds")?,
            min_depth: num(flow_v, "flow", "min_depth")?,
            start_reach: num(flow_v, "flow", "start_reach")? as i32,
            full_reach: num(flow_v, "flow", "full_reach")? as i32,
            max_cells: num(flow_v, "flow", "max_cells")? as usize,
            outlet_count: num(flow_v, "flow", "outlet_count")? as usize,
        };
        if valve.reach <= 0.0 || valve.hold_seconds < 0.0 || valve.turn_seconds < 0.0 || valve.noise < 0.0 {
            return Err("valve: reach must be above 0 and the times and noise not negative".into());
        }
        if flow.rise_seconds <= 0.0 || flow.creep_seconds <= 0.0 || flow.shut_drain_seconds <= 0.0 {
            return Err("flow: rise, creep and drain times must be above 0".into());
        }
        if !(0.0..=flow.deep_depth).contains(&flow.max_depth) || flow.min_depth < 0.0 {
            return Err("flow: depths must satisfy 0 <= max_depth <= deep_depth and min_depth >= 0".into());
        }
        if flow.start_reach < 0 || flow.full_reach < flow.start_reach || flow.outlet_count == 0 {
            return Err("flow: reaches must satisfy 0 <= start_reach <= full_reach, and outlet_count must be 1 or more".into());
        }
        let mut effects = BTreeMap::new();
        for (name, def) in root.get("effects").and_then(Value::as_object).ok_or("effects: an object is needed")? {
            let kind = match def.get("kind").and_then(Value::as_str) {
                Some("flood") => EffectKind::Flood,
                Some("gate") => EffectKind::Gate,
                Some("flow") => EffectKind::Flow,
                Some("redirect") => EffectKind::Redirect,
                Some("count") => EffectKind::Count,
                _ => return Err(format!("effects.{name}.kind: flood, gate, flow, redirect or count")),
            };
            let prompts = def
                .get("prompt")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("effects.{name}.prompt: an object is needed"))?
                .iter()
                .filter_map(|(k, t)| t.as_str().map(|t| (k.clone(), t.to_string())))
                .collect();
            effects.insert(name.clone(), EffectDef { kind, prompts });
        }
        let mut difficulty = BTreeMap::new();
        for (name, d) in root.get("difficulty").and_then(Value::as_object).ok_or("difficulty: an object is needed")? {
            let scale = DifficultyScale { creep: num(d, name, "creep_scale")?, rise: num(d, name, "rise_scale")? };
            if scale.creep <= 0.0 || scale.rise <= 0.0 {
                return Err(format!("difficulty.{name}: scales must be above 0"));
            }
            difficulty.insert(name.clone(), scale);
        }
        if !difficulty.contains_key("standard") {
            return Err("difficulty.standard: missing".into());
        }
        let mut durability = BTreeMap::new();
        for (kind, v) in root.get("durability").and_then(Value::as_object).ok_or("durability: an object is needed")? {
            if kind.starts_with('_') {
                continue;
            }
            let strikes = v.as_u64().filter(|n| *n >= 1).ok_or_else(|| format!("durability.{kind}: a whole number of 1 or more is needed"))?;
            durability.insert(kind.clone(), strikes as u32);
        }
        for kind in ["crawl", "outflow"] {
            if !durability.contains_key(kind) {
                return Err(format!("durability.{kind}: missing"));
            }
        }
        Ok(Rules { valve, flow, effects, difficulty, durability })
    }

    /// The scales for a difficulty name ("relaxed", "standard", "hard", "nightmare"); Standard when unknown.
    pub fn scale(&self, difficulty: &str) -> DifficultyScale {
        self.difficulty
            .get(&difficulty.to_ascii_lowercase())
            .or_else(|| self.difficulty.get("standard"))
            .copied()
            .unwrap_or(DifficultyScale { creep: 1.0, rise: 1.0 })
    }

    /// An empty `valve_effect` is the flood valve (Godot: `get("valve_effect", "flood")`).
    pub fn effect(&self, name: &str) -> Option<&EffectDef> {
        self.effects.get(if name.is_empty() { "flood" } else { name })
    }

    /// Axe strikes that burst a pipe of kind `pipe` ("crawl" or "outflow"); one for an unknown kind.
    pub fn strikes_to_burst(&self, pipe: &str) -> u32 {
        self.durability.get(pipe).copied().unwrap_or(1)
    }
}

/// The embedded rules (data/pipes.json), parsed once.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under("")
            .find(|(p, _)| *p == PIPES_PATH)
            .map(|(_, t)| t)
            .expect("data/pipes.json is embedded by build.rs");
        Rules::parse(text).expect("data/pipes.json is valid")
    })
}

/// A valve prop id: `SEWER:x:y:z:PROP:n`, the chunk coordinate and the prop's number (Godot `_valve_info`).
pub fn parse_valve_id(id: &str, max_len: usize) -> Option<Chunk> {
    let parts: Vec<&str> = id.split(':').collect();
    if id.len() > max_len || parts.len() != 6 || parts[0] != "SEWER" || parts[4] != "PROP" {
        return None;
    }
    Some((parts[1].parse().ok()?, parts[2].parse().ok()?, parts[3].parse().ok()?))
}

/// The prefix of every flow id in a chunk: `BRK:x:y:z:` (how a flood valve finds its chunk's flows).
pub fn flow_prefix(chunk: Chunk) -> String {
    format!("BRK:{}:{}:{}:", chunk.0, chunk.1, chunk.2)
}

/// The chunk and pipe number of a burst flow id `BRK:x:y:z:n`; `None` for anything else.
pub fn parse_burst_id(id: &str) -> Option<(Chunk, usize)> {
    let parts: Vec<&str> = id.split(':').collect();
    if parts.len() != 5 || parts[0] != "BRK" {
        return None;
    }
    Some(((parts[1].parse().ok()?, parts[2].parse().ok()?, parts[3].parse().ok()?), parts[4].parse().ok()?))
}

/// 0 when the flow settles, 1 after the creep (Godot `worsening`).
pub fn worsening(r: &FlowRules, scale: DifficultyScale, age: f32) -> f32 {
    let rise = r.rise_seconds * scale.rise;
    ((age - rise) / (r.creep_seconds * scale.creep)).clamp(0.0, 1.0)
}

/// Water depth `age` seconds after the flow started while nobody has shut it: a smooth rise, then the slow creep.
pub fn depth_at(r: &FlowRules, scale: DifficultyScale, age: f32) -> f32 {
    if age <= 0.0 {
        return 0.0;
    }
    let rise = r.rise_seconds * scale.rise;
    if age < rise {
        let k = age / rise;
        return r.max_depth * k * k * (3.0 - 2.0 * k);
    }
    r.max_depth + (r.deep_depth - r.max_depth) * worsening(r, scale, age)
}

/// Walkway steps from the pipe the water reaches `age` seconds in.
pub fn reach_at(r: &FlowRules, scale: DifficultyScale, age: f32) -> i32 {
    if age <= 0.0 {
        return 0;
    }
    let rise = r.rise_seconds * scale.rise;
    if age < rise {
        return (r.start_reach as f32 * age / rise).ceil() as i32;
    }
    r.start_reach + ((r.full_reach - r.start_reach) as f32 * worsening(r, scale, age)).floor() as i32
}

/// Depth when a valve shut the flow `shut` seconds in: from then it drains in `shut_drain_seconds`.
pub fn depth_shut(r: &FlowRules, scale: DifficultyScale, age: f32, shut: Option<f32>) -> f32 {
    match shut {
        Some(s) if age >= s => depth_at(r, scale, s) * (1.0 - (age - s) / r.shut_drain_seconds).max(0.0),
        _ => depth_at(r, scale, age),
    }
}

/// The spread a shut flow keeps: frozen at the turn.
pub fn reach_shut(r: &FlowRules, scale: DifficultyScale, age: f32, shut: Option<f32>) -> i32 {
    match shut {
        Some(s) if age >= s => reach_at(r, scale, s),
        _ => reach_at(r, scale, age),
    }
}

/// A shut flow that has drained.
pub fn spent(r: &FlowRules, age: f32, shut: Option<f32>) -> bool {
    shut.is_some_and(|s| age >= s + r.shut_drain_seconds)
}

/// One running or shut flow of a pipe. `shut_at` is seconds after `opened_at`.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub opened_at: f32,
    pub shut_at: Option<f32>,
    /// Which outlet the water leaves by (0..`outlet_count`); a redirect valve moves it.
    pub outlet: usize,
}

/// One valve's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Valve {
    pub effect: String,
    pub gate: String,
    pub turns: u32,
}

/// Why a turn was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    BadId,
    UnknownEffect,
    WrongStorey,
    TooFar,
}

/// What a turn did, published by the host and applied everywhere.
#[derive(Clone, Debug, PartialEq)]
pub struct TurnEvent {
    pub valve_id: String,
    pub chunk: Chunk,
    pub effect: String,
    /// A flood valve that shut running flows.
    pub shut: bool,
    pub turns: u32,
    pub gate: String,
    /// The turner's noise (radius, seconds); `None` for a bot or a chased fake player, so no danger cue.
    pub noise: Option<(f32, f32)>,
}

impl TurnEvent {
    /// Whether the turn is a sound the hunters can hear: every turn is, by a bot as much as by the runner. The
    /// runner's meter noise (`noise`) stays player-only. Derived locally, never on the wire.
    pub fn noise_event(&self) -> bool {
        true
    }
}

/// Where a turn is asked: the valve and the turner.
#[derive(Clone, Copy, Debug)]
pub struct TurnAt<'a> {
    pub valve_id: &'a str,
    /// The prop's `valve_effect` ("" is a flood valve) and `gate` keys.
    pub effect: &'a str,
    pub gate: &'a str,
    pub valve_at: (f32, f32),
    pub valve_storey: i32,
    pub at: (f32, f32),
    pub storey: i32,
    /// True only for the local, real runner: only then does the event carry noise.
    pub by_player: bool,
}

/// The valves and flows of the sewer pipes in play.
#[derive(Clone, Debug, Default)]
pub struct PipeNetwork {
    valves: BTreeMap<String, Valve>,
    runs: BTreeMap<String, Run>,
    /// Axe strikes so far on each pipe that has not burst yet (`BRK:x:y:z:n`).
    strikes: BTreeMap<String, u32>,
}

impl PipeNetwork {
    pub fn new() -> Self {
        Self::default()
    }

    /// A pipe's flow starts (burst by the Axe) at `id` from `now`; one already known is left as it is.
    pub fn start_flow(&mut self, id: &str, now: f32) {
        self.runs.entry(id.to_string()).or_insert(Run { opened_at: now, shut_at: None, outlet: 0 });
    }

    /// One Axe strike on the unburst pipe `id`, which takes `need` strikes to burst. Returns true on the strike that
    /// bursts it (its count is then cleared); a strike short of that only dents it.
    pub fn strike(&mut self, id: &str, need: u32) -> bool {
        let hits = {
            let h = self.strikes.entry(id.to_string()).or_insert(0);
            *h += 1;
            *h
        };
        if hits >= need.max(1) {
            self.strikes.remove(id);
            true
        } else {
            false
        }
    }

    /// The strikes a pipe has taken so far (0 once it bursts or before the first).
    pub fn strikes_on(&self, id: &str) -> u32 {
        self.strikes.get(id).copied().unwrap_or(0)
    }

    pub fn run(&self, id: &str) -> Option<&Run> {
        self.runs.get(id)
    }

    pub fn valve(&self, id: &str) -> Option<&Valve> {
        self.valves.get(id)
    }

    /// Valve turns so far (0 for one never turned).
    pub fn turns(&self, id: &str) -> u32 {
        self.valves.get(id).map_or(0, |v| v.turns)
    }

    /// Every flow of `chunk` (running or shut) as `(flow id, outlet, number n)`. A burst flow is `BRK:x:y:z:<pipe n>`;
    /// a flow valve's flow is named after its valve, so its `n` is the valve prop's number. Flows of other chunks and
    /// ids that do not parse are ignored.
    pub fn flows_in(&self, chunk: Chunk) -> Vec<(String, usize, usize)> {
        let prefix = flow_prefix(chunk);
        self.runs
            .iter()
            .filter_map(|(id, run)| {
                let rest = id.strip_prefix(&prefix)?;
                let n = match rest.parse::<usize>() {
                    Ok(n) => n,
                    Err(_) => {
                        parse_valve_id(rest, rules().valve.max_id_length).filter(|c| *c == chunk)?;
                        rest.rsplit(':').next()?.parse().ok()?
                    }
                };
                Some((id.clone(), run.outlet, n))
            })
            .collect()
    }

    /// Whether a flow of the chunk is running (started and not shut).
    pub fn flows_running(&self, chunk: Chunk) -> bool {
        let prefix = flow_prefix(chunk);
        self.runs.iter().any(|(id, r)| id.starts_with(&prefix) && r.shut_at.is_none())
    }

    /// Whether a valve is turned an odd number of times: a gate valve's gate is down (a crossing for runners and
    /// hunters), a flow valve is open.
    pub fn is_open(&self, valve_id: &str) -> bool {
        self.turns(valve_id) % 2 == 1
    }

    /// Whether the gate `gate` is down (a crossing): some gate valve naming it has been turned an odd number of times.
    /// The valve need not be in the loaded window; a valve this network never saw leaves its gate closed.
    pub fn gate_open(&self, gate: &str) -> bool {
        self.valves.values().any(|v| v.effect == "gate" && v.gate == gate && v.turns % 2 == 1)
    }

    /// The host's decision for a turn.
    pub fn turn(&self, q: TurnAt) -> Result<TurnEvent, Refused> {
        let r = rules();
        let chunk = parse_valve_id(q.valve_id, r.valve.max_id_length).ok_or(Refused::BadId)?;
        let def = r.effect(q.effect).ok_or(Refused::UnknownEffect)?;
        if q.storey != q.valve_storey || chunk.1 != q.valve_storey {
            return Err(Refused::WrongStorey);
        }
        let (dx, dy) = (q.at.0 - q.valve_at.0, q.at.1 - q.valve_at.1);
        if (dx * dx + dy * dy).sqrt() > r.valve.reach {
            return Err(Refused::TooFar);
        }
        let shut = def.kind == EffectKind::Flood && self.flows_running(chunk);
        Ok(TurnEvent {
            valve_id: q.valve_id.to_string(),
            chunk,
            effect: if q.effect.is_empty() { "flood".into() } else { q.effect.to_string() },
            shut,
            turns: self.turns(q.valve_id) + u32::from(!shut),
            gate: q.gate.to_string(),
            noise: q.by_player.then_some((r.valve.noise, r.valve.turn_seconds)),
        })
    }

    /// Every machine: the new valve state and what it does to the flows at time `now`. Replaying an event sets the
    /// same turn count again and shuts nothing twice, so it is harmless.
    pub fn apply(&mut self, ev: &TurnEvent, now: f32) {
        let r = rules();
        let kind = r.effect(&ev.effect).map_or(EffectKind::Count, |d| d.kind);
        let prefix = flow_prefix(ev.chunk);
        if ev.shut {
            for (id, run) in self.runs.iter_mut() {
                if id.starts_with(&prefix) && run.shut_at.is_none() {
                    run.shut_at = Some(now - run.opened_at);
                }
            }
            return;
        }
        let valve = self.valves.entry(ev.valve_id.clone()).or_insert(Valve {
            effect: ev.effect.clone(),
            gate: ev.gate.clone(),
            turns: 0,
        });
        valve.turns = ev.turns;
        match kind {
            // a flow valve owns the flow named after it: opened on an odd count, shut on an even one
            EffectKind::Flow => {
                let id = format!("{prefix}{}", ev.valve_id);
                if ev.turns % 2 == 1 {
                    match self.runs.get_mut(&id) {
                        Some(run) if run.shut_at.is_some() => {
                            *run = Run { opened_at: now, shut_at: None, outlet: run.outlet };
                        }
                        Some(_) => {}
                        None => {
                            self.runs.insert(id, Run { opened_at: now, shut_at: None, outlet: 0 });
                        }
                    }
                } else if let Some(run) = self.runs.get_mut(&id) {
                    if run.shut_at.is_none() {
                        run.shut_at = Some(now - run.opened_at);
                    }
                }
            }
            EffectKind::Redirect => {
                let outlet = ev.turns as usize % r.flow.outlet_count;
                for (id, run) in self.runs.iter_mut() {
                    if id.starts_with(&prefix) {
                        run.outlet = outlet;
                    }
                }
            }
            EffectKind::Flood | EffectKind::Gate | EffectKind::Count => {}
        }
    }

    /// Water depth over a walkway cell `step` steps from the pipe, on outlet `outlet` of flow `id`, at `now`: 0
    /// beyond the spread, on another outlet, below the film depth, or for an unknown flow. Wading and the water mesh
    /// read this.
    pub fn depth_at_step(&self, id: &str, step: i32, outlet: usize, now: f32, difficulty: &str) -> f32 {
        let r = rules();
        let Some(run) = self.runs.get(id) else { return 0.0 };
        if run.outlet != outlet {
            return 0.0;
        }
        let scale = r.scale(difficulty);
        let age = now - run.opened_at;
        if step > reach_shut(&r.flow, scale, age, run.shut_at) {
            return 0.0;
        }
        let d = depth_shut(&r.flow, scale, age, run.shut_at);
        if d < r.flow.min_depth {
            0.0
        } else {
            d
        }
    }

    /// Drops flows that have drained, so a long run keeps no dead ones.
    pub fn prune(&mut self, now: f32) {
        let drain = rules().flow.shut_drain_seconds;
        self.runs.retain(|_, run| !run.shut_at.is_some_and(|s| now - run.opened_at >= s + drain));
    }

    /// The interact prompt for a valve with `effect` in its current state: "closed" before a first turn or after
    /// an even count, "open" after an odd one; a flood valve over running water says "shut_flood".
    pub fn prompt(&self, valve_id: &str, effect: &str) -> String {
        let r = rules();
        let Some(def) = r.effect(effect) else { return String::new() };
        let key = match parse_valve_id(valve_id, r.valve.max_id_length) {
            Some(chunk) if def.kind == EffectKind::Flood && self.flows_running(chunk) => "shut_flood",
            _ if self.is_open(valve_id) => "open",
            _ => "closed",
        };
        def.prompts.get(key).or_else(|| def.prompts.get("closed")).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: &str = "SEWER:2:-1:3:PROP:7";

    fn q<'a>(effect: &'a str, by_player: bool) -> TurnAt<'a> {
        TurnAt {
            valve_id: V,
            effect,
            gate: "",
            valve_at: (10.5, 10.5),
            valve_storey: -1,
            at: (10.0, 11.0),
            storey: -1,
            by_player,
        }
    }

    #[test]
    fn the_standard_numbers_match_the_game() {
        let r = rules();
        assert_eq!((r.valve.reach, r.valve.hold_seconds, r.valve.turn_seconds, r.valve.noise), (3.0, 2.0, 1.6, 9.0));
        assert_eq!((r.flow.rise_seconds, r.flow.max_depth, r.flow.deep_depth), (4.0, 0.35, 0.9));
        assert_eq!((r.flow.creep_seconds, r.flow.shut_drain_seconds, r.flow.min_depth), (180.0, 15.0, 0.03));
        assert_eq!((r.flow.start_reach, r.flow.full_reach, r.flow.max_cells), (4, 12, 160));
        assert_eq!(r.scale("standard"), DifficultyScale { creep: 1.0, rise: 1.0 });
        assert_eq!(r.scale("no such"), r.scale("standard"));
    }

    #[test]
    fn the_embedded_file_parses_and_a_bad_one_is_refused() {
        let text = include_str!("../data/pipes.json");
        assert!(Rules::parse(text).is_ok());
        assert!(Rules::parse("{}").is_err());
        assert!(Rules::parse(&text.replace("\"reach\": 3.0", "\"reach\": 0")).is_err());
        assert!(Rules::parse(&text.replace("\"outlet_count\": 2", "\"outlet_count\": 0")).is_err());
    }

    #[test]
    fn pipe_durability_is_data_and_above_the_old_breach_points() {
        let r = rules();
        assert!(r.strikes_to_burst("crawl") > 3, "above the ported crawl hit points (3)");
        assert!(r.strikes_to_burst("outflow") > 4, "above the ported outflow hit points (4)");
        assert!(r.strikes_to_burst("outflow") >= r.strikes_to_burst("crawl"));
        assert_eq!(r.strikes_to_burst("no such pipe"), 1);
        let text = include_str!("../data/pipes.json");
        assert!(Rules::parse(&text.replace("\"outflow\": 10", "\"outflow\": 0")).is_err(), "a zero durability is refused");
        assert!(Rules::parse(&text.replace("\"crawl\": 6,", "")).is_err(), "a missing durability is refused");
    }

    #[test]
    fn a_pipe_bursts_on_the_strike_that_reaches_its_durability() {
        let mut n = PipeNetwork::new();
        assert!(!n.strike("BRK:1:-1:1:0", 3));
        assert!(!n.strike("BRK:1:-1:1:0", 3));
        assert_eq!(n.strikes_on("BRK:1:-1:1:0"), 2);
        assert!(n.strike("BRK:1:-1:1:0", 3), "the third strike bursts it");
        assert_eq!(n.strikes_on("BRK:1:-1:1:0"), 0);
        assert!(n.strike("BRK:1:-1:1:1", 0), "a need of 0 still takes one strike");
    }

    #[test]
    fn the_water_rises_then_creeps_and_never_exceeds_the_deep_depth() {
        let r = &rules().flow;
        let s = rules().scale("standard");
        let mut last = 0.0;
        for i in 0..=400 {
            let d = depth_at(r, s, i as f32);
            assert!(d >= last - 1e-6 && d <= r.deep_depth + 1e-6);
            last = d;
        }
        assert_eq!(depth_at(r, s, 0.0), 0.0);
        assert!((depth_at(r, s, r.rise_seconds) - r.max_depth).abs() < 1e-5);
        assert!((depth_at(r, s, r.rise_seconds + r.creep_seconds) - r.deep_depth).abs() < 1e-5);
        assert_eq!(reach_at(r, s, 1e6), r.full_reach);
    }

    #[test]
    fn a_shut_flow_freezes_its_spread_and_drains() {
        let r = &rules().flow;
        let s = rules().scale("standard");
        let at_shut = depth_at(r, s, 30.0);
        assert_eq!(depth_shut(r, s, 30.0, Some(30.0)), at_shut);
        let half = depth_shut(r, s, 30.0 + r.shut_drain_seconds / 2.0, Some(30.0));
        assert!((half - at_shut / 2.0).abs() < 1e-5);
        assert_eq!(depth_shut(r, s, 30.0 + r.shut_drain_seconds, Some(30.0)), 0.0);
        assert_eq!(reach_shut(r, s, 100.0, Some(30.0)), reach_at(r, s, 30.0));
        assert!(spent(r, 30.0 + r.shut_drain_seconds, Some(30.0)) && !spent(r, 1e6, None));
    }

    #[test]
    fn a_harder_difficulty_floods_faster_and_a_data_value_changes_the_depth() {
        let r = rules();
        let relaxed = depth_at(&r.flow, r.scale("relaxed"), 60.0);
        let nightmare = depth_at(&r.flow, r.scale("nightmare"), 60.0);
        assert!(nightmare > relaxed);
        let mut deeper = r.flow.clone();
        deeper.max_depth = 0.5;
        assert!(depth_at(&deeper, r.scale("standard"), 10.0) > depth_at(&r.flow, r.scale("standard"), 10.0));
    }

    #[test]
    fn valve_ids_are_checked() {
        assert_eq!(parse_valve_id(V, 80), Some((2, -1, 3)));
        assert_eq!(parse_valve_id("SEWER:a:1:1:PROP:1", 80), None);
        assert_eq!(parse_valve_id("SEWER:1:1:1:BRK:1", 80), None);
        assert_eq!(parse_valve_id(V, 5), None);
        let n = PipeNetwork::new();
        assert_eq!(n.turn(q("flood", true)).map(|e| e.chunk), Ok((2, -1, 3)));
        assert_eq!(n.turn(TurnAt { valve_id: "nope", ..q("flood", true) }), Err(Refused::BadId));
    }

    #[test]
    fn a_turn_needs_the_storey_and_the_reach() {
        let n = PipeNetwork::new();
        let reach = rules().valve.reach;
        assert_eq!(n.turn(TurnAt { storey: 0, ..q("flood", true) }), Err(Refused::WrongStorey));
        assert_eq!(n.turn(TurnAt { at: (10.5, 10.5 + reach + 0.1), ..q("flood", true) }), Err(Refused::TooFar));
        assert!(n.turn(TurnAt { at: (10.5, 10.5 + reach - 0.1), ..q("flood", true) }).is_ok());
        assert_eq!(n.turn(q("bogus", true)), Err(Refused::UnknownEffect));
    }

    #[test]
    fn a_bot_turn_is_a_noise_event_but_carries_no_meter_noise() {
        let n = PipeNetwork::new();
        let ev = n.turn(q("flood", false)).unwrap();
        assert!(ev.noise_event());
        assert_eq!(ev.noise, None);
    }

    #[test]
    fn flows_in_ignores_other_chunks() {
        let mut n = PipeNetwork::new();
        n.start_flow("BRK:2:-1:3:4", 0.0);
        n.start_flow("BRK:2:-1:3:9", 0.0);
        n.start_flow("BRK:2:-1:4:1", 0.0);
        n.start_flow("BRK:2:-1:3:SEWER:2:-1:3:PROP:7", 0.0);
        n.start_flow("BRK:2:-1:3:junk", 0.0);
        let mut f: Vec<usize> = n.flows_in((2, -1, 3)).iter().map(|x| x.2).collect();
        f.sort();
        assert_eq!(f, vec![4, 7, 9]);
        assert!(n.flows_in((0, 0, 0)).is_empty());
    }

    #[test]
    fn only_the_real_runner_makes_noise() {
        let n = PipeNetwork::new();
        let r = rules();
        assert_eq!(n.turn(q("flood", true)).unwrap().noise, Some((r.valve.noise, r.valve.turn_seconds)));
        assert_eq!(n.turn(q("flood", false)).unwrap().noise, None);
    }

    #[test]
    fn a_flood_valve_shuts_running_flows_else_it_only_counts() {
        let mut n = PipeNetwork::new();
        let ev = n.turn(q("flood", true)).unwrap();
        assert!(!ev.shut && ev.turns == 1);
        n.apply(&ev, 5.0);
        assert_eq!(n.turns(V), 1);
        n.start_flow("BRK:2:-1:3:9", 10.0);
        n.start_flow("BRK:5:-1:5:9", 10.0);
        assert!(n.flows_running((2, -1, 3)));
        let ev = n.turn(q("flood", true)).unwrap();
        assert!(ev.shut && ev.turns == 1);
        n.apply(&ev, 40.0);
        assert!(!n.flows_running((2, -1, 3)));
        assert!(n.flows_running((5, -1, 5)), "another chunk's flow is untouched");
        assert_eq!(n.run("BRK:2:-1:3:9").unwrap().shut_at, Some(30.0));
        let before = n.runs.clone();
        n.apply(&ev, 41.0);
        assert_eq!(n.runs, before, "a replay shuts nothing twice");
    }

    #[test]
    fn a_flow_valve_starts_then_stops_the_water_and_the_depth_follows() {
        let mut n = PipeNetwork::new();
        let id = format!("{}{}", flow_prefix((2, -1, 3)), V);
        assert_eq!(n.depth_at_step(&id, 0, 0, 50.0, "standard"), 0.0);
        let open = n.turn(q("flow", true)).unwrap();
        n.apply(&open, 100.0);
        assert!(n.is_open(V) && n.flows_running((2, -1, 3)));
        let r = rules();
        let wet = n.depth_at_step(&id, 0, 0, 100.0 + r.flow.rise_seconds, "standard");
        assert!((wet - r.flow.max_depth).abs() < 1e-4);
        assert_eq!(n.depth_at_step(&id, r.flow.full_reach + 1, 0, 1e5, "standard"), 0.0, "beyond the spread is dry");
        let close = n.turn(q("flow", true)).unwrap();
        assert_eq!(close.turns, 2);
        n.apply(&close, 120.0);
        assert!(!n.flows_running((2, -1, 3)));
        assert_eq!(n.depth_at_step(&id, 0, 0, 120.0 + r.flow.shut_drain_seconds, "standard"), 0.0);
        n.prune(120.0 + r.flow.shut_drain_seconds + 1.0);
        assert!(n.run(&id).is_none());
    }

    #[test]
    fn a_redirect_valve_moves_the_water_to_the_next_outlet() {
        let mut n = PipeNetwork::new();
        n.start_flow("BRK:2:-1:3:1", 0.0);
        let t = 30.0;
        assert!(n.depth_at_step("BRK:2:-1:3:1", 0, 0, t, "standard") > 0.0);
        assert_eq!(n.depth_at_step("BRK:2:-1:3:1", 0, 1, t, "standard"), 0.0);
        let ev = n.turn(q("redirect", true)).unwrap();
        n.apply(&ev, t);
        assert_eq!(n.depth_at_step("BRK:2:-1:3:1", 0, 0, t, "standard"), 0.0);
        assert!(n.depth_at_step("BRK:2:-1:3:1", 0, 1, t, "standard") > 0.0);
    }

    #[test]
    fn a_gate_valve_opens_on_odd_turns_and_prompts_follow_the_state() {
        let mut n = PipeNetwork::new();
        let closed = n.prompt(V, "gate");
        assert!(!closed.is_empty());
        let ev = n.turn(q("gate", true)).unwrap();
        n.apply(&ev, 0.0);
        assert!(n.is_open(V));
        assert_ne!(n.prompt(V, "gate"), closed);
        let ev = n.turn(q("gate", true)).unwrap();
        n.apply(&ev, 0.0);
        assert!(!n.is_open(V));
        assert_eq!(n.prompt(V, "gate"), closed);
        let other = "SEWER:1:-1:1:PROP:0";
        let plain = n.prompt(other, "flood");
        n.start_flow("BRK:1:-1:1:0", 0.0);
        assert_ne!(n.prompt(other, "flood"), plain, "a flood valve over running water offers to shut it off");
        assert_eq!(n.prompt(V, "unknown"), "");
        assert_eq!(n.prompt(other, ""), n.prompt(other, "flood"));
    }

    #[test]
    fn the_sewer_pipes_region_file_is_a_region_with_pipe_rules() {
        let text = include_str!("../data/regions/sewer_pipes.json");
        let region = crate::content::regions::parse_file("regions/sewer_pipes.json", text).expect("a valid region");
        assert_eq!(region.id, "sewer_pipes");
        let v: Value = serde_json::from_str(text).unwrap();
        let effects = v["layout"]["pipes"]["valve_effects"].as_array().expect("valve_effects");
        assert!(!effects.is_empty());
        for e in effects {
            assert!(rules().effect(e.as_str().unwrap()).is_some(), "every placed effect is defined in pipes.json");
        }
    }
}
