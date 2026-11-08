//! Where Dead Air's hunters are (contract 9.1), pure: the sim decides WHEN and WHERE, this walks her there on the nav graph
//! (`data/nav/lure.json`), carries the phantoms in and out of the room they show in, and runs the idle behaviour (stare at the
//! camera, do a task, wait) through `Crowd`. Nothing here reads or writes sim state: the sim position of Xivvy is her room;
//! the walker only decides how she gets to that room's stand spot, and the attack timers keep running meanwhile.
//!
//! The mode and the headless run share this, so both walk the same hunters.
#![allow(dead_code)]

use super::data::{is_office, is_vent_room};
use super::map::{door_stand, map};
use super::sim::{Over, Sim};
use crate::core::budget_for;
use crate::hunters::behaviour::{Agent, BehaviourData, BehaviourEvent, Crowd};
use crate::hunters::nav::{NavMap, Pace};
use macroquad::prelude::{vec3, Vec3};
use std::rc::Rc;

pub const LURE_NAV_JSON: &str = include_str!("../../data/nav/lure.json");

/// Seconds before a phantom's time is up that she starts to walk back out.
pub const PHANTOM_LEAVE_S: f32 = 1.4;

/// The booth's centre of attention (where hunters look when they stand at a hatch).
pub fn booth_focus() -> Vec3 {
    vec3(0.0, 1.6, 0.9)
}

/// The station's nav graph (embedded). A broken file gives an empty map, which the tests and `--check` report.
pub fn nav_map() -> NavMap {
    NavMap::from_json(LURE_NAV_JSON).unwrap_or_else(|_| NavMap::new("lure"))
}

/// Where a climate visitor (Wynelle from the left, Solyth from the right) is, from how far in she is (0..1): from the hatch
/// toward the desk. Godot: lerp(door_stand, (-+0.7, 0, -0.9), t).
pub fn visitor_pos(id: &str, t: f32) -> Vec3 {
    let left = id == "wynelle";
    let from = door_stand(if left { "officeL" } else { "officeR" }, 0, 0.0);
    let to = vec3(if left { -0.7 } else { 0.7 }, 0.0, -0.9);
    from.lerp(to, t.clamp(0.0, 1.0))
}

#[derive(Clone, Debug)]
struct PhantomWalk {
    id: String,
    room: String,
    leaving: bool,
}

pub struct Presence {
    pub crowd: Crowd,
    phantom: Option<PhantomWalk>,
    /// Manual clock: hunters are put on their spots at once.
    pub snap: bool,
}

impl Presence {
    pub fn new(seed: u32, snap: bool) -> Presence {
        let mut crowd = Crowd::new(Rc::new(nav_map()), BehaviourData::embedded(), seed ^ 0x4c55_5245);
        crowd.add("xivvy", "lobby", 0);
        Presence { crowd, phantom: None, snap }
    }

    pub fn xivvy(&self) -> Option<&Agent> {
        self.crowd.agent("xivvy")
    }

    /// The phantom currently walking in a room, if any.
    pub fn phantom(&self) -> Option<&Agent> {
        let p = self.phantom.as_ref()?;
        self.crowd.agent(&p.id)
    }

    pub fn phantom_id(&self) -> Option<&str> {
        self.phantom.as_ref().map(|p| p.id.as_str())
    }

    /// Put Xivvy where the sim has her, at once (admin, restart).
    pub fn teleport_xivvy(&mut self, sim: &Sim, focus: Vec3) {
        let room = sim.killer.room.clone();
        if is_office(&room) {
            let p = door_stand(&room, 0, progress_of(sim));
            self.crowd.add_at("xivvy", &room, p, Some(focus));
        } else {
            self.crowd.teleport("xivvy", &room, 0);
        }
    }

