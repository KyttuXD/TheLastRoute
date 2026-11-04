//! A Classic night on screen: the shell's driver for `modes::classic`. `ClassicRun` owns the night, maps the
//! frame's `Controls` (doors A/D, lights Q/E held, monitor Space, cameras 1-8) onto the mode's `Input`, plays the
//! events it returns (power out, a hunter pushed back, caught, win) and saves a cleared night. `draw` paints the
//! office, or the monitor with the camera feed beside the full building map, plus the HUD and the call box.
//! Hunters are the web build's cut-outs (`sprites::draw_hunter`). Timings and labels come from the `ui` block of
//! `data/rules/classic.json`. Everything but `poll_controls` and `draw` is plain state, so it is tested without a window.

use macroquad::prelude::*;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;

use crate::building_map::{self, BuildingMap, MapView};
use crate::calls_view::CallView;
use crate::data::{self, CallsData, CALL_INCOMING_TEXT};
use crate::framework::CallEvent;
use crate::nw_sound;
use crate::modes::classic::{self, ClassicNight, Event, Input, Meet, Outcome};
use crate::save::{NightWatchSave, CLASSIC, NIGHTS};
use crate::sprites::{self, Facing};

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);

/// The mode whose rules file holds this driver's `ui` block.
const MODE: &str = "classic";

/// A timing from the `ui` block of the rules file (`max_step_ms`, `note_ms`, ...).
fn ui_num(key: &str) -> f64 {
    data::num(data::rules(MODE), &format!("ui.{key}"))
}

/// A label from `ui.labels` in the rules file. Placeholders such as `{name}` are filled by `fill`.
fn label(key: &str) -> &'static str {
    data::text(data::rules(MODE), &format!("ui.labels.{key}"))
}

/// `template` with each `{key}` replaced by its value.
fn fill(template: &str, pairs: &[(&str, String)]) -> String {
    pairs.iter().fold(template.to_owned(), |text, (key, value)| text.replace(&format!("{{{key}}}"), value))
}

/// The five stalkers from `ui.stalkers`: each entry has an id, the caller-box name and the silhouette tint.
fn stalkers() -> &'static [Value] {
    data::list(data::rules(MODE), "ui.stalkers")
}

fn stalker(id: &str) -> Option<&'static Value> {
    stalkers().iter().find(|s| s["id"].as_str() == Some(id))
}

fn stalker_ids() -> Vec<&'static str> {
    stalkers().iter().filter_map(|s| s["id"].as_str()).collect()
}

fn hunter_name(id: &str) -> &'static str {
    stalker(id).and_then(|s| s["name"].as_str()).unwrap_or_else(|| label("someone"))
}

/// The silhouette colour for the fallback figure, from the stalker's tint.
fn hunter_color(id: &str) -> Color {
    let Some(tint) = stalker(id).and_then(|s| s["tint"].as_array()) else { return WHITE };
    let c = |i: usize| tint.get(i).and_then(Value::as_f64).unwrap_or(1.0) as f32;
    Color::new(c(0), c(1), c(2), 1.0)
}

/// What the player did this frame. The door and monitor fields are presses (they toggle); the lights are held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub door_l: bool,
    pub door_r: bool,
    pub light_l: bool,
    pub light_r: bool,
    pub toggle_cam: bool,
    pub pick_cam: Option<u32>,
    /// -1 or 1 steps the monitor to the previous or next camera.
    pub cam_step: i32,
    pub skip_call: bool,
    pub mute_call: bool,
    /// Meet Kathy's eyes on the monitor (Night 7).
    pub meet: bool,
    pub confirm: bool,
    pub back: bool,
}

/// Where the shell goes after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Stay,
    Hub,
    /// Start this Classic night (the next one after a win, the same one to retry).
    Night(u32),
}

#[derive(Clone, Copy, Debug)]
struct End {
    outcome: Outcome,
    darkness: bool,
}

pub struct ClassicRun {
    night: ClassicNight,
    map: &'static BuildingMap,
    persist: bool,
    shut: [bool; 2],
    lights: [bool; 2],
    cam_up: bool,
    cam: u32,
    end: Option<End>,
    end_ms: f64,
    flash_ms: f64,
    notes: Vec<(String, f64)>,
    view: CallView,
    anim_ms: f64,
}

impl ClassicRun {
    /// Opens Classic night `night`. `persist` lets a won night write the save file (off for the screenshot runs).
    pub fn new(calls: &CallsData, night: u32, seed: i64, persist: bool) -> Self {
        Self::with_night(classic::start(calls, night, seed, &[], false), persist)
    }

    /// Opens the Custom Night: each slider is that hunter's AI level (`levels`, by id). Nothing is saved from it.
    pub fn new_custom(calls: &CallsData, night: u32, levels: &[(&str, i32)], seed: i64) -> Self {
        Self::with_night(classic::start(calls, night, seed, levels, true), false)
    }

