//! The profile screen (docs/PROFILES.md): the list of profiles with when each was last used, NEW PROFILE, DELETE
//! (with a confirm step) and PROFILE IN USE. It is shown before the game loads anything, in its own short process
//! (app/instance.rs), so it needs only the window and a system font: it draws with macroquad directly and never
//! touches `Assets`. Picking a profile starts the game again with `--profile NAME`.
//!
//! Every string and number is data/ui/profiles.json (`lr.profiles_ui/1`), checked by [`Labels::parse`]. The rules
//! (names, index, trash, lock) are shared/profiles.rs; this file only draws and forwards.

use super::style::{hex, METER_LOW, REF_ACCENT, REF_ACCENT_EDGE, REF_BODY, REF_INK, REF_INSET, REF_ON_AMBER, REF_PANEL, REF_PANEL_EDGE};
use crate::app::instance::{self, Picker};
use crate::shared::profiles::{self, Index, LockState, ProfileError};
use macroquad::prelude::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const FILE: &str = "ui/profiles.json";
const BACKGROUND: Color = hex(0x0d080f);

/// data/ui/profiles.json.
#[derive(Clone, Debug, PartialEq)]
pub struct Labels {
    /// A lock not rewritten for this long is stale.
    pub stale_after_seconds: u64,
    /// How often a running game rewrites its lock.
    pub heartbeat_seconds: u64,
    /// How long after the title first shows a held SHIFT opens this screen.
    pub shift_hold_seconds: f32,
    /// How often this screen rereads the index and the locks.
    pub lock_refresh_seconds: f32,
    /// The title's button.
    pub switch_label: String,
    pub kicker: String,
    pub title: String,
    /// Contains `{time}`.
    pub last_used: String,
    pub never_used: String,
    pub in_use_tag: String,
    pub start_label: String,
    pub new_label: String,
    pub delete_label: String,
    pub quit_label: String,
    pub hints: Vec<String>,
    pub new_title: String,
    pub new_hint: String,
    pub create_label: String,
    pub cancel_label: String,
    /// Contains `{name}`.
    pub delete_title: String,
    pub delete_note: String,
    pub delete_yes_label: String,
    pub delete_no_label: String,
    pub delete_in_use: String,
    pub in_use_title: String,
    /// Contains `{name}`.
    pub in_use_note: String,
    pub continue_label: String,
    pub back_label: String,
    pub restart_failed: String,
    errors: BTreeMap<String, String>,
}

/// Every refusal the rules can give; each needs a line in `errors`.
const ERROR_KEYS: [&str; 8] = ["name_empty", "name_long", "name_chars", "name_reserved", "exists", "missing", "default", "io"];

