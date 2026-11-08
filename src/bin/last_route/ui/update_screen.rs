//! The UPDATING screen (web build): a full-screen dim overlay drawn over everything (menus, lobby, pause menu, the run)
//! when the deployed version differs from the one the page loaded with. It reads
//! "UPDATING // NEW VERSION READY", counts "THE GAME RELOADS IN N" down and reloads the page; UPDATE NOW reloads at
//! once (Enter does the same), and LATER, offered only during a run, hides it for a while.
//!
//! The state and the timing are `app/web_update.rs` (a pure state machine, tested natively); the numbers and every
//! string are `data/ui/update_screen.json` (`lr.update_screen/1`), checked by [`parse`] when the game starts.
//! While the screen is up it holds the input (see `Overlay::swallow_input` and the gates in `app/mod.rs`), lets go of
//! the mouse, and pauses nothing else.

use super::style::*;
use crate::app::assets::Assets;
use crate::app::web_pointer;
use crate::app::web_update::{self, Event, Timing, UpdateScreen};
use macroquad::prelude::*;
use serde_json::Value;
use std::sync::OnceLock;

const FILE: &str = "ui/update_screen.json";
const SCHEMA: &str = "lr.update_screen/1";

/// Everything the screen says and the numbers it runs on.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub timing: Timing,
    /// Opacity of the black dim over whatever is behind the screen.
    pub scrim_alpha: f32,
    pub kicker: String,
    pub title: String,
    /// Contains `{n}`, the whole seconds left.
    pub countdown_line: String,
    pub reloading_line: String,
    /// Shown during a run: what a reload costs.
    pub run_warning: String,
    /// Shown outside a run.
    pub idle_note: String,
    pub update_now_label: String,
    pub later_label: String,
}

/// The shipped rules. The file is embedded, so a missing or broken one is a build defect and panics with the reason.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under(FILE)
            .next()
            .map(|x| x.1)
            .unwrap_or_else(|| panic!("data/{FILE} is missing from the build"));
        parse(text).unwrap_or_else(|e| panic!("data/{FILE}: {e}"))
    })
}

fn number(v: &Value, key: &str, min: f64, max: f64) -> Result<f64, String> {
    let x = v[key].as_f64().ok_or_else(|| format!("{key} is not a number"))?;
    if !x.is_finite() || x < min || x > max {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(x)
}

fn text(v: &Value, key: &str, max_chars: usize) -> Result<String, String> {
    let s = v[key].as_str().ok_or_else(|| format!("{key} is not text"))?;
    if s.trim().is_empty() {
        return Err(format!("{key} is empty"));
    }
    if s.chars().count() > max_chars {
        return Err(format!("{key} is longer than {max_chars} characters"));
    }
    if s.chars().any(|c| c.is_control()) {
        return Err(format!("{key} holds a control character"));
    }
    Ok(s.to_string())
}

/// Reads and checks the file's text. The error names the field that is wrong.
pub fn parse(source: &str) -> Result<Rules, String> {
    let v: Value = serde_json::from_str(source).map_err(|e| format!("not valid JSON: {e}"))?;
    if v["schema"].as_str() != Some(SCHEMA) {
        return Err(format!("schema must be {SCHEMA}"));
    }
    let countdown_line = text(&v, "countdown_line", 60)?;
    if countdown_line.matches("{n}").count() != 1 {
        return Err("countdown_line must contain {n} exactly once".into());
    }
    Ok(Rules {
        timing: Timing {
            countdown: number(&v, "countdown_seconds", 1.0, 60.0)?,
            postpone: number(&v, "postpone_seconds", 10.0, 3600.0)?,
            max_step: number(&v, "max_step_seconds", 0.01, 5.0)?,
        },
        scrim_alpha: number(&v, "scrim_alpha", 0.3, 1.0)? as f32,
        kicker: text(&v, "kicker", 40)?,
        title: text(&v, "title", 40)?,
        countdown_line,
        reloading_line: text(&v, "reloading_line", 40)?,
        run_warning: text(&v, "run_warning", 120)?,
        idle_note: text(&v, "idle_note", 120)?,
        update_now_label: text(&v, "update_now_label", 16)?,
        later_label: text(&v, "later_label", 16)?,
    })
}

/// The countdown line with `n` put in.
pub fn countdown_text(rules: &Rules, n: u32) -> String {
    rules.countdown_line.replace("{n}", &n.to_string())
}

/// What the player pressed this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    None,
    UpdateNow,
    Later,
}