    fn with_night(night: ClassicNight, persist: bool) -> Self {
        let map = building_map::load(CLASSIC).expect("the classic building map is embedded");
        let cam = map.cameras.first().map_or(1, |c| c.id);
        let mut view = CallView::new();
        view.player.enabled = persist;
        Self {
            night,
            map,
            persist,
            shut: [false; 2],
            lights: [false; 2],
            cam_up: false,
            cam,
            end: None,
            end_ms: 0.0,
            flash_ms: 0.0,
            notes: Vec::new(),
            view,
            anim_ms: 0.0,
        }
    }

    fn note(&mut self, text: impl Into<String>) {
        self.notes.push((text.into(), ui_num("note_ms")));
        if self.notes.len() > 3 {
            self.notes.remove(0);
        }
    }

    /// The room the active camera covers.
    fn cam_room(&self) -> &'static str {
        let map: &'static BuildingMap = self.map;
        map.camera(self.cam)
            .and_then(|c| c.covers.first())
            .map_or("stage", |&i| map.rooms[i].id.as_str())
    }

    fn cam_ids(&self) -> Vec<u32> {
        self.map.cameras.iter().map(|c| c.id).collect()
    }

    /// Every hunter on the map as (room id, hunter id), from the mode's own positions.
    fn markers(&self) -> Vec<(&'static str, &'static str)> {
        stalker_ids().into_iter().filter_map(|id| self.night.hunter_room(id).map(|room| (room, id))).collect()
    }

    fn hunters_in(&self, room: &str) -> Vec<&'static str> {
        stalker_ids().into_iter().filter(|id| self.night.hunter_room(id) == Some(room)).collect()
    }

    pub fn update(&mut self, dt_ms: f64, c: &Controls) -> Flow {
        self.anim_ms += dt_ms;
        self.flash_ms = (self.flash_ms - dt_ms).max(0.0);
        for n in self.notes.iter_mut() {
            n.1 -= dt_ms;
        }
        self.notes.retain(|n| n.1 > 0.0);
        if c.back {
            return Flow::Hub;
        }
        if let Some(end) = self.end {
            self.end_ms += dt_ms;
            if c.confirm && self.end_ms >= ui_num("result_lock_ms") {
                return match end.outcome {
                    Outcome::Win if self.night.night < NIGHTS => Flow::Night(self.night.night + 1),
                    Outcome::Win => Flow::Hub,
                    Outcome::Caught(_) => Flow::Night(self.night.night),
                };
            }
            return Flow::Stay;
        }

        if c.skip_call {
            self.night.call.skip();
        }
        if c.mute_call && self.night.mute_call() {
            self.note(label("call_muted"));
        }
        let out = self.night.is_out();
        if out {
            self.shut = [false; 2];
            self.cam_up = false;
        } else {
            if c.door_l {
                self.shut[0] = !self.shut[0];
                nw_sound::event(MODE, "door_toggle", -0.6);
            }
            if c.door_r {
                self.shut[1] = !self.shut[1];
                nw_sound::event(MODE, "door_toggle", 0.6);
            }
            if c.toggle_cam {
                self.cam_up = !self.cam_up;
                nw_sound::event(MODE, "camera", 0.0);
            }
        }
        let lit = [c.light_l && !out, c.light_r && !out];
        for (i, side) in [-0.6f32, 0.6].into_iter().enumerate() {
            if lit[i] && !self.lights[i] {
                nw_sound::event(MODE, "light_on", side);
            }
        }
        self.lights = lit;
        let ids = self.cam_ids();
        if let Some(id) = c.pick_cam.filter(|id| ids.contains(id)) {
            self.cam = id;
        }
        if c.cam_step != 0 && !ids.is_empty() {
            let at = ids.iter().position(|id| *id == self.cam).unwrap_or(0) as i32;
            self.cam = ids[(at + c.cam_step).rem_euclid(ids.len() as i32) as usize];
        }
        if c.meet && self.cam_up {
            match self.night.meet(true, Some(self.cam_room()), false) {
                Meet::Met(_) => self.note(label("met_kathy")),
                Meet::Miss => self.note(label("missed_kathy")),
                Meet::Locked | Meet::None => {}
            }
        }

        let input = Input {
            door_l: self.shut[0],
            door_r: self.shut[1],
            light_l: self.lights[0],
            light_r: self.lights[1],
            cam: self.cam_up,
            cam_room: self.cam_up.then(|| self.cam_room()),
        };
        let events = self.night.step(dt_ms.min(ui_num("max_step_ms")), input);
        self.view.update(&self.night.call);
        for event in events {
            self.play(event);
        }
        Flow::Stay
    }

    fn play(&mut self, event: Event) {
        self.sound(&event);
        match event {
            Event::Out => {
                self.note(label("power_out_note"));
                self.flash_ms = ui_num("flash_ms");
            }
            Event::Repelled { id, .. } => self.note(fill(label("backs_off"), &[("name", hunter_name(id).to_owned())])),
            Event::Pushed { id, .. } => self.note(fill(label("pushed_back"), &[("name", hunter_name(id).to_owned())])),
            Event::Flinch { id, .. } => self.note(fill(label("flinches"), &[("name", hunter_name(id).to_owned())])),
            Event::Hum { door, muffled, .. } => {
                let side = if door == "doorL" { label("side_left") } else { label("side_right") };
                let how = if muffled { label("humming_muffled") } else { label("humming") };
                self.note(fill(how, &[("side", side.to_owned())]));
            }
            Event::Caught { id, darkness } => {
                self.end = Some(End { outcome: Outcome::Caught(id), darkness });
                self.flash_ms = ui_num("flash_ms");
            }
            Event::Win => {
                self.end = Some(End { outcome: Outcome::Win, darkness: false });
                self.save_clear();
            }
            Event::Moved { .. } | Event::Arrived { .. } | Event::Attack { .. } | Event::Call(_) => {}
        }
    }

    /// The cue for a rules event (the event-to-cue map is `events.classic` in `data/sounds.json`).
    fn sound(&self, event: &Event) {
        let side = |door: &str| if door == "doorL" { -0.6 } else { 0.6 };
        match event {
            Event::Out => nw_sound::event(MODE, "power_out", 0.0),
            Event::Caught { .. } => nw_sound::event(MODE, "caught", 0.0),
            Event::Win => nw_sound::event(MODE, "win", 0.0),
            Event::Moved { .. } => nw_sound::event(MODE, "step", 0.0),
            Event::Arrived { .. } => nw_sound::event(MODE, "arrive", 0.0),
            Event::Attack { .. } => nw_sound::event(MODE, "lure", 0.0),
            Event::Repelled { .. } => nw_sound::event(MODE, "repelled", 0.0),
            Event::Pushed { .. } => nw_sound::event(MODE, "pushed", 0.0),
            Event::Flinch { .. } => nw_sound::event(MODE, "flinch", 0.0),
            Event::Hum { door, muffled, .. } => nw_sound::event(MODE, if *muffled { "hum_muffled" } else { "hum" }, side(door)),
            Event::Call(CallEvent::Ring) => nw_sound::event(MODE, "ring", 0.0),
            Event::Call(_) => {}
        }
    }

    /// Writes the cleared night and its points to the progress file (best effort; the screenshot runs skip it).
    fn save_clear(&mut self) {
        if !self.persist {
            return;
        }
        let mut save = NightWatchSave::load();
        save.record_level_clear(CLASSIC, self.night.night);
        save.record_best(CLASSIC, self.night.points());
        save.record_night_best(CLASSIC, self.night.night, self.night.points());
        if save.save().is_err() {
            self.note(label("save_failed"));
        }
    }
}