impl Labels {
    pub fn parse(text: &str) -> Result<Labels, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|x| !x.trim().is_empty())
                .ok_or_else(|| format!("{k} must be a non-empty string"))
        };
        let with = |k: &str, token: &str| -> Result<String, String> {
            let text = s(k)?;
            if text.contains(token) {
                Ok(text)
            } else {
                Err(format!("{k} must contain {token}"))
            }
        };
        let whole = |k: &str| {
            v.get(k)
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
                .ok_or_else(|| format!("{k} must be a whole number above 0"))
        };
        let seconds = |k: &str| {
            v.get(k)
                .and_then(Value::as_f64)
                .filter(|n| *n > 0.0 && n.is_finite())
                .map(|n| n as f32)
                .ok_or_else(|| format!("{k} must be a number above 0"))
        };
        let hints: Vec<String> = v
            .get("hints")
            .and_then(Value::as_array)
            .ok_or("hints must be a list of strings")?
            .iter()
            .filter_map(|h| h.as_str().map(str::to_string))
            .collect();
        let mut errors = BTreeMap::new();
        for key in ERROR_KEYS {
            let line = v
                .get("errors")
                .and_then(|e| e.get(key))
                .and_then(Value::as_str)
                .filter(|x| !x.trim().is_empty())
                .ok_or_else(|| format!("errors.{key} must be a non-empty string"))?;
            errors.insert(key.to_string(), line.to_string());
        }
        let labels = Labels {
            stale_after_seconds: whole("stale_after_seconds")?,
            heartbeat_seconds: whole("heartbeat_seconds")?,
            shift_hold_seconds: seconds("shift_hold_seconds")?,
            lock_refresh_seconds: seconds("lock_refresh_seconds")?,
            switch_label: s("switch_label")?,
            kicker: s("kicker")?,
            title: s("title")?,
            last_used: with("last_used", "{time}")?,
            never_used: s("never_used")?,
            in_use_tag: s("in_use_tag")?,
            start_label: s("start_label")?,
            new_label: s("new_label")?,
            delete_label: s("delete_label")?,
            quit_label: s("quit_label")?,
            hints,
            new_title: s("new_title")?,
            new_hint: s("new_hint")?,
            create_label: s("create_label")?,
            cancel_label: s("cancel_label")?,
            delete_title: with("delete_title", "{name}")?,
            delete_note: s("delete_note")?,
            delete_yes_label: s("delete_yes_label")?,
            delete_no_label: s("delete_no_label")?,
            delete_in_use: s("delete_in_use")?,
            in_use_title: s("in_use_title")?,
            in_use_note: with("in_use_note", "{name}")?,
            continue_label: s("continue_label")?,
            back_label: s("back_label")?,
            restart_failed: s("restart_failed")?,
            errors,
        };
        if labels.stale_after_seconds <= labels.heartbeat_seconds {
            return Err("stale_after_seconds must be above heartbeat_seconds".into());
        }
        Ok(labels)
    }

    /// The line for a refusal.
    pub fn error(&self, e: &ProfileError) -> &str {
        self.errors.get(e.key()).map(String::as_str).unwrap_or_default()
    }
}

/// The embedded labels (data/ui/profiles.json), parsed once.
pub fn labels() -> &'static Labels {
    static LABELS: OnceLock<Labels> = OnceLock::new();
    LABELS.get_or_init(|| {
        let text = crate::content::embedded::data_under("")
            .find(|(p, _)| *p == FILE)
            .map(|(_, t)| t)
            .expect("data/ui/profiles.json is embedded by build.rs");
        Labels::parse(text).expect("data/ui/profiles.json is valid")
    })
}

/// The first row shown, so that row `focus` is among the `visible` rows of a list scrolled to `top`.
pub fn scroll_top(focus: usize, top: usize, visible: usize, len: usize) -> usize {
    let visible = visible.max(1);
    let top = top.min(len.saturating_sub(visible));
    if focus < top {
        focus
    } else if focus >= top + visible {
        focus + 1 - visible
    } else {
        top
    }
}

/// What the screen is showing.
enum Mode {
    List,
    /// NEW PROFILE: the name typed so far.
    New(String),
    /// DELETE <name>?
    Delete,
    /// PROFILE IN USE: pick another or continue anyway.
    InUse,
}

/// What the player asked for this frame.
enum Cmd {
    Start,
    Continue,
    New,
    Create(String),
    AskDelete,
    Delete,
    Back,
    Quit,
}

/// Text and boxes at the window's scale.
struct Pen<'a> {
    font: Option<&'a Font>,
    /// Window pixels per design pixel (the design is 540 high).
    s: f32,
    mouse: Vec2,
    click: bool,
}

