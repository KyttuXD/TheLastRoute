//! The shared look of every screen, after the original's game/config/palette.gd, ui_theme.gd
//! (ref_theme.tres) and signal_backdrop.gd.
//!
//! The original lays every screen out on a 480x270 design canvas with Godot's `canvas_items`
//! stretch and `expand` aspect: one design unit is `min(w / 480, h / 270)` window pixels, the
//! canvas grows along the longer axis, and anchors are relative to the grown canvas. Every
//! position and font size here is in those design units ("u"), so a screen scales exactly
//! like the original at any window size.
//!
//! Colours come from the palette tokens of the active scheme (Last Route's purple, or the
//! City's navy glass from palette.gd SCHEMES); the project theme (`ref_theme.tres`) is a fixed
//! purple resource the original never re-tints, so its controls (tab bars, sliders) read
//! `REF_*` constants instead.

use crate::app::assets::Assets;
use macroquad::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const fn hex(v: u32) -> Color {
    Color::new(
        ((v >> 16) & 255) as f32 / 255.0,
        ((v >> 8) & 255) as f32 / 255.0,
        (v & 255) as f32 / 255.0,
        1.0,
    )
}

pub const fn hexa(v: u32) -> Color {
    Color::new(
        ((v >> 24) & 255) as f32 / 255.0,
        ((v >> 16) & 255) as f32 / 255.0,
        ((v >> 8) & 255) as f32 / 255.0,
        (v & 255) as f32 / 255.0,
    )
}

pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::new(r, g, b, a)
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, a)
}

/// Godot's Color.lightened: towards white by `amount`.
pub fn lightened(c: Color, amount: f32) -> Color {
    Color::new(
        c.r + (1.0 - c.r) * amount,
        c.g + (1.0 - c.g) * amount,
        c.b + (1.0 - c.b) * amount,
        c.a,
    )
}

/// Godot's Color.darkened: towards black by `amount`.
pub fn darkened(c: Color, amount: f32) -> Color {
    Color::new(
        c.r * (1.0 - amount),
        c.g * (1.0 - amount),
        c.b * (1.0 - amount),
        c.a,
    )
}

// -- palette (palette.gd) -------------------------------------------------------------------

/// One colour scheme's tokens (palette.gd's static vars).
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct Pal {
    pub backdrop: Color,
    pub panel: Color,
    pub panel_edge: Color,
    pub inset: Color,
    pub scrim: Color,
    pub ink: Color,
    pub body: Color,
    pub muted: Color,
    pub dim: Color,
    pub disabled: Color,
    pub amber: Color,
    pub amber_fill: Color,
    pub on_amber: Color,
    pub accent: Color,
    pub accent_edge: Color,
    pub accent_deep: Color,
    pub teal: Color,
    pub teal_bright: Color,
    pub teal_deep: Color,
    pub alert: Color,
    pub stable: Color,
    pub field_edge: Color,
    pub keycap_edge: Color,
    /// Corner radius of panels and buttons (0 for Last Route, 3 for the City).
    pub corner: f32,
    pub edge_width: f32,
}

pub const LAST_ROUTE: Pal = Pal {
    backdrop: hex(0x0d0810),
    panel: hex(0x170e19),
    panel_edge: hex(0x2e1f33),
    inset: hex(0x1e1322),
    scrim: rgba(0.05, 0.03, 0.07, 0.82),
    ink: hex(0xf2ede1),
    body: hex(0xbcb3a8),
    muted: hex(0x8c817e),
    dim: hex(0x4c4449),
    disabled: hex(0x766e72),
    amber: hex(0xc9a9d8),
    amber_fill: hex(0xb48ec0),
    on_amber: hex(0x1a0f14),
    accent: hex(0xa67fb0),
    accent_edge: hex(0xc4a2d0),
    accent_deep: hex(0x5a3f63),
    teal: hex(0x37b9b0),
    teal_bright: hex(0xbff5ec),
    teal_deep: hex(0x1f6e6a),
    alert: hex(0xe2515d),
    stable: hex(0x6fbf8a),
    field_edge: hex(0x4b3948),
    keycap_edge: hex(0x6a5264),
    corner: 0.0,
    edge_width: 1.0,
};

