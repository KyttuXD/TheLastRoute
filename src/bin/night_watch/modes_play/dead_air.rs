//! Dead Air on screen: the shell's driver for `modes::dead_air` (the rules). `DeadAirRun` owns one shift, maps the
//! frame's `Controls` onto the rules' calls, plays the events they return and saves a won night. With the monitor down
//! the booth shows: A and D seal the vents, H and G run the heater and the fan. With the monitor up: 1-8 pick a camera,
//! L lures Xivvy to the room on screen, Q, W, E, R and T reboot audio, cameras, vents, heater and fan. SPACE toggles the
//! monitor, M or N mutes Zellia, ENTER skips a line, ESC goes back to the hub.
//!
//! Every number and every label comes from `data/rules/dead_air.json` (`play`, `art`, `systems`, `air`, `climate`).
//! Everything but `poll_controls`, `draw` and `Art` is plain state, so it is tested without a window.

use macroquad::prelude::*;
use serde_json::Value;

use std::collections::HashMap;

use crate::building_map::{self, BuildingMap, MapView};
use crate::calls_view::CallView;
use crate::data::{self, CallsData, CALL_INCOMING_TEXT};
use crate::framework::CallScript;
use crate::modes::dead_air::{self, BrokenBy, DeadAir, DeadAirEvent, DeadAirSetup, Hunter, Outcome, Phantom, Side, System};
use crate::save::NightWatchSave;
use crate::sprites::{self, Facing};

/// The data file's mode id (rules, cast, buildings and map).
const DATA: &str = "dead_air";
/// The id the save and the hub use (Godot's `lure`), so `cleared_lure` keeps its meaning.
const SAVE: &str = "lure";

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);
const SHUTTER: Color = Color::new(0.30, 0.33, 0.37, 0.92);

/// The five systems in the data file's order (their index is `system as usize`).
const SYSTEMS: [System; 5] = [System::Audio, System::Cams, System::Vent, System::Heater, System::Fan];
const REBOOT_KEYS: [(KeyCode, System); 5] =
    [(KeyCode::Q, System::Audio), (KeyCode::W, System::Cams), (KeyCode::E, System::Vent), (KeyCode::R, System::Heater), (KeyCode::T, System::Fan)];
const DIGITS: [KeyCode; 8] =
    [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5, KeyCode::Key6, KeyCode::Key7, KeyCode::Key8];

// ---- data ------------------------------------------------------------------------------------------------------------

fn rules() -> &'static Value {
    data::rules(DATA)
}

fn text(key: &str) -> &'static str {
    data::text(rules(), &format!("play.text.{key}"))
}

fn play(key: &str) -> f64 {
    data::num(rules(), &format!("play.{key}"))
}

/// A number of the rules file's `ui` block: draw timings, sizes and alphas.
fn ui(key: &str) -> f64 {
    data::num(rules(), &format!("ui.{key}"))
}

/// A string of the rules file's `ui` block.
fn ui_text(key: &str) -> &'static str {
    data::text(rules(), &format!("ui.{key}"))
}

/// Which cut-out the monitor draws each hunter with (`ui.cam_facing`: "front" or "back").
fn cam_facing() -> Facing {
    match ui_text("cam_facing") {
        "back" => Facing::Back,
        _ => Facing::Front,
    }
}

fn name(hunter: &str) -> &'static str {
    data::text(rules(), &format!("play.names.{hunter}"))
}

fn nights() -> i32 {
    data::int(rules(), "nights") as i32
}

fn points_for(night: i32) -> u32 {
    night.max(0) as u32 * data::int(rules(), "points_per_level") as u32
}

/// Fills each `{}` in `template` with the next argument.
fn fill<S: AsRef<str>>(template: &str, args: &[S]) -> String {
    let mut out = String::new();
    let mut rest = template;
    let mut i = 0;
    while let Some(at) = rest.find("{}") {
        out.push_str(&rest[..at]);
        out.push_str(args.get(i).map_or("", AsRef::as_ref));
        i += 1;
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

fn system_id(system: System) -> &'static str {
    match system {
        System::Audio => "audio",
        System::Cams => "cams",
        System::Vent => "vent",
        System::Heater => "heater",
        System::Fan => "fan",
    }
}

fn system_entry(system: System) -> &'static Value {
    let id = system_id(system);
    data::list(rules(), "systems")
        .iter()
        .find(|entry| data::text(entry, "id") == id)
        .unwrap_or_else(|| panic!("night_watch data for mode 'dead_air': no system '{id}'"))
}

fn system_label(system: System) -> &'static str {
    data::text(system_entry(system), "label")
}

fn system_key(system: System) -> &'static str {
    data::text(system_entry(system), "key")
}

fn side_label(side: Side) -> &'static str {
    match side {
        Side::Left => text("side_left"),
        Side::Right => text("side_right"),
    }
}

fn hunter_id(hunter: Hunter) -> &'static str {
    match hunter {
        Hunter::Xivvy => "xivvy",
        Hunter::Ferryn => "ferryn",
        Hunter::Briselle => "briselle",
        Hunter::Mavrine => "mavrine",
        Hunter::Solyth => "solyth",
        Hunter::Wynelle => "wynelle",
    }
}

fn phantom_id(phantom: Phantom) -> &'static str {
    match phantom {
        Phantom::Ferryn => "ferryn",
        Phantom::Briselle => "briselle",
        Phantom::Mavrine => "mavrine",
    }
}

fn hunter_color(id: &str) -> Color {
    match id {
        "xivvy" => Color::new(0.55, 0.95, 0.60, 1.0),
        "ferryn" => Color::new(0.80, 0.70, 0.95, 1.0),
        "briselle" => Color::new(0.95, 0.60, 0.75, 1.0),
        "mavrine" => Color::new(0.60, 0.75, 0.95, 1.0),
        _ => WHITE,
    }
}

// ---- controls and flow -----------------------------------------------------------------------------------------------

