//! Safe rooms on the live Infinite route: the spec of a room and its live state, as Godot keeps them.
//!
//! The spec is the Godot `safe_room_data` dictionary (streamed_route_generator.gd:1416), built by
//! `world::route::live::safe_room`. The state is `StreamedWorldState.safe_rooms` (`_safe_room_state`,
//! `damage_safe_room`, `repair_safe_room`, `apply_safe_room_status`), the active checkpoint
//! (`activate_checkpoint`, `apply_checkpoint`) and the breach damage carried between steps
//! (`InfiniteRouteTest._update_safe_room_breaches`).
//!
//! The state lives with the run (`InfState` in `game::endless`), so it survives the window moving
//! away and back, as Godot's world state does. It is solo for now: nothing is sent to other players.

use std::collections::HashMap;

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
