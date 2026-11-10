#![allow(dead_code)]
//! Where each Masked Hours hunter is and how she gets there (contract 9.1). The rules (`sim.rs`) decide WHEN a hunter moves
//! and WHERE; this module turns that into targets (port of `_target` in `mask_game.gd`) and lets the shared `Crowd`
//! (hunters/behaviour.rs: a `NavWalker` on the map's nav graph plus an idle-behaviour brain per hunter) carry her there over
//! real time. Pure: no drawing, no GL, so tests and `--check` run without a window.
//!
//! * A sim move to another room is an A* trip along `data/nav/mask.json` at the Godot budget (2 s, speed clamped 1.2..6).
//! * A small move inside a room (a hall step, a creep, Glyra's advance) is a straight walk at walking pace.
//! * Umbriel's run is driven by the rules' progress (placed every frame), as in Godot.
//! * Idle behaviour (stare / task / wait) is on only while the rules leave a hunter at home; the rules' timers never see it.

use super::data::{dark_grace_ms, map, rules, side_of_room, NAV_JSON, RUSHER, VENT_HUNTERS};
use super::sim::{HallState, MaskSim, RushState, VentState};
use crate::hunters::{BehaviourData, BehaviourEvent, Crowd, NavMap, Pace};
use macroquad::prelude::{vec3, Vec3};
use std::collections::BTreeMap;
use std::rc::Rc;

/// Where one hunter should be right now and what the picture needs to know about her.
#[derive(Clone, Debug)]
pub struct Target {
    pub room: String,
    pub pos: Vec3,
    /// What she looks at (the player, or the room's camera).
    pub face: Vec3,
    /// The rules place her every frame (Umbriel's run, Nyxie's walk).
    pub driven: bool,
    /// Idle behaviour may run (she is at home and the rules do nothing with her).
    pub enabled: bool,
    /// She is on the floor tonight and the light lets you see her.
    pub shown: bool,
    /// In the office's side vent (the side letter).
    pub vent: Option<char>,
    /// In a duct camera room (the side letter).
    pub duct: Option<char>,
    /// In a shaft (a duct or an office vent): the vent crawl pose.
    pub shaft: bool,
    /// Running (Umbriel's rush).
    pub run: bool,
}

impl Target {
    fn new(room: &str, pos: Vec3, face: Vec3) -> Target {
        Target {
            room: room.to_string(),
            pos,
            face,
            driven: false,
            enabled: false,
            shown: true,
            vent: None,
            duct: None,
            shaft: false,
            run: false,
        }
    }
}

/// Is this hunter on the floor tonight (Nyxie only in the dark, handled by `target_of`)?
pub fn present(sim: &MaskSim, id: &str) -> bool {
    match id {
        "glyra" => sim.glyra_present(),
        "nyxie" => sim.dark > 0.0 && sim.base_level("nyxie") > 0,
        _ => sim.level(id) > 0,
    }
}

