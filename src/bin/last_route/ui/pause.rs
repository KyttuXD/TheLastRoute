//! The pause overlay (pause_menu.gd): the scrim, a 380x236 panel with SIGNAL SUSPENDED, PAUSED
//! and the PAUSE / STATISTICS / PLAYERS tabs; RETURN, HUNTER DOSSIER, SKIN, SETTINGS and ABANDON
//! ROUTE (a two-step question), the run statistics with the route-signal block, and the player
//! list. SETTINGS opens the shared settings overlay; SKIN opens the same overlay on its SKIN tab
//! (ui/skin_tab.rs), so a skin can be changed in a run: the game loop wears the applied skin and
//! sends it to the room.

use super::menus::{self, PlayerRow, Ui};
use super::settings_panel;
use super::style::*;
use crate::app::assets::Assets;
use crate::app::settings::Settings;
use crate::content::Content;
use crate::game::Game;
use crate::net::hosted::{self, OwnerAction};
use crate::net::world_sync::{skin_colour, RosterEntry, RunnerState, QUIET_DISCONNECT};
use macroquad::prelude::*;
use std::time::Duration;

/// PLAYERS statuses (Godot `room_player_list.gd` `STATUS_*`).
pub const STATUS_ALIVE: &str = "ALIVE";
pub const STATUS_CAUGHT: &str = "CAUGHT";
pub const STATUS_SPECTATING: &str = "SPECTATING";
pub const STATUS_DISCONNECTED: &str = "DISCONNECTED";
pub const STATUS_LOBBY: &str = "IN LOBBY";

/// The pause panel (pause_menu.gd): 380x236 with a 16 inset on every side.
pub const PANEL_W: f32 = 380.0;
pub const PANEL_H: f32 = 236.0;
pub const PANEL_INSET: f32 = 16.0;
/// The tabs' height (80 wide, 18 tall).
const TAB_H: f32 = 18.0;
/// The ABANDON ROUTE? panel (pause_menu.gd _build_confirm): 236 wide, a 14 inset on every side.
pub const CONFIRM_W: f32 = 236.0;
pub const CONFIRM_INSET: f32 = 14.0;

/// Offset from the inner top of the pause panel to the page area, below SIGNAL SUSPENDED,
/// PAUSED and the tabs.
pub fn page_offset() -> f32 {
    lh(F::Body, 7.0) + 8.0 + lh(F::Title, 22.0) + 8.0 + TAB_H + 8.0
}

/// The page area under the tabs, inside the panel's inner rect.
pub fn page_area(inner: Rect) -> Rect {
    let top = inner.y + page_offset();
    Rect::new(inner.x, top, inner.w, inner.y + inner.h - top)
}

/// Top edges of a column of `heights` with `gap` between items, centred in the band
/// `area_top..area_top + area_h`.
pub fn centred_column(area_top: f32, area_h: f32, heights: &[f32], gap: f32) -> Vec<f32> {
    let total: f32 = heights.iter().sum::<f32>() + gap * heights.len().saturating_sub(1) as f32;
    let mut y = area_top + (area_h - total) * 0.5;
    let mut tops = Vec::with_capacity(heights.len());
    for h in heights {
        tops.push(y);
        y += h + gap;
    }
    tops
}

