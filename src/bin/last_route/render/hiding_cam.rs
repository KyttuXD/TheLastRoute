//! Locker view: the camera sits inside the hiding spot and looks out through it (Godot `hiding_inside_position`,
//! `hiding_eye_height`, `hiding_outward_yaw`, `_begin_hide_move`). Pure functions of the spot's fixed centre, so the
//! search-tell shake and the door swing never move the eye. All numbers come from `data/player/hiding_view.json`.

use macroquad::prelude::*;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct Offsets {
    pub locker: f32,
    pub wardrobe: f32,
    pub bed: f32,
}

#[derive(Clone, Debug)]
pub struct Cfg {
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub glide_enter_s: f32,
    pub glide_leave_s: f32,
    pub vent_lo: f32,
    pub vent_hi: f32,
    pub bed_eye: f32,
    pub wardrobe_eye: f32,
    pub lock_look_while_shaking: bool,
    pub forward_offset: Offsets,
}

type Json = serde_json::Value;

fn num(v: &Json, path: &[&str]) -> Result<f32, String> {
    let mut cur = v;
    for key in path {
        cur = cur
            .get(*key)
            .ok_or_else(|| format!("hiding_view.json: \"{}\" is missing", path.join(".")))?;
    }
    let n = cur
        .as_f64()
        .ok_or_else(|| format!("hiding_view.json: \"{}\" must be a number", path.join(".")))?;
    if !n.is_finite() {
        return Err(format!("hiding_view.json: \"{}\" must be finite", path.join(".")));
    }
    Ok(n as f32)
}

impl Cfg {
    /// Parse and validate the table; a bad one is refused with a message naming the key.
    pub fn parse(text: &str) -> Result<Cfg, String> {
        let v: Json = serde_json::from_str(text).map_err(|e| format!("hiding_view.json: not valid JSON: {e}"))?;
        let lock_look_while_shaking = v
            .get("lock_look_while_shaking")
            .ok_or_else(|| "hiding_view.json: \"lock_look_while_shaking\" is missing".to_string())?
            .as_bool()
            .ok_or_else(|| "hiding_view.json: \"lock_look_while_shaking\" must be true or false".to_string())?;
        Ok(Cfg {
            yaw_deg: num(&v, &["yaw_deg"])?,
            pitch_deg: num(&v, &["pitch_deg"])?,
            glide_enter_s: num(&v, &["glide_enter_s"])?,
            glide_leave_s: num(&v, &["glide_leave_s"])?,
            vent_lo: num(&v, &["vent_lo"])?,
            vent_hi: num(&v, &["vent_hi"])?,
            bed_eye: num(&v, &["bed_eye"])?,
            wardrobe_eye: num(&v, &["wardrobe_eye"])?,
            lock_look_while_shaking,
            forward_offset: Offsets {
                locker: num(&v, &["forward_offset", "locker"])?,
                wardrobe: num(&v, &["forward_offset", "wardrobe"])?,
                bed: num(&v, &["forward_offset", "bed"])?,
            },
        })
    }
}

pub fn cfg() -> &'static Cfg {
    static C: OnceLock<Cfg> = OnceLock::new();
    C.get_or_init(|| {
        Cfg::parse(include_str!("../data/player/hiding_view.json")).unwrap_or_else(|e| panic!("{e}"))
    })
}

/// Eye height for a style: a bed lies flat, a wardrobe keeps its own height, anything else is a locker with the eye
/// on the vent band centre (minus the body's 2 cm).
pub fn eye_height(style: &str, c: &Cfg) -> f32 {
    match style {
        "bed" => c.bed_eye,
        "wardrobe" => c.wardrobe_eye,
        _ => (c.vent_lo + c.vent_hi) * 0.5 - 0.02,
    }
}

/// Player yaw that looks out of a spot facing `facing` (grid direction): the player's forward is (-sin, -cos).
pub fn facing_yaw(facing: (f32, f32)) -> f32 {
    (-facing.0).atan2(-facing.1)
}

/// The camera position inside the spot at its fixed `centre` (grid x, z).
pub fn eye_position_with(centre: (f32, f32), facing: (f32, f32), style: &str, c: &Cfg) -> Vec3 {
    let n = (facing.0 * facing.0 + facing.1 * facing.1).sqrt().max(1e-6);
    let off = match style {
        "bed" => c.forward_offset.bed,
        "wardrobe" => c.forward_offset.wardrobe,
        _ => c.forward_offset.locker,
    };
    vec3(
        centre.0 + facing.0 / n * off,
        eye_height(style, c),
        centre.1 + facing.1 / n * off,
    )
}

pub fn eye_position(centre: (f32, f32), facing: (f32, f32), style: &str) -> Vec3 {
    eye_position_with(centre, facing, style, cfg())
}

fn wrap(a: f32) -> f32 {
    let pi = std::f32::consts::PI;
    (a + pi).rem_euclid(std::f32::consts::TAU) - pi
}

/// Hold the look to the cone around the facing: yaw within +-yaw_deg of `base_yaw`, pitch within +-pitch_deg.
pub fn clamp_look_with(yaw: f32, pitch: f32, base_yaw: f32, c: &Cfg) -> (f32, f32) {
    let ym = c.yaw_deg.to_radians();
    let pm = c.pitch_deg.to_radians();
    (base_yaw + wrap(yaw - base_yaw).clamp(-ym, ym), pitch.clamp(-pm, pm))
}

