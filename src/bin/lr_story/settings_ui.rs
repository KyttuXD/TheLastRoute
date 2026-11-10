//! The settings screen: tabs "Controls", "Skins" and "Settings". Opened from the main menu and from the pause menu. Every
//! change is written through `settings::set` at once. Keyboard (Up/Down/Left/Right, Enter, Esc, PageUp/PageDown) and the
//! mouse both work. The logic that does not need a window (slider maths, tab cycling, the key capture state machine, the
//! skin list order) is in plain functions with tests.

use crate::input::{self, Action};
use crate::model::Pack;
use crate::settings::{self, Settings};
use crate::ui;
use macroquad::prelude::*;

/// What a frame of the screen asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsAct {
    Stay,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Controls,
    Skins,
    Settings,
}

const TABS: [Tab; 3] = [Tab::Controls, Tab::Skins, Tab::Settings];

fn tab_label(t: Tab) -> &'static str {
    match t {
        Tab::Controls => "Controls",
        Tab::Skins => "Skins",
        Tab::Settings => "Settings",
    }
}

/// The next tab in `dir` (+1 or -1), wrapping.
pub fn cycle_tab(t: Tab, dir: i32) -> Tab {
    let i = TABS.iter().position(|x| *x == t).unwrap_or(0) as i32;
    TABS[(i + dir).rem_euclid(TABS.len() as i32) as usize]
}

// ---- slider maths ----

/// The slider value for a mouse x over a track from `left` of width `width`, clamped to `min..=max`.
pub fn slider_value_at(x: f32, left: f32, width: f32, min: f32, max: f32) -> f32 {
    if width <= 0.0 {
        return min;
    }
    (min + ((x - left) / width).clamp(0.0, 1.0) * (max - min)).clamp(min, max)
}

/// The value after one arrow step (`dir` +1 or -1); a step is 1/20 of the range. Clamped.
pub fn slider_step(v: f32, min: f32, max: f32, dir: i32) -> f32 {
    (v + dir as f32 * (max - min) / 20.0).clamp(min, max)
}

/// Where a value sits on its track, 0..1.
pub fn slider_frac(v: f32, min: f32, max: f32) -> f32 {
    if max <= min {
        return 0.0;
    }
    ((v - min) / (max - min)).clamp(0.0, 1.0)
}

// ---- key capture ----

/// "Press a key" capture: idle, then waiting for a key on an action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Capture {
    Idle,
    Waiting(Action),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureEvent {
    /// Nothing happened (still waiting, or idle).
    None,
    Bound(Action, KeyCode),
    Cancelled,
}

/// One step of the capture machine. `key` is the key pressed this frame, if any. Escape cancels; any other key binds.
pub fn capture_step(c: Capture, key: Option<KeyCode>) -> (Capture, CaptureEvent) {
    match (c, key) {
        (Capture::Waiting(_), Some(KeyCode::Escape)) => (Capture::Idle, CaptureEvent::Cancelled),
        (Capture::Waiting(a), Some(k)) => (Capture::Idle, CaptureEvent::Bound(a, k)),
        (c, _) => (c, CaptureEvent::None),
    }
}

// ---- skins list ----

/// The skin ids offered: "mannequin" first, then the pack skin ids sorted and de-duplicated.
pub fn skin_list<'a>(pack_skins: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut v: Vec<String> = pack_skins.filter(|s| *s != "mannequin").map(|s| s.to_string()).collect();
    v.sort();
    v.dedup();
    v.insert(0, "mannequin".to_string());
    v
}

/// Moves a grid index by (dx, dy) in a grid `cols` wide with `n` cells; stays put when the move leaves the grid.
pub fn grid_move(i: usize, dx: i32, dy: i32, cols: usize, n: usize) -> usize {
    if n == 0 || cols == 0 {
        return 0;
    }
    let x = (i % cols) as i32 + dx;
    let y = (i / cols) as i32 + dy;
    if x < 0 || x >= cols as i32 || y < 0 {
        return i;
    }
    let j = (y as usize) * cols + x as usize;
    if j < n {
        j
    } else {
        i
    }
}

