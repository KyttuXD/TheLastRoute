//! The start menu: the title (single player or multiplayer), the story list, and the multiplayer screens: a guest name or
//! a signed-in account, the list of stories a room can play, joining by code, and the lobby.
//!
//! W/S or the arrows move, Enter picks, Esc goes back (Esc on the title quits). Text screens take letters and
//! digits, Tab moves between fields. Mouse: hovering a row selects it, a click picks it. No key hints are drawn.
//!
//! The menu only asks; the server (multiplayer.rs, net.rs) answers. A single-player pick comes back as `Act::Play`.

use crate::model::Pack;
use crate::multiplayer::{self, RoomView, Seat, Who};
use crate::net::{self, Pending, Reply};
use crate::ui;
use macroquad::prelude::*;
use serde_json::{json, Value};

/// What the menu asks the caller to do this frame.
pub enum Act {
    Stay,
    /// Play this story alone.
    Play(String),
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Title,
    Stories,
    Multi,
    Create,
    Join,
    SignIn,
    Lobby,
}

/// What a request in flight was for. Its reply is handled by `on_reply`.
#[derive(Clone, Debug)]
enum Ask {
    Health,
    Guest,
    Login(String),
    Create,
    Lookup(String),
    Join,
    /// A ready toggle. Its answer carries the room as it now stands.
    Room,
    /// A pass of the host. Its answer ends the pass list either way.
    Pass,
    /// The lobby's heartbeat: the same call as a ready toggle, but it does not end a pass in progress.
    Beat,
    Leave,
    Logout,
}

/// One row of a screen. `id` is what activating it means (see `activate`).
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct Row {
    id: usize,
    label: String,
    sub: String,
    enabled: bool,
}

const BACK: usize = 1000;
/// Title rows: PLAY, MULTIPLAYER, SETTINGS, QUIT.
const TITLE_PLAY: usize = 0;
const TITLE_MULTI: usize = 1;
const TITLE_SETTINGS: usize = 2;
const TITLE_QUIT: usize = 3;

