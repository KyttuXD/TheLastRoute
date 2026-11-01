//! Vent duct painters of ref_textures.gd (32 x 32, one tile per metre of run): galvanised sheet, flange ring, grille.

use super::canvas::*;
use super::rng::GodotRng;

fn steel() -> Col {
    hex(0x5d666c)
}

pub fn paint_duct_sheet() -> Img {
    let mut rng = GodotRng::from_key("duct_sheet");
    let mut img = Img::new(PX, PX);
    let s = steel();
    noise(&mut img, 0, 0, PX, PX, s, 0.07, &mut rng, 1);
    for _ in 0..10 {
        img.dot(
            rng.randi_range(2, 29),
            rng.randi_range(2, 29),
            mul(s, 1.22),
            rng.randi_range(1, 3),
            1,
        );
    }
    for i in 0..PX {
        img.dot1(i, i, mul(s, 1.14));
        img.dot1(PX - 1 - i, i, mul(s, 0.86));
    }
    for y in 20..PX {
        for x in 0..PX {
            if rng.randf() < (y - 20) as f64 / 40.0 {
                let c = mul(img.get(x, y), 0.8);
                img.dot1(x, y, c);
            }
        }
    }
    for _ in 0..3 {
        img.dot(
            rng.randi_range(4, 27),
            rng.randi_range(18, 26),
            hex(0x3d3027),
            1,
            rng.randi_range(3, 6),
        );
    }
    img.dot(0, 0, mul(s, 1.55), 2, PX);
    img.dot(2, 0, mul(s, 0.5), 1, PX);
    img.dot(PX - 1, 0, mul(s, 0.7), 1, PX);
    for ry in [4, 12, 20, 28] {
        rivet(&mut img, 4, ry, s);
    }
    img.dot(0, 0, mul(s, 1.3), PX, 1);
    img.dot(0, PX - 1, mul(s, 0.55), PX, 1);
    img
}

pub fn paint_duct_flange() -> Img {
    let mut rng = GodotRng::from_key("duct_flange");
    let mut img = Img::new(PX, PX);
    let base = mul(steel(), 0.62);
    noise(&mut img, 0, 0, PX, PX, base, 0.1, &mut rng, 1);
    img.dot(0, 0, mul(base, 1.6), PX, 2);
    img.dot(0, PX - 2, mul(base, 0.5), PX, 2);
    for by in [6, 16, 26] {
        img.dot(13, by - 1, mul(base, 0.4), 6, 4);
        img.dot(14, by - 1, mul(base, 1.7), 4, 2);
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
        rivet(&mut img, x, y, frame);
    }
    img
}
