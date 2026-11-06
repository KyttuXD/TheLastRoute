//! RESET PASSWORD: the FIELDS tab's action for full admin owners (KYTTU, AYTTU). The owner types an account name; the
//! game calls `POST /api/admin/account/reset-password` (`net::api::admin_reset_password`) and shows the temporary
//! password the server generated once, in a message box with a COPY button. The server signs that account out
//! everywhere. Nothing here stores or logs the password: it lives in the message box until CLOSE (or Esc), and `Debug`
//! of the answer hides it.
//!
//! The field and its button exist only while the signed-in account is a full admin owner (`Admin::owner`, which the app
//! sets from the server's `adminOwner` flag); the server checks the session again and refuses anyone else.

use super::look::*;
use super::menu::wrap;
use super::Admin;
use crate::app::assets::Assets;
use crate::net::api::{self, PasswordReset};
use crate::net::{BackendConfig, NetError, Pending};
use macroquad::prelude::*;

/// The index of the RESET PASSWORD line edit in `data/admin/menu.json` "fields". It is the last one, so hiding it leaves
/// the others where they were.
pub(super) const RESET_FIELD: usize = 3;

/// The message box: what happened, and the one-time password while there is one.
pub(super) struct Notice {
    pub heading: String,
    pub lines: Vec<String>,
    pub password: Option<String>,
    /// COPY was pressed (the button then reads COPIED).
    pub copied: bool,
}

#[derive(Default)]
pub(super) struct ResetState {
    /// The account the field asked for, not yet sent (the app sends it with `Admin::run_password_reset`).
    pub request: Option<String>,
    /// The request in flight, with the name it was made for.
    pub pending: Option<(String, Pending<PasswordReset>)>,
    pub notice: Option<Notice>,
}

