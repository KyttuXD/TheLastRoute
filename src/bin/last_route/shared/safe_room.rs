//! Safe rooms on the live Infinite route: the spec of a room and its live state, as Godot keeps them.
//!
//! The spec is the Godot `safe_room_data` dictionary (streamed_route_generator.gd:1416), built by
//! `world::route::live::safe_room`. The state is `StreamedWorldState.safe_rooms` (`_safe_room_state`,
//! `damage_safe_room`, `repair_safe_room`, `apply_safe_room_status`), the active checkpoint
//! (`activate_checkpoint`, `apply_checkpoint`) and the breach damage carried between steps
//! (`InfiniteRouteTest._update_safe_room_breaches`).
//!
//! The state lives with the run (`InfState` in `game::endless`), so it survives the window moving
//! away and back, as Godot's world state does. In a co-op room the host alone breaches rooms and publishes each room's
//! status as a `stream_safe_room` world event (`event_fields` / `parse_event`); every machine applies it.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// The `what` of the host's published safe-room status (every machine applies it, `Game::apply_safe_room_event`).
pub const EVENT: &str = "stream_safe_room";

/// The fields of a `stream_safe_room` event for `Outbox::emit`: the room id and its status.
pub fn event_fields(room_id: &str, status: Status) -> Vec<(&'static str, Value)> {
    vec![
        ("room_id", json!(room_id)),
        ("integrity", json!(status.integrity)),
        ("breached", json!(status.breached)),
    ]
}

/// A published room status, read back. `None` for another kind, an id that is not a `:SAFE` room id, or a status
/// that does not agree with itself (a breached room is at 0 integrity).
pub fn parse_event(o: &Map<String, Value>) -> Option<(String, Status)> {
    if o.get("what").and_then(Value::as_str) != Some(EVENT) {
        return None;
    }
    let id = o.get("room_id").and_then(Value::as_str)?;
    if id.is_empty() || id.len() > 64 || !id.ends_with(":SAFE") {
        return None;
    }
    let integrity = i32::try_from(o.get("integrity").and_then(Value::as_i64)?).ok()?;
    let breached = o.get("breached").and_then(Value::as_bool)?;
    if integrity < 0 || breached != (integrity <= 0) {
        return None;
    }
    Some((id.to_string(), Status { integrity, breached }))
}

/// A safe room's spec: the Godot `safe_room_data` dictionary.
#[derive(Clone, Debug, PartialEq)]
pub struct SafeRoom {
    /// `"x:y:z:SAFE"` (chunk_id + ":SAFE").
    pub id: String,
    /// `"x:y:z:CHECKPOINT"`: what the runner activates by standing in the intact room.
    pub checkpoint_id: String,
    pub max_integrity: i32,
    /// Hunters can batter the room (`breachable`).
    pub breachable: bool,
    /// A crowbar or rescue axe can repair it (`repairable`).
    pub repairable: bool,
    /// Scales the breach damage (`breach_pressure`, snapped to 0.01).
    pub breach_pressure: f64,
}

/// A safe room's live state (`_safe_room_state`): its integrity and whether hunters have breached it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    pub integrity: i32,
    pub breached: bool,
}

/// Breach damage per second for one hunter at pressure 1 (`delta * breach_pressure * 7.0 * breachers`).
pub const BREACH_RATE: f64 = 7.0;

/// The run's safe-room state: integrity by room id (only rooms something has touched; the rest are at
/// their spec integrity), the active checkpoint, and the breach damage carried between steps.
#[derive(Clone, Debug, Default)]
pub struct SafeRooms {
    rooms: HashMap<String, Status>,
    /// The checkpoint id the runner last activated (`active_checkpoint`), "" before any.
    active_checkpoint: String,
    /// Room id -> fractional breach damage carried into the next step (`_safe_damage_accumulators`).
    carried: HashMap<String, f64>,
}

impl SafeRooms {
    /// The room's state: its spec integrity and not breached until something changes it.
    pub fn status(&self, room: &SafeRoom) -> Status {
        self.rooms.get(&room.id).copied().unwrap_or(Status {
            integrity: room.max_integrity,
            breached: false,
        })
    }

    /// Hunters batter the room: integrity down by `amount` (never below 0), breached at 0.
    /// A room that is not breachable is left as it is (`damage_safe_room`).
    pub fn damage(&mut self, room: &SafeRoom, amount: i32) -> Status {
        let mut s = self.status(room);
        if !room.breachable {
            return s;
        }
        s.integrity = (s.integrity - amount.max(0)).max(0);
        s.breached = s.integrity <= 0;
        self.rooms.insert(room.id.clone(), s);
        s
    }