impl Pen<'_> {
    fn px(&self, size: f32) -> u16 {
        (size * self.s).round().max(1.0) as u16
    }

    fn width(&self, text: &str, size: f32) -> f32 {
        measure_text(text, self.font, self.px(size), 1.0).width
    }

    fn text(&self, text: &str, x: f32, top: f32, size: f32, col: Color) {
        let px = self.px(size);
        let params = TextParams { font: self.font, font_size: px, color: col, ..Default::default() };
        draw_text_ex(text, x.round(), (top + f32::from(px) * 0.8).round(), params);
    }

    fn centred(&self, text: &str, cx: f32, top: f32, size: f32, col: Color) {
        self.text(text, cx - self.width(text, size) * 0.5, top, size, col);
    }

    /// `text` in lines no wider than `width`.
    fn wrap(&self, text: &str, width: f32, size: f32) -> Vec<String> {
        let mut lines = vec![];
        let mut line = String::new();
        for word in text.split_whitespace() {
            let longer = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if !line.is_empty() && self.width(&longer, size) > width {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = longer;
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    fn hovered(&self, r: Rect) -> bool {
        r.contains(self.mouse)
    }

    /// A button; true when it was clicked this frame.
    fn button(&self, r: Rect, text: &str, primary: bool) -> bool {
        let hover = self.hovered(r);
        let (face, edge, ink) = if primary {
            (if hover { REF_ACCENT_EDGE } else { REF_ACCENT }, REF_ACCENT_EDGE, REF_ON_AMBER)
        } else {
            (if hover { REF_PANEL_EDGE } else { REF_INSET }, REF_PANEL_EDGE, REF_INK)
        };
        draw_rectangle(r.x, r.y, r.w, r.h, face);
        draw_rectangle_lines(r.x, r.y, r.w, r.h, (2.0 * self.s).max(1.0), edge);
        let size = 12.0;
        let px = f32::from(self.px(size));
        self.centred(text, r.x + r.w * 0.5, r.y + (r.h - px) * 0.5, size, ink);
        hover && self.click
    }
}

fn enter_pressed() -> bool {
    is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter)
}

/// Starts the game on `name`. True when the new process started (this one then ends).
fn launch(name: &str, force: bool) -> bool {
    if force {
        instance::restart(&["--profile", name, "--profile-force"])
    } else {
        instance::restart(&["--profile", name])
    }
}

/// The profile screen. Returns when the player quits, or when the game was started again on the picked profile.
pub async fn run(start: Picker) {
    let l = labels();
    let font = crate::shared::assets::system_font(&[
        "courbd.ttf",
        "Courier New Bold.ttf",
        "Courier_New_Bold.ttf",
        "LiberationMono-Bold.ttf",
    ]);
    let root = start.root.clone();
    let pid = std::process::id();
    let mut index = Index::load(&root);
    let preferred = start.in_use.clone().unwrap_or_else(|| index.last.clone());
    let mut focus = index.profiles.iter().position(|p| p.name == preferred).unwrap_or(0);
    let mut top = 0usize;
    let mut mode = Mode::List;
    let mut notice = match &start.in_use {
        Some(name) => l.in_use_note.replace("{name}", name),
        None => String::new(),
    };
    // which profiles another instance holds, reread every `lock_refresh_seconds`
    let mut held: Vec<bool> = vec![];
    let mut held_at = f64::NEG_INFINITY;

    loop {
        crate::app::gfx::boot_guard_tick();
        let clock = get_time();
        if clock - held_at >= f64::from(l.lock_refresh_seconds) || held.len() != index.profiles.len() {
            if matches!(mode, Mode::List) {
                // another instance may have added or deleted a profile
                let fresh = Index::load(&root);
                if fresh != index {
                    let name = index.profiles.get(focus).map(|p| p.name.clone()).unwrap_or_default();
                    focus = fresh.profiles.iter().position(|p| p.name == name).unwrap_or(0);
                    index = fresh;
                }
            }
            let now = instance::unix_now();
            held = index
                .profiles
                .iter()
                .map(|p| {
                    let dir = profiles::profile_dir(&root, &p.name);
                    matches!(profiles::lock_state(&dir, pid, now, l.stale_after_seconds), LockState::InUse(_))
                })
                .collect();
            held_at = clock;
        }
        focus = focus.min(index.profiles.len().saturating_sub(1));
        let focus_name = index.profiles.get(focus).map(|p| p.name.clone()).unwrap_or_default();
        let focus_held = held.get(focus).copied().unwrap_or(false);

        clear_background(BACKGROUND);
        let (sw, sh) = (screen_width(), screen_height());
        let (mx, my) = mouse_position();
        let pen = Pen {
            font: font.as_ref(),
            s: (sh / 540.0).clamp(0.8, 2.5),
            mouse: vec2(mx, my),
            click: is_mouse_button_pressed(MouseButton::Left),
        };
        let s = pen.s;
        let w = (sw - 32.0).min(560.0 * s);
        let x0 = ((sw - w) * 0.5).round();
        let pad = 16.0 * s;
        let row_h = 30.0 * s;
        let button_h = 30.0 * s;
        let gap = 8.0 * s;
        let mut y = 28.0 * s;
        pen.text(&l.kicker, x0, y, 11.0, REF_ACCENT);
        y += 18.0 * s;
        pen.text(&l.title, x0, y, 22.0, REF_INK);
        y += 34.0 * s;
        if !notice.is_empty() {
            for line in pen.wrap(&notice, w, 11.0) {
                pen.text(&line, x0, y, 11.0, METER_LOW);
                y += 15.0 * s;
            }
            y += 6.0 * s;
        }

        let mut cmd: Option<Cmd> = None;
        let deleting = matches!(mode, Mode::Delete);
        match &mut mode {
            Mode::List => {
                // the list, scrolled so the focused row shows
                let below = button_h + gap * 2.0 + 16.0 * s * l.hints.len() as f32 + pad;
                let visible = (((sh - y - below) / row_h).floor().max(1.0) as usize).min(index.profiles.len().max(1));
                top = scroll_top(focus, top, visible, index.profiles.len());
                let list = Rect::new(x0, y, w, row_h * visible as f32);
                draw_rectangle(list.x, list.y, list.w, list.h, REF_PANEL);
                for (i, p) in index.profiles.iter().enumerate().skip(top).take(visible) {
                    let r = Rect::new(x0, y + row_h * (i - top) as f32, w, row_h);
                    let focused = i == focus;
                    if focused {
                        draw_rectangle(r.x, r.y, r.w, r.h, REF_INSET);
                        draw_rectangle(r.x, r.y, 3.0 * s, r.h, REF_ACCENT);
                    } else if pen.hovered(r) {
                        draw_rectangle(r.x, r.y, r.w, r.h, REF_PANEL_EDGE);
                    }
                    let text_top = r.y + (row_h - 14.0 * s) * 0.5;
                    pen.text(&p.name, r.x + pad, text_top, 14.0, if focused { REF_INK } else { REF_BODY });
                    let in_use = held.get(i).copied().unwrap_or(false);
                    let (side, col) = if in_use {
                        (l.in_use_tag.clone(), METER_LOW)
                    } else if p.last_used == 0 {
                        (l.never_used.clone(), REF_BODY)
                    } else {
                        let unix = i64::try_from(p.last_used).unwrap_or(i64::MAX);
                        let time = crate::app::updater::time_rules().format(unix, crate::app::updater::time_zone());
                        (l.last_used.replace("{time}", &time), REF_BODY)
                    };
                    let side_top = r.y + (row_h - 11.0 * s) * 0.5;
                    pen.text(&side, r.x + r.w - pad - pen.width(&side, 11.0), side_top, 11.0, col);
                    if pen.click && pen.hovered(r) {
                        if focused {
                            cmd = Some(Cmd::Start);
                        } else {
                            focus = i;
                        }
                    }
                }
                draw_rectangle_lines(list.x, list.y, list.w, list.h, (2.0 * s).max(1.0), REF_PANEL_EDGE);
                y += list.h + gap;
                // START, NEW PROFILE, DELETE, QUIT
                let bw = (w - gap * 3.0) / 4.0;
                let at = |n: f32| Rect::new(x0 + (bw + gap) * n, y, bw, button_h);
                if pen.button(at(0.0), &l.start_label, true) {
                    cmd = Some(Cmd::Start);
                }
                if pen.button(at(1.0), &l.new_label, false) {
                    cmd = Some(Cmd::New);
                }
                if pen.button(at(2.0), &l.delete_label, false) {
                    cmd = Some(Cmd::AskDelete);
                }
                if pen.button(at(3.0), &l.quit_label, false) {
                    cmd = Some(Cmd::Quit);
                }
                y += button_h + gap;
                for hint in &l.hints {
                    pen.text(hint, x0, y, 10.0, REF_BODY);
                    y += 16.0 * s;
                }
                if is_key_pressed(KeyCode::Up) && focus > 0 {
                    focus -= 1;
                }
                if is_key_pressed(KeyCode::Down) && focus + 1 < index.profiles.len() {
                    focus += 1;
                }
                if enter_pressed() {
                    cmd = Some(Cmd::Start);
                } else if is_key_pressed(KeyCode::N) {
                    cmd = Some(Cmd::New);
                } else if is_key_pressed(KeyCode::Delete) {
                    cmd = Some(Cmd::AskDelete);
                } else if is_key_pressed(KeyCode::Escape) {
                    cmd = Some(Cmd::Quit);
                }
            }
            Mode::New(typed) => {
                while let Some(c) = get_char_pressed() {
                    for u in c.to_uppercase() {
                        if profiles::name_char_ok(u) && typed.chars().count() < profiles::NAME_MAX {
                            typed.push(u);
                        }
                    }
                }
                if is_key_pressed(KeyCode::Backspace) {
                    typed.pop();
                }
                pen.text(&l.new_title, x0, y, 14.0, REF_INK);
                y += 24.0 * s;
                let field = Rect::new(x0, y, w, row_h);
                draw_rectangle(field.x, field.y, field.w, field.h, REF_INSET);
                draw_rectangle_lines(field.x, field.y, field.w, field.h, (2.0 * s).max(1.0), REF_ACCENT);
                // a caret that blinks twice a second
                let caret = if (clock * 2.0) as i64 % 2 == 0 { "_" } else { "" };
                pen.text(&format!("{typed}{caret}"), x0 + pad, y + (row_h - 14.0 * s) * 0.5, 14.0, REF_INK);
                y += row_h + gap;
                pen.text(&l.new_hint, x0, y, 10.0, REF_BODY);
                y += 18.0 * s;
                let bw = (w - gap) / 2.0;
                if pen.button(Rect::new(x0, y, bw, button_h), &l.create_label, true) || enter_pressed() {
                    cmd = Some(Cmd::Create(typed.clone()));
                }
                if pen.button(Rect::new(x0 + bw + gap, y, bw, button_h), &l.cancel_label, false)
                    || is_key_pressed(KeyCode::Escape)
                {
                    cmd = Some(Cmd::Back);
                }
            }
            Mode::Delete | Mode::InUse => {
                let (title, note, yes, no) = if deleting {
                    (l.delete_title.replace("{name}", &focus_name), l.delete_note.clone(), &l.delete_yes_label, &l.delete_no_label)
                } else {
                    (l.in_use_title.clone(), l.in_use_note.replace("{name}", &focus_name), &l.continue_label, &l.back_label)
                };
                pen.text(&title, x0, y, 14.0, REF_INK);
                y += 24.0 * s;
                for line in pen.wrap(&note, w, 11.0) {
                    pen.text(&line, x0, y, 11.0, REF_BODY);
                    y += 15.0 * s;
                }
                y += gap;
                let bw = (w - gap) / 2.0;
                // the safe answer is the primary button and the Escape key; Enter confirms
                if pen.button(Rect::new(x0, y, bw, button_h), yes, false) || enter_pressed() {
                    cmd = Some(if deleting { Cmd::Delete } else { Cmd::Continue });
                }
                if pen.button(Rect::new(x0 + bw + gap, y, bw, button_h), no, true) || is_key_pressed(KeyCode::Escape) {
                    cmd = Some(Cmd::Back);
                }
            }
        }

        match cmd {
            None => {}
            Some(Cmd::Quit) => return,
            Some(Cmd::Back) => mode = Mode::List,
            Some(Cmd::Start) => {
                if focus_held {
                    notice.clear();
                    mode = Mode::InUse;
                } else if launch(&focus_name, false) {
                    return;
                } else {
                    notice = l.restart_failed.clone();
                }
            }
            Some(Cmd::Continue) => {
                if launch(&focus_name, true) {
                    return;
                }
                notice = l.restart_failed.clone();
                mode = Mode::List;
            }
            Some(Cmd::New) => {
                // the N that opened the field is not part of the name
                while get_char_pressed().is_some() {}
                notice.clear();
                mode = Mode::New(String::new());
            }
            Some(Cmd::Create(typed)) => {
                let mut fresh = Index::load(&root);
                match fresh.add(&root, &typed, instance::unix_now()) {
                    Ok(name) => match fresh.save(&root) {
                        Ok(()) => {
                            focus = fresh.profiles.iter().position(|p| p.name == name).unwrap_or(0);
                            index = fresh;
                            held_at = f64::NEG_INFINITY;
                            notice.clear();
                            mode = Mode::List;
                        }
                        Err(e) => notice = l.error(&ProfileError::Io(e.to_string())).to_string(),
                    },
                    Err(e) => notice = l.error(&e).to_string(),
                }
            }
            Some(Cmd::AskDelete) => {
                if focus_name == profiles::DEFAULT_NAME {
                    notice = l.error(&ProfileError::Default).to_string();
                } else if focus_held {
                    notice = l.delete_in_use.clone();
                } else {
                    notice.clear();
                    mode = Mode::Delete;
                }
            }
            Some(Cmd::Delete) => {
                let mut fresh = Index::load(&root);
                match fresh.remove(&root, &focus_name, instance::unix_now()) {
                    Ok(moved) => {
                        if let Some(to) = moved {
                            crate::lr_info!("profile: {focus_name} moved to {}", to.display());
                        }
                        notice = match fresh.save(&root) {
                            Ok(()) => String::new(),
                            Err(e) => l.error(&ProfileError::Io(e.to_string())).to_string(),
                        };
                    }
                    Err(e) => notice = l.error(&e).to_string(),
                }
                index = fresh;
                held_at = f64::NEG_INFINITY;
                mode = Mode::List;
            }
        }
        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = include_str!("../data/ui/profiles.json");

    #[test]
    fn the_embedded_labels_parse_and_the_lock_ages_make_sense() {
        let l = labels();
        assert_eq!(Labels::parse(GOOD).as_ref(), Ok(l));
        assert!(l.stale_after_seconds > l.heartbeat_seconds);
        assert!(l.last_used.contains("{time}"));
        assert!(l.delete_title.contains("{name}") && l.in_use_note.contains("{name}"));
        assert!(!l.hints.is_empty());
    }

    #[test]
    fn every_refusal_has_a_line() {
        use crate::shared::profiles::NameError;
        let l = labels();
        let all = [
            ProfileError::Name(NameError::Empty),
            ProfileError::Name(NameError::TooLong),
            ProfileError::Name(NameError::BadChar),
            ProfileError::Name(NameError::Reserved),
            ProfileError::Exists,
            ProfileError::Missing,
            ProfileError::Default,
            ProfileError::Io("x".into()),
        ];
        assert_eq!(all.len(), ERROR_KEYS.len());
        for e in &all {
            assert!(ERROR_KEYS.contains(&e.key()), "{e:?}");
            assert!(!l.error(e).is_empty(), "{e:?}");
        }
    }

    #[test]
    fn broken_labels_are_refused() {
        let mut v: Value = serde_json::from_str(GOOD).unwrap();
        v["last_used"] = "LAST USED".into();
        assert!(Labels::parse(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(GOOD).unwrap();
        v["errors"].as_object_mut().unwrap().remove("exists");
        assert!(Labels::parse(&v.to_string()).is_err());
        let mut v: Value = serde_json::from_str(GOOD).unwrap();
        v["stale_after_seconds"] = 5.into();
        assert!(Labels::parse(&v.to_string()).is_err(), "a lock must outlive one heartbeat");
        let mut v: Value = serde_json::from_str(GOOD).unwrap();
        v["switch_label"] = " ".into();
        assert!(Labels::parse(&v.to_string()).is_err());
    }

    #[test]
    fn the_list_scrolls_to_keep_the_focused_row_in_view() {
        assert_eq!(scroll_top(0, 0, 5, 3), 0);
        assert_eq!(scroll_top(4, 0, 5, 10), 0);
        assert_eq!(scroll_top(5, 0, 5, 10), 1);
        assert_eq!(scroll_top(9, 0, 5, 10), 5);
        assert_eq!(scroll_top(2, 5, 5, 10), 2);
        assert_eq!(scroll_top(6, 5, 5, 10), 5);
        // a list that got shorter pulls the top back
        assert_eq!(scroll_top(1, 7, 5, 6), 1);
        assert_eq!(scroll_top(0, 3, 0, 2), 0);
    }
}
