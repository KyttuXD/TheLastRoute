//! The title / hub ADMIN panel (hub_admin_panel.gd): "ADMIN // STATUS" with REFRESH and CLOSE,
//! then centred sections: THIS BUILD, ACCOUNT, SERVER, UPDATES, PUBLIC ROUTE, SERVER NOTICES,
//! DEBUG TOGGLES (the admin preferences the run's admin menu applies, as ON/OFF toggles), SKINS,
//! MODERATION and ROUTE TOOLS. On open and on REFRESH, like `refresh_all()`, the panel asks the
//! server for `/api/health` (SERVER), `/api/updates/<channel>/<platform>` for every platform
//! (UPDATES, one line each) and `/api/public/status` (PUBLIC ROUTE); skins and moderation are
//! owner-only.
//!
//! PORT DEVIATION: Godot's `_fetch` turns every failure into `{}`, so a server that answers with
//! an error reads the same as one that cannot be reached. Here an HTTP 5xx is reported as the
//! server being reached but failing ("SERVER ERROR (HTTP 500)"), and only a connection failure or
//! timeout reads "CAN'T REACH THE SERVER" / "NO ANSWER". The SERVER section also says ONLINE when
//! the health check answers.

use super::look::*;
use super::prefs::{Prefs, KEYS};
use crate::app::assets::Assets;
use crate::net::api::{self, Json};
use crate::net::http::{Method, Request};
use crate::net::{BackendConfig, NetError, Pending};
use macroquad::prelude::*;
use serde_json::Value;
use std::time::Duration;

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

/// hub_admin_panel.gd `PLATFORMS` (macos, windows, phone, quest) plus linux, which the Rust port
/// also builds and the updater asks for (`updater::PLATFORM`).
const PLATFORMS: [&str; 5] = ["macos", "windows", "linux", "phone", "quest"];
/// `Build.update_channel()`: the port only ships debug builds (the updater asks `/api/updates/debug/..`).
const CHANNEL: &str = "debug";
/// `request.timeout = 6.0` in `_fetch`.
const FETCH_TIMEOUT: Duration = Duration::from_secs(6);
/// The health keys `refresh_all()` lists, in its order.
const HEALTH_KEYS: [&str; 7] = ["instance", "service", "rooms", "webRooms", "relay", "runtime", "now"];
/// `ServerPrivacy.FRIENDLY_UNREACHABLE`.
const UNREACHABLE: &str = "CAN'T REACH THE SERVER";

pub struct HubPanel {
    prefs: Prefs,
    scroll: f32,
    content_h: f32,
    checking: f32,
    config: BackendConfig,
    health: Probe,
    updates: Vec<(&'static str, Probe)>,
    public: Probe,
    requests: Vec<(Target, Pending<Probe>)>,
}

impl Default for HubPanel {
    fn default() -> Self {
        HubPanel::new(BackendConfig::primary())
    }
}

/// Which section a running request fills in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Health,
    Update(usize),
    Public,
}

impl HubPanel {
    /// Opening the panel reads the current admin preferences and asks the server for its status.
    /// Uses the same [`BackendConfig`] as the rest of the client (`--backend=` / primary).
    pub fn new(config: BackendConfig) -> HubPanel {
        let mut hub = HubPanel {
            prefs: Prefs::load(),
            scroll: 0.0,
            content_h: 0.0,
            checking: 0.0,
            config,
            health: Probe::Checking,
            updates: PLATFORMS.iter().map(|p| (*p, Probe::Checking)).collect(),
            public: Probe::Checking,
            requests: Vec::new(),
        };
        hub.refresh_all();
        hub
    }

