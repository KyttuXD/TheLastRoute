//! One classic hunter (port of `night_watch_stalker.gd`, which is `createStalker` of the 2D game). She walks the building
//! graph toward her door after a random wait, then waits at it: a shut door (`t_closed_ms`) or the light held on her
//! (`push_ms`) sends her back, otherwise she attacks after `attack_ms`. Night 7's Kathy differs: the light never holds her
//! wait, a shut door has to stay shut for a rolled time and she then FADES to a fresh door.
//!
//! Pure and deterministic: every roll comes from the one seeded `Mulberry32` the sim passes in, in the same order as the
//! GDScript draws. No drawing, no sound, no window.
#![allow(dead_code)]

use super::sim::SimEvent;
use crate::core::{building, distances_to, roll_hold, rules, CastDef, Mulberry32, Pacer};
use std::collections::HashMap;

/// 0 for "doorL", 1 for "doorR", None for any other room.
pub fn door_index(room: &str) -> Option<usize> {
    match room {
        "doorL" => Some(0),
        "doorR" => Some(1),
        _ => None,
    }
}

/// The door id of an index (0 left, 1 right).
pub fn door_name(i: usize) -> &'static str {
    if i == 0 {
        "doorL"
    } else {
        "doorR"
    }
}

/// What the scene tells a hunter each step (the `ctx` dictionary of the GDScript).
#[derive(Clone, Copy, Debug)]
pub struct StepCtx<'a> {
    /// The monitor is up (and the power is on).
    pub cam: bool,
    pub cam_room: &'a str,
    /// Door lights held (left, right), already false once the power is out.
    pub lit: [bool; 2],
    /// A partner stands in front of her in the line at this door.
    pub behind: bool,
}

/// The pose a hunter shows at her door, from how long she has waited.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DoorPose {
    Peek,
    Stand,
    Lean,
}

#[derive(Clone, Debug)]
pub struct Stalker {
    pub id: String,
    pub room: String,
    pub level: i32,
    pub base: i32,
    pub at_door_ms: f64,
    pub closed_ms: f64,
    pub lit_ms: f64,
    pub attack_ms: f64,
    pub push_ms: f64,
    pub behind: bool,
    pub focus: f64,
    pace: f64,
    doors: Vec<String>,
    which: usize,
    door: String,
    dist: HashMap<String, i32>,
    retreat_to: String,
    push_to: String,
    alternate: bool,
    only_when_cams_down: bool,
    freezes_when_watched: bool,
    pacer: Pacer,
    /// Kathy: the light never holds her wait.
    pub unseen: bool,
    /// How long a shut door has to stay shut on her this time.
    pub need_ms: f64,
    need_range: Option<(f64, f64)>,
    need_tail: f64,
    fade_to: Vec<String>,
}

impl Stalker {
    /// A floor hunter from the cast file (`make`). `attack` is the door grace, `pace` the night's stretch on every wait,
    /// `push` how long the light must stay on her.
    pub fn make(def: &CastDef, base_level: i32, attack: f64, pace: f64, push: f64) -> Stalker {
        let door = def.doors.first().cloned().unwrap_or_default();
        Stalker {
            id: def.id.clone(),
            room: def.start.clone(),
            level: base_level,
            base: base_level,
            at_door_ms: 0.0,
            closed_ms: 0.0,
            lit_ms: 0.0,
            attack_ms: attack,
            push_ms: push,
            behind: false,
            focus: rules().focus,
            pace,
            doors: def.doors.clone(),
            which: 0,
            dist: distances_to(&door, building()),
            door,
            retreat_to: def.retreat_to.clone(),
            push_to: def.push_to.clone(),
            alternate: def.alternate,
            only_when_cams_down: def.only_when_cams_down,
            freezes_when_watched: def.freezes_when_watched,
            pacer: Pacer::new(),
            unseen: false,
            need_ms: rules().t_closed_ms,
            need_range: None,
            need_tail: 0.0,
            fade_to: Vec::new(),
        }
    }

