//! The menus, laid out like the original's screens on its 480x270 design canvas (see style.rs):
//! the main menu (main_menu.tscn / main_menu.gd) with its CREATE ROOM, JOIN ROOM and QUIT
//! dialogs, the account panel (account_panel.gd), the route loading screen (route_loading.gd),
//! room staging (room_lobby.gd) and its hunter dossier.

use super::settings_panel::tab_bar;
use super::style::*;
use super::text_field;
use crate::app::assets::Assets;
use crate::app::checkpoint::Checkpoint;
use crate::config::difficulty::LOBBY;
use crate::config::Difficulty;
use crate::content::hunters::HunterDef;
use crate::content::Content;
use crate::net::hosted::{self, OwnerAction};
use macroquad::prelude::*;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Screen {
    Title,
    Create,
    Join,
    Quit,
    Account,
    Settings,
    Lobby,
    Dossier,
    /// The title's ADMIN panel (hub_admin_panel.gd, ui/admin).
    Admin,
}

pub enum Action {
    None,
    Go(Screen),
    StartRun,
    ContinueRun,
    ClearSave,
    Reroll,
    Quit,
    /// CREATE ROOM, online by default: ask the server to open a room, wait for it, and join it as its owner (a guest).
    CreateOnline,
    /// A room-owner command (START, END, RESTART, KICK, difficulty) for the room the owner is in.
    Owner(OwnerAction),
    /// The code the JOIN ROOM dialog submitted (already checked by `world_sync::parse_join_code`).
    JoinCode(String),
    /// PUBLIC ROUTE: join the always-open public room, or open it when nobody holds it (`world_sync::join_public_room`).
    PublicRoute,
    /// SIGN IN with the typed name and password; the app sends them to the server.
    SignIn { name: String, password: String },
    /// CREATE ACCOUNT with the typed name and password; the app sends them to the server.
    CreateAccount { name: String, password: String },
    /// SIGN OUT: the app revokes the session and clears the profile name.
    SignOut,
}

/// Which room the lobby is showing: the solo room of CREATE ROOM, a room this player hosts, or one it joined.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoomMode {
    #[default]
    Solo,
    Host,
    Guest,
}

/// Whether the game offers QUIT (the title button, its QUIT TO DESKTOP? dialog and Escape on the title). The browser
/// build has no desktop to quit to, so it offers none of them and the page stays open.
pub const QUIT_OFFERED: bool = !cfg!(target_arch = "wasm32");

/// The title's buttons. ADMIN shows for the admin's account only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TitleEntry {
    /// PUBLIC ROUTE: joins the always-open public room, or opens it (`Action::PublicRoute`).
    PublicRoute,
    Create,
    Join,
    Settings,
    Account,
    Admin,
    Quit,
}

/// What pressing a title button does. PUBLIC ROUTE asks the app to join or open the public room.
pub fn title_entry_action(entry: TitleEntry) -> Action {
    match entry {
        TitleEntry::PublicRoute => Action::PublicRoute,
        TitleEntry::Create => Action::Go(Screen::Create),
        TitleEntry::Join => Action::Go(Screen::Join),
        TitleEntry::Settings => Action::Go(Screen::Settings),
        TitleEntry::Account => Action::Go(Screen::Account),
        TitleEntry::Admin => Action::Go(Screen::Admin),
        TitleEntry::Quit => Action::Go(Screen::Quit),
    }
}

/// The title's buttons in order for this account (the web title's order: PUBLIC ROUTE,
/// CREATE ROOM, JOIN ROOM, then ACCOUNT, SETTINGS and QUIT; ADMIN sits before QUIT).
pub fn title_entries(username: &str) -> Vec<TitleEntry> {
    let mut v = vec![
        TitleEntry::PublicRoute,
        TitleEntry::Create,
        TitleEntry::Join,
        TitleEntry::Account,
        TitleEntry::Settings,
        TitleEntry::Admin,
        TitleEntry::Quit,
    ];
    if !crate::ui::admin::is_admin_owner(username) {
        v.retain(|e| *e != TitleEntry::Admin);
    }
    if !QUIT_OFFERED {
        v.retain(|e| *e != TitleEntry::Quit);
    }
    v
}

/// The focus index of a title button for this account (the dialogs hand focus back to their button).
pub fn title_focus(entry: TitleEntry, username: &str) -> usize {
    title_entries(username).iter().position(|e| *e == entry).unwrap_or(0)
}

/// The scroll step for one frame's mouse-wheel delta: `px` per notch, sign taken from the wheel,
/// and nothing when the wheel did not move. `signum(0.0)` is 1.0, so without this guard a resting
/// mouse over a list would drift it every frame.
pub fn wheel_notch(wheel: f32, px: f32) -> f32 {
    if wheel == 0.0 {
        0.0
    } else {
        wheel.signum() * px
    }
}

pub const VERSION: &str = "VERSION 0.0.1.0";

/// What the lobby and the pause menu show of an online room the server hosts: the owner controls and the wait for the room
/// to open. The app sets it every frame from the open session (`net/hosted.rs`); nothing here is trusted by the server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OwnerView {
    /// This runner owns the open online room (the signed-in account that created it): the controls show. Anybody else
    /// never sees them.
    pub owner: bool,
    /// This runner has joined the room (the controls need the room to send to). False while the room is still opening.
    pub joined: bool,
    /// The other runners of the room with their session ids (KICK names a guest by id).
    pub peers: Vec<(u32, String)>,
    /// The room is being opened by the server: the two lines the lobby shows while it waits.
    pub waiting: Option<(String, String)>,
    /// The owner's feedback line (confirming, refused).
    pub status: String,
}

impl OwnerView {
    /// The session id of the room's runner called `name`, for KICK. None for this runner, the host and any name that is
    /// not in the room.
    pub fn peer_of(&self, name: &str) -> Option<u32> {
        self.peers.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(p, _)| *p)
    }
}

/// Menu state that lives across frames (focus, fields, scroll, dialogs).
pub struct Ui {
    pub screen: Screen,
    pub focus: usize,
    pub settings_tab: usize,
    pub controls_tab: usize,
    pub scroll: f32,
    pub join_code: String,
    pub join_status: String,
    /// The JOIN dialog opened by JOIN BY CODE submits a networked join (not the offline message).
    pub join_net: bool,
    /// The room the lobby shows (see [`RoomMode`]).
    pub room_mode: RoomMode,
    /// The lobby's COPY CODE was just used (shows COPIED TO CLIPBOARD until the screen changes).
    pub code_copied: bool,
    /// Where the account screen's back button goes: the screen it was opened from (the lobby keeps its room).
    pub account_back: Screen,
    pub create_offline: bool,
    pub create_note: String,
    /// The player is signed in to an account (CREATE ROOM online needs one). Set by the app each frame.
    pub signed_in: bool,
    /// The owner controls of the open online room (see [`OwnerView`]). Set by the app each frame.
    pub owner: OwnerView,
    pub acct_manage: bool,
    pub acct_user: String,
    pub acct_pass: String,
    pub acct_field: usize,
    /// SELECT ALL (Ctrl/Cmd+A) was used in the focused text field: the next typed character or paste replaces its text.
    pub field_sel: bool,
    pub acct_status: String,
    /// A sign in or account creation is waiting on the server (the form shows SIGNING IN).
    pub acct_busy: bool,
    pub confirm_clear: bool,
    pub dossier_sel: usize,
    pub dossier_scroll: f32,
    /// The dossier tab: 0 Route Dossier (built-in hunters), 1 Custom Hunters (mod packs).
    pub dossier_tab: usize,
    pub drag: Option<u32>,
    pub notice: String,
    pub notice_t: f32,
    pub t: f32,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            screen: Screen::Title,
            focus: 0,
            settings_tab: 0,
            controls_tab: 0,
            scroll: 0.0,
            join_code: String::new(),
            join_status: String::new(),
            join_net: true,
            room_mode: RoomMode::Solo,
            code_copied: false,
            account_back: Screen::Title,
            create_offline: false,
            create_note: String::new(),
            signed_in: false,
            owner: OwnerView::default(),
            acct_manage: false,
            acct_user: String::new(),
            acct_pass: String::new(),
            acct_field: 0,
            field_sel: false,
            acct_status: String::new(),
            acct_busy: false,
            confirm_clear: false,
            dossier_sel: 0,
            dossier_scroll: 0.0,
            dossier_tab: 0,
            drag: None,
            notice: String::new(),
            notice_t: 0.0,
            t: 0.0,
        }
    }
}

impl Ui {
    /// Moves to another screen with a fresh focus (a dialog's default button).
    pub fn go(&mut self, s: Screen) {
        // no quit dialog where quitting is not offered (QUIT_OFFERED): it lands on the title
        let s = if s == Screen::Quit && !QUIT_OFFERED { Screen::Title } else { s };
        self.screen = s;
        // keys typed in the game or on another screen wait in the character queue; they must not land in the next text
        // field (the JOIN ROOM code filled itself with the game's WASD presses)
        #[cfg(not(test))]
        {
            while get_char_pressed().is_some() {}
            // and a paste still waiting from before (the web page keeps it for a moment) is not for the next field either
            let _ = super::clipboard::poll_paste(false);
        }
        self.field_sel = false;
        self.focus = 0;
        self.scroll = 0.0;
        self.drag = None;
        self.code_copied = false;
        match s {
            Screen::Create => {
                self.create_offline = false;
                self.create_note.clear();
                self.focus = 2;
            }
            Screen::Join => {
                self.join_code.clear();
                self.join_status.clear();
                self.join_net = true;
            }
            Screen::Quit => self.focus = 1,
            Screen::Account => {
                self.acct_manage = false;
                self.confirm_clear = false;
                self.acct_status.clear();
                self.acct_pass.clear();
            }
            Screen::Dossier => {
                // every opening starts on the Route Dossier tab, at its first hunter
                self.dossier_tab = 0;
                self.dossier_sel = 0;
            }
            _ => {}
        }
    }
}

/// The caption colour main_menu.tscn writes for the kicker: web amber #f2bb62 on Last Route.
fn kicker() -> Color {
    lr(hex(0xf2bb62), rgba(0.788, 0.663, 0.847, 1.0))
}

/// The title's wordmark, in two lines as the web title wraps it at its size.
const TITLE_LINES: [&str; 2] = ["THE LAST", "ROUTE"];
/// The small kicker above the account screen's modal title, drawn and measured in every account dialog.
const ACCOUNT_KICKER: &str = "KYTTU: THE LAST ROUTE";

/// The ACCOUNT button's caption: ACCOUNT once signed in (the web's account button), otherwise
/// REGISTER / SIGN IN.
pub fn account_label(username: &str) -> &'static str {
    if username.is_empty() {
        "REGISTER / SIGN IN"
    } else {
        "ACCOUNT"
    }
}

/// The ADMIN box's caption colour: the web's developer-toggle purple (#c9a7ff).
const ADMIN_PURPLE: Color = hex(0xc9a7ff);

/// The title's ADMIN box in the top-right corner, drawn like the web's corner-button
/// developer-toggle: a dark translucent face with a hairline edge and a purple caption.
/// Returns true when clicked.
fn admin_corner_button(a: &Assets, r: Rect, text: &str, focused: bool) -> bool {
    let p = pal();
    let hover = hovered(r);
    let face = if hover {
        lightened(hexa(0x0403058c), 0.12)
    } else {
        hexa(0x0403058c)
    };
    let edge = if focused {
        focus_edge_colour()
    } else {
        hexa(0xffffff24)
    };
    sbox(r, face, edge, 1.0, p.corner);
    let size = 5.0;
    let tx = r.x + (r.w - tw(a, F::Body, text, size)) * 0.5;
    label_vc(a, F::Body, text, tx, r.y, r.h, size, ADMIN_PURPLE);
    let pressed = hover && is_mouse_button_pressed(MouseButton::Left);
    if pressed {
        crate::audio::sound::menu_blip();
    }
    pressed
}

// -- main menu ------------------------------------------------------------------------------

/// The main menu. `active` is false while a dialog sits over it (no focus, no input).
pub fn draw_title(a: &Assets, ui: &mut Ui, username: &str, active: bool) -> Action {
    let _scope = MenuScope::enter();
    let p = pal();
    backdrop(1.0);
    let v = view();
    let c = v * 0.5;
    let x0 = c.x - 155.0;
    // THE LAST ROUTE: two lines of Impact at the web's title size (menu_size), left-aligned in the
    // centred title column like the web's title-copy. Its bottom line ends just above the lede.
    let size = menu_size(28.0, 1.20);
    let step = size * 0.88;
    let top = c.y - 2.0 - step - size * 0.9;
    // the kicker sits above the wordmark's top, so the wordmark never covers it
    label(
        a,
        F::Body,
        "SIGNAL ARCHIVE // ROUTE UNSTABLE",
        x0,
        top - 22.0,
        5.0,
        kicker(),
    );
    let tshadow = lr(hexa(0x210e1beb), rgba(0.27, 0.17, 0.32, 0.92));
    for (i, line) in TITLE_LINES.iter().enumerate() {
        let y = top + step * i as f32;
        label_outline(a, F::Title, line, x0 + 4.0, y + 4.0, size, tshadow, tshadow, 1.0);
    }
    for (i, line) in TITLE_LINES.iter().enumerate() {
        let y = top + step * i as f32;
        label_outline(
            a,
            F::Title,
            line,
            x0,
            y,
            size,
            p.ink,
            lr(hex(0x210e1b), rgba(0.1, 0.05, 0.08, 1.0)),
            1.0,
        );
    }
    // the web title's lede and route notice, wrapped to the title column (one line per row)
    let lede = "The facility expands as you explore. Recover signal relays, search for supplies, keep the signal alive, and do whatever you can to avoid being caught.";
    let route_notice = "Open a room to run solo or with others — every room rebuilds the facility from a fresh signal.";
    let mut text_y = c.y + 26.0;
    for line in wrap(a, F::Body, lede, 310.0, 7.0) {
        label(
            a,
            F::Body,
            &line,
            x0,
            text_y,
            7.0,
            lr(hex(0xbbb0ad), rgba(0.72, 0.68, 0.66, 1.0)),
        );
        text_y += lh(F::Body, 7.0);
    }
    text_y += 4.0;
    for line in wrap(a, F::Body, route_notice, 310.0, 6.0) {
        label(a, F::Body, &line, x0, text_y, 6.0, p.muted);
        text_y += lh(F::Body, 6.0);
    }
    // the button rows: PUBLIC ROUTE, CREATE ROOM, JOIN ROOM (primary) on the first row,
    // ACCOUNT, SETTINGS and QUIT (secondary) on the second (all compact). ADMIN is not in the rows:
    // it sits in the top-right corner (see admin_corner_button) but keeps its focus index.
    let account = account_label(username);
    // ADMIN shows for the admin's account only (GameHub.admin_entry_visible)
    let entries = title_entries(username);
    let items: Vec<(&str, f32, Kind)> = entries
        .iter()
        .map(|e| match e {
            TitleEntry::PublicRoute => ("PUBLIC ROUTE", menu_size(6.0, 1.0), Kind::Primary),
            TitleEntry::Create => ("CREATE ROOM", menu_size(6.0, 1.0), Kind::Primary),
            TitleEntry::Join => ("JOIN ROOM", menu_size(6.0, 1.0), Kind::Primary),
            TitleEntry::Settings => ("SETTINGS", 6.0, Kind::Secondary),
            TitleEntry::Account => (account, 6.0, Kind::Secondary),
            TitleEntry::Admin => ("ADMIN", 5.0, Kind::Secondary),
            TitleEntry::Quit => ("QUIT", 6.0, Kind::Secondary),
        })
        .collect();
    let row_h = items
        .iter()
        .map(|i| button_min(a, i.0, i.1, true).y)
        .fold(0.0, f32::max);
    let enter = active && nav(&mut ui.focus, items.len(), false);
    let mut out = Action::None;
    let (mut bx, mut by) = (x0, text_y + 10.0);
    // the kind of the last button placed in a row (a change from primary to secondary starts a row)
    let mut row_kind: Option<Kind> = None;
    for (i, (text, fs, kind)) in items.iter().enumerate() {
        let hit = if entries[i] == TitleEntry::Admin {
            let m = button_min(a, text, *fs, true);
            let r = Rect::new(v.x - m.x - 12.0, 10.0, m.x, m.y);
            admin_corner_button(a, r, text, active && ui.focus == i) && active
        } else {
            let w = button_min(a, text, *fs, true).x;
            let new_row = bx + w > x0 + 310.0 + 0.01 || row_kind.is_some_and(|k| k != *kind);
            if new_row && bx > x0 {
                bx = x0;
                by += row_h + 4.0;
            }
            let r = Rect::new(bx, by, w, row_h);
            let hit = button(a, r, text, *fs, *kind, state(active && ui.focus == i)) && active;
            row_kind = Some(*kind);
            bx += w + 5.0;
            hit
        };
        if hit {
            ui.focus = i;
        }
        if hit || (enter && ui.focus == i) {
            out = title_entry_action(entries[i]);
        }
    }
    let vw = tw(a, F::Body, VERSION, 6.0);
    label(
        a,
        F::Body,
        VERSION,
        c.x - vw * 0.5,
        v.y - 18.0,
        6.0,
        p.muted,
    );
    if ui.notice_t > 0.0 {
        notice(a, &ui.notice);
    }
    if active && QUIT_OFFERED && is_key_pressed(KeyCode::Escape) {
        out = Action::Go(Screen::Quit);
    }
    out
}

