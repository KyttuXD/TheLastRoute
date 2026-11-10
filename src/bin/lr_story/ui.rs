//! The shared look of lr_story's screens: the palette, the two fonts, and the drawing pieces every screen uses
//! (backdrop, panels, buttons, bars, key hints, text with a shadow). The menu (menu.rs) and the in-game HUD use these, so
//! the game has one look.
//!
//! Fonts: Anton (titles) and Liberation Mono Bold (body), copied from last_route's assets into `assets/fonts/` with their
//! OFL licences. They are read through `assets_vfs`, so the browser build has them too. Without them (a missing file),
//! macroquad's built-in font is used and nothing else changes.

use crate::assets_vfs;
use crate::model::Era;
use macroquad::prelude::*;
use std::cell::RefCell;
use std::path::Path;

// ---- palette ----
pub const BG_TOP: Color = Color::new(0.035, 0.04, 0.06, 1.0);
pub const BG_BOTTOM: Color = Color::new(0.01, 0.012, 0.02, 1.0);
pub const PANEL: Color = Color::new(0.07, 0.075, 0.10, 0.92);
pub const PANEL_LIT: Color = Color::new(0.15, 0.13, 0.08, 0.95);
pub const BORDER: Color = Color::new(0.22, 0.24, 0.30, 1.0);
pub const ACCENT: Color = Color::new(1.0, 0.82, 0.38, 1.0);
pub const TEXT: Color = Color::new(0.93, 0.93, 0.95, 1.0);
pub const MUTED: Color = Color::new(0.66, 0.67, 0.72, 1.0);
pub const DIM: Color = Color::new(0.44, 0.45, 0.50, 1.0);
pub const GOOD: Color = Color::new(0.45, 0.90, 0.55, 1.0);
pub const BAD: Color = Color::new(1.0, 0.45, 0.42, 1.0);
pub const SHADOW: Color = Color::new(0.0, 0.0, 0.0, 0.55);

/// The colour that marks a story's era (menu cards, the HUD's map banner).
pub fn era_color(era: Era) -> Color {
    match era {
        Era::Sample => Color::new(0.55, 0.75, 1.0, 1.0),
        Era::Past => Color::new(0.85, 0.62, 0.38, 1.0),
        Era::LastRoute => Color::new(0.95, 0.35, 0.32, 1.0),
        Era::Simulora => Color::new(0.55, 0.90, 0.75, 1.0),
        Era::Future => Color::new(0.72, 0.55, 1.0, 1.0),
    }
}

/// `c` with its alpha multiplied by `a`.
pub fn fade(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, c.a * a.clamp(0.0, 1.0))
}

// ---- fonts ----
struct Fonts {
    title: Option<Font>,
    body: Option<Font>,
}

thread_local! {
    static FONTS: RefCell<Option<Fonts>> = const { RefCell::new(None) };
}

fn font_file(name: &str) -> Option<Font> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/lr_story/assets/fonts").join(name);
    let bytes = assets_vfs::read(&path).ok()?;
    let mut font = load_ttf_font_from_bytes(&bytes).ok()?;
    font.set_filter(FilterMode::Linear);
    Some(font)
}

/// Loads the two fonts once. Call after the game files are in (the browser reads them from the bundle).
pub fn init() {
    FONTS.with(|f| {
        if f.borrow().is_none() {
            *f.borrow_mut() = Some(Fonts { title: font_file("Anton-Regular.ttf"), body: font_file("LiberationMono-Bold.ttf") });
        }
    });
}

fn with_font<R>(title: bool, run: impl FnOnce(Option<&Font>) -> R) -> R {
    FONTS.with(|f| {
        let fonts = f.borrow();
        let font = fonts.as_ref().and_then(|x| if title { x.title.as_ref() } else { x.body.as_ref() });
        run(font)
    })
}

fn put(title: bool, s: &str, x: f32, y: f32, size: f32, color: Color) {
    with_font(title, |font| {
        draw_text_ex(s, x, y, TextParams { font, font_size: size.round().max(1.0) as u16, color, ..Default::default() });
    });
}

fn width(title: bool, s: &str, size: f32) -> f32 {
    with_font(title, |font| measure_text(s, font, size.round().max(1.0) as u16, 1.0).width)
}

/// Body text, baseline at `y`.
pub fn text(s: &str, x: f32, y: f32, size: f32, color: Color) {
    put(false, s, x, y, size, color);
}

/// Body text with a soft shadow, for text over the 3D view.
pub fn text_shadow(s: &str, x: f32, y: f32, size: f32, color: Color) {
    put(false, s, x + 2.0, y + 2.0, size, fade(SHADOW, color.a));
    put(false, s, x, y, size, color);
}

/// Title text (Anton), baseline at `y`.
pub fn title(s: &str, x: f32, y: f32, size: f32, color: Color) {
    put(true, s, x, y, size, color);
}

/// The width of body text.
pub fn measure(s: &str, size: f32) -> f32 {
    width(false, s, size)
}

