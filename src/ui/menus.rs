//! The menus, laid out like the original's screens on its 480x270 design canvas (see style.rs):
//! the main menu (main_menu.tscn / main_menu.gd) with its CREATE ROOM, JOIN ROOM and QUIT
//! dialogs, the account panel (account_panel.gd), the route loading screen (route_loading.gd),
//! room staging (room_lobby.gd) and its hunter dossier.

use super::style::*;
use crate::app::assets::Assets;
use crate::app::checkpoint::Checkpoint;
use crate::config::difficulty::ALL;
use crate::config::Difficulty;
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
            create_offline: true,
            create_note: String::new(),
            acct_manage: false,
            acct_user: String::new(),
            acct_pass: String::new(),
            acct_field: 0,
            acct_status: String::new(),
            confirm_clear: false,
            dossier_sel: 0,
            dossier_scroll: 0.0,
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
        match s {
            Screen::Create => {
                self.create_offline = true;
                self.create_note.clear();
                self.focus = 2;
            }
            Screen::Join => {
                self.join_code.clear();
                self.join_status.clear();
            }
            Screen::Quit => self.focus = 1,
            Screen::Account => {
                self.acct_manage = false;
                self.confirm_clear = false;
                self.acct_status.clear();
                self.acct_pass.clear();
            }
            _ => {}
        }
    }
}

/// The caption colour main_menu.tscn writes for the kicker and brand.
const KICKER: Color = rgba(0.788, 0.663, 0.847, 1.0);

// -- main menu ------------------------------------------------------------------------------

