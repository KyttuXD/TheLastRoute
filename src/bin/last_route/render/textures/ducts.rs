//! Vent duct painters of ref_textures.gd (32 x 32, one tile per metre of run): galvanised sheet, flange ring, grille.
//!
//! Flat style: the sheet is one steel tone with a light and a dark tone in bands 30 luma apart, the seams and the
//! corner lines keep their place, and the grime dither and stains are not painted (their draws are still taken).

use super::canvas::*;
use super::rng::GodotRng;

fn steel() -> Col {
    hex(0x5d666c)
}

// Flat sheet tones: a light and a dark band either side of the steel (luma about 130, 100 and 66).
const SHEET_LIGHT: u32 = 0x7a858b;
const SHEET_DARK: u32 = 0x3d4448;

/// Flat rivet: a 2 x 2 dark block with a light centre pixel (the painted `rivet` is shaded, so flat does not use it).
fn rivet_flat(img: &mut Img, x: i32, y: i32, dark: Col, light: Col) {
    img.dot(x, y, dark, 2, 2);
    img.dot1(x, y, light);
}

pub fn paint_duct_sheet() -> Img {
    let mut rng = GodotRng::from_key("duct_sheet");
    let mut img = Img::new(PX, PX);
    let s = steel();
    let lit = |f: f64| flat_or(mul(s, f), hex(SHEET_LIGHT));
    let dim = |f: f64| flat_or(mul(s, f), hex(SHEET_DARK));
    noise(&mut img, 0, 0, PX, PX, s, 0.07, &mut rng, 1);
    for _ in 0..10 {
        img.dot(
            rng.randi_range(2, 29),
            rng.randi_range(2, 29),
            lit(1.22),
            rng.randi_range(1, 3),
            1,
        );
    }
    for i in 0..PX {
        img.dot1(i, i, lit(1.14));
        img.dot1(PX - 1 - i, i, dim(0.86));
    }
    for y in 20..PX {
        for x in 0..PX {
            // The lower grime dither is painted only; flat takes the same draw and paints nothing.
            if rng.randf() < (y - 20) as f64 / 40.0 && !flat() {
                let c = mul(img.get(x, y), 0.8);
                img.dot1(x, y, c);
            }
        }
    }
    for _ in 0..3 {
        let x = rng.randi_range(4, 27);
        let y = rng.randi_range(18, 26);
        let h = rng.randi_range(3, 6);
        if !flat() {
            img.dot(x, y, hex(0x3d3027), 1, h);
        }
    }
    img.dot(0, 0, lit(1.55), 2, PX);
    img.dot(2, 0, dim(0.5), 1, PX);
    img.dot(PX - 1, 0, dim(0.7), 1, PX);
    for ry in [4, 12, 20, 28] {
        if flat() {
            rivet_flat(&mut img, 4, ry, hex(SHEET_DARK), hex(SHEET_LIGHT));
        } else {
            rivet(&mut img, 4, ry, s);
        }
    }
    img.dot(0, 0, lit(1.3), PX, 1);
    img.dot(0, PX - 1, dim(0.55), PX, 1);
    img
}

pub fn paint_duct_flange() -> Img {
    let mut rng = GodotRng::from_key("duct_flange");
    let mut img = Img::new(PX, PX);
    let base = mul(steel(), 0.62);
    noise(&mut img, 0, 0, PX, PX, base, 0.1, &mut rng, 1);
    img.dot(0, 0, mul(base, 1.6), PX, 2);
    img.dot(0, PX - 2, mul(base, 0.5), PX, 2);
    // Flat: the rivet's light centre is the top lip's tone, so the flange keeps to four tones plus the rust.
    let rivet_light = flat_or(mul(base, 1.7), mul(base, 1.6));
    for by in [6, 16, 26] {
        img.dot(13, by - 1, mul(base, 0.4), 6, 4);
        img.dot(14, by - 1, rivet_light, 4, 2);
    }
    for _ in 0..3 {
        img.dot(
            rng.randi_range(2, 28),
            rng.randi_range(3, 28),
            hex(0x4a2c14),
            2,
            1,
        );
    }
    img
}

pub fn paint_duct_grille() -> Img {
    let mut img = Img::new(PX, PX);
    let frame = mul(steel(), 1.1);
    img.fill(frame);
    img.dot(0, 0, mul(frame, 1.35), PX, 1);
    img.dot(0, 0, mul(frame, 1.35), 1, PX);
    img.dot(PX - 1, 0, mul(frame, 0.55), 1, PX);
    img.dot(0, PX - 1, mul(frame, 0.55), PX, 1);
    img.dot(4, 4, hex(0x07080a), PX - 8, PX - 8);
    for gy in range(5, PX - 5, 3) {
        img.dot(4, gy, mul(frame, 1.25), PX - 8, 1);
        img.dot(4, gy + 1, mul(frame, 0.75), PX - 8, 1);
    }
    for (x, y) in [(1, 1), (PX - 3, 1), (1, PX - 3), (PX - 3, PX - 3)] {
        if flat() {
            rivet_flat(&mut img, x, y, mul(frame, 0.55), mul(frame, 1.35));
        } else {
            rivet(&mut img, x, y, frame);
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_duct_sheet_has_3_to_6_colours() {
        check_flat_painter("duct sheet", |_| paint_duct_sheet(), 0);
    }

    #[test]
    fn flat_duct_flange_has_3_to_6_colours() {
        check_flat_painter("duct flange", |_| paint_duct_flange(), 0);
    }

    #[test]
    fn flat_duct_grille_has_3_to_6_colours() {
        check_flat_painter("duct grille", |_| paint_duct_grille(), 0);
    }
}
