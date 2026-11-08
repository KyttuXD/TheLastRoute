//! Where and how big a hunter stands: yaw helpers, the hunter scale (NightWatchHunterScale) and the shared stand
//! slot offsets. Pure maths, no GL.
#![allow(dead_code)]

use super::anim::clip::Rig;
use super::anim::MODEL_SCALE;
use macroquad::prelude::{vec3, Vec3};

/// The humanoid hunters' height in metres at the Godot scale (1.5x, about 2.58 m). Kept as the reference the
/// port scales down from; the height actually drawn comes from `data/hunter_scale.json` (see `HunterScale`).
pub const HUNTER_HEIGHT_M: f32 = 2.58;

/// data/hunter_scale.json: ONE world height target for the humanoid hunters (the Godot maps are built for 2.58 m
/// hunters but the port's player, desks and chairs read smaller, so they looked like giants), an optional multiplier
/// per hunter id and one per mode. Every instantiate path (office, feeds, death moment) goes through `world_scale`.
pub struct HunterScale {
    pub reference_height_m: f32,
    pub world_height_m: f32,
    per_hunter: std::collections::HashMap<String, f32>,
    per_mode: std::collections::HashMap<String, f32>,
}

const HUNTER_SCALE_JSON: &str = include_str!("../data/hunter_scale.json");

impl HunterScale {
    fn parse(text: &str) -> HunterScale {
        let j: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
        let map = |k: &str| -> std::collections::HashMap<String, f32> {
            j[k].as_object()
                .map(|o| o.iter().filter_map(|(id, v)| v.as_f64().map(|f| (id.clone(), f as f32))).collect())
                .unwrap_or_default()
        };
        let reference = j["reference_height_m"].as_f64().unwrap_or(HUNTER_HEIGHT_M as f64) as f32;
        HunterScale {
            reference_height_m: reference,
            world_height_m: j["world_height_m"].as_f64().unwrap_or(reference as f64) as f32,
            per_hunter: map("per_hunter"),
            per_mode: map("per_mode"),
        }
    }

    /// The shared config, parsed once.
    pub fn get() -> &'static HunterScale {
        static CFG: std::sync::OnceLock<HunterScale> = std::sync::OnceLock::new();
        CFG.get_or_init(|| HunterScale::parse(HUNTER_SCALE_JSON))
    }

    /// The factor over the Godot draw scale for a hunter in a mode ("" = no mode): world/reference height times the
    /// hunter's and the mode's multipliers (1.0 when absent).
    pub fn factor(&self, id: &str, mode: &str) -> f32 {
        let base = self.world_height_m / self.reference_height_m.max(1e-6);
        base * self.per_hunter.get(id).copied().unwrap_or(1.0) * self.per_mode.get(mode).copied().unwrap_or(1.0)
    }

    /// A humanoid's drawn height in metres.
    pub fn height_m(&self, id: &str, mode: &str) -> f32 {
        self.reference_height_m * self.factor(id, mode)
    }

    /// The tallest humanoid in any mode (for clearance checks).
    pub fn max_height_m(&self) -> f32 {
        let h = self.per_hunter.values().copied().fold(1.0_f32, f32::max);
        let m = self.per_mode.values().copied().fold(1.0_f32, f32::max);
        self.world_height_m * h * m
    }

    /// The shortest humanoid in any mode.
    pub fn min_height_m(&self) -> f32 {
        let h = self.per_hunter.values().copied().fold(1.0_f32, f32::min);
        let m = self.per_mode.values().copied().fold(1.0_f32, f32::min);
        self.world_height_m * h * m
    }
}

/// THE instantiate-path scale: metres per bbmodel unit a hunter rig is drawn at in `mode` (Godot draw scale times
/// the data-driven factor).
pub fn world_scale(rig: &Rig, id: &str, mode: &str) -> f32 {
    rig.draw_scale() * HunterScale::get().factor(id, mode)
}

/// The yaw that turns a hunter standing at `from` to face `to` on the ground plane. Yaw 0 faces -z and the forward
/// vector is (-sin yaw, -cos yaw), the same convention as the mannequin and the player.
pub fn yaw_toward(from: Vec3, to: Vec3) -> f32 {
    (from.x - to.x).atan2(from.z - to.z)
}

/// `face()` of the contract: the yaw to look along a direction (not a target point). Zero length keeps `keep`.
pub fn yaw_of_dir(dir: Vec3, keep: f32) -> f32 {
    if dir.x * dir.x + dir.z * dir.z < 1e-6 {
        keep
    } else {
        (-dir.x).atan2(-dir.z)
    }
}

/// The yaw to face `to` from `from`, keeping `keep` when they coincide on the ground.
pub fn face(from: Vec3, to: Vec3, keep: f32) -> f32 {
    yaw_of_dir(to - from, keep)
}

/// Metres per bbmodel unit a rig is drawn at (HunterRoster.scale_for over the glb's unit): the humanoids at 1.5x,
/// creature rigs at their own scale.
pub fn hunter_scale(rig: &Rig) -> f32 {
    rig.draw_scale()
}

/// Same with the data-driven world scale for `id` in `mode` applied (what every actor is drawn at).
pub fn hunter_scale_for(rig: &Rig, id: &str, mode: &str) -> f32 {
    world_scale(rig, id, mode)
}

