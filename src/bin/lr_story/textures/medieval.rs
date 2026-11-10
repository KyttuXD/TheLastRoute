//! Medieval surfaces for the old-town packs: timber-frame and ashlar walls (32 x 128), cobble and dirt floors and a
//! beamed ceiling (32 x 32). Same painter conventions as walls.rs / floors.rs / ceilings.rs: a seeded `GodotRng` keyed
//! by the kind name, `Img` out, tileable with repeat. Each kind has a flat version (2-3 tones, no grain) that
//! `paint_named` picks when `--flat-textures` is on.
//!
//! Names (see `paint_named`): "timber0", "stone0" (the ashlar wall; the plain wall table already has an unrelated
//! "stone0", so the table key of this one is "ashlar0"), "cobble0", "dirt0", "beams0". Variant 1 exists for timber,
//! ashlar and cobble.

use super::canvas::{hex, mul, noise, Col, Img, PX, WALL_H};
use super::rng::GodotRng;
use super::{flat_textures, Style};
use std::f64::consts::TAU;

// -- helpers ------------------------------------------------------------------------------------------------------------

/// Dot with wrap-around in x and y so patterns tile.
fn wdot(img: &mut Img, x: i32, y: i32, c: Col) {
    img.dot1(x.rem_euclid(img.w), y.rem_euclid(img.h), c);
}

fn variant_pick(variant: i32, a: u32, b: u32) -> Col {
    hex(if variant == 0 { a } else { b })
}

// -- timber wall --------------------------------------------------------------------------------------------------------

const POST_W: i32 = 3;
const PLATE_H: i32 = 6;
const SILL_H: i32 = 8;
const MID_Y: i32 = 62;
const MID_H: i32 = 4;

fn is_rail_row(y: i32) -> bool {
    y < PLATE_H || y >= WALL_H - SILL_H || (MID_Y..MID_Y + MID_H).contains(&y)
}

/// True when the texel is on a frame member (post, plate, mid rail, sill) or a brace.
fn timber_beam_at(x: i32, y: i32) -> bool {
    if y < 0 || y >= WALL_H {
        return false;
    }
    if x < POST_W || x >= PX - POST_W || is_rail_row(y) {
        return true;
    }
    // Braces: the upper panel falls left to right, the lower panel right to left; about 3 texels thick.
    let (top, bot) = (PLATE_H, MID_Y);
    if y >= top && y < bot {
        let t = (y - top) as f64 / (bot - top) as f64;
        let cx = POST_W as f64 + t * (PX - 2 * POST_W) as f64;
        if (x as f64 - cx).abs() < 1.8 {
            return true;
        }
    }
    let (top, bot) = (MID_Y + MID_H, WALL_H - SILL_H);
    if y >= top && y < bot {
        let t = (y - top) as f64 / (bot - top) as f64;
        let cx = (PX - POST_W) as f64 - t * (PX - 2 * POST_W) as f64;
        if (x as f64 - cx).abs() < 1.8 {
            return true;
        }
    }
    false
}

