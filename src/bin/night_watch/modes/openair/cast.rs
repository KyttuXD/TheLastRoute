//! The living hunters of Open Air (contract 9.1). The rules (`sim.rs`) say where a hunter is: a distance on a side, a
//! step at a time. This module carries her there over real time on the watch's nav graph (A*, `hunters/nav.rs`), keeps her
//! facing the player while she stands, and lets her stare into a camera or go about a small task between moves
//! (`hunters/behaviour.rs`). It only READS the sim: attack timers, pacers and the random stream are untouched, so a watch
//! plays out the same with or without it. Pure logic: no drawing, no GL.
//!
//! Wynelle is the one hunter without steps: her place on the road follows the chill, so she is sent after a moving target
//! (a short walk each time it gets far enough from her).
#![allow(dead_code)]

use super::data::{active_hunters, behaviour, build_nav, map_of};
use super::sim::{OpenAirSim, Over};
use crate::core::{budget_for, Fnv};
use crate::hunters::behaviour::{Activity, Agent, BehaviourEvent, Crowd};
use crate::hunters::nav::{NavMap, Pace};
use macroquad::prelude::Vec3;
use std::rc::Rc;

/// The nav node a hunter stands on, from the rules ("L2", "R0", "V3", "Q1", "Y2"); empty for Wynelle, who follows the road.
pub fn hunter_key(sim: &OpenAirSim, id: &str) -> String {
    match id {
        "verrine" => {
            if sim.watch == 0 {
                format!("{}{}", sim.climber.side.as_str(), sim.climber.dist)
            } else {
                format!("V{}", sim.v.dist)
            }
        }
        "quenelle" => format!("Q{}", sim.q.dist),
        "ysolde" => format!("Y{}", sim.y.dist),
        _ => String::new(),
    }
}

/// Idle behaviour is allowed only while the rules leave her alone: she is on, the watch is running, and she is not at the
/// door or the edge (or, for Ysolde, out of the trees).
pub fn behaviour_allowed(sim: &OpenAirSim, id: &str) -> bool {
    if sim.over != Over::Running || !sim.hunter_on(id) {
        return false;
    }
    match id {
        "verrine" => {
            if sim.watch == 0 {
                sim.climber.dist > 0
            } else {
                sim.v.dist > 0
            }
        }
        "quenelle" => sim.q.dist > 0,
        "ysolde" => sim.y.dist >= 2,
        _ => true,
    }
}

pub struct Cast {
    pub watch: usize,
    nav: Rc<NavMap>,
    pub crowd: Crowd,
    ids: Vec<&'static str>,
    last_key: Vec<String>,
    /// The road Wynelle walks: first and last node.
    road: Option<(Vec3, Vec3)>,
    /// What a standing hunter looks at (the player's eye).
    pub look: Vec3,
}

impl Cast {
    pub fn new(watch: usize, seed: u32, look: Vec3) -> Cast {
        let watch = watch.min(2);
        let nav = Rc::new(build_nav(watch));
        let crowd = Crowd::new(nav.clone(), behaviour(), seed);
        let map = map_of(watch);
        let road = map.road.as_ref().and_then(|(a, b)| match (nav.pos(a), nav.pos(b)) {
            (Some(pa), Some(pb)) => Some((pa, pb)),
            _ => None,
        });
        let ids = active_hunters(watch);
        let last_key = vec![String::new(); ids.len()];
        Cast { watch, nav, crowd, ids, last_key, road, look }
    }

    pub fn nav(&self) -> &NavMap {
        &self.nav
    }

    pub fn ids(&self) -> &[&'static str] {
        &self.ids
    }

    /// Where Wynelle stands for this chill: along the road, 0 (far down it) to 100 (at the shelter).
    pub fn road_point(&self, chill: f64) -> Vec3 {
        match self.road {
            Some((a, b)) => a.lerp(b, (chill / 100.0).clamp(0.0, 1.0) as f32),
            None => Vec3::ZERO,
        }
    }

    fn room_of_key(&self, key: &str) -> String {
        self.nav.node(key).map(|n| n.room.clone()).unwrap_or_default()
    }

    /// Put everybody on the spot the rules give them, at once.
    pub fn place_all(&mut self, sim: &OpenAirSim) {
        let look = Some(self.look);
        for i in 0..self.ids.len() {
            let id = self.ids[i];
            if id == "wynelle" {
                let pos = self.road_point(sim.chill);
                let room = self.nav.room_at(pos).map(|r| r.to_string()).unwrap_or_else(|| self.room_of_key("W0"));
                self.crowd.add_at(id, &room, pos, look);
            } else {
                let key = hunter_key(sim, id);
                if let Some(n) = self.nav.node(&key) {
                    let (room, p) = (n.room.clone(), n.p);
                    self.crowd.add_at(id, &room, p, look);
                }
                self.last_key[i] = key;
            }
            self.crowd.set_enabled(id, behaviour_allowed(sim, id));
        }
    }