// ---- layout ----------------------------------------------------------------------------------------------------

/// Every clickable or drawn region, for a window of `sw` by `sh`. Pure, so the click tests and the paint agree.
struct Layout {
    door_btn: [Rect; 2],
    light_btn: [Rect; 2],
    cam_btn: Rect,
    cam_buttons: Vec<Rect>,
    feed: Rect,
    map: Rect,
    call: Rect,
    doorway: [Rect; 2],
}

fn layout(sw: f32, sh: f32, cameras: usize) -> Layout {
    let btn_w = (sw * 0.11).clamp(110.0, 150.0);
    let btn_y = sh * 0.76;
    let door_x = [sw * 0.06, sw - sw * 0.06 - btn_w];
    let light_x = [door_x[0] + btn_w + 12.0, door_x[1] - btn_w - 12.0];
    let door_btn = [Rect::new(door_x[0], btn_y, btn_w, 44.0), Rect::new(door_x[1], btn_y, btn_w, 44.0)];
    let light_btn = [Rect::new(light_x[0], btn_y, btn_w, 44.0), Rect::new(light_x[1], btn_y, btn_w, 44.0)];
    let cam_btn = Rect::new((sw - 260.0) * 0.5, sh * 0.86, 260.0, 44.0);
    let doorway_w = sw * 0.22;
    let doorway = [
        Rect::new(sw * 0.06, sh * 0.30, doorway_w, sh * 0.42),
        Rect::new(sw - sw * 0.06 - doorway_w, sh * 0.30, doorway_w, sh * 0.42),
    ];
    let pad = 24.0;
    // the call box sits right under the HUD bar and the monitor panels start below it, so an open call covers neither
    let call_w = (sw * 0.56).clamp(420.0, 760.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let top = call.bottom() + 8.0;
    // the monitor panels end above the door and light buttons, with the camera buttons in between
    let panels_h = sh * 0.76 - 60.0 - top - 10.0;
    let feed = Rect::new(pad, top, (sw - 3.0 * pad) * 0.5, panels_h);
    let map = Rect::new(feed.x + feed.w + pad, top, feed.w, panels_h);
    let gap = 6.0;
    let cell = ((map.w - gap * (cameras.max(1) as f32 - 1.0)) / cameras.max(1) as f32).min(64.0);
    let cam_buttons = (0..cameras)
        .map(|i| Rect::new(map.x + i as f32 * (cell + gap), map.y + map.h + 10.0, cell, 38.0))
        .collect();
    Layout { door_btn, light_btn, cam_btn, cam_buttons, feed, map, call, doorway }
}

impl ClassicRun {
    fn layout(&self) -> Layout {
        layout(screen_width(), screen_height(), self.map.cameras.len())
    }

    fn map_view<'a>(&self, markers: &'a [(&'a str, &'a str)]) -> MapView<'a> {
        MapView { active_camera: Some(self.cam), hunters: markers, floor: None, legend: true }
    }
}

