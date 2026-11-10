//! Flat lab surfaces: the lab kinds' patterns (metal plates and seams, grate slots, hazard stripes, epoxy joints, wall
//! joints, rail and skirting, ceiling coffers) painted from a few flat colours, with no bevel and no grain.
//!
//! Each texture uses 3-6 distinct colours in 2-3 tone groups (luma bands 20 apart or more). The painted painters in
//! floors.rs, walls.rs and ceilings.rs are unchanged; `Style::Flat` in mod.rs picks these instead.
//!
//! Where a later feature depends on the painted version's random draws (the plate's cracks, rust and drain), the flat
//! painter makes the same draws, so the features fall where the painted plate has them.

use super::canvas::*;
use super::rng::GodotRng;

// -- palettes: each group is one tone band ----------------------------------------------------------------------------

// Steel (metal, grate, hazard): seam is the dark band, plate and rust the middle, edge the light band.
const SEAM: u32 = 0x141012;
const PLATE: u32 = 0x2c2829;
const RUST: u32 = 0x4a2c14;
const EDGE: u32 = 0x564e50;
const YELLOW: u32 = 0x7a5c12;

// Epoxy floor: joint dark, floor middle, marker cyan (light), hatch yellow (middle, keep-clear band).
const EPOXY_JOINT: u32 = 0x2e3234;
const EPOXY_FLOOR: u32 = 0x535b60;
const MARKER: u32 = 0x2a8f9c;

// Lab wall: joint and skirting dark, paint middle, rail cyan, rivet edge light.
const WALL_DARK: u32 = 0x1e2224;
const WALL_PAINT: u32 = 0x5a5f63;
const RAIL: u32 = 0x1d8a96;
const WALL_EDGE: u32 = 0x8a9094;

// Ceiling: groove darkest, panel dark, base dark, beam light.
const GROOVE: u32 = 0x060405;
const PANEL: u32 = 0x0e0b0d;
const CEIL_BASE: u32 = 0x1a1518;
const BEAM: u32 = 0x342c31;

/// A two-pixel rivet: a dark 2 x 2 block with a light centre pixel.
fn rivet_flat(img: &mut Img, x: i32, y: i32, dark: u32, light: u32) {
    img.dot(x, y, hex(dark), 2, 2);
    img.dot1(x, y, hex(light));
}

/// `noise` over a whole texture draws one value per texel; the flat painters take no grain but make the same draws.
fn skip_grain(rng: &mut GodotRng, cells: usize) {
    for _ in 0..cells {
        rng.randi_range(-1, 1);
    }
}

/// Flat version of `floors::paint_metal`: a plate with a lit top-left edge, dark bottom-right seam, four rivets,
/// and the same specks, crack, rust and drain draws as the painted plate. Variant 3 has a drain grille.
pub fn paint_metal(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("metal{variant}"));
    let mut img = Img::new(PX, PX);
    img.fill(hex(PLATE));
    skip_grain(&mut rng, (PX * PX) as usize);
    for _ in 0..3 {
        let x = rng.randi_range(1, 28);
        let y = rng.randi_range(1, 28);
        let w = rng.randi_range(2, 4);
        img.dot(x, y, hex(SEAM), w, 1);
    }
    img.dot(0, 0, hex(EDGE), PX, 1);
    img.dot(0, 1, hex(EDGE), PX, 1);
    img.dot(0, 0, hex(EDGE), 1, PX);
    img.dot(PX - 1, 0, hex(SEAM), 1, PX);
    img.dot(0, PX - 1, hex(SEAM), PX, 1);
    for (x, y) in [(3, 3), (26, 3), (3, 26), (26, 26)] {
        rivet_flat(&mut img, x, y, SEAM, EDGE);
    }
    if rng.randf() < 0.35 {
        let x = rng.randi_range(5, 25);
        let y = rng.randi_range(5, 25);
        let len = rng.randi_range(4, 9);
        crack(&mut img, &mut rng, x, y, len, hex(SEAM));
    }
    if rng.randf() < 0.5 {
        for _ in 0..4 {
            let x = rng.randi_range(2, 29);
            let y = rng.randi_range(2, 29);
            img.dot1(x, y, hex(RUST));
        }
    }
    if rng.randf() < 0.4 {
        let x = rng.randi_range(4, 26);
        let h = rng.randi_range(8, 18);
        img.dot(x, 5, hex(EDGE), 1, h);
    }
    if variant == 3 {
        for gy in range(11, 22, 2) {
            img.dot(9, gy, hex(SEAM), 14, 1);
        }
    }
    img
}