/// The City's night scheme (palette.gd SCHEMES["city"]).
pub const CITY: Pal = Pal {
    backdrop: hex(0x070a18),
    panel: hex(0x0d1128),
    panel_edge: hex(0x2a3a7a),
    inset: hex(0x141a3c),
    scrim: hexa(0x04061488),
    ink: hex(0xeaf2ff),
    body: hex(0xb9c4de),
    muted: hex(0x8a94b4),
    dim: hex(0x454c6a),
    disabled: hex(0x7d87a8),
    amber: hex(0x8fe3f5),
    amber_fill: hex(0x55c7e0),
    on_amber: hex(0x070a18),
    accent: hex(0xf5a742),
    accent_edge: hex(0xffd28a),
    accent_deep: hex(0x6b4412),
    teal: hex(0x4ff0d2),
    teal_bright: hex(0xc8fff4),
    teal_deep: hex(0x1b7a70),
    alert: hex(0xff5c7a),
    stable: hex(0x5de0a0),
    field_edge: hex(0x3b4c96),
    keycap_edge: hex(0x56689f),
    corner: 3.0,
    edge_width: 1.0,
};

static SCHEME: AtomicUsize = AtomicUsize::new(0);

/// The active scheme's tokens.
pub fn pal() -> Pal {
    if SCHEME.load(Ordering::Relaxed) == 1 {
        CITY
    } else {
        LAST_ROUTE
    }
}

