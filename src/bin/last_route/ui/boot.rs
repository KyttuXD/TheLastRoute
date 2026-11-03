//! The boot loader: the browser game's loading card (`games/last-route/index.html` `#boot-loading`,
//! `styles.css` `.boot-loading*`), drawn over the title while the game starts.
//!
//! The web values are CSS pixels at a 1280 x 720 viewport. They scale with the design canvas
//! (`style::unit()`), so the card is the browser's card at 1280 x 720 and grows with the window the
//! way the other screens do. The progress is real: `load_world` and `run` report each startup step
//! through `Step`, and nothing is timed to look busy.
//!
//! The state (`BootState`), the layout (`Layout`), the easing and the pulse curves are plain Rust
//! with no macroquad calls, so they are unit tested. `BootLoader` does the drawing.

use macroquad::prelude::*;
use std::f32::consts::{FRAC_1_SQRT_2, TAU};

// -- the startup steps and their progress (the web's ladder: 18, 35, 65, 88, 96, 100) --------

/// The loader's state, in the order the startup reaches them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    /// The window is open and the first loader frame is drawn (before any load).
    Connect,
    /// `Content::load` and `Catalog::load`: the route and facility data.
    Route,
    /// `Assets::load`: textures, models, props, icons and portraits.
    Art,
    /// `Sounds::new` and `shader::load_world_material`: the audio cues and the world material.
    Signal,
    /// The settings, the colour scheme, the render targets and the saved run.
    Saves,
    /// The first title frame is drawn: the card reads SIGNAL READY and fades out.
    Ready,
}

impl Step {
    /// The bar and percent shown once this step is reached.
    pub fn percent(self) -> u32 {
        match self {
            Step::Connect => 18,
            Step::Route => 35,
            Step::Art => 65,
            Step::Signal => 88,
            Step::Saves => 96,
            Step::Ready => 100,
        }
    }

    /// The status text shown once this step is reached.
    pub fn text(self) -> &'static str {
        match self {
            Step::Connect => "CONNECTING TO RELAY...",
            Step::Route => "LOADING ROUTE DATA...",
            Step::Art => "LOADING FACILITY ART...",
            Step::Signal => "PREPARING SIGNAL...",
            Step::Saves => "SYNCING SAVES...",
            Step::Ready => "SIGNAL READY // ENGAGING",
        }
    }

    fn fraction(self) -> f32 {
        self.percent() as f32 / 100.0
    }
}

/// A required content list came back empty: the game cannot build a route.
pub const FAIL_LOAD: &str = "LOAD FAILED // RESTART THE GAME";
/// The load ran past the watchdog (the web's 20 s timeout).
pub const FAIL_SLOW: &str = "LOAD TAKING TOO LONG // RESTART";
/// The web's timeout, in seconds of wall-clock time since the loader opened.
pub const WATCHDOG_SECS: f32 = 20.0;
/// The web's `.boot-loading` fade (240 ms, CSS `ease`).
pub const FADE_SECS: f32 = 0.24;
/// The loader is removed 260 ms after its fade starts (the web's `setTimeout(..., 260)`).
pub const REMOVE_SECS: f32 = 0.26;
/// The bar's width transition (220 ms ease-out), as a time constant for an exponential ease.
const BAR_TAU: f32 = 0.11;

// -- pure state -----------------------------------------------------------------------------

/// The loader's progress, failure and fade, with no drawing in it.
#[derive(Clone, Debug, PartialEq)]
pub struct BootState {
    step: Step,
    /// The bar's fraction on screen; it eases toward the step's fraction.
    shown: f32,
    /// Set once: the status line replaces the step text and the loader stays up.
    failed: Option<&'static str>,
    /// Seconds since the fade began (`None` until `ready`).
    fade: Option<f32>,
    /// The loader is gone: no drawing and no input blocking.
    done: bool,
}

impl BootState {
    pub fn new() -> BootState {
        BootState {
            step: Step::Connect,
            shown: Step::Connect.fraction(),
            failed: None,
            fade: None,
            done: false,
        }
    }

    #[cfg(test)]
    pub fn step(&self) -> Step {
        self.step
    }

    pub fn failed(&self) -> Option<&'static str> {
        self.failed
    }

    /// Test helper: the card has finished loading.
    #[cfg(test)]
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// The title has drawn and the card is fading (or gone). Input is free from this point.
    pub fn is_ready(&self) -> bool {
        self.step == Step::Ready
    }

    pub fn shown(&self) -> f32 {
        self.shown
    }

    /// The percent text (the last reached step's value; it keeps that value after a failure).
    pub fn percent(&self) -> u32 {
        self.step.percent()
    }

    /// The status line: the failure message if there is one, else the reached step's text.
    pub fn status(&self) -> &'static str {
        self.failed.unwrap_or(self.step.text())
    }

    /// A startup step has completed. Steps only move forward, and nothing moves after a failure.
    pub fn reach(&mut self, step: Step) {
        if self.failed.is_none() && step > self.step {
            self.step = step;
        }
    }

    /// The web's `fail()`: a no-op once the load is complete.
    pub fn fail(&mut self, message: &'static str) {
        if self.failed.is_none() && self.step != Step::Ready {
            self.failed = Some(message);
        }
    }

    /// The web's `ready()`. A failed load stays failed and does not fade.
    pub fn ready(&mut self) {
        if self.failed.is_none() {
            self.step = Step::Ready;
        }
    }

    /// The watchdog: `elapsed` seconds of wall-clock time since the loader opened.
    pub fn watchdog(&mut self, elapsed: f32) {
        if elapsed >= WATCHDOG_SECS {
            self.fail(FAIL_SLOW);
        }
    }

    /// Advances the bar and the fade by `dt` seconds. Returns the loader's alpha for this frame,
    /// or `None` once it has been removed.
    pub fn tick(&mut self, dt: f32) -> Option<f32> {
        if self.done {
            return None;
        }
        let target = self.step.fraction();
        self.shown += (target - self.shown) * (1.0 - (-dt / BAR_TAU).exp());
        let alpha = match self.fade {
            Some(t) if t >= REMOVE_SECS => {
                self.done = true;
                return None;
            }
            Some(t) => fade_alpha(t),
            None => {
                // the first frame after `ready` is drawn at full strength (the web's rAF wait)
                if self.step == Step::Ready {
                    self.fade = Some(0.0);
                }
                1.0
            }
        };
        if let Some(t) = self.fade.as_mut() {
            *t += dt;
        }
        Some(alpha)
    }
}