// ---- setting rows ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingRow {
    Sensitivity,
    Fov,
    UiScale,
    InvertY,
    FlatTextures,
    NameTags,
    HotbarFade,
}

const SETTING_ROWS: [SettingRow; 7] = [
    SettingRow::Sensitivity,
    SettingRow::Fov,
    SettingRow::UiScale,
    SettingRow::InvertY,
    SettingRow::FlatTextures,
    SettingRow::NameTags,
    SettingRow::HotbarFade,
];

fn row_label(r: SettingRow) -> &'static str {
    match r {
        SettingRow::Sensitivity => "Mouse sensitivity",
        SettingRow::Fov => "Field of view",
        SettingRow::UiScale => "UI scale",
        SettingRow::InvertY => "Invert Y",
        SettingRow::FlatTextures => "Flat textures",
        SettingRow::NameTags => "Name tags",
        SettingRow::HotbarFade => "Hotbar fade",
    }
}

fn is_slider(r: SettingRow) -> bool {
    matches!(r, SettingRow::Sensitivity | SettingRow::Fov | SettingRow::UiScale)
}

fn slider_range(r: SettingRow) -> (f32, f32) {
    match r {
        SettingRow::Sensitivity => (0.25, 3.0),
        SettingRow::Fov => (60.0, 100.0),
        _ => (0.8, 1.3),
    }
}

fn slider_get(s: &Settings, r: SettingRow) -> f32 {
    match r {
        SettingRow::Sensitivity => s.mouse_sensitivity,
        SettingRow::Fov => s.fov_deg,
        _ => s.ui_scale,
    }
}

fn set_slider(s: &mut Settings, r: SettingRow, v: f32) {
    match r {
        SettingRow::Sensitivity => s.mouse_sensitivity = v,
        SettingRow::Fov => s.fov_deg = v.round(),
        SettingRow::UiScale => s.ui_scale = (v * 20.0).round() / 20.0,
        _ => {}
    }
}

fn step_slider(s: &mut Settings, r: SettingRow, dir: i32) {
    let (min, max) = slider_range(r);
    set_slider(s, r, slider_step(slider_get(s, r), min, max, dir));
}

fn slider_text(r: SettingRow, v: f32) -> String {
    match r {
        SettingRow::Sensitivity => format!("{:.2}x", v),
        SettingRow::Fov => format!("{:.0}", v),
        _ => format!("{:.0}%", v * 100.0),
    }
}

fn toggle_get(s: &Settings, r: SettingRow) -> bool {
    match r {
        SettingRow::InvertY => s.invert_y,
        SettingRow::FlatTextures => s.flat_textures,
        SettingRow::NameTags => s.show_name_tags,
        SettingRow::HotbarFade => s.hotbar_fade,
        _ => false,
    }
}

fn toggle(s: &mut Settings, r: SettingRow) {
    match r {
        SettingRow::InvertY => s.invert_y = !s.invert_y,
        SettingRow::FlatTextures => s.flat_textures = !s.flat_textures,
        SettingRow::NameTags => s.show_name_tags = !s.show_name_tags,
        SettingRow::HotbarFade => s.hotbar_fade = !s.hotbar_fade,
        _ => {}
    }
}

// ---- the screen ----

const ROW_H: f32 = 30.0;

pub struct SettingsScreen {
    tab: Tab,
    row: usize,
    skin_cursor: usize,
    capture: Capture,
    dragging: Option<SettingRow>,
}

impl SettingsScreen {
    pub fn new() -> SettingsScreen {
        SettingsScreen { tab: Tab::Controls, row: 0, skin_cursor: 0, capture: Capture::Idle, dragging: None }
    }