/// What the player did this frame. Every field is a press; the monitor and the booth read different ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    /// Booth: seal the left or right vent (toggles).
    pub seal_l: bool,
    pub seal_r: bool,
    /// Booth: heater and fan (toggle each other).
    pub heater: bool,
    pub fan: bool,
    /// Monitor up or down (SPACE).
    pub toggle_cam: bool,
    /// Monitor: pick this camera by its number.
    pub pick_cam: Option<u32>,
    /// Monitor: -1 or 1 steps to the previous or next camera.
    pub cam_step: i32,
    /// Monitor: lure Xivvy to the room on screen.
    pub lure: bool,
    /// Monitor: reboot one system.
    pub reboot: Option<System>,
    pub skip_call: bool,
    pub mute_call: bool,
    pub confirm: bool,
    pub back: bool,
}

/// Where the shell goes after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Stay,
    Hub,
    /// Start this night (the next one after a win, the same one to retry).
    Night(i32),
}

pub struct DeadAirRun {
    pub night: i32,
    dead: DeadAir,
    pub call: CallScript,
    view: CallView,
    map: &'static BuildingMap,
    persist: bool,
    cam_up: bool,
    cam: u32,
    /// Systems the player has sent to reboot and that are not back yet (the rules only say broken or working).
    rebooting: [bool; 5],
    end: Option<Outcome>,
    end_ms: f64,
    flash_ms: f64,
    notes: Vec<(String, f64)>,
    anim_ms: f64,
}

impl DeadAirRun {
    /// Opens Dead Air night `night` (1 to 6). `persist` lets a won night write the save file (off for tests and shots).
    pub fn new(calls: &CallsData, night: i32, seed: i64, persist: bool) -> Self {
        let night = night.clamp(1, nights());
        let map = building_map::load(DATA).expect("the dead air building map is embedded");
        let cam = map.cameras.first().map_or(1, |c| c.id);
        let mut view = CallView::new();
        view.player.enabled = persist;
        Self {
            night,
            dead: DeadAir::new(DeadAirSetup::night(night, seed)),
            call: dead_air::call_script(calls, night, false),
            view,
            map,
            persist,
            cam_up: false,
            cam,
            rebooting: [false; 5],
            end: None,
            end_ms: 0.0,
            flash_ms: 0.0,
            notes: Vec::new(),
            anim_ms: 0.0,
        }
    }

    /// Opens the Custom Night: each slider is that hunter's level (`levels`, by id). Nothing is saved from it.
    pub fn new_custom(calls: &CallsData, night: i32, levels: &[(&str, i32)], seed: i64) -> Self {
        let levels: HashMap<Hunter, i32> = levels.iter().map(|(id, level)| (Hunter::from_id(id), *level)).collect();
        let mut run = Self::new(calls, night, seed, false);
        run.dead = DeadAir::new(DeadAirSetup::custom(seed, levels));
        run.call = dead_air::call_script(calls, night, true);
        run
    }

    fn note(&mut self, text: impl Into<String>) {
        self.notes.push((text.into(), play("note_ms")));
        let keep = play("max_notes") as usize;
        while self.notes.len() > keep {
            self.notes.remove(0);
        }
    }

