//! Where the classic hunters physically are (contract 9.1). The sim decides WHEN and WHERE a hunter moves (the Godot rule is
//! untouched); this module hands each sim move to the `Crowd`, which carries her there over real time along the nav graph
//! (`data/nav/classic.json`, A*), and runs her idle behaviour (stare at a camera, do a task, wait) between moves.
//!
//! Pure logic: no drawing, no GL. Nothing here is read by the sim, so attack timers keep running while she stares or works.
#![allow(dead_code)]

use super::sim::ClassicSim;
use super::stalker::door_index;
use crate::core::budget_for;
use crate::gfx::office::Office;
use crate::hunters::{BehaviourData, BehaviourEvent, Crowd, NavMap, Pace};
use macroquad::prelude::Vec3;
use std::collections::BTreeMap;
use std::rc::Rc;

pub struct Placement {
    pub crowd: Crowd,
    /// The sim room each hunter was last handed to the crowd in.
    last_room: BTreeMap<String, String>,
}

impl Placement {
    pub fn new(seed: u32) -> Placement {
        Placement { crowd: Crowd::new(Rc::new(NavMap::classic()), BehaviourData::embedded(), seed), last_room: BTreeMap::new() }
    }

    /// Forget everyone (a restart) and let the first-stare cue fire again.
    pub fn reset(&mut self, seed: u32) {
        *self = Placement::new(seed);
    }

    /// Where a hunter at a door stands and what she looks at (the player). Slot 0 creeps closer with the wait.
    pub fn door_spot(room: &str, slot: usize, progress: f32, player: Vec3) -> (Vec3, Vec3) {
        let slot = slot.min(1);
        let p = if slot == 0 { progress } else { 0.0 };
        (Office::door_stand(room, slot, p), player)
    }

    /// Hand the sim's state to the crowd. A hunter whose sim room changed since the last call is SENT (she walks there; with
    /// `snap` she is put there at once); one that stays at a door has her final spot updated (it creeps closer); one that
    /// stays in a camera room is left to her idle behaviour. `owned(id)` says the mode shows its own pose for her right now
    /// (flinch, retreat, attack), which switches her idle behaviour off.
    pub fn sync(&mut self, sim: &ClassicSim, player_pos: Vec3, snap: bool, owned: &dyn Fn(&str) -> bool) {
        let mut slots: BTreeMap<String, usize> = BTreeMap::new();
        for h in &sim.hunters {
            if h.base <= 0 {
                // not on tonight's floor
                self.crowd.remove(&h.id);
                self.last_room.remove(&h.id);
                continue;
            }
            let slot_idx = *slots.entry(h.room.clone()).and_modify(|n| *n += 1).or_insert(0);
            let slot = slot_idx.min(1);
            let at_door = door_index(&h.room).is_some();
            let progress = (h.at_door_ms / h.attack_ms.max(1.0)) as f32;
            let known = self.crowd.agent(&h.id).is_some();
            let changed = self.last_room.get(&h.id).map_or(true, |r| r != &h.room);
            if !known {
                if at_door {
                    let (pos, look) = Placement::door_spot(&h.room, slot, progress, player_pos);
                    self.crowd.add_at(&h.id, &h.room, pos, Some(look));
                } else {
                    self.crowd.add(&h.id, &h.room, slot);
                }
                self.last_room.insert(h.id.clone(), h.room.clone());
            } else if changed {
                self.last_room.insert(h.id.clone(), h.room.clone());
                if at_door {
                    let (pos, look) = Placement::door_spot(&h.room, slot, progress, player_pos);
                    if snap {
                        self.crowd.add_at(&h.id, &h.room, pos, Some(look));
                    } else {
                        self.crowd.send_to(&h.id, &h.room, pos, Some(look), Pace::Budget(budget_for(h.level)));
                    }
                } else if snap {
                    self.crowd.teleport(&h.id, &h.room, slot);
                } else {
                    self.crowd.send(&h.id, &h.room, slot, Pace::Budget(budget_for(h.level)));
                }
            } else if at_door {
                let (pos, look) = Placement::door_spot(&h.room, slot, progress, player_pos);
                self.crowd.set_final(&h.id, pos, Some(look));
            }
            // her idle behaviour belongs to her camera room: off at a door, and while the mode shows its own pose
            self.crowd.set_enabled(&h.id, !at_door && !owned(&h.id));
        }
    }

