//! Data validation and the headless simulation sweep behind `night_watch --check`. No window, no GPU.
#![allow(dead_code)]

use super::calls::parse_calls;
use super::data::{
    parse_cast, BuildingGraph, ModeEntry, Rules, MAX_AI, MODE_ID_LIST, NIGHTS, NIGHT_MS,
};
use super::graph::Graph;
use super::mode::{Fnv, Headless, Outcome};
use super::session::Session;
use macroquad::prelude::vec3;
use serde_json::Value;
use std::collections::BTreeMap;

/// Mode ids whose calls file exists.
const CALL_MODES: [&str; 8] = ["classic", "mask", "lure", "bedroom", "story", "descent", "dayshift", "extra"];
/// The seed every check run uses.
pub const CHECK_SEED: u32 = 12345;

/// Parse every embedded data file and assert the counts and keys the game relies on. Returns the problems found.
pub fn check_data() -> Vec<String> {
    let mut p: Vec<String> = Vec::new();
    check_modes_data(&mut p);
    check_rules(&mut p);
    check_building_and_cast(&mut p);
    check_calls(&mut p);
    check_poses(&mut p);
    check_death_lines(&mut p);
    p
}

fn check_modes_data(p: &mut Vec<String>) {
    let mut seen_nums: Vec<i32> = Vec::new();
    for id in MODE_ID_LIST.iter() {
        let entry = match ModeEntry::parse(crate::assets::mode_json(id)) {
            Ok(e) => e,
            Err(e) => {
                p.push(format!("mode {id}: {e}"));
                continue;
            }
        };
        if entry.id != *id {
            p.push(format!("mode {id}: file says id {}", entry.id));
        }
        if entry.title.is_empty() || entry.label.is_empty() {
            p.push(format!("mode {id}: missing title or label"));
        }
        if entry.levels.is_empty() || entry.levels.len() > 20 {
            p.push(format!("mode {id}: {} levels (want 1..20)", entry.levels.len()));
        }
        if entry.scored == 0 || entry.scored > entry.levels.len() {
            p.push(format!("mode {id}: scored {} of {} levels", entry.scored, entry.levels.len()));
        }
        if seen_nums.contains(&entry.num) {
            p.push(format!("mode {id}: menu number {} used twice", entry.num));
        }
        seen_nums.push(entry.num);
        if let Some(custom) = &entry.custom {
            for preset in &entry.presets {
                for (k, v) in &preset.ai {
                    if !custom.iter().any(|(cid, _)| cid == k) {
                        p.push(format!("mode {id}: preset {} names {k}, which has no slider", preset.id));
                    }
                    if *v < 0 || *v > MAX_AI {
                        p.push(format!("mode {id}: preset {} has {k} at {v}", preset.id));
                    }
                }
            }
        } else if !entry.presets.is_empty() {
            p.push(format!("mode {id}: presets without a custom night"));
        }
        if entry.art.is_empty() {
            p.push(format!("mode {id}: no art hunters"));
        }
        for h in entry.art.iter().chain(entry.cast.iter()) {
            if crate::assets::rig_text(h).is_none() {
                p.push(format!("mode {id}: {h} has no embedded rig"));
            }
            if crate::assets::bbmodel_text(h).is_none() {
                p.push(format!("mode {id}: {h} has no embedded model"));
            }
        }
    }
}

fn check_rules(p: &mut Vec<String>) {
    match Rules::parse(crate::assets::RULES_JSON) {
        Err(e) => p.push(e),
        Ok(r) => {
            if r.night_ms != NIGHT_MS {
                p.push(format!("rules: night_ms {} (the constants say {})", r.night_ms, NIGHT_MS));
            }
            if r.nights != NIGHTS {
                p.push(format!("rules: nights {} (the constants say {})", r.nights, NIGHTS));
            }
            let tuned = r.tuned_nights.max(0) as usize;
            for (name, len) in [("door_grace_ms", r.door_grace_ms.len()), ("night_pace", r.night_pace.len()), ("light_push_ms", r.light_push_ms.len())] {
                if len < tuned || len == 0 {
                    p.push(format!("rules: {name} has {len} entries for {tuned} tuned nights"));
                }
            }
            if r.kathy_hold_ms.len() != 2 || r.kathy_hold_ms[0] > r.kathy_hold_ms[1] {
                p.push("rules: kathy hold_ms must be [lo, hi]".to_string());
            }
            if r.kathy_dip_cycle_ms <= 0.0 || r.kathy_dips.is_empty() {
                p.push("rules: kathy dips are missing".to_string());
            }
            if r.kathy_night < 1 || r.kathy_night > r.nights {
                p.push(format!("rules: kathy_night {} is outside the nights", r.kathy_night));
            }
        }
    }
}