/// Vertical layout of the ABANDON ROUTE? panel, relative to its top edge: returns the top of the
/// button row and the panel height. The panel is exactly the content plus the bottom inset, so
/// the buttons always sit inside it.
pub fn confirm_layout(body_lines: usize, button_h: f32) -> (f32, f32) {
    let buttons_top = CONFIRM_INSET
        + lh(F::Body, 7.0)
        + 5.0
        + lh(F::Title, 16.0)
        + 5.0
        + body_lines as f32 * lh(F::Body, 8.0)
        + 5.0;
    (buttons_top, buttons_top + button_h + CONFIRM_INSET)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sub {
    None,
    Settings,
    /// The settings overlay opened on its SKIN tab (the pause menu's SKIN entry).
    Skin,
    Dossier,
    Confirm,
}

/// The settings overlay's SKIN tab (settings_panel TABS).
const SKIN_TAB: usize = 4;

/// The PAUSE page's buttons, top to bottom: the label, the least width and height, and the kind.
pub const MAIN_ITEMS: [(&str, f32, f32, Kind); 5] = [
    ("RETURN", 74.0, 18.0, Kind::Primary),
    ("HUNTER DOSSIER", 112.0, 18.0, Kind::Secondary),
    ("SKIN", 112.0, 18.0, Kind::Secondary),
    ("SETTINGS", 112.0, 18.0, Kind::Secondary),
    ("ABANDON ROUTE", 88.0, 14.0, Kind::Secondary),
];
/// The index of each button in [`MAIN_ITEMS`].
const ITEM_DOSSIER: usize = 1;
const ITEM_SKIN: usize = 2;
const ITEM_SETTINGS: usize = 3;
const ITEM_ABANDON: usize = 4;

pub struct Pause {
    pub open: bool,
    pub tab: usize,
    pub focus: usize,
    pub sub: Sub,
    pub copied: String,
}

impl Default for Pause {
    fn default() -> Self {
        Pause {
            open: false,
            tab: 0,
            focus: 0,
            sub: Sub::None,
            copied: String::new(),
        }
    }
}

pub enum Out {
    None,
    Resume,
    Abandon,
    /// The room owner's command from the OWNER tab or a KICK in the PLAYERS tab.
    Owner(OwnerAction),
}

impl Pause {
    pub fn show(&mut self) {
        *self = Pause {
            open: true,
            ..Default::default()
        };
    }
}

/// The pause menu's tabs: the three everybody has, and the room owner's OWNER tab after them.
pub fn tab_names(owner: bool) -> Vec<String> {
    let mut v: Vec<String> = ["PAUSE", "STATISTICS", "PLAYERS"].iter().map(|s| s.to_string()).collect();
    if owner {
        v.push(hosted::rules().strings.tab_owner.clone());
    }
    v
}

/// The OWNER tab's index, when there is one.
pub const OWNER_TAB: usize = 3;

/// ESC handling: backs out of the settings, dossier or question first, then closes the menu.
pub fn escape(pz: &mut Pause) -> Out {
    match pz.sub {
        Sub::None => {
            pz.open = false;
            Out::Resume
        }
        _ => {
            pz.sub = Sub::None;
            Out::None
        }
    }
}

/// PLAYERS status (Godot `room_player_list.gd` `_status`). Outside a run every row is IN LOBBY. With no state yet the
/// runner just joined, so ALIVE. Another runner quiet for longer than [`QUIET_DISCONNECT`] is DISCONNECTED; then
/// SPECTATING; then CAUGHT once the route has ended or the escape struggle is on.
pub fn status_of(state: Option<&RunnerState>, you: bool, in_run: bool) -> &'static str {
    if !in_run {
        return STATUS_LOBBY;
    }
    let Some(state) = state else {
        return STATUS_ALIVE;
    };
    if !you && state.quiet > QUIET_DISCONNECT {
        return STATUS_DISCONNECTED;
    }
    if state.spectating {
        return STATUS_SPECTATING;
    }
    if state.game_over || state.escape_active {
        return STATUS_CAUGHT;
    }
    STATUS_ALIVE
}

/// The floor column (Godot `room_player_list.gd` `floor_text`): the row's level, `-depth`, with its sign; `-` with no state.
pub fn floor_text(level: Option<i32>) -> String {
    level.map_or("-".into(), |l| format!("{l:+}"))
}

/// The ping column (Godot `ping_text`): `N MS`, or `-` until a round trip is measured.
pub fn ping_text(ping: Option<u32>) -> String {
    ping.map_or("-".into(), |ms| format!("{ms} MS"))
}

/// Godot `room_player_list.gd` `build` order: the host first, then by name.
pub fn order(rows: &mut [PlayerRow]) {
    rows.sort_by(|a, b| b.host.cmp(&a.host).then_with(|| a.name.cmp(&b.name)));
}

