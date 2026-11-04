//! Where a hider's body stands (or lies) inside a locker, so hunter locker animations play against a real body.
//! Godot hiding_inside_position (route_interactable_3d.gd 489): two hiders shift by hide_size.x * .22 across and
//! +-0.05 in depth. Pure placement; the scene draws the body with `net::runner_figure::draw_figure` (task 4).
//! Tunables: data/render/locker_shell.json, `occupant` object.

use super::locker_shell::PATH;
use crate::content::embedded;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyState {
    /// Standing in a wardrobe locker, idle at `idle_phase`.
    Standing,
    /// Lying on a bed.
    Lying,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OccupantPose {
    /// World position of the feet.
    pub pos: [f32; 3],
    pub yaw: f32,
    pub state: BodyState,
    /// Idle clip phase (standing only).
    pub idle_phase: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupantData {
    pub hide_size_x: f32,
    pub slot_shift_factor: f32,
    pub slot_z_shift: f32,
    pub inside_back_m: f32,
    pub idle_phase: f32,
    pub bed_height_m: f32,
    pub bed_lying_yaw_rad: f32,
    pub bed_styles: Vec<String>,
}

impl OccupantData {
    /// Parses the file's `occupant` object. A missing or malformed key is reported by name.
    pub fn parse(text: &str) -> Result<OccupantData, String> {
        let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let o = root.get("occupant").ok_or_else(|| "occupant: missing".to_string())?;
        let num = |key: &str| {
            o.get(key)
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
                .ok_or_else(|| format!("occupant.{key}: expected a number"))
        };
        // read in the file's order so the first missing key is the one reported
        let hide_size_x = num("hide_size_x")?;
        let slot_shift_factor = num("slot_shift_factor")?;
        let slot_z_shift = num("slot_z_shift")?;
        let inside_back_m = num("inside_back_m")?;
        let idle_phase = num("idle_phase")?;
        let bed_height_m = num("bed_height_m")?;
        let bed_lying_yaw_rad = num("bed_lying_yaw_rad")?;
        let bed_styles = o
            .get("bed_styles")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            .ok_or_else(|| "occupant.bed_styles: expected a list of names".to_string())?;
        Ok(OccupantData {
            hide_size_x,
            slot_shift_factor,
            slot_z_shift,
            inside_back_m,
            idle_phase,
            bed_height_m,
            bed_lying_yaw_rad,
            bed_styles,
        })
    }
}

/// The occupant data compiled into the executable; Standard values if the file fails to parse (reported on stderr).
pub fn data() -> &'static OccupantData {
    static DATA: OnceLock<OccupantData> = OnceLock::new();
    DATA.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        OccupantData::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}; using the Standard occupant");
            OccupantData {
                hide_size_x: 0.8,
                slot_shift_factor: 0.22,
                slot_z_shift: 0.05,
                inside_back_m: 0.0,
                idle_phase: 0.55,
                bed_height_m: 0.45,
                bed_lying_yaw_rad: std::f32::consts::FRAC_PI_2,
                bed_styles: vec!["bed".to_string()],
            }
        })
    })
}

/// The pose of the hider in `slot` (0-based) of a locker holding `occupants` of `capacity`, or None when that slot
/// is empty. `centre` is the locker's (x, z) centre; `facing` points from the back wall into the room.
pub fn occupant_pose(
    d: &OccupantData,
    style: &str,
    slot: usize,
    occupants: usize,
    capacity: usize,
    centre: (f32, f32),
    facing: (i32, i32),
) -> Option<OccupantPose> {
    if slot >= occupants || slot >= capacity.max(1) {
        return None;
    }
    let (fx, fz) = (facing.0 as f32, facing.1 as f32);
    let len = (fx * fx + fz * fz).sqrt().max(1e-6);
    let (fx, fz) = (fx / len, fz / len);
    let (rx, rz) = (fz, -fx);
    let yaw = fx.atan2(fz);
    let (side, depth) = if capacity >= 2 {
        let s = if slot == 0 { -1.0 } else { 1.0 };
        (s * d.hide_size_x * d.slot_shift_factor, s * d.slot_z_shift)
    } else {
        (0.0, 0.0)
    };
    let back = -d.inside_back_m + depth;
    let (x, z) = (centre.0 + rx * side + fx * back, centre.1 + rz * side + fz * back);
    let lying = d.bed_styles.iter().any(|s| s == style);
    Some(if lying {
        OccupantPose {
            pos: [x, d.bed_height_m, z],
            yaw: yaw + d.bed_lying_yaw_rad,
            state: BodyState::Lying,
            idle_phase: 0.0,
        }
    } else {
        OccupantPose { pos: [x, 0.0, z], yaw, state: BodyState::Standing, idle_phase: d.idle_phase }
    })
}