    pub fn frame(&mut self, _dt: f32, pack: &Pack, skins: &dyn Fn(&str) -> Option<Texture2D>) -> SettingsAct {
        let (sw, sh) = (screen_width(), screen_height());
        let pw = (sw - 40.0).min(900.0);
        let ph = (sh - 40.0).min(580.0);
        let panel = Rect::new((sw - pw) / 2.0, (sh - ph) / 2.0, pw, ph);
        let (mx, my) = mouse_position();
        let click = is_mouse_button_pressed(MouseButton::Left);
        let skin_ids = skin_list(pack.skins.keys().map(|s| s.as_str()));

        let mut act = SettingsAct::Stay;
        if self.capture != Capture::Idle {
            // Capture takes every key until it ends.
            let (c, ev) = capture_step(self.capture, get_last_key_pressed());
            self.capture = c;
            if let CaptureEvent::Bound(a, k) = ev {
                let mut s = settings::get();
                s.bindings.rebind(a, k);
                settings::set(s);
            }
        } else {
            act = self.handle_input(panel, mx, my, click, &skin_ids);
        }

        // Previews: the caller's textures first, else the pack's skin file or the default mannequin atlas, loaded once.
        let lookup = |id: &str| skins(id).or_else(|| preview_texture(id, pack));
        self.draw(panel, mx, my, &skin_ids, &lookup);
        act
    }

    fn tab_rect(panel: Rect, i: usize) -> Rect {
        Rect::new(panel.x + 20.0 + i as f32 * 150.0, panel.y + 16.0, 140.0, 34.0)
    }

    fn back_rect(panel: Rect) -> Rect {
        Rect::new(panel.x + panel.w - 160.0, panel.y + panel.h - 54.0, 140.0, 38.0)
    }

    fn content_top(panel: Rect) -> f32 {
        panel.y + 70.0
    }

    fn row_rect(panel: Rect, i: usize, width: f32) -> Rect {
        Rect::new(panel.x + 20.0, Self::content_top(panel) + i as f32 * (ROW_H + 4.0), width, ROW_H)
    }

    fn controls_width(panel: Rect) -> f32 {
        if panel.w >= 640.0 {
            panel.w * 0.5 - 30.0
        } else {
            panel.w - 40.0
        }
    }

    fn slider_track(row: Rect) -> (f32, f32) {
        (row.x + row.w * 0.5, row.w * 0.35)
    }

    fn skin_cols(panel: Rect) -> usize {
        (((panel.w - 40.0) / 124.0).floor() as usize).max(1)
    }

    /// The first skin row shown: the grid scrolls so the cursor's row is always inside the space above the Back button.
    fn skin_first_row(panel: Rect, cursor: usize) -> usize {
        let cols = Self::skin_cols(panel);
        let row_h = 110.0 + 26.0 + 14.0;
        let visible = (((Self::back_rect(panel).y - 10.0 - Self::content_top(panel)) / row_h).floor() as usize).max(1);
        let row = cursor / cols;
        row.saturating_sub(visible - 1)
    }

    /// Whether a skin cell is inside the visible grid area (cells scrolled away are neither drawn nor clickable).
    fn skin_visible(panel: Rect, r: Rect) -> bool {
        r.y >= Self::content_top(panel) - 1.0 && r.y + r.h <= Self::back_rect(panel).y - 4.0
    }

    fn skin_cell(panel: Rect, i: usize, first_row: usize) -> Rect {
        let cols = Self::skin_cols(panel);
        let cell = 110.0;
        let gap = 14.0;
        let (cx, cy) = ((i % cols) as f32, (i / cols) as f32 - first_row as f32);
        Rect::new(panel.x + 20.0 + cx * (cell + gap), Self::content_top(panel) + cy * (cell + 26.0 + gap), cell, cell + 26.0)
    }

