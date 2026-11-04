//! Masked Hours on screen: the playable driver for `modes::masked_hours`. `MaskRun` owns one shift, maps the frame's
//! `Controls` (hold S for the mask, F the flashlight, L the desk lamp, R the music box, Q and E the vent lights; SPACE
//! or W the monitor, arrows and 1-8 the cameras) onto the mode's `MaskInput`, plays the events it returns, runs the
//! call script and saves a cleared night. `draw` paints the office (the web build's office picture with the hall
//! doorway, the two vent grilles and the desk) or the monitor with the camera picture beside the wing map, plus the HUD,
//! the controls and the call box. Hunters are the shared cut-out sprites. Timings, spots and every label come from the
//! rules file's `ui` block. Everything but `poll_controls` and `draw` is plain state, so the tests run a whole shift headless.

use macroquad::prelude::*;

use crate::building_map::{self, BuildingMap, MapView};
use crate::calls_view::CallView;
use crate::data::{self, CallsData};
use crate::framework::{CallScript, Mulberry};
use crate::mask::{VentLightHeld, VentSide};
use crate::modes::masked_hours::{self as mh, MaskEvent, MaskInput, MaskOptions, MaskedHours, Outcome};
use crate::save::NightWatchSave;
use crate::sprites::{self, Facing};

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);
const SCREEN_TEXT: Color = Color::new(0.55, 0.95, 0.80, 1.0);

/// The data folder's name: `data/cast`, `data/rules` and `data/maps` all hold `masked_hours.json`.
const DATA_ID: &str = "masked_hours";

/// The `ui.text` keys this driver reads. A test checks each one is in the rules file.
#[cfg(test)]
const TEXT_KEYS: &[&str] = &[
    "hint", "monitor", "monitor_close", "desk", "hall_label", "left_vent", "right_vent", "mask_on", "battery_dead",
    "empty", "held_still", "cam_tag", "night", "hour", "midnight", "music_box", "battery", "side_left", "side_right",
    "muted", "save_failed", "note_hall", "note_office", "note_leave", "note_rush", "note_flash_hit", "note_duct",
    "note_vent", "note_vent_retreat", "note_fetch", "note_dark", "note_box_start", "note_box_empty", "note_box_saved",
    "caught_glyra", "caught_nyxie", "caught_other", "caught_title", "dark_title", "cleared", "win_line", "next_night",
    "last_night", "retry", "esc_hub",
];

/// The seven hunters by id, with their silhouette colours (the fallback when a hunter has no sprite). Names and homes
/// come from the cast data.
const HUNTERS: [(&str, Color); 7] = [
    ("thessine", Color::new(0.92, 0.55, 0.70, 1.0)),
    ("umbriel", Color::new(0.95, 0.40, 0.30, 1.0)),
    ("verrine", Color::new(0.35, 0.80, 0.78, 1.0)),
    ("haelith", Color::new(0.55, 0.85, 0.40, 1.0)),
    ("ilyra", Color::new(0.62, 0.50, 0.92, 1.0)),
    ("glyra", Color::new(0.95, 0.75, 0.35, 1.0)),
    ("nyxie", Color::new(0.55, 0.62, 0.95, 1.0)),
];

/// A driver number from the rules file's `ui` block, by its path below `ui` (for example `max_step_ms` or `office.wash`).
fn ui_num(path: &str) -> f64 {
    data::num(data::rules(DATA_ID), &format!("ui.{path}"))
}

/// A driver string from the rules file's `ui.text` block.
fn ui_text(key: &str) -> &'static str {
    data::text(data::rules(DATA_ID), &format!("ui.text.{key}"))
}

/// A template from `ui.text` with its `{slot}`s filled in.
fn fill(template: &str, slots: &[(&str, &str)]) -> String {
    slots.iter().fold(template.to_owned(), |s, (slot, value)| s.replace(slot, value))
}

/// The hold-key label at `i` (Q, E, S, F, L, R in that order).
fn hold_label(i: usize) -> &'static str {
    data::list(data::rules(DATA_ID), "ui.text.holds")[i].as_str().unwrap_or("")
}

fn hunter_name(id: &str) -> &'static str {
    data::text(data::cast(DATA_ID), &format!("names.{id}"))
}

fn home_of(id: &str) -> &'static str {
    data::text(data::cast(DATA_ID), &format!("homes.{id}"))
}

fn hunter_color(id: &str) -> Color {
    HUNTERS.iter().find(|h| h.0 == id).map_or(WHITE, |h| h.1)
}

fn side_word(side: VentSide) -> &'static str {
    match side {
        VentSide::L => ui_text("side_left"),
        VentSide::R => ui_text("side_right"),
    }
}

/// A note or caption naming one hunter.
fn named(key: &str, id: &str) -> String {
    fill(ui_text(key), &[("{name}", hunter_name(id))])
}

/// A note naming one hunter and the vent side.
fn with_side(key: &str, id: &str, side: VentSide) -> String {
    fill(ui_text(key), &[("{name}", hunter_name(id)), ("{side}", side_word(side))])
}

/// What the player did this frame. Hold keys are true while held; the monitor and camera fields are presses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    /// S: the mask over the face. It cannot be worn while the monitor is up.
    pub mask: bool,
    /// F: the flashlight on the hall.
    pub flashlight: bool,
    /// L: the desk lamp (drains the battery).
    pub lamp: bool,
    /// R: wind the music box.
    pub wind: bool,
    /// Q: the left vent light.
    pub vent_l: bool,
    /// E: the right vent light.
    pub vent_r: bool,
    /// SPACE or W: the monitor up or down.
    pub toggle_cam: bool,
    pub pick_cam: Option<u32>,
    /// -1 or 1 steps the monitor to the previous or next camera.
    pub cam_step: i32,
    pub skip_call: bool,
    pub mute_call: bool,
    pub confirm: bool,
    pub back: bool,
}

