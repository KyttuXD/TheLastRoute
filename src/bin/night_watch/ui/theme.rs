//! Night Watch UI palette and the 480x270 virtual canvas. Screen-space UI is authored in virtual pixels
//! and scaled by `k = min(screen_width / 480, screen_height / 270)`, centred with a letterbox offset.
#![allow(dead_code)]

use macroquad::prelude::*;

pub const VW: f32 = 480.0;
pub const VH: f32 = 270.0;

macro_rules! rgb {
    ($r:expr, $g:expr, $b:expr) => {
        Color::new($r as f32 / 255.0, $g as f32 / 255.0, $b as f32 / 255.0, 1.0)
    };
}

/// Text on the scene and panels (Godot `e8e2d0`).
pub const INK: Color = rgb!(232, 226, 208);
/// Secondary text on a panel (`9b947f`).
pub const DIM: Color = rgb!(155, 148, 127);
pub const RED: Color = rgb!(196, 48, 43);
/// The red for text on dark panels (`e0504a`).
pub const RED_TEXT: Color = rgb!(224, 80, 74);
pub const AMBER: Color = rgb!(224, 176, 74);
/// Secondary text straight on the scene (`c9c3b0`).
pub const BODY: Color = rgb!(201, 195, 176);
/// Menu panel fill: rgba(0.024, 0.02, 0.016, 0.72).
pub const PANEL: Color = Color::new(0.024, 0.02, 0.016, 0.72);
/// Menu panel border: rgb(0.16, 0.15, 0.12).
pub const PANEL_EDGE: Color = Color::new(0.16, 0.15, 0.12, 1.0);
/// Button fill: rgba(0.024, 0.02, 0.016, 0.8).
pub const BUTTON_BG: Color = Color::new(0.024, 0.02, 0.016, 0.8);
/// Hovered button fill.
pub const BUTTON_HOVER: Color = Color::new(0.13, 0.12, 0.10, 0.95);
/// Hovered, unselected button border.
pub const BUTTON_HOVER_EDGE: Color = Color::new(0.42, 0.40, 0.33, 1.0);
/// Primary button (START SHIFT): fill and border.
pub const PRIMARY_BG: Color = Color::new(0.45, 0.10, 0.09, 1.0);
pub const PRIMARY_HOVER: Color = Color::new(0.56, 0.13, 0.11, 1.0);
/// Keyboard focus wash on the title items.
pub const FOCUS_WASH: Color = Color::new(0.88, 0.69, 0.29, 0.10);
pub const AMBER_DEEP: Color = rgb!(90, 63, 16);
pub const PANEL_SOLID: Color = rgb!(15, 13, 10);
pub const INSET: Color = rgb!(6, 5, 4);
pub const EDGE: Color = rgb!(42, 38, 32);
pub const EDGE_LIGHT: Color = rgb!(74, 68, 58);
/// The dim over the backdrop office: rgba(0, 0.03, 0.04, 0.6).
pub const TINT: Color = Color::new(0.0, 0.03, 0.04, 0.6);

/// Scale and letterbox offset for a window of `w` x `h` pixels: `(k, ox, oy)`. Pure.
pub fn scale_for(w: f32, h: f32) -> (f32, f32, f32) {
    let k = (w / VW).min(h / VH).max(0.01);
    (k, (w - VW * k) * 0.5, (h - VH * k) * 0.5)
}

/// `(k, ox, oy)` for the current window.
pub fn frame() -> (f32, f32, f32) {
    scale_for(screen_width(), screen_height())
}

pub fn k() -> f32 {
    frame().0
}

pub fn ox() -> f32 {
    frame().1
}

pub fn oy() -> f32 {
    frame().2
}

/// Screen pixels to virtual coordinates for a window of `w` x `h`. Pure.
pub fn to_virtual_with(p: (f32, f32), w: f32, h: f32) -> Vec2 {
    let (k, ox, oy) = scale_for(w, h);
    vec2((p.0 - ox) / k, (p.1 - oy) / k)
}

/// Screen pixels (mouse) to virtual coordinates for the current window.
pub fn to_virtual(p: (f32, f32)) -> Vec2 {
    to_virtual_with(p, screen_width(), screen_height())
}

/// A virtual rectangle in screen pixels.
pub fn to_screen_rect(r: Rect) -> Rect {
    let (k, ox, oy) = frame();
    Rect::new(ox + r.x * k, oy + r.y * k, r.w * k, r.h * k)
}

pub fn with_alpha(c: Color, a: f32) -> Color {
    Color::new(c.r, c.g, c.b, a)
}

/// Fill a virtual rectangle.
pub fn fill_rect(r: Rect, c: Color) {
    let s = to_screen_rect(r);
    draw_rectangle(s.x, s.y, s.w, s.h, c);
}

/// Border of a virtual rectangle; `t` is the thickness in virtual pixels (at least one screen pixel).
pub fn stroke_rect(r: Rect, c: Color, t: f32) {
    let s = to_screen_rect(r);
    let th = (t * k()).round().max(1.0);
    draw_rectangle_lines(s.x, s.y, s.w, s.h, th, c);
}

/// Filled box with a border: the look of every panel and button.
pub fn boxed(r: Rect, fill: Color, edge: Color, t: f32) {
    fill_rect(r, fill);
    stroke_rect(r, edge, t);
}

/// The menu panel: dark translucent fill, thin warm border.
pub fn panel(r: Rect) {
    boxed(r, PANEL, PANEL_EDGE, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_is_the_smaller_ratio() {
        assert_eq!(scale_for(960.0, 540.0), (2.0, 0.0, 0.0));
        let (k, ox, oy) = scale_for(1000.0, 540.0);
        assert_eq!(k, 2.0);
        assert_eq!(ox, 20.0);
        assert_eq!(oy, 0.0);
        let (k, ox, oy) = scale_for(960.0, 600.0);
        assert_eq!(k, 2.0);
        assert_eq!(ox, 0.0);
        assert_eq!(oy, 30.0);
    }

    #[test]
    fn mouse_maps_to_virtual_coordinates() {
        let p = to_virtual_with((20.0 + 2.0 * 100.0, 2.0 * 50.0), 1000.0, 540.0);
        assert!((p.x - 100.0).abs() < 1e-4);
        assert!((p.y - 50.0).abs() < 1e-4);
    }

    #[test]
    fn palette_matches_godot_hex() {
        assert!((INK.r - 232.0 / 255.0).abs() < 1e-6);
        assert!((AMBER.g - 176.0 / 255.0).abs() < 1e-6);
        assert!((RED_TEXT.b - 74.0 / 255.0).abs() < 1e-6);
    }
}
