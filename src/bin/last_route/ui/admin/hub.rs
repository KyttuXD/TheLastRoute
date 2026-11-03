//! The title / hub ADMIN panel (hub_admin_panel.gd): "ADMIN // STATUS" with REFRESH and CLOSE,
//! then centred sections: THIS BUILD, ACCOUNT, SERVER, UPDATES, PUBLIC ROUTE, SERVER NOTICES,
//! DEBUG TOGGLES (the admin preferences the run's admin menu applies, as ON/OFF toggles), SKINS,
//! MODERATION and ROUTE TOOLS. The SERVER and UPDATES rows show the answer of one status request,
//! made on open and on REFRESH; the other network sections read as the original does when the
//! server cannot be reached; skins and moderation are owner-only.

use super::look::*;
use super::prefs::{Prefs, KEYS};
use crate::app::assets::Assets;
use crate::app::updater::PLATFORM;
use crate::net::api::{self, UpdateManifest};
use crate::net::config::PRIMARY_BASE_URL;
use crate::net::Pending;
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
    request: Option<Pending<UpdateManifest>>,
    link: ServerLink,
}

impl Default for HubPanel {
    fn default() -> Self {
        HubPanel::new()
    }
}

impl HubPanel {
    /// Opening the panel reads the current admin preferences and asks the server for its status.
    pub fn new() -> HubPanel {
        let mut hub = HubPanel {
            prefs: Prefs::load(),
            scroll: 0.0,
            content_h: 0.0,
            checking: 0.0,
            request: None,
            link: ServerLink::Checking,
        };
        hub.request_status();
        hub
    }

    /// One status request for this platform's update manifest. A request still running is dropped.
    fn request_status(&mut self) {
        self.link = ServerLink::Checking;
        self.request = Some(Pending::spawn(|| api::update_manifest(PLATFORM)));
    }

    fn section(&self, title: &str, username: &str) -> String {
        match title {
            "THIS BUILD" => format!(
                "{} // BUILD RUST PORT (#0) // CHANNEL LOCAL\n{} DESKTOP // MACROQUAD // HALF-RES PIXEL VIEW",
                crate::ui::menus::VERSION,
                std::env::consts::OS.to_uppercase()
            ),
            "ACCOUNT" => format!("{} // ADMIN // SCOPES NONE", if username.is_empty() { "NOT SIGNED IN".to_string() } else { username.to_uppercase() }),
            "SERVER" => server_text(&self.link, PRIMARY_BASE_URL),
            "UPDATES" => updates_text(&self.link, env!("LR_BUILD_NUMBER").parse().unwrap_or(0), env!("LR_REVISION")),
            "PUBLIC ROUTE" => "NO ANSWER".into(),
            "SERVER NOTICES" => "NONE THIS SESSION".into(),
            "DEBUG TOGGLES" => "Applied in the next run (the run's admin menu changes them live).".into(),
            "SKINS" | "MODERATION" => "OWNERS ONLY".into(),
            _ => "In a run: press 0 (or the right stick click) for the admin menu: items, hunters, map reveal, god mode,\nroom chat mute / kick, FULLBRIGHT (F6).".into(),
        }
    }
}

/// What the status request found: still running, no answer from the server, or the server's manifest.
#[derive(Clone, Debug, PartialEq)]
enum ServerLink {
    Checking,
    Unreachable,
    Reached(UpdateManifest),
}

/// The SERVER row. A server that answered is ONLINE, whether or not it publishes a build.
fn server_text(link: &ServerLink, base_url: &str) -> String {
    match link {
        ServerLink::Checking => "CHECKING...".into(),
        ServerLink::Unreachable => "OFFLINE (SOLO) // CHANNEL LOCAL\nCAN'T REACH THE SERVER".into(),
        ServerLink::Reached(manifest) => {
            format!("ONLINE // CHANNEL {}\n{}", channel_text(&manifest.channel), host_name(base_url).to_uppercase())
        }
    }
}

/// The UPDATES row: this platform's published build against this build. A build made without CI (build 0)
/// never updates, so a newer published build is not announced for it.
fn updates_text(link: &ServerLink, build: i64, revision: &str) -> String {
    match link {
        ServerLink::Checking => "CHECKING...".into(),
        ServerLink::Unreachable => "CAN'T REACH THE SERVER\nOTHERS: SEE THE SITE".into(),
        ServerLink::Reached(manifest) => match &manifest.update {
            None => "THIS PLATFORM: NONE PUBLISHED\nOTHERS: SEE THE SITE".into(),
            Some(published) => {
                let status = if same_commit(&published.revision, revision) {
                    "UP TO DATE"
                } else if build > 0 && published.build > build {
                    "UPDATE ON NEXT START"
                } else {
                    "NOT THE PUBLISHED BUILD"
                };
                format!(
                    "THIS PLATFORM: BUILD {} ({})\nTHIS BUILD: {build} ({}) // {status}\nOTHERS: SEE THE SITE",
                    published.build,
                    short_revision(&published.revision),
                    short_revision(revision)
                )
            }
        },
    }
}

