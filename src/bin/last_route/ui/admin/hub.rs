//! The title / hub ADMIN panel (hub_admin_panel.gd): "ADMIN // STATUS" with REFRESH and CLOSE,
//! then centred sections: THIS BUILD, ACCOUNT, SERVER, UPDATES, PUBLIC ROUTE, SERVER NOTICES,
//! DEBUG TOGGLES (the admin preferences the run's admin menu applies, as ON/OFF toggles), SKINS,
//! MODERATION and ROUTE TOOLS. This build has no server, so the network sections read as the
//! original does when the server cannot be reached; skins and moderation are owner-only.

use super::look::*;
use super::prefs::{Prefs, KEYS};
use crate::app::assets::Assets;
use macroquad::prelude::*;

const SECTIONS: [&str; 10] = [
    "THIS BUILD",
    "ACCOUNT",
    "SERVER",
    "UPDATES",
    "PUBLIC ROUTE",
    "SERVER NOTICES",
    "DEBUG TOGGLES",
    "SKINS",
    "MODERATION",
    "ROUTE TOOLS",
];

pub struct HubPanel {
    prefs: Prefs,
    scroll: f32,
    content_h: f32,
    checking: f32,
}

impl Default for HubPanel {
    fn default() -> Self {
        HubPanel::new()
    }
}

impl HubPanel {
    /// Opening the panel reads the current admin preferences.
    pub fn new() -> HubPanel {
        HubPanel {
            prefs: Prefs::load(),
            scroll: 0.0,
            content_h: 0.0,
            checking: 0.0,
        }
    }

    fn section(&self, title: &str, username: &str) -> String {
        match title {
            "THIS BUILD" => format!(
                "{} // BUILD RUST PORT (#0) // CHANNEL LOCAL\n{} DESKTOP // MACROQUAD // HALF-RES PIXEL VIEW",
                crate::ui::menus::VERSION,
                std::env::consts::OS.to_uppercase()
            ),
            "ACCOUNT" => format!("{} // ADMIN // SCOPES NONE", if username.is_empty() { "NOT SIGNED IN".to_string() } else { username.to_uppercase() }),
            "SERVER" => {
                if self.checking > 0.0 {
                    "SERVER OFFLINE (SOLO, NO SERVER) // CHANNEL LOCAL\nCHECKING...".into()
                } else {
                    "SERVER OFFLINE (SOLO, NO SERVER) // CHANNEL LOCAL\nCAN'T REACH THE SERVER".into()
                }
            }
            "UPDATES" => "MACOS: NONE PUBLISHED\nWINDOWS: NONE PUBLISHED\nPHONE: NONE PUBLISHED\nQUEST: NONE PUBLISHED".into(),
            "PUBLIC ROUTE" => "NO ANSWER".into(),
            "SERVER NOTICES" => "NONE THIS SESSION".into(),
            "DEBUG TOGGLES" => "Applied in the next run (the run's admin menu changes them live).".into(),
            "SKINS" | "MODERATION" => "OWNERS ONLY".into(),
            _ => "In a run: press 0 (or the right stick click) for the admin menu: items, hunters, map reveal, god mode,\nroom chat mute / kick, FULLBRIGHT (F6).".into(),
        }
    }
}

fn wrap(ui: &Ui, text: &str, size: f32, width: f32) -> Vec<String> {
    let mut out = vec![];
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let trial = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && ui.tw(&trial, size) > width {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            } else {
                line = trial;
            }
        }
        out.push(line);
    }
    out
}