/// A dialog action's font: primary labels keep their pre-scale pixel size (menu_size, ratio 1.00).
fn action_size(kind: Kind) -> f32 {
    if kind == Kind::Primary {
        menu_size(7.0, 1.0)
    } else {
        7.0
    }
}

/// The height of a dialog's action row, tall enough for a primary label.
fn dialog_action_h(a: &Assets) -> f32 {
    button_min(a, "X", 7.0, false)
        .y
        .max(button_min(a, "X", menu_size(7.0, 1.0), false).y)
}

/// The actions row of a menu dialog (right-aligned, separation 5, font 7, full margins).
fn dialog_actions(
    a: &Assets,
    right: f32,
    y: f32,
    items: &[(&str, Kind)],
    focus: usize,
) -> Option<usize> {
    let h = dialog_action_h(a);
    let total: f32 = items
        .iter()
        .map(|(t, k)| button_min(a, t, action_size(*k), false).x)
        .sum::<f32>()
        + 5.0 * (items.len() as f32 - 1.0);
    let mut x = right - total;
    let mut hit = None;
    for (i, (text, kind)) in items.iter().enumerate() {
        let w = button_min(a, text, action_size(*kind), false).x;
        if button(
            a,
            Rect::new(x, y, w, h),
            text,
            action_size(*kind),
            *kind,
            state(focus == i),
        ) {
            hit = Some(i);
        }
        x += w + 5.0;
    }
    hit
}

/// CREATE ROOM (main_menu.gd _open_create_dialog): the OFFLINE toggle, its note and the actions.
pub fn draw_create(a: &Assets, ui: &mut Ui, username: &str) -> Action {
    let _scope = MenuScope::enter();
    let p = pal();
    draw_title(a, ui, username, false);
    let note = if !ui.create_note.is_empty() {
        ui.create_note.clone()
    } else if ui.create_offline {
        "SOLO ROOM ON THIS DEVICE. NO SERVER, NO CO-OP. THIS ONLY LASTS UNTIL YOU LEAVE THE ROOM."
            .to_string()
    } else if !ui.signed_in {
        // an online room belongs to an account: say so before the player presses CREATE ROOM
        hosted::rules().strings.sign_in_required.clone()
    } else {
        "ONLINE ROOM: THE SERVER RUNS IT, YOU OWN IT, FRIENDS JOIN WITH ITS CODE.".to_string()
    };
    // the panel's width comes from the kicker row; the note wraps inside it
    let kick_w = tw(a, F::Body, &track("ROUTE SIGNAL // NEW ROOM"), 6.0) + esc_close_size(a).x;
    let inner_w = kick_w.max(196.0);
    let note_lines = wrap(a, F::Body, &note, inner_w, 6.0);
    let act_h = dialog_action_h(a);
    let content_h = 16.0 + 5.0 + note_lines.len() as f32 * lh(F::Body, 6.0) + 5.0 + act_h;
    let m = modal(
        a,
        "ROUTE SIGNAL // NEW ROOM",
        "CREATE ROOM",
        15.0,
        vec2(220.0, 110.0),
        inner_w,
        content_h,
    );
    let enter = nav(&mut ui.focus, 3, false);
    let toggle = Rect::new(m.inner.x, m.y, m.inner.w, 16.0);
    if state_toggle(
        a,
        toggle,
        "OFFLINE (SOLO, NO SERVER)",
        ui.create_offline,
        true,
        state(ui.focus == 0),
    ) || (enter && ui.focus == 0)
    {
        ui.create_offline = !ui.create_offline;
        ui.create_note.clear();
        ui.focus = 0;
    }
    let mut y = toggle.y + toggle.h + 5.0;
    for l in &note_lines {
        label(a, F::Body, l, m.inner.x, y, 6.0, p.amber);
        y += lh(F::Body, 6.0);
    }
    let hit = dialog_actions(
        a,
        m.inner.x + m.inner.w,
        m.inner.y + m.inner.h - act_h,
        &[("CANCEL", Kind::Secondary), ("CREATE ROOM", Kind::Primary)],
        ui.focus.wrapping_sub(1),
    );
    let pick = hit.map(|i| i + 1).or(if enter && ui.focus > 0 {
        Some(ui.focus)
    } else {
        None
    });
    if m.close || is_key_pressed(KeyCode::Escape) || pick == Some(1) {
        return Action::Go(Screen::Title);
    }
    if pick == Some(2) {
        if ui.create_offline {
            return Action::Go(Screen::Lobby);
        }
        // online by default: the server opens the room (a failure, or no account, is shown by the app)
        return Action::CreateOnline;
    }
    Action::None
}

/// JOIN ROOM (main_menu.gd _on_join_pressed): the code field, its hint and status.
pub fn draw_join(a: &Assets, ui: &mut Ui, username: &str) -> Action {
    let _scope = MenuScope::enter();
    let p = pal();
    draw_title(a, ui, username, false);
    // typing, Backspace, and Ctrl/Cmd + V / C / X / A (a pasted code replaces what is there)
    text_field::edit(text_field::Kind::Code, &mut ui.join_code, &mut ui.field_sel);
    let kick_w = tw(a, F::Body, &track("ROUTE SIGNAL // ROOM ACCESS"), 6.0) + esc_close_size(a).x;
    let inner_w = kick_w.max(196.0);
    let hint = wrap(
        a,
        F::Body,
        "FIVE-CHARACTER ROOM CODE, E.G. ABC12",
        inner_w,
        6.0,
    );
    let status = if ui.join_status.is_empty() {
        vec![]
    } else {
        wrap(a, F::Body, &ui.join_status, inner_w, 6.0)
    };
    let act_h = dialog_action_h(a);
    let lines = hint.len() + status.len().max(1);
    let content_h = 27.0 + 5.0 + lines as f32 * lh(F::Body, 6.0) + 10.0 + act_h;
    let m = modal(
        a,
        "ROUTE SIGNAL // ROOM ACCESS",
        "ENTER ROOM CODE",
        15.0,
        vec2(220.0, 112.0),
        inner_w,
        content_h,
    );
    let field = Rect::new(m.inner.x, m.y, m.inner.w, 27.0);
    line_edit(a, field, &ui.join_code, "ABC12", 12.0, true, false, ui.t);
    if ui.field_sel {
        line_edit_selection(a, field, &ui.join_code, false, 12.0);
    }
    let mut y = field.y + field.h + 5.0;
    for l in &hint {
        label(a, F::Body, l, m.inner.x, y, 6.0, p.muted);
        y += lh(F::Body, 6.0);
    }
    y += 5.0;
    for l in &status {
        label(a, F::Body, l, m.inner.x, y, 6.0, p.alert);
        y += lh(F::Body, 6.0);
    }
    let hit = dialog_actions(
        a,
        m.inner.x + m.inner.w,
        m.inner.y + m.inner.h - act_h,
        &[("CANCEL", Kind::Secondary), ("JOIN ROOM", Kind::Primary)],
        9,
    );
    if m.close || is_key_pressed(KeyCode::Escape) || hit == Some(0) {
        return Action::Go(Screen::Title);
    }
    if hit == Some(1) || is_key_pressed(KeyCode::Enter) {
        if ui.join_net {
            return match crate::net::world_sync::parse_join_code(&ui.join_code) {
                Some(code) => Action::JoinCode(code),
                // an owner's invalid or empty code becomes a valid code (api::admin_code_from_input) and is hosted
                None if crate::ui::admin::is_admin_owner(username) => {
                    let alpha = crate::net::api::CODE_ALPHABET.as_bytes();
                    let code = crate::net::api::admin_code_from_input(&ui.join_code, || {
                        alpha[crate::net::random_below(alpha.len())] as char
                    });
                    Action::JoinCode(code)
                }
                None => {
                    ui.join_status = "ENTER A VALID FIVE-CHARACTER ROOM CODE".into();
                    Action::None
                }
            };
        }
        ui.join_status = if ui.join_code.len() != 5 {
            "ENTER A VALID FIVE-CHARACTER ROOM CODE".into()
        } else {
            "OFFLINE MODE // GO ONLINE TO JOIN ROOMS".into()
        };
    }
    Action::None
}

/// QUIT TO DESKTOP? (quit_confirm.gd): YES | NO, NO focused.
pub fn draw_quit(a: &Assets, ui: &mut Ui, username: &str) -> Action {
    let _scope = MenuScope::enter();
    let p = pal();
    draw_title(a, ui, username, false);
    let v = view();
    fill(Rect::new(0.0, 0.0, v.x, v.y), rgba(0.0, 0.0, 0.0, 0.45));
    let q = "QUIT TO DESKTOP?";
    let yes = button_min(a, "YES", 8.0, true);
    let no = button_min(a, "NO", 8.0, true);
    let row_w = yes.x + 6.0 + no.x;
    let inner_w = tw(a, F::Body, q, 8.0).max(row_w);
    let h = 20.0 + lh(F::Body, 8.0) + 6.0 + yes.y;
    let panel = centered(inner_w + 20.0, h);
    panel_box(panel, p.panel, p.accent_edge);
    label_c(
        a,
        F::Body,
        q,
        panel.x + panel.w * 0.5,
        panel.y + 10.0,
        8.0,
        p.ink,
    );
    let ry = panel.y + 10.0 + lh(F::Body, 8.0) + 6.0;
    let rx = panel.x + (panel.w - row_w) * 0.5;
    let enter = nav(&mut ui.focus, 2, false);
    let y_hit = button(
        a,
        Rect::new(rx, ry, yes.x, yes.y),
        "YES",
        8.0,
        Kind::Secondary,
        state(ui.focus == 0),
    );
    let n_hit = button(
        a,
        Rect::new(rx + yes.x + 6.0, ry, no.x, no.y),
        "NO",
        8.0,
        Kind::Secondary,
        state(ui.focus == 1),
    );
    if y_hit || (enter && ui.focus == 0) {
        return Action::Quit;
    }
    let pressed = [MouseButton::Left, MouseButton::Right, MouseButton::Middle]
        .into_iter()
        .any(is_mouse_button_pressed);
    let outside = pressed && !hovered(panel);
    if n_hit || (enter && ui.focus == 1) || is_key_pressed(KeyCode::Escape) || outside {
        return Action::Go(Screen::Title);
    }
    Action::None
}

// -- account (account_panel.gd) -------------------------------------------------------------

