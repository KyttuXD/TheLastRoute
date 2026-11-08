//! The monitor overlay (port of the `_build_monitor` part of `night_watch_hud.gd`): an opaque dark backdrop, the 16:9
//! feed in the left 338x190 frame (the right column holds the clock, the mode panel, the camera map and CLOSE CAMERAS; the
//! strip under the feed holds the call box and the key guide), `CAM n  LABEL` above it, a blinking REC and the static flash.
#![allow(dead_code)]

use super::cam_map;
use super::status::*;
use crate::core::{Bindings, HudState, Mulberry32};
use crate::gfx::Gfx;
use crate::ui::text as ut;
use macroquad::prelude::*;

/// The area the picture may use (the Godot AspectRatioContainer's box), in virtual px.
pub const AREA: Rect = Rect { x: 8.0, y: 28.0, w: 338.0, h: 190.0 };
pub const STATIC_SECONDS: f32 = 0.14;
/// Top of the key guide, under the call box strip.
pub const GUIDE_Y: f32 = 256.0;

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

/// The camera number of a "CAM n  LABEL ..." title (0 when it has none).
fn label_number(label: &str) -> i32 {
    let rest = label.trim_start().strip_prefix("CAM ").unwrap_or("");
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

/// The title above the picture. It always follows the selected camera (`monitor_room`): the layout gives the
/// "CAM n  LABEL"; the mode's `cam_label` is used when it names the same camera (so extras such as the music
/// box readout show), and a stale label of another camera is replaced by the derived one.
pub fn title_of(st: &HudState) -> String {
    let derived = st.cam_map.as_ref().map(|l| cam_map::title_for(l, &st.monitor_room)).unwrap_or_default();
    if st.cam_label.is_empty() {
        return derived;
    }
    let n = label_number(&derived);
    if derived.is_empty() || n == 0 || label_number(&st.cam_label) == n {
        return st.cam_label.clone();
    }
    derived
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
    let vp = picture_rect();
    vtext_center(&title, vp.x + vp.w * 0.5, 17.0, 9.0, FEED_INK);
    if (get_time() * 1000.0 / 600.0) as i64 % 2 == 0 {
        dot(vec2(366.0, 11.0), 3.0, REC_RED);
        vtext("REC", 372.0, 5.0, 10.0, REC_RED);
    }
    // the right column: CLOSE CAMERAS under the map; the live key guide in the strip under the feed (below the call box)
    let close = format!("CLOSE CAMERAS  {}", bindings.key_text("nw_monitor").to_uppercase());
    let hot = close_rect().contains(mouse_live());
    button_box(close_rect(), &close, 6.0, hot, false);
    let guide = ut::glyph(&guide_of(st, bindings));
    for (i, l) in ut::wrap(&guide, vp.w, 5.0).iter().take(2).enumerate() {
        vtext(l, vp.x, GUIDE_Y + i as f32 * 7.0, 5.0, DIM);
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

    #[test]
    fn the_title_follows_the_selected_camera() {
        use crate::core::CamLayout;
        let mut st = HudState::default();
        st.cam_map = Some(CamLayout {
            rooms: vec![("stage".into(), vec2(0.0, 0.0)), ("kitchen".into(), vec2(8.0, 0.0))],
            cams: vec!["stage".into(), "kitchen".into()],
            labels: vec![("stage".into(), "Show Stage".into()), ("kitchen".into(), "Kitchen".into())],
            ..Default::default()
        });
        st.cam_label = "CAM 1  SHOW STAGE".into();
        st.monitor_room = "stage".into();
        assert_eq!(title_of(&st), "CAM 1  SHOW STAGE");
        // the map selection moves on while the mode's label is stale: the header follows
        st.monitor_room = "kitchen".into();
        assert_eq!(title_of(&st), "CAM 2  KITCHEN");
        // same camera: the mode's label (with its extras) is kept
        st.cam_label = "CAM 2  KITCHEN    BOX 40%".into();
        assert_eq!(title_of(&st), "CAM 2  KITCHEN    BOX 40%");
        assert_eq!(label_number("BUS"), 0);
    }
}
