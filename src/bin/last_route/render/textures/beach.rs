//! The Beach district's painters (docs/BEACH_LOOK.md in the original): sand and damp sand, boardwalk planks, seawall,
//! whitewash, turquoise panelling and the skylight ceiling.

use super::canvas::*;
use super::rng::GodotRng;
use std::f64::consts::TAU;

fn sand() -> Col {
    hex(0x9a8460)
}
fn wet_sand() -> Col {
    hex(0x6c6150)
}
fn driftwood() -> Col {
    hex(0x8a7c68)
}
fn sea_stone() -> Col {
    hex(0x78807a)
}
fn sea_teal() -> Col {
    hex(0x3f8a8a)
}
fn sea_cream() -> Col {
    hex(0xb0a68c)
}

/// 0 dry sand with ripples, 1 sand with shells and pebbles, 2 damp sand (kind "wetsand0").
pub fn paint_sand(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("sand{variant}"));
    let mut img = Img::new(PX, PX);
    let wet = variant == 2;
    let base = if wet { wet_sand() } else { sand() };
    noise(&mut img, 0, 0, PX, PX, base, 0.06, &mut rng, 2);
    for _ in 0..90 {
        let speck = mul(base, if rng.randf() < 0.5 { 0.86 } else { 1.12 });
        let (x, y) = (rng.randi_range(0, PX - 1), rng.randi_range(0, PX - 1));
        if !flat() {
            img.dot1(x, y, speck); // flat: no grit, the draws stay
        }
    }
    // flat: the dune shadow is one dark tone (0.7), the crest the light one (1.12)
    let shadow = mul(base, 0.7);
    let ripples = [2, 1, 3][variant as usize];
    for _ in 0..ripples {
        let y0 = rng.randi_range(5, PX - 6);
        let amp = rng.randf_range(1.0, 2.0);
        let phase = rng.randf() * TAU;
        for x in 0..PX {
            let y = y0 + roundi((x as f64 / PX as f64 * TAU + phase).sin() * amp);
            img.dot1(
                x,
                y,
                flat_or(mul(base, if wet { 0.8 } else { 0.84 }), shadow),
            );
            img.dot1(x, y - 1, mul(base, 1.12));
        }
    }
    if wet {
        for _ in 0..14 {
            img.dot(
                rng.randi_range(0, PX - 2),
                rng.randi_range(0, PX - 1),
                hex(0x8a8878),
                rng.randi_range(1, 2),
                1,
            );
        }
        for _ in 0..3 {
            img.dot(
                rng.randi_range(0, PX - 5),
                rng.randi_range(0, PX - 3),
                hex(0x52483a),
                rng.randi_range(3, 5),
                2,
            );
        }
    } else if variant == 1 {
        for _ in 0..7 {
            img.dot(
                rng.randi_range(0, PX - 2),
                rng.randi_range(0, PX - 2),
                hex(0xc4b896),
                rng.randi_range(1, 2),
                1,
            );
        }
        for _ in 0..4 {
            let px = rng.randi_range(0, PX - 3);
            let py = rng.randi_range(0, PX - 4);
            img.dot(px, py + 1, flat_or(hex(0x4a4238), mul(base, 0.7)), 2, 2);
            img.dot(px, py, flat_or(hex(0x6e6658), mul(base, 1.12)), 2, 2);
        }
    } else {
        for _ in 0..2 {
            img.dot(
                rng.randi_range(0, PX - 2),
                rng.randi_range(0, PX - 2),
                hex(0xc4b896),
                2,
                1,
            );
        }
    }
    img
}