/// The factor over the authored size (1.5 for the humanoids). Creature rigs report their own ratio to 1.5/unit.
pub fn scale_factor(rig: &Rig) -> f32 {
    let base = 1.0 / rig.unit.max(1e-6);
    rig.draw_scale() / base
}

/// The shared stand spot offsets inside a room (NightWatchBuilding stand slots): slot 0 and slot 1; further slots
/// step along x.
pub fn slot_offset(slot: usize) -> Vec3 {
    match slot {
        0 => vec3(-0.6, 0.0, 0.4),
        1 => vec3(1.0, 0.0, -0.6),
        n => vec3(1.0 + 1.3 * (n as f32 - 1.0), 0.0, -0.6),
    }
}

/// A stand spot: room centre plus the slot offset, with an optional platform lift (the stage adds 0.45 up, 1.4 back).
pub fn stand_spot(centre: Vec3, slot: usize, platform: bool) -> Vec3 {
    let mut p = centre + slot_offset(slot);
    if platform {
        p += vec3(0.0, 0.45, -1.4);
    }
    p
}

/// The default rig scale when no rig is at hand.
pub const DEFAULT_MODEL_SCALE: f32 = MODEL_SCALE;

#[cfg(test)]
mod tests {
    use super::*;

    fn forward(yaw: f32) -> Vec3 {
        vec3(-yaw.sin(), 0.0, -yaw.cos())
    }

    #[test]
    fn yaw_toward_points_the_forward_vector_at_the_target() {
        let from = vec3(0.0, 0.0, 0.0);
        for to in [vec3(0.0, 0.0, -5.0), vec3(5.0, 0.0, 0.0), vec3(0.0, 0.0, 5.0), vec3(-5.0, 0.0, 0.0), vec3(3.0, 0.0, -4.0)] {
            let f = forward(yaw_toward(from, to));
            let d = (to - from).normalize();
            assert!((f - d).length() < 1e-4, "{to:?} -> {f:?}");
        }
    }

    #[test]
    fn yaw_toward_quadrants() {
        let o = Vec3::ZERO;
        assert!(yaw_toward(o, vec3(0.0, 0.0, -1.0)).abs() < 1e-6);
        assert!((yaw_toward(o, vec3(1.0, 0.0, 0.0)) + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((yaw_toward(o, vec3(-1.0, 0.0, 0.0)) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((yaw_toward(o, vec3(0.0, 0.0, 1.0)).abs() - std::f32::consts::PI).abs() < 1e-5);
    }

    #[test]
    fn face_agrees_with_yaw_toward_and_keeps_yaw_when_on_the_spot() {
        let a = vec3(1.0, 0.0, 2.0);
        let b = vec3(-3.0, 1.0, 0.5);
        assert!((face(a, b, 0.0) - yaw_toward(a, b)).abs() < 1e-5);
        assert_eq!(face(a, a + vec3(0.0, 2.0, 0.0), 1.25), 1.25);
    }

    #[test]
    fn hunter_scale_file_parses_and_factor_is_world_over_reference() {
        let c = HunterScale::get();
        assert!(c.world_height_m > 0.0 && c.reference_height_m > 0.0);
        let f = c.factor("dovelle", "classic");
        assert!((f - c.world_height_m / c.reference_height_m).abs() < 1e-6);
        assert!((c.height_m("dovelle", "") - c.world_height_m).abs() < 1e-5);
    }

    #[test]
    fn multipliers_apply_per_hunter_and_per_mode() {
        let c = HunterScale::parse(r#"{"reference_height_m":2.0,"world_height_m":1.0,"per_hunter":{"a":2.0},"per_mode":{"m":0.5}}"#);
        assert!((c.factor("a", "m") - 0.5).abs() < 1e-6);
        assert!((c.factor("b", "") - 0.5).abs() < 1e-6);
        assert!((c.max_height_m() - 2.0).abs() < 1e-6);
        assert!((c.min_height_m() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn every_modes_door_clears_every_hunter_with_head_room_and_hunters_top_the_player_eye() {
        use crate::gfx::{building::DOOR_TOP, office::DOOR_H, prims::DOOR_HEIGHT};
        let c = HunterScale::get();
        // The door tops each mode builds with: office (classic, dayshift, story), building wings, bedroom wing, lure,
        // and the shared wall_with_gaps doorway (mask, mixed).
        let doors = [("office", DOOR_H), ("building", DOOR_TOP), ("prims", DOOR_HEIGHT), ("bedroom", 3.2), ("lure", 3.2)];
        for (name, door) in doors {
            assert!(door >= c.max_height_m() + 0.3, "{name} door {door} leaves under 0.3 m over a {} m hunter", c.max_height_m());
        }
        assert!(c.min_height_m() >= crate::player::EYE_HEIGHT, "hunters must be at least eye height ({} < {})", c.min_height_m(), crate::player::EYE_HEIGHT);
        assert!(c.max_height_m() < crate::gfx::office::HEIGHT - 0.5, "ceiling clearance");
    }

    #[test]
    fn stand_spots() {
        assert_eq!(slot_offset(0), vec3(-0.6, 0.0, 0.4));
        assert_eq!(slot_offset(1), vec3(1.0, 0.0, -0.6));
        let s = stand_spot(vec3(0.0, 0.0, -40.0), 0, true);
        assert!((s - vec3(-0.6, 0.45, -41.0)).length() < 1e-5);
    }
}