// ---- input -----------------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse. Q and E (or holding a light button) are held; the rest are presses.
pub fn poll_controls(run: &ClassicRun) -> Controls {
    let l = run.layout();
    let mouse = Vec2::from(mouse_position());
    let click = is_mouse_button_pressed(MouseButton::Left);
    let held = is_mouse_button_down(MouseButton::Left);
    let mut c = Controls {
        door_l: is_key_pressed(KeyCode::A),
        door_r: is_key_pressed(KeyCode::D),
        light_l: is_key_down(KeyCode::Q),
        light_r: is_key_down(KeyCode::E),
        toggle_cam: is_key_pressed(KeyCode::Space),
        skip_call: is_key_pressed(KeyCode::Enter),
        mute_call: is_key_pressed(KeyCode::M),
        meet: is_key_pressed(KeyCode::F),
        confirm: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space),
        back: is_key_pressed(KeyCode::Escape),
        ..Controls::default()
    };
    if is_key_pressed(KeyCode::Left) {
        c.cam_step = -1;
    }
    if is_key_pressed(KeyCode::Right) {
        c.cam_step = 1;
    }
    let digits = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5, KeyCode::Key6, KeyCode::Key7, KeyCode::Key8];
    for (i, key) in digits.iter().enumerate() {
        if is_key_pressed(*key) {
            c.pick_cam = run.map.cameras.get(i).map(|cam| cam.id);
        }
    }
    if run.end.is_some() {
        c.confirm |= click;
        return c;
    }
    c.light_l |= held && l.light_btn[0].contains(mouse);
    c.light_r |= held && l.light_btn[1].contains(mouse);
    if click {
        c.door_l |= l.door_btn[0].contains(mouse);
        c.door_r |= l.door_btn[1].contains(mouse);
        c.toggle_cam |= l.cam_btn.contains(mouse);
        if run.cam_up {
            for (i, r) in l.cam_buttons.iter().enumerate() {
                if r.contains(mouse) {
                    c.pick_cam = run.map.cameras.get(i).map(|cam| cam.id);
                }
            }
            let markers = run.markers();
            let view = run.map_view(&markers);
            if let Some(cam) = run.map.camera_at_screen(l.map, &view, mouse) {
                c.pick_cam = Some(cam.id);
            }
        }
    }
    c
}

// ---- drawing ---------------------------------------------------------------------------------------------------

thread_local! {
    // Classic's office and camera art, decoded on first use (after the window exists). Leaked on purpose, like the
    // sprites: the textures live for the whole run.
    static ART: RefCell<HashMap<String, &'static Texture2D>> = RefCell::new(HashMap::new());
}

/// The decoded art for `key`: `office`, `office_fg`, or a room id for its camera view. None for an unknown key.
fn art(key: &str) -> Option<&'static Texture2D> {
    ART.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(tex) = cache.get(key) {
            return Some(*tex);
        }
        let png = match key {
            "office" => data::CLASSIC_OFFICE_PNG,
            "office_fg" => data::CLASSIC_OFFICE_FG_PNG,
            room => data::CLASSIC_CAM_FILES.iter().find(|(r, _)| *r == room)?.1,
        };
        let tex: &'static Texture2D = Box::leak(Box::new(Texture2D::from_file_with_format(png, Some(ImageFormat::Png))));
        cache.insert(key.to_owned(), tex);
        Some(tex)
    })
}

/// Paints `tex` over the whole rect `(x, y, w, h)`.
fn draw_art(tex: &Texture2D, x: f32, y: f32, w: f32, h: f32, tint: Color) {
    draw_texture_ex(tex, x, y, tint, DrawTextureParams { dest_size: Some(vec2(w, h)), ..Default::default() });
}

fn text_centered(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_text(text, cx - w * 0.5, y, size, color);
}

fn button(rect: Rect, text: &str, active: bool) {
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, if active { AMBER } else { DIM });
    text_centered(text, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5 + 7.0, 22.0, if active { AMBER } else { INK });
}

/// A hunter silhouette standing on `base_y`, `h` tall, with glowing eyes. Only the fallback when a sprite is missing.
fn figure(cx: f32, base_y: f32, h: f32, color: Color, alpha: f32) {
    let body = Color::new(color.r * 0.35, color.g * 0.35, color.b * 0.35, alpha);
    let edge = Color::new(color.r, color.g, color.b, alpha);
    let w = h * 0.34;
    draw_rectangle(cx - w * 0.5, base_y - h * 0.62, w, h * 0.62, body);
    draw_rectangle_lines(cx - w * 0.5, base_y - h * 0.62, w, h * 0.62, 2.0, edge);
    let r = h * 0.14;
    draw_circle(cx, base_y - h * 0.62 - r, r, body);
    draw_circle_lines(cx, base_y - h * 0.62 - r, r, 2.0, edge);
    let eye = Color::new(1.0, 0.95, 0.7, alpha);
    draw_circle(cx - r * 0.4, base_y - h * 0.62 - r * 1.1, r * 0.14, eye);
    draw_circle(cx + r * 0.4, base_y - h * 0.62 - r * 1.1, r * 0.14, eye);
}