    /// The room the active camera covers.
    fn cam_room(&self) -> &'static str {
        let map: &'static BuildingMap = self.map;
        map.camera(self.cam).and_then(|c| c.covers.first()).map_or("lobby", |&i| map.rooms[i].id.as_str())
    }

    fn room_label(&self, room: &str) -> &'static str {
        let map: &'static BuildingMap = self.map;
        map.room(room).map_or("", |r| r.label.as_str())
    }

    fn cam_ids(&self) -> Vec<u32> {
        self.map.cameras.iter().map(|c| c.id).collect()
    }

    /// Every hunter on the map as (room id, hunter id): Xivvy, and a phantom while she is on a camera.
    fn markers(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![(self.dead.killer_room(), hunter_id(Hunter::Xivvy))];
        if let Some((who, room)) = self.dead.phantom() {
            out.push((room, phantom_id(who)));
        }
        out
    }

    fn hunters_in(&self, room: &str) -> Vec<&'static str> {
        self.markers().into_iter().filter(|(at, _)| *at == room).map(|(_, id)| id).collect()
    }

    fn hour(&self) -> u32 {
        let hours = data::int(rules(), "hours").max(1) as u32;
        let hour_ms = data::num(rules(), "night_ms") / f64::from(hours);
        ((self.dead.elapsed_ms() / hour_ms).floor() as u32).min(hours - 1)
    }

    /// The air left, 0 to 100 (the rules count stress up to `air.stress_max`).
    fn air_percent(&self) -> i32 {
        let max = data::num(rules(), "air.stress_max");
        ((1.0 - self.dead.stress() / max) * 100.0).round().clamp(0.0, 100.0) as i32
    }

    /// Solyth or Wynelle is on tonight, so the booth has a heater and a temperature.
    fn has_climate(&self) -> bool {
        self.dead.level_of(Hunter::Solyth) > 0 || self.dead.level_of(Hunter::Wynelle) > 0
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
        if let Some(outcome) = self.end {
            self.end_ms += dt_ms;
            if c.confirm && self.end_ms >= play("result_lock_ms") {
                return match outcome {
                    Outcome::Win if self.night < nights() => Flow::Night(self.night + 1),
                    Outcome::Win => Flow::Hub,
                    Outcome::Caught(_) => Flow::Night(self.night),
                };
            }
            return Flow::Stay;
        }

        if c.skip_call {
            self.call.skip();
        }
        if c.mute_call && self.call.mute() {
            self.note(text("muted"));
        }
        if c.toggle_cam {
            self.cam_up = !self.cam_up;
        }
        if self.cam_up {
            let ids = self.cam_ids();
            if let Some(id) = c.pick_cam.filter(|id| ids.contains(id)) {
                self.cam = id;
            }
            if c.cam_step != 0 && !ids.is_empty() {
                let at = ids.iter().position(|id| *id == self.cam).unwrap_or(0) as i32;
                self.cam = ids[(at + c.cam_step).rem_euclid(ids.len() as i32) as usize];
            }
            if c.lure {
                self.lure();
            }
            if let Some(system) = c.reboot {
                self.reboot(system);
            }
        } else {
            if c.seal_l {
                self.seal(Side::Left);
            }
            if c.seal_r {
                self.seal(Side::Right);
            }
            if c.heater {
                self.appliance(System::Heater);
            }
            if c.fan {
                self.appliance(System::Fan);
            }
        }

        let dt = dt_ms.min(play("max_step_ms"));
        let watch = self.cam_up.then(|| self.cam_room());
        let events = self.dead.step(dt, watch);
        self.call.update(self.dead.elapsed_ms(), dt);
        self.view.update(&self.call);
        for event in events {
            self.handle(event);
        }
        if self.end.is_none() {
            if let Some(outcome) = self.dead.outcome() {
                self.finish(outcome);
            }
        }
        Flow::Stay
    }

    fn finish(&mut self, outcome: Outcome) {
        self.end = Some(outcome);
        self.end_ms = 0.0;
        self.flash_ms = play("flash_ms");
        if outcome == Outcome::Win {
            self.save_clear();
        }
    }

    fn seal(&mut self, side: Side) {
        if self.dead.seal(side, None) {
            let key = if self.dead.seal_closed(side) { "seal_on" } else { "seal_off" };
            let line = fill(text(key), &[side_label(side)]);
            self.note(line);
        }
    }

    fn appliance(&mut self, which: System) {
        if !self.dead.toggle(which) {
            let line = fill(text("not_running"), &[system_label(which)]);
            self.note(line);
            return;
        }
        let on = match which {
            System::Heater => self.dead.heater_on(),
            _ => self.dead.fan_on(),
        };
        let line = fill(text(if on { "toggle_on" } else { "toggle_off" }), &[system_label(which)]);
        self.note(line);
    }

    fn reboot(&mut self, which: System) {
        if self.dead.reboot(which) {
            self.rebooting[which as usize] = true;
            let line = fill(text("rebooting"), &[system_label(which)]);
            self.note(line);
        } else {
            let line = fill(text("not_down"), &[system_label(which)]);
            self.note(line);
        }
    }

    fn lure(&mut self) {
        let room = self.cam_room();
        if self.dead.lure(room) {
            let line = fill(text("lure_ok"), &[self.room_label(room)]);
            self.note(line);
        } else {
            self.note(text("lure_fail"));
        }
    }

    fn handle(&mut self, event: DeadAirEvent) {
        match event {
            DeadAirEvent::Moved { .. } => {}
            DeadAirEvent::AtOffice { .. } => self.note(text("at_office")),
            DeadAirEvent::Retreat { side } => {
                let line = fill(text("retreat"), &[side_label(side)]);
                self.note(line);
            }
            DeadAirEvent::Broken { system, by } => {
                let line = match by {
                    BrokenBy::Phantom(who) => fill(text("broken_phantom"), &[name(phantom_id(who)), system_label(system)]),
                    BrokenBy::Strain => fill(text("broken_strain"), &[system_label(system)]),
                };
                self.flash_ms = self.flash_ms.max(play("flash_ms") * ui("broken_flash_scale"));
                self.note(line);
            }
            DeadAirEvent::Fixed { system } => {
                self.rebooting[system as usize] = false;
                let line = fill(text("fixed"), &[system_label(system)]);
                self.note(line);
            }
            DeadAirEvent::PhantomSeen { who, room } => {
                let line = fill(text("phantom_seen"), &[name(phantom_id(who)), self.room_label(room)]);
                self.note(line);
            }
            DeadAirEvent::Caught { .. } | DeadAirEvent::Win => {}
        }
    }

    /// Writes the cleared night and its points to the progress file (best effort; test and shot runs skip it).
    fn save_clear(&mut self) {
        if !self.persist {
            return;
        }
        let mut save = NightWatchSave::load();
        save.record_level_clear(SAVE, self.night as u32);
        save.record_best(SAVE, points_for(self.night));
        save.record_night_best(SAVE, self.night as u32, points_for(self.night));
        if save.save().is_err() {
            self.note(text("save_failed"));
        }
    }
}

// ---- layout ----------------------------------------------------------------------------------------------------------

/// Every clickable or drawn region for a window of `sw` by `sh`. Pure, so the click tests and the paint agree.
struct Layout {
    seal: [Rect; 2],
    appliance: [Rect; 2],
    lure: Rect,
    reboot: [Rect; 5],
    toggle: Rect,
    call: Rect,
    feed: Rect,
    map: Rect,
    cam_buttons: Vec<Rect>,
}