    /// Night 7: Kathy, alone (`make_kathy`). `grace_ms` and `pace` are Night 6's scaled. Draws once (her first door).
    pub fn make_kathy(rng: &mut Mulberry32, base_level: i32, grace_ms: f64, pace: f64) -> Stalker {
        let b = building();
        let r = rules();
        let doors = b.kathy_doors.clone();
        let which = rng.pick(doors.len().max(1)).min(doors.len().saturating_sub(1));
        let door = doors.get(which).cloned().unwrap_or_default();
        let lo = r.kathy_hold_ms.first().copied().unwrap_or(2500.0);
        let hi = r.kathy_hold_ms.get(1).copied().unwrap_or(20000.0);
        Stalker {
            id: "kathy".to_string(),
            room: b.kathy_start.clone(),
            level: base_level,
            base: base_level,
            at_door_ms: 0.0,
            closed_ms: 0.0,
            lit_ms: 0.0,
            attack_ms: grace_ms,
            push_ms: f64::INFINITY,
            behind: false,
            focus: r.focus,
            pace,
            doors,
            which,
            dist: distances_to(&door, b),
            door,
            retreat_to: b.kathy_start.clone(),
            push_to: b.kathy_start.clone(),
            alternate: false,
            only_when_cams_down: false,
            freezes_when_watched: false,
            pacer: Pacer::new(),
            unseen: true,
            need_ms: lo,
            need_range: Some((lo, hi)),
            need_tail: r.kathy_hold_tail_ms,
            fade_to: b.kathy_fade_to.clone(),
        }
    }

    pub fn door(&self) -> &str {
        &self.door
    }

    /// The index (0 left, 1 right) of her current door.
    pub fn door_idx(&self) -> usize {
        door_index(&self.door).unwrap_or(0)
    }

    pub fn is_at_door(&self) -> bool {
        self.room == self.door
    }

    pub fn reset_pacer(&mut self) {
        self.pacer.reset();
    }

    fn set_door(&mut self, which: usize) {
        self.which = which;
        self.door = self.doors.get(which).cloned().unwrap_or_default();
        self.dist = distances_to(&self.door, building());
    }

    /// Kathy fades: she turns up in one of the fade_to rooms, on a door chosen afresh, with a fresh wait. Two draws.
    pub fn fade(&mut self, rng: &mut Mulberry32) {
        if !self.fade_to.is_empty() {
            self.room = self.fade_to[rng.pick(self.fade_to.len())].clone();
        }
        self.at_door_ms = 0.0;
        self.closed_ms = 0.0;
        self.lit_ms = 0.0;
        self.behind = false;
        self.reset_pacer();
        let which = if self.doors.is_empty() { 0 } else { rng.pick(self.doors.len()) };
        self.set_door(which);
    }

    /// Admin: put her in `to_room` now. A door node makes it her door (she waits there as if she had walked up to it).
    /// Shots tour: leave the door for the room her push sends her to.
    pub fn tour_pull_back(&mut self) {
        let to = self.push_to.clone();
        self.admin_send(&to);
    }

    pub fn admin_send(&mut self, to_room: &str) {
        self.room = to_room.to_string();
        self.at_door_ms = 0.0;
        self.closed_ms = 0.0;
        self.lit_ms = 0.0;
        self.behind = false;
        self.reset_pacer();
        if to_room.starts_with("door") {
            self.door = to_room.to_string();
            self.dist = distances_to(&self.door, building());
            if let Some(i) = self.doors.iter().position(|d| d == to_room) {
                self.which = i;
            }
        }
    }

    fn pacer_step(&mut self, dt_ms: f64, rng: &mut Mulberry32) -> bool {
        self.pacer.step(dt_ms, self.level, &mut || rng.next())
    }

    fn can_move(&self, ctx: &StepCtx) -> bool {
        if self.only_when_cams_down && ctx.cam {
            return false;
        }
        if self.freezes_when_watched && ctx.cam && ctx.cam_room == self.room {
            return false;
        }
        true
    }