fn valid_name(n: &str) -> bool {
    (3..=24).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The run summary GameRegistry.save_summary shows: DEPTH n // SCORE n // DIFFICULTY.
pub fn save_summary(save: Option<&Checkpoint>) -> Option<String> {
    save.map(|c| {
        format!(
            "DEPTH {} // SCORE {} // {}",
            if c.infinite { 1 - c.level() } else { c.depth },
            c.score,
            c.difficulty.name()
        )
    })
}

/// The account panel. Signed in (a profile name is set) it is the account card with the SAVES
/// rows; signed out it is the sign-in form. SIGN IN and CREATE ACCOUNT return an action: the app
/// sends the credentials to the server, and the profile name is set only once the server confirms.
pub fn draw_account(
    a: &Assets,
    ui: &mut Ui,
    username: &mut String,
    save: Option<&Checkpoint>,
) -> Action {
    let _scope = MenuScope::enter();
    let p = pal();
    draw_title(a, ui, username, false);
    let btn_h = dialog_action_h(a);
    let mut out = Action::None;
    let kick_w = tw(a, F::Body, &track(ACCOUNT_KICKER), 6.0) + esc_close_size(a).x;
    if username.is_empty() {
        // the sign-in form (240x150)
        if is_key_pressed(KeyCode::Tab) {
            ui.acct_field = (ui.acct_field + 1) % 2;
            ui.field_sel = false;
        }
        // the focused field takes the typing and the clipboard keys; the password is never copied or cut
        if ui.acct_field == 0 {
            text_field::edit(text_field::Kind::Username, &mut ui.acct_user, &mut ui.field_sel);
        } else {
            text_field::edit(text_field::Kind::Password, &mut ui.acct_pass, &mut ui.field_sel);
        }
        let status = if ui.acct_busy {
            "SIGNING IN".to_string()
        } else if ui.acct_status.is_empty() {
            "NOT SIGNED IN // PLAY WITHOUT AN ACCOUNT OR SIGN IN TO SAVE ACROSS DEVICES".to_string()
        } else {
            ui.acct_status.clone()
        };
        let inner_w = kick_w.max(216.0);
        let lines = wrap(a, F::Body, &status, inner_w, 6.0);
        let content_h =
            lines.len() as f32 * lh(F::Body, 6.0) + 5.0 + 22.0 + 5.0 + 22.0 + 5.0 + btn_h;
        let m = modal(
            a,
            ACCOUNT_KICKER,
            "ACCOUNT",
            14.0,
            vec2(240.0, 150.0),
            inner_w,
            content_h,
        );
        let mut y = m.y;
        for l in &lines {
            label(a, F::Body, l, m.inner.x, y, 6.0, p.muted);
            y += lh(F::Body, 6.0);
        }
        y += 5.0;
        let f1 = Rect::new(m.inner.x, y, m.inner.w, 22.0);
        line_edit(
            a,
            f1,
            &ui.acct_user,
            "USERNAME // 3-24 CHARS: A-Z 0-9 _",
            8.0,
            ui.acct_field == 0,
            false,
            ui.t,
        );
        let f2 = Rect::new(m.inner.x, y + 27.0, m.inner.w, 22.0);
        line_edit(
            a,
            f2,
            &ui.acct_pass,
            "PASSWORD // 6+ CHARACTERS",
            8.0,
            ui.acct_field == 1,
            true,
            ui.t,
        );
        if ui.field_sel {
            if ui.acct_field == 0 {
                line_edit_selection(a, f1, &ui.acct_user, false, 8.0);
            } else {
                line_edit_selection(a, f2, &ui.acct_pass, true, 8.0);
            }
        }
        if clicked(f1) {
            if ui.acct_field != 0 {
                ui.field_sel = false;
            }
            ui.acct_field = 0;
        }
        if clicked(f2) {
            if ui.acct_field != 1 {
                ui.field_sel = false;
            }
            ui.acct_field = 1;
        }
        y += 54.0;
        let w1 = button_min(a, "SIGN IN", action_size(Kind::Primary), false).x;
        let w2 = button_min(a, "CREATE ACCOUNT", 7.0, false).x;
        let sign = button(
            a,
            Rect::new(m.inner.x, y, w1, btn_h),
            "SIGN IN",
            action_size(Kind::Primary),
            Kind::Primary,
            plain(),
        );
        let make = button(
            a,
            Rect::new(m.inner.x + w1 + 5.0, y, w2, btn_h),
            "CREATE ACCOUNT",
            7.0,
            Kind::Secondary,
            plain(),
        );
        if (sign || make || is_key_pressed(KeyCode::Enter)) && !ui.acct_busy {
            if make && crate::net::guest_name::reserved_for_guests(&ui.acct_user) {
                // GUEST_1234 is what a runner who is not signed in is called: an account may not pose as one
                ui.acct_status = "THAT NAME IS RESERVED FOR GUESTS".into();
            } else if valid_name(&ui.acct_user) {
                // the password leaves the form here and is not kept; the app sets the name once the server confirms
                let name = ui.acct_user.clone();
                let password = std::mem::take(&mut ui.acct_pass);
                ui.acct_status.clear();
                out = if make {
                    Action::CreateAccount { name, password }
                } else {
                    Action::SignIn { name, password }
                };
            } else {
                ui.acct_status = "USERNAME MUST BE 3-24 CHARACTERS: A-Z 0-9 _".into();
            }
        }
        if m.close || is_key_pressed(KeyCode::Escape) {
            out = Action::Go(ui.account_back);
        }
        return out;
    }
    if ui.acct_manage {
        // ACCOUNT SETTINGS (280x214): rename the local profile; passwords need the server
        text_field::edit(text_field::Kind::Username, &mut ui.acct_user, &mut ui.field_sel);
        let status = if ui.acct_status.is_empty() {
            format!("SIGNED IN AS {username}")
        } else {
            ui.acct_status.clone()
        };
        let l6 = lh(F::Body, 6.0);
        let content_h = l6 * 3.0 + 22.0 * 2.0 + btn_h * 3.0 + 7.0 * 5.0;
        let m = modal(
            a,
            ACCOUNT_KICKER,
            "ACCOUNT SETTINGS",
            14.0,
            vec2(280.0, 214.0),
            256.0,
            content_h,
        );
        let w = |t: &str| button_min(a, t, 7.0, false).x;
        let mut y = m.y;
        label(a, F::Body, &status, m.inner.x, y, 6.0, p.muted);
        y += l6 + 5.0;
        label(a, F::Body, "USERNAME", m.inner.x, y, 6.0, p.amber);
        y += l6 + 5.0;
        let name_box = Rect::new(m.inner.x, y, m.inner.w, 22.0);
        line_edit(a, name_box, &ui.acct_user, "", 8.0, true, false, ui.t);
        if ui.field_sel {
            line_edit_selection(a, name_box, &ui.acct_user, false, 8.0);
        }
        y += 27.0;
        if button(
            a,
            Rect::new(m.inner.x, y, m.inner.w, btn_h),
            "UPDATE USERNAME",
            7.0,
            Kind::Secondary,
            plain(),
        ) || is_key_pressed(KeyCode::Enter)
        {
            if valid_name(&ui.acct_user) {
                *username = ui.acct_user.clone();
                ui.acct_status = format!("USERNAME CHANGED TO {username}");
            } else {
                ui.acct_status = "USERNAME MUST BE 3-24 CHARACTERS: A-Z 0-9 _".into();
            }
        }
        y += btn_h + 5.0;
        label(a, F::Body, "NEW PASSWORD", m.inner.x, y, 6.0, p.amber);
        y += l6 + 5.0;
        line_edit(
            a,
            Rect::new(m.inner.x, y, m.inner.w, 22.0),
            "",
            "6-200 CHARACTERS",
            8.0,
            false,
            true,
            ui.t,
        );
        y += 27.0;
        if button(
            a,
            Rect::new(m.inner.x, y, m.inner.w, btn_h),
            "UPDATE PASSWORD",
            7.0,
            Kind::Secondary,
            plain(),
        ) {
            ui.acct_status =
                "PASSWORDS LIVE ON THE ACCOUNT SERVER // NOT IN THIS OFFLINE BUILD".into();
        }
        y += btn_h + 5.0;
        if button(
            a,
            Rect::new(
                m.inner.x,
                y,
                m.inner.w.min(w("BACK TO ACCOUNT").max(m.inner.w)),
                btn_h,
            ),
            "BACK TO ACCOUNT",
            7.0,
            Kind::Secondary,
            plain(),
        ) {
            ui.acct_manage = false;
            ui.field_sel = false;
            ui.acct_status.clear();
        }
        if m.close || is_key_pressed(KeyCode::Escape) {
            out = Action::Go(ui.account_back);
        }
        return out;
    }
    // the account card: 260 x (130 + 22 per game)
    let l6 = lh(F::Body, 6.0);
    let row_h = btn_h;
    let content_h = l6 + 5.0 + l6 + 5.0 + row_h * 2.0 + 3.0 + 5.0 + btn_h;
    let m = modal(
        a,
        ACCOUNT_KICKER,
        "ACCOUNT",
        14.0,
        vec2(260.0, 130.0 + 22.0 * 2.0),
        kick_w.max(236.0),
        content_h,
    );
    let mut y = m.y;
    let status = if ui.acct_status.is_empty() {
        format!("SIGNED IN AS {username}")
    } else {
        ui.acct_status.clone()
    };
    label(a, F::Body, &status, m.inner.x, y, 6.0, p.muted);
    y += l6 + 5.0;
    label(a, F::Body, &track("SAVES"), m.inner.x, y, 6.0, p.amber);
    y += l6 + 5.0;
    // THE LAST ROUTE: summary (clipped), OPEN, CLEAR (SURE?)
    let open_w = button_min(a, "OPEN", 7.0, false).x;
    let clear_t = if ui.confirm_clear { "SURE?" } else { "CLEAR" };
    let clear_w = button_min(a, clear_t, 7.0, false).x;
    let right = m.inner.x + m.inner.w;
    let has = save.is_some();
    let mut text_w = m.inner.w - open_w - 5.0;
    if has {
        text_w -= clear_w + 5.0;
    }
    let summary = save_summary(save).unwrap_or_else(|| "NONE".into());
    let line1 = clip(
        a,
        F::Body,
        &format!("THE LAST ROUTE // {summary}"),
        text_w,
        6.0,
    );
    label_vc(
        a,
        F::Body,
        &line1,
        m.inner.x,
        y,
        row_h,
        6.0,
        if has { p.body } else { p.muted },
    );
    let mut ox = right - open_w;
    if has {
        let cr = Rect::new(right - clear_w, y, clear_w, row_h);
        if button(a, cr, clear_t, 7.0, Kind::Secondary, plain()) {
            if ui.confirm_clear {
                ui.confirm_clear = false;
                out = Action::ClearSave;
            } else {
                ui.confirm_clear = true;
            }
        }
        ox = cr.x - 5.0 - open_w;
    }
    if button(
        a,
        Rect::new(ox, y, open_w, row_h),
        "OPEN",
        7.0,
        Kind::Secondary,
        plain(),
    ) {
        out = if has {
            Action::ContinueRun
        } else {
            Action::Go(ui.account_back)
        };
    }
    y += row_h + 5.0;
    let w1 = button_min(a, "ACCOUNT SETTINGS", 7.0, false).x;
    let w2 = button_min(a, "SIGN OUT", 7.0, false).x;
    if button(
        a,
        Rect::new(m.inner.x, y, w1, btn_h),
        "ACCOUNT SETTINGS",
        7.0,
        Kind::Secondary,
        plain(),
    ) {
        ui.acct_manage = true;
        ui.field_sel = false;
        ui.acct_user = username.clone();
        ui.acct_status.clear();
    }
    if button(
        a,
        Rect::new(m.inner.x + w1 + 5.0, y, w2, btn_h),
        "SIGN OUT",
        7.0,
        Kind::Secondary,
        plain(),
    ) {
        ui.acct_field = 0;
        ui.acct_status.clear();
        out = Action::SignOut;
    }
    if m.close || is_key_pressed(KeyCode::Escape) {
        ui.confirm_clear = false;
        out = Action::Go(ui.account_back);
    }
    out
}

// -- route loading (route_loading.gd) -------------------------------------------------------

pub fn draw_loading(a: &Assets, seed_line: &str, detail: &str, progress: f32) {
    let _scope = MenuScope::enter();
    let p = pal();
    backdrop(0.82);
    report_frames();
    let panel = centered(340.0, 128.0);
    panel_box(panel, p.panel, lr(p.panel_edge, hex(0x563041)));
    let x = panel.x + 13.0;
    let w = panel.w - 26.0;
    let mut y = panel.y + 13.0;
    label(a, F::Body, "ROUTE SIGNAL // TRANSFER", x, y, 7.0, p.amber);
    y += lh(F::Body, 7.0) + 5.0;
    label(a, F::Title, "BUILDING ROUTE", x, y, 21.0, p.ink);
    y += lh(F::Title, 21.0) + 5.0;
    let rh = 12.0 + lh(F::Body, 7.0) + 1.0 + lh(F::Body, 6.0);
    panel_box(Rect::new(x, y, w, rh), p.inset, p.accent_deep);
    label(
        a,
        F::Body,
        &ellipsize(a, F::Body, seed_line, w - 12.0, 7.0),
        x + 6.0,
        y + 6.0,
        7.0,
        p.ink,
    );
    label(
        a,
        F::Body,
        detail,
        x + 6.0,
        y + 6.0 + lh(F::Body, 7.0) + 1.0,
        6.0,
        p.muted,
    );
    y += rh + 5.0;
    let bar = Rect::new(x, y, w, 8.0);
    panel_box(bar, p.inset, p.panel_edge);
    let f = progress.clamp(0.0, 1.0);
    if f > 0.0 {
        panel_box(Rect::new(x, y, w * f, 8.0), p.accent, p.accent_edge);
    }
    y += 13.0;
    label(
        a,
        F::Body,
        "BUILDING FACILITY FROM ROUTE SIGNAL",
        x,
        y,
        7.0,
        p.body,
    );
    y += lh(F::Body, 7.0) + 5.0;
    label(
        a,
        F::Body,
        &format!("{:.0}% // BUILDING FACILITY FROM ROUTE SIGNAL", f * 100.0),
        x,
        y,
        6.0,
        p.muted,
    );
}

// -- room staging (room_lobby.gd) -----------------------------------------------------------

/// The local runner's colour (RemotePlayers.local_colour / facility_map.gd COOP_PLAYER_BLUE).
pub const COOP_BLUE: Color = hex(0x63d9f1);

/// The co-op runner's colour: web #00f0ff on Last Route, COOP_BLUE on the City.
pub fn coop_blue() -> Color {
    lr(hex(0x00f0ff), COOP_BLUE)
}

// -- the room lobby (web #room-lobby) --------------------------------------------------------

/// One lobby difficulty option, as `data/lobby.json` (`lr.lobby/1`) lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct LobbyOption {
    pub difficulty: Difficulty,
    pub label: String,
    pub blurb: String,
}

/// The lobby's difficulty row: its title, the guest's locked title (`{label}` is the host's label), and the options in
/// row order.
#[derive(Clone, Debug, PartialEq)]
pub struct LobbyDifficulty {
    pub title: String,
    pub guest_title: String,
    pub options: Vec<LobbyOption>,
}

/// Reads the lobby's difficulty row from `data/lobby.json` text. An option whose id is no difficulty is an error, so a
/// typo in the data fails the tests instead of quietly dropping a button.
pub fn parse_lobby_difficulty(text: &str) -> Result<LobbyDifficulty, String> {
    let root: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let block = root.get("difficulty").ok_or("lobby.json has no difficulty block")?;
    let field = |key: &str| -> Result<String, String> {
        block.get(key).and_then(|v| v.as_str()).map(str::to_string).ok_or(format!("difficulty.{key} is missing"))
    };
    let mut options = Vec::new();
    for entry in block.get("options").and_then(|v| v.as_array()).ok_or("difficulty.options is missing")? {
        let id = entry.get("id").and_then(|v| v.as_str()).ok_or("an option has no id")?;
        let difficulty = LOBBY
            .iter()
            .copied()
            .find(|d| d.id() == id)
            .ok_or(format!("option id {id:?} is no difficulty"))?;
        let text_of = |key: &str| entry.get(key).and_then(|v| v.as_str()).map(str::to_string);
        options.push(LobbyOption {
            difficulty,
            label: text_of("label").ok_or(format!("option {id:?} has no label"))?,
            blurb: text_of("blurb").ok_or(format!("option {id:?} has no blurb"))?,
        });
    }
    Ok(LobbyDifficulty { title: field("title")?, guest_title: field("guest_title")?, options })
}

/// The lobby difficulty content, read once from the embedded `data/lobby.json`.
pub fn lobby_difficulty() -> &'static LobbyDifficulty {
    static CONTENT: std::sync::OnceLock<LobbyDifficulty> = std::sync::OnceLock::new();
    CONTENT.get_or_init(|| {
        let (_, text) = crate::content::embedded::data_under("lobby.json")
            .find(|(p, _)| *p == "lobby.json")
            .expect("data/lobby.json is embedded");
        parse_lobby_difficulty(text).expect("data/lobby.json is valid")
    })
}

/// The option the lobby data gives a difficulty, when it lists one.
fn lobby_option(d: Difficulty) -> Option<&'static LobbyOption> {
    lobby_difficulty().options.iter().find(|o| o.difficulty == d)
}

/// The lobby's difficulty row, in the order `data/lobby.json` lists it. Difficulties the data leaves out are not shown.
pub fn lobby_row() -> Vec<Difficulty> {
    lobby_difficulty().options.iter().map(|o| o.difficulty).collect()
}

/// The difficulty's display label (web NORMAL, from `data/lobby.json`). The id and the wire name stay
/// `Difficulty::Standard`'s "STANDARD".
pub fn difficulty_label(d: Difficulty) -> &'static str {
    lobby_option(d).map_or(d.name(), |o| o.label.as_str())
}

/// The staging blurb under the difficulty buttons, from `data/lobby.json`.
pub fn difficulty_blurb(d: Difficulty) -> &'static str {
    lobby_option(d).map_or("", |o| o.blurb.as_str())
}

/// Only the host (a solo runner is its own host) sets the route's difficulty; a guest waits for the host's.
pub fn difficulty_editable(mode: RoomMode) -> bool {
    mode != RoomMode::Guest
}

/// The lobby's difficulty row title: a guest reads the host's difficulty name here, the host and a solo runner get the
/// plain title.
pub fn difficulty_row_title(mode: RoomMode, current: Difficulty) -> String {
    let content = lobby_difficulty();
    if difficulty_editable(mode) {
        content.title.clone()
    } else {
        content.guest_title.replace("{label}", difficulty_label(current))
    }
}

/// The difficulty after a click on `picked`: a guest keeps the host's difficulty.
pub fn pick_difficulty(mode: RoomMode, current: Difficulty, picked: Difficulty) -> Difficulty {
    if difficulty_editable(mode) {
        picked
    } else {
        current
    }
}

/// The lobby's actions row, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LobbyButton {
    /// ENTER ROUTE (a guest sees WAITING FOR HOST, locked).
    Enter,
    /// Solo only: resume the one saved run.
    ContinueRun,
    /// The room owner's START ROUTE (replaces ENTER ROUTE, which a guest does not have).
    StartRoute,
    /// The room owner's RESTART ROUTE.
    RestartRoute,
    /// The room owner's END ROUTE.
    EndRoute,
    /// The hunter dossier.
    CharacterFiles,
    /// Solo: BACK TO LAST ROUTE. A host or guest: LEAVE ROOM.
    Leave,
}

/// The actions row for this room, left to right.
#[cfg(test)]
pub fn lobby_buttons(mode: RoomMode) -> Vec<LobbyButton> {
    lobby_buttons_for(mode, false)
}

/// The actions row for this room. The owner of an online room (a guest who created it) has START, RESTART and END ROUTE in
/// place of the guest's locked WAITING FOR HOST.
pub fn lobby_buttons_for(mode: RoomMode, owner: bool) -> Vec<LobbyButton> {
    if owner && mode == RoomMode::Guest {
        return vec![
            LobbyButton::StartRoute,
            LobbyButton::RestartRoute,
            LobbyButton::EndRoute,
            LobbyButton::CharacterFiles,
            LobbyButton::Leave,
        ];
    }
    let mut v = vec![LobbyButton::Enter];
    if mode == RoomMode::Solo {
        v.push(LobbyButton::ContinueRun);
    }
    v.push(LobbyButton::CharacterFiles);
    v.push(LobbyButton::Leave);
    v
}