impl Default for BootState {
    fn default() -> Self {
        BootState::new()
    }
}

// -- curves ---------------------------------------------------------------------------------

/// CSS `cubic-bezier(x1, y1, x2, y2)` at input `x`: the curve's y for the t where its x is `x`.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bez = |p1: f32, p2: f32, t: f32| {
        let u = 1.0 - t;
        3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if bez(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(y1, y2, 0.5 * (lo + hi))
}

/// CSS `ease`.
pub fn ease(x: f32) -> f32 {
    cubic_bezier(0.25, 0.1, 0.25, 1.0, x)
}

/// CSS `ease-in-out`.
pub fn ease_in_out(x: f32) -> f32 {
    cubic_bezier(0.42, 0.0, 0.58, 1.0, x)
}

/// The loader's alpha `t` seconds into the fade: `1 - ease(t / 0.24)`.
pub fn fade_alpha(t: f32) -> f32 {
    1.0 - ease((t / FADE_SECS).clamp(0.0, 1.0))
}

/// `boot-loading-pulse` on the status text and percent: opacity .68 to 1, 1.4 s, ease-in-out,
/// alternate (so a full back-and-forth takes 2.8 s).
pub fn pulse_alpha(clock: f32) -> f32 {
    let x = (clock / 1.4).rem_euclid(2.0);
    let p = if x > 1.0 { 2.0 - x } else { x };
    0.68 + 0.32 * ease_in_out(p)
}

/// `boot-blip-pulse` on the radar blip, 2.4 s ease-in-out: (opacity, scale), from (.15, .9) to
/// (1, 1.1) at the half period.
pub fn blip_pulse(clock: f32) -> (f32, f32) {
    let u = (clock / 2.4).rem_euclid(1.0);
    let p = 1.0 - (2.0 * u - 1.0).abs();
    let e = ease_in_out(p);
    (0.15 + 0.85 * e, 0.9 + 0.2 * e)
}

/// `boot-radar-spin`: the sweep's clockwise angle from 12 o'clock, a 2.4 s turn, linear.
pub fn sweep_angle(clock: f32) -> f32 {
    TAU * (clock / 2.4).rem_euclid(1.0)
}

/// The error function (Abramowitz and Stegun 7.1.26, error below 1.5e-7).
fn erf(x: f32) -> f32 {
    let sign = x.signum();
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = ((((1.061_405_4 * t - 1.453_152) * t + 1.421_414) * t - 0.284_496_7) * t + 0.254_829_6) * t;
    sign * (1.0 - poly * (-x * x).exp())
}

/// The standard normal CDF: the coverage of a blurred edge (CSS blur radius `b` is sigma `b / 2`).
fn phi(x: f32) -> f32 {
    0.5 * (1.0 + erf(x * FRAC_1_SQRT_2))
}

// -- layout ---------------------------------------------------------------------------------

/// Window pixels per CSS pixel: 1.0 at 1280 x 720, where the web values were read. It is the
/// design-canvas scale (`style::unit()`) divided by the canvas's 2.6667 units per 1280 px.
pub fn scale_for(w: f32, h: f32) -> f32 {
    (w / 480.0).min(h / 270.0).max(0.05) * (480.0 / 1280.0)
}

/// Every box of the card, in window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub w: f32,
    pub h: f32,
    pub k: f32,
    /// The card, border included.
    pub card: Rect,
    /// The overline's line-box top.
    pub overline_top: f32,
    pub overline_size: f32,
    /// The title's line-box top and font size.
    pub title_top: f32,
    pub title_size: f32,
    /// The radar grid (72 CSS px square, border included).
    pub radar: Rect,
    /// The progress track (the card's content width, 8 CSS px tall, border included).
    pub track: Rect,
    /// The status row's line-box top and font size.
    pub status_top: f32,
    pub status_size: f32,
}

/// CSS line-height `normal` for Courier New is 1.1328 em; its baseline sits 0.8325 em below the top.
const LINE_NORMAL: f32 = 1.1328;
const ASCENT_NORMAL: f32 = 0.8325;
/// `.boot-loading__title` line-height .95. For Impact-class metrics the baseline is 0.87395 em below
/// the line-box top (the content area of 1.2197 em centred in the box).
const TITLE_LINE: f32 = 0.95;
const TITLE_BASELINE: f32 = 0.87395;
const GAP: f32 = 16.0;
const RADAR_WRAP: f32 = 80.0;
const STATUS_SIZE: f32 = 9.92;

