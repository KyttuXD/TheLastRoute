//! Mouse look maths, copied from last_route's `config/look.rs` and `data/player/look.json` (copy only; last_route is not
//! changed), so lr_story turns exactly like the main game: the same delta-to-pixel scale, the same sensitivity, the same
//! per-platform multiplier and the same sign (moving the mouse right turns right, down looks down).

use macroquad::prelude::Vec2;

/// Radians per Godot viewport pixel (last_route `config::MOUSE_SENSITIVITY`).
pub const SENSITIVITY: f32 = 0.0075;
/// Pitch limit in radians (look.json `pitch_limit_rad`, Godot `clamp(_pitch, -1.4, 1.4)`).
pub const PITCH_LIMIT: f32 = 1.4;
/// Hard cap on one frame's move in pixels (look.json `per_frame_max_px`): guards junk values, never scales a real move.
pub const PER_FRAME_MAX_PX: f32 = 700.0;

/// The per-platform multiplier (look.json `mouse_multiplier_*`): the browser and Windows feel, macOS and others.
pub fn multiplier() -> f32 {
    if cfg!(target_arch = "wasm32") || cfg!(target_os = "windows") {
        0.583_333_3
    } else {
        0.333_333_3
    }
}

/// One frame's mouse move in pixels. `delta` is macroquad's `mouse_delta_position` (last minus current, in [-1, 1]
/// screen units, so a full-width sweep is 2); `screen` is the window size. The result has the Godot signs: moving right
/// is +x, moving down is +y. (last_route `delta_px` without its dpi and fallback inputs, which lr_story does not use.)
pub fn delta_px(delta: Vec2, screen: Vec2) -> Vec2 {
    let px = Vec2::new(-delta.x, -delta.y) * screen * 0.5;
    px.clamp_length_max(PER_FRAME_MAX_PX)
}

/// The look step in radians for one frame's move: pixels times the sensitivity times the platform multiplier.
pub fn radians(delta: Vec2, screen: Vec2) -> Vec2 {
    delta_px(delta, screen) * SENSITIVITY * multiplier()
}

#[cfg(test)]
mod tests {
    use super::*;
    use macroquad::prelude::vec2;

    /// A sweep across the whole width (a delta of 2) is the window width in pixels, as in last_route.
    #[test]
    fn a_full_width_sweep_is_the_window_width_in_pixels() {
        let px = delta_px(vec2(-2.0, 0.0), vec2(1280.0, 720.0));
        assert!((px.x - 700.0).abs() < 1e-3, "capped at the per-frame limit: {}", px.x);
        let px = delta_px(vec2(-0.5, 0.0), vec2(1280.0, 720.0));
        assert!((px.x - 320.0).abs() < 1e-3, "{}", px.x);
    }

    /// Moving the mouse right and down (macroquad's delta goes negative) gives positive x and y.
    #[test]
    fn right_and_down_are_positive() {
        let r = radians(vec2(-0.01, -0.01), vec2(1280.0, 720.0));
        assert!(r.x > 0.0 && r.y > 0.0);
        assert!((r.x - 6.4 * SENSITIVITY * multiplier()).abs() < 1e-6);
    }
}
