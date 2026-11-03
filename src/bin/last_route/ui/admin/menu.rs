//! The admin panel itself (admin_menu.gd `_build` / `_fit_panel` / `_fill_section`): a black
//! 80% dimmer, the opaque PANEL box with an AMBER hairline, "A D M I N  / /  A U T H O R I Z E D"
//! and the "0 / ESC" close button, a clipped TabBar (STATUS, one tab per section, DEBUG VIEW,
//! FIELDS) with Godot's round scroll arrows, the INSET body holding the scrolling page, and the
//! hint line. Section pages hold: dropdown rows (caption, OptionButton, RUN), ON/OFF toggle rows,
//! one ACTION dropdown for the section's plain actions, the hunter picker, the NPC panel and the
//! FIELDS line edits. Clicks become `Cmd`s that `actions.rs` runs on the Game.

use super::look::*;
use super::{Admin, Popup, TABS};
use crate::app::assets::Assets;
use crate::content::Content;
use crate::game::Game;
use macroquad::prelude::*;

pub(super) enum Cmd {
    /// A dropdown row's RUN (or the section ACTION dropdown): action id, chosen index.
    Run(&'static str, usize),
    Toggle(&'static str),
    /// The hunter picker's RUN: hunter index, DO index, emote index.
    Hunter(usize, usize, usize),
    Npc(&'static str),
    Field(usize),
}

pub(super) const ITEMS: [(&str, &str); 11] = [
    ("flashlight", "FLASHLIGHT"),
    ("taser", "TASER"),
    ("lockpick", "LOCKPICK SET"),
    ("crowbar", "CROWBAR"),
    ("rescue_axe", "AXE"),
    ("battery", "FLASHLIGHT BATTERY"),
    ("stim_pack", "STIM PACK"),
    ("revive_kit", "REVIVE KIT"),
    ("backpack_small", "SMALL BACKPACK"),
    ("backpack_medium", "MEDIUM BACKPACK"),
    ("backpack_large", "LARGE BACKPACK"),
];
pub(super) const TELEPORTS: [(&str, &str); 8] = [
    ("north", "CHUNK N"),
    ("south", "CHUNK S"),
    ("east", "CHUNK E"),
    ("west", "CHUNK W"),
    ("safe_room", "SAFE ROOM"),
    ("relay", "NEAREST RELAY"),
    ("start", "START"),
    ("checkpoint", "CHECKPOINT"),
];
pub(super) const RUN_SPEEDS: [(f32, &str); 4] =
    [(1.0, "1X"), (1.5, "1.5X"), (2.0, "2X"), (3.0, "3X")];
pub(super) const GAME_SPEEDS: [(f32, &str); 6] = [
    (0.0, "PAUSE"),
    (0.25, "0.25X"),
    (1.0, "1X"),
    (2.0, "2X"),
    (4.0, "4X"),
    (-1.0, "STEP 1 FRAME"),
];
pub(super) const STORIES: [(&str, &str); 3] = [
    ("admin_sample_tour", "ADMIN SAMPLE TOUR"),
    ("first_signal", "FIRST SIGNAL"),
    ("ilyra_warning", "ILYRA WARNING"),
];
pub(super) const HUNTER_DO: [&str; 4] = [
    "SUMMON",
    "REMOVE (ALL MATCHES)",
    "TOGGLE (SUMMON / REMOVE ALL MATCHES)",
    "PLAY EMOTE (NEARBY)",
];
pub(super) const EMOTES: [&str; 14] = [
    "TAUNT",
    "LAUGH",
    "POINT",
    "BECKON",
    "SHRUG",
    "LOOK AROUND",
    "SMUG",
    "DANCE",
    "FLINCH",
    "STRETCH",
    "YAWN",
    "HUM",
    "WAVE",
    "VICTORY",
];
const ACT_GENERAL: [(&str, &str); 7] = [
    ("REFRESH ADMIN ROSTER", "roster_refresh"),
    ("RESTART ROUTE // NEW SEED", "restart_new"),
    ("RESTART ROUTE // SAME SEED", "restart_same"),
    ("END ROUTE", "end_route"),
    ("ROOM MUTE LAST SPEAKER", "chat_mute"),
    ("ROOM UNMUTE EVERYONE", "chat_unmute"),
    ("KICK LAST SPEAKER", "chat_kick"),
];
const ACT_PLAYER: [(&str, &str); 3] = [
    ("FULL LAMP", "full_lamp"),
    ("RESTORE PLAYER", "restore_player"),
    ("CLEAR STUN + COOLDOWNS", "clear_stun"),
];
const ACT_HUNTERS: [(&str, &str); 7] = [
    ("STUN ALL HUNTERS", "stun_hunters"),
    ("WAKE ALL HUNTERS", "wake_hunters"),
    ("CALM ALL HUNTERS", "calm_hunters"),
    ("FREEZE ALL HUNTERS", "freeze_hunters"),
    ("RELEASE ALL HUNTERS", "release_hunters"),
    ("SUMMON ALL HUNTERS", "summon_all"),
    ("REMOVE SUMMONED HUNTERS", "remove_summoned"),
];
const ACT_WORLD: [(&str, &str); 6] = [
    ("GRANT RELAY", "grant_relay"),
    ("ALL LOADED RELAYS", "all_relays"),
    ("DAMAGE CHECKPOINT", "damage_checkpoint"),
    ("REPAIR CHECKPOINT", "repair_checkpoint"),
    ("COPY SEED", "copy_seed"),
    ("COPY ROOM CODE", "copy_room_code"),
];
const ACT_STORY: [(&str, &str); 2] = [
    ("STOP STORY / CUTSCENE", "story_stop"),
    ("RESET STORY STATE", "story_reset"),
];
const ACT_VISUAL: [(&str, &str); 4] = [
    ("TELEPORT TO WAYPOINT", "waypoint_teleport"),
    ("CLEAR WAYPOINT", "waypoint_clear"),
    ("SPAWN RAVELLE TRAP HERE", "ravelle_trap"),
    ("LIGHT PROBE HERE", "light_probe"),
];
const NPC_OPS: [(&str, &str); 17] = [
    ("TELEPORT TO ME", "teleport"),
    ("GO TO ME", "goto"),
    ("FOLLOW ME", "follow"),
    ("STOP FOLLOWING", "unfollow"),
    ("FREEZE", "freeze"),
    ("RESUME", "resume"),
    ("HUNTERS IGNORE", "ignore"),
    ("HUNTERS TARGET", "target"),
    ("CHAT ON", "chat_on"),
    ("CHAT OFF", "chat_off"),
    ("SMART = SPAWN SMART", "smart"),
    ("DOWN", "down"),
    ("REVIVE", "revive"),
    ("KILL", "kill"),
    ("REMOVE", "remove"),
    ("CLEAR HUNTER FLAGS", "clear"),
    ("SPECTATE", "spectate"),
];
const NPC_ALL: [(&str, &str); 11] = [
    ("ALL TO ME", "all_teleport"),
    ("ALL FOLLOW ME", "all_follow"),
    ("ALL STOP FOLLOW", "all_unfollow"),
    ("ALL FREEZE", "all_freeze"),
    ("ALL RESUME", "all_resume"),
    ("ALL IGNORE", "all_ignore"),
    ("ALL TARGET", "all_target"),
    ("ALL CLEAR", "all_clear"),
    ("ALL CHAT ON", "all_chat_on"),
    ("ALL CHAT OFF", "all_chat_off"),
    ("ALL SMART", "all_smart"),
];
pub(super) const NPC_SMARTS: [i32; 4] = [30, 50, 70, 90];
pub(super) const FIELDS: [(&str, &str); 3] = [
    ("SAFE TELEPORT", "X,Z or X,Z,LEVEL"),
    (
        "ADD / UPDATE LIMITED ADMIN",
        "USERNAME:player,world,hunters,visual,inventory",
    ),
    ("REMOVE LIMITED ADMIN", "USERNAME"),
];

/// A VBox cursor down the page (pixels).
struct Col {
    x: f32,
    w: f32,
    y: f32,
    u: f32,
}

impl Col {
    fn take(&mut self, h: f32, sep: f32) -> Rect {
        let r = Rect::new(self.x, self.y, self.w, h * self.u);
        self.y += (h + sep) * self.u;
        r
    }
}

fn labels<const N: usize>(list: &[(&str, &str); N]) -> Vec<String> {
    list.iter().map(|(l, _)| l.to_string()).collect()
}

/// Hunter picker order: the A-Z roster, then everything else (Void, pack hunters), by id.
pub(super) fn hunter_list(content: &Content) -> Vec<(String, String)> {
    let mut az: Vec<(String, String)> = content
        .hunters
        .iter()
        .filter(|h| h.category == "az")
        .map(|h| (h.id.clone(), h.name.to_uppercase()))
        .collect();
    let mut rest: Vec<(String, String)> = content
        .hunters
        .iter()
        .filter(|h| h.category != "az")
        .map(|h| (h.id.clone(), h.name.to_uppercase()))
        .collect();
    az.sort();
    rest.sort();
    az.extend(rest);
    az
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

impl Admin {
    pub(super) fn draw_menu(&mut self, g: &mut Game, a: &Assets, content: &Content) {
        let (sw, sh) = (screen_width(), screen_height());
        let ppu = px_per_unit();
        let zoom = (1.6 / ppu).clamp(1.0, 2.4);
        let u = ppu * zoom;
        let room = vec2(sw / ppu, sh / ppu) / zoom;
        let size = vec2(
            (room.x * 0.94).clamp(200.0, 420.0),
            (room.y * 0.92).clamp(150.0, 300.0),
        )
        .min(room);
        let panel = Rect::new(
            (sw - size.x * u) / 2.0,
            (sh - size.y * u) / 2.0,
            size.x * u,
            size.y * u,
        );

        let mut click = is_mouse_button_pressed(MouseButton::Left);
        let wheel = mouse_wheel().1;
        let hunters = hunter_list(content);
        let mut cmds: Vec<Cmd> = vec![];

        // an open dropdown list takes the click first (picking an item or closing it)
        if let Some(p) = self.popup.as_mut() {
            let (rect, first, rows) = popup_geometry(p, u, sh);
            if wheel != 0.0 && rect.contains(Vec2::from(mouse_position())) {
                let max = (p.items.len() as f32 * PopupLook::ROW * u
                    - (rect.h - 2.0 * PopupLook::PAD * u))
                    .max(0.0);
                p.scroll = (p.scroll - wheel.signum() * PopupLook::ROW * u).clamp(0.0, max);
            }
            if click {
                click = false;
                let m = Vec2::from(mouse_position());
                let mut picked = None;
                for i in first..first + rows {
                    if popup_row(rect, i, p.scroll, u).contains(m) && rect.contains(m) {
                        picked = Some(i);
                    }
                }
                let key = p.key;
                if let Some(i) = picked {
                    self.sel.insert(key, i);
                }
                self.popup = None;
            }
        }

        // dimmer and frame
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.8));
        let mut ui = Ui::new(a, u, click);
        ui.boxed(panel, PANEL, AMBER);
        let cx = panel.x + 8.0 * u;
        let cw = panel.w - 16.0 * u;

        // header: tracked heading and the 0 / ESC close button
        ui.tracked(
            "ADMIN // AUTHORIZED",
            cx,
            panel.y + 8.0 * u + 2.5 * u,
            8.0,
            AMBER,
        );
        let close_w = ui.tw("0 / ESC", 7.0) + 10.0 * u;
        let close = Rect::new(cx + cw - close_w, panel.y + 8.0 * u, close_w, 15.0 * u);
        let close_focus = self.focus.as_deref() == Some("close");
        if ui.button(close, "0 / ESC", 7.0, false, close_focus) {
            self.set_open(false);
            return;
        }

        // tabs
        let ty = panel.y + 26.0 * u;
        let th = 17.0 * u;
        let gap = 0.8 * u;
        let widths: Vec<f32> = TABS.iter().map(|t| ui.tw(t, 7.0) + 20.0 * u).collect();
        let total: f32 = widths.iter().sum::<f32>() + gap * (TABS.len() - 1) as f32;
        let overflow = total > cw;
        let limit = if overflow { cw - 32.0 * u } else { cw };
        let fits = |from: usize, to: usize| {
            widths[from..=to].iter().sum::<f32>() + gap * (to - from) as f32 <= limit
        };
        if self.tab < self.tab_offset {
            self.tab_offset = self.tab;
        }
        while self.tab_offset < self.tab && !fits(self.tab_offset, self.tab) {
            self.tab_offset += 1;
        }
        let mut x = cx;
        let mut last_visible = self.tab_offset;
        for i in self.tab_offset..TABS.len() {
            if x + widths[i] > cx + limit + 0.5 {
                break;
            }
            let r = Rect::new(x, ty, widths[i], th);
            let selected = i == self.tab;
            let hover = ui.hover(r);
            let (fill, edge, col) = if selected {
                (ACCENT, ACCENT_EDGE, ON_AMBER)
            } else if hover {
                (TAB_HOVER, ACCENT_EDGE, INK)
            } else {
                (PANEL, PANEL_EDGE, BODY)
            };
            ui.boxed(r, fill, edge);
            ui.text_center(TABS[i], r, 7.0, col);
            if ui.clicked(r) {
                self.tab = i;
                self.popup = None;
                self.field_focus = None;
            }
            last_visible = i;
            x += widths[i] + gap;
        }
        if overflow {
            let missing_right = last_visible + 1 < TABS.len();
            let left = vec2(cx + cw - 24.0 * u, ty + th / 2.0);
            let right = vec2(cx + cw - 8.0 * u, ty + th / 2.0);
            scroll_arrow(&ui, left, 6.0 * u, false, self.tab_offset > 0);
            scroll_arrow(&ui, right, 6.0 * u, true, missing_right);
            if ui.click && left.distance(ui.mouse) <= 7.0 * u && self.tab_offset > 0 {
                ui.click = false;
                self.tab_offset -= 1;
            }
            if ui.click && right.distance(ui.mouse) <= 7.0 * u && missing_right {
                ui.click = false;
                self.tab_offset += 1;
            }
        }

        // body and hint
        let body = Rect::new(cx, panel.y + 46.0 * u, cw, (size.y - 64.0) * u);
        ui.boxed(body, INSET, PANEL_EDGE);
        ui.text(
            "0 OR ESC CLOSES  //  DROPDOWN, THEN RUN",
            cx,
            panel.y + (size.y - 15.0) * u,
            6.0,
            MUTED,
        );

        let page = Rect::new(
            body.x + 5.0 * u,
            body.y + 5.0 * u,
            body.w - 10.0 * u,
            body.h - 10.0 * u,
        );
        let tab = self.tab;
        let max_scroll = (self.content_h[tab] - page.h).max(0.0);
        if wheel != 0.0 && self.popup.is_none() && page.contains(ui.mouse) {
            self.scroll[tab] -= wheel.signum() * 20.0 * u;
        }
        self.scroll[tab] = self.scroll[tab].clamp(0.0, max_scroll);
        let mut col = Col {
            x: page.x,
            w: page.w,
            y: page.y - self.scroll[tab],
            u,
        };
        let top = col.y;
        ui.clip = Some(page);
        unsafe {
            get_internal_gl().quad_gl.scissor(Some((
                page.x as i32,
                page.y as i32,
                page.w.ceil() as i32,
                page.h.ceil() as i32,
            )));
        }
        let had_click = ui.click;
        match tab {
            0 => {
                let status = self.status(g);
                for line in wrap(&ui, &status, 8.0, page.w) {
                    let r = col.take(13.0, 0.0);
                    if ui.visible(r) {
                        ui.text(&line, r.x, r.y, 8.0, INK);
                    }
                }
            }
            1 => {
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "give_item",
                    "ITEMS",
                    ITEMS.iter().map(|(_, l)| l.to_string()).collect(),
                );
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_general",
                    "ACTION",
                    labels(&ACT_GENERAL),
                );
            }
            2 => {
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "teleport",
                    "TELEPORT",
                    TELEPORTS.iter().map(|(_, l)| l.to_string()).collect(),
                );
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "run_speed",
                    "RUN SPEED",
                    RUN_SPEEDS.iter().map(|(_, l)| l.to_string()).collect(),
                );
                for (id, label) in [
                    ("no_target", "NO TARGET"),
                    ("god_mode", "GOD MODE"),
                    ("noclip", "NOCLIP"),
                ] {
                    self.toggle_row(&mut ui, &mut col, &mut cmds, id, label);
                }
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_player",
                    "ACTION",
                    labels(&ACT_PLAYER),
                );
            }
            3 => {
                self.hunter_tool(&mut ui, &mut col, &mut cmds, &hunters);
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_hunters",
                    "ACTION",
                    labels(&ACT_HUNTERS),
                );
            }
            4 => self.npc_panel(&mut ui, &mut col, &mut cmds),
            5 => {
                let seeds = format!("RUN SEED {}  FLOOR SEED {}", g.run_seed, g.level_seed());
                for line in wrap(&ui, &seeds, 7.0, page.w) {
                    let r = col.take(line_h(7.0), 2.0);
                    if ui.visible(r) {
                        ui.text(&line, r.x, r.y, 7.0, INK);
                    }
                }
                let infinite = g.inf.is_some();
                let (label, key, hint) = if infinite {
                    (
                        "REGENERATE WINDOW",
                        "regen_window",
                        "REBUILDS THIS WINDOW FROM ITS SEED",
                    )
                } else {
                    (
                        "REGENERATE FLOOR",
                        "regen_floor",
                        "REBUILDS THIS FLOOR FROM ITS SEED",
                    )
                };
                let r = col.take(17.0, 4.0);
                let w = ui.tw(label, 7.0) + 20.0 * u;
                let focus = self.focus.as_deref() == Some(key);
                if ui.button(Rect::new(r.x, r.y, w, r.h), label, 7.0, false, focus) {
                    self.focus = Some(key.into());
                    if infinite {
                        self.regen_window(g, content);
                    } else {
                        self.regen_floor(g, a, content);
                    }
                }
                let r = col.take(line_h(6.0), 4.0);
                if ui.visible(r) {
                    ui.text(hint, r.x, r.y, 6.0, MUTED);
                }
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "game_speed",
                    "GAME SPEED // SOLO RUNS ONLY",
                    GAME_SPEEDS.iter().map(|(_, l)| l.to_string()).collect(),
                );
                self.toggle_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "region_lights",
                    "ALL REGION LIGHTS",
                );
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_world",
                    "ACTION",
                    labels(&ACT_WORLD),
                );
            }
            6 => {
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "story_start",
                    "PLAY STORY / CUTSCENE",
                    STORIES.iter().map(|(_, l)| l.to_string()).collect(),
                );
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_story",
                    "ACTION",
                    labels(&ACT_STORY),
                );
            }
            7 => {
                for (id, label) in [
                    ("show_traps", "SHOW TRAPS"),
                    ("light_sight", "MUTUAL LIGHT SIGHT"),
                    ("map_reveal", "MAP REVEAL"),
                    ("region_map", "REGION MAP (F8)"),
                    ("show_collision", "SHOW COLLISION"),
                    ("hunter_reveal", "HUNTER REVEAL"),
                    ("perf_overlay", "FPS / SPIKE OVERLAY"),
                ] {
                    self.toggle_row(&mut ui, &mut col, &mut cmds, id, label);
                }
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_visual",
                    "ACTION",
                    labels(&ACT_VISUAL),
                );
            }
            8 => {
                self.toggle_row(&mut ui, &mut col, &mut cmds, "fullbright", "FULLBRIGHT");
                self.toggle_row(&mut ui, &mut col, &mut cmds, "wireframe", "WIREFRAME");
            }
            _ => self.fields_page(&mut ui, &mut col, &mut cmds),
        }
        self.content_h[tab] = (col.y - top - 4.0 * u).max(0.0);
        unsafe {
            get_internal_gl().quad_gl.scissor(None);
        }
        ui.clip = None;
        // a click anywhere else takes the focus off a text field
        if had_click && ui.click && tab == 9 {
            self.field_focus = None;
        }

        // the open list draws over everything
        if let Some(p) = self.popup.as_ref() {
            let selected = self.sel_of(p.key);
            draw_popup(&ui, p, selected, u, sh);
        }

        for c in cmds {
            self.run_cmd(c, g, content, &hunters);
        }
    }

    fn open_popup(&mut self, key: &'static str, anchor: Rect, items: Vec<String>) {
        self.focus = Some(key.to_string());
        self.popup = Some(Popup {
            key,
            anchor,
            items,
            scroll: 0.0,
        });
    }

    /// A dropdown filling the width (no RUN), as the hunter picker's.
    fn dropdown(&mut self, ui: &mut Ui, r: Rect, key: &'static str, items: Vec<String>, pad: f32) {
        let sel = self.sel_of(key).min(items.len().saturating_sub(1));
        let text = items.get(sel).cloned().unwrap_or_default();
        ui.dropdown_face(r, &text, self.focus.as_deref() == Some(key), pad);
        let pending = self.pending_popup == Some(key);
        if (ui.clicked(r) || pending) && !items.is_empty() {
            self.pending_popup = None;
            self.open_popup(key, r, items);
        }
    }

    /// `_add_items_dropdown` / `_add_action_dropdown`: AMBER caption, then the dropdown and a
    /// 46-unit RUN slab, 4 units apart.
    fn dd_row(
        &mut self,
        ui: &mut Ui,
        col: &mut Col,
        cmds: &mut Vec<Cmd>,
        key: &'static str,
        caption: &str,
        items: Vec<String>,
    ) {
        let u = col.u;
        let c = col.take(line_h(7.0), 4.0);
        if ui.visible(c) {
            ui.text(caption, c.x, c.y, 7.0, AMBER);
        }
        let r = col.take(18.0, 4.0);
        let dd = Rect::new(r.x, r.y, r.w - 50.0 * u, r.h);
        let run = Rect::new(r.x + r.w - 46.0 * u, r.y, 46.0 * u, r.h);
        self.dropdown(ui, dd, key, items, 5.0);
        let run_key = format!("{key}:run");
        if ui.button(
            run,
            "RUN",
            8.0,
            true,
            self.focus.as_deref() == Some(run_key.as_str()),
        ) {
            self.focus = Some(run_key);
            cmds.push(Cmd::Run(key, self.sel_of(key)));
        }
    }

    fn toggle_row(
        &mut self,
        ui: &mut Ui,
        col: &mut Col,
        cmds: &mut Vec<Cmd>,
        id: &'static str,
        label: &str,
    ) {
        let r = col.take(17.0, 4.0);
        let on = self.state_of(id);
        if ui.toggle_row(r, label, on, self.focus.as_deref() == Some(id)) {
            self.focus = Some(id.to_string());
            cmds.push(Cmd::Toggle(id));
        }
    }

    /// `_add_hunter_tool`: HUNTER dropdown, DO dropdown, the EMOTE row (only for Play emote) and
    /// one full-width RUN.
    fn hunter_tool(
        &mut self,
        ui: &mut Ui,
        col: &mut Col,
        cmds: &mut Vec<Cmd>,
        hunters: &[(String, String)],
    ) {
        let u = col.u;
        let c = col.take(line_h(7.0), 4.0);
        if ui.visible(c) {
            ui.text("HUNTER", c.x, c.y, 7.0, AMBER);
        }
        let r = col.take(18.0, 4.0);
        self.dropdown(
            ui,
            r,
            "hunter",
            hunters.iter().map(|(_, l)| l.clone()).collect(),
            5.0,
        );
        let c = col.take(line_h(7.0), 4.0);
        if ui.visible(c) {
            ui.text("DO", c.x, c.y, 7.0, AMBER);
        }
        let r = col.take(18.0, 4.0);
        self.dropdown(
            ui,
            r,
            "hunter_do",
            HUNTER_DO.iter().map(|s| s.to_string()).collect(),
            5.0,
        );
        if self.sel_of("hunter_do") == 3 {
            let r = col.take(18.0, 4.0);
            let lw = ui.tw("EMOTE", 7.0);
            ui.text_mid("EMOTE", r.x, r, 7.0, AMBER);
            let dd = Rect::new(r.x + lw + 4.0 * u, r.y, r.w - lw - 4.0 * u, r.h);
            self.dropdown(
                ui,
                dd,
                "emote",
                EMOTES.iter().map(|s| s.to_string()).collect(),
                5.0,
            );
        }
        let r = col.take(18.0, 4.0);
        let focus = self.focus.as_deref() == Some("hunter:run");
        if ui.button(r, "RUN", 8.0, true, focus) {
            self.focus = Some("hunter:run".into());
            cmds.push(Cmd::Hunter(
                self.sel_of("hunter"),
                self.sel_of("hunter_do"),
                self.sel_of("emote"),
            ));
        }
    }

    /// AdminNpcPanel: SPAWN (count stepper, smartness, SPAWN NPC), the removes, the NPC list and
    /// its per-NPC grid, and the ALL NPCS grid. Separation 2 inside the panel.
    fn npc_panel(&mut self, ui: &mut Ui, col: &mut Col, cmds: &mut Vec<Cmd>) {
        let u = col.u;
        let cap = |ui: &Ui, col: &mut Col, s: &str| {
            let r = col.take(line_h(7.0), 2.0);
            if ui.visible(r) {
                ui.text(s, r.x, r.y, 7.0, MUTED);
            }
        };
        cap(ui, col, "SPAWN");
        let row = col.take(19.5, 2.0);
        let less = Rect::new(row.x, row.y, 26.6 * u, row.h);
        let count = Rect::new(less.x + less.w + 4.0 * u, row.y, 20.0 * u, row.h);
        let more = Rect::new(count.x + count.w + 4.0 * u, row.y, 26.6 * u, row.h);
        let smart = Rect::new(more.x + more.w + 4.0 * u, row.y, 70.0 * u, row.h);
        for (r, right) in [(less, false), (more, true)] {
            if ui.visible(r) {
                ui.button_face(r, "", 7.0, false, false);
                let col = if ui.hover(r) { INK } else { BODY };
                ui.triangle(vec2(r.x + r.w / 2.0, r.y + r.h / 2.0), 4.6 * u, col, right);
                if ui.clicked(r) {
                    // no upper limit, as the original
                    self.npc_count = if right {
                        self.npc_count + 1
                    } else {
                        (self.npc_count - 1).max(1)
                    };
                }
            }
        }
        if ui.visible(count) {
            ui.text_center(&self.npc_count.to_string(), count, 7.0, MUTED);
        }
        let smarts: Vec<String> = NPC_SMARTS.iter().map(|s| format!("SMART {s}")).collect();
        if !self.sel.contains_key("npc_smart") {
            self.sel.insert("npc_smart", 2);
        }
        self.dropdown(ui, smart, "npc_smart", smarts, 8.0);
        let r = col.take(17.0, 2.0);
        if ui.button(r, "SPAWN NPC", 7.0, false, false) {
            cmds.push(Cmd::Npc("spawn"));
        }
        let row = col.take(17.0, 2.0);
        let w1 = ui.tw("REMOVE NEAREST", 7.0) + 20.0 * u;
        let w2 = ui.tw("REMOVE ALL", 7.0) + 20.0 * u;
        if ui.button(
            Rect::new(row.x, row.y, w1, row.h),
            "REMOVE NEAREST",
            7.0,
            false,
            false,
        ) {
            cmds.push(Cmd::Npc("remove_nearest"));
        }
        if ui.button(
            Rect::new(row.x + w1 + 4.0 * u, row.y, w2, row.h),
            "REMOVE ALL",
            7.0,
            false,
            false,
        ) {
            cmds.push(Cmd::Npc("remove_all"));
        }
        cap(ui, col, "NPC");
        let r = col.take(19.5, 2.0);
        if ui.visible(r) {
            ui.dropdown_face(r, "", false, 8.0);
            let _ = ui.clicked(r);
        }
        cap(ui, col, "NO NPCS");
        self.grid(ui, col, cmds, &NPC_OPS);
        cap(ui, col, "ALL NPCS");
        self.grid(ui, col, cmds, &NPC_ALL);
    }

    /// GridContainer, 3 columns, 4-unit separations, columns as wide as their widest button.
    fn grid(
        &mut self,
        ui: &mut Ui,
        col: &mut Col,
        cmds: &mut Vec<Cmd>,
        list: &[(&'static str, &'static str)],
    ) {
        let u = col.u;
        let mut widths = [0.0f32; 3];
        for (i, (l, _)) in list.iter().enumerate() {
            widths[i % 3] = widths[i % 3].max(ui.tw(l, 7.0) + 20.0 * u);
        }
        let rows = (list.len() + 2) / 3;
        for r in 0..rows {
            let row = col.take(17.0, if r + 1 == rows { 2.0 } else { 4.0 });
            let mut x = row.x;
            for c in 0..3 {
                if let Some((label, op)) = list.get(r * 3 + c) {
                    if ui.button(
                        Rect::new(x, row.y, widths[c], row.h),
                        label,
                        7.0,
                        false,
                        false,
                    ) {
                        cmds.push(Cmd::Npc(op));
                    }
                }
                x += widths[c] + 4.0 * u;
            }
        }
    }

    /// Rebuilds the current floor from its seed (the same layout, the runner back at its start)
    /// and forgets the hunters summoned on the old one.
    fn regen_floor(&mut self, g: &mut Game, a: &Assets, content: &Content) {
        let catalog = crate::world::catalog::Catalog::load();
        g.start_floor(content, &catalog, a);
        self.summoned.clear();
        self.mark_debug();
        g.say("ADMIN // FLOOR REGENERATED FROM ITS SEED");
    }

    /// Rebuilds the infinite route's live window from its seed (the runner and the window origin stay put).
    fn regen_window(&mut self, g: &mut Game, content: &Content) {
        let catalog = crate::world::catalog::Catalog::load();
        g.regen_window(content, &catalog);
        self.mark_debug();
        g.say("ADMIN // WINDOW REGENERATED FROM ITS SEED");
    }

    fn fields_page(&mut self, ui: &mut Ui, col: &mut Col, cmds: &mut Vec<Cmd>) {
        if let Some(i) = self.field_focus {
            while let Some(ch) = get_char_pressed() {
                if !ch.is_control() && self.fields[i].chars().count() < 120 {
                    self.fields[i].push(ch);
                }
            }
            if is_key_pressed(KeyCode::Backspace) {
                self.fields[i].pop();
            }
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                cmds.push(Cmd::Field(i));
            }
        }
        for (i, (label, placeholder)) in FIELDS.iter().enumerate() {
            let c = col.take(line_h(7.0), 4.0);
            if ui.visible(c) {
                ui.text(label, c.x, c.y, 7.0, AMBER);
            }
            let r = col.take(22.0, 4.0);
            if ui.line_edit(
                r,
                &self.fields[i],
                placeholder,
                self.field_focus == Some(i),
                self.clock,
            ) {
                self.field_focus = Some(i);
            }
        }
    }
}