    fn handle_input(&mut self, panel: Rect, mx: f32, my: f32, click: bool, skin_ids: &[String]) -> SettingsAct {
        let mouse = vec2(mx, my);
        if is_key_pressed(KeyCode::Escape) {
            return SettingsAct::Close;
        }
        if click && Self::back_rect(panel).contains(mouse) {
            return SettingsAct::Close;
        }
        let mut s = settings::get();

        // Tabs.
        let mut dir = 0;
        if is_key_pressed(KeyCode::PageDown) {
            dir = 1;
        }
        if is_key_pressed(KeyCode::PageUp) {
            dir = -1;
        }
        if click {
            for (i, t) in TABS.iter().enumerate() {
                if Self::tab_rect(panel, i).contains(mouse) && *t != self.tab {
                    self.tab = *t;
                    self.row = 0;
                    self.dragging = None;
                }
            }
        }
        if dir != 0 {
            self.tab = cycle_tab(self.tab, dir);
            self.row = 0;
            self.dragging = None;
        }

        let up = is_key_pressed(KeyCode::Up);
        let down = is_key_pressed(KeyCode::Down);
        let left = is_key_pressed(KeyCode::Left);
        let right = is_key_pressed(KeyCode::Right);
        let enter = is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter);
        if !is_mouse_button_down(MouseButton::Left) {
            // A drag saves once, when it ends (not every frame while held).
            if self.dragging.is_some() {
                settings::flush();
            }
            self.dragging = None;
        }