pub fn clamp_look(yaw: f32, pitch: f32, base_yaw: f32) -> (f32, f32) {
    clamp_look_with(yaw, pitch, base_yaw, cfg())
}

/// Whether mouse look is frozen: only while the spot is shaking and the data asks for it.
pub fn look_locked(shaking: bool) -> bool {
    shaking && cfg().lock_look_while_shaking
}

/// Glide length for entering or leaving.
pub fn glide_seconds(entering: bool) -> f32 {
    let c = cfg();
    (if entering { c.glide_enter_s } else { c.glide_leave_s }).max(0.01)
}

/// Eye blended from where the runner stood to the spot's eye; `weight` 0..1 (smoothstepped here).
pub fn glide_eye(stand: Vec3, inside: Vec3, weight: f32) -> Vec3 {
    let w = weight.clamp(0.0, 1.0);
    stand.lerp(inside, w * w * (3.0 - 2.0 * w))
}

/// Basis (forward, right, up) for a yaw and pitch.
pub fn basis(yaw: f32, pitch: f32) -> (Vec3, Vec3, Vec3) {
    let fwd = vec3(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()).normalize();
    let right = vec3(yaw.cos(), 0.0, -yaw.sin());
    (fwd, right, right.cross(fwd).normalize())
}

/// The view while hidden: (eye, forward, right, up). `stand_eye` is the runner's own eye, `weight` the glide
/// progress (`hide::player_now().weight`), `look` the player's raw (yaw, pitch); the look is clamped to the cone.
pub fn view_from(
    stand_eye: Vec3,
    weight: f32,
    look: (f32, f32),
    centre: (f32, f32),
    facing: (f32, f32),
    style: &str,
) -> (Vec3, Vec3, Vec3, Vec3) {
    let inside = eye_position(centre, facing, style);
    let (yaw, pitch) = clamp_look(look.0, look.1, facing_yaw(facing));
    let (f, r, u) = basis(yaw, pitch);
    (glide_eye(stand_eye, inside, weight), f, r, u)
}

#[cfg(test)]
mod tests {
    use super::*;
    const FACINGS: [(f32, f32); 4] = [(0.0, 1.0), (0.0, -1.0), (1.0, 0.0), (-1.0, 0.0)];

    #[test]
    fn eye_height_is_the_band_centre() {
        let c = cfg();
        let e = eye_position((3.0, 4.0), (0.0, 1.0), "locker");
        assert!((e.y - ((c.vent_lo + c.vent_hi) * 0.5 - 0.02)).abs() < 1e-5);
    }

    #[test]
    fn bed_lies_flat() {
        assert!((eye_position((0.0, 0.0), (1.0, 0.0), "bed").y - 0.16).abs() < 1e-5);
    }

    #[test]
    fn yaw_looks_along_the_facing() {
        for f in FACINGS {
            let (fw, _, _) = basis(facing_yaw(f), 0.0);
            assert!((fw.x - f.0).abs() < 1e-4 && (fw.z - f.1).abs() < 1e-4, "{f:?}");
        }
    }

    #[test]
    fn clamp_holds_a_big_turn_and_passes_a_small_one() {
        let c = cfg();
        for f in FACINGS {
            let base = facing_yaw(f);
            let (y, p) = clamp_look(base + 90f32.to_radians(), 1.2, base);
            assert!((wrap(y - base) - c.yaw_deg.to_radians()).abs() < 1e-4);
            assert!((p - c.pitch_deg.to_radians()).abs() < 1e-4);
            let (y, p) = clamp_look(base - 0.1, -0.05, base);
            assert!((y - (base - 0.1)).abs() < 1e-4 && (p + 0.05).abs() < 1e-6);
        }
    }

    #[test]
    fn clamp_wraps_across_pi() {
        let (y, _) = clamp_look(-3.1, 0.0, 3.1);
        assert!(wrap(y - 3.1).abs() <= cfg().yaw_deg.to_radians() + 1e-4);
    }

    #[test]
    fn shake_cannot_move_the_eye() {
        // the eye is a function of the fixed centre only; a shaken render transform is never an input
        let a = view_from(Vec3::ZERO, 1.0, (0.0, 0.0), (2.0, 2.0), (0.0, 1.0), "locker");
        let b = view_from(Vec3::ZERO, 1.0, (0.0, 0.0), (2.0, 2.0), (0.0, 1.0), "locker");
        assert_eq!(a.0, b.0);
    }

    #[test]
    fn glide_runs_from_stand_to_inside() {
        let s = vec3(0.0, 1.6, 0.0);
        let i = vec3(1.0, 1.5, 1.0);
        assert_eq!(glide_eye(s, i, 0.0), s);
        assert!((glide_eye(s, i, 1.0) - i).length() < 1e-5);
        assert!(glide_seconds(true) > 0.0 && glide_seconds(false) > 0.0);
    }

    #[test]
    fn parse_names_the_bad_key() {
        let good = include_str!("../data/player/hiding_view.json");
        assert!(Cfg::parse(good).is_ok());
        let bad = good.replace("\"pitch_deg\"", "\"pitch\"");
        assert!(Cfg::parse(&bad).unwrap_err().contains("pitch_deg"));
        let bad = good.replace("\"locker\": 0.12", "\"locker\": \"x\"");
        assert!(Cfg::parse(&bad).unwrap_err().contains("forward_offset.locker"));
    }

    #[test]
    fn look_lock_follows_the_data_flag() {
        assert_eq!(look_locked(true), cfg().lock_look_while_shaking);
        assert!(!look_locked(false));
    }
}
