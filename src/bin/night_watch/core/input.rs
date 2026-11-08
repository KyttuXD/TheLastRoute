//! One input snapshot per frame. Built from macroquad by `InputFrame::from_macroquad` (the only core function that
//! reads input), or by hand in tests.
#![allow(dead_code)]

use super::controls::{Bindings, ACTIONS};
use macroquad::prelude::{
    get_frame_time, is_key_down, is_key_pressed, is_key_released, is_mouse_button_down, is_mouse_button_pressed, mouse_delta_position,
    mouse_position, screen_dpi_scale, screen_height, screen_width, KeyCode, MouseButton,
};
use std::collections::HashSet;
use std::sync::Mutex;

#[derive(Clone, Default)]
pub struct InputFrame {
    /// Actions held.
    pub down: HashSet<&'static str>,
    /// Actions that went down this frame.
    pub pressed: HashSet<&'static str>,
    pub released: HashSet<&'static str>,
    /// 0..=9 pressed this frame (camera and keypad).
    pub digit: Option<u8>,
    /// Screen px.
    pub mouse: (f32, f32),
    pub mouse_delta: (f32, f32),
    pub click: bool,
    pub click_down: bool,
    pub click_right: bool,
    /// Real seconds of this frame, already clamped to 0.1.
    pub dt: f32,
}

impl InputFrame {
    pub fn is_down(&self, a: &str) -> bool {
        self.down.contains(a)
    }
    pub fn just(&self, a: &str) -> bool {
        self.pressed.contains(a)
    }
    pub fn up(&self, a: &str) -> bool {
        self.released.contains(a)
    }

    /// A frame with `dt` seconds and nothing pressed (tests, headless runs).
    pub fn idle(dt: f32) -> InputFrame {
        InputFrame { dt, ..InputFrame::default() }
    }

    /// Builder for tests: an action held down (and counted as pressed this frame).
    pub fn with_press(mut self, action: &'static str) -> InputFrame {
        self.down.insert(action);
        self.pressed.insert(action);
        self
    }

    /// Builder for tests: an action held, not newly pressed.
    pub fn with_held(mut self, action: &'static str) -> InputFrame {
        self.down.insert(action);
        self
    }

    /// Snapshot of this frame's keyboard and mouse. Mouse look is the platform's relative motion in screen px (right
    /// and down are positive), with a position-delta fallback for backends that report none.
    pub fn from_macroquad(b: &Bindings) -> InputFrame {
        let mut f = InputFrame::default();
        for a in ACTIONS {
            let keys = b.keys(a.name);
            if keys.iter().any(|k| is_key_down(*k)) {
                f.down.insert(a.name);
            }
            if keys.iter().any(|k| is_key_pressed(*k)) {
                f.pressed.insert(a.name);
            }
            if keys.iter().any(|k| is_key_released(*k)) {
                f.released.insert(a.name);
            }
        }
        // first-person movement (player.rs reads these names; they are not rebindable actions)
        for (name, key) in MOVE_KEYS.iter() {
            if is_key_down(*key) {
                f.down.insert(*name);
            }
        }
        if is_key_down(KeyCode::C) {
            f.down.insert("crouch");
        }
        // browsers cannot always grab the mouse, so the arrow keys also look there
        #[cfg(target_arch = "wasm32")]
        for (name, key) in LOOK_KEYS.iter() {
            if is_key_down(*key) {
                f.down.insert(*name);
            }
        }
        const DIGITS: [(KeyCode, KeyCode); 10] = [
            (KeyCode::Key0, KeyCode::Kp0),
            (KeyCode::Key1, KeyCode::Kp1),
            (KeyCode::Key2, KeyCode::Kp2),
            (KeyCode::Key3, KeyCode::Kp3),
            (KeyCode::Key4, KeyCode::Kp4),
            (KeyCode::Key5, KeyCode::Kp5),
            (KeyCode::Key6, KeyCode::Kp6),
            (KeyCode::Key7, KeyCode::Kp7),
            (KeyCode::Key8, KeyCode::Kp8),
            (KeyCode::Key9, KeyCode::Kp9),
        ];
        for (n, (k, kp)) in DIGITS.iter().enumerate() {
            if is_key_pressed(*k) || is_key_pressed(*kp) {
                f.digit = Some(n as u8);
                break;
            }
        }
        let pos = mouse_position();
        f.mouse = pos;
        let delta = mouse_delta_position();
        let dpi = screen_dpi_scale().max(0.01);
        let last = {
            let mut guard = match LAST_MOUSE.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let before = *guard;
            *guard = Some(pos);
            before
        };
        f.mouse_delta = if delta.length_squared() > 0.0 {
            // macroquad reports last - current in [-1, 1] screen units; a full-width sweep is 2
            (-delta.x * screen_width() * 0.5 * dpi, -delta.y * screen_height() * 0.5 * dpi)
        } else if let Some(l) = last {
            ((pos.0 - l.0) * dpi, (pos.1 - l.1) * dpi)
        } else {
            (0.0, 0.0)
        };
        f.click = is_mouse_button_pressed(MouseButton::Left);
        f.click_down = is_mouse_button_down(MouseButton::Left);
        f.click_right = is_mouse_button_pressed(MouseButton::Right);
        f.dt = get_frame_time().clamp(0.0, 0.1);
        f
    }
}

const MOVE_KEYS: &[(&str, KeyCode)] = &[
    ("move_forward", KeyCode::W),
    ("move_back", KeyCode::S),
    ("move_left", KeyCode::A),
    ("move_right", KeyCode::D),
    ("sprint", KeyCode::LeftShift),
    ("crouch", KeyCode::LeftControl),
];

#[cfg(target_arch = "wasm32")]
const LOOK_KEYS: &[(&str, KeyCode)] = &[
    ("look_left", KeyCode::Left),
    ("look_right", KeyCode::Right),
    ("look_up", KeyCode::Up),
    ("look_down", KeyCode::Down),
];

static LAST_MOUSE: Mutex<Option<(f32, f32)>> = Mutex::new(None);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_read_the_sets() {
        let f = InputFrame::idle(0.016).with_press("nw_lamp").with_held("nw_door_left");
        assert!(f.is_down("nw_lamp") && f.just("nw_lamp"));
        assert!(f.is_down("nw_door_left") && !f.just("nw_door_left"));
        assert!(!f.up("nw_lamp"));
        assert!(!f.is_down("nw_monitor"));
        assert_eq!(f.dt, 0.016);
    }

    #[test]
    fn default_frame_is_empty() {
        let f = InputFrame::default();
        assert!(f.down.is_empty() && f.pressed.is_empty() && f.released.is_empty());
        assert_eq!(f.digit, None);
        assert!(!f.click && !f.click_down && !f.click_right);
    }
}