    /// Applies a room status the host published, by room id (every machine). Returns whether the room was breached
    /// before, so the caller can tell a fresh breach.
    pub fn set_by_id(&mut self, id: &str, status: Status) -> bool {
        let was = self.rooms.get(id).is_some_and(|s| s.breached);
        self.rooms.insert(id.to_string(), status);
        was
    }

    /// A crowbar or rescue axe repairs the room: integrity up by `amount` (capped at the spec max).
    /// A breached room comes back as soon as its integrity is above 0 (`repair_safe_room`).
    pub fn repair(&mut self, room: &SafeRoom, amount: i32) -> Status {
        let mut s = self.status(room);
        if !room.repairable {
            return s;
        }
        s.integrity = (s.integrity + amount.max(0)).min(room.max_integrity);
        s.breached = s.integrity <= 0;
        self.rooms.insert(room.id.clone(), s);
        s
    }

    /// Standing in an intact room activates its checkpoint. A breached room activates nothing
    /// (`activate_checkpoint`). Returns true when the checkpoint is now active.
    pub fn activate(&mut self, room: &SafeRoom) -> bool {
        if self.status(room).breached || room.checkpoint_id.is_empty() {
            return false;
        }
        self.active_checkpoint = room.checkpoint_id.clone();
        true
    }

    /// The checkpoint id the runner last activated, "" before any.
    pub fn active_checkpoint(&self) -> &str {
        &self.active_checkpoint
    }

    /// One step of breaching by `breachers` hunters over `dt` seconds. Returns the whole points of
    /// damage due this step (0 when none yet); the fraction is carried to the next step.
    pub fn breach(&mut self, room: &SafeRoom, dt: f32, breachers: u32) -> i32 {
        let carried = self.carried.get(&room.id).copied().unwrap_or(0.0);
        let accumulated = carried
            + f64::from(dt.max(0.0)) * room.breach_pressure * BREACH_RATE * f64::from(breachers);
        let damage = accumulated.floor();
        self.carried.insert(room.id.clone(), accumulated - damage);
        damage as i32
    }

    /// Drops the carried damage of every room not being breached this step (the Godot step erases
    /// accumulators for rooms absent from its breach list).
    pub fn keep_breaching(&mut self, breaching: &[&str]) {
        self.carried.retain(|id, _| breaching.contains(&id.as_str()));
    }
}

impl SafeRooms {
    /// Walking into an intact safe room records its checkpoint (TODO "Safe rooms with breach and checkpoints"). Returns
    /// the checkpoint id only when it changed, so the caller saves once per entry, not every step.
    pub fn enter(&mut self, room: &SafeRoom) -> Option<String> {
        if self.active_checkpoint == room.checkpoint_id {
            return None;
        }
        if self.activate(room) {
            Some(room.checkpoint_id.clone())
        } else {
            None
        }
    }
}

/// The safe-room generation and breach rules (`data/safe_room.json`, `lr.safe_room/1`).
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub interval: i32,
    pub offset: i32,
    pub first_depth: i32,
    pub max_integrity: i32,
    pub breach_warmup_s: f32,
    pub breach_seconds: f32,
    /// Breach pressure scale: relaxed, standard, hard, nightmare.
    pub pressure_scale: [f32; 4],
    pub checkpoint_on_entry: bool,
}

const RULES_TEXT: &str = include_str!("../data/safe_room.json");

pub fn parse_rules(text: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("safe_room.json: {e}"))?;
    if v["schema"].as_str() != Some("lr.safe_room/1") {
        return Err("safe_room.json: 'schema' must be lr.safe_room/1".into());
    }
    let i = |k: &str| v[k].as_i64().ok_or_else(|| format!("safe_room.json: '{k}' missing or not a whole number")).map(|n| n as i32);
    let f = |x: &Value, k: &str| {
        x[k].as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| n as f32).ok_or_else(|| format!("safe_room.json: '{k}' missing or negative"))
    };
    let p = &v["breach_pressure_scale"];
    let r = Rules {
        interval: i("interval")?,
        offset: i("offset")?,
        first_depth: i("first_depth")?,
        max_integrity: i("max_integrity")?,
        breach_warmup_s: f(&v, "breach_warmup_s")?,
        breach_seconds: f(&v, "breach_seconds")?,
        pressure_scale: [f(p, "relaxed")?, f(p, "standard")?, f(p, "hard")?, f(p, "nightmare")?],
        checkpoint_on_entry: v["checkpoint_on_entry"].as_bool().unwrap_or(true),
    };
    if r.interval < 1 || r.first_depth < 1 || r.max_integrity < 1 || r.breach_seconds <= 0.0 {
        return Err("safe_room.json: interval, first_depth, max_integrity and breach_seconds must be above 0".into());
    }
    Ok(r)
}