/// The PLAYERS rows: this runner and the others in the room (Godot `RoomPlayerList.build`), in [`order`]. The local row
/// reads this runner's own run state; a remote one reads its last sample's state.
pub fn player_rows(g: &Game, username: &str, local_host: bool, roster: &[RosterEntry], kickable: bool) -> Vec<PlayerRow> {
    let local = RunnerState {
        depth: g.depth,
        spectating: false,
        game_over: g.report.is_some(),
        escape_active: g.escape.is_some(),
        quiet: Duration::ZERO,
    };
    let me = PlayerRow {
        name: if username.is_empty() {
            "RUNNER".into()
        } else {
            username.to_uppercase()
        },
        host: local_host,
        you: true,
        ping: ping_text(Some(0)),
        status: status_of(Some(&local), true, true).into(),
        floor: floor_text(Some(-local.depth)),
        colour: menus::coop_blue(),
        kick: None,
    };
    let mut rows = vec![me];
    rows.extend(roster.iter().map(|e| PlayerRow {
        name: if e.name.is_empty() { "RUNNER".into() } else { e.name.clone() },
        host: e.host,
        you: false,
        ping: ping_text(e.ping),
        status: status_of(e.state.as_ref(), false, true).into(),
        floor: floor_text(e.state.map(|s| -s.depth)),
        colour: skin_colour(&e.skin, e.peer),
        // the owner can remove any guest, never the host
        kick: kick_target(kickable, e.host, e.peer),
    }));
    order(&mut rows);
    rows
}

/// The session id KICK names for a row: only the room's owner has one, and only for a guest (never the host, peer 1).
pub fn kick_target(owner: bool, host: bool, peer: u32) -> Option<u32> {
    (owner && !host && peer > 1).then_some(peer)
}