/// The keyboard focus index of `button` in this room's actions row.
#[cfg(test)]
pub fn lobby_focus(mode: RoomMode, button: LobbyButton) -> usize {
    lobby_focus_for(mode, false, button)
}

/// [`lobby_focus`] for a room whose owner may be this runner.
pub fn lobby_focus_for(mode: RoomMode, owner: bool, button: LobbyButton) -> usize {
    lobby_buttons_for(mode, owner).iter().position(|b| *b == button).unwrap_or(0)
}

/// The label of an actions-row button in this room.
pub fn lobby_label(button: LobbyButton, mode: RoomMode) -> &'static str {
    match button {
        LobbyButton::Enter if mode == RoomMode::Guest => "WAITING FOR HOST",
        LobbyButton::Enter => "ENTER ROUTE",
        LobbyButton::ContinueRun => "CONTINUE RUN",
        LobbyButton::CharacterFiles => "CHARACTER FILES",
        LobbyButton::Leave if mode == RoomMode::Solo => "BACK TO LAST ROUTE",
        LobbyButton::Leave => "LEAVE ROOM",
        // the owner's labels are data (data/net/hosted_room.json); lobby_text gives them
        LobbyButton::StartRoute | LobbyButton::RestartRoute | LobbyButton::EndRoute => "",
    }
}

/// The text on an actions-row button: [`lobby_label`], or the owner buttons' data labels.
pub fn lobby_text(button: LobbyButton, mode: RoomMode) -> String {
    let s = &hosted::rules().strings;
    match button {
        LobbyButton::StartRoute => s.label_start.clone(),
        LobbyButton::RestartRoute => s.label_restart.clone(),
        LobbyButton::EndRoute => s.label_end.clone(),
        other => lobby_label(other, mode).to_string(),
    }
}

/// The command an owner button sends.
pub fn lobby_owner_action(button: LobbyButton) -> Option<OwnerAction> {
    match button {
        LobbyButton::StartRoute => Some(OwnerAction::Start),
        LobbyButton::RestartRoute => Some(OwnerAction::Restart),
        LobbyButton::EndRoute => Some(OwnerAction::End),
        _ => None,
    }
}

/// Whether an actions-row button is locked in a room whose owner controls may be showing. The owner's buttons need the
/// room to be joined (while the server opens it there is nobody to send to); the others follow [`lobby_locked`].
pub fn lobby_locked_for(button: LobbyButton, mode: RoomMode, has_save: bool, joined: bool) -> bool {
    match button {
        LobbyButton::StartRoute | LobbyButton::RestartRoute | LobbyButton::EndRoute => !joined,
        other => lobby_locked(other, mode, has_save),
    }
}

/// Whether an actions-row button is locked: a guest waits for the host's route, and CONTINUE RUN needs a saved run.
pub fn lobby_locked(button: LobbyButton, mode: RoomMode, has_save: bool) -> bool {
    match button {
        LobbyButton::Enter => mode == RoomMode::Guest,
        LobbyButton::ContinueRun => !has_save,
        _ => false,
    }
}

/// The capacity line under the room code. The session's live count when it has one (a host counts its
/// guests); otherwise UNLIMITED for a host, and the room size for a guest. A solo room is one runner.
pub fn capacity_text(mode: RoomMode, live: Option<u32>) -> String {
    let max = crate::net::api::MAX_PLAYERS;
    let signals = |n: u32| if n == 1 { "SIGNAL" } else { "SIGNALS" };
    match (mode, live) {
        (RoomMode::Solo, _) => "1 SIGNAL".into(),
        (RoomMode::Host, Some(n)) => format!("{n} {} // UNLIMITED", signals(n)),
        (RoomMode::Host, None) => "UNLIMITED".into(),
        (RoomMode::Guest, Some(n)) => format!("{n} / {max} {}", signals(n)),
        (RoomMode::Guest, None) => format!("UP TO {max} SIGNALS"),
    }
}

/// One row of CONNECTED PLAYERS.
#[derive(Clone, Debug, PartialEq)]
pub struct LobbyPlayer {
    pub name: String,
    /// The host's row (amber dot). The others have a green dot.
    pub host: bool,
    /// The local runner's row (NAME // YOU).
    pub you: bool,
    pub status: String,
    /// The owner's KICK target: set by [`with_kick_targets`] for a runner the owner may remove.
    pub kick: Option<u32>,
}

/// Marks the rows the room's owner can KICK: every runner but this one and the host, matched by name to the room's
/// session ids. Nobody else gets a target.
pub fn with_kick_targets(mut rows: Vec<LobbyPlayer>, view: &OwnerView) -> Vec<LobbyPlayer> {
    if !view.owner {
        return rows;
    }
    for row in rows.iter_mut().filter(|r| !r.you && !r.host) {
        row.kick = view.peer_of(&row.name);
    }
    rows
}

/// The roster the lobby shows, each runner once: this runner first, marked YOU. A host adds its guests. A guest adds
/// the host, which is the first of `guests` (as `NetSession::guest_names` orders them), then the other guests.
/// `username` is the name this runner goes by in the room (the app passes the account name, else the guest name);
/// alone (solo) an empty one reads RUNNER, in a room it reads as a guest (GUEST_0000).
pub fn lobby_players(mode: RoomMode, username: &str, guests: &[String]) -> Vec<LobbyPlayer> {
    let host = mode == RoomMode::Host;
    let mut v = vec![LobbyPlayer {
        name: if mode == RoomMode::Solo { username_or(username) } else { room_name_or(username) },
        host,
        you: true,
        status: if host { "HOST // STAGING" } else { "STAGING" }.into(),
        kick: None,
    }];
    match mode {
        RoomMode::Solo => {}
        RoomMode::Host => v.extend(guests.iter().map(|g| LobbyPlayer {
            name: g.to_uppercase(),
            host: false,
            you: false,
            status: "STAGING".into(),
            kick: None,
        })),
        RoomMode::Guest => {
            for (i, g) in guests.iter().enumerate().filter(|(_, g)| !g.is_empty() && !g.eq_ignore_ascii_case(username)) {
                let is_host = i == 0;
                v.push(LobbyPlayer {
                    name: g.to_uppercase(),
                    host: is_host,
                    you: false,
                    status: if is_host { "HOST // STAGING" } else { "STAGING" }.into(),
                    kick: None,
                });
            }
        }
    }
    v
}

/// The room lobby (web #room-lobby): the eyebrow, the room code with COPY CODE, the capacity, the route seed,
/// ROUTE CONFIGURATION, CONNECTED PLAYERS, the feedback line and the actions row. The panel scrolls when short.
#[allow(clippy::too_many_arguments)]
pub fn draw_lobby(
    a: &Assets,
    ui: &mut Ui,
    diff: &mut Difficulty,
    code: &str,
    room_text: &str,
    username: &str,
    save: Option<&Checkpoint>,
    live: Option<u32>,
    guests: &[String],
    interactive: bool,
) -> Action {
    let _scope = MenuScope::enter();
    let p = pal();
    backdrop(0.7);
    let v = view();
    let row = 13.0;
    let l6 = lh(F::Body, 6.0);
    let l7 = lh(F::Body, 7.0);
    let mode = ui.room_mode;
    let solo = mode == RoomMode::Solo;
    let guest = mode == RoomMode::Guest;
    // the owner of an online room sees the owner controls; nobody else does
    let owner = guest && ui.owner.owner;
    let owner_joined = ui.owner.joined;
    let waiting = ui.owner.waiting.clone();
    let strings = &hosted::rules().strings;
    let btn_h = button_min(a, "X", 7.0, false).y.max(row);
    let panel = Rect::new(v.x * 0.5 - 190.0, 10.0, 380.0, v.y - 20.0);
    panel_box(panel, p.panel, p.panel_edge);
    let inner = Rect::new(
        panel.x + 12.0,
        panel.y + 12.0,
        panel.w - 24.0,
        panel.h - 24.0,
    );
    let x = inner.x;
    let w = inner.w;
    let mut out = Action::None;
    if interactive {
        let wheel = mouse_wheel().1;
        if wheel != 0.0 {
            ui.scroll -= wheel.signum() * 20.0;
        }
        if is_key_pressed(KeyCode::PageDown) || is_key_pressed(KeyCode::End) {
            ui.scroll += 200.0;
        }
        if is_key_pressed(KeyCode::PageUp) || is_key_pressed(KeyCode::Home) {
            ui.scroll = 0.0;
        }
    }
    clip_begin(Rect::new(inner.x, inner.y, inner.w + 4.0, inner.h));
    let y0 = inner.y - ui.scroll;
    let mut y = y0;
    // the eyebrow, then the room code with COPY CODE under it
    label(a, F::Body, &track("CO-OP STAGING SIGNAL"), x, y, 7.0, p.amber);
    y += l7 + 5.0;
    // the lobby title is 1.17 of the old size (menu_size)
    let room_u = menu_size(18.0, 1.17);
    label(a, F::Title, "ROOM", x, y, room_u, p.ink);
    label(
        a,
        F::Title,
        code,
        x + tw(a, F::Title, "ROOM", room_u) + 6.0,
        y,
        room_u,
        p.alert,
    );
    y += lh(F::Title, room_u) + 4.0;
    let copy_w = button_min(a, "COPY CODE", 7.0, false).x.max(88.0);
    if button(
        a,
        Rect::new(x, y, copy_w, btn_h),
        "COPY CODE",
        7.0,
        Kind::Secondary,
        plain(),
    ) && interactive
    {
        super::clipboard::copy(code);
        ui.code_copied = true;
    }
    if ui.code_copied {
        label_vc(
            a,
            F::Body,
            "COPIED TO CLIPBOARD",
            x + copy_w + 6.0,
            y,
            btn_h,
            7.0,
            p.stable,
        );
    }
    y += btn_h + 5.0;
    label(a, F::Body, &capacity_text(mode, live), x, y, 7.0, p.amber);
    y += l7 + 5.0;
    // the route seed; a solo runner can reroll it (a hosted or joined room keeps its seed: the code is in it)
    let sh = 12.0 + l6 + 1.0 + lh(F::Body, 9.0) + 1.0 + l6;
    panel_box(
        Rect::new(x, y, w, sh),
        lr(hexa(0x1B101CA3), p.inset),
        p.panel_edge,
    );
    fill(Rect::new(x, y, 3.0, sh), p.alert);
    label(a, F::Body, &track("ROUTE SEED"), x + 6.0, y + 6.0, 6.0, p.muted);
    label(a, F::Body, room_text, x + 6.0, y + 7.0 + l6, 9.0, p.ink);
    label(
        a,
        F::Body,
        "Players enter from separate nearby access points. The route is shared.",
        x + 6.0,
        y + 8.0 + l6 + lh(F::Body, 9.0),
        6.0,
        lr(hex(0x776c72), p.muted),
    );
    if solo {
        let rb = button_min(a, "REROLL ROUTE", 6.0, true);
        let rr = Rect::new(x + w - 6.0 - rb.x, y + 5.0, rb.x, rb.y);
        if button(a, rr, "REROLL ROUTE", 6.0, Kind::Secondary, plain()) && interactive {
            out = Action::Reroll;
        }
    }
    y += sh + 5.0;
    // ROUTE CONFIGURATION: the shared route's difficulty, and the save slot (a placeholder until a slot store exists)
    const NOTE: &str = "The host configures the shared route. Difficulty affects both runners and hunters.";
    let note = wrap(a, F::Body, if ui.owner.owner && guest { &strings.owner_lobby_note } else { NOTE }, w - 12.0, 6.0);
    let cfg_h = 6.0
        + l7
        + 3.0
        + l6
        + 2.0
        + btn_h
        + 2.0
        + l6
        + 5.0
        + l6
        + 2.0
        + btn_h
        + 4.0
        + note.len() as f32 * lh(F::Body, 6.0)
        + 6.0;
    panel_box(
        Rect::new(x, y, w, cfg_h),
        lr(hexa(0x070509B8), p.inset),
        lr(hex(0x392c38), p.panel_edge),
    );
    let cx = x + 6.0;
    let cw = w - 12.0;
    let mut cy = y + 6.0;
    label(a, F::Body, &track("ROUTE CONFIGURATION"), cx, cy, 7.0, p.amber);
    if owner {
        label_r(a, F::Body, &strings.owner_heading, cx + cw, cy + (l7 - l6) * 0.5, 6.0, p.amber);
    } else if guest {
        label_r(
            a,
            F::Body,
            "HOST LOCKED // VIEW ONLY",
            cx + cw,
            cy + (l7 - l6) * 0.5,
            6.0,
            p.muted,
        );
    } else {
        label_r(
            a,
            F::Body,
            "HOST CONTROLS",
            cx + cw,
            cy + (l7 - l6) * 0.5,
            6.0,
            p.amber,
        );
    }
    cy += l7 + 3.0;
    // a guest sees the host's difficulty as a locked row; only the host (or a solo runner) can pick another
    let title = if owner { lobby_difficulty().title.clone() } else { difficulty_row_title(mode, *diff) };
    label(a, F::Body, &title, cx, cy, 6.0, p.muted);
    let by = cy + l6 + 2.0;
    let mut bx = cx;
    let row = lobby_row();
    for d in row.iter().copied() {
        let bw = button_min(a, difficulty_label(d), 7.0, false).x;
        let kind = if d == *diff {
            Kind::Selected
        } else {
            Kind::Secondary
        };
        // the owner picks the room's difficulty through the server (it applies from the next route); a host or a solo
        // runner picks it here; any other guest only reads the host's
        let st = BtnState { disabled: if owner { !owner_joined } else { !difficulty_editable(mode) }, ..plain() };
        if button(a, Rect::new(bx, by, bw, btn_h), difficulty_label(d), 7.0, kind, st)
            && interactive
        {
            if owner {
                if d != *diff {
                    out = Action::Owner(OwnerAction::Difficulty(d));
                }
            } else {
                *diff = pick_difficulty(mode, *diff, d);
            }
        }
        bx += bw + 4.0;
    }
    if interactive
        && difficulty_editable(mode)
        && !row.is_empty()
        && (is_key_pressed(KeyCode::Left) || is_key_pressed(KeyCode::Right))
        && (is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift))
    {
        let n = row.len();
        let i = row.iter().position(|d| d == diff).unwrap_or(0);
        *diff = row[if is_key_pressed(KeyCode::Left) {
            (i + n - 1) % n
        } else {
            (i + 1) % n
        }];
    }
    label(a, F::Body, difficulty_blurb(*diff), cx, by + btn_h + 2.0, 6.0, p.muted);
    cy = by + btn_h + 2.0 + l6 + 5.0;
    label(a, F::Body, "SAVE SLOT", cx, cy, 6.0, p.muted);
    let sy = cy + l6 + 2.0;
    // AUTOSAVE, LOAD ROUTE and SAVE ROUTE are placeholders: this build has no slot store yet
    let mut sx = cx;
    button(
        a,
        Rect::new(sx, sy, 96.0, btn_h),
        "AUTOSAVE",
        7.0,
        Kind::Secondary,
        disabled(),
    );
    sx += 96.0 + 4.0;
    for t in ["LOAD ROUTE", "SAVE ROUTE"] {
        let bw = button_min(a, t, 7.0, false).x;
        button(a, Rect::new(sx, sy, bw, btn_h), t, 7.0, Kind::Secondary, disabled());
        sx += bw + 4.0;
    }
    let mut ny = sy + btn_h + 4.0;
    for l in &note {
        label(a, F::Body, l, cx, ny, 6.0, p.muted);
        ny += lh(F::Body, 6.0);
    }
    y += cfg_h + 5.0;
    // CONNECTED PLAYERS
    label(a, F::Body, &track("CONNECTED PLAYERS"), x, y, 7.0, p.amber);
    y += l7 + 5.0;
    let rh = btn_h + 4.0;
    for pl in with_kick_targets(lobby_players(mode, username, guests), &ui.owner) {
        let r = Rect::new(x, y, w, rh);
        let edge = if pl.you {
            lr(hex(0x73596d), p.panel_edge)
        } else {
            lr(hex(0x342833), p.panel_edge)
        };
        panel_box(r, lr(hex(0x0e0910), p.inset), edge);
        let dot = if pl.host { p.amber } else { p.stable };
        fill(Rect::new(x + 6.0, y + (rh - 9.0) * 0.5, 4.0, 9.0), dot);
        let status_w = tw(a, F::Body, &pl.status, 7.0);
        let name = if pl.you {
            format!("{} // YOU", pl.name)
        } else {
            pl.name.clone()
        };
        // the owner's KICK sits left of the status
        let kick_w = if pl.kick.is_some() { button_min(a, &strings.label_kick, 6.0, false).x + 4.0 } else { 0.0 };
        if let Some(peer) = pl.kick {
            let kr = Rect::new(x + w - 6.0 - status_w - 6.0 - kick_w, y + 1.0, kick_w, rh - 2.0);
            if button(a, kr, &strings.label_kick, 6.0, Kind::Secondary, plain()) && interactive {
                out = Action::Owner(OwnerAction::Kick(peer));
            }
        }
        let name_w = (w - 14.0 - status_w - 14.0 - if kick_w > 0.0 { kick_w + 6.0 } else { 0.0 }).max(20.0);
        label_vc(
            a,
            F::Body,
            &ellipsize(a, F::Body, &name, name_w, 7.0),
            x + 14.0,
            y,
            rh,
            7.0,
            p.ink,
        );
        let st = if pl.host { p.amber } else { p.body };
        label_vc(a, F::Body, &pl.status, x + w - 6.0 - status_w, y, rh, 7.0, st);
        y += rh + 3.0;
    }
    y += 5.0;
    // the feedback line: the room's status
    let (mode_line, mode_colour, mode_detail) = match mode {
        RoomMode::Solo => (
            "OFFLINE SOLO ROOM".to_string(),
            p.alert,
            format!("SOLO ROOM {code} // PRIMARY SERVER UNAVAILABLE"),
        ),
        RoomMode::Host => (
            "ONLINE ROOM // YOU ARE HOSTING".to_string(),
            coop_blue(),
            format!("ROOM {code} // FRIENDS JOIN WITH THIS CODE"),
        ),
        RoomMode::Guest if waiting.is_some() => (
            strings.lobby_waiting_line.clone(),
            coop_blue(),
            waiting.as_ref().map(|w| w.0.clone()).unwrap_or_default(),
        ),
        RoomMode::Guest if owner => (
            strings.lobby_owner_line.clone(),
            coop_blue(),
            hosted::fill(&strings.lobby_owner_detail, code, 0, 0, ""),
        ),
        RoomMode::Guest => (
            "ONLINE ROOM // JOINED".to_string(),
            coop_blue(),
            format!("ROOM {code} // THE HOST STARTS THE ROUTE"),
        ),
    };
    label(a, F::Body, &mode_line, x, y, 7.0, mode_colour);
    y += l7 + 5.0;
    label(a, F::Body, &mode_detail, x, y, 7.0, p.muted);
    y += l7 + 8.0;
    if let Some((_, checks)) = waiting.as_ref() {
        label(a, F::Body, checks, x, y - 3.0, 7.0, p.muted);
        y += l7 + 5.0;
    }
    if !ui.owner.status.is_empty() {
        label(a, F::Body, &ui.owner.status, x, y - 3.0, 7.0, p.amber);
        y += l7 + 5.0;
    }
    // the actions row: wraps onto a second line when the window is narrow
    let items = lobby_buttons_for(mode, owner);
    let enter = interactive && nav(&mut ui.focus, items.len(), false);
    let ah = button_min(a, "X", menu_size(10.0, 1.0), false).y.max(18.0);
    let mut ax = x;
    let mut ay = y;
    for (i, b) in items.iter().enumerate() {
        let (fs, kind) = if matches!(*b, LobbyButton::Enter | LobbyButton::StartRoute) {
            (menu_size(10.0, 1.0), Kind::Primary)
        } else {
            (7.0, Kind::Secondary)
        };
        let text = lobby_text(*b, mode);
        let text = text.as_str();
        let locked = lobby_locked_for(*b, mode, save.is_some(), owner_joined);
        let bw = button_min(a, text, fs, false).x;
        if ax + bw > x + w && ax > x {
            ax = x;
            ay += ah + 4.0;
        }
        let st = if locked {
            disabled()
        } else {
            state(interactive && ui.focus == i)
        };
        let hit = button(a, Rect::new(ax, ay, bw, ah), text, fs, kind, st) && interactive && !locked;
        if (hit || (enter && ui.focus == i)) && !locked {
            out = match b {
                LobbyButton::Enter => Action::StartRun,
                LobbyButton::ContinueRun => Action::ContinueRun,
                LobbyButton::CharacterFiles => Action::Go(Screen::Dossier),
                LobbyButton::Leave => Action::Go(Screen::Title),
                owner_button => lobby_owner_action(*owner_button).map_or(Action::None, Action::Owner),
            };
        }
        ax += bw + 5.0;
    }
    y = ay + ah;
    clip_end();
    let content_h = y - y0;
    ui.scroll = ui.scroll.clamp(0.0, (content_h - inner.h).max(0.0));
    if interactive && is_key_pressed(KeyCode::Escape) {
        out = Action::Go(Screen::Title);
    }
    out
}

