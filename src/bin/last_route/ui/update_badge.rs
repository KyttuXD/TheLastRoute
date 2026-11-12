//! The top-centre update badge (TODO 358, Godot `game/menus/update_badge.gd`): the download percentage while an update comes
//! down, then RESTART NOW or LATER once it is ready, and a short status line during a run. The state it reads is the
//! updater's (`app::updater::UpdateStatus`); the texts and sizes are `data/startup.json` (`update_badge`).
//!
//! [`badge_for`] and [`status_line`] are pure (tested); [`draw`] only paints a [`Badge`] and reports a click.

use crate::app::startup;
use crate::app::updater::{percent, UpdateStatus};
use macroquad::prelude::*;

/// What the badge shows right now.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub struct Badge {
    pub text: String,
    /// 0-100 while downloading.
    pub percent: Option<u32>,
    /// RESTART NOW / LATER are offered.
    pub choices: bool,
}

/// What the player pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // not wired into the game yet
pub enum Click {
    RestartNow,
    Later,
}

#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
fn text_of(key: &str) -> String {
    startup::data().get("update_badge").and_then(|b| b.get(key)).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

#[allow(dead_code)] // not wired into the game yet
fn number_of(key: &str, fallback: f32) -> f32 {
    startup::data().get("update_badge").and_then(|b| b.get(key)).and_then(|v| v.as_f64()).map_or(fallback, |n| n as f32)
}

#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
fn with_percent(template: &str, p: u32) -> String {
    template.replace("{percent}", &p.to_string())
}

/// The badge for an updater state; `None` when nothing is to be shown (also after LATER, `dismissed`).
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn badge_for(status: &UpdateStatus, dismissed: bool) -> Option<Badge> {
    match status {
        UpdateStatus::Downloading { got, total, .. } => {
            let p = percent(*got, *total).min(100);
            Some(Badge { text: with_percent(&text_of("downloading"), p), percent: Some(p), choices: false })
        }
        UpdateStatus::Installing if !dismissed => Some(Badge { text: text_of("ready"), percent: None, choices: true }),
        _ => None,
    }
}

/// The status-bar text during a run (empty when there is nothing to say).
#[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
pub fn status_line(status: &UpdateStatus) -> String {
    match status {
        UpdateStatus::Downloading { got, total, .. } => with_percent(&text_of("run_status"), percent(*got, *total).min(100)),
        UpdateStatus::Installing => text_of("ready"),
        _ => String::new(),
    }
}

#[allow(dead_code)] // not wired into the game yet
fn button_rects(r: Rect) -> (Rect, Rect) {
    let w = r.w * 0.5 - 6.0;
    (Rect::new(r.x + 4.0, r.y + r.h + 4.0, w, r.h), Rect::new(r.x + r.w * 0.5 + 2.0, r.y + r.h + 4.0, w, r.h))
}

/// Paints the badge top-centre and returns the button pressed this frame.
#[allow(dead_code)] // not wired into the game yet
pub fn draw(badge: &Badge) -> Option<Click> {
    let (w, h, top) = (number_of("width", 360.0), number_of("height", 34.0), number_of("top_margin", 10.0));
    let r = Rect::new((screen_width() - w) * 0.5, top, w, h);
    draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.05, 0.05, 0.08, 0.88));
    if let Some(p) = badge.percent {
        draw_rectangle(r.x, r.y + r.h - 4.0, r.w * (p as f32 / 100.0), 4.0, Color::new(0.4, 0.8, 0.5, 1.0));
    }
    draw_text(&badge.text, r.x + 10.0, r.y + r.h * 0.65, 22.0, WHITE);
    if !badge.choices {
        return None;
    }
    let (now, later) = button_rects(r);
    let mut click = None;
    let mouse: Vec2 = mouse_position().into();
    for (rect, label, which) in [(now, text_of("restart_now"), Click::RestartNow), (later, text_of("later"), Click::Later)] {
        let hot = rect.contains(mouse);
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, if hot { Color::new(0.25, 0.45, 0.3, 0.95) } else { Color::new(0.1, 0.15, 0.12, 0.92) });
        draw_text(&label, rect.x + 10.0, rect.y + rect.h * 0.65, 22.0, WHITE);
        if hot && is_mouse_button_pressed(MouseButton::Left) {
            click = Some(which);
        }
    }
    click
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloading_shows_the_percentage() {
        let b = badge_for(&UpdateStatus::Downloading { build: 9, got: 50, total: 200 }, false).unwrap();
        assert_eq!(b.percent, Some(25));
        assert!(b.text.contains("25%") && !b.choices);
        assert!(status_line(&UpdateStatus::Downloading { build: 9, got: 50, total: 200 }).contains("25"));
    }

    #[test]
    fn ready_offers_restart_now_or_later_and_later_hides_it() {
        let b = badge_for(&UpdateStatus::Installing, false).unwrap();
        assert!(b.choices);
        assert_eq!(badge_for(&UpdateStatus::Installing, true), None);
        assert!(!text_of("restart_now").is_empty() && !text_of("later").is_empty());
    }

    #[test]
    fn idle_states_show_nothing() {
        for s in [UpdateStatus::Idle, UpdateStatus::Checking, UpdateStatus::UpToDate(3), UpdateStatus::Failed("x".into())] {
            assert_eq!(badge_for(&s, false), None);
            assert_eq!(status_line(&s), "");
        }
    }

    #[test]
    fn badge_texts_are_upper_case() {
        for key in ["downloading", "ready", "restart_now", "later", "run_status"] {
            let t = text_of(key);
            // the {placeholders} are filled in at run time and stay lower case; every other letter is upper case
            let mut in_slot = false;
            let words: String = t.chars().filter(|c| { match c { '{' => in_slot = true, '}' => in_slot = false, _ => {} } !in_slot }).collect();
            assert_eq!(words, words.to_uppercase(), "{key}");
        }
    }
}