/// Flat version of `floors::paint_grate`: metal1 with dark slots and a light lip under each.
pub fn paint_grate(_variant: i32) -> Img {
    let mut img = paint_metal(1);
    for gy in range(8, 26, 3) {
        img.dot(6, gy, hex(SEAM), 20, 2);
        img.dot(6, gy + 2, hex(EDGE), 20, 1);
    }
    img
}

/// Flat version of `floors::paint_hazard`: metal0 with a 7 px yellow and dark stripe band along the top edge.
pub fn paint_hazard(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("hazard{variant}"));
    let mut img = paint_metal(0);
    for y in 0..7 {
        for x in 0..PX {
            img.dot1(x, y, hazard_stripe(x, y, hex(YELLOW), hex(SEAM)));
        }
    }
    img.dot(0, 7, hex(SEAM), PX, 1);
    if rng.randf() < 0.5 {
        crack(&mut img, &mut rng, 4, 3, 6, hex(SEAM));
    }
    img
}

/// Flat version of `floors::paint_epoxy`: a grey floor on a metre grid with cyan corner markers; variant 1 adds a
/// yellow keep-clear hatch along the bottom.
pub fn paint_epoxy(variant: i32) -> Img {
    let mut img = Img::new(PX, PX);
    img.fill(hex(EPOXY_FLOOR));
    img.dot(0, 0, hex(EPOXY_JOINT), PX, 1);
    img.dot(0, 0, hex(EPOXY_JOINT), 1, PX);
    img.dot(0, 0, hex(MARKER), 3, 1);
    img.dot(0, 0, hex(MARKER), 1, 3);
    if variant == 1 {
        for x in 0..PX {
            if (x >> 2) & 1 == 0 {
                img.dot(x, PX - 4, hex(YELLOW), 1, 3);
            }
        }
    }
    img
}

/// Flat version of `walls::paint_wall_lab`: white composite panels with dark joints every 40 rows, a cyan rail, a
/// riveted edge, a flat cornice and skirting. Variant 1 has a yellow and dark hazard foot instead of the skirting.
pub fn paint_wall_lab(variant: i32) -> Img {
    let mut img = Img::new(PX, WALL_H);
    img.fill(hex(WALL_PAINT));
    for y in range(CORNICE_H, WALL_H, 40) {
        img.dot(0, y, hex(WALL_DARK), PX, 1);
    }
    img.dot(PX - 1, 0, hex(WALL_DARK), 1, WALL_H);
    img.dot(0, RAIL_Y, hex(RAIL), PX, 2);
    for y in [CORNICE_H + 4, CORNICE_H + 44] {
        rivet_flat(&mut img, 3, y, WALL_DARK, WALL_EDGE);
        rivet_flat(&mut img, PX - 5, y, WALL_DARK, WALL_EDGE);
    }
    if variant == 1 {
        for y in WALL_H - 12..WALL_H {
            for x in 0..PX {
                img.dot1(x, y, hazard_stripe(x, y, hex(YELLOW), hex(WALL_DARK)));
            }
        }
    } else {
        img.dot(0, WALL_H - SKIRT_H, hex(WALL_DARK), PX, SKIRT_H);
        img.dot(0, WALL_H - SKIRT_H, hex(WALL_PAINT), PX, 1);
        img.dot(0, WALL_H - SKIRT_H - 1, hex(WALL_DARK), PX, 1);
    }
    img.dot(0, 0, hex(WALL_DARK), PX, CORNICE_H);
    img.dot(0, CORNICE_H - 3, hex(WALL_PAINT), PX, 1);
    img.dot(0, CORNICE_H, hex(WALL_DARK), PX, 1);
    img
}

