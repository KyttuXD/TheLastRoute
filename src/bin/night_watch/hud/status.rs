//! Screen furniture in the 480x270 virtual canvas: shared draw helpers, then the status block, clock, pause and CAMERAS
//! buttons, crosshair, prompt, note, bars, free panels and Zellia's call box. Ports `night_watch_hud.gd` (status, call, note).
#![allow(dead_code)]

use crate::core::{hour_label, Bindings, HudState, InputFrame};
use crate::ui::text as ut;
use crate::ui::theme::{k, ox, oy};
use macroquad::prelude::*;

pub const AMBER: Color = Color::new(0.96, 0.72, 0.28, 1.0);
pub const INK: Color = Color::new(0.93, 0.89, 0.80, 1.0);
pub const DIM: Color = Color::new(0.54, 0.52, 0.45, 1.0);
pub const PANEL: Color = Color::new(0.05, 0.04, 0.07, 0.88);
pub const REC_RED: Color = Color::new(1.0, 0.3, 0.3, 1.0);
pub const FEED_INK: Color = Color::new(0.8, 1.0, 0.85, 1.0);
/// The always-on bottom key legend. Off on purpose, as in Godot.
pub const SHOW_KEY_HINT: bool = false;
/// Virtual canvas.
pub const W: f32 = 480.0;
pub const H: f32 = 270.0;

// ---- virtual-canvas drawing helpers (rendering only) ----

/// Virtual rect to real pixels.
pub fn v2r(r: Rect) -> Rect {
    Rect::new(ox() + r.x * k(), oy() + r.y * k(), r.w * k(), r.h * k())
}

pub fn fill(r: Rect, c: Color) {
    let q = v2r(r);
    draw_rectangle(q.x, q.y, q.w, q.h, c);
}

pub fn stroke(r: Rect, c: Color, w: f32) {
    let q = v2r(r);
    draw_rectangle_lines(q.x, q.y, q.w, q.h, (w * k()).max(1.0), c);
}

pub fn line(a: Vec2, b: Vec2, c: Color, w: f32) {
    draw_line(ox() + a.x * k(), oy() + a.y * k(), ox() + b.x * k(), oy() + b.y * k(), (w * k()).max(1.0), c);
}

/// Dashed line: `dash` long dashes with equal gaps (CanvasItem.draw_dashed_line).
pub fn dashed(a: Vec2, b: Vec2, c: Color, w: f32, dash: f32) {
    let d = b - a;
    let len = d.length();
    if len < 0.01 || dash <= 0.0 {
        return;
    }
    let dir = d / len;
    let mut t = 0.0;
    while t < len {
        let e = (t + dash).min(len);
        line(a + dir * t, a + dir * e, c, w);
        t += dash * 2.0;
    }
}

pub fn dot(p: Vec2, r: f32, c: Color) {
    draw_circle(ox() + p.x * k(), oy() + p.y * k(), r * k(), c);
}

/// Text at virtual (x, y = top of the line).
pub fn vtext(s: &str, x: f32, y: f32, size: f32, c: Color) {
    ut::text(s, x, y, size, c);
}

pub fn vtext_center(s: &str, cx: f32, y: f32, size: f32, c: Color) {
    ut::text_center(s, cx, y, size, c);
}

pub fn vtext_right(s: &str, rx: f32, y: f32, size: f32, c: Color) {
    ut::text_right(s, rx, y, size, c);
}

/// Text width in virtual px.
pub fn tw(s: &str, size: f32) -> f32 {
    // Unit tests have no window, so macroquad cannot measure: use a fixed advance.
    #[cfg(test)]
    {
        return s.chars().count() as f32 * size * 0.6;
    }
    #[cfg(not(test))]
    tw_measured(s, size)
}

#[cfg(not(test))]
fn tw_measured(s: &str, size: f32) -> f32 {
    let kk = k().max(0.01);
    measure_text(s, None, (size * kk).round().max(1.0) as u16, 1.0).width / kk
}

pub fn c3(c: [f32; 3]) -> Color {
    Color::new(c[0], c[1], c[2], 1.0)
}

/// Mouse in virtual coordinates from the frame snapshot.
pub fn mouse_v(input: &InputFrame) -> Vec2 {
    vec2((input.mouse.0 - ox()) / k(), (input.mouse.1 - oy()) / k())
}

/// Mouse in virtual coordinates read live (hover tint only).
pub fn mouse_live() -> Vec2 {
    let (mx, my) = mouse_position();
    vec2((mx - ox()) / k(), (my - oy()) / k())
}