fn channel_text(channel: &str) -> String {
    if channel.is_empty() {
        "DEBUG".into()
    } else {
        channel.to_uppercase()
    }
}

/// `http://host:port` -> `host`.
fn host_name(base_url: &str) -> &str {
    let authority = base_url.split_once("://").map_or(base_url, |(_, rest)| rest);
    authority.split([':', '/']).next().unwrap_or(authority)
}

fn short_revision(revision: &str) -> String {
    revision.chars().take(7).collect()
}

/// `updater::same_commit`: a short hash of the same commit counts as the same commit.
fn same_commit(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
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
    let answered = hub.request.as_mut().and_then(|p| p.poll());
    if let Some(answer) = answered {
        hub.request = None;
        hub.link = match answer {
            Ok(manifest) => ServerLink::Reached(manifest),
            Err(_) => ServerLink::Unreachable,
        };
    }
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
    if ui.button(refresh, "REFRESH", 8.0, false, hub.checking > 0.0) {
        hub.checking = 0.6;
        hub.request_status();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::api::PublishedBuild;

    fn published(build: i64, revision: &str) -> ServerLink {
        ServerLink::Reached(UpdateManifest { channel: "debug".into(), update: Some(PublishedBuild { build, revision: revision.into() }) })
    }

    #[test]
    fn server_row_online_offline_and_checking() {
        let answered = ServerLink::Reached(UpdateManifest { channel: "debug".into(), update: None });
        assert_eq!(server_text(&answered, PRIMARY_BASE_URL), "ONLINE // CHANNEL DEBUG\nLASTROUTE.KYTTUSLAB.ORG");
        assert_eq!(server_text(&ServerLink::Unreachable, PRIMARY_BASE_URL), "OFFLINE (SOLO) // CHANNEL LOCAL\nCAN'T REACH THE SERVER");
        assert_eq!(server_text(&ServerLink::Checking, PRIMARY_BASE_URL), "CHECKING...");
    }

    #[test]
    fn server_row_without_a_channel_says_debug() {
        let answered = ServerLink::Reached(UpdateManifest::default());
        assert_eq!(server_text(&answered, "http://127.0.0.1:18105"), "ONLINE // CHANNEL DEBUG\n127.0.0.1");
    }

    #[test]
    fn updates_row_for_a_newer_published_build() {
        assert_eq!(
            updates_text(&published(11, "0123456789abcdef"), 10, "fedcba9876543210"),
            "THIS PLATFORM: BUILD 11 (0123456)\nTHIS BUILD: 10 (fedcba9) // UPDATE ON NEXT START\nOTHERS: SEE THE SITE"
        );
        assert_eq!(
            updates_text(&published(11, "0123456789abcdef"), 0, "unknown"),
            "THIS PLATFORM: BUILD 11 (0123456)\nTHIS BUILD: 0 (unknown) // NOT THE PUBLISHED BUILD\nOTHERS: SEE THE SITE"
        );
    }

    #[test]
    fn updates_row_for_the_same_commit() {
        assert_eq!(
            updates_text(&published(11, "0123456789abcdef"), 11, "0123456"),
            "THIS PLATFORM: BUILD 11 (0123456)\nTHIS BUILD: 11 (0123456) // UP TO DATE\nOTHERS: SEE THE SITE"
        );
    }

    #[test]
    fn updates_row_without_a_published_build_or_an_answer() {
        let answered = ServerLink::Reached(UpdateManifest::default());
        assert_eq!(updates_text(&answered, 11, "abc1234"), "THIS PLATFORM: NONE PUBLISHED\nOTHERS: SEE THE SITE");
        assert_eq!(updates_text(&ServerLink::Unreachable, 11, "abc1234"), "CAN'T REACH THE SERVER\nOTHERS: SEE THE SITE");
    }

    #[test]
    fn host_name_drops_scheme_and_port() {
        assert_eq!(host_name(PRIMARY_BASE_URL), "lastroute.kyttuslab.org");
        assert_eq!(host_name("http://127.0.0.1"), "127.0.0.1");
    }
}
