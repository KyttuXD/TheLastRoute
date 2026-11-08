//! Floor painters of ref_textures.gd (32 x 32, one texture per 1 m cell): metal, floral, star, parquet, stone, check,
//! carpet, carpet tile, vinyl, concrete, hazard, terrazzo, epoxy, terracotta, runner (+ three-strip parts), safe-room
//! pad, grate and cave rock.

use super::canvas::*;
use super::rng::GodotRng;
use std::f64::consts::TAU;

/// RefLook palette entries the painters use.
pub fn metal() -> Col {
    hex(0x2a2627)
}
pub fn stone() -> Col {
    hex(0x3a3532)
}
pub fn wood() -> Col {
    hex(0x3a2418)
}

/// Flat style tones of the steel family: the dark seam band and the lit edge band.
fn flat_seam() -> Col {
    hex(0x141012)
}
fn flat_edge() -> Col {
    hex(0x564e50)
}

/// Bevelled riveted plate; variant 3 has a drain grille. Flat: the bevel is two bands (lit top/left, dark seam
/// bottom/right) and the rivets are gone (`rivet` is a no-op in the flat style).
pub fn paint_metal(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("metal{variant}"));
    let mut img = Img::new(PX, PX);
    let base = mul(metal(), 0.86);
    noise(&mut img, 0, 0, PX, PX, base, 0.14, &mut rng, 1);
    for _ in 0..3 {
        img.dot(
            rng.randi_range(1, 28),
            rng.randi_range(1, 28),
            flat_or(mul(base, 0.78), flat_seam()),
            rng.randi_range(2, 4),
            1,
        );
    }
    // lit top/left edges, dark seams bottom/right
    img.dot(0, 0, flat_or(mul(base, 2.0), flat_edge()), PX, 1);
    img.dot(0, 1, flat_or(mul(base, 1.4), flat_edge()), PX, 1);
    img.dot(0, 0, flat_or(mul(base, 1.6), flat_edge()), 1, PX);
    img.dot(PX - 1, 0, flat_or(hex(0x080607), flat_seam()), 1, PX);
    img.dot(0, PX - 1, flat_or(hex(0x080607), flat_seam()), PX, 1);
    for (x, y) in [(3, 3), (26, 3), (3, 26), (26, 26)] {
        rivet(&mut img, x, y, base);
    }
    if rng.randf() < 0.35 {
        let (x, y, len) = (
            rng.randi_range(5, 25),
            rng.randi_range(5, 25),
            rng.randi_range(4, 9),
        );
        crack(&mut img, &mut rng, x, y, len, flat_or(hex(0x1c1819), flat_seam()));
    }
    if rng.randf() < 0.5 {
        for _ in 0..4 {
            img.dot1(
                rng.randi_range(2, 29),
                rng.randi_range(2, 29),
                hex(0x4a2c14),
            );
        }
    }
    if rng.randf() < 0.4 {
        img.dot(
            rng.randi_range(4, 26),
            5,
            flat_or(hex(0x3b3836), flat_edge()),
            1,
            rng.randi_range(8, 18),
        );
    }
    if variant == 3 {
        for gy in range(11, 22, 2) {
            img.dot(9, gy, flat_or(hex(0x0b090a), flat_seam()), 14, 1);
        }
    }
    img
}

pub fn paint_floral(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("floral{variant}"));
    let mut img = Img::new(PX, PX);
    let alt = variant == 1;
    mosaic(
        &mut img,
        if alt { hex(0x2a1526) } else { hex(0x2e1729) },
        &mut rng,
    );
    let petal = hex(0x6e5c52);
    let core = hex(0x94601f);
    flower(&mut img, if alt { 22 } else { 9 }, 9, petal, core);
    flower(
        &mut img,
        if alt { 9 } else { 24 },
        24,
        flat_or(mul(petal, 0.85), petal),
        core,
    );
    for _ in 0..6 {
        img.dot(
            rng.randi_range(1, 30),
            rng.randi_range(1, 30),
            hex(0x3d2138),
            2,
            1,
        );
    }
    img
}