    /// A sim move: A* from where she stands now to the node, walked in about `budget_for(level)` seconds.
    fn send(&mut self, id: &str, key: &str, level: i32) {
        let nav = self.nav.clone();
        let (room, pos) = match nav.node(key) {
            Some(n) => (n.room.clone(), n.p),
            None => return,
        };
        let look = Some(self.look);
        if let Some(a) = self.crowd.agent_mut(id) {
            let legs = nav.route_from(a.walker.pos, key);
            a.brain.moved();
            a.walker.go(&nav, &legs, &room, pos, look, Pace::Budget(budget_for(level)));
        }
    }

    /// Follow the sim: a hunter whose place changed is sent there; Wynelle follows the chill; a standing hunter faces
    /// `look`. `snap` puts everybody on the spot at once (manual clock). Call after every sim step.
    pub fn sync(&mut self, sim: &OpenAirSim, snap: bool) {
        let look = self.look;
        for i in 0..self.ids.len() {
            let id = self.ids[i];
            self.crowd.set_enabled(id, behaviour_allowed(sim, id));
            if id == "wynelle" {
                self.follow_road(sim, snap);
                continue;
            }
            let key = hunter_key(sim, id);
            if key != self.last_key[i] {
                self.last_key[i] = key.clone();
                if snap {
                    if let Some(n) = self.nav.node(&key) {
                        let (room, p) = (n.room.clone(), n.p);
                        self.crowd.add_at(id, &room, p, Some(look));
                    }
                } else {
                    self.send(id, &key, sim.level_of(id));
                }
            }
            if let Some(a) = self.crowd.agent_mut(id) {
                face_player(a, look);
            }
        }
    }

    fn follow_road(&mut self, sim: &OpenAirSim, snap: bool) {
        let target = self.road_point(sim.chill);
        let look = self.look;
        let room = self.nav.room_at(target).map(|r| r.to_string()).unwrap_or_else(|| self.room_of_key("W0"));
        let Some(a) = self.crowd.agent_mut("wynelle") else { return };
        if snap {
            a.walker.place(target, &room, Some(look));
            return;
        }
        if a.brain.activity() == Activity::Staring {
            // she holds her stare; the road moves on under her
            a.walker.final_pos = target;
            return;
        }
        let mut gap = target - a.walker.pos;
        gap.y = 0.0;
        if !a.walker.is_walking() {
            if gap.length() > 0.5 {
                a.brain.moved();
                a.walker.go_points(&[target], &room, target, Some(look), Pace::Walk);
            } else {
                a.walker.final_pos = target;
            }
        }
        face_player(a, look);
    }

    /// Advance walkers and brains by `dt_s` real seconds. Returns the idle-behaviour events (stare, task).
    pub fn tick(&mut self, dt_s: f32) -> Vec<BehaviourEvent> {
        self.crowd.tick(dt_s)
    }

    pub fn agent(&self, id: &str) -> Option<&Agent> {
        self.crowd.agent(id)
    }

    /// The soft static strength for a camera that watches these nav rooms (the strongest stare in view).
    pub fn stare_pulse(&self, rooms: &[String]) -> f32 {
        rooms.iter().map(|r| self.crowd.stare_pulse(r)).fold(0.0, f32::max)
    }

    /// Is anybody staring into a camera that watches these rooms?
    pub fn staring_at(&self, rooms: &[String]) -> bool {
        rooms.iter().any(|r| self.crowd.staring_at(r))
    }

    /// Positions and facing, quantised, for a digest.
    pub fn hash_into(&self, h: &mut Fnv) {
        for a in self.crowd.agents() {
            h.str(&a.id);
            h.i32((a.walker.pos.x * 100.0).round() as i32).i32((a.walker.pos.z * 100.0).round() as i32);
            h.i32((a.walker.yaw * 100.0).round() as i32).i32(a.brain.activity() as i32);
        }
    }
}

/// A standing hunter faces the player; one at a task, in a stare or walking home keeps her own look.
fn face_player(a: &mut Agent, look: Vec3) {
    if matches!(a.brain.activity(), Activity::Held | Activity::Waiting) {
        a.walker.look = Some(look);
    }
}

