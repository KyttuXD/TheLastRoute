//! Night Watch Classic: the night's rules (port of `night_watch_classic.gd`, itself a port of `createClassicGame` in the 2D
//! game plus the power meter). No scene, no input, no drawing: the shift scene feeds `step` the state of the doors, lights and
//! monitor and shows what happens. The same seed gives the same night.
//!
//! `ClassicSim::step(dt_ms, &SimInput) -> Vec<SimEvent>`; `over` is `Win` or `Caught` (and `by` the hunter) once the night is
//! decided. Hunters keep the Godot move rules; where they WALK is not decided here (see `placement.rs`).
#![allow(dead_code)]

use super::stalker::{door_index, door_name, Stalker, StepCtx};
use crate::core::{
    active_hunters, blackout_ms, cast, effective_level, floor_hunters, hour_index, is_kathy_night, level_for, nyxie_level, rules,
    Mulberry32, NIGHT_MS,
};
use std::collections::BTreeMap;

/// What happened in a step.
#[derive(Clone, Debug, PartialEq)]
pub enum SimEvent {
    Moved { id: String, room: String, from: String },
    Arrived { id: String, room: String, from: String },
    Repelled { id: String, room: String, from: String },
    Pushed { id: String, room: String, from: String },
    Flinch { id: String, room: String },
    Attack { id: String, room: String },
    /// The power is gone.
    Out,
    /// The night is lost: `cause` is "darkness" for Nyxie.
    Caught { id: String, cause: Option<String> },
    Win,
    /// Kathy hums at her door (a shut door muffles it).
    Hum { id: String, door: String, muffled: bool },
}

impl SimEvent {
    /// The GDScript event type name.
    pub fn kind(&self) -> &'static str {
        match self {
            SimEvent::Moved { .. } => "moved",
            SimEvent::Arrived { .. } => "arrived",
            SimEvent::Repelled { .. } => "repelled",
            SimEvent::Pushed { .. } => "pushed",
            SimEvent::Flinch { .. } => "flinch",
            SimEvent::Attack { .. } => "attack",
            SimEvent::Out => "out",
            SimEvent::Caught { .. } => "caught",
            SimEvent::Win => "win",
            SimEvent::Hum { .. } => "hum",
        }
    }

    pub fn id(&self) -> Option<&str> {
        match self {
            SimEvent::Moved { id, .. }
            | SimEvent::Arrived { id, .. }
            | SimEvent::Repelled { id, .. }
            | SimEvent::Pushed { id, .. }
            | SimEvent::Flinch { id, .. }
            | SimEvent::Attack { id, .. }
            | SimEvent::Caught { id, .. }
            | SimEvent::Hum { id, .. } => Some(id),
            SimEvent::Out | SimEvent::Win => None,
        }
    }

    /// The room the event happened in ("" when it has none).
    pub fn room(&self) -> &str {
        match self {
            SimEvent::Moved { room, .. }
            | SimEvent::Arrived { room, .. }
            | SimEvent::Repelled { room, .. }
            | SimEvent::Pushed { room, .. }
            | SimEvent::Flinch { room, .. }
            | SimEvent::Attack { room, .. } => room,
            _ => "",
        }
    }

    /// The room she came from ("" when the event has none).
    pub fn from_room(&self) -> &str {
        match self {
            SimEvent::Moved { from, .. } | SimEvent::Arrived { from, .. } | SimEvent::Repelled { from, .. } | SimEvent::Pushed { from, .. } => from,
            _ => "",
        }
    }

    /// A short stable name for the headless event list: `kind:id:room`.
    pub fn name(&self) -> String {
        format!("{}:{}:{}", self.kind(), self.id().unwrap_or("-"), self.room())
    }
}

/// How the night ended (the GDScript `over` string).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Over {
    Running,
    Win,
    Caught,
}

/// The doors, lights and monitor as the player has them (the GDScript input dictionary).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimInput {
    /// Left, right: shut.
    pub door_shut: [bool; 2],
    /// Left, right: the light is held.
    pub light_held: [bool; 2],
    /// The monitor is up.
    pub cam: bool,
    pub cam_room: String,
}

impl SimInput {
    /// Devices in use (door shut, light held, monitor up) for the power drain and the HUD.
    pub fn usage(&self) -> i32 {
        self.door_shut.iter().filter(|b| **b).count() as i32 + self.light_held.iter().filter(|b| **b).count() as i32 + self.cam as i32
    }
}