pub fn paint_star(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("star{variant}"));
    let mut img = Img::new(PX, PX);
    let alt = variant == 1;
    mosaic(
        &mut img,
        if alt { hex(0x2b1220) } else { hex(0x2f1424) },
        &mut rng,
    );
    star(&mut img, if alt { 22 } else { 9 }, 10, 5, hex(0x7a561e));
    star(&mut img, if alt { 8 } else { 23 }, 25, 3, hex(0x55391a));
    let (rx, ry) = if alt { (10.0, 24.0) } else { (24.0, 8.0) };
    for a in 0..40 {
        let t = a as f64 / 40.0 * TAU;
        img.dot1(
            roundi(rx + t.cos() * 5.0),
            roundi(ry + t.sin() * 5.0),
            hex(0x5a1d26),
        );
    }
    img
}

/// 8 px planks in four bands.
pub fn paint_parquet(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("parquet{variant}"));
    let mut img = Img::new(PX, PX);
    let base = wood();
    for band in 0..4 {
        let y0 = band * 8;
        let shade = rng.randf_range(0.8, 1.25);
        // flat: two plank tones, light and dark
        let shade = if flat() {
            if shade < 1.0 {
                0.84
            } else {
                1.14
            }
        } else {
            shade
        };
        noise(&mut img, 0, y0, PX, 8, mul(base, shade), 0.1, &mut rng, 1);
        img.dot(0, y0 + 7, hex(0x0c0705), PX, 1);
        let joint = (rng.randi() % 20) as i32 + 4 + (band % 2) * 6;
        img.dot(joint % PX, y0, hex(0x0c0705), 1, 8);
        for _ in 0..3 {
            img.dot(
                rng.randi_range(0, 30),
                y0 + rng.randi_range(1, 6),
                flat_or(mul(base, 0.7), hex(0x0c0705)),
                rng.randi_range(3, 7),
                1,
            );
        }
    }
    img.dot(0, 0, mul(base, 1.5), PX, 1);
    img
}

/// Wet flagstones.
pub fn paint_stone(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("stone{variant}"));
    let mut img = Img::new(PX, PX);
    let base = stone();
    noise(&mut img, 0, 0, PX, PX, base, 0.12, &mut rng, 2);
    let mortar = hex(0x0d0b0b);
    let split = rng.randi_range(10, 20);
    img.dot(0, split, mortar, PX, 1);
    img.dot(rng.randi_range(6, 26), 0, mortar, 1, split);
    img.dot(rng.randi_range(6, 26), split + 1, mortar, 1, PX - split);
    img.dot(0, 0, mortar, PX, 1);
    img.dot(0, 0, mortar, 1, PX);
    for i in 0..26 {
        let c = if i % 3 == 0 {
            hex(0x3e4a4c)
        } else {
            hex(0x141012)
        };
        img.dot1(rng.randi_range(0, 31), rng.randi_range(0, 31), c);
    }
    let (x, y, len) = (
        rng.randi_range(4, 26),
        rng.randi_range(4, 26),
        rng.randi_range(5, 10),
    );
    crack(&mut img, &mut rng, x, y, len, hex(0x0f0c0c));
    img
}

/// 16 px checker tiles.
pub fn paint_check(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("check{variant}"));
    let mut img = Img::new(PX, PX);
    let a = hex(0x1d1618);
    let b = if variant == 0 {
        hex(0x3a2a2c)
    } else {
        hex(0x4a2024)
    };
    for cy in 0..2 {
        for cx in 0..2 {
            noise(
                &mut img,
                cx * 16,
                cy * 16,
                16,
                16,
                if (cx + cy) % 2 == 0 { a } else { b },
                0.1,
                &mut rng,
                1,
            );
        }
    }
    let g = hex(0x0a0708);
    img.dot(0, 0, g, PX, 1);
    img.dot(0, 0, g, 1, PX);
    img.dot(15, 0, g, 1, PX);
    img.dot(0, 15, g, PX, 1);
    img
}