pub fn rules() -> &'static Rules {
    static R: std::sync::OnceLock<Rules> = std::sync::OnceLock::new();
    R.get_or_init(|| parse_rules(RULES_TEXT).unwrap_or_else(|e| panic!("{e}")))
}

impl Rules {
    /// Whether the Infinite route level `level` (0 is the start, negative is down) gets a safe room: never level 0,
    /// then every `interval` levels from `first_depth`.
    pub fn level_has_safe_room(&self, level: i32) -> bool {
        let depth = level.abs();
        level != 0 && depth >= self.first_depth && (depth - self.first_depth + self.offset).rem_euclid(self.interval) == 0
    }

    pub fn pressure_for(&self, d: crate::config::Difficulty) -> f32 {
        use crate::config::Difficulty::*;
        self.pressure_scale[match d {
            Relaxed => 0,
            Standard => 1,
            Hard => 2,
            Nightmare => 3,
        }]
    }
}

/// The host-only breach timer of one room: a hunter must stand at the door for `breach_warmup_s`, then the room breaks
/// `breach_seconds` later. A guest never advances it (it applies the host `stream_safe_room` event instead).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BreachTimer {
    at_door: f32,
    breaking: f32,
}

impl BreachTimer {
    /// Returns true on the step the room breaks.
    pub fn step(&mut self, dt: f32, is_host: bool, hunter_at_door: bool, pressure: f32, r: &Rules) -> bool {
        if !is_host {
            return false;
        }
        let dt = dt.max(0.0);
        if !hunter_at_door {
            *self = BreachTimer::default();
            return false;
        }
        self.at_door += dt;
        if self.at_door < r.breach_warmup_s {
            return false;
        }
        self.breaking += dt * pressure.max(0.0);
        self.breaking >= r.breach_seconds
    }

