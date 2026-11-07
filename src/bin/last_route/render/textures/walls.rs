//! Wall painters of ref_textures.gd (32 x 128: one storey of wall, cornice at the top row, skirting at the bottom):
//! panel, wallpaper (damask over wainscot), brick, stone, hedge, office, hospital, lab, glasshouse, mall, corridor,
//! inferno, cave rock.
//!
//! Flat style (`canvas::flat()`): the two wall tones from `wall_base` stay, every joint, line, frame, cornice and
//! skirting is the one dark `TRIM`, the brass and cyan rails stay as accents, and the highlights, shadows and speckled
//! leaves go. Each painter takes the same RNG draws in both styles, so the layouts do not move.

use super::canvas::*;
use super::floors::metal;
use super::rng::GodotRng;
use std::f64::consts::{PI, TAU};

/// The flat style's dark: every joint, pattern line, frame, cornice and skirting band is this one tone.
const TRIM: u32 = 0x060405;
/// The brass rail and ledge colour.
const BRASS: u32 = 0x8a6a2c;

/// A pattern line (joint, seam, frame, shadow): as painted, or in the flat style the one dark tone.
fn ink(rgb: u32) -> Col {
    flat_or(hex(rgb), hex(TRIM))
}

/// Cornice: the canvas cornice, or in the flat style one dark band over the top rows.
fn wall_cornice(img: &mut Img, c: Col) {
    if flat() {
        img.dot(0, 0, hex(TRIM), PX, CORNICE_H + 1);
    } else {
        cornice(img, c);
    }
}

/// Skirting: the canvas skirting, or in the flat style one dark band over the bottom rows.
fn wall_skirt(img: &mut Img, c: Col) {
    if flat() {
        img.dot(0, WALL_H - SKIRT_H - 1, hex(TRIM), PX, SKIRT_H + 1);
    } else {
        skirt(img, c);
    }
}

/// Chair rail: the canvas rail, or in the flat style two brass rows between dark edges.
fn wall_rail(img: &mut Img) {
    if flat() {
        img.dot(0, RAIL_Y - 2, hex(TRIM), PX, 1);
        img.dot(0, RAIL_Y - 1, hex(BRASS), PX, 2);
        img.dot(0, RAIL_Y + 1, hex(TRIM), PX, 1);
    } else {
        rail(img);
    }
}

/// `canvas::flower` in the flat style: the five petals and the core, with no highlight dot on the petals.
fn bloom(img: &mut Img, cx: i32, cy: i32, petal: Col, core: Col) {
    if !flat() {
        flower(img, cx, cy, petal, core);
        return;
    }
    for a in 0..5 {
        let ang = a as f64 / 5.0 * TAU - PI * 0.5;
        let px = roundi(cx as f64 + ang.cos() * 3.0);
        let py = roundi(cy as f64 + ang.sin() * 3.0);
        img.dot(px - 1, py - 1, petal, 3, 3);
    }
    img.dot(cx - 1, cy - 1, core, 3, 3);
}

/// `canvas::leaf_cluster` in the flat style: each leaf is a dark or a light tone, picked by the draw that sets its shade.
fn flat_leaves(img: &mut Img, rng: &mut GodotRng, cx: i32, cy: i32, r: i32) {
    for _ in 0..r * 6 {
        let a = rng.randf() * TAU;
        let d = rng.randf() * r as f64;
        let x = cx + roundi(a.cos() * d);
        let y = cy + roundi(a.sin() * d * 0.8);
        let shade = rng.randf_range(0.5, 1.6);
        let c = if shade < 1.05 {
            hex(0x1a2418)
        } else {
            hex(0x3e5232)
        };
        img.dot(x, y, c, 2, 1);
    }
}