    /// The room she walks to next. Draw order as in GDScript: the focus draw happens whenever a closer room exists (the
    /// `or` evaluates the draw first), then one pick draw when the pool is not empty. `door_counts` is how many OTHER
    /// hunters stand at each door; a full door is not an option.
    pub fn next_room(&self, rng: &mut Mulberry32, door_counts: [i32; 2]) -> String {
        let cap = rules().door_capacity;
        let empty: Vec<String> = Vec::new();
        let around: Vec<&String> = building()
            .adj
            .get(&self.room)
            .unwrap_or(&empty)
            .iter()
            .filter(|r| match door_index(r) {
                Some(i) => door_counts[i] < cap,
                None => true,
            })
            .collect();
        let here = self.dist.get(&self.room).copied().unwrap_or(99);
        let mut closer: Vec<&String> = Vec::new();
        let mut sideways: Vec<&String> = Vec::new();
        for r in around.iter().copied() {
            let d = self.dist.get(r).copied().unwrap_or(99);
            if d < here {
                closer.push(r);
            } else if r.as_str() != self.door.as_str() && !r.starts_with("door") && d >= here {
                sideways.push(r);
            }
        }
        let take_closer = if closer.is_empty() { false } else { rng.next() < self.focus || sideways.is_empty() };
        let pool: &Vec<&String> = if take_closer {
            &closer
        } else if !sideways.is_empty() {
            &sideways
        } else {
            &closer
        };
        if pool.is_empty() {
            return self.room.clone();
        }
        pool[rng.pick(pool.len())].clone()
    }

    /// Advance by `dt_ms`. `door_closed` says whether her door is shut. Returns the events that happened, in order.
    pub fn step(&mut self, dt_ms: f64, door_closed: bool, ctx: &StepCtx, rng: &mut Mulberry32, door_counts: [i32; 2]) -> Vec<SimEvent> {
        let mut events = Vec::new();
        self.behind = self.room == self.door && ctx.behind;
        if self.room != self.door && self.can_move(ctx) {
            let moved = if self.pace == 1.0 { self.pacer_step(dt_ms, rng) } else { self.pacer_step(dt_ms / self.pace, rng) };
            if moved {
                let from = self.room.clone();
                self.room = self.next_room(rng, door_counts);
                if self.room != from {
                    let (id, room) = (self.id.clone(), self.room.clone());
                    events.push(if self.room == self.door { SimEvent::Arrived { id, room, from } } else { SimEvent::Moved { id, room, from } });
                    if self.room == self.door {
                        self.at_door_ms = 0.0;
                        self.closed_ms = 0.0;
                        self.lit_ms = 0.0;
                        if let Some((lo, hi)) = self.need_range {
                            self.need_ms = roll_hold(rng.next(), lo, hi, self.need_tail);
                        }
                        return events;
                    }
                }
            }
        }
        if self.room == self.door {
            // A shut door sends her away almost at once (about half a second); an open one gives her the wait. A light on
            // her door pauses the wait. A shut door works on the one behind too, so it clears both at once.
            if door_closed {
                self.closed_ms += dt_ms;
                self.lit_ms = 0.0;
                if self.closed_ms >= self.need_ms {
                    let from = self.room.clone();
                    self.room = self.retreat_to.clone();
                    self.at_door_ms = 0.0;
                    self.closed_ms = 0.0;
                    self.lit_ms = 0.0;
                    self.behind = false;
                    self.reset_pacer();
                    if !self.fade_to.is_empty() {
                        self.fade(rng);
                    } else if self.alternate && !self.doors.is_empty() {
                        let next = (self.which + 1) % self.doors.len();
                        self.set_door(next);
                    }
                    events.push(SimEvent::Repelled { id: self.id.clone(), room: self.room.clone(), from });
                }
                return events;
            }
            self.closed_ms = 0.0;
            let lit = door_index(&self.door).map(|i| ctx.lit[i]).unwrap_or(false);
            if self.unseen || !lit {
                self.at_door_ms += dt_ms;
                self.lit_ms = 0.0;
            } else if self.behind {
                self.lit_ms = 0.0;
            } else {
                // The light on her: her wait holds, and if it stays on her long enough she backs off to push_to. Halfway
                // there she flinches (once per stare-down).
                let before = self.lit_ms;
                self.lit_ms += dt_ms;
                if before < self.push_ms / 2.0 && self.lit_ms >= self.push_ms / 2.0 && self.lit_ms < self.push_ms {
                    events.push(SimEvent::Flinch { id: self.id.clone(), room: self.room.clone() });
                }
                if self.lit_ms >= self.push_ms {
                    let from = self.room.clone();
                    self.room = self.push_to.clone();
                    self.at_door_ms = 0.0;
                    self.closed_ms = 0.0;
                    self.lit_ms = 0.0;
                    self.behind = false;
                    self.reset_pacer();
                    events.push(SimEvent::Pushed { id: self.id.clone(), room: self.room.clone(), from });
                    return events;
                }
            }
            if self.at_door_ms >= self.attack_ms {
                events.push(SimEvent::Attack { id: self.id.clone(), room: self.room.clone() });
            }
        }
        events
    }