/// Flat version of `ceilings::paint_ceiling`: a coffered ceiling with raised beams on the top and left edges, a groove
/// line inside them and two rivets; variant 1 adds a darker centre panel.
pub fn paint_ceiling(variant: i32) -> Img {
    let mut img = Img::new(PX, PX);
    img.fill(hex(CEIL_BASE));
    img.dot(0, 0, hex(BEAM), PX, 2);
    img.dot(0, 0, hex(BEAM), 2, PX);
    img.dot(2, 2, hex(GROOVE), PX - 2, 1);
    img.dot(2, 2, hex(GROOVE), 1, PX - 2);
    for (x, y) in [(8, 8), (22, 22)] {
        rivet_flat(&mut img, x, y, GROOVE, BEAM);
    }
    if variant == 1 {
        img.dot(14, 14, hex(PANEL), 6, 6);
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Luma of an RGBA8 texel, 0..255.
    fn luma(p: &[u8]) -> f64 {
        0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64
    }

    /// Distinct colours and tone groups of a texture: a group ends where the next luma band is more than 20 away.
    pub(crate) fn palette(img: &Img) -> (usize, usize) {
        let mut colours: Vec<[u8; 4]> = img.px.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
        colours.sort_unstable();
        colours.dedup();
        let mut lumas: Vec<f64> = colours.iter().map(|c| luma(c)).collect();
        lumas.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut groups = 1;
        for pair in lumas.windows(2) {
            if pair[1] - pair[0] > 20.0 {
                groups += 1;
            }
        }
        (colours.len(), groups)
    }

    #[test]
    fn flat_lab_floors_have_3_to_6_colours_in_2_or_3_tone_groups() {
        for (kind, variant) in [("epoxy0", 0), ("epoxy1", 0), ("grate", 0), ("hazard", 0), ("metal1", 0)] {
            let img = super::super::paint_styled("floor", kind, variant, super::super::Style::Flat).unwrap();
            let img = Img { w: img.0 as i32, h: img.1 as i32, px: img.2 };
            let (colours, groups) = palette(&img);
            assert!((3..=6).contains(&colours), "floor:{kind} has {colours} colours");
            assert!((2..=3).contains(&groups), "floor:{kind} has {groups} tone groups");
        }
    }

    #[test]
    fn flat_lab_walls_and_ceilings_have_3_to_6_colours_in_2_or_3_tone_groups() {
        for (group, kind) in [("wall", "lab0"), ("wall", "lab1"), ("ceiling", "ceil0")] {
            let (w, h, px) = super::super::paint_styled(group, kind, 0, super::super::Style::Flat).unwrap();
            let img = Img { w: w as i32, h: h as i32, px };
            let (colours, groups) = palette(&img);
            assert!((3..=6).contains(&colours), "{group}:{kind} has {colours} colours");
            assert!((2..=3).contains(&groups), "{group}:{kind} has {groups} tone groups");
        }
    }

    /// The flat pattern stays: the epoxy joints and markers, the grate slots, the hazard band, the wall rail.
    /// Whether the texel at (x, y) holds the colour `rgb` as `Img::set` stores it (Godot's truncation included).
    fn is(img: &Img, x: i32, y: i32, rgb: u32) -> bool {
        let mut one = Img::new(1, 1);
        one.dot1(0, 0, hex(rgb));
        img.get(x, y) == one.get(0, 0)
    }

    #[test]
    fn flat_lab_keeps_its_patterns() {
        let epoxy = paint_epoxy(1);
        assert!(is(&epoxy, 0, 10, EPOXY_JOINT), "epoxy joint line");
        assert!(is(&epoxy, 1, 0, MARKER), "epoxy marker");
        let grate = paint_grate(0);
        assert!(is(&grate, 10, 8, SEAM), "grate slot");
        assert!(is(&grate, 10, 10, EDGE), "grate lip");
        let hazard = paint_hazard(0);
        assert!(is(&hazard, 12, 0, YELLOW), "hazard stripe");
        assert!(is(&hazard, 0, 0, SEAM), "hazard black stripe");
        let wall = paint_wall_lab(0);
        assert!(is(&wall, 5, RAIL_Y, RAIL), "wall rail");
        assert!(is(&wall, 5, CORNICE_H, WALL_DARK), "wall cornice line");
    }

    #[test]
    fn flat_lab_is_not_blank() {
        for (group, kind) in [("floor", "metal1"), ("floor", "epoxy0"), ("wall", "lab0"), ("ceiling", "ceil1")] {
            let (_, _, px) = super::super::paint_styled(group, kind, 0, super::super::Style::Flat).unwrap();
            let first = &px[0..4];
            assert!(px.chunks_exact(4).any(|p| p != first), "{group}:{kind} is blank");
        }
    }
}
