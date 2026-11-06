//! Wall painters of ref_textures.gd (32 x 128: one storey of wall, cornice at the top row, skirting at the bottom):
//! panel, wallpaper (damask over wainscot), brick, stone, hedge, office, hospital, lab, glasshouse, mall, corridor,
//! inferno, cave rock.

use super::canvas::*;
use super::floors::metal;
use super::rng::GodotRng;

/// Industrial plating: riveted plates, maroon lower half; 1 adds a brass pipe, 2 a vent grille.
pub fn paint_wall_panel(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wpanel{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(&mut img, &mut rng, hex(0x32202a), metal());
    for y in range(0, WALL_H, 32) {
        img.dot(0, y, hex(0x0a0809), PX, 1);
        img.dot(0, y + 1, mul(metal(), 1.7), PX, 1);
    }
    img.dot(0, 0, hex(0x0a0809), 1, WALL_H);
    img.dot(PX - 1, 0, hex(0x0a0809), 1, WALL_H);
    for y in range(4, WALL_H, 32) {
        for x in [3, PX - 5] {
            rivet(&mut img, x, y, metal());
        }
    }
    rail(&mut img);
    skirt(&mut img, hex(0x2a1418));
    cornice(&mut img, metal());
    if variant == 1 {
        let px = 4 + rng.randi_range(0, 18);
        img.dot(px, 0, hex(0x3a2a14), 3, WALL_H);
        img.dot(px, 0, hex(0x8a6a2c), 1, WALL_H);
        for y in range(12, WALL_H, 36) {
            img.dot(px - 1, y, hex(0x1a1209), 5, 3);
        }
    }
    if variant == 2 {
        for gy in range(40, 62, 3) {
            img.dot(8, gy, hex(0x070506), 16, 1);
        }
        img.dot(7, 38, hex(0x5a4a40), 18, 1);
        img.dot(7, 62, hex(0x5a4a40), 18, 1);
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
                flower(&mut img, cx, cy, hex(0x4a2634), hex(0x7a5a2c));
            }
        }
        for dx in 0..PX {
            let cy2 = CORNICE_H + 22 + row;
            if cy2 < RAIL_Y - 4 && (dx % 8) < 2 {
                img.dot1(dx, cy2 + (dx % 8), hex(0x4a2230));
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
        img.dot(3, top, mul(wood, 0.45), PX - 6, h);
        img.dot(3, top, mul(wood, 1.6), PX - 6, 1);
        img.dot(3, top, mul(wood, 1.3), 1, h);
        img.dot(PX - 4, top, hex(0x060303), 1, h);
    }
    rail(&mut img);
    skirt(&mut img, wood);
    cornice(&mut img, hex(0x3a2820));
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
    let mortar = hex(0x0e0a0a);
    noise(&mut img, 0, 0, PX, WALL_H, base, 0.13, &mut rng, 1);
    let mut row = 0;
    for y in range(0, WALL_H, 6) {
        img.dot(0, y, mortar, PX, 1);
        let off = if row % 2 == 0 { 0 } else { 8 };
        for x in range(off, PX + 8, 16) {
            img.dot(x % PX, y, mortar, 1, 6);
            img.dot(
                (x + 2) % PX,
                y + 2,
                mul(base, rng.randf_range(0.7, 1.3)),
                6,
                3,
            );
        }
        row += 1;
    }
    for _ in 0..5 {
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, WALL_H - 24),
            hex(0x141018),
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
    img.dot(0, WALL_H - 4, hex(0x0a0707), PX, 4);
    cornice(&mut img, hex(0x2a2022));
    img
}

/// Ashlar courses of 10/14/18 px with water stains.
pub fn paint_wall_stone(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wstone{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let base = hex(0x2c2a2c);
    let mortar = hex(0x0b0a0c);
    noise(&mut img, 0, 0, PX, WALL_H, base, 0.12, &mut rng, 2);
    let mut y = 0;
    while y < WALL_H {
        let h = [10, 14, 18][(rng.randi() % 3) as usize];
        img.dot(0, y, mortar, PX, 1);
        img.dot(rng.randi_range(4, 27), y, mortar, 1, h);
        img.dot(0, y + 1, mul(base, 1.4), PX, 1);
        y += h;
    }
    for _ in 0..4 {
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, 60),
            hex(0x17141a),
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
    img.dot(0, WALL_H - 5, hex(0x0b090a), PX, 5);
    cornice(&mut img, hex(0x2a2628));
    img
}

/// Ivy over a brass-banded masonry plinth, dusky flowers and hanging vines.
pub fn paint_wall_hedge(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("whedge{variant}"));
    let mut img = Img::new(PX, WALL_H);
    noise(&mut img, 0, 0, PX, WALL_H, hex(0x26301f), 0.22, &mut rng, 1);
    for _ in 0..28 {
        let (cx, cy, r) = (
            rng.randi_range(0, 31),
            rng.randi_range(4, WALL_H - 6),
            rng.randi_range(2, 5),
        );
        leaf_cluster(&mut img, &mut rng, cx, cy, r, hex(0x2a3a26));
    }
    for i in 0..10 {
        let fx = rng.randi_range(2, 28);
        let fy = rng.randi_range(14, WALL_H - 20);
        img.dot(
            fx,
            fy,
            if i % 2 == 0 {
                hex(0x8a2e52)
            } else {
                hex(0xb09a88)
            },
            3,
            3,
        );
    }
    img.dot(0, WALL_H - 30, hex(0x3a3030), PX, 30);
    for by in range(WALL_H - 30, WALL_H, 10) {
        img.dot(0, by, hex(0x0a0808), PX, 1);
    }
    img.dot(0, WALL_H - 31, hex(0x8a6a2c), PX, 1);
    skirt(&mut img, hex(0x2a2224));
    for _ in 0..4 {
        let vx = rng.randi_range(1, 30);
        img.dot(
            vx,
            rng.randi_range(0, 20),
            hex(0x162016),
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
    rail(&mut img);
    skirt(&mut img, hex(0x1c1214));
    cornice(&mut img, hex(0x2a2828));
    if variant == 1 {
        for _ in 0..3 {
            let x = rng.randi_range(3, 22);
            let y = rng.randi_range(18, 52);
            img.dot(x, y, hex(0x4c4842), 5, 6);
            img.dot1(x + 2, y, hex(0x7a2a2a));
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
    img.dot(0, RAIL_Y - 4, hex(0x0a0c0c), PX, 1);
    img.dot(0, RAIL_Y - 3, hex(0x2f6a74), PX, 4);
    img.dot(0, RAIL_Y + 1, hex(0x173a40), PX, 1);
    skirt(&mut img, hex(0x2a3a38));
    cornice(&mut img, hex(0x4a4c48));
    if variant == 1 {
        img.dot(0, RAIL_Y + 14, hex(0x2c5f8a), PX, 2);
        img.dot(10, 30, hex(0x1b3a5c), 12, 6);
        img.dot(12, 32, hex(0xc8cfd0), 8, 2);
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
        img.dot(0, y, hex(0x1e2224), PX, 1);
    }
    img.dot(PX - 1, 0, hex(0x1e2224), 1, WALL_H);
    img.dot(0, RAIL_Y, hex(0x1d8a96), PX, 2);
    for y in [CORNICE_H + 4, CORNICE_H + 44] {
        rivet(&mut img, 3, y, hex(0x5a5f63));
        rivet(&mut img, PX - 5, y, hex(0x5a5f63));
    }
    if variant == 1 {
        for y in WALL_H - 12..WALL_H {
            for x in 0..PX {
                img.dot1(x, y, hazard_stripe(x, y, hex(0x7a5c12), hex(0x14110e)));
            }
        }
    } else {
        skirt(&mut img, hex(0x2c3134));
    }
    cornice(&mut img, hex(0x3a3f42));
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
    cornice(&mut img, bar);
    img
}

/// Arcade wall: cream plaster over a tiled dado, a lit cove strip under the cornice (amber on 0, teal on 1).
pub fn paint_wall_mall(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wmall{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(&mut img, &mut rng, hex(0x3a2c2a), hex(0x5c524a));
    for y in range(RAIL_Y + 2, WALL_H - SKIRT_H, 8) {
        img.dot(0, y, hex(0x1e1614), PX, 1);
    }
    for x in [0, 16] {
        img.dot(x, RAIL_Y, hex(0x1e1614), 1, WALL_H - RAIL_Y - SKIRT_H);
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
    skirt(&mut img, hex(0x1e1614));
    cornice(&mut img, hex(0x4a423c));
    grime(&mut img, &mut rng, 2);
    img
}

/// Service corridor: dark metal bays, hazard-striped foot, an amber conduit (lower on variant 1).
pub fn paint_wall_corridor(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wcorr{variant}"));
    let mut img = Img::new(PX, WALL_H);
    wall_base(&mut img, &mut rng, hex(0x2c2224), hex(0x232022));
    for y in range(0, WALL_H, 42) {
        img.dot(0, y, hex(0x080607), PX, 1);
    }
    for x in [0, PX - 1] {
        img.dot(x, 0, hex(0x080607), 1, WALL_H);
    }
    for y in WALL_H - 14..WALL_H {
        for x in 0..PX {
            img.dot1(x, y, hazard_stripe(x, y, hex(0x6a5010), hex(0x100d0b)));
        }
    }
    let cy = 22 + variant * 10;
    img.dot(0, cy, hex(0x2a1a0c), PX, 3);
    img.dot(0, cy, hex(0x9a6a2a), PX, 1);
    for x in [5, 21] {
        img.dot(x, cy - 1, hex(0x140d06), 3, 5);
    }
    cornice(&mut img, hex(0x262224));
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
        crack(
            &mut img,
            &mut rng,
            x,
            y,
            len,
            if i % 2 == 0 {
                hex(0xc8481a)
            } else {
                hex(0x7a2a14)
            },
        );
    }
    cornice(&mut img, hex(0x261614));
    skirt(&mut img, hex(0x1a0e0e));
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
    noise(&mut img, 0, 0, PX, WALL_H, base, 0.16, &mut rng, 2);
    let mut y = 0;
    while y < WALL_H {
        let band = rng.randi_range(6, 18);
        let tone = rng.randf_range(0.78, 1.18);
        for x in 0..PX {
            if rng.randf() < 0.7 {
                let c = mul(base, tone * rng.randf_range(0.9, 1.1));
                img.dot(x, y, c, 1, band);
            }
        }
        img.dot(0, y, hex(0x0d0909), PX, 1);
        y += band;
    }
    for _ in 0..10 {
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, WALL_H - 8),
            hex(0x0c0808),
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
        crack(
            &mut img,
            &mut rng,
            x,
            y,
            len,
            if i % 2 == 0 {
                hex(0xc8481a)
            } else {
                hex(0x7a2a14)
            },
        );
    }
    for x in 0..PX {
        img.dot(x, 0, hex(0x0c0809), 1, rng.randi_range(2, 9));
    }
    img.dot(0, WALL_H - 3, hex(0x100b0c), PX, 3);
    img
}