/// This runner's name in a room: an empty one is a guest-shaped name, never RUNNER.
fn room_name_or(n: &str) -> String {
    if n.trim().is_empty() {
        crate::net::guest_name::placeholder(0)
    } else {
        n.to_uppercase()
    }
}

fn username_or(n: &str) -> String {
    if n.is_empty() {
        "RUNNER".into()
    } else {
        n.to_uppercase()
    }
}

pub struct PlayerRow {
    pub name: String,
    pub host: bool,
    pub you: bool,
    pub ping: String,
    pub status: String,
    pub floor: String,
    pub colour: Color,
    /// The session id the room owner's KICK button removes; None for a row nobody may kick.
    pub kick: Option<u32>,
}

/// PlayerListPanel: title, the column header and one row per player.
pub fn player_list(a: &Assets, r: Rect, rows: &[PlayerRow], interactive: bool) -> Option<u32> {
    let p = pal();
    panel_box(r, p.panel, p.panel_edge);
    let x = r.x + 6.0;
    let mut y = r.y + 6.0;
    label(a, F::Body, "PLAYERS IN THIS ROOM", x, y, 7.0, p.amber);
    y += lh(F::Body, 7.0) + 3.0;
    // swatch 8 | badge 34 | NAME 92 | PING 38 | STATUS 70 | FLOOR 32 | VOICE 26, separation 4
    let cols = [8.0, 34.0, 92.0, 38.0, 70.0, 32.0, 26.0];
    let mut xs = [0.0f32; 7];
    let mut cxp = x;
    for (i, w) in cols.iter().enumerate() {
        xs[i] = cxp;
        cxp += w + 4.0;
    }
    // the last column is the owner's KICK where a row has one (the voice column has nothing in it yet)
    let any_kick = interactive && rows.iter().any(|r| r.kick.is_some());
    let last = if any_kick { hosted::rules().strings.label_kick.as_str() } else { "VOICE" };
    for (i, h) in ["", "", "NAME", "PING", "STATUS", "FLOOR", last].iter().enumerate() {
        label(a, F::Body, h, xs[i], y, 6.0, p.muted);
    }
    y += lh(F::Body, 6.0) + 3.0;
    let rh = if any_kick { lh(F::Body, 7.0).max(13.0) } else { lh(F::Body, 7.0) };
    let mut kicked = None;
    for row in rows {
        fill(Rect::new(xs[0], y + (rh - 9.0) * 0.5, 4.0, 9.0), row.colour);
        if row.host {
            let br = Rect::new(xs[1], y + 1.0, 34.0, rh - 2.0);
            fill(br, p.accent);
            label_vc(a, F::Body, "HOST", br.x + 2.0, br.y, br.h, 6.0, p.on_amber);
        }
        let name = format!("{}{}", row.name, if row.you { " (YOU)" } else { "" });
        label_vc(
            a,
            F::Body,
            &ellipsize(a, F::Body, &name, 92.0, 7.0),
            xs[2],
            y,
            rh,
            7.0,
            lightened(row.colour, 0.35),
        );
        label_vc(a, F::Body, &row.ping, xs[3], y, rh, 7.0, p.body);
        let st = match row.status.as_str() {
            "ALIVE" => p.stable,
            "CAUGHT" | "DISCONNECTED" => p.alert,
            _ => p.amber,
        };
        label_vc(a, F::Body, &row.status, xs[4], y, rh, 7.0, st);
        label_vc(a, F::Body, &row.floor, xs[5], y, rh, 7.0, p.body);
        if let (Some(peer), true) = (row.kick, interactive) {
            let kr = Rect::new(xs[6], y, cols[6] + 8.0, rh);
            if button(a, kr, &hosted::rules().strings.label_kick, 6.0, Kind::Secondary, plain()) {
                kicked = Some(peer);
            }
        }
        y += rh + 2.0;
    }
    kicked
}

// -- the hunter dossier (room_lobby.gd / pause_menu.gd _open_dossier) -----------------------

/// The stat cards of the dossier detail, in order (the web's CHASE, SIGHT and HEARING).
pub const DOSSIER_STAT_LABELS: [&str; 3] = ["CHASE", "SIGHT", "HEARING"];

/// The text the dossier detail shows for one hunter. `stats` holds the three card values, in
/// `DOSSIER_STAT_LABELS` order.
#[derive(Debug, Clone, PartialEq)]
pub struct DossierText {
    pub title: String,
    pub description: String,
    pub stats: [String; 3],
}

/// The three card values for a discovered hunter: chase in cells a second, sight in cells, and
/// hearing as a percentage (DEAF when the scale is zero or less).
fn stat_values(chase: f32, sight: f32, hearing: f32) -> [String; 3] {
    let hearing = if hearing <= 0.0 {
        "DEAF".to_string()
    } else {
        format!("{:.0}%", hearing * 100.0)
    };
    [format!("{chase:.2} c/s"), format!("{sight:.1} cells"), hearing]
}

/// What the dossier detail shows. An undiscovered hunter (`known` false) is blanked: an empty
/// name (drawn as a black square), "???" for each stat card, and no description at all, so neither the real name, the
/// description nor any number is ever drawn. A discovered hunter shows every field.
pub fn dossier_text(
    known: bool,
    name: &str,
    description: &str,
    chase: f32,
    sight: f32,
    hearing: f32,
) -> DossierText {
    if known {
        DossierText {
            title: name.to_uppercase(),
            description: description.to_string(),
            stats: stat_values(chase, sight, hearing),
        }
    } else {
        DossierText {
            // no name text: the detail draws a black square in its place
            title: String::new(),
            description: String::new(),
            stats: std::array::from_fn(|_| LOCKED_PLACEHOLDER.to_string()),
        }
    }
}

/// The placeholder the dossier shows for anything still locked (names, stat cards).
const LOCKED_PLACEHOLDER: &str = "???";

/// The tint that turns a locked hunter's portrait into a near-black silhouette.
const DOSSIER_SILHOUETTE: Color = rgba(0.02, 0.015, 0.03, 1.0);

/// A roster category (A-Z or void) is locked while none of its hunters is discovered, so a
/// locked category cannot reveal what its hunters are. `hunter_ids` are the category's members.
pub fn category_locked<'a>(
    hunter_ids: impl IntoIterator<Item = &'a str>,
    found: &[String],
) -> bool {
    !hunter_ids
        .into_iter()
        .any(|id| found.iter().any(|d| d == id))
}

/// The two dossier tabs: the built-in roster, and the hunters that mod packs add or replace.
pub const DOSSIER_TABS: [&str; 2] = ["ROUTE DOSSIER", "CUSTOM HUNTERS"];
const CUSTOM_TAB: usize = 1;

/// One roster row: a category header (its text), or a hunter (its index in the roster).
pub type DossierRow = (Option<String>, Option<usize>);

fn is_void_hunter(h: &HunterDef) -> bool {
    h.category == "void"
}

/// The Inferno's secret creatures: out of every list until the runner has met one (then a FOUND section lists it).
fn is_secret_hunter(h: &HunterDef) -> bool {
    h.category == "secret"
}

/// The Route tab's category lock: built-in members only, so a pack hunter never unlocks a
/// built-in category header.
fn route_category_locked(hs: &[HunterDef], found: &[String], void: bool) -> bool {
    category_locked(
        hs.iter()
            .filter(|h| !h.is_pack() && !is_secret_hunter(h) && is_void_hunter(h) == void)
            .map(|h| h.id.as_str()),
        found,
    )
}

/// The roster rows of one dossier tab. The Route tab lists the built-in hunters under their
/// A-Z and VOID headers, each section sorted by name as the web roster is. The headers always
/// show their names, even while a section is locked. The Custom tab lists the pack hunters as
/// one list, with no headers.
pub fn dossier_rows(hs: &[HunterDef], tab: usize) -> Vec<DossierRow> {
    if tab == CUSTOM_TAB {
        return (0..hs.len())
            .filter(|&i| hs[i].is_pack())
            .map(|i| (None, Some(i)))
            .collect();
    }
    let built_in = |void: bool| -> Vec<DossierRow> {
        let mut ids: Vec<usize> = (0..hs.len())
            .filter(|&i| !hs[i].is_pack() && !is_secret_hunter(&hs[i]) && is_void_hunter(&hs[i]) == void)
            .collect();
        ids.sort_by_cached_key(|&i| hs[i].name.to_lowercase());
        ids.into_iter().map(|i| (None, Some(i))).collect()
    };
    let mut rows: Vec<DossierRow> = vec![(Some("A-Z HUNTERS".to_string()), None)];
    rows.extend(built_in(false));
    rows.push((Some("VOID".to_string()), None));
    rows.extend(built_in(true));
    rows
}

/// The Route tab's rows with the FOUND section after VOID: the secret creatures the account has met, and no header, row or
/// counter at all while it has met none (nothing says they exist).
pub fn dossier_rows_found(hs: &[HunterDef], tab: usize, found: &[String]) -> Vec<DossierRow> {
    let mut rows = dossier_rows(hs, tab);
    if tab == CUSTOM_TAB {
        return rows;
    }
    let mut met: Vec<usize> = (0..hs.len()).filter(|&i| !hs[i].is_pack() && is_secret_hunter(&hs[i]) && found.iter().any(|d| *d == hs[i].id)).collect();
    if !met.is_empty() {
        met.sort_by_cached_key(|&i| hs[i].name.to_lowercase());
        rows.push((Some("FOUND".to_string()), None));
        rows.extend(met.into_iter().map(|i| (None, Some(i))));
    }
    rows
}

/// One visual line of the roster: a full-width category header, or up to two hunter cells side
/// by side (the web's two-column grid). A cell is (selection index, roster index).
#[derive(Debug, Clone, PartialEq)]
pub enum RosterLine {
    Header(String),
    Cells(Vec<(usize, usize)>),
}

/// Groups roster rows into visual lines. A header always starts a new line, and hunters fill
/// each line two at a time.
pub fn roster_lines(rows: &[DossierRow]) -> Vec<RosterLine> {
    let mut lines = Vec::new();
    let mut pending: Vec<(usize, usize)> = Vec::new();
    let mut sel = 0;
    for (head, hi) in rows {
        if let Some(text) = head {
            if !pending.is_empty() {
                lines.push(RosterLine::Cells(std::mem::take(&mut pending)));
            }
            lines.push(RosterLine::Header(text.clone()));
        } else if let Some(&i) = hi.as_ref() {
            pending.push((sel, i));
            sel += 1;
            if pending.len() == 2 {
                lines.push(RosterLine::Cells(std::mem::take(&mut pending)));
            }
        }
    }
    if !pending.is_empty() {
        lines.push(RosterLine::Cells(pending));
    }
    lines
}