    /// `refresh_all()`: health, every platform's manifest and the public route, all at once (the
    /// original awaits them one after another). Requests still running are dropped.
    fn refresh_all(&mut self) {
        self.health = Probe::Checking;
        self.public = Probe::Checking;
        for (_, probe) in &mut self.updates {
            *probe = Probe::Checking;
        }
        self.requests.clear();
        let mut targets = vec![(Target::Health, "/api/health".to_string())];
        for (i, p) in PLATFORMS.iter().enumerate() {
            let files = nightly_files(p);
            self.requests.push((Target::Update(i), Pending::spawn(move || Ok(fetch_nightly(files)))));
        }
        targets.push((Target::Public, "/api/public/status".to_string()));
        for (target, path) in targets {
            let config = self.config.clone();
            self.requests.push((target, Pending::spawn(move || Ok(fetch(&config, &path)))));
        }
    }

    /// Takes in every answer that arrived this frame.
    fn poll(&mut self) {
        let mut i = 0;
        while i < self.requests.len() {
            match self.requests[i].1.poll() {
                Some(answer) => {
                    let (target, _) = self.requests.remove(i);
                    let probe = answer.unwrap_or(Probe::Unreachable);
                    match target {
                        Target::Health => self.health = probe,
                        Target::Update(n) => self.updates[n].1 = probe,
                        Target::Public => self.public = probe,
                    }
                }
                None => i += 1,
            }
        }
    }

    fn section(&self, title: &str, account: Option<&api::AccountReply>) -> String {
        match title {
            "THIS BUILD" => this_build_text(),
            "ACCOUNT" => account_text(account),
            "SERVER" => server_text(&self.health),
            "UPDATES" => updates_text(&self.updates),
            "PUBLIC ROUTE" => public_text(&self.public),
            "SERVER NOTICES" => "NONE THIS SESSION".into(),
            "DEBUG TOGGLES" => "Applied in the next run (the run's admin menu changes them live).".into(),
            "SKINS" | "MODERATION" => "OWNERS ONLY".into(),
            _ => "In a run: press 0 (or the right stick click) for the admin menu: items, hunters, map reveal, god mode,\nroom chat mute / kick, FULLBRIGHT (F6).".into(),
        }
    }
}

/// THIS BUILD: the CI build number and commit (or LOCAL for a development build), and the platform.
fn this_build_text() -> String {
    format!("BUILD {}\n{} DESKTOP // MACROQUAD", crate::app::updater::build_label(), std::env::consts::OS.to_uppercase())
}

/// ACCOUNT: the signed-in name, the role the server reports, and the admin scopes it grants.
/// `None` means not signed in (the backend keeps the account answer only while signed in).
fn account_text(account: Option<&api::AccountReply>) -> String {
    let Some(a) = account else {
        return "NOT SIGNED IN // NO ADMIN SCOPES".into();
    };
    let role = if a.admin_owner {
        "OWNER"
    } else if a.admin {
        "ADMIN"
    } else {
        "PLAYER"
    };
    let scopes = if !a.has_admin_permissions {
        "NOT REPORTED BY SERVER".to_string()
    } else if a.admin_permissions.is_empty() {
        "NONE".to_string()
    } else {
        a.admin_permissions.join(" ").to_uppercase()
    };
    format!("{} // {role} // SCOPES {scopes}", a.username)
}

/// One status request's outcome: still running, no answer (could not connect / timed out), or an
/// HTTP answer with its status and JSON object body (`{}` when the body is not a JSON object).
#[derive(Clone, Debug, PartialEq)]
enum Probe {
    Checking,
    Unreachable,
    Answered { status: u16, body: Json },
}

impl Probe {
    fn from_result(result: Result<Json, NetError>) -> Probe {
        match result {
            Ok(body) => Probe::Answered { status: 200, body },
            Err(NetError::Http { status, body, .. }) => {
                Probe::Answered { status, body: if let Value::Object(map) = body { map } else { Json::new() } }
            }
            Err(_) => Probe::Unreachable,
        }
    }
}

/// `_fetch(path, {})`: a GET with the 6 s timeout.
fn fetch(config: &BackendConfig, path: &str) -> Probe {
    if config.offline {
        return Probe::Unreachable;
    }
    let sent = Request::new(Method::Get, config.url(path))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .timeout(FETCH_TIMEOUT)
        .send();
    Probe::from_result(sent.and_then(api::check))
}

