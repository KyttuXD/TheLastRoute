//! Ceiling painters of ref_textures.gd (32 x 32, sampled like a floor seen from below): coffered ceiling, sewer vault,
//! conservatory glass, cave rock.

use super::canvas::*;
use super::rng::GodotRng;

/// Coffered grid with raised beams on the top and left edges, two rivets; variant 1 adds a dark centre panel.
pub fn paint_ceiling(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("ceil{variant}"));
    let mut img = Img::new(PX, PX);
    let base = if variant == 0 {
        hex(0x100d0f)
    } else {
        hex(0x120a0c)
    };
    noise(&mut img, 0, 0, PX, PX, base, 0.14, &mut rng, 1);
    img.dot(0, 0, mul(base, 2.2), PX, 2);
    img.dot(0, 0, mul(base, 2.2), 2, PX);
    img.dot(2, 2, hex(0x050304), PX - 2, 1);
    img.dot(2, 2, hex(0x050304), 1, PX - 2);
    for (x, y) in [(8, 8), (22, 22)] {
        rivet(&mut img, x, y, base);
    }
    if variant == 1 {
        img.dot(14, 14, mul(base, 0.6), 6, 6);
    }
    img
}

/// The sewer's barrel vault: damp concrete with one course line.
pub fn paint_vault(_variant: i32) -> Img {
    let mut rng = GodotRng::from_key("vault");
    let mut img = Img::new(PX, PX);
    let base = hex(0x4a443d);
    noise(&mut img, 0, 0, PX, PX, base, 0.18, &mut rng, 1);
    img.dot(0, 0, mul(base, 1.35), PX, 1);
    for _ in 0..4 {
        img.dot(
            rng.randi_range(1, PX - 3),
            rng.randi_range(3, PX - 3),
            mul(base, 0.55),
            2,
            1,
        );
    }
    img
}

/// Glass-and-iron roof: night panes in a black lattice every 16 px with a bronze edge.
pub fn paint_glass(_variant: i32) -> Img {
    let mut rng = GodotRng::from_key("glass");
    let mut img = Img::new(PX, PX);
    for y in 0..PX {
        for x in 0..PX {
            let shade = 1.0 + rng.randf_range(-0.15, 0.15);
            img.dot1(x, y, mul(hex(0x0c1426), shade));
        }
    }
    for _ in 0..5 {
        img.dot1(
            rng.randi_range(2, 29),
            rng.randi_range(2, 29),
            hex(0x2a4a66),
        );
    }
    for k in [0, 16] {
        img.dot(k, 0, hex(0x07070a), 2, PX);
        img.dot(0, k, hex(0x07070a), PX, 2);
        img.dot(k + 2, 0, hex(0x3a2e1c), 1, PX);
        img.dot(0, k + 2, hex(0x3a2e1c), PX, 1);
    }
    img
}

pub fn paint_rock_ceiling(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("rockceil{variant}"));
    let mut img = Img::new(PX, PX);
    noise(&mut img, 0, 0, PX, PX, hex(0x151012), 0.22, &mut rng, 2);
    for i in 0..14 {
        let c = if i % 2 == 0 {
            hex(0x07040a)
        } else {
            hex(0x2a2022)
        };
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, 31),
            c,
            rng.randi_range(2, 5),
            rng.randi_range(2, 4),
        );
    }
    img
}