/// The result of trying to meet Kathy's eyes (Night 7).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MeetResult {
    Met,
    Miss,
    Locked,
    None,
}

/// A working-together rule: while `a` waits at a door, `b` loses patience (and the other way round).
#[derive(Clone, Debug)]
struct Combo {
    a: String,
    b: String,
    min_level: i32,
    patience_ms: f64,
}

pub struct ClassicSim {
    pub night: i32,
    pub seed: u32,
    pub elapsed: f64,
    pub over: Over,
    pub by: String,
    pub power: f64,
    pub out: bool,
    pub out_ms: f64,
    /// The floor hunters in cast order (Kathy alone on her night).
    pub hunters: Vec<Stalker>,
    pub nyxie_level: i32,
    /// The doors and lights as the rules saw them on the last step (doors open and lights dark once the power is out).
    pub door_shut: [bool; 2],
    pub light_on: [bool; 2],
    pub kathy_night: bool,
    /// Admin hooks: god mode (attacks are shrugged off), infinite power, hunter speed multiplier.
    pub god: bool,
    pub infinite_power: bool,
    pub hunter_speed: f64,
    rng: Mulberry32,
    grace_ms: f64,
    pace: f64,
    push_ms: f64,
    lines: [Vec<String>; 2],
    combos: Vec<Combo>,
    hum_t: f64,
    meet_lock_ms: f64,
}

impl ClassicSim {
    /// `levels` optionally overrides the night's AI levels { id: level } (Custom Night, admin tools, tests).
    pub fn new(night: i32, seed: u32, levels: &BTreeMap<String, i32>, custom: bool) -> ClassicSim {
        let r = rules();
        let night = night.clamp(1, r.nights.max(1));
        let grace = r.door_grace(night);
        let pace = r.pace(night);
        let push = r.light_push(night);
        let kathy_night = is_kathy_night(night, custom);
        let mut rng = Mulberry32::new(seed);
        let mut hunters: Vec<Stalker> = Vec::new();
        let mut nyx = 0;
        if kathy_night {
            // Night 7: Kathy alone, on Night 6's door pacing scaled (she walks faster, waits less at an open door; the
            // light never moves her)
            let g = (grace * r.kathy_grace_scale).round();
            let p = ((pace * r.kathy_pace_scale) / 0.001 + 0.5).floor() * 0.001;
            let level = levels.get("kathy").copied().unwrap_or(r.kathy_level);
            hunters.push(Stalker::make_kathy(&mut rng, level, g, p));
        } else {
            for def in floor_hunters() {
                let base = levels.get(&def.id).copied().unwrap_or_else(|| level_for(&def.id, night));
                hunters.push(Stalker::make(def, base, grace, pace, push));
            }
            nyx = levels.get("nyxie").copied().unwrap_or_else(|| nyxie_level(night));
        }
        let combos = cast()
            .iter()
            .filter_map(|c| c.combo.as_ref().map(|(with, min, patience)| Combo { a: c.id.clone(), b: with.clone(), min_level: *min, patience_ms: *patience }))
            .collect();
        ClassicSim {
            night,
            seed,
            elapsed: 0.0,
            over: Over::Running,
            by: String::new(),
            power: 100.0,
            out: false,
            out_ms: 0.0,
            hunters,
            nyxie_level: nyx,
            door_shut: [false, false],
            light_on: [false, false],
            kathy_night,
            god: false,
            infinite_power: false,
            hunter_speed: 1.0,
            rng,
            grace_ms: grace,
            pace,
            push_ms: push,
            lines: [Vec::new(), Vec::new()],
            combos,
            hum_t: 0.0,
            meet_lock_ms: 0.0,
        }
    }

    pub fn hour(&self) -> i32 {
        hour_index(self.elapsed)
    }

    /// The state of the random stream (for digests and determinism tests).
    pub fn rng_state(&self) -> u32 {
        self.rng.state()
    }

    pub fn hunter(&self, id: &str) -> Option<&Stalker> {
        self.hunters.iter().find(|h| h.id == id)
    }

    pub fn hunter_mut(&mut self, id: &str) -> Option<&mut Stalker> {
        self.hunters.iter_mut().find(|h| h.id == id)
    }

