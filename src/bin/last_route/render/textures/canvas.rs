//! The "tiny painter" of ref_textures.gd: a Godot RGBA8 `Image`, Godot `Color` (float32 channels) and the shared
//! brushes (_dot, _noise, _rivet, _crack, _flower, _star, _mosaic, the wall bands).

use super::rng::GodotRng;
use std::f64::consts::{PI, TAU};

pub const PX: i32 = 32;
pub const WALL_H: i32 = 128;
/// Shared wall bands, y measured from the TOP of the 128 px wall texture.
pub const SKIRT_H: i32 = 8;
pub const RAIL_Y: i32 = 70;
pub const CORNICE_H: i32 = 10;

/// Godot `Color`: four float32 channels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Col {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// `Color("rrggbb")`: each channel `n / 255.0f` in float32.
pub fn hex(rgb: u32) -> Col {
    Col {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

pub const BLACK: Col = Col {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// `RefTextures._mul`: channels times `f` in double precision, clamped to 0..1, stored back as float32; alpha kept.
pub fn mul(c: Col, f: f64) -> Col {
    let m = |v: f32| (v as f64 * f).clamp(0.0, 1.0) as f32;
    Col {
        r: m(c.r),
        g: m(c.g),
        b: m(c.b),
        a: c.a,
    }
}

/// Godot `Image::_set_color_at_ofs` for FORMAT_RGBA8: `uint8_t(CLAMP(v * 255.0, 0, 255))` (truncating).
fn to_u8(v: f32) -> u8 {
    (v as f64 * 255.0).clamp(0.0, 255.0) as u8
}

/// A Godot RGBA8 image (row-major, 4 bytes per pixel).
#[derive(Clone, Debug)]
pub struct Img {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>,
}

impl Img {
    /// `_new_image`: RGBA8 filled with Color.BLACK.
    pub fn new(w: i32, h: i32) -> Img {
        let mut img = Img {
            w,
            h,
            px: vec![0; (w * h * 4) as usize],
        };
        img.fill(BLACK);
        img
    }

    pub fn fill(&mut self, c: Col) {
        let rgba = [to_u8(c.r), to_u8(c.g), to_u8(c.b), to_u8(c.a)];
        for p in self.px.chunks_exact_mut(4) {
            p.copy_from_slice(&rgba);
        }
    }

    pub fn set(&mut self, x: i32, y: i32, c: Col) {
        let o = ((y * self.w + x) * 4) as usize;
        self.px[o] = to_u8(c.r);
        self.px[o + 1] = to_u8(c.g);
        self.px[o + 2] = to_u8(c.b);
        self.px[o + 3] = to_u8(c.a);
    }

    /// `get_pixel` of an RGBA8 image: `byte / 255.0f`.
    pub fn get(&self, x: i32, y: i32) -> Col {
        let o = ((y * self.w + x) * 4) as usize;
        let f = |b: u8| b as f32 / 255.0;
        Col {
            r: f(self.px[o]),
            g: f(self.px[o + 1]),
            b: f(self.px[o + 2]),
            a: f(self.px[o + 3]),
        }
    }

    /// `_dot`: a w x h block, clipped to the image.
    pub fn dot(&mut self, x: i32, y: i32, c: Col, w: i32, h: i32) {
        for yy in 0..h {
            for xx in 0..w {
                let (px, py) = (x + xx, y + yy);
                if px >= 0 && py >= 0 && px < self.w && py < self.h {
                    self.set(px, py, c);
                }
            }
        }
    }

    /// `_dot` with the default 1 x 1.
    pub fn dot1(&mut self, x: i32, y: i32, c: Col) {
        self.dot(x, y, c, 1, 1);
    }

    /// `Image.rotate_90(COUNTERCLOCKWISE)`: new(x, y) = old(w - 1 - y, x); width and height swap.
    pub fn rotate_ccw(&mut self) {
        let (w, h) = (self.w, self.h);
        let mut out = Img {
            w: h,
            h: w,
            px: vec![0; self.px.len()],
        };
        for y in 0..out.h {
            for x in 0..out.w {
                let src = (((x) * w + (w - 1 - y)) * 4) as usize;
                let dst = ((y * out.w + x) * 4) as usize;
                out.px[dst..dst + 4].copy_from_slice(&self.px[src..src + 4]);
            }
        }
        *self = out;
    }
}

/// Godot `Math::fposmod`.
pub fn fposmod(x: f64, y: f64) -> f64 {
    let mut v = x % y;
    if (v < 0.0 && y > 0.0) || (v > 0.0 && y < 0.0) {
        v += y;
    }
    v + 0.0
}

/// `roundi`: round half away from zero.
pub fn roundi(v: f64) -> i32 {
    v.round() as i32
}

/// GDScript `range(from, to, step)` for a positive step.
pub fn range(from: i32, to: i32, step: i32) -> impl Iterator<Item = i32> {
    (0..)
        .map(move |i| from + i * step)
        .take_while(move |v| *v < to)
}

// -- brushes ---------------------------------------------------------------------------------------------------------

/// `_noise`: three-step brightness grain, one draw per `cell` block, rows then columns.
pub fn noise(
    img: &mut Img,
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
    base: Col,
    amp: f64,
    rng: &mut GodotRng,
    cell: i32,
) {
    for y in range(0, h, cell) {
        for x in range(0, w, cell) {
            let step = rng.randi_range(-1, 1);
            img.dot(
                x0 + x,
                y0 + y,
                mul(base, 1.0 + step as f64 * amp),
                cell,
                cell,
            );
        }
    }
}

pub fn rivet(img: &mut Img, x: i32, y: i32, base: Col) {
    img.dot(x, y, mul(base, 0.35), 2, 2);
    img.dot1(x, y, mul(base, 1.7));
}

pub fn crack(img: &mut Img, rng: &mut GodotRng, x: i32, y: i32, length: i32, c: Col) {
    let (mut cx, mut cy) = (x, y);
    for _ in 0..length {
        img.dot1(cx, cy, c);
        let r = rng.randf();
        if r < 0.4 {
            cx += 1;
        } else if r < 0.7 {
            cy += 1;
        } else if r < 0.85 {
            cx -= 1;
        } else {
            cy -= 1;
        }
    }
}

pub fn mosaic(img: &mut Img, base: Col, rng: &mut GodotRng) {
    noise(img, 0, 0, PX, PX, base, 0.22, rng, 2);
    for _ in 0..20 {
        let x = rng.randi_range(0, 30);
        let y = rng.randi_range(0, 30);
        img.dot(x, y, mul(base, 0.55), 2, 1);
    }
}

/// Five 3x3 petals round a 3x3 core.
pub fn flower(img: &mut Img, cx: i32, cy: i32, petal: Col, core: Col) {
    for a in 0..5 {
        let ang = a as f64 / 5.0 * TAU - PI * 0.5;
        let px = roundi(cx as f64 + ang.cos() * 3.0);
        let py = roundi(cy as f64 + ang.sin() * 3.0);
        img.dot(px - 1, py - 1, petal, 3, 3);
        img.dot1(px, py, mul(petal, 1.25));
    }
    img.dot(cx - 1, cy - 1, core, 3, 3);
}

/// Five-pointed star of radius `outer` (inner radius .45 of it).
pub fn star(img: &mut Img, cx: i32, cy: i32, outer: i32, c: Col) {
    let inner = outer as f64 * 0.45;
    for y in -outer..=outer {
        for x in -outer..=outer {
            // Vector2(x, y).length(): float32
            let r = ((x * x + y * y) as f32).sqrt() as f64;
            let ang = (y as f64).atan2(x as f64) + PI * 0.5;
            let seg = fposmod(ang, TAU / 5.0);
            let t = (seg / (PI / 5.0) - 1.0).abs();
            if r <= outer as f64 + (inner - outer as f64) * t + 0.4 {
                img.dot1(cx + x, cy + y, c);
            }
        }
    }
}

/// Speckled leaves scattered in an ellipse.
pub fn leaf_cluster(img: &mut Img, rng: &mut GodotRng, cx: i32, cy: i32, r: i32, base: Col) {
    for _ in 0..r * 6 {
        let a = rng.randf() * TAU;
        let d = rng.randf() * r as f64;
        let x = cx + roundi(a.cos() * d);
        let y = cy + roundi(a.sin() * d * 0.8);
        let c = mul(base, rng.randf_range(0.5, 1.6));
        img.dot(x, y, c, 2, 1);
    }
}

// -- wall bands -------------------------------------------------------------------------------------------------------

/// Upper paint above the rail, lower paint below, each with grain.
pub fn wall_base(img: &mut Img, rng: &mut GodotRng, lower: Col, upper: Col) {
    noise(img, 0, 0, PX, RAIL_Y, upper, 0.1, rng, 1);
    noise(img, 0, RAIL_Y, PX, WALL_H - RAIL_Y, lower, 0.1, rng, 1);
}

pub fn cornice(img: &mut Img, c: Col) {
    img.dot(0, 0, mul(c, 0.45), PX, CORNICE_H);
    img.dot(0, CORNICE_H - 3, mul(c, 1.4), PX, 1);
    img.dot(0, CORNICE_H, hex(0x060405), PX, 1);
}

pub fn skirt(img: &mut Img, c: Col) {
    img.dot(0, WALL_H - SKIRT_H, mul(c, 0.5), PX, SKIRT_H);
    img.dot(0, WALL_H - SKIRT_H, mul(c, 1.5), PX, 1);
    img.dot(0, WALL_H - SKIRT_H - 1, hex(0x060405), PX, 1);
}

/// Brass chair rail at RAIL_Y.
pub fn rail(img: &mut Img) {
    img.dot(0, RAIL_Y - 2, hex(0x070405), PX, 1);
    img.dot(0, RAIL_Y - 1, hex(0x8a6a2c), PX, 1);
    img.dot(0, RAIL_Y, hex(0x5a4220), PX, 1);
    img.dot(0, RAIL_Y + 1, hex(0x070405), PX, 1);
}

/// Vertical grime streaks between the cornice and the skirting.
pub fn grime(img: &mut Img, rng: &mut GodotRng, count: i32) {
    for _ in 0..count {
        let x = rng.randi_range(0, PX - 1);
        let y = rng.randi_range(CORNICE_H, WALL_H - SKIRT_H - 2);
        let len = rng.randi_range(4, 22);
        let c = if rng.randf() < 0.6 {
            hex(0x0a0707)
        } else {
            hex(0x2a1810)
        };
        img.dot(x, y, c, 1, len);
    }
}

/// Beach cornice: keeps the wall's own value instead of darkening to half.
pub fn sea_cornice(img: &mut Img, c: Col) {
    img.dot(0, 0, mul(c, 0.82), PX, CORNICE_H);
    img.dot(0, CORNICE_H - 3, mul(c, 1.15), PX, 1);
    img.dot(0, CORNICE_H, mul(c, 0.55), PX, 1);
}

pub fn sea_skirt(img: &mut Img, c: Col) {
    img.dot(0, WALL_H - SKIRT_H, mul(c, 0.78), PX, SKIRT_H);
    img.dot(0, WALL_H - SKIRT_H, mul(c, 1.15), PX, 1);
    img.dot(0, WALL_H - SKIRT_H - 1, mul(c, 0.55), PX, 1);
}

/// The diagonal yellow / black hazard stripe colour at (x, y): `((x + y) >> 2) & 1`.
pub fn hazard_stripe(x: i32, y: i32, yellow: Col, black: Col) -> Col {
    if ((x + y) >> 2) & 1 == 1 {
        yellow
    } else {
        black
    }
}