/// Patterned carpet: 0 gothic red diamond lattice, 1 hotel teal medallions, 2 library green border, 3 plum lounge.
pub fn paint_carpet(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("carpet{variant}"));
    let mut img = Img::new(PX, PX);
    let bases = [0x3a1014, 0x0f2a2c, 0x132a17, 0x2a1430];
    let accents = [0x6e4a1c, 0x6a5420, 0x2e4a24, 0x5a3a52];
    let base = hex(bases[variant as usize]);
    let accent = hex(accents[variant as usize]);
    noise(&mut img, 0, 0, PX, PX, base, 0.1, &mut rng, 2);
    match variant {
        0 => {
            for i in 0..PX {
                img.dot1(i, (i + 16) % PX, accent);
                img.dot1(i, (PX - i + 15) % PX, accent);
            }
            img.dot(15, 15, mul(accent, 1.3), 2, 2);
        }
        1 => {
            for a in 0..32 {
                let t = a as f64 / 32.0 * TAU;
                img.dot1(
                    roundi(16.0 + t.cos() * 7.0),
                    roundi(16.0 + t.sin() * 7.0),
                    accent,
                );
            }
            img.dot(15, 15, mul(accent, 1.2), 2, 2);
            img.dot(0, 0, mul(accent, 0.8), 2, 2);
        }
        2 => {
            img.dot(1, 1, accent, PX - 2, 1);
            img.dot(1, PX - 2, accent, PX - 2, 1);
            img.dot(1, 1, accent, 1, PX - 2);
            img.dot(PX - 2, 1, accent, 1, PX - 2);
        }
        _ => {
            for y in range(4, PX, 8) {
                for x in range(4, PX, 8) {
                    img.dot(x, y, accent, 2, 2);
                }
            }
        }
    }
    if flat() {
        // flat: the shading goes; a light medallion at the centre is the third tone
        img.dot(15, 15, mul(accent, 1.5), 2, 2);
    }
    img
}

/// Office carpet tiles: four 16 px squares of alternating pile.
pub fn paint_carpet_tile(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("carpet_tile{variant}"));
    let mut img = Img::new(PX, PX);
    let a = if variant == 0 {
        hex(0x2a2b30)
    } else {
        hex(0x23262e)
    };
    let b = mul(a, 1.12);
    for cy in 0..2 {
        for cx in 0..2 {
            noise(
                &mut img,
                cx * 16,
                cy * 16,
                16,
                16,
                if (cx + cy) % 2 == 0 { a } else { b },
                0.08,
                &mut rng,
                1,
            );
        }
    }
    // flat: the tile seams are one dark band; the two tiles stay as they are
    let line = flat_or(mul(a, 0.6), mul(a, 0.4));
    let inner = flat_or(mul(a, 0.7), mul(a, 0.4));
    img.dot(0, 0, line, PX, 1);
    img.dot(0, 0, line, 1, PX);
    img.dot(16, 0, inner, 1, PX);
    img.dot(0, 16, inner, PX, 1);
    img
}

/// Lab / clinic sheet vinyl.
pub fn paint_vinyl(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("vinyl{variant}"));
    let mut img = Img::new(PX, PX);
    let base = if variant == 0 {
        hex(0x3c4240)
    } else {
        hex(0x3a3c44)
    };
    noise(&mut img, 0, 0, PX, PX, base, 0.05, &mut rng, 2);
    for i in 0..18 {
        let c = mul(base, if i % 2 == 0 { 0.75 } else { 1.15 });
        let (x, y) = (rng.randi_range(0, 31), rng.randi_range(0, 31));
        if !flat() {
            img.dot1(x, y, c);
        }
    }
    if flat() {
        // flat: a dark sheet seam on the top and left edges, a light lip inside each; no flecks
        img.dot(0, 0, mul(base, 0.5), PX, 1);
        img.dot(0, 1, mul(base, 1.2), PX, 1);
        img.dot(0, 0, mul(base, 0.5), 1, PX);
        img.dot(1, 0, mul(base, 1.2), 1, PX);
    } else {
        img.dot(0, 0, mul(base, 0.7), PX, 1);
    }
    img
}

/// Bare concrete with a saw-cut joint.
pub fn paint_concrete(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("concrete{variant}"));
    let mut img = Img::new(PX, PX);
    let base = if variant == 0 {
        hex(0x2c2a2a)
    } else {
        hex(0x272828)
    };
    noise(&mut img, 0, 0, PX, PX, base, 0.08, &mut rng, 2);
    noise(&mut img, 0, 0, PX, PX, base, 0.05, &mut rng, 4);
    // flat: the saw cut, the pits and the crack are one dark seam tone, with a light lip under each saw cut
    let seam = |painted: f64| flat_or(mul(base, painted), mul(base, 0.45));
    img.dot(0, 0, seam(0.55), PX, 1);
    if variant == 1 {
        img.dot(0, 16, seam(0.6), PX, 1);
    }
    if flat() {
        img.dot(0, 1, mul(base, 1.5), PX, 1);
        if variant == 1 {
            img.dot(0, 17, mul(base, 1.5), PX, 1);
        }
    }
    for _ in 0..3 {
        img.dot(
            rng.randi_range(2, 26),
            rng.randi_range(2, 26),
            seam(0.8),
            rng.randi_range(2, 4),
            rng.randi_range(1, 3),
        );
    }
    let (x, y, len) = (
        rng.randi_range(4, 26),
        rng.randi_range(4, 26),
        rng.randi_range(4, 8),
    );
    crack(&mut img, &mut rng, x, y, len, seam(0.5));
    img
}