fn check_building_and_cast(p: &mut Vec<String>) {
    let b = match BuildingGraph::parse(crate::assets::BUILDING_JSON) {
        Ok(b) => b,
        Err(e) => {
            p.push(e);
            return;
        }
    };
    if b.rooms.len() != 10 {
        p.push(format!("building: {} rooms (want 10)", b.rooms.len()));
    }
    let mut cams: Vec<i32> = b.rooms.iter().map(|r| r.cam).filter(|c| *c > 0).collect();
    cams.sort();
    if cams != (1..=8).collect::<Vec<i32>>() {
        p.push(format!("building: camera numbers are {cams:?} (want 1..8 once each)"));
    }
    for (a, nbrs) in &b.adj {
        if b.room(a).is_none() {
            p.push(format!("building: graph names unknown room {a}"));
        }
        for n in nbrs {
            if b.room(n).is_none() {
                p.push(format!("building: {a} links to unknown room {n}"));
            } else if !b.adj.get(n).map_or(false, |back| back.contains(a)) {
                p.push(format!("building: {a} -> {n} is one way"));
            }
        }
    }
    // reachability, in the terms of core::graph (positions are only for the weights)
    let mut g = Graph::new();
    for (i, r) in b.rooms.iter().enumerate() {
        let (x, z) = if r.grid.0 >= 0 { (r.grid.0 as f32 * 8.0, r.grid.1 as f32 * 8.0) } else { (i as f32 * 3.0, 30.0) };
        g.add_node(&r.id, vec3(x, 0.0, z));
    }
    for (a, c) in &b.edges {
        g.link(a, c);
    }
    let start = if b.kathy_start.is_empty() { "stage".to_string() } else { b.kathy_start.clone() };
    for r in &b.rooms {
        if g.route(&start, &r.id).is_empty() {
            p.push(format!("building: {} cannot be reached from {start}", r.id));
        }
    }
    for room in b.kathy_fade_to.iter().chain(b.kathy_doors.iter()).chain(b.kathy_meet_rooms.iter()).chain(std::iter::once(&b.kathy_start)) {
        if b.room(room).is_none() {
            p.push(format!("building: kathy path names unknown room {room}"));
        }
    }

    match parse_cast(crate::assets::CAST_JSON) {
        Err(e) => p.push(e),
        Ok(cast) => {
            for h in &cast {
                if crate::assets::rig_text(&h.id).is_none() || crate::assets::bbmodel_text(&h.id).is_none() {
                    p.push(format!("cast: {} has no embedded rig or model", h.id));
                }
                if h.floor.is_some() {
                    for room in h.doors.iter().chain([&h.start, &h.retreat_to, &h.push_to]) {
                        if b.room(room).is_none() {
                            p.push(format!("cast: {} names unknown room '{room}'", h.id));
                        }
                    }
                    if h.levels.is_empty() {
                        p.push(format!("cast: {} has no levels", h.id));
                    }
                }
                if let Some((with, _, _)) = &h.combo {
                    if !cast.iter().any(|c| &c.id == with) {
                        p.push(format!("cast: {} combos with unknown {with}", h.id));
                    }
                }
            }
            if cast.iter().filter(|c| c.floor.is_some()).count() != 4 {
                p.push("cast: want four floor hunters".to_string());
            }
        }
    }
}

fn check_calls(p: &mut Vec<String>) {
    for id in CALL_MODES.iter() {
        match parse_calls(crate::assets::calls_json(id)) {
            Err(e) => p.push(format!("calls {id}: {e}")),
            Ok(levels) => {
                if levels.is_empty() {
                    p.push(format!("calls {id}: no levels"));
                }
                for (i, l) in levels.iter().enumerate() {
                    if l.lines.is_empty() {
                        p.push(format!("calls {id}: level {} has no lines", l.level));
                    }
                    if l.at_s < 0.0 {
                        p.push(format!("calls {id}: level {} rings at {}", l.level, l.at_s));
                    }
                    if levels.iter().skip(i + 1).any(|o| o.level == l.level) {
                        p.push(format!("calls {id}: level {} appears twice", l.level));
                    }
                }
            }
        }
    }
}