/// The files of the public nightly release (GitHub, replaced by CI on every push to main) that each UPDATES line shows.
fn nightly_files(platform: &str) -> Vec<(&'static str, &'static str)> {
    match platform {
        "macos" => vec![("INTEL", "last_route-macos-x86_64.zip"), ("ARM", "last_route-macos-arm64.zip")],
        "windows" => vec![("EXE", "last_route-windows-x86_64.exe")],
        "linux" => vec![("TAR", "last-route-linux-x86_64.tar.gz")],
        _ => Vec::new(),
    }
}

/// One platform's UPDATES state from the nightly release on GitHub: each file with the time CI last published it.
/// A platform with no file in the release answers an empty body, which the line shows as NONE PUBLISHED.
fn fetch_nightly(files: Vec<(&'static str, &'static str)>) -> Probe {
    if files.is_empty() {
        return Probe::Answered { status: 200, body: Json::new() };
    }
    let sent = Request::new(Method::Get, "https://api.github.com/repos/KyttuXD/TheLastRoute/releases/tags/nightly")
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "last-route-admin")
        .timeout(FETCH_TIMEOUT)
        .send();
    let body = match sent.and_then(api::check) {
        Ok(body) => body,
        Err(e) => return Probe::from_result(Err(e)),
    };
    let assets = body.get("assets").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    let parts: Vec<String> = files
        .iter()
        .filter_map(|(label, name)| {
            let asset = assets.iter().find(|a| a.get("name").and_then(|n| n.as_str()) == Some(*name))?;
            let when = asset.get("updated_at").and_then(|t| t.as_str()).unwrap_or("").replace('T', " ");
            let when: String = when.chars().take(16).collect();
            Some(format!("{label} {when} UTC"))
        })
        .collect();
    let mut out = Json::new();
    if !parts.is_empty() {
        out.insert("nightly".into(), Value::String(parts.join(" / ")));
    }
    Probe::Answered { status: 200, body: out }
}

/// The update channel line; the server address is never shown.
fn debug_line() -> String {
    format!("CHANNEL {}", CHANNEL.to_uppercase())
}

/// "SERVER ERROR (HTTP 500)" for a 5xx, "HTTP 404" for any other non-2xx.
fn http_trouble(status: u16) -> String {
    if status >= 500 {
        format!("SERVER ERROR (HTTP {status})")
    } else {
        format!("HTTP {status}")
    }
}

/// The SERVER section: the debug line, then ONLINE and the health keys when `/api/health` answered,
/// the reached-but-failing status for an error answer, or CAN'T REACH THE SERVER.
fn server_text(health: &Probe) -> String {
    let mut lines = vec![debug_line()];
    match health {
        Probe::Checking => lines.push("CHECKING...".into()),
        Probe::Unreachable => lines.push(UNREACHABLE.into()),
        Probe::Answered { status, body } if (200..300).contains(status) => {
            lines.push("ONLINE".into());
            for key in HEALTH_KEYS {
                if let Some(value) = body.get(key) {
                    lines.push(format!("{}: {}", key.to_uppercase(), short(value)));
                }
            }
        }
        Probe::Answered { status, .. } => lines.push(format!("REACHED // {}", http_trouble(*status))),
    }
    lines.join("\n")
}

