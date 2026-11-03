//! The left and right vent lights of Masked Hours: the hold rule (`MaskControls.press`), when a held light
//! shines (`MaskControls.control_input`), the battery gate (`MaskLogic`, mask_logic.gd:351) and where the
//! office puts the vent hatch, its station, the beam and the glint (`MaskOffice._build_vents`).

use macroquad::prelude::{Color, Vec3};

/// Half the office's width (`MaskOffice.HALF_W`); the vents sit in the side walls.
pub const HALF_W: f32 = 3.4;

/// Which side wall's vent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VentSide {
    L,
    R,
}

impl VentSide {
    pub const ALL: [VentSide; 2] = [VentSide::L, VentSide::R];

    /// -1 for the left wall, +1 for the right (`side` in `_build_vents`).
    pub fn sign(self) -> f32 {
        match self {
            VentSide::L => -1.0,
            VentSide::R => 1.0,
        }
    }

    /// The `"L"` / `"R"` key Godot uses for `vent_beams`, `lit` and `DUCT_CAM`.
    pub fn key(self) -> &'static str {
        match self {
            VentSide::L => "L",
            VentSide::R => "R",
        }
    }

    /// The control action held for this light.
    pub fn action(self) -> &'static str {
        match self {
            VentSide::L => "lightL",
            VentSide::R => "lightR",
        }
    }

    /// The duct room whose camera also lets this light work (`MaskData.DUCT_CAM`).
    pub fn duct_cam(self) -> &'static str {
        match self {
            VentSide::L => "ductL",
            VentSide::R => "ductR",
        }
    }

    /// The duct camera's number on the monitor (`MaskData.ROOMS[duct]["cam"]`).
    pub fn duct_cam_number(self) -> u8 {
        match self {
            VentSide::L => 6,
            VentSide::R => 7,
        }
    }

    /// The physical station's id in the office (`_station("ventL" / "ventR", ...)`).
    pub fn station_id(self) -> &'static str {
        match self {
            VentSide::L => "ventL",
            VentSide::R => "ventR",
        }
    }

    pub fn station_label(self) -> &'static str {
        match self {
            VentSide::L => "LEFT VENT // HOLD FOR LIGHT",
            VentSide::R => "RIGHT VENT // HOLD FOR LIGHT",
        }
    }
}

/// Which vent lights are held down (`MaskControls.held["lightL"/"lightR"]`). Only one at a time: a press
/// lets go of the other side, so the latest press wins; letting go of one never brings the other back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VentLightHeld {
    pub l: bool,
    pub r: bool,
}

/// The vent lights as the rules see them this frame (`control_input()["lightL"/"lightR"]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VentLightActive {
    pub l: bool,
    pub r: bool,
}

impl VentLightHeld {
    /// A vent light held down (`on`) or let go (`press("lightL" / "lightR", on)`).
    pub fn press(&mut self, side: VentSide, on: bool) {
        match side {
            VentSide::L => self.l = on,
            VentSide::R => self.r = on,
        }
        if on {
            match side {
                VentSide::L => self.r = false,
                VentSide::R => self.l = false,
            }
        }
    }

    pub fn held(&self, side: VentSide) -> bool {
        match side {
            VentSide::L => self.l,
            VentSide::R => self.r,
        }
    }

    /// A held light works with both panels down (`clear()`), or with the monitor fully up on that side's
    /// duct camera. `monitor_up` is `is_up("monitor")`; `cam` is the monitor's current room.
    pub fn active(&self, panels_clear: bool, monitor_up: bool, cam: &str) -> VentLightActive {
        let duct_up = |side: VentSide| monitor_up && cam == side.duct_cam();
        VentLightActive {
            l: self.l && (panels_clear || duct_up(VentSide::L)),
            r: self.r && !self.l && (panels_clear || duct_up(VentSide::R)),
        }
    }
}

impl VentLightActive {
    pub fn get(&self, side: VentSide) -> bool {
        match side {
            VentSide::L => self.l,
            VentSide::R => self.r,
        }
    }

    /// What actually shines: nothing on a flat battery, and never both (`lit` in `MaskLogic.step`).
    pub fn lit(&self, battery: f32) -> VentLightActive {
        if battery > 0.0 {
            VentLightActive {
                l: self.l,
                r: self.r && !self.l,
            }
        } else {
            VentLightActive::default()
        }
    }

    /// Whether either vent light draws on the battery this frame (part of `using`).
    pub fn any(&self) -> bool {
        self.l || self.r
    }
}

/// A vent's SpotLight3D (`VentBeamL` / `VentBeamR`), hidden until its light is lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VentBeam {
    pub position: Vec3,
    /// Where the beam points from `position`, with +Y up.
    pub look_at: Vec3,
    pub color: Color,
    pub energy: f32,
    pub range: f32,
    /// `spot_angle` in degrees (Godot's half-angle of the cone).
    pub angle_deg: f32,
}

/// Where one side's vent pieces sit in the office.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VentPlacement {
    /// The hatch prop in the wall (`VentHatch` / `VentHatchR`).
    pub hatch: Vec3,
    /// The hold-for-light station (`ventL` / `ventR`).
    pub station: Vec3,
    pub beam: VentBeam,
    /// The glowing eyes shown in a lit vent (`VentGlintL` / `VentGlintR`); its two eyes sit at
    /// `GLINT_EYE_Z` along z, each `GLINT_EYE_SIZE`.
    pub glint: Vec3,
}

