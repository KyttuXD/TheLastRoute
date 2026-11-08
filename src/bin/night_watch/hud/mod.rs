//! Screen-space HUD for every mode (port of `night_watch_hud.gd`). The Hud reads `HudState`, never the mode. Call order per
//! frame (main.rs): `draw_base`, the mode's `draw_overlay`, then `draw_cards` (which also draws the death moment).
#![allow(dead_code)]

pub mod admin;
pub mod cam_map;
pub mod cards;
pub mod death;
pub mod monitor;
pub mod status;

pub use admin::{common_entries as admin_common_entries, AdminAction, AdminGate, AdminOverlay, AdminRegistry, ClassicAdmin, ModeAdmin};
pub use death::{pick_line, DeathMoment};

use crate::core::{Bindings, EndCard, HudState, InputFrame, Mulberry32, Save};
use crate::gfx::Gfx;
use macroquad::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HudAction {
    None,
    Resume,
    QuitToBoard,
    Retry,
    Next,
    Menu,
    ToggleMute,
}

/// A click on the always-on HUD buttons (extra to the contract: draw_base cannot return actions).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BaseClick {
    Pause,
    /// The CAMERAS button (open the monitor).
    Cameras,
    /// CLOSE CAMERAS (monitor up).
    CloseCameras,
    /// Zellia's Mute button.
    Mute,
    /// A camera button on the map.
    Camera(String),
}

/// Seconds a freshly shown card ignores keys and clicks, so the key that opened it cannot dismiss it.
pub const PAUSE_GUARD_S: f32 = 0.12;
pub const END_GUARD_S: f32 = 0.3;

pub struct Hud {
    frame: u32,
    calm: bool,
    subtitles: bool,
    card_kind: u8,
    card_age: f32,
    death: Option<DeathMoment>,
    blips: u32,
    /// The dev overlay (key 0); main.rs drives it through `admin.update_and_draw`.
    pub admin: AdminOverlay,
}

impl Hud {
    pub fn new() -> Hud {
        Hud {
            frame: 0,
            calm: false,
            subtitles: true,
            card_kind: 0,
            card_age: 0.0,
            death: None,
            blips: 0,
            admin: AdminOverlay::new(),
        }
    }

    /// Status, clock, call box, prompt, note, crosshair, bars, panels; with the monitor up, the feed inset, CAM label,
    /// REC, static flash, CLOSE CAMERAS strip and the camera map under them.
    pub fn draw_base(&mut self, st: &HudState, bindings: &Bindings, save: &Save, gfx: &Gfx) {
        self.frame = self.frame.wrapping_add(1);
        self.calm = save.reduced_motion();
        self.subtitles = save.subtitles;
        if st.monitor_up {
            let mut rng = Mulberry32::new(self.frame.wrapping_mul(2_654_435_761));
            monitor::draw_monitor(st, bindings, gfx, self.calm, &mut rng);
        }
        status::draw_all(st, bindings, self.subtitles);
    }