/// metal0 with a 7 px hazard stripe band along the top edge.
pub fn paint_hazard(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("hazard{variant}"));
    let mut img = paint_metal(0);
    for y in 0..7 {
        for x in 0..PX {
            img.dot1(
                x,
                y,
                hazard_stripe(x, y, hex(0x7a5c12), flat_or(hex(0x14110e), flat_seam())),
            );
        }
    }
    img.dot(0, 7, flat_or(hex(0x070606), flat_seam()), PX, 1);
    if rng.randf() < 0.5 {
        crack(&mut img, &mut rng, 4, 3, 6, flat_or(hex(0x0a0808), flat_seam()));
    }
    img
}

/// Mall terrazzo with brass divider strips on the top and left edges.
pub fn paint_terrazzo(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("terrazzo{variant}"));
    let mut img = Img::new(PX, PX);
    let base = if variant == 0 {
        hex(0x5a5048)
    } else {
        hex(0x4c4a50)
    };
    noise(&mut img, 0, 0, PX, PX, base, 0.04, &mut rng, 2);
    let chips = [hex(0x8a8278), hex(0x2e2a28), hex(0x7a5a44), hex(0x6a7470)];
    // flat: the chips fall into a pale and a dark tone
    let flat_chips = [hex(0x8a8278), hex(0x2e2a28)];
    for i in 0..40 {
        let chip = if flat() {
            flat_chips[(i % 2) as usize]
        } else {
            chips[(i % 4) as usize]
        };
        img.dot(
            rng.randi_range(0, PX - 2),
            rng.randi_range(0, PX - 2),
            chip,
            1 + i % 2,
            1,
        );
    }
    // flat: the divider strips take the pale chip tone, so the floor stays in three tones
    let strip = flat_or(hex(0x7a5c2c), flat_chips[0]);
    img.dot(0, 0, strip, PX, 1);
    img.dot(0, 0, strip, 1, PX);
    img
}

/// Lab epoxy on a metre grid with cyan joint markers; variant 1 adds a keep-clear hatch.
pub fn paint_epoxy(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("epoxy{variant}"));
    let mut img = Img::new(PX, PX);
    let base = hex(0x4b5155);
    noise(&mut img, 0, 0, PX, PX, base, 0.04, &mut rng, 2);
    img.dot(0, 0, mul(base, 0.62), PX, 1);
    img.dot(0, 0, mul(base, 0.62), 1, PX);
    img.dot(0, 0, hex(0x2a8f9c), 3, 1);
    img.dot(0, 0, hex(0x2a8f9c), 1, 3);
    if variant == 1 {
        for x in 0..PX {
            if (x >> 2) & 1 == 0 {
                img.dot(x, PX - 4, hex(0x7a5c12), 1, 3);
            }
        }
    }
    img
}

/// Terracotta quarry tiles in dark grout. (Seed key is "terra<v>".)
pub fn paint_terracotta(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("terra{variant}"));
    let mut img = Img::new(PX, PX);
    img.dot(0, 0, hex(0x1c1612), PX, PX);
    for ty in 0..2 {
        for tx in 0..2 {
            let tone = if (tx + ty + variant) % 2 == 0 {
                hex(0x6a3a26)
            } else {
                hex(0x5c3424)
            };
            noise(
                &mut img,
                tx * 16 + 1,
                ty * 16 + 1,
                14,
                14,
                tone,
                0.1,
                &mut rng,
                1,
            );
        }
    }
    for _ in 0..3 {
        img.dot(
            rng.randi_range(0, PX - 2),
            rng.randi_range(0, PX - 2),
            hex(0x2f4a22),
            2,
            1,
        );
    }
    img
}

fn runner_diamond(img: &mut Img) {
    let c = hex(0x9a7a50);
    for d in 0..7 {
        img.dot1(16 - d, 16 - (6 - d), c);
        img.dot1(16 + d, 16 - (6 - d), c);
        img.dot1(16 - d, 16 + (6 - d), c);
        img.dot1(16 + d, 16 + (6 - d), c);
    }
}