/// Where the shell goes after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskFlow {
    Stay,
    Hub,
    /// Start this Masked Hours night (the next one after a win, the same one to retry).
    Night(u32),
}

/// One shift on screen.
pub struct MaskRun {
    night: MaskedHours,
    night_no: u32,
    persist: bool,
    map: &'static BuildingMap,
    call: CallScript,
    view: CallView,
    hour_ms: f64,
    held: VentLightHeld,
    vent_down: [bool; 2],
    cam_up: bool,
    cam: u32,
    last_c: Controls,
    last_input: MaskInput,
    end: Option<Outcome>,
    end_ms: f64,
    flash_ms: f64,
    anim_ms: f64,
    notes: Vec<(String, f64)>,
}

impl MaskRun {
    /// Opens Masked Hours night `night` (1 to 6). `persist` lets a won night write the save file (off for tests and
    /// the screenshot runs).
    pub fn new(calls: &CallsData, night: u32, seed: i64, persist: bool) -> Self {
        let night_no = night.clamp(1, mh::nights());
        let map = building_map::load(DATA_ID).expect("the masked hours building map is embedded");
        let mut view = CallView::new();
        view.player.enabled = persist;
        Self {
            night: MaskedHours::new(night_no as i32, Mulberry::new(seed), MaskOptions::default()),
            night_no,
            persist,
            map,
            call: CallScript::for_level(calls, mh::MODE_ID, night_no as i32),
            view,
            hour_ms: mh::night_ms() / f64::from(mh::hours()),
            held: VentLightHeld::default(),
            vent_down: [false; 2],
            cam_up: false,
            cam: map.cameras.first().map_or(1, |c| c.id),
            last_c: Controls::default(),
            last_input: MaskInput::default(),
            end: None,
            end_ms: 0.0,
            flash_ms: 0.0,
            anim_ms: 0.0,
            notes: Vec::new(),
        }
    }