/// One hunter standing at (`cx`, `base_y`), `h` tall: the web cut-out for `facing`, or the old silhouette when the
/// sprite is missing.
fn hunter(id: &str, facing: Facing, cx: f32, base_y: f32, h: f32, alpha: f32) {
    let rect = Rect::new(cx - h * 0.4, base_y - h, h * 0.8, h);
    if sprites::draw_hunter(id, facing, rect, Color::new(1.0, 1.0, 1.0, alpha)) {
        return;
    }
    figure(cx, base_y, h, hunter_color(id), alpha);
}

/// A cheap deterministic hash for the monitor static.
fn hash(a: u64, b: u64) -> f32 {
    let x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    ((x ^ (x >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 40) as f32 / (1u64 << 24) as f32
}

fn hour_label(hour: u32) -> String {
    if hour == 0 {
        label("hour_midnight").to_owned()
    } else {
        fill(label("hour"), &[("hour", hour.to_string())])
    }
}

pub fn draw(run: &ClassicRun) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    let l = run.layout();
    if run.cam_up {
        draw_monitor(run, &l);
    } else {
        draw_office(run, &l, sw, sh);
    }
    draw_controls(run, &l);
    draw_hud(run, sw, sh);
    run.view.draw(&run.night.call, l.call);
    for (i, (note, _)) in run.notes.iter().enumerate() {
        text_centered(note, sw * 0.5, sh - 18.0 - (run.notes.len() - 1 - i) as f32 * 24.0, 22.0, AMBER);
    }
    if run.flash_ms > 0.0 {
        let a = (run.flash_ms / ui_num("flash_ms")) as f32 * 0.55;
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, a));
    }
    if run.night.is_out() && run.end.is_none() {
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.55));
        text_centered(label("power_out_title"), sw * 0.5, sh * 0.52, 64.0, RED_TEXT);
    }
    if let Some(end) = run.end {
        draw_result(run, end, sw, sh);
    }
}

fn draw_office(run: &ClassicRun, l: &Layout, sw: f32, sh: f32) {
    match art("office") {
        Some(back) => draw_art(back, 0.0, 0.0, sw, sh, WHITE),
        None => {
            // the plain back wall, floor and desk edge, when the office art is missing
            draw_rectangle(0.0, 0.0, sw, sh * 0.72, Color::new(0.07, 0.065, 0.085, 1.0));
            for i in 0..9 {
                let x = sw * (0.30 + 0.05 * i as f32);
                draw_line(x, sh * 0.30, x, sh * 0.72, 1.0, Color::new(0.10, 0.095, 0.12, 1.0));
            }
            draw_rectangle(0.0, sh * 0.72, sw, sh * 0.28, Color::new(0.045, 0.04, 0.05, 1.0));
            draw_rectangle(sw * 0.30, sh * 0.62, sw * 0.40, sh * 0.10, Color::new(0.10, 0.09, 0.08, 1.0));
            draw_rectangle_lines(sw * 0.30, sh * 0.62, sw * 0.40, sh * 0.10, 2.0, Color::new(0.20, 0.18, 0.15, 1.0));
        }
    }
    text_centered(label("monitor_office"), sw * 0.5, sh * 0.675, 22.0, DIM);

    for side in 0..2 {
        let r = l.doorway[side];
        let id = if side == 0 { "doorL" } else { "doorR" };
        let lit = run.lights[side];
        let dip = run.night.light_dip(id);
        // the opening: dark, or washed by the door lamp
        let lamp = if lit && !dip { Color::new(0.80, 0.72, 0.45, 0.32) } else if lit { Color::new(0.80, 0.72, 0.45, 0.08) } else { Color::new(0.0, 0.0, 0.0, 1.0) };
        draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.01, 0.01, 0.015, 1.0));
        if lit {
            draw_rectangle(r.x, r.y, r.w, r.h, lamp);
            for (n, who) in run.night.waiting_at(id).iter().enumerate() {
                let cx = r.x + r.w * 0.5 + n as f32 * 34.0;
                hunter(who, Facing::Front, cx, r.y + r.h, r.h * 0.82, if dip { 0.25 } else { 0.95 });
                text_centered(hunter_name(who), cx, r.y + r.h - r.h * 0.9, 18.0, INK);
            }
        }
        if run.shut[side] {
            draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.30, 0.33, 0.37, 1.0));
            for i in 1..8 {
                let y = r.y + r.h * i as f32 / 8.0;
                draw_line(r.x, y, r.x + r.w, y, 3.0, Color::new(0.22, 0.24, 0.28, 1.0));
            }
            text_centered(label("shut"), r.x + r.w * 0.5, r.y + r.h * 0.5, 30.0, Color::new(0.85, 0.88, 0.92, 1.0));
        }
        draw_rectangle_lines(r.x - 6.0, r.y - 6.0, r.w + 12.0, r.h + 12.0, 6.0, Color::new(0.22, 0.20, 0.18, 1.0));
        text_centered(label(if side == 0 { "door_label_left" } else { "door_label_right" }), r.x + r.w * 0.5, r.y - 18.0, 20.0, DIM);
    }
    // the office's foreground overlay goes over the room and the hunters standing in it
    if let Some(front) = art("office_fg") {
        draw_art(front, 0.0, 0.0, sw, sh, WHITE);
    }
}