/// A flat button: dark fill, grey-brown border, amber when hot.
pub fn button_box(r: Rect, label: &str, size: f32, hover: bool, accent: bool) {
    let bg = if hover { Color::new(0.16, 0.13, 0.10, 0.95) } else { Color::new(0.07, 0.06, 0.09, 0.92) };
    fill(r, bg);
    stroke(r, if accent || hover { AMBER } else { Color::new(0.35, 0.33, 0.27, 1.0) }, 1.0);
    vtext_center(label, r.x + r.w * 0.5, r.y + (r.h - size) * 0.5 - 0.5, size, if hover { AMBER } else { INK });
}

// ---- fixed rects ----

pub fn pause_rect() -> Rect {
    Rect::new(436.0, 22.0, 36.0, 12.0)
}
pub fn cameras_rect() -> Rect {
    Rect::new(195.0, 253.0, 90.0, 12.0)
}
pub fn close_rect() -> Rect {
    Rect::new(356.0, 222.0, 112.0, 16.0)
}

// ---- Zellia's call box ----

/// With the monitor up the call box sits in the strip under the feed (x 8..346, from y 221), never over the picture.
pub const MONITOR_CALL: Rect = Rect { x: 8.0, y: 221.0, w: 338.0, h: 32.0 };

pub struct CallLayout {
    pub rect: Rect,
    pub name_pos: Vec2,
    pub text_x: f32,
    pub text_y: f32,
    pub lines: Vec<String>,
    pub mute: Rect,
    pub mute_label: String,
    pub step: f32,
}

/// What the call box shows now: the script's typed text, or the plain `call_text`.
pub fn call_text_of(st: &HudState) -> String {
    if !st.call_text.is_empty() {
        st.call_text.clone()
    } else if st.call.open {
        st.call.text.clone()
    } else {
        String::new()
    }
}

/// Centred, 20%..80% of the width (20%..68% on the camera screen), bottom at 90% of the height. None when nothing shows
/// (no text, muted, or subtitles off).
pub fn call_layout(st: &HudState, bindings: &Bindings, subtitles: bool) -> Option<CallLayout> {
    let text = ut::glyph(&call_text_of(st));
    if text.is_empty() || st.zellia_muted || !subtitles {
        return None;
    }
    let x0 = if st.monitor_up { MONITOR_CALL.x } else { W * 0.2 };
    let x1 = if st.monitor_up { MONITOR_CALL.x + MONITOR_CALL.w } else { W * 0.8 };
    let w = x1 - x0;
    let mute_label = format!("Mute  {}", bindings.key_text("office_radio"));
    let mute_w = tw(&mute_label, 5.0) + 8.0;
    // the name and Mute share a header row; the text wraps across the full box width under them
    let text_w = (w - 12.0).max(40.0);
    let lines = ut::wrap(&text, text_w, 5.0);
    let step = 7.0;
    // while a call is typing, words arrive one by one: keep room for 3 rows so the box never jumps or clips
    let reserve = if st.monitor_up { 2 } else if st.call.open { 3 } else { 1 };
    let rows = lines.len().max(reserve);
    let head = 14.0;
    let h = head + rows as f32 * step + 3.0;
    let y = if st.monitor_up { MONITOR_CALL.y } else { H * 0.9 - h };
    Some(CallLayout {
        rect: Rect::new(x0, y, w, h),
        name_pos: vec2(x0 + 6.0, y + 4.0),
        text_x: x0 + 6.0,
        text_y: y + head,
        lines,
        mute: Rect::new(x1 - 6.0 - mute_w, y + 3.0, mute_w, 11.0),
        mute_label,
        step,
    })
}

fn draw_call(st: &HudState, bindings: &Bindings, subtitles: bool) {
    let Some(l) = call_layout(st, bindings, subtitles) else { return };
    fill(l.rect, Color::new(0.0, 0.0, 0.0, 0.78));
    stroke(l.rect, Color::new(0.353, 0.329, 0.275, 1.0), 1.0);
    vtext("Z E L L I A", l.name_pos.x, l.name_pos.y, 5.0, AMBER);
    for (i, s) in l.lines.iter().enumerate() {
        vtext(s, l.text_x, l.text_y + i as f32 * l.step, 5.0, INK);
    }
    let hot = l.mute.contains(mouse_live());
    button_box(l.mute, &l.mute_label, 5.0, hot, false);
}

// ---- the status block and the rest ----

/// Power text, with the default "Power left: n%".
pub fn power_line(st: &HudState) -> String {
    match &st.power_text {
        Some(t) => t.clone(),
        None => format!("Power left: {}%", st.power.ceil() as i32),
    }
}