    pub fn progress(&self, r: &Rules) -> f32 {
        (self.breaking / r.breach_seconds.max(1e-6)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod rest_tests {
    use super::*;
    use crate::config::Difficulty;

    fn spec(id: &str) -> SafeRoom {
        SafeRoom { id: format!("{id}:SAFE"), checkpoint_id: format!("{id}:CHECKPOINT"), max_integrity: 100, breachable: true, repairable: true, breach_pressure: 1.0 }
    }

    #[test]
    fn rules_parse_and_level_zero_never_has_one() {
        let r = rules();
        assert_eq!(&parse_rules(RULES_TEXT).unwrap(), r);
        assert!(!r.level_has_safe_room(0));
        assert!(r.level_has_safe_room(-r.first_depth));
        assert!(r.level_has_safe_room(-(r.first_depth + r.interval)));
        assert!(!r.level_has_safe_room(-(r.first_depth + 1)) || r.interval == 1);
        assert!(parse_rules("{}").is_err());
    }

    #[test]
    fn a_changed_interval_changes_which_levels_get_one() {
        let two = parse_rules(&RULES_TEXT.replace("\"interval\": 4", "\"interval\": 2")).unwrap();
        let count = |r: &Rules| (-40..=40).filter(|l| r.level_has_safe_room(*l)).count();
        assert!(count(&two) > count(rules()));
    }

    #[test]
    fn entering_an_intact_room_records_the_checkpoint_once() {
        let mut s = SafeRooms::default();
        let room = spec("1:0:-2");
        assert_eq!(s.enter(&room), Some(room.checkpoint_id.clone()));
        assert_eq!(s.active_checkpoint(), room.checkpoint_id);
        assert_eq!(s.enter(&room), None, "already recorded");
        let other = spec("2:0:-6");
        s.damage(&other, 1000);
        assert_eq!(s.enter(&other), None, "a breached room records nothing");
        assert_eq!(s.active_checkpoint(), room.checkpoint_id);
    }

    #[test]
    fn the_breach_timer_runs_on_the_host_only_after_the_warmup() {
        let r = rules();
        let mut host = BreachTimer::default();
        let mut guest = BreachTimer::default();
        let (mut host_broke, mut guest_broke) = (false, false);
        let mut t = 0.0;
        while t < r.breach_warmup_s + r.breach_seconds + 2.0 {
            host_broke |= host.step(0.1, true, true, 1.0, r);
            guest_broke |= guest.step(0.1, false, true, 1.0, r);
            if t < r.breach_warmup_s - 0.2 {
                assert_eq!(host.progress(r), 0.0, "warming up");
            }
            t += 0.1;
        }
        assert!(host_broke && !guest_broke);
        assert_eq!(guest, BreachTimer::default());
        // leaving the door resets it
        host.step(0.1, true, false, 1.0, r);
        assert_eq!(host, BreachTimer::default());
        assert!(r.pressure_for(Difficulty::Nightmare) > r.pressure_for(Difficulty::Relaxed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> SafeRoom {
        SafeRoom {
            id: "0:0:0:SAFE".into(),
            checkpoint_id: "0:0:0:CHECKPOINT".into(),
            max_integrity: 100,
            breachable: true,
            repairable: true,
            breach_pressure: 1.08,
        }
    }

    #[test]
    fn an_untouched_room_is_at_full_integrity() {
        let s = SafeRooms::default();
        assert_eq!(s.status(&room()), Status { integrity: 100, breached: false });
    }

    #[test]
    fn damage_clamps_at_zero_and_breaches() {
        let mut s = SafeRooms::default();
        let r = room();
        assert_eq!(s.damage(&r, 60), Status { integrity: 40, breached: false });
        assert_eq!(s.damage(&r, 500), Status { integrity: 0, breached: true });
    }

    #[test]
    fn repair_caps_at_max_and_unbreaches() {
        let mut s = SafeRooms::default();
        let r = room();
        s.damage(&r, 100);
        assert!(s.status(&r).breached);
        assert_eq!(s.repair(&r, 18), Status { integrity: 18, breached: false });
        assert_eq!(s.repair(&r, 500), Status { integrity: 100, breached: false });
    }

    #[test]
    fn a_room_that_cannot_be_breached_or_repaired_stays_put() {
        let mut s = SafeRooms::default();
        let mut r = room();
        r.breachable = false;
        r.repairable = false;
        assert_eq!(s.damage(&r, 50).integrity, 100);
        s.damage(&SafeRoom { breachable: true, ..room() }, 50);
        assert_eq!(s.repair(&r, 50).integrity, 50);
    }

    #[test]
    fn a_breached_room_activates_no_checkpoint() {
        let mut s = SafeRooms::default();
        let r = room();
        assert!(s.activate(&r));
        assert_eq!(s.active_checkpoint(), "0:0:0:CHECKPOINT");
        s.damage(&r, 100);
        let other = SafeRoom { checkpoint_id: "1:0:0:CHECKPOINT".into(), ..room() };
        assert!(!s.activate(&other), "the same room, breached, activates nothing");
        assert_eq!(s.active_checkpoint(), "0:0:0:CHECKPOINT");
    }

    #[test]
    fn a_published_room_status_round_trips_and_a_bad_one_is_refused() {
        let st = Status { integrity: 0, breached: true };
        let mut o = Map::new();
        for (k, v) in event_fields("0:0:0:SAFE", st) {
            o.insert(k.into(), v);
        }
        assert!(parse_event(&o).is_none(), "an event with no kind is not a room status");
        o.insert("what".into(), json!(EVENT));
        assert_eq!(parse_event(&o), Some(("0:0:0:SAFE".to_string(), st)));
        o.insert("breached".into(), json!(false));
        assert!(parse_event(&o).is_none(), "a breached-flag that disagrees with integrity is dropped");
        o.insert("breached".into(), json!(true));
        o.insert("room_id".into(), json!("0:0:0:CHECKPOINT"));
        assert!(parse_event(&o).is_none(), "only a SAFE room id is a room");
    }

    #[test]
    fn a_host_status_replaces_a_guests_room_and_reports_a_fresh_breach() {
        let mut s = SafeRooms::default();
        let r = room();
        assert!(!s.set_by_id(&r.id, Status { integrity: 40, breached: false }));
        assert_eq!(s.status(&r), Status { integrity: 40, breached: false });
        assert!(!s.set_by_id(&r.id, Status { integrity: 0, breached: true }), "the first breach is fresh");
        assert!(s.set_by_id(&r.id, Status { integrity: 0, breached: true }), "already breached before this one");
    }

    #[test]
    fn one_hunter_for_a_second_does_seven_points() {
        let mut s = SafeRooms::default();
        let r = room();
        let mut total = 0;
        for _ in 0..30 {
            total += s.breach(&r, 1.0 / 30.0, 1);
        }
        // 1.08 * 7 = 7.56 a second: seven whole points, the rest carried
        assert_eq!(total, 7);
        assert!((s.carried["0:0:0:SAFE"] - 0.56).abs() < 0.05);
    }

    #[test]
    fn the_carry_is_dropped_when_nobody_breaches() {
        let mut s = SafeRooms::default();
        let r = room();
        s.breach(&r, 0.5, 1);
        assert!(s.carried.contains_key("0:0:0:SAFE"));
        s.keep_breaching(&[]);
        assert!(!s.carried.contains_key("0:0:0:SAFE"));
    }
}
