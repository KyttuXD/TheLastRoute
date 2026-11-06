//! The dark two-sided shell the local hider sees from inside a locker (Godot route_interactable_3d.gd
//! _build_hide_seal: albedo (.06,.06,.07), roughness 1, cull disabled). The normal model's faces point
//! outward, so from inside they vanish; this shell sits just inside the prop bounds so nothing shows through.
//! Pure geometry: the scene draws the quads for the hidden locker only. Tunables: data/render/locker_shell.json.

use crate::content::embedded;
use std::sync::OnceLock;

/// The data file's path under data/ (the embedded file's name).
pub const PATH: &str = "render/locker_shell.json";

/// One quad, four corners in locker-local space (x across, y up, z toward the front), in loop order.
pub type Quad = [[f32; 3]; 4];

/// Prop bounds in locker-local space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShellData {
    /// Metres the shell sits inside the prop bounds.
    pub inset_m: f32,
    /// The vent slit on the front, as fractions of the bounds' height.
    pub vent_band_min: f32,
    pub vent_band_max: f32,
    pub color: [f32; 4],
    pub roughness: f32,
    pub two_sided: bool,
}

impl ShellData {
    /// Parses the file's `shell` object. A missing or malformed key is reported by name.
    pub fn parse(text: &str) -> Result<ShellData, String> {
        let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let s = root.get("shell").ok_or_else(|| "shell: missing".to_string())?;
        let num = |key: &str| {
            s.get(key)
                .and_then(|v| v.as_f64())
                .map(|v| v as f32)
                .ok_or_else(|| format!("shell.{key}: expected a number"))
        };
        // read in the file's order so the first missing key is the one reported
        let inset_m = num("inset_m")?;
        let vent_band_min = num("vent_band_min")?;
        let vent_band_max = num("vent_band_max")?;
        let color = s
            .get("color")
            .and_then(|v| v.as_array())
            .filter(|a| a.len() == 4)
            .and_then(|a| {
                let mut out = [0.0f32; 4];
                for (i, v) in a.iter().enumerate() {
                    out[i] = v.as_f64()? as f32;
                }
                Some(out)
            })
            .ok_or_else(|| "shell.color: expected four numbers".to_string())?;
        let data = ShellData {
            inset_m,
            vent_band_min,
            vent_band_max,
            color,
            roughness: num("roughness")?,
            two_sided: s
                .get("two_sided")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| "shell.two_sided: expected true or false".to_string())?,
        };
        if data.inset_m < 0.0 {
            return Err("shell.inset_m: expected zero or more".into());
        }
        if !(0.0..=1.0).contains(&data.vent_band_min)
            || !(0.0..=1.0).contains(&data.vent_band_max)
            || data.vent_band_min > data.vent_band_max
        {
            return Err("shell.vent_band_min/max: expected 0..1 with min <= max".into());
        }
        Ok(data)
    }
}

/// The shell data compiled into the executable. A file that fails to parse is reported on stderr and the Standard
/// values below are used.
pub fn data() -> &'static ShellData {
    static DATA: OnceLock<ShellData> = OnceLock::new();
    DATA.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        ShellData::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}; using the Standard shell");
            ShellData {
                inset_m: 0.02,
                vent_band_min: 0.62,
                vent_band_max: 0.78,
                color: [0.06, 0.06, 0.07, 1.0],
                roughness: 1.0,
                two_sided: true,
            }
        })
    })
}

/// The shell's quads: back, both sides, top, floor, and the front in two pieces with the vent band left open.
pub fn shell_boxes(bounds: Bounds, inset: f32, vent_min: f32, vent_max: f32) -> Vec<Quad> {
    let (x0, y0, z0) = (bounds.min[0] + inset, bounds.min[1] + inset, bounds.min[2] + inset);
    let (x1, y1, z1) = (bounds.max[0] - inset, bounds.max[1] - inset, bounds.max[2] - inset);
    let h = y1 - y0;
    let (v0, v1) = (y0 + h * vent_min, y0 + h * vent_max);
    let mut quads = vec![
        [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]], // back
        [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]], // left
        [[x1, y0, z0], [x1, y0, z1], [x1, y1, z1], [x1, y1, z0]], // right
        [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], // top
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]], // floor
    ];
    if v0 > y0 {
        quads.push([[x0, y0, z1], [x1, y0, z1], [x1, v0, z1], [x0, v0, z1]]);
    }
    if v1 < y1 {
        quads.push([[x0, v1, z1], [x1, v1, z1], [x1, y1, z1], [x0, y1, z1]]);
    }
    quads
}

/// The shell for a camera that looks out at heights `eye_lo`..`eye_hi` (metres above the locker's floor, the hiding
/// camera's vent band): the same quads as [`shell_boxes`], with the vent slit cut at those heights instead of the
/// data's fractions, so the slit sits where the camera looks whatever height the prop model has. A band the bounds do
/// not reach is clamped to them.
pub fn view_shell(bounds: Bounds, d: &ShellData, eye_lo: f32, eye_hi: f32) -> Vec<Quad> {
    // `shell_boxes` reads the band as fractions of the inset shell's height, so measure against that span
    let (y0, y1) = (bounds.min[1] + d.inset_m, bounds.max[1] - d.inset_m);
    let h = (y1 - y0).max(1e-4);
    let frac = |y: f32| ((y - y0) / h).clamp(0.0, 1.0);
    let (lo, hi) = (frac(eye_lo.min(eye_hi)), frac(eye_lo.max(eye_hi)));
    shell_boxes(bounds, d.inset_m, lo, hi)
}