/// The canonical account name for what was typed: trimmed, UPPERCASE, 3 to 24 letters, digits or underscores.
pub(super) fn canonical_username(text: &str) -> Option<String> {
    let name = text.trim().to_uppercase();
    ((3..=24).contains(&name.chars().count()) && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .then_some(name)
}

/// What a refused or failed reset says, in the menu's voice. `asked` is the account that was asked for.
pub(super) fn failure_text(error: &NetError, asked: &str) -> String {
    match error {
        NetError::Offline => "OFFLINE // PASSWORD RESET NEEDS THE SERVER".to_string(),
        NetError::Refused(text) => text.to_uppercase(),
        NetError::Http { status, code, error, .. } => match (*status, code.as_str()) {
            (401, _) => "YOUR SESSION ENDED // SIGN IN AGAIN".to_string(),
            (403, _) => "ONLY A FULL ADMIN ACCOUNT CAN RESET A PASSWORD".to_string(),
            (404, "ACCOUNT_NOT_FOUND") => format!("NO ACCOUNT NAMED {asked}"),
            (404, _) => "THIS SERVER CANNOT RESET PASSWORDS YET".to_string(),
            (400, "USE_PASSWORD_CHANGE") => "THAT IS YOUR OWN ACCOUNT // CHANGE YOUR PASSWORD IN THE ACCOUNT PANEL".to_string(),
            (400, "INVALID_USERNAME") => "THAT IS NOT A VALID ACCOUNT NAME".to_string(),
            (429, _) => "TOO MANY REQUESTS // TRY AGAIN IN A MINUTE".to_string(),
            _ if error.is_empty() => format!("THE SERVER ANSWERED {status}"),
            _ => error.to_uppercase(),
        },
        NetError::Timeout => "THE SERVER DID NOT ANSWER IN TIME".to_string(),
        _ => "COULD NOT REACH THE SERVER".to_string(),
    }
}

impl Notice {
    pub(super) fn failure(error: &NetError, asked: &str) -> Notice {
        Notice { heading: "RESET FAILED".into(), lines: vec![failure_text(error, asked)], password: None, copied: false }
    }

    pub(super) fn success(reset: &PasswordReset, asked: &str) -> Notice {
        let who = if reset.username.is_empty() { asked } else { reset.username.as_str() };
        let signed_out = match reset.sessions_revoked {
            0 => "THEY HAD NO OPEN SESSIONS.".to_string(),
            1 => "THEIR 1 OPEN SESSION WAS SIGNED OUT.".to_string(),
            n => format!("THEIR {n} OPEN SESSIONS WERE SIGNED OUT."),
        };
        let mut lines = Vec::new();
        match &reset.temporary_password {
            Some(_) => {
                lines.push(format!("TEMPORARY PASSWORD FOR {who} // SHOWN ONCE:"));
                lines.push(String::new());
                lines.push(format!("{signed_out} TELL THEM TO CHANGE IT AFTER SIGNING IN."));
            }
            None => lines.push(format!("{who} HAS A NEW PASSWORD. {signed_out}")),
        }
        Notice { heading: "PASSWORD RESET".into(), lines, password: reset.temporary_password.clone(), copied: false }
    }
}

impl Admin {
    /// Whether the signed-in account is a full admin owner as far as this menu knows (the app sets `owner` from the
    /// server's `adminOwner`; the admin gate itself must also be on).
    pub(super) fn is_owner(&self) -> bool {
        self.enabled && self.owner
    }

    /// A line edit of the FIELDS tab is shown: all of them, except RESET PASSWORD for anyone but an owner.
    pub(super) fn field_visible(&self, index: usize) -> bool {
        index != RESET_FIELD || self.is_owner()
    }

    /// How many line edits the FIELDS tab shows (RESET PASSWORD is the last one).
    pub(super) fn field_count(&self) -> usize {
        super::menu_data::menu().fields.len() - usize::from(!self.is_owner())
    }

    /// True while a reset has been asked for or is waiting on the server.
    pub(super) fn reset_busy(&self) -> bool {
        self.reset.request.is_some() || self.reset.pending.is_some()
    }

    /// The RESET PASSWORD field was run with `text`. Queues the request for [`Admin::run_password_reset`] and returns the
    /// line for the game's message strip.
    pub(super) fn ask_password_reset(&mut self, text: &str) -> String {
        if !self.is_owner() {
            return "ADMIN // RESET PASSWORD NEEDS A FULL ADMIN ACCOUNT".into();
        }
        let Some(name) = canonical_username(text) else {
            return "ADMIN // TYPE THE ACCOUNT NAME TO RESET (3-24 LETTERS, NUMBERS OR _)".into();
        };
        if self.reset_busy() || self.reset.notice.is_some() {
            return "ADMIN // A PASSWORD RESET IS ALREADY RUNNING".into();
        }
        let line = format!("ADMIN // RESETTING THE PASSWORD OF {name}...");
        self.reset.request = Some(name);
        self.fields[RESET_FIELD].clear();
        self.field_focus = None;
        line
    }

    /// Sends the queued reset with the signed-in owner's session. The app calls this once per frame; it does nothing
    /// unless the field asked for a reset.
    pub fn run_password_reset(&mut self, config: &BackendConfig, session_token: &str) {
        if self.reset.pending.is_some() {
            return;
        }
        let Some(name) = self.reset.request.take() else { return };
        let (cfg, token, who) = (config.clone(), session_token.to_string(), name.clone());
        let pending = if token.is_empty() {
            Pending::done(Err(NetError::Refused("Sign in with a full admin account first".into())))
        } else {
            Pending::spawn(move || api::admin_reset_password(&cfg, &token, &who, None))
        };
        self.reset.pending = Some((name, pending));
    }

    /// Takes in the answer when it has arrived and opens the message box on the FIELDS tab.
    pub(super) fn poll_password_reset(&mut self) {
        let Some((_, pending)) = self.reset.pending.as_mut() else { return };
        let Some(result) = pending.poll() else { return };
        let (name, _) = self.reset.pending.take().expect("a pending reset was just polled");
        self.reset.notice = Some(match result {
            Ok(reset) => {
                crate::lr_info!("admin: password reset for {}", if reset.username.is_empty() { &name } else { &reset.username });
                Notice::success(&reset, &name)
            }
            Err(error) => {
                crate::lr_info!("admin: password reset for {name} failed");
                Notice::failure(&error, &name)
            }
        });
        // the message box must be seen and clickable: open the menu on FIELDS
        self.set_open(true);
        self.tab = 9;
    }

    /// CLOSE / Esc: the box and the password in it are gone.
    pub(super) fn close_notice(&mut self) {
        self.reset.notice = None;
    }

    /// The message box over the menu: dimmer, the heading, the lines, the password in a boxed row, COPY and CLOSE.
    pub(super) fn draw_notice(&mut self, a: &Assets) {
        if self.reset.notice.is_none() {
            return;
        }
        let (sw, sh) = (screen_width(), screen_height());
        let ppu = px_per_unit();
        let zoom = (1.6 / ppu).clamp(1.0, 2.4);
        let u = ppu * zoom;
        let mut ui = Ui::new(a, u, is_mouse_button_pressed(MouseButton::Left));
        let Some(notice) = self.reset.notice.as_mut() else { return };

        let width = ((sw / u) - 24.0).clamp(160.0, 300.0) * u;
        let inner_w = width - 20.0 * u;
        let mut lines: Vec<(String, bool)> = Vec::new();
        for text in &notice.lines {
            if text.is_empty() {
                lines.push((String::new(), true));
                continue;
            }
            for line in wrap(&ui, text, 7.0, inner_w) {
                lines.push((line, false));
            }
        }
        let line_step = (line_h(7.0) + 2.0) * u;
        let pass_h = if notice.password.is_some() { 24.0 * u } else { 0.0 };
        let mut body_h = 0.0;
        for (_, is_gap) in &lines {
            body_h += if *is_gap { pass_h.max(line_step) } else { line_step };
        }
        let height = 10.0 * u + 14.0 * u + body_h + 6.0 * u + 17.0 * u + 10.0 * u;
        let panel = Rect::new((sw - width) / 2.0, ((sh - height) / 2.0).max(4.0 * u), width, height);

        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.7));
        ui.boxed(panel, PANEL, AMBER);
        let x = panel.x + 10.0 * u;
        let mut y = panel.y + 10.0 * u;
        let heading_col = if notice.password.is_some() { STABLE } else if notice.heading == "RESET FAILED" { ALERT } else { AMBER };
        ui.tracked(&notice.heading, x, y, 8.0, heading_col);
        y += 14.0 * u;
        for (line, is_gap) in &lines {
            if *is_gap {
                if let Some(password) = &notice.password {
                    let r = Rect::new(x, y, inner_w, pass_h - 4.0 * u);
                    ui.boxed(r, INSET, FIELD_EDGE);
                    ui.text_center(password, r, 11.0, INK);
                }
                y += pass_h.max(line_step);
            } else {
                ui.text(line, x, y, 7.0, BODY);
                y += line_step;
            }
        }
        y += 6.0 * u;
        let close_w = ui.tw("CLOSE", 7.0) + 20.0 * u;
        let close = Rect::new(x + inner_w - close_w, y, close_w, 17.0 * u);
        let mut shut = ui.button(close, "CLOSE", 7.0, notice.password.is_none(), false);
        if let Some(password) = notice.password.clone() {
            let label = if notice.copied { "COPIED" } else { "COPY" };
            let copy_w = ui.tw("COPIED", 7.0) + 20.0 * u;
            let copy = Rect::new(close.x - 6.0 * u - copy_w, y, copy_w, 17.0 * u);
            if ui.button(copy, label, 7.0, true, false) {
                macroquad::miniquad::window::clipboard_set(&password);
                notice.copied = true;
            }
        }
        shut |= is_key_pressed(KeyCode::Enter) && notice.password.is_none();
        if shut {
            self.close_notice();
        }
    }
}