/// "last_route" or "city" (palette.gd scheme ids); unknown ids keep Last Route's.
pub fn set_scheme(id: &str) {
    SCHEME.store(if id == "city" { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn is_last_route() -> bool {
    SCHEME.load(Ordering::Relaxed) == 0
}

// The project theme (ref_theme.tres), fixed purple whatever the scheme.
pub const REF_PANEL: Color = hex(0x170e19);
pub const REF_PANEL_EDGE: Color = hex(0x2e1f33);
pub const REF_INSET: Color = hex(0x1e1322);
pub const REF_ACCENT: Color = hex(0xa67fb0);
pub const REF_ACCENT_EDGE: Color = hex(0xc4a2d0);
pub const REF_TEAL: Color = hex(0x37b9b0);
pub const REF_BODY: Color = hex(0xbcb3a8);
pub const REF_INK: Color = hex(0xf2ede1);
pub const REF_ON_AMBER: Color = hex(0x1a0f14);

// HUD meter colours (hud.gd), shared with the maps.
pub const METER_LOW: Color = rgba(0.902, 0.267, 0.318, 1.0);
pub const METER_WARN: Color = rgba(0.949, 0.733, 0.384, 1.0);
pub const METER_SAFE: Color = rgba(0.455, 0.722, 0.553, 1.0);
pub const STAMINA: Color = rgba(0.941, 0.761, 0.392, 1.0);

// -- the design canvas ----------------------------------------------------------------------

/// Window pixels per design unit (the 480x270 canvas, canvas_items stretch).
pub fn unit() -> f32 {
    (screen_width() / 480.0)
        .min(screen_height() / 270.0)
        .max(0.05)
}

/// The canvas size in design units (expand aspect: the long axis grows past 480 or 270).
pub fn view() -> Vec2 {
    let s = unit();
    vec2(screen_width() / s, screen_height() / s)
}

pub fn mouse_u() -> Vec2 {
    Vec2::from(mouse_position()) / unit()
}

pub fn hovered(r: Rect) -> bool {
    r.contains(mouse_u())
}

pub fn clicked(r: Rect) -> bool {
    hovered(r) && is_mouse_button_pressed(MouseButton::Left)
}

/// The pixel scale against 1280x720 (callers outside the UI module still pass it around).
pub fn ui_scale() -> f32 {
    (screen_width() / 1280.0)
        .min(screen_height() / 720.0)
        .max(0.25)
}

/// A rectangle centred on the canvas (Godot's PRESET_CENTER with offsets -w/2..w/2).
pub fn centered(w: f32, h: f32) -> Rect {
    let v = view();
    Rect::new((v.x - w) * 0.5, (v.y - h) * 0.5, w, h)
}

// -- shapes ---------------------------------------------------------------------------------

pub fn fill(r: Rect, col: Color) {
    if col.a <= 0.0 {
        return;
    }
    let s = unit();
    draw_rectangle(r.x * s, r.y * s, r.w * s, r.h * s, col);
}

/// A border drawn inside the rectangle (StyleBoxFlat's border), `width` in units.
pub fn border(r: Rect, width: f32, col: Color) {
    if col.a <= 0.0 || width <= 0.0 {
        return;
    }
    let s = unit();
    let (x, y, w, h, b) = (r.x * s, r.y * s, r.w * s, r.h * s, (width * s).max(1.0));
    draw_rectangle(x, y, w, b, col);
    draw_rectangle(x, y + h - b, w, b, col);
    draw_rectangle(x, y + b, b, h - 2.0 * b, col);
    draw_rectangle(x + w - b, y + b, b, h - 2.0 * b, col);
}

/// A stroke centred on the rectangle's edge (CanvasItem.draw_rect with filled = false).
pub fn stroke(r: Rect, width: f32, col: Color) {
    border(
        Rect::new(
            r.x - width * 0.5,
            r.y - width * 0.5,
            r.w + width,
            r.h + width,
        ),
        width,
        col,
    );
}

pub fn line(a: Vec2, b: Vec2, width: f32, col: Color) {
    let s = unit();
    draw_line(
        a.x * s,
        a.y * s,
        b.x * s,
        b.y * s,
        (width * s).max(1.0),
        col,
    );
}

pub fn circle(c: Vec2, r: f32, col: Color) {
    let s = unit();
    draw_circle(c.x * s, c.y * s, r * s, col);
}

pub fn tri(a: Vec2, b: Vec2, c: Vec2, col: Color) {
    let s = unit();
    draw_triangle(a * s, b * s, c * s, col);
}

fn corner_fan(cx: f32, cy: f32, r: f32, start: f32, col: Color) {
    let steps = 6;
    for i in 0..steps {
        let a0 = start + i as f32 / steps as f32 * std::f32::consts::FRAC_PI_2;
        let a1 = start + (i + 1) as f32 / steps as f32 * std::f32::consts::FRAC_PI_2;
        draw_triangle(
            vec2(cx, cy),
            vec2(cx + a0.cos() * r, cy + a0.sin() * r),
            vec2(cx + a1.cos() * r, cy + a1.sin() * r),
            col,
        );
    }
}

/// A filled rectangle with rounded corners (radius in units), drawn without overlaps so a
/// translucent colour stays even.
pub fn rounded(r: Rect, radius: f32, col: Color) {
    if radius <= 0.01 {
        fill(r, col);
        return;
    }
    if col.a <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let s = unit();
    let (x, y, w, h) = (r.x * s, r.y * s, r.w * s, r.h * s);
    let rad = (radius * s).min(w * 0.5).min(h * 0.5);
    draw_rectangle(x + rad, y, w - 2.0 * rad, h, col);
    draw_rectangle(x, y + rad, rad, h - 2.0 * rad, col);
    draw_rectangle(x + w - rad, y + rad, rad, h - 2.0 * rad, col);
    use std::f32::consts::PI;
    corner_fan(x + rad, y + rad, rad, PI, col);
    corner_fan(x + w - rad, y + rad, rad, PI * 1.5, col);
    corner_fan(x + w - rad, y + h - rad, rad, 0.0, col);
    corner_fan(x + rad, y + h - rad, rad, PI * 0.5, col);
}

/// A rounded ring (for translucent faces whose edge must not show through).
fn ring(r: Rect, radius: f32, width: f32, col: Color) {
    let s = unit();
    let (x, y, w, h) = (r.x * s, r.y * s, r.w * s, r.h * s);
    let rad = (radius * s).min(w * 0.5).min(h * 0.5);
    let b = (width * s).max(1.0);
    draw_rectangle(x + rad, y, w - 2.0 * rad, b, col);
    draw_rectangle(x + rad, y + h - b, w - 2.0 * rad, b, col);
    draw_rectangle(x, y + rad, b, h - 2.0 * rad, col);
    draw_rectangle(x + w - b, y + rad, b, h - 2.0 * rad, col);
    use std::f32::consts::PI;
    for (cx, cy, start) in [
        (x + rad, y + rad, PI),
        (x + w - rad, y + rad, PI * 1.5),
        (x + w - rad, y + h - rad, 0.0),
        (x + rad, y + h - rad, PI * 0.5),
    ] {
        let steps = 6;
        let inner = (rad - b).max(0.0);
        for i in 0..steps {
            let a0 = start + i as f32 / steps as f32 * PI * 0.5;
            let a1 = start + (i + 1) as f32 / steps as f32 * PI * 0.5;
            let (c0, s0, c1, s1) = (a0.cos(), a0.sin(), a1.cos(), a1.sin());
            let p0 = vec2(cx + c0 * rad, cy + s0 * rad);
            let p1 = vec2(cx + c1 * rad, cy + s1 * rad);
            let q0 = vec2(cx + c0 * inner, cy + s0 * inner);
            let q1 = vec2(cx + c1 * inner, cy + s1 * inner);
            draw_triangle(p0, p1, q1, col);
            draw_triangle(p0, q1, q0, col);
        }
    }
}

/// A StyleBoxFlat: fill, an inside border and a corner radius.
pub fn sbox(r: Rect, fill_col: Color, edge: Color, width: f32, radius: f32) {
    let inner = Rect::new(
        r.x + width,
        r.y + width,
        r.w - 2.0 * width,
        r.h - 2.0 * width,
    );
    if radius <= 0.01 {
        fill(inner, fill_col);
        border(r, width, edge);
        return;
    }
    if edge.a > 0.0 && width > 0.0 {
        ring(r, radius, width, edge);
    }
    rounded(inner, (radius - width).max(0.0), fill_col);
}

/// Palette.panel_box: a panel in the scheme's corner style.
pub fn panel_box(r: Rect, fill_col: Color, edge: Color) {
    let p = pal();
    sbox(r, fill_col, edge, p.edge_width, p.corner);
}

// -- text -----------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum F {
    /// Courier New, weight 600 (the theme's default font).
    Body,
    /// Impact (Palette.title_font).
    Title,
    /// Courier New italic (the death moment's captions).
    Italic,
}

impl F {
    /// Baseline below the top of the line box, per unit of font size.
    pub fn ascent(self) -> f32 {
        match self {
            F::Title => 1.0088,
            _ => 0.8325,
        }
    }

    /// Height of one line of text (the font's ascent + descent), per unit of font size.
    pub fn height(self) -> f32 {
        match self {
            F::Title => 1.2197,
            _ => 1.1328,
        }
    }

    /// Height a Label takes for one line, per unit of font size (measured on the original's
    /// screens: Courier labels sit on a slightly taller line than the font's own box).
    pub fn label_line(self) -> f32 {
        match self {
            F::Title => 1.282,
            _ => 1.254,
        }
    }
}

fn font_of(a: &Assets, f: F) -> Option<&Font> {
    match f {
        F::Body => a.font.as_ref(),
        F::Title => a.title_font.as_ref().or(a.font.as_ref()),
        F::Italic => a.caption_font.as_ref().or(a.font.as_ref()),
    }
}

fn px(size: f32) -> (u16, f32) {
    let p = (size * unit()).max(1.0);
    let n = p.round().max(1.0);
    (n as u16, p / n)
}

/// Width of a string in units.
pub fn tw(a: &Assets, f: F, text: &str, size: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    if f != F::Title && a.font.is_some() {
        // Courier New: every glyph advances 0.6 em, as Godot lays it out
        return text
            .chars()
            .map(|c| if c == THIN { 0.2 } else { 0.6 })
            .sum::<f32>()
            * size;
    }
    let (n, sc) = px(size);
    measure_text(text, font_of(a, f), n, sc).width / unit()
}

/// Draws a string with its baseline at `base` (units).
pub fn draw_str(a: &Assets, f: F, text: &str, x: f32, base: f32, size: f32, col: Color) {
    if text.is_empty() || col.a <= 0.0 {
        return;
    }
    if text.contains(THIN) {
        // tracked captions: each run of glyphs, then a fifth-of-an-em thin space
        let mut cx = x;
        for part in text.split(THIN) {
            draw_str(a, f, part, cx, base, size, col);
            cx += tw(a, f, part, size) + 0.2 * size;
        }
        return;
    }
    let s = unit();
    let (n, sc) = px(size);
    draw_text_ex(
        text,
        (x * s).round(),
        (base * s).round(),
        TextParams {
            font: font_of(a, f),
            font_size: n,
            font_scale: sc,
            color: col,
            ..Default::default()
        },
    );
}

/// A one-line Label whose box top is `top`: the baseline sits one ascent below.
pub fn label(a: &Assets, f: F, text: &str, x: f32, top: f32, size: f32, col: Color) {
    draw_str(a, f, text, x, top + f.ascent() * size, size, col);
}

/// A label centred vertically in a box of height `h` starting at `top`.
pub fn label_vc(a: &Assets, f: F, text: &str, x: f32, top: f32, h: f32, size: f32, col: Color) {
    let th = f.height() * size;
    label(a, f, text, x, top + (h - th) * 0.5, size, col);
}

/// Horizontally centred on `cx`.
pub fn label_c(a: &Assets, f: F, text: &str, cx: f32, top: f32, size: f32, col: Color) {
    label(a, f, text, cx - tw(a, f, text, size) * 0.5, top, size, col);
}

/// Right-aligned to `rx`.
pub fn label_r(a: &Assets, f: F, text: &str, rx: f32, top: f32, size: f32, col: Color) {
    label(a, f, text, rx - tw(a, f, text, size), top, size, col);
}

/// Height of a one-line Label.
pub fn lh(f: F, size: f32) -> f32 {
    f.label_line() * size
}

/// A label with an outline of `width` units (font_outline_color / outline_size).
pub fn label_outline(
    a: &Assets,
    f: F,
    text: &str,
    x: f32,
    top: f32,
    size: f32,
    col: Color,
    outline: Color,
    width: f32,
) {
    if outline.a > 0.0 && width > 0.0 {
        let r = (width * 0.5).max(0.5 / unit());
        for (dx, dy) in [
            (-1.0, 0.0),
            (1.0, 0.0),
            (0.0, -1.0),
            (0.0, 1.0),
            (-0.7, -0.7),
            (0.7, -0.7),
            (-0.7, 0.7),
            (0.7, 0.7),
        ] {
            label(a, f, text, x + dx * r, top + dy * r, size, outline);
        }
    }
    label(a, f, text, x, top, size, col);
}

/// The thin space Palette.track puts between letters (Courier has none: the fallback face's,
/// a fifth of an em).
pub const THIN: char = '\u{2009}';

/// Palette.track: letters joined by thin spaces ("S I G N A L").
pub fn track(text: &str) -> String {
    text.chars()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(&THIN.to_string())
}

/// Splits `text` into lines no wider than `width` units (Label autowrap, word mode).
pub fn wrap(a: &Assets, f: F, text: &str, width: f32, size: f32) -> Vec<String> {
    let mut lines = vec![];
    for para in text.split('\n') {
        let mut cur = String::new();
        for w in para.split_whitespace() {
            let test = if cur.is_empty() {
                w.to_string()
            } else {
                format!("{cur} {w}")
            };
            if tw(a, f, &test, size) > width && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                cur = w.to_string();
            } else {
                cur = test;
            }
        }
        lines.push(cur);
    }
    lines
}

/// Text cut to `width` units with an ellipsis (OVERRUN_TRIM_ELLIPSIS).
pub fn ellipsize(a: &Assets, f: F, text: &str, width: f32, size: f32) -> String {
    if tw(a, f, text, size) <= width {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        let test = format!("{out}{c}…");
        if tw(a, f, &test, size) > width {
            break;
        }
        out.push(c);
    }
    format!("{out}…")
}

/// Text cut to `width` units (clip_text).
pub fn clip(a: &Assets, f: F, text: &str, width: f32, size: f32) -> String {
    let mut out = String::new();
    for c in text.chars() {
        out.push(c);
        if tw(a, f, &out, size) > width {
            out.pop();
            break;
        }
    }
    out
}

// -- buttons --------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// Palette.style_button(primary = true): the purple slab.
    Primary,
    /// Palette.style_button(primary = false): the panel face with a hairline.
    Secondary,
    /// The lobby's selected toggle: amber fill, dark label.
    Selected,
}