    /// Is she giving way? Lit at her door for at least half of the stare-down.
    pub fn flinching(&self) -> bool {
        self.lit_ms > 0.0 && self.lit_ms >= self.push_ms / 2.0
    }

    /// The pose she shows at her door, from how long she has waited (the 2D game's doorPose stages scale with the night's
    /// door wait).
    pub fn door_pose(&self) -> DoorPose {
        let waited = self.at_door_ms * 5200.0 / self.attack_ms.max(1.0);
        if waited > 3600.0 {
            DoorPose::Lean
        } else if waited > 1900.0 {
            DoorPose::Stand
        } else {
            DoorPose::Peek
        }
    }

    /// The wait the pacer has drawn for her next move (None before the first draw). For tests and the admin rows.
    pub fn next_wait_ms(&self) -> Option<f64> {
        if self.pacer.next < 0.0 {
            None
        } else {
            Some(self.pacer.next)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::cast;

    fn def(id: &str) -> &'static CastDef {
        cast().iter().find(|c| c.id == id).expect("cast entry")
    }

    fn ctx() -> StepCtx<'static> {
        StepCtx { cam: false, cam_room: "", lit: [false, false], behind: false }
    }

    #[test]
    fn dovelle_starts_on_the_stage_and_aims_at_the_left_door() {
        let s = Stalker::make(def("dovelle"), 5, 6000.0, 1.0, 3000.0);
        assert_eq!((s.room.as_str(), s.door()), ("stage", "doorL"));
        assert_eq!(s.door_idx(), 0);
        assert!(!s.is_at_door());
        assert_eq!(s.dist.get("stage"), Some(&3));
    }

    #[test]
    fn next_room_never_leaves_the_graph_and_never_picks_a_full_door() {
        let mut rng = Mulberry32::new(7);
        let mut s = Stalker::make(def("dovelle"), 10, 6000.0, 1.0, 3000.0);
        s.room = "westhall".to_string();
        for _ in 0..200 {
            let n = s.next_room(&mut rng, [2, 0]);
            assert_ne!(n, "doorL", "the door holds two already");
            assert!(building().adj.get("westhall").map_or(false, |v| v.contains(&n)) || n == "westhall");
        }
        let mut hit_door = false;
        for _ in 0..200 {
            if s.next_room(&mut rng, [0, 0]) == "doorL" {
                hit_door = true;
            }
        }
        assert!(hit_door, "an open door is chosen sometimes");
    }

    #[test]
    fn the_focus_draw_happens_whenever_a_closer_room_exists() {
        // westhall has a closer room (doorL) so one focus draw and one pick draw are consumed
        let s = {
            let mut s = Stalker::make(def("dovelle"), 10, 6000.0, 1.0, 3000.0);
            s.room = "westhall".to_string();
            s
        };
        let mut a = Mulberry32::new(99);
        let mut b = Mulberry32::new(99);
        let _ = s.next_room(&mut a, [0, 0]);
        b.next();
        b.next();
        assert_eq!(a.state(), b.state());
        // a room with nothing around (blocked) draws nothing
        let mut s2 = Stalker::make(def("dovelle"), 10, 6000.0, 1.0, 3000.0);
        s2.room = "nowhere".to_string();
        let mut c = Mulberry32::new(99);
        assert_eq!(s2.next_room(&mut c, [0, 0]), "nowhere");
        assert_eq!(c.state(), 99);
    }

