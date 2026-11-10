//! Account (real sign in), settings, credits and the controls screen with key rebinding.
#![allow(dead_code)]

use super::*;
use crate::core::ACTIONS;
use crate::net::account::{AccountEvent, TEXT_NOTE, TEXT_PLACEHOLDER_PASSWORD, TEXT_PLACEHOLDER_USER, TEXT_SIGNED_OUT, TEXT_WORKING};

const SCARE_SETTINGS: [&str; 3] = ["full", "short", "off"];
const DEFAULT_SENS: f32 = 0.0075;
const SENS_MIN: f32 = 0.002;
const SENS_MAX: f32 = 0.02;
const CONTROL_ROWS: usize = 20;

const CREDITS: [(&str, &str); 4] = [
    ("", "KYTTU - Night Watch is an unofficial fan project, made for fun and not affiliated with or endorsed by anyone below."),
    (
        "INSPIRED BY",
        "The Five Nights at Freddy's series by Scott Cawthon, including its sequels, Sister Location, Pizzeria Simulator and the Help Wanted games, and the community that documented how they work.",
    ),
    ("MADE BY", "Kyttu's Lab. Characters are from The Last Route. Office art, menus and code are original to this project."),
    ("SOUND", "All sound is synthesized in the game for now."),
];

/// "short" -> "Short".
pub fn capitalize(s: &str) -> String {
    let mut ch = s.chars();
    match ch.next() {
        Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
        None => String::new(),
    }
}

/// The scare setting after `cur` (full, short, off, back to full; unknown starts over).
pub fn next_scare(cur: &str) -> &'static str {
    let i = SCARE_SETTINGS.iter().position(|s| *s == cur).map(|i| i + 1).unwrap_or(0);
    SCARE_SETTINGS[i % SCARE_SETTINGS.len()]
}

/// First row to show so that `sel` is visible in a window of `rows` rows over `n`.
pub fn keep_visible(sel: usize, scroll: usize, rows: usize, n: usize) -> usize {
    let max = n.saturating_sub(rows);
    let mut s = scroll.min(max);
    if sel < s {
        s = sel;
    } else if sel >= s + rows {
        s = sel + 1 - rows;
    }
    s.min(max)
}

impl Shell {
    // --- account (real: the lr_server account API, see crate::net::account)

    /// Once per frame, on every screen: restore the stored session on the first frame, and apply a finished request
    /// to the save (the token is stored with the local save, never the password).
    pub(super) fn account_tick(&mut self, save: &mut Save) {
        if !self.acct_init {
            self.acct_init = true;
            if !save.session_token.is_empty() {
                self.acct.restore(&save.session_token);
            }
        }
        if let Some(ev) = self.acct.poll() {
            match &ev {
                AccountEvent::SignedIn { account, .. } => {
                    save.account_name = account.username.clone();
                    save.session_token = account.session_token.clone();
                    save.store();
                    self.acct_pass.clear();
                }
                AccountEvent::Rejected => {
                    save.account_name.clear();
                    save.session_token.clear();
                    save.store();
                }
                AccountEvent::Failed { .. } => {}
            }
            self.acct_status = ev.status_text();
        }
    }

    /// What the title shows under ACCOUNT: the signed-in name, else GUEST.
    pub(super) fn account_label() -> String {
        match crate::net::account::current() {
            Some(a) if !a.username.is_empty() => a.username,
            _ => "GUEST".to_string(),
        }
    }