    /// Advance the hunters by `dt_s` seconds of the sim's clock. Returns the idle behaviour's events.
    pub fn tick(&mut self, sim: &Sim, dt_s: f32, focus: Vec3) -> Vec<BehaviourEvent> {
        self.follow_killer(sim, focus);
        self.follow_phantom(sim);
        // idle behaviour is off while the sim owns her pose (the vent crawl, the hatch) and while she is not on the floor
        let room = sim.killer.room.as_str();
        let on = sim.lvl > 0 && !is_vent_room(room) && sim.over == Over::None;
        self.crowd.set_enabled("xivvy", on);
        self.crowd.tick(dt_s)
    }

    /// Xivvy's walker follows the sim's room: a new room is a trip along the graph (A*), a vent opening keeps creeping closer.
    fn follow_killer(&mut self, sim: &Sim, focus: Vec3) {
        let room = sim.killer.room.clone();
        let Some(agent) = self.crowd.agent("xivvy") else { return };
        let dest = agent.walker.dest_room().to_string();
        if dest != room {
            let pace = Pace::Budget(budget_for(sim.level_of("xivvy")));
            if is_office(&room) {
                let p = door_stand(&room, 0, 0.0);
                if self.snap {
                    self.crowd.add_at("xivvy", &room, p, Some(focus));
                } else {
                    self.crowd.send_to("xivvy", &room, p, Some(focus), pace);
                }
            } else if self.snap {
                self.crowd.teleport("xivvy", &room, 0);
            } else {
                self.crowd.send("xivvy", &room, 0, pace);
            }
        }
        if is_office(&room) {
            self.crowd.set_final("xivvy", door_stand(&room, 0, progress_of(sim)), Some(focus));
        }
    }

    /// A phantom shows on a camera for a few seconds: she walks in through the room's first doorway, stands, and walks out again.
    fn follow_phantom(&mut self, sim: &Sim) {
        let want = sim.phantom.as_ref().map(|p| (p.id.clone(), p.room.clone(), p.t));
        // an old one goes
        let drop_old = match (&self.phantom, &want) {
            (Some(cur), Some((id, room, _))) => cur.id != *id || cur.room != *room,
            (Some(_), None) => true,
            _ => false,
        };
        if drop_old {
            if let Some(cur) = self.phantom.take() {
                self.crowd.remove(&cur.id);
            }
        }
        let Some((id, room, t)) = want else { return };
        let m = map();
        if self.phantom.is_none() {
            let door = m.sim_to_map(&room).map(|r| m.door_points(r)).and_then(|d| d.first().copied());
            let look = Some(m.camera_transform(&room).0);
            if self.snap {
                self.crowd.add_at(&id, &room, m.stand_position(&room, 1), look);
            } else if let Some(d) = door {
                self.crowd.add_at(&id, &room, d, look);
                self.crowd.send(&id, &room, 1, Pace::Walk);
            } else {
                self.crowd.add(&id, &room, 1);
            }
            self.phantom = Some(PhantomWalk { id: id.clone(), room: room.clone(), leaving: false });
        }
        let show = super::data::tuning().t.phantom_show_ms as f32;
        let leave_now = (t as f32) >= show - PHANTOM_LEAVE_S * 1000.0;
        if leave_now && !self.snap {
            let door = m.sim_to_map(&room).map(|r| m.door_points(r)).and_then(|d| d.first().copied());
            if let (Some(p), Some(d)) = (self.phantom.as_mut(), door) {
                if !p.leaving {
                    p.leaving = true;
                    let id = p.id.clone();
                    self.crowd.send_to(&id, &room, d, None, Pace::Walk);
                }
            }
        }
    }

    /// FNV-friendly snapshot of where everyone stands (millimetres), for digests.
    pub fn snapshot(&self) -> Vec<(String, i32, i32, i32)> {
        self.crowd
            .agents()
            .iter()
            .map(|a| (a.id.clone(), (a.walker.pos.x * 1000.0).round() as i32, (a.walker.pos.y * 1000.0).round() as i32, (a.walker.pos.z * 1000.0).round() as i32))
            .collect()
    }
}

/// How far into her wait at the hatch Xivvy is, 0..1.
pub fn progress_of(sim: &Sim) -> f32 {
    (sim.killer.at_ms / super::data::tuning().t.office_wait_ms.max(1.0)) as f32
}