/// Bleached boardwalk: 0 planks run across (along u), 1 run down (along v).
pub fn paint_driftwood(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("driftwood{variant}"));
    let mut img = Img::new(PX, PX);
    let seam = hex(0x4e4436);
    let nail = hex(0x3e362c);
    for band in 0..4 {
        let shade = rng.randf_range(0.86, 1.14);
        // flat: two board tones, light and dark
        let shade = if flat() {
            if shade < 1.0 {
                0.9
            } else {
                1.1
            }
        } else {
            shade
        };
        let board = mul(driftwood(), shade);
        let joint = (rng.randi() % 20) as i32 + 4 + (band % 2) * 6;
        if variant == 0 {
            let y0 = band * 8;
            noise(&mut img, 0, y0, PX, 8, board, 0.08, &mut rng, 1);
            img.dot(0, y0 + 7, seam, PX, 1);
            img.dot(joint % PX, y0, seam, 1, 8);
            for _ in 0..4 {
                img.dot(
                    rng.randi_range(0, 30),
                    y0 + rng.randi_range(1, 6),
                    flat_or(mul(board, 0.78), seam),
                    rng.randi_range(3, 8),
                    1,
                );
            }
            img.dot(
                rng.randi_range(0, 28),
                y0 + rng.randi_range(1, 6),
                mul(board, 1.18),
                rng.randi_range(2, 5),
                1,
            );
            img.dot1((joint + 2) % PX, y0 + 3, nail);
            img.dot1((joint + PX - 3) % PX, y0 + 3, nail);
        } else {
            let x0 = band * 8;
            noise(&mut img, x0, 0, 8, PX, board, 0.08, &mut rng, 1);
            img.dot(x0 + 7, 0, seam, 1, PX);
            img.dot(x0, joint % PX, seam, 8, 1);
            for _ in 0..4 {
                img.dot(
                    x0 + rng.randi_range(1, 6),
                    rng.randi_range(0, 24),
                    flat_or(mul(board, 0.78), seam),
                    1,
                    rng.randi_range(3, 8),
                );
            }
            img.dot(
                x0 + rng.randi_range(1, 6),
                rng.randi_range(0, 26),
                mul(board, 1.18),
                1,
                rng.randi_range(2, 5),
            );
            img.dot1(x0 + 3, (joint + 2) % PX, nail);
            img.dot1(x0 + 3, (joint + PX - 3) % PX, nail);
        }
    }
    img
}