/// Whether a roster row shows its hunter's name, description and stats. Discovery decides on
/// both tabs. The Route tab also keeps a whole A-Z or VOID category locked until one of its
/// built-in members is discovered, so a locked category hides its hunters (its header keeps
/// the category name).
pub fn dossier_open(hs: &[HunterDef], tab: usize, found: &[String], i: usize) -> bool {
    let h = &hs[i];
    let known = found.iter().any(|d| *d == h.id);
    if tab == CUSTOM_TAB || is_secret_hunter(h) {
        known
    } else {
        known && !route_category_locked(hs, found, is_void_hunter(h))
    }
}

/// The line the roster shows when a tab has no hunters. Only the Custom tab can be empty.
pub fn dossier_empty_line(tab: usize) -> Option<&'static str> {
    (tab == CUSTOM_TAB).then_some("No custom hunters installed")
}

/// The dossier modal over whatever screen is below. Returns true when it closes.
pub fn draw_dossier(a: &Assets, ui: &mut Ui, content: &Content, username: &str) -> bool {
    let _scope = MenuScope::enter();
    let p = pal();
    // entries stay locked until the account has met the hunter (backend_client.gd hunter_discovered)
    let found = crate::app::profile::discovered(username);
    let signed_in = crate::app::profile::signed_in(username);
    // the web's CHARACTER FILES dialog: a two-column roster beside the detail column
    // the web's dossier grid is minmax(260px, 340px) for the roster and 1fr for the details
    const ROSTER_W: f32 = 190.0;
    let min = vec2(420.0, 260.0);
    let m = modal(
        a,
        "HUNTER DOSSIERS",
        "CHARACTER FILES",
        15.0,
        min,
        440.0,
        40.0,
    );
    // the web's muted "SELECT A UNIT" hint, after the kicker
    let top_h = esc_close_size(a).y.max(lh(F::Body, 6.0));
    let hint_x = m.inner.x + tw(a, F::Body, &track("HUNTER DOSSIERS"), 6.0) + 8.0;
    label_vc(a, F::Body, &track("SELECT A UNIT"), hint_x, m.inner.y, top_h, 6.0, p.muted);
    // the tab strip sits under the heading, and the roster starts below it
    let strip = Rect::new(m.inner.x, m.y, m.inner.w, 22.0);
    let split_y = m.y + 28.0;
    let split_h = m.inner.y + m.inner.h - split_y;
    // the roster: a two-column grid of 48x48 portraits under full-width section headers
    let list = Rect::new(m.inner.x, split_y, ROSTER_W, split_h);
    panel_box(list, p.inset, p.panel_edge);
    let hs = &content.hunters;
    let mut tab = ui.dossier_tab.min(DOSSIER_TABS.len() - 1);
    if let Some(i) = tab_bar(a, strip, &DOSSIER_TABS, tab, true) {
        tab = i;
    }
    if is_key_pressed(KeyCode::Tab) {
        let back = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
        let n = DOSSIER_TABS.len();
        tab = if back {
            (tab + n - 1) % n
        } else {
            (tab + 1) % n
        };
    }
    if tab != ui.dossier_tab {
        // a new tab starts at its first hunter, at the top of the roster
        ui.dossier_tab = tab;
        ui.dossier_sel = 0;
        ui.dossier_scroll = 0.0;
    }
    let rows = dossier_rows_found(hs, tab, &found);
    let roster = roster_lines(&rows);
    let ids: Vec<usize> = rows.iter().filter_map(|r| r.1).collect();
    // the web's keys: Up/Down and Left/Right step one, PageUp/PageDown five, Home/End jump
    let nav_keys = [
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageDown,
        KeyCode::PageUp,
        KeyCode::Home,
        KeyCode::End,
    ];
    let moved = nav_keys.iter().any(|&key| is_key_pressed(key));
    if !ids.is_empty() {
        let last = ids.len() - 1;
        ui.dossier_sel = ui.dossier_sel.min(last);
        if is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::Right) {
            ui.dossier_sel = (ui.dossier_sel + 1).min(last);
        }
        if is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::Left) {
            ui.dossier_sel = ui.dossier_sel.saturating_sub(1);
        }
        if is_key_pressed(KeyCode::PageDown) {
            ui.dossier_sel = (ui.dossier_sel + 5).min(last);
        }
        if is_key_pressed(KeyCode::PageUp) {
            ui.dossier_sel = ui.dossier_sel.saturating_sub(5);
        }
        if is_key_pressed(KeyCode::Home) {
            ui.dossier_sel = 0;
        }
        if is_key_pressed(KeyCode::End) {
            ui.dossier_sel = last;
        }
    }
    let head_h = 13.0;
    let item_h = 53.0;
    let total: f32 = roster
        .iter()
        .map(|l| match l {
            RosterLine::Header(_) => head_h,
            RosterLine::Cells(_) => item_h,
        })
        .sum();
    let view_h = list.h - 6.0;
    if hovered(list) {
        ui.dossier_scroll -= wheel_notch(mouse_wheel().1, 26.0);
    }
    if moved {
        // the selected cell's line, found by walking the visual lines from the top
        let (mut acc, mut sel_top) = (0.0, 0.0);
        for l in &roster {
            match l {
                RosterLine::Header(_) => acc += head_h,
                RosterLine::Cells(cells) => {
                    if cells.iter().any(|&(k, _)| k == ui.dossier_sel) {
                        sel_top = acc;
                    }
                    acc += item_h;
                }
            }
        }
        if sel_top < ui.dossier_scroll {
            ui.dossier_scroll = sel_top;
        }
        if sel_top + item_h > ui.dossier_scroll + view_h {
            ui.dossier_scroll = sel_top + item_h - view_h;
        }
    }
    ui.dossier_scroll = ui.dossier_scroll.clamp(0.0, (total - view_h).max(0.0));
    clip_begin(Rect::new(
        list.x + 1.0,
        list.y + 1.0,
        list.w - 2.0,
        list.h - 2.0,
    ));
    // two cells per line, with a small gap; each cell is a 48x48 portrait and a name
    let cell_gap = 4.0;
    let cell_w = (list.w - 6.0 - cell_gap) * 0.5;
    let mut y = list.y + 3.0 - ui.dossier_scroll;
    for entry in &roster {
        match entry {
            RosterLine::Header(text) => {
                // a full-width amber section name over a rule
                let hr = Rect::new(list.x + 3.0, y, list.w - 6.0, head_h);
                label_vc(a, F::Body, &track(text), hr.x + 2.0, hr.y, head_h, 6.0, p.amber);
                line(
                    vec2(hr.x, hr.y + head_h - 1.0),
                    vec2(hr.x + hr.w, hr.y + head_h - 1.0),
                    0.35,
                    alpha(p.amber, 0.5),
                );
                y += head_h;
            }
            RosterLine::Cells(cells) => {
                for (col, &(k, hi)) in cells.iter().enumerate() {
                    let h = &hs[hi];
                    let rr = Rect::new(
                        list.x + 3.0 + col as f32 * (cell_w + cell_gap),
                        y,
                        cell_w,
                        item_h,
                    );
                    if k == ui.dossier_sel {
                        sbox(rr, p.amber_fill, p.amber, 1.0, p.corner);
                    }
                    if hovered(rr) && hovered(list) && is_mouse_button_pressed(MouseButton::Left) {
                        ui.dossier_sel = k;
                    }
                    let open = dossier_open(hs, tab, &found, hi);
                    if let Some(t) = a.portraits.get(&h.id) {
                        let s = unit();
                        // a locked entry is a silhouette, the same tint as the detail portrait
                        let tint = if open { WHITE } else { DOSSIER_SILHOUETTE };
                        draw_texture_ex(
                            t,
                            (rr.x + 2.0) * s,
                            (rr.y + 2.5) * s,
                            tint,
                            DrawTextureParams {
                                dest_size: Some(vec2(48.0 * s, 48.0 * s)),
                                ..Default::default()
                            },
                        );
                    }
                    if open {
                        let name = clip(a, F::Body, &h.name.to_uppercase(), rr.w - 54.0, 7.0);
                        label_vc(
                            a,
                            F::Body,
                            &name,
                            rr.x + 52.0,
                            rr.y,
                            rr.h,
                            7.0,
                            if k == ui.dossier_sel {
                                p.on_amber
                            } else {
                                p.body
                            },
                        );
                    } else {
                        // a locked hunter's name is a black square, centred on the row
                        let side = lh(F::Body, 7.0);
                        fill(
                            Rect::new(rr.x + 52.0, rr.y + (rr.h - side) * 0.5, side, side),
                            DOSSIER_SILHOUETTE,
                        );
                    }
                }
                y += item_h;
            }
        }
    }
    clip_end();
    if ids.is_empty() {
        if let Some(line) = dossier_empty_line(tab) {
            let lines = wrap(a, F::Body, line, list.w - 10.0, 7.0);
            for (n, l) in lines.iter().enumerate() {
                label(
                    a,
                    F::Body,
                    l,
                    list.x + 5.0,
                    list.y + 6.0 + n as f32 * lh(F::Body, 7.0),
                    7.0,
                    alpha(p.body, 0.5),
                );
            }
        }
    }
    // the details column
    let dx = list.x + list.w + 8.0;
    let dw = m.inner.x + m.inner.w - dx;
    if let Some(&hi) = ids.get(ui.dossier_sel) {
        let h = &hs[hi];
        let tier = ["EASY", "INTERMEDIATE", "DIFFICULT"][h.tier.min(2)];
        let open = dossier_open(hs, tab, &found, hi);
        let text = dossier_text(
            open,
            &h.name,
            crate::hunters::gimmicks::describe(&h.id),
            h.chase,
            h.sight,
            h.hearing,
        );
        let status = if !open {
            if signed_in {
                "ACCOUNT DISCOVERY REQUIRED".to_string()
            } else {
                "SIGN IN TO RETAIN DISCOVERIES".to_string()
            }
        } else if tab == CUSTOM_TAB {
            // the pack's name is shown only once the hunter is open (it hints at the origin)
            match content.packs.iter().find(|pk| pk.id == h.pack) {
                Some(pk) => format!("MOD PACK // {}", pk.name.to_uppercase()),
                None => "MOD PACK".to_string(),
            }
        } else if is_secret_hunter(h) {
            "FOUND // UNLISTED".to_string()
        } else if h.category == "void" {
            "VOID INCURSION // EXTRA HUNTER".to_string()
        } else if crate::content::hunters::SPECIAL_IDS.contains(&h.id.as_str()) {
            "RULE-SUMMONED // A-Z SPECIAL".to_string()
        } else {
            format!("A-Z ROUTE DRAW // {tier}")
        };
        // the 72x72 portrait, with the title and the status line beside it
        if let Some(t) = a.portraits.get(&h.id) {
            let s = unit();
            let tint = if open { WHITE } else { DOSSIER_SILHOUETTE };
            draw_texture_ex(
                t,
                dx * s,
                split_y * s,
                tint,
                DrawTextureParams {
                    dest_size: Some(vec2(72.0 * s, 72.0 * s)),
                    ..Default::default()
                },
            );
        }
        let tx = dx + 72.0 + 8.0;
        let text_w = dw - 80.0;
        let name_h = lh(F::Title, 16.0);
        let status_h = lh(F::Body, 6.0);
        let status_lines = wrap(a, F::Body, &status, text_w, 6.0);
        let block_h = name_h + 3.0 + status_lines.len() as f32 * status_h;
        let ty = split_y + (72.0 - block_h) * 0.5;
        if open {
            label(a, F::Title, &clip(a, F::Title, &text.title, text_w, 16.0), tx, ty, 16.0, p.ink);
        } else {
            // a locked hunter's name is a black square, the height of the title line
            fill(Rect::new(tx, ty, name_h, name_h), DOSSIER_SILHOUETTE);
        }
        for (n, l) in status_lines.iter().enumerate() {
            label(a, F::Body, l, tx, ty + name_h + 3.0 + n as f32 * status_h, 6.0, p.amber);
        }
        // three stat cards along the bottom: CHASE, SIGHT, HEARING
        let stats_h = 4.0 + status_h + 3.0 + lh(F::Body, 7.0) + 4.0;
        let stats_y = split_y + split_h - stats_h;
        // the description, wrapped under the portrait (blank for a locked hunter)
        let mut y = split_y + 72.0 + 8.0;
        for l in wrap(a, F::Body, &text.description, dw, 7.0) {
            if y + lh(F::Body, 7.0) > stats_y - 6.0 {
                break;
            }
            label(a, F::Body, &l, dx, y, 7.0, p.body);
            y += lh(F::Body, 7.0);
        }
        let gap = 4.0;
        let card_w = (dw - 2.0 * gap) / 3.0;
        for (i, (name, value)) in DOSSIER_STAT_LABELS.iter().zip(text.stats.iter()).enumerate() {
            let card = Rect::new(dx + i as f32 * (card_w + gap), stats_y, card_w, stats_h);
            panel_box(card, p.inset, p.panel_edge);
            let cx = card.x + 5.0;
            label(a, F::Body, name, cx, card.y + 4.0, 6.0, p.muted);
            label(
                a,
                F::Body,
                &clip(a, F::Body, value, card_w - 10.0, 7.0),
                cx,
                card.y + 4.0 + status_h + 3.0,
                7.0,
                if open { p.ink } else { p.muted },
            );
        }
    }
    m.close || is_key_pressed(KeyCode::Escape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_notch_scrolls_only_while_the_wheel_moves() {
        assert_eq!(wheel_notch(0.0, 26.0), 0.0);
        assert_eq!(wheel_notch(1.0, 26.0), 26.0);
        assert_eq!(wheel_notch(-0.5, 26.0), -26.0);
    }

    #[test]
    fn title_buttons_follow_the_web_order_for_a_normal_account() {
        assert_eq!(
            title_entries("ALICE"),
            vec![
                TitleEntry::PublicRoute,
                TitleEntry::Create,
                TitleEntry::Join,
                TitleEntry::Account,
                TitleEntry::Settings,
                TitleEntry::Quit,
            ]
        );
    }

    #[test]
    fn title_buttons_follow_the_web_order_for_the_admin_account() {
        for name in ["KYTTU", "kyttu", "AYTTU", "ayttu"] {
            assert_eq!(
                title_entries(name),
                vec![
                    TitleEntry::PublicRoute,
                    TitleEntry::Create,
                    TitleEntry::Join,
                    TitleEntry::Account,
                    TitleEntry::Settings,
                    TitleEntry::Admin,
                    TitleEntry::Quit,
                ]
            );
        }
    }

    #[test]
    fn the_public_route_button_asks_the_app_to_join_the_public_room_and_the_others_go_where_they_did() {
        assert!(matches!(title_entry_action(TitleEntry::PublicRoute), Action::PublicRoute));
        assert!(matches!(title_entry_action(TitleEntry::Create), Action::Go(Screen::Create)));
        assert!(matches!(title_entry_action(TitleEntry::Join), Action::Go(Screen::Join)));
        assert!(matches!(title_entry_action(TitleEntry::Quit), Action::Go(Screen::Quit)));
        for name in ["ALICE", "KYTTU"] {
            for e in title_entries(name) {
                assert!(!matches!(title_entry_action(e), Action::None), "{e:?} does something");
            }
        }
    }

    #[test]
    fn title_focus_starts_on_public_route_and_finds_each_button() {
        assert_eq!(title_focus(TitleEntry::PublicRoute, "ALICE"), 0);
        assert_eq!(title_focus(TitleEntry::Create, "ALICE"), 1);
        assert_eq!(title_focus(TitleEntry::Quit, "ALICE"), 5);
        assert_eq!(title_focus(TitleEntry::Quit, "KYTTU"), 6);
    }

    #[test]
    fn title_account_button_reads_account_once_signed_in() {
        assert_eq!(account_label("ALICE"), "ACCOUNT");
        assert_eq!(account_label("kyttu"), "ACCOUNT");
        assert_eq!(account_label(""), "REGISTER / SIGN IN");
    }

    #[test]
    fn create_room_focus_index_is_its_own_button() {
        // CREATE ROOM's cancel hands focus back to CREATE ROOM (app/mod.rs), not PUBLIC ROUTE
        for name in ["ALICE", "KYTTU"] {
            assert_ne!(title_focus(TitleEntry::Create, name), 0, "{name}");
            assert_eq!(title_entries(name)[title_focus(TitleEntry::Create, name)], TitleEntry::Create);
        }
    }

    #[test]
    fn admin_is_still_a_title_entry_for_the_admin_account_only() {
        assert!(title_entries("KYTTU").contains(&TitleEntry::Admin));
        assert!(title_entries("AYTTU").contains(&TitleEntry::Admin));
        assert!(title_entries("ayttu").contains(&TitleEntry::Admin));
        assert!(!title_entries("ALICE").contains(&TitleEntry::Admin));
    }

    const DESC: &str = "Hold your light on her and a fuse starts...";
    // Aki's real profile: chase 3.55, sight 8.8, hearing 0.82
    const CHASE: f32 = 3.55;
    const SIGHT: f32 = 8.8;
    const HEARING: f32 = 0.82;

    #[test]
    fn undiscovered_hunter_is_blank_in_every_field() {
        for name in ["Aki", "Mara Vex", "The Twin"] {
            let t = dossier_text(false, name, DESC, CHASE, SIGHT, HEARING);
            assert_eq!(t.title, "");
            assert_eq!(t.description, "");
            assert_eq!(t.stats, ["???", "???", "???"].map(String::from));
            let title_and_stats = std::iter::once(&t.title).chain(t.stats.iter());
            for field in title_and_stats {
                assert!(
                    !field.to_lowercase().contains(&name.to_lowercase()),
                    "{name} leaked through {field:?}"
                );
            }
        }
    }

    #[test]
    fn undiscovered_hunter_shows_no_number_and_no_description_text() {
        let t = dossier_text(false, "Aki", DESC, CHASE, SIGHT, HEARING);
        for field in std::iter::once(&t.title).chain(t.stats.iter()) {
            for value in ["3.55", "8.8", "82", "c/s", "cells", "DEAF"] {
                assert!(!field.contains(value), "{value} leaked through {field:?}");
            }
        }
        assert!(t.description.is_empty());
        assert!(!t.description.contains("fuse"));
    }

    #[test]
    fn undiscovered_hunter_has_no_name_text_so_the_black_square_is_drawn_instead() {
        let t = dossier_text(false, "Aki", DESC, CHASE, SIGHT, HEARING);
        assert!(t.title.is_empty());
        let open = dossier_text(true, "Aki", DESC, CHASE, SIGHT, HEARING);
        assert_eq!(open.title, "AKI");
    }

    #[test]
    fn draw_pool_weights_match_godot_tier_table_and_each_row_sums_to_one() {
        // data/hunters/draw_pool.json; Godot game/config/hunter_roster.gd TIER_WEIGHTS_BY_DIFFICULTY
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../data/hunters/draw_pool.json")).unwrap();
        let w = |d: &str| -> [f64; 3] {
            ["easy", "intermediate", "difficult"].map(|t| v["tier_weights"][d][t].as_f64().unwrap())
        };
        assert_eq!(w("relaxed"), [0.70, 0.25, 0.05]);
        assert_eq!(w("standard"), [0.35, 0.50, 0.15]);
        assert_eq!(w("hard"), [0.2625, 0.5125, 0.225]);
        assert_eq!(w("nightmare"), [0.175, 0.525, 0.30]);
        for d in ["relaxed", "standard", "hard", "nightmare"] {
            let sum: f64 = w(d).iter().sum();
            assert!((sum - 1.0).abs() < 1e-9, "{d} weights sum to {sum}");
        }
    }

    #[test]
    fn a_secret_creature_has_no_row_header_or_counter_until_it_is_found() {
        let mut hs = mixed_roster();
        hs.push(hunter("white_wolf_spider", "secret", ""));
        let spider = hs.len() - 1;
        // nothing met: the Route tab is what it was, with no FOUND header and no spider row
        assert_eq!(dossier_rows_found(&hs, 0, &found(&[])), dossier_rows(&hs, 0));
        assert!(!listed(&dossier_rows(&hs, 0)).contains(&spider));
        assert!(dossier_rows(&hs, 0).iter().all(|r| r.0.as_deref() != Some("FOUND")));
        // met one: a FOUND section after VOID lists it
        let rows = dossier_rows_found(&hs, 0, &found(&["white_wolf_spider"]));
        let at = rows.iter().position(|r| r.0.as_deref() == Some("FOUND")).expect("a FOUND header");
        assert_eq!(rows[at + 1].1, Some(spider));
        assert!(at > rows.iter().position(|r| r.0.as_deref() == Some("VOID")).unwrap());
        // it opens on its own, and meeting it unlocks no A-Z or VOID category
        assert!(dossier_open(&hs, 0, &found(&["white_wolf_spider"]), spider));
        assert!(!dossier_open(&hs, 0, &found(&["white_wolf_spider"]), 0), "aki stays locked");
        assert!(!dossier_open(&hs, 0, &found(&[]), spider));
        // the Custom tab never shows it
        assert!(!listed(&dossier_rows_found(&hs, 1, &found(&["white_wolf_spider"]))).contains(&spider));
    }

    #[test]
    fn category_with_no_discovered_hunter_is_locked() {
        let found: Vec<String> = vec!["other".to_string()];
        assert!(category_locked(["void_a", "void_b"], &found));
        assert!(category_locked(std::iter::empty::<&str>(), &found));
    }

    #[test]
    fn category_with_one_discovered_hunter_is_unlocked() {
        let found: Vec<String> = vec!["void_b".to_string()];
        assert!(!category_locked(["void_a", "void_b"], &found));
    }

    #[test]
    fn discovered_hunter_shows_every_field() {
        let t = dossier_text(true, "Aki", DESC, CHASE, SIGHT, HEARING);
        assert_eq!(
            t,
            DossierText {
                title: "AKI".to_string(),
                description: DESC.to_string(),
                stats: ["3.55 c/s", "8.8 cells", "82%"].map(String::from),
            }
        );
    }

    #[test]
    fn a_deaf_hunter_shows_deaf_and_a_full_hearing_scale_shows_100() {
        let deaf = dossier_text(true, "Aki", DESC, CHASE, SIGHT, 0.0);
        assert_eq!(deaf.stats[2], "DEAF");
        let full = dossier_text(true, "Aki", DESC, CHASE, SIGHT, 1.0);
        assert_eq!(full.stats[2], "100%");
    }

    // -- the dossier tabs ----------------------------------------------------------------

    /// One hunter parsed from an lr.hunter/1 document, the way the loaders make them.
    fn hunter(id: &str, category: &str, pack: &str) -> HunterDef {
        let v = serde_json::json!({ "id": id, "name": id, "category": category });
        crate::content::hunters::parse(&v, None, pack).unwrap()
    }

    /// Built-in: 0 aki (A-Z), 1 kathy (A-Z), 2 mara (VOID). Pack "dossier_test": 3 test_mod_a
    /// (A-Z), 4 test_mod_void (VOID).
    fn mixed_roster() -> Vec<HunterDef> {
        vec![
            hunter("aki", "az", ""),
            hunter("kathy", "az", ""),
            hunter("mara", "void", ""),
            hunter("test_mod_a", "az", "dossier_test"),
            hunter("test_mod_void", "void", "dossier_test"),
        ]
    }

    fn found(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    /// The roster hunter indexes a tab lists, in order.
    fn listed(rows: &[DossierRow]) -> Vec<usize> {
        rows.iter().filter_map(|r| r.1).collect()
    }

    #[test]
    fn route_tab_lists_only_built_in_hunters_under_their_headers() {
        let hs = mixed_roster();
        let rows = dossier_rows(&hs, 0);
        assert_eq!(
            rows,
            vec![
                (Some("A-Z HUNTERS".to_string()), None),
                (None, Some(0)),
                (None, Some(1)),
                (Some("VOID".to_string()), None),
                (None, Some(2)),
            ]
        );
        assert!(listed(&rows).iter().all(|&i| !hs[i].is_pack()));
    }

    #[test]
    fn route_tab_headers_show_their_names_even_while_locked() {
        let hs = mixed_roster();
        let rows = dossier_rows(&hs, 0);
        assert_eq!(rows[0].0.as_deref(), Some("A-Z HUNTERS"));
        assert_eq!(rows[3].0.as_deref(), Some("VOID"));
        assert!(!dossier_open(&hs, 0, &found(&[]), 0));
        assert!(!dossier_open(&hs, 0, &found(&[]), 2));
    }

    #[test]
    fn route_sections_are_sorted_by_name() {
        let hs = vec![
            hunter("zed", "az", ""),
            hunter("ann", "az", ""),
            hunter("ivy", "void", ""),
            hunter("bo", "void", ""),
        ];
        assert_eq!(listed(&dossier_rows(&hs, 0)), vec![1, 0, 3, 2]);
    }

    #[test]
    fn route_roster_lines_put_two_hunters_per_line_under_full_width_headers() {
        let hs = mixed_roster();
        let lines = roster_lines(&dossier_rows(&hs, 0));
        assert_eq!(
            lines,
            vec![
                RosterLine::Header("A-Z HUNTERS".to_string()),
                RosterLine::Cells(vec![(0, 0), (1, 1)]),
                RosterLine::Header("VOID".to_string()),
                RosterLine::Cells(vec![(2, 2)]),
            ]
        );
    }

    #[test]
    fn a_header_starts_a_new_line_even_after_a_single_hunter() {
        let hs = vec![
            hunter("aki", "az", ""),
            hunter("mara", "void", ""),
            hunter("tobi", "void", ""),
            hunter("zed", "void", ""),
        ];
        let lines = roster_lines(&dossier_rows(&hs, 0));
        assert_eq!(
            lines,
            vec![
                RosterLine::Header("A-Z HUNTERS".to_string()),
                RosterLine::Cells(vec![(0, 0)]),
                RosterLine::Header("VOID".to_string()),
                RosterLine::Cells(vec![(1, 1), (2, 2)]),
                RosterLine::Cells(vec![(3, 3)]),
            ]
        );
    }

    #[test]
    fn custom_tab_lists_only_pack_hunters_as_one_list() {
        let hs = mixed_roster();
        let rows = dossier_rows(&hs, 1);
        assert_eq!(rows, vec![(None, Some(3)), (None, Some(4))]);
    }

    #[test]
    fn a_mixed_list_splits_into_two_disjoint_tabs_that_cover_every_hunter() {
        let hs = mixed_roster();
        let route = listed(&dossier_rows(&hs, 0));
        let custom = listed(&dossier_rows(&hs, 1));
        assert!(
            route.iter().all(|i| !custom.contains(i)),
            "a hunter is on both tabs"
        );
        let mut both: Vec<usize> = route.iter().chain(custom.iter()).copied().collect();
        both.sort_unstable();
        assert_eq!(both, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_pack_override_of_a_built_in_id_is_a_custom_hunter_not_a_route_hunter() {
        let base = hunter("aki", "az", "");
        let v = serde_json::json!({ "id": "aki", "speed": 1.1 });
        let overridden = crate::content::hunters::parse(&v, Some(&base), "dossier_test").unwrap();
        let hs = vec![overridden, hunter("kathy", "az", "")];
        assert_eq!(listed(&dossier_rows(&hs, 0)), vec![1]);
        assert_eq!(listed(&dossier_rows(&hs, 1)), vec![0]);
    }

    #[test]
    fn empty_custom_list_is_empty_and_says_so() {
        let hs = vec![hunter("aki", "az", ""), hunter("mara", "void", "")];
        assert!(dossier_rows(&hs, 1).is_empty());
        assert_eq!(dossier_empty_line(1), Some("No custom hunters installed"));
        assert_eq!(dossier_empty_line(0), None);
    }

    #[test]
    fn custom_rows_are_locked_until_each_hunter_is_discovered() {
        let hs = mixed_roster();
        // a built-in discovery does not open a pack hunter
        assert!(!dossier_open(&hs, 1, &found(&["aki"]), 3));
        // the locked row hides the name, description and stats
        let t = dossier_text(
            dossier_open(&hs, 1, &found(&[]), 3),
            "test_mod_a",
            DESC,
            CHASE,
            SIGHT,
            HEARING,
        );
        // the name is a black square drawn over an empty title (see undiscovered_hunter_has_no_name_text...)
        assert_eq!(t.title, "");
        assert!(t.description.is_empty() && t.stats == ["???", "???", "???"].map(String::from));
        assert!(!t.title.contains("test_mod_a"));
        // each pack hunter unlocks on its own, with no category to unlock first
        assert!(dossier_open(&hs, 1, &found(&["test_mod_a"]), 3));
        assert!(!dossier_open(&hs, 1, &found(&["test_mod_a"]), 4));
        assert!(dossier_open(&hs, 1, &found(&["test_mod_void"]), 4));
    }

    #[test]
    fn pack_discovery_does_not_unlock_a_built_in_category() {
        let hs = mixed_roster();
        // a pack VOID hunter is discovered, but the built-in VOID category stays locked
        assert!(!dossier_open(&hs, 0, &found(&["test_mod_void"]), 2));
        // the A-Z category opens only on a built-in A-Z member: a pack A-Z hunter does not open
        // the built-in A-Z hunters
        assert!(route_category_locked(&hs, &found(&["test_mod_a"]), false));
        assert!(route_category_locked(&hs, &found(&["test_mod_void"]), true));
        // a built-in A-Z member opens its own category, so its neighbour opens too
        assert!(dossier_open(&hs, 0, &found(&["kathy"]), 1));
        assert!(!dossier_open(&hs, 0, &found(&["kathy"]), 0));
        // the headers keep their names while locked
        assert_eq!(dossier_rows(&hs, 0)[0].0.as_deref(), Some("A-Z HUNTERS"));
    }

    #[test]
    fn custom_tab_ignores_categories() {
        let hs = mixed_roster();
        // the pack A-Z hunter is locked while the pack VOID hunter is open: no category lock
        let all = found(&["test_mod_void"]);
        assert!(dossier_open(&hs, 1, &all, 4));
        assert!(!dossier_open(&hs, 1, &all, 3));
        assert!(dossier_rows(&hs, 1).iter().all(|r| r.0.is_none()));
    }

    #[test]
    fn opening_the_dossier_starts_on_the_route_tab_at_its_first_hunter() {
        let mut ui = Ui::default();
        assert_eq!(ui.dossier_tab, 0);
        ui.dossier_tab = 1;
        ui.dossier_sel = 4;
        ui.go(Screen::Dossier);
        assert_eq!(ui.dossier_tab, 0);
        assert_eq!(ui.dossier_sel, 0);
    }

    #[test]
    fn the_tabs_are_named_as_the_spec_names_them() {
        assert_eq!(DOSSIER_TABS, ["ROUTE DOSSIER", "CUSTOM HUNTERS"]);
    }
}

#[cfg(test)]
mod lobby_tests {
    use super::*;

    fn labels(mode: RoomMode) -> Vec<&'static str> {
        lobby_buttons(mode)
            .into_iter()
            .map(|b| lobby_label(b, mode))
            .collect()
    }

    #[test]
    fn solo_actions_row_order() {
        assert_eq!(
            labels(RoomMode::Solo),
            vec![
                "ENTER ROUTE",
                "CONTINUE RUN",
                "CHARACTER FILES",
                "BACK TO LAST ROUTE",
            ]
        );
    }

    #[test]
    fn host_actions_row_order() {
        assert_eq!(
            labels(RoomMode::Host),
            vec!["ENTER ROUTE", "CHARACTER FILES", "LEAVE ROOM"]
        );
    }

    #[test]
    fn guest_actions_row_order() {
        assert_eq!(
            labels(RoomMode::Guest),
            vec!["WAITING FOR HOST", "CHARACTER FILES", "LEAVE ROOM"]
        );
    }

    #[test]
    fn the_lobby_has_no_account_button() {
        for mode in [RoomMode::Solo, RoomMode::Host, RoomMode::Guest] {
            {
                assert!(!labels(mode).iter().any(|l| l.contains("ACCOUNT") || l.contains("SIGN IN")), "{mode:?}");
            }
        }
    }

    #[test]
    fn continue_run_is_solo_only() {
        for mode in [RoomMode::Host, RoomMode::Guest] {
            assert!(!lobby_buttons(mode).contains(&LobbyButton::ContinueRun), "{mode:?}");
        }
        assert!(lobby_buttons(RoomMode::Solo).contains(&LobbyButton::ContinueRun));
    }

    #[test]
    fn guest_enter_is_locked_and_continue_needs_a_save() {
        assert!(lobby_locked(LobbyButton::Enter, RoomMode::Guest, false));
        assert!(!lobby_locked(LobbyButton::Enter, RoomMode::Host, false));
        assert!(!lobby_locked(LobbyButton::Enter, RoomMode::Solo, false));
        assert!(lobby_locked(LobbyButton::ContinueRun, RoomMode::Solo, false));
        assert!(!lobby_locked(LobbyButton::ContinueRun, RoomMode::Solo, true));
    }

    #[test]
    fn focus_finds_each_button_in_its_mode() {
        assert_eq!(lobby_focus(RoomMode::Solo, LobbyButton::CharacterFiles), 2);
        assert_eq!(lobby_focus(RoomMode::Host, LobbyButton::CharacterFiles), 1);
        assert_eq!(lobby_focus(RoomMode::Guest, LobbyButton::Leave), 2);
    }

    #[test]
    fn difficulty_labels_show_standard_as_normal() {
        assert_eq!(difficulty_label(Difficulty::Relaxed), "RELAXED");
        assert_eq!(difficulty_label(Difficulty::Standard), "NORMAL");
        assert_eq!(difficulty_label(Difficulty::Hard), "HARD");
        // the internal name and id are what the protocol and the saves use
        assert_eq!(Difficulty::Standard.name(), "STANDARD");
        assert_eq!(Difficulty::Standard.id(), "standard");
    }

    #[test]
    fn difficulty_row_is_relaxed_normal_hard_nightmare() {
        let shown: Vec<&str> = lobby_row().iter().map(|d| difficulty_label(*d)).collect();
        assert_eq!(shown, vec!["RELAXED", "NORMAL", "HARD", "NIGHTMARE"]);
    }

    /// The embedded lobby.json, read as the game reads it.
    fn embedded_lobby_text() -> &'static str {
        crate::content::embedded::data_under("lobby.json").find(|(p, _)| *p == "lobby.json").expect("embedded").1
    }

    #[test]
    fn lobby_json_standard_values_match_the_current_game() {
        let content = lobby_difficulty();
        assert_eq!(content.title, "DIFFICULTY");
        assert_eq!(content.guest_title, "DIFFICULTY // HOST SETS {label}");
        let standard = content.options.iter().find(|o| o.difficulty == Difficulty::Standard).expect("standard");
        assert_eq!(standard.label, "NORMAL");
        assert_eq!(standard.blurb, "The route as meant: hunters work together, so keep moving.");
        assert_eq!(difficulty_blurb(Difficulty::Hard), "Hunters wake at once and hear more. Spend your light well.");
    }

    #[test]
    fn changing_a_label_in_the_data_changes_the_lobby_label() {
        let edited = embedded_lobby_text().replacen("\"RELAXED\"", "\"CALM\"", 1);
        let parsed = parse_lobby_difficulty(&edited).expect("valid");
        let relaxed = parsed.options.iter().find(|o| o.difficulty == Difficulty::Relaxed).expect("relaxed");
        assert_eq!(relaxed.label, "CALM");
        // the untouched labels stay as they were
        assert_eq!(parsed.options.iter().find(|o| o.difficulty == Difficulty::Standard).unwrap().label, "NORMAL");
    }

    #[test]
    fn changing_the_row_order_in_the_data_changes_the_row() {
        let mut root: serde_json::Value = serde_json::from_str(embedded_lobby_text()).expect("json");
        root["difficulty"]["options"].as_array_mut().expect("array").reverse();
        let parsed = parse_lobby_difficulty(&root.to_string()).expect("valid");
        let order: Vec<Difficulty> = parsed.options.iter().map(|o| o.difficulty).collect();
        assert_eq!(order, vec![Difficulty::Nightmare, Difficulty::Hard, Difficulty::Standard, Difficulty::Relaxed]);
    }

    #[test]
    fn an_option_with_an_unknown_id_is_an_error() {
        let edited = embedded_lobby_text().replacen("\"hard\"", "\"brutal\"", 1);
        assert!(parse_lobby_difficulty(&edited).is_err());
    }

    #[test]
    fn the_guest_title_takes_the_host_label_from_the_data() {
        let edited = embedded_lobby_text().replacen("HOST SETS {label}", "HOST PICKED {label}", 1);
        let parsed = parse_lobby_difficulty(&edited).expect("valid");
        assert!(parsed.guest_title.contains("{label}"));
        assert_eq!(parsed.guest_title.replace("{label}", "HARD"), "DIFFICULTY // HOST PICKED HARD");
    }

    #[test]
    fn nightmare_is_selectable_in_the_lobby() {
        assert!(LOBBY.contains(&Difficulty::Nightmare));
        assert_eq!(difficulty_label(Difficulty::Nightmare), "NIGHTMARE");
        assert_eq!(
            pick_difficulty(RoomMode::Host, Difficulty::Standard, Difficulty::Nightmare),
            Difficulty::Nightmare
        );
        assert_eq!(difficulty_row_title(RoomMode::Solo, Difficulty::Nightmare), "DIFFICULTY");
    }

    #[test]
    fn guest_cannot_change_difficulty() {
        assert!(!difficulty_editable(RoomMode::Guest));
        assert_eq!(
            pick_difficulty(RoomMode::Guest, Difficulty::Standard, Difficulty::Hard),
            Difficulty::Standard
        );
        assert_eq!(
            pick_difficulty(RoomMode::Guest, Difficulty::Relaxed, Difficulty::Relaxed),
            Difficulty::Relaxed
        );
    }

    #[test]
    fn a_guest_cannot_change_any_lobby_difficulty_including_nightmare() {
        for d in lobby_row() {
            assert_eq!(pick_difficulty(RoomMode::Guest, Difficulty::Hard, d), Difficulty::Hard, "{d:?}");
        }
    }

    #[test]
    fn a_guest_sees_a_nightmare_host_locked_in_the_row_title() {
        assert_eq!(
            difficulty_row_title(RoomMode::Guest, Difficulty::Nightmare),
            "DIFFICULTY // HOST SETS NIGHTMARE"
        );
    }

    #[test]
    fn guest_lobby_shows_the_hosts_difficulty_locked() {
        assert_eq!(difficulty_row_title(RoomMode::Guest, Difficulty::Hard), "DIFFICULTY // HOST SETS HARD");
        assert_eq!(difficulty_row_title(RoomMode::Guest, Difficulty::Standard), "DIFFICULTY // HOST SETS NORMAL");
        assert_eq!(difficulty_row_title(RoomMode::Host, Difficulty::Hard), "DIFFICULTY");
        assert_eq!(difficulty_row_title(RoomMode::Solo, Difficulty::Relaxed), "DIFFICULTY");
    }

    #[test]
    fn host_and_solo_can_change_difficulty() {
        assert!(difficulty_editable(RoomMode::Host));
        assert!(difficulty_editable(RoomMode::Solo));
        assert_eq!(
            pick_difficulty(RoomMode::Host, Difficulty::Standard, Difficulty::Hard),
            Difficulty::Hard
        );
        assert_eq!(
            pick_difficulty(RoomMode::Solo, Difficulty::Standard, Difficulty::Relaxed),
            Difficulty::Relaxed
        );
    }

    #[test]
    fn capacity_uses_the_live_count_and_never_invents_one() {
        let max = crate::net::api::MAX_PLAYERS;
        assert_eq!(capacity_text(RoomMode::Host, None), "UNLIMITED");
        assert_eq!(capacity_text(RoomMode::Host, Some(3)), "3 SIGNALS // UNLIMITED");
        assert_eq!(capacity_text(RoomMode::Host, Some(1)), "1 SIGNAL // UNLIMITED");
        assert_eq!(capacity_text(RoomMode::Guest, None), format!("UP TO {max} SIGNALS"));
        assert_eq!(capacity_text(RoomMode::Guest, Some(2)), format!("2 / {max} SIGNALS"));
        assert_eq!(capacity_text(RoomMode::Solo, None), "1 SIGNAL");
    }

    #[test]
    fn roster_is_this_runner_plus_the_hosts_guests() {
        let solo = lobby_players(RoomMode::Solo, "kyttu", &[]);
        assert_eq!(solo.len(), 1);
        assert_eq!(solo[0].name, "KYTTU");
        assert!(solo[0].you);

        let host = lobby_players(RoomMode::Host, "kyttu", &["ANA".to_string()]);
        assert_eq!(host.len(), 2);
        assert!(host[0].host && host[0].you);
        assert_eq!(host[0].status, "HOST // STAGING");
        assert_eq!(host[1].name, "ANA");
        assert!(!host[1].host && !host[1].you);

        // a guest sees itself, the host and the other guests
        let guest = lobby_players(RoomMode::Guest, "ana", &["KYTTU".to_string(), "BO".to_string()]);
        assert_eq!(guest.len(), 3);
        assert!(guest[0].you && !guest[0].host);
        assert_eq!((guest[1].name.as_str(), guest[1].host, guest[1].you), ("KYTTU", true, false));
        assert_eq!((guest[2].name.as_str(), guest[2].host, guest[2].you), ("BO", false, false));
    }

    #[test]
    fn the_lobby_lists_every_connected_runner_once_with_the_local_one_marked() {
        // a fake room: host KYTTU with guests ANA and BO, seen from the host and then from guest ANA
        let host_roster = ["ANA".to_string(), "BO".to_string()];
        let host = lobby_players(RoomMode::Host, "kyttu", &host_roster);
        let names: Vec<&str> = host.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["KYTTU", "ANA", "BO"]);
        assert_eq!(host.iter().filter(|p| p.you).count(), 1);
        assert!(host[0].you && host[0].host);

        let guest_roster = ["KYTTU".to_string(), "ANA".to_string(), "BO".to_string()];
        let guest = lobby_players(RoomMode::Guest, "ana", &guest_roster);
        let names: Vec<&str> = guest.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["ANA", "KYTTU", "BO"]);
        assert_eq!(guest.iter().filter(|p| p.you).count(), 1);
        assert_eq!(guest.iter().filter(|p| p.host).count(), 1);
        assert!(guest[0].you && !guest[0].host);
        assert!(guest[1].host && !guest[1].you);
        assert_eq!(guest[1].status, "HOST // STAGING");
    }


    fn owner_view(owner: bool, peers: &[(u32, &str)]) -> OwnerView {
        OwnerView { owner, joined: true, peers: peers.iter().map(|(p, n)| (*p, n.to_string())).collect(), waiting: None, status: String::new() }
    }

    #[test]
    fn the_owner_has_start_restart_and_end_where_other_guests_wait_for_the_host() {
        let labels: Vec<String> = lobby_buttons_for(RoomMode::Guest, true).into_iter().map(|b| lobby_text(b, RoomMode::Guest)).collect();
        assert_eq!(labels, ["START ROUTE", "RESTART ROUTE", "END ROUTE", "CHARACTER FILES", "LEAVE ROOM"]);
        // any other guest keeps the locked WAITING FOR HOST row, and the owner flag means nothing outside a guest's room
        assert_eq!(lobby_buttons_for(RoomMode::Guest, false), lobby_buttons(RoomMode::Guest));
        for mode in [RoomMode::Solo, RoomMode::Host] {
            assert_eq!(lobby_buttons_for(mode, true), lobby_buttons(mode), "{mode:?}");
        }
        assert_eq!(lobby_focus_for(RoomMode::Guest, true, LobbyButton::Leave), 4);
        assert_eq!(lobby_focus_for(RoomMode::Guest, true, LobbyButton::CharacterFiles), 3);
    }

    #[test]
    fn each_owner_button_sends_its_command() {
        assert_eq!(lobby_owner_action(LobbyButton::StartRoute), Some(OwnerAction::Start));
        assert_eq!(lobby_owner_action(LobbyButton::RestartRoute), Some(OwnerAction::Restart));
        assert_eq!(lobby_owner_action(LobbyButton::EndRoute), Some(OwnerAction::End));
        for b in [LobbyButton::Enter, LobbyButton::ContinueRun, LobbyButton::CharacterFiles, LobbyButton::Leave] {
            assert_eq!(lobby_owner_action(b), None, "{b:?}");
        }
    }

    #[test]
    fn the_owner_buttons_wait_for_the_room_to_open() {
        for b in [LobbyButton::StartRoute, LobbyButton::RestartRoute, LobbyButton::EndRoute] {
            assert!(lobby_locked_for(b, RoomMode::Guest, false, false), "{b:?} while the server opens the room");
            assert!(!lobby_locked_for(b, RoomMode::Guest, false, true), "{b:?} once joined");
        }
        // the rest follow the ordinary rule
        assert!(!lobby_locked_for(LobbyButton::Leave, RoomMode::Guest, false, false), "a player can always leave");
        assert!(lobby_locked_for(LobbyButton::Enter, RoomMode::Guest, false, true));
    }

    #[test]
    fn only_the_owner_gets_kick_targets_and_never_for_itself_or_the_host() {
        let roster = vec!["KYTTU".to_string(), "BO".to_string(), "MAX".to_string()];
        let rows = |view: &OwnerView| with_kick_targets(lobby_players(RoomMode::Guest, "ana", &roster), view);
        // the owner ANA, in a room hosted by the server process KYTTU(host row), with BO and MAX
        let view = owner_view(true, &[(2, "ANA"), (3, "BO"), (4, "MAX"), (1, "KYTTU")]);
        let r = rows(&view);
        assert_eq!(r.iter().map(|p| (p.name.as_str(), p.kick)).collect::<Vec<_>>(), [("ANA", None), ("KYTTU", None), ("BO", Some(3)), ("MAX", Some(4))]);
        // a guest who does not own the room has no target at all, even with the same roster
        assert!(rows(&owner_view(false, &[(3, "BO"), (4, "MAX")])).iter().all(|p| p.kick.is_none()));
        // a name the room has no id for gets none
        assert!(rows(&owner_view(true, &[])).iter().all(|p| p.kick.is_none()));
        assert_eq!(view.peer_of("bo"), Some(3), "names match without regard to case");
    }

    #[test]
    fn the_create_dialog_asks_a_signed_out_player_to_sign_in() {
        // the same line the app flashes and the dialog shows (data/net/hosted_room.json)
        assert_eq!(hosted::rules().strings.sign_in_required, "SIGN IN TO CREATE AN ONLINE ROOM");
        assert!(!Ui::default().signed_in, "a fresh menu is signed out until the app says otherwise");
    }

    #[test]
    fn an_unnamed_runner_reads_runner() {
        assert_eq!(lobby_players(RoomMode::Solo, "", &[])[0].name, "RUNNER");
    }

    #[test]
    fn a_guest_in_a_room_is_listed_by_its_guest_name_never_runner() {
        for mode in [RoomMode::Guest, RoomMode::Host] {
            let rows = lobby_players(mode, "guest_4829", &["GUEST_2468".to_string()]);
            assert_eq!(rows[0].name, "GUEST_4829");
            assert!(rows[0].you);
            assert_eq!(rows[1].name, "GUEST_2468");
            assert!(rows.iter().all(|r| r.name != "RUNNER"));
            // a missing name still reads as a guest in a room
            assert_eq!(lobby_players(mode, "", &[])[0].name, "GUEST_0000");
        }
        // an account keeps its own name
        assert_eq!(lobby_players(RoomMode::Guest, "ana", &[])[0].name, "ANA");
    }
}