/// Hands each occupied slot's pose to `draw_one`, which calls `runner_figure::draw_figure` (the scene's local runner
/// skips this in the first-person camera; remote hiders come from their RemoteView `hidden` field).
pub fn draw(
    d: &OccupantData,
    style: &str,
    occupants: usize,
    capacity: usize,
    centre: (f32, f32),
    facing: (i32, i32),
    draw_one: &mut dyn FnMut(usize, &OccupantPose),
) {
    for slot in 0..occupants.min(capacity.max(1)) {
        if let Some(p) = occupant_pose(d, style, slot, occupants, capacity, centre, facing) {
            draw_one(slot, &p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> OccupantData {
        OccupantData::parse(include_str!("../data/render/locker_shell.json")).unwrap()
    }

    #[test]
    fn two_hider_capacity_gives_distinct_slots() {
        let d = d();
        let a = occupant_pose(&d, "wardrobe", 0, 2, 2, (5.0, 5.0), (0, 1)).unwrap();
        let b = occupant_pose(&d, "wardrobe", 1, 2, 2, (5.0, 5.0), (0, 1)).unwrap();
        assert_ne!(a.pos, b.pos);
        let gap = (a.pos[0] - b.pos[0]).abs();
        assert!((gap - 2.0 * d.hide_size_x * d.slot_shift_factor).abs() < 1e-4);
    }

    #[test]
    fn single_hider_stands_at_the_centre() {
        let d = d();
        let p = occupant_pose(&d, "wardrobe", 0, 1, 1, (3.0, 4.0), (1, 0)).unwrap();
        assert_eq!(p.state, BodyState::Standing);
        assert!((p.pos[0] - 3.0).abs() < 1e-4 && (p.pos[2] - 4.0).abs() < 1e-4);
        assert_eq!(p.idle_phase, d.idle_phase);
    }

    #[test]
    fn bed_and_wardrobe_return_their_own_pose() {
        let d = d();
        let bed = occupant_pose(&d, "bed", 0, 1, 1, (0.0, 0.0), (0, 1)).unwrap();
        let wd = occupant_pose(&d, "wardrobe", 0, 1, 1, (0.0, 0.0), (0, 1)).unwrap();
        assert_eq!(bed.state, BodyState::Lying);
        assert_eq!(wd.state, BodyState::Standing);
        assert!(bed.pos[1] > wd.pos[1]);
    }

    #[test]
    fn empty_locker_draws_no_body() {
        let d = d();
        assert!(occupant_pose(&d, "wardrobe", 0, 0, 1, (0.0, 0.0), (0, 1)).is_none());
        let mut n = 0;
        draw(&d, "wardrobe", 0, 2, (0.0, 0.0), (0, 1), &mut |_, _| n += 1);
        assert_eq!(n, 0);
        draw(&d, "wardrobe", 2, 2, (0.0, 0.0), (0, 1), &mut |_, _| n += 1);
        assert_eq!(n, 2);
    }

    #[test]
    fn missing_key_reports_its_name() {
        let e = OccupantData::parse(r#"{"occupant":{"hide_size_x":0.8}}"#).unwrap_err();
        assert!(e.contains("slot_shift_factor"), "{e}");
    }
}
