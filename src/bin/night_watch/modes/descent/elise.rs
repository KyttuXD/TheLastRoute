//! Elise as a living hunter on the shaft's nav graph (contract 9.1). The rules do not change: the sim owns her
//! CLOSENESS (0..100), which decides where on the sill she stands (`map::sill_pos`), exactly as `descent_game.gd`
//! (`_animate_elise`: she walks to that spot at 1.4 m/s, a new side puts her at the far end). What is new is the how:
//!
//! * when a ride starts she comes in from the stairwell (left) or the pipe gallery (right) along the nav graph, hidden
//!   behind the ride, and is on the sill when the car stops;
//! * on the sill she walks to her spot at 1.4 m/s; past 70% she follows the bar exactly;
//! * while the bar barely moves she is free for idle behaviour from `hunters/behaviour.rs` (stare into the window,
//!   tap the terminal, check the locker, sort crates, sit, pace). The moment the bar asks for more than 0.35 m of
//!   progress (or past 70%) she drops the task and walks on. Behaviour never reads the sim, so the timers that decide
//!   the descent are untouched.
//!
//! Pure: no drawing. `update` takes plain numbers.
#![allow(dead_code)]

use super::map;
use super::sim::Side;
use crate::hunters::behaviour::{Agent, BehaviourData, BehaviourEvent, Crowd};
use crate::hunters::nav::Pace;
use macroquad::prelude::Vec3;
use std::rc::Rc;

pub const ELISE: &str = "elise";
/// Seconds she takes to come in from the stairwell during a ride (the ride lasts 1.8 s).
pub const RIDE_WALK_S: f32 = 1.7;
/// Past this closeness she follows the bar exactly and does nothing else.
pub const FOLLOW_ABOVE: f64 = 70.0;
/// How far (m) the bar may get ahead of her while she is free before she walks on.
pub const DEV_MAX: f32 = 0.35;
/// Seconds she must stand still before idle behaviour may start.
pub const CALM_S: f32 = 0.5;

fn dist_xz(a: Vec3, b: Vec3) -> f32 {
    let (dx, dz) = (a.x - b.x, a.z - b.z);
    (dx * dx + dz * dz).sqrt()
}

pub struct EliseCtl {
    crowd: Crowd,
    side: Side,
    /// The spot the sim last asked her to stand at.
    anchor: Vec3,
    in_ride: bool,
    calm: f32,
    placed: bool,
}

impl EliseCtl {
    pub fn new(seed: u32) -> EliseCtl {
        let crowd = Crowd::new(Rc::new(map::nav()), BehaviourData::embedded(), seed ^ 0x454c_4953);
        EliseCtl { crowd, side: Side::L, anchor: Vec3::ZERO, in_ride: false, calm: 0.0, placed: false }
    }

    pub fn agent(&self) -> Option<&Agent> {
        self.crowd.agent(ELISE)
    }

    /// Her sim position and facing.
    pub fn pose(&self) -> Option<(Vec3, f32)> {
        self.crowd.agent(ELISE).map(|a| (a.walker.pos, a.walker.yaw))
    }

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn anchor(&self) -> Vec3 {
        self.anchor
    }

    /// 0..1 soft static while she stares into the window.
    pub fn stare_pulse(&self) -> f32 {
        self.crowd.stare_pulse(map::sill_room(self.side))
    }

    pub fn is_walking(&self) -> bool {
        self.crowd.agent(ELISE).map_or(false, |a| a.walker.is_walking() || a.walker.moving)
    }

    /// A new descent: the first-stare cue can fire again.
    pub fn new_night(&mut self) {
        self.crowd.new_night();
    }

    /// Put her on her spot at once (Retry, the manual clock, an admin jump).
    pub fn snap(&mut self, side: Side, threat: f64) {
        self.side = side;
        self.placed = true;
        self.in_ride = false;
        let target = map::sill_pos(side, threat);
        self.crowd.add_at(ELISE, map::sill_room(side), target, Some(map::look_target(side)));
        self.crowd.set_enabled(ELISE, false);
        self.anchor = target;
        self.calm = 0.0;
    }