fn layout(sw: f32, sh: f32, cameras: usize) -> Layout {
    let pad = 24.0;
    let gap = 12.0;
    let row_y = sh * 0.74;
    // booth: the seals sit at the outer edges, the heater and fan just inside them
    let seal_w = (sw * 0.11).clamp(110.0, 150.0);
    let seal_x = [sw * 0.06, sw - sw * 0.06 - seal_w];
    let seal = [Rect::new(seal_x[0], row_y, seal_w, 44.0), Rect::new(seal_x[1], row_y, seal_w, 44.0)];
    let appliance = [Rect::new(seal_x[0] + seal_w + gap, row_y, seal_w, 44.0), Rect::new(seal_x[1] - seal_w - gap, row_y, seal_w, 44.0)];
    // monitor: the lure and the five reboots share one row
    let slot = (((sw - 2.0 * pad) - 5.0 * gap) / 6.0).min(170.0);
    let x0 = (sw - (6.0 * slot + 5.0 * gap)) * 0.5;
    let row: Vec<Rect> = (0..6).map(|i| Rect::new(x0 + i as f32 * (slot + gap), row_y, slot, 44.0)).collect();
    let toggle = Rect::new((sw - 260.0) * 0.5, sh * 0.86, 260.0, 44.0);
    // the call box sits under the HUD bar, and the monitor panels start below it
    let call_w = (sw * 0.56).clamp(420.0, 760.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let top = call.bottom() + 8.0;
    let panels_h = sh * 0.64 - top;
    let feed = Rect::new(pad, top, (sw - 3.0 * pad) * 0.5, panels_h);
    let map = Rect::new(feed.x + feed.w + pad, top, feed.w, panels_h);
    let cam_gap = 6.0;
    let cams = cameras.max(1) as f32;
    let cell = ((map.w - cam_gap * (cams - 1.0)) / cams).min(64.0);
    let cam_buttons = (0..cameras).map(|i| Rect::new(map.x + i as f32 * (cell + cam_gap), map.bottom() + 10.0, cell, 38.0)).collect();
    Layout {
        seal,
        appliance,
        lure: row[0],
        reboot: [row[1], row[2], row[3], row[4], row[5]],
        toggle,
        call,
        feed,
        map,
        cam_buttons,
    }
}

impl DeadAirRun {
    fn layout(&self) -> Layout {
        layout(screen_width(), screen_height(), self.map.cameras.len())
    }

    fn map_view<'a>(&self, markers: &'a [(&'a str, &'a str)]) -> MapView<'a> {
        MapView { active_camera: Some(self.cam), hunters: markers, floor: None, legend: true }
    }
}

/// The camera pictures, one per room the monitor can cover (the web build's `assets/cams/lure`).
const CAM_PICTURES: [(&str, &[u8]); 8] = [
    ("lobby", include_bytes!("../assets/cams/lure/lobby.png")),
    ("arcade", include_bytes!("../assets/cams/lure/arcade.png")),
    ("party", include_bytes!("../assets/cams/lure/party.png")),
    ("ballpit", include_bytes!("../assets/cams/lure/ballpit.png")),
    ("prize", include_bytes!("../assets/cams/lure/prize.png")),
    ("storage", include_bytes!("../assets/cams/lure/storage.png")),
    ("ventl", include_bytes!("../assets/cams/lure/ventl.png")),
    ("ventr", include_bytes!("../assets/cams/lure/ventr.png")),
];

/// The booth's art (`lure.png`, with `lure-fg.png` over it), the camera pictures and the vent quads, from the Godot
/// office layout.
pub struct Art {
    bg: Texture2D,
    fg: Texture2D,
    cams: Vec<(&'static str, Texture2D)>,
}

impl Art {
    pub fn load() -> Art {
        Art {
            bg: Texture2D::from_file_with_format(include_bytes!("../assets/offices/lure.png"), Some(ImageFormat::Png)),
            fg: Texture2D::from_file_with_format(include_bytes!("../assets/offices/lure-fg.png"), Some(ImageFormat::Png)),
            cams: CAM_PICTURES.iter().map(|(room, png)| (*room, Texture2D::from_file_with_format(png, Some(ImageFormat::Png)))).collect(),
        }
    }

    /// The picture for camera `room`, or None when the room has none (the monitor then draws plain panels).
    fn cam_picture(&self, room: &str) -> Option<&Texture2D> {
        self.cams.iter().find(|(id, _)| *id == room).map(|(_, tex)| tex)
    }
}

fn art_size() -> (f32, f32) {
    let size = data::list(rules(), "art.size");
    (size[0].as_f64().unwrap_or(480.0) as f32, size[1].as_f64().unwrap_or(270.0) as f32)
}

/// The art fitted into the window at its own aspect ratio, centred.
fn art_rect(sw: f32, sh: f32) -> Rect {
    let (w0, h0) = art_size();
    let mut w = sw;
    let mut h = sw * h0 / w0;
    if h > sh {
        h = sh;
        w = sh * w0 / h0;
    }
    Rect::new((sw - w) * 0.5, (sh - h) * 0.5, w, h)
}

fn to_screen(a: Rect, p: Vec2) -> Vec2 {
    let (w0, h0) = art_size();
    vec2(a.x + p.x / w0 * a.w, a.y + p.y / h0 * a.h)
}

fn vent_quad(side: Side) -> [Vec2; 4] {
    let key = match side {
        Side::Left => "art.vent_l",
        Side::Right => "art.vent_r",
    };
    let points = data::list(rules(), key);
    let point = |i: usize| {
        let xy = points[i].as_array().expect("a vent quad point is a pair");
        vec2(xy[0].as_f64().unwrap_or(0.0) as f32, xy[1].as_f64().unwrap_or(0.0) as f32)
    };
    [point(0), point(1), point(2), point(3)]
}

// ---- input -----------------------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse. The booth keys (A, D, H, G) and the monitor keys (L, Q-T, 1-8) are separate,
/// as in the rules: the booth keys do nothing while the monitor is up.
pub fn poll_controls(run: &DeadAirRun) -> Controls {
    let l = run.layout();
    let mouse = Vec2::from(mouse_position());
    let click = is_mouse_button_pressed(MouseButton::Left);
    let mut c = Controls {
        toggle_cam: is_key_pressed(KeyCode::Space),
        skip_call: is_key_pressed(KeyCode::Enter),
        mute_call: is_key_pressed(KeyCode::M) || is_key_pressed(KeyCode::N),
        confirm: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space),
        back: is_key_pressed(KeyCode::Escape),
        ..Controls::default()
    };
    if run.end.is_some() {
        c.confirm |= click;
        return c;
    }
    if run.cam_up {
        c.lure = is_key_pressed(KeyCode::L);
        c.reboot = REBOOT_KEYS.iter().find(|(key, _)| is_key_pressed(*key)).map(|(_, system)| *system);
        if is_key_pressed(KeyCode::Left) {
            c.cam_step = -1;
        }
        if is_key_pressed(KeyCode::Right) {
            c.cam_step = 1;
        }
        for (i, key) in DIGITS.iter().enumerate() {
            if is_key_pressed(*key) {
                c.pick_cam = run.map.cameras.get(i).map(|cam| cam.id);
            }
        }
        if click {
            c.lure |= l.lure.contains(mouse);
            for (i, r) in l.reboot.iter().enumerate() {
                if r.contains(mouse) {
                    c.reboot = Some(SYSTEMS[i]);
                }
            }
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
    } else {
        c.seal_l = is_key_pressed(KeyCode::A);
        c.seal_r = is_key_pressed(KeyCode::D);
        c.heater = is_key_pressed(KeyCode::H);
        c.fan = is_key_pressed(KeyCode::G);
        if click {
            c.seal_l |= l.seal[0].contains(mouse);
            c.seal_r |= l.seal[1].contains(mouse);
            c.heater |= l.appliance[0].contains(mouse);
            c.fan |= l.appliance[1].contains(mouse);
        }
    }
    if click {
        c.toggle_cam |= l.toggle.contains(mouse);
    }
    c
}

// ---- drawing ---------------------------------------------------------------------------------------------------------

fn text_centered(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_text(text, cx - w * 0.5, y, size, color);
}

fn button(rect: Rect, label: &str, active: bool) {
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, if active { AMBER } else { DIM });
    text_centered(label, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5 + 7.0, 20.0, if active { AMBER } else { INK });
}