/// Focus and disabled state of a control this frame.
#[derive(Clone, Copy, Default)]
pub struct BtnState {
    pub focused: bool,
    pub disabled: bool,
}

pub fn plain() -> BtnState {
    BtnState::default()
}

pub fn state(focused: bool) -> BtnState {
    BtnState {
        focused,
        disabled: false,
    }
}

pub fn disabled() -> BtnState {
    BtnState {
        focused: false,
        disabled: true,
    }
}

/// Content margins of a style_button face: (horizontal, vertical).
pub fn margins(compact: bool) -> (f32, f32) {
    if compact {
        (5.0, 3.0)
    } else {
        (10.0, 4.0)
    }
}

/// A button's minimum size for its label (text plus content margins).
pub fn button_min(a: &Assets, text: &str, size: f32, compact: bool) -> Vec2 {
    let (h, v) = margins(compact);
    vec2(
        tw(a, F::Body, text, size) + 2.0 * h,
        F::Body.height() * size + 2.0 * v,
    )
}

/// Draws a style_button face with its label centred; returns true when clicked.
pub fn button(a: &Assets, r: Rect, text: &str, size: f32, kind: Kind, st: BtnState) -> bool {
    let p = pal();
    let hover = hovered(r) && !st.disabled;
    let down = hover && is_mouse_button_down(MouseButton::Left);
    let (base_fill, base_edge) = match kind {
        Kind::Primary => (p.accent, p.accent_edge),
        Kind::Secondary => (p.panel, p.panel_edge),
        Kind::Selected => (p.amber_fill, p.amber),
    };
    let (mut face, mut edge) = (base_fill, base_edge);
    let mut ink = match kind {
        Kind::Primary | Kind::Selected => p.on_amber,
        Kind::Secondary => p.body,
    };
    if st.disabled {
        face = p.panel;
        edge = p.panel_edge;
        ink = p.disabled;
    } else if down {
        face = if kind == Kind::Primary {
            p.accent_deep
        } else {
            darkened(base_fill, 0.2)
        };
        ink = p.ink;
    } else if hover {
        face = lightened(base_fill, 0.12);
        edge = lightened(base_edge, 0.2);
        if kind == Kind::Secondary {
            ink = p.ink;
        }
    }
    if st.focused && !st.disabled && !down {
        // the focus box is drawn over the face: the base fill with a teal edge
        face = base_fill;
        edge = p.teal;
        if kind == Kind::Secondary {
            ink = p.ink;
        }
    }
    sbox(r, face, edge, p.edge_width, p.corner);
    if !text.is_empty() {
        let tx = r.x + (r.w - tw(a, F::Body, text, size)) * 0.5;
        label_vc(a, F::Body, text, tx, r.y, r.h, size, ink);
    }
    let pressed = !st.disabled && hover && is_mouse_button_pressed(MouseButton::Left);
    if pressed {
        crate::audio::sound::menu_blip(); // every BaseButton press blips (sound.gd)
    }
    pressed
}