/// The width of title text.
pub fn measure_title(s: &str, size: f32) -> f32 {
    width(true, s, size)
}

/// Body text centred on `cx`.
pub fn text_centered(s: &str, cx: f32, y: f32, size: f32, color: Color) {
    text(s, cx - measure(s, size) / 2.0, y, size, color);
}

/// Title text centred on `cx`, with a shadow.
pub fn title_centered(s: &str, cx: f32, y: f32, size: f32, color: Color) {
    let x = cx - measure_title(s, size) / 2.0;
    put(true, s, x + 3.0, y + 3.0, size, SHADOW);
    put(true, s, x, y, size, color);
}

/// Splits `s` into lines no wider than `max_w` at `size`, on spaces.
pub fn wrap(s: &str, max_w: f32, size: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        let next = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if !line.is_empty() && measure(&next, size) > max_w {
            lines.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

// ---- shapes ----

/// A vertical gradient from `top` to `bottom` over the rectangle (drawn as bands).
pub fn gradient(x: f32, y: f32, w: f32, h: f32, top: Color, bottom: Color) {
    const BANDS: usize = 24;
    let band = h / BANDS as f32;
    for i in 0..BANDS {
        let t = i as f32 / (BANDS - 1) as f32;
        let c = Color::new(
            top.r + (bottom.r - top.r) * t,
            top.g + (bottom.g - top.g) * t,
            top.b + (bottom.b - top.b) * t,
            top.a + (bottom.a - top.a) * t,
        );
        draw_rectangle(x, y + band * i as f32, w, band + 1.0, c);
    }
}

/// The menu backdrop: a dark gradient, a faint floor grid in perspective, slow drifting motes and a vignette. `t` is the
/// time in seconds; `tint` colours the glow (an era colour, or the accent).
pub fn backdrop(t: f32, tint: Color) {
    let (sw, sh) = (screen_width(), screen_height());
    gradient(0.0, 0.0, sw, sh, BG_TOP, BG_BOTTOM);
    // a soft glow behind the title
    for i in 0..6 {
        let r = sh * (0.55 - i as f32 * 0.07);
        draw_circle(sw * 0.5, sh * 0.18, r, fade(tint, 0.018));
    }
    // a floor grid running to a horizon at 58% of the height
    let horizon = sh * 0.58;
    let grid = fade(tint, 0.10);
    for i in 0..14 {
        let depth = ((i as f32 + (t * 0.25).fract()) / 14.0).powf(2.2);
        let y = horizon + (sh - horizon) * depth;
        draw_line(0.0, y, sw, y, 1.0, fade(grid, depth));
    }
    for i in -12..=12 {
        let x = sw * 0.5 + i as f32 * sw * 0.09;
        draw_line(sw * 0.5 + i as f32 * 12.0, horizon, x, sh, 1.0, fade(grid, 0.6));
    }
    // drifting motes, placed by a fixed hash so each frame agrees with the last
    for i in 0..48u32 {
        let h = (i.wrapping_mul(2654435761) >> 8) as f32 / 16_777_216.0;
        let k = (i.wrapping_mul(40503).wrapping_add(17) % 997) as f32 / 997.0;
        let x = (h * sw + t * (6.0 + k * 10.0)) % sw;
        let y = (k * sh * 0.9 - t * (3.0 + h * 6.0)).rem_euclid(sh);
        draw_circle(x, y, 1.0 + k * 1.5, fade(tint, 0.10 + 0.25 * k));
    }
    vignette(0.55);
}

/// Darkens the screen edges.
pub fn vignette(strength: f32) {
    let (sw, sh) = (screen_width(), screen_height());
    let edge = (sw.min(sh) * 0.18).max(40.0);
    for i in 0..8 {
        let a = strength * (1.0 - i as f32 / 8.0) * 0.12;
        let d = edge * i as f32 / 8.0;
        draw_rectangle(0.0, d, sw, edge / 8.0, Color::new(0.0, 0.0, 0.0, a));
        draw_rectangle(0.0, sh - d - edge / 8.0, sw, edge / 8.0, Color::new(0.0, 0.0, 0.0, a));
        draw_rectangle(d, 0.0, edge / 8.0, sh, Color::new(0.0, 0.0, 0.0, a));
        draw_rectangle(sw - d - edge / 8.0, 0.0, edge / 8.0, sh, Color::new(0.0, 0.0, 0.0, a));
    }
}

/// A panel: drop shadow, fill, a one-pixel border and a lighter top edge. `accent`, when given, is a strip on the left.
pub fn panel(r: Rect, fill: Color, accent: Option<Color>) {
    draw_rectangle(r.x + 4.0, r.y + 5.0, r.w, r.h, fade(SHADOW, 0.6));
    draw_rectangle(r.x, r.y, r.w, r.h, fill);
    draw_rectangle_lines(r.x, r.y, r.w, r.h, 1.0, BORDER);
    draw_line(r.x + 1.0, r.y + 1.0, r.x + r.w - 1.0, r.y + 1.0, 1.0, Color::new(1.0, 1.0, 1.0, 0.06));
    if let Some(c) = accent {
        draw_rectangle(r.x, r.y, 4.0, r.h, c);
    }
}

/// A menu row or button. `lit` is the selected or hovered state, `glow` (0..1) eases it in.
pub fn button(r: Rect, label: &str, sub: &str, lit: bool, enabled: bool, glow: f32, accent: Color) {
    let fill = if lit && enabled { PANEL_LIT } else { PANEL };
    let lift = if enabled { glow * 3.0 } else { 0.0 };
    let r = Rect::new(r.x + lift, r.y, r.w, r.h);
    panel(r, fill, Some(if enabled { fade(accent, 0.35 + 0.65 * glow) } else { DIM }));
    if lit && enabled {
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, fade(accent, 0.5 + 0.5 * glow));
    }
    let label_color = if !enabled {
        DIM
    } else if lit {
        accent
    } else {
        TEXT
    };
    let has_sub = !sub.is_empty();
    let label_y = if has_sub { r.y + r.h * 0.46 } else { r.y + r.h * 0.5 + 9.0 };
    text(label, r.x + 22.0, label_y, 24.0, label_color);
    if has_sub {
        text(sub, r.x + 22.0, r.y + r.h * 0.46 + 22.0, 15.0, if enabled { MUTED } else { DIM });
    }
    if lit && enabled {
        text(">", r.x + r.w - 30.0, label_y, 24.0, accent);
    }
}

/// A horizontal bar filled to `frac` (0..1).
pub fn bar(r: Rect, frac: f32, color: Color) {
    draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.0, 0.0, 0.0, 0.55));
    draw_rectangle(r.x, r.y, r.w * frac.clamp(0.0, 1.0), r.h, color);
    draw_rectangle_lines(r.x, r.y, r.w, r.h, 1.0, fade(BORDER, 0.8));
}

