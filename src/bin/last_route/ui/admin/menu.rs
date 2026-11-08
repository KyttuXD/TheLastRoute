//! The admin panel itself (admin_menu.gd `_build` / `_fit_panel` / `_fill_section`): a black
//! 80% dimmer, the opaque PANEL box with an AMBER hairline, "A D M I N  / /  A U T H O R I Z E D"
//! and the "0 / ESC" close button, a clipped TabBar (STATUS, one tab per section, DEBUG VIEW,
//! FIELDS) with Godot's round scroll arrows, the INSET body holding the scrolling page, and the
//! hint line. Section pages hold: dropdown rows (caption, OptionButton, RUN), ON/OFF toggle rows,
//! one ACTION dropdown for the section's plain actions, the hunter picker, the NPC panel and the
//! FIELDS line edits. Clicks become `Cmd`s that `actions.rs` runs on the Game.

use super::look::*;
use super::actions::{npc_button, NpcButton};
use super::menu_data::menu;
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

/// The NPC spawn count after one click of the left or right arrow, kept within 1 and the data's maximum.
pub(super) fn step_count(count: i32, up: bool) -> i32 {
    let next = if up { count + 1 } else { count - 1 };
    next.clamp(1, menu().npc_spawn_max)
}

pub(super) fn labels(list: &[(&str, &str)]) -> Vec<String> {
    list.iter().map(|(_, label)| label.to_string()).collect()
}

/// Hunter picker order: the A-Z roster, then everything else (Void, the secret creatures, pack hunters), by id, then the wolf spider pair.
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