/// Palette.style_escape_close: the outlined ESC keycap and CLOSE caption (54x16 minimum).
pub fn esc_close(a: &Assets, r: Rect) -> bool {
    let p = pal();
    let hover = hovered(r);
    let (face, edge) = if hover {
        (lightened(p.panel, 0.08), lightened(p.panel_edge, 0.2))
    } else {
        (p.panel, p.panel_edge)
    };
    sbox(r, face, edge, 1.0, p.corner);
    // content: offsets 3/2, separation 4; the keycap is an INSET box with a KEYCAP_EDGE, margins 3,1
    let cap_w = tw(a, F::Body, "ESC", 6.0) + 6.0;
    let inner_h = r.h - 4.0;
    let cap_h = (F::Body.height() * 6.0 + 2.0).min(inner_h);
    let cap = Rect::new(r.x + 3.0, r.y + 2.0 + (inner_h - cap_h) * 0.5, cap_w, cap_h);
    sbox(cap, p.inset, p.keycap_edge, 1.0, p.corner);
    label_vc(a, F::Body, "ESC", cap.x + 3.0, cap.y, cap.h, 6.0, p.ink);
    label_vc(
        a,
        F::Body,
        "CLOSE",
        cap.x + cap.w + 4.0,
        r.y + 2.0,
        inner_h,
        6.0,
        p.body,
    );
    let pressed = hover && is_mouse_button_pressed(MouseButton::Left);
    if pressed {
        crate::audio::sound::menu_blip();
    }
    pressed
}

