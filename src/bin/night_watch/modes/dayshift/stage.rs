//! Salvage Run's hunters on the map (contract 9.1): the sim decides WHEN a visitor moves and WHERE she is (her distance
//! from her door, Ysolde's window state); this file makes her WALK there. Every hunter has a `NavWalker` and an idle
//! brain in one `Crowd` over the ship's navigation graph (`data/nav/dayshift.json`), so Ravelle can be seen coming in from
//! the airlock along the dock and the west gantry, taking one step down the hall each time the sim says so, and Ysolde
//! walking across the scrap yard to the glass. Between moves the brains stare into the room camera, do a small task at a
//! task spot, or wait. None of it reaches the sim: a brain has no access to the visitors' timers.
//!
//! Pure logic (no drawing): used by the Mode and by the headless run. The headless run is `DayshiftHeadless`, at the bottom.
#![allow(dead_code)]

use super::sim::{bot_basket, quota_for, tun, Autoplay, DayOpts, DayshiftSim, YState, VISITORS};
use crate::core::{Fnv, Headless, Mulberry32, Outcome, Session};
use crate::gfx::office::{Office, HALF_W};
use crate::hunters::behaviour::{BehaviourData, BehaviourEvent, Crowd};
use crate::hunters::nav::{NavMap, Pace};
use macroquad::prelude::{vec3, Vec3};
use std::rc::Rc;

pub const NAV_JSON: &str = include_str!("../../data/nav/dayshift.json");

/// Where a visitor stands `dist` steps from her door (`CargoBay.hall_stand`): 0 is the door itself (see `Office::door_stand`).
pub fn hall_stand(door: &str, dist: i32) -> Vec3 {
    let side = if door == "doorL" { -1.0 } else { 1.0 };
    vec3(side * (HALF_W + 1.9 + dist as f32 * 1.3), 0.0, 0.0)
}

/// The room a visitor walks in.
pub fn hall_room(idx: usize) -> &'static str {
    if idx == 0 {
        "gantryL"
    } else {
        "gantryR"
    }
}

pub fn door_of(idx: usize) -> &'static str {
    if idx == 0 {
        "doorL"
    } else {
        "doorR"
    }
}

/// The nav node of a visitor's step `dist` (0..=4; further out uses the outermost).
pub fn hall_key(idx: usize, dist: i32) -> String {
    format!("D:{}{}", if idx == 0 { "L" } else { "R" }, dist.clamp(0, 4))
}

/// Where each visitor starts her walk (her end of the ship) and Ysolde waits.
pub fn origin_key(id: &str) -> &'static str {
    match id {
        "ravelle" => "C:airlock",
        "ceryth" => "C:hold",
        _ => "C:crane",
    }
}

/// The yard side of the glass (CargoBay.YARD_STAND).
pub const WINDOW_STAND: Vec3 = Vec3::new(-1.5, 0.0, crate::gfx::office::HALF_D + 0.95);

pub fn load_nav() -> NavMap {
    NavMap::from_json(NAV_JSON).unwrap_or_else(|_| NavMap::new("dayshift"))
}

/// The hunters in the world.
pub struct Stage {
    nav: Rc<NavMap>,
    pub crowd: Crowd,
    active: [bool; 3],
    dist: [i32; 2],
    ys: YState,
}

impl Stage {
    pub fn new(sim: &DayshiftSim, seed: u32) -> Stage {
        let nav = Rc::new(load_nav());
        let crowd = Crowd::new(nav.clone(), BehaviourData::embedded(), seed ^ 0x4441_5953);
        let act = |id: &str| sim.base_level(id) > 0;
        Stage { nav, crowd, active: [act("ravelle"), act("ceryth"), act("ysolde")], dist: [sim.stalkers[0].dist, sim.stalkers[1].dist], ys: sim.ysolde.state }
    }

    pub fn nav(&self) -> &NavMap {
        &self.nav
    }

    /// Whether the hunter is in this shift.
    pub fn is_active(&self, id: &str) -> bool {
        match id {
            "ravelle" => self.active[0],
            "ceryth" => self.active[1],
            "ysolde" => self.active[2],
            _ => false,
        }
    }

