//! The menus, laid out like the original's screens on its 480x270 design canvas (see style.rs):
//! the main menu (main_menu.tscn / main_menu.gd) with its CREATE ROOM, JOIN ROOM and QUIT
//! dialogs, the account panel (account_panel.gd), the route loading screen (route_loading.gd),
//! room staging (room_lobby.gd) and its hunter dossier.

use super::settings_panel::tab_bar;
use super::style::*;
use crate::app::assets::Assets;
use crate::app::checkpoint::Checkpoint;
use crate::config::difficulty::ALL;
use crate::config::Difficulty;
use crate::content::hunters::HunterDef;
use crate::content::Content;
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
    /// CREATE ROOM, online by default: open a room on the server and show its lobby.
    HostRoom,
    /// The code the JOIN ROOM dialog submitted (already checked by `world_sync::parse_join_code`).
    JoinCode(String),
}

/// Which room the lobby is showing: the solo room of CREATE ROOM, a room this player hosts, or one it joined.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RoomMode {
    #[default]
    Solo,
    Host,
    Guest,
}

/// The title's buttons. ADMIN shows for the admin's account only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TitleEntry {
    /// PUBLIC ROUTE: no behaviour in this build yet (it shows a notice).
    PublicRoute,
    Create,
    Join,
    Settings,
    Account,
    Admin,
    Quit,
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
    if !username.eq_ignore_ascii_case("KYTTU") {
        v.retain(|e| *e != TitleEntry::Admin);
    }
    v
}

/// The focus index of a title button for this account (the dialogs hand focus back to their button).
pub fn title_focus(entry: TitleEntry, username: &str) -> usize {
    title_entries(username).iter().position(|e| *e == entry).unwrap_or(0)
}

pub const VERSION: &str = "VERSION 0.0.1.0";

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
    pub acct_manage: bool,
    pub acct_user: String,
    pub acct_pass: String,
    pub acct_field: usize,
    pub acct_status: String,
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
            acct_manage: false,
            acct_user: String::new(),
            acct_pass: String::new(),
            acct_field: 0,
            acct_status: String::new(),
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
        self.screen = s;
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
            out = match entries[i] {
                TitleEntry::PublicRoute => {
                    // no public rooms in this build yet: show a plain notice, change nothing
                    ui.notice = "PUBLIC ROUTE IS NOT AVAILABLE IN THIS BUILD YET".into();
                    ui.notice_t = 4.0;
                    Action::None
                }
                TitleEntry::Create => Action::Go(Screen::Create),
                TitleEntry::Join => Action::Go(Screen::Join),
                TitleEntry::Settings => Action::Go(Screen::Settings),
                TitleEntry::Account => Action::Go(Screen::Account),
                TitleEntry::Admin => Action::Go(Screen::Admin),
                TitleEntry::Quit => Action::Go(Screen::Quit),
            };
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
    if active && is_key_pressed(KeyCode::Escape) {
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
    } else {
        "ONLINE ROOM: HOSTED THROUGH THE PRIMARY SERVER SO FRIENDS CAN JOIN.".to_string()
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
        // online by default: open the room on the server (a failure is flashed by the app)
        return Action::HostRoom;
    }
    Action::None
}

/// JOIN ROOM (main_menu.gd _on_join_pressed): the code field, its hint and status.
pub fn draw_join(a: &Assets, ui: &mut Ui, username: &str) -> Action {
    let _scope = MenuScope::enter();
    let p = pal();
    draw_title(a, ui, username, false);
    while let Some(ch) = get_char_pressed() {
        if ch.is_ascii_alphanumeric() && ui.join_code.len() < 5 {
            ui.join_code.push(ch.to_ascii_uppercase());
        }
    }
    if is_key_pressed(KeyCode::Backspace) {
        ui.join_code.pop();
    }
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
    let outside = is_mouse_button_pressed(MouseButton::Left) && !hovered(panel);
    if n_hit || (enter && ui.focus == 1) || is_key_pressed(KeyCode::Escape) || outside {
        return Action::Go(Screen::Title);
    }
    Action::None
}

// -- account (account_panel.gd) -------------------------------------------------------------