/// The ESC CLOSE button's size.
pub fn esc_close_size(a: &Assets) -> Vec2 {
    let w = 3.0 + tw(a, F::Body, "ESC", 6.0) + 6.0 + 4.0 + tw(a, F::Body, "CLOSE", 6.0) + 3.0;
    vec2(w.max(54.0), (F::Body.height() * 6.0 + 2.0 + 4.0).max(16.0))
}

/// Palette.style_state_toggle: caption on the left, a boxed ON/OFF chip on the right.
pub fn state_toggle(
    a: &Assets,
    r: Rect,
    caption: &str,
    on: bool,
    compact: bool,
    st: BtnState,
) -> bool {
    let p = pal();
    let clicked = button(a, r, "", 7.0, Kind::Secondary, st);
    let cap_size = if compact { 6.0 } else { 7.0 };
    let inner = Rect::new(r.x + 5.0, r.y + 2.0, r.w - 10.0, r.h - 4.0);
    label_vc(
        a,
        F::Body,
        caption,
        inner.x,
        inner.y,
        inner.h,
        cap_size,
        p.body,
    );
    let text = if on { "ON" } else { "OFF" };
    let chip_w = (tw(a, F::Body, text, 6.0) + 6.0).max(20.0);
    let chip_h = (F::Body.height() * 6.0 + 2.0).min(inner.h);
    let chip = Rect::new(
        inner.x + inner.w - chip_w,
        inner.y + (inner.h - chip_h) * 0.5,
        chip_w,
        chip_h,
    );
    let col = if on { p.stable } else { p.muted };
    sbox(
        chip,
        p.inset,
        if on { p.stable } else { p.keycap_edge },
        1.0,
        p.corner,
    );
    label_vc(
        a,
        F::Body,
        text,
        chip.x + (chip.w - tw(a, F::Body, text, 6.0)) * 0.5,
        chip.y,
        chip.h,
        6.0,
        col,
    );
    clicked
}