    /// Puts the hunters in: a visitor starts at her end of the ship (the airlock, the hold) and walks in to her first step
    /// (`snap`: she is simply there); Ysolde waits at the crane. `face` is where they look (the player).
    pub fn begin(&mut self, sim: &DayshiftSim, face: Vec3, snap: bool) {
        let nav = self.nav.clone();
        let first_budget = tun().first_budget_s;
        for i in 0..2 {
            if !self.active[i] {
                continue;
            }
            let id = VISITORS[i];
            let dist = sim.stalkers[i].dist;
            self.dist[i] = dist;
            let room = hall_room(i);
            let fin = if dist == 0 { Office::door_stand(door_of(i), 0, 0.0) } else { hall_stand(door_of(i), dist) };
            if snap {
                self.crowd.add_at(id, room, fin, Some(face));
            } else {
                let ok = nav.node(origin_key(id)).map(|n| (n.p, n.room.clone()));
                let (start, start_room) = ok.unwrap_or((fin, room.to_string()));
                self.crowd.add_at(id, &start_room, start, Some(face));
                let legs = nav.route(origin_key(id), &hall_key(i, dist));
                if let Some(a) = self.crowd.agent_mut(id) {
                    a.walker.go(&nav, &legs, room, fin, Some(face), Pace::Budget(first_budget));
                }
            }
            self.crowd.set_enabled(id, dist > 0);
        }
        if self.active[2] {
            let (pos, room) = nav.node("C:crane").map(|n| (n.p, n.room.clone())).unwrap_or((Vec3::ZERO, "yard".to_string()));
            self.crowd.add_at("ysolde", &room, pos, Some(WINDOW_STAND));
            self.ys = sim.ysolde.state;
            if self.ys != YState::Away {
                self.enter_ysolde(face, true);
            }
        }
    }

    /// The sim moved a hunter to somewhere else: A* from where she stands to the node, walked at `pace`.
    fn go_node(&mut self, id: &str, key: &str, room: &str, fin: Vec3, look: Option<Vec3>, pace: Pace) {
        let nav = self.nav.clone();
        if let Some(a) = self.crowd.agent_mut(id) {
            a.brain.moved();
            let legs = nav.route_from(a.walker.pos, key);
            a.walker.go(&nav, &legs, room, fin, look, pace);
        }
    }

    fn enter_ysolde(&mut self, face: Vec3, snap: bool) {
        self.crowd.set_enabled("ysolde", false);
        if snap {
            self.crowd.add_at("ysolde", "yard", WINDOW_STAND, Some(face));
            self.crowd.set_enabled("ysolde", false);
        } else {
            self.go_node("ysolde", "W:stand", "yard", WINDOW_STAND, Some(face), Pace::Budget(tun().knock_budget_s));
        }
    }

    /// Call after every `sim.step` (and after an admin move): a visitor whose distance changed walks to her new step, one
    /// at a door creeps closer as her wait runs down, Ysolde walks to the glass on a knock and back to the crane when she goes.
    pub fn after_step(&mut self, sim: &DayshiftSim, face: Vec3, snap: bool) {
        let step_budget = tun().step_budget_s;
        for i in 0..2 {
            if !self.active[i] {
                continue;
            }
            let id = VISITORS[i];
            let d = sim.stalkers[i].dist;
            let room = hall_room(i);
            if d != self.dist[i] {
                self.dist[i] = d;
                let fin = if d == 0 { Office::door_stand(door_of(i), 0, 0.0) } else { hall_stand(door_of(i), d.min(4)) };
                self.crowd.set_enabled(id, d > 0);
                if snap {
                    self.crowd.add_at(id, room, fin, Some(face));
                    self.crowd.set_enabled(id, d > 0);
                } else {
                    self.go_node(id, &hall_key(i, d), room, fin, Some(face), Pace::Budget(step_budget));
                }
            }
            if d == 0 {
                let progress = (sim.stalkers[i].at_ms / sim.door_ms.max(1.0)) as f32;
                self.crowd.set_final(id, Office::door_stand(door_of(i), 0, progress), Some(face));
            }
        }
        if self.active[2] && sim.ysolde.state != self.ys {
            let was = self.ys;
            self.ys = sim.ysolde.state;
            match self.ys {
                YState::Knock => self.enter_ysolde(face, snap),
                YState::Here => {
                    // from a knock she is already on her way; an admin jump puts her at the glass at once
                    if was == YState::Away {
                        self.enter_ysolde(face, true);
                    }
                }
                YState::Away => {
                    self.crowd.set_enabled("ysolde", true);
                    let crane = self.nav.pos("C:crane").unwrap_or(Vec3::ZERO);
                    if snap {
                        self.crowd.add_at("ysolde", "yard", crane, Some(WINDOW_STAND));
                    } else {
                        self.go_node("ysolde", "C:crane", "yard", crane, Some(WINDOW_STAND), Pace::Walk);
                    }
                }
            }
        }
    }