/// The main menu. `active` is false while a dialog sits over it (no focus, no input).
pub fn draw_title(a: &Assets, ui: &mut Ui, username: &str, active: bool) -> Action {
    let p = pal();
    backdrop(1.0);
    let v = view();
    let c = v * 0.5;
    let x0 = c.x - 155.0;
    label(
        a,
        F::Body,
        "SIGNAL ARCHIVE // ROUTE UNSTABLE",
        x0,
        c.y - 76.0,
        5.0,
        KICKER,
    );
    // the brand: a dilated offset shadow, the dark outline, then the purple caps
    let brand = "K Y T T U ' S   L A B";
    let shadow = rgba(0.22, 0.13, 0.27, 0.95);
    label_outline(
        a,
        F::Body,
        brand,
        x0 + 3.0,
        c.y - 62.0 + 3.0,
        10.0,
        shadow,
        shadow,
        1.0,
    );
    label_outline(
        a,
        F::Body,
        brand,
        x0,
        c.y - 62.0,
        10.0,
        rgba(0.784, 0.663, 0.847, 1.0),
        rgba(0.09, 0.045, 0.07, 1.0),
        1.0,
    );
    // THE LAST / ROUTE: Impact 28, line spacing -8, the label grows up from its bottom edge
    let size = 28.0;
    let line_h = F::Title.height() * size;
    let total = 2.0 * line_h - 8.0;
    let top = (c.y + 24.0 - total).min(c.y - 30.0);
    let tshadow = rgba(0.27, 0.17, 0.32, 0.92);
    for (i, line_text) in ["THE LAST", "ROUTE"].iter().enumerate() {
        let y = top + i as f32 * (line_h - 8.0);
        label_outline(
            a,
            F::Title,
            line_text,
            x0 + 4.0,
            y + 4.0,
            size,
            tshadow,
            tshadow,
            1.0,
        );
    }
    for (i, line_text) in ["THE LAST", "ROUTE"].iter().enumerate() {
        let y = top + i as f32 * (line_h - 8.0);
        label_outline(
            a,
            F::Title,
            line_text,
            x0,
            y,
            size,
            p.ink,
            rgba(0.1, 0.05, 0.08, 1.0),
            1.0,
        );
    }
    label(
        a,
        F::Body,
        "Keep moving. Do not look back.",
        x0,
        c.y + 26.0,
        7.0,
        rgba(0.72, 0.68, 0.66, 1.0),
    );
    // the button flow: CREATE ROOM and JOIN ROOM are primary, the rest secondary (all compact)
    let account = if username.is_empty() {
        "REGISTER / SIGN IN".to_string()
    } else {
        username.to_uppercase()
    };
    // ADMIN shows for the admin's account only (GameHub.admin_entry_visible)
    let mut items: Vec<(&str, f32, Kind)> = vec![
        ("CREATE ROOM", 6.0, Kind::Primary),
        ("JOIN ROOM", 6.0, Kind::Primary),
        ("SETTINGS", 6.0, Kind::Secondary),
        (account.as_str(), 5.0, Kind::Secondary),
        ("ADMIN", 5.0, Kind::Secondary),
        ("QUIT", 5.0, Kind::Secondary),
    ];
    if !username.eq_ignore_ascii_case("KYTTU") {
        items.remove(4);
    }
    let row_h = items
        .iter()
        .map(|i| button_min(a, i.0, i.1, true).y)
        .fold(0.0, f32::max);
    let enter = active && nav(&mut ui.focus, items.len(), false);
    let mut out = Action::None;
    let (mut bx, mut by) = (x0, c.y + 50.0);
    for (i, (text, fs, kind)) in items.iter().enumerate() {
        let w = button_min(a, text, *fs, true).x;
        if bx + w > x0 + 310.0 + 0.01 && bx > x0 {
            bx = x0;
            by += row_h + 4.0;
        }
        let r = Rect::new(bx, by, w, row_h);
        let hit = button(a, r, text, *fs, *kind, state(active && ui.focus == i)) && active;
        if hit {
            ui.focus = i;
        }
        if hit || (enter && ui.focus == i) {
            out = match (i, *text) {
                (0, _) => Action::Go(Screen::Create),
                (1, _) => Action::Go(Screen::Join),
                (2, _) => Action::Go(Screen::Settings),
                (3, _) => Action::Go(Screen::Account),
                (_, "ADMIN") => Action::Go(Screen::Admin),
                _ => Action::Go(Screen::Quit),
            };
        }
        bx += w + 5.0;
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

/// The actions row of a menu dialog (right-aligned, separation 5, font 7, full margins).
fn dialog_actions(
    a: &Assets,
    right: f32,
    y: f32,
    items: &[(&str, Kind)],
    focus: usize,
) -> Option<usize> {
    let h = button_min(a, "X", 7.0, false).y;
    let total: f32 = items
        .iter()
        .map(|(t, _)| button_min(a, t, 7.0, false).x)
        .sum::<f32>()
        + 5.0 * (items.len() as f32 - 1.0);
    let mut x = right - total;
    let mut hit = None;
    for (i, (text, kind)) in items.iter().enumerate() {
        let w = button_min(a, text, 7.0, false).x;
        if button(
            a,
            Rect::new(x, y, w, h),
            text,
            7.0,
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
    let act_h = button_min(a, "X", 7.0, false).y;
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
        // no primary server in this build: the original's unreachable-server fallback
        ui.create_offline = true;
        ui.create_note = "SERVER NOT REACHABLE // OFFLINE TURNED ON. PRESS CREATE ROOM TO PLAY SOLO, OR GO ONLINE TO TRY AGAIN.".into();
    }
    Action::None
}

/// JOIN ROOM (main_menu.gd _on_join_pressed): the code field, its hint and status.
pub fn draw_join(a: &Assets, ui: &mut Ui, username: &str) -> Action {
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
    let act_h = button_min(a, "X", 7.0, false).y;
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
    let p = pal();
    draw_title(a, ui, username, false);
    let btn_h = button_min(a, "X", 7.0, false).y;
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
        let w1 = button_min(a, "SIGN IN", 7.0, false).x;
        let w2 = button_min(a, "CREATE ACCOUNT", 7.0, false).x;
        let sign = button(
            a,
            Rect::new(m.inner.x, y, w1, btn_h),
            "SIGN IN",
            7.0,
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
            out = Action::Go(Screen::Title);
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
            out = Action::Go(Screen::Title);
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
            Action::Go(Screen::Title)
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
        out = Action::Go(Screen::Title);
    }
    out
}

// -- route loading (route_loading.gd) -------------------------------------------------------

pub fn draw_loading(a: &Assets, seed_line: &str, detail: &str, progress: f32) {
    let p = pal();
    backdrop(0.82);
    report_frames();
    let panel = centered(340.0, 128.0);
    panel_box(panel, p.panel, hex(0x563041));
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

/// What staging shows about the solo save: "RUN SAVED AT LEVEL +0, 11:26".
pub fn saved_run_status(save: Option<&Checkpoint>) -> String {
    let Some(c) = save else {
        return "NO SAVED RUN".into();
    };
    let place = if c.infinite {
        format!("LEVEL {:+}", c.depth)
    } else {
        format!("DEPTH {:02}", c.depth)
    };
    match crate::app::checkpoint_clock() {
        Some((h, m)) => format!("RUN SAVED AT {place}, {h:02}:{m:02}"),
        None => format!("RUN SAVED AT {place}"),
    }
}

/// The local runner's colour (RemotePlayers.local_colour / facility_map.gd COOP_PLAYER_BLUE).
pub const COOP_BLUE: Color = hex(0x63d9f1);

/// The staging screen: header, room code, route seed, ROUTE SETUP, the player list and the
/// actions row; the panel scrolls when the window is short.
#[allow(clippy::too_many_arguments)]
pub fn draw_lobby(
    a: &Assets,
    ui: &mut Ui,
    diff: &mut Difficulty,
    code: &str,
    seed: u32,
    username: &str,
    save: Option<&Checkpoint>,
    interactive: bool,
) -> Action {
    let p = pal();
    backdrop(0.7);
    let v = view();
    let row = 13.0;
    let l6 = lh(F::Body, 6.0);
    let l7 = lh(F::Body, 7.0);
    // ROUTE SETUP's groups at their minimum widths: DIFFICULTY (the blurb does not wrap), the
    // admin-only NPC TEAMMATES scroller, and SAVED RUN (its CONTINUE RUN button)
    let admin = username.eq_ignore_ascii_case("KYTTU");
    let blurb_w = tw(a, F::Body, diff.blurb(), 6.0);
    let arrow = vec2(26.7, 20.0);
    let npc_w = if admin {
        tw(a, F::Body, "NPC TEAMMATES (ADMIN)", 6.0).max(arrow.x * 2.0 + 22.0 + 6.0)
    } else {
        0.0
    };
    let arch_min = button_min(a, "CONTINUE RUN", 8.0, false).x;
    let cfg_min = blurb_w + 5.0 + if admin { npc_w + 5.0 } else { 0.0 } + arch_min;
    // the panel grows past 380 when the setup row needs it (plus the scroll bar it then shows)
    let pw = 380.0f32.max(cfg_min + 12.0 + 24.0 + 4.5);
    let panel = Rect::new(v.x * 0.5 - pw * 0.5, 10.0, pw, v.y - 20.0);
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
    label(
        a,
        F::Body,
        &track("CO-OP STAGING SIGNAL"),
        x,
        y,
        7.0,
        p.amber,
    );
    label_r(a, F::Body, "1 / 16 SIGNALS", x + w, y, 7.0, p.muted);
    y += l7 + 5.0;
    label(a, F::Title, "ROOM", x, y, 18.0, p.ink);
    label(
        a,
        F::Title,
        code,
        x + tw(a, F::Title, "ROOM", 18.0) + 6.0,
        y,
        18.0,
        p.accent,
    );
    y += lh(F::Title, 18.0) + 5.0;
    // the route seed block
    let sh = 12.0 + l6 + 1.0 + lh(F::Body, 9.0) + 1.0 + l6;
    panel_box(Rect::new(x, y, w, sh), p.inset, p.accent_deep);
    label(
        a,
        F::Body,
        &track("ROUTE SEED"),
        x + 6.0,
        y + 6.0,
        6.0,
        p.amber,
    );
    label(
        a,
        F::Body,
        &format!("ROOM-{code}-{seed:08X}"),
        x + 6.0,
        y + 7.0 + l6,
        9.0,
        p.ink,
    );
    label(
        a,
        F::Body,
        "Players enter from separate nearby access points. The route is shared.",
        x + 6.0,
        y + 8.0 + l6 + lh(F::Body, 9.0),
        6.0,
        p.muted,
    );
    y += sh + 5.0;
    let btn_h = button_min(a, "X", 7.0, false).y.max(row);
    let cx = x + 6.0;
    let cw = w - 12.0;
    // the two expanding groups share what the scroller leaves; a group never drops under its minimum
    let avail = cw - 5.0 - if admin { npc_w + 5.0 } else { 0.0 };
    let mut gw = avail * 0.5;
    let mut aw = avail * 0.5;
    if gw < blurb_w {
        gw = blurb_w;
        aw = avail - gw;
    }
    let group_h = l6 + 2.0 + btn_h * 2.0 + 4.0 + 2.0 + l6;
    let ch = 12.0 + l7 + 3.0 + group_h + 3.0 + l6;
    panel_box(Rect::new(x, y, w, ch), p.inset, p.panel_edge);
    let mut cy = y + 6.0;
    label(a, F::Body, &track("ROUTE SETUP"), cx, cy, 7.0, p.amber);
    label_r(
        a,
        F::Body,
        "HOST CONTROLS",
        cx + cw,
        cy + (l7 - l6) * 0.5,
        6.0,
        p.muted,
    );
    cy += l7 + 3.0;
    label(a, F::Body, "DIFFICULTY", cx, cy, 6.0, p.muted);
    let mut bx = cx;
    let mut by = cy + l6 + 2.0;
    for (i, n) in ["RELAXED", "STANDARD", "HARD", "NIGHTMARE"]
        .iter()
        .enumerate()
    {
        let bw = button_min(a, n, 7.0, false).x;
        if bx + bw > cx + gw && bx > cx {
            bx = cx;
            by += btn_h + 4.0;
        }
        let r = Rect::new(bx, by, bw, btn_h);
        if i < 3 {
            let d = ALL[i];
            let kind = if d == *diff {
                Kind::Selected
            } else {
                Kind::Secondary
            };
            if button(a, r, n, 7.0, kind, plain()) && interactive {
                *diff = d;
            }
        } else {
            button(a, r, n, 7.0, Kind::Secondary, disabled());
        }
        bx += bw + 4.0;
    }
    if interactive
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
    let mut sx = cx + gw + 5.0;
    if admin {
        // NPC TEAMMATES (ADMIN) [<] 0 [>]: co-op only, so disabled in this solo room
        label(a, F::Body, "NPC TEAMMATES (ADMIN)", sx, cy, 6.0, p.muted);
        let ry = cy + l6 + 2.0;
        for (k, right) in [(0.0f32, false), (1.0, true)] {
            let r = Rect::new(sx + k * (arrow.x + 3.0 + 22.0 + 3.0), ry, arrow.x, arrow.y);
            button(a, r, "", 8.0, Kind::Secondary, disabled());
            let c = vec2(r.x + r.w * 0.5, r.y + r.h * 0.5 + 0.5);
            let d = if right { 1.0 } else { -1.0 };
            tri(
                c + vec2(3.0 * d, 0.0),
                c + vec2(-3.0 * d, -3.2),
                c + vec2(-3.0 * d, 3.2),
                p.disabled,
            );
        }
        label_vc(
            a,
            F::Body,
            "0",
            sx + arrow.x + 3.0 + (22.0 - tw(a, F::Body, "0", 8.0)) * 0.5,
            ry,
            arrow.y,
            8.0,
            p.ink,
        );
        label(
            a,
            F::Body,
            "ONLINE ROOMS ONLY",
            sx,
            ry + arrow.y + 2.0,
            6.0,
            p.muted,
        );
        sx += npc_w + 5.0;
    }
    label(a, F::Body, "SAVED RUN", sx, cy, 6.0, p.muted);
    let mut sy = cy + l6 + 2.0;
    for l in wrap(a, F::Body, &saved_run_status(save), aw, 7.0) {
        label(a, F::Body, &l, sx, sy, 7.0, p.body);
        sy += l7 + 3.0;
    }
    let cont = Rect::new(sx, sy, aw, button_min(a, "X", 8.0, false).y.max(row));
    let cont_st = if save.is_some() { plain() } else { disabled() };
    if button(a, cont, "CONTINUE RUN", 8.0, Kind::Secondary, cont_st) && interactive {
        out = Action::ContinueRun;
    }
    label(
        a,
        F::Body,
        "THE RUN SAVES ITSELF // DEATH ENDS IT",
        cx,
        y + ch - 6.0 - l6,
        6.0,
        p.muted,
    );
    y += ch + 5.0;
    label(a, F::Body, &track("CONNECTED PLAYERS"), x, y, 7.0, p.amber);
    y += l7 + 5.0;
    let ph = 12.0 + l7 + 3.0 + l6 + 3.0 + 40.0;
    let me = PlayerRow {
        name: username_or(username),
        host: true,
        you: true,
        ping: "0 MS".into(),
        status: "IN LOBBY".into(),
        floor: "+0".into(),
        colour: COOP_BLUE,
    };
    player_list(a, Rect::new(x, y, w, ph), &[me]);
    y += ph + 5.0;
    label(a, F::Body, "OFFLINE SOLO ROOM", x, y, 7.0, p.alert);
    y += l7 + 5.0;
    label(
        a,
        F::Body,
        &format!("SOLO ROOM {code} // PRIMARY SERVER UNAVAILABLE"),
        x,
        y,
        7.0,
        p.muted,
    );
    y += l7 + 5.0;
    // ENTER ROUTE / REROLL ROUTE / HUNTER DOSSIER / BACK TO LAST ROUTE
    let ah = button_min(a, "X", 10.0, false).y.max(18.0);
    let items = [
        ("ENTER ROUTE", 10.0, Kind::Primary),
        ("REROLL ROUTE", 7.0, Kind::Secondary),
        ("HUNTER DOSSIER", 7.0, Kind::Secondary),
        ("BACK TO LAST ROUTE", 7.0, Kind::Secondary),
    ];
    let enter = interactive && nav(&mut ui.focus, items.len(), false);
    let mut ax = x;
    for (i, (t, fs, k)) in items.iter().enumerate() {
        let bw = button_min(a, t, *fs, false).x;
        let hit = button(
            a,
            Rect::new(ax, y, bw, ah),
            t,
            *fs,
            *k,
            state(interactive && ui.focus == i),
        ) && interactive;
        if hit || (enter && ui.focus == i) {
            out = match i {
                0 => Action::StartRun,
                1 => Action::Reroll,
                2 => Action::Go(Screen::Dossier),
                _ => Action::Go(Screen::Title),
            };
        }
        ax += bw + 5.0;
    }
    y += ah;
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

/// The dossier modal over whatever screen is below. Returns true when it closes.
pub fn draw_dossier(a: &Assets, ui: &mut Ui, content: &Content, username: &str) -> bool {
    let p = pal();
    // entries stay locked until the account has met the hunter (backend_client.gd hunter_discovered)
    let found = crate::app::profile::discovered(username);
    let known = |id: &str| found.iter().any(|d| d == id);
    let signed_in = crate::app::profile::signed_in(username);
    let min = vec2(420.0, 232.0);
    let m = modal(
        a,
        "ARCHIVED HUNTER SIGNALS",
        "HUNTER DOSSIER",
        15.0,
        min,
        396.0,
        40.0,
    );
    let split_y = m.y;
    let split_h = m.inner.y + m.inner.h - split_y;
    // the roster: an ItemList with category headers and 48x48 portraits
    let list = Rect::new(m.inner.x, split_y, 108.0, split_h);
    panel_box(list, p.inset, p.panel_edge);
    let hs = &content.hunters;
    let mut rows: Vec<(Option<&str>, Option<usize>)> = vec![(Some("-- A-Z HUNTERS --"), None)];
    rows.extend(
        (0..hs.len())
            .filter(|&i| hs[i].category != "void")
            .map(|i| (None, Some(i))),
    );
    rows.push((Some("-- VOID HUNTERS --"), None));
    rows.extend(
        (0..hs.len())
            .filter(|&i| hs[i].category == "void")
            .map(|i| (None, Some(i))),
    );
    let ids: Vec<usize> = rows.iter().filter_map(|r| r.1).collect();
    let moved = is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::Up);
    if !ids.is_empty() {
        ui.dossier_sel = ui.dossier_sel.min(ids.len() - 1);
        if is_key_pressed(KeyCode::Down) {
            ui.dossier_sel = (ui.dossier_sel + 1).min(ids.len() - 1);
        }
        if is_key_pressed(KeyCode::Up) {
            ui.dossier_sel = ui.dossier_sel.saturating_sub(1);
        }
    }
    let head_h = 13.0;
    let item_h = 53.0;
    let total: f32 = rows
        .iter()
        .map(|r| if r.0.is_some() { head_h } else { item_h })
        .sum();
    let view_h = list.h - 6.0;
    if hovered(list) {
        ui.dossier_scroll -= mouse_wheel().1.signum() * 26.0;
    }
    if moved {
        let (mut acc, mut k, mut sel_top) = (0.0, 0, 0.0);
        for r in &rows {
            if r.1.is_some() {
                if k == ui.dossier_sel {
                    sel_top = acc;
                }
                k += 1;
            }
            acc += if r.0.is_some() { head_h } else { item_h };
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
    let mut y = list.y + 3.0 - ui.dossier_scroll;
    let mut k = 0;
    for (i, r) in rows.iter().enumerate() {
        if i > 0 {
            line(
                vec2(list.x + 3.0, y),
                vec2(list.x + list.w - 3.0, y),
                0.35,
                alpha(p.body, 0.25),
            );
        }
        if let Some(h) = r.0 {
            label_vc(
                a,
                F::Body,
                h,
                list.x + 5.0,
                y,
                head_h,
                7.0,
                alpha(p.body, 0.5),
            );
            y += head_h;
            continue;
        }
        let h = &hs[r.1.unwrap()];
        let rr = Rect::new(list.x + 3.0, y, list.w - 6.0, item_h);
        if k == ui.dossier_sel {
            sbox(rr, p.amber_fill, p.amber, 1.0, p.corner);
        }
        if hovered(rr) && hovered(list) && is_mouse_button_pressed(MouseButton::Left) {
            ui.dossier_sel = k;
        }
        let open = known(&h.id);
        if let Some(t) = a.portraits.get(&h.id) {
            let s = unit();
            // HunterDossier.SILHOUETTE for a locked entry
            let tint = if open {
                WHITE
            } else {
                Color::new(0.02, 0.015, 0.03, 1.0)
            };
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
            "???".to_string()
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
        y += item_h;
        k += 1;
    }
    clip_end();
    // the details column
    let dx = list.x + list.w + 8.0;
    let dw = m.inner.x + m.inner.w - dx;
    if let Some(&hi) = ids.get(ui.dossier_sel) {
        let h = &hs[hi];
        let tier = ["EASY", "INTERMEDIATE", "DIFFICULT"][h.tier.min(2)];
        let open = known(&h.id);
        // HunterDossier.redacted: her own name blanked out
        let redact = |t: &str| -> String {
            if open {
                return t.to_string();
            }
            let bar = "\u{2588}".repeat(h.name.chars().count());
            t.replace(&h.name, &bar)
                .replace(&h.name.to_uppercase(), &bar)
                .replace(&h.name.to_lowercase(), &bar)
        };
        let status = if !open {
            if signed_in {
                "ACCOUNT DISCOVERY REQUIRED".to_string()
            } else {
                "SIGN IN TO RETAIN DISCOVERIES".to_string()
            }
        } else if h.category == "void" {
            "VOID INCURSION // EXTRA HUNTER".to_string()
        } else if crate::content::hunters::SPECIAL_IDS.contains(&h.id.as_str()) {
            "RULE-SUMMONED // A-Z SPECIAL".to_string()
        } else {
            format!("A-Z ROUTE DRAW // {tier}")
        };
        let name_h = lh(F::Title, 16.0);
        label(
            a,
            F::Title,
            &h.name.to_uppercase(),
            dx,
            split_y,
            16.0,
            p.ink,
        );
        label_r(
            a,
            F::Body,
            &status,
            dx + dw,
            split_y + (name_h - lh(F::Body, 6.0)) * 0.5,
            6.0,
            p.amber,
        );
        let mut y = split_y + name_h + 4.0;
        for l in wrap(
            a,
            F::Body,
            crate::hunters::gimmicks::describe(&h.id),
            dw,
            7.0,
        ) {
            label(a, F::Body, &l, dx, y, 7.0, p.body);
            y += lh(F::Body, 7.0);
        }
        let stats = format!(
            "PATROL {:.2}  //  CHASE {:.2}  //  SIGHT {:.1}  //  HEARING {:.2}",
            h.speed, h.chase, h.sight, h.hearing
        );
        let lines = wrap(a, F::Body, &stats, dw, 6.0);
        let mut sy = split_y + split_h - lines.len() as f32 * lh(F::Body, 6.0);
        for l in lines {
            label(a, F::Body, &l, dx, sy, 6.0, p.muted);
            sy += lh(F::Body, 6.0);
        }
    }
    m.close || is_key_pressed(KeyCode::Escape)
}