    /// How many OTHER hunters stand at each door.
    fn door_counts_except(&self, except: usize) -> [i32; 2] {
        let mut n = [0, 0];
        for (i, h) in self.hunters.iter().enumerate() {
            if i != except {
                if let Some(d) = door_index(&h.room) {
                    n[d] += 1;
                }
            }
        }
        n
    }

    /// Who is at a door, in the order they arrived (front first), as indices into `hunters`.
    pub fn at_door(&mut self, door: usize) -> Vec<usize> {
        self.refresh_lines();
        self.lines[door.min(1)].iter().filter_map(|id| self.hunters.iter().position(|h| &h.id == id)).collect()
    }

    fn refresh_lines(&mut self) {
        for d in 0..2 {
            let name = door_name(d);
            let hunters = &self.hunters;
            self.lines[d].retain(|id| hunters.iter().any(|h| &h.id == id && h.room == name));
        }
        for h in &self.hunters {
            if let Some(d) = door_index(&h.room) {
                if !self.lines[d].contains(&h.id) {
                    self.lines[d].push(h.id.clone());
                }
            }
        }
    }

    pub fn step(&mut self, dt_ms: f64, input: &SimInput) -> Vec<SimEvent> {
        let mut events = Vec::new();
        if self.over != Over::Running {
            return events;
        }
        self.elapsed += dt_ms;
        let h = self.hour();
        let r = rules();
        // Power: base drain plus a little for every device in use. At zero the doors open, the lights fail and Nyxie comes.
        if !self.out && !self.infinite_power {
            self.power = (self.power - (r.power_base() + r.power_per_device() * input.usage() as f64) * dt_ms / 1000.0).max(0.0);
            if self.power <= 0.0 {
                self.out = true;
                self.out_ms = 0.0;
                events.push(SimEvent::Out);
            }
        } else {
            self.out_ms += dt_ms;
        }
        let shut = [input.door_shut[0] && !self.out, input.door_shut[1] && !self.out];
        let lit = [input.light_held[0] && !self.out, input.light_held[1] && !self.out];
        self.door_shut = shut;
        self.light_on = lit;
        let cam = input.cam && !self.out;
        for i in 0..self.hunters.len() {
            let level = effective_level(self.hunters[i].base, h);
            self.hunters[i].level = level;
            // Working together: while a partner already waits at a door, this one's patience runs out sooner.
            let mut patience = 0.0f64;
            let id = self.hunters[i].id.clone();
            for c in &self.combos {
                let other_id = if c.a == id {
                    &c.b
                } else if c.b == id {
                    &c.a
                } else {
                    continue;
                };
                if let Some(other) = self.hunters.iter().find(|o| &o.id == other_id) {
                    if other.room.starts_with("door") && other.level >= c.min_level && level >= c.min_level {
                        patience = patience.max(c.patience_ms);
                    }
                }
            }
            if !self.kathy_night {
                self.hunters[i].attack_ms = self.grace_ms - patience;
            }
            self.refresh_lines();
            let room = self.hunters[i].room.clone();
            let is_behind = match door_index(&room) {
                Some(d) => self.lines[d].iter().position(|x| x == &id).map_or(false, |p| p > 0),
                None => false,
            };
            let counts = self.door_counts_except(i);
            let ctx = StepCtx { cam, cam_room: &input.cam_room, lit, behind: is_behind };
            let door_closed = door_index(self.hunters[i].door()).map_or(false, |d| shut[d]);
            let stepped = self.hunters[i].step(dt_ms * self.hunter_speed, door_closed, &ctx, &mut self.rng, counts);
            for e in stepped {
                let attack = matches!(e, SimEvent::Attack { .. });
                events.push(e);
                if attack {
                    if self.god {
                        self.hunters[i].at_door_ms = 0.0; // god mode: she is sent back to wait again instead of catching you
                        continue;
                    }
                    self.over = Over::Caught;
                    self.by = id.clone();
                    events.push(SimEvent::Caught { id: id.clone(), cause: None });
                }
            }
            if self.over != Over::Running {
                return events;
            }
        }
        // Nyxie never walks the building: in the dark she reaches the office after a while.
        if self.out && self.nyxie_level > 0 && !self.god && self.out_ms >= blackout_ms(self.nyxie_level) {
            self.over = Over::Caught;
            self.by = "nyxie".to_string();
            events.push(SimEvent::Caught { id: "nyxie".to_string(), cause: Some("darkness".to_string()) });
            return events;
        }
        if self.kathy_night {
            self.meet_lock_ms = (self.meet_lock_ms - dt_ms).max(0.0);
            if let Some(k) = self.hunters.iter().find(|k| k.id == "kathy") {
                if k.is_at_door() {
                    self.hum_t += dt_ms;
                    if self.hum_t >= r.kathy_hum_ms {
                        self.hum_t = 0.0;
                        let d = k.door_idx();
                        events.push(SimEvent::Hum { id: "kathy".to_string(), door: k.door().to_string(), muffled: shut[d] });
                    }
                } else {
                    self.hum_t = 0.0;
                }
            }
        }
        self.refresh_lines();
        if self.elapsed >= NIGHT_MS {
            self.over = Over::Win;
            events.push(SimEvent::Win);
        }
        events
    }