/// One UPDATES line: `PLATFORM: BUILD N @ REVISION` (8 characters), `NONE PUBLISHED` (the server
/// answered without an `update`, e.g. 404 NO UPDATE PUBLISHED), or the honest failure.
fn update_line(platform: &str, probe: &Probe) -> String {
    let state = match probe {
        Probe::Checking => "CHECKING...".to_string(),
        Probe::Unreachable => "NO ANSWER".to_string(),
        Probe::Answered { status, .. } if *status >= 500 => http_trouble(*status),
        Probe::Answered { body, .. } if body.contains_key("nightly") => body["nightly"].as_str().unwrap_or("").to_string(),
        Probe::Answered { body, .. } => match body.get("update") {
            Some(Value::Object(update)) if !update.is_empty() => {
                let build = update.get("build").and_then(|b| b.as_f64()).unwrap_or(0.0) as i64;
                let revision = match update.get("revision") {
                    Some(Value::String(r)) => r.chars().take(8).collect::<String>().to_uppercase(),
                    _ => String::new(),
                };
                format!("BUILD {build} @ {revision}")
            }
            _ => "NONE PUBLISHED".to_string(),
        },
    };
    format!("{}: {state}", platform.to_uppercase())
}

fn updates_text(updates: &[(&str, Probe)]) -> String {
    updates.iter().map(|(platform, probe)| update_line(platform, probe)).collect::<Vec<_>>().join("\n")
}

/// The PUBLIC ROUTE section: `_short(/api/public/status)`, or NO ANSWER.
fn public_text(public: &Probe) -> String {
    match public {
        Probe::Checking => "CHECKING...".into(),
        Probe::Answered { status, body } if (200..300).contains(status) && !body.is_empty() => short_map(body),
        Probe::Answered { status, .. } if *status >= 500 => format!("NO ANSWER // {}", http_trouble(*status)),
        _ => "NO ANSWER".into(),
    }
}

/// `_short(value)`: an object as `key value, key value` (nested objects and arrays as their size,
/// keys naming a token, secret or password left out), anything else as text.
fn short(value: &Value) -> String {
    match value {
        Value::Object(map) => short_map(map),
        Value::String(s) => s.clone(),
        Value::Null => "<null>".into(),
        Value::Array(items) => format!("[{}]", items.iter().map(short).collect::<Vec<_>>().join(", ")),
        other => other.to_string(),
    }
}