/// Hold-O overlay (Godot `player_list`): the PLAYERS panel without pause chrome.
pub fn draw_hold_player_list(a: &Assets, g: &Game, username: &str, roster: &[RosterEntry], local_host: bool) {
    let rows = player_rows(g, username, local_host, roster, false);
    let sw = screen_width();
    let sh = screen_height();
    let r = Rect::new(
        (sw - PANEL_W) * 0.5,
        (sh - PANEL_H) * 0.5,
        PANEL_W,
        PANEL_H,
    );
    menus::player_list(a, r, &rows, false);
}
#[allow(clippy::too_many_arguments)]
pub fn draw_pause(
    a: &Assets,
    pz: &mut Pause,
    ui: &mut Ui,
    st: &mut Settings,
    content: &Content,
    g: &Game,
    username: &str,
    roster: &[RosterEntry],
    local_host: bool,
) -> Out {
    let p = pal();
    let v = view();
    fill(Rect::new(0.0, 0.0, v.x, v.y), p.scrim);
    match pz.sub {
        Sub::Settings | Sub::Skin => {
            if settings_panel::draw_settings(a, ui, st, content, true) {
                pz.focus = if pz.sub == Sub::Skin { ITEM_SKIN } else { ITEM_SETTINGS };
                pz.sub = Sub::None;
            }
            return Out::None;
        }
        Sub::Dossier => {
            if menus::draw_dossier(a, ui, content, username) {
                pz.sub = Sub::None;
            }
            return Out::None;
        }
        Sub::Confirm => return draw_confirm(a, pz),
        Sub::None => {}
    }
    let panel = centered(PANEL_W, PANEL_H);
    panel_box(panel, p.panel, p.panel_edge);
    let inner = Rect::new(
        panel.x + PANEL_INSET,
        panel.y + PANEL_INSET,
        panel.w - PANEL_INSET * 2.0,
        panel.h - PANEL_INSET * 2.0,
    );
    let cx = inner.x + inner.w * 0.5;
    let mut y = inner.y;
    label_c(a, F::Body, &track("SIGNAL SUSPENDED"), cx, y, 7.0, p.amber);
    y += lh(F::Body, 7.0) + 8.0;
    label_c(a, F::Title, "PAUSED", cx, y, 22.0, p.ink);
    y += lh(F::Title, 22.0) + 8.0;
    // tabs: 80x18 each, separation 4, the open page's tab is the primary slab
    let tab_h = TAB_H;
    // the room owner has a fourth tab
    let owner = ui.owner.owner;
    let names = tab_names(owner);
    let tabs = names.len();
    let tx = cx - (80.0 * tabs as f32 + 4.0 * (tabs as f32 - 1.0)) * 0.5;
    if pz.tab >= tabs {
        pz.tab = 0;
    }
    for (i, t) in names.iter().enumerate() {
        let kind = if i == pz.tab {
            Kind::Primary
        } else {
            Kind::Secondary
        };
        if button(
            a,
            Rect::new(tx + i as f32 * 84.0, y, 80.0, tab_h),
            t,
            8.0,
            kind,
            plain(),
        ) {
            pz.tab = i;
            pz.focus = 0;
        }
    }
    if is_key_pressed(KeyCode::Tab) {
        pz.tab = (pz.tab + 1) % tabs;
        pz.focus = 0;
    }
    let view_r = page_area(inner);
    let mut out = Out::None;
    match pz.tab {
        0 => {
            // RETURN (74x18 primary), HUNTER DOSSIER, SKIN and SETTINGS (112x18), ABANDON ROUTE (88x14)
            let bh8 = button_min(a, "X", 8.0, false).y.max(18.0);
            let items = MAIN_ITEMS;
            let sizes: Vec<Vec2> = items
                .iter()
                .map(|(t, w, h, _)| {
                    let m = button_min(a, t, 8.0, false);
                    vec2(m.x.max(*w), m.y.max(*h).min(bh8))
                })
                .collect();
            let heights: Vec<f32> = sizes.iter().map(|s| s.y).collect();
            let tops = centred_column(view_r.y, view_r.h, &heights, 8.0);
            let enter = nav(&mut pz.focus, MAIN_ITEMS.len(), true);
            for (i, (t, _, _, k)) in items.iter().enumerate() {
                let r = Rect::new(cx - sizes[i].x * 0.5, tops[i], sizes[i].x, sizes[i].y);
                let hit = button(a, r, t, 8.0, *k, state(pz.focus == i));
                if hit {
                    pz.focus = i;
                }
                if hit || (enter && pz.focus == i) {
                    match i {
                        0 => {
                            pz.open = false;
                            out = Out::Resume;
                        }
                        ITEM_DOSSIER => {
                            pz.sub = Sub::Dossier;
                            // every opening starts on the Route Dossier tab, at its first hunter
                            ui.dossier_tab = 0;
                            ui.dossier_sel = 0;
                            ui.dossier_scroll = 0.0;
                        }
                        ITEM_SKIN => {
                            pz.sub = Sub::Skin;
                            ui.settings_tab = SKIN_TAB;
                            ui.scroll = 0.0;
                            // a text box or file chooser left from an earlier visit is dropped
                            super::skin_tab::blur();
                        }
                        ITEM_SETTINGS => {
                            pz.sub = Sub::Settings;
                            ui.settings_tab = 0;
                            ui.scroll = 0.0;
                        }
                        _ => {
                            pz.sub = Sub::Confirm;
                            pz.focus = 1;
                        }
                    }
                }
            }
        }
        1 => draw_statistics(a, pz, view_r, g),
        OWNER_TAB if owner => {
            if let Some(action) = draw_owner_tab(a, ui, view_r, g) {
                out = Out::Owner(action);
            }
        }
        _ => {
            let rows = player_rows(g, username, local_host, roster, owner);
            if let Some(peer) = menus::player_list(a, view_r, &rows, true) {
                out = Out::Owner(OwnerAction::Kick(peer));
            }
        }
    }
    out
}