    /// Pause card, end card, or the death moment. Returns what the player chose this frame.
    pub fn draw_cards(&mut self, paused: bool, end: Option<&EndCard>, input: &InputFrame) -> HudAction {
        if let Some(m) = &self.death {
            if !m.done {
                cards::draw_death(m);
                return HudAction::None;
            }
        }
        let kind: u8 = if end.is_some() { 2 } else if paused { 1 } else { 0 };
        if kind != self.card_kind {
            self.card_kind = kind;
            self.card_age = 0.0;
        }
        if kind == 0 {
            return HudAction::None;
        }
        self.card_age += input.dt;
        let guard = if kind == 2 { END_GUARD_S } else { PAUSE_GUARD_S };
        let live = self.card_age >= guard;
        let mouse = status::mouse_v(input);
        if kind == 1 {
            let labels: Vec<String> = cards::PAUSE_BUTTONS.iter().map(|s| s.to_string()).collect();
            let rects = cards::draw_card("PAUSED", "", &labels, mouse);
            if !live {
                return HudAction::None;
            }
            if is_key_pressed(KeyCode::Escape) || input.just("pause") {
                return HudAction::Resume;
            }
            if is_key_pressed(KeyCode::Backspace) {
                return HudAction::QuitToBoard;
            }
            if input.click {
                if rects[0].contains(mouse) {
                    return HudAction::Resume;
                }
                if rects[1].contains(mouse) {
                    return HudAction::QuitToBoard;
                }
            }
            return HudAction::None;
        }
        let Some(card) = end else { return HudAction::None };
        let labels = cards::end_labels(card.can_next);
        let rects = cards::draw_card(&card.title, &card.body, &labels, mouse);
        if !live {
            return HudAction::None;
        }
        let enter = is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter);
        if enter {
            return if card.can_next { HudAction::Next } else { HudAction::Retry };
        }
        if is_key_pressed(KeyCode::Escape) {
            return HudAction::Menu;
        }
        if input.click {
            let mut i = 0;
            if card.can_next {
                if rects[i].contains(mouse) {
                    return HudAction::Next;
                }
                i += 1;
            }
            if rects[i].contains(mouse) {
                return HudAction::Retry;
            }
            if rects[i + 1].contains(mouse) {
                return HudAction::Menu;
            }
        }
        HudAction::None
    }

    /// Start the death moment for the catcher. Does nothing when the pick is not a hunter or cause line (Kathy, void
    /// hunters, the fallback copy), so `death_moment_active()` stays false and the end card follows at once.
    pub fn start_death_moment(&mut self, name: &str, hunter_id: &str, cause: &str, seed: u32, depth: i32, short: bool) {
        let (line, caption, plays) = pick_line(hunter_id, cause, &format!("nightwatch:{seed}"), depth, name);
        self.death = if plays && !line.trim().is_empty() {
            Some(DeathMoment::new(name, &line, caption, short, self.calm))
        } else {
            None
        };
    }

    pub fn death_moment_active(&self) -> bool {
        self.death.as_ref().map(|m| !m.done).unwrap_or(false)
    }

    /// Advance the moment; any key, click or tap finishes the typing then dismisses; Escape dismisses at once.
    pub fn update_death_moment(&mut self, dt_s: f32, input: &InputFrame) {
        let Some(m) = &mut self.death else { return };
        let pressed = get_last_key_pressed().is_some() || input.click || input.click_right;
        let escape = is_key_pressed(KeyCode::Escape);
        m.update(dt_s, pressed, escape);
        self.blips += m.take_blips();
        if m.done {
            self.death = None;
        }
    }

    /// Menu blips owed for typed words (the caller plays `audio::menu("move")` that many times); resets the count.
    pub fn take_death_blips(&mut self) -> u32 {
        std::mem::take(&mut self.blips)
    }

    /// The shown text of the moment, for tests and screen readers.
    pub fn death_line(&self) -> String {
        self.death.as_ref().map(|m| m.shown_line()).unwrap_or_default()
    }

    /// The camera button under a click on the map.
    pub fn cam_map_click(&self, st: &HudState, input: &InputFrame) -> Option<String> {
        if !st.monitor_up || !input.click {
            return None;
        }
        let layout = st.cam_map.as_ref()?;
        cam_map::hit(layout, status::mouse_v(input))
    }

    /// A click on the always-on buttons this frame (pause, CAMERAS, CLOSE CAMERAS, Mute, a map camera).
    pub fn base_click(&self, st: &HudState, bindings: &Bindings, input: &InputFrame) -> Option<BaseClick> {
        if !input.click {
            return None;
        }
        let m = status::mouse_v(input);
        if status::pause_rect().contains(m) {
            return Some(BaseClick::Pause);
        }
        if let Some(l) = status::call_layout(st, bindings, self.subtitles) {
            if l.mute.contains(m) {
                return Some(BaseClick::Mute);
            }
        }
        if st.monitor_up {
            if status::close_rect().contains(m) {
                return Some(BaseClick::CloseCameras);
            }
            return self.cam_map_click(st, input).map(BaseClick::Camera);
        }
        if st.monitor_available && status::cameras_rect().contains(m) {
            return Some(BaseClick::Cameras);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hunter_with_a_line_starts_a_moment_and_kathy_does_not() {
        let mut h = Hud::new();
        h.start_death_moment("Dovelle", "dovelle", "", 7, 1, false);
        assert!(h.death_moment_active());
        h.start_death_moment("Kathy", "kathy", "", 7, 7, false);
        assert!(!h.death_moment_active());
    }

    #[test]
    fn the_moment_types_then_ends_without_input() {
        let mut h = Hud::new();
        h.start_death_moment("Aki", "aki", "", 3, 2, true);
        assert!(h.death_moment_active());
        assert!(!h.death_line().is_empty(), "short mode shows the whole line at once");
    }
}