/// The screen as the app holds it: the state machine plus the drawing.
pub struct Overlay {
    rules: &'static Rules,
    state: UpdateScreen,
    /// When the new version was found (Unix seconds, UTC), shown as "LAST CHECKED: NOV 8, 2026 3:42 PM EST" in the
    /// player's zone (app/updater.rs `checked_line`).
    found_at: Option<i64>,
}

impl Overlay {
    pub fn new() -> Overlay {
        let rules = rules();
        Overlay { rules, state: UpdateScreen::new(rules.timing), found_at: None }
    }

    /// Whether the screen was up on the last frame: the rest of the game then takes no input.
    pub fn is_shown(&self) -> bool {
        self.state.is_shown()
    }

    /// Eats the typed characters of this frame so a text field behind the screen does not fill up.
    pub fn swallow_input(&self) {
        while get_char_pressed().is_some() {}
    }

    /// The end-of-frame hook: tick with the frame's time, act on the events, and draw the screen last when it is up.
    pub fn frame(&mut self, a: &Assets, dt: f32, update_available: bool, in_run: bool) {
        let events = self.state.tick(dt, update_available, in_run);
        if events.contains(&Event::Shown) {
            self.found_at = Some(crate::app::updater::now_unix());
        }
        if events.contains(&Event::Shown) || (self.state.is_shown() && web_pointer::locked()) {
            // let go of the mouse so the buttons can be clicked; a lock the browser grants late is released again
            set_cursor_grab(false);
            show_mouse(true);
        }
        for event in events {
            if event == Event::Reload {
                web_update::reload();
            }
        }
        if !self.state.is_shown() {
            return;
        }
        let checked = crate::app::updater::checked_line(self.found_at);
        match draw(a, self.rules, &self.state, &checked) {
            Choice::UpdateNow => {
                if let Some(Event::Reload) = self.state.update_now() {
                    web_update::reload();
                }
            }
            Choice::Later => {
                self.state.later();
            }
            Choice::None => {}
        }
    }
}

/// Draws the screen over whatever is behind it and returns the player's choice. `checked` is the "LAST CHECKED" line
/// (local zone, 12-hour clock), drawn under the notice.
pub fn draw(a: &Assets, rules: &Rules, screen: &UpdateScreen, checked: &str) -> Choice {
    let _scope = MenuScope::enter();
    let p = pal();
    let v = view();
    fill(Rect::new(0.0, 0.0, v.x, v.y), rgba(0.0, 0.0, 0.0, rules.scrim_alpha));

    let reloading = screen.seconds_shown().is_none();
    let later = screen.can_postpone();
    let heading = menu_size(13.0, 0.88);
    let line = if reloading {
        rules.reloading_line.clone()
    } else {
        countdown_text(rules, screen.seconds_shown().unwrap_or(0))
    };
    let note = if later { &rules.run_warning } else { &rules.idle_note };

    let btn_size = menu_size(7.0, 1.0);
    let btn_h = button_min(a, "X", 7.0, false).y.max(button_min(a, "X", btn_size, false).y);
    let now_w = button_min(a, &rules.update_now_label, btn_size, false).x;
    let later_w = button_min(a, &rules.later_label, 7.0, false).x;

    // the panel is as wide as its widest line (the notice wraps inside it) and never narrower than the dialogs
    let kicker = track(&rules.kicker);
    let inner_w = tw(a, F::Title, &rules.title, heading)
        .max(tw(a, F::Body, &kicker, 6.0))
        .max(tw(a, F::Body, &countdown_text(rules, 88), 9.0))
        .max(tw(a, F::Body, checked, 6.0))
        .max(now_w + later_w + 5.0)
        .max(196.0);
    let note_lines = wrap(a, F::Body, note, inner_w, 6.0);
    let kicker_h = lh(F::Body, 6.0);
    let head_h = lh(F::Title, heading);
    let line_h = lh(F::Body, 9.0);
    let note_h = (note_lines.len() + 1) as f32 * lh(F::Body, 6.0);
    let bar_h = 3.0;
    let h = 24.0 + kicker_h + 5.0 + head_h + 8.0 + line_h + 5.0 + bar_h + 6.0 + note_h + 10.0 + btn_h;
    let panel = centered(inner_w + 24.0, h);
    sbox(panel, hex(0x0c080e), p.accent_edge, 1.0, p.corner);
    let inner = Rect::new(panel.x + 12.0, panel.y + 12.0, panel.w - 24.0, panel.h - 24.0);

    let mut y = inner.y;
    label(a, F::Body, &kicker, inner.x, y, 6.0, p.amber);
    y += kicker_h + 5.0;
    label(a, F::Title, &rules.title, inner.x, y, heading, p.ink);
    y += head_h + 8.0;
    label(a, F::Body, &line, inner.x, y, 9.0, p.ink);
    y += line_h + 5.0;
    // the bar runs down with the countdown (full while the page restarts)
    let left = if reloading { 0.0 } else { screen.progress() };
    fill(Rect::new(inner.x, y, inner.w, bar_h), p.inset);
    if left > 0.0 {
        fill(Rect::new(inner.x, y, inner.w * left, bar_h), p.amber);
    }
    y += bar_h + 6.0;
    for l in &note_lines {
        label(a, F::Body, l, inner.x, y, 6.0, p.muted);
        y += lh(F::Body, 6.0);
    }
    label(a, F::Body, checked, inner.x, y, 6.0, p.muted);

    let mut choice = Choice::None;
    // Enter always means UPDATE NOW, so that button is drawn as the focused one
    let by = inner.y + inner.h - btn_h;
    let now_rect = Rect::new(inner.x + inner.w - now_w, by, now_w, btn_h);
    let busy = reloading;
    let now_state = if busy { disabled() } else { state(true) };
    if button(a, now_rect, &rules.update_now_label, btn_size, Kind::Primary, now_state) {
        choice = Choice::UpdateNow;
    }
    if later {
        let later_rect = Rect::new(now_rect.x - 5.0 - later_w, by, later_w, btn_h);
        if button(a, later_rect, &rules.later_label, 7.0, Kind::Secondary, plain()) {
            choice = Choice::Later;
        }
    }
    if !busy && (is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter)) {
        choice = Choice::UpdateNow;
    }
    choice
}