/// The OWNER tab: END ROUTE, RESTART ROUTE and the room's difficulty. The server carries them out; the choice shown is the
/// room's own difficulty, which changes when the server says so.
fn draw_owner_tab(a: &Assets, ui: &mut Ui, r: Rect, g: &Game) -> Option<OwnerAction> {
    let p = pal();
    let s = &hosted::rules().strings;
    let mut out = None;
    let x = r.x;
    let mut y = r.y;
    label_c(a, F::Body, &track(&s.owner_heading), x + r.w * 0.5, y, 8.0, p.amber);
    y += lh(F::Body, 8.0) + 8.0;
    let bh = button_min(a, "X", 8.0, false).y.max(18.0);
    let joined = ui.owner.joined;
    let st = if joined { plain() } else { disabled() };
    let end_w = button_min(a, &s.label_end, 8.0, false).x.max(88.0);
    let restart_w = button_min(a, &s.label_restart, 8.0, false).x.max(88.0);
    let row_w = end_w + 6.0 + restart_w;
    let bx = x + (r.w - row_w) * 0.5;
    if button(a, Rect::new(bx, y, restart_w, bh), &s.label_restart, 8.0, Kind::Primary, st) && joined {
        out = Some(OwnerAction::Restart);
    }
    if button(a, Rect::new(bx + restart_w + 6.0, y, end_w, bh), &s.label_end, 8.0, Kind::Secondary, st) && joined {
        out = Some(OwnerAction::End);
    }
    y += bh + 10.0;
    label(a, F::Body, &menus::lobby_difficulty().title, x, y, 7.0, p.muted);
    y += lh(F::Body, 7.0) + 3.0;
    let mut dx = x;
    for d in menus::lobby_row() {
        let w = button_min(a, menus::difficulty_label(d), 7.0, false).x;
        let kind = if d == g.difficulty { Kind::Selected } else { Kind::Secondary };
        if button(a, Rect::new(dx, y, w, bh), menus::difficulty_label(d), 7.0, kind, st) && joined && d != g.difficulty {
            out = Some(OwnerAction::Difficulty(d));
        }
        dx += w + 4.0;
    }
    y += bh + 6.0;
    for line in wrap(a, F::Body, &s.owner_pause_note, r.w, 6.0) {
        label(a, F::Body, &line, x, y, 6.0, p.muted);
        y += lh(F::Body, 6.0);
    }
    if !ui.owner.status.is_empty() {
        y += 4.0;
        label(a, F::Body, &ui.owner.status, x, y, 7.0, p.amber);
    }
    out
}

fn draw_statistics(a: &Assets, pz: &mut Pause, r: Rect, g: &Game) {
    let p = pal();
    // the run's stored room seed text and its room code
    let route_code = crate::world::route::room_text_code(&g.room_text);
    let x = r.x;
    let w = r.w - 9.0;
    let mut y = r.y;
    // the page scrolls in the original (a slim scrollbar on the right)
    fill(Rect::new(r.x + r.w - 6.0, r.y, 6.0, r.h), p.inset);
    border(Rect::new(r.x + r.w - 6.0, r.y, 6.0, r.h), 1.0, p.panel_edge);
    fill(
        Rect::new(r.x + r.w - 5.0, r.y + 1.0, 4.0, r.h * 0.55),
        p.accent,
    );
    label_c(
        a,
        F::Body,
        &track("CURRENT RUN"),
        x + w * 0.5,
        y,
        8.0,
        p.amber,
    );
    y += lh(F::Body, 8.0) + 6.0;
    let seed_text = g.room_text.clone();
    let bh = button_min(a, "X", 7.0, false).y.max(16.0);
    let sh = 12.0 + lh(F::Body, 6.0) + 2.0 + lh(F::Body, 9.0) + 2.0 + lh(F::Body, 8.0) + 2.0 + bh;
    panel_box(Rect::new(x, y, w, sh), p.inset, p.accent_deep);
    let mut sy = y + 6.0;
    label(
        a,
        F::Body,
        &track("ROUTE SIGNAL"),
        x + 6.0,
        sy,
        6.0,
        p.amber,
    );
    sy += lh(F::Body, 6.0) + 2.0;
    label(
        a,
        F::Body,
        &format!("ROOM {route_code}"),
        x + 6.0,
        sy,
        9.0,
        p.accent,
    );
    sy += lh(F::Body, 9.0) + 2.0;
    label(a, F::Body, &seed_text, x + 6.0, sy, 8.0, p.ink);
    sy += lh(F::Body, 8.0) + 2.0;
    let cw = button_min(a, "COPY ROUTE", 7.0, false).x.max(88.0);
    if button(
        a,
        Rect::new(x + 6.0, sy, cw, bh),
        "COPY ROUTE",
        7.0,
        Kind::Primary,
        plain(),
    ) {
        let text = format!("ROOM {route_code} // SEED {seed_text}");
        let _ = &text;
        pz.copied = "COPIED TO CLIPBOARD".into();
        macroquad::miniquad::window::clipboard_set(&text);
    }
    label_vc(
        a,
        F::Body,
        &pz.copied,
        x + 6.0 + cw + 6.0,
        sy,
        bh,
        7.0,
        p.stable,
    );
    y += sh + 6.0;
    let t = g.time as i32;
    let rows = [
        ("DEPTH", format!("{:02}", g.depth)),
        (
            "FLOOR RELAYS",
            format!("{} / {}", g.relays_collected, g.fac.relays.len()),
        ),
        ("SCORE", format!("{}", g.score)),
        ("COINS", format!("{}", g.coins)),
        (
            "TIME ON ROUTE",
            format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60),
        ),
        ("DIFFICULTY", g.difficulty.name().to_string()),
    ];
    for (k, val) in rows {
        if y + lh(F::Body, 8.0) > r.y + r.h {
            break;
        }
        label(a, F::Body, k, x, y, 8.0, p.muted);
        label_r(a, F::Body, &val, x + w, y, 8.0, p.ink);
        y += lh(F::Body, 8.0) + 5.0;
    }
}