    /// A ride starts: she appears in her start room and walks the graph to the sill, in about the ride's time.
    pub fn begin_ride(&mut self, side: Side, threat: f64) {
        self.side = side;
        self.placed = true;
        let target = map::sill_pos(side, threat);
        let look = map::look_target(side);
        if self.crowd.agent(ELISE).is_none() && !self.crowd.add(ELISE, map::start_room(side), 0) {
            self.crowd.add_at(ELISE, map::sill_room(side), target, Some(look));
        }
        self.crowd.set_enabled(ELISE, false);
        self.crowd.send_to(ELISE, map::sill_room(side), target, Some(look), Pace::Budget(RIDE_WALK_S));
        self.anchor = target;
        self.calm = 0.0;
    }

    /// On the sill after the ride: walk to the bar's spot, or leave her to idle behaviour while the bar hardly moves.
    fn follow(&mut self, dt: f32, side: Side, threat: f64) {
        if self.crowd.agent(ELISE).is_none() {
            self.snap(side, threat);
            return;
        }
        let target = map::sill_pos(side, threat);
        let dev = dist_xz(target, self.anchor);
        let close = threat >= FOLLOW_ABOVE;
        if (close && dev > 0.02) || dev > DEV_MAX {
            self.crowd.set_enabled(ELISE, false);
            let look = map::look_target(side);
            let speed = map::View::get().elise_speed.max(0.1);
            if let Some(a) = self.crowd.agent_mut(ELISE) {
                let room = a.walker.room().to_string();
                let dist = dist_xz(a.walker.pos, target);
                a.walker.go_points(&[target], &room, target, Some(look), Pace::Budget(dist / speed));
                a.brain.moved();
            }
            self.anchor = target;
            self.calm = 0.0;
            return;
        }
        let (enabled, resting) = match self.crowd.agent(ELISE) {
            Some(a) => (a.brain.enabled(), a.walker.at_rest() && !a.walker.is_walking()),
            None => return,
        };
        if close {
            if enabled {
                self.crowd.set_enabled(ELISE, false);
            }
            self.calm = 0.0;
        } else if !enabled {
            if resting {
                self.calm += dt;
                if self.calm >= CALM_S {
                    self.crowd.set_enabled(ELISE, true);
                    self.calm = 0.0;
                }
            } else {
                self.calm = 0.0;
            }
        }
    }