        match self.tab {
            Tab::Controls => {
                let n = Action::ALL.len() + 1;
                if up {
                    self.row = (self.row + n - 1) % n;
                }
                if down {
                    self.row = (self.row + 1) % n;
                }
                // Left/Right on the tab row: with the cursor on the first row they also change tabs.
                let w = Self::controls_width(panel);
                let mut activate = enter.then_some(self.row);
                if click {
                    for i in 0..n {
                        if Self::row_rect(panel, i, w).contains(mouse) {
                            self.row = i;
                            activate = Some(i);
                        }
                    }
                }
                if let Some(i) = activate {
                    if i < Action::ALL.len() {
                        self.capture = Capture::Waiting(Action::ALL[i]);
                    } else {
                        s.bindings = Default::default();
                        settings::set(s);
                    }
                }
                if left || right {
                    self.tab = cycle_tab(self.tab, if right { 1 } else { -1 });
                    self.row = 0;
                }
            }
            Tab::Skins => {
                let cols = Self::skin_cols(panel);
                let n = skin_ids.len();
                self.skin_cursor = self.skin_cursor.min(n.saturating_sub(1));
                if left {
                    self.skin_cursor = grid_move(self.skin_cursor, -1, 0, cols, n);
                }
                if right {
                    self.skin_cursor = grid_move(self.skin_cursor, 1, 0, cols, n);
                }
                if up {
                    self.skin_cursor = grid_move(self.skin_cursor, 0, -1, cols, n);
                }
                if down {
                    self.skin_cursor = grid_move(self.skin_cursor, 0, 1, cols, n);
                }
                let mut choose = enter.then_some(self.skin_cursor);
                if click {
                    for i in 0..n {
                        let r = Self::skin_cell(panel, i, Self::skin_first_row(panel, self.skin_cursor));
                        if Self::skin_visible(panel, r) && r.contains(mouse) {
                            self.skin_cursor = i;
                            choose = Some(i);
                        }
                    }
                }
                if let Some(id) = choose.and_then(|i| skin_ids.get(i)) {
                    s.player_skin = id.clone();
                    settings::set(s);
                }
            }
            Tab::Settings => {
                let n = SETTING_ROWS.len();
                if up {
                    self.row = (self.row + n - 1) % n;
                }
                if down {
                    self.row = (self.row + 1) % n;
                }
                let w = panel.w - 40.0;
                let mut changed = false;
                if click {
                    for i in 0..n {
                        if Self::row_rect(panel, i, w).contains(mouse) {
                            self.row = i;
                            let row = SETTING_ROWS[i];
                            if is_slider(row) {
                                self.dragging = Some(row);
                            } else {
                                toggle(&mut s, row);
                                changed = true;
                            }
                        }
                    }
                }
                let row = SETTING_ROWS[self.row.min(n - 1)];
                if is_slider(row) {
                    let d = right as i32 - left as i32;
                    if d != 0 {
                        step_slider(&mut s, row, d);
                        changed = true;
                    }
                } else if enter || left || right {
                    toggle(&mut s, row);
                    changed = true;
                }
                if let Some(dr) = self.dragging {
                    let i = SETTING_ROWS.iter().position(|r| *r == dr).unwrap_or(0);
                    let (tl, tw) = Self::slider_track(Self::row_rect(panel, i, w));
                    let (min, max) = slider_range(dr);
                    set_slider(&mut s, dr, slider_value_at(mx, tl, tw, min, max));
                    if !changed {
                        settings::set_cached(s.clone());
                    }
                }
                if changed {
                    settings::set(s);
                }
            }
        }
        SettingsAct::Stay
    }

    fn draw(&self, panel: Rect, mx: f32, my: f32, skin_ids: &[String], skins: &dyn Fn(&str) -> Option<Texture2D>) {
        let (sw, sh) = (screen_width(), screen_height());
        let mouse = vec2(mx, my);
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.72));
        ui::panel(panel, ui::PANEL, None);
        let s = settings::get();

        for (i, t) in TABS.iter().enumerate() {
            let r = Self::tab_rect(panel, i);
            let on = *t == self.tab;
            let hot = r.contains(mouse);
            draw_rectangle(r.x, r.y, r.w, r.h, if on { ui::PANEL_LIT } else { Color::new(0.0, 0.0, 0.0, 0.3) });
            if on {
                draw_rectangle(r.x, r.y + r.h - 3.0, r.w, 3.0, ui::ACCENT);
            }
            let col = if on {
                ui::ACCENT
            } else if hot {
                ui::TEXT
            } else {
                ui::MUTED
            };
            ui::text_centered(tab_label(*t), r.x + r.w / 2.0, r.y + 23.0, 20.0, col);
        }

        match self.tab {
            Tab::Controls => self.draw_controls(panel, &s),
            Tab::Skins => self.draw_skins(panel, &s, skin_ids, skins),
            Tab::Settings => self.draw_settings(panel, &s),
        }

        let b = Self::back_rect(panel);
        let hot = b.contains(mouse);
        ui::button(b, "Back", "", hot, true, if hot { 1.0 } else { 0.0 }, ui::ACCENT);
    }

    fn draw_controls(&self, panel: Rect, s: &Settings) {
        let w = Self::controls_width(panel);
        for (i, a) in Action::ALL.iter().enumerate() {
            let r = Self::row_rect(panel, i, w);
            let sel = self.row == i;
            draw_rectangle(r.x, r.y, r.w, r.h, if sel { ui::PANEL_LIT } else { Color::new(0.0, 0.0, 0.0, 0.25) });
            ui::text(a.label(), r.x + 12.0, r.y + 21.0, 18.0, if sel { ui::ACCENT } else { ui::TEXT });
            let waiting = self.capture == Capture::Waiting(*a);
            let key = if waiting { "press a key...".to_string() } else { input::key_label(s.bindings.key(*a)).to_string() };
            let col = if waiting { ui::ACCENT } else { ui::MUTED };
            ui::text(&key, r.x + r.w - 12.0 - ui::measure(&key, 18.0), r.y + 21.0, 18.0, col);
        }
        let ri = Action::ALL.len();
        let r = Self::row_rect(panel, ri, w);
        let sel = self.row == ri;
        draw_rectangle(r.x, r.y, r.w, r.h, if sel { ui::PANEL_LIT } else { Color::new(0.0, 0.0, 0.0, 0.25) });
        ui::text("Reset to defaults", r.x + 12.0, r.y + 21.0, 18.0, if sel { ui::ACCENT } else { ui::BAD });

        if panel.w >= 640.0 {
            let x = panel.x + panel.w * 0.5 + 10.0;
            let mut y = Self::content_top(panel) + 20.0;
            ui::text("Fixed keys", x, y, 20.0, ui::ACCENT);
            for (k, l) in FIXED_KEYS {
                y += 28.0;
                ui::text(k, x, y, 17.0, ui::TEXT);
                ui::text(l, x + 150.0, y, 17.0, ui::MUTED);
            }
        }
    }

    fn draw_skins(&self, panel: Rect, s: &Settings, ids: &[String], skins: &dyn Fn(&str) -> Option<Texture2D>) {
        let first = Self::skin_first_row(panel, self.skin_cursor);
        for (i, id) in ids.iter().enumerate() {
            let r = Self::skin_cell(panel, i, first);
            if !Self::skin_visible(panel, r) {
                continue;
            }
            let chosen = *id == s.player_skin;
            let cursor = i == self.skin_cursor;
            draw_rectangle(r.x, r.y, r.w, r.h, if cursor { ui::PANEL_LIT } else { Color::new(0.0, 0.0, 0.0, 0.3) });
            let img = Rect::new(r.x + 6.0, r.y + 6.0, r.w - 12.0, r.w - 12.0);
            match skins(id) {
                Some(tex) => {
                    let (tw, th) = (tex.width().max(1.0), tex.height().max(1.0));
                    let k = (img.w / tw).min(img.h / th);
                    let (dw, dh) = (tw * k, th * k);
                    draw_texture_ex(
                        &tex,
                        img.x + (img.w - dw) / 2.0,
                        img.y + (img.h - dh) / 2.0,
                        WHITE,
                        DrawTextureParams { dest_size: Some(vec2(dw, dh)), ..Default::default() },
                    );
                }
                None => {
                    // No atlas to show: a plain figure silhouette.
                    let c = vec2(img.x + img.w / 2.0, img.y + img.h / 2.0);
                    draw_circle(c.x, c.y - img.h * 0.28, img.w * 0.12, ui::DIM);
                    draw_rectangle(c.x - img.w * 0.14, c.y - img.h * 0.14, img.w * 0.28, img.h * 0.42, ui::DIM);
                }
            }
            if chosen {
                draw_rectangle_lines(r.x, r.y, r.w, r.h, 3.0, ui::ACCENT);
            } else if cursor {
                draw_rectangle_lines(r.x, r.y, r.w, r.h, 1.0, ui::MUTED);
            }
            let name = if id == "mannequin" { "Mannequin" } else { id.as_str() };
            let name = ui::wrap(name, r.w - 8.0, 15.0).into_iter().next().unwrap_or_default();
            ui::text_centered(&name, r.x + r.w / 2.0, r.y + r.h - 6.0, 15.0, if chosen { ui::ACCENT } else { ui::TEXT });
        }
    }

    fn draw_settings(&self, panel: Rect, s: &Settings) {
        let w = panel.w - 40.0;
        for (i, row) in SETTING_ROWS.iter().enumerate() {
            let r = Self::row_rect(panel, i, w);
            let sel = self.row == i;
            draw_rectangle(r.x, r.y, r.w, r.h, if sel { ui::PANEL_LIT } else { Color::new(0.0, 0.0, 0.0, 0.25) });
            ui::text(row_label(*row), r.x + 12.0, r.y + 21.0, 18.0, if sel { ui::ACCENT } else { ui::TEXT });
            if is_slider(*row) {
                let (tl, tw) = Self::slider_track(r);
                let (min, max) = slider_range(*row);
                let v = slider_get(s, *row);
                ui::bar(Rect::new(tl, r.y + 11.0, tw, 8.0), slider_frac(v, min, max), ui::ACCENT);
                ui::text(&slider_text(*row, v), tl + tw + 12.0, r.y + 21.0, 18.0, ui::MUTED);
            } else {
                let on = toggle_get(s, *row);
                let t = if on { "ON" } else { "OFF" };
                ui::text(t, r.x + r.w - 12.0 - ui::measure(t, 18.0), r.y + 21.0, 18.0, if on { ui::GOOD } else { ui::DIM });
                if *row == SettingRow::FlatTextures {
                    ui::text("applies on next story start", r.x + 230.0, r.y + 21.0, 14.0, ui::DIM);
                }
            }
        }
    }
}