fn draw_monitor(run: &ClassicRun, l: &Layout) {
    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.015, 0.03, 0.025, 1.0));
    // the camera feed: the room's web camera view when the art is there, else a dark panel
    let f = l.feed;
    let room = run.cam_room();
    match art(room) {
        Some(view) => draw_art(view, f.x, f.y, f.w, f.h, WHITE),
        None => draw_rectangle(f.x, f.y, f.w, f.h, Color::new(0.03, 0.06, 0.05, 1.0)),
    }
    let cam_label = run.map.camera(run.cam).map_or("", |c| c.label.as_str());
    draw_rectangle(f.x, f.y + f.h * 0.66, f.w, f.h * 0.34, Color::new(0.04, 0.08, 0.06, 0.55));
    draw_line(f.x, f.y + f.h * 0.66, f.x + f.w, f.y + f.h * 0.66, 2.0, Color::new(0.10, 0.20, 0.16, 1.0));
    let here = run.hunters_in(room);
    if here.is_empty() {
        text_centered(label("empty"), f.x + f.w * 0.5, f.y + f.h * 0.5, 30.0, Color::new(0.30, 0.50, 0.42, 1.0));
    }
    for (n, who) in here.iter().enumerate() {
        let cx = f.x + f.w * (n as f32 + 1.0) / (here.len() as f32 + 1.0);
        // the cameras watch from behind the hunters, so the monitor shows their back cut-out
        hunter(who, Facing::Back, cx, f.y + f.h * 0.86, f.h * 0.55, 0.9);
        text_centered(hunter_name(who), cx, f.y + f.h * 0.90, 20.0, INK);
    }
    // static and scanlines
    let tick = (run.anim_ms / 60.0) as u64;
    for i in 0..60u64 {
        let y = f.y + hash(tick, i) * f.h;
        let x = f.x + hash(i, tick) * f.w * 0.7;
        draw_line(x, y, x + hash(tick, i + 99) * f.w * 0.3, y, 1.0, Color::new(0.6, 0.9, 0.8, 0.10));
    }
    let mut y = f.y;
    while y < f.y + f.h {
        draw_line(f.x, y, f.x + f.w, y, 1.0, Color::new(0.0, 0.0, 0.0, 0.18));
        y += 4.0;
    }
    draw_rectangle_lines(f.x, f.y, f.w, f.h, 3.0, Color::new(0.30, 0.55, 0.45, 1.0));
    let caption = fill(label("cam_caption"), &[("cam", run.cam.to_string()), ("label", cam_label.to_owned())]);
    draw_text(&caption, f.x + 12.0, f.y + f.h - 12.0, 24.0, Color::new(0.55, 0.95, 0.80, 1.0));
    if (run.anim_ms / 500.0) as u64 % 2 == 0 {
        draw_circle(f.x + f.w - 24.0, f.y + 24.0, 7.0, RED_TEXT);
    }

    // the whole building, the active camera lit, a marker for every hunter
    let markers = run.markers();
    let view = run.map_view(&markers);
    run.map.draw(l.map, &view);
    for (i, r) in l.cam_buttons.iter().enumerate() {
        let cam = &run.map.cameras[i];
        button(*r, &cam.id.to_string(), cam.id == run.cam);
    }
}

fn draw_controls(run: &ClassicRun, l: &Layout) {
    if run.end.is_some() {
        return;
    }
    button(l.door_btn[0], label("door_left"), run.shut[0]);
    button(l.door_btn[1], label("door_right"), run.shut[1]);
    button(l.light_btn[0], label("light_left"), run.lights[0]);
    button(l.light_btn[1], label("light_right"), run.lights[1]);
    button(l.cam_btn, label(if run.cam_up { "monitor_close" } else { "monitor" }), run.cam_up);
}

fn draw_hud(run: &ClassicRun, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    draw_text(&fill(label("hud_night"), &[("night", run.night.night.to_string())]), 24.0, 30.0, 28.0, INK);
    let clock = hour_label(run.night.hour());
    text_centered(&clock, sw * 0.5, 32.0, 32.0, AMBER);
    let power = run.night.power();
    let pct = fill(label("hud_power"), &[("power", (power.ceil() as i32).to_string())]);
    let w = measure_text(&pct, None, 28, 1.0).width;
    let colour = if power < 20.0 { RED_TEXT } else { INK };
    draw_text(&pct, sw - w - 24.0, 30.0, 28.0, colour);
    let bar = Rect::new(sw - 24.0 - 200.0, 36.0, 200.0, 6.0);
    draw_rectangle(bar.x, bar.y, bar.w, bar.h, Color::new(0.2, 0.2, 0.2, 1.0));
    draw_rectangle(bar.x, bar.y, bar.w * (power / 100.0) as f32, bar.h, colour);
    let usage = [run.shut[0], run.shut[1], run.lights[0], run.lights[1], run.cam_up].iter().filter(|u| **u).count();
    draw_text(&fill(label("hud_usage"), &[("bars", "|".repeat(usage + 1))]), sw - w - 24.0 - 190.0, 30.0, 22.0, DIM);
    text_centered(label("hint"), sw * 0.5, sh - 2.0 - run.notes.len() as f32 * 24.0 - 6.0, 16.0, DIM);
}