pub fn paint_timber_wall(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("wtimber{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let plaster = variant_pick(variant, 0x9c937c, 0x948a74);
    let oak = variant_pick(variant, 0x2a1b10, 0x251a12);
    let dirt = hex(0x3e3526);
    for y in 0..WALL_H {
        for x in 0..PX {
            if timber_beam_at(x, y) {
                // Wood grain runs along the member: horizontal on rails, vertical elsewhere.
                let stripe = if is_rail_row(y) { (y / 2) % 2 } else { (x / 2) % 2 };
                let g = rng.randi_range(-1, 1) + stripe;
                img.set(x, y, mul(oak, 1.0 + g as f64 * 0.16));
            } else {
                let mut c = mul(plaster, 1.0 + rng.randi_range(-1, 1) as f64 * 0.07);
                // Weathering: dirt creeping up from the sill.
                let from_bottom = (WALL_H - SILL_H - 1 - y) as f64;
                if from_bottom < 14.0 && rng.randf() < (1.0 - from_bottom / 14.0) * 0.75 {
                    c = mul(dirt, rng.randf_range(0.8, 1.3));
                }
                img.set(x, y, c);
            }
        }
    }
    // Lit edge on the top of each beam, dark edge on its underside, so the frame stands proud of the plaster.
    for y in 0..WALL_H {
        for x in 0..PX {
            if timber_beam_at(x, y) {
                if y > 0 && !timber_beam_at(x, y - 1) {
                    img.set(x, y, mul(oak, 1.7));
                } else if y + 1 < WALL_H && !timber_beam_at(x, y + 1) {
                    img.set(x, y, mul(oak, 0.55));
                }
            } else if y > 0 && timber_beam_at(x, y - 1) {
                // Shadow cast on the plaster just under a beam.
                let c = img.get(x, y);
                img.set(x, y, mul(c, 0.72));
            }
        }
    }
    // Rain streaks.
    for _ in 0..4 {
        let x = rng.randi_range(POST_W + 1, PX - POST_W - 2);
        let y = rng.randi_range(PLATE_H + 1, 40);
        let len = rng.randi_range(6, 22);
        for k in 0..len {
            if !timber_beam_at(x, y + k) && y + k < WALL_H {
                let c = img.get(x, y + k);
                img.set(x, y + k, mul(c, 0.86));
            }
        }
    }
    img
}

pub fn flat_timber_wall(variant: i32) -> Img {
    let mut img = Img::new(PX, WALL_H);
    let plaster = variant_pick(variant, 0x9c937c, 0x948a74);
    let oak = variant_pick(variant, 0x2a1b10, 0x251a12);
    for y in 0..WALL_H {
        for x in 0..PX {
            img.set(x, y, if timber_beam_at(x, y) { oak } else { plaster });
        }
    }
    img
}

// -- stone wall ---------------------------------------------------------------------------------------------------------

/// Rough ashlar: courses of uneven height, blocks of uneven width, a different stagger per course.
/// `(x, y, w, h)`; x may run past the right edge and is wrapped when painted.
fn stone_layout(rng: &mut GodotRng) -> Vec<(i32, i32, i32, i32)> {
    let mut blocks = Vec::new();
    let mut y = 0;
    while y < WALL_H {
        let h = rng.randi_range(9, 15).min(WALL_H - y);
        let mut x = rng.randi_range(0, PX - 1);
        let end = x + PX;
        while x < end {
            let mut w = rng.randi_range(8, 16);
            if end - (x + w) < 7 {
                w = end - x;
            }
            blocks.push((x, y, w, h));
            x += w;
        }
        y += h;
    }
    blocks
}

pub fn paint_stone_wall(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("washlar{variant}"));
    let mut img = Img::new(PX, WALL_H);
    let mortar = hex(0x14110e);
    img.fill(mortar);
    let tones = [variant_pick(variant, 0x6a655c, 0x66625a), variant_pick(variant, 0x5f574a, 0x5c5548), hex(0x726c60)];
    for (bx, by, bw, bh) in stone_layout(&mut rng) {
        let base = mul(tones[rng.randi_range(0, 2) as usize], rng.randf_range(0.82, 1.15));
        for y in 1..bh {
            for x in 1..bw {
                // Rounded corners: skip the four corner texels.
                if (x == 1 || x == bw - 1) && (y == 1 || y == bh - 1) {
                    continue;
                }
                let mut f = 1.0 + rng.randi_range(-1, 1) as f64 * 0.09;
                if y == 1 {
                    f *= 1.3;
                } else if y == bh - 1 {
                    f *= 0.7;
                }
                if x == bw - 1 {
                    f *= 0.82;
                }
                wdot(&mut img, bx + x, by + y, mul(base, f));
            }
        }
        // A chip or two on each block.
        for _ in 0..2 {
            let cx = rng.randi_range(2, (bw - 3).max(2));
            let cy = rng.randi_range(2, (bh - 3).max(2));
            wdot(&mut img, bx + cx, by + cy, mul(base, 0.6));
        }
    }
    // Moss: green tufts, more toward the bottom.
    let moss = hex(0x3d5a2a);
    for _ in 0..22 {
        let x = rng.randi_range(0, PX - 1);
        let y = rng.randi_range(WALL_H / 3, WALL_H - 3);
        let n = rng.randi_range(1, 3);
        for k in 0..n {
            wdot(&mut img, x + k, y, mul(moss, rng.randf_range(0.7, 1.3)));
        }
        if rng.randf() < 0.5 {
            wdot(&mut img, x, y - 1, mul(moss, 0.8));
        }
    }
    img
}

pub fn flat_stone_wall(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("washlar{variant}"));
    let mut img = Img::new(PX, WALL_H);
    img.fill(hex(0x14110e));
    let tones = [variant_pick(variant, 0x6a655c, 0x66625a), variant_pick(variant, 0x5f574a, 0x5c5548)];
    for (bx, by, bw, bh) in stone_layout(&mut rng) {
        let base = tones[rng.randi_range(0, 1) as usize];
        for y in 1..bh {
            for x in 1..bw {
                wdot(&mut img, bx + x, by + y, base);
            }
        }
    }
    img
}