/// A key cap with a label after it ("[E] Talk"). Returns the x after the label.
#[allow(dead_code)]
pub fn key_hint(x: f32, y: f32, key: &str, label: &str) -> f32 {
    let size = 15.0;
    let kw = measure(key, size).max(10.0) + 12.0;
    draw_rectangle(x, y - 16.0, kw, 22.0, Color::new(0.12, 0.13, 0.17, 0.95));
    draw_rectangle_lines(x, y - 16.0, kw, 22.0, 1.0, MUTED);
    text(key, x + 6.0, y, size, TEXT);
    text_shadow(label, x + kw + 8.0, y, size, MUTED);
    x + kw + 8.0 + measure(label, size) + 18.0
}

/// A row of key hints along the bottom of the screen, centred.
#[allow(dead_code)]
pub fn hint_bar(hints: &[(&str, &str)]) {
    let size = 15.0;
    let total: f32 = hints.iter().map(|(k, l)| measure(k, size).max(10.0) + 12.0 + 8.0 + measure(l, size) + 18.0).sum();
    let mut x = (screen_width() - total) / 2.0;
    let y = screen_height() - 18.0;
    for (k, l) in hints {
        x = key_hint(x, y, k, l);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_era_has_a_distinct_colour() {
        let eras = [Era::Sample, Era::Past, Era::LastRoute, Era::Simulora, Era::Future];
        for (i, a) in eras.iter().enumerate() {
            for b in &eras[i + 1..] {
                assert_ne!(era_color(*a), era_color(*b));
            }
        }
    }

    #[test]
    fn fade_scales_alpha_and_clamps() {
        assert_eq!(fade(ACCENT, 0.5).a, 0.5);
        assert_eq!(fade(ACCENT, 3.0).a, 1.0);
        assert_eq!(fade(ACCENT, -1.0).a, 0.0);
    }

    #[test]
    fn fade_multiplies_an_existing_alpha_and_keeps_the_colour() {
        let c = fade(SHADOW, 0.5);
        assert!((c.a - SHADOW.a * 0.5).abs() < 1e-6);
        assert_eq!((c.r, c.g, c.b), (SHADOW.r, SHADOW.g, SHADOW.b));
        assert_eq!(fade(TEXT, 1.0), TEXT);
    }

    #[test]
    fn theme_and_era_colours_are_opaque_and_in_range() {
        let eras = [Era::Sample, Era::Past, Era::LastRoute, Era::Simulora, Era::Future];
        let mut all: Vec<Color> = eras.iter().map(|e| era_color(*e)).collect();
        all.extend([TEXT, MUTED, DIM, GOOD, BAD]);
        for c in all {
            for v in [c.r, c.g, c.b, c.a] {
                assert!((0.0..=1.0).contains(&v), "{c:?}");
            }
            assert_eq!(c.a, 1.0, "{c:?}");
        }
    }

    #[test]
    fn text_greys_step_down_in_brightness() {
        let lum = |c: Color| c.r + c.g + c.b;
        assert!(lum(TEXT) > lum(MUTED) && lum(MUTED) > lum(DIM));
    }
}