    #[test]
    fn a_hunter_walks_to_her_door_and_arrives() {
        let mut rng = Mulberry32::new(12345);
        let mut s = Stalker::make(def("dovelle"), 20, 6000.0, 1.0, 3000.0);
        let mut arrived = false;
        let mut moves = 0;
        for _ in 0..4000 {
            for e in s.step(100.0, false, &ctx(), &mut rng, [0, 0]) {
                match e {
                    SimEvent::Moved { .. } => moves += 1,
                    SimEvent::Arrived { .. } => arrived = true,
                    _ => {}
                }
            }
            if arrived {
                break;
            }
        }
        assert!(arrived, "level 20 reaches her door within 400 s");
        assert!(moves >= 1 || s.room == "doorL");
        assert!(s.is_at_door());
        assert_eq!(s.at_door_ms, 0.0);
    }

    #[test]
    fn a_shut_door_sends_her_away_after_450_ms() {
        let mut rng = Mulberry32::new(1);
        let mut s = Stalker::make(def("dovelle"), 5, 6000.0, 1.0, 3000.0);
        s.admin_send("doorL");
        let mut t = 0.0;
        let mut repelled_at = None;
        while t < 2000.0 && repelled_at.is_none() {
            t += 50.0;
            for e in s.step(50.0, true, &ctx(), &mut rng, [0, 0]) {
                if let SimEvent::Repelled { room, from, .. } = e {
                    assert_eq!((room.as_str(), from.as_str()), ("dining", "doorL"));
                    repelled_at = Some(t);
                }
            }
        }
        let at = repelled_at.expect("repelled");
        assert!((450.0..=500.0).contains(&at), "{at}");
        assert_eq!(s.room, "dining");
        assert!(!s.is_at_door());
    }

    #[test]
    fn an_open_unlit_door_ends_in_an_attack_after_the_grace() {
        let mut rng = Mulberry32::new(1);
        let mut s = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        s.admin_send("doorL");
        let mut t = 0.0;
        let mut attack = None;
        while t < 8000.0 && attack.is_none() {
            t += 100.0;
            for e in s.step(100.0, false, &ctx(), &mut rng, [0, 0]) {
                if matches!(e, SimEvent::Attack { .. }) {
                    attack = Some(t);
                }
            }
        }
        assert_eq!(attack, Some(5200.0));
    }

    #[test]
    fn the_light_holds_her_wait_flinches_halfway_and_pushes_her_back() {
        let mut rng = Mulberry32::new(1);
        let mut s = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        s.admin_send("doorL");
        let lit = StepCtx { lit: [true, false], ..ctx() };
        let (mut flinch, mut pushed) = (None, None);
        let mut t = 0.0;
        while t < 6000.0 && pushed.is_none() {
            t += 100.0;
            for e in s.step(100.0, false, &lit, &mut rng, [0, 0]) {
                match e {
                    SimEvent::Flinch { .. } => flinch = Some(t),
                    SimEvent::Pushed { room, .. } => {
                        assert_eq!(room, "stage");
                        pushed = Some(t);
                    }
                    SimEvent::Attack { .. } => panic!("the light holds the wait"),
                    _ => {}
                }
            }
        }
        assert_eq!(flinch, Some(1500.0));
        assert_eq!(pushed, Some(3000.0));
        assert_eq!(s.room, "stage");
    }

    #[test]
    fn the_one_behind_in_a_line_is_not_pushed_by_the_light() {
        let mut rng = Mulberry32::new(1);
        let mut s = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        s.admin_send("doorL");
        let c = StepCtx { lit: [true, false], behind: true, ..ctx() };
        for _ in 0..40 {
            s.step(100.0, false, &c, &mut rng, [0, 0]);
        }
        assert!(s.behind);
        assert_eq!(s.lit_ms, 0.0);
        assert_eq!(s.room, "doorL");
    }

    #[test]
    fn ravelle_alternates_doors_after_a_shut_door() {
        let mut rng = Mulberry32::new(3);
        let mut s = Stalker::make(def("ravelle"), 5, 5200.0, 1.0, 3000.0);
        assert_eq!(s.door(), "doorR");
        s.admin_send("doorR");
        for _ in 0..10 {
            s.step(100.0, true, &ctx(), &mut rng, [0, 0]);
        }
        assert_eq!(s.room, "dining");
        assert_eq!(s.door(), "doorL");
    }