/// Usage text, with the default "Usage: |||" (usage + 1 bars, 1 to 7).
pub fn usage_line(st: &HudState) -> String {
    match &st.usage_text {
        Some(t) => t.clone(),
        None => format!("Usage: {}", "|".repeat((st.usage + 1).clamp(1, 7) as usize)),
    }
}

/// The clock text: the override, else the hour label.
pub fn clock_line(st: &HudState) -> String {
    match &st.clock_text {
        Some(t) => t.clone(),
        None => hour_label(st.hour),
    }
}

fn night_line(st: &HudState) -> String {
    if !st.mode_text.is_empty() {
        st.mode_text.clone()
    } else {
        st.night_label.clone()
    }
}

/// The bar tracks while the monitor is up: small bars in the top strip, right-aligned above the feed, the last bar
/// rightmost. Returns each bar's track and the label's left x, in bar order.
pub fn monitor_bar_tracks(st: &HudState) -> Vec<(Rect, f32)> {
    let mut right = super::monitor::AREA.x + super::monitor::AREA.w;
    let mut out = Vec::new();
    for (label, _, _) in st.bars.iter().rev() {
        let track = Rect::new(right - 30.0, 6.0, 30.0, 4.0);
        let lx = track.x - 3.0 - tw(&ut::glyph(label), 5.0);
        out.push((track, lx));
        right = lx - 8.0;
    }
    out.reverse();
    out
}