/// `text` cut with "..." so that it is no wider than `width` (the one hint line has no room to wrap).
fn fit(ui: &Ui, text: &str, size: f32, width: f32) -> String {
    if ui.tw(text, size) <= width {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while !s.is_empty() && ui.tw(&format!("{s}..."), size) > width {
        s.pop();
    }
    format!("{s}...")
}

pub(super) fn wrap(ui: &Ui, text: &str, size: f32, width: f32) -> Vec<String> {
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

        // (the password message box over the menu takes the click, see password.rs)
        let mut click = is_mouse_button_pressed(MouseButton::Left) && self.reset.notice.is_none();
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
        let hint = if self.tab == 9 {
            "ENTER RUNS A FIELD  //  UP/DOWN MOVES  //  ESC CLOSES"
        } else {
            "0 OR ESC CLOSES  //  TAB SWITCHES  //  UP/DOWN SCROLLS"
        };
        // a gray NPC button under the mouse says why it is gray in place of the hint (set by the grid last frame)
        let tip = self.npc_tip.take().filter(|_| self.tab == 4);
        let (hint, hint_col) = match tip.as_deref() {
            Some(t) => (fit(&ui, t, 6.0, cw), AMBER),
            None => (hint.to_string(), MUTED),
        };
        ui.text(
            &hint,
            cx,
            panel.y + (size.y - 15.0) * u,
            6.0,
            hint_col,
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
        if self.popup.is_none() {
            self.scroll[tab] += self.scroll_keys * u;
        }
        self.scroll_keys = 0.0;
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
                let status = self.status_with(g, &self.room);
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
                    menu().items.iter().map(|(_, l)| l.to_string()).collect(),
                );
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_general",
                    "ACTION",
                    labels(&menu().act_general),
                );
                self.reset_row(&mut ui, &mut col, g);
            }
            2 => {
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "teleport",
                    "TELEPORT",
                    menu().teleports.iter().map(|(_, l)| l.to_string()).collect(),
                );
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "run_speed",
                    "RUN SPEED",
                    menu().run_speeds.iter().map(|(_, l)| l.to_string()).collect(),
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
                    labels(&menu().act_player),
                );
                self.reset_row(&mut ui, &mut col, g);
            }
            3 => {
                self.hunter_tool(&mut ui, &mut col, &mut cmds, &hunters);
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_hunters",
                    "ACTION",
                    labels(&menu().act_hunters),
                );
            }
            4 => self.npc_panel(&mut ui, &mut col, &mut cmds, g),
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
                    menu().game_speeds.iter().map(|(_, l)| l.to_string()).collect(),
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
                    labels(&menu().act_world),
                );
            }
            6 => {
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "story_start",
                    "PLAY STORY / CUTSCENE",
                    menu().stories.iter().map(|(_, l)| l.to_string()).collect(),
                );
                self.dd_row(
                    &mut ui,
                    &mut col,
                    &mut cmds,
                    "act_story",
                    "ACTION",
                    labels(&menu().act_story),
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
                    labels(&menu().act_visual),
                );
                self.reset_row(&mut ui, &mut col, g);
            }
            8 => {
                self.toggle_row(&mut ui, &mut col, &mut cmds, "fullbright", "FULLBRIGHT");
                self.toggle_row(&mut ui, &mut col, &mut cmds, "wireframe", "WIREFRAME");
                self.reset_row(&mut ui, &mut col, g);
            }
            _ => self.fields_page(&mut ui, &mut col, &mut cmds, content),
        }
        self.content_h[tab] = (col.y - top - 4.0 * u).max(0.0);
        unsafe {
            get_internal_gl().quad_gl.scissor(None);
        }
        ui.clip = None;
        // a page taller than the body shows where it is (it scrolls by wheel, keys and PgUp/PgDn)
        if self.content_h[tab] > page.h + 0.5 {
            let track = Rect::new(body.x + body.w - 4.0 * u, page.y, 2.0 * u, page.h);
            let thumb_h = (track.h * page.h / self.content_h[tab]).max(8.0 * u).min(track.h);
            let frac = self.scroll[tab] / (self.content_h[tab] - page.h).max(1.0);
            draw_rectangle(track.x, track.y, track.w, track.h, PANEL_EDGE);
            draw_rectangle(track.x, track.y + (track.h - thumb_h) * frac.clamp(0.0, 1.0), track.w, thumb_h, ACCENT);
        }
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

    /// RESET ALL SETTINGS: the switches, speeds and fields back to their defaults (and the saved
    /// preferences with them). On every tab that holds switches.
    fn reset_row(&mut self, ui: &mut Ui, col: &mut Col, g: &mut Game) {
        let r = col.take(17.0, 4.0);
        let label = "RESET ALL SETTINGS";
        let w = ui.tw(label, 7.0) + 20.0 * col.u;
        let focus = self.focus.as_deref() == Some("reset_all");
        if ui.button(Rect::new(r.x, r.y, w, r.h), label, 7.0, false, focus) {
            self.focus = Some("reset_all".into());
            self.reset_all();
            crate::lr_info!("admin: reset all settings");
            g.say("ADMIN // ALL ADMIN SETTINGS RESET");
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
            match self.current_of(key) {
                Some(now) => ui.text(&format!("{caption} // NOW {now}"), c.x, c.y, 7.0, AMBER),
                None => ui.text(caption, c.x, c.y, 7.0, AMBER),
            }
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
            menu().hunter_do.iter().map(|s| s.to_string()).collect(),
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
                menu().emotes.iter().map(|s| s.to_string()).collect(),
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

    /// AdminNpcPanel: SPAWN (count stepper, smartness, SPAWN NPC), the removes, the NPC picker and
    /// its per-NPC grid, and the ALL NPCS grid. Separation 2 inside the panel.
    fn npc_panel(&mut self, ui: &mut Ui, col: &mut Col, cmds: &mut Vec<Cmd>, g: &Game) {
        let u = col.u;
        let cap = |ui: &Ui, col: &mut Col, s: &str| {
            let r = col.take(line_h(7.0), 2.0);
            if ui.visible(r) {
                ui.text(s, r.x, r.y, 7.0, MUTED);
            }
        };
        cap(ui, col, &format!("SPAWN // 1 TO {} AT ONCE", menu().npc_spawn_max));
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
                    // the original has no upper limit; a stray hold of the arrow would spawn thousands
                    self.npc_count = step_count(self.npc_count, right);
                }
            }
        }
        if ui.visible(count) {
            ui.text_center(&self.npc_count.to_string(), count, 7.0, MUTED);
        }
        let smarts: Vec<String> = menu().npc_smarts.iter().map(|s| format!("SMART {s}")).collect();
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
        // the removes act on whatever NPCs exist, so they are gray (with the reason) only when there are none
        let removes = if g.npcs.is_empty() { NpcButton::NoNpcs } else { NpcButton::Ready };
        if self.npc_button_at(ui, Rect::new(row.x, row.y, w1, row.h), "REMOVE NEAREST", removes) {
            cmds.push(Cmd::Npc("remove_nearest"));
        }
        if self.npc_button_at(ui, Rect::new(row.x + w1 + 4.0 * u, row.y, w2, row.h), "REMOVE ALL", removes) {
            cmds.push(Cmd::Npc("remove_all"));
        }
        cap(ui, col, "NPC");
        let r = col.take(19.5, 2.0);
        let names: Vec<String> = g.npcs.iter().map(|n| n.name.clone()).collect();
        self.dropdown(ui, r, "npc_pick", names, 8.0);
        let caption = match self.picked_npc(g) {
            Some(i) => format!("{} COMMANDS", g.npcs[i].name),
            None => "NO NPCS".to_string(),
        };
        cap(ui, col, caption.as_str());
        self.grid(ui, col, cmds, &menu().npc_ops, g.npcs.len());
        cap(ui, col, "ALL NPCS");
        self.grid(ui, col, cmds, &menu().npc_all, g.npcs.len());
    }

    /// One NPC button: a normal one when `state` is Ready, else drawn gray. A gray button still takes the click (the
    /// run says why nothing happened) and, while the mouse is on it, puts its reason in the hint line.
    fn npc_button_at(&mut self, ui: &mut Ui, rect: Rect, label: &str, state: NpcButton) -> bool {
        if state == NpcButton::Ready {
            return ui.button(rect, label, 7.0, false, false);
        }
        if ui.visible(rect) && ui.hover(rect) {
            self.npc_tip = state.tip();
        }
        ui.button_muted(rect, label, 7.0)
    }

    /// GridContainer, 3 columns, 4-unit separations, columns as wide as their widest button.
    fn grid(
        &mut self,
        ui: &mut Ui,
        col: &mut Col,
        cmds: &mut Vec<Cmd>,
        list: &[(&'static str, &'static str)],
        npcs: usize,
    ) {
        let u = col.u;
        let mut widths = [0.0f32; 3];
        for (i, (_, l)) in list.iter().enumerate() {
            widths[i % 3] = widths[i % 3].max(ui.tw(l, 7.0) + 20.0 * u);
        }
        let rows = (list.len() + 2) / 3;
        for r in 0..rows {
            let row = col.take(17.0, if r + 1 == rows { 2.0 } else { 4.0 });
            let mut x = row.x;
            for c in 0..3 {
                if let Some((op, label)) = list.get(r * 3 + c) {
                    let rect = Rect::new(x, row.y, widths[c], row.h);
                    if self.npc_button_at(ui, rect, label, npc_button(op, npcs)) {
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

    fn fields_page(
        &mut self,
        ui: &mut Ui,
        col: &mut Col,
        cmds: &mut Vec<Cmd>,
        content: &Content,
    ) {
        if let Some(i) = self.field_focus {
            // typing, Backspace and Ctrl/Cmd + V / C / X on the focused box (a dev tool: no select all)
            let mut no_selection = false;
            crate::ui::text_field::edit(crate::ui::text_field::Kind::Free, &mut self.fields[i], &mut no_selection);
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                cmds.push(Cmd::Field(i));
            }
        }
        for (i, (label, placeholder)) in menu().fields.iter().enumerate() {
            // RESET PASSWORD is for the full admin owners only
            if !self.field_visible(i) {
                continue;
            }
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
            if i == super::password::RESET_FIELD {
                let r = col.take(17.0, 4.0);
                let busy = self.reset_busy();
                let text = if busy { "RESETTING..." } else { "RESET PASSWORD" };
                let button = Rect::new(r.x, r.y, ui.tw("RESET PASSWORD", 7.0) + 20.0 * col.u, r.h);
                if busy {
                    if ui.visible(button) {
                        ui.button_face(button, text, 7.0, false, false);
                    }
                } else if ui.button(button, text, 7.0, false, false) {
                    cmds.push(Cmd::Field(i));
                }
            }
        }

        let c = col.take(line_h(7.0), 6.0);
        if ui.visible(c) {
            ui.text("TUNABLES // READ ONLY", c.x, c.y, 7.0, MUTED);
        }
        let mut group = String::new();
        for (path, key, value) in tunable_rows(content, &self.prefs) {
            if path != group {
                if !group.is_empty() {
                    col.y += 4.0 * col.u;
                }
                let c = col.take(line_h(7.0), 2.0);
                if ui.visible(c) {
                    ui.text(&path, c.x, c.y, 7.0, AMBER);
                }
                group = path;
            }
            let r = col.take(line_h(6.0), 1.0);
            if ui.visible(r) {
                ui.text(&format!("{key} = {value}"), r.x, r.y, 6.0, BODY);
            }
        }
    }
}

/// One `config_rows!` entry per named constant of src/config/mod.rs: its file, name and value.
macro_rules! config_rows {
    ($rows:ident, $file:expr, $($name:ident),* $(,)?) => {
        $(
            $rows.push((
                $file.to_string(),
                stringify!($name).to_string(),
                format!("{}", crate::config::$name),
            ));
        )*
    };
}

/// Every tunable the run reads, as (file, key, value): the config constants, each difficulty
/// profile, hunter data, region data and the admin preferences (the FIELDS list, read-only).
fn tunable_rows(content: &Content, prefs: &super::prefs::Prefs) -> Vec<(String, String, String)> {
    const CONFIG: &str = "src/config/mod.rs";
    const DIFFICULTY: &str = "src/config/difficulty.rs";
    let mut rows = vec![];

    config_rows!(
        rows,
        CONFIG,
        // player movement
        TILE_SIZE,
        PLAYER_WALK_SPEED,
        PLAYER_SPRINT_SPEED,
        PLAYER_CROUCH_SPEED,
        PLAYER_GRAVITY,
        PLAYER_JUMP_VELOCITY,
        PLAYER_RADIUS,
        PLAYER_HEIGHT,
        PLAYER_MODEL_SCALE,
        CAMERA_HEIGHT,
        CROUCH_EYE_RATIO,
        CAMERA_BOB_WALK,
        CAMERA_BOB_SPRINT,
        CAMERA_BOB_CROUCH,
        FIELD_OF_VIEW_DEG,
        MOUSE_SENSITIVITY,
        MOUSE_LOOK_DISPLAY_SCALE,
        MOUSE_LOOK_GODOT_WINDOW_SCALE,
        FOV_MIN_DEG,
        FOV_MAX_DEG,
        SENSITIVITY_MIN,
        SENSITIVITY_MAX,
        KEY_LOOK_RADIANS_PER_SECOND,
        // stamina
        MAX_STAMINA,
        STAMINA_RECOVERY,
        STAMINA_EXHAUSTED_RECOVERY,
        STIM_INJECT_SECONDS,
        STIM_BOOST_SECONDS,
        STIM_SPEED_SCALE,
        STIM_DRAIN_SCALE,
        STIM_CRASH_SECONDS,
        STIM_CRASH_REGEN_SCALE,
        // flashlight
        MAX_LAMP_CHARGE,
        LAMP_IDLE_DRAIN,
        LAMP_SPRINT_DRAIN,
        BATTERY_RECHARGE,
        // noise and meters
        NOISE_CROUCH,
        NOISE_WALK,
        NOISE_SPRINT,
        HEARING_OUTER_SCALE,
        HEARING_OPEN_AIR_SCALE,
        // tools
        TASER_RANGE,
        TASER_HALF_ANGLE,
        TASER_SLOW_FACTOR,
        TASER_SLOW_SECONDS,
        MELEE_STRIKE_RANGE,
        MELEE_STRIKE_HALF_ANGLE,
        MELEE_STRIKE_COOLDOWN,
        CROWBAR_DURABILITY,
        AXE_DURABILITY,
        CROWBAR_SWING_COST,
        AXE_SWING_COST,
        AXE_NOISE,
        AXE_NOISE_SECONDS,
        BATTERY_STACK_MAX,
        STIM_STACK_MAX,
        HOTBAR_SIZE,
        INTERACT_REACH,
        LOCKER_REACH,
        // hunters
        HUNTER_RADIUS,
        HUNTER_GRAB_REACH,
        HUNTER_REPATH_CHASE,
        HUNTER_REPATH_WANDER,
        HUNTER_STAMINA_MAX,
        HUNTER_STAMINA_REGEN,
        HUNTER_STAMINA_RECOVER,
        HUNTER_WINDED_SPEED_SCALE,
        HUNTER_SENSE_INTERVAL,
        HUNTER_VISIBLE_CHASE_SPEED_SCALE,
        UNLIT_SIGHT_SCALE,
        FLASHLIGHT_SIGHT_SCALE,
        FLASHLIGHT_SIGHT_MIN,
        HUNTER_RELAY_SPEED_BONUS,
        HUNTER_DEPTH_SPEED_BONUS_PER_LEVEL,
        HUNTER_DEPTH_SPEED_BONUS_MAX,
        HUNTER_ARRIVAL_DISTANCE,
        HUNTER_WAYPOINT_DISTANCE,
        UNREACHABLE_GIVE_UP,
        HIDE_CHECK_REACH,
        CAMP_RADIUS,
        CAMP_COOLDOWN,
        CAMP_LEAVE_DISTANCE,
        HUNTER_HEIGHT,
        // capture and escape
        ESCAPE_DURATION,
        ESCAPE_PRESSES,
        ESCAPE_COOLDOWN,
        ESCAPE_GRACE,
        ESCAPE_KNOCKBACK,
        // facility
        FACILITY_COLS,
        FACILITY_ROWS,
        FACILITY_RELAY_COUNT,
        FACILITY_BATTERY_COUNT,
        FACILITY_ROOM_COUNT,
        HIDING_SPOT_COUNT,
        HALL_WIDTH,
        LOBBY_SIZE,
        SECTOR_GAP,
        SECTOR_MARGIN,
        MIN_AUTHORED_ROOM_SIZE,
        ROOM_SECTOR_FILL,
        WALL_HEIGHT,
        SAFE_ZONE_RADIUS,
        SAFE_ZONE_BLOCK_RADIUS,
        RELAY_SCORE,
    );

    for d in crate::config::difficulty::ALL {
        let p = d.profile();
        let values = [
            ("player_speed", p.player_speed.to_string()),
            ("crouch_speed", p.crouch_speed.to_string()),
            ("sprint_speed", p.sprint_speed.to_string()),
            ("lamp_drain", p.lamp_drain.to_string()),
            ("sprint_drain", p.sprint_drain.to_string()),
            ("hunter_speed", p.hunter_speed.to_string()),
            ("hunter_sight", p.hunter_sight.to_string()),
            ("hunter_hearing", p.hunter_hearing.to_string()),
            ("capture_distance", p.capture_distance.to_string()),
            ("search_time", p.search_time.to_string()),
            ("lost_sight_grace", p.lost_sight_grace.to_string()),
            ("stamina_drain", p.stamina_drain.to_string()),
            ("hide_tell", p.hide_tell.to_string()),
            ("hide_trail", p.hide_trail.to_string()),
            ("hide_cooldown", p.hide_cooldown.to_string()),
            ("hide_random", p.hide_random.to_string()),
            ("taser_slow_factor", p.taser_slow_factor.to_string()),
            ("taser_slow_seconds", p.taser_slow_seconds.to_string()),
            ("taser_cooldown", p.taser_cooldown.to_string()),
            ("escape_stun", p.escape_stun.to_string()),
            ("crowbar_stun", p.crowbar_stun.to_string()),
            ("axe_stun", p.axe_stun.to_string()),
            ("relay_alert_radius", p.relay_alert_radius.to_string()),
            ("relay_alert_chance", p.relay_alert_chance.to_string()),
            ("camp_limit", p.camp_limit.to_string()),
            ("activation_relays", p.activation_relays.to_string()),
            ("tier_weights", format!("{:?}", d.tier_weights())),
            ("safe_zone_limit", d.safe_zone_limit().to_string()),
        ];
        for (key, value) in values {
            rows.push((DIFFICULTY.to_string(), format!("{}.{key}", d.name()), value));
        }
    }

    for h in &content.hunters {
        let file = if h.pack.is_empty() {
            format!("data/hunters/{}.json", h.id)
        } else {
            format!("pack {}", h.pack)
        };
        let tier = ["easy", "intermediate", "difficult"][h.tier.min(2)];
        let values = [
            ("tier", tier.to_string()),
            ("speed", h.speed.to_string()),
            ("chase", h.chase.to_string()),
            ("prog", h.prog.to_string()),
            ("sight", h.sight.to_string()),
            ("hearing", h.hearing.to_string()),
            ("spawn", h.spawn.to_string()),
            ("chase_style", h.chase_style.clone()),
            ("taser_immune", h.taser_immune.to_string()),
            ("melee_immune", h.melee_immune.to_string()),
        ];
        for (key, value) in values {
            rows.push((file.clone(), key.to_string(), value));
        }
    }

    for r in &content.regions.defs {
        let file = if r.source == crate::content::regions::BUILTIN {
            format!("src/bin/last_route/data/regions/{}.json", r.id)
        } else {
            format!("pack {}", r.source)
        };
        let geo_note = if r.geometry.is_some() { "" } else { " (game default)" };
        let geo = r.geometry.unwrap_or_default();
        let light_note = if r.safe_light.is_some() { "" } else { " (game default)" };
        let light = r.safe_light.unwrap_or_default();
        let mut values: Vec<(String, String)> = vec![
            ("hall_runner".into(), r.hall_runner.to_string()),
            ("hall_floor".into(), r.hall_floor.join(", ")),
            ("room_default_floor".into(), r.room_default_floor.join(", ")),
            ("room_roles".into(), r.room_roles.join(", ")),
            ("room_door".into(), r.room_door.clone().unwrap_or_else(|| "-".into())),
            (
                "double_room_door".into(),
                r.double_room_door.clone().unwrap_or_else(|| "-".into()),
            ),
            ("geometry.wall_height".into(), format!("{}{geo_note}", geo.wall_height)),
            ("geometry.room_min".into(), format!("{}{geo_note}", geo.room_min)),
            ("geometry.room_max".into(), format!("{}{geo_note}", geo.room_max)),
            ("safe_light.color".into(), format!("{:?}{light_note}", light.color)),
            ("safe_light.energy".into(), format!("{}{light_note}", light.energy)),
        ];
        for p in &r.prop_set {
            values.push((format!("prop_set.{}.size", p.prop_type), format!("{:?}", p.size)));
            values.push((format!("prop_set.{}.solid", p.prop_type), p.solid.to_string()));
        }
        for (key, value) in values {
            rows.push((file.clone(), key, value));
        }
    }

    let st = crate::game::stamina::rules();
    for (key, value) in [
        ("sprint_seconds", st.sprint_seconds),
        ("weak_below", st.weak_below),
        ("min_speed_scale", st.min_speed_scale),
    ] {
        rows.push(("data/player/stamina.json".to_string(), key.to_string(), value.to_string()));
    }

    for (key, _) in super::prefs::KEYS {
        rows.push((
            "admin.cfg (user folder)".to_string(),
            key.to_string(),
            prefs.get(key).to_string(),
        ));
    }
    rows
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