// -- cobble floor -------------------------------------------------------------------------------------------------------

/// 4 x 4 stones of about 7 texels, odd rows shifted by 4; `(x, y, w, h)` of each cell.
fn cobble_cells(rng: &mut GodotRng) -> Vec<(i32, i32, i32, i32)> {
    let mut cells = Vec::new();
    for row in 0..4 {
        let off = if row % 2 == 0 { 0 } else { 4 };
        for col in 0..4 {
            let jx = rng.randi_range(0, 1);
            let jy = rng.randi_range(0, 1);
            cells.push((col * 8 + off + jx, row * 8 + jy, 7 - jx, 7 - jy));
        }
    }
    cells
}

/// Rounded rectangle test: cut the corner texels.
fn in_round_rect(x: i32, y: i32, w: i32, h: i32) -> bool {
    !((x == 0 || x == w - 1) && (y == 0 || y == h - 1))
}

pub fn paint_cobble_floor(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("fcobble{variant}"));
    let mut img = Img::new(PX, PX);
    let gap = hex(0x16120e);
    // Grit in the gaps.
    noise(&mut img, 0, 0, PX, PX, gap, 0.3, &mut rng, 1);
    let tones = [
        variant_pick(variant, 0x6b6861, 0x68655e),
        variant_pick(variant, 0x5a5348, 0x57514a),
        hex(0x7a7468),
        hex(0x4e4a44),
    ];
    for (cx, cy, w, h) in cobble_cells(&mut rng) {
        let base = mul(tones[rng.randi_range(0, 3) as usize], rng.randf_range(0.85, 1.12));
        for y in 0..h {
            for x in 0..w {
                if !in_round_rect(x, y, w, h) {
                    continue;
                }
                let mut f = 1.0 + rng.randi_range(-1, 1) as f64 * 0.07;
                if y <= 1 && x <= w - 3 {
                    f *= 1.22;
                }
                if y >= h - 2 || x >= w - 1 {
                    f *= 0.72;
                }
                wdot(&mut img, cx + x, cy + y, mul(base, f));
            }
        }
    }
    img
}

pub fn flat_cobble_floor(variant: i32) -> Img {
    let mut rng = GodotRng::from_key(&format!("fcobble{variant}"));
    let mut img = Img::new(PX, PX);
    img.fill(hex(0x16120e));
    let tones = [variant_pick(variant, 0x6b6861, 0x68655e), variant_pick(variant, 0x5a5348, 0x57514a)];
    for (cx, cy, w, h) in cobble_cells(&mut rng) {
        let base = tones[rng.randi_range(0, 1) as usize];
        for y in 0..h {
            for x in 0..w {
                if in_round_rect(x, y, w, h) {
                    wdot(&mut img, cx + x, cy + y, base);
                }
            }
        }
    }
    img
}

// -- dirt floor ---------------------------------------------------------------------------------------------------------

fn rut_rows(x: i32) -> [i32; 2] {
    let wave = ((x as f64 / PX as f64 * TAU).sin() * 1.6).round() as i32;
    [8 + wave, 23 - wave]
}

pub fn paint_dirt_floor(_variant: i32) -> Img {
    let mut rng = GodotRng::from_key("fdirt0");
    let mut img = Img::new(PX, PX);
    let base = hex(0x4c3b28);
    noise(&mut img, 0, 0, PX, PX, base, 0.13, &mut rng, 1);
    // Cart ruts: two wavy dark troughs along x, a lit lip above each.
    for x in 0..PX {
        for r in rut_rows(x) {
            wdot(&mut img, x, r - 2, mul(base, 1.25));
            wdot(&mut img, x, r - 1, mul(base, 0.7));
            wdot(&mut img, x, r, mul(base, 0.52));
            wdot(&mut img, x, r + 1, mul(base, 0.68));
        }
    }
    // Pebbles with a dark shadow texel below.
    for _ in 0..9 {
        let x = rng.randi_range(0, PX - 1);
        let y = rng.randi_range(0, PX - 1);
        let c = mul(hex(0x7d7568), rng.randf_range(0.8, 1.2));
        wdot(&mut img, x, y, c);
        if rng.randf() < 0.5 {
            wdot(&mut img, x + 1, y, mul(c, 0.8));
        }
        wdot(&mut img, x, y + 1, hex(0x1c140c));
    }
    // Straw: short pale slashes.
    let straw = hex(0xa88b44);
    for _ in 0..8 {
        let x = rng.randi_range(0, PX - 1);
        let y = rng.randi_range(0, PX - 1);
        let len = rng.randi_range(3, 5);
        let slope = rng.randi_range(-1, 1);
        for k in 0..len {
            wdot(&mut img, x + k, y + (k * slope) / 2, mul(straw, rng.randf_range(0.75, 1.15)));
        }
    }
    img
}

