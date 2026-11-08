//! Immediate-mode widgets on the 480x270 canvas. A `Ui` is made once per frame from the input snapshot; every
//! widget draws itself and answers in the same call. Widget ids are call order, so keep the layout order stable.
//! Slider drags and the previous frame's hover live in thread-locals (the app is single threaded, wasm included).
#![allow(dead_code)]

use super::text::{fit, line_height, measure, text, text_center};
use super::theme::RED; // explicit: macroquad's prelude has its own RED
use super::theme::*;
use crate::core::InputFrame;
use macroquad::prelude::*;
use std::cell::Cell;

thread_local! {
    static ACTIVE: Cell<Option<u32>> = Cell::new(None);
    static PREV_HOVER: Cell<Option<u32>> = Cell::new(None);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonStyle {
    Normal,
    Primary,
    Selected,
    Tab,
    Row,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct UiResult {
    pub clicked: bool,
    pub hovered: bool,
    /// Clicked while disabled (play the deny blip).
    pub denied: bool,
}

pub struct Ui {
    /// Mouse in virtual coordinates.
    pub mouse: Vec2,
    /// Left button went down this frame.
    pub click: bool,
    /// Left button is held.
    pub down: bool,
    /// The widget under the mouse this frame (the last one drawn wins).
    hot: Option<u32>,
    prev_hover: Option<u32>,
    active: Option<u32>,
    next_id: u32,
}

impl Ui {
    pub fn begin(input: &InputFrame) -> Ui {
        let down = input.click_down;
        let active = if down { ACTIVE.with(|a| a.get()) } else { None };
        Ui {
            mouse: to_virtual(input.mouse),
            click: input.click,
            down,
            hot: None,
            prev_hover: PREV_HOVER.with(|p| p.get()),
            active,
            next_id: 0,
        }
    }

    /// Same as `begin` with an explicit mouse (virtual coordinates). For tests and tools.
    pub fn at(mouse: Vec2, click: bool, down: bool) -> Ui {
        Ui { mouse, click, down, hot: None, prev_hover: None, active: None, next_id: 0 }
    }

    fn next(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    /// True when the mouse moved onto a different widget than last frame (the "move" blip).
    pub fn hover_changed(&self) -> bool {
        self.hot.is_some() && self.hot != self.prev_hover
    }

    pub fn is_hovering(&self) -> bool {
        self.hot.is_some()
    }

    /// An invisible hit region: hover and click only, nothing drawn.
    pub fn region(&mut self, r: Rect, enabled: bool) -> UiResult {
        let id = self.next();
        let hovered = r.contains(self.mouse);
        if hovered {
            self.hot = Some(id);
        }
        UiResult { clicked: hovered && self.click && enabled, hovered, denied: hovered && self.click && !enabled }
    }

    pub fn button(&mut self, r: Rect, label: &str, size: f32, style: ButtonStyle, enabled: bool) -> UiResult {
        let res = self.region(r, enabled);
        let hot = res.hovered && enabled;
        let (bg, edge, t, fg) = button_colors(style, hot, enabled);
        boxed(r, bg, edge, t);
        let ty = r.y + (r.h - size) * 0.5;
        let label = fit(label, r.w - 6.0, size);
        if style == ButtonStyle::Row {
            text(&label, r.x + 4.0, ty, size, fg);
        } else {
            text_center(&label, r.x + r.w * 0.5, ty, size, fg);
        }
        res
    }

    /// A tile with several short lines (shift tiles). First line is the title; the rest are dim.
    pub fn tile(&mut self, r: Rect, lines: &[String], size: f32, selected: bool, enabled: bool) -> UiResult {
        let res = self.region(r, enabled);
        let hot = res.hovered && enabled;
        let style = if selected { ButtonStyle::Selected } else { ButtonStyle::Normal };
        let (bg, edge, t, fg) = button_colors(style, hot, enabled);
        boxed(r, bg, edge, t);
        let mut y = r.y + 3.0;
        for (i, l) in lines.iter().enumerate() {
            let c = if i == 0 { fg } else if enabled { BODY } else { DIM };
            text(&fit(l, r.w - 8.0, size), r.x + 4.0, y, size, c);
            y += line_height(size);
        }
        res
    }

    pub fn slider(&mut self, r: Rect, value: f32, lo: f32, hi: f32) -> f32 {
        self.slider_step(r, value, lo, hi, 0.0)
    }

    /// Horizontal slider; `step` > 0 snaps the value. Returns the new value.
    pub fn slider_step(&mut self, r: Rect, value: f32, lo: f32, hi: f32, step: f32) -> f32 {
        let id = self.next();
        let hit = Rect::new(r.x - 3.0, r.y - 2.0, r.w + 6.0, r.h + 4.0);
        let hovered = hit.contains(self.mouse);
        if hovered {
            self.hot = Some(id);
        }
        if hovered && self.click {
            self.active = Some(id);
        }
        let mut v = value;
        let dragging = self.active == Some(id) && self.down;
        if dragging {
            v = slider_value_at(self.mouse.x, r.x, r.w, lo, hi, step);
        }
        let t = if hi > lo { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
        let track = Rect::new(r.x, r.y + r.h * 0.5 - 2.0, r.w, 4.0);
        fill_rect(track, EDGE);
        fill_rect(Rect::new(track.x, track.y, track.w * t, track.h), AMBER);
        let knob = Rect::new(r.x + r.w * t - 4.0, r.y + r.h * 0.5 - 4.0, 8.0, 8.0);
        boxed(knob, if hovered || dragging { INK } else { AMBER }, AMBER, 1.0);
        v
    }

    /// Check box; the box is 9x9 at the left of `r`, the whole of `r` is clickable. Returns the new value.
    pub fn checkbox(&mut self, r: Rect, value: bool) -> bool {
        let res = self.region(r, true);
        let b = Rect::new(r.x, r.y + (r.h - 9.0) * 0.5, 9.0, 9.0);
        if value {
            boxed(b, AMBER, AMBER, 1.0);
        } else {
            boxed(b, INSET, if res.hovered { INK } else { AMBER }, 1.0);
        }
        if res.clicked {
            !value
        } else {
            value
        }
    }

    /// Single-line text field. Typing is read only while `focused` (characters and Backspace).
    pub fn text_field(&mut self, r: Rect, value: &mut String, secret: bool, focused: bool) {
        let _ = self.region(r, true);
        boxed(r, INSET, if focused { AMBER } else { EDGE_LIGHT }, if focused { 2.0 } else { 1.0 });
        if focused {
            while let Some(ch) = get_char_pressed() {
                apply_text_char(value, ch, 64);
            }
            if is_key_pressed(KeyCode::Backspace) {
                value.pop();
            }
        }
        let shown: String = if secret { "*".repeat(value.chars().count()) } else { value.clone() };
        let size = 6.0;
        let ty = r.y + (r.h - size) * 0.5;
        text(&shown, r.x + 4.0, ty, size, INK);
        if focused && ((get_time() * 2.0) as i64) % 2 == 0 {
            let cx = r.x + 4.0 + measure(&shown, size) + 1.0;
            fill_rect(Rect::new(cx, r.y + 2.0, 1.0, r.h - 4.0), AMBER);
        }
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        let active = self.active;
        let hot = self.hot;
        ACTIVE.with(|a| a.set(active));
        PREV_HOVER.with(|p| p.set(hot));
    }
}

/// Fill, border, border width and text colour of a button.
pub fn button_colors(style: ButtonStyle, hot: bool, enabled: bool) -> (Color, Color, f32, Color) {
    if !enabled {
        return (BUTTON_BG, PANEL_EDGE, 1.0, DIM);
    }
    match style {
        ButtonStyle::Primary => (if hot { PRIMARY_HOVER } else { PRIMARY_BG }, RED, 1.0, INK),
        ButtonStyle::Selected => (if hot { BUTTON_HOVER } else { BUTTON_BG }, AMBER, 2.0, AMBER),
        ButtonStyle::Normal | ButtonStyle::Tab | ButtonStyle::Row => {
            if hot {
                (BUTTON_HOVER, BUTTON_HOVER_EDGE, 1.0, INK)
            } else {
                (BUTTON_BG, PANEL_EDGE, 1.0, INK)
            }
        }
    }
}

/// Slider value for a mouse x over a track at `x` of width `w`. Pure.
pub fn slider_value_at(mx: f32, x: f32, w: f32, lo: f32, hi: f32, step: f32) -> f32 {
    let t = if w > 0.0 { ((mx - x) / w).clamp(0.0, 1.0) } else { 0.0 };
    let mut v = lo + t * (hi - lo);
    if step > 0.0 {
        v = lo + ((v - lo) / step).round() * step;
    }
    v.clamp(lo.min(hi), lo.max(hi))
}

/// Append a typed character unless it is a control character or the field is full. Pure.
pub fn apply_text_char(value: &mut String, ch: char, max_chars: usize) {
    if !ch.is_control() && value.chars().count() < max_chars {
        value.push(ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_maps_the_mouse_and_snaps() {
        assert_eq!(slider_value_at(0.0, 10.0, 100.0, 0.0, 20.0, 1.0), 0.0);
        assert_eq!(slider_value_at(110.0, 10.0, 100.0, 0.0, 20.0, 1.0), 20.0);
        assert_eq!(slider_value_at(60.0, 10.0, 100.0, 0.0, 20.0, 1.0), 10.0);
        assert_eq!(slider_value_at(500.0, 10.0, 100.0, 0.0, 20.0, 1.0), 20.0);
        let v = slider_value_at(37.0, 10.0, 100.0, 0.0, 20.0, 0.0);
        assert!((v - 5.4).abs() < 1e-4);
    }

    #[test]
    fn region_reports_hover_click_and_denial() {
        let mut ui = Ui::at(vec2(10.0, 10.0), true, true);
        let r = ui.region(Rect::new(0.0, 0.0, 20.0, 20.0), true);
        assert!(r.hovered && r.clicked && !r.denied);
        let d = ui.region(Rect::new(0.0, 0.0, 20.0, 20.0), false);
        assert!(d.hovered && !d.clicked && d.denied);
        let off = ui.region(Rect::new(100.0, 100.0, 5.0, 5.0), true);
        assert!(!off.hovered && !off.clicked && !off.denied);
        assert!(ui.hover_changed());
    }

    #[test]
    fn text_input_skips_control_characters_and_caps_length() {
        let mut s = String::new();
        apply_text_char(&mut s, 'A', 3);
        apply_text_char(&mut s, '\n', 3);
        apply_text_char(&mut s, 'b', 3);
        apply_text_char(&mut s, 'c', 3);
        apply_text_char(&mut s, 'd', 3);
        assert_eq!(s, "Abc");
    }

    #[test]
    fn disabled_buttons_are_dim() {
        let (_, _, _, fg) = button_colors(ButtonStyle::Normal, true, false);
        assert_eq!(fg, DIM);
        let (_, edge, w, _) = button_colors(ButtonStyle::Selected, false, true);
        assert_eq!(edge, AMBER);
        assert_eq!(w, 2.0);
    }
}