fn draw_result(run: &ClassicRun, end: End, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let (title, colour, line) = match end.outcome {
        Outcome::Win => (
            fill(label("cleared_title"), &[("night", run.night.night.to_string())]),
            AMBER,
            fill(label("cleared_line"), &[("points", run.night.points().to_string())]),
        ),
        Outcome::Caught(_) if end.darkness => (label("dark_title").to_owned(), RED_TEXT, label("dark_line").to_owned()),
        Outcome::Caught(id) => (label("caught_title").to_owned(), RED_TEXT, fill(label("caught_line"), &[("name", hunter_name(id).to_owned())])),
    };
    text_centered(&title, sw * 0.5, sh * 0.40, 64.0, colour);
    text_centered(&line, sw * 0.5, sh * 0.40 + 48.0, 28.0, INK);
    let next = match end.outcome {
        Outcome::Win if run.night.night < NIGHTS => label("next_night"),
        Outcome::Win => label("to_hub"),
        Outcome::Caught(_) => label("try_again"),
    };
    if run.end_ms >= ui_num("result_lock_ms") {
        text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
        text_centered(label("esc_hub"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
    }
}

// ---- scripted runs (the --screenshot flag) --------------------------------------------------------------------

/// What a `--screenshot` run does before the picture: the night, how many fixed frames to run, the held controls, and
/// optionally to keep going until the first call line has this many letters showing (a call caught mid-reveal).
#[derive(Clone, Debug, Default)]
pub struct ShotPlan {
    pub night: u32,
    pub frames: u32,
    /// Monitor up on this camera from the first frame.
    pub cam: Option<u32>,
    pub door_l: bool,
    pub door_r: bool,
    pub light_l: bool,
    pub light_r: bool,
    pub call_chars: Option<usize>,
}

/// Steps a fresh night by the plan at a fixed 60 fps (`ui.shot_step_ms`) and returns it ready to draw. Nothing is saved or played.
pub fn run_script(calls: &CallsData, seed: i64, plan: &ShotPlan) -> ClassicRun {
    let mut run = ClassicRun::new(calls, plan.night.max(1), seed, false);
    let mut first = Controls {
        door_l: plan.door_l,
        door_r: plan.door_r,
        toggle_cam: plan.cam.is_some(),
        pick_cam: plan.cam,
        ..Controls::default()
    };
    let held = Controls { light_l: plan.light_l, light_r: plan.light_r, ..Controls::default() };
    let step = ui_num("shot_step_ms");
    let mut frame = 0u32;
    while frame < 200_000 {
        let reveal_done = plan.call_chars.map_or(true, |k| {
            let call = &run.night.call;
            call.open && call.line == 0 && call.text != CALL_INCOMING_TEXT && call.text.chars().count() >= k
        });
        if frame >= plan.frames && reveal_done || run.end.is_some() {
            break;
        }
        let controls = Controls { door_l: first.door_l, door_r: first.door_r, toggle_cam: first.toggle_cam, pick_cam: first.pick_cam, ..held };
        run.update(step, &controls);
        first = Controls::default();
        frame += 1;
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(night: u32) -> ClassicRun {
        ClassicRun::new(&CallsData::load(), night, 7, false)
    }

    fn peaceful() -> ClassicRun {
        let calls = CallsData::load();
        let levels = [("dovelle", 0), ("ravelle", 0), ("ilyra", 0), ("aki", 0), ("nyxie", 0)];
        ClassicRun::with_night(classic::start(&calls, 1, 3, &levels, true), false)
    }

    #[test]
    fn the_layout_fits_the_window_and_the_buttons_do_not_overlap() {
        let l = layout(1280.0, 720.0, 8);
        let all = [l.door_btn[0], l.door_btn[1], l.light_btn[0], l.light_btn[1], l.cam_btn];
        for (i, a) in all.iter().enumerate() {
            assert!(a.x >= 0.0 && a.right() <= 1280.0 && a.bottom() <= 720.0, "{a:?}");
            for b in &all[i + 1..] {
                assert!(a.intersect(*b).is_none(), "{a:?} overlaps {b:?}");
            }
        }
        assert_eq!(l.cam_buttons.len(), 8);
        assert!(l.cam_buttons.iter().all(|r| r.right() <= l.map.right() + 0.5 && r.bottom() <= 720.0));
        assert!(l.call.bottom() < l.doorway[0].y, "the call box clears the doorways");
        assert!(l.call.y >= 44.0, "the call box clears the HUD bar");
        assert!(l.call.intersect(l.feed).is_none() && l.call.intersect(l.map).is_none(), "the call box clears the monitor panels");
        assert!(l.cam_buttons.iter().all(|r| r.bottom() < l.door_btn[0].y), "the monitor panels clear the buttons");
    }

    #[test]
    fn every_camera_covers_a_room_the_mode_knows() {
        let r = run(1);
        for id in r.cam_ids() {
            let mut one = run(1);
            one.update(16.0, &Controls { pick_cam: Some(id), ..Controls::default() });
            let room = one.cam_room();
            assert!(r.map.room(room).is_some());
        }
    }

    #[test]
    fn every_camera_room_has_its_view_and_every_stalker_its_sprites() {
        let r = run(1);
        for id in r.cam_ids() {
            let mut one = run(1);
            one.update(16.0, &Controls { pick_cam: Some(id), ..Controls::default() });
            let room = one.cam_room();
            assert!(data::CLASSIC_CAM_FILES.iter().any(|(name, _)| *name == room), "no camera view for room '{room}'");
        }
        for id in stalker_ids() {
            assert!(data::HUNTER_SPRITE_FILES.iter().any(|(sprite, _, _)| *sprite == id), "no sprite for stalker '{id}'");
        }
    }

    #[test]
    fn the_ui_timings_and_labels_load_from_the_rules_file() {
        for key in ["max_step_ms", "result_lock_ms", "note_ms", "flash_ms", "shot_step_ms"] {
            assert!(ui_num(key) > 0.0, "{key}");
        }
        assert!((ui_num("shot_step_ms") - 1000.0 / 60.0).abs() < 1e-9, "the screenshot step stays at 60 fps");
        assert_eq!(hunter_name("dovelle"), "DOVELLE");
        assert_eq!(hunter_name("nobody"), "SOMEONE");
        assert_eq!(stalker_ids().len(), 5);
        assert_eq!(fill(label("cam_caption"), &[("cam", "3".to_owned()), ("label", "BACKSTAGE".to_owned())]), "CAM 3  BACKSTAGE");
        assert_eq!(hour_label(0), "12 AM");
        assert_eq!(hour_label(3), "3 AM");
    }

    #[test]
    fn a_door_press_toggles_and_drains_power_faster() {
        let mut idle = run(1);
        let mut shut = run(1);
        shut.update(16.0, &Controls { door_l: true, ..Controls::default() });
        assert!(shut.shut[0]);
        for _ in 0..600 {
            idle.update(16.0, &Controls::default());
            shut.update(16.0, &Controls::default());
        }
        assert!(shut.shut[0], "the door stays shut with no further press");
        assert!(shut.night.power() < idle.night.power());
        shut.update(16.0, &Controls { door_l: true, ..Controls::default() });
        assert!(!shut.shut[0]);
    }

    #[test]
    fn lights_are_held_and_the_monitor_toggles() {
        let mut r = run(1);
        r.update(16.0, &Controls { light_r: true, toggle_cam: true, ..Controls::default() });
        assert!(r.lights[1] && r.cam_up);
        r.update(16.0, &Controls::default());
        assert!(!r.lights[1], "released");
        r.update(16.0, &Controls { cam_step: 1, ..Controls::default() });
        assert_eq!(r.cam, 2);
        r.update(16.0, &Controls { cam_step: -1, ..Controls::default() });
        r.update(16.0, &Controls { cam_step: -1, ..Controls::default() });
        assert_eq!(r.cam, 8, "stepping wraps");
        r.update(16.0, &Controls { toggle_cam: true, ..Controls::default() });
        assert!(!r.cam_up);
    }

    #[test]
    fn the_scripted_run_catches_the_call_mid_reveal() {
        let plan = ShotPlan { night: 1, frames: 0, call_chars: Some(20), ..ShotPlan::default() };
        let r = run_script(&CallsData::load(), 20261103, &plan);
        let call = &r.night.call;
        assert!(call.open && call.line == 0);
        let shown = call.text.chars().count();
        assert!((20..call.full_text().chars().count()).contains(&shown), "{shown} of {}", call.full_text());
    }

    #[test]
    fn a_peaceful_night_is_won_and_enter_moves_to_the_next() {
        let mut r = peaceful();
        let mut flow = Flow::Stay;
        for _ in 0..20_000 {
            flow = r.update(16.0, &Controls::default());
            if r.end.is_some() {
                break;
            }
        }
        assert!(matches!(r.end, Some(End { outcome: Outcome::Win, .. })));
        assert_eq!(flow, Flow::Stay);
        assert_eq!(r.update(16.0, &Controls { confirm: true, ..Controls::default() }), Flow::Stay, "locked at first");
        r.update(1000.0, &Controls::default());
        assert_eq!(r.update(16.0, &Controls { confirm: true, ..Controls::default() }), Flow::Night(2));
        assert_eq!(r.update(16.0, &Controls { back: true, ..Controls::default() }), Flow::Hub);
    }

    #[test]
    fn running_out_of_power_opens_the_doors_and_drops_the_monitor() {
        let mut r = peaceful();
        r.update(16.0, &Controls { door_l: true, door_r: true, toggle_cam: true, ..Controls::default() });
        let held = Controls { light_l: true, light_r: true, ..Controls::default() };
        for _ in 0..40_000 {
            r.update(16.0, &held);
            if r.night.is_out() {
                break;
            }
        }
        assert!(r.night.is_out());
        r.update(16.0, &held);
        assert_eq!(r.shut, [false; 2]);
        assert_eq!(r.lights, [false; 2]);
        assert!(!r.cam_up);
    }
}