/// A hunter silhouette standing on `base_y`, `h` tall, with glowing eyes.
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

/// A cheap deterministic hash for the monitor static.
fn hash(a: u64, b: u64) -> f32 {
    let x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    ((x ^ (x >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 40) as f32 / (1u64 << 24) as f32
}

fn hour_label(run: &DeadAirRun) -> String {
    match run.hour() {
        0 => text("hud_midnight").to_owned(),
        h => fill(text("hud_hour"), &[h.to_string()]),
    }
}

pub fn draw(run: &DeadAirRun, art: &Art) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    let l = run.layout();
    if run.cam_up {
        draw_monitor(run, art, &l, sw, sh);
    } else {
        draw_booth(run, art, sw, sh);
    }
    draw_controls(run, &l);
    draw_hud(run, sw, sh);
    run.view.draw(&run.call, l.call);
    for (i, (note, _)) in run.notes.iter().enumerate() {
        text_centered(note, sw * 0.5, sh - 18.0 - (run.notes.len() - 1 - i) as f32 * 24.0, 22.0, AMBER);
    }
    if run.flash_ms > 0.0 {
        let a = (run.flash_ms / play("flash_ms")) as f32 * play("flash_alpha") as f32;
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, a));
    }
    if let Some(outcome) = run.end {
        draw_result(run, outcome, sw, sh);
    }
}

fn draw_booth(run: &DeadAirRun, art: &Art, sw: f32, sh: f32) {
    let a = art_rect(sw, sh);
    let fit = |w: f32, h: f32| DrawTextureParams { dest_size: Some(vec2(w, h)), ..Default::default() };
    draw_texture_ex(&art.bg, a.x, a.y, WHITE, fit(a.w, a.h));
    draw_texture_ex(&art.fg, a.x, a.y, WHITE, fit(a.w, a.h));
    for side in [Side::Left, Side::Right] {
        if !run.dead.seal_closed(side) {
            continue;
        }
        let q = vent_quad(side).map(|p| to_screen(a, p));
        draw_triangle(q[0], q[1], q[2], SHUTTER);
        draw_triangle(q[0], q[2], q[3], SHUTTER);
        let centre = q.iter().fold(Vec2::ZERO, |acc, p| acc + *p) * 0.25;
        text_centered(text("shut"), centre.x, centre.y + 8.0, 26.0, Color::new(0.85, 0.88, 0.92, 1.0));
    }
}

fn draw_monitor(run: &DeadAirRun, art: &Art, l: &Layout, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.015, 0.03, 0.025, 1.0));
    let f = l.feed;
    let room = run.cam_room();
    let label = run.map.camera(run.cam).map_or("", |c| c.label.as_str());
    match art.cam_picture(room) {
        Some(picture) => {
            let fit = DrawTextureParams { dest_size: Some(vec2(f.w, f.h)), ..Default::default() };
            draw_texture_ex(picture, f.x, f.y, WHITE, fit);
        }
        None => {
            draw_rectangle(f.x, f.y, f.w, f.h, Color::new(0.03, 0.06, 0.05, 1.0));
            draw_rectangle(f.x, f.y + f.h * 0.66, f.w, f.h * 0.34, Color::new(0.04, 0.08, 0.06, 1.0));
            draw_line(f.x, f.y + f.h * 0.66, f.x + f.w, f.y + f.h * 0.66, 2.0, Color::new(0.10, 0.20, 0.16, 1.0));
        }
    }
    let here = run.hunters_in(room);
    if here.is_empty() {
        text_centered(text("empty"), f.x + f.w * 0.5, f.y + f.h * 0.5, 30.0, Color::new(0.30, 0.50, 0.42, 1.0));
    }
    let alpha = ui("hunter_alpha") as f32;
    for (n, who) in here.iter().enumerate() {
        let cx = f.x + f.w * (n as f32 + 1.0) / (here.len() as f32 + 1.0);
        let feet = f.y + f.h * ui("hunter_feet") as f32;
        let h = f.h * ui("hunter_height") as f32;
        // the cut-out fills a box h tall and 0.6 h wide, feet on the box's bottom edge; a missing sprite falls back to the shape
        let rect = Rect::new(cx - h * 0.3, feet - h, h * 0.6, h);
        if !sprites::draw_hunter(who, cam_facing(), rect, Color::new(1.0, 1.0, 1.0, alpha)) {
            figure(cx, feet, h, hunter_color(who), alpha);
        }
        text_centered(name(who), cx, f.y + f.h * ui("hunter_name") as f32, 20.0, INK);
    }
    // static and scanlines
    let tick = (run.anim_ms / ui("static_frame_ms")) as u64;
    for i in 0..ui("static_lines") as u64 {
        let y = f.y + hash(tick, i) * f.h;
        let x = f.x + hash(i, tick) * f.w * 0.7;
        draw_line(x, y, x + hash(tick, i + 99) * f.w * 0.3, y, 1.0, Color::new(0.6, 0.9, 0.8, ui("static_alpha") as f32));
    }
    let mut y = f.y;
    while y < f.y + f.h {
        draw_line(f.x, y, f.x + f.w, y, 1.0, Color::new(0.0, 0.0, 0.0, ui("scanline_alpha") as f32));
        y += ui("scanline_gap") as f32;
    }
    draw_rectangle_lines(f.x, f.y, f.w, f.h, 3.0, Color::new(0.30, 0.55, 0.45, 1.0));
    let cam = run.cam.to_string();
    let caption = fill(text("cam_title"), &[cam.as_str(), label]);
    draw_text(&caption, f.x + 12.0, f.y + f.h - 12.0, 24.0, Color::new(0.55, 0.95, 0.80, 1.0));
    if (run.anim_ms / ui("alarm_blink_ms")) as u64 % 2 == 0 {
        draw_circle(f.x + f.w - 24.0, f.y + 24.0, 7.0, RED_TEXT);
    }

    // the whole station, the active camera lit, a marker for every hunter on the map
    let markers = run.markers();
    let view = run.map_view(&markers);
    run.map.draw(l.map, &view);
    for (i, r) in l.cam_buttons.iter().enumerate() {
        let cam = &run.map.cameras[i];
        button(*r, &cam.id.to_string(), cam.id == run.cam);
    }
}