pub fn flat_dirt_floor(_variant: i32) -> Img {
    let mut rng = GodotRng::from_key("fdirt0");
    let mut img = Img::new(PX, PX);
    let (earth, rut, stone) = (hex(0x4c3b28), hex(0x2a2016), hex(0x7d7568));
    img.fill(earth);
    for x in 0..PX {
        for r in rut_rows(x) {
            wdot(&mut img, x, r, rut);
            wdot(&mut img, x, r + 1, rut);
        }
    }
    for _ in 0..9 {
        wdot(&mut img, rng.randi_range(0, PX - 1), rng.randi_range(0, PX - 1), stone);
    }
    img
}

// -- beam ceiling -------------------------------------------------------------------------------------------------------

const BEAM_X: i32 = 12;
const BEAM_W: i32 = 8;

pub fn paint_beam_ceiling(_variant: i32) -> Img {
    let mut rng = GodotRng::from_key("cbeams0");
    let mut img = Img::new(PX, PX);
    let plank = hex(0x2c1e12);
    let beam = hex(0x1c120a);
    let seam = hex(0x0a0604);
    // Planks run along x, 8 texels wide each, with a seam and a staggered butt joint.
    for row in 0..4 {
        let y0 = row * 8;
        let base = mul(plank, rng.randf_range(0.85, 1.15));
        for y in 0..8 {
            for x in 0..PX {
                let mut f = 1.0 + rng.randi_range(-1, 1) as f64 * 0.1;
                if (x / 3 + y) % 5 == 0 {
                    f *= 0.9;
                }
                img.set(x, y0 + y, mul(base, f));
            }
        }
        img.dot(0, y0 + 7, seam, PX, 1);
        img.dot((row * 11 + 5) % PX, y0, seam, 1, 7);
    }
    // Heavy cross beam over the full height.
    for y in 0..PX {
        for x in BEAM_X..BEAM_X + BEAM_W {
            let mut f = 1.0 + rng.randi_range(-1, 1) as f64 * 0.12;
            if x == BEAM_X + 1 {
                f *= 1.5;
            }
            if x >= BEAM_X + BEAM_W - 2 {
                f *= 0.65;
            }
            img.set(x, y, mul(beam, f));
        }
        img.set(BEAM_X, y, hex(0x050302));
        img.set(BEAM_X + BEAM_W - 1, y, hex(0x050302));
        // Shadow on the planks either side of the beam.
        for x in [BEAM_X - 1, BEAM_X + BEAM_W] {
            let c = img.get(x, y);
            img.set(x, y, mul(c, 0.6));
        }
    }
    // Iron nails in the beam.
    for y in [4, 16, 28] {
        img.dot1(BEAM_X + 3, y, hex(0x6a645c));
    }
    img
}

pub fn flat_beam_ceiling(_variant: i32) -> Img {
    let mut img = Img::new(PX, PX);
    let (plank, beam, seam) = (hex(0x2c1e12), hex(0x1c120a), hex(0x0a0604));
    for y in 0..PX {
        for x in 0..PX {
            let c = if (BEAM_X..BEAM_X + BEAM_W).contains(&x) {
                beam
            } else if y % 8 == 7 {
                seam
            } else {
                plank
            };
            img.set(x, y, c);
        }
    }
    img
}

// -- lookup -------------------------------------------------------------------------------------------------------------

/// Every medieval name `paint_named` knows.
pub const NAMES: &[&str] = &["timber0", "timber1", "stone0", "stone1", "cobble0", "cobble1", "dirt0", "beams0"];