    /// The pose each hunter at a door shows, front first.
    pub fn door_poses(&mut self, door: usize) -> Vec<super::stalker::DoorPose> {
        let idx = self.at_door(door);
        idx.into_iter().map(|i| self.hunters[i].door_pose()).collect()
    }

    /// Night 7's tell: is this door's light dipping right now? Only while she stands at that door and its light is on
    /// (open or shut).
    pub fn light_dip(&self, door: usize) -> bool {
        if !self.kathy_night || !self.light_on[door.min(1)] {
            return false;
        }
        match self.hunters.iter().find(|k| k.id == "kathy") {
            Some(k) => k.room == door_name(door) && crate::core::kathy_dip_on(self.elapsed),
            None => false,
        }
    }

    /// Meet her eyes (Night 7): works while she is on the feed you are watching and that feed is one of the two halls; she
    /// fades back to the stage or the kitchen on a fresh door. Anything else is a miss, and the key is dead for
    /// `kathy_miss_lock_ms` after it. The fade is silent, so the repelled event is returned for tests, not for sound.
    pub fn meet(&mut self, cam: bool, cam_room: &str, lost: bool) -> (MeetResult, Vec<SimEvent>) {
        if !self.kathy_night || self.over != Over::Running {
            return (MeetResult::None, Vec::new());
        }
        if self.meet_lock_ms > 0.0 {
            return (MeetResult::Locked, Vec::new());
        }
        let meet_rooms = &crate::core::building().kathy_meet_rooms;
        let k = match self.hunters.iter().position(|k| k.id == "kathy") {
            Some(i) => i,
            None => return (MeetResult::None, Vec::new()),
        };
        if cam && !lost && !self.out && cam_room == self.hunters[k].room && meet_rooms.iter().any(|m| m == &self.hunters[k].room) {
            let from = self.hunters[k].room.clone();
            self.hunters[k].fade(&mut self.rng);
            let room = self.hunters[k].room.clone();
            return (MeetResult::Met, vec![SimEvent::Repelled { id: "kathy".to_string(), room, from }]);
        }
        self.meet_lock_ms = rules().kathy_miss_lock_ms;
        (MeetResult::Miss, Vec::new())
    }

    // ---- admin ------------------------------------------------------------------------------------------------------

    /// Put a hunter in `room` now (a door node makes it her door).
    pub fn admin_send(&mut self, id: &str, room: &str) {
        if let Some(h) = self.hunter_mut(id) {
            h.admin_send(room);
        }
    }

    /// The shots tour only: god mode shrugs off the attack and leaves her at her door, so send every hunter standing at a door
    /// back to where her push sends her (her camera rooms are then walked again).
    pub fn tour_pull_back(&mut self) {
        for h in &mut self.hunters {
            if super::stalker::door_index(&h.room).is_some() {
                h.tour_pull_back();
            }
        }
    }

    pub fn admin_all_to_doors(&mut self) {
        for h in &mut self.hunters {
            let d = h.door().to_string();
            h.admin_send(&d);
        }
    }

    pub fn admin_set_level(&mut self, id: &str, level: i32) {
        let hour = self.hour();
        if let Some(h) = self.hunter_mut(id) {
            h.base = level;
            h.level = effective_level(level, hour);
        }
    }