pub const GLINT_EYE_Z: [f32; 2] = [-0.12, 0.12];
pub const GLINT_EYE_SIZE: Vec3 = Vec3::new(0.04, 0.04, 0.06);

impl VentSide {
    pub fn placement(self) -> VentPlacement {
        let s = self.sign();
        VentPlacement {
            hatch: Vec3::new(s * HALF_W, 0.45, 0.6),
            station: Vec3::new(s * (HALF_W - 0.2), 0.45, 0.6),
            beam: VentBeam {
                position: Vec3::new(s * 0.7, 1.5, 1.7),
                look_at: Vec3::new(s * HALF_W, 0.6, 0.6),
                color: Color::new(1.0, 0.97, 0.85, 1.0),
                energy: 6.0,
                range: 5.0,
                angle_deg: 38.0,
            },
            glint: Vec3::new(s * (HALF_W - 0.2), 0.5, 0.6),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_press_wins_and_release_does_not_restore() {
        let mut h = VentLightHeld::default();
        h.press(VentSide::L, true);
        assert!(h.l && !h.r);
        h.press(VentSide::R, true);
        assert!(!h.l && h.r);
        h.press(VentSide::R, false);
        assert_eq!(h, VentLightHeld::default());
        h.press(VentSide::R, true);
        h.press(VentSide::L, false);
        assert!(h.r, "letting go of the other side leaves this one held");
    }

    #[test]
    fn active_with_panels_clear() {
        let mut h = VentLightHeld::default();
        h.press(VentSide::L, true);
        assert_eq!(h.active(true, false, "stage"), VentLightActive { l: true, r: false });
        h.press(VentSide::R, true);
        assert_eq!(h.active(true, false, "stage"), VentLightActive { l: false, r: true });
        assert_eq!(VentLightHeld::default().active(true, false, ""), VentLightActive::default());
    }

    #[test]
    fn inactive_under_mask_or_monitor_off_duct() {
        let both = VentLightHeld { l: true, r: true };
        let right = VentLightHeld { l: false, r: true };
        // mask up or a panel moving: not clear, monitor not up
        assert_eq!(both.active(false, false, "ductL"), VentLightActive::default());
        assert_eq!(right.active(false, false, "ductR"), VentLightActive::default());
        // monitor up on another camera
        assert_eq!(both.active(false, true, "stage"), VentLightActive::default());
        assert_eq!(right.active(false, true, "music"), VentLightActive::default());
    }

    #[test]
    fn active_on_own_duct_cam_only() {
        let left = VentLightHeld { l: true, r: false };
        let right = VentLightHeld { l: false, r: true };
        assert!(left.active(false, true, "ductL").l);
        assert!(!left.active(false, true, "ductR").l);
        assert!(right.active(false, true, "ductR").r);
        assert!(!right.active(false, true, "ductL").r);
    }

    #[test]
    fn right_suppressed_while_left_held() {
        let both = VentLightHeld { l: true, r: true };
        assert_eq!(both.active(true, false, ""), VentLightActive { l: true, r: false });
        // on the right duct cam the left light is off, but holding it still blocks the right
        assert_eq!(both.active(false, true, "ductR"), VentLightActive::default());
    }

    #[test]
    fn lit_needs_battery_and_never_both() {
        let a = VentLightActive { l: true, r: true };
        assert_eq!(a.lit(1.0), VentLightActive { l: true, r: false });
        assert_eq!(a.lit(0.0), VentLightActive::default());
        assert!(VentLightActive { l: false, r: true }.lit(0.5).r);
        assert!(!a.lit(0.0).any());
    }

    #[test]
    fn duct_cams_match_mask_data() {
        assert_eq!(VentSide::L.duct_cam(), "ductL");
        assert_eq!(VentSide::R.duct_cam(), "ductR");
        assert_eq!(VentSide::L.duct_cam_number(), 6);
        assert_eq!(VentSide::R.duct_cam_number(), 7);
        assert_eq!(VentSide::L.station_id(), "ventL");
        assert_eq!(VentSide::R.station_id(), "ventR");
        assert_eq!(VentSide::L.action(), "lightL");
        assert_eq!(VentSide::R.key(), "R");
    }

    #[test]
    fn placement_matches_godot() {
        let eq = |a: Vec3, b: Vec3| assert!((a - b).length() < 1e-5, "{a:?} != {b:?}");
        let l = VentSide::L.placement();
        let r = VentSide::R.placement();
        eq(l.hatch, Vec3::new(-3.4, 0.45, 0.6));
        eq(r.hatch, Vec3::new(3.4, 0.45, 0.6));
        eq(l.station, Vec3::new(-3.2, 0.45, 0.6));
        eq(r.station, Vec3::new(3.2, 0.45, 0.6));
        eq(l.beam.position, Vec3::new(-0.7, 1.5, 1.7));
        eq(r.beam.position, Vec3::new(0.7, 1.5, 1.7));
        eq(l.beam.look_at, Vec3::new(-3.4, 0.6, 0.6));
        eq(r.beam.look_at, Vec3::new(3.4, 0.6, 0.6));
        eq(l.glint, Vec3::new(-3.2, 0.5, 0.6));
        eq(r.glint, Vec3::new(3.2, 0.5, 0.6));
        for p in [l, r] {
            assert_eq!(p.beam.color, Color::new(1.0, 0.97, 0.85, 1.0));
            assert_eq!(p.beam.energy, 6.0);
            assert_eq!(p.beam.range, 5.0);
            assert_eq!(p.beam.angle_deg, 38.0);
        }
    }
}