#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = include_str!("../data/ui/update_screen.json");

    #[test]
    fn the_shipped_file_parses_with_the_agreed_numbers() {
        let r = rules();
        assert_eq!(r.timing.countdown, 10.0);
        assert_eq!(r.timing.postpone, 120.0);
        assert_eq!(r.title, "UPDATING // NEW VERSION READY");
        assert_eq!(r.update_now_label, "UPDATE NOW");
        assert_eq!(r.later_label, "LATER");
        assert_eq!(countdown_text(r, 7), "THE GAME RELOADS IN 7");
        assert!(r.scrim_alpha > 0.0 && r.scrim_alpha <= 1.0);
    }

    #[test]
    fn the_loader_refuses_a_broken_file() {
        assert!(parse(GOOD).is_ok());
        let bad = |from: &str, to: &str| {
            assert!(GOOD.contains(from), "the shipped file has {from}");
            assert!(parse(&GOOD.replace(from, to)).is_err(), "{from} -> {to} must be refused");
        };
        bad("lr.update_screen/1", "lr.update_screen/2");
        bad("\"countdown_seconds\": 10.0", "\"countdown_seconds\": 0.0");
        bad("\"countdown_seconds\": 10.0", "\"countdown_seconds\": 600.0");
        bad("\"countdown_seconds\": 10.0", "\"countdown_seconds\": \"ten\"");
        bad("\"postpone_seconds\": 120.0", "\"postpone_seconds\": 1.0");
        bad("\"max_step_seconds\": 0.5", "\"max_step_seconds\": 0.0");
        bad("\"scrim_alpha\": 0.82", "\"scrim_alpha\": 1.5");
        bad("THE GAME RELOADS IN {n}", "THE GAME RELOADS SOON");
        bad("THE GAME RELOADS IN {n}", "{n} {n}");
        bad("\"title\": \"UPDATING // NEW VERSION READY\"", "\"title\": \"\"");
        bad("\"later_label\": \"LATER\"", "\"later_label\": 3");
        assert!(parse("not json").is_err());
        assert!(parse("{}").is_err());
    }

    #[test]
    fn the_overlay_starts_hidden_and_is_built_from_the_shipped_rules() {
        let o = Overlay::new();
        assert!(!o.is_shown());
        assert_eq!(o.rules.timing, rules().timing);
        assert_eq!(o.found_at, None, "nothing found yet");
    }

    #[test]
    fn the_found_time_reads_in_the_local_zone_on_a_twelve_hour_clock() {
        let line = crate::app::updater::checked_line(Some(1_762_634_520));
        assert!(line.starts_with("LAST CHECKED: NOV "), "{line}");
        assert!(line.contains(" AM ") || line.contains(" PM "), "{line}");
        assert!(!line.contains("UTC") || line.ends_with(" UTC"), "a zone name, never an ISO stamp: {line}");
    }
}