/// Pale sea-washed stone blocks, algae thickening towards the tide line.
pub fn paint_wall_seawall(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wseawall{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let base = mul(sea_stone(), if variant == 0 { 1.0 } else { 0.94 });
    let mortar = mul(base, 0.68);
    noise(&mut img, 0, 0, PX, WALL_H, base, 0.08, &mut rng, 2);
    let mut y = CORNICE_H + 1;
    while y < WALL_H - SKIRT_H {
        let h = [14, 18, 22][(rng.randi() % 3) as usize];
        img.dot(0, y, mortar, PX, 1);
        img.dot(rng.randi_range(4, 27), y, mortar, 1, h);
        img.dot(0, y + 1, mul(base, 1.18), PX, 1);
        y += h;
    }
    let algae = hex(0x4f8478);
    for i in 0..46 {
        let ay = WALL_H - SKIRT_H - 1 - (rng.randf().powf(1.8) * 46.0) as i32;
        let x = rng.randi_range(0, PX - 1);
        let c = if i % 3 != 0 { algae } else { mul(algae, 0.8) };
        img.dot(x, ay, c, 1, rng.randi_range(2, 6));
    }
    for _ in 0..5 {
        img.dot(
            rng.randi_range(0, PX - 2),
            rng.randi_range(WALL_H - 40, WALL_H - SKIRT_H - 3),
            hex(0xc0b89c),
            2,
            1,
        );
    }
    sea_cornice(&mut img, mul(base, 1.1));
    sea_skirt(&mut img, mul(base, 0.95));
    img
}

/// Cream vertical boards; 0 a turquoise stripe under the rail, 1 a full turquoise dado.
pub fn paint_wall_whitewash(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wwhitewash{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let cream = sea_cream();
    for band in 0..4 {
        let x0 = band * 8;
        let board = mul(cream, rng.randf_range(0.92, 1.06));
        noise(
            &mut img,
            x0,
            CORNICE_H,
            8,
            WALL_H - CORNICE_H,
            board,
            0.05,
            &mut rng,
            1,
        );
        img.dot(x0 + 7, CORNICE_H, mul(cream, 0.66), 1, WALL_H - CORNICE_H);
        img.dot(x0, CORNICE_H, mul(cream, 1.08), 1, WALL_H - CORNICE_H);
    }
    for _ in 0..14 {
        img.dot(
            rng.randi_range(1, 30),
            rng.randi_range(CORNICE_H + 2, RAIL_Y - 4),
            mul(cream, 0.8),
            1,
            rng.randi_range(4, 14),
        );
    }
    for _ in 0..9 {
        img.dot(
            rng.randi_range(0, 29),
            rng.randi_range(CORNICE_H + 4, WALL_H - SKIRT_H - 4),
            hex(0x7a7466),
            rng.randi_range(1, 3),
            1,
        );
    }
    if variant == 1 {
        for band in 0..4 {
            img.dot(
                band * 8,
                RAIL_Y,
                mul(sea_teal(), rng.randf_range(0.94, 1.06)),
                8,
                WALL_H - RAIL_Y,
            );
            img.dot(
                band * 8 + 7,
                RAIL_Y,
                mul(sea_teal(), 0.7),
                1,
                WALL_H - RAIL_Y,
            );
        }
    } else {
        img.dot(0, RAIL_Y + 2, sea_teal(), PX, 6);
    }
    img.dot(0, RAIL_Y - 1, mul(cream, 1.2), PX, 2);
    img.dot(0, RAIL_Y + 1, mul(cream, 0.6), PX, 1);
    sea_cornice(&mut img, cream);
    sea_skirt(
        &mut img,
        mul(if variant == 1 { sea_teal() } else { cream }, 0.9),
    );
    img
}

/// Turquoise beadboard with a cream cap rail; variant 1 is lighter aqua under a sandy upper wall.
pub fn paint_wall_tealpanel(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wtealpanel{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let paint = if variant == 0 {
        sea_teal()
    } else {
        hex(0x58a8a0)
    };
    let upper = if variant == 0 { paint } else { hex(0xa89c80) };
    noise(
        &mut img,
        0,
        CORNICE_H,
        PX,
        RAIL_Y - CORNICE_H,
        upper,
        0.05,
        &mut rng,
        1,
    );
    noise(
        &mut img,
        0,
        RAIL_Y,
        PX,
        WALL_H - RAIL_Y,
        paint,
        0.05,
        &mut rng,
        1,
    );
    for x in range(3, PX, 4) {
        img.dot(
            x,
            CORNICE_H + 1,
            mul(upper, 0.72),
            1,
            RAIL_Y - CORNICE_H - 2,
        );
        img.dot(
            x,
            RAIL_Y + 3,
            mul(paint, 0.72),
            1,
            WALL_H - RAIL_Y - SKIRT_H - 4,
        );
    }
    img.dot(0, RAIL_Y + 8, mul(paint, 1.2), PX, 1);
    img.dot(0, WALL_H - SKIRT_H - 6, mul(paint, 0.7), PX, 1);
    img.dot(0, RAIL_Y - 2, hex(0xc0b698), PX, 3);
    img.dot(0, RAIL_Y + 1, mul(paint, 0.6), PX, 1);
    for _ in 0..6 {
        img.dot(
            rng.randi_range(0, 30),
            rng.randi_range(CORNICE_H + 4, WALL_H - SKIRT_H - 4),
            mul(paint, 0.85),
            1,
            rng.randi_range(2, 5),
        );
    }
    sea_cornice(
        &mut img,
        if variant == 1 {
            hex(0xa89c80)
        } else {
            sea_cream()
        },
    );
    sea_skirt(&mut img, paint);
    img
}

/// Pale sky panes with cloud flecks inside a warm timber lattice.
pub fn paint_skylight(_variant: i32) -> Img {
    let mut rng = GodotRng::from_key("skylight");
    let mut img = Img::new(PX, PX);
    noise(&mut img, 0, 0, PX, PX, hex(0x7a9fb2), 0.04, &mut rng, 2);
    for _ in 0..7 {
        img.dot(
            rng.randi_range(2, 28),
            rng.randi_range(2, 28),
            hex(0xa8c4cc),
            rng.randi_range(2, 4),
            1,
        );
    }
    let beam = hex(0xa8946e);
    for k in [0, 16] {
        img.dot(k, 0, beam, 2, PX);
        img.dot(0, k, beam, PX, 2);
        img.dot(k + 2, 0, mul(beam, 0.6), 1, PX);
        img.dot(0, k + 2, mul(beam, 0.6), PX, 1);
    }
    img
}
