//! Night Watch shell: title, shift board, custom night, account stub, settings, credits and the controls screen,
//! drawn over the classic office (`Backdrop`). Port of `night_watch_menu.gd`. Everything is immediate mode: one call,
//! `Shell::update_and_draw`, reads the frame's input, draws the current screen and answers with a `ShellAction`.
//! Navigation, scoring and layout rules live in `modes_data` (pure, tested); screens are `menu_*.rs`.
#![allow(dead_code)]

use crate::audio::cue_names::menu as menu_cues;
use crate::core::{Bindings, Cue, InputFrame, ModesData, Mulberry32, Save, Session};
use crate::gfx::art::Textures;
use crate::gfx::camera::FpCamera;
use crate::gfx::office::{Office, OfficeConfig, OfficeVariant};
use crate::gfx::shader::LightSet;
use crate::gfx::Gfx;
use crate::hunters::{Actor, HunterLib, PoseState};
use crate::ui::*;
use macroquad::prelude::*;
use std::collections::BTreeMap;

mod menu_board;
mod menu_custom;
mod menu_misc;
mod menu_title;
pub mod modes_data;

pub use modes_data::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Title,
    Board,
    Custom,
    Account,
    Settings,
    Credits,
    Controls,
}

#[derive(Clone)]
pub enum ShellAction {
    None,
    StartShift { mode: String, level: i32, custom: BTreeMap<String, i32> },
    Quit,
    PlayCue(Vec<Cue>),
}

/// Keys the menus read, one snapshot per frame. `Keys::default()` is "nothing pressed" (tests).
#[derive(Clone, Default)]
pub struct Keys {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub enter: bool,
    pub esc: bool,
    /// Mouse wheel direction: -1, 0 or 1 (up is positive).
    pub wheel: f32,
    /// The last key that went down this frame (rebind capture).
    pub last_key: Option<KeyCode>,
}

impl Keys {
    pub fn read() -> Keys {
        let w = mouse_wheel().1;
        Keys {
            up: is_key_pressed(KeyCode::Up),
            down: is_key_pressed(KeyCode::Down),
            left: is_key_pressed(KeyCode::Left),
            right: is_key_pressed(KeyCode::Right),
            enter: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) || is_key_pressed(KeyCode::Space),
            esc: is_key_pressed(KeyCode::Escape),
            wheel: if w > 0.0 {
                1.0
            } else if w < 0.0 {
                -1.0
            } else {
                0.0
            },
            last_key: get_last_key_pressed(),
        }
    }
}

/// Per-frame context handed to the screens.
pub(self) struct Ctx<'a> {
    pub input: &'a InputFrame,
    pub keys: &'a Keys,
    pub ui: Ui,
    pub save: &'a mut Save,
    pub bindings: &'a mut Bindings,
    pub modes: &'a ModesData,
    pub cues: Vec<Cue>,
}

impl<'a> Ctx<'a> {
    /// Queue a menu blip: "move", "pick" or "deny".
    pub fn cue(&mut self, kind: &str) {
        self.cues.extend(menu_cues(kind));
    }
}

struct Portrait {
    id: String,
    rt: RenderTarget,
    actor: Option<Actor>,
}

pub struct Shell {
    pub screen: Screen,
    title_index: usize,
    board_mode: usize,
    board_level: usize,
    info_scroll: f32,
    custom_mode: String,
    custom_loaded: String,
    custom_vals: BTreeMap<String, i32>,
    custom_dirty: bool,
    settings_dirty: bool,
    toast: String,
    clock_t: f32,
    erase_armed: bool,
    erase_done: bool,
    ctl_sel: usize,
    ctl_scroll: usize,
    ctl_capture: bool,
    rng: Mulberry32,
    portrait: Option<Portrait>,
    /// ACCOUNT screen: the account client, the form and its status line.
    acct: crate::net::account::AccountClient,
    acct_init: bool,
    acct_user: String,
    acct_pass: String,
    acct_focus: u8,
    acct_status: String,
}

/// The key hint line under each screen.
pub fn hint_for(screen: Screen) -> &'static str {
    match screen {
        Screen::Title => "↑ ↓ select · Enter confirm",
        Screen::Board => "↑ ↓ mode · ← → shift · Enter start · Esc back",
        Screen::Controls => "Click a row (or Enter), then press a key · Esc cancels · wheel scrolls",
        Screen::Account => "Tab next field · Enter confirm · Esc back",
        _ => "Esc back",
    }
}