    /// Opens the Custom Night: each slider is that hunter's level (`levels`, by id). Nothing is saved from it.
    pub fn new_custom(calls: &CallsData, night: u32, levels: &[(&'static str, i32)], seed: i64) -> Self {
        let mut run = Self::new(calls, night, seed, false);
        let options = MaskOptions { levels: levels.iter().copied().collect(), custom: true, ..MaskOptions::default() };
        run.night = MaskedHours::new(run.night_no as i32, Mulberry::new(seed), options);
        run.call = CallScript::new(Vec::new(), 0.0);
        run
    }

    /// How the shift ended, once it has.
    #[cfg(test)]
    pub fn outcome(&self) -> Option<Outcome> {
        self.end
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
        map.camera(self.cam).and_then(|c| c.covers.first()).map_or("office", |&i| map.rooms[i].id.as_str())
    }

    fn cam_ids(&self) -> Vec<u32> {
        self.map.cameras.iter().map(|c| c.id).collect()
    }

    /// Every hunter on the map as (room id, hunter id). Glyra sits at her home room, the music box.
    fn markers(&self) -> Vec<(&'static str, &'static str)> {
        let mut out: Vec<(&'static str, &'static str)> = self.night.hunter_rooms().iter().map(|&(id, room)| (room, id)).collect();
        out.push((home_of("glyra"), "glyra"));
        out
    }

    fn hunters_in(&self, room: &str) -> Vec<&'static str> {
        self.markers().into_iter().filter(|(r, _)| *r == room).map(|(_, id)| id).collect()
    }

    /// One frame of the shift: reads the controls, steps the mode, runs the call and plays what happened.
    pub fn update(&mut self, dt_ms: f64, c: &Controls) -> MaskFlow {
        self.anim_ms += dt_ms;
        self.flash_ms = (self.flash_ms - dt_ms).max(0.0);
        for n in self.notes.iter_mut() {
            n.1 -= dt_ms;
        }
        self.notes.retain(|n| n.1 > 0.0);
        if c.back {
            return MaskFlow::Hub;
        }
        if let Some(outcome) = self.end {
            self.end_ms += dt_ms;
            if c.confirm && self.end_ms >= ui_num("result_lock_ms") {
                return match outcome {
                    Outcome::Win if self.night_no < mh::nights() => MaskFlow::Night(self.night_no + 1),
                    Outcome::Win => MaskFlow::Hub,
                    Outcome::Caught(_) => MaskFlow::Night(self.night_no),
                };
            }
            return MaskFlow::Stay;
        }

        self.last_c = *c;
        if c.skip_call {
            self.call.skip();
        }
        if c.mute_call && self.call.mute() {
            self.note(ui_text("muted"));
        }
        if c.toggle_cam {
            self.cam_up = !self.cam_up;
        }
        let ids = self.cam_ids();
        if let Some(id) = c.pick_cam.filter(|id| ids.contains(id)) {
            self.cam = id;
        }
        if c.cam_step != 0 && !ids.is_empty() {
            let at = ids.iter().position(|id| *id == self.cam).unwrap_or(0) as i32;
            self.cam = ids[(at + c.cam_step).rem_euclid(ids.len() as i32) as usize];
        }
        // A vent light is a press: the latest press wins and letting go of one never brings the other back.
        for (i, side, down) in [(0, VentSide::L, c.vent_l), (1, VentSide::R, c.vent_r)] {
            if down != self.vent_down[i] {
                self.held.press(side, down);
                self.vent_down[i] = down;
            }
        }
        let room = self.cam_room();
        let input = MaskInput {
            mask: c.mask && !self.cam_up,
            monitor: self.cam_up,
            cam: self.cam_up.then_some(room),
            // The desk lamps work from the desk, so the vent panels are always clear in this wing.
            vent_lights: self.held.active(true, self.cam_up, room),
            hall_light: c.flashlight,
            winding: c.wind,
            lamp: c.lamp,
        };
        self.last_input = input;
        let dt = dt_ms.min(ui_num("max_step_ms"));
        let events = self.night.step(dt, input);
        self.call.update(self.night.elapsed_ms(), dt);
        self.view.update(&self.call);
        for event in events {
            self.play(event);
        }
        MaskFlow::Stay
    }

    fn play(&mut self, event: MaskEvent) {
        match event {
            MaskEvent::Hall { id } => self.note(named("note_hall", id)),
            MaskEvent::Office { id } => self.note(named("note_office", id)),
            MaskEvent::Leave { id } => self.note(named("note_leave", id)),
            MaskEvent::Rush { id, .. } => self.note(named("note_rush", id)),
            MaskEvent::FlashHit { id, .. } => self.note(named("note_flash_hit", id)),
            MaskEvent::Duct { id, side, .. } => self.note(with_side("note_duct", id, side)),
            MaskEvent::Vent { id, side } => self.note(with_side("note_vent", id, side)),
            MaskEvent::VentRetreat { id, side } => self.note(with_side("note_vent_retreat", id, side)),
            MaskEvent::Fetch { id, .. } => self.note(named("note_fetch", id)),
            MaskEvent::Dark { id } => self.note(named("note_dark", id)),
            MaskEvent::BoxStart => self.note(ui_text("note_box_start")),
            MaskEvent::BoxEmpty => self.note(ui_text("note_box_empty")),
            MaskEvent::BoxSaved => self.note(ui_text("note_box_saved")),
            MaskEvent::Caught { by } => {
                self.end = Some(Outcome::Caught(by));
                self.flash_ms = ui_num("flash_ms");
            }
            MaskEvent::Win => {
                self.end = Some(Outcome::Win);
                self.save_clear();
            }
            _ => {}
        }
    }

    /// Writes the cleared night and its points to the progress file (best effort; test and screenshot runs skip it).
    fn save_clear(&mut self) {
        if !self.persist {
            return;
        }
        let mut save = NightWatchSave::load();
        save.record_level_clear(mh::MODE_ID, self.night_no);
        save.record_best(mh::MODE_ID, mh::points_for(Outcome::Win));
        save.record_night_best(mh::MODE_ID, self.night_no, mh::points_for(Outcome::Win));
        if save.save().is_err() {
            self.note(ui_text("save_failed"));
        }
    }
}

// ---- layout ----------------------------------------------------------------------------------------------------

/// Every clickable or drawn region, for a window of `sw` by `sh`. Pure, so the click tests and the paint agree.
struct Layout {
    /// Q, E, S, F, L and R, left to right.
    holds: [Rect; 6],
    monitor_btn: Rect,
    cam_buttons: Vec<Rect>,
    feed: Rect,
    map: Rect,
    call: Rect,
    /// The office picture's box, under the HUD bar and above the desk strip.
    office: Rect,
    hall: Rect,
    vents: [Rect; 2],
}

fn layout(sw: f32, sh: f32, cameras: usize) -> Layout {
    let pad = 24.0;
    let call_w = (sw * 0.56).clamp(420.0, 760.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let top = call.bottom() + 8.0;
    let panels_bottom = sh * 0.66;
    let panels_h = panels_bottom - top;
    let feed = Rect::new(pad, top, (sw - 3.0 * pad) * 0.5, panels_h);
    let map = Rect::new(feed.x + feed.w + pad, top, feed.w, panels_h);
    let gap = 6.0;
    let cams = cameras.max(1) as f32;
    let cell = ((map.w - gap * (cams - 1.0)) / cams).min(64.0);
    let cam_buttons = (0..cameras).map(|i| Rect::new(map.x + i as f32 * (cell + gap), map.y + map.h + 10.0, cell, 38.0)).collect();
    let hold_w = ((sw - 2.0 * pad - 50.0) / 6.0).min(170.0);
    let row_x = (sw - (hold_w * 6.0 + 50.0)) * 0.5;
    let row_y = sh * 0.76;
    let holds = std::array::from_fn(|i| Rect::new(row_x + i as f32 * (hold_w + 10.0), row_y, hold_w, 44.0));
    let monitor_btn = Rect::new((sw - 260.0) * 0.5, sh * 0.86, 260.0, 44.0);
    let office = office_frame(sw, sh);
    let hall = office_part(office, "hall");
    let vents = [office_part(office, "vent_l"), office_part(office, "vent_r")];
    Layout { holds, monitor_btn, cam_buttons, feed, map, call, office, hall, vents }
}

/// `area` shrunk to `aspect` (width over height), centred in it.
fn fit(area: Rect, aspect: f32) -> Rect {
    let w = area.w.min(area.h * aspect);
    let h = w / aspect;
    Rect::new(area.x + (area.w - w) * 0.5, area.y + (area.h - h) * 0.5, w, h)
}

fn picture_size() -> (f32, f32) {
    (ui_num("picture.0") as f32, ui_num("picture.1") as f32)
}

/// The office picture's box: the picture fitted under the HUD bar, above the desk strip.
fn office_frame(sw: f32, sh: f32) -> Rect {
    let top = ui_num("office.top") as f32;
    let area = Rect::new(0.0, top, sw, sh * ui_num("office.height") as f32 - top);
    let (pw, ph) = picture_size();
    fit(area, pw / ph)
}

/// A part of the office picture from its percentage box in the rules file (`office.hall`, `office.vent_l`, ...).
fn office_part(frame: Rect, key: &str) -> Rect {
    let p = |i: usize| ui_num(&format!("office.{key}.{i}")) as f32 / 100.0;
    Rect::new(frame.x + frame.w * p(0), frame.y + frame.h * p(1), frame.w * p(2), frame.h * p(3))
}

/// Where a figure stands on a camera picture, in picture pixels: x in percent of the width, then the feet's y and the
/// figure's height in pixels. None for a room the rules file does not place.
fn cam_stand(room: &str) -> Option<[f32; 3]> {
    let a = data::rules(DATA_ID)["ui"]["cam_stand"][room].as_array()?;
    Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
}

impl MaskRun {
    fn layout(&self) -> Layout {
        layout(screen_width(), screen_height(), self.map.cameras.len())
    }

    fn map_view<'a>(&self, markers: &'a [(&'a str, &'a str)]) -> MapView<'a> {
        MapView { active_camera: Some(self.cam), hunters: markers, floor: None, legend: true }
    }
}

