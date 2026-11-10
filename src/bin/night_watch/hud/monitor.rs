//! The monitor overlay (port of the `_build_monitor` part of `night_watch_hud.gd`): an opaque dark backdrop, the 16:9
//! feed inset (22 px left and right, 12 top, 30 bottom), `CAM n  LABEL` above it, a blinking REC, the static flash, the
//! CLOSE CAMERAS strip with the key guide, and the camera map bottom right.
#![allow(dead_code)]

use super::cam_map;
use super::status::*;
use crate::core::{Bindings, HudState, Mulberry32};
use crate::gfx::Gfx;
use crate::ui::text as ut;
use macroquad::prelude::*;

/// The area the picture may use (the Godot AspectRatioContainer's box), in virtual px.
pub const AREA: Rect = Rect { x: 22.0, y: 12.0, w: 436.0, h: 228.0 };
pub const STATIC_SECONDS: f32 = 0.14;

/// The 16:9 picture centred in AREA (virtual px).
pub fn picture_rect() -> Rect {
    fit_16_9(AREA)
}

/// The largest 16:9 rectangle that fits `a`, centred.
pub fn fit_16_9(a: Rect) -> Rect {
    let ratio = 16.0 / 9.0;
    let (w, h) = if a.w / a.h > ratio { (a.h * ratio, a.h) } else { (a.w, a.w / ratio) };
    Rect::new(a.x + (a.w - w) * 0.5, a.y + (a.h - h) * 0.5, w, h)
}

/// The static flash colour for a remaining time (alpha fades to 0 over STATIC_SECONDS, peak 0.9). `r1`, `r2` in 0..1.
pub fn static_color(left_s: f32, r1: f32, r2: f32) -> Color {
    Color::new(0.7 + r1 * 0.2, 0.74 + r2 * 0.2, 0.72, (left_s / STATIC_SECONDS).clamp(0.0, 1.0) * 0.9)
}

/// The title above the picture: the mode's `cam_label`, else derived from the layout and the active room.
pub fn title_of(st: &HudState) -> String {
    if !st.cam_label.is_empty() {
        return st.cam_label.clone();
    }
    match &st.cam_map {
        Some(l) => cam_map::title_for(l, &st.monitor_room),
        None => String::new(),
    }
}

/// The guide line beside CLOSE CAMERAS: the mode's hint, else the generic camera hint line.
pub fn guide_of(st: &HudState, bindings: &Bindings) -> String {
    if !st.hint.is_empty() {
        st.hint.clone()
    } else {
        bindings.hint_line(true, &[])
    }
}

/// Draw the monitor. The feed was already rendered into `gfx.feed` by the mode this frame.
pub fn draw_monitor(st: &HudState, bindings: &Bindings, gfx: &Gfx, calm: bool, rng: &mut Mulberry32) {
    // opaque backdrop over the whole window (the HUD text under it must not ghost through)
    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.02, 0.03, 0.03, 1.0));
    let pic = v2r(picture_rect());
    gfx.feed.draw_to_rect(pic.x, pic.y, pic.w, pic.h, WHITE);
    // static flash over the whole feed area
    if st.cam_flash_s > 0.0 && !calm {
        let c = static_color(st.cam_flash_s, rng.next() as f32, rng.next() as f32);
        fill(AREA, c);
        for _ in 0..10 {
            let y = AREA.y + rng.next() as f32 * AREA.h;
            let h = 0.6 + rng.next() as f32 * 2.0;
            fill(Rect::new(AREA.x, y, AREA.w, h), Color::new(0.9, 0.92, 0.9, c.a * 0.5));
        }
    }
    // furniture: room name (centred above the picture), REC (left of the clock)
    let title = ut::glyph(&title_of(st));
    vtext_center(&title, 240.0, 14.0, 11.0, FEED_INK);
    if (get_time() * 1000.0 / 600.0) as i64 % 2 == 0 {
        dot(vec2(364.0, 14.0), 3.0, REC_RED);
        vtext("REC", 370.0, 8.0, 10.0, REC_RED);
    }
    // the strip under the picture: CLOSE CAMERAS and the live key guide
    let close = format!("CLOSE CAMERAS  {}", bindings.key_text("nw_monitor").to_uppercase());
    let hot = close_rect().contains(mouse_live());
    button_box(close_rect(), &close, 6.0, hot, false);
    let guide = ut::glyph(&guide_of(st, bindings));
    let gx = close_rect().x + close_rect().w + 8.0;
    for (i, l) in ut::wrap(&guide, 458.0 - gx, 5.0).iter().take(2).enumerate() {
        vtext(l, gx, 248.0 + i as f32 * 7.0, 5.0, DIM);
    }
    cam_map::draw(st, mouse_live(), get_time());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_16_9_and_inside_the_area() {
        let p = picture_rect();
        assert!((p.w / p.h - 16.0 / 9.0).abs() < 0.001);
        assert!(p.x >= AREA.x - 0.001 && p.y >= AREA.y - 0.001);
        assert!(p.x + p.w <= AREA.x + AREA.w + 0.001 && p.y + p.h <= AREA.y + AREA.h + 0.001);
        // the area is wider than 16:9, so the picture fills the height and is centred
        assert!((p.h - AREA.h).abs() < 0.001);
        assert!(((p.x + p.w * 0.5) - (AREA.x + AREA.w * 0.5)).abs() < 0.001);
    }

    #[test]
    fn a_tall_area_fits_by_width() {
        let p = fit_16_9(Rect::new(0.0, 0.0, 160.0, 400.0));
        assert!((p.w - 160.0).abs() < 0.001 && (p.h - 90.0).abs() < 0.001);
        assert!((p.y - 155.0).abs() < 0.001);
    }

    #[test]
    fn static_fades_out_over_140_ms() {
        assert!((static_color(0.14, 0.0, 0.0).a - 0.9).abs() < 0.001);
        assert!((static_color(0.07, 0.0, 0.0).a - 0.45).abs() < 0.001);
        assert_eq!(static_color(0.0, 0.5, 0.5).a, 0.0);
        let c = static_color(0.14, 1.0, 1.0);
        assert!((c.r - 0.9).abs() < 0.001 && (c.g - 0.94).abs() < 0.001);
    }

    #[test]
    fn the_title_is_derived_when_the_mode_gives_none() {
        let mut st = HudState::default();
        assert_eq!(title_of(&st), "");
        st.cam_label = "CAM 3  PIRATE COVE".into();
        assert_eq!(title_of(&st), "CAM 3  PIRATE COVE");
    }
}