fn short_map(map: &Json) -> String {
    let mut parts = vec![];
    for (key, value) in map {
        let lower = key.to_lowercase();
        if lower.contains("token") || lower.contains("secret") || lower.contains("password") {
            continue; // never secrets
        }
        let shown = match value {
            Value::Object(o) => o.len().to_string(),
            Value::Array(a) => a.len().to_string(),
            other => short(other),
        };
        parts.push(format!("{key} {shown}"));
    }
    parts.join(", ")
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
pub fn draw_hub(a: &Assets, hub: &mut HubPanel, account: Option<&api::AccountReply>) -> bool {
    let (sw, sh) = (screen_width(), screen_height());
    let u = px_per_unit();
    hub.checking = (hub.checking - get_frame_time()).max(0.0);
    hub.poll();
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
        hub.refresh_all();
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
        for line in wrap(&ui, &hub.section(title, account), 7.0, body.w) {
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
    use serde_json::json;

    fn answered(status: u16, body: Value) -> Probe {
        Probe::Answered { status, body: body.as_object().cloned().unwrap_or_default() }
    }

    #[test]
    fn account_line_shows_the_servers_role_and_scopes() {
        assert_eq!(account_text(None), "NOT SIGNED IN // NO ADMIN SCOPES");
        let owner = api::AccountReply {
            username: "KYTTU".into(),
            admin: true,
            has_admin_owner: true,
            admin_owner: true,
            has_admin_permissions: true,
            admin_permissions: vec!["hunters".into(), "world".into()],
            ..Default::default()
        };
        assert_eq!(account_text(Some(&owner)), "KYTTU // OWNER // SCOPES HUNTERS WORLD");
        let player = api::AccountReply { username: "BOB".into(), has_admin_permissions: true, ..Default::default() };
        assert_eq!(account_text(Some(&player)), "BOB // PLAYER // SCOPES NONE");
        let unknown = api::AccountReply { username: "BOB".into(), ..Default::default() };
        assert_eq!(account_text(Some(&unknown)), "BOB // PLAYER // SCOPES NOT REPORTED BY SERVER");
    }

    #[test]
    fn this_build_line_names_the_build_not_a_fixed_label() {
        let text = this_build_text();
        assert!(text.starts_with("BUILD "), "{text}");
        assert!(!text.contains("RUST PORT") && !text.contains("CHANNEL LOCAL") && !text.contains("HALF-RES"), "{text}");
    }

    #[test]
    fn debug_line_uses_backend_override_host() {
        let cfg = BackendConfig::loopback(18105);
        assert_eq!(cfg.base_url, "http://127.0.0.1:18105");
        assert_eq!(debug_line(), "CHANNEL DEBUG");
    }

    #[test]
    fn server_online_lists_health_keys_in_godot_order() {
        let health = answered(
            200,
            json!({"ok": true, "runtime": "node", "service": "last-route-backend", "instance": "main", "port": 8105,
                   "rooms": 2, "relay": {"enabled": true, "rooms": [1, 2]}, "webRooms": 0, "now": "2026-11-03T19:00:00.000Z"}),
        );
        assert_eq!(
            server_text(&health),
            "CHANNEL DEBUG\nONLINE\nINSTANCE: main\nSERVICE: last-route-backend\nROOMS: 2\nWEBROOMS: 0\nRELAY: enabled true, rooms 2\nRUNTIME: node\nNOW: 2026-11-03T19:00:00.000Z"
        );
    }

    #[test]
    fn server_unreachable_error_and_checking() {
        assert_eq!(server_text(&Probe::Unreachable), "CHANNEL DEBUG\nCAN'T REACH THE SERVER");
        assert_eq!(
            server_text(&answered(500, json!({}))),
            "CHANNEL DEBUG\nREACHED // SERVER ERROR (HTTP 500)"
        );
        assert_eq!(server_text(&Probe::Checking), "CHANNEL DEBUG\nCHECKING...");
    }

    #[test]
    fn a_5xx_reads_as_reached_and_a_connect_failure_as_unreachable() {
        let err = NetError::Http { status: 500, code: String::new(), error: String::new(), body: json!({}) };
        assert_eq!(Probe::from_result(Err(err)), answered(500, json!({})));
        assert_eq!(Probe::from_result(Err(NetError::Timeout)), Probe::Unreachable);
        assert_eq!(Probe::from_result(Err(NetError::Transport("refused".into()))), Probe::Unreachable);
    }

    #[test]
    fn updates_list_every_platform_one_line_each() {
        let updates = vec![
            ("macos", answered(200, json!({"ok": true, "channel": "debug", "update": {"build": 42, "revision": "0123456789abcdef"}}))),
            ("windows", answered(500, json!({}))),
            ("linux", Probe::Unreachable),
            ("phone", answered(404, json!({"ok": false, "error": "NO UPDATE PUBLISHED"}))),
            ("quest", Probe::Checking),
        ];
        assert_eq!(
            updates_text(&updates),
            "MACOS: BUILD 42 @ 01234567\nWINDOWS: SERVER ERROR (HTTP 500)\nLINUX: NO ANSWER\nPHONE: NONE PUBLISHED\nQUEST: CHECKING..."
        );
        assert_eq!(PLATFORMS, ["macos", "windows", "linux", "phone", "quest"]);
    }

    #[test]
    fn public_route_is_short_and_never_shows_secrets() {
        let public = answered(200, json!({"ok": true, "game": "last-route", "hostToken": "x", "players": [1, 2, 3], "route": {"a": 1}}));
        assert_eq!(public_text(&public), "ok true, game last-route, players 3, route 1");
        assert_eq!(public_text(&Probe::Unreachable), "NO ANSWER");
        assert_eq!(public_text(&answered(500, json!({}))), "NO ANSWER // SERVER ERROR (HTTP 500)");
        assert_eq!(public_text(&answered(200, json!({}))), "NO ANSWER");
    }
}
