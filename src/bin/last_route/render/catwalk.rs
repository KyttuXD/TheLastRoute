//! Catwalk visuals (Godot's Mezzanine decks): a see-through grating floor of slats in a frame, railings on the
//! open sides, support beams under the deck with posts down to the ground, and a lamp every few cells. Every
//! size and colour is data/render/catwalk.json. This module only builds boxes (local to the cell centre, y up,
//! y = 0 at the deck's walking surface); drawing them is hooked in by the scene.

use crate::content::embedded;
use std::sync::OnceLock;

pub const PATH: &str = "render/catwalk.json";

#[derive(Clone, Debug, PartialEq)]
pub struct CatwalkCfg {
    pub slat_width_m: f32,
    pub slat_gap_m: f32,
    pub slat_thickness_m: f32,
    pub frame_width_m: f32,
    pub grating_color: [f32; 3],
    pub frame_color: [f32; 3],
    pub rail_height_m: f32,
    pub rail_post_size_m: f32,
    pub rail_post_spacing_m: f32,
    pub rail_size_m: f32,
    pub mid_rail_fraction: f32,
    pub kick_plate_height_m: f32,
    pub rail_color: [f32; 3],
    pub beam_height_m: f32,
    pub beam_width_m: f32,
    pub beam_drop_m: f32,
    pub support_post_size_m: f32,
    pub post_every_cells: i32,
    pub support_color: [f32; 3],
    pub light_every_cells: i32,
    pub light_height_m: f32,
    pub light_color: [f32; 3],
    pub light_energy: f32,
    pub light_range_m: f32,
}

fn sect<'a>(v: &'a serde_json::Value, key: &str) -> Result<&'a serde_json::Value, String> {
    v.get(key).ok_or_else(|| format!("{key}: missing"))
}

fn num(v: &serde_json::Value, key: &str) -> Result<f32, String> {
    v.get(key)
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .ok_or_else(|| format!("{key}: expected a number"))
}

fn rgb(v: &serde_json::Value, key: &str) -> Result<[f32; 3], String> {
    let a = v
        .get(key)
        .and_then(|x| x.as_array())
        .filter(|a| a.len() == 3)
        .ok_or_else(|| format!("{key}: expected 3 numbers"))?;
    let mut out = [0.0; 3];
    for (i, x) in a.iter().enumerate() {
        out[i] = x.as_f64().ok_or_else(|| format!("{key}: expected numbers"))? as f32;
    }
    Ok(out)
}

impl CatwalkCfg {
    pub fn parse(text: &str) -> Result<CatwalkCfg, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let (g, r, s, l) = (sect(&v, "grating")?, sect(&v, "railing")?, sect(&v, "support")?, sect(&v, "light")?);
        let cfg = CatwalkCfg {
            slat_width_m: num(g, "slat_width_m")?,
            slat_gap_m: num(g, "slat_gap_m")?,
            slat_thickness_m: num(g, "slat_thickness_m")?,
            frame_width_m: num(g, "frame_width_m")?,
            grating_color: rgb(g, "color")?,
            frame_color: rgb(g, "frame_color")?,
            rail_height_m: num(r, "height_m")?,
            rail_post_size_m: num(r, "post_size_m")?,
            rail_post_spacing_m: num(r, "post_spacing_m")?,
            rail_size_m: num(r, "rail_size_m")?,
            mid_rail_fraction: num(r, "mid_rail_fraction")?,
            kick_plate_height_m: num(r, "kick_plate_height_m")?,
            rail_color: rgb(r, "color")?,
            beam_height_m: num(s, "beam_height_m")?,
            beam_width_m: num(s, "beam_width_m")?,
            beam_drop_m: num(s, "beam_drop_m")?,
            support_post_size_m: num(s, "post_size_m")?,
            post_every_cells: num(s, "post_every_cells")? as i32,
            support_color: rgb(s, "color")?,
            light_every_cells: num(l, "every_cells")? as i32,
            light_height_m: num(l, "height_m")?,
            light_color: rgb(l, "color")?,
            light_energy: num(l, "energy")?,
            light_range_m: num(l, "range_m")?,
        };
        if cfg.slat_width_m <= 0.0
            || cfg.slat_gap_m <= 0.0
            || cfg.rail_post_spacing_m <= 0.0
            || cfg.post_every_cells < 1
            || cfg.light_every_cells < 1
        {
            return Err("slat width, gap, post spacing and the every-N-cells values must be positive".into());
        }
        Ok(cfg)
    }
}