/// Industrial plating: riveted plates, maroon lower half; 1 adds a brass pipe, 2 a vent grille.
pub fn paint_wall_panel(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wpanel{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(&mut img, &mut rng, hex(0x32202a), metal());
    for y in range(0, WALL_H, 32) {
        img.dot(0, y, ink(0x0a0809), PX, 1);
        img.dot(0, y + 1, flat_or(mul(metal(), 1.7), metal()), PX, 1);
    }
    img.dot(0, 0, ink(0x0a0809), 1, WALL_H);
    img.dot(PX - 1, 0, ink(0x0a0809), 1, WALL_H);
    for y in range(4, WALL_H, 32) {
        for x in [3, PX - 5] {
            rivet(&mut img, x, y, metal());
        }
    }
    wall_rail(&mut img);
    wall_skirt(&mut img, hex(0x2a1418));
    wall_cornice(&mut img, metal());
    if variant == 1 {
        let px = 4 + rng.randi_range(0, 18);
        img.dot(px, 0, hex(0x3a2a14), 3, WALL_H);
        img.dot(px, 0, hex(0x8a6a2c), 1, WALL_H);
        for y in range(12, WALL_H, 36) {
            img.dot(px - 1, y, ink(0x1a1209), 5, 3);
        }
    }
    if variant == 2 {
        for gy in range(40, 62, 3) {
            img.dot(8, gy, ink(0x070506), 16, 1);
        }
        img.dot(7, 38, ink(0x5a4a40), 18, 1);
        img.dot(7, 62, ink(0x5a4a40), 18, 1);
    }
    grime(&mut img, &mut rng, 5);
    img
}

/// Damask paper (flowers + lattice) over a dark wood wainscot with two recessed panels, brass rail.
pub fn paint_wall_wallpaper(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wpaper{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let paper = if variant == 0 {
        hex(0x2c1a26)
    } else {
        hex(0x2e1a22)
    };
    noise(&mut img, 0, 0, PX, RAIL_Y, paper, 0.1, &mut rng, 1);
    for row in range(0, RAIL_Y - 12, 28) {
        for col in 0..2 {
            let cx = 8 + col * 16;
            let cy = CORNICE_H + 8 + row + (col * 14) % 28;
            if cy < RAIL_Y - 8 {
                bloom(&mut img, cx, cy, hex(0x4a2634), hex(0x7a5a2c));
            }
        }
        for dx in 0..PX {
            let cy2 = CORNICE_H + 22 + row;
            if cy2 < RAIL_Y - 4 && (dx % 8) < 2 {
                img.dot1(dx, cy2 + (dx % 8), ink(0x4a2230));
            }
        }
    }
    let wood = if variant == 0 {
        hex(0x2a1a14)
    } else {
        hex(0x241612)
    };
    noise(
        &mut img,
        0,
        RAIL_Y,
        PX,
        WALL_H - RAIL_Y,
        wood,
        0.12,
        &mut rng,
        1,
    );
    for (top, h) in [(RAIL_Y + 6, 22), (RAIL_Y + 6 + 26, 22)] {
        if flat() {
            img.dot(3, top, hex(TRIM), PX - 6, h);
            img.dot(4, top + 1, wood, PX - 8, h - 2);
        } else {
            img.dot(3, top, mul(wood, 0.45), PX - 6, h);
            img.dot(3, top, mul(wood, 1.6), PX - 6, 1);
            img.dot(3, top, mul(wood, 1.3), 1, h);
            img.dot(PX - 4, top, hex(0x060303), 1, h);
        }
    }
    wall_rail(&mut img);
    wall_skirt(&mut img, wood);
    wall_cornice(&mut img, hex(0x3a2820));
    grime(&mut img, &mut rng, 3);
    img
}

/// 6 px brick courses with a half-brick offset, wet streaks and moss near the foot.
pub fn paint_wall_brick(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wbrick{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let base = if variant == 0 {
        hex(0x2e1c1a)
    } else {
        hex(0x261a1c)
    };
    let mortar = ink(0x0e0a0a);
    noise(&mut img, 0, 0, PX, WALL_H, base, 0.13, &mut rng, 1);
    let mut row = 0;
    for y in range(0, WALL_H, 6) {
        img.dot(0, y, mortar, PX, 1);
        let off = if row % 2 == 0 { 0 } else { 8 };
        for x in range(off, PX + 8, 16) {
            img.dot(x % PX, y, mortar, 1, 6);
            let tone = mul(base, rng.randf_range(0.7, 1.3));
            img.dot((x + 2) % PX, y + 2, flat_or(tone, base), 6, 3);
        }
        row += 1;
    }
    for _ in 0..5 {
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, WALL_H - 24),
            ink(0x141018),
            1,
            rng.randi_range(8, 28),
        );
    }
    for _ in 0..10 {
        img.dot(
            rng.randi_range(0, 31),
            WALL_H - rng.randi_range(2, 22),
            hex(0x1c2a1c),
            2,
            1,
        );
    }
    img.dot(0, WALL_H - 4, ink(0x0a0707), PX, 4);
    wall_cornice(&mut img, hex(0x2a2022));
    img
}

/// Ashlar courses of 10/14/18 px with water stains.
pub fn paint_wall_stone(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wstone{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let base = hex(0x2c2a2c);
    let mortar = ink(0x0b0a0c);
    noise(&mut img, 0, 0, PX, WALL_H, base, 0.12, &mut rng, 2);
    let mut y = 0;
    while y < WALL_H {
        let h = [10, 14, 18][(rng.randi() % 3) as usize];
        img.dot(0, y, mortar, PX, 1);
        img.dot(rng.randi_range(4, 27), y, mortar, 1, h);
        img.dot(0, y + 1, flat_or(mul(base, 1.4), base), PX, 1);
        y += h;
    }
    for _ in 0..4 {
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, 60),
            ink(0x17141a),
            2,
            rng.randi_range(10, 40),
        );
    }
    for _ in 0..12 {
        img.dot1(
            rng.randi_range(0, 31),
            rng.randi_range(0, WALL_H - 1),
            hex(0x3a4648),
        );
    }
    img.dot(0, WALL_H - 5, ink(0x0b090a), PX, 5);
    wall_cornice(&mut img, hex(0x2a2628));
    img
}