/// Why this mode cannot start, if so: the Lure nav file does not parse. The mode refuses to start on it and `--check` reports it.
pub fn load_error() -> Option<String> {
    crate::hunters::NavMap::from_json(LURE_NAV_JSON).err().map(|e| format!("lure nav: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hunters::nav::NavKind;
    use super::super::sim::SimOpts;

    #[test]
    fn a_broken_file_is_an_error_and_the_embedded_ones_load() {
        assert!(crate::hunters::NavMap::from_json("{ not json").is_err());
        assert_eq!(load_error(), None);
    }

    fn quiet() -> Sim {
        let mut o = SimOpts::night(6, 1);
        o.level = Some(8);
        o.phantoms = false;
        Sim::new(o)
    }

    fn run(p: &mut Presence, sim: &Sim, seconds: f32) {
        let mut t = 0.0;
        while t < seconds {
            p.tick(sim, 0.05, booth_focus());
            t += 0.05;
        }
    }

    #[test]
    fn the_nav_graph_is_sound() {
        let nav = nav_map();
        let problems = nav.validate();
        assert!(problems.is_empty(), "{problems:?}");
        assert!(nav.len() >= 60);
        assert_eq!(nav.cameras("lobby").len(), 1);
    }

    #[test]
    fn every_camera_room_is_reachable_on_the_nav_graph() {
        let nav = nav_map();
        for from in super::super::data::ROOMS {
            for to in super::super::data::ROOMS {
                let a = nav.hub_key(from).unwrap_or_default();
                let b = nav.stand_key(to, 0).unwrap_or_default();
                assert!(!a.is_empty() && !b.is_empty(), "{from} {to}");
                let legs = nav.route(&a, &b);
                assert!(!legs.is_empty(), "no route {from} -> {to}");
                assert!(nav.distance(&a, &b).is_some());
            }
            assert_eq!(nav.cameras(from).len(), 1, "{from} has one camera node");
            assert!(nav.stand_node(from, 0).is_some() && nav.stand_node(from, 1).is_some());
        }
    }

    #[test]
    fn the_vent_openings_hang_off_the_crawlspaces_by_a_duct() {
        let nav = nav_map();
        let legs = nav.route("C:storage", "D:officeL");
        assert!(legs.len() >= 4);
        assert!(legs.iter().all(|l| !l.jump), "no jump edges: the duct is walked");
        assert!(legs.iter().any(|l| l.key == "V:officeL:4"), "the duct nodes are on the route");
        let r = nav.route("C:storage", "D:officeR");
        assert!(r.iter().all(|l| !l.jump) && r.iter().any(|l| l.key == "V:officeR:4"));
        // both openings are standing spots in their own rooms
        assert_eq!(nav.node("D:officeL").map(|n| n.kind), Some(NavKind::Door));
        assert_eq!(nav.hub_key("officeR").as_deref(), Some("D:officeR"));
    }

    #[test]
    fn the_nav_agrees_with_the_floor_plan() {
        let nav = nav_map();
        let m = map();
        for (a, b) in &m.edges {
            let d = m.door_between(a, b).expect("door");
            let key = format!("DW:{}|{}", m.map_to_sim(a).unwrap_or(""), m.map_to_sim(b).unwrap_or(""));
            let p = nav.pos(&key).unwrap_or_else(|| panic!("no node {key}"));
            let want = m.origin + vec3(d.point.0, 0.0, d.point.1);
            assert!((p - want).length() < 1e-3, "{key}: {p:?} vs {want:?}");
        }
        for sim in super::super::data::ROOMS {
            let (cam, _) = m.camera_transform(sim);
            let node = nav.cameras(sim)[0];
            assert!((node.p - cam).length() < 1e-3, "camera {sim}");
            let s0 = nav.stand_node(sim, 0).map(|n| n.p).unwrap_or(Vec3::ZERO);
            assert!((s0 - m.stand_position(sim, 0)).length() < 1e-3, "stand {sim}");
            let s1 = nav.stand_node(sim, 1).map(|n| n.p).unwrap_or(Vec3::ZERO);
            assert!((s1 - m.stand_position(sim, 1)).length() < 1e-3, "stand 1 {sim}");
        }
        let d = nav.pos("D:officeL").unwrap_or(Vec3::ZERO);
        assert!((d - door_stand("officeL", 0, 0.0)).length() < 1e-3);
    }

    #[test]
    fn every_task_spot_has_a_task_and_lies_in_its_room() {
        let nav = nav_map();
        let data = BehaviourData::embedded();
        let mut rooms_with_tasks = 0;
        for sim in super::super::data::ROOMS {
            let spots = nav.task_spots(sim, None);
            if !spots.is_empty() {
                rooms_with_tasks += 1;
                assert!(spots.len() >= 2, "{sim}");
            }
            for s in spots {
                let task = s.task.clone().unwrap_or_default();
                assert!(data.tasks.contains_key(&task), "{task}");
                assert_eq!(nav.room_at(s.p), Some(sim), "{} is in {sim}", s.key);
            }
        }
        assert_eq!(rooms_with_tasks, 6, "the crawlspaces have none");
    }

    #[test]
    fn xivvy_walks_to_the_next_room_over_real_time() {
        let mut sim = quiet();
        let mut p = Presence::new(5, false);
        run(&mut p, &sim, 0.5);
        assert_eq!(p.xivvy().map(|a| a.walker.room().to_string()), Some("lobby".to_string()));
        sim.killer.room = "arcade".into();
        p.tick(&sim, 0.05, booth_focus());
        let a = p.xivvy().expect("agent");
        assert!(a.walker.is_walking(), "she does not teleport");
        assert_eq!(a.walker.room(), "lobby", "she is still in the room she left");
        let mut seen_between = false;
        let mut t = 0.0;
        while t < 20.0 {
            p.tick(&sim, 0.05, booth_focus());
            t += 0.05;
            let a = p.xivvy().expect("agent");
            if a.walker.moving && p.crowd.nav().room_at(a.walker.pos) == Some("lobby") && p.crowd.visible_in("lobby").len() == 1 {
                seen_between = true; // visible on the reception feed while she walks out
            }
            if a.walker.at_rest() {
                break;
            }
        }
        let a = p.xivvy().expect("agent");
        assert!(seen_between);
        assert_eq!(a.walker.room(), "arcade");
        assert!(a.walker.at_rest());
        assert!((a.walker.pos - map().stand_position("arcade", 0)).length() < 0.1);
        assert_eq!(p.crowd.visible_in("arcade").len(), 1);
        assert!(p.crowd.visible_in("lobby").is_empty());
    }

    #[test]
    fn she_follows_the_graph_through_doorways_not_through_walls() {
        let mut sim = quiet();
        let mut p = Presence::new(5, false);
        sim.killer.room = "storage".into();
        p.tick(&sim, 0.05, booth_focus());
        let nav = nav_map();
        let mut t = 0.0;
        while t < 40.0 {
            p.tick(&sim, 0.05, booth_focus());
            t += 0.05;
            let a = p.xivvy().expect("agent");
            // she is always inside some room rectangle or a doorway on its edge: never in empty space between rooms
            assert!(nav.room_at(a.walker.pos).is_some(), "left the building at {:?}", a.walker.pos);
            if a.walker.at_rest() {
                break;
            }
        }
        assert_eq!(p.xivvy().map(|a| a.walker.room().to_string()), Some("storage".to_string()));
    }

    #[test]
    fn at_the_hatch_she_creeps_up_the_duct_as_her_wait_runs_out() {
        let mut sim = quiet();
        let mut p = Presence::new(5, false);
        sim.killer.room = "officeL".into();
        sim.killer.at_ms = 0.0;
        run(&mut p, &sim, 20.0);
        let far = p.xivvy().expect("agent").walker.pos.x;
        sim.killer.at_ms = 5000.0;
        run(&mut p, &sim, 5.0);
        let near = p.xivvy().expect("agent").walker.pos.x;
        assert!(far < -4.5, "{far}");
        assert!(near > far + 1.0, "{near} vs {far}");
        assert!(near > -3.7);
        assert!(p.xivvy().expect("agent").walker.room() == "officeL");
    }

    #[test]
    fn a_retreat_walks_her_back_out_through_the_crawlspace_jump() {
        let mut sim = quiet();
        let mut p = Presence::new(5, false);
        sim.killer.room = "officeR".into();
        run(&mut p, &sim, 20.0);
        sim.killer.room = "storage".into();
        let mut t = 0.0;
        while t < 30.0 {
            p.tick(&sim, 0.05, booth_focus());
            t += 0.05;
            if p.xivvy().expect("agent").walker.at_rest() {
                break;
            }
        }
        let a = p.xivvy().expect("agent");
        assert_eq!(a.walker.room(), "storage");
        assert!((a.walker.pos - map().stand_position("storage", 0)).length() < 0.1);
    }

    #[test]
    fn a_phantom_walks_in_stands_and_walks_out() {
        let mut sim = quiet();
        sim.phantoms_on = true;
        let mut p = Presence::new(5, false);
        sim.phantom = Some(super::super::sim::Phantom { id: "ferryn".into(), room: "prize".into(), t: 0.0 });
        p.tick(&sim, 0.05, booth_focus());
        let a = p.phantom().expect("she is in the crowd");
        assert_eq!(a.id, "ferryn");
        assert!(a.walker.is_walking(), "she walks in");
        let start = a.walker.pos;
        for _ in 0..40 {
            p.tick(&sim, 0.05, booth_focus());
        }
        let a = p.phantom().expect("agent");
        assert!((a.walker.pos - start).length() > 0.5);
        assert_eq!(p.crowd.visible_in("prize").iter().filter(|x| x.id == "ferryn").count(), 1);
        // her time is nearly up: she heads for the door
        if let Some(ph) = sim.phantom.as_mut() {
            ph.t = 3000.0;
        }
        p.tick(&sim, 0.05, booth_focus());
        assert!(p.phantom().expect("agent").walker.is_walking());
        // gone
        sim.phantom = None;
        p.tick(&sim, 0.05, booth_focus());
        assert!(p.phantom().is_none());
        assert!(p.crowd.agent("ferryn").is_none());
    }

    #[test]
    fn the_idle_behaviour_never_reaches_the_sims_clocks() {
        // the sim is stepped with and without the crowd ticking: same events, same state
        let play = |with_crowd: bool| {
            let mut o = SimOpts::night(5, 4242);
            o.level = Some(10);
            let mut sim = Sim::new(o);
            let mut p = Presence::new(4242, false);
            let mut names = Vec::new();
            for _ in 0..1800 {
                for e in sim.step(100.0, "") {
                    names.push(e.name());
                }
                if with_crowd {
                    p.tick(&sim, 0.1, booth_focus());
                }
                if sim.over != Over::None {
                    break;
                }
            }
            (names, sim.digest())
        };
        assert_eq!(play(true), play(false));
    }

    #[test]
    fn climate_visitors_creep_in_from_their_side() {
        let w0 = visitor_pos("wynelle", 0.0);
        let w1 = visitor_pos("wynelle", 1.0);
        assert!(w0.x < -4.0 && (w1 - vec3(-0.7, 0.0, -0.9)).length() < 1e-5);
        assert!(visitor_pos("solyth", 0.0).x > 4.0);
        assert!(visitor_pos("solyth", 1.0).x > 0.0);
    }

    #[test]
    fn she_stares_into_the_camera_or_works_a_task_between_moves() {
        let sim = quiet();
        let mut p = Presence::new(11, false);
        let mut staring = false;
        let mut working = false;
        let mut t = 0.0;
        while t < 300.0 && !(staring && working) {
            for e in p.tick(&sim, 0.05, booth_focus()) {
                match e {
                    BehaviourEvent::StareStart { .. } => staring = true,
                    BehaviourEvent::TaskStart { .. } => working = true,
                    _ => {}
                }
            }
            t += 0.05;
        }
        assert!(staring || working, "she does something between moves");
    }
}