/// The keys that cannot be rebound, shown read-only on the Controls tab.
const FIXED_KEYS: [(&str, &str); 9] = [
    ("Esc", "Pause"),
    ("1-9", "Hotbar slots"),
    ("Mouse", "Look"),
    ("Left click", "Attack"),
    ("Right click", "Block"),
    ("Wheel", "Change slot"),
    ("Arrows", "Move"),
    ("Tab", "Inventory"),
    ("Enter", "Interact"),
];

thread_local! {
    static PREVIEWS: std::cell::RefCell<std::collections::HashMap<String, Option<Texture2D>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// The atlas of skin `id` for its preview: a pack skin's PNG, or the default mannequin atlas for "mannequin". Read once
/// through the asset store (so the browser build has it too) and kept.
fn preview_texture(id: &str, pack: &Pack) -> Option<Texture2D> {
    if let Some(found) = PREVIEWS.with(|p| p.borrow().get(id).cloned()) {
        return found;
    }
    let path = if id == "mannequin" {
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/lr_story/assets/player/player_mannequin_atlas.png"))
    } else {
        pack.skins.get(id).cloned()
    };
    let tex = path
        .and_then(|p| crate::assets_vfs::read(&p).ok())
        .and_then(|bytes| Image::from_file_with_format(&bytes, Some(ImageFormat::Png)).ok())
        .map(|img| {
            let t = Texture2D::from_image(&img);
            t.set_filter(FilterMode::Nearest);
            t
        });
    PREVIEWS.with(|p| p.borrow_mut().insert(id.to_string(), tex.clone()));
    tex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_value_follows_the_mouse_and_clamps() {
        assert_eq!(slider_value_at(100.0, 100.0, 200.0, 0.0, 10.0), 0.0);
        assert_eq!(slider_value_at(200.0, 100.0, 200.0, 0.0, 10.0), 5.0);
        assert_eq!(slider_value_at(900.0, 100.0, 200.0, 0.0, 10.0), 10.0);
        assert_eq!(slider_value_at(-50.0, 100.0, 200.0, 2.0, 10.0), 2.0);
        assert_eq!(slider_value_at(150.0, 100.0, 0.0, 2.0, 10.0), 2.0);
    }

    #[test]
    fn slider_step_moves_a_twentieth_and_stops_at_the_ends() {
        assert!((slider_step(60.0, 60.0, 100.0, 1) - 62.0).abs() < 1e-4);
        assert_eq!(slider_step(60.0, 60.0, 100.0, -1), 60.0);
        assert_eq!(slider_step(100.0, 60.0, 100.0, 1), 100.0);
        assert_eq!(slider_frac(80.0, 60.0, 100.0), 0.5);
    }

    #[test]
    fn tabs_cycle_both_ways_and_wrap() {
        assert_eq!(cycle_tab(Tab::Controls, 1), Tab::Skins);
        assert_eq!(cycle_tab(Tab::Settings, 1), Tab::Controls);
        assert_eq!(cycle_tab(Tab::Controls, -1), Tab::Settings);
    }

    #[test]
    fn capture_goes_idle_to_bound_or_cancelled() {
        let w = Capture::Waiting(Action::Jump);
        assert_eq!(capture_step(w, None), (w, CaptureEvent::None));
        assert_eq!(capture_step(w, Some(KeyCode::G)), (Capture::Idle, CaptureEvent::Bound(Action::Jump, KeyCode::G)));
        assert_eq!(capture_step(w, Some(KeyCode::Escape)), (Capture::Idle, CaptureEvent::Cancelled));
        assert_eq!(capture_step(Capture::Idle, Some(KeyCode::G)), (Capture::Idle, CaptureEvent::None));
    }

    #[test]
    fn skin_list_has_the_mannequin_first_then_sorted_pack_skins() {
        let v = skin_list(["zed", "amy", "mannequin", "amy"].into_iter());
        assert_eq!(v, vec!["mannequin", "amy", "zed"]);
        assert_eq!(skin_list(std::iter::empty()), vec!["mannequin"]);
    }

    #[test]
    fn grid_moves_stay_inside_the_grid() {
        // 3 columns, 5 cells: rows are [0 1 2] [3 4].
        assert_eq!(grid_move(0, 1, 0, 3, 5), 1);
        assert_eq!(grid_move(2, 1, 0, 3, 5), 2);
        assert_eq!(grid_move(1, 0, 1, 3, 5), 4);
        assert_eq!(grid_move(2, 0, 1, 3, 5), 2);
        assert_eq!(grid_move(0, 0, -1, 3, 5), 0);
        assert_eq!(grid_move(0, 0, 0, 3, 0), 0);
    }
}