/// Ivy over a brass-banded masonry plinth, dusky flowers and hanging vines.
pub fn paint_wall_hedge(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("whedge{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let ground = hex(0x26301f);
    noise(&mut img, 0, 0, PX, WALL_H, ground, 0.22, &mut rng, 1);
    for _ in 0..28 {
        let (cx, cy, r) = (
            rng.randi_range(0, 31),
            rng.randi_range(4, WALL_H - 6),
            rng.randi_range(2, 5),
        );
        if flat() {
            flat_leaves(&mut img, &mut rng, cx, cy, r);
        } else {
            leaf_cluster(&mut img, &mut rng, cx, cy, r, hex(0x2a3a26));
        }
    }
    for i in 0..10 {
        let fx = rng.randi_range(2, 28);
        let fy = rng.randi_range(14, WALL_H - 20);
        let petal = if i % 2 == 0 {
            hex(0x8a2e52)
        } else {
            hex(0xb09a88)
        };
        img.dot(fx, fy, flat_or(petal, hex(0x5a2e3e)), 3, 3);
    }
    img.dot(0, WALL_H - 30, flat_or(hex(0x3a3030), ground), PX, 30);
    for by in range(WALL_H - 30, WALL_H, 10) {
        img.dot(0, by, ink(0x0a0808), PX, 1);
    }
    img.dot(0, WALL_H - 31, hex(BRASS), PX, 1);
    wall_skirt(&mut img, hex(0x2a2224));
    for _ in 0..4 {
        let vx = rng.randi_range(1, 30);
        img.dot(
            vx,
            rng.randi_range(0, 20),
            ink(0x162016),
            1,
            rng.randi_range(26, 70),
        );
    }
    img
}

/// Two-tone plaster: maroon lower, grey-green upper; variant 1 has pinned papers.
pub fn paint_wall_office(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("woffice{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(&mut img, &mut rng, hex(0x36202a), hex(0x2c2a28));
    wall_rail(&mut img);
    wall_skirt(&mut img, hex(0x1c1214));
    wall_cornice(&mut img, hex(0x2a2828));
    if variant == 1 {
        for _ in 0..3 {
            let x = rng.randi_range(3, 22);
            let y = rng.randi_range(18, 52);
            img.dot(x, y, flat_or(hex(0x4c4842), hex(0x7e7868)), 5, 6);
            img.dot1(x + 2, y, flat_or(hex(0x7a2a2a), hex(TRIM)));
        }
    }
    grime(&mut img, &mut rng, 4);
    img
}

/// Clinical mint lower / off-white upper with a teal crash rail; variant 1 adds a ward stripe and a room plate.
pub fn paint_wall_hospital(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("whosp{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(&mut img, &mut rng, hex(0x39504b), hex(0x575a55));
    img.dot(0, RAIL_Y - 4, ink(0x0a0c0c), PX, 1);
    img.dot(0, RAIL_Y - 3, hex(0x2f6a74), PX, 4);
    img.dot(0, RAIL_Y + 1, ink(0x173a40), PX, 1);
    wall_skirt(&mut img, hex(0x2a3a38));
    wall_cornice(&mut img, hex(0x4a4c48));
    if variant == 1 {
        img.dot(0, RAIL_Y + 14, hex(0x2c5f8a), PX, 2);
        img.dot(10, 30, flat_or(hex(0x1b3a5c), hex(0x2c5f8a)), 12, 6);
        img.dot(12, 32, flat_or(hex(0xc8cfd0), hex(TRIM)), 8, 2);
    }
    grime(&mut img, &mut rng, 3);
    img
}

/// White composite panels, cyan guide line; variant 1 has a hazard-striped foot instead of skirting.
pub fn paint_wall_lab(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wlab{variant}"));
    let mut img = Img::new(PX, WALL_H);
    noise(&mut img, 0, 0, PX, WALL_H, hex(0x5a5f63), 0.04, &mut rng, 2);
    for y in range(CORNICE_H, WALL_H, 40) {
        img.dot(0, y, ink(0x1e2224), PX, 1);
    }
    img.dot(PX - 1, 0, ink(0x1e2224), 1, WALL_H);
    img.dot(0, RAIL_Y, hex(0x1d8a96), PX, 2);
    for y in [CORNICE_H + 4, CORNICE_H + 44] {
        rivet(&mut img, 3, y, hex(0x5a5f63));
        rivet(&mut img, PX - 5, y, hex(0x5a5f63));
    }
    if variant == 1 {
        for y in WALL_H - 12..WALL_H {
            for x in 0..PX {
                img.dot1(x, y, hazard_stripe(x, y, hex(0x7a5c12), ink(0x14110e)));
            }
        }
    } else {
        wall_skirt(&mut img, hex(0x2c3134));
    }
    wall_cornice(&mut img, hex(0x3a3f42));
    img
}

/// Conservatory glazing: green iron bars over night panes, a low brick plinth; variant 1 has leaves on the glass.
pub fn paint_wall_glasshouse(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wglass{variant}"));
    let mut img = Img::new(PX, WALL_H);
    noise(&mut img, 0, 0, PX, WALL_H, hex(0x1e2a2e), 0.08, &mut rng, 2);
    for _ in 0..5 {
        let x = rng.randi_range(2, PX - 6);
        let y = rng.randi_range(CORNICE_H + 2, RAIL_Y);
        img.dot(x, y, hex(0x3a4e52), 1, 6);
    }
    let bar = hex(0x2b4a30);
    for x in [0, 15, PX - 2] {
        img.dot(x, 0, bar, 2, WALL_H);
    }
    for y in range(CORNICE_H, WALL_H - 26, 24) {
        img.dot(0, y, bar, PX, 2);
    }
    if flat() {
        // the brick cells sit on dark mortar, as the joints do in the painted plinth
        img.dot(0, WALL_H - 26, hex(TRIM), PX, 26);
    }
    for by in range(WALL_H - 26, WALL_H, 6) {
        // range(-((by / 6) % 2) * 4, PX, 8); the noise starts at maxi(0, bx) but keeps its 7 px width
        for bx in range(-((by / 6) % 2) * 4, PX, 8) {
            noise(
                &mut img,
                bx.max(0),
                by,
                7,
                5,
                hex(0x4a2a22),
                0.1,
                &mut rng,
                1,
            );
        }
    }
    if variant == 1 {
        for _ in 0..10 {
            img.dot(
                rng.randi_range(0, PX - 3),
                rng.randi_range(RAIL_Y - 20, WALL_H - 28),
                hex(0x2f5a28),
                3,
                2,
            );
        }
    }
    wall_cornice(&mut img, bar);
    img
}

/// Arcade wall: cream plaster over a tiled dado, a lit cove strip under the cornice (amber on 0, teal on 1).
pub fn paint_wall_mall(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wmall{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(
        &mut img,
        &mut rng,
        flat_or(hex(0x3a2c2a), hex(0x4a3f3c)),
        hex(0x5c524a),
    );
    for y in range(RAIL_Y + 2, WALL_H - SKIRT_H, 8) {
        img.dot(0, y, ink(0x1e1614), PX, 1);
    }
    for x in [0, 16] {
        img.dot(x, RAIL_Y, ink(0x1e1614), 1, WALL_H - RAIL_Y - SKIRT_H);
    }
    img.dot(0, RAIL_Y - 2, hex(0x7a5c2c), PX, 2);
    img.dot(
        0,
        CORNICE_H + 2,
        if variant == 0 {
            hex(0xc89a5a)
        } else {
            hex(0x5fb0b8)
        },
        PX,
        2,
    );
    wall_skirt(&mut img, hex(0x1e1614));
    wall_cornice(&mut img, hex(0x4a423c));
    grime(&mut img, &mut rng, 2);
    img
}

/// Service corridor: dark metal bays, hazard-striped foot, an amber conduit (lower on variant 1).
pub fn paint_wall_corridor(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wcorr{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(&mut img, &mut rng, hex(0x2c2224), hex(0x232022));
    for y in range(0, WALL_H, 42) {
        img.dot(0, y, ink(0x080607), PX, 1);
    }
    for x in [0, PX - 1] {
        img.dot(x, 0, ink(0x080607), 1, WALL_H);
    }
    for y in WALL_H - 14..WALL_H {
        for x in 0..PX {
            img.dot1(x, y, hazard_stripe(x, y, hex(0x6a5010), ink(0x100d0b)));
        }
    }
    let cy = 22 + variant * 10;
    img.dot(0, cy, hex(0x2a1a0c), PX, 3);
    img.dot(0, cy, flat_or(hex(0x9a6a2a), hex(0x6a5010)), PX, 1);
    for x in [5, 21] {
        img.dot(x, cy - 1, ink(0x140d06), 3, 5);
    }
    wall_cornice(&mut img, hex(0x262224));
    grime(&mut img, &mut rng, 5);
    img
}

/// Foundry wall with glowing ember cracks.
pub fn paint_wall_inferno(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("winferno{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(&mut img, &mut rng, hex(0x2a1210), hex(0x1c1214));
    for i in 0..6 {
        let x = rng.randi_range(2, 29);
        let y = rng.randi_range(14, WALL_H - 20);
        let len = rng.randi_range(10, 26);
        let ember = if i % 2 == 0 {
            hex(0xc8481a)
        } else {
            hex(0x7a2a14)
        };
        crack(&mut img, &mut rng, x, y, len, flat_or(ember, hex(0xc8481a)));
    }
    wall_cornice(&mut img, hex(0x261614));
    wall_skirt(&mut img, hex(0x1a0e0e));
    img
}

/// Cave wall: uneven strata, weathering streaks, per-variant ember cracks, a ragged dark top.
pub fn paint_wall_rock(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("rockwall{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let base = if variant % 2 == 0 {
        hex(0x2a2224)
    } else {
        hex(0x2f2325)
    };
    let light = mul(base, 1.4);
    noise(&mut img, 0, 0, PX, WALL_H, base, 0.16, &mut rng, 2);
    let mut y = 0;
    while y < WALL_H {
        let band = rng.randi_range(6, 18);
        let tone = rng.randf_range(0.78, 1.18);
        for x in 0..PX {
            if rng.randf() < 0.7 {
                let c = mul(base, tone * rng.randf_range(0.9, 1.1));
                // flat: each stratum is the wall tone or the light one, by its band tone
                let stratum = if tone > 1.0 { light } else { base };
                img.dot(x, y, flat_or(c, stratum), 1, band);
            }
        }
        img.dot(0, y, ink(0x0d0909), PX, 1);
        y += band;
    }
    for _ in 0..10 {
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, WALL_H - 8),
            ink(0x0c0808),
            1,
            rng.randi_range(4, 20),
        );
    }
    let cracks = [3, 0, 2, 4, 1, 0, 3, 2][(variant % 8) as usize];
    for i in 0..cracks {
        let (x, y, len) = (
            rng.randi_range(2, 29),
            rng.randi_range(12, WALL_H - 16),
            rng.randi_range(8, 24),
        );
        let ember = if i % 2 == 0 {
            hex(0xc8481a)
        } else {
            hex(0x7a2a14)
        };
        crack(&mut img, &mut rng, x, y, len, flat_or(ember, hex(0xc8481a)));
    }
    for x in 0..PX {
        img.dot(x, 0, ink(0x0c0809), 1, rng.randi_range(2, 9));
    }
    img.dot(0, WALL_H - 3, ink(0x100b0c), PX, 3);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(p: [u8; 3]) -> f64 {
        0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64
    }

    /// Distinct opaque colours, and the luma bands they fall in (a new band starts where luma rises more than 20).
    fn colours_and_bands(img: &Img) -> (usize, usize) {
        let mut seen: Vec<[u8; 3]> = img
            .px
            .chunks_exact(4)
            .filter(|p| p[3] == 255)
            .map(|p| [p[0], p[1], p[2]])
            .collect();
        seen.sort_unstable();
        seen.dedup();
        let mut lumas: Vec<f64> = seen.iter().map(|c| luma(*c)).collect();
        lumas.sort_by(|a, b| a.total_cmp(b));
        let bands = 1 + lumas.windows(2).filter(|w| w[1] - w[0] > 20.0).count();
        (seen.len(), bands)
    }

    /// One painter in the flat style: 3-6 colours in 2-3 luma bands, not the painted look, the same bytes twice.
    fn check_flat(name: &str, painter: fn(i32) -> Img, variant: i32) {
        let flat_img = with_style(true, || painter(variant));
        let again = with_style(true, || painter(variant));
        let painted = with_style(false, || painter(variant));
        assert_eq!(flat_img.px, again.px, "{name}/{variant}: flat output differs between runs");
        assert_ne!(flat_img.px, painted.px, "{name}/{variant}: flat output equals the painted one");
        let (colours, bands) = colours_and_bands(&flat_img);
        assert!((3..=6).contains(&colours), "{name}/{variant}: {colours} colours");
        assert!((2..=3).contains(&bands), "{name}/{variant}: {bands} luma bands");
    }

    #[test]
    fn flat_panel_walls() {
        for v in 0..3 {
            check_flat("panel", paint_wall_panel, v);
        }
    }

    #[test]
    fn flat_wallpaper_walls() {
        for v in 0..2 {
            check_flat("paper", paint_wall_wallpaper, v);
        }
    }

    #[test]
    fn flat_brick_walls() {
        for v in 0..2 {
            check_flat("brick", paint_wall_brick, v);
        }
    }

    #[test]
    fn flat_stone_walls() {
        for v in 0..2 {
            check_flat("stone", paint_wall_stone, v);
        }
    }

    #[test]
    fn flat_hedge_walls() {
        for v in 0..2 {
            check_flat("hedge", paint_wall_hedge, v);
        }
    }

    #[test]
    fn flat_office_walls() {
        for v in 0..2 {
            check_flat("office", paint_wall_office, v);
        }
    }

    #[test]
    fn flat_hospital_walls() {
        for v in 0..2 {
            check_flat("hospital", paint_wall_hospital, v);
        }
    }

    #[test]
    fn flat_lab_walls() {
        for v in 0..2 {
            check_flat("lab", paint_wall_lab, v);
        }
    }

    #[test]
    fn flat_glasshouse_walls() {
        for v in 0..2 {
            check_flat("glasshouse", paint_wall_glasshouse, v);
        }
    }

    #[test]
    fn flat_mall_walls() {
        for v in 0..2 {
            check_flat("mall", paint_wall_mall, v);
        }
    }

    #[test]
    fn flat_corridor_walls() {
        for v in 0..2 {
            check_flat("corridor", paint_wall_corridor, v);
        }
    }

    #[test]
    fn flat_inferno_walls() {
        check_flat("inferno", paint_wall_inferno, 0);
    }

    #[test]
    fn flat_rock_walls() {
        for v in 0..8 {
            check_flat("rockwall", paint_wall_rock, v);
        }
    }
}