fn clampf(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

impl Layout {
    pub fn new(w: f32, h: f32) -> Layout {
        let k = scale_for(w, h);
        // the web viewport, in CSS pixels
        let vw = w / k;
        let vh = h / k;
        let pad_overlay = clampf(0.035 * vw, 16.0, 44.0);
        let pad_card = clampf(0.04 * vw, 24.0, 36.0);
        let overline = clampf(0.012 * vw, 8.8, 11.2);
        let title = clampf(0.06 * vw, 28.8, 44.8);
        let card_w = (vw - 2.0 * pad_overlay).min(480.0);
        let overline_h = LINE_NORMAL * overline;
        let title_h = TITLE_LINE * title;
        let status_h = LINE_NORMAL * STATUS_SIZE;
        let progress_h = 8.0 + 8.0 + status_h;
        // two 1 px borders, two paddings, the four children and their three 16 px gaps
        let card_h = 2.0
            + 2.0 * pad_card
            + overline_h
            + GAP
            + title_h
            + GAP
            + RADAR_WRAP
            + GAP
            + progress_h;
        let card_x = (vw - card_w) / 2.0;
        let card_y = (vh - card_h) / 2.0;
        let inner_x = card_x + 1.0 + pad_card;
        let inner_w = card_w - 2.0 - 2.0 * pad_card;
        let overline_top = card_y + 1.0 + pad_card;
        let title_top = overline_top + overline_h + GAP;
        let radar_wrap_top = title_top + title_h + GAP;
        let track_top = radar_wrap_top + RADAR_WRAP + GAP;
        let status_top = track_top + 16.0;
        // the radar wrap centres its 72 px grid; its 4 px top margin sits above the grid
        let radar_left = card_x + card_w / 2.0 - 36.0;
        let radar_top = radar_wrap_top + 4.0;
        Layout {
            w,
            h,
            k,
            card: Rect::new(card_x * k, card_y * k, card_w * k, card_h * k),
            overline_top: overline_top * k,
            overline_size: overline * k,
            title_top: title_top * k,
            title_size: title * k,
            radar: Rect::new(radar_left * k, radar_top * k, 72.0 * k, 72.0 * k),
            track: Rect::new(inner_x * k, track_top * k, inner_w * k, 8.0 * k),
            status_top: status_top * k,
            status_size: STATUS_SIZE * k,
        }
    }

    fn centre_x(&self) -> f32 {
        self.card.x + self.card.w / 2.0
    }
}

// -- colour ---------------------------------------------------------------------------------

/// An 8-bit sRGB colour with a 0..1 alpha.
const fn c8(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

const INK: (u8, u8, u8) = (240, 231, 220);
const AMBER: (u8, u8, u8) = (242, 187, 98);
const RED: (u8, u8, u8) = (230, 68, 81);
const TITLE_INK: (u8, u8, u8) = (238, 231, 229);
const TITLE_SHADOW: (u8, u8, u8) = (33, 14, 27);
const RADAR_RIM: (u8, u8, u8) = (115, 89, 109);
const TRACK_EDGE: (u8, u8, u8) = (74, 57, 69);
const TRACK_FILL: (u8, u8, u8) = (12, 8, 13);
const BAR_TOP: (u8, u8, u8) = (182, 45, 62);
const BAR_BOTTOM: (u8, u8, u8) = (111, 25, 39);
const BAR_EDGE: (u8, u8, u8) = (250, 108, 114);

fn rgb(c: (u8, u8, u8), a: f32) -> Color {
    c8(c.0, c.1, c.2, a)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn to3(c: (u8, u8, u8)) -> [f32; 3] {
    [c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0]
}

/// `over`: a colour at alpha `a` laid over the running colour `c`.
fn over(c: [f32; 3], src: [f32; 3], a: f32) -> [f32; 3] {
    mix3(c, src, a)
}

// -- the background bake --------------------------------------------------------------------

/// The overlay's background, scanlines and the card's shadow, computed once per window size:
/// `.boot-loading` (three gradient layers over #020103), `.boot-loading__scanlines` (opacity .16)
/// and the card's `box-shadow: 0 24px 90px rgba(0,0,0,.78)`.
fn bake(l: &Layout) -> Image {
    let (w, h) = (l.w as u32, l.h as u32);
    let (wf, hf, k) = (l.w, l.h, l.k);
    // radial-gradient(circle at 76% 36%): the 28% stop is of the farthest-corner distance
    let (rcx, rcy) = (0.76 * wf, 0.36 * hf);
    let far = [(0.0, 0.0), (wf, 0.0), (0.0, hf), (wf, hf)]
        .iter()
        .map(|&(x, y)| ((x - rcx).powi(2) + (y - rcy).powi(2)).sqrt())
        .fold(0.0f32, f32::max);
    let r_end = 0.28 * far;
    // repeating-linear-gradient(45deg): the gradient line's half length, from its centred start
    let half_len = (wf + hf) * FRAC_1_SQRT_2 * 0.5;
    // the card's shadow: a blurred rectangle, offset 24 px down, sigma 45 px
    let sigma = 45.0 * k;
    let off = 24.0 * k;
    let (x0, x1) = (l.card.x, l.card.x + l.card.w);
    let (y0, y1) = (l.card.y + off, l.card.y + l.card.h + off);
    let cov_x: Vec<f32> = (0..w)
        .map(|x| {
            let x = x as f32 + 0.5;
            phi((x - x0) / sigma) - phi((x - x1) / sigma)
        })
        .collect();
    let cov_y: Vec<f32> = (0..h)
        .map(|y| {
            let y = y as f32 + 0.5;
            phi((y - y0) / sigma) - phi((y - y1) / sigma)
        })
        .collect();

    let mut img = Image::gen_image_color(w as u16, h as u16, BLACK);
    let data = img.get_image_data_mut();
    let q = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let stripe_a = to3((93, 58, 72));
    let stripe_b = to3((22, 16, 25));
    let radial = to3((185, 113, 49));
    let scan = to3((220, 190, 210));
    for y in 0..h as usize {
        let fy = y as f32 + 0.5;
        let ts = fy.rem_euclid(4.0 * k);
        for x in 0..w as usize {
            let fx = x as f32 + 0.5;
            let mut c = to3((2, 1, 3));
            // the 45deg stripes: 18 px of each colour, hard stops
            let p = (fx - wf * 0.5 - (fy - hf * 0.5)) * FRAC_1_SQRT_2 + half_len;
            let stripe = if p.rem_euclid(36.0 * k) < 18.0 * k {
                stripe_a
            } else {
                stripe_b
            };
            c = over(c, stripe, 0.14);
            // the radial glow, amber fading to transparent at r_end
            let d = ((fx - rcx).powi(2) + (fy - rcy).powi(2)).sqrt();
            if d < r_end {
                c = over(c, radial, 0.14 * (1.0 - d / r_end));
            }
            // linear-gradient(90deg): 5,3,6 .98 at 0%, 7,4,8 .93 at 46%, 11,6,13 .7 at 100%
            let (lin, la) = linear_stop(fx / wf);
            c = over(c, lin, la);
            // scanlines: a ramp from transparent at 3 px to .2 at 4 px, repeating, times .16
            if ts >= 3.0 * k {
                c = over(c, scan, 0.16 * 0.2 * (ts - 3.0 * k) / k);
            }
            // the card's shadow
            let shadow = 0.78 * cov_x[x] * cov_y[y];
            c = over(c, [0.0; 3], shadow);
            data[y * w as usize + x] = [q(c[0]), q(c[1]), q(c[2]), 255];
        }
    }
    img
}

/// The `linear-gradient(90deg, ...)` colour and alpha at fraction `f` across the width.
fn linear_stop(f: f32) -> ([f32; 3], f32) {
    let s0 = ([5.0, 3.0, 6.0], 0.98);
    let s1 = ([7.0, 4.0, 8.0], 0.93);
    let s2 = ([11.0, 6.0, 13.0], 0.70);
    let (a, b, t) = if f <= 0.46 {
        (s0, s1, (f / 0.46).clamp(0.0, 1.0))
    } else {
        (s1, s2, ((f - 0.46) / 0.54).clamp(0.0, 1.0))
    };
    let rgb = [
        (a.0[0] + (b.0[0] - a.0[0]) * t) / 255.0,
        (a.0[1] + (b.0[1] - a.0[1]) * t) / 255.0,
        (a.0[2] + (b.0[2] - a.0[2]) * t) / 255.0,
    ];
    (rgb, a.1 + (b.1 - a.1) * t)
}

// -- shapes ---------------------------------------------------------------------------------

/// Concentric bands `(radius, colour)`, drawn as one mesh. Adjacent rings are joined by quads, so
/// a first ring of radius 0 makes a filled disc.
fn ring_mesh(cx: f32, cy: f32, rings: &[(f32, Color)]) -> Mesh {
    const SEGS: usize = 48;
    let mut vertices = Vec::with_capacity(rings.len() * SEGS);
    for &(r, col) in rings {
        for s in 0..SEGS {
            let a = TAU * s as f32 / SEGS as f32;
            vertices.push(Vertex::new(cx + r * a.sin(), cy - r * a.cos(), 0.0, 0.0, 0.0, col));
        }
    }
    let mut indices = Vec::with_capacity(rings.len().saturating_sub(1) * SEGS * 6);
    for ring in 0..rings.len().saturating_sub(1) {
        for s in 0..SEGS {
            let a0 = (ring * SEGS + s) as u16;
            let a1 = (ring * SEGS + (s + 1) % SEGS) as u16;
            let b0 = a0 + SEGS as u16;
            let b1 = a1 + SEGS as u16;
            indices.extend_from_slice(&[a0, b0, a1, a1, b0, b1]);
        }
    }
    Mesh {
        vertices,
        indices,
        texture: None,
    }
}

/// The radar sweep: a 55 degree wedge from `angle` (clockwise from 12 o'clock), alpha falling from
/// .45 at its leading edge to 0 at its trailing edge (the web's conic gradient), clipped to radius `r`.
fn sweep_mesh(cx: f32, cy: f32, r: f32, angle: f32, alpha: f32) -> Mesh {
    let at = |deg: f32| {
        let a = angle + deg.to_radians();
        (cx + r * a.sin(), cy - r * a.cos())
    };
    let col = |deg: f32| c8(RED.0, RED.1, RED.2, alpha * 0.45 * (1.0 - deg / 55.0).max(0.0));
    let mut vertices = Vec::with_capacity(55 * 3);
    let mut indices = Vec::with_capacity(55 * 3);
    for i in 0..55 {
        let (d0, d1) = (i as f32, (i + 1) as f32);
        let base = vertices.len() as u16;
        let (x0, y0) = at(d0);
        let (x1, y1) = at(d1);
        vertices.push(Vertex::new(cx, cy, 0.0, 0.0, 0.0, col(d0 + 0.5)));
        vertices.push(Vertex::new(x0, y0, 0.0, 0.0, 0.0, col(d0)));
        vertices.push(Vertex::new(x1, y1, 0.0, 0.0, 0.0, col(d1)));
        indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    Mesh {
        vertices,
        indices,
        texture: None,
    }
}

/// A dashed 1 px circle: 3 px dashes and 3 px gaps, as Chrome draws a dashed border.
fn dashed_ring(cx: f32, cy: f32, r: f32, thick: f32, col: Color) {
    let n = ((TAU * r) / 6.0).round().max(8.0) as usize;
    let step = TAU / n as f32;
    let at = |a: f32| (cx + r * a.sin(), cy - r * a.cos());
    for i in (0..n).step_by(2) {
        let a0 = i as f32 * step;
        let segs = 4;
        for s in 0..segs {
            let t0 = a0 + step * s as f32 / segs as f32;
            let t1 = a0 + step * (s + 1) as f32 / segs as f32;
            let (x0, y0) = at(t0);
            let (x1, y1) = at(t1);
            draw_line(x0, y0, x1, y1, thick, col);
        }
    }
}

/// `Font` text at a CSS pixel size, with CSS letter-spacing, baseline at `base`.
fn put(font: Option<&Font>, s: &str, x: f32, base: f32, size: f32, spacing: f32, col: Color) {
    if s.is_empty() || col.a <= 0.0 {
        return;
    }
    let n = size.round().max(1.0);
    let scale = size / n;
    let params = |c: Color| TextParams {
        font,
        font_size: n as u16,
        font_scale: scale,
        color: c,
        ..Default::default()
    };
    if spacing == 0.0 {
        draw_text_ex(s, x, base, params(col));
        return;
    }
    let mut cx = x;
    let mut buf = [0u8; 4];
    for ch in s.chars() {
        let g = ch.encode_utf8(&mut buf);
        draw_text_ex(&*g, cx, base, params(col));
        cx += measure_text(&*g, font, n as u16, scale).width + spacing;
    }
}

/// The advance of `s` with CSS letter-spacing after every character, as CSS lays out a box.
fn text_w(font: Option<&Font>, s: &str, size: f32, spacing: f32) -> f32 {
    let n = size.round().max(1.0);
    let scale = size / n;
    let mut buf = [0u8; 4];
    let mut w = 0.0;
    let mut count = 0;
    for ch in s.chars() {
        let g = ch.encode_utf8(&mut buf);
        w += measure_text(&*g, font, n as u16, scale).width + spacing;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        w
    }
}

fn load_font(names: &[&str]) -> Option<Font> {
    let mut dirs: Vec<String> = [
        "C:/Windows/Fonts/",
        "/System/Library/Fonts/Supplemental/",
        "/Library/Fonts/",
        "/usr/share/fonts/truetype/msttcorefonts/",
        "/usr/share/fonts/TTF/",
        "/usr/share/fonts/msttcore/",
        "/usr/share/fonts/truetype/liberation/",
        "/usr/share/fonts/liberation/",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(format!("{home}/Library/Fonts/"));
        dirs.push(format!("{home}/.local/share/fonts/"));
        dirs.push(format!("{home}/.fonts/"));
    }
    for d in &dirs {
        for n in names {
            if let Ok(b) = std::fs::read(format!("{d}{n}")) {
                if let Ok(f) = load_ttf_font_from_bytes(&b) {
                    return Some(f);
                }
            }
        }
    }
    None
}

// -- the loader -----------------------------------------------------------------------------

/// The boot loader: the state, the animation clock, the fonts and the background bake.
pub struct BootLoader {
    state: BootState,
    /// The radar and pulse clock, in seconds (frame time clamped, so a long step does not jump it).
    clock: f32,
    /// `get_time()` when the loader opened: the watchdog's start.
    started: f64,
    body: Option<Font>,
    title: Option<Font>,
    bake: Option<Texture2D>,
}

impl BootLoader {
    /// Opens the loader. It loads its own fonts, so it can draw before `Assets` exists.
    pub fn new() -> BootLoader {
        BootLoader {
            state: BootState::new(),
            clock: 0.0,
            started: get_time(),
            // Courier New Bold for the overline, status and percent (the web's 700 and 800)
            body: load_font(&[
                "courbd.ttf",
                "Courier New Bold.ttf",
                "Courier_New_Bold.ttf",
                "LiberationMono-Bold.ttf",
            ]),
            // Arial Black first, as the web's title font stack
            title: load_font(&[
                "ariblk.ttf",
                "Arial Black.ttf",
                "impact.ttf",
                "Impact.ttf",
                "LiberationSans-Bold.ttf",
            ]),
            bake: None,
        }
    }

    /// A startup step has completed: the bar eases to its value and the status shows its text.
    pub fn reach(&mut self, step: Step) {
        self.state.reach(step);
    }

    /// A required load found a hard problem: the card shows the failure and stays up.
    pub fn fail(&mut self, message: &'static str) {
        self.state.fail(message);
    }

    /// The title has drawn: the card reads SIGNAL READY and fades out.
    pub fn ready(&mut self) {
        self.state.ready();
    }

    /// Whether the card is still loading. Clicks on the title are ignored only until the title has
    /// drawn; the fade-out does not hold input back. A failed load keeps blocking.
    pub fn blocks_input(&self) -> bool {
        self.state.failed().is_none() && !self.state.is_ready()
    }

    /// A startup step has completed: reach it, then draw a frame and yield to the window.
    pub async fn step(&mut self, step: Step) {
        self.reach(step);
        self.pump().await;
    }

    /// Draws a frame of the card and yields. Called between load steps, so the window stays alive.
    pub async fn pump(&mut self) {
        self.state.watchdog((get_time() - self.started) as f32);
        self.draw_frame();
        next_frame().await;
    }

    /// Draws the card for this frame. Call it last, over the title. Once it has faded it does nothing.
    pub fn draw_frame(&mut self) {
        let dt = get_frame_time().clamp(0.0, 0.1);
        self.clock += dt;
        let Some(alpha) = self.state.tick(dt) else {
            self.release();
            return;
        };
        if alpha <= 0.0 {
            return;
        }
        let lay = Layout::new(screen_width(), screen_height());
        self.draw(alpha, &lay);
    }

    /// Frees the fonts and the bake once the card is gone.
    fn release(&mut self) {
        self.body = None;
        self.title = None;
        self.bake = None;
    }

    fn ensure_bake(&mut self, lay: &Layout) {
        let fits = self
            .bake
            .as_ref()
            .map(|t| t.width() as f32 == lay.w && t.height() as f32 == lay.h)
            .unwrap_or(false);
        if !fits {
            let tex = Texture2D::from_image(&bake(lay));
            tex.set_filter(FilterMode::Nearest);
            self.bake = Some(tex);
        }
    }

    fn draw(&mut self, a: f32, l: &Layout) {
        self.ensure_bake(l);
        let k = l.k;
        let failed = self.state.failed().is_some();
        let pulse = if failed {
            1.0
        } else {
            pulse_alpha(self.clock)
        };
        let body = self.body.as_ref();
        let title_font = self.title.as_ref();

        // the background, the scanlines and the card's shadow
        if let Some(tex) = &self.bake {
            draw_texture(tex, 0.0, 0.0, Color::new(1.0, 1.0, 1.0, a));
        }

        // the card: rgba(13,8,15,.91) with a 1 px rgba(238,222,210,.2) border
        let card = l.card;
        draw_rectangle(card.x, card.y, card.w, card.h, c8(13, 8, 15, 0.91 * a));
        draw_rectangle_lines(card.x, card.y, card.w, card.h, k, c8(238, 222, 210, 0.2 * a));
        let cx = l.centre_x();

        // the overline: amber, 700, .19em
        let over = "SIGNAL ARCHIVE // ROUTE SYNC";
        let ls_over = 0.19 * l.overline_size;
        let ow = text_w(body, over, l.overline_size, ls_over);
        put(
            body,
            over,
            cx - ow / 2.0,
            l.overline_top + ASCENT_NORMAL * l.overline_size,
            l.overline_size,
            ls_over,
            rgb(AMBER, a),
        );

        // the title: Arial Black, -.075em, #eee7e5, with the web's two text shadows
        let title = "THE LAST ROUTE";
        let ts = l.title_size;
        let ls_title = -0.075 * ts;
        let tw = text_w(title_font, title, ts, ls_title);
        let tx = cx - tw / 2.0;
        let tb = l.title_top + TITLE_BASELINE * ts;
        // 0 0 30px rgba(230,68,81,.18): a faint halo, drawn as offset copies
        for radius in [6.0, 14.0, 24.0] {
            for i in 0..8 {
                let a0 = TAU * i as f32 / 8.0;
                let r = radius * k;
                put(
                    title_font,
                    title,
                    tx + r * a0.cos(),
                    tb + r * a0.sin(),
                    ts,
                    ls_title,
                    c8(RED.0, RED.1, RED.2, 0.008 * a),
                );
            }
        }
        // 4px 5px 0 #210e1b, then the title itself
        put(
            title_font,
            title,
            tx + 4.0 * k,
            tb + 5.0 * k,
            ts,
            ls_title,
            rgb(TITLE_SHADOW, a),
        );
        put(title_font, title, tx, tb, ts, ls_title, rgb(TITLE_INK, a));

        self.draw_radar(l, a);
        self.draw_progress(l, a, failed, pulse, body);
    }

    fn draw_radar(&self, l: &Layout, a: f32) {
        let k = l.k;
        let (cx, cy) = (l.radar.x + 36.0 * k, l.radar.y + 36.0 * k);
        let r_pad = 35.0 * k;
        let r_out = 36.0 * k;

        // box-shadow: 0 0 16px rgba(0,0,0,.5), outside the rim
        let shadow: Vec<(f32, Color)> = (0..=12)
            .map(|i| {
                let d = 2.0 * i as f32;
                let alpha = 0.5 * a * (1.0 - phi(d / 8.0));
                (r_out + d * k, c8(0, 0, 0, alpha))
            })
            .collect();
        draw_mesh(&ring_mesh(cx, cy, &shadow));

        // the disc: radial-gradient(circle, rgba(42,17,28,.6), rgba(14,8,15,.95)) over the 49.5 px
        // farthest-corner radius, with the inset glow rgba(230,68,81,.2) 10 px deep
        let fill: Vec<(f32, Color)> = (0..=16)
            .map(|i| {
                let r_css = 35.0 * i as f32 / 16.0;
                let t = (r_css / 49.497_48).clamp(0.0, 1.0);
                let base = [
                    (42.0 + (14.0 - 42.0) * t) / 255.0,
                    (17.0 + (8.0 - 17.0) * t) / 255.0,
                    (28.0 + (15.0 - 28.0) * t) / 255.0,
                ];
                let base_a = 0.6 + (0.95 - 0.6) * t;
                let inset = 0.2 * phi(-(35.0 - r_css) / 5.0);
                let red = [RED.0 as f32 / 255.0, RED.1 as f32 / 255.0, RED.2 as f32 / 255.0];
                let c = mix3(base, red, inset);
                (r_css * k, Color::new(c[0], c[1], c[2], base_a * a))
            })
            .collect();
        draw_mesh(&ring_mesh(cx, cy, &fill));
        // the 1 px #73596d rim (72 px outer, 70 px inside)
        draw_mesh(&ring_mesh(
            cx,
            cy,
            &[(r_pad, rgb(RADAR_RIM, a)), (r_out, rgb(RADAR_RIM, a))],
        ));

        // rings at 45% and 90% of the 70 px inside, dashed, rgba(238,222,210,.2)
        let ring_col = c8(238, 222, 210, 0.2 * a);
        dashed_ring(cx, cy, 15.25 * k, k, ring_col);
        dashed_ring(cx, cy, 31.0 * k, k, ring_col);

        // crosshairs, rgba(238,222,210,.15), 1 px through the centre
        let cross = c8(238, 222, 210, 0.15 * a);
        draw_rectangle(l.radar.x, l.radar.y + 36.0 * k, 72.0 * k, k, cross);
        draw_rectangle(l.radar.x + 36.0 * k, l.radar.y, k, 72.0 * k, cross);

        // the sweep: clockwise, 2.4 s a turn
        draw_mesh(&sweep_mesh(cx, cy, r_pad, sweep_angle(self.clock), a));

        // the blip: 5 px amber dot at (49, 24.5) of the grid, 6 px glow, pulsing 2.4 s
        let (op, scale) = blip_pulse(self.clock);
        let (bx, by) = (l.radar.x + 49.0 * k, l.radar.y + 24.5 * k);
        let core = 2.5 * scale;
        let mut blip: Vec<(f32, Color)> = vec![
            (0.0, c8(AMBER.0, AMBER.1, AMBER.2, op * a)),
            (core * k, c8(AMBER.0, AMBER.1, AMBER.2, op * a)),
        ];
        for i in 1..=6 {
            let d = 1.5 * i as f32;
            let alpha = op * a * (1.0 - phi(d / 3.0));
            blip.push(((core + d) * k, c8(AMBER.0, AMBER.1, AMBER.2, alpha)));
        }
        draw_mesh(&ring_mesh(bx, by, &blip));
    }

    fn draw_progress(&self, l: &Layout, a: f32, failed: bool, pulse: f32, body: Option<&Font>) {
        let k = l.k;
        let t = l.track;
        // the track: 1 px #4a3945 border, #0c080d fill, inset 0 1px 3px rgba(0,0,0,.6)
        draw_rectangle(t.x, t.y, t.w, t.h, rgb(TRACK_EDGE, a));
        let inner = Rect::new(t.x + k, t.y + k, t.w - 2.0 * k, t.h - 2.0 * k);
        draw_rectangle(inner.x, inner.y, inner.w, inner.h, rgb(TRACK_FILL, a));
        for (i, al) in [0.38, 0.22, 0.10].iter().enumerate() {
            draw_rectangle(inner.x, inner.y + i as f32 * k, inner.w, k, c8(0, 0, 0, al * a));
        }

        let bw = (inner.w * self.state.shown().clamp(0.0, 1.0)).min(inner.w);
        if bw <= 0.0 {
            return;
        }
        // the bar's outer glow, 0 0 12px rgba(230,68,81,.4): clipped by the track
        for i in 0..(inner.w - bw).max(0.0) as usize {
            let d = i as f32 + 0.5;
            if d > 18.0 {
                break;
            }
            let alpha = 0.4 * a * (1.0 - phi(d / 6.0));
            draw_rectangle(
                inner.x + bw + i as f32 * k,
                inner.y,
                k,
                inner.h,
                c8(RED.0, RED.1, RED.2, alpha),
            );
        }
        // the bar: #b62d3e to #6f1927 down the track, or solid red after a failure
        let rows = inner.h.round().max(1.0) as usize;
        let row_h = inner.h / rows as f32;
        for row in 0..rows {
            let col = if failed {
                rgb(RED, a)
            } else {
                let t = (row as f32 + 0.5) / rows as f32;
                let m = mix3(to3(BAR_TOP), to3(BAR_BOTTOM), t);
                Color::new(m[0], m[1], m[2], a)
            };
            draw_rectangle(inner.x, inner.y + row as f32 * row_h, bw, row_h, col);
        }
        // inset 0 1px rgba(255,255,255,.2) on the bar's top edge, and its 1 px #fa6c72 right edge
        draw_rectangle(inner.x, inner.y, (bw - k).max(0.0), k, c8(255, 255, 255, 0.2 * a));
        if bw >= k {
            draw_rectangle(inner.x + bw - k, inner.y, k, inner.h, rgb(BAR_EDGE, a));
        }

        // the status row: the status text (ink, or red after a failure) and the percent (amber)
        let base = l.status_top + ASCENT_NORMAL * l.status_size;
        let ls = 0.08 * l.status_size;
        // the failure replaces the status text in red; neither span pulses after a failure
        let (status_col, status_a) = if failed {
            (RED, a)
        } else {
            (INK, a * pulse)
        };
        let status = self.state.status();
        put(body, status, t.x, base, l.status_size, ls, rgb(status_col, status_a));

        let pct = format!("{}%", self.state.percent());
        let pw = text_w(body, &pct, l.status_size, ls);
        put(
            body,
            &pct,
            t.x + t.w - pw,
            base,
            l.status_size,
            ls,
            c8(AMBER.0, AMBER.1, AMBER.2, a * pulse),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn each_startup_step_has_the_web_percent_and_text() {
        let table = [
            (Step::Connect, 18, "CONNECTING TO RELAY..."),
            (Step::Route, 35, "LOADING ROUTE DATA..."),
            (Step::Art, 65, "LOADING FACILITY ART..."),
            (Step::Signal, 88, "PREPARING SIGNAL..."),
            (Step::Saves, 96, "SYNCING SAVES..."),
            (Step::Ready, 100, "SIGNAL READY // ENGAGING"),
        ];
        for (step, pct, text) in table {
            assert_eq!(step.percent(), pct, "{step:?} percent");
            assert_eq!(step.text(), text, "{step:?} text");
        }
        // the web's ladder only ever rises
        let pcts: Vec<u32> = table.iter().map(|t| t.0.percent()).collect();
        assert!(pcts.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn a_fresh_state_shows_the_connect_step() {
        let s = BootState::new();
        assert_eq!(s.step(), Step::Connect);
        assert_eq!(s.percent(), 18);
        assert_eq!(s.status(), "CONNECTING TO RELAY...");
        assert!(close(s.shown(), 0.18, 1e-6));
        assert!(!s.is_done());
    }

    #[test]
    fn steps_move_forward_only_and_show_their_text() {
        let mut s = BootState::new();
        s.reach(Step::Art);
        assert_eq!((s.percent(), s.status()), (65, "LOADING FACILITY ART..."));
        s.reach(Step::Route);
        assert_eq!(s.step(), Step::Art, "a step never moves back");
        s.reach(Step::Saves);
        assert_eq!((s.percent(), s.status()), (96, "SYNCING SAVES..."));
    }

    #[test]
    fn the_bar_eases_toward_each_step() {
        let mut s = BootState::new();
        s.reach(Step::Route);
        let mut last = s.shown();
        for _ in 0..200 {
            let before = s.shown();
            s.tick(1.0 / 60.0);
            assert!(s.shown() >= before - 1e-6, "the bar never moves back");
            last = s.shown();
        }
        assert!(close(last, 0.35, 1e-3), "the bar settles on 35% (got {last})");
    }

    #[test]
    fn a_failure_keeps_the_last_percent_and_replaces_the_text() {
        let mut s = BootState::new();
        s.reach(Step::Route);
        s.fail(FAIL_LOAD);
        assert_eq!(s.failed(), Some(FAIL_LOAD));
        assert_eq!(s.status(), FAIL_LOAD);
        assert_eq!(s.percent(), 35, "the bar keeps its last value");
        s.reach(Step::Art);
        assert_eq!(s.step(), Step::Route, "no progress after a failure");
        s.ready();
        assert_eq!(s.step(), Step::Route, "a failed load never reaches ready");
        for _ in 0..600 {
            assert!(s.tick(1.0 / 60.0).is_some(), "a failed loader never fades out");
        }
        assert!(!s.is_done());
    }

    #[test]
    fn the_watchdog_fails_a_load_that_runs_past_twenty_seconds() {
        let mut s = BootState::new();
        s.reach(Step::Route);
        s.watchdog(19.9);
        assert_eq!(s.failed(), None);
        s.watchdog(20.0);
        assert_eq!(s.failed(), Some(FAIL_SLOW));
        assert_eq!(s.status(), "LOAD TAKING TOO LONG // RESTART");
    }

    #[test]
    fn the_first_failure_wins_and_a_finished_load_cannot_fail() {
        let mut s = BootState::new();
        s.fail(FAIL_LOAD);
        s.fail(FAIL_SLOW);
        assert_eq!(s.failed(), Some(FAIL_LOAD));

        let mut done = BootState::new();
        done.ready();
        done.fail(FAIL_LOAD);
        assert_eq!(done.failed(), None, "fail() is a no-op once ready");
        assert_eq!(done.status(), "SIGNAL READY // ENGAGING");
    }

    #[test]
    fn the_fade_starts_on_the_frame_after_ready_and_removes_the_card_at_260_ms() {
        let mut s = BootState::new();
        s.reach(Step::Saves);
        // no fade before ready: the card is fully opaque
        assert_eq!(s.tick(0.02), Some(1.0));
        s.ready();
        // the first frame after ready is drawn at full strength (the web's rAF wait)
        assert_eq!(s.tick(0.01), Some(1.0));
        let mut t = 0.01f32;
        let alpha_at = |s: &mut BootState, t: f32| s.tick(0.01).map(|a| (t, a));
        let mut removed_at = None;
        let mut last = 1.0f32;
        for _ in 0..60 {
            match alpha_at(&mut s, t) {
                Some((_, a)) => {
                    assert!(a <= last + 1e-6, "the fade only darkens");
                    last = a;
                }
                None => {
                    removed_at = Some(t);
                    break;
                }
            }
            t += 0.01;
        }
        let removed = removed_at.expect("the card is removed");
        assert!(
            (0.25..=0.28).contains(&removed),
            "removed about 260 ms after the fade starts (at {removed})"
        );
        assert!(s.is_done());
        assert!(s.tick(0.01).is_none(), "nothing draws after removal");
    }

    #[test]
    fn the_fade_curve_is_css_ease_over_240_ms() {
        assert!(close(fade_alpha(0.0), 1.0, 1e-6));
        assert!(close(fade_alpha(FADE_SECS), 0.0, 1e-4));
        assert!(close(fade_alpha(0.5), 0.0, 1e-6), "clamped after the fade");
        // CSS ease at the midpoint is about 0.80, so the card is about 20% opaque at 120 ms
        let mid = fade_alpha(0.12);
        assert!((0.15..=0.25).contains(&mid), "alpha at 120 ms is {mid}");
        assert!(close(ease(0.0), 0.0, 1e-6) && close(ease(1.0), 1.0, 1e-6));
        assert!(close(ease_in_out(0.5), 0.5, 1e-3), "ease-in-out is symmetric");
    }

    #[test]
    fn the_pulses_follow_their_css_periods() {
        // the status pulse: .68 at 0, 1 at 1.4 s, back to .68 at 2.8 s (alternate)
        assert!(close(pulse_alpha(0.0), 0.68, 1e-4));
        assert!(close(pulse_alpha(1.4), 1.0, 1e-4));
        assert!(close(pulse_alpha(2.8), 0.68, 1e-4));
        assert!(close(pulse_alpha(0.7), 0.84, 1e-3), "half way is ease-in-out's midpoint");
        // the blip: .15 and .9 at 0, full 1 and 1.1 at 1.2 s (the half period), 2.4 s a cycle
        let (o0, s0) = blip_pulse(0.0);
        assert!(close(o0, 0.15, 1e-4) && close(s0, 0.9, 1e-4));
        let (o1, s1) = blip_pulse(1.2);
        assert!(close(o1, 1.0, 1e-4) && close(s1, 1.1, 1e-4));
        let (o2, _) = blip_pulse(2.4);
        assert!(close(o2, 0.15, 1e-4));
        // the sweep turns once every 2.4 s, clockwise from 12 o'clock
        assert!(close(sweep_angle(0.0), 0.0, 1e-6));
        assert!(close(sweep_angle(1.2), PI, 1e-4));
        assert!(close(sweep_angle(2.4), 0.0, 1e-4));
    }

    #[test]
    fn the_card_is_the_web_card_at_1280_by_720() {
        assert!(close(scale_for(1280.0, 720.0), 1.0, 1e-6));
        assert!(close(scale_for(1920.0, 1080.0), 1.5, 1e-6));
        let l = Layout::new(1280.0, 720.0);
        // the web card: 480 px wide, 36 px padding, 406 px of content
        assert!(close(l.card.w, 480.0, 1e-3));
        assert!(close(l.card.x, 400.0, 1e-3));
        // 2 borders + 72 padding + 12.69 overline + 42.56 title + 80 radar + 27.24 progress + 48 gaps
        assert!(close(l.card.h, 284.49, 0.02), "card height {}", l.card.h);
        assert!(close(l.card.y, (720.0 - l.card.h) / 2.0, 1e-3), "the card is centred");
        assert!(close(l.track.w, 406.0, 1e-3), "the track is the card content width");
        assert!(close(l.radar.w, 72.0, 1e-3));
        assert!(close(l.radar.x, 400.0 + 240.0 - 36.0, 1e-3), "the grid is centred");
        assert!(close(l.overline_size, 11.2, 1e-3));
        assert!(close(l.title_size, 44.8, 1e-3));
    }

    #[test]
    fn a_narrow_window_scales_the_card_with_the_design_canvas() {
        // like the other screens, a window narrower than 480 units scales the whole canvas: at
        // 390 px wide the 480 px card is 480 * scale, centred, and fits the window
        let l = Layout::new(390.0, 844.0);
        let k = scale_for(390.0, 844.0);
        assert!(close(l.card.w, 480.0 * k, 1e-3));
        assert!(l.card.x >= 0.0 && l.card.x + l.card.w <= 390.0 + 1e-3);
        assert!(close(l.card.x + l.card.w / 2.0, 195.0, 1e-3), "the card is centred");
    }
}