/// The title screen's buttons: one word each.
fn title_rows() -> [(usize, &'static str); 4] {
    [(TITLE_PLAY, "PLAY"), (TITLE_MULTI, "MULTIPLAYER"), (TITLE_SETTINGS, "SETTINGS"), (TITLE_QUIT, "QUIT")]
}

/// `n` corner points of a regular polygon around (cx, cy) with radius `r`, turned by `rot` radians.
fn polygon_points(cx: f32, cy: f32, r: f32, rot: f32, n: usize) -> Vec<(f32, f32)> {
    (0..n)
        .map(|i| {
            let a = rot + std::f32::consts::TAU * i as f32 / n.max(1) as f32;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}
/// Lobby rows that pass the host: a player's row is `PASS_BASE + index`.
const PASS_BASE: usize = 100;
const CANCEL: usize = 99;
const HEARTBEAT_SECONDS: f32 = 2.0;
/// A request with no answer after this long is a failure, so the menu never waits forever on a silent server.
const REPLY_DEADLINE_SECONDS: f64 = 20.0;
const MAX_USERNAME: usize = 24;
const MAX_PASSWORD: usize = 64;
/// How long a notice stays on screen before it fades out, and how long the fade takes.
const TOAST_SECONDS: f32 = 4.5;
const TOAST_FADE_SECONDS: f32 = 1.0;
/// How long a new screen takes to fade in.
const SCREEN_FADE_SECONDS: f32 = 0.25;
/// The tint of the backdrop on the multiplayer screens.
const COOL_BLUE: Color = Color::new(0.45, 0.65, 1.0, 1.0);

/// Moves a hover glow (0..1) toward `target`, eased by `dt`. It snaps when it is close, so it settles exactly.
fn ease_glow(current: f32, target: f32, dt: f32) -> f32 {
    let step = (dt * 10.0).clamp(0.0, 1.0);
    let next = current + (target - current) * step;
    if (next - target).abs() < 0.01 {
        target
    } else {
        next
    }
}

/// How visible a notice is (1 = fully, 0 = gone) when it is `age` seconds old.
fn toast_alpha(age: f32) -> f32 {
    if age < 0.0 || age >= TOAST_SECONDS {
        return 0.0;
    }
    let fade_start = TOAST_SECONDS - TOAST_FADE_SECONDS;
    if age <= fade_start {
        1.0
    } else {
        (TOAST_SECONDS - age) / TOAST_FADE_SECONDS
    }
}

/// How far a screen's fade-in has come (0..1) after `age` seconds.
/// The rows that fit in `avail` pixels, as a range `first..last`, chosen so the selected row is inside it. Rows have
/// the heights in `steps`. When everything fits, that is all of them.
fn visible_rows(steps: &[f32], avail: f32, sel: usize) -> (usize, usize) {
    let n = steps.len();
    if n == 0 || steps.iter().sum::<f32>() <= avail {
        return (0, n);
    }
    let sel = sel.min(n - 1);
    // grow the window from the selection: down first, then up, while rows fit
    let (mut first, mut last, mut used) = (sel, sel + 1, steps[sel]);
    loop {
        let mut grew = false;
        if last < n && used + steps[last] <= avail {
            used += steps[last];
            last += 1;
            grew = true;
        }
        if first > 0 && used + steps[first - 1] <= avail {
            first -= 1;
            used += steps[first];
            grew = true;
        }
        if !grew {
            return (first, last);
        }
    }
}

fn screen_fade(age: f32) -> f32 {
    (age / SCREEN_FADE_SECONDS).clamp(0.0, 1.0)
}

/// The scale of the layout: 1.0 at 720 pixels high, clamped so 800x600 and 1920x1080 both fit.
/// The menu scale: the screen height against 720p, times the player's UI scale setting (0.8 to 1.3).
fn ui_scale(sh: f32) -> f32 {
    (sh / 720.0).clamp(0.82, 1.5) * crate::settings::get().ui_scale
}

/// The largest font size at most `size` (down to `min`) whose text `measure` finds no wider than `max_w`.
fn fit_size(size: f32, min: f32, max_w: f32, measure: impl Fn(f32) -> f32) -> f32 {
    let mut s = size;
    while s > min && measure(s) > max_w {
        s -= 1.0;
    }
    s
}

/// The menu's state between frames.
pub struct Menu<'a> {
    pack: &'a Pack,
    screen: Screen,
    sel: usize,
    /// The row boxes of the last frame, with each row's index, for the mouse.
    hits: Vec<(Rect, usize)>,
    last_mouse: (f32, f32),
    who: Option<Who>,
    /// None until the first health check answers: Ok(rooms) or Err(why).
    health: Option<Result<usize, String>>,
    /// The player's request in flight (one at a time), with the time it was sent.
    asking: Option<(Ask, Pending)>,
    asked_at: f64,
    /// The lobby's heartbeat in flight. It never overlaps a player's request, so the two cannot reorder.
    beat: Option<Pending>,
    beat_at: f64,
    /// Set when Esc leaves a create or join still on its way: its answer is dropped (a room it joins is left again).
    cancel_pending: bool,
    seat: Option<Seat>,
    /// In the lobby, the host is choosing whom to pass the host to.
    passing: bool,
    /// The row under the mouse, and the screen the last drawing showed (mouse picks count only on that screen).
    hover: Option<usize>,
    drawn: Option<Screen>,
    join_code: String,
    sign_user: String,
    sign_pass: String,
    heartbeat: f32,
    notice: String,
    notice_error: bool,
    /// True until the first frame has run (see `frame`).
    first_frame: bool,
    /// Seconds since the notice was set (it fades out), and since the screen last changed (it fades in).
    notice_age: f32,
    screen_age: f32,
    /// Per-row hover glow (0..1), eased by the frame time; reset when the screen changes.
    glow: Vec<f32>,
    glow_screen: Option<Screen>,
    dt: f32,
    /// The settings screen, while it is open over the title.
    settings: Option<crate::settings_ui::SettingsScreen>,
}

/// Runs the menu until a story is picked (its id) or the player quits (None).
pub async fn run(pack: &Pack) -> Option<String> {
    let mut menu = Menu::new(pack);
    loop {
        match menu.frame(get_frame_time().min(0.1)) {
            Act::Stay => {}
            Act::Play(id) => return Some(id),
            Act::Quit => return None,
        }
        next_frame().await;
    }
}

impl<'a> Menu<'a> {
    pub fn new(pack: &'a Pack) -> Menu<'a> {
        Menu {
            pack,
            screen: Screen::Title,
            sel: 0,
            hits: Vec::new(),
            last_mouse: (0.0, 0.0),
            who: None,
            health: None,
            asking: None,
            asked_at: 0.0,
            beat: None,
            beat_at: 0.0,
            cancel_pending: false,
            seat: None,
            passing: false,
            hover: None,
            drawn: None,
            join_code: String::new(),
            sign_user: String::new(),
            sign_pass: String::new(),
            heartbeat: 0.0,
            notice: String::new(),
            notice_error: false,
            first_frame: true,
            notice_age: 0.0,
            screen_age: 0.0,
            glow: Vec::new(),
            glow_screen: None,
            dt: 0.0,
            settings: None,
        }
    }

    /// One frame: the reply that came in, the heartbeat, the keys and the mouse, then the drawing.
    pub fn frame(&mut self, dt: f32) -> Act {
        self.dt = dt;
        self.notice_age += dt;
        self.screen_age += dt;
        self.take_reply();
        self.tick(dt);
        if let Some(mut settings) = self.settings.take() {
            self.first_frame = false;
            if matches!(settings.frame(dt, self.pack, &|_| None), crate::settings_ui::SettingsAct::Stay) {
                self.settings = Some(settings);
            } else {
                self.screen_age = 0.0;
            }
            return Act::Stay;
        }
        let rows = self.rows();
        // The key that opened the menu (Enter on "Back to menu") is still down on its first frame: that frame takes no
        // input, so it cannot pick a row the player never saw.
        let act = if self.first_frame {
            self.first_frame = false;
            while get_char_pressed().is_some() {}
            Act::Stay
        } else {
            self.input(&rows)
        };
        if matches!(act, Act::Stay) {
            // The keys may have changed the screen, so the rows are read again for the drawing.
            let rows = self.rows();
            self.draw(&rows);
        }
        act
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.notice = text.into();
        self.notice_error = error;
        self.notice_age = 0.0;
    }

    fn enter(&mut self, screen: Screen) {
        if screen != self.screen {
            self.screen_age = 0.0;
        }
        self.screen = screen;
        self.sel = 0;
    }

    /// Starts a request, unless one is already on its way (then the player is told to wait).
    fn ask(&mut self, ask: Ask, method: &str, path: &str, body: Option<Value>) {
        if self.asking.is_some() || self.beat.is_some() {
            self.say("Wait a moment: the server is still answering", true);
            return;
        }
        let pending = net::send(method, path, body.as_ref());
        self.asked_at = get_time();
        self.asking = Some((ask, pending));
    }

    fn take_reply(&mut self) {
        let polled = match self.asking.as_mut() {
            Some((_, pending)) => pending.poll(),
            None => None,
        };
        let overdue = self.asking.is_some() && get_time() - self.asked_at > REPLY_DEADLINE_SECONDS;
        let done = polled.or_else(|| overdue.then(|| Err("the server did not answer in time".to_string())));
        if let Some(result) = done {
            if let Some((ask, _)) = self.asking.take() {
                self.on_reply(ask, result);
            }
        }
        let beat_result = self.beat.as_mut().and_then(|pending| pending.poll());
        if beat_result.is_some() || (self.beat.is_some() && get_time() - self.beat_at > REPLY_DEADLINE_SECONDS) {
            self.beat = None;
            if let Some(result) = beat_result {
                self.on_reply(Ask::Beat, result);
            }
        }
    }

    fn tick(&mut self, dt: f32) {
        // The multiplayer screen asks for the server's state until it has an answer (a request refused while another
        // was in flight is asked again here).
        if self.screen == Screen::Multi && self.health.is_none() && self.asking.is_none() {
            self.ask(Ask::Health, "GET", "/api/health", None);
        }
        if self.screen != Screen::Lobby || self.beat.is_some() || self.asking.is_some() {
            return;
        }
        self.heartbeat += dt;
        if self.heartbeat < HEARTBEAT_SECONDS {
            return;
        }
        self.heartbeat = 0.0;
        let Some(seat) = self.seat.clone() else { return };
        // The ready call also tells the server this player is still here (presence), so the lobby polls with it.
        let ready = self.me_ready(&seat);
        self.beat_at = get_time();
        self.beat = Some(net::send("POST", "/api/rooms/ready", Some(&json!({ "code": seat.code, "playerToken": seat.token, "ready": ready }))));
    }

    fn me_ready(&self, seat: &Seat) -> bool {
        self.who.as_ref().and_then(|w| seat.room.player(w.name())).map(|p| p.ready).unwrap_or(false)
    }

    fn on_reply(&mut self, ask: Ask, result: Result<Reply, String>) {
        let cancelled = if matches!(ask, Ask::Create | Ask::Join | Ask::Lookup(_)) {
            std::mem::take(&mut self.cancel_pending)
        } else {
            false
        };
        let reply = match result {
            Ok(reply) => reply,
            Err(why) => {
                match ask {
                    Ask::Health => self.health = Some(Err(why)),
                    Ask::Beat | Ask::Logout => {}
                    Ask::Leave => self.say("Could not tell the server you left; the room may still hold your seat", true),
                    Ask::Pass => {
                        self.passing = false;
                        self.say(why, true);
                    }
                    _ => self.say(why, true),
                }
                return;
            }
        };
        if cancelled {
            // The player left the screen that asked. A room that was joined anyway is left again, quietly.
            if let (Ask::Create | Ask::Join, true) = (&ask, reply.ok()) {
                if let Some(seat) = multiplayer::seat_from(&reply.json()) {
                    let _ = net::send("POST", "/api/rooms/leave", Some(&json!({ "code": seat.code, "playerToken": seat.token })));
                }
            }
            return;
        }
        match ask {
            Ask::Health => {
                self.health = Some(if reply.ok() {
                    Ok(reply.json().get("rooms").and_then(Value::as_u64).unwrap_or(0) as usize)
                } else {
                    Err(reply.failure())
                });
            }
            Ask::Guest => {
                let body = reply.json();
                let name = body.get("name").and_then(Value::as_str);
                let token = body.get("token").and_then(Value::as_str);
                match (reply.ok(), name, token) {
                    (true, Some(name), Some(token)) => {
                        self.who = Some(Who::Guest { name: name.to_string(), token: token.to_string() });
                        self.say(format!("Playing as {name}"), false);
                    }
                    _ => self.say(reply.failure(), true),
                }
            }
            Ask::Login(typed) => {
                let body = reply.json();
                match body.get("sessionToken").and_then(Value::as_str) {
                    Some(session) if reply.ok() => {
                        let username = body.get("username").and_then(Value::as_str).unwrap_or(&typed).to_string();
                        self.say(format!("Signed in as {username}"), false);
                        self.who = Some(Who::Account { username, session: session.to_string() });
                        self.sign_pass.clear();
                        self.enter(Screen::Multi);
                    }
                    _ => self.say(reply.failure(), true),
                }
            }
            Ask::Create | Ask::Join => match (reply.ok(), multiplayer::seat_from(&reply.json())) {
                (true, Some(seat)) => {
                    self.seat = Some(seat);
                    self.join_code.clear();
                    self.heartbeat = 0.0;
                    self.say("", false);
                    self.enter(Screen::Lobby);
                }
                _ => self.say(reply.failure(), true),
            },
            Ask::Lookup(code) => {
                if !reply.ok() {
                    self.say(reply.failure(), true);
                    return;
                }
                // GET /api/rooms/<code> answers { ok, code, room }: the view is under `room`.
                let Some(room) = reply.json().get("room").and_then(RoomView::from_json) else {
                    self.say("The server sent a room this game cannot read", true);
                    return;
                };
                if let Err(why) = self.check_pack(&room) {
                    self.say(why, true);
                    return;
                }
                let Some(who) = self.who.clone() else {
                    self.say("Get a guest name or sign in first", true);
                    return;
                };
                let mut body = who.fields();
                body["code"] = json!(code);
                body["pack"] = json!(room.pack);
                body["packFingerprint"] = json!(room.fingerprint);
                self.ask(Ask::Join, "POST", "/api/rooms/join", Some(body));
            }
            Ask::Room | Ask::Beat | Ask::Pass => {
                if matches!(ask, Ask::Pass) {
                    self.passing = false;
                }
                let room = reply.json().get("room").and_then(RoomView::from_json);
                match (reply.ok(), room) {
                    (true, Some(room)) => {
                        if let Some(seat) = self.seat.as_mut() {
                            if seat.code == room.code {
                                seat.room = room;
                            }
                        }
                    }
                    _ => {
                        // The server says the room or this seat is gone: back to the multiplayer menu. Any other refusal
                        // (a pass to someone who left, a full room) keeps the seat and says why.
                        let gone = matches!(reply.json().get("code").and_then(Value::as_str), Some("ROOM_NOT_FOUND") | Some("NOT_IN_ROOM"));
                        if gone {
                            self.seat = None;
                            self.passing = false;
                            if self.screen == Screen::Lobby {
                                self.enter(Screen::Multi);
                            }
                        }
                        if !matches!(ask, Ask::Beat) || self.screen == Screen::Lobby {
                            self.say(reply.failure(), true);
                        }
                    }
                }
            }
            Ask::Leave => {
                self.seat = None;
                self.passing = false;
                if self.screen == Screen::Lobby {
                    self.enter(Screen::Multi);
                }
            }
            Ask::Logout => {}
        }
    }

    /// The fingerprint of this computer's copy of a story (the pack folder's files, as the server counts them).
    fn local_fingerprint(&self, story_id: &str) -> Result<String, String> {
        let Some(info) = self.pack.packs.iter().find(|p| p.id == story_id).filter(|_| self.pack.story(story_id).is_some()) else {
            return Err(format!("This computer does not have the story '{story_id}'"));
        };
        multiplayer::fingerprint(&info.dir).map_err(|e| format!("could not read the story: {e}"))
    }

    /// Whether this computer has the room's story with the same files as the server. Joining needs that match.
    fn check_pack(&self, room: &RoomView) -> Result<(), String> {
        let local = self.local_fingerprint(&room.pack)?;
        if local != room.fingerprint {
            return Err(format!("Your copy of '{}' is not the same as the server's; update it to join", room.pack));
        }
        Ok(())
    }

    /// The rows of the current screen.
    fn rows(&self) -> Vec<Row> {
        let row = |id: usize, label: &str, sub: &str, enabled: bool| Row { id, label: label.to_string(), sub: sub.to_string(), enabled };
        match self.screen {
            Screen::Title => title_rows().iter().map(|(id, label)| row(*id, label, "", true)).collect(),
            Screen::Stories => {
                let mut rows: Vec<Row> = self
                    .pack
                    .stories_in_menu_order()
                    .iter()
                    .enumerate()
                    .map(|(i, s)| row(i, &s.title, &format!("{}   {}", s.era.label(), who_plays(&s.player)), true))
                    .collect();
                rows.push(row(BACK, "Back", "", true));
                rows
            }
            Screen::Multi => {
                let identity = match &self.who {
                    None => row(2, "Get a guest name", "Needed to make or join a room", true),
                    Some(Who::Guest { .. }) => row(2, "Sign in to an account", "Use an account", true),
                    Some(Who::Account { .. }) => row(2, "Sign out", "Play as a guest again", true),
                };
                let signed_in = self.who.is_some();
                vec![
                    row(0, "Create a room", "Pick a story that allows multiplayer", signed_in),
                    row(1, "Join a room", "Enter the code a host gives you", signed_in),
                    identity,
                    row(BACK, "Back", "", true),
                ]
            }
            Screen::Create => {
                let mut rows: Vec<Row> = self
                    .multiplayer_stories()
                    .iter()
                    .enumerate()
                    .map(|(i, s)| row(i, &s.title, &format!("{}   up to six players", s.era.label()), true))
                    .collect();
                if rows.is_empty() {
                    rows.push(row(0, "No story allows multiplayer yet", "Packs turn it on with story.multiplayer", false));
                }
                rows.push(row(BACK, "Back", "", true));
                rows
            }
            Screen::Join => vec![
                row(0, "Join this room", "Type the code above first", self.join_code.len() >= 5),
                row(BACK, "Back", "", true),
            ],
            Screen::SignIn => vec![
                row(0, "Username", if self.sign_user.is_empty() { "type your username" } else { self.sign_user.as_str() }, true),
                row(1, "Password", &"*".repeat(self.sign_pass.chars().count()), true),
                row(2, "Sign in", "", true),
                row(BACK, "Back", "", true),
            ],
            Screen::Lobby => self.lobby_rows(),
        }
    }

    fn lobby_rows(&self) -> Vec<Row> {
        let row = |id: usize, label: &str, sub: &str| Row { id, label: label.to_string(), sub: sub.to_string(), enabled: true };
        let Some(seat) = &self.seat else { return vec![row(BACK, "Back", "")] };
        if self.passing {
            let me = self.who.as_ref().map(|w| w.name().to_string()).unwrap_or_default();
            let mut rows: Vec<Row> = seat
                .room
                .players
                .iter()
                .filter(|p| p.name != me)
                .enumerate()
                .map(|(i, p)| row(PASS_BASE + i, &p.name, "pass the host to this player"))
                .collect();
            rows.push(row(CANCEL, "Cancel", ""));
            return rows;
        }
        let is_host = self.who.as_ref().is_some_and(|w| w.name() == seat.room.host);
        let ready = self.me_ready(seat);
        let mut rows = vec![row(0, if ready { "Not ready" } else { "Ready" }, "the room starts when everyone is ready")];
        if is_host {
            rows.push(row(1, "Pass the host", "hand the room to another player"));
        }
        rows.push(row(2, "Leave the room", ""));
        rows
    }

    /// The stories that allow multiplayer, in menu order.
    fn multiplayer_stories(&self) -> Vec<&crate::model::StoryInfo> {
        self.pack.stories_in_menu_order().into_iter().filter(|s| s.multiplayer).collect()
    }

    fn input(&mut self, rows: &[Row]) -> Act {
        let text_screen = matches!(self.screen, Screen::Join | Screen::SignIn);
        self.type_text(text_screen);

        if is_key_pressed(KeyCode::Escape) {
            return self.back();
        }
        let up = is_key_pressed(KeyCode::Up) || (!text_screen && is_key_pressed(KeyCode::W));
        let down = is_key_pressed(KeyCode::Down) || (!text_screen && is_key_pressed(KeyCode::S));
        if is_key_pressed(KeyCode::Tab) && text_screen && !rows.is_empty() {
            self.sel = (self.sel + 1) % rows.len();
        }
        if !rows.is_empty() {
            if up {
                self.sel = (self.sel + rows.len() - 1) % rows.len();
            }
            if down {
                self.sel = (self.sel + 1) % rows.len();
            }
            self.sel = self.sel.min(rows.len() - 1);
        }

        // The mouse: hovering selects, a click picks. The boxes come from the last drawing, so they count only when that
        // drawing showed this screen (a reply can change the screen between frames).
        let m = mouse_position();
        let moved = m != self.last_mouse;
        self.last_mouse = m;
        let point = vec2(m.0, m.1);
        let live = self.drawn == Some(self.screen);
        self.hover = None;
        let mut picked = None;
        for (rect, index) in &self.hits {
            if live && rect.contains(point) {
                self.hover = Some(*index);
                if moved {
                    self.sel = *index;
                }
                if is_mouse_button_pressed(MouseButton::Left) {
                    picked = Some(*index);
                }
            }
        }
        let enter = is_key_pressed(KeyCode::Enter);
        if let Some(index) = picked {
            return self.pick(rows, index);
        }
        if enter && !rows.is_empty() {
            return self.pick(rows, self.sel);
        }
        Act::Stay
    }

    /// Picks row `index` of the rows on screen. A disabled row says why nothing happens.
    fn pick(&mut self, rows: &[Row], index: usize) -> Act {
        let Some(row) = rows.get(index).cloned() else { return Act::Stay };
        self.sel = index;
        if !row.enabled {
            if self.screen == Screen::Multi {
                self.say("Get a guest name or sign in first", true);
            } else if self.screen == Screen::Join {
                self.say("A room code is five to eight letters or digits", true);
            }
            return Act::Stay;
        }
        self.activate(row.id)
    }

    /// Types into the field the screen takes text for: the room code, or the username or password being edited.
    fn type_text(&mut self, text_screen: bool) {
        let mut typed = Vec::new();
        while let Some(c) = get_char_pressed() {
            typed.push(c);
        }
        if !text_screen {
            return;
        }
        let backspace = is_key_pressed(KeyCode::Backspace);
        match self.screen {
            Screen::Join => {
                let typed: String = typed.into_iter().collect();
                self.join_code.push_str(&typed);
                self.join_code = crate::multiplayer::clean_room_code(&self.join_code);
                if backspace {
                    self.join_code.pop();
                }
            }
            Screen::SignIn => {
                for c in typed {
                    if c.is_control() {
                        continue;
                    }
                    match self.sel {
                        0 if (c.is_ascii_alphanumeric() || c == '_' || c == '-') && self.sign_user.len() < MAX_USERNAME => {
                            self.sign_user.push(c)
                        }
                        1 if self.sign_pass.chars().count() < MAX_PASSWORD => self.sign_pass.push(c),
                        _ => {}
                    }
                }
                if backspace {
                    match self.sel {
                        0 => {
                            self.sign_user.pop();
                        }
                        1 => {
                            self.sign_pass.pop();
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Esc: the screen above, or quit on the title.
    fn back(&mut self) -> Act {
        match self.screen {
            Screen::Title => return Act::Quit,
            Screen::Stories | Screen::Multi => self.enter(Screen::Title),
            Screen::Create | Screen::Join | Screen::SignIn => {
                // A create or join still on its way is cancelled: its answer is dropped, and a room it joins is left.
                if matches!(self.screen, Screen::Create | Screen::Join) && self.asking.is_some() {
                    self.cancel_pending = true;
                }
                self.enter(Screen::Multi);
            }
            Screen::Lobby => {
                if self.passing {
                    self.passing = false;
                    self.sel = 0;
                } else {
                    self.leave();
                }
            }
        }
        Act::Stay
    }

    /// Leaves the room: the server is told, and the menu goes back to the multiplayer screen at once.
    fn leave(&mut self) {
        if let Some(seat) = self.seat.clone() {
            let body = json!({ "code": seat.code, "playerToken": seat.token });
            if self.asking.is_none() {
                self.ask(Ask::Leave, "POST", "/api/rooms/leave", Some(body));
            } else {
                // Another call is in flight: the leave still goes out, and its answer is not waited for. A room the
                // player left must not keep their seat.
                let _ = net::send("POST", "/api/rooms/leave", Some(&body));
            }
        }
        self.seat = None;
        self.passing = false;
        self.enter(Screen::Multi);
    }

    /// Acts on row `id` of the current screen.
    fn activate(&mut self, id: usize) -> Act {
        match self.screen {
            Screen::Title => match id {
                TITLE_PLAY => self.enter(Screen::Stories),
                TITLE_MULTI => {
                    self.enter(Screen::Multi);
                    self.health = None;
                }
                TITLE_SETTINGS => self.settings = Some(crate::settings_ui::SettingsScreen::new()),
                _ => return Act::Quit,
            },
            Screen::Stories => {
                if id == BACK {
                    self.enter(Screen::Title);
                } else if let Some(story) = self.pack.stories_in_menu_order().get(id) {
                    return Act::Play(story.id.clone());
                }
            }
            Screen::Multi => match id {
                0 => self.enter(Screen::Create),
                1 => self.enter(Screen::Join),
                2 => match self.who.clone() {
                    None => self.ask(Ask::Guest, "POST", "/api/guest/name", Some(json!({}))),
                    Some(Who::Guest { .. }) => {
                        self.sign_pass.clear();
                        self.enter(Screen::SignIn);
                    }
                    Some(Who::Account { session, .. }) => {
                        // The identity is dropped only once the logout is actually on its way, so the session is never orphaned.
                        if self.asking.is_some() || self.beat.is_some() {
                            self.say("Wait a moment: the server is still answering", true);
                        } else {
                            self.ask(Ask::Logout, "POST", "/api/account/logout", Some(json!({ "sessionToken": session })));
                            self.who = None;
                            self.say("Signed out. Playing as a guest needs a new guest name.", false);
                        }
                    }
                },
                _ => self.enter(Screen::Title),
            },
            Screen::Create => {
                if id == BACK {
                    self.enter(Screen::Multi);
                } else if let (Some(story), Some(who)) = (self.multiplayer_stories().get(id).map(|s| s.id.clone()), self.who.clone()) {
                    // The server refuses a room whose pack files differ from its own, so the fingerprint goes with the create.
                    match self.local_fingerprint(&story) {
                        Ok(fingerprint) => {
                            let mut body = who.fields();
                            body["pack"] = json!(story);
                            body["packFingerprint"] = json!(fingerprint);
                            self.ask(Ask::Create, "POST", "/api/rooms/create", Some(body));
                        }
                        Err(why) => self.say(why, true),
                    }
                }
            }
            Screen::Join => {
                if id == BACK {
                    self.enter(Screen::Multi);
                } else if self.join_code.len() >= 5 {
                    let code = self.join_code.clone();
                    self.ask(Ask::Lookup(code.clone()), "GET", &format!("/api/rooms/{code}"), None);
                }
            }
            Screen::SignIn => match id {
                0 | 1 => self.sel = id,
                2 => {
                    if self.sign_user.trim().is_empty() || self.sign_pass.is_empty() {
                        self.say("Type your username and password", true);
                    } else {
                        let username = self.sign_user.clone();
                        let body = json!({ "username": username, "password": self.sign_pass });
                        self.ask(Ask::Login(username), "POST", "/api/account/login", Some(body));
                    }
                }
                _ => {
                    self.sign_pass.clear();
                    self.enter(Screen::Multi);
                }
            },
            Screen::Lobby => self.activate_lobby(id),
        }
        Act::Stay
    }

    fn activate_lobby(&mut self, id: usize) {
        let Some(seat) = self.seat.clone() else {
            self.enter(Screen::Multi);
            return;
        };
        if self.passing {
            if id == CANCEL {
                self.passing = false;
                self.sel = 0;
            } else if id >= PASS_BASE {
                let me = self.who.as_ref().map(|w| w.name().to_string()).unwrap_or_default();
                let others: Vec<String> = seat.room.players.iter().filter(|p| p.name != me).map(|p| p.name.clone()).collect();
                if let Some(to) = others.get(id - PASS_BASE) {
                    let body = json!({ "code": seat.code, "playerToken": seat.token, "to": to });
                    self.ask(Ask::Pass, "POST", "/api/rooms/pass-host", Some(body));
                }
            }
            return;
        }
        match id {
            0 => {
                let ready = !self.me_ready(&seat);
                self.ask(Ask::Room, "POST", "/api/rooms/ready", Some(json!({ "code": seat.code, "playerToken": seat.token, "ready": ready })));
            }
            1 => {
                if matches!(&self.asking, Some((Ask::Room | Ask::Pass, _))) {
                    self.say("Wait a moment: the server is still answering", true);
                } else {
                    self.passing = true;
                    self.sel = 0;
                }
            }
            2 => self.leave(),
            _ => self.enter(Screen::Multi),
        }
    }

    /// The backdrop's tint: the accent on the title, the selected story's era on the story list, a cool blue elsewhere.
    fn tint(&self) -> Color {
        match self.screen {
            Screen::Title => ui::ACCENT,
            Screen::Stories => self.pack.stories_in_menu_order().get(self.sel).map(|s| ui::era_color(s.era)).unwrap_or(ui::ACCENT),
            _ => COOL_BLUE,
        }
    }

    fn draw(&mut self, rows: &[Row]) {
        self.drawn = Some(self.screen);
        let pack: &Pack = self.pack;
        let (sw, sh) = (screen_width(), screen_height());
        let t = get_time() as f32;
        let s = ui_scale(sh);
        ui::backdrop(t, self.tint());

        // Each row's glow eases toward lit (selected or hovered) or dark.
        if self.glow_screen != Some(self.screen) {
            self.glow.clear();
            self.glow_screen = Some(self.screen);
        }
        self.glow.resize(rows.len(), 0.0);
        for (i, row) in rows.iter().enumerate() {
            let lit = row.enabled && (i == self.sel || self.hover == Some(i));
            self.glow[i] = ease_glow(self.glow[i], if lit { 1.0 } else { 0.0 }, self.dt);
        }

        if self.screen == Screen::Title {
            self.draw_title(rows, sw, sh, s, t);
            self.draw_toast(sw, sh);
            self.draw_veil(sw, sh);
            return;
        }

        // The header: the big title at the top left, then this screen's name.
        let cx = sw / 2.0;
        let title_size = (52.0 * s).clamp(36.0, 80.0);
        let title_y = sh * 0.03 + title_size;
        ui::title("KYTTU - LRS", 28.0, title_y, title_size, ui::ACCENT);
        let (heading, _) = self.heading();
        let mut y = title_y + 54.0;
        if self.screen != Screen::Lobby {
            ui::title_centered(&heading, cx, title_y + 40.0, (30.0 * s).clamp(26.0, 40.0), ui::TEXT);
            y = title_y + 62.0;
        }

        // The columns: one centred panel, or on a wide story list the cards and an info panel beside them.
        let row_h = (64.0 * s).max(52.0);
        let card_h = (60.0 * s).max(54.0);
        let lobby_h = (row_h - 6.0).max(52.0);
        let mut w = (620.0 * s).min(sw - 40.0);
        let mut x = (sw - w) / 2.0;
        let stories_screen = self.screen == Screen::Stories;
        let mut info_col = None;
        if stories_screen && sw >= 900.0 {
            let total = (sw - 60.0).min(1100.0);
            let list_w = total * 0.58;
            x = (sw - total) / 2.0;
            w = list_w;
            info_col = Some((x + list_w + 20.0, total - list_w - 20.0));
        }

        if self.screen == Screen::Multi {
            y = self.draw_multi_status(x, y, w);
        }
        if self.screen == Screen::Join {
            y = self.draw_code_field(x, y, w, s);
        }
        if self.screen == Screen::Lobby {
            y = self.draw_lobby_players(x, y, w, s);
        }

        let stories = pack.stories_in_menu_order();
        let list_top = y;
        self.hits.clear();
        let step_of = |row: &Row| {
            if stories_screen && row.id != BACK {
                card_h
            } else if self.screen == Screen::Lobby {
                lobby_h
            } else {
                row_h
            }
        };
        // Only the rows that fit on screen are drawn; the window follows the selection, and a marker shows
        // that more rows are above or below.
        let steps: Vec<f32> = rows.iter().map(step_of).collect();
        let avail = (sh - 24.0 - list_top).max(row_h);
        let (first, last) = visible_rows(&steps, avail, self.sel);
        if first > 0 {
            ui::text_centered("^  more above  ^", x + w / 2.0, y - 4.0, 14.0, ui::DIM);
        }
        for (i, row) in rows.iter().enumerate().take(last).skip(first) {
            let card = stories_screen && row.id != BACK;
            let step = steps[i];
            let rect = Rect::new(x, y, w, step - 6.0);
            let lit = row.enabled && (i == self.sel || self.hover == Some(i));
            let glow = self.glow.get(i).copied().unwrap_or(0.0);
            if card {
                if let Some(story) = stories.get(row.id) {
                    draw_story_card(story, rect, lit, glow);
                }
            } else if self.screen == Screen::SignIn && row.id < 2 {
                let (value, placeholder) = if row.id == 0 {
                    (self.sign_user.clone(), "type your username")
                } else {
                    ("*".repeat(self.sign_pass.chars().count()), "type your password")
                };
                draw_field(rect, &row.label, &value, placeholder, i == self.sel, glow);
            } else {
                ui::button(rect, &row.label, "", lit, row.enabled, glow, ui::ACCENT);
            }
            self.hits.push((rect, i));
            y += step;
        }

        if last < rows.len() {
            ui::text_centered("v  more below  v", x + w / 2.0, y + 6.0, 14.0, ui::DIM);
        }

        if let Some((ix, iw)) = info_col {
            let rect = Rect::new(ix, list_top, iw, (y - list_top - 6.0).max(222.0));
            draw_story_info(rect, stories.get(self.sel).copied());
        }

        self.draw_toast(sw, sh);
        self.draw_veil(sw, sh);
    }

    /// A short fade in when the screen changes.
    fn draw_veil(&self, sw: f32, sh: f32) {
        let veil = 1.0 - screen_fade(self.screen_age);
        if veil > 0.0 {
            draw_rectangle(0.0, 0.0, sw, sh, ui::fade(ui::BG_BOTTOM, veil * 0.85));
        }
    }

    /// The title screen: the title top left, compact one-word buttons below it with an accent bar on the selected one,
    /// and a slowly turning emblem on the right.
    fn draw_title(&mut self, rows: &[Row], sw: f32, sh: f32, s: f32, t: f32) {
        let left = (sw * 0.07).max(28.0);
        let title_size = (72.0 * s).clamp(40.0, 110.0);
        let title_y = sh * 0.06 + title_size;
        ui::title("KYTTU - LRS", left, title_y, title_size, ui::ACCENT);
        draw_rectangle(left, title_y + 14.0, 120.0 * s, 3.0, ui::fade(ui::ACCENT, 0.7));

        let bw = (280.0 * s).min(sw * 0.6);
        let bh = (46.0 * s).max(38.0);
        let gap = (14.0 * s).max(8.0);
        let mut y = (title_y + 60.0 * s).min(sh - rows.len() as f32 * (bh + gap));
        self.hits.clear();
        for (i, row) in rows.iter().enumerate() {
            let rect = Rect::new(left, y, bw, bh);
            let glow = self.glow.get(i).copied().unwrap_or(0.0);
            let lit = i == self.sel || self.hover == Some(i);
            draw_rectangle(rect.x, rect.y, rect.w, rect.h, ui::fade(ui::PANEL, 0.35 + 0.4 * glow));
            let bar_h = rect.h * glow;
            draw_rectangle(rect.x, rect.y + (rect.h - bar_h) / 2.0, 4.0, bar_h, ui::ACCENT);
            let size = 24.0 * s.min(1.3);
            let color = if lit { ui::ACCENT } else { ui::TEXT };
            ui::text(&row.label, rect.x + 20.0 + glow * 6.0, rect.y + rect.h / 2.0 + size * 0.35, size, color);
            self.hits.push((rect, i));
            y += bh + gap;
        }

        // The emblem: rings and two counter-turning shapes.
        let (ex, ey) = (sw * 0.72, sh * 0.5);
        let r = (sh * 0.3).min(sw * 0.2);
        let tint = ui::ACCENT;
        for k in 0..3 {
            let rr = r * (0.55 + 0.22 * k as f32);
            draw_circle_lines(ex, ey, rr, 1.5, ui::fade(tint, 0.35 - 0.08 * k as f32));
        }
        for (n, scale, dir, alpha) in [(6usize, 1.0f32, 1.0f32, 0.9f32), (3, 0.62, -1.6, 0.6)] {
            let pts = polygon_points(ex, ey, r * scale, t * 0.2 * dir, n);
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                draw_line(a.0, a.1, b.0, b.1, 2.5, ui::fade(tint, alpha));
            }
            for p in &pts {
                draw_circle(p.0, p.1, 4.0 * s, ui::fade(tint, alpha));
            }
        }
        draw_circle(ex, ey, 10.0 * s * (1.0 + 0.1 * (t * 1.5).sin()), ui::fade(tint, 0.9));
    }

    /// The title and subtitle of the current screen.
    fn heading(&self) -> (String, String) {
        match self.screen {
            Screen::Title => ("Start".to_string(), String::new()),
            Screen::Stories => ("Single player".to_string(), "Choose a story".to_string()),
            Screen::Multi => ("Multiplayer".to_string(), "Rooms of up to six players".to_string()),
            Screen::Create => ("Create a room".to_string(), "Choose the story the room plays".to_string()),
            Screen::Join => ("Join a room".to_string(), "Type the room code".to_string()),
            Screen::SignIn => ("Sign in".to_string(), "Your account".to_string()),
            Screen::Lobby => match &self.seat {
                Some(seat) => {
                    let story = self.pack.story(&seat.room.pack).map(|s| s.title.clone()).unwrap_or_else(|| seat.room.pack.clone());
                    (format!("Room {}", seat.code), story)
                }
                None => ("Room".to_string(), String::new()),
            },
        }
    }

    /// The notice as a toast panel above the key hints. It fades out a few seconds after it was set.
    fn draw_toast(&self, sw: f32, sh: f32) {
        let a = toast_alpha(self.notice_age);
        if self.notice.is_empty() || a <= 0.0 {
            return;
        }
        let color = if self.notice_error { ui::BAD } else { ui::GOOD };
        let max_w = sw - 80.0;
        let size = fit_size(17.0, 12.0, max_w - 40.0, |sz| ui::measure(&self.notice, sz));
        let tw = ui::measure(&self.notice, size);
        let (bw, bh) = ((tw + 44.0).min(max_w), 34.0);
        let r = Rect::new((sw - bw) / 2.0, sh - 40.0 - bh - 8.0, bw, bh);
        draw_rectangle(r.x + 3.0, r.y + 4.0, r.w, r.h, ui::fade(ui::SHADOW, a * 0.6));
        draw_rectangle(r.x, r.y, r.w, r.h, ui::fade(ui::PANEL, a));
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 1.0, ui::fade(color, a * 0.7));
        draw_rectangle(r.x, r.y, 4.0, r.h, ui::fade(color, a));
        ui::text(&self.notice, r.x + 22.0, r.y + r.h * 0.5 + size * 0.35, size, ui::fade(color, a));
    }

    /// The server's state (a chip) and who the player is, on the multiplayer screen. Returns the y below it.
    fn draw_multi_status(&self, x: f32, y: f32, w: f32) -> f32 {
        let (label, color, why) = match &self.health {
            None => ("CHECKING".to_string(), ui::MUTED, String::new()),
            Some(Ok(rooms)) => (format!("ONLINE, {rooms} ROOM(S)"), ui::GOOD, String::new()),
            Some(Err(why)) => ("OFFLINE".to_string(), ui::BAD, why.clone()),
        };
        let cw = chip(x, y, &label, color);
        if !why.is_empty() {
            let room = w - cw - 20.0;
            let size = fit_size(14.0, 11.0, room, |sz| ui::measure(&why, sz));
            ui::text(&why, x + cw + 10.0, y + 15.0, size, ui::DIM);
        }
        let who = match &self.who {
            Some(who) => format!("Playing as {}", who.label()),
            None => "No name yet: get a guest name to make or join a room".to_string(),
        };
        ui::text(&who, x, y + 44.0, 16.0, ui::MUTED);
        y + 60.0
    }

    /// The room code being typed, above the join rows: a framed field with a blinking caret. Returns the y below it.
    fn draw_code_field(&self, x: f32, y: f32, w: f32, s: f32) -> f32 {
        let h = (68.0 * s).max(60.0);
        let r = Rect::new(x, y, w, h);
        ui::panel(r, ui::PANEL, None);
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, ui::fade(ui::ACCENT, 0.8));
        ui::text("ROOM CODE", x + 16.0, y + 18.0, 12.0, ui::DIM);
        let shown = if self.join_code.is_empty() {
            "_ _ _ _ _".to_string()
        } else {
            self.join_code.chars().map(String::from).collect::<Vec<_>>().join(" ")
        };
        let size = 34.0 * s.min(1.2);
        let color = if self.join_code.is_empty() { ui::DIM } else { ui::TEXT };
        let tw = ui::measure(&shown, size);
        let base = y + h - 14.0;
        ui::text(&shown, x + w / 2.0 - tw / 2.0, base, size, color);
        if blink() {
            draw_rectangle(x + w / 2.0 + tw / 2.0 + 4.0, base - size * 0.75, 3.0, size * 0.85, ui::ACCENT);
        }
        y + h + 14.0
    }

    /// The lobby: the room code large, the story and player count, and the players in a panel. Returns the y below it.
    fn draw_lobby_players(&self, x: f32, y: f32, w: f32, s: f32) -> f32 {
        let Some(seat) = &self.seat else { return y };
        let mut y = y;

        // The room code, large, in its own frame (centred on the column).
        let bh = (70.0 * s).max(58.0);
        let bw = w.min(380.0 * s.max(1.0));
        let r = Rect::new(x + (w - bw) / 2.0, y, bw, bh);
        ui::panel(r, ui::PANEL, None);
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, ui::fade(ui::ACCENT, 0.8));
        ui::text_centered("ROOM CODE", r.x + r.w / 2.0, r.y + 16.0, 12.0, ui::DIM);
        let spaced = seat.code.chars().map(String::from).collect::<Vec<_>>().join(" ");
        let code_size = (bh - 26.0).min(46.0);
        ui::title_centered(&spaced, r.x + r.w / 2.0, r.y + bh - 10.0, code_size, ui::ACCENT);
        y += bh + 12.0;

        // The story on the left, the players count on the right.
        let story = self.pack.story(&seat.room.pack).map(|st| st.title.clone()).unwrap_or_else(|| seat.room.pack.clone());
        let count = format!("{} of {} players", seat.room.players.len(), seat.room.max_players);
        ui::text(&story, x, y + 12.0, 16.0, ui::MUTED);
        ui::text(&count, x + w - ui::measure(&count, 16.0), y + 12.0, 16.0, ui::TEXT);
        y += 22.0;

        // The player list.
        let prow = (34.0 * s).max(28.0);
        let panel_h = prow * seat.room.players.len() as f32 + 8.0;
        ui::panel(Rect::new(x, y, w, panel_h), ui::PANEL, None);
        for (i, p) in seat.room.players.iter().enumerate() {
            let ry = y + 4.0 + prow * i as f32;
            if i > 0 {
                draw_line(x + 12.0, ry, x + w - 12.0, ry, 1.0, ui::fade(ui::BORDER, 0.5));
            }
            let you = self.who.as_ref().is_some_and(|me| me.name() == p.name);
            let base = ry + prow * 0.5 + 6.0;
            ui::text(&p.name, x + 16.0, base, 19.0, if p.connected { ui::TEXT } else { ui::DIM });
            if you {
                ui::text("(you)", x + 16.0 + ui::measure(&p.name, 19.0) + 8.0, base, 15.0, ui::MUTED);
            }
            let mut tag_x = x + w - 12.0;
            let tags: [(bool, &str, Color); 3] = [(!p.connected, "OFFLINE", ui::BAD), (p.ready, "READY", ui::GOOD), (p.host, "HOST", ui::ACCENT)];
            for (on, text, color) in tags {
                if on {
                    let cw = ui::measure(text, 13.0) + 14.0;
                    tag_x -= cw;
                    chip(tag_x, ry + (prow - 20.0) / 2.0, text, color);
                    tag_x -= 6.0;
                }
            }
        }
        y += panel_h + 8.0;
        if seat.room.started {
            ui::text("Everyone is ready. Multiplayer play is not in the game yet, so the room waits here.", x, y + 12.0, 14.0, ui::ACCENT);
            y += 22.0;
        }
        y + 6.0
    }
}

/// Whether the text caret is showing (it blinks twice a second).
fn blink() -> bool {
    (get_time() * 2.0) as i64 % 2 == 0
}

/// A small label chip (top-left at x, y; 20 high). Returns its width.
fn chip(x: f32, y: f32, label: &str, color: Color) -> f32 {
    let size = 13.0;
    let w = ui::measure(label, size) + 14.0;
    draw_rectangle(x, y, w, 20.0, ui::fade(color, 0.16));
    draw_rectangle_lines(x, y, w, 20.0, 1.0, ui::fade(color, 0.8));
    ui::text(label, x + 7.0, y + 14.0, size, color);
    w
}

/// A framed text field: a small label, the value (or a dim placeholder) and, when active, a blinking caret.
fn draw_field(r: Rect, label: &str, value: &str, placeholder: &str, active: bool, glow: f32) {
    ui::panel(r, if active { ui::PANEL_LIT } else { ui::PANEL }, None);
    let frame = if active { ui::fade(ui::ACCENT, 0.6 + 0.4 * glow) } else { ui::BORDER };
    draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, frame);
    ui::text(&label.to_uppercase(), r.x + 16.0, r.y + 17.0, 12.0, ui::MUTED);
    let base = r.y + r.h - 10.0;
    let max_w = r.w - 44.0;
    let shown: String = if value.is_empty() {
        placeholder.to_string()
    } else {
        // A long value shows its end, where the typing is.
        let mut chars: Vec<char> = value.chars().collect();
        while chars.len() > 1 && ui::measure(&chars.iter().collect::<String>(), 20.0) > max_w {
            chars.remove(0);
        }
        chars.into_iter().collect()
    };
    let color = if value.is_empty() { ui::DIM } else { ui::TEXT };
    ui::text(&shown, r.x + 16.0, base, 20.0, color);
    if active && blink() {
        let cx = if value.is_empty() { r.x + 16.0 } else { r.x + 16.0 + ui::measure(&shown, 20.0) + 2.0 };
        draw_rectangle(cx, base - 16.0, 2.0, 20.0, ui::ACCENT);
    }
}

/// A story as a compact card: the era colour strip, a small era colour chip and the title. Details are in the side panel.
fn draw_story_card(story: &crate::model::StoryInfo, r: Rect, lit: bool, glow: f32) {
    let era = ui::era_color(story.era);
    let r = Rect::new(r.x + glow * 3.0, r.y, r.w, r.h);
    ui::panel(r, if lit { ui::PANEL_LIT } else { ui::PANEL }, None);
    draw_rectangle(r.x, r.y, 8.0, r.h, ui::fade(era, 0.45 + 0.55 * glow));
    if lit {
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, ui::fade(era, 0.5 + 0.5 * glow));
    }
    let x = r.x + 26.0;
    // A small era colour chip, then the title.
    let chip_size = 14.0;
    draw_rectangle(x, r.y + r.h / 2.0 - chip_size / 2.0 - 3.0, chip_size, chip_size, era);
    let tx = x + chip_size + 14.0;
    let avail = r.w - (tx - r.x) - 36.0;
    let size = fit_size(22.0, 14.0, avail, |sz| ui::measure(&story.title, sz));
    ui::text(&story.title, tx, r.y + r.h / 2.0 + size * 0.3 - 3.0, size, if lit { ui::ACCENT } else { ui::TEXT });
    if lit {
        ui::text(">", r.x + r.w - 28.0, r.y + r.h * 0.5 + 8.0, 24.0, ui::ACCENT);
    }
}

/// The side panel of the story list: the selected story's era, title, player and whether rooms can play it.
fn draw_story_info(r: Rect, story: Option<&crate::model::StoryInfo>) {
    let Some(story) = story else {
        ui::panel(r, ui::PANEL, None);
        ui::text("Back to the title screen", r.x + 22.0, r.y + 40.0, 18.0, ui::DIM);
        return;
    };
    let era = ui::era_color(story.era);
    ui::panel(r, ui::PANEL, Some(era));
    let x = r.x + 24.0;
    let mut cx = x + chip(x, r.y + 16.0, story.era.label(), era) + 8.0;
    if story.multiplayer {
        cx += chip(cx, r.y + 16.0, "MULTIPLAYER", ui::GOOD);
    }
    let _ = cx;
    let size = fit_size(34.0, 18.0, r.w - 48.0, |sz| ui::measure_title(&story.title, sz));
    ui::title(&story.title, x, r.y + 78.0, size, ui::TEXT);
    let mut y = r.y + 112.0;
    let mode = if story.multiplayer { "Alone, or in a room of up to six" } else { "Single player only" };
    let fields = [("YOU PLAY", who_plays(&story.player), ui::TEXT), ("ERA", story.era.label().to_string(), era), ("PLAYERS", mode.to_string(), ui::TEXT)];
    for (label, value, color) in fields {
        ui::text(label, x, y, 12.0, ui::DIM);
        let vs = fit_size(18.0, 12.0, r.w - 48.0, |sz| ui::measure(&value, sz));
        ui::text(&value, x, y + 22.0, vs, color);
        y += 54.0;
    }
}

/// The player's name and role, as the menu shows them: "Dia, test subject" or just "Dia".
fn who_plays(player: &crate::model::Player) -> String {
    if player.role.is_empty() {
        player.name.clone()
    } else {
        format!("{}, {}", player.name, player.role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glow_eases_toward_its_target_and_settles() {
        let up = ease_glow(0.0, 1.0, 0.016);
        assert!(up > 0.0 && up < 1.0);
        let mut g = 0.0;
        for _ in 0..120 {
            g = ease_glow(g, 1.0, 0.016);
        }
        assert_eq!(g, 1.0);
        for _ in 0..120 {
            g = ease_glow(g, 0.0, 0.016);
        }
        assert_eq!(g, 0.0);
    }

    #[test]
    fn glow_never_overshoots_on_a_long_frame() {
        assert_eq!(ease_glow(0.2, 1.0, 5.0), 1.0);
        assert_eq!(ease_glow(0.8, 0.0, 5.0), 0.0);
        assert_eq!(ease_glow(0.5, 1.0, 0.0), 0.5);
    }

    #[test]
    fn toast_is_solid_then_fades_then_gone() {
        assert_eq!(toast_alpha(0.0), 1.0);
        assert_eq!(toast_alpha(TOAST_SECONDS - TOAST_FADE_SECONDS), 1.0);
        let mid = toast_alpha(TOAST_SECONDS - TOAST_FADE_SECONDS / 2.0);
        assert!(mid > 0.0 && mid < 1.0);
        assert_eq!(toast_alpha(TOAST_SECONDS), 0.0);
        assert_eq!(toast_alpha(99.0), 0.0);
        assert_eq!(toast_alpha(-1.0), 0.0);
    }

    #[test]
    fn the_row_window_keeps_the_selection_in_view() {
        let steps = vec![80.0; 9];
        assert_eq!(visible_rows(&steps, 2000.0, 4), (0, 9), "all fit");
        let (a, b) = visible_rows(&steps, 400.0, 0);
        assert_eq!((a, b - a), (0, 5));
        let (a, b) = visible_rows(&steps, 400.0, 8);
        assert!(a <= 8 && 8 < b && b == 9 && b - a == 5, "{a}..{b}");
        let (a, b) = visible_rows(&steps, 400.0, 4);
        assert!(a <= 4 && 4 < b && b - a == 5);
        assert_eq!(visible_rows(&steps, 10.0, 3), (3, 4), "the selected row always shows");
        assert_eq!(visible_rows(&[], 100.0, 0), (0, 0));
    }

    #[test]
    fn the_title_has_four_one_word_buttons_with_settings() {
        let rows = title_rows();
        assert_eq!(rows.len(), 4);
        for (i, (id, label)) in rows.iter().enumerate() {
            assert_eq!(*id, i);
            assert!(!label.contains(' ') && *label == label.to_uppercase(), "{label}");
        }
        assert_eq!(rows[TITLE_SETTINGS].1, "SETTINGS");
        assert_eq!(rows[TITLE_QUIT].1, "QUIT");
    }

    #[test]
    fn polygon_points_lie_on_the_circle() {
        let pts = polygon_points(10.0, 20.0, 5.0, 0.7, 6);
        assert_eq!(pts.len(), 6);
        for (x, y) in pts {
            let d = ((x - 10.0).powi(2) + (y - 20.0).powi(2)).sqrt();
            assert!((d - 5.0).abs() < 1e-4);
        }
    }

    #[test]
    fn screen_fade_runs_zero_to_one() {
        assert_eq!(screen_fade(0.0), 0.0);
        assert_eq!(screen_fade(SCREEN_FADE_SECONDS), 1.0);
        assert_eq!(screen_fade(10.0), 1.0);
        assert!(screen_fade(SCREEN_FADE_SECONDS / 2.0) > 0.4);
    }

    #[test]
    fn ui_scale_is_clamped_for_small_and_large_screens() {
        assert_eq!(ui_scale(400.0), 0.82);
        assert_eq!(ui_scale(720.0), 1.0);
        assert_eq!(ui_scale(1080.0), 1.5);
        assert_eq!(ui_scale(4000.0), 1.5);
    }

    #[test]
    fn fit_size_shrinks_until_it_fits_or_hits_the_floor() {
        // a text 10 px wide per point of size
        let width = |sz: f32| sz * 10.0;
        assert_eq!(fit_size(22.0, 12.0, 1000.0, width), 22.0);
        assert_eq!(fit_size(22.0, 12.0, 150.0, width), 15.0);
        assert_eq!(fit_size(22.0, 12.0, 10.0, width), 12.0);
    }
}