fn take_text(field: &mut String, max: usize, upper: bool) {
    while let Some(ch) = get_char_pressed() {
        let c = if upper { ch.to_ascii_uppercase() } else { ch };
        if c.is_control() || field.chars().count() >= max {
            continue;
        }
        if upper && !(c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        field.push(c);
    }
    if is_key_pressed(KeyCode::Backspace) {
        field.pop();
    }
}

fn valid_name(n: &str) -> bool {
    (3..=24).contains(&n.len()) && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The run summary GameRegistry.save_summary shows: DEPTH n // SCORE n // DIFFICULTY.
pub fn save_summary(save: Option<&Checkpoint>) -> Option<String> {
    save.map(|c| {
        format!(
            "DEPTH {} // SCORE {} // {}",
            if c.infinite { 1 - c.depth } else { c.depth },
            c.score,
            c.difficulty.name()
        )
    })
}

/// The account panel. Signed in (a profile name is set) it is the account card with the SAVES
/// rows; signed out it is the sign-in form. This build has no account server: the profile is
/// local, so signing in takes the name alone and passwords are not checked.
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
    let kick_w = tw(a, F::Body, &track("KYTTU'S LAB // ACCOUNT"), 6.0) + esc_close_size(a).x;
    if username.is_empty() {
        // the sign-in form (240x150)
        if is_key_pressed(KeyCode::Tab) {
            ui.acct_field = (ui.acct_field + 1) % 2;
        }
        if ui.acct_field == 0 {
            take_text(&mut ui.acct_user, 24, true);
        } else {
            take_text(&mut ui.acct_pass, 200, false);
        }
        let status = if ui.acct_status.is_empty() {
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
            "KYTTU'S LAB // ACCOUNT",
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
        if clicked(f1) {
            ui.acct_field = 0;
        }
        if clicked(f2) {
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
        if sign || make || is_key_pressed(KeyCode::Enter) {
            if valid_name(&ui.acct_user) {
                *username = ui.acct_user.clone();
                ui.acct_user.clear();
                ui.acct_pass.clear();
                ui.acct_status.clear();
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
        take_text(&mut ui.acct_user, 24, true);
        let status = if ui.acct_status.is_empty() {
            format!("SIGNED IN AS {username}")
        } else {
            ui.acct_status.clone()
        };
        let l6 = lh(F::Body, 6.0);
        let content_h = l6 * 3.0 + 22.0 * 2.0 + btn_h * 3.0 + 7.0 * 5.0;
        let m = modal(
            a,
            "KYTTU'S LAB // ACCOUNT",
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
        line_edit(
            a,
            Rect::new(m.inner.x, y, m.inner.w, 22.0),
            &ui.acct_user,
            "",
            8.0,
            true,
            false,
            ui.t,
        );
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
        "KYTTU'S LAB // ACCOUNT",
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
    y += row_h + 3.0;
    label_vc(
        a,
        F::Body,
        "NIGHT WATCH // NONE",
        m.inner.x,
        y,
        row_h,
        6.0,
        p.muted,
    );
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
        username.clear();
        ui.acct_field = 0;
        ui.acct_status.clear();
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

/// The difficulty's display label (web NORMAL). The id and the wire name stay `Difficulty::Standard`'s "STANDARD".
pub fn difficulty_label(d: Difficulty) -> &'static str {
    match d {
        Difficulty::Relaxed => "RELAXED",
        Difficulty::Standard => "NORMAL",
        Difficulty::Hard => "HARD",
    }
}

/// Only the host (a solo runner is its own host) sets the route's difficulty; a guest waits for the host's.
pub fn difficulty_editable(mode: RoomMode) -> bool {
    mode != RoomMode::Guest
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
    /// REGISTER / SIGN IN, or ACCOUNT once a name is set. Opens the account screen, which returns here.
    Account,
    /// The hunter dossier.
    CharacterFiles,
    /// Solo: BACK TO LAST ROUTE. A host or guest: LEAVE ROOM.
    Leave,
}

/// The actions row for this room, left to right.
pub fn lobby_buttons(mode: RoomMode) -> Vec<LobbyButton> {
    let mut v = vec![LobbyButton::Enter];
    if mode == RoomMode::Solo {
        v.push(LobbyButton::ContinueRun);
    }
    v.push(LobbyButton::Account);
    v.push(LobbyButton::CharacterFiles);
    v.push(LobbyButton::Leave);
    v
}

/// The keyboard focus index of `button` in this room's actions row.
pub fn lobby_focus(mode: RoomMode, button: LobbyButton) -> usize {
    lobby_buttons(mode).iter().position(|b| *b == button).unwrap_or(0)
}

/// The label of an actions-row button in this room.
pub fn lobby_label(button: LobbyButton, mode: RoomMode, signed_in: bool) -> &'static str {
    match button {
        LobbyButton::Enter if mode == RoomMode::Guest => "WAITING FOR HOST",
        LobbyButton::Enter => "ENTER ROUTE",
        LobbyButton::ContinueRun => "CONTINUE RUN",
        LobbyButton::Account if signed_in => "ACCOUNT",
        LobbyButton::Account => "REGISTER / SIGN IN",
        LobbyButton::CharacterFiles => "CHARACTER FILES",
        LobbyButton::Leave if mode == RoomMode::Solo => "BACK TO LAST ROUTE",
        LobbyButton::Leave => "LEAVE ROOM",
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
}

/// The roster the lobby shows: this runner first, then a host's guests. A guest only knows itself.
pub fn lobby_players(mode: RoomMode, username: &str, guests: &[String]) -> Vec<LobbyPlayer> {
    let host = mode == RoomMode::Host;
    let mut v = vec![LobbyPlayer {
        name: username_or(username),
        host,
        you: true,
        status: if host { "HOST // STAGING" } else { "STAGING" }.into(),
    }];
    if host {
        v.extend(guests.iter().map(|g| LobbyPlayer {
            name: g.to_uppercase(),
            host: false,
            you: false,
            status: "STAGING".into(),
        }));
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
    let signed_in = !username.is_empty();
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
        macroquad::miniquad::window::clipboard_set(code);
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
    let note = wrap(a, F::Body, NOTE, w - 12.0, 6.0);
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
    if guest {
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
    label(a, F::Body, "DIFFICULTY", cx, cy, 6.0, p.muted);
    let by = cy + l6 + 2.0;
    let mut bx = cx;
    for d in ALL.iter().copied() {
        let bw = button_min(a, difficulty_label(d), 7.0, false).x;
        let kind = if d == *diff {
            Kind::Selected
        } else {
            Kind::Secondary
        };
        if button(a, Rect::new(bx, by, bw, btn_h), difficulty_label(d), 7.0, kind, plain())
            && interactive
        {
            *diff = pick_difficulty(mode, *diff, d);
        }
        bx += bw + 4.0;
    }
    if interactive
        && difficulty_editable(mode)
        && (is_key_pressed(KeyCode::Left) || is_key_pressed(KeyCode::Right))
        && (is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift))
    {
        let i = ALL.iter().position(|d| d == diff).unwrap_or(1);
        *diff = ALL[if is_key_pressed(KeyCode::Left) {
            (i + 2) % 3
        } else {
            (i + 1) % 3
        }];
    }
    label(a, F::Body, diff.blurb(), cx, by + btn_h + 2.0, 6.0, p.muted);
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
    for pl in lobby_players(mode, username, guests) {
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
        let name_w = (w - 14.0 - status_w - 14.0).max(20.0);
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
            "OFFLINE SOLO ROOM",
            p.alert,
            format!("SOLO ROOM {code} // PRIMARY SERVER UNAVAILABLE"),
        ),
        RoomMode::Host => (
            "ONLINE ROOM // YOU ARE HOSTING",
            coop_blue(),
            format!("ROOM {code} // FRIENDS JOIN WITH THIS CODE"),
        ),
        RoomMode::Guest => (
            "ONLINE ROOM // JOINED",
            coop_blue(),
            format!("ROOM {code} // THE HOST STARTS THE ROUTE"),
        ),
    };
    label(a, F::Body, mode_line, x, y, 7.0, mode_colour);
    y += l7 + 5.0;
    label(a, F::Body, &mode_detail, x, y, 7.0, p.muted);
    y += l7 + 8.0;
    // the actions row: wraps onto a second line when the window is narrow
    let items = lobby_buttons(mode);
    let enter = interactive && nav(&mut ui.focus, items.len(), false);
    let ah = button_min(a, "X", menu_size(10.0, 1.0), false).y.max(18.0);
    let mut ax = x;
    let mut ay = y;
    for (i, b) in items.iter().enumerate() {
        let (fs, kind) = if *b == LobbyButton::Enter {
            (menu_size(10.0, 1.0), Kind::Primary)
        } else {
            (7.0, Kind::Secondary)
        };
        let text = lobby_label(*b, mode, signed_in);
        let locked = lobby_locked(*b, mode, save.is_some());
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
                LobbyButton::Account => Action::Go(Screen::Account),
                LobbyButton::CharacterFiles => Action::Go(Screen::Dossier),
                LobbyButton::Leave => Action::Go(Screen::Title),
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
}

/// PlayerListPanel: title, the column header and one row per player.
pub fn player_list(a: &Assets, r: Rect, rows: &[PlayerRow]) {
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
    for (i, h) in ["", "", "NAME", "PING", "STATUS", "FLOOR", "VOICE"]
        .iter()
        .enumerate()
    {
        label(a, F::Body, h, xs[i], y, 6.0, p.muted);
    }
    y += lh(F::Body, 6.0) + 3.0;
    let rh = lh(F::Body, 7.0);
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
        y += rh + 2.0;
    }
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

/// What the dossier detail shows. An undiscovered hunter (`known` false) is blanked: "???" for
/// the name and for each stat card, and no description at all, so neither the real name, the
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
            title: "??? (LOCKED)".to_string(),
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

/// The Route tab's category lock: built-in members only, so a pack hunter never unlocks a
/// built-in category header.
fn route_category_locked(hs: &[HunterDef], found: &[String], void: bool) -> bool {
    category_locked(
        hs.iter()
            .filter(|h| !h.is_pack() && is_void_hunter(h) == void)
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
            .filter(|&i| !hs[i].is_pack() && is_void_hunter(&hs[i]) == void)
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
    if tab == CUSTOM_TAB {
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
    const ROSTER_W: f32 = 240.0;
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
    let rows = dossier_rows(hs, tab);
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
        ui.dossier_scroll -= mouse_wheel().1.signum() * 26.0;
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
                    let shown = if open {
                        h.name.to_uppercase()
                    } else {
                        LOCKED_PLACEHOLDER.to_string()
                    };
                    let name = clip(a, F::Body, &shown, rr.w - 54.0, 7.0);
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
        label(a, F::Title, &clip(a, F::Title, &text.title, text_w, 16.0), tx, ty, 16.0, p.ink);
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
        for name in ["KYTTU", "kyttu"] {
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
            assert_eq!(t.title, "??? (LOCKED)");
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
        assert_eq!(t.title, "??? (LOCKED)");
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

    fn labels(mode: RoomMode, signed_in: bool) -> Vec<&'static str> {
        lobby_buttons(mode)
            .into_iter()
            .map(|b| lobby_label(b, mode, signed_in))
            .collect()
    }

    #[test]
    fn solo_actions_row_order() {
        assert_eq!(
            labels(RoomMode::Solo, false),
            vec![
                "ENTER ROUTE",
                "CONTINUE RUN",
                "REGISTER / SIGN IN",
                "CHARACTER FILES",
                "BACK TO LAST ROUTE",
            ]
        );
    }

    #[test]
    fn host_actions_row_order() {
        assert_eq!(
            labels(RoomMode::Host, false),
            vec!["ENTER ROUTE", "REGISTER / SIGN IN", "CHARACTER FILES", "LEAVE ROOM"]
        );
    }

    #[test]
    fn guest_actions_row_order() {
        assert_eq!(
            labels(RoomMode::Guest, false),
            vec!["WAITING FOR HOST", "REGISTER / SIGN IN", "CHARACTER FILES", "LEAVE ROOM"]
        );
    }

    #[test]
    fn account_button_reads_account_once_signed_in() {
        for mode in [RoomMode::Solo, RoomMode::Host, RoomMode::Guest] {
            let row = labels(mode, true);
            assert!(row.contains(&"ACCOUNT"), "{mode:?}");
            assert!(!row.contains(&"REGISTER / SIGN IN"), "{mode:?}");
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
        assert_eq!(lobby_focus(RoomMode::Solo, LobbyButton::Account), 2);
        assert_eq!(lobby_focus(RoomMode::Solo, LobbyButton::CharacterFiles), 3);
        assert_eq!(lobby_focus(RoomMode::Host, LobbyButton::Account), 1);
        assert_eq!(lobby_focus(RoomMode::Host, LobbyButton::CharacterFiles), 2);
        assert_eq!(lobby_focus(RoomMode::Guest, LobbyButton::Leave), 3);
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
    fn difficulty_row_is_relaxed_normal_hard() {
        let shown: Vec<&str> = ALL.iter().map(|d| difficulty_label(*d)).collect();
        assert_eq!(shown, vec!["RELAXED", "NORMAL", "HARD"]);
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

        // a guest sees only itself
        let guest = lobby_players(RoomMode::Guest, "ana", &["KYTTU".to_string()]);
        assert_eq!(guest.len(), 1);
        assert!(!guest[0].host && guest[0].you);
    }

    #[test]
    fn an_unnamed_runner_reads_runner() {
        assert_eq!(lobby_players(RoomMode::Solo, "", &[])[0].name, "RUNNER");
    }
}