    /// Advance by `dt` real seconds. `riding` is the sim's ride flag, `active` false for a level 0 descent (she is not
    /// out there at all). Returns the idle behaviour events of this tick.
    pub fn update(&mut self, dt: f32, side: Side, threat: f64, riding: bool, active: bool) -> Vec<BehaviourEvent> {
        if !active {
            return Vec::new();
        }
        if riding {
            if !self.in_ride || side != self.side || !self.placed {
                self.in_ride = true;
                self.begin_ride(side, threat);
            }
        } else {
            self.in_ride = false;
            self.follow(dt, side, threat);
        }
        self.crowd.tick(dt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(c: &mut EliseCtl, side: Side, threat: f64, riding: bool, seconds: f32) -> Vec<BehaviourEvent> {
        let mut ev = Vec::new();
        let steps = (seconds / 0.05).round() as i32;
        for _ in 0..steps {
            ev.extend(c.update(0.05, side, threat, riding, true));
        }
        ev
    }

    #[test]
    fn a_ride_brings_her_from_the_stairwell_to_the_far_end_of_the_left_sill() {
        let mut c = EliseCtl::new(7);
        c.update(0.0, Side::L, 0.0, true, true);
        let a = c.agent().expect("elise");
        assert_eq!(a.walker.room(), "stair_1");
        assert!(dist_xz(a.walker.pos, vec3_of(-6.0, -6.3)) < 0.01);
        run(&mut c, Side::L, 0.0, true, 2.5);
        let a = c.agent().expect("elise");
        assert!(a.walker.arrived(), "she is there when the car stops");
        assert_eq!(a.walker.room(), "sill_l");
        assert!(dist_xz(a.walker.pos, map::sill_pos(Side::L, 0.0)) < 0.05);
    }

    fn vec3_of(x: f32, z: f32) -> Vec3 {
        macroquad::prelude::vec3(x, 0.0, z)
    }

    #[test]
    fn on_the_right_she_climbs_out_of_the_pipe_gallery() {
        let mut c = EliseCtl::new(3);
        c.update(0.0, Side::R, 30.0, true, true);
        let a = c.agent().expect("elise");
        assert_eq!(a.walker.room(), "pipe_gallery");
        assert_eq!(a.walker.pos.y, -3.0, "the gallery is a floor below");
        let mut max_y = -3.0f32;
        for _ in 0..60 {
            c.update(0.05, Side::R, 30.0, true, true);
            if let Some(a) = c.agent() {
                max_y = max_y.max(a.walker.pos.y);
            }
        }
        assert!(max_y > -0.5, "she climbed the flight, {max_y}");
        run(&mut c, Side::R, 30.0, true, 1.0);
        let a = c.agent().expect("elise");
        assert_eq!(a.walker.room(), "sill_r");
        assert!(dist_xz(a.walker.pos, map::sill_pos(Side::R, 30.0)) < 0.1);
        assert!(a.walker.pos.y.abs() < 0.01);
    }

    #[test]
    fn a_new_side_sends_her_in_again() {
        let mut c = EliseCtl::new(11);
        run(&mut c, Side::L, 10.0, true, 2.5);
        run(&mut c, Side::L, 10.0, false, 1.0);
        let before = c.agent().expect("elise").walker.pos;
        let replaced = c.crowd.replaced;
        c.update(0.05, Side::R, 0.0, true, true);
        let a = c.agent().expect("elise");
        assert!(dist_xz(a.walker.pos, before) < 1.0, "a new ride walks her on from where she stands, it does not put her at the start room");
        assert_eq!(c.crowd.replaced, replaced, "nothing re-places her");
        assert_eq!(c.side(), Side::R);
        run(&mut c, Side::R, 0.0, true, 40.0);
        let a = c.agent().expect("elise");
        assert!(dist_xz(a.walker.pos, map::sill_pos(Side::R, 0.0)) < 1.5, "she walks to the new sill");
    }

    #[test]
    fn past_seventy_percent_she_follows_the_bar_exactly() {
        let mut c = EliseCtl::new(5);
        run(&mut c, Side::L, 70.0, true, 2.5);
        let mut threat = 70.0;
        let mut worst = 0.0f32;
        for i in 0..600 {
            threat += 0.05; // 30% over 30 s
            c.update(0.05, Side::L, threat, false, true);
            if i > 40 {
                let (p, _) = c.pose().expect("pose");
                worst = worst.max(dist_xz(p, map::sill_pos(Side::L, threat)));
            }
        }
        assert!(worst < 0.2, "she stays at the bar, off by {worst}");
        let (p, yaw) = c.pose().expect("pose");
        assert!(p.z > -2.4, "close to the glass, z {}", p.z);
        // she faces the car: front is -z at yaw 0, so facing +z is a half turn
        let d = (yaw.rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI).abs();
        assert!(d < 0.8 || d > std::f32::consts::TAU - 0.8, "yaw {yaw}");
    }

    #[test]
    fn a_sudden_jump_of_the_bar_walks_her_there_at_about_1_4_metres_a_second() {
        let mut c = EliseCtl::new(9);
        run(&mut c, Side::L, 5.0, true, 2.5);
        run(&mut c, Side::L, 5.0, false, 0.5);
        let (before, _) = c.pose().expect("pose");
        run(&mut c, Side::L, 90.0, false, 1.0);
        let (after, _) = c.pose().expect("pose");
        let walked = dist_xz(before, after);
        assert!(walked > 1.2 && walked < 1.6, "one second at 1.4 m/s, walked {walked}");
        run(&mut c, Side::L, 90.0, false, 3.0);
        let (end, _) = c.pose().expect("pose");
        assert!(dist_xz(end, map::sill_pos(Side::L, 90.0)) < 0.05);
    }

    #[test]
    fn while_the_bar_is_still_she_stares_works_and_stays_on_the_sill() {
        for seed in [1u32, 2, 3, 4] {
            let mut c = EliseCtl::new(seed);
            run(&mut c, Side::R, 10.0, true, 2.5);
            let mut events = Vec::new();
            for _ in 0..2400 {
                events.extend(c.update(0.05, Side::R, 10.0, false, true));
                let (p, _) = c.pose().expect("pose");
                assert!(p.x > 0.4 && p.x < 1.9, "seed {seed}: x {}", p.x);
                assert!(p.z > -6.0 && p.z < -1.5, "seed {seed}: z {}", p.z);
            }
            let acted = events.iter().any(|e| matches!(e, BehaviourEvent::StareStart { .. } | BehaviourEvent::TaskStart { .. }));
            assert!(acted, "seed {seed}: two minutes on the sill and she did nothing");
        }
    }

    #[test]
    fn the_first_stare_of_the_descent_is_announced_once() {
        let mut c = EliseCtl::new(21);
        run(&mut c, Side::L, 10.0, true, 2.5);
        let mut firsts = 0;
        for _ in 0..4800 {
            for e in c.update(0.05, Side::L, 10.0, false, true) {
                if matches!(e, BehaviourEvent::FirstStare { .. }) {
                    firsts += 1;
                }
            }
        }
        assert!(firsts <= 1);
    }

    #[test]
    fn the_bar_pulling_ahead_cuts_a_task_short() {
        let mut c = EliseCtl::new(2);
        run(&mut c, Side::L, 5.0, true, 2.5);
        run(&mut c, Side::L, 5.0, false, 60.0);
        // now move the bar a metre
        run(&mut c, Side::L, 35.0, false, 0.2);
        let a = c.agent().expect("elise");
        assert!(!a.brain.enabled(), "idle behaviour is off while she walks on");
        run(&mut c, Side::L, 35.0, false, 2.5);
        let (p, _) = c.pose().expect("pose");
        assert!(dist_xz(p, map::sill_pos(Side::L, 35.0)) < 0.05);
    }

    #[test]
    fn the_same_seed_walks_the_same_way() {
        let trace = |seed: u32| {
            let mut c = EliseCtl::new(seed);
            let mut out: Vec<u32> = Vec::new();
            for i in 0..2400 {
                c.update(0.05, Side::L, 12.0 + (i / 100) as f64, i < 40, true);
                let (p, y) = c.pose().expect("pose");
                out.push(p.x.to_bits() ^ p.z.to_bits().rotate_left(7) ^ y.to_bits().rotate_left(13));
            }
            out
        };
        assert_eq!(trace(42), trace(42));
        assert_ne!(trace(42), trace(43));
    }

    #[test]
    fn level_zero_never_brings_her_out() {
        let mut c = EliseCtl::new(1);
        for _ in 0..100 {
            assert!(c.update(0.05, Side::L, 0.0, true, false).is_empty());
        }
        assert!(c.agent().is_none());
    }

    #[test]
    fn snap_puts_her_on_the_spot() {
        let mut c = EliseCtl::new(1);
        c.snap(Side::R, 60.0);
        let (p, _) = c.pose().expect("pose");
        assert!(dist_xz(p, map::sill_pos(Side::R, 60.0)) < 1e-4);
        assert!(!c.is_walking());
    }

    #[test]
    fn every_camera_room_of_the_graph_is_reachable_from_both_start_rooms() {
        let nav = map::nav();
        for n in nav.nodes().iter().filter(|n| n.kind == crate::hunters::nav::NavKind::Camera) {
            let hub = format!("C:{}", n.room);
            for start in ["P0:stair_1", "P0:pipe_gallery"] {
                assert!(nav.distance(start, &hub).is_some(), "{start} reaches {hub}");
            }
        }
    }
}