impl Shell {
    pub fn new(open_on: Screen, session: &Session, modes: &ModesData) -> Shell {
        let mut board_mode = 0;
        let mut board_level = 0;
        if open_on == Screen::Board {
            if let Some(i) = modes.modes.iter().position(|m| m.id == session.mode_id) {
                board_mode = i;
                let n = modes.modes[i].levels.len().max(1);
                board_level = ((session.night - 1).max(0) as usize).min(n - 1);
            }
        }
        Shell {
            screen: open_on,
            title_index: 0,
            board_mode,
            board_level,
            info_scroll: 0.0,
            custom_mode: session.mode_id.clone(),
            custom_loaded: String::new(),
            custom_vals: BTreeMap::new(),
            custom_dirty: false,
            settings_dirty: false,
            toast: String::new(),
            clock_t: 0.0,
            erase_armed: false,
            erase_done: false,
            ctl_sel: 0,
            ctl_scroll: 0,
            ctl_capture: false,
            rng: Mulberry32::new(Session::fresh_seed()),
            portrait: None,
            acct: crate::net::account::AccountClient::new(),
            acct_init: false,
            acct_user: String::new(),
            acct_pass: String::new(),
            acct_focus: 0,
            acct_status: String::new(),
        }
    }

    /// Switch screen (Godot `show_screen`): the toast only survives on the title.
    pub(self) fn go(&mut self, s: Screen) {
        self.screen = s;
        self.erase_armed = false;
        self.erase_done = false;
        self.ctl_capture = false;
        if s != Screen::Title {
            self.toast.clear();
        }
        if s == Screen::Board {
            self.info_scroll = 0.0;
        }
    }

    /// Esc / BACK: the title quits, Controls returns to Settings, everything else returns to the title.
    pub(self) fn back(&mut self) -> ShellAction {
        match self.screen {
            Screen::Title => ShellAction::Quit,
            Screen::Controls => {
                self.go(Screen::Settings);
                ShellAction::None
            }
            _ => {
                self.go(Screen::Title);
                ShellAction::None
            }
        }
    }

    /// `← BACK  ESC` button and the screen title. Returns Quit/None.
    pub(self) fn header(&mut self, c: &mut Ctx<'_>, title: &str, extra: &str) -> ShellAction {
        let res = c.ui.button(Rect::new(8.0, 6.0, 58.0, 14.0), "← BACK  ESC", 6.0, ButtonStyle::Normal, true);
        text(title, 74.0, 7.0, 9.0, INK);
        if !extra.is_empty() {
            text(extra, 74.0 + measure(title, 9.0) + 10.0, 9.0, 7.0, AMBER);
        }
        if res.clicked {
            c.cue("pick");
            return self.back();
        }
        ShellAction::None
    }

    fn clock_text(&self) -> String {
        if self.screen == Screen::Board {
            crate::core::clock::local_hhmm()
        } else {
            title_clock(self.clock_t)
        }
    }

    fn draw_chrome(&self) {
        text_right(&self.clock_text(), 474.0, 6.0, 9.0, INK);
        if !self.toast.is_empty() {
            text_center(&self.toast, 240.0, 245.0, 7.0, AMBER);
        }
        text_center(hint_for(self.screen), 240.0, 257.0, 6.0, BODY);
    }

    /// One frame of the shell: logic, then drawing, over the backdrop. The caller applies the returned action.
    pub fn update_and_draw(
        &mut self,
        input: &InputFrame,
        save: &mut Save,
        bindings: &mut Bindings,
        modes: &ModesData,
        backdrop: &mut Backdrop,
        gfx: &Gfx,
        hunters: &mut HunterLib,
    ) -> ShellAction {
        self.clock_t += input.dt;
        self.account_tick(save);
        let mut keys = Keys::read();
        let capturing = self.screen == Screen::Controls && self.ctl_capture;
        if keys.esc {
            keys.esc = false;
            if capturing {
                self.ctl_capture = false;
                keys.last_key = None;
            } else if let ShellAction::Quit = self.back() {
                return ShellAction::Quit;
            }
        }
        if self.screen == Screen::Board {
            self.portrait_pass(modes, gfx, hunters, input.dt);
        }
        backdrop.draw(gfx);

        let mut c = Ctx { input, keys: &keys, ui: Ui::begin(input), save, bindings, modes, cues: Vec::new() };
        let act = match self.screen {
            Screen::Title => self.title(&mut c),
            Screen::Board => self.board(&mut c),
            Screen::Custom => self.custom(&mut c),
            Screen::Account => self.account(&mut c),
            Screen::Settings => self.settings(&mut c),
            Screen::Credits => self.credits(&mut c),
            Screen::Controls => self.controls(&mut c),
        };
        self.draw_chrome();
        if c.ui.hover_changed() {
            c.cue("move");
        }
        let cues = std::mem::take(&mut c.cues);
        drop(c);
        match act {
            ShellAction::None if !cues.is_empty() => ShellAction::PlayCue(cues),
            other => other,
        }
    }