// ---- input -----------------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse. The lamp, mask, flashlight, box and vent keys are held; the rest are presses.
pub fn poll_controls(run: &MaskRun) -> Controls {
    let l = run.layout();
    let mouse = Vec2::from(mouse_position());
    let click = is_mouse_button_pressed(MouseButton::Left);
    let held = is_mouse_button_down(MouseButton::Left);
    let mut c = Controls {
        mask: is_key_down(KeyCode::S),
        flashlight: is_key_down(KeyCode::F),
        lamp: is_key_down(KeyCode::L),
        wind: is_key_down(KeyCode::R),
        vent_l: is_key_down(KeyCode::Q),
        vent_r: is_key_down(KeyCode::E),
        toggle_cam: is_key_pressed(KeyCode::Space) || is_key_pressed(KeyCode::W),
        skip_call: is_key_pressed(KeyCode::Enter),
        mute_call: is_key_pressed(KeyCode::M),
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
    let down = |rect: Rect| held && rect.contains(mouse);
    c.vent_l |= down(l.holds[0]);
    c.vent_r |= down(l.holds[1]);
    c.mask |= down(l.holds[2]);
    c.flashlight |= down(l.holds[3]);
    c.lamp |= down(l.holds[4]);
    c.wind |= down(l.holds[5]);
    if click {
        c.toggle_cam |= l.monitor_btn.contains(mouse);
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

/// The embedded pictures: the web build's office with its foreground (desk), and one picture per camera room.
struct Art {
    office: Texture2D,
    office_fg: Texture2D,
    cams: Vec<(&'static str, Texture2D)>,
}

fn png(bytes: &[u8]) -> Texture2D {
    Texture2D::from_file_with_format(bytes, Some(ImageFormat::Png))
}

fn load_art() -> Art {
    Art {
        office: png(include_bytes!("../assets/offices/mask.png")),
        office_fg: png(include_bytes!("../assets/offices/mask-fg.png")),
        cams: vec![
            ("ballroom", png(include_bytes!("../assets/cams/mask/ballroom.png"))),
            ("costume", png(include_bytes!("../assets/cams/mask/costume.png"))),
            ("crawl", png(include_bytes!("../assets/cams/mask/crawl.png"))),
            ("ductL", png(include_bytes!("../assets/cams/mask/ductL.png"))),
            ("ductR", png(include_bytes!("../assets/cams/mask/ductR.png"))),
            ("gallery", png(include_bytes!("../assets/cams/mask/gallery.png"))),
            ("hall", png(include_bytes!("../assets/cams/mask/hall.png"))),
            ("music", png(include_bytes!("../assets/cams/mask/music.png"))),
        ],
    }
}

thread_local! {
    // Decoded on first use, which is on the window thread, and leaked so the textures live for the run.
    static ART: &'static Art = Box::leak(Box::new(load_art()));
}

fn art() -> &'static Art {
    ART.with(|a| *a)
}

fn cam_picture(room: &str) -> Option<&'static Texture2D> {
    art().cams.iter().find(|(r, _)| *r == room).map(|(_, tex)| tex)
}

fn text_centered(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_text(text, cx - w * 0.5, y, size, color);
}

fn button(rect: Rect, label: &str, active: bool) {
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, if active { AMBER } else { DIM });
    text_centered(label, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5 + 7.0, 20.0, if active { AMBER } else { INK });
}

/// A hunter silhouette standing on `base_y`, `h` tall, with glowing eyes. Only the fallback for a hunter with no sprite.
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

/// One hunter standing with its feet on `base_y`, `h` tall: its front cut-out, or the silhouette when it has no sprite.
/// Every hunter in Masked Hours faces the player, as the web build draws them, so all are front cut-outs.
fn hunter(id: &str, cx: f32, base_y: f32, h: f32, alpha: f32) {
    let rect = Rect::new(cx - h * 0.25, base_y - h, h * 0.5, h);
    if !sprites::draw_hunter(id, Facing::Front, rect, Color::new(1.0, 1.0, 1.0, alpha)) {
        figure(cx, base_y, h, hunter_color(id), alpha);
    }
}

/// Hunters standing side by side along the bottom of `rect`, each named at the top.
fn stand_at(rect: Rect, who: &[&str], alpha: f32) {
    for (n, id) in who.iter().enumerate() {
        let cx = rect.x + rect.w * (n as f32 + 1.0) / (who.len() as f32 + 1.0);
        hunter(id, cx, rect.y + rect.h, rect.h * 0.82, alpha);
        text_centered(hunter_name(id), cx, rect.y + 22.0, 18.0, INK);
    }
}

/// A cheap deterministic hash for the monitor static.
fn hash(a: u64, b: u64) -> f32 {
    let x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    ((x ^ (x >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 40) as f32 / (1u64 << 24) as f32
}

fn hour_label(hour: u32) -> String {
    if hour == 0 {
        ui_text("midnight").to_owned()
    } else {
        fill(ui_text("hour"), &[("{h}", hour.to_string().as_str())])
    }
}

pub fn draw(run: &MaskRun) {
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
    run.view.draw(&run.call, l.call);
    for (i, (note, _)) in run.notes.iter().enumerate() {
        text_centered(note, sw * 0.5, sh - 18.0 - (run.notes.len() - 1 - i) as f32 * 24.0, 22.0, AMBER);
    }
    if run.flash_ms > 0.0 {
        let a = (run.flash_ms / ui_num("flash_ms")) as f32 * 0.55;
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, a));
    }
    if run.night.battery() <= 0.0 && run.end.is_none() {
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.45));
        text_centered(ui_text("battery_dead"), sw * 0.5, sh * 0.52, 56.0, RED_TEXT);
    }
    if let Some(end) = run.end {
        draw_result(run, end, sw, sh);
    }
}