fn check_poses(p: &mut Vec<String>) {
    let v: Value = match serde_json::from_str(crate::assets::POSES_JSON) {
        Ok(v) => v,
        Err(e) => {
            p.push(format!("poses json: {e}"));
            return;
        }
    };
    let poses = match v.get("poses").and_then(|x| x.as_object()) {
        Some(o) if !o.is_empty() => o,
        _ => {
            p.push("poses json: no poses".to_string());
            return;
        }
    };
    match v.get("states").and_then(|x| x.as_object()) {
        None => p.push("poses json: no states".to_string()),
        Some(states) => {
            for (state, pose) in states {
                match pose.as_str() {
                    Some(name) if poses.contains_key(name) => {}
                    _ => p.push(format!("poses json: state {state} names a missing pose")),
                }
            }
        }
    }
    if let Some(hs) = v.get("hunter_states").and_then(|x| x.as_object()) {
        for (hunter, map) in hs {
            if let Some(map) = map.as_object() {
                for (state, pose) in map {
                    match pose.as_str() {
                        Some(name) if poses.contains_key(name) => {}
                        _ => p.push(format!("poses json: {hunter} state {state} names a missing pose")),
                    }
                }
            }
        }
    }
    for (name, pose) in poses {
        if pose.get("length").and_then(|x| x.as_f64()).map_or(true, |l| l <= 0.0) {
            p.push(format!("poses json: pose {name} has no length"));
        }
        if !pose.get("tracks").map_or(false, |t| t.is_object()) {
            p.push(format!("poses json: pose {name} has no tracks"));
        }
    }
}

fn check_death_lines(p: &mut Vec<String>) {
    match serde_json::from_str::<Value>(crate::assets::DEATH_LINES_JSON) {
        Err(e) => p.push(format!("death lines json: {e}")),
        Ok(v) => {
            if !v.get("hunters").map_or(false, |h| h.is_object()) {
                p.push("death lines json: no hunters".to_string());
            }
        }
    }
}

// ---- the headless sweep --------------------------------------------------------------------------------------------

/// What one headless run ended with.
#[derive(Clone, Debug)]
pub struct ModeRun {
    pub mode: String,
    pub level: i32,
    pub outcome: Outcome,
    pub digest: u64,
    pub elapsed_ms: f64,
    pub events: usize,
    /// FNV of the event names, in order.
    pub event_hash: u64,
}

impl ModeRun {
    pub fn outcome_name(&self) -> &'static str {
        match self.outcome {
            Outcome::Running => "running",
            Outcome::Won => "won",
            Outcome::Lost => "lost",
        }
    }
}

/// Simulated seconds a mode's shift is run for (descent runs until decided, at most this long).
pub fn run_limit_ms(mode: &str) -> f64 {
    if mode == "descent" {
        600_000.0
    } else {
        270_000.0
    }
}

/// Run one shift headlessly in 100 ms steps. A panic is caught and returned as an error.
pub fn run_headless(factory: &dyn Fn(&Session) -> Option<Box<dyn Headless>>, mode: &str, level: i32) -> Result<ModeRun, String> {
    let mut session = Session::new();
    session.start_mode(mode, level, BTreeMap::new(), Some(CHECK_SEED));
    let mut sim = match factory(&session) {
        Some(s) => s,
        None => return Err(format!("{mode} level {level}: no headless sim")),
    };
    let limit = run_limit_ms(mode);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut events = 0usize;
        let mut hash = Fnv::new();
        let mut t = 0.0;
        while t < limit && sim.outcome() == Outcome::Running {
            for e in sim.step_ms(100.0) {
                events += 1;
                hash.str(&e);
            }
            t += 100.0;
        }
        (events, hash.finish())
    }));
    match result {
        Ok((events, event_hash)) => Ok(ModeRun {
            mode: mode.to_string(),
            level,
            outcome: sim.outcome(),
            digest: sim.digest(),
            elapsed_ms: sim.elapsed_ms(),
            events,
            event_hash,
        }),
        Err(_) => Err(format!("{mode} level {level}: the simulation panicked")),
    }
}