    pub(super) fn account(&mut self, c: &mut Ctx<'_>) -> ShellAction {
        let act = self.header(c, "ACCOUNT", "");
        panel(Rect::new(8.0, 26.0, 464.0, 150.0));
        let signed = crate::net::account::current();
        let reason = self.acct.unavailable_reason();
        let busy = self.acct.busy();
        let lh = line_height(6.0);
        let mut y = 32.0;
        let intro = match &signed {
            Some(a) => format!("Signed in as {}.", a.username),
            None => format!("Playing as a guest ({}). Sign in or create an account and your guest progress carries over.", c.save.guest),
        };
        for l in wrap(&intro, 450.0, 6.0) {
            text(&l, 13.0, y, 6.0, BODY);
            y += lh;
        }
        y += 4.0;

        if let Some(a) = signed {
            if a.admin {
                text("Admin tools are on for this account.", 13.0, y, 6.0, AMBER);
                y += lh;
            }
            text("Your progress is kept on this device.", 13.0, y, 6.0, BODY);
            y += lh + 6.0;
            if c.ui.button(Rect::new(13.0, y, 70.0, 14.0), "SIGN OUT", 6.0, ButtonStyle::Normal, !busy).clicked {
                self.acct.sign_out(&a.session_token);
                c.save.account_name.clear();
                c.save.session_token.clear();
                c.save.store();
                self.acct_status = TEXT_SIGNED_OUT.to_string();
                c.cue("pick");
            }
            y += 20.0;
        } else {
            // the form
            let user_rect = Rect::new(90.0, y - 2.0, 150.0, 12.0);
            text("Username", 13.0, y, 7.0, INK);
            if c.ui.region(user_rect, true).clicked {
                self.acct_focus = 0;
            }
            c.ui.text_field(user_rect, &mut self.acct_user, false, self.acct_focus == 0);
            if self.acct_user.is_empty() && self.acct_focus != 0 {
                text(TEXT_PLACEHOLDER_USER, user_rect.x + 4.0, user_rect.y + 3.0, 6.0, DIM);
            }
            y += 16.0;
            let pass_rect = Rect::new(90.0, y - 2.0, 150.0, 12.0);
            text("Password", 13.0, y, 7.0, INK);
            if c.ui.region(pass_rect, true).clicked {
                self.acct_focus = 1;
            }
            c.ui.text_field(pass_rect, &mut self.acct_pass, true, self.acct_focus == 1);
            if self.acct_pass.is_empty() && self.acct_focus != 1 {
                text(TEXT_PLACEHOLDER_PASSWORD, pass_rect.x + 4.0, pass_rect.y + 3.0, 6.0, DIM);
            }
            y += 14.0;
            if is_key_pressed(KeyCode::Tab) {
                self.acct_focus = 1 - self.acct_focus.min(1);
            }
            text(TEXT_NOTE, 13.0, y, 6.0, BODY);
            y += lh + 4.0;

            let can = !busy && reason.is_none();
            let enter = is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter);
            let mut submit: Option<bool> = None; // Some(create)
            let sign_in = c.ui.button(Rect::new(13.0, y, 60.0, 14.0), "SIGN IN", 6.0, ButtonStyle::Primary, can);
            let create = c.ui.button(Rect::new(81.0, y, 90.0, 14.0), "CREATE ACCOUNT", 6.0, ButtonStyle::Normal, can);
            let guest = c.ui.button(Rect::new(179.0, y, 100.0, 14.0), "PLAY AS GUEST", 6.0, ButtonStyle::Normal, true);
            if sign_in.denied || create.denied {
                c.cue("deny");
            }
            if sign_in.clicked || (enter && can) {
                submit = Some(false);
            } else if create.clicked {
                submit = Some(true);
            }
            if guest.clicked {
                self.toast = "Playing as a guest.".to_string();
                self.acct_pass.clear();
                c.cue("pick");
                self.go(Screen::Title);
                self.toast = "Playing as a guest.".to_string();
                return act;
            }
            if let Some(create) = submit {
                match crate::net::account::validate(&self.acct_user, &self.acct_pass) {
                    Err(why) => {
                        self.acct_status = why.to_string();
                        c.cue("deny");
                    }
                    Ok((name, pass)) => {
                        self.acct_status = TEXT_WORKING.to_string();
                        if create {
                            self.acct.create_account(&name, &pass);
                        } else {
                            self.acct.sign_in(&name, &pass);
                        }
                        c.cue("pick");
                    }
                }
            }
            y += 20.0;
        }