/// Paints a medieval surface by name in a style: `(width, height, RGBA8)`. `None` for any other name.
/// "stone0" / "stone1" are the ashlar wall (table keys "ashlar0" / "ashlar1" work too).
pub fn paint_named_styled(name: &str, style: Style) -> Option<(u16, u16, Vec<u8>)> {
    let flat = style == Style::Flat;
    let img = match (name, flat) {
        ("timber0", false) => paint_timber_wall(0),
        ("timber1", false) => paint_timber_wall(1),
        ("timber0", true) => flat_timber_wall(0),
        ("timber1", true) => flat_timber_wall(1),
        ("stone0" | "ashlar0", false) => paint_stone_wall(0),
        ("stone1" | "ashlar1", false) => paint_stone_wall(1),
        ("stone0" | "ashlar0", true) => flat_stone_wall(0),
        ("stone1" | "ashlar1", true) => flat_stone_wall(1),
        ("cobble0", false) => paint_cobble_floor(0),
        ("cobble1", false) => paint_cobble_floor(1),
        ("cobble0", true) => flat_cobble_floor(0),
        ("cobble1", true) => flat_cobble_floor(1),
        ("dirt0", false) => paint_dirt_floor(0),
        ("dirt0", true) => flat_dirt_floor(0),
        ("beams0", false) => paint_beam_ceiling(0),
        ("beams0", true) => flat_beam_ceiling(0),
        _ => return None,
    };
    Some((img.w as u16, img.h as u16, img.px))
}

/// `paint_named_styled` in the style `--flat-textures` selects.
pub fn paint_named(name: &str) -> Option<(u16, u16, Vec<u8>)> {
    paint_named_styled(name, if flat_textures() { Style::Flat } else { Style::Painted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn lum(px: &[u8], i: usize) -> u32 {
        px[i * 4] as u32 + px[i * 4 + 1] as u32 + px[i * 4 + 2] as u32
    }

    fn tones(px: &[u8]) -> usize {
        px.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect::<HashSet<_>>().len()
    }

    #[test]
    fn sizes() {
        for s in [Style::Painted, Style::Flat] {
            for (n, w, h) in [("timber0", 32, 128), ("stone0", 32, 128), ("cobble0", 32, 32), ("dirt0", 32, 32), ("beams0", 32, 32)] {
                let (pw, ph, px) = paint_named_styled(n, s).unwrap();
                assert_eq!((pw, ph, px.len()), (w, h, w as usize * h as usize * 4), "{n} {s:?}");
            }
        }
        assert!(paint_named("nope").is_none());
    }

    #[test]
    fn deterministic() {
        for n in NAMES {
            for s in [Style::Painted, Style::Flat] {
                assert!(paint_named_styled(n, s) == paint_named_styled(n, s), "{n} {s:?}");
            }
        }
    }

    #[test]
    fn timber_has_dark_beams_and_light_plaster() {
        let (_, _, px) = paint_named_styled("timber0", Style::Painted).unwrap();
        let n = px.len() / 4;
        let dark = (0..n).filter(|&i| lum(&px, i) < 150).count();
        let light = (0..n).filter(|&i| lum(&px, i) > 330).count();
        assert!(dark > n / 8, "dark beam pixels: {dark}");
        assert!(light > n / 3, "light plaster pixels: {light}");
    }

    #[test]
    fn cobble_has_dark_gaps() {
        let (_, _, px) = paint_named_styled("cobble0", Style::Painted).unwrap();
        let n = px.len() / 4;
        let gaps = (0..n).filter(|&i| lum(&px, i) < 100).count();
        let stones = (0..n).filter(|&i| lum(&px, i) > 150).count();
        assert!(gaps > n / 10, "gap pixels: {gaps}");
        assert!(stones > n / 3, "stone pixels: {stones}");
    }

    #[test]
    fn flat_mode_uses_at_most_three_tones() {
        for n in NAMES {
            let (_, _, px) = paint_named_styled(n, Style::Flat).unwrap();
            let t = tones(&px);
            assert!((2..=3).contains(&t), "{n}: {t} tones");
        }
    }

    #[test]
    fn painted_differs_from_flat() {
        for n in NAMES {
            assert!(paint_named_styled(n, Style::Painted) != paint_named_styled(n, Style::Flat), "{n}");
        }
    }

    #[test]
    fn table_kinds_match_named() {
        for (group, key, name) in [("wall", "timber0", "timber0"), ("wall", "ashlar0", "stone0"), ("floor", "cobble0", "cobble0"), ("floor", "dirt0", "dirt0"), ("ceiling", "beams0", "beams0")] {
            for s in [Style::Painted, Style::Flat] {
                assert!(super::super::paint_styled(group, key, 0, s) == paint_named_styled(name, s), "{group}:{key} {s:?}");
            }
        }
    }
}