/// Every mode and every level, each run twice: the runs and the problems found (a panic, a missing sim, a run that
/// differs from its twin).
pub fn run_all_modes(factory: &dyn Fn(&Session) -> Option<Box<dyn Headless>>) -> (Vec<ModeRun>, Vec<String>) {
    let modes = super::data::ModesData::load();
    let mut runs = Vec::new();
    let mut problems = Vec::new();
    for id in MODE_ID_LIST.iter() {
        let levels = modes.get(id).map_or(0, |m| m.levels.len()) as i32;
        if levels == 0 {
            problems.push(format!("{id}: no levels to run"));
            continue;
        }
        for level in 1..=levels {
            let first = run_headless(factory, id, level);
            let second = run_headless(factory, id, level);
            match (first, second) {
                (Ok(a), Ok(b)) => {
                    if a.digest != b.digest || a.event_hash != b.event_hash || a.events != b.events || a.outcome != b.outcome {
                        problems.push(format!("{id} level {level}: two runs of seed {CHECK_SEED} differ ({:016x} vs {:016x})", a.digest, b.digest));
                    }
                    runs.push(a);
                }
                (Err(e), _) | (_, Err(e)) => problems.push(e),
            }
        }
    }
    (runs, problems)
}

/// Simulate every mode and level headlessly and report the problems (determinism, panics, missing sims).
pub fn check_modes(modes: &dyn Fn(&Session) -> Option<Box<dyn Headless>>) -> Vec<String> {
    run_all_modes(modes).1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sim that wins at 270 s and mixes the clock into its digest.
    struct Clock {
        t: f64,
        id: &'static str,
        noise: bool,
    }
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    impl Headless for Clock {
        fn id(&self) -> &'static str {
            self.id
        }
        fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
            self.t += dt_ms;
            if (self.t % 90_000.0) < dt_ms {
                vec!["hour".to_string()]
            } else {
                Vec::new()
            }
        }
        fn outcome(&self) -> Outcome {
            if self.t >= 270_000.0 {
                Outcome::Won
            } else {
                Outcome::Running
            }
        }
        fn elapsed_ms(&self) -> f64 {
            self.t
        }
        fn digest(&self) -> u64 {
            let extra = if self.noise { COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) } else { 0 };
            Fnv::new().f64(self.t).u64(extra).finish()
        }
    }

    fn good(_s: &Session) -> Option<Box<dyn Headless>> {
        Some(Box::new(Clock { t: 0.0, id: "clock", noise: false }))
    }

    #[test]
    fn a_deterministic_sim_passes_every_mode_and_level() {
        let (runs, problems) = run_all_modes(&good);
        assert!(problems.is_empty(), "{problems:?}");
        let modes = super::super::data::ModesData::load();
        let expected: usize = modes.modes.iter().map(|m| m.levels.len()).sum();
        assert_eq!(runs.len(), expected);
        assert!(runs.iter().all(|r| r.outcome == Outcome::Won && r.elapsed_ms == 270_000.0));
        assert!(runs.iter().all(|r| r.events == 3));
        assert!(check_modes(&good).is_empty());
    }

    #[test]
    fn a_nondeterministic_sim_is_reported() {
        let noisy = |_s: &Session| -> Option<Box<dyn Headless>> { Some(Box::new(Clock { t: 0.0, id: "noisy", noise: true })) };
        let problems = check_modes(&noisy);
        assert!(!problems.is_empty());
        assert!(problems[0].contains("differ"), "{}", problems[0]);
    }

    #[test]
    fn a_missing_sim_is_reported() {
        let none = |s: &Session| -> Option<Box<dyn Headless>> {
            if s.mode_id == "mask" {
                None
            } else {
                good(s)
            }
        };
        let problems = check_modes(&none);
        assert!(problems.iter().any(|p| p.contains("mask") && p.contains("no headless")), "{problems:?}");
    }

    #[test]
    fn a_panicking_sim_is_caught() {
        struct Boom;
        impl Headless for Boom {
            fn id(&self) -> &'static str {
                "boom"
            }
            fn step_ms(&mut self, _dt: f64) -> Vec<String> {
                panic!("on purpose");
            }
            fn outcome(&self) -> Outcome {
                Outcome::Running
            }
            fn elapsed_ms(&self) -> f64 {
                0.0
            }
            fn digest(&self) -> u64 {
                0
            }
        }
        let f = |_s: &Session| -> Option<Box<dyn Headless>> { Some(Box::new(Boom)) };
        let r = run_headless(&f, "classic", 1);
        assert!(r.is_err());
    }

    #[test]
    fn descent_may_run_longer_than_a_night() {
        assert_eq!(run_limit_ms("descent"), 600_000.0);
        assert_eq!(run_limit_ms("classic"), 270_000.0);
    }

    #[test]
    fn the_shipped_data_checks_clean() {
        let problems = check_data();
        assert!(problems.is_empty(), "{problems:?}");
    }
}