    /// Advance the walkers and brains by `dt_s` real seconds.
    pub fn tick(&mut self, dt_s: f32) -> Vec<BehaviourEvent> {
        self.crowd.tick(dt_s)
    }

    pub fn pos(&self, id: &str) -> Option<Vec3> {
        self.crowd.agent(id).map(|a| a.walker.pos)
    }

    /// The room a hunter is standing in right now.
    pub fn room_of(&self, id: &str) -> Option<String> {
        self.crowd.agent(id).map(|a| a.walker.room().to_string())
    }

    /// Hunters in view of a room's camera (by position).
    pub fn visible_in(&self, room: &str) -> Vec<String> {
        self.crowd.visible_in(room).iter().map(|a| a.id.clone()).collect()
    }

    /// Positions in centimetres, for the digest.
    fn quantized(&self) -> Vec<(i32, i32)> {
        self.crowd.agents().iter().map(|a| ((a.walker.pos.x * 100.0) as i32, (a.walker.pos.z * 100.0) as i32)).collect()
    }
}

/// A behaviour event as a short name.
pub fn behaviour_name(e: &BehaviourEvent) -> String {
    match e {
        BehaviourEvent::FirstStare { hunter, .. } => format!("first_stare:{hunter}"),
        BehaviourEvent::StareStart { hunter, .. } => format!("stare:{hunter}"),
        BehaviourEvent::StareEnd { hunter, .. } => format!("stare_end:{hunter}"),
        BehaviourEvent::TaskStart { hunter, task, .. } => format!("task:{hunter}:{task}"),
        BehaviourEvent::TaskEnd { hunter, task, .. } => format!("task_end:{hunter}:{task}"),
    }
}

// ---- the headless run -----------------------------------------------------------------------------------------------------------

/// The player's spot the hunters look toward in a headless run (the middle of the bay).
pub const HEADLESS_FACE: Vec3 = Vec3::new(0.0, 0.0, 0.9);

/// The week, the options and the seed a session starts a shift with (shared by the Mode and the headless run).
pub fn shift_setup(session: &Session) -> (i32, DayOpts, u32) {
    let week = if session.is_custom { 4 } else { session.night.clamp(1, 5) };
    let opts = DayOpts {
        upgrades: Vec::new(),
        levels: session.custom_levels.clone(),
        infinite_power: false,
        custom: session.is_custom,
        quota: Some(quota_for(week)),
    };
    (week, opts, session.seed_or_fresh())
}

/// The sim, the hunters and a scripted player: what `--check` and the tests run.
pub struct DayshiftHeadless {
    pub sim: DayshiftSim,
    pub stage: Stage,
    auto: Autoplay,
    behaviour_events: i32,
}

impl DayshiftHeadless {
    pub fn new(session: &Session) -> DayshiftHeadless {
        let (week, mut opts, seed) = shift_setup(session);
        opts.upgrades = bot_basket(week, 0);
        let sim = DayshiftSim::new(week, Mulberry32::new(seed), opts);
        let mut stage = Stage::new(&sim, seed);
        stage.begin(&sim, HEADLESS_FACE, false);
        DayshiftHeadless { sim, stage, auto: Autoplay::new(seed), behaviour_events: 0 }
    }
}

impl Headless for DayshiftHeadless {
    fn id(&self) -> &'static str {
        "dayshift"
    }

    fn step_ms(&mut self, dt_ms: f64) -> Vec<String> {
        let mut names = Vec::new();
        if self.sim.over == super::sim::Over::None {
            let input = self.auto.input(&self.sim, dt_ms);
            for e in self.sim.step(dt_ms, &input) {
                names.push(e.name());
            }
            self.stage.after_step(&self.sim, HEADLESS_FACE, false);
        }
        for e in self.stage.tick((dt_ms / 1000.0) as f32) {
            self.behaviour_events += 1;
            names.push(behaviour_name(&e));
        }
        names
    }

    fn outcome(&self) -> Outcome {
        self.sim.outcome()
    }

    fn elapsed_ms(&self) -> f64 {
        self.sim.elapsed
    }

    fn digest(&self) -> u64 {
        let mut f = Fnv::new();
        f.u64(self.sim.digest()).i32(self.behaviour_events);
        for (x, z) in self.stage.quantized() {
            f.i32(x).i32(z);
        }
        f.finish()
    }
}

