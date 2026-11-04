//! Void Red: a hall runner. She moves at 155% speed in corridors and 78% speed inside rooms, and the
//! room test is re-read every frame, so she changes pace the moment she crosses a threshold.
//!
//! Godot source: game/hunters/gimmicks/void_red_gimmick.gd (VoidRedGimmick, extends VoidGimmick).
//! Player-facing text: game/hunters/gimmicks/void_hunter_gimmicks/void_hunter_gimmick_chart.gd:44-46.
//! She has no timers, no events and no player-side effect; the whole power is the speed scale.

use crate::hunters::brain::Brain;
use crate::hunters::gimmicks::Ctx;
use crate::world::facility::Facility;

/// Speed multiplier while she is in a hallway (void_red_gimmick.gd:6, CORRIDOR_SPEED_SCALE).
pub const CORRIDOR_SPEED_SCALE: f32 = 1.55;
/// Speed multiplier while she is inside a room (void_red_gimmick.gd:7, ROOM_SPEED_SCALE).
pub const ROOM_SPEED_SCALE: f32 = 0.78;

/// Void Red's gimmick state. Godot `_in_room` (void_red_gimmick.gd:9) starts false, so before her first
/// perception pass she reads as being in a corridor, exactly as the default does here.
#[derive(Default)]
pub struct VoidRed {
    in_room: bool,
}

impl VoidRed {
    /// Godot before_perception (void_red_gimmick.gd:12-13): `_in_room = not generator.room_at(position).is_empty()`.
    /// The room lookup is the facility's own footprint test, the same one the other spatial gimmicks use (Ilyra).
    pub fn before(&mut self, b: &Brain, c: &Ctx) {
        self.in_room = in_room_at(c.fac, b.pos);
    }

    /// Godot speed_scale (void_red_gimmick.gd:16-17): ROOM_SPEED_SCALE when in a room, else CORRIDOR_SPEED_SCALE.
    pub fn speed_scale(&self) -> f32 {
        scale_for(self.in_room)
    }
}

/// The trigger rule (void_red_gimmick.gd:13): she counts as inside a room whenever her position is inside any
/// room footprint on the facility. Lobby rooms count, as they do for `room_at` in the Godot generator.
pub fn in_room_at(fac: &Facility, pos: (f32, f32)) -> bool {
    fac.room_at(pos.0 as f64, pos.1 as f64).is_some()
}

/// The speed multiplier for a room flag (void_red_gimmick.gd:17).
pub fn scale_for(in_room: bool) -> f32 {
    if in_room {
        ROOM_SPEED_SCALE
    } else {
        CORRIDOR_SPEED_SCALE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::catalog::Catalog;
    use crate::world::facility::Options;

    #[test]
    fn the_scales_are_the_godot_numbers() {
        assert_eq!(CORRIDOR_SPEED_SCALE, 1.55, "void_red_gimmick.gd:6");
        assert_eq!(ROOM_SPEED_SCALE, 0.78, "void_red_gimmick.gd:7");
        assert_eq!(scale_for(false), 1.55, "a hallway runs at 155%");
        assert_eq!(scale_for(true), 0.78, "a room runs at 78%");
    }

    #[test]
    fn she_starts_in_a_corridor_like_the_godot_default() {
        // _in_room := false (void_red_gimmick.gd:9): before any perception pass she is at the hallway speed.
        assert_eq!(VoidRed::default().speed_scale(), CORRIDOR_SPEED_SCALE);
    }

    #[test]
    fn the_scale_follows_the_room_flag_each_frame() {
        // No hysteresis and no timer in Godot: the flag set by the latest pass decides the scale at once.
        let mut g = VoidRed::default();
        g.in_room = true;
        assert_eq!(g.speed_scale(), ROOM_SPEED_SCALE, "inside a room she slows to 78%");
        g.in_room = false;
        assert_eq!(g.speed_scale(), CORRIDOR_SPEED_SCALE, "back in a hallway she speeds to 155% at once");
    }

    #[test]
    fn the_room_trigger_reads_the_facility_footprint() {
        let cat = Catalog::load();
        let fac = Facility::generate("VOID-RED-1", &Options::default(), &cat);
        let room = &fac.rooms[0];
        let inside = (room.x as f32 + 0.5, room.y as f32 + 0.5);
        assert!(in_room_at(&fac, inside), "the centre of a room counts as in the room");
        let corridor = fac
            .floor_cells()
            .iter()
            .map(|c| c.center())
            .find(|p| !in_room_at(&fac, *p))
            .expect("a generated facility has at least one hallway cell");
        assert!(!in_room_at(&fac, corridor), "a hallway cell is not in a room");
        assert_eq!(scale_for(in_room_at(&fac, inside)), ROOM_SPEED_SCALE);
        assert_eq!(scale_for(in_room_at(&fac, corridor)), CORRIDOR_SPEED_SCALE);
    }
}