    /// Render the selected mode's first hunter into the portrait target (before any 2D of this frame is queued).
    fn portrait_pass(&mut self, modes: &ModesData, gfx: &Gfx, hunters: &mut HunterLib, dt: f32) {
        let Some(m) = modes.modes.get(self.board_mode) else { return };
        let id = m.art.first().cloned().unwrap_or_default();
        let stale = self.portrait.as_ref().map(|p| p.id != id).unwrap_or(true);
        if stale {
            let rt = match self.portrait.take() {
                Some(p) => p.rt,
                None => {
                    let rt = render_target_ex(192, 108, RenderTargetParams { sample_count: 1, depth: true });
                    rt.texture.set_filter(FilterMode::Nearest);
                    rt
                }
            };
            let mut actor = if id.is_empty() { None } else { hunters.actor(&id) };
            if let Some(a) = actor.as_mut() {
                a.pos = vec3(0.0, 0.0, 0.0);
                a.visible = true;
                a.set_pose_state(PoseState::Rig);
                a.set_locomotion(false, false);
            }
            self.portrait = Some(Portrait { id, rt, actor });
        }
        let Some(p) = self.portrait.as_mut() else { return };
        let cam_pos = vec3(0.0, 2.2, -4.2);
        if let Some(a) = p.actor.as_mut() {
            a.yaw = crate::hunters::yaw_toward(a.pos, cam_pos);
            a.animate(dt);
        }
        set_camera(&Camera3D {
            position: cam_pos,
            target: vec3(0.0, 1.8, 0.0),
            up: vec3(0.0, 1.0, 0.0),
            fovy: 28f32.to_radians(),
            aspect: Some(16.0 / 9.0),
            render_target: Some(p.rt.clone()),
            z_near: 0.05,
            z_far: 40.0,
            ..Default::default()
        });
        clear_background(Color::new(0.05, 0.07, 0.09, 1.0));
        if let Some(a) = p.actor.as_ref() {
            let lights = LightSet { ambient: [0.8, 0.8, 0.9], omni: Vec::new(), spot: Vec::new(), fog: None };
            a.draw(gfx, &lights, cam_pos);
        }
        set_default_camera();
    }

    /// The board portrait: the 3D hunter render in a frame with a REC mark, or a text card when there is no actor.
    pub(self) fn draw_portrait(&self, r: Rect, fallback: &str) {
        let shown = self.portrait.as_ref().filter(|p| p.actor.is_some());
        match shown {
            Some(p) => {
                let s = to_screen_rect(r);
                draw_texture_ex(
                    &p.rt.texture,
                    s.x,
                    s.y,
                    WHITE,
                    DrawTextureParams { dest_size: Some(vec2(s.w, s.h)), flip_y: true, ..Default::default() },
                );
            }
            None => {
                fill_rect(r, Color::new(0.05, 0.07, 0.09, 1.0));
                text_center(&fallback.to_uppercase(), r.x + r.w * 0.5, r.y + r.h * 0.5 - 6.0, 8.0, INK);
                text_center("NO SIGNAL", r.x + r.w * 0.5, r.y + r.h * 0.5 + 6.0, 5.0, DIM);
            }
        }
        stroke_rect(r, PANEL_EDGE, 1.0);
        text("● REC", r.x + 4.0, r.y + 3.0, 5.0, RED_TEXT);
    }
}

/// The classic office behind every menu: camera fov 62 at (0, 1.6, -0.25), pitch -0.16, and a dark tint.
pub struct Backdrop {
    office: Office,
}

impl Backdrop {
    pub fn new(tex: &Textures) -> Backdrop {
        let cfg = OfficeConfig {
            variant: OfficeVariant::Classic,
            extra: Vec::new(),
            drop_hall_end_walls: false,
            palette: true,
            signature: true,
        };
        let mut office = Office::build(cfg, tex);
        office.snap_slabs([false, false]);
        Backdrop { office }
    }

    pub fn draw(&self, gfx: &Gfx) {
        clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
        let cam = FpCamera { pos: vec3(0.0, 1.6, -0.25), yaw: 0.0, pitch: -0.16, fov_deg: 62.0, near: 0.06, far: 100.0 };
        cam.apply();
        self.office.draw(gfx, cam.pos, None);
        set_default_camera();
        draw_rectangle(0.0, 0.0, screen_width(), screen_height(), TINT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_name_the_keys() {
        assert!(hint_for(Screen::Title).contains("Enter"));
        assert!(hint_for(Screen::Board).contains("Esc back"));
        assert_eq!(hint_for(Screen::Settings), "Esc back");
        assert_eq!(hint_for(Screen::Credits), "Esc back");
    }

    #[test]
    fn no_keys_by_default() {
        let k = Keys::default();
        assert!(!k.up && !k.down && !k.left && !k.right && !k.enter && !k.esc);
        assert_eq!(k.wheel, 0.0);
        assert!(k.last_key.is_none());
    }
}