/// Draws the panel over whatever is behind it (the title screen). Returns true when it closed
/// (CLOSE, or Esc / ui_cancel).
pub fn draw_hub(a: &Assets, hub: &mut HubPanel, username: &str) -> bool {
    let (sw, sh) = (screen_width(), screen_height());
    let u = px_per_unit();
    hub.checking = (hub.checking - get_frame_time()).max(0.0);
    let mut ui = Ui::new(a, u, is_mouse_button_pressed(MouseButton::Left));
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.78));
    let panel = Rect::new(16.0 * u, 12.0 * u, sw - 32.0 * u, sh - 24.0 * u);
    ui.boxed(panel, PANEL, AMBER);
    let inner = Rect::new(
        panel.x + 8.0 * u,
        panel.y + 8.0 * u,
        panel.w - 16.0 * u,
        panel.h - 16.0 * u,
    );

    // head: centred title, REFRESH, CLOSE (compact secondary buttons, size 8)
    let head_h = 16.0 * u;
    let cw = ui.tw("CLOSE", 8.0) + 10.0 * u;
    let rw = ui.tw("REFRESH", 8.0) + 10.0 * u;
    let close = Rect::new(inner.x + inner.w - cw, inner.y, cw, head_h);
    let refresh = Rect::new(close.x - 4.0 * u - rw, inner.y, rw, head_h);
    let title_box = Rect::new(inner.x, inner.y, refresh.x - 4.0 * u - inner.x, head_h);
    ui.text_center("ADMIN // STATUS", title_box, 10.0, AMBER);
    if ui.button(refresh, "REFRESH", 8.0, false, false) {
        hub.checking = 0.6;
    }
    let closed = ui.button(close, "CLOSE", 8.0, false, false) || is_key_pressed(KeyCode::Escape);

    // the scrolling column
    let body = Rect::new(
        inner.x,
        inner.y + head_h + 4.0 * u,
        inner.w,
        inner.h - head_h - 4.0 * u,
    );
    let wheel = mouse_wheel().1;
    if wheel != 0.0 && body.contains(ui.mouse) {
        hub.scroll -= wheel.signum() * 20.0 * u;
    }
    hub.scroll = hub.scroll.clamp(0.0, (hub.content_h - body.h).max(0.0));
    ui.clip = Some(body);
    unsafe {
        get_internal_gl().quad_gl.scissor(Some((
            body.x as i32,
            body.y as i32,
            body.w.ceil() as i32,
            body.h.ceil() as i32,
        )));
    }
    let top = body.y - hub.scroll;
    let mut y = top;
    let row = (line_h(7.0) + 3.0) * u;
    for title in SECTIONS {
        let w = ui.tracked_w(title, 7.0);
        if ui.visible(Rect::new(body.x, y, body.w, row)) {
            ui.tracked(title, body.x + (body.w - w) / 2.0, y, 7.0, AMBER);
        }
        y += row;
        for line in wrap(&ui, &hub.section(title, username), 7.0, body.w) {
            let lw = ui.tw(&line, 7.0);
            if ui.visible(Rect::new(body.x, y, body.w, row)) {
                ui.text(&line, body.x + (body.w - lw) / 2.0, y, 7.0, BODY);
            }
            y += row;
        }
        if title == "DEBUG TOGGLES" {
            // GridContainer, 4 columns of state toggles
            let gw = (body.w - 3.0 * 4.0 * u) / 4.0;
            let gh = 15.0 * u;
            for (i, (key, _)) in KEYS.iter().enumerate() {
                let (c, r) = (i % 4, i / 4);
                let rect = Rect::new(
                    body.x + c as f32 * (gw + 4.0 * u),
                    y + r as f32 * (gh + 4.0 * u),
                    gw,
                    gh,
                );
                let on = hub.prefs.get(key);
                let label = key.replace('_', " ").to_uppercase();
                if ui.toggle_row(rect, &label, on, false) {
                    hub.prefs.set(key, !on);
                }
            }
            y += ((KEYS.len() + 3) / 4) as f32 * (gh + 4.0 * u) - 4.0 * u + 3.0 * u;
        }
        if title == "SKINS" {
            // the (owner-only, hidden) remove row's status line keeps its slot
            y += (line_h(6.0) + 3.0) * u;
        }
    }
    hub.content_h = y - top;
    unsafe {
        get_internal_gl().quad_gl.scissor(None);
    }
    if closed {
        hub.scroll = 0.0;
    }
    closed
}
