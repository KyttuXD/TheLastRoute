//! The emote selection wheel: a ring of slices around the screen centre, the first at the top and the rest clockwise.
//! The mouse or a stick picks the slice under its angle; a pointer inside the inner circle (or a stick pushed less than
//! the dead zone) picks nothing. The picking maths is pure so it can be tested; [`draw`] paints the ring over the HUD.
//! Sizes come from the `wheel` block of `data/player/emotes.json`.

use crate::render::anim::emote_pose::EmoteSet;
use macroquad::prelude::*;
use std::f32::consts::TAU;

/// The slice (0 = top, clockwise) that the offset (`dx`, `dy`) from the wheel's centre points into, for a wheel of
/// `count` slices. Screen axes: x right, y down. None for an empty wheel or a point closer than `inner` to the centre.
pub fn slice_at(count: usize, dx: f32, dy: f32, inner: f32) -> Option<usize> {
    if count == 0 || dx.hypot(dy) < inner.max(1e-4) {
        return None;
    }
    let angle = dx.atan2(-dy).rem_euclid(TAU);
    let step = TAU / count as f32;
    Some((((angle + step / 2.0) / step) as usize) % count)
}

/// The slice under a mouse pointer, given the wheel's centre and its inner radius in pixels.
pub fn pick_pointer(count: usize, centre: Vec2, pointer: Vec2, inner_px: f32) -> Option<usize> {
    slice_at(count, pointer.x - centre.x, pointer.y - centre.y, inner_px)
}

/// The slice a stick points at: `x` right and `y` down, each -1..1. Nothing under `dead_zone` (0..1) of full push.
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn pick_stick(count: usize, x: f32, y: f32, dead_zone: f32) -> Option<usize> {
    slice_at(count, x, y, dead_zone.max(1e-4))
}

/// The angle (radians, clockwise from the top) at the middle of slice `i` of `count`.
pub fn slice_centre_angle(count: usize, i: usize) -> f32 {
    TAU / count.max(1) as f32 * i as f32
}

/// Where the wheel is drawn: the screen centre and its outer and inner radius in pixels.
pub fn layout(set: &EmoteSet, screen: Vec2) -> (Vec2, f32, f32) {
    let outer = set.wheel.radius * screen.x.min(screen.y);
    (screen / 2.0, outer, outer * set.wheel.inner_fraction)
}

fn point(centre: Vec2, angle: f32, r: f32) -> Vec2 {
    centre + vec2(angle.sin(), -angle.cos()) * r
}

/// Paints the wheel over the HUD: one ring segment per emote with its upper-case name, the `hover` slice lit.
pub fn draw(set: &EmoteSet, hover: Option<usize>) {
    let n = set.emotes.len();
    if n == 0 {
        return;
    }
    let (centre, outer, inner) = layout(set, vec2(screen_width(), screen_height()));
    let step = TAU / n as f32;
    // the gap between slices, as an angle
    let gap = 0.02;
    let font = (set.wheel.text_scale * screen_width().min(screen_height())).max(10.0);
    for (i, e) in set.emotes.iter().enumerate() {
        let mid = slice_centre_angle(n, i);
        let (a0, a1) = (mid - step / 2.0 + gap, mid + step / 2.0 - gap);
        let lit = hover == Some(i);
        let fill = if lit { Color::new(0.95, 0.75, 0.2, 0.85) } else { Color::new(0.05, 0.05, 0.08, 0.6) };
        let segs = 8;
        for s in 0..segs {
            let (b0, b1) = (a0 + (a1 - a0) * s as f32 / segs as f32, a0 + (a1 - a0) * (s + 1) as f32 / segs as f32);
            let (p0, p1, q0, q1) = (point(centre, b0, inner), point(centre, b1, inner), point(centre, b0, outer), point(centre, b1, outer));
            draw_triangle(p0, q0, q1, fill);
            draw_triangle(p0, q1, p1, fill);
        }
        let label = point(centre, mid, (inner + outer) / 2.0);
        let dims = measure_text(&e.name, None, font as u16, 1.0);
        let ink = if lit { BLACK } else { WHITE };
        draw_text(&e.name, label.x - dims.width / 2.0, label.y + dims.height / 2.0, font, ink);
    }
    // the name of the pick in the middle, so a stick user sees what releasing does
    if let Some(e) = hover.and_then(|i| set.emotes.get(i)) {
        let dims = measure_text(&e.name, None, font as u16, 1.0);
        draw_text(&e.name, centre.x - dims.width / 2.0, centre.y + dims.height / 2.0, font, WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slice_under_an_angle_is_picked() {
        // 8 slices of 45 degrees: slice 0 straddles the top, slice 2 points right, slice 4 down, slice 6 left
        assert_eq!(slice_at(8, 0.0, -10.0, 1.0), Some(0));
        assert_eq!(slice_at(8, 10.0, 0.0, 1.0), Some(2));
        assert_eq!(slice_at(8, 0.0, 10.0, 1.0), Some(4));
        assert_eq!(slice_at(8, -10.0, 0.0, 1.0), Some(6));
        // just inside the edge of slice 0 on each side
        assert_eq!(slice_at(8, 3.0, -10.0, 1.0), Some(0));
        assert_eq!(slice_at(8, -3.0, -10.0, 1.0), Some(0));
        assert_eq!(slice_at(8, 10.0, -10.0, 1.0), Some(1));
        assert_eq!(slice_at(8, -10.0, -10.0, 1.0), Some(7));
        // 6 slices: 60 degrees each
        assert_eq!(slice_at(6, 10.0, -5.0, 1.0), Some(1));
    }

    #[test]
    fn the_dead_zone_and_an_empty_wheel_pick_nothing() {
        assert_eq!(slice_at(8, 0.5, 0.5, 5.0), None);
        assert_eq!(slice_at(0, 10.0, 0.0, 1.0), None);
        assert_eq!(pick_pointer(8, vec2(100.0, 100.0), vec2(100.0, 100.0), 10.0), None);
        assert_eq!(pick_pointer(8, vec2(100.0, 100.0), vec2(100.0, 40.0), 10.0), Some(0));
        assert_eq!(pick_stick(8, 0.2, 0.1, 0.5), None);
        assert_eq!(pick_stick(8, 0.9, 0.0, 0.5), Some(2));
    }

    #[test]
    fn a_slice_centre_picks_its_own_slice() {
        for n in 1..=8usize {
            for i in 0..n {
                let a = slice_centre_angle(n, i);
                assert_eq!(slice_at(n, a.sin() * 50.0, -a.cos() * 50.0, 1.0), Some(i));
            }
        }
    }
}