/// Everything drawn over the 3D view (and over the monitor): status, clock, buttons, call, note, prompt, crosshair, bars, panels.
pub fn draw_all(st: &HudState, bindings: &Bindings, subtitles: bool) {
    // status block, top left
    if st.monitor_up {
        // the feed owns the middle: the status goes on one line above it, the bars beside it, the panels are the mode's
        let line = format!("{}    {}    {}", ut::glyph(&power_line(st)), ut::glyph(&usage_line(st)), ut::glyph(&night_line(st)));
        vtext(&line, 8.0, 3.0, 8.0, INK);
        for ((label, v, col), (track, lx)) in st.bars.iter().zip(monitor_bar_tracks(st)) {
            fill(track, Color::new(0.0, 0.0, 0.0, 0.55));
            fill(Rect::new(track.x, track.y, track.w * v.clamp(0.0, 1.0), track.h), c3(*col));
            stroke(track, Color::new(0.35, 0.33, 0.27, 1.0), 1.0);
            vtext(&ut::glyph(label), lx, 5.0, 5.0, c3(*col));
        }
    } else {
    vtext(&ut::glyph(&power_line(st)), 8.0, 6.0, 9.0, INK);
    vtext(&ut::glyph(&usage_line(st)), 8.0, 18.0, 9.0, INK);
    vtext(&ut::glyph(&night_line(st)), 8.0, 30.0, 8.0, Color::new(0.80, 0.77, 0.68, 1.0));
    // small bars under it
    let mut y = 42.0;
    for (label, v, col) in &st.bars {
        vtext(&ut::glyph(label), 8.0, y, 6.0, c3(*col));
        let track = Rect::new(8.0, y + 8.0, 70.0, 4.0);
        fill(track, Color::new(0.0, 0.0, 0.0, 0.55));
        fill(Rect::new(track.x, track.y, track.w * v.clamp(0.0, 1.0), track.h), c3(*col));
        stroke(track, Color::new(0.35, 0.33, 0.27, 1.0), 1.0);
        y += 15.0;
    }
    }
    // free text panels
    for p in &st.panels {
        let mut py = p.y;
        for (s, col, size) in &p.lines {
            vtext(&ut::glyph(s), p.x, py, *size, c3(*col));
            py += size + 3.0;
        }
    }
    // clock and pause, top right
    vtext_right(&ut::glyph(&clock_line(st)), W - 8.0, 6.0, 12.0, INK);
    let hot = pause_rect().contains(mouse_live());
    button_box(pause_rect(), "II Pause", 6.0, hot, false);
    // CAMERAS button
    if st.monitor_available && !st.monitor_up {
        let label = format!("^ CAMERAS  {}", bindings.key_text("nw_monitor").to_uppercase());
        let hot = cameras_rect().contains(mouse_live());
        button_box(cameras_rect(), &label, 6.0, hot, false);
    }
    if !st.monitor_up {
        // crosshair
        if st.show_crosshair {
            let c = vec2(W * 0.5, H * 0.5);
            let col = Color::new(0.95, 0.93, 0.88, 0.85);
            fill(Rect::new(c.x - 0.5, c.y - 0.5, 1.0, 1.0), col);
            fill(Rect::new(c.x - 4.5, c.y - 0.25, 2.5, 0.5), col);
            fill(Rect::new(c.x + 2.0, c.y - 0.25, 2.5, 0.5), col);
            fill(Rect::new(c.x - 0.25, c.y - 4.5, 0.5, 2.5), col);
            fill(Rect::new(c.x - 0.25, c.y + 2.0, 0.5, 2.5), col);
        }
        // prompt just under the crosshair, with a 1px shadow
        if !st.prompt.is_empty() {
            let p = ut::glyph(&st.prompt);
            vtext_center(&p, W * 0.5 + 1.0, H * 0.5 + 16.0 + 1.0, 10.0, Color::new(0.0, 0.0, 0.0, 1.0));
            vtext_center(&p, W * 0.5, H * 0.5 + 16.0, 10.0, Color::new(0.949, 0.929, 0.882, 1.0));
        }
        if SHOW_KEY_HINT && !st.hint.is_empty() && call_layout(st, bindings, subtitles).is_none() {
            vtext(&ut::glyph(&st.hint), 8.0, H - 22.0, 6.0, DIM);
        }
    }
    // toast
    if let Some((text, left)) = &st.note {
        if *left > 0.0 && !text.is_empty() {
            let col = Color::new(0.85, 0.82, 0.75, 1.0);
            if st.monitor_up {
                // with the monitor up the middle is the picture: the toast goes in the right column under CLOSE CAMERAS
                let cr = close_rect();
                for (i, l) in ut::wrap(&ut::glyph(text), cr.w, 6.0).iter().take(3).enumerate() {
                    vtext_center(l, cr.x + cr.w * 0.5, cr.y + cr.h + 4.0 + i as f32 * 8.0, 6.0, col);
                }
            } else {
                vtext_center(&ut::glyph(text), W * 0.5, H * 0.5 + 30.0, 8.0, col);
            }
        }
    }
    draw_call(st, bindings, subtitles);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_lines_follow_the_godot_text() {
        let mut st = HudState::default();
        st.power = 99.2;
        st.usage = 0;
        assert_eq!(power_line(&st), "Power left: 100%");
        assert_eq!(usage_line(&st), "Usage: |");
        st.usage = 3;
        assert_eq!(usage_line(&st), "Usage: ||||");
        st.usage = 40;
        assert_eq!(usage_line(&st), "Usage: |||||||");
        st.usage = -5;
        assert_eq!(usage_line(&st), "Usage: |");
    }

    #[test]
    fn overrides_win() {
        let mut st = HudState::default();
        st.power_text = Some("Fuel: 4".into());
        st.usage_text = Some("Noise".into());
        st.clock_text = Some("DEPTH 3".into());
        assert_eq!(power_line(&st), "Fuel: 4");
        assert_eq!(usage_line(&st), "Noise");
        assert_eq!(clock_line(&st), "DEPTH 3");
        st.clock_text = None;
        st.hour = 0;
        assert_eq!(clock_line(&st), "12 AM");
        st.hour = 3;
        assert_eq!(clock_line(&st), "3 AM");
    }

    #[test]
    fn night_line_prefers_the_mode_text() {
        let mut st = HudState::default();
        st.night_label = "A".into();
        assert_eq!(night_line(&st), "A");
        st.mode_text = "B".into();
        assert_eq!(night_line(&st), "B");
    }

    #[test]
    fn the_call_text_comes_from_the_plain_text_first() {
        let mut st = HudState::default();
        assert_eq!(call_text_of(&st), "");
        st.call_text = "hello".into();
        assert_eq!(call_text_of(&st), "hello");
    }

    #[test]
    fn a_long_call_line_wraps_and_every_word_shows() {
        let mut st = HudState::default();
        st.call_text = "The cameras are on Space or W, and the music box is breathing on the left, so keep winding it before it sleeps".into();
        let l = call_layout(&st, &Bindings::from_save(&crate::core::save::Save::default()), true).unwrap();
        assert!(l.lines.len() > 1);
        assert_eq!(l.lines.join(" "), st.call_text);
        assert!(l.rect.h >= 14.0 + l.lines.len() as f32 * l.step);
        assert!(l.rect.y + l.rect.h <= H);
    }

    #[test]
    fn fixed_buttons_sit_where_godot_puts_them() {
        assert_eq!(pause_rect().x + pause_rect().w, 472.0);
        assert_eq!(cameras_rect().x + cameras_rect().w * 0.5, 240.0);
        assert_eq!(cameras_rect().y + cameras_rect().h, 265.0);
    }
}