/// Triangle list for the hidden locker's shell: vertices and indices (both windings when two-sided).
pub fn shell_mesh(bounds: Bounds, d: &ShellData) -> (Vec<[f32; 3]>, Vec<u16>) {
    let quads = shell_boxes(bounds, d.inset_m, d.vent_band_min, d.vent_band_max);
    let mut verts = Vec::with_capacity(quads.len() * 4);
    let mut idx = Vec::with_capacity(quads.len() * 12);
    for q in &quads {
        let base = verts.len() as u16;
        verts.extend_from_slice(q);
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        if d.two_sided {
            idx.extend_from_slice(&[base + 2, base + 1, base, base + 3, base + 2, base]);
        }
    }
    (verts, idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Bounds = Bounds { min: [-0.5, 0.0, -0.4], max: [0.5, 2.0, 0.4] };

    #[test]
    fn shell_quads_lie_inside_bounds_by_the_inset() {
        let inset = 0.05;
        for q in shell_boxes(B, inset, 0.6, 0.8) {
            for p in q {
                for a in 0..3 {
                    assert!(p[a] >= B.min[a] + inset - 1e-5 && p[a] <= B.max[a] - inset + 1e-5);
                }
            }
        }
    }

    #[test]
    fn vent_band_stays_open() {
        let quads = shell_boxes(B, 0.02, 0.6, 0.8);
        let front_z = B.max[2] - 0.02;
        let h = 2.0 - 0.04;
        let (v0, v1) = (0.02 + h * 0.6, 0.02 + h * 0.8);
        for q in quads.iter().filter(|q| q.iter().all(|p| (p[2] - front_z).abs() < 1e-5)) {
            let (lo, hi) = (q[0][1].min(q[2][1]), q[0][1].max(q[2][1]));
            assert!(hi <= v0 + 1e-4 || lo >= v1 - 1e-4, "front quad covers the vent band");
        }
        assert_eq!(quads.len(), 7);
    }

    /// A prop shorter than the one the data fractions were written for (1.9 m, like the hiding_spot model).
    const SHORT: Bounds = Bounds { min: [-0.33, 0.0, -0.4], max: [0.33, 1.9, 0.42] };

    #[test]
    fn the_view_shell_slit_covers_the_cameras_look_heights_on_any_prop_height() {
        let d = ShellData::parse(include_str!("../data/render/locker_shell.json")).unwrap();
        let (lo, hi) = (1.38, 1.72);
        for b in [B, SHORT] {
            let front_z = b.max[2] - d.inset_m;
            let fronts: Vec<Quad> = view_shell(b, &d, lo, hi)
                .into_iter()
                .filter(|q| q.iter().all(|p| (p[2] - front_z).abs() < 1e-5))
                .collect();
            assert_eq!(fronts.len(), 2, "a plate below and one above the slit");
            for y in [lo, (lo + hi) * 0.5, hi] {
                for q in &fronts {
                    let (qlo, qhi) = (q[0][1].min(q[2][1]), q[0][1].max(q[2][1]));
                    assert!(!(qlo < y - 1e-3 && qhi > y + 1e-3), "a plate covers the look height {y}");
                }
            }
        }
    }

    #[test]
    fn the_view_shell_slit_follows_the_look_heights_in_data() {
        let d = ShellData::parse(include_str!("../data/render/locker_shell.json")).unwrap();
        let top = |lo: f32, hi: f32| {
            let front_z = SHORT.max[2] - d.inset_m;
            view_shell(SHORT, &d, lo, hi)
                .into_iter()
                .filter(|q| q.iter().all(|p| (p[2] - front_z).abs() < 1e-5))
                .map(|q| q[0][1].max(q[2][1]))
                .fold(f32::MIN, f32::max)
        };
        assert!(top(1.38, 1.72) > 1.7, "the upper plate starts above the slit");
        // a slit higher than the prop leaves only the lower plate, and a reversed pair is the same slit
        assert_eq!(top(5.0, 6.0), top(6.0, 5.0));
        assert_eq!(view_shell(SHORT, &d, 1.38, 1.72), view_shell(SHORT, &d, 1.72, 1.38));
        assert_eq!(view_shell(SHORT, &d, 5.0, 6.0).len(), 6, "no plate above a slit that the prop does not reach");
    }

    #[test]
    fn standard_file_parses_and_matches_godot() {
        let d = ShellData::parse(include_str!("../data/render/locker_shell.json")).unwrap();
        assert_eq!(d.color, [0.06, 0.06, 0.07, 1.0]);
        assert_eq!(d.roughness, 1.0);
        assert!(d.two_sided);
    }

    #[test]
    fn missing_key_reports_its_name() {
        let e = ShellData::parse(r#"{"shell":{"inset_m":0.02}}"#).unwrap_err();
        assert!(e.contains("vent_band_min"), "{e}");
    }

    #[test]
    fn data_change_changes_the_mesh() {
        let mut d = ShellData::parse(include_str!("../data/render/locker_shell.json")).unwrap();
        let (_, a) = shell_mesh(B, &d);
        d.two_sided = false;
        let (_, b) = shell_mesh(B, &d);
        assert!(a.len() > b.len());
    }
}