/// The target of one hunter from the rules' state. `player` is where the player stands (the hunters at the office face it).
pub fn target_of(id: &str, sim: &MaskSim, player: Vec3) -> Option<Target> {
    let m = map();
    let r = rules();
    let steps = r.t("hallSteps");
    let mut t = match id {
        "thessine" => {
            let h = &sim.hall;
            let mut t = match h.state {
                HallState::Hall => Target::new("hall", m.hall_pos(h.step, steps), player),
                HallState::Office => Target::new("office", m.stand_at("office", 0, "stand"), player),
                _ => {
                    let cam = m.camera_origin(&h.room).unwrap_or(player);
                    Target::new(&h.room, m.stand_at(&h.room, 0, "stand"), cam)
                }
            };
            t.enabled = matches!(h.state, HallState::Away | HallState::Room);
            t
        }
        "umbriel" => {
            let x = &sim.rush;
            if matches!(x.state, RushState::Run | RushState::Door) {
                let p = if x.state == RushState::Door { 1.0 } else { (x.t / x.need.max(1.0)).min(1.0) as f32 };
                let mut t = Target::new("hall", m.rush_pos(p), player);
                t.driven = true;
                t.run = true;
                t
            } else {
                let cam = m.camera_origin(&x.room).unwrap_or(player);
                let slot = if x.state == RushState::Poised { 1 } else { 0 };
                let mut t = Target::new(&x.room, m.stand_at(&x.room, slot, "stand"), cam);
                t.enabled = matches!(x.state, RushState::Away | RushState::Poised);
                t
            }
        }
        "verrine" | "haelith" | "ilyra" => {
            let v = sim.vent_state(id);
            if v.state == VentState::Vent {
                let mut t = Target::new(&format!("vent{}", v.side), m.vent_stand_of(v.side), player);
                t.vent = Some(v.side);
                t.shaft = true;
                t
            } else {
                let at = if id == "ilyra" && side_of_room(&v.room).is_some() && v.room.starts_with("duct") {
                    ["far", "stand", "near"][(v.step - 1).clamp(0, 2) as usize]
                } else {
                    "stand"
                };
                let slot = if id == "haelith" { 1 } else { 0 };
                let cam = m.camera_origin(&v.room).unwrap_or(player);
                let mut t = Target::new(&v.room, m.stand_at(&v.room, slot, at), cam);
                if v.room.starts_with("duct") {
                    t.duct = side_of_room(&v.room);
                }
                t.shaft = v.state == VentState::Duct;
                t.enabled = v.state == VentState::Away;
                t
            }
        }
        "glyra" => {
            let back = (sim.box_zero / r.t("boxGraceMs").max(1.0)).clamp(0.0, 1.0) as f32;
            let cam = m.camera_origin("music").unwrap_or(player);
            let mut t = Target::new("music", m.glyra_pos(back), cam);
            t.enabled = sim.box_zero == 0.0;
            t
        }
        "nyxie" => {
            let lv = sim.base_level("nyxie");
            let k = (sim.dark / dark_grace_ms(lv).max(1.0)).clamp(0.0, 1.0) as f32;
            let mut t = Target::new("hall", m.nyxie_pos(k), player);
            t.driven = true;
            t
        }
        _ => return None,
    };
    let mut shown = present(sim, id);
    if let Some(side) = t.vent {
        shown = shown && sim.lit.side(side);
    } else if let Some(side) = t.duct {
        shown = shown && sim.lit.side(side);
    }
    t.shown = shown;
    Some(t)
}

/// The hunters, walking. One per cast member.
pub struct Stage {
    pub crowd: Crowd,
    last: BTreeMap<String, Target>,
}

impl Stage {
    pub fn new(seed: u32) -> Stage {
        let nav = NavMap::from_json(NAV_JSON).unwrap_or_else(|_| NavMap::new("mask"));
        Stage { crowd: Crowd::new(Rc::new(nav), BehaviourData::embedded(), seed), last: BTreeMap::new() }
    }

    pub fn nav(&self) -> &NavMap {
        self.crowd.nav()
    }

    /// The last target a hunter was given.
    pub fn target(&self, id: &str) -> Option<&Target> {
        self.last.get(id)
    }

    pub fn pos(&self, id: &str) -> Option<Vec3> {
        self.crowd.agent(id).map(|a| a.walker.pos)
    }

    /// Bring the walkers in line with the rules. `snap` (a screenshot, a test) puts everyone on the spot at once.
    pub fn sync(&mut self, sim: &MaskSim, player: Vec3, snap: bool) {
        for id in rules().cast.iter() {
            let t = match target_of(id, sim, player) {
                Some(t) => t,
                None => continue,
            };
            let (first, moved_room, moved_pos) = match self.last.get(id.as_str()) {
                Some(p) => (false, p.room != t.room, p.pos.distance(t.pos) > 0.01),
                None => (true, true, true),
            };
            let look = Some(t.face);
            if first || t.driven || (snap && (moved_room || moved_pos)) {
                self.crowd.add_at(id, &t.room, t.pos, look);
            } else if moved_room {
                self.crowd.send_to(id, &t.room, t.pos, look, Pace::Budget(2.0));
            } else if moved_pos {
                let room = t.room.clone();
                if let Some(a) = self.crowd.agent_mut(id) {
                    a.brain.moved();
                    a.walker.go_points(&[t.pos], &room, t.pos, look, Pace::Walk);
                }
            }
            self.crowd.set_enabled(id, t.enabled && !t.driven);
            self.last.insert(id.clone(), t);
        }
    }