/// Palette.style_line_edit: an inset field with a teal edge while focused.
pub fn line_edit(
    a: &Assets,
    r: Rect,
    text: &str,
    placeholder: &str,
    size: f32,
    focused: bool,
    secret: bool,
    t: f32,
) {
    let p = pal();
    sbox(
        r,
        p.inset,
        if focused { p.teal } else { p.field_edge },
        1.0,
        p.corner,
    );
    let shown = if secret {
        "*".repeat(text.chars().count())
    } else {
        text.to_string()
    };
    let x = r.x + 6.0;
    if shown.is_empty() {
        label_vc(a, F::Body, placeholder, x, r.y, r.h, size, p.muted);
    } else {
        label_vc(a, F::Body, &shown, x, r.y, r.h, size, p.ink);
    }
    if focused && (t * 1.6).fract() < 0.6 {
        let cx = x + tw(a, F::Body, &shown, size);
        let h = F::Body.height() * size;
        fill(Rect::new(cx, r.y + (r.h - h) * 0.5, 1.0, h), p.teal);
    }
}

// -- backdrop (signal_backdrop.gd) ----------------------------------------------------------

/// The archive backdrop: warm field, checker diamonds, scanlines, grid and the dual frame.
pub fn backdrop(strength: f32) {
    let p = pal();
    let v = view();
    clear_background(p.backdrop);
    // warm field: eight strips left to right, and a faint violet glow
    for i in 0..8 {
        let ratio = i as f32 / 7.0;
        let strip = Rect::new(v.x * ratio, 0.0, v.x / 8.0 + 1.0, v.y);
        fill(
            strip,
            rgba(
                0.027 + ratio * 0.016,
                0.014,
                0.025 + ratio * 0.018,
                0.42 - ratio * 0.16,
            ),
        );
    }
    let mut radius = 90;
    while radius > 8 {
        let a = 0.0024 * strength * (1.0 - radius as f32 / 100.0);
        circle(
            vec2(v.x * 0.74, v.y * 0.38),
            radius as f32,
            rgba(0.45, 0.18, 0.34, a),
        );
        radius -= 12;
    }
    // the checker archive: diamonds fading in from a quarter of the way across
    let tile = (v.x.min(v.y) * 0.13).max(28.0);
    let start_x = v.x * 0.24;
    let mut y = -tile;
    let mut row = 0;
    while y < v.y + tile {
        let mut x = start_x + if row % 2 == 1 { -tile } else { 0.0 };
        while x < v.x + tile {
            let c = vec2(x, y);
            let fade = ((x - start_x) / (v.x - start_x).max(1.0)).clamp(0.0, 1.0);
            let al = (0.012 + fade * 0.018) * strength;
            let col = rgba(0.56, 0.43, 0.49, al);
            let (top, right, bottom, left) = (
                c + vec2(0.0, -tile * 0.48),
                c + vec2(tile * 0.7, 0.0),
                c + vec2(0.0, tile * 0.48),
                c + vec2(-tile * 0.7, 0.0),
            );
            tri(top, right, bottom, col);
            tri(top, bottom, left, col);
            x += tile * 1.42;
        }
        y += tile * 0.98;
        row += 1;
    }
    // scanlines every 8 units, a faint vertical grid every 56
    let mut y = 1.0;
    while y < v.y {
        line(
            vec2(0.0, y),
            vec2(v.x, y),
            1.0,
            rgba(0.82, 0.68, 0.76, 0.022),
        );
        y += 8.0;
    }
    let mut x = 56.0;
    while x < v.x {
        line(vec2(x, 0.0), vec2(x, v.y), 1.0, rgba(1.0, 1.0, 1.0, 0.008));
        x += 56.0;
    }
    // the two perimeter rails
    if v.x >= 20.0 && v.y >= 20.0 {
        stroke(
            Rect::new(5.0, 4.0, v.x - 10.0, v.y - 8.0),
            1.0,
            rgba(0.16, 0.12, 0.16, 0.94),
        );
        stroke(
            Rect::new(7.0, 6.0, v.x - 14.0, v.y - 12.0),
            1.0,
            rgba(0.28, 0.22, 0.28, 0.86),
        );
    }
}