/// The list's screen rect (below the button, pushed up to stay on screen, as tall as the
/// window at most), the first row drawn, and how many rows fit.
fn popup_geometry(p: &Popup, u: f32, sh: f32) -> (Rect, usize, usize) {
    let pad = PopupLook::PAD * u;
    let row = PopupLook::ROW * u;
    let full = p.items.len() as f32 * row + 2.0 * pad;
    let h = full.min(sh);
    let mut y = p.anchor.y + p.anchor.h;
    if y + h > sh {
        y = (sh - h).max(0.0);
    }
    let rect = Rect::new(p.anchor.x, y, p.anchor.w, h);
    let first = (p.scroll / row).floor() as usize;
    let rows = ((h / row).ceil() as usize + 1).min(p.items.len().saturating_sub(first));
    (rect, first, rows)
}

fn popup_row(rect: Rect, i: usize, scroll: f32, u: f32) -> Rect {
    let pad = PopupLook::PAD * u;
    Rect::new(
        rect.x + pad,
        rect.y + pad + i as f32 * PopupLook::ROW * u - scroll,
        rect.w - 2.0 * pad,
        PopupLook::ROW * u,
    )
}

/// Godot's default PopupMenu: grey rounded panel, radio items, the hovered (else the chosen)
/// row lifted.
fn draw_popup(ui: &Ui, p: &Popup, selected: usize, u: f32, sh: f32) {
    let (rect, first, rows) = popup_geometry(p, u, sh);
    rounded(rect, 3.0 * u, PopupLook::EDGE);
    rounded(
        Rect::new(rect.x + u, rect.y + u, rect.w - 2.0 * u, rect.h - 2.0 * u),
        2.5 * u,
        PopupLook::BG,
    );
    unsafe {
        get_internal_gl().quad_gl.scissor(Some((
            rect.x as i32,
            rect.y as i32,
            rect.w.ceil() as i32,
            rect.h.ceil() as i32,
        )));
    }
    let hovered = (first..first + rows)
        .find(|&i| popup_row(rect, i, p.scroll, u).contains(ui.mouse) && rect.contains(ui.mouse));
    let lit = hovered.unwrap_or(selected);
    for i in first..first + rows {
        let r = popup_row(rect, i, p.scroll, u);
        if i == lit {
            rounded(r, 1.5 * u, PopupLook::HILITE);
        }
        let c = vec2(r.x + 10.0 * u, r.y + r.h / 2.0);
        if i == selected {
            draw_circle(c.x, c.y, 6.9 * u, Color::new(0.86, 0.86, 0.86, 1.0));
            draw_circle(c.x, c.y, 4.4 * u, Color::new(0.11, 0.11, 0.11, 1.0));
        } else {
            draw_circle(c.x, c.y, 6.9 * u, Color::new(0.19, 0.19, 0.19, 1.0));
        }
        let col = if hovered == Some(i) {
            WHITE
        } else {
            PopupLook::TEXT
        };
        ui.text_mid(&p.items[i], r.x + 22.0 * u, r, 8.0, col);
    }
    unsafe {
        get_internal_gl().quad_gl.scissor(None);
    }
}