/// A short name for an idle-behaviour event (headless events, the digest).
pub fn behaviour_label(e: &BehaviourEvent) -> String {
    match e {
        BehaviourEvent::FirstStare { hunter, room, .. } => format!("b:first_stare:{hunter}:{room}"),
        BehaviourEvent::StareStart { hunter, room, .. } => format!("b:stare:{hunter}:{room}"),
        BehaviourEvent::StareEnd { hunter, .. } => format!("b:stare_end:{hunter}"),
        BehaviourEvent::TaskStart { hunter, task, .. } => format!("b:task:{hunter}:{task}"),
        BehaviourEvent::TaskEnd { hunter, task, .. } => format!("b:task_end:{hunter}:{task}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Stream;
    use macroquad::prelude::vec3;

    const EYE: Vec3 = vec3(0.0, 1.7, 1.6);

    fn sim(watch: usize) -> OpenAirSim {
        OpenAirSim::with_rng(watch, Stream::fixed(0.5), -1, -1, -1, false)
    }

    fn settle(c: &mut Cast, seconds: f32) -> Vec<BehaviourEvent> {
        let mut all = Vec::new();
        let steps = (seconds / 0.1).round() as i32;
        for _ in 0..steps {
            all.extend(c.tick(0.1));
        }
        all
    }

    #[test]
    fn everybody_starts_on_the_spot_the_rules_give() {
        for watch in 0..3usize {
            let s = sim(watch);
            let mut c = Cast::new(watch, 5, EYE);
            c.place_all(&s);
            for id in active_hunters(watch) {
                let a = c.agent(id).expect("agent");
                let want = if id == "wynelle" { c.road_point(0.0) } else { c.nav().pos(&hunter_key(&s, id)).expect("node") };
                assert!(a.walker.pos.distance(want) < 0.01, "{id} on watch {watch}");
            }
        }
        let s = sim(0);
        assert_eq!(hunter_key(&s, "verrine"), "R3", "a draw of exactly 0.5 gives the hatch");
        assert_eq!(hunter_key(&sim(2), "ysolde"), "Y2");
        assert_eq!(hunter_key(&sim(2), "quenelle"), "Q3");
        assert_eq!(hunter_key(&sim(2), "verrine"), "V3");
    }

    #[test]
    fn a_sim_step_is_walked_not_jumped() {
        let mut s = sim(0);
        let mut c = Cast::new(0, 5, EYE);
        c.place_all(&s);
        let from = c.agent("verrine").unwrap().walker.pos;
        s.climber.dist = 2;
        c.sync(&s, false);
        let a = c.agent("verrine").unwrap();
        assert!(a.walker.is_walking(), "the move starts a walk");
        assert!(a.walker.pos.distance(from) < 0.01, "she has not teleported");
        settle(&mut c, 3.0);
        let a = c.agent("verrine").unwrap();
        let want = c.nav().pos("R2").unwrap();
        assert!(a.walker.pos.distance(want) < 0.06, "now at R2: {:?}", a.walker.pos);
        assert!(!a.walker.is_walking());
    }

    #[test]
    fn she_is_seen_between_the_two_spots_while_she_walks() {
        let mut s = sim(0);
        let mut c = Cast::new(0, 5, EYE);
        c.place_all(&s);
        s.climber.dist = 2;
        c.sync(&s, false);
        let (from, to) = (c.nav().pos("R3").unwrap(), c.nav().pos("R2").unwrap());
        c.tick(0.5);
        let mid = c.agent("verrine").unwrap().walker.pos;
        assert!(mid.distance(from) > 0.1 && mid.distance(to) > 0.1, "half way along the catwalk: {mid:?}");
        assert!(c.agent("verrine").unwrap().walker.moving);
    }

    #[test]
    fn sent_to_the_other_door_she_walks_round_the_back() {
        let mut s = sim(0);
        let mut c = Cast::new(0, 5, EYE);
        s.climber.dist = 0;
        c.place_all(&s);
        assert!(c.nav().distance("R0", "L2").unwrap() > 20.0, "the long way round");
        s.climber.dist = 2;
        s.climber.side = super::super::sim::Side::L;
        c.sync(&s, false);
        let mut visited_back = false;
        for _ in 0..100 {
            c.tick(0.1);
            let a = c.agent("verrine").unwrap();
            if a.walker.pos.distance(c.nav().pos("B").unwrap()) < 0.45 {
                visited_back = true;
            }
            if !a.walker.is_walking() {
                break;
            }
        }
        assert!(visited_back, "she crossed the back catwalk");
        let a = c.agent("verrine").unwrap();
        assert!(a.walker.pos.distance(c.nav().pos("L2").unwrap()) < 0.06);
    }

    #[test]
    fn wynelle_follows_the_chill_down_the_road() {
        let mut s = sim(1);
        let mut c = Cast::new(1, 5, EYE);
        c.place_all(&s);
        let start = c.agent("wynelle").unwrap().walker.pos;
        assert!(start.distance(c.nav().pos("W0").unwrap()) < 0.01);
        s.chill = 50.0;
        for _ in 0..200 {
            c.sync(&s, false);
            c.tick(0.1);
        }
        let want = c.road_point(50.0);
        let p = c.agent("wynelle").unwrap().walker.pos;
        assert!(p.distance(want) < 0.2, "{p:?} vs {want:?}");
        assert!((want.x - (-13.6)).abs() < 0.01);
    }

    #[test]
    fn a_hunter_at_the_door_is_held_by_the_rules_not_the_brain() {
        let mut s = sim(0);
        s.climber.dist = 0;
        let mut c = Cast::new(0, 5, EYE);
        c.place_all(&s);
        c.sync(&s, false);
        let ev = settle(&mut c, 120.0);
        assert!(ev.is_empty(), "no stare and no task at the door: {ev:?}");
        let p = c.agent("verrine").unwrap().walker.pos;
        assert!(p.distance(c.nav().pos("R0").unwrap()) < 0.06);
        assert!(!behaviour_allowed(&s, "verrine"));
    }

    #[test]
    fn far_out_she_stares_into_the_camera_and_goes_about_small_tasks() {
        let s = sim(0);
        let mut c = Cast::new(0, 7, EYE);
        c.place_all(&s);
        let ev = settle(&mut c, 400.0);
        let stares = ev.iter().filter(|e| matches!(e, BehaviourEvent::StareStart { .. })).count();
        let firsts = ev.iter().filter(|e| matches!(e, BehaviourEvent::FirstStare { .. })).count();
        let tasks = ev.iter().filter(|e| matches!(e, BehaviourEvent::TaskStart { .. })).count();
        assert!(stares >= 1, "she stares at the camera of her room");
        assert_eq!(firsts, 1, "the first stare of the night fires once");
        assert!(tasks >= 1, "she works at a task spot of the gantry");
    }

    #[test]
    fn task_spots_are_never_nearer_the_door_than_her_own_step() {
        // R2 is the nearest she stands before the door rooms; every east gantry task spot is farther from the hatch than R2
        let nav = build_nav(0);
        let hatch = nav.pos("R0").unwrap();
        let r2 = nav.pos("R2").unwrap();
        for room in ["catwalk_e", "catwalk_w"] {
            for spot in nav.task_spots(room, None) {
                let door = if room == "catwalk_e" { hatch } else { nav.pos("L0").unwrap() };
                let own = if room == "catwalk_e" { r2 } else { nav.pos("L2").unwrap() };
                assert!(spot.p.distance(door) >= own.distance(door) - 0.01, "{} is nearer the door than her stand spot", spot.key);
            }
        }
    }

    #[test]
    fn the_cast_never_touches_the_rules() {
        let run = |with_cast: bool| {
            let mut s = OpenAirSim::new(2, 99, -1, -1, -1, false);
            let mut c = Cast::new(2, 99, EYE);
            c.place_all(&s);
            let mut events = Vec::new();
            for _ in 0..2_000 {
                for e in s.step(100.0, &Default::default()) {
                    events.push(e.label());
                }
                if with_cast {
                    c.sync(&s, false);
                    c.tick(0.1);
                }
            }
            (s.digest(), events)
        };
        assert_eq!(run(true), run(false), "same moves, same timers, same catch");
    }

    #[test]
    fn the_same_seed_walks_the_same_way() {
        let walk = || {
            let s = sim(2);
            let mut c = Cast::new(2, 31, EYE);
            c.place_all(&s);
            let ev = settle(&mut c, 120.0);
            let mut h = Fnv::new();
            c.hash_into(&mut h);
            (h.finish(), ev.iter().map(behaviour_label).collect::<Vec<_>>())
        };
        assert_eq!(walk(), walk());
    }

    #[test]
    fn every_camera_room_can_be_reached_on_the_nav_graph() {
        for watch in 0..3usize {
            let nav = build_nav(watch);
            let start = match watch {
                0 => "L3",
                1 => "W0",
                _ => "V3",
            };
            for cam in &map_of(watch).cameras {
                for room in &cam.rooms {
                    let node = nav
                        .nodes()
                        .iter()
                        .find(|n| n.room == *room && n.kind.walkable())
                        .unwrap_or_else(|| panic!("watch {watch}: room {room} has no walkable node"));
                    assert!(!nav.route(start, &node.key).is_empty(), "watch {watch}: {} cannot reach camera room {room}", start);
                    assert!(!nav.cameras(room).is_empty(), "watch {watch}: room {room} has a camera to stare into");
                }
            }
        }
    }

    #[test]
    fn every_hunter_stand_spot_is_reachable_from_every_other() {
        for (watch, keys) in [(0usize, vec!["L3", "L2", "L1", "L0", "R3", "R2", "R1", "R0"]), (2, vec!["V3", "V0", "Q3", "Q0", "Y2", "Y0"])] {
            let nav = build_nav(watch);
            for a in &keys {
                for b in &keys {
                    assert!(!nav.route(a, b).is_empty(), "watch {watch}: {a} to {b}");
                }
            }
        }
    }
}
