//! Pause card, end card and the death-moment box (port of `_build_pause`, `_build_end`, `_card` in `night_watch_hud.gd`
//! and the look of `game/hud/death_moment.gd`). Drawing and layout only; the actions are decided in `hud/mod.rs`.
#![allow(dead_code)]

use super::death::DeathMoment;
use super::status::*;
use crate::ui::text as ut;
use macroquad::prelude::*;

pub const PAUSE_BUTTONS: [&str; 2] = ["RESUME  ESC", "QUIT TO MENU  BACKSPACE"];
pub const BUTTON_H: f32 = 16.0;
pub const BUTTON_GAP: f32 = 6.0;

/// Button labels of the end card: NEXT only when `can_next`.
pub fn end_labels(can_next: bool) -> Vec<String> {
    let mut v = Vec::new();
    if can_next {
        v.push("NEXT NIGHT  ENTER".to_string());
    }
    v.push("RETRY  ENTER".to_string());
    v.push("MENU  ESC".to_string());
    v
}

/// A centred row of buttons of the given widths: rects left to right.
pub fn row_rects(widths: &[f32], cx: f32, y: f32, h: f32, gap: f32) -> Vec<Rect> {
    let total: f32 = widths.iter().sum::<f32>() + gap * (widths.len().max(1) as f32 - 1.0);
    let mut x = cx - total * 0.5;
    let mut out = Vec::with_capacity(widths.len());
    for w in widths {
        out.push(Rect::new(x, y, *w, h));
        x += w + gap;
    }
    out
}

fn fade(c: Color, f: f32) -> Color {
    Color::new(c.r, c.g, c.b, c.a * f)
}

/// Draw a card (dim, panel, title, body, buttons). Returns the button rects in label order (virtual px).
/// `mouse` is in virtual px; the button under it is lit.
pub fn draw_card(title: &str, body: &str, labels: &[String], mouse: Vec2) -> Vec<Rect> {
    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.0, 0.0, 0.0, 0.6));
    let widths: Vec<f32> = labels.iter().map(|l| (tw(&ut::glyph(l), 8.0) + 14.0).max(48.0)).collect();
    let row_w = widths.iter().sum::<f32>() + BUTTON_GAP * (widths.len().max(1) as f32 - 1.0);
    let w = (row_w + 20.0).max(280.0);
    let body_lines = if body.is_empty() { Vec::new() } else { ut::wrap(&ut::glyph(body), w - 20.0, 8.0) };
    let body_h = (body_lines.len() as f32 * 11.0).max(10.0);
    let h = 10.0 + 26.0 + 8.0 + body_h + 8.0 + BUTTON_H + 10.0;
    let panel = Rect::new(W * 0.5 - w * 0.5, H * 0.5 - 60.0, w, h);
    fill(panel, PANEL);
    stroke(panel, AMBER, 1.0);
    let mut y = panel.y + 10.0;
    vtext_center(&ut::glyph(title), W * 0.5, y, 22.0, AMBER);
    y += 26.0 + 8.0;
    for l in &body_lines {
        vtext_center(l, W * 0.5, y, 8.0, INK);
        y += 11.0;
    }
    y = panel.y + 10.0 + 26.0 + 8.0 + body_h + 8.0;
    let rects = row_rects(&widths, W * 0.5, y, BUTTON_H, BUTTON_GAP);
    for (r, l) in rects.iter().zip(labels) {
        button_box(*r, &ut::glyph(l), 8.0, r.contains(mouse), false);
    }
    rects
}

/// The death-moment box: a dark shade, then a subtitle box low on the screen with the spaced name and the typed line.
pub fn draw_death(m: &DeathMoment) {
    let f = m.fade();
    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.02, 0.01, 0.03, 0.55 * f));
    let text = m.shown_line();
    let full = m.words.join(" ");
    let full_lines = ut::wrap(&ut::glyph(&full), 322.0, 9.0);
    let lines = ut::wrap(&ut::glyph(&text), 322.0, 9.0);
    let content_h = 8.0 + 4.0 + full_lines.len().max(1) as f32 * 11.0 + 4.0 + 7.0;
    let h = 64.0f32.max(content_h + 18.0);
    let r = Rect::new(W * 0.5 - 170.0, H - 22.0 - h, 340.0, h);
    fill(r, fade(Color::new(0.04, 0.02, 0.06, 0.94), f));
    stroke(r, fade(Color::new(0.353, 0.329, 0.275, 1.0), f), 1.0);
    let x = r.x + 9.0;
    let mut y = r.y + 9.0;
    vtext(&m.name, x, y, 6.0, fade(AMBER, f));
    y += 8.0 + 4.0;
    for l in &lines {
        vtext(l, x, y, 9.0, fade(INK, f));
        y += 11.0;
    }
    vtext_right("CLICK OR PRESS A KEY", r.x + r.w - 9.0, r.y + r.h - 9.0 - 5.0, 5.0, fade(DIM, f));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_centred_in_order_without_overlap() {
        let r = row_rects(&[100.0, 60.0, 40.0], 240.0, 100.0, 16.0, 6.0);
        assert_eq!(r.len(), 3);
        let left = r[0].x;
        let right = r[2].x + r[2].w;
        assert!(((left + right) * 0.5 - 240.0).abs() < 0.001);
        assert!(r[0].x + r[0].w + 6.0 <= r[1].x + 0.001 && r[1].x + r[1].w + 6.0 <= r[2].x + 0.001);
        assert!(r.iter().all(|q| q.y == 100.0 && q.h == 16.0));
    }

    #[test]
    fn a_single_button_row_is_centred() {
        let r = row_rects(&[80.0], 240.0, 0.0, 16.0, 6.0);
        assert!((r[0].x - 200.0).abs() < 0.001);
    }

    #[test]
    fn end_labels_show_next_only_when_allowed() {
        assert_eq!(end_labels(true), vec!["NEXT NIGHT  ENTER", "RETRY  ENTER", "MENU  ESC"]);
        assert_eq!(end_labels(false), vec!["RETRY  ENTER", "MENU  ESC"]);
    }
}