fn draw_office(run: &MaskRun, l: &Layout, sw: f32, sh: f32) {
    let art = art();
    let frame = l.office;
    draw_rectangle(0.0, frame.bottom(), sw, sh - frame.bottom(), Color::new(0.045, 0.04, 0.05, 1.0));
    draw_texture_ex(&art.office, frame.x, frame.y, WHITE, DrawTextureParams { dest_size: Some(vec2(frame.w, frame.h)), ..Default::default() });
    draw_rectangle(frame.x, frame.y, frame.w, frame.h, Color::new(0.0, 0.0, 0.0, ui_num("office.wash") as f32));

    // the hall doorway: dark unless the flashlight is on it, but whoever waits at the office door always shows
    let h = l.hall;
    let lit = run.last_input.hall_light && run.night.battery() > 0.0;
    draw_rectangle(h.x, h.y, h.w, h.h, Color::new(0.01, 0.01, 0.015, 1.0));
    let mut at_door = run.hunters_in("office");
    if lit {
        draw_rectangle(h.x, h.y, h.w, h.h, Color::new(0.80, 0.72, 0.45, 0.30));
        at_door.extend(run.hunters_in("hall"));
    }
    stand_at(h, &at_door, if lit { 0.95 } else { 0.35 });
    draw_rectangle_lines(h.x - 6.0, h.y - 6.0, h.w + 12.0, h.h + 12.0, 6.0, Color::new(0.22, 0.20, 0.18, 1.0));
    text_centered(ui_text("hall_label"), h.x + h.w * 0.5, h.y - 16.0, 20.0, DIM);

    // the two vent grilles in the side walls: a lit grille shows whoever is in that duct
    for (i, side) in [VentSide::L, VentSide::R].into_iter().enumerate() {
        let v = l.vents[i];
        let lit = run.last_input.vent_lights.get(side);
        draw_rectangle(v.x, v.y, v.w, v.h, Color::new(0.01, 0.01, 0.01, 1.0));
        if lit {
            draw_rectangle(v.x, v.y, v.w, v.h, Color::new(0.45, 0.85, 0.80, 0.28));
            let duct = if side == VentSide::L { "ductL" } else { "ductR" };
            stand_at(v, &run.hunters_in(duct), 0.9);
        }
        for k in 1..6 {
            let y = v.y + v.h * k as f32 / 6.0;
            draw_line(v.x, y, v.x + v.w, y, 3.0, Color::new(0.22, 0.24, 0.24, 1.0));
        }
        draw_rectangle_lines(v.x - 4.0, v.y - 4.0, v.w + 8.0, v.h + 8.0, 4.0, Color::new(0.22, 0.20, 0.18, 1.0));
    }

    // the desk over the hunters' feet
    draw_texture_ex(&art.office_fg, frame.x, frame.y, WHITE, DrawTextureParams { dest_size: Some(vec2(frame.w, frame.h)), ..Default::default() });
    for (i, v) in l.vents.iter().enumerate() {
        text_centered(ui_text(if i == 0 { "left_vent" } else { "right_vent" }), v.x + v.w * 0.5, v.y + v.h + 24.0, 20.0, DIM);
    }
    text_centered(ui_text("desk"), sw * 0.5, sh * 0.675, 22.0, DIM);

    // the mask, worn over the face while S is held
    if run.last_input.mask {
        let (mx, my) = (sw * 0.5 - 70.0, sh * 0.40);
        draw_rectangle(mx, my, 140.0, 120.0, Color::new(0.86, 0.83, 0.77, 0.94));
        draw_rectangle(mx + 26.0, my + 36.0, 34.0, 18.0, Color::new(0.02, 0.02, 0.02, 1.0));
        draw_rectangle(mx + 80.0, my + 36.0, 34.0, 18.0, Color::new(0.02, 0.02, 0.02, 1.0));
        draw_rectangle(mx + 46.0, my + 84.0, 48.0, 8.0, Color::new(0.02, 0.02, 0.02, 1.0));
        text_centered(ui_text("mask_on"), sw * 0.5, my + 150.0, 20.0, INK);
    }
}