    #[test]
    fn aki_waits_while_the_cameras_are_up_and_ilyra_freezes_on_her_own_feed() {
        let mut rng = Mulberry32::new(5);
        let mut aki = Stalker::make(def("aki"), 20, 5200.0, 1.0, 3000.0);
        let up = StepCtx { cam: true, cam_room: "stage", ..ctx() };
        for _ in 0..600 {
            aki.step(100.0, false, &up, &mut rng, [0, 0]);
        }
        assert_eq!(aki.room, "closet", "aki never moves with the monitor up");
        let mut ilyra = Stalker::make(def("ilyra"), 20, 5200.0, 1.0, 3000.0);
        let watch = StepCtx { cam: true, cam_room: "backstage", ..ctx() };
        for _ in 0..600 {
            ilyra.step(100.0, false, &watch, &mut rng, [0, 0]);
        }
        assert_eq!(ilyra.room, "backstage", "ilyra freezes while her room is watched");
        let other = StepCtx { cam: true, cam_room: "stage", ..ctx() };
        let mut moved = false;
        for _ in 0..600 {
            if !ilyra.step(100.0, false, &other, &mut rng, [0, 0]).is_empty() {
                moved = true;
            }
        }
        assert!(moved, "ilyra moves while another feed is watched");
    }

    #[test]
    fn kathy_rolls_a_hold_on_arrival_and_fades_instead_of_retreating() {
        let mut rng = Mulberry32::new(11);
        let mut k = Stalker::make_kathy(&mut rng, 20, 2610.0, 0.8);
        assert!(k.unseen && k.push_ms.is_infinite());
        assert_eq!(k.room, "stage");
        let door = k.door().to_string();
        k.admin_send(&door);
        k.need_ms = 500.0;
        let mut ev = Vec::new();
        for _ in 0..20 {
            ev.extend(k.step(100.0, true, &ctx(), &mut rng, [0, 0]));
        }
        let repelled = ev.iter().find(|e| matches!(e, SimEvent::Repelled { .. })).expect("she fades");
        if let SimEvent::Repelled { room, .. } = repelled {
            assert!(room == "stage" || room == "kitchen", "{room}");
        }
        assert!(!k.is_at_door());
    }

    #[test]
    fn the_light_never_holds_kathy() {
        let mut rng = Mulberry32::new(11);
        let mut k = Stalker::make_kathy(&mut rng, 20, 2610.0, 0.8);
        let door = k.door().to_string();
        k.admin_send(&door);
        let lit = if door == "doorL" { StepCtx { lit: [true, false], ..ctx() } } else { StepCtx { lit: [false, true], ..ctx() } };
        let mut attacked = false;
        for _ in 0..40 {
            if k.step(100.0, false, &lit, &mut rng, [0, 0]).iter().any(|e| matches!(e, SimEvent::Attack { .. })) {
                attacked = true;
            }
        }
        assert!(attacked);
    }

    #[test]
    fn the_door_pose_follows_the_wait() {
        let mut s = Stalker::make(def("dovelle"), 5, 5200.0, 1.0, 3000.0);
        assert_eq!(s.door_pose(), DoorPose::Peek);
        s.at_door_ms = 2000.0;
        assert_eq!(s.door_pose(), DoorPose::Stand);
        s.at_door_ms = 3700.0;
        assert_eq!(s.door_pose(), DoorPose::Lean);
        // the stages scale with the night's wait
        s.attack_ms = 2600.0;
        s.at_door_ms = 1900.0;
        assert_eq!(s.door_pose(), DoorPose::Lean);
    }

    #[test]
    fn a_pace_stretches_the_wait_between_moves() {
        let mut fast = Stalker::make(def("dovelle"), 10, 5200.0, 1.0, 3000.0);
        let mut slow = Stalker::make(def("dovelle"), 10, 5200.0, 2.0, 3000.0);
        let (mut r1, mut r2) = (Mulberry32::new(4), Mulberry32::new(4));
        let (mut t1, mut t2) = (0, 0);
        for i in 1..=2000 {
            if t1 == 0 && !fast.step(100.0, false, &ctx(), &mut r1, [0, 0]).is_empty() {
                t1 = i;
            }
            if t2 == 0 && !slow.step(100.0, false, &ctx(), &mut r2, [0, 0]).is_empty() {
                t2 = i;
            }
        }
        assert!(t1 > 0 && t2 > t1, "{t1} {t2}");
    }
}