/// The report and loading screens' extra frames (OuterFrame at 6, InnerFrame at 9 units).
pub fn report_frames() {
    let v = view();
    let p = pal();
    panel_box(
        Rect::new(6.0, 6.0, v.x - 12.0, v.y - 12.0),
        rgba(0.0, 0.0, 0.0, 0.0),
        p.panel_edge,
    );
    panel_box(
        Rect::new(9.0, 9.0, v.x - 18.0, v.y - 18.0),
        rgba(0.0, 0.0, 0.0, 0.0),
        hex(0x46303c),
    );
}

/// Clips drawing to a rectangle (units) until `clip_end` (ScrollContainer's clip).
pub fn clip_begin(r: Rect) {
    let s = unit();
    let mut gl = unsafe { get_internal_gl() };
    gl.flush();
    gl.quad_gl.scissor(Some((
        (r.x * s) as i32,
        (r.y * s) as i32,
        (r.w * s).max(0.0) as i32,
        (r.h * s).max(0.0) as i32,
    )));
}

pub fn clip_end() {
    let mut gl = unsafe { get_internal_gl() };
    gl.flush();
    gl.quad_gl.scissor(None);
}

/// Keyboard focus over `n` controls in order: Tab / arrows move it, returns true on Enter.
pub fn nav(focus: &mut usize, n: usize, vertical: bool) -> bool {
    if n == 0 {
        return false;
    }
    let shift = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
    let (next, prev) = if vertical {
        (KeyCode::Down, KeyCode::Up)
    } else {
        (KeyCode::Right, KeyCode::Left)
    };
    if is_key_pressed(next) || (is_key_pressed(KeyCode::Tab) && !shift) {
        *focus = (*focus + 1) % n;
    }
    if is_key_pressed(prev) || (is_key_pressed(KeyCode::Tab) && shift) {
        *focus = (*focus + n - 1) % n;
    }
    *focus = (*focus).min(n - 1);
    is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter)
}

/// The scrim a modal lays over the screen below it.
pub fn scrim() {
    let v = view();
    fill(Rect::new(0.0, 0.0, v.x, v.y), pal().scrim);
}

/// The menu's one-line notice at the top (main_menu.gd _show_room_notice: AMBER, size 7).
pub fn notice(a: &Assets, msg: &str) {
    let v = view();
    let p = pal();
    label_c(a, F::Body, msg, v.x * 0.5, 8.0, 7.0, p.amber);
}

/// The modal frame shared by the menu dialogs, the account panel and the dossier
/// (main_menu.gd _modal_content): scrim, a 0c080e panel with a 12-unit pad, the tracked
/// kicker beside ESC CLOSE, then the title. Returns (panel, content top-left y after the
/// heading, close clicked).
#[allow(dead_code)]
pub struct Modal {
    pub panel: Rect,
    pub inner: Rect,
    pub y: f32,
    pub close: bool,
}

/// `content_h` is the height of what follows the heading (5-unit separations included).
pub fn modal(
    a: &Assets,
    kicker: &str,
    heading: &str,
    heading_size: f32,
    min: Vec2,
    content_w: f32,
    content_h: f32,
) -> Modal {
    let p = pal();
    scrim();
    let kick = track(kicker);
    let esc = esc_close_size(a);
    let top_h = esc.y.max(lh(F::Body, 6.0));
    let head_h = lh(F::Title, heading_size);
    let need_w = (tw(a, F::Body, &kick, 6.0) + esc.x).max(content_w) + 24.0;
    let need_h = 24.0 + top_h + 5.0 + head_h + content_h;
    let w = min.x.max(need_w);
    let h = min.y.max(need_h);
    let panel = centered(w, h);
    sbox(panel, hex(0x0c080e), p.panel_edge, 1.0, p.corner);
    let inner = Rect::new(
        panel.x + 12.0,
        panel.y + 12.0,
        panel.w - 24.0,
        panel.h - 24.0,
    );
    label_vc(a, F::Body, &kick, inner.x, inner.y, top_h, 6.0, p.amber);
    let er = Rect::new(
        inner.x + inner.w - esc.x,
        inner.y + (top_h - esc.y) * 0.5,
        esc.x,
        esc.y,
    );
    let close = esc_close(a, er);
    let hy = inner.y + top_h + 5.0;
    label(a, F::Title, heading, inner.x, hy, heading_size, p.ink);
    Modal {
        panel,
        inner,
        y: hy + head_h + 5.0,
        close,
    }
}