/// Single-cell maroon runner with brass guide lines top and bottom and a diamond.
pub fn paint_runner(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("runner{variant}"));
    let mut img = Img::new(PX, PX);
    mosaic(&mut img, hex(0x3a1522), &mut rng);
    img.dot(0, 3, hex(0x6e4e1c), PX, 1);
    img.dot(0, 5, hex(0x3e2a14), PX, 1);
    img.dot(0, PX - 4, hex(0x6e4e1c), PX, 1);
    img.dot(0, PX - 6, hex(0x3e2a14), PX, 1);
    runner_diamond(&mut img);
    img.dot(15, 15, hex(0xb97a2a), 3, 3);
    img
}

/// One strip of the three-cell runner: part 0 (left) has the brass edge at the top (v = 0 side), part 2 (right) at
/// the bottom, part 1 (centre) the diamond. `rotated` = the runnerz_* kinds (rotate 90 degrees counter-clockwise).
pub fn paint_runner_part(part: i32, rotated: bool) -> Img {
    let mut rng = GodotRng::from_key(&format!("runnerp{part}"));
    let mut img = Img::new(PX, PX);
    mosaic(&mut img, hex(0x3a1522), &mut rng);
    if part == 0 {
        img.dot(0, 3, hex(0x6e4e1c), PX, 1);
        img.dot(0, 5, hex(0x3e2a14), PX, 1);
    } else if part == 2 {
        img.dot(0, PX - 4, hex(0x6e4e1c), PX, 1);
        img.dot(0, PX - 6, hex(0x3e2a14), PX, 1);
    }
    if part == 1 {
        runner_diamond(&mut img);
        img.dot(15, 15, hex(0xb97a2a), 3, 3);
    }
    if rotated {
        img.rotate_ccw();
    }
    img
}

pub fn paint_runner_z(_variant: i32) -> Img {
    let mut img = paint_runner(0);
    img.rotate_ccw();
    img
}

/// Safe-room pad ring: metal0 with a teal inlay line 1 px inside the top/left and 2 px inside the bottom/right edges.
pub fn paint_safe(_variant: i32) -> Img {
    let mut img = paint_metal(0);
    let c = hex(0x1c8a8c);
    for i in 0..PX {
        img.dot1(i, 1, c);
        img.dot1(i, PX - 3, c);
        img.dot1(1, i, c);
        img.dot1(PX - 3, i, c);
    }
    img
}

/// Safe-room pad interior: plain metal0.
pub fn paint_safe_plain(_variant: i32) -> Img {
    paint_metal(0)
}

/// Safe-room pad centre: metal0 with a large teal cross.
pub fn paint_safe_mark(_variant: i32) -> Img {
    let mut img = paint_metal(0);
    let c = PX / 2;
    img.dot(c - 3, 4, hex(0x1c8a8c), 6, PX - 8);
    img.dot(4, c - 3, hex(0x1c8a8c), PX - 8, 6);
    // flat: the bright core line is the dark seam tone (a groove down the cross)
    let core = flat_or(hex(0x5cf0e0), flat_seam());
    img.dot(c - 1, 6, core, 2, PX - 12);
    img.dot(6, c - 1, core, PX - 12, 2);
    img
}

/// metal1 with a slotted grate.
pub fn paint_grate(_variant: i32) -> Img {
    let mut img = paint_metal(1);
    for gy in range(8, 26, 3) {
        img.dot(6, gy, flat_or(hex(0x0a0809), flat_seam()), 20, 2);
        img.dot(6, gy + 2, flat_or(hex(0x4a4244), flat_edge()), 20, 1);
    }
    img
}