    /// Advance the walkers and their idle behaviour by real seconds.
    pub fn tick(&mut self, dt_s: f32) -> Vec<BehaviourEvent> {
        self.crowd.tick(dt_s)
    }

    /// The room a hunter was last sent to.
    pub fn sim_room(&self, id: &str) -> Option<&str> {
        self.last_room.get(id).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{building, cam_rooms};
    use crate::gfx::building::Building;
    use crate::hunters::nav::NavMap;
    use crate::hunters::behaviour::Activity;
    use macroquad::prelude::vec3;
    use std::collections::BTreeMap;

    fn sim(night: i32) -> ClassicSim {
        ClassicSim::new(night, 5, &BTreeMap::new(), false)
    }

    fn free(_: &str) -> bool {
        false
    }

    fn run(p: &mut Placement, seconds: f32) {
        let mut t = 0.0;
        while t < seconds {
            p.tick(0.05);
            t += 0.05;
        }
    }

    #[test]
    fn every_camera_room_is_reachable_on_the_nav_graph_from_the_stage() {
        let nav = NavMap::classic();
        let from = nav.hub_key("stage").expect("stage hub");
        for room in cam_rooms() {
            let to = nav.hub_key(&room).unwrap_or_else(|| panic!("hub of {room}"));
            let legs = nav.route(&from, &to);
            assert!(!legs.is_empty(), "no route stage -> {room}");
            assert!(nav.stand_node(&room, 0).is_some() && nav.stand_node(&room, 1).is_some(), "stand spots of {room}");
            assert_eq!(nav.cameras(&room).len(), 1, "one camera node in {room}");
        }
        // and the two office doors hang off the halls
        for door in ["doorL", "doorR"] {
            let to = nav.hub_key(door).expect("door hub");
            assert!(!nav.route(&from, &to).is_empty(), "no route to {door}");
        }
        assert!(nav.validate().is_empty(), "{:?}", nav.validate());
    }

    #[test]
    fn the_nav_cameras_sit_on_the_building_camera_mounts() {
        let nav = NavMap::classic();
        for r in building().rooms.iter().filter(|r| r.cam > 0) {
            let cam = nav.cameras(&r.id);
            assert_eq!(cam.len(), 1);
            let (mount, _) = Building::camera_transform(&r.id);
            assert!(cam[0].p.distance(mount) < 0.01, "{}: {:?} vs {:?}", r.id, cam[0].p, mount);
            assert_eq!(cam[0].cam, r.cam);
        }
    }

    #[test]
    fn a_sim_move_becomes_a_walk_not_a_jump() {
        let mut s = sim(3);
        let mut p = Placement::new(5);
        let player = vec3(0.0, 0.0, 0.9);
        p.sync(&s, player, false, &free);
        let start = p.crowd.agent("dovelle").map(|a| a.walker.pos).expect("dovelle placed");
        assert!(start.distance(Building::stand_position("stage", 0)) < 0.01);
        // the sim sends her to the dining area
        s.admin_send("dovelle", "dining");
        p.sync(&s, player, false, &free);
        let a = p.crowd.agent("dovelle").expect("agent");
        assert!(a.walker.is_walking(), "she is on her way");
        assert!(a.walker.pos.distance(Building::stand_position("dining", 0)) > 1.0, "she has not arrived yet");
        // she crosses real ground over real time and ends on the dining stand spot
        let mut seen_moving = 0;
        let mut positions = Vec::new();
        for _ in 0..600 {
            p.tick(0.05);
            let a = p.crowd.agent("dovelle").expect("agent");
            if a.walker.moving {
                seen_moving += 1;
                positions.push(a.walker.pos);
            }
            if a.walker.at_rest() && !a.walker.is_walking() {
                break;
            }
        }
        assert!(seen_moving > 5, "{seen_moving}");
        let a = p.crowd.agent("dovelle").expect("agent");
        assert!(a.walker.pos.distance(Building::stand_position("dining", 0)) < 0.3, "{:?}", a.walker.pos);
        assert_eq!(a.walker.room(), "dining");
        // consecutive steps are short: she never flew
        for w in positions.windows(2) {
            assert!(w[0].distance(w[1]) < 0.45, "a step of {}", w[0].distance(w[1]));
        }
        // she is visible on the dining feed now and not on the stage feed
        assert!(p.crowd.visible_in("dining").iter().any(|a| a.id == "dovelle"));
        assert!(!p.crowd.visible_in("stage").iter().any(|a| a.id == "dovelle"));
    }

    #[test]
    fn she_is_seen_leaving_one_feed_and_entering_the_next() {
        let mut s = sim(3);
        let mut p = Placement::new(5);
        let player = vec3(0.0, 0.0, 0.9);
        p.sync(&s, player, false, &free);
        s.admin_send("dovelle", "backstage");
        p.sync(&s, player, false, &free);
        let mut rooms_seen: Vec<&'static str> = Vec::new();
        for _ in 0..600 {
            p.tick(0.05);
            for r in ["stage", "backstage"] {
                if p.crowd.visible_in(r).iter().any(|a| a.id == "dovelle") && !rooms_seen.contains(&r) {
                    rooms_seen.push(r);
                }
            }
        }
        assert_eq!(rooms_seen, vec!["stage", "backstage"], "she shows on the stage feed first, then on backstage");
    }

    #[test]
    fn the_door_spot_creeps_closer_and_faces_the_player() {
        let player = vec3(0.0, 0.0, 0.9);
        let (far, look) = Placement::door_spot("doorL", 0, 0.0, player);
        let (near, _) = Placement::door_spot("doorL", 0, 1.0, player);
        assert!(near.x > far.x && far.x < -4.0, "{far:?} {near:?}");
        assert_eq!(look, player);
        let (second, _) = Placement::door_spot("doorL", 1, 0.9, player);
        assert!(second.x < far.x, "the second stands further down the hall");
        let (b, _) = Placement::door_spot("doorL", 5, 0.0, player);
        let (c, _) = Placement::door_spot("doorL", 1, 0.0, player);
        assert_eq!(b, c, "any slot above 1 stands in slot 1");
        assert_eq!(second, c, "the second one does not creep");
    }

    #[test]
    fn a_hunter_at_a_door_walks_up_to_it_and_idle_behaviour_stays_off() {
        let mut s = sim(3);
        let mut p = Placement::new(5);
        let player = vec3(0.0, 0.0, 0.9);
        p.sync(&s, player, false, &free);
        s.admin_send("dovelle", "doorL");
        p.sync(&s, player, false, &free);
        run(&mut p, 40.0);
        let a = p.crowd.agent("dovelle").expect("agent");
        let (spot, _) = Placement::door_spot("doorL", 0, 0.0, player);
        assert!(a.walker.pos.distance(spot) < 0.3, "{:?} vs {:?}", a.walker.pos, spot);
        assert_eq!(a.walker.room(), "doorL");
        assert_eq!(a.brain.activity(), Activity::Held, "no staring or tasks at the door");
        // her wait creeps her closer: the final spot follows the progress
        s.hunters[0].at_door_ms = s.hunters[0].attack_ms * 0.9;
        p.sync(&s, player, false, &free);
        run(&mut p, 5.0);
        let a = p.crowd.agent("dovelle").expect("agent");
        let (near, _) = Placement::door_spot("doorL", 0, 0.9, player);
        assert!(a.walker.pos.distance(near) < 0.3);
    }

    #[test]
    fn idle_behaviour_runs_between_moves_without_touching_the_sim() {
        let mut s = sim(3);
        let mut p = Placement::new(11);
        let player = vec3(0.0, 0.0, 0.9);
        p.sync(&s, player, false, &free);
        let before = (s.hunters[0].room.clone(), s.hunters[0].at_door_ms, s.hunters[0].next_wait_ms(), s.rng_state());
        let mut did_something = false;
        for _ in 0..1200 {
            p.tick(0.05);
            let a = p.crowd.agent("dovelle").expect("agent");
            if matches!(a.brain.activity(), Activity::Staring | Activity::GoingToTask | Activity::Working) {
                did_something = true;
            }
        }
        assert!(did_something, "in 60 s she stares or works");
        // the sim was never given to the crowd
        let after = (s.hunters[0].room.clone(), s.hunters[0].at_door_ms, s.hunters[0].next_wait_ms(), s.rng_state());
        assert_eq!(before, after);
        // and stepping the sim afterwards is unaffected by what the crowd did
        let mut twin = sim(3);
        for _ in 0..50 {
            s.step(100.0, &Default::default());
            twin.step(100.0, &Default::default());
        }
        assert_eq!(s.rng_state(), twin.rng_state());
    }

    #[test]
    fn a_seeded_crowd_is_deterministic() {
        let trace = |seed: u32| {
            let s = sim(3);
            let mut p = Placement::new(seed);
            p.sync(&s, vec3(0.0, 0.0, 0.9), false, &free);
            let mut out = Vec::new();
            for _ in 0..800 {
                p.tick(0.05);
                let a = p.crowd.agent("dovelle").expect("agent");
                out.push((a.walker.pos.x.to_bits(), a.walker.pos.z.to_bits(), a.brain.activity() as u8));
            }
            out
        };
        assert_eq!(trace(3), trace(3));
        assert_ne!(trace(3), trace(4));
    }

    #[test]
    fn a_hunter_at_level_zero_is_not_on_the_floor() {
        let s = sim(1);
        let mut p = Placement::new(5);
        p.sync(&s, vec3(0.0, 0.0, 0.9), false, &free);
        assert!(p.crowd.agent("dovelle").is_some());
        assert!(p.crowd.agent("ravelle").is_none(), "night 1 starts with Dovelle alone");
        assert!(p.crowd.agent("aki").is_none() && p.crowd.agent("ilyra").is_none());
    }

    #[test]
    fn a_manual_clock_snaps_her_to_the_new_room() {
        let mut s = sim(3);
        let mut p = Placement::new(5);
        let player = vec3(0.0, 0.0, 0.9);
        p.sync(&s, player, true, &free);
        s.admin_send("dovelle", "kitchen");
        p.sync(&s, player, true, &free);
        let a = p.crowd.agent("dovelle").expect("agent");
        assert!(!a.walker.is_walking());
        assert!(a.walker.pos.distance(Building::stand_position("kitchen", 0)) < 0.01);
    }

    #[test]
    fn two_hunters_in_one_room_take_the_two_stand_slots() {
        let mut s = ClassicSim::new(5, 5, &BTreeMap::new(), false);
        s.admin_send("dovelle", "dining");
        s.admin_send("ravelle", "dining");
        let mut p = Placement::new(5);
        p.sync(&s, vec3(0.0, 0.0, 0.9), true, &free);
        let a = p.crowd.agent("dovelle").map(|a| a.walker.pos).expect("a");
        let b = p.crowd.agent("ravelle").map(|a| a.walker.pos).expect("b");
        assert!(a.distance(Building::stand_position("dining", 0)) < 0.01);
        assert!(b.distance(Building::stand_position("dining", 1)) < 0.01);
    }

    #[test]
    fn the_mode_can_switch_her_behaviour_off_for_its_own_pose() {
        let s = sim(3);
        let mut p = Placement::new(5);
        let player = vec3(0.0, 0.0, 0.9);
        p.sync(&s, player, false, &free);
        run(&mut p, 5.0);
        p.sync(&s, player, false, &|id| id == "dovelle");
        assert!(!p.crowd.agent("dovelle").expect("agent").brain.enabled());
        p.sync(&s, player, false, &free);
        assert!(p.crowd.agent("dovelle").expect("agent").brain.enabled());
    }
}