/// The catwalk tunables compiled in. A bad file is reported on stderr and the Standard values are used.
pub fn config() -> &'static CatwalkCfg {
    static CFG: OnceLock<CatwalkCfg> = OnceLock::new();
    CFG.get_or_init(|| {
        let text = embedded::data_under(PATH).next().map(|(_, t)| t).unwrap_or("{}");
        CatwalkCfg::parse(text).unwrap_or_else(|e| {
            eprintln!("{PATH}: {e}");
            CatwalkCfg::parse(include_str!("../data/render/catwalk.json")).expect("catwalk.json is valid")
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Slat,
    Frame,
    RailPost,
    Rail,
    KickPlate,
    Beam,
    SupportPost,
}

/// One axis-aligned box: centre and full size in metres, local to the cell centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Part {
    pub kind: PartKind,
    pub center: [f32; 3],
    pub size: [f32; 3],
    pub color: [f32; 3],
}

/// Which sides of the deck cell have a railing (no deck neighbour there).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenSides {
    pub north: bool,
    pub south: bool,
    pub west: bool,
    pub east: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub energy: f32,
    pub range_m: f32,
}

fn part(kind: PartKind, center: [f32; 3], size: [f32; 3], color: [f32; 3]) -> Part {
    Part { kind, center, size, color }
}

/// The grating: slats along z with gaps between (the gaps are what you see through), between two frame bars.
pub fn grating(cfg: &CatwalkCfg, cell_m: f32) -> Vec<Part> {
    let h = cell_m * 0.5;
    let t = cfg.slat_thickness_m;
    let fw = cfg.frame_width_m;
    let y = -t * 0.5;
    let mut out = vec![
        part(PartKind::Frame, [0.0, y, -h + fw * 0.5], [cell_m, t, fw], cfg.frame_color),
        part(PartKind::Frame, [0.0, y, h - fw * 0.5], [cell_m, t, fw], cfg.frame_color),
    ];
    let inner = cell_m - 2.0 * fw;
    let pitch = cfg.slat_width_m + cfg.slat_gap_m;
    let n = ((cell_m + cfg.slat_gap_m) / pitch).floor().max(0.0) as usize;
    let used = n as f32 * pitch - cfg.slat_gap_m;
    let start = -used * 0.5 + cfg.slat_width_m * 0.5;
    for i in 0..n {
        let x = start + i as f32 * pitch;
        out.push(part(PartKind::Slat, [x, y, 0.0], [cfg.slat_width_m, t, inner], cfg.grating_color));
    }
    out
}

/// Railings on the open sides: posts every `post_spacing_m`, a top rail, a mid rail and a low kick plate.
pub fn railings(cfg: &CatwalkCfg, cell_m: f32, open: OpenSides) -> Vec<Part> {
    let h = cell_m * 0.5;
    let mut out = Vec::new();
    let sides = [(open.north, true, -h), (open.south, true, h), (open.west, false, -h), (open.east, false, h)];
    for (on, along_x, fixed) in sides {
        if !on {
            continue;
        }
        let pos = |a: f32, y: f32| if along_x { [a, y, fixed] } else { [fixed, y, a] };
        let size = |len: f32, thick: f32, hgt: f32| if along_x { [len, hgt, thick] } else { [thick, hgt, len] };
        let segments = (cell_m / cfg.rail_post_spacing_m).ceil().max(1.0) as usize;
        for i in 0..=segments {
            let a = -h + cell_m * i as f32 / segments as f32;
            out.push(part(
                PartKind::RailPost,
                pos(a, cfg.rail_height_m * 0.5),
                [cfg.rail_post_size_m, cfg.rail_height_m, cfg.rail_post_size_m],
                cfg.rail_color,
            ));
        }
        let r = cfg.rail_size_m;
        out.push(part(PartKind::Rail, pos(0.0, cfg.rail_height_m - r * 0.5), size(cell_m, r, r), cfg.rail_color));
        out.push(part(
            PartKind::Rail,
            pos(0.0, cfg.rail_height_m * cfg.mid_rail_fraction),
            size(cell_m, r, r),
            cfg.rail_color,
        ));
        if cfg.kick_plate_height_m > 0.0 {
            out.push(part(
                PartKind::KickPlate,
                pos(0.0, cfg.kick_plate_height_m * 0.5),
                size(cell_m, r, cfg.kick_plate_height_m),
                cfg.rail_color,
            ));
        }
    }
    out
}

/// Beams under the grating along both edges, and a post down to the ground on every `post_every_cells`-th
/// cell (by cell coordinates), `deck_height_m` tall.
pub fn supports(cfg: &CatwalkCfg, cell_m: f32, deck_height_m: f32, cell: (i32, i32)) -> Vec<Part> {
    let h = cell_m * 0.5;
    let top = -cfg.slat_thickness_m - cfg.beam_drop_m;
    let by = top - cfg.beam_height_m * 0.5;
    let bs = [cell_m, cfg.beam_height_m, cfg.beam_width_m];
    let mut out = vec![
        part(PartKind::Beam, [0.0, by, -h + cfg.beam_width_m * 0.5], bs, cfg.support_color),
        part(PartKind::Beam, [0.0, by, h - cfg.beam_width_m * 0.5], bs, cfg.support_color),
    ];
    if cell.0.rem_euclid(cfg.post_every_cells) == 0 && cell.1.rem_euclid(cfg.post_every_cells) == 0 {
        let s = cfg.support_post_size_m;
        let len = deck_height_m + top;
        if len > 0.0 {
            out.push(part(PartKind::SupportPost, [0.0, top - len * 0.5, 0.0], [s, len, s], cfg.support_color));
        }
    }
    out
}

/// The lamp for this cell, if it carries one (every `light_every_cells`-th cell in both axes).
pub fn lamp(cfg: &CatwalkCfg, cell: (i32, i32)) -> Option<Lamp> {
    (cell.0.rem_euclid(cfg.light_every_cells) == 0 && cell.1.rem_euclid(cfg.light_every_cells) == 0).then_some(Lamp {
        pos: [0.0, cfg.light_height_m, 0.0],
        color: cfg.light_color,
        energy: cfg.light_energy,
        range_m: cfg.light_range_m,
    })
}

/// Every box of one catwalk cell.
pub fn build(cfg: &CatwalkCfg, cell_m: f32, deck_height_m: f32, open: OpenSides, cell: (i32, i32)) -> Vec<Part> {
    let mut v = grating(cfg, cell_m);
    v.extend(railings(cfg, cell_m, open));
    v.extend(supports(cfg, cell_m, deck_height_m, cell));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> CatwalkCfg {
        CatwalkCfg::parse(include_str!("../data/render/catwalk.json")).unwrap()
    }

    #[test]
    fn embedded_file_parses_and_matches_the_source_file() {
        assert_eq!(config(), &cfg());
    }

    #[test]
    fn bad_data_is_rejected() {
        assert!(CatwalkCfg::parse("{}").is_err());
        let bad = include_str!("../data/render/catwalk.json").replace("\"slat_gap_m\": 0.07", "\"slat_gap_m\": 0.0");
        assert!(CatwalkCfg::parse(&bad).is_err());
    }

    #[test]
    fn grating_has_gaps_you_can_see_through_and_stays_in_the_cell() {
        let c = cfg();
        let slats: Vec<Part> = grating(&c, 3.0).into_iter().filter(|p| p.kind == PartKind::Slat).collect();
        assert!(slats.len() > 4);
        for w in slats.windows(2) {
            let gap = (w[1].center[0] - w[0].center[0]) - c.slat_width_m;
            assert!(gap > 0.0 && (gap - c.slat_gap_m).abs() < 1e-4);
        }
        for s in &slats {
            assert!(s.center[0].abs() + s.size[0] * 0.5 <= 1.5 + 1e-4);
        }
        let covered: f32 = slats.iter().map(|s| s.size[0]).sum();
        assert!(covered < 3.0 * 0.6, "mostly open");
    }

    #[test]
    fn grating_data_changes_slat_count() {
        let mut c = cfg();
        let before = grating(&c, 3.0).len();
        c.slat_gap_m *= 3.0;
        assert!(grating(&c, 3.0).len() < before);
    }

    #[test]
    fn railings_only_on_open_sides_at_the_data_height() {
        let c = cfg();
        assert!(railings(&c, 3.0, OpenSides::default()).is_empty());
        let one = railings(&c, 3.0, OpenSides { north: true, ..Default::default() });
        let two = railings(&c, 3.0, OpenSides { north: true, east: true, ..Default::default() });
        assert!(!one.is_empty());
        assert_eq!(two.len(), one.len() * 2);
        let top = one
            .iter()
            .filter(|p| p.kind == PartKind::Rail)
            .map(|p| p.center[1] + p.size[1] * 0.5)
            .fold(0.0, f32::max);
        assert!((top - c.rail_height_m).abs() < 1e-4);
        assert!(one.iter().filter(|p| p.kind == PartKind::RailPost).count() >= 2);
    }

    #[test]
    fn supports_sit_under_the_deck_and_posts_reach_the_ground() {
        let c = cfg();
        let s = supports(&c, 3.0, 4.0, (0, 0));
        for p in &s {
            assert!(p.center[1] + p.size[1] * 0.5 <= 1e-4, "nothing pokes above the walking surface");
        }
        let post = s.iter().find(|p| p.kind == PartKind::SupportPost).unwrap();
        assert!((post.center[1] - post.size[1] * 0.5 + 4.0).abs() < 1e-4);
        let off = supports(&c, 3.0, 4.0, (1, 0));
        assert!(off.iter().all(|p| p.kind != PartKind::SupportPost));
        assert!(off.iter().any(|p| p.kind == PartKind::Beam));
    }

    #[test]
    fn lamps_follow_the_data_spacing() {
        let c = cfg();
        let l = lamp(&c, (0, 0)).unwrap();
        assert_eq!(l.pos[1], c.light_height_m);
        assert!(lamp(&c, (1, 0)).is_none());
        assert!(lamp(&c, (c.light_every_cells, -c.light_every_cells)).is_some());
    }

    #[test]
    fn build_combines_all_parts() {
        let c = cfg();
        let all = build(&c, 3.0, 4.0, OpenSides { west: true, ..Default::default() }, (0, 0));
        for k in [PartKind::Slat, PartKind::Frame, PartKind::RailPost, PartKind::Beam, PartKind::SupportPost] {
            assert!(all.iter().any(|p| p.kind == k), "{k:?}");
        }
    }
}