    /// Advance every walker and brain by `dt` seconds (real time).
    pub fn tick(&mut self, dt: f32) -> Vec<BehaviourEvent> {
        self.crowd.tick(dt)
    }

    /// The vent hunters in a shaft right now (the vent crawl pose).
    pub fn in_shaft(&self, id: &str) -> bool {
        VENT_HUNTERS.contains(&id) && self.last.get(id).map(|t| t.shaft).unwrap_or(false)
    }

    /// A stable summary of every hunter's position, to the millimetre (digests, determinism checks).
    pub fn positions(&self) -> Vec<(String, i32, i32, i32)> {
        let mut out = Vec::new();
        for a in self.crowd.agents() {
            let p = a.walker.pos;
            out.push((a.id.clone(), (p.x * 1000.0).round() as i32, (p.y * 1000.0).round() as i32, (p.z * 1000.0).round() as i32));
        }
        out
    }
}

/// The player's feet in the office (hunters at the desk face this).
pub fn office_player_spot() -> Vec3 {
    let s = map().spawn;
    if s == Vec3::ZERO {
        vec3(0.0, 0.0, 1.35)
    } else {
        s
    }
}

pub fn rusher_id() -> &'static str {
    RUSHER
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sim(night: i32) -> MaskSim {
        let mut s = MaskSim::new(night, 11, BTreeMap::new(), false);
        s.hall.cool = 1e12;
        s.rush.cool = 1e12;
        s.vent_clock.cool = 1e12;
        s
    }

    fn player() -> Vec3 {
        vec3(0.0, 1.7, 1.35)
    }

    /// Tick until `pred` holds or `max_s` seconds pass.
    fn until(st: &mut Stage, max_s: f32, mut pred: impl FnMut(&Stage) -> bool) -> Option<f32> {
        let dt = 0.05;
        let mut t = 0.0;
        while t < max_s {
            st.tick(dt);
            t += dt;
            if pred(st) {
                return Some(t);
            }
        }
        None
    }

    #[test]
    fn every_camera_room_is_reachable_on_the_nav_graph_from_every_home() {
        let st = Stage::new(1);
        let nav = st.nav();
        assert!(nav.validate().is_empty(), "{:?}", nav.validate());
        let homes: Vec<String> = ["thessine", "umbriel", "verrine", "haelith", "ilyra", "glyra"].iter().map(|id| rules().home(id).to_string()).collect();
        for home in &homes {
            let from = nav.hub_key(home).expect("home hub");
            for room in rules().cam_rooms() {
                let to = nav.hub_key(&room).expect("camera room hub");
                let legs = nav.route(&from, &to);
                assert!(!legs.is_empty() || from == to, "no way from {home} to {room}");
            }
            for room in ["office", "ventL", "ventR"] {
                let to = nav.hub_key(room).expect("hub");
                assert!(!nav.route(&from, &to).is_empty(), "no way from {home} to {room}");
            }
        }
    }

    #[test]
    fn every_stand_spot_matches_the_map_numbers() {
        let st = Stage::new(1);
        let nav = st.nav();
        for room in ["ballroom", "music", "costume", "gallery", "crawl"] {
            for slot in 0..2 {
                let n = nav.stand_node(room, slot).expect("stand node");
                let want = map().stand_at(room, slot, "stand");
                assert!((n.p - want).length() < 1e-3, "{room} slot {slot}: {:?} vs {:?}", n.p, want);
            }
        }
        // every camera node sits at its mount
        for room in rules().cam_rooms() {
            let c = nav.cameras(&room);
            assert_eq!(c.len(), 1, "{room}");
            assert!((c[0].p - map().camera(&room).expect("cam").mount).length() < 1e-3);
        }
    }

    #[test]
    fn a_hunter_walks_between_rooms_over_real_time_and_shows_in_the_feeds_on_the_way() {
        let mut s = sim(1);
        let mut st = Stage::new(1);
        st.sync(&s, player(), true);
        assert!(st.crowd.visible_in("costume").iter().any(|a| a.id == "thessine"));
        // the rules put her in the ballroom: she is not there at once
        s.hall.state = HallState::Room;
        s.hall.room = "ballroom".to_string();
        st.sync(&s, player(), false);
        // (no idle behaviour in this test: she goes straight to her spot)
        st.crowd.set_enabled("thessine", false);
        let start = st.pos("thessine").expect("pos");
        st.tick(0.1);
        let after = st.pos("thessine").expect("pos");
        let moved = start.distance(after);
        assert!(moved > 0.0 && moved <= 0.7, "moved {moved} in 0.1 s");
        // she crosses the prize-corner-free lane: she shows up on the game area feed before she arrives
        let mut saw_ballroom_early = false;
        let mut saw_costume = false;
        let end = map().stand_at("ballroom", 0, "stand");
        for _ in 0..400 {
            st.tick(0.05);
            let p = st.pos("thessine").expect("pos");
            if st.crowd.visible_in("costume").iter().any(|a| a.id == "thessine") {
                saw_costume = true;
            }
            if st.crowd.visible_in("ballroom").iter().any(|a| a.id == "thessine") && p.distance(end) > 1.0 {
                saw_ballroom_early = true;
            }
        }
        assert!(saw_costume, "she leaves the parts room on foot");
        assert!(saw_ballroom_early, "and walks across the game area to her stand spot");
        assert!(st.pos("thessine").expect("pos").distance(end) < 0.1, "she arrives");
        assert_eq!(st.crowd.agent("thessine").expect("agent").walker.room(), "ballroom");
    }

    #[test]
    fn hall_steps_are_short_walks_toward_the_office() {
        let mut s = sim(1);
        let mut st = Stage::new(1);
        s.place_hunter("thessine", "hall");
        s.hall.step = 0;
        st.sync(&s, player(), true);
        let z0 = st.pos("thessine").expect("pos").z;
        assert!((z0 + 15.0).abs() < 1e-3);
        s.hall.step = 1;
        st.sync(&s, player(), false);
        until(&mut st, 6.0, |st| st.crowd.agent("thessine").map(|a| a.walker.at_rest()).unwrap_or(false));
        let z1 = st.pos("thessine").expect("pos").z;
        assert!((z1 - map().hall_pos(1, 4.0).z).abs() < 0.05, "z {z1}");
        // the hall light and the camera see her in the hall rectangle
        assert!(st.crowd.visible_in("hall").iter().any(|a| a.id == "thessine"));
    }

    #[test]
    fn the_rusher_is_driven_by_the_rules_progress() {
        let mut s = sim(3);
        s.place_hunter("umbriel", "hall");
        let mut st = Stage::new(1);
        s.rush.t = 0.0;
        st.sync(&s, player(), false);
        assert!((st.pos("umbriel").expect("pos").z + 15.0).abs() < 1e-3);
        s.rush.t = s.rush.need * 0.5;
        st.sync(&s, player(), false);
        let z = st.pos("umbriel").expect("pos").z;
        assert!((z - map().rush_pos(0.5).z).abs() < 1e-3);
        s.rush.state = RushState::Door;
        st.sync(&s, player(), false);
        assert!((st.pos("umbriel").expect("pos").z + 2.6).abs() < 1e-3);
        let t = st.target("umbriel").expect("target");
        assert!(t.driven && t.run && !t.enabled);
    }

    #[test]
    fn verrine_walks_from_the_hub_through_the_duct_to_the_vent() {
        let mut s = sim(2);
        let mut st = Stage::new(1);
        st.sync(&s, player(), true);
        assert_eq!(st.crowd.agent("verrine").expect("a").walker.room(), "crawl");
        s.place_hunter("verrine", "ductL");
        st.sync(&s, player(), false);
        let mut in_duct = false;
        let step = |st: &mut Stage, in_duct: &mut bool| {
            st.tick(0.05);
            if st.crowd.visible_in("ductL").iter().any(|a| a.id == "verrine") {
                *in_duct = true;
            }
        };
        for _ in 0..200 {
            step(&mut st, &mut in_duct);
        }
        assert!(in_duct);
        let at = st.pos("verrine").expect("pos");
        assert!((at - map().stand_at("ductL", 0, "stand")).length() < 0.1, "waits mid duct: {at:?}");
        // now at the vent: she goes through the hatch
        s.place_hunter("verrine", "ventL");
        st.sync(&s, player(), false);
        for _ in 0..200 {
            st.tick(0.05);
        }
        let at = st.pos("verrine").expect("pos");
        assert!((at - map().vent_stand_of('L')).length() < 0.1, "in the vent: {at:?}");
        assert!(st.crowd.visible_in("ventL").iter().any(|a| a.id == "verrine"));
        assert!(st.in_shaft("verrine"));
    }

    #[test]
    fn a_vent_hunter_is_only_shown_in_the_light() {
        let mut s = sim(2);
        s.place_hunter("verrine", "ventL");
        let t = target_of("verrine", &s, player()).expect("t");
        assert!(!t.shown, "dark vent: nothing to see");
        s.lit.l = true;
        assert!(target_of("verrine", &s, player()).expect("t").shown);
        s.place_hunter("verrine", "ductR");
        assert!(!target_of("verrine", &s, player()).expect("t").shown);
        s.lit.r = true;
        assert!(target_of("verrine", &s, player()).expect("t").shown);
    }

    #[test]
    fn glyra_walks_toward_the_office_as_the_box_stays_empty() {
        let mut s = sim(2);
        s.box_pct = 0.0;
        s.box_zero = 0.0;
        let a = target_of("glyra", &s, player()).expect("t");
        assert!(a.enabled && a.shown);
        s.box_zero = 3000.0;
        let b = target_of("glyra", &s, player()).expect("t");
        assert!(!b.enabled);
        assert!((b.pos.z - a.pos.z - 1.5).abs() < 1e-4, "half of 3 m at half the grace");
    }

    #[test]
    fn nyxie_comes_down_the_hall_only_in_the_dark() {
        let mut s = sim(1);
        let hidden = target_of("nyxie", &s, player()).expect("t");
        assert!(!hidden.shown && hidden.driven);
        s.dark = 1.0;
        assert!(target_of("nyxie", &s, player()).expect("t").shown);
        s.dark = dark_grace_ms(3);
        let near = target_of("nyxie", &s, player()).expect("t");
        assert!((near.pos - map().nyxie_to).length() < 1e-4);
    }

    #[test]
    fn idle_hunters_stare_and_work_between_the_rules_moves() {
        let s = sim(5);
        let mut st = Stage::new(7);
        st.sync(&s, player(), true);
        let mut doing = 0;
        let mut first_stare = false;
        for _ in 0..4000 {
            for e in st.tick(0.05) {
                match e {
                    BehaviourEvent::TaskStart { .. } | BehaviourEvent::StareStart { .. } => doing += 1,
                    BehaviourEvent::FirstStare { .. } => first_stare = true,
                    _ => {}
                }
            }
        }
        assert!(doing > 0, "200 s at home without a single stare or task");
        let _ = first_stare;
    }

    #[test]
    fn the_rules_own_a_hunter_while_she_is_at_the_door() {
        let mut s = sim(1);
        let mut st = Stage::new(1);
        s.place_hunter("thessine", "office");
        st.sync(&s, player(), true);
        assert!(!st.target("thessine").expect("t").enabled);
        for _ in 0..2000 {
            let ev = st.tick(0.05);
            assert!(ev.iter().all(|e| !matches!(e, BehaviourEvent::TaskStart { hunter, .. } if hunter == "thessine")));
        }
        assert!((st.pos("thessine").expect("pos") - map().stand_at("office", 0, "stand")).length() < 0.05);
    }

    #[test]
    fn the_same_seed_walks_the_same_way() {
        let run = |seed: u32| {
            let mut s = sim(4);
            let mut st = Stage::new(seed);
            st.sync(&s, player(), true);
            s.place_hunter("thessine", "ballroom");
            st.sync(&s, player(), false);
            for _ in 0..3000 {
                st.tick(0.05);
            }
            st.positions()
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6), "a different seed idles differently");
    }
}