fn draw_monitor(run: &MaskRun, l: &Layout) {
    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.015, 0.03, 0.025, 1.0));
    let f = l.feed;
    draw_rectangle(f.x, f.y, f.w, f.h, Color::new(0.03, 0.06, 0.05, 1.0));
    let room = run.cam_room();
    let (pw, ph) = picture_size();
    let pic = fit(f, pw / ph);
    if let Some(tex) = cam_picture(room) {
        draw_texture_ex(tex, pic.x, pic.y, WHITE, DrawTextureParams { dest_size: Some(vec2(pic.w, pic.h)), ..Default::default() });
    }
    let label = run.map.camera(run.cam).map_or("", |c| c.label.as_str());
    let here = run.hunters_in(room);
    if here.is_empty() {
        text_centered(ui_text("empty"), f.x + f.w * 0.5, f.y + f.h * 0.5, 30.0, Color::new(0.30, 0.50, 0.42, 1.0));
    }
    let frozen = run.night.frozen();
    let spot = cam_stand(room);
    let k = here.len() as f32;
    for (n, who) in here.iter().enumerate() {
        let (cx, foot, height) = match spot {
            Some([x, y, h]) => {
                let scale = pic.h / ph;
                let spread = (n as f32 - (k - 1.0) * 0.5) * pic.w * ui_num("cam_spread") as f32;
                (pic.x + pic.w * x / 100.0 + spread, pic.y + pic.h * y / ph, h * scale)
            }
            None => (f.x + f.w * (n as f32 + 1.0) / (k + 1.0), f.y + f.h * 0.86, f.h * 0.55),
        };
        hunter(who, cx, foot, height, 0.9);
        text_centered(hunter_name(who), cx, f.y + f.h * 0.90, 20.0, INK);
        if frozen.contains(who) {
            text_centered(ui_text("held_still"), cx, f.y + f.h * 0.14, 18.0, AMBER);
        }
    }
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
    let cam_no = run.cam.to_string();
    draw_text(&fill(ui_text("cam_tag"), &[("{n}", cam_no.as_str()), ("{label}", label)]), f.x + 12.0, f.y + f.h - 12.0, 24.0, SCREEN_TEXT);
    if (run.anim_ms / 500.0) as u64 % 2 == 0 {
        draw_circle(f.x + f.w - 24.0, f.y + 24.0, 7.0, RED_TEXT);
    }

    // the whole wing, the active camera lit, a marker for every hunter
    let markers = run.markers();
    let view = run.map_view(&markers);
    run.map.draw(l.map, &view);
    for (i, r) in l.cam_buttons.iter().enumerate() {
        let cam = &run.map.cameras[i];
        button(*r, &cam.id.to_string(), cam.id == run.cam);
    }
}

fn draw_controls(run: &MaskRun, l: &Layout) {
    if run.end.is_some() {
        return;
    }
    let c = run.last_c;
    let holds = [
        (hold_label(0), c.vent_l),
        (hold_label(1), c.vent_r),
        (hold_label(2), c.mask && !run.cam_up),
        (hold_label(3), c.flashlight),
        (hold_label(4), c.lamp),
        (hold_label(5), c.wind),
    ];
    for (rect, (label, on)) in l.holds.iter().zip(holds) {
        button(*rect, label, on);
    }
    button(l.monitor_btn, if run.cam_up { ui_text("monitor_close") } else { ui_text("monitor") }, run.cam_up);
}

fn draw_hud(run: &MaskRun, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    let night = run.night_no.to_string();
    draw_text(&fill(ui_text("night"), &[("{n}", night.as_str())]), 24.0, 30.0, 28.0, INK);
    let hour = ((run.night.elapsed_ms() / run.hour_ms) as u32).min(mh::hours());
    text_centered(&hour_label(hour), sw * 0.5, 32.0, 32.0, AMBER);

    let music = run.night.music_box();
    let music_colour = if music < 20.0 { RED_TEXT } else { INK };
    let music_pct = (music.ceil() as i32).to_string();
    draw_text(&fill(ui_text("music_box"), &[("{p}", music_pct.as_str())]), 180.0, 30.0, 22.0, music_colour);
    draw_rectangle(180.0, 36.0, 160.0, 6.0, Color::new(0.2, 0.2, 0.2, 1.0));
    draw_rectangle(180.0, 36.0, 160.0 * (music / 100.0) as f32, 6.0, music_colour);

    let battery = run.night.battery();
    let battery_pct = (battery.ceil() as i32).to_string();
    let pct = fill(ui_text("battery"), &[("{p}", battery_pct.as_str())]);
    let w = measure_text(&pct, None, 28, 1.0).width;
    let colour = if battery < 20.0 { RED_TEXT } else { INK };
    draw_text(&pct, sw - w - 24.0, 30.0, 28.0, colour);
    let bar = Rect::new(sw - 24.0 - 200.0, 36.0, 200.0, 6.0);
    draw_rectangle(bar.x, bar.y, bar.w, bar.h, Color::new(0.2, 0.2, 0.2, 1.0));
    draw_rectangle(bar.x, bar.y, bar.w * (battery / 100.0) as f32, bar.h, colour);

    text_centered(ui_text("hint"), sw * 0.5, sh - 2.0 - run.notes.len() as f32 * 24.0 - 6.0, 16.0, DIM);
}