fn draw_controls(run: &DeadAirRun, l: &Layout) {
    if run.end.is_some() {
        return;
    }
    if run.cam_up {
        button(l.lure, text("btn_lure"), false);
        for (i, r) in l.reboot.iter().enumerate() {
            let system = SYSTEMS[i];
            let busy = if run.rebooting[i] { ui_text("busy_suffix") } else { "" };
            let label = format!("{}  {}{}", system_key(system), system_label(system), busy);
            button(*r, &label, !run.dead.online(system));
        }
    } else {
        button(l.seal[0], text("btn_seal_l"), run.dead.seal_closed(Side::Left));
        button(l.seal[1], text("btn_seal_r"), run.dead.seal_closed(Side::Right));
        if run.has_climate() {
            button(l.appliance[0], text("btn_heater"), run.dead.heater_on());
            button(l.appliance[1], text("btn_fan"), run.dead.fan_on());
        }
    }
    button(l.toggle, text(if run.cam_up { "btn_monitor_close" } else { "btn_monitor" }), run.cam_up);
}

fn draw_hud(run: &DeadAirRun, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    draw_text(&fill(text("hud_night"), &[run.night.to_string()]), 24.0, 30.0, 28.0, INK);
    text_centered(&hour_label(run), sw * 0.5, 32.0, 32.0, AMBER);

    let air = run.air_percent();
    let colour = if (air as f64) < play("air_warn") { RED_TEXT } else { INK };
    let pct = fill(text("hud_air"), &[air.to_string()]);
    let w = measure_text(&pct, None, 28, 1.0).width;
    draw_text(&pct, sw - w - 24.0, 30.0, 28.0, colour);
    let bar = Rect::new(sw - 24.0 - 200.0, 36.0, 200.0, 6.0);
    draw_rectangle(bar.x, bar.y, bar.w, bar.h, Color::new(0.2, 0.2, 0.2, 1.0));
    draw_rectangle(bar.x, bar.y, bar.w * air as f32 / 100.0, bar.h, colour);

    // one status line above the buttons: what is down, and the temperature when there is a climate
    let down: Vec<&str> = SYSTEMS
        .iter()
        .enumerate()
        .filter(|(i, s)| !run.dead.online(**s) || run.rebooting[*i])
        .map(|(_, s)| system_label(*s))
        .collect();
    let down_line = if down.is_empty() { text("hud_all_up").to_owned() } else { fill(text("hud_down"), &[down.join(", ")]) };
    let mut status = down_line;
    if run.has_climate() {
        let temp = format!("{}", run.dead.temperature().round() as i32);
        status = format!("{}     {}", status, fill(text("hud_temp"), &[temp]));
    }
    draw_text(&status, 24.0, sh * 0.74 - 14.0, 20.0, if down.is_empty() { DIM } else { RED_TEXT });

    let hint = text(if run.cam_up { "hint_monitor" } else { "hint_office" });
    text_centered(hint, sw * 0.5, sh - 2.0 - run.notes.len() as f32 * 24.0 - 6.0, 16.0, DIM);
}

/// The end card: the title, its line and the keys.
fn ending(outcome: Outcome, night: i32) -> (String, String, Color) {
    let n = night.to_string();
    let points = points_for(night).to_string();
    match outcome {
        Outcome::Win => (fill(text("win_title"), &[&n]), fill(text("win_line"), &[&points]), AMBER),
        Outcome::Caught(Hunter::Mavrine) => (text("air_title").to_owned(), text("air_line").to_owned(), RED_TEXT),
        Outcome::Caught(Hunter::Solyth) => (text("hot_title").to_owned(), text("hot_line").to_owned(), RED_TEXT),
        Outcome::Caught(Hunter::Wynelle) => (text("cold_title").to_owned(), text("cold_line").to_owned(), RED_TEXT),
        Outcome::Caught(by) => (text("caught_title").to_owned(), fill(text("caught_line"), &[name(hunter_id(by))]), RED_TEXT),
    }
}