/// ABANDON ROUTE? (the confirm panel, 236x88): the warning and ABANDON ROUTE | CANCEL.
fn draw_confirm(a: &Assets, pz: &mut Pause) -> Out {
    let p = pal();
    let body = wrap(
        a,
        F::Body,
        "This ends the run and returns to the main menu.",
        208.0,
        8.0,
    );
    let bh = button_min(a, "X", 8.0, false).y.max(14.0);
    // the panel is sized to its content, so the button row always lands inside it
    let (buttons_y, height) = confirm_layout(body.len(), bh);
    let panel = centered(CONFIRM_W, height);
    panel_box(panel, p.panel, p.accent_deep);
    let x = panel.x + CONFIRM_INSET;
    let mut y = panel.y + CONFIRM_INSET;
    label(a, F::Body, &track("SIGNAL SUSPENDED"), x, y, 7.0, p.amber);
    y += lh(F::Body, 7.0) + 5.0;
    label(a, F::Title, "ABANDON ROUTE?", x, y, 16.0, p.ink);
    y += lh(F::Title, 16.0) + 5.0;
    for l in &body {
        label(a, F::Body, l, x, y, 8.0, p.body);
        y += lh(F::Body, 8.0);
    }
    let y = panel.y + buttons_y;
    let w1 = button_min(a, "ABANDON ROUTE", 8.0, false).x;
    let w2 = button_min(a, "CANCEL", 8.0, false).x;
    let enter = nav(&mut pz.focus, 2, false);
    let r1 = Rect::new(x, y, w1, bh);
    let yes = button(
        a,
        r1,
        "ABANDON ROUTE",
        8.0,
        Kind::Secondary,
        state(pz.focus == 0),
    );
    if pz.focus != 0 && !hovered(r1) {
        border(r1, 1.0, p.accent_deep);
    }
    let no = button(
        a,
        Rect::new(x + w1 + 6.0, y, w2, bh),
        "CANCEL",
        8.0,
        Kind::Secondary,
        state(pz.focus == 1),
    );
    if yes || (enter && pz.focus == 0) {
        pz.open = false;
        pz.sub = Sub::None;
        return Out::Abandon;
    }
    if no || (enter && pz.focus == 1) {
        pz.sub = Sub::None;
        pz.focus = ITEM_ABANDON;
    }
    Out::None
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 0.01;
    // Rust's measured button heights at size 8: the label line times 8 plus the 4 + 4 margins
    const BUTTON_8: f32 = 17.06;

    #[test]
    fn confirm_buttons_sit_inside_the_panel_with_the_same_inset_as_the_sides() {
        // two body lines, as the warning wraps at 208 units
        let (buttons_y, h) = confirm_layout(2, BUTTON_8);
        let bottom = buttons_y + BUTTON_8;
        assert!(buttons_y >= CONFIRM_INSET - EPS, "buttons start above the inset");
        assert!(
            bottom <= h - CONFIRM_INSET + EPS,
            "buttons end at {bottom}, the panel's bottom inset starts at {}",
            h - CONFIRM_INSET
        );
        assert!((h - bottom - CONFIRM_INSET).abs() < EPS, "bottom inset is not 14");
        // the old fixed 88-unit panel was too short for the content
        assert!(h > 88.0 + 7.0, "panel is {h} tall, the old 88 hung the buttons out");
    }

    #[test]
    fn confirm_panel_grows_with_more_body_lines() {
        let (_, one) = confirm_layout(1, BUTTON_8);
        let (_, two) = confirm_layout(2, BUTTON_8);
        assert!((two - one - lh(F::Body, 8.0)).abs() < EPS);
    }

    #[test]
    fn main_page_buttons_fit_inside_the_pause_panel() {
        let panel = Rect::new(0.0, 0.0, PANEL_W, PANEL_H);
        let inner_top = panel.y + PANEL_INSET;
        let inner_bottom = panel.y + PANEL_H - PANEL_INSET;
        let area = page_area(Rect::new(
            panel.x + PANEL_INSET,
            inner_top,
            PANEL_W - PANEL_INSET * 2.0,
            PANEL_H - PANEL_INSET * 2.0,
        ));
        // RETURN, HUNTER DOSSIER, SKIN and SETTINGS are 18 tall, ABANDON ROUTE is 17.06 (the 14 minimum
        // grows to the measured label height, capped at 18)
        let heights = [18.0, 18.0, 18.0, 18.0, BUTTON_8];
        assert_eq!(heights.len(), MAIN_ITEMS.len());
        let tops = centred_column(area.y, area.h, &heights, 8.0);
        assert_eq!(tops.len(), heights.len());
        for (t, h) in tops.iter().zip(heights.iter()) {
            assert!(*t >= area.y - EPS, "button top {t} is above the page area");
            assert!(
                t + h <= inner_bottom + EPS,
                "button bottom {} is below the panel's inner bottom {inner_bottom}",
                t + h
            );
            assert!(t + h <= panel.y + PANEL_H - PANEL_INSET + EPS);
        }
    }

    #[test]
    fn the_pause_page_has_a_skin_entry_between_the_dossier_and_settings() {
        let labels: Vec<&str> = MAIN_ITEMS.iter().map(|i| i.0).collect();
        assert_eq!(labels, ["RETURN", "HUNTER DOSSIER", "SKIN", "SETTINGS", "ABANDON ROUTE"]);
        assert_eq!(MAIN_ITEMS[ITEM_DOSSIER].0, "HUNTER DOSSIER");
        assert_eq!(MAIN_ITEMS[ITEM_SKIN].0, "SKIN");
        assert_eq!(MAIN_ITEMS[ITEM_SETTINGS].0, "SETTINGS");
        assert_eq!(MAIN_ITEMS[ITEM_ABANDON].0, "ABANDON ROUTE");
        assert_eq!(MAIN_ITEMS[0].3 as u8, Kind::Primary as u8, "RETURN stays the primary button");
        assert!(MAIN_ITEMS.iter().skip(1).all(|i| i.3 as u8 == Kind::Secondary as u8));
        assert_eq!(SKIN_TAB, 4, "the settings overlay's SKIN tab");
    }

    #[test]
    fn escape_leaves_the_skin_tab_before_it_closes_the_pause_menu() {
        let mut pz = Pause::default();
        pz.show();
        pz.sub = Sub::Skin;
        assert!(matches!(escape(&mut pz), Out::None), "the first Escape goes back to the pause page");
        assert!(pz.open && pz.sub == Sub::None);
        assert!(matches!(escape(&mut pz), Out::Resume), "the second one resumes the run");
        assert!(!pz.open);
    }

    fn runner(depth: i32, quiet_s: u64, spectating: bool, game_over: bool, escape_active: bool) -> RunnerState {
        RunnerState {
            depth,
            spectating,
            game_over,
            escape_active,
            quiet: Duration::from_secs(quiet_s),
        }
    }

    fn row(name: &str, host: bool) -> PlayerRow {
        PlayerRow {
            name: name.into(),
            host,
            you: false,
            ping: String::new(),
            status: String::new(),
            floor: String::new(),
            colour: WHITE,
            kick: None,
        }
    }

    #[test]
    fn status_follows_the_godot_order_of_checks() {
        // outside a run every row is IN LOBBY, whatever its state
        assert_eq!(status_of(Some(&runner(1, 0, false, true, false)), false, false), STATUS_LOBBY);
        // no state packet yet: just joined
        assert_eq!(status_of(None, false, true), STATUS_ALIVE);
        assert_eq!(status_of(Some(&runner(1, 0, false, false, false)), false, true), STATUS_ALIVE);
        // quiet for more than 8 s is DISCONNECTED, exactly 8 s is not (Godot `>`)
        assert_eq!(status_of(Some(&runner(1, 9, false, false, false)), false, true), STATUS_DISCONNECTED);
        assert_eq!(status_of(Some(&runner(1, 8, false, false, false)), false, true), STATUS_ALIVE);
        // the viewer's own row never goes DISCONNECTED: its state is local
        assert_eq!(status_of(Some(&runner(1, 99, false, false, false)), true, true), STATUS_ALIVE);
        // DISCONNECTED wins over SPECTATING and CAUGHT
        assert_eq!(status_of(Some(&runner(1, 9, true, true, true)), false, true), STATUS_DISCONNECTED);
        // SPECTATING wins over CAUGHT
        assert_eq!(status_of(Some(&runner(1, 0, true, true, false)), false, true), STATUS_SPECTATING);
        // a route ended or an escape struggle on: CAUGHT
        assert_eq!(status_of(Some(&runner(1, 0, false, true, false)), false, true), STATUS_CAUGHT);
        assert_eq!(status_of(Some(&runner(1, 0, false, false, true)), true, true), STATUS_CAUGHT);
    }

    #[test]
    fn order_puts_the_host_first_then_by_name() {
        let mut rows = vec![row("ZED", false), row("MAX", false), row("HOST", true), row("ANA", false)];
        order(&mut rows);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["HOST", "ANA", "MAX", "ZED"]);
    }

    #[test]
    fn floor_and_ping_text_match_the_godot_list() {
        // level is -depth; the sign is always shown; no state is "-"
        assert_eq!(floor_text(Some(-1)), "-1");
        assert_eq!(floor_text(Some(0)), "+0");
        assert_eq!(floor_text(Some(-4)), "-4");
        assert_eq!(floor_text(None), "-");
        // a ping not measured yet is "-", a measured one is "N MS"
        assert_eq!(ping_text(None), "-");
        assert_eq!(ping_text(Some(0)), "0 MS");
        assert_eq!(ping_text(Some(42)), "42 MS");
    }

    #[test]
    fn only_the_owner_has_the_owner_tab() {
        assert_eq!(tab_names(false), ["PAUSE", "STATISTICS", "PLAYERS"]);
        let owner = tab_names(true);
        assert_eq!(owner.len(), 4);
        assert_eq!(owner[OWNER_TAB], hosted::rules().strings.tab_owner);
        assert_eq!(&owner[..3], &tab_names(false)[..]);
        // four tabs of 80 and three gaps of 4 fit the panel's inner width
        assert!(80.0 * 4.0 + 4.0 * 3.0 <= PANEL_W - PANEL_INSET * 2.0);
    }

    #[test]
    fn kick_is_for_the_owner_and_for_guests_only() {
        assert_eq!(kick_target(true, false, 7), Some(7));
        assert_eq!(kick_target(false, false, 7), None, "a guest who does not own the room cannot kick");
        assert_eq!(kick_target(true, true, 1), None, "the host is never kicked");
        assert_eq!(kick_target(true, false, 1), None, "peer 1 is the host whatever the row says");
        assert_eq!(kick_target(true, false, 0), None);
    }

    #[test]
    fn centred_column_spaces_items_by_the_gap_and_centres_the_block() {
        let tops = centred_column(10.0, 100.0, &[20.0, 20.0], 10.0);
        // block = 20 + 10 + 20 = 50, so it starts at 10 + 25
        assert!((tops[0] - 35.0).abs() < EPS);
        assert!((tops[1] - 65.0).abs() < EPS);
    }
}