/// Why this mode cannot start, if so: the Day Shift nav file does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    crate::hunters::NavMap::from_json(NAV_JSON).err().map(|e| format!("dayshift nav: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::sim::{DayInput, Over};
    use crate::hunters::nav::NavKind;

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(crate::hunters::NavMap::from_json("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    fn session(week: i32, seed: u32) -> Session {
        let mut s = Session::new();
        s.start_mode("dayshift", week, Default::default(), Some(seed));
        s
    }

    fn sim(week: i32, seed: u32) -> DayshiftSim {
        DayshiftSim::new(week, Mulberry32::new(seed), DayOpts::default())
    }

    const ROOMS: [&str; 8] = ["airlock", "dockL", "gantryL", "engine", "bridge", "hold", "gantryR", "yard"];

    #[test]
    fn the_nav_graph_loads_and_validates() {
        let nav = NavMap::from_json(NAV_JSON).unwrap_or_else(|e| panic!("{e}"));
        let problems = nav.validate();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(nav.id, "dayshift");
        for r in ROOMS {
            assert!(nav.hub_key(r).is_some(), "{r}");
        }
    }

    #[test]
    fn every_camera_room_is_reachable_from_every_start() {
        let nav = load_nav();
        for r in ROOMS {
            assert_eq!(nav.cameras(r).len(), 1, "one camera node in {r}");
            let hub = nav.hub_key(r).expect("hub");
            for start in ["C:airlock", "C:hold", "C:crane", "W:stand", "D:L0", "D:R0"] {
                let legs = nav.route(start, &hub);
                assert!(!legs.is_empty(), "{start} -> {hub}");
            }
        }
        // the door steps chain up to the hall and every one has a node
        for i in 0..2 {
            for d in 0..=4 {
                assert!(nav.node(&hall_key(i, d)).is_some(), "{i} {d}");
            }
            let a = nav.node(&hall_key(i, 3)).map(|n| n.p).unwrap_or_default();
            let b = hall_stand(door_of(i), 3);
            assert!(a.distance(b) < 1e-4, "{a:?} {b:?}");
        }
        assert_eq!(nav.node("D:L0").map(|n| n.kind), Some(NavKind::Door));
        assert!(nav.node("W:stand").map(|n| n.p.distance(WINDOW_STAND) < 1e-4).unwrap_or(false));
        // a walk between far rooms is long and goes through the doorways
        let legs = nav.route("C:airlock", "D:L3");
        assert!(legs.iter().any(|l| l.key.starts_with("DW:airlock")));
        assert!(legs.iter().any(|l| l.key == "C:dockL"));
    }

    #[test]
    fn the_door_steps_match_the_godot_hall_stand_and_the_door_stand() {
        assert!(hall_stand("doorL", 3).distance(vec3(-8.8, 0.0, 0.0)) < 1e-5);
        assert!(hall_stand("doorR", 1).distance(vec3(6.2, 0.0, 0.0)) < 1e-5);
        let door = Office::door_stand("doorL", 0, 0.0);
        let n = load_nav().node("D:L0").map(|n| n.p).unwrap_or_default();
        assert!(door.distance(n) < 1e-5, "{door:?} {n:?}");
        let n = load_nav().node("D:R0").map(|n| n.p).unwrap_or_default();
        assert!(Office::door_stand("doorR", 0, 0.0).distance(n) < 1e-5);
    }

    fn walk(stage: &mut Stage, seconds: f32) {
        let mut t = 0.0;
        while t < seconds {
            stage.tick(0.05);
            t += 0.05;
        }
    }

    #[test]
    fn ravelle_walks_in_from_the_airlock_through_the_rooms_to_her_first_step() {
        let s = sim(1, 5);
        let mut stage = Stage::new(&s, 5);
        stage.begin(&s, HEADLESS_FACE, false);
        assert!(stage.is_active("ravelle") && !stage.is_active("ceryth") && !stage.is_active("ysolde"));
        let start = stage.pos("ravelle").expect("ravelle");
        assert!(start.distance(vec3(-29.0, 0.0, 0.0)) < 1e-4, "{start:?}");
        let mut rooms: Vec<String> = Vec::new();
        let mut t = 0.0;
        while t < 7.6 {
            stage.tick(0.05);
            t += 0.05;
            let r = stage.room_of("ravelle").unwrap_or_default();
            if rooms.last() != Some(&r) {
                rooms.push(r);
            }
        }
        assert_eq!(rooms, vec!["airlock".to_string(), "dockL".to_string(), "gantryL".to_string()], "{rooms:?}");
        let a = stage.crowd.agent("ravelle").expect("agent");
        assert!(a.walker.arrived());
        assert!(a.walker.pos.distance(hall_stand("doorL", 3)) < 0.06, "{:?}", a.walker.pos);
        // the sim has not moved her: she is on screen walking for most of the budget, never teleported
        assert!(stage.visible_in("gantryL").contains(&"ravelle".to_string()));
    }

    #[test]
    fn she_is_visible_on_the_feeds_of_the_rooms_she_crosses() {
        let s = sim(1, 5);
        let mut stage = Stage::new(&s, 5);
        stage.begin(&s, HEADLESS_FACE, false);
        let mut seen: Vec<&str> = Vec::new();
        let mut t = 0.0;
        while t < 7.6 {
            stage.tick(0.05);
            t += 0.05;
            for r in ROOMS {
                if stage.visible_in(r).contains(&"ravelle".to_string()) && !seen.contains(&r) {
                    seen.push(r);
                }
            }
        }
        for r in ["airlock", "dockL", "gantryL"] {
            assert!(seen.contains(&r), "never seen in {r}: {seen:?}");
        }
        assert!(!seen.contains(&"hold") && !seen.contains(&"bridge"));
    }

    #[test]
    fn a_close_walks_her_one_step_down_the_hall_and_a_repel_walks_her_back() {
        let mut s = sim(1, 5);
        let mut stage = Stage::new(&s, 5);
        stage.begin(&s, HEADLESS_FACE, true);
        assert!(stage.pos("ravelle").unwrap_or_default().distance(hall_stand("doorL", 3)) < 1e-4);
        s.stalkers[0].dist = 2;
        stage.after_step(&s, HEADLESS_FACE, false);
        let mut moved = false;
        let mut t = 0.0;
        while t < 3.0 {
            stage.tick(0.05);
            t += 0.05;
            moved |= stage.crowd.agent("ravelle").map(|a| a.walker.moving).unwrap_or(false);
        }
        assert!(moved, "she must be seen walking");
        assert!(stage.pos("ravelle").unwrap_or_default().distance(hall_stand("doorL", 2)) < 0.06);
        // at the door she creeps closer as the wait runs down
        s.stalkers[0].dist = 0;
        s.stalkers[0].at_ms = 0.0;
        stage.after_step(&s, HEADLESS_FACE, false);
        walk(&mut stage, 4.0);
        let door0 = Office::door_stand("doorL", 0, 0.0);
        assert!(stage.pos("ravelle").unwrap_or_default().distance(door0) < 0.06);
        assert!(!stage.crowd.agent("ravelle").map(|a| a.brain.enabled()).unwrap_or(true), "the sim owns her at the door");
        s.stalkers[0].at_ms = s.door_ms;
        stage.after_step(&s, HEADLESS_FACE, false);
        walk(&mut stage, 3.0);
        let door1 = Office::door_stand("doorL", 0, 1.0);
        assert!(stage.pos("ravelle").unwrap_or_default().distance(door1) < 0.06, "{:?} vs {:?}", stage.pos("ravelle"), door1);
        // turned away: back to two steps
        s.stalkers[0].dist = 2;
        stage.after_step(&s, HEADLESS_FACE, false);
        walk(&mut stage, 3.5);
        assert!(stage.pos("ravelle").unwrap_or_default().distance(hall_stand("doorL", 2)) < 0.06);
        assert!(stage.crowd.agent("ravelle").map(|a| a.brain.enabled()).unwrap_or(false));
    }

    #[test]
    fn ysolde_crosses_the_yard_on_a_knock_and_walks_back_when_she_goes() {
        let mut s = sim(3, 8);
        let mut stage = Stage::new(&s, 8);
        stage.begin(&s, HEADLESS_FACE, true);
        assert!(stage.is_active("ysolde"));
        let crane = stage.pos("ysolde").unwrap_or_default();
        assert!(crane.distance(vec3(10.0, 0.0, 20.0)) < 1e-4);
        s.ysolde.state = YState::Knock;
        stage.after_step(&s, HEADLESS_FACE, false);
        let mut crossed_yard = false;
        let mut t = 0.0;
        while t < 6.0 {
            stage.tick(0.05);
            t += 0.05;
            let p = stage.pos("ysolde").unwrap_or_default();
            crossed_yard |= p.z > 10.0 && p.z < 18.0;
        }
        assert!(crossed_yard, "seen crossing the yard");
        assert!(stage.pos("ysolde").unwrap_or_default().distance(WINDOW_STAND) < 0.06);
        s.ysolde.state = YState::Here;
        stage.after_step(&s, HEADLESS_FACE, false);
        walk(&mut stage, 2.0);
        assert!(stage.pos("ysolde").unwrap_or_default().distance(WINDOW_STAND) < 0.06, "she stays at the glass");
        s.ysolde.state = YState::Away;
        stage.after_step(&s, HEADLESS_FACE, false);
        let mut min = f32::MAX;
        let mut t = 0.0;
        while t < 16.0 {
            stage.tick(0.05);
            t += 0.05;
            min = min.min(stage.pos("ysolde").unwrap_or_default().distance(crane));
        }
        assert!(min < 0.1, "she walked back to the crane: {min}");
    }

    #[test]
    fn idle_behaviour_never_takes_a_visitor_past_her_step() {
        let s = sim(1, 9);
        let mut stage = Stage::new(&s, 9);
        stage.begin(&s, HEADLESS_FACE, true);
        let stand = hall_stand("doorL", 3);
        let mut events = 0;
        let mut t = 0.0;
        while t < 300.0 {
            events += stage.tick(0.05).len();
            t += 0.05;
            let p = stage.pos("ravelle").unwrap_or_default();
            assert!(p.x <= stand.x + 0.1, "she walked toward the door on her own: {p:?}");
            assert!(p.x >= -14.6);
        }
        assert!(events > 0, "five minutes of waiting and she never stared or worked");
    }

    #[test]
    fn behaviour_does_not_change_the_sim() {
        // the same shift with and without the crowd ticking gives the same sim digest
        let play = |with_crowd: bool| {
            let mut s = sim(3, 31);
            let mut stage = Stage::new(&s, 31);
            stage.begin(&s, HEADLESS_FACE, false);
            let mut a = Autoplay::new(31);
            while s.over == Over::None {
                let inp = a.input(&s, 100.0);
                s.step(100.0, &inp);
                if with_crowd {
                    stage.after_step(&s, HEADLESS_FACE, false);
                    stage.tick(0.1);
                }
            }
            s.digest()
        };
        assert_eq!(play(true), play(false));
    }

    #[test]
    fn the_headless_run_is_deterministic_and_reaches_an_outcome() {
        for week in 1..=5 {
            let run = |seed: u32| {
                let mut h = DayshiftHeadless::new(&session(week, seed));
                let mut names = Vec::new();
                let mut steps = 0;
                while h.outcome() == Outcome::Running && steps < 4000 {
                    names.extend(h.step_ms(100.0));
                    steps += 1;
                }
                (h.digest(), h.outcome(), names.len())
            };
            let a = run(12345);
            let b = run(12345);
            assert_eq!(a, b, "week {week}");
            assert_ne!(a.1, Outcome::Running, "week {week}");
        }
        let h = DayshiftHeadless::new(&session(2, 1));
        assert_eq!(h.id(), "dayshift");
        assert_eq!(h.elapsed_ms(), 0.0);
    }

    #[test]
    fn a_custom_night_uses_week_four_and_the_sliders() {
        let mut s = Session::new();
        let mut c = std::collections::BTreeMap::new();
        c.insert("ravelle".to_string(), 0);
        c.insert("ceryth".to_string(), 9);
        c.insert("ysolde".to_string(), 0);
        s.start_mode("dayshift", 2, c, Some(4));
        let (week, opts, seed) = shift_setup(&s);
        assert_eq!((week, seed), (4, 4));
        assert!(opts.custom);
        assert_eq!(opts.quota, Some(340));
        let h = DayshiftHeadless::new(&s);
        assert!(h.stage.is_active("ceryth") && !h.stage.is_active("ravelle") && !h.stage.is_active("ysolde"));
    }

    #[test]
    fn nothing_the_player_does_reaches_the_walkers() {
        // a visitor held by the light stays put in the sim; the stage only follows the sim's distance
        let mut s = sim(1, 2);
        let mut stage = Stage::new(&s, 2);
        stage.begin(&s, HEADLESS_FACE, true);
        let input = DayInput { light_l: true, ..DayInput::default() };
        for _ in 0..600 {
            s.step(100.0, &input);
            stage.after_step(&s, HEADLESS_FACE, false);
            stage.tick(0.1);
        }
        assert_eq!(s.stalkers[0].dist, 3);
        assert!(stage.pos("ravelle").unwrap_or_default().x <= hall_stand("doorL", 3).x + 0.1);
    }
}
