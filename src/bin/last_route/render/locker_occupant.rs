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

// -- the hunter checking a locker against the occupant (TODO 374) --------------------------------------------------------
//
// Simulating side only: the brain's locker check runs `LockerCheck` and the scene draws the result for everyone (a guest
// gets the phase and progress through the hunter's frame). Timings are data/hiding.json `hunter_anim`.

/// The steps of a hunter checking a locker, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckPhase {
    /// The search tell: the hunter stops and listens at the door.
    Tell,
    /// The door is opened.
    Open,
    /// The arm reaches in toward the occupant.
    ReachIn,
    /// The occupant is pulled out (only when someone is inside).
    PullOut,
    /// Done.
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckTiming {
    pub tell: f32,
    pub open: f32,
    pub reach_in: f32,
    pub pull_out: f32,
    /// How high the hand goes on the occupant, metres.
    pub reach_height: f32,
    /// How far the occupant is dragged out, metres.
    pub pull_distance: f32,
}

impl CheckTiming {
    pub const STANDARD: CheckTiming =
        CheckTiming { tell: 0.6, open: 0.5, reach_in: 0.6, pull_out: 0.8, reach_height: 1.1, pull_distance: 1.2 };

    /// Parses data/hiding.json's `hunter_anim` object; a missing value keeps the Standard one.
    pub fn parse(text: &str) -> CheckTiming {
        let mut t = CheckTiming::STANDARD;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return t };
        let a = &v["hunter_anim"];
        let f = |k: &str, into: &mut f32| {
            if let Some(x) = a[k].as_f64().filter(|x| *x >= 0.0 && *x < 60.0) {
                *into = x as f32;
            }
        };
        f("tell", &mut t.tell);
        f("open", &mut t.open);
        f("reach_in", &mut t.reach_in);
        f("pull_out", &mut t.pull_out);
        f("reach_height", &mut t.reach_height);
        f("pull_distance", &mut t.pull_distance);
        t
    }

    /// Seconds to the end of the check; the pull-out only counts when someone is inside.
    pub fn total(&self, occupied: bool) -> f32 {
        self.tell + self.open + self.reach_in + if occupied { self.pull_out } else { 0.0 }
    }
}

/// The embedded timing.
pub fn check_timing() -> &'static CheckTiming {
    static T: OnceLock<CheckTiming> = OnceLock::new();
    T.get_or_init(|| CheckTiming::parse(include_str!("../data/hiding.json")))
}

/// The phase `t` seconds into a check and its progress 0..1. An empty locker has no pull-out: it finishes after the reach.
pub fn check_phase(timing: &CheckTiming, t: f32, occupied: bool) -> (CheckPhase, f32) {
    let steps = [
        (CheckPhase::Tell, timing.tell),
        (CheckPhase::Open, timing.open),
        (CheckPhase::ReachIn, timing.reach_in),
        (CheckPhase::PullOut, if occupied { timing.pull_out } else { 0.0 }),
    ];
    let mut left = t.max(0.0);
    for (phase, len) in steps {
        if left < len {
            return (phase, if len > 0.0 { left / len } else { 1.0 });
        }
        left -= len;
    }
    (CheckPhase::Finished, 1.0)
}

/// Where the reaching hand goes: the occupant's chest (their feet plus the reach height). None for an empty slot.
pub fn reach_target(timing: &CheckTiming, pose: Option<&OccupantPose>) -> Option<[f32; 3]> {
    let p = pose?;
    let h = if p.state == BodyState::Lying { timing.reach_height * 0.4 } else { timing.reach_height };
    Some([p.pos[0], p.pos[1] + h, p.pos[2]])
}

/// The occupant's position while being pulled out: from its slot along the locker's facing by `progress` of the pull
/// distance. The yaw turns toward the hunter by the same share (a half turn at the end).
pub fn pulled_pose(timing: &CheckTiming, pose: &OccupantPose, facing: (i32, i32), progress: f32) -> OccupantPose {
    let (fx, fz) = (facing.0 as f32, facing.1 as f32);
    let len = (fx * fx + fz * fz).sqrt().max(1e-6);
    let s = progress.clamp(0.0, 1.0);
    let mut out = *pose;
    out.pos[0] += fx / len * timing.pull_distance * s;
    out.pos[2] += fz / len * timing.pull_distance * s;
    out.yaw += std::f32::consts::PI * s;
    out
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

    #[test]
    fn the_hunter_check_steps_run_tell_open_reach_pull() {
        let t = CheckTiming::parse(include_str!("../data/hiding.json"));
        assert_eq!(t, CheckTiming::STANDARD);
        assert_eq!(check_phase(&t, 0.0, true).0, CheckPhase::Tell);
        assert_eq!(check_phase(&t, t.tell + 0.01, true).0, CheckPhase::Open);
        assert_eq!(check_phase(&t, t.tell + t.open + 0.01, true).0, CheckPhase::ReachIn);
        assert_eq!(check_phase(&t, t.tell + t.open + t.reach_in + 0.01, true).0, CheckPhase::PullOut);
        assert_eq!(check_phase(&t, t.total(true) + 0.01, true).0, CheckPhase::Finished);
        let (_, p) = check_phase(&t, t.tell / 2.0, true);
        assert!((p - 0.5).abs() < 1e-4);
    }

    #[test]
    fn an_empty_locker_has_no_pull_out() {
        let t = CheckTiming::STANDARD;
        assert!(t.total(false) < t.total(true));
        assert_eq!(check_phase(&t, t.tell + t.open + t.reach_in + 0.01, false).0, CheckPhase::Finished);
    }

    #[test]
    fn the_hand_reaches_the_occupant_and_the_pull_moves_them_out_of_the_door() {
        let d = d();
        let t = CheckTiming::STANDARD;
        let pose = occupant_pose(&d, "wardrobe", 0, 1, 1, (3.0, 4.0), (0, 1)).unwrap();
        let hand = reach_target(&t, Some(&pose)).unwrap();
        assert!((hand[0] - pose.pos[0]).abs() < 1e-5 && (hand[2] - pose.pos[2]).abs() < 1e-5);
        assert!((hand[1] - (pose.pos[1] + t.reach_height)).abs() < 1e-5);
        assert_eq!(reach_target(&t, None), None, "nobody inside: nothing to reach for");
        let bed = occupant_pose(&d, "bed", 0, 1, 1, (0.0, 0.0), (0, 1)).unwrap();
        assert!(reach_target(&t, Some(&bed)).unwrap()[1] < hand[1] + bed.pos[1], "a lying body is reached lower");
        let out = pulled_pose(&t, &pose, (0, 1), 1.0);
        assert!((out.pos[2] - pose.pos[2] - t.pull_distance).abs() < 1e-4, "dragged out along the facing");
        assert_eq!(pulled_pose(&t, &pose, (0, 1), 0.0), pose);
        let fast = CheckTiming::parse(r#"{"hunter_anim":{"pull_distance":2.0,"tell":0.1}}"#);
        assert_eq!(fast.pull_distance, 2.0);
        assert_eq!(fast.tell, 0.1);
        assert_eq!(fast.open, t.open);
    }
}