    /// Set the power; returns `Out` when it ran to zero.
    pub fn admin_set_power(&mut self, percent: f64) -> Vec<SimEvent> {
        self.power = percent.clamp(0.0, 100.0);
        self.out = self.power <= 0.0;
        self.out_ms = 0.0;
        if self.out {
            vec![SimEvent::Out]
        } else {
            Vec::new()
        }
    }

    /// Jump the clock to the start of an hour (0..5).
    pub fn admin_set_hour(&mut self, hour: i32) {
        self.elapsed = (hour as f64 * crate::core::HOUR_MS).clamp(0.0, NIGHT_MS - 1000.0);
    }

    /// The hunters the night starts with (for tests): the active ids of the night.
    pub fn expected_ids(night: i32, custom: bool) -> Vec<String> {
        if is_kathy_night(night, custom) {
            vec!["kathy".to_string()]
        } else {
            floor_hunters().iter().map(|c| c.id.clone()).collect()
        }
    }

    /// Hunters above level zero on this night (the ones that show on the floor).
    pub fn awake_ids(&self) -> Vec<String> {
        self.hunters.iter().filter(|h| h.base > 0).map(|h| h.id.clone()).collect()
    }

    /// `active_hunters` of the night, from the level table (no overrides).
    pub fn table_ids(night: i32) -> Vec<String> {
        active_hunters(night)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Fnv;

    fn idle() -> SimInput {
        SimInput::default()
    }

    fn levels(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn quiet(night: i32) -> ClassicSim {
        // every floor hunter at level 0 and Nyxie off: nothing moves
        ClassicSim::new(night, 1, &levels(&[("dovelle", 0), ("ravelle", 0), ("aki", 0), ("ilyra", 0), ("nyxie", 0)]), true)
    }

    #[test]
    fn night_seven_is_kathy_alone_and_a_custom_night_seven_is_not() {
        let s = ClassicSim::new(7, 5, &BTreeMap::new(), false);
        assert!(s.kathy_night);
        assert_eq!(s.hunters.len(), 1);
        assert_eq!(s.hunters[0].id, "kathy");
        assert_eq!(s.hunters[0].base, 20);
        assert_eq!(s.nyxie_level, 0);
        // her pacing is Night 6's scaled: grace 5800 * 0.45 rounded, pace 1.03 * 0.65 snapped to 0.001
        assert_eq!(s.hunters[0].attack_ms, 2610.0);
        let c = ClassicSim::new(7, 5, &levels(&[("dovelle", 3), ("ravelle", 3), ("aki", 3), ("ilyra", 3), ("nyxie", 3)]), true);
        assert!(!c.kathy_night);
        assert_eq!(c.hunters.len(), 4);
        assert_eq!(c.nyxie_level, 3);
        assert!(c.hunters.iter().all(|h| h.id != "kathy"));
    }

    #[test]
    fn the_night_table_sets_the_levels_and_the_cast_order() {
        let s = ClassicSim::new(4, 1, &BTreeMap::new(), false);
        let ids: Vec<&str> = s.hunters.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["dovelle", "ravelle", "aki", "ilyra"]);
        let lv: Vec<i32> = s.hunters.iter().map(|h| h.base).collect();
        assert_eq!(lv, vec![9, 7, 6, 5]);
        assert_eq!(s.nyxie_level, 9);
        assert_eq!(s.hunters[0].attack_ms, 6900.0);
        assert_eq!(s.hunters[0].push_ms, 3500.0);
        let one = ClassicSim::new(1, 1, &BTreeMap::new(), false);
        assert_eq!(one.awake_ids(), vec!["dovelle".to_string()]);
    }

    #[test]
    fn a_custom_night_uses_the_sliders() {
        let c = ClassicSim::new(1, 1, &levels(&[("dovelle", 12), ("ravelle", 0), ("aki", 7), ("ilyra", 20), ("nyxie", 1)]), true);
        assert_eq!(c.hunters.iter().map(|h| h.base).collect::<Vec<_>>(), vec![12, 0, 7, 20]);
        assert_eq!(c.nyxie_level, 1);
    }

    #[test]
    fn the_night_is_won_at_270_seconds_with_infinite_power() {
        let mut s = quiet(1);
        s.infinite_power = true;
        let mut win_at = None;
        let mut t = 0.0;
        while win_at.is_none() && t < 300_000.0 {
            t += 100.0;
            if s.step(100.0, &idle()).iter().any(|e| matches!(e, SimEvent::Win)) {
                win_at = Some(t);
            }
        }
        assert_eq!(win_at, Some(270_000.0));
        assert_eq!(s.over, Over::Win);
        assert_eq!(s.hour(), 5);
        assert!(s.step(100.0, &idle()).is_empty(), "a decided night does nothing");
    }

    #[test]
    fn power_drains_with_use_and_runs_out() {
        let mut a = quiet(1);
        let mut b = quiet(1);
        let busy = SimInput { door_shut: [true, true], light_held: [true, true], cam: true, cam_room: "stage".into() };
        assert_eq!(busy.usage(), 5);
        for _ in 0..100 {
            a.step(100.0, &idle());
            b.step(100.0, &busy);
        }
        assert!(a.power < 100.0 && b.power < a.power);
        // 10 s idle costs power_base percent per second
        assert!((100.0 - a.power - rules().power_base() * 10.0).abs() < 1e-6);
        let mut c = quiet(1);
        c.power = 0.01;
        let ev = c.step(100.0, &busy);
        assert!(ev.contains(&SimEvent::Out));
        assert!(c.out);
    }

    #[test]
    fn power_out_opens_the_doors_and_nyxie_catches_at_the_blackout_time() {
        // Nyxie alone at level 5: blackout = 11500 - 5 * 400 = 9500 ms
        let mut s = ClassicSim::new(1, 1, &levels(&[("dovelle", 0), ("ravelle", 0), ("aki", 0), ("ilyra", 0), ("nyxie", 5)]), true);
        s.power = 0.001;
        let shut = SimInput { door_shut: [true, true], light_held: [true, false], cam: false, cam_room: String::new() };
        let ev = s.step(100.0, &shut);
        assert!(ev.contains(&SimEvent::Out));
        assert_eq!(s.door_shut, [false, false], "the doors open in the dark");
        assert_eq!(s.light_on, [false, false], "and the lights fail");
        let mut caught_at = None;
        let mut t = 100.0;
        while caught_at.is_none() && t < 20_000.0 {
            t += 100.0;
            for e in s.step(100.0, &shut) {
                if let SimEvent::Caught { id, cause } = e {
                    assert_eq!(id, "nyxie");
                    assert_eq!(cause.as_deref(), Some("darkness"));
                    caught_at = Some(t);
                }
            }
        }
        let at = caught_at.expect("caught in the dark");
        assert!((at - 100.0_f64 - 9500.0).abs() <= 100.0, "{at}");
        assert_eq!(s.over, Over::Caught);
        assert_eq!(s.by, "nyxie");
    }

    #[test]
    fn god_mode_keeps_nyxie_away() {
        let mut s = ClassicSim::new(1, 1, &levels(&[("dovelle", 0), ("ravelle", 0), ("aki", 0), ("ilyra", 0), ("nyxie", 20)]), true);
        s.god = true;
        s.power = 0.001;
        for _ in 0..200 {
            s.step(100.0, &idle());
        }
        assert!(s.out);
        assert_eq!(s.over, Over::Running);
    }

    #[test]
    fn a_shut_door_repels_dovelle_after_450_ms() {
        let mut s = ClassicSim::new(2, 1, &BTreeMap::new(), false);
        s.admin_send("dovelle", "doorL");
        let shut = SimInput { door_shut: [true, false], ..SimInput::default() };
        let mut at = None;
        let mut t = 0.0;
        while at.is_none() && t < 2000.0 {
            t += 50.0;
            for e in s.step(50.0, &shut) {
                if let SimEvent::Repelled { id, room, from } = e {
                    assert_eq!((id.as_str(), room.as_str(), from.as_str()), ("dovelle", "dining", "doorL"));
                    at = Some(t);
                }
            }
        }
        let at = at.expect("repelled");
        assert!((450.0..=500.0).contains(&at), "{at}");
        assert_eq!(s.over, Over::Running);
    }

    #[test]
    fn an_unanswered_door_catches_you_after_the_grace() {
        let mut s = ClassicSim::new(1, 1, &BTreeMap::new(), false);
        s.admin_send("dovelle", "doorL");
        let mut caught = None;
        let mut t = 0.0;
        while caught.is_none() && t < 12_000.0 {
            t += 100.0;
            for e in s.step(100.0, &idle()) {
                if let SimEvent::Caught { id, cause } = e {
                    assert_eq!(id, "dovelle");
                    assert!(cause.is_none());
                    caught = Some(t);
                }
            }
        }
        // night 1 grace is 9000 ms
        assert_eq!(caught, Some(9000.0));
        assert_eq!(s.over, Over::Caught);
        assert_eq!(s.by, "dovelle");
    }

    #[test]
    fn god_mode_shrugs_off_an_attack_and_resets_the_wait() {
        let mut s = ClassicSim::new(1, 1, &BTreeMap::new(), false);
        s.god = true;
        s.admin_send("dovelle", "doorL");
        let mut attacks = 0;
        for _ in 0..200 {
            for e in s.step(100.0, &idle()) {
                if matches!(e, SimEvent::Attack { .. }) {
                    attacks += 1;
                }
            }
        }
        assert!(attacks >= 2);
        assert_eq!(s.over, Over::Running);
    }

    #[test]
    fn partners_lose_patience_when_both_are_strong_enough() {
        // dovelle and ravelle at 8+: while ravelle waits at a door, dovelle's wait is 1400 ms shorter
        let mut s = ClassicSim::new(1, 1, &levels(&[("dovelle", 8), ("ravelle", 8), ("aki", 0), ("ilyra", 0), ("nyxie", 0)]), true);
        s.admin_send("ravelle", "doorR");
        s.step(100.0, &idle());
        s.step(100.0, &idle()); // the other's level is read from the previous step
        let grace = rules().door_grace(1);
        assert_eq!(s.hunter("dovelle").map(|h| h.attack_ms), Some(grace - 1400.0));
        // a weak partner does not count
        let mut w = ClassicSim::new(1, 1, &levels(&[("dovelle", 8), ("ravelle", 7), ("aki", 0), ("ilyra", 0), ("nyxie", 0)]), true);
        w.admin_send("ravelle", "doorR");
        w.step(100.0, &idle());
        w.step(100.0, &idle());
        assert_eq!(w.hunter("dovelle").map(|h| h.attack_ms), Some(grace));
    }

    #[test]
    fn a_full_door_turns_a_third_hunter_away_and_the_second_stands_behind() {
        let mut s = ClassicSim::new(5, 1, &levels(&[("dovelle", 20), ("ravelle", 20), ("aki", 0), ("ilyra", 20), ("nyxie", 0)]), true);
        s.admin_send("dovelle", "doorL");
        s.admin_send("ilyra", "doorL");
        assert_eq!(s.at_door(0), vec![0, 3]);
        let lit = SimInput { light_held: [true, false], ..SimInput::default() };
        s.step(100.0, &lit);
        // the one in front is held by the light, the one behind is not
        assert!(s.hunters[0].lit_ms > 0.0);
        assert_eq!(s.hunters[3].lit_ms, 0.0);
        assert!(s.hunters[3].behind);
    }

    #[test]
    fn the_same_seed_gives_the_same_night() {
        let run = |seed: u32| {
            let mut s = ClassicSim::new(5, seed, &BTreeMap::new(), false);
            let mut names = Vec::new();
            let mut input = SimInput::default();
            for i in 0..2700 {
                input.door_shut = [i % 70 < 35, i % 90 < 20];
                input.cam = i % 40 < 15;
                input.cam_room = "dining".to_string();
                for e in s.step(100.0, &input) {
                    names.push(format!("{e:?}"));
                }
                if s.over != Over::Running {
                    break;
                }
            }
            (names, s.rng_state(), s.elapsed)
        };
        let a = run(777);
        let b = run(777);
        assert_eq!(a, b);
        assert!(!a.0.is_empty());
        let c = run(778);
        assert_ne!(a.0, c.0, "a different seed plays a different night");
    }

    #[test]
    fn hunters_actually_move_through_the_building_on_a_normal_night() {
        let mut s = ClassicSim::new(6, 31, &BTreeMap::new(), false);
        let mut rooms = std::collections::BTreeSet::new();
        for _ in 0..1200 {
            for e in s.step(100.0, &SimInput { door_shut: [true, true], ..SimInput::default() }) {
                if let SimEvent::Moved { room, .. } = e {
                    rooms.insert(room);
                }
            }
        }
        assert!(rooms.len() >= 3, "{rooms:?}");
    }

    #[test]
    fn kathy_meets_only_on_a_hall_feed_and_a_miss_locks_the_key() {
        let mut s = ClassicSim::new(7, 9, &BTreeMap::new(), false);
        // not on the hall: a miss
        let (r, _) = s.meet(true, "stage", false);
        assert_eq!(r, MeetResult::Miss);
        let (r, _) = s.meet(true, "westhall", false);
        assert_eq!(r, MeetResult::Locked);
        // the lock runs out after 2000 ms
        for _ in 0..21 {
            s.step(100.0, &idle());
        }
        s.admin_send("kathy", "westhall");
        let (r, ev) = s.meet(true, "westhall", false);
        assert_eq!(r, MeetResult::Met);
        assert_eq!(ev.len(), 1);
        let k = s.hunter("kathy").map(|k| k.room.clone()).unwrap_or_default();
        assert!(k == "stage" || k == "kitchen", "{k}");
        // the monitor must be up and on her feed, and not in the dark
        s.admin_send("kathy", "easthall");
        for _ in 0..21 {
            s.step(100.0, &idle());
        }
        let (r, _) = s.meet(false, "easthall", false);
        assert_eq!(r, MeetResult::Miss);
        // a classic night has nothing to meet
        let mut c = ClassicSim::new(3, 1, &BTreeMap::new(), false);
        assert_eq!(c.meet(true, "westhall", false).0, MeetResult::None);
    }

    #[test]
    fn kathys_light_dips_only_at_her_door_while_it_is_on() {
        let mut s = ClassicSim::new(7, 9, &BTreeMap::new(), false);
        s.admin_send("kathy", "doorL");
        s.step(10.0, &SimInput { light_held: [true, false], ..SimInput::default() });
        assert_eq!(s.light_on, [true, false]);
        assert!(s.light_dip(0), "elapsed 10 ms is inside the first dip [0, 180)");
        assert!(!s.light_dip(1));
        s.step(2000.0, &SimInput { light_held: [true, false], ..SimInput::default() });
        // 2010 ms into the cycle of 2600: not in a dip window [1500,1760)
        assert!(!s.light_dip(0));
        let mut off = ClassicSim::new(7, 9, &BTreeMap::new(), false);
        off.admin_send("kathy", "doorL");
        off.step(10.0, &idle());
        assert!(!off.light_dip(0), "the light has to be on");
    }

    #[test]
    fn kathy_hums_every_1100_ms_at_her_door_and_a_shut_door_muffles_it() {
        let mut s = ClassicSim::new(7, 9, &BTreeMap::new(), false);
        s.admin_send("kathy", "doorR");
        let mut hums = Vec::new();
        for _ in 0..30 {
            for e in s.step(100.0, &SimInput { door_shut: [false, false], ..SimInput::default() }) {
                if let SimEvent::Hum { door, muffled, .. } = e {
                    hums.push((door, muffled));
                }
            }
            if s.over != Over::Running {
                break;
            }
        }
        assert!(!hums.is_empty() || s.over == Over::Caught);
        for (door, muffled) in &hums {
            assert_eq!(door, "doorR");
            assert!(!muffled);
        }
    }

    #[test]
    fn events_have_stable_names() {
        let e = SimEvent::Moved { id: "aki".into(), room: "corner".into(), from: "easthall".into() };
        assert_eq!(e.name(), "moved:aki:corner");
        assert_eq!(e.from_room(), "easthall");
        assert_eq!(SimEvent::Win.name(), "win:-:");
        assert_eq!(SimEvent::Out.kind(), "out");
    }

    #[test]
    fn a_digest_of_the_state_is_stable() {
        let digest = |seed: u32| {
            let mut s = ClassicSim::new(3, seed, &BTreeMap::new(), false);
            for _ in 0..500 {
                s.step(100.0, &idle());
            }
            let mut f = Fnv::new();
            f.f64(s.elapsed).f64(s.power).u64(s.rng_state() as u64);
            for h in &s.hunters {
                f.str(&h.room);
            }
            f.finish()
        };
        assert_eq!(digest(5), digest(5));
    }
}