        // status line: the unavailable reason, the working note, or the last result
        let status = if let Some(r) = &reason {
            r.clone()
        } else if busy && self.acct_status.is_empty() {
            TEXT_WORKING.to_string()
        } else {
            self.acct_status.clone()
        };
        for l in wrap(&status, 450.0, 6.0) {
            text(&l, 13.0, y, 6.0, AMBER);
            y += lh;
        }
        text("Guest progress stays with this browser or computer.", 13.0, 166.0, 6.0, DIM);
        act
    }

    // --- settings

    pub(super) fn settings(&mut self, c: &mut Ctx<'_>) -> ShellAction {
        let act = self.header(c, "SETTINGS", "");
        panel(Rect::new(8.0, 28.0, 464.0, 150.0));
        let row = |i: usize| 36.0 + i as f32 * 18.0;
        let lx = 14.0;
        let cx = 314.0;

        // volume
        let y = row(0);
        text("Volume", lx, y + 2.0, 7.0, INK);
        let v = c.ui.slider_step(Rect::new(cx, y + 1.0, 130.0, 10.0), c.save.master_volume, 0.0, 100.0, 1.0);
        if v != c.save.master_volume {
            c.save.master_volume = v;
            self.settings_dirty = true;
        }
        text(&format!("{}", v as i32), 450.0, y + 2.0, 7.0, INK);

        // mouse sensitivity
        let y = row(1);
        text("Mouse sensitivity", lx, y + 2.0, 7.0, INK);
        let s = c.ui.slider_step(Rect::new(cx, y + 1.0, 130.0, 10.0), c.save.mouse_sens, SENS_MIN, SENS_MAX, 0.0005);
        if (s - c.save.mouse_sens).abs() > 1e-6 {
            c.save.mouse_sens = s;
            self.settings_dirty = true;
        }
        text(&format!("{:.2}x", s / DEFAULT_SENS), 450.0, y + 2.0, 7.0, INK);

        // subtitles
        let y = row(2);
        text("Subtitles (Zellia's calls)", lx, y + 2.0, 7.0, INK);
        let on = c.ui.checkbox(Rect::new(cx, y, 40.0, 12.0), c.save.subtitles);
        if on != c.save.subtitles {
            c.save.subtitles = on;
            c.save.store();
            c.cue("pick");
        }

        // calm
        let y = row(3);
        text("Calm mode (no lamp flicker or glitching)", lx, y + 2.0, 7.0, INK);
        let on = c.ui.checkbox(Rect::new(cx, y, 40.0, 12.0), c.save.calm);
        if on != c.save.calm {
            c.save.calm = on;
            c.save.store();
            c.cue("pick");
        }

        // scare
        let y = row(4);
        text("Scare (Short: a quick one · Off: just the card)", lx, y + 2.0, 7.0, INK);
        let label = capitalize(&c.save.scare);
        if c.ui.button(Rect::new(cx, y - 1.0, 60.0, 13.0), &label, 6.0, ButtonStyle::Normal, true).clicked {
            c.save.scare = next_scare(&c.save.scare).to_string();
            c.save.store();
            c.cue("pick");
        }

        // controls
        let y = row(5);
        text("Keyboard controls", lx, y + 2.0, 7.0, INK);
        if c.ui.button(Rect::new(cx, y - 1.0, 60.0, 13.0), "REBIND", 6.0, ButtonStyle::Normal, true).clicked {
            c.cue("pick");
            self.go(Screen::Controls);
            return act;
        }

        // erase
        let y = row(6);
        text("Erase progress and settings (this device)", lx, y + 2.0, 7.0, INK);
        let label = if self.erase_done {
            "ERASED"
        } else if self.erase_armed {
            "SURE?"
        } else {
            "ERASE"
        };
        if c.ui.button(Rect::new(cx, y - 1.0, 60.0, 13.0), label, 6.0, ButtonStyle::Normal, true).clicked {
            if !self.erase_armed {
                self.erase_armed = true;
                self.erase_done = false;
                c.cue("pick");
            } else {
                c.save.reset();
                *c.bindings = Bindings::from_save(c.save);
                c.save.store();
                self.erase_armed = false;
                self.erase_done = true;
                self.custom_loaded.clear();
                self.custom_vals.clear();
                self.settings_dirty = false;
                self.toast.clear();
                c.cue("pick");
            }
        }

        if self.settings_dirty && !c.input.click_down {
            c.save.store();
            self.settings_dirty = false;
        }
        act
    }

    // --- credits

    pub(super) fn credits(&mut self, c: &mut Ctx<'_>) -> ShellAction {
        let act = self.header(c, "CREDITS", "");
        panel(Rect::new(8.0, 26.0, 464.0, 170.0));
        let mut y = 32.0;
        for (head, body) in CREDITS.iter() {
            if !head.is_empty() {
                text(head, 13.0, y, 7.0, AMBER);
                y += line_height(7.0) + 1.0;
            }
            for l in wrap(body, 450.0, 6.0) {
                text(&l, 13.0, y, 6.0, INK);
                y += line_height(6.0);
            }
            y += 4.0;
        }
        act
    }

    // --- controls (rebinding)

    pub(super) fn controls(&mut self, c: &mut Ctx<'_>) -> ShellAction {
        let act = self.header(c, "CONTROLS", "");
        let n = ACTIONS.len();
        if n == 0 {
            return act;
        }
        self.ctl_sel = self.ctl_sel.min(n - 1);

        let reset = c.ui.button(Rect::new(396.0, 6.0, 76.0, 14.0), "RESET ALL", 6.0, ButtonStyle::Normal, true);
        if reset.clicked {
            c.bindings.clear(c.save);
            c.save.store();
            self.ctl_capture = false;
            self.toast = "Controls reset to the defaults.".to_string();
            c.cue("pick");
        }

        if self.ctl_capture {
            // Escape was taken by the caller (it cancels); any other key is the new binding
            if let Some(key) = c.keys.last_key {
                let def = &ACTIONS[self.ctl_sel];
                c.bindings.rebind(c.save, def.name, key);
                c.save.store();
                self.toast = format!("{}: {}", def.label, c.bindings.key_text(def.name));
                self.ctl_capture = false;
                c.cue("pick");
            }
        } else {
            if c.keys.down {
                self.ctl_sel = wrap_index(self.ctl_sel, 1, n);
                c.cue("move");
            }
            if c.keys.up {
                self.ctl_sel = wrap_index(self.ctl_sel, -1, n);
                c.cue("move");
            }
            if c.keys.enter {
                self.ctl_capture = true;
                self.toast = "Press a key (Esc cancels).".to_string();
            }
            if c.keys.wheel != 0.0 {
                let max = n.saturating_sub(CONTROL_ROWS);
                self.ctl_scroll = if c.keys.wheel > 0.0 { self.ctl_scroll.saturating_sub(3) } else { (self.ctl_scroll + 3).min(max) };
            }
        }
        self.ctl_scroll = keep_visible(self.ctl_sel, self.ctl_scroll, CONTROL_ROWS, n);

        for r in 0..CONTROL_ROWS {
            let idx = self.ctl_scroll + r;
            if idx >= n {
                break;
            }
            let def = &ACTIONS[idx];
            let y = 26.0 + r as f32 * 11.0;
            let rect = Rect::new(10.0, y, 452.0, 10.0);
            let res = c.ui.region(rect, true);
            if res.clicked {
                self.ctl_sel = idx;
                self.ctl_capture = true;
                self.toast = "Press a key (Esc cancels).".to_string();
                c.cue("pick");
            }
            let selected = idx == self.ctl_sel;
            if selected {
                boxed(rect, FOCUS_WASH, AMBER, 1.0);
            } else if res.hovered {
                fill_rect(rect, BUTTON_HOVER);
            }
            text(def.label, 14.0, y + 2.0, 6.0, if selected { AMBER } else { INK });
            if selected && self.ctl_capture {
                text_right("PRESS A KEY...", 458.0, y + 2.0, 6.0, AMBER);
            } else {
                text_right(&c.bindings.key_text(def.name), 458.0, y + 2.0, 6.0, if selected { INK } else { BODY });
            }
        }

        if n > CONTROL_ROWS {
            let track = Rect::new(466.0, 26.0, 3.0, CONTROL_ROWS as f32 * 11.0);
            fill_rect(track, EDGE);
            let thick = track.h * CONTROL_ROWS as f32 / n as f32;
            let top = track.y + (track.h - thick) * self.ctl_scroll as f32 / (n - CONTROL_ROWS) as f32;
            fill_rect(Rect::new(track.x, top, track.w, thick), AMBER);
        }
        act
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scare_cycles_full_short_off() {
        assert_eq!(next_scare("full"), "short");
        assert_eq!(next_scare("short"), "off");
        assert_eq!(next_scare("off"), "full");
        assert_eq!(next_scare("weird"), "full");
    }

    #[test]
    fn capitalize_words() {
        assert_eq!(capitalize("short"), "Short");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn selection_stays_in_the_visible_window() {
        assert_eq!(keep_visible(0, 0, 20, 50), 0);
        assert_eq!(keep_visible(25, 0, 20, 50), 6);
        assert_eq!(keep_visible(3, 6, 20, 50), 3);
        assert_eq!(keep_visible(49, 0, 20, 50), 30);
        assert_eq!(keep_visible(5, 99, 20, 10), 0);
    }

    #[test]
    fn credits_text_is_the_godot_text() {
        assert_eq!(CREDITS.len(), 4);
        assert_eq!(CREDITS[3], ("SOUND", "All sound is synthesized in the game for now."));
    }
}