fn draw_result(run: &DeadAirRun, outcome: Outcome, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let (title, line, colour) = ending(outcome, run.night);
    text_centered(&title, sw * 0.5, sh * 0.40, 64.0, colour);
    text_centered(&line, sw * 0.5, sh * 0.40 + 48.0, 28.0, INK);
    let next = match outcome {
        Outcome::Win if run.night < nights() => text("next_night"),
        Outcome::Win => text("back_hub"),
        Outcome::Caught(_) => text("try_again"),
    };
    if run.end_ms >= play("result_lock_ms") {
        text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
        text_centered(text("esc_hub"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
    }
}

// ---- scripted runs (the --mode lure --screenshot flag) ---------------------------------------------------------------

/// What a `--mode lure --screenshot` run does before the picture: the night, how many fixed frames to run, the booth or
/// monitor state, and optionally to keep going until the first call line has this many letters showing.
#[derive(Clone, Debug, Default)]
pub struct ShotPlan {
    pub night: i32,
    pub frames: u32,
    /// Monitor up on this camera from the first frame.
    pub cam: Option<u32>,
    pub seal_l: bool,
    pub seal_r: bool,
    pub heater: bool,
    pub fan: bool,
    pub call_chars: Option<usize>,
}

impl ShotPlan {
    /// Reads the flags after `--mode lure --screenshot PATH`.
    pub fn from_args(args: &[String]) -> Self {
        let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let number = |flag: &str| value(flag).and_then(|v| v.parse::<u32>().ok());
        let has = |flag: &str| args.iter().any(|a| a == flag);
        Self {
            night: number("--night").map_or(1, |n| n as i32),
            frames: number("--frames").unwrap_or(play("shot_frames") as u32),
            cam: number("--cam"),
            seal_l: has("--seal-l"),
            seal_r: has("--seal-r"),
            heater: has("--heater"),
            fan: has("--fan"),
            call_chars: number("--call-chars").map(|n| n as usize),
        }
    }
}

/// Steps a fresh shift by the plan at a fixed 60 fps and returns it ready to draw. Nothing is saved or played.
pub fn run_script(calls: &CallsData, seed: i64, plan: &ShotPlan) -> DeadAirRun {
    let mut run = DeadAirRun::new(calls, plan.night, seed, false);
    let mut first = Controls {
        seal_l: plan.seal_l,
        seal_r: plan.seal_r,
        heater: plan.heater,
        fan: plan.fan,
        toggle_cam: plan.cam.is_some(),
        pick_cam: plan.cam,
        ..Controls::default()
    };
    let step = play("shot_frame_ms");
    let max_frames = play("shot_max_frames") as u32;
    let mut frame = 0u32;
    while frame < max_frames {
        let reveal_done = plan.call_chars.map_or(true, |k| {
            let call = &run.call;
            call.open && call.line == 0 && call.text != CALL_INCOMING_TEXT && call.text.chars().count() >= k
        });
        if (frame >= plan.frames && reveal_done) || run.end.is_some() {
            break;
        }
        run.update(step, &std::mem::take(&mut first));
        frame += 1;
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(night: i32) -> DeadAirRun {
        DeadAirRun::new(&CallsData::load(), night, 7, false)
    }

    /// A scripted player: reboots what is down, seals the vent Xivvy waits at and opens it once she has gone.
    fn policy(run: &DeadAirRun) -> Controls {
        let down = SYSTEMS.iter().copied().find(|s| !run.dead.online(*s) && !run.rebooting[*s as usize]);
        let mut c = Controls::default();
        if run.cam_up {
            match down {
                Some(system) => c.reboot = Some(system),
                None => c.toggle_cam = true,
            }
            return c;
        }
        if down.is_some() {
            c.toggle_cam = true;
            return c;
        }
        for side in [Side::Left, Side::Right] {
            let at = run.dead.killer_room() == side.office();
            let closed = run.dead.seal_closed(side);
            if at != closed {
                match side {
                    Side::Left => c.seal_l = true,
                    Side::Right => c.seal_r = true,
                }
            }
        }
        c
    }

    /// Steps the shift with the policy until it has an ending; panics if it takes more than `limit` steps.
    fn play_to_end(run: &mut DeadAirRun, limit: usize) -> usize {
        for step in 0..limit {
            let c = policy(run);
            run.update(16.0, &c);
            if run.end.is_some() {
                return step;
            }
        }
        panic!("the night did not end within {limit} steps (night {})", run.night);
    }

    #[test]
    fn the_layouts_fit_the_window_and_the_buttons_do_not_overlap() {
        for (sw, sh) in [(1280.0, 720.0), (800.0, 600.0)] {
            let l = layout(sw, sh, 8);
            let booth = [l.seal[0], l.seal[1], l.appliance[0], l.appliance[1], l.toggle];
            let monitor = [l.lure, l.reboot[0], l.reboot[1], l.reboot[2], l.reboot[3], l.reboot[4], l.toggle];
            for set in [&booth[..], &monitor[..]] {
                for (i, a) in set.iter().enumerate() {
                    assert!(a.x >= 0.0 && a.right() <= sw && a.bottom() <= sh, "{a:?} leaves the window");
                    for b in &set[i + 1..] {
                        assert!(a.intersect(*b).is_none(), "{a:?} overlaps {b:?}");
                    }
                }
            }
            assert_eq!(l.cam_buttons.len(), 8);
            assert!(l.cam_buttons.iter().all(|r| r.right() <= l.map.right() + 0.5 && r.bottom() <= l.reboot[0].y));
            assert!(l.call.intersect(l.feed).is_none() && l.call.intersect(l.map).is_none(), "the call box clears the panels");
            assert!(l.call.y >= 44.0, "the call box clears the HUD bar");
        }
    }

    #[test]
    fn every_camera_covers_a_room_the_map_knows() {
        let r = run(1);
        for id in r.cam_ids() {
            let mut one = run(1);
            one.update(16.0, &Controls { toggle_cam: true, pick_cam: Some(id), ..Controls::default() });
            assert!(r.map.room(one.cam_room()).is_some(), "camera {id}");
        }
    }

    #[test]
    fn the_booth_keys_seal_the_vents_and_the_heater_needs_a_climate() {
        let mut r = run(1);
        r.update(16.0, &Controls { seal_l: true, ..Controls::default() });
        assert!(r.dead.seal_closed(Side::Left));
        r.update(16.0, &Controls { seal_l: true, ..Controls::default() });
        assert!(!r.dead.seal_closed(Side::Left), "a second press opens it");

        // night 1 has no climate, so the heater says so
        r.update(16.0, &Controls { heater: true, ..Controls::default() });
        assert!(r.notes.last().is_some_and(|n| n.0.contains("not running")), "{:?}", r.notes);

        // night 4 has Wynelle on, so the heater runs
        let mut hot = run(4);
        assert!(hot.has_climate());
        hot.update(16.0, &Controls { heater: true, ..Controls::default() });
        assert!(hot.dead.heater_on());
    }

    #[test]
    fn the_lure_needs_the_monitor_and_goes_to_the_room_on_screen() {
        let mut booth = run(2);
        booth.update(16.0, &Controls { lure: true, ..Controls::default() });
        assert_eq!(booth.dead.lure_target(), None, "no lure from the booth");

        let mut monitor = run(2);
        monitor.update(16.0, &Controls { toggle_cam: true, lure: true, ..Controls::default() });
        assert_eq!(monitor.dead.lure_target(), Some("lobby"), "camera 1 covers reception");
    }

    #[test]
    fn a_scripted_shift_reaches_an_ending_on_every_night() {
        for night in 1..=nights() {
            let mut r = DeadAirRun::new(&CallsData::load(), night, 20_261_103 + i64::from(night), false);
            let steps = play_to_end(&mut r, 60_000);
            assert!(r.end.is_some(), "night {night} ended after {steps} steps");
        }
    }

    #[test]
    fn a_shift_that_is_kept_alive_wins_on_the_clock_and_the_call_plays_through() {
        let mut r = run(1);
        let mut saw_open = false;
        for _ in 0..60_000 {
            let c = policy(&r);
            r.update(16.0, &c);
            saw_open |= r.call.open;
            if r.end.is_some() {
                break;
            }
        }
        assert!(saw_open, "the call rang");
        assert!(r.call.line >= 0, "the first line was typed");
        if r.end == Some(Outcome::Win) {
            assert!(r.call.done, "the lines ran out before the 6 AM win");
            assert!(r.dead.elapsed_ms() >= data::num(rules(), "night_ms"));
        }
    }

    #[test]
    fn a_broken_system_is_rebooted_and_comes_back() {
        let mut r = run(1);
        let mut rebooted = false;
        let mut back = false;
        for _ in 0..60_000 {
            let c = policy(&r);
            r.update(16.0, &c);
            rebooted |= r.rebooting.iter().any(|b| *b);
            back |= rebooted && r.rebooting.iter().all(|b| !*b);
            if r.end.is_some() {
                break;
            }
        }
        assert!(rebooted, "a phantom broke something to reboot");
        assert!(back, "the reboot finished");
    }

    #[test]
    fn every_label_the_driver_reads_is_in_the_data() {
        for key in [
            "hint_office", "hint_monitor", "btn_seal_l", "btn_seal_r", "btn_heater", "btn_fan", "btn_lure", "btn_monitor",
            "btn_monitor_close", "shut", "side_left", "side_right", "hud_night", "hud_air", "hud_temp", "hud_down", "hud_all_up",
            "hud_midnight", "hud_hour", "cam_title", "empty", "lure_ok", "lure_fail", "seal_on", "seal_off", "toggle_on",
            "toggle_off", "not_running", "rebooting", "not_down", "fixed", "broken_phantom", "broken_strain", "phantom_seen",
            "retreat", "at_office", "muted", "save_failed", "win_title", "win_line", "caught_title", "caught_line", "air_title",
            "air_line", "hot_title", "hot_line", "cold_title", "cold_line", "next_night", "back_hub", "try_again", "esc_hub",
        ] {
            assert!(!text(key).is_empty(), "{key}");
        }
        for name_id in ["xivvy", "ferryn", "briselle", "mavrine", "solyth", "wynelle"] {
            assert!(!name(name_id).is_empty());
        }
        assert_eq!(fill("a {} b {}", &["x", "y"]), "a x b y");
        assert_eq!(art_size(), (480.0, 270.0));
        assert_eq!(vent_quad(Side::Left)[0], vec2(26.0, 131.8));
        assert!(!ui_text("busy_suffix").is_empty());
        assert!(matches!(cam_facing(), Facing::Front | Facing::Back));
    }

    #[test]
    fn every_hunter_the_monitor_draws_has_a_sprite_and_every_camera_a_picture() {
        for id in ["xivvy", "ferryn", "briselle", "mavrine"] {
            assert!(data::HUNTER_SPRITE_FILES.iter().any(|(h, _, _)| *h == id), "no sprite for {id}");
        }
        let map = building_map::load(DATA).expect("the dead air building map is embedded");
        let art_rooms: Vec<&str> = CAM_PICTURES.iter().map(|(room, _)| *room).collect();
        for cam in &map.cameras {
            let room = map.rooms[cam.covers[0]].id.as_str();
            assert!(art_rooms.contains(&room), "camera {} covers {room} with no picture", cam.id);
        }
    }

    #[test]
    fn the_ui_block_holds_every_number_the_driver_draws_with() {
        for key in ["static_frame_ms", "static_lines", "static_alpha", "scanline_gap", "scanline_alpha", "alarm_blink_ms", "broken_flash_scale", "hunter_alpha", "hunter_height", "hunter_feet", "hunter_name"] {
            assert!(ui(key) > 0.0, "{key}");
        }
    }
}
