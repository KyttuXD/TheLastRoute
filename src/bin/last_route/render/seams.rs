//! Where two storeys meet. A level's ceiling used to sit at `wall_h` = the storey, exactly on the floor of the level
//! above (the neighbour is baked at dy = STOREY), so both quads z-fought wherever both levels were drawn. Godot keeps a
//! slab between them (SLAB .1, CEILING_INSET .02, `streamed_facility_view_3d.gd:40-44`); this module is that rule for
//! us. The slab thickness is `flights().slab` (data/flights.json, the one slab number); the ceiling inset and the
//! camera planes are data/render/seams.json. A level's walls end at [`wall_top`], its ceiling quad is drawn at
//! [`ceiling_y`], and an opening through the slab (a stairwell, a ladder shaft, the falls void) is lined from the
//! ceiling up to the storey, so nothing of one level shares a plane with anything of the next.

use crate::content::embedded;
use crate::world::flight::flights;
use crate::world::vertical::STOREY;
use std::sync::OnceLock;

pub const PATH: &str = "render/seams.json";

/// The seam tunables (data/render/seams.json).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seams {
    /// How far under the slab's underside the ceiling quad is drawn (metres).
    pub ceiling_inset: f32,
    pub camera_near: f32,
    pub camera_far: f32,
    /// Depth buffer bits of the worst-case path (GL render target: 16).
    pub depth_bits: u32,
    /// The farthest distance a seam must stay resolvable, metres.
    pub seam_range_m: f32,
}

impl Seams {
    /// Parses the file; every number must be positive, the planes ordered and the depth a real buffer width.
    pub fn parse(text: &str) -> Result<Seams, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v.get("schema").and_then(|s| s.as_str()) != Some("lr.seams/1") {
            return Err("schema: expected lr.seams/1".into());
        }
        let num = |key: &str| -> Result<f32, String> {
            v.get(key)
                .and_then(|x| x.as_f64())
                .map(|x| x as f32)
                .filter(|x| x.is_finite() && *x > 0.0)
                .ok_or_else(|| format!("{key}: expected a positive number"))
        };
        let s = Seams {
            ceiling_inset: num("ceiling_inset")?,
            camera_near: num("camera_near")?,
            camera_far: num("camera_far")?,
            depth_bits: num("depth_bits")? as u32,
            seam_range_m: num("seam_range_m")?,
        };
        if s.ceiling_inset > 0.25 {
            return Err("ceiling_inset: at most 0.25 m".into());
        }
        if s.camera_near >= 1.0 || s.camera_far <= s.camera_near * 100.0 {
            return Err("camera_near must be under 1 m and camera_far over 100 times it".into());
        }
        if ![16, 24, 32].contains(&s.depth_bits) {
            return Err("depth_bits: 16, 24 or 32".into());
        }
        Ok(s)
    }
}

/// The seam tunables compiled in. A bad file is reported on stderr and the built-in copy is used.
pub fn seams() -> &'static Seams {
    static CFG: OnceLock<Seams> = OnceLock::new();
    CFG.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        Seams::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            Seams::parse(include_str!("../data/render/seams.json")).expect("seams.json is valid")
        })
    })
}

/// The slab thickness between a level's ceiling and the floor above (`data/flights.json` `slab`).
pub fn slab() -> f32 {
    flights().slab
}

/// The tallest a wall may stand: the region's wall height capped under the slab (Godot `LEVEL_HEIGHT - SLAB`).
pub fn wall_top(wall_h: f32) -> f32 {
    wall_h.min(STOREY - slab())
}

/// Where a cell's ceiling quad is drawn: its (capped) wall top, inset so it shares no plane with the slab.
pub fn ceiling_y(wall_h: f32) -> f32 {
    wall_top(wall_h) - seams().ceiling_inset
}

/// The air gap between the tallest ceiling and the floor of the next level.
pub fn seam_gap() -> f32 {
    STOREY - ceiling_y(STOREY)
}

/// Where the lining of an opening through the slab ends: the underside of the floor above (the storey).
pub fn shaft_top() -> f32 {
    STOREY
}

/// One step of the depth buffer, in metres, at distance `d` (a perspective buffer with far much larger than d).
pub fn depth_resolution(d: f32) -> f32 {
    let s = seams();
    d * d / (s.camera_near * (1u64 << s.depth_bits) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_parses_and_the_slab_is_the_flights_one() {
        let s = Seams::parse(include_str!("../data/render/seams.json")).unwrap();
        assert_eq!(&s, seams());
        assert!(embedded::data_under(PATH).next().is_some(), "seams.json is embedded");
        assert_eq!(slab(), flights().slab, "one slab number");
    }

    #[test]
    fn bad_files_are_refused() {
        let good = include_str!("../data/render/seams.json");
        for (from, to) in [
            ("\"ceiling_inset\": 0.05", "\"ceiling_inset\": 0.0"),
            ("\"ceiling_inset\": 0.05", "\"ceiling_inset\": 0.5"),
            ("\"camera_near\": 0.06", "\"camera_near\": 2.0"),
            ("\"camera_far\": 80.0", "\"camera_far\": 5.0"),
            ("\"depth_bits\": 16", "\"depth_bits\": 20"),
            ("\"seam_range_m\": 24.0", "\"seam_range_m\": -1.0"),
            ("lr.seams/1", "lr.seams/2"),
        ] {
            assert!(Seams::parse(&good.replace(from, to)).is_err(), "{from} -> {to}");
        }
    }

    #[test]
    fn the_cap_sits_under_the_slab_and_the_ceiling_under_the_cap() {
        assert!((wall_top(STOREY) - (STOREY - slab())).abs() < 1e-6);
        assert_eq!(wall_top(3.0), 3.0, "a lower region ceiling is left alone");
        assert_eq!(wall_top(STOREY + 7.0), wall_top(STOREY), "a taller one is clamped");
        assert!((ceiling_y(STOREY) - (wall_top(STOREY) - seams().ceiling_inset)).abs() < 1e-6);
        assert!((seam_gap() - (slab() + seams().ceiling_inset)).abs() < 1e-5);
        assert_eq!(shaft_top(), STOREY);
    }

    /// The seam must be at least one depth step wide at the fog range, in the worst buffer (16 bit), or the two
    /// storeys z-fight again however clean the geometry is. Changing a plane or the slab cannot silently break it.
    #[test]
    fn the_seam_is_wider_than_a_depth_step_at_the_fog_range() {
        let s = seams();
        assert!(
            seam_gap() >= depth_resolution(s.seam_range_m),
            "gap {} m < one {}-bit step {} m at {} m",
            seam_gap(),
            s.depth_bits,
            depth_resolution(s.seam_range_m),
            s.seam_range_m
        );
        assert!(s.camera_far > s.seam_range_m, "the far plane clears the fog range");
    }

    #[test]
    fn the_near_plane_is_godots_first_person_near() {
        assert!((seams().camera_near - 0.06).abs() < 1e-6);
    }
}