fn caught_line(by: &str) -> String {
    match by {
        "glyra" => ui_text("caught_glyra").to_owned(),
        "nyxie" => ui_text("caught_nyxie").to_owned(),
        _ => named("caught_other", by),
    }
}

fn draw_result(run: &MaskRun, end: Outcome, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let night = run.night_no.to_string();
    let points = mh::points_for(Outcome::Win).to_string();
    let (title, colour, line) = match end {
        Outcome::Win => (fill(ui_text("cleared"), &[("{n}", night.as_str())]), AMBER, fill(ui_text("win_line"), &[("{points}", points.as_str())])),
        Outcome::Caught("nyxie") => (ui_text("dark_title").to_owned(), RED_TEXT, caught_line("nyxie")),
        Outcome::Caught(by) => (ui_text("caught_title").to_owned(), RED_TEXT, caught_line(by)),
    };
    text_centered(&title, sw * 0.5, sh * 0.40, 64.0, colour);
    text_centered(&line, sw * 0.5, sh * 0.40 + 48.0, 28.0, INK);
    let next = match end {
        Outcome::Win if run.night_no < mh::nights() => ui_text("next_night"),
        Outcome::Win => ui_text("last_night"),
        Outcome::Caught(_) => ui_text("retry"),
    };
    if run.end_ms >= ui_num("result_lock_ms") {
        text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
        text_centered(ui_text("esc_hub"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
    }
}

// ---- scripted runs (the --screenshot --mode mask flags) --------------------------------------------------------

/// What a `--mode mask` screenshot run does before the picture: the night, how many fixed frames to run, the held
/// keys, and optionally to keep going until the first call line has this many letters showing.
#[derive(Clone, Debug, Default)]
pub struct ShotPlan {
    pub night: u32,
    pub frames: u32,
    /// Monitor up on this camera from the first frame.
    pub cam: Option<u32>,
    pub mask: bool,
    pub flashlight: bool,
    pub lamp: bool,
    pub wind: bool,
    pub vent_l: bool,
    pub vent_r: bool,
    pub call_chars: Option<usize>,
}

/// Steps a fresh shift by the plan at a fixed 60 fps and returns it ready to draw. Nothing is saved.
pub fn run_script(calls: &CallsData, seed: i64, plan: &ShotPlan) -> MaskRun {
    let mut run = MaskRun::new(calls, plan.night.max(1), seed, false);
    let held = Controls {
        mask: plan.mask,
        flashlight: plan.flashlight,
        lamp: plan.lamp,
        wind: plan.wind,
        vent_l: plan.vent_l,
        vent_r: plan.vent_r,
        ..Controls::default()
    };
    let step_ms = ui_num("script_step_ms");
    let mut frame = 0u32;
    while frame < 200_000 {
        let reveal_done = plan.call_chars.map_or(true, |k| {
            let call = &run.call;
            call.open && call.line >= 0 && call.text.chars().count() >= k
        });
        if (frame >= plan.frames && reveal_done) || run.end.is_some() {
            break;
        }
        let first = frame == 0;
        let controls = Controls { toggle_cam: first && plan.cam.is_some(), pick_cam: if first { plan.cam } else { None }, ..held };
        run.update(step_ms, &controls);
        frame += 1;
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(night: u32) -> MaskRun {
        MaskRun::new(&CallsData::load(), night, 7, false)
    }

    /// A player who keeps the wing in hand: the mask when Thessine stands at the office door, the flashlight on the
    /// hall, a vent light on whichever duct a vent hunter crawls into, and the music box wound below half.
    fn bot(run: &MaskRun) -> Controls {
        let rooms = run.markers();
        let in_room = |room: &str| rooms.iter().any(|(r, _)| *r == room);
        Controls {
            mask: in_room("office"),
            flashlight: in_room("hall"),
            wind: run.night.music_box() < 50.0,
            vent_l: in_room("ductL"),
            vent_r: in_room("ductR"),
            ..Controls::default()
        }
    }

    #[test]
    fn the_layout_fits_the_window_and_the_controls_do_not_overlap() {
        let l = layout(1280.0, 720.0, 8);
        let mut all: Vec<Rect> = l.holds.to_vec();
        all.push(l.monitor_btn);
        for (i, a) in all.iter().enumerate() {
            assert!(a.x >= 0.0 && a.right() <= 1280.0 && a.bottom() <= 720.0, "{a:?}");
            for b in &all[i + 1..] {
                assert!(a.intersect(*b).is_none(), "{a:?} overlaps {b:?}");
            }
        }
        assert_eq!(l.cam_buttons.len(), 8);
        assert!(l.cam_buttons.iter().all(|r| r.right() <= l.map.right() + 0.5 && r.bottom() < l.holds[0].y));
        for other in [l.feed, l.map, l.hall, l.vents[0], l.vents[1]] {
            assert!(l.call.intersect(other).is_none(), "the call box clears {other:?}");
        }
        assert!(l.call.y >= 44.0, "the call box clears the HUD bar");
    }

    #[test]
    fn the_office_picture_keeps_its_aspect_and_sits_under_the_hud() {
        let l = layout(1280.0, 720.0, 8);
        assert!(l.office.y >= 44.0 && l.office.bottom() <= 720.0 * 0.72 + 0.5);
        assert!((l.office.w / l.office.h - 480.0 / 270.0).abs() < 0.01, "{:?}", l.office);
        assert!(l.hall.x > l.office.x && l.hall.right() < l.office.right());
        assert!(l.vents[0].right() < l.hall.x && l.hall.right() < l.vents[1].x);
    }

    #[test]
    fn every_camera_covers_a_room_the_mode_knows() {
        let r = fresh(1);
        for id in r.cam_ids() {
            let mut one = fresh(1);
            one.update(16.0, &Controls { pick_cam: Some(id), ..Controls::default() });
            assert!(r.map.room(one.cam_room()).is_some());
        }
    }

    #[test]
    fn every_camera_room_has_a_picture_spot_in_the_rules() {
        let map = building_map::load(DATA_ID).expect("the masked hours building map is embedded");
        for cam in &map.cameras {
            let room = cam.covers.first().map(|&i| map.rooms[i].id.as_str()).expect("a camera covers a room");
            assert!(cam_stand(room).is_some(), "{room} has no ui.cam_stand spot");
        }
    }

    #[test]
    fn the_ui_block_names_every_timing_and_string_the_driver_reads() {
        for key in ["max_step_ms", "result_lock_ms", "note_ms", "flash_ms", "script_step_ms"] {
            assert!(ui_num(key) > 0.0, "{key}");
        }
        assert!(ui_num("office.wash") >= 0.0 && ui_num("office.top") > 0.0);
        assert_eq!(hold_label(5), "R  MUSIC BOX");
        for key in TEXT_KEYS {
            assert!(!ui_text(key).is_empty(), "ui.text.{key}");
        }
    }

    #[test]
    fn the_latest_vent_press_wins_and_letting_go_does_not_bring_the_other_back() {
        let mut r = fresh(1);
        r.update(16.0, &Controls { vent_l: true, ..Controls::default() });
        assert!(r.last_input.vent_lights.l && !r.last_input.vent_lights.r);
        r.update(16.0, &Controls { vent_l: true, vent_r: true, ..Controls::default() });
        assert!(!r.last_input.vent_lights.l && r.last_input.vent_lights.r);
        r.update(16.0, &Controls { vent_r: true, ..Controls::default() });
        assert!(!r.last_input.vent_lights.l && r.last_input.vent_lights.r);
    }

    #[test]
    fn the_mask_cannot_be_worn_with_the_monitor_up() {
        let mut r = fresh(1);
        r.update(16.0, &Controls { mask: true, toggle_cam: true, ..Controls::default() });
        assert!(r.cam_up);
        assert!(!r.last_input.mask, "the monitor takes the face");
        r.update(16.0, &Controls { mask: true, toggle_cam: true, ..Controls::default() });
        assert!(!r.cam_up);
        assert!(r.last_input.mask, "the mask goes on with the monitor down");
    }

    #[test]
    fn a_scripted_shift_reaches_a_win_or_a_loss_in_bounded_steps() {
        let mut r = fresh(1);
        let mut steps = 0;
        while r.outcome().is_none() && steps < 200_000 {
            let c = bot(&r);
            r.update(16.0, &c);
            steps += 1;
        }
        assert!(matches!(r.outcome(), Some(Outcome::Win) | Some(Outcome::Caught(_))), "no end after {steps} steps");
    }

    #[test]
    fn an_idle_player_is_caught() {
        let mut r = fresh(1);
        let mut steps = 0;
        while r.outcome().is_none() && steps < 200_000 {
            r.update(16.0, &Controls::default());
            steps += 1;
        }
        assert!(matches!(r.outcome(), Some(Outcome::Caught(_))), "idle won after {steps} steps");
    }

    #[test]
    fn the_call_script_plays_every_line_of_every_night() {
        let calls = CallsData::load();
        for night in 1..=mh::nights() {
            let lines = calls.for_level(mh::MODE_ID, night as i32).expect("every night has a call").lines.len() as i32;
            let mut call = CallScript::for_level(&calls, mh::MODE_ID, night as i32);
            let mut seen = Vec::new();
            let mut t = 0.0;
            while !call.done && t < 600_000.0 {
                call.update(t, 16.0);
                if call.open && call.line >= 0 && seen.last() != Some(&call.line) {
                    seen.push(call.line);
                }
                t += 16.0;
            }
            assert!(call.done, "night {night} call never finished");
            assert_eq!(seen, (0..lines).collect::<Vec<_>>(), "night {night}");
        }
    }

    #[test]
    fn the_shift_rings_the_call_and_it_types_its_first_line() {
        let mut r = fresh(1);
        let mut shown = false;
        for _ in 0..1200 {
            let c = bot(&r);
            r.update(16.0, &c);
            if r.call.open && r.call.line == 0 && r.call.text.chars().count() > 3 {
                shown = true;
                break;
            }
        }
        assert!(shown, "the call box opened with its first line");
    }

    #[test]
    fn a_scripted_screenshot_run_keeps_the_hunters_and_the_call() {
        let plan = ShotPlan { night: 1, frames: 0, call_chars: Some(12), ..ShotPlan::default() };
        let run = run_script(&CallsData::load(), 20_261_103, &plan);
        assert!(run.call.open && run.call.line == 0);
        assert!(run.call.text.chars().count() >= 12);
    }
}