/// Inferno cave basalt; variant 2 has ember cracks.
pub fn paint_rock_floor(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("rockfloor{variant}"));
    let mut img = Img::new(PX, PX);
    let base = if variant != 1 {
        hex(0x2c2527)
    } else {
        hex(0x30272a)
    };
    noise(&mut img, 0, 0, PX, PX, base, 0.2, &mut rng, 2);
    noise(&mut img, 0, 0, PX, PX, base, 0.1, &mut rng, 1);
    if flat() {
        // flat: hand-set basalt blocks cut by dark joints (no draws, so the chips and cracks stay put)
        let joint = hex(0x0c0809);
        for y in [10, 21] {
            img.dot(0, y, joint, PX, 1);
        }
        for (x, y0, y1) in [(12, 0, 10), (24, 0, 10), (6, 11, 21), (20, 11, 21), (14, 22, 32)] {
            img.dot(x, y0, joint, 1, y1 - y0);
        }
    }
    for i in 0..34 {
        let shade = if i % 3 != 0 {
            hex(0x0f0b0c)
        } else {
            // flat: a lit chip stays in the base's tone band
            flat_or(hex(0x4a403f), mul(base, 1.45))
        };
        img.dot(
            rng.randi_range(0, 31),
            rng.randi_range(0, 31),
            shade,
            rng.randi_range(1, 3),
            rng.randi_range(1, 2),
        );
    }
    for _ in 0..3 {
        let (x, y, len) = (
            rng.randi_range(2, 29),
            rng.randi_range(2, 29),
            rng.randi_range(5, 12),
        );
        crack(&mut img, &mut rng, x, y, len, hex(0x0c0809));
    }
    if variant == 2 {
        for i in 0..2 {
            let (x, y, len) = (
                rng.randi_range(2, 29),
                rng.randi_range(2, 29),
                rng.randi_range(5, 11),
            );
            crack(
                &mut img,
                &mut rng,
                x,
                y,
                len,
                if i == 0 { hex(0xc8481a) } else { hex(0x7a2a14) },
            );
        }
    }
    img
}

#[cfg(test)]
mod flat_tests {
    use super::super::{paint, paint_flat};
    use super::*;

    /// Luma of an opaque colour, 0..255.
    fn luma(c: [u8; 3]) -> f64 {
        0.299 * c[0] as f64 + 0.587 * c[1] as f64 + 0.114 * c[2] as f64
    }

    /// (distinct opaque colours, tone bands): sorted by luma, a band ends where the next luma is more than 20 higher.
    fn bands(img: &Img) -> (usize, usize) {
        let mut colours: Vec<[u8; 3]> = img
            .px
            .chunks_exact(4)
            .filter(|p| p[3] == 255)
            .map(|p| [p[0], p[1], p[2]])
            .collect();
        colours.sort_unstable();
        colours.dedup();
        let mut lumas: Vec<f64> = colours.iter().map(|c| luma(*c)).collect();
        lumas.sort_by(f64::total_cmp);
        let bands = 1 + lumas.windows(2).filter(|w| w[1] - w[0] > 20.0).count();
        (colours.len(), bands)
    }

    /// Each kind's flat output: 3-6 colours in 2-3 tone bands, different from the painted one, the same bytes twice.
    fn check_family(kinds: &[&str]) {
        for kind in kinds {
            let flat = paint_flat("floor", kind, 0).unwrap();
            assert_eq!(flat, paint_flat("floor", kind, 0).unwrap(), "{kind}: not deterministic");
            assert_ne!(flat, paint("floor", kind, 0).unwrap(), "{kind}: flat equals painted");
            let img = Img {
                w: flat.0 as i32,
                h: flat.1 as i32,
                px: flat.2,
            };
            let (colours, tone_bands) = bands(&img);
            assert!((3..=6).contains(&colours), "{kind}: {colours} colours");
            assert!((2..=3).contains(&tone_bands), "{kind}: {tone_bands} tone bands");
        }
    }

    #[test]
    fn flat_metal_family() {
        check_family(&["metal0", "metal1", "metal2", "metal3"]);
    }

    #[test]
    fn flat_epoxy_family() {
        check_family(&["epoxy0", "epoxy1"]);
    }

    #[test]
    fn flat_grate_family() {
        check_family(&["grate"]);
    }

    #[test]
    fn flat_hazard_family() {
        check_family(&["hazard"]);
    }

    #[test]
    fn flat_carpet_family() {
        check_family(&[
            "carpet_red",
            "carpet_teal",
            "carpet_green",
            "carpet_plum",
            "carpet_tile0",
            "carpet_tile1",
        ]);
    }

    #[test]
    fn flat_concrete_family() {
        check_family(&["concrete0", "concrete1"]);
    }

    #[test]
    fn flat_vinyl_family() {
        check_family(&["vinyl0", "vinyl1"]);
    }

    #[test]
    fn flat_sand_family() {
        check_family(&["sand0", "sand1", "wetsand0"]);
    }

    #[test]
    fn flat_rock_family() {
        check_family(&["rock0", "rock1", "rock2"]);
    }
}
