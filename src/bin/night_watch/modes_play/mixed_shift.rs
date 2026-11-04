//! Mixed Shift on screen: the shell's driver for `modes::mixed_shift`. `MixedRun` holds the shift select, the night and
//! the ending. Each frame it maps the player's `Controls` (doors Z and X, vent seals V, N and B held, vent lights G, H
//! and Y tapped, wind R held, monitor U, cameras 1 to 9, the arrow keys and the mouse) onto the rules' setters, runs the
//! call script from `data/rules/mixed_shift.json`, turns the events `step` returns into notes, and saves a cleared shift.
//! `draw` paints the transit hub or the monitor (camera feed beside the building map), plus the HUD, the call box, the
//! shift select and the endings. Everything but `poll_controls`, `draw` and `Art` is plain state, so tests run headless.

use macroquad::prelude::*;

use crate::building_map::{self, BuildingMap, MapView};
use crate::calls_view::CallView;
use crate::data;
use crate::framework::CallScript;
use crate::modes::mixed_shift::{Beat, Event, Kind, MixedShift, Mood, Outcome, Room, Side, SpikePhase, VentLightError};
use crate::save::NightWatchSave;
use crate::sprites::{self, Facing};
use serde_json::Value;

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);
const GREEN_TEXT: Color = Color::new(0.55, 0.95, 0.80, 1.0);

/// The mode id in the data registry and on the building map.
pub const MODE: &str = "mixed_shift";
/// The ENDLESS best hour is kept under its own key, apart from the timed shifts' points.
pub const ENDLESS_BEST: &str = "mixed_shift_endless";

const CAM_COUNT: u32 = 9;

/// The shell's timings and labels: the `ui` block of `data/rules/mixed_shift.json`. Timings are in milliseconds; a
/// frame longer than `max_step_ms` (a dragged window, a debugger stop) is cut so no threat skips its tell, the ending
/// ignores ENTER for `result_lock_ms` so a key held in panic does not skip it, and `script_fps` sets the fixed step of
/// the scripted screenshot run.
fn ui() -> &'static Value {
    &data::rules(MODE)["ui"]
}

/// A timing in milliseconds from the `ui` block, by its key.
fn ui_ms(key: &str) -> f64 {
    data::num(ui(), key)
}

/// A label from the `ui` block by dotted path (`labels.cleared`, `rooms.stage`, `notes.arrive`).
fn label(path: &str) -> &'static str {
    data::text(ui(), path)
}

/// Fills the `{}` holes of a label with `vals`, in order.
fn fill(tpl: &str, vals: &[&str]) -> String {
    let mut out = String::with_capacity(tpl.len() + 16);
    let mut parts = tpl.split("{}");
    out.push_str(parts.next().unwrap_or(""));
    for (part, val) in parts.zip(vals.iter().copied().chain(std::iter::repeat(""))) {
        out.push_str(val);
        out.push_str(part);
    }
    out
}

/// The hunter id a pool index names, from the `pool` list of `data/cast/mixed_shift.json`.
fn hunter_id(pool: usize) -> &'static str {
    data::text(&data::list(data::cast(MODE), "pool")[pool], "id")
}

/// Each threat's colour by its pool index (the order of `data/cast/mixed_shift.json`).
const PALETTE: [Color; 8] = [
    Color::new(0.85, 0.75, 0.35, 1.0),
    Color::new(0.62, 0.50, 0.92, 1.0),
    Color::new(0.95, 0.65, 0.30, 1.0),
    Color::new(0.55, 0.85, 0.45, 1.0),
    Color::new(0.92, 0.55, 0.70, 1.0),
    Color::new(0.35, 0.80, 0.78, 1.0),
    Color::new(0.90, 0.25, 0.25, 1.0),
    Color::new(0.45, 0.60, 0.95, 1.0),
];

/// Every room the rules can place a threat in, with its id on the building map.
const ROOM_IDS: [(Room, &str); 14] = [
    (Room::Stage, "stage"),
    (Room::Dining, "dining"),
    (Room::Backstage, "backstage"),
    (Room::WestHall, "westhall"),
    (Room::Closet, "closet"),
    (Room::Kitchen, "kitchen"),
    (Room::EastHall, "easthall"),
    (Room::Corner, "corner"),
    (Room::Vent, "vent"),
    (Room::DoorL, "doorL"),
    (Room::DoorR, "doorR"),
    (Room::VentL, "ventL"),
    (Room::VentR, "ventR"),
    (Room::VentT, "ventT"),
];

/// Camera 1 to 9 and the room each one watches (the map's cameras, in order).
const CAM_ROOMS: [Room; 9] = [
    Room::Stage,
    Room::Dining,
    Room::Backstage,
    Room::WestHall,
    Room::Closet,
    Room::Kitchen,
    Room::EastHall,
    Room::Corner,
    Room::Vent,
];

/// A room's name on screen, from `ui.rooms` (keyed by its id on the building map).
fn room_label(room: Room) -> &'static str {
    let id = ROOM_IDS.iter().find(|(r, _)| *r == room).map_or("", |(_, id)| id);
    label(&format!("rooms.{id}"))
}

fn side_name(side: Side) -> &'static str {
    label(match side {
        Side::L => "sides.l",
        Side::R => "sides.r",
        Side::T => "sides.t",
    })
}

/// The index of a side in the per-side arrays here: 0 left, 1 right, 2 top (the order of `Side::ALL`).
fn side_ix(side: Side) -> usize {
    match side {
        Side::L => 0,
        Side::R => 1,
        Side::T => 2,
    }
}

fn place(room: Option<Room>, side: Option<Side>) -> String {
    match (room, side) {
        (Some(r), _) => room_label(r).to_string(),
        (None, Some(s)) => fill(label("side_place"), &[side_name(s)]),
        (None, None) => label("building").to_string(),
    }
}

fn hour_label(hour: u32) -> String {
    if hour == 0 {
        label("hour.midnight").to_owned()
    } else {
        fill(label("hour.am"), &[&hour.to_string()])
    }
}

fn threat_color(pool: usize) -> Color {
    PALETTE[pool % PALETTE.len()]
}

fn mood_word(mood: Mood) -> &'static str {
    label(match mood {
        Mood::Patient => "moods.patient",
        Mood::Hungry => "moods.hungry",
        Mood::Shy => "moods.shy",
        Mood::Reckless => "moods.reckless",
    })
}

/// The short tag a threat's kind shows under its name on the monitor.
fn kind_tag(kind: Kind) -> &'static str {
    label(match kind {
        Kind::Counter => "kinds.counter",
        Kind::Watched => "kinds.watched",
        Kind::Monitor => "kinds.monitor",
        Kind::Path => "kinds.path",
        Kind::Vent => "kinds.vent",
        Kind::Winder => "kinds.winder",
    })
}

/// One shift row of the select screen, read from `data/rules/mixed_shift.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShiftRow {
    pub name: String,
    pub threats: (u32, u32),
    pub tell_s: f64,
    pub endless: bool,
}

fn shift_rows() -> Vec<ShiftRow> {
    data::list(data::rules(MODE), "shifts")
        .iter()
        .map(|s| ShiftRow {
            name: data::text(s, "name").to_owned(),
            threats: (data::int(s, "roster.0") as u32, data::int(s, "roster.1") as u32),
            tell_s: data::num(s, "tell_ms") / 1000.0,
            endless: s["endless"].as_bool().unwrap_or(false),
        })
        .collect()
}

/// The call the night opens with: its ring time and lines, from the rules file's `call` block.
fn call_script() -> CallScript {
    let call = &data::rules(MODE)["call"];
    let lines = data::list(call, "lines").iter().filter_map(|l| l.as_str().map(str::to_owned)).collect();
    CallScript::new(lines, data::num(call, "at_s"))
}

/// What the player did this frame. Index the per-side arrays by side (0 left, 1 right, 2 top); doors use 0 and 1.
/// The door and light fields are presses; the seals and the wind are held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub door: [bool; 2],
    pub seal: [bool; 3],
    pub light: [bool; 3],
    pub wind: bool,
    pub toggle_monitor: bool,
    pub pick_cam: Option<u32>,
    /// -1 or 1 steps the monitor to the previous or next camera.
    pub cam_step: i32,
    /// -1 or 1 moves the highlight on the shift select.
    pub row_step: i32,
    /// A click on a row of the shift select starts that shift.
    pub pick_row: Option<usize>,
    pub confirm: bool,
    pub back: bool,
}

/// Where the shell goes after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Stay,
    Hub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Select,
    Night,
}

/// How a night ended: the outcome, the points, the ENDLESS hours (caught only), the hour reached and the threat's name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct End {
    outcome: Outcome,
    points: u32,
    best: Option<u32>,
    hour: u32,
    who: &'static str,
}

pub struct MixedRun {
    rows: Vec<ShiftRow>,
    best_endless: u32,
    phase: Phase,
    pick: usize,
    shift: usize,
    seed: i64,
    nights: u32,
    night: Option<MixedShift>,
    map: &'static BuildingMap,
    persist: bool,
    /// The doors the player has asked shut (left, right). The rules drop a door's request when it pops open.
    want: [bool; 2],
    door_shut: [bool; 2],
    /// The vent seals the player is holding (left, right, top).
    seal_held: [bool; 3],
    vent_shut: [bool; 3],
    light_ms: [f64; 3],
    cam_up: bool,
    cam: u32,
    end: Option<End>,
    end_ms: f64,
    flash_ms: f64,
    notes: Vec<(String, f64)>,
    call: CallScript,
    view: CallView,
    anim_ms: f64,
    /// Where a persisting run writes its save (None: the game's own progress file). Tests point it at a temp file.
    save_path: Option<std::path::PathBuf>,
    /// Sound events raised since the shell last drained them (`take_sound_events`): the event id and its pan.
    sounds: Vec<(&'static str, f32)>,
    was_winding: bool,
}

impl MixedRun {
    /// The shift select. `persist` lets a cleared shift write the save file (off for tests and screenshots).
    /// Starts shift `shift` (0-based) straight away, as the select screen does.
    pub fn with_shift(shift: usize, seed: i64, persist: bool) -> Self {
        let mut run = Self::new(seed, persist);
        run.begin(shift);
        run
    }

    pub fn new(seed: i64, persist: bool) -> Self {
        let map = building_map::load(MODE).expect("the mixed shift building map is embedded");
        let mut view = CallView::new();
        view.player.enabled = persist;
        let best_endless = if persist { NightWatchSave::load().best_score(ENDLESS_BEST) } else { 0 };
        Self {
            rows: shift_rows(),
            best_endless,
            phase: Phase::Select,
            pick: 0,
            shift: 0,
            seed,
            nights: 0,
            night: None,
            map,
            persist,
            want: [false; 2],
            door_shut: [false; 2],
            seal_held: [false; 3],
            vent_shut: [false; 3],
            light_ms: [0.0; 3],
            cam_up: false,
            cam: 1,
            end: None,
            end_ms: 0.0,
            flash_ms: 0.0,
            notes: Vec::new(),
            call: call_script(),
            view,
            anim_ms: 0.0,
            save_path: None,
            sounds: Vec::new(),
            was_winding: false,
        }
    }

    /// Makes this run persist its clears to `path` instead of the game's progress file (tests and tools).
    #[cfg(test)]
    fn persist_to(&mut self, path: std::path::PathBuf) {
        self.best_endless = NightWatchSave::load_from(&path).best_score(ENDLESS_BEST);
        self.persist = true;
        self.save_path = Some(path);
    }

    /// The sound events raised since the last call, as (event id, pan). HOOK: the shell passes each to
    /// `nw_sound::event("mixed", id, pan)`; `sounds` in `data/rules/mixed_shift.json` names the cue behind each id.
    #[allow(dead_code)]
    pub fn take_sound_events(&mut self) -> Vec<(&'static str, f32)> {
        std::mem::take(&mut self.sounds)
    }

    /// Starts shift `shift` (0 QUIET to 4 ENDLESS) on a fresh seed.
    fn begin(&mut self, shift: usize) {
        let shift = shift.min(self.rows.len().saturating_sub(1));
        self.nights += 1;
        let seed = self.seed.wrapping_add(i64::from(self.nights).wrapping_mul(7_919));
        self.shift = shift;
        self.pick = shift;
        self.night = Some(MixedShift::start(seed, shift));
        self.phase = Phase::Night;
        self.want = [false; 2];
        self.door_shut = [false; 2];
        self.seal_held = [false; 3];
        self.vent_shut = [false; 3];
        self.light_ms = [0.0; 3];
        self.cam_up = false;
        self.cam = 1;
        self.end = None;
        self.end_ms = 0.0;
        self.flash_ms = 0.0;
        self.notes.clear();
        self.sounds.clear();
        self.was_winding = false;
        self.call = call_script();
    }

    fn to_select(&mut self) {
        self.night = None;
        self.end = None;
        self.phase = Phase::Select;
    }

    fn note(&mut self, text: impl Into<String>) {
        self.notes.push((text.into(), ui_ms("note_ms")));
        if self.notes.len() > 3 {
            self.notes.remove(0);
        }
    }

    /// Advances the shell by `dt_ms` with the frame's controls.
    pub fn update(&mut self, dt_ms: f64, c: &Controls) -> Flow {
        self.anim_ms += dt_ms;
        self.flash_ms = (self.flash_ms - dt_ms).max(0.0);
        for l in self.light_ms.iter_mut() {
            *l = (*l - dt_ms).max(0.0);
        }
        for n in self.notes.iter_mut() {
            n.1 -= dt_ms;
        }
        self.notes.retain(|n| n.1 > 0.0);
        match self.phase {
            Phase::Select => self.update_select(c),
            Phase::Night => {
                self.update_night(dt_ms, c);
                Flow::Stay
            }
        }
    }

    fn update_select(&mut self, c: &Controls) -> Flow {
        if c.back {
            return Flow::Hub;
        }
        let count = self.rows.len().max(1) as i32;
        if c.row_step != 0 {
            self.pick = (self.pick as i32 + c.row_step).rem_euclid(count) as usize;
        }
        if let Some(row) = c.pick_row {
            if row < self.rows.len() {
                self.begin(row);
            }
        } else if c.confirm {
            self.begin(self.pick);
        }
        Flow::Stay
    }

    fn update_night(&mut self, dt_ms: f64, c: &Controls) {
        if c.back {
            self.to_select();
            return;
        }
        if let Some(end) = self.end {
            self.end_ms += dt_ms;
            if c.confirm && self.end_ms >= ui_ms("result_lock_ms") {
                match end.outcome {
                    Outcome::Win if self.shift + 1 < self.rows.len() => self.begin(self.shift + 1),
                    Outcome::Win => self.to_select(),
                    _ => self.begin(self.shift),
                }
            }
            return;
        }
        let Some(mut night) = self.night.take() else {
            self.to_select();
            return;
        };
        self.play_frame(&mut night, dt_ms, c);
        if night.outcome() != Outcome::Ongoing {
            let who = match night.outcome() {
                Outcome::Caught(p) => night.threat(p).name(),
                _ => "",
            };
            let end = End {
                outcome: night.outcome(),
                points: night.points(),
                best: night.best_hours(),
                hour: night.hour_reached(),
                who,
            };
            self.flash_ms = if matches!(end.outcome, Outcome::Caught(_)) { ui_ms("flash_ms") } else { 0.0 };
            self.end = Some(end);
            self.end_ms = 0.0;
            self.save_end(end);
        }
        self.night = Some(night);
        self.view.update(&self.call);
    }

    /// One frame of the night: the player's input, the call, the rules' step and the events it returns.
    fn play_frame(&mut self, night: &mut MixedShift, dt_ms: f64, c: &Controls) {
        for ix in 0..2 {
            if c.door[ix] {
                self.want[ix] = !self.want[ix];
            }
            self.door_shut[ix] = night.set_door(Side::ALL[ix], self.want[ix]);
        }
        for ix in 0..3 {
            let side = Side::ALL[ix];
            self.seal_held[ix] = c.seal[ix];
            self.vent_shut[ix] = night.set_vent(side, c.seal[ix]);
            if c.light[ix] {
                match night.light_vent(side) {
                    Ok(seen) if seen.is_empty() => self.light_ms[ix] = ui_ms("light_ms"),
                    Ok(seen) => {
                        self.light_ms[ix] = ui_ms("light_ms");
                        let names: Vec<&str> = seen.iter().map(|&p| night.threat(p).name()).collect();
                        let names = names.join(" and ");
                        self.note(fill(label("notes.light_catch"), &[side_name(side), &names]));
                    }
                    Err(VentLightError::Shut) => self.note(fill(label("notes.vent_shut"), &[side_name(side)])),
                    Err(VentLightError::Cooling) => self.note(label("notes.lamp_cooling")),
                    Err(VentLightError::Over) => {}
                }
            }
        }
        night.set_wind(c.wind);
        if c.toggle_monitor {
            self.cam_up = !self.cam_up;
        }
        if let Some(id) = c.pick_cam {
            self.cam = id.clamp(1, CAM_COUNT);
        }
        if c.cam_step != 0 {
            self.cam = ((self.cam as i32 - 1 + c.cam_step).rem_euclid(CAM_COUNT as i32) + 1) as u32;
        }
        night.set_monitor(self.cam_up);
        night.set_room(CAM_ROOMS[(self.cam - 1) as usize]);
        let winding = c.wind && self.cam_up;
        if winding && !self.was_winding {
            self.sounds.push(("box_winding", 0.0));
        }
        self.was_winding = winding;

        let dt = dt_ms.min(ui_ms("max_step_ms"));
        self.call.update(night.elapsed_ms(), dt);
        let events = night.step(dt);
        for ev in &events {
            if let Some(sound) = sound_for(ev, night) {
                self.sounds.push(sound);
            }
            if let Some(text) = describe(ev, night) {
                self.note(text);
            }
            match ev {
                Event::Strain { side } if *side != Side::T => self.want[side_ix(*side)] = false,
                Event::Kill { .. } => self.flash_ms = ui_ms("flash_ms"),
                _ => {}
            }
        }
    }

    /// Writes a cleared shift or the ENDLESS best hour (best effort; runs that do not persist skip it).
    fn save_end(&mut self, end: End) {
        if !self.persist {
            return;
        }
        let mut save = match &self.save_path {
            Some(path) => NightWatchSave::load_from(path),
            None => NightWatchSave::load(),
        };
        match end.outcome {
            Outcome::Win => {
                save.record_level_clear(MODE, self.shift as u32 + 1);
                save.record_best(MODE, end.points);
                save.record_night_best(MODE, self.shift as u32 + 1, end.points);
            }
            Outcome::Caught(_) => match end.best {
                Some(hours) => {
                    save.record_best(ENDLESS_BEST, hours);
                    self.best_endless = self.best_endless.max(hours);
                }
                None => return,
            },
            Outcome::Ongoing => return,
        }
        let saved = match &self.save_path {
            Some(path) => save.save_to(path),
            None => save.save(),
        };
        if saved.is_err() {
            self.note(label("notes.save_failed"));
        }
    }

}

/// Every sound event id the shell raises. Each must have a cue in `sounds` of `data/rules/mixed_shift.json`.
#[cfg(test)]
const SOUND_IDS: [&str; 12] = [
    "arrive", "vent_knock", "door_bang", "counter_tell", "box_winding", "box_wound", "box_stop", "door_pop", "caught", "dawn",
    "decoy", "echo",
];

/// The cue behind a sound event id, from the rules' `sounds` map (None for an id the rules do not know).
#[allow(dead_code)]
pub fn cue_for_event(id: &str) -> Option<&'static str> {
    data::rules(MODE)["sounds"][id].as_str()
}

fn pan_of(side: Option<Side>) -> f32 {
    match side {
        Some(Side::L) => -0.6,
        Some(Side::R) => 0.6,
        _ => 0.0,
    }
}

/// The sound for an event: the tells (a vent knock, a door bang), the music box, a threat's arrival and the ending.
/// Pure: the id and pan only; `cue_for_event` turns the id into a cue.
fn sound_for(ev: &Event, night: &MixedShift) -> Option<(&'static str, f32)> {
    Some(match ev {
        Event::Arrive { side, .. } => ("arrive", pan_of(*side)),
        Event::Tell { pool, side, .. } => match (night.threat(*pool).kind(), side) {
            (Kind::Vent, _) => ("vent_knock", pan_of(*side)),
            (_, Some(_)) => ("door_bang", pan_of(*side)),
            (_, None) => ("counter_tell", 0.0),
        },
        Event::Wound { .. } => ("box_wound", 0.0),
        Event::Stop { .. } => ("box_stop", 0.0),
        Event::Strain { side } => ("door_pop", pan_of(Some(*side))),
        Event::Decoy { side, .. } => ("decoy", pan_of(Some(*side))),
        Event::Echo { .. } => ("echo", 0.0),
        Event::Kill { .. } => ("caught", 0.0),
        Event::Win => ("dawn", 0.0),
        _ => return None,
    })
}

/// The notes for the events a player can act on or must notice. Pure: the names come from the night's threats.
fn describe(ev: &Event, night: &MixedShift) -> Option<String> {
    let name = |pool: usize| night.threat(pool).name();
    let note = |key: &str, vals: &[&str]| fill(label(&format!("notes.{key}")), vals);
    Some(match ev {
        Event::Arrive { pool, room, side } => note("arrive", &[name(*pool), &place(*room, *side)]),
        Event::Tell { pool, side, .. } => match side {
            Some(s) => note("tell_side", &[name(*pool), side_name(*s)]),
            None => note("tell", &[name(*pool)]),
        },
        Event::Turned { pool, room } => note("turned", &[name(*pool), room_label(*room)]),
        Event::Answered { pool, .. } => note("answered", &[name(*pool)]),
        Event::Gone { pool } => note("gone", &[name(*pool)]),
        Event::Mood { pool, mood } => note("mood", &[name(*pool), mood_word(*mood)]),
        Event::Stop { pool } => note("stop", &[name(*pool)]),
        Event::Wound { pool } => note("wound", &[name(*pool)]),
        Event::Feint { room, side } => note("feint", &[&place(*room, *side)]),
        Event::Strain { side } => note("strain", &[side_name(*side)]),
        Event::DoorShut { side } => note("door_shut", &[side_name(*side)]),
        Event::VentShut { side } => note("vent_shut", &[side_name(*side)]),
        Event::VentOpen { side } => note("vent_open", &[side_name(*side)]),
        Event::VentStrain { side } => note("vent_strain", &[side_name(*side)]),
        Event::Decoy { side, .. } => note("decoy", &[side_name(*side)]),
        Event::Chaos { start: true, .. } => note("chaos", &[]),
        Event::Rotate { .. } => note("rotate", &[]),
        Event::Spike(SpikePhase::Start) => note("spike_start", &[]),
        Event::Spike(SpikePhase::End) => note("spike_end", &[]),
        _ => return None,
    })
}

// ---- layout ----------------------------------------------------------------------------------------------------

/// Every clickable or drawn region for a window of `sw` by `sh`. Pure, so the click tests and the paint agree.
struct Layout {
    /// Door buttons by side (left, right).
    doors: [Rect; 2],
    /// Seal and light buttons by side (left, right, top).
    seals: [Rect; 3],
    lights: [Rect; 3],
    monitor: Rect,
    wind: Rect,
    cams: Vec<Rect>,
    feed: Rect,
    map: Rect,
    call: Rect,
    /// The two doorways on the hub picture, by side (left, right).
    doorway: [Rect; 2],
}

fn layout(sw: f32, sh: f32) -> Layout {
    let pad = 24.0;
    let gap = 8.0;
    let row_y = sh * 0.76;
    let bw = (sw - 2.0 * pad - 9.0 * gap) / 10.0;
    // ten buttons in one row: door L, seal L, light L, seal T, light T, monitor, wind, seal R, light R, door R
    let slot = |i: usize| Rect::new(pad + i as f32 * (bw + gap), row_y, bw, 44.0);
    let call_w = (sw * 0.56).clamp(420.0, 760.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let top = call.bottom() + 8.0;
    let panels_h = (sh * 0.76 - 60.0 - top - 10.0).max(60.0);
    let feed = Rect::new(pad, top, (sw - 3.0 * pad) * 0.5, panels_h);
    let map = Rect::new(feed.x + feed.w + pad, top, feed.w, panels_h);
    let cell = ((map.w - gap * (CAM_COUNT as f32 - 1.0)) / CAM_COUNT as f32).min(64.0);
    let cams = (0..CAM_COUNT as usize)
        .map(|i| Rect::new(map.x + i as f32 * (cell + gap), map.y + map.h + 10.0, cell, 38.0))
        .collect();
    Layout {
        doors: [slot(0), slot(9)],
        seals: [slot(1), slot(7), slot(3)],
        lights: [slot(2), slot(8), slot(4)],
        monitor: slot(5),
        wind: slot(6),
        cams,
        feed,
        map,
        call,
        doorway: [Rect::new(sw * 0.03, sh * 0.15, sw * 0.18, sh * 0.58), Rect::new(sw * 0.79, sh * 0.15, sw * 0.18, sh * 0.58)],
    }
}

/// The shift select rows: one per shift, stacked under the title.
fn select_row(i: usize, sw: f32, sh: f32) -> Rect {
    Rect::new(sw * 0.5 - 320.0, sh * 0.30 + i as f32 * 66.0, 640.0, 56.0)
}

impl MixedRun {
    fn layout(&self) -> Layout {
        layout(screen_width(), screen_height())
    }

    fn cam_room(&self) -> Room {
        CAM_ROOMS[(self.cam - 1) as usize]
    }

    /// Every threat standing on the map as (room id, threat name).
    fn markers(night: &MixedShift) -> Vec<(&'static str, &'static str)> {
        ROOM_IDS.iter().flat_map(|&(room, id)| night.occupants(room).into_iter().map(move |p| (id, night.threat(p).name()))).collect()
    }
}

// ---- input -----------------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse. Z and X, the camera digits, U and the arrows are presses; V, N, B and R are
/// held; G, H and Y tap the vent lights.
pub fn poll_controls(run: &MixedRun) -> Controls {
    let (sw, sh) = (screen_width(), screen_height());
    let mouse = Vec2::from(mouse_position());
    let click = is_mouse_button_pressed(MouseButton::Left);
    let held = is_mouse_button_down(MouseButton::Left);
    let mut c = Controls {
        confirm: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space),
        back: is_key_pressed(KeyCode::Escape),
        ..Controls::default()
    };
    if is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::W) {
        c.row_step = -1;
    }
    if is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::S) {
        c.row_step = 1;
    }
    if run.phase == Phase::Select {
        if click {
            c.pick_row = (0..run.rows.len()).find(|&i| select_row(i, sw, sh).contains(mouse));
        }
        return c;
    }
    if run.end.is_some() {
        c.confirm |= click;
        return c;
    }

    c.door = [is_key_pressed(KeyCode::Z), is_key_pressed(KeyCode::X)];
    c.seal = [is_key_down(KeyCode::V), is_key_down(KeyCode::N), is_key_down(KeyCode::B)];
    c.light = [is_key_pressed(KeyCode::G), is_key_pressed(KeyCode::H), is_key_pressed(KeyCode::Y)];
    c.wind = is_key_down(KeyCode::R);
    c.toggle_monitor = is_key_pressed(KeyCode::U);
    let digits = [
        KeyCode::Key1,
        KeyCode::Key2,
        KeyCode::Key3,
        KeyCode::Key4,
        KeyCode::Key5,
        KeyCode::Key6,
        KeyCode::Key7,
        KeyCode::Key8,
        KeyCode::Key9,
    ];
    for (i, key) in digits.iter().enumerate() {
        if is_key_pressed(*key) {
            c.pick_cam = Some(i as u32 + 1);
        }
    }
    if is_key_pressed(KeyCode::Left) {
        c.cam_step = -1;
    }
    if is_key_pressed(KeyCode::Right) {
        c.cam_step = 1;
    }

    let l = run.layout();
    if click {
        for ix in 0..2 {
            c.door[ix] |= l.doors[ix].contains(mouse);
        }
        for ix in 0..3 {
            c.light[ix] |= l.lights[ix].contains(mouse);
        }
        if l.monitor.contains(mouse) {
            c.toggle_monitor = true;
        }
        if run.cam_up {
            if let Some(i) = l.cams.iter().position(|r| r.contains(mouse)) {
                c.pick_cam = Some(i as u32 + 1);
            } else if l.map.contains(mouse) {
                if let Some(night) = run.night.as_ref() {
                    let markers = MixedRun::markers(night);
                    let view = map_view(run, &markers);
                    if let Some(cam) = run.map.camera_at_screen(l.map, &view, mouse) {
                        c.pick_cam = Some(cam.id);
                    }
                }
            }
        }
    }
    if held {
        for ix in 0..3 {
            c.seal[ix] |= l.seals[ix].contains(mouse);
        }
        c.wind |= l.wind.contains(mouse);
    }
    c
}

fn map_view<'a>(run: &MixedRun, markers: &'a [(&'a str, &'a str)]) -> MapView<'a> {
    MapView { active_camera: Some(run.cam), hunters: markers, floor: None, legend: true }
}

// ---- drawing ---------------------------------------------------------------------------------------------------

/// The pictures: the hub, the desk over it, and each vent's shut and lit overlay (indexed by side).
pub struct Art {
    office: Texture2D,
    desk: Texture2D,
    vents: [[Texture2D; 2]; 3],
}

impl Art {
    /// Loads the embedded pictures. Call once the window is open.
    pub fn load() -> Self {
        let tex = |bytes: &[u8]| Texture2D::from_file_with_format(bytes, None);
        Self {
            office: tex(include_bytes!("../art/mixed/mixed.png")),
            desk: tex(include_bytes!("../art/mixed/mixed-fg.png")),
            vents: [
                [tex(include_bytes!("../art/mixed/mixed-vent-L-closed.png")), tex(include_bytes!("../art/mixed/mixed-vent-L-flash.png"))],
                [tex(include_bytes!("../art/mixed/mixed-vent-R-closed.png")), tex(include_bytes!("../art/mixed/mixed-vent-R-flash.png"))],
                [tex(include_bytes!("../art/mixed/mixed-vent-T-closed.png")), tex(include_bytes!("../art/mixed/mixed-vent-T-flash.png"))],
            ],
        }
    }
}

fn paint(tex: &Texture2D, sw: f32, sh: f32) {
    draw_texture_ex(tex, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(vec2(sw, sh)), ..Default::default() });
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

/// A threat drawn as its hunter's cut-out (`facing` picks the front or the back), standing on `base_y`, `h` tall. Only
/// when the hunter has no sprite does it fall back to the shape.
fn threat_figure(pool: usize, facing: Facing, cx: f32, base_y: f32, h: f32, alpha: f32) {
    let rect = Rect::new(cx - h * 0.3, base_y - h, h * 0.6, h);
    if !sprites::draw_hunter(hunter_id(pool), facing, rect, Color::new(1.0, 1.0, 1.0, alpha)) {
        figure(cx, base_y, h, threat_color(pool), alpha);
    }
}

/// A threat silhouette standing on `base_y`, `h` tall, with glowing eyes (as in Classic's figures). The fallback when a
/// hunter has no cut-out.
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

pub fn draw(run: &MixedRun, art: &Art) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    if run.phase == Phase::Select {
        draw_select(run, sw, sh);
        return;
    }
    let Some(night) = run.night.as_ref() else { return };
    let l = run.layout();
    if run.cam_up {
        draw_monitor(run, night, &l);
    } else {
        draw_office(run, night, art, &l, sw, sh);
    }
    draw_controls(run, night, &l);
    draw_hud(night, sw, sh);
    run.view.draw(&run.call, l.call);
    for (i, (note, _)) in run.notes.iter().enumerate() {
        text_centered(note, sw * 0.5, sh - 34.0 - (run.notes.len() - 1 - i) as f32 * 24.0, 22.0, AMBER);
    }
    if run.flash_ms > 0.0 {
        let a = (run.flash_ms / ui_ms("flash_ms")) as f32 * 0.55;
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, a));
    }
    if let Some(end) = run.end {
        draw_result(run, end, sw, sh);
    }
}

fn draw_office(run: &MixedRun, night: &MixedShift, art: &Art, l: &Layout, sw: f32, sh: f32) {
    paint(&art.office, sw, sh);
    for ix in 0..3 {
        if run.vent_shut[ix] {
            paint(&art.vents[ix][0], sw, sh);
        } else if run.light_ms[ix] > 0.0 {
            paint(&art.vents[ix][1], sw, sh);
        }
    }
    paint(&art.desk, sw, sh);
    for ix in 0..2 {
        let r = l.doorway[ix];
        let room = if ix == 0 { Room::DoorL } else { Room::DoorR };
        let waiting = night.occupants(room);
        if run.door_shut[ix] {
            draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.30, 0.33, 0.37, 0.92));
            for i in 1..8 {
                let y = r.y + r.h * i as f32 / 8.0;
                draw_line(r.x, y, r.x + r.w, y, 3.0, Color::new(0.22, 0.24, 0.28, 1.0));
            }
            text_centered(label("labels.shut"), r.x + r.w * 0.5, r.y + r.h * 0.5, 30.0, Color::new(0.85, 0.88, 0.92, 1.0));
        }
        for (n, p) in waiting.iter().enumerate() {
            let cx = r.x + r.w * 0.5 + (n as f32 - (waiting.len() as f32 - 1.0) * 0.5) * 40.0;
            let alpha = if run.door_shut[ix] {
                0.35
            } else if night.threat(*p).armed() {
                0.95
            } else {
                0.55
            };
            // The hunter at the door faces the player; one standing behind her is the back cut-out, smaller and dimmer.
            if n == 0 {
                threat_figure(*p, Facing::Front, cx, r.y + r.h, r.h * 0.7, alpha);
            } else {
                threat_figure(*p, Facing::Back, cx, r.y + r.h, r.h * 0.56, alpha * 0.6);
            }
            let threat = night.threat(*p);
            text_centered(threat.name(), cx, r.y + r.h * 0.12, 18.0, INK);
            text_centered(mood_word(threat.mood()), cx, r.y + r.h * 0.19, 14.0, DIM);
        }
        draw_rectangle_lines(r.x - 6.0, r.y - 6.0, r.w + 12.0, r.h + 12.0, 6.0, Color::new(0.22, 0.20, 0.18, 1.0));
        let door = label(if ix == 0 { "labels.door_left" } else { "labels.door_right" });
        text_centered(door, r.x + r.w * 0.5, r.y - 14.0, 20.0, DIM);
    }
    let vents: Vec<String> = [("vent_tag_l", Room::VentL), ("vent_tag_t", Room::VentT), ("vent_tag_r", Room::VentR)]
        .iter()
        .map(|(tag, room)| {
            let names: Vec<&str> = night.occupants(*room).into_iter().map(|p| night.threat(p).name()).collect();
            let who = if names.is_empty() { label("labels.vent_none").to_owned() } else { names.join(", ") };
            fill(label("labels.vent_row"), &[label(&format!("labels.{tag}")), &who])
        })
        .collect();
    text_centered(&fill(label("labels.vents"), &[&vents.join("     ")]), sw * 0.5, sh * 0.71, 20.0, GREEN_TEXT);
}

fn draw_monitor(run: &MixedRun, night: &MixedShift, l: &Layout) {
    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), Color::new(0.015, 0.03, 0.025, 1.0));
    let f = l.feed;
    draw_rectangle(f.x, f.y, f.w, f.h, Color::new(0.03, 0.06, 0.05, 1.0));
    let room = run.cam_room();
    let cam_name = run.map.camera(run.cam).map_or("", |c| c.label.as_str());
    draw_rectangle(f.x, f.y + f.h * 0.66, f.w, f.h * 0.34, Color::new(0.04, 0.08, 0.06, 1.0));
    draw_line(f.x, f.y + f.h * 0.66, f.x + f.w, f.y + f.h * 0.66, 2.0, Color::new(0.10, 0.20, 0.16, 1.0));
    let here = night.occupants(room);
    if here.is_empty() {
        text_centered(label("labels.empty"), f.x + f.w * 0.5, f.y + f.h * 0.5, 30.0, Color::new(0.30, 0.50, 0.42, 1.0));
    }
    for (n, p) in here.iter().enumerate() {
        let cx = f.x + f.w * (n as f32 + 1.0) / (here.len() as f32 + 1.0);
        threat_figure(*p, Facing::Front, cx, f.y + f.h * 0.86, f.h * 0.55, 0.9);
        let threat = night.threat(*p);
        text_centered(&format!("{}  {}", threat.name(), kind_tag(threat.kind())), cx, f.y + f.h * 0.90, 20.0, INK);
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
    draw_text(&fill(label("labels.cam"), &[&run.cam.to_string(), cam_name]), f.x + 12.0, f.y + f.h - 12.0, 24.0, GREEN_TEXT);
    if (run.anim_ms / 500.0) as u64 % 2 == 0 {
        draw_circle(f.x + f.w - 24.0, f.y + 24.0, 7.0, RED_TEXT);
    }

    let markers = MixedRun::markers(night);
    let view = map_view(run, &markers);
    run.map.draw(l.map, &view);
    for (i, r) in l.cams.iter().enumerate() {
        button(*r, &(i + 1).to_string(), i as u32 + 1 == run.cam);
    }
}

fn draw_controls(run: &MixedRun, night: &MixedShift, l: &Layout) {
    if run.end.is_some() {
        return;
    }
    button(l.doors[0], label("labels.btn_door_l"), run.door_shut[0]);
    button(l.seals[0], label("labels.btn_seal_l"), run.vent_shut[0]);
    button(l.lights[0], label("labels.btn_light_l"), run.light_ms[0] > 0.0);
    button(l.seals[2], label("labels.btn_seal_t"), run.vent_shut[2]);
    button(l.lights[2], label("labels.btn_light_t"), run.light_ms[2] > 0.0);
    button(l.monitor, label("labels.btn_monitor"), run.cam_up);
    button(l.wind, label("labels.btn_wind"), night.winding());
    button(l.seals[1], label("labels.btn_seal_r"), run.vent_shut[1]);
    button(l.lights[1], label("labels.btn_light_r"), run.light_ms[1] > 0.0);
    button(l.doors[1], label("labels.btn_door_r"), run.door_shut[1]);
}

fn draw_hud(night: &MixedShift, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    draw_text(&fill(label("labels.hud_shift"), &[night.shift_name()]), 24.0, 30.0, 26.0, INK);
    let beat = match night.beat() {
        Beat::Lull => label("labels.beat_lull"),
        Beat::Build => label("labels.beat_build"),
        Beat::Spike => label("labels.beat_spike"),
        Beat::Breather => label("labels.beat_breather"),
    };
    let threats = night.roster().len().to_string();
    draw_text(&fill(label("labels.hud_threats"), &[&threats, beat]), 250.0, 30.0, 18.0, DIM);
    text_centered(&hour_label(night.hour_reached()), sw * 0.5, 32.0, 30.0, AMBER);
    let (doors, vents) = night.seal_levels();
    let bar = |x: f32, text: &str, frac: f64| {
        let colour = if frac < 0.2 { RED_TEXT } else { INK };
        draw_text(text, x, 22.0, 18.0, DIM);
        draw_rectangle(x, 30.0, 150.0, 6.0, Color::new(0.2, 0.2, 0.2, 1.0));
        draw_rectangle(x, 30.0, 150.0 * frac.clamp(0.0, 1.0) as f32, 6.0, colour);
    };
    bar(sw - 360.0, label("labels.door_seal"), doors);
    bar(sw - 190.0, label("labels.vent_seal"), vents);
    let tension = night.tension().clamp(0.0, 1.0) as f32;
    draw_rectangle(0.0, 44.0, sw * tension, 3.0, RED_TEXT);
    text_centered(label("labels.hint"), sw * 0.5, sh - 6.0, 16.0, DIM);
}

fn draw_result(run: &MixedRun, end: End, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let name = run.rows.get(run.shift).map_or("", |r| r.name.as_str());
    let (title, colour, line) = match end.outcome {
        Outcome::Win => {
            let dawn = data::int(data::rules(MODE), "dawn_hour").to_string();
            let points = end.points.to_string();
            (fill(label("labels.cleared"), &[name]), AMBER, fill(label("labels.dawn"), &[&dawn, &points]))
        }
        Outcome::Caught(_) => match end.best {
            Some(hours) => (label("labels.caught").to_owned(), RED_TEXT, fill(label("labels.caught_by"), &[end.who, &hour_label(hours)])),
            None => (label("labels.caught").to_owned(), RED_TEXT, fill(label("labels.caught_office"), &[end.who])),
        },
        Outcome::Ongoing => return,
    };
    text_centered(&title, sw * 0.5, sh * 0.40, 64.0, colour);
    text_centered(&line, sw * 0.5, sh * 0.40 + 48.0, 28.0, INK);
    let next = match end.outcome {
        Outcome::Win if run.shift + 1 < run.rows.len() => label("labels.next_shift"),
        Outcome::Win => label("labels.back_to_shifts"),
        _ => label("labels.try_again"),
    };
    if run.end_ms >= ui_ms("result_lock_ms") {
        text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
        text_centered(label("labels.esc_shifts"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
    }
}

fn draw_select(run: &MixedRun, sw: f32, sh: f32) {
    text_centered(label("labels.select_title"), sw * 0.5, sh * 0.18, 64.0, INK);
    text_centered(label("labels.select_sub"), sw * 0.5, sh * 0.25, 22.0, DIM);
    for (i, row) in run.rows.iter().enumerate() {
        let r = select_row(i, sw, sh);
        let active = i == run.pick;
        draw_rectangle(r.x, r.y, r.w, r.h, PANEL);
        if active {
            draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, AMBER);
        }
        let colour = if active { AMBER } else { INK };
        draw_text(&format!("{}. {}", i + 1, row.name), r.x + 16.0, r.y + 36.0, 28.0, colour);
        let detail = if row.endless {
            fill(label("labels.select_endless"), &[&hour_label(run.best_endless)])
        } else {
            let (lo, hi) = (row.threats.0.to_string(), row.threats.1.to_string());
            fill(label("labels.select_timed"), &[&lo, &hi, &format!("{:.1}", row.tell_s)])
        };
        let w = measure_text(&detail, None, 18, 1.0).width;
        draw_text(&detail, r.x + r.w - w - 16.0, r.y + 34.0, 18.0, DIM);
    }
    text_centered(label("labels.select_footer"), sw * 0.5, sh - 20.0, 18.0, DIM);
}

// ---- scripted runs (the --mixed-screenshot flag) ----------------------------------------------------------------

/// What a `--mixed-screenshot` run does before the picture: the shift, how many fixed frames to play, and optionally
/// the monitor up on one camera (the scripted presses then leave the camera alone).
#[derive(Clone, Debug, Default)]
pub struct ShotPlan {
    pub shift: usize,
    pub frames: u32,
    pub cam: Option<u32>,
}

/// Plays a fresh shift by the plan at a fixed 60 fps and returns it ready to draw. Nothing is saved.
pub fn run_script(seed: i64, plan: &ShotPlan) -> MixedRun {
    let mut run = MixedRun::new(seed, false);
    run.begin(plan.shift);
    let step = 1000.0 / data::num(ui(), "script_fps");
    if let Some(cam) = plan.cam {
        run.update(step, &Controls { toggle_monitor: true, pick_cam: Some(cam), ..Controls::default() });
    }
    let mut frame = 0u32;
    while frame < plan.frames && run.end.is_none() {
        run.update(step, &scripted(frame, plan.cam.is_none()));
        frame += 1;
    }
    run
}

/// The fixed presses and holds the scripted runs play: doors, seals, lights, the winder, the monitor and, when `roam`
/// is set, the cameras.
fn scripted(frame: u32, roam: bool) -> Controls {
    Controls {
        door: [frame % 620 == 310, frame % 470 == 200],
        seal: [(frame / 280) % 5 == 1, (frame / 350) % 4 == 2, (frame / 500) % 6 == 3],
        light: [frame % 240 == 7, frame % 310 == 150, frame % 360 == 90],
        wind: frame % 900 < 90,
        toggle_monitor: frame % 400 == 200,
        pick_cam: if roam && frame % 130 == 0 { Some(1 + (frame / 130) % CAM_COUNT) } else { None },
        ..Controls::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// About 53 minutes of game time at the frame step: far past any shift's end.
    const MAX_STEPS: u32 = 200_000;

    fn idle() -> Controls {
        Controls::default()
    }

    /// A reactive player: shuts each door while a threat stands at it, seals and lights a vent while a crawler is in
    /// it, and sweeps the cameras with the monitor up for a while in every cycle.
    fn bot(run: &MixedRun, frame: u32) -> Controls {
        let Some(night) = run.night.as_ref() else { return idle() };
        let at = |room: Room| !night.occupants(room).is_empty();
        let want = [at(Room::DoorL), at(Room::DoorR)];
        let vents = [at(Room::VentL), at(Room::VentR), at(Room::VentT)];
        // the monitor goes up on the first camera room with a threat in it, and down when the building is empty
        let watched = CAM_ROOMS.iter().position(|r| !night.occupants(*r).is_empty());
        let cam = watched.map_or(run.cam, |i| i as u32 + 1);
        Controls {
            door: [want[0] != run.want[0], want[1] != run.want[1]],
            seal: vents,
            light: [vents[0] && frame % 40 == 0, vents[1] && frame % 40 == 0, vents[2] && frame % 40 == 0],
            wind: frame % 300 < 30,
            toggle_monitor: watched.is_some() != run.cam_up,
            pick_cam: (cam != run.cam).then_some(cam),
            ..idle()
        }
    }

    #[test]
    fn the_five_shifts_come_from_the_data() {
        let rows = shift_rows();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["QUIET", "STEADY", "RESTLESS", "CHAOS", "ENDLESS"]);
        assert!(rows[4].endless && !rows[0].endless);
        assert_eq!(rows[3].threats, (6, 7));
    }

    #[test]
    fn the_layout_fits_the_window_and_the_buttons_do_not_overlap() {
        let l = layout(1280.0, 720.0);
        let mut all = vec![l.doors[0], l.doors[1], l.monitor, l.wind];
        all.extend(l.seals);
        all.extend(l.lights);
        for (i, a) in all.iter().enumerate() {
            assert!(a.x >= 0.0 && a.right() <= 1280.0 && a.bottom() <= 720.0, "{a:?}");
            for b in &all[i + 1..] {
                assert!(a.intersect(*b).is_none(), "{a:?} overlaps {b:?}");
            }
        }
        assert_eq!(l.cams.len(), CAM_COUNT as usize);
        assert!(l.cams.iter().all(|r| r.right() <= l.map.right() + 0.5 && r.bottom() <= 720.0));
        assert!(l.call.intersect(l.feed).is_none() && l.call.intersect(l.map).is_none(), "the call box clears the monitor");
    }

    /// Plays shift `shift` to its end with `press` choosing each frame's controls. Returns the outcome and the frame count.
    fn play_to_end(shift: usize, seed: i64, press: impl Fn(&MixedRun, u32) -> Controls) -> (Outcome, u32) {
        let mut run = MixedRun::new(seed, false);
        run.begin(shift);
        let mut steps = 0;
        while run.end.is_none() && steps < MAX_STEPS {
            let c = press(&run, steps);
            run.update(16.0, &c);
            steps += 1;
        }
        let outcome = run.end.map(|e| e.outcome).unwrap_or_else(|| panic!("shift {shift} did not end in {steps} steps"));
        (outcome, steps)
    }

    #[test]
    fn every_shift_reaches_an_end_under_scripted_and_reactive_input() {
        for shift in 0..5 {
            for (name, press) in [("scripted", scripted_press as fn(&MixedRun, u32) -> Controls), ("reactive", bot as fn(&MixedRun, u32) -> Controls)] {
                let (outcome, steps) = play_to_end(shift, 20_261_103 + shift as i64, press);
                if shift == 4 {
                    assert!(matches!(outcome, Outcome::Caught(_)), "ENDLESS has no dawn to win ({name}, {steps} steps)");
                } else {
                    assert!(matches!(outcome, Outcome::Win | Outcome::Caught(_)), "shift {shift} ({name}): {outcome:?}");
                }
            }
        }
    }

    /// The scripted presses on the frame count alone, with the cameras roaming.
    fn scripted_press(_run: &MixedRun, frame: u32) -> Controls {
        scripted(frame, true)
    }

    #[test]
    fn an_idle_timed_shift_ends_at_dawn_or_in_a_catch() {
        let mut run = MixedRun::new(41, false);
        run.begin(0);
        let mut steps = 0;
        while run.end.is_none() && steps < MAX_STEPS {
            run.update(16.0, &idle());
            steps += 1;
        }
        assert!(run.end.is_some(), "no end in {steps} steps");
    }

    #[test]
    fn the_call_script_rings_types_and_closes() {
        let mut run = MixedRun::new(7, false);
        run.begin(0);
        let (mut opened, mut longest) = (false, 0);
        for _ in 0..4000 {
            run.update(16.0, &idle());
            opened |= run.call.open;
            longest = longest.max(run.call.text.chars().count());
            if run.call.done {
                break;
            }
        }
        assert!(opened, "the call rang");
        assert!(run.call.done, "the call closed after its last line");
        assert!(longest > 40, "the first line was typed out ({longest} letters)");
    }

    #[test]
    fn keys_drive_the_doors_seals_lights_monitor_and_cameras() {
        let mut run = MixedRun::new(3, false);
        run.begin(0);
        run.update(16.0, &Controls { door: [true, false], ..idle() });
        assert!(run.want[0] && run.door_shut[0], "Z shuts the left door");
        run.update(16.0, &Controls { door: [true, false], ..idle() });
        assert!(!run.want[0] && !run.door_shut[0], "Z again opens it");
        run.update(16.0, &Controls { light: [true, false, false], ..idle() });
        assert!(run.light_ms[0] > 0.0, "a tap of G lights the left vent");
        run.update(16.0, &Controls { seal: [true, false, false], ..idle() });
        assert!(run.vent_shut[0], "holding V seals the left vent");
        run.update(16.0, &Controls { light: [true, false, false], ..idle() });
        assert!(run.notes.iter().any(|n| n.0.contains("sealed")), "a sealed vent takes no light");
        run.update(16.0, &Controls { toggle_monitor: true, pick_cam: Some(5), ..idle() });
        assert!(run.cam_up && run.cam == 5);
        assert_eq!(run.cam_room(), Room::Closet);
        run.update(16.0, &Controls { cam_step: 1, ..idle() });
        assert_eq!(run.cam, 6);
    }

    #[test]
    fn the_select_starts_the_chosen_shift_and_escape_steps_back() {
        let mut run = MixedRun::new(5, false);
        assert_eq!(run.phase, Phase::Select);
        run.update(16.0, &Controls { row_step: 2, ..idle() });
        assert_eq!(run.pick, 2);
        run.update(16.0, &Controls { confirm: true, ..idle() });
        assert_eq!(run.phase, Phase::Night);
        assert_eq!(run.shift, 2);
        run.update(16.0, &Controls { back: true, ..idle() });
        assert_eq!(run.phase, Phase::Select, "ESC leaves the night for the select");
        assert_eq!(run.update(16.0, &Controls { back: true, ..idle() }), Flow::Hub, "ESC on the select goes to the hub");
    }

    #[test]
    fn the_shell_timings_and_labels_load_from_the_rules_ui_block() {
        for key in ["max_step_ms", "result_lock_ms", "note_ms", "flash_ms", "light_ms", "script_fps"] {
            assert!(data::num(ui(), key) > 0.0, "ui.{key} must be a positive number");
        }
        assert_eq!(label("hour.midnight"), "12 AM");
        assert_eq!(fill("{} AM", &["3"]), "3 AM");
        assert_eq!(fill("{} is {}.", &["Opalie"]), "Opalie is .");
        assert_eq!(room_label(Room::DoorL), "the left door");
        assert_eq!(side_name(Side::T), "top");
        assert_eq!(mood_word(Mood::Shy), "keeping out of sight");
        assert_eq!(kind_tag(Kind::Winder), "WINDER");
    }

    #[test]
    fn every_threat_in_the_cast_has_a_hunter_cut_out() {
        for entry in data::list(data::cast(MODE), "pool") {
            let id = data::text(entry, "id");
            assert!(data::HUNTER_SPRITE_FILES.iter().any(|(h, _, _)| *h == id), "{id} has no sprite embedded");
        }
        assert_eq!(hunter_id(0), "haelith", "pool 0 is the first threat in the cast");
    }

    /// A scripted defender, driven only through `Controls`. It watches which threats stand armed at a gate (the tell)
    /// and answers each: the matching door for a walker, a held seal for a crawler. It never touches the monitor, so a
    /// counter never builds and the monitor-gated threat never gets its echoes. The old reactive `bot` loses because it
    /// looks for threats with `occupants`, which leaves out armed ones: a threat at a door is armed the moment it
    /// arrives, so the bot's door logic never fires. (No rules bug: the tell lead is never below the floor.)
    fn defender(run: &MixedRun, _frame: u32) -> Controls {
        let Some(night) = run.night.as_ref() else { return idle() };
        let (mut door, mut seal) = ([false; 2], [false; 3]);
        for &p in night.roster() {
            let t = night.threat(p);
            let Some(side) = t.gate() else { continue };
            if t.kind() == Kind::Vent {
                seal[side_ix(side)] = true;
            } else if side != Side::T {
                door[side_ix(side)] = true;
            }
        }
        Controls { door: [door[0] != run.want[0], door[1] != run.want[1]], seal, ..idle() }
    }

    fn temp_save(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("nw_mixed_{tag}_{}", std::process::id())).join("progress.json")
    }

    /// Plays QUIET with the defender, persisting to `path`. Returns the outcome and every sound event raised.
    fn defend_quiet(seed: i64, path: &std::path::Path) -> (Outcome, Vec<&'static str>) {
        let mut run = MixedRun::new(seed, false);
        run.persist_to(path.to_path_buf());
        run.begin(0);
        let (mut steps, mut heard) = (0, Vec::new());
        while run.end.is_none() && steps < MAX_STEPS {
            let c = defender(&run, steps);
            run.update(16.0, &c);
            heard.extend(run.take_sound_events().into_iter().map(|(id, _)| id));
            steps += 1;
        }
        (run.end.expect("QUIET ended").outcome, heard)
    }

    #[test]
    fn the_scripted_defender_clears_quiet_and_the_win_is_saved() {
        for seed in [1, 20_261_104, 77] {
            let path = temp_save("win");
            let _ = std::fs::remove_file(&path);
            let (outcome, _) = defend_quiet(seed, &path);
            assert_eq!(outcome, Outcome::Win, "the defender clears QUIET (seed {seed})");
            let save = NightWatchSave::load_from(&path);
            assert_eq!(save.cleared_levels(MODE), vec![1], "cleared_mixed_shift lists the shift (seed {seed})");
            assert_eq!(save.best_score(MODE), 10);
            assert_eq!(save.night_best(MODE, 1), Some(10));
            let _ = std::fs::remove_dir_all(path.parent().unwrap());
        }
    }

    #[test]
    fn a_second_win_keeps_the_best_score_and_does_not_duplicate_the_clear() {
        let path = temp_save("best");
        let _ = std::fs::remove_file(&path);
        let mut seeded = NightWatchSave::default();
        seeded.record_level_clear(MODE, 1);
        seeded.record_best(MODE, 25);
        seeded.record_night_best(MODE, 1, 25);
        seeded.save_to(&path).unwrap();
        let (outcome, _) = defend_quiet(5, &path);
        assert_eq!(outcome, Outcome::Win);
        let save = NightWatchSave::load_from(&path);
        assert_eq!(save.best_score(MODE), 25, "a lower score never replaces the best");
        assert_eq!(save.night_best(MODE, 1), Some(25));
        assert_eq!(save.cleared_levels(MODE), vec![1]);
        let (again, _) = defend_quiet(6, &path);
        assert_eq!(again, Outcome::Win);
        assert_eq!(NightWatchSave::load_from(&path).cleared_levels(MODE), vec![1], "two clears list the shift once");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn every_sound_event_resolves_to_a_cue_and_the_tells_are_heard() {
        let cues: Value = serde_json::from_str(include_str!("../data/sounds.json")).expect("sounds.json parses");
        for id in SOUND_IDS {
            let cue = cue_for_event(id).unwrap_or_else(|| panic!("sound event {id} has no cue in rules.sounds"));
            assert!(cues["cues"].get(cue).is_some(), "{id} -> {cue} is not a cue in data/sounds.json");
        }
        assert_eq!(cue_for_event("no_such_event"), None);
        // the tells by kind: a crawler knocks in the vent, a walker bangs the door, a counter has no side
        let night = MixedShift::start(3, 3);
        let tell = |pool: usize, side: Option<Side>| Event::Tell { pool, side, lead: 3000.0, rearm: false };
        assert_eq!(sound_for(&tell(6, Some(Side::L)), &night), Some(("vent_knock", -0.6)));
        assert_eq!(sound_for(&tell(4, Some(Side::R)), &night), Some(("door_bang", 0.6)));
        assert_eq!(sound_for(&tell(0, None), &night), Some(("counter_tell", 0.0)));
        assert_eq!(sound_for(&Event::Wound { pool: 7 }, &night).map(|s| s.0), Some("box_wound"));
        assert_eq!(sound_for(&Event::Arrive { pool: 4, room: None, side: Some(Side::L) }, &night).map(|s| s.0), Some("arrive"));
        // a whole night raises only known ids, and at least one tell is among them
        let path = temp_save("sound");
        let (_, heard) = defend_quiet(9, &path);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        assert!(heard.iter().all(|id| SOUND_IDS.contains(id) && cue_for_event(id).is_some()), "{heard:?}");
        assert!(heard.iter().any(|id| matches!(*id, "door_bang" | "vent_knock" | "counter_tell")), "no tell was heard: {heard:?}");
        assert!(heard.contains(&"dawn"));
    }

    #[test]
    fn endless_never_ends_before_the_data_minimum_time() {
        let rules = data::rules(MODE);
        let endless = data::list(rules, "shifts").iter().find(|s| data::text(s, "name") == "ENDLESS").expect("an ENDLESS shift");
        // nothing can arm before its grace, and a kill needs the tell floor after that
        let min_ms = data::num(endless, "grace_ms") + data::num(rules, "fairness.tell_floor_ms");
        assert!(data::num(rules, "endless.gap_min") > 0.0 && data::num(rules, "endless.pace_cap") >= 1.0, "pacing is data");
        for seed in [2, 31, 20_261_104] {
            let mut run = MixedRun::new(seed, false);
            run.begin(4);
            let mut steps = 0;
            while run.end.is_none() && steps < MAX_STEPS {
                run.update(16.0, &idle());
                steps += 1;
            }
            let night = run.night.as_ref().expect("the night is kept at its end");
            assert!(matches!(run.end.map(|e| e.outcome), Some(Outcome::Caught(_))), "ENDLESS only ends in a catch");
            assert!(night.elapsed_ms() >= min_ms, "caught at {} ms, before {} ms (seed {seed})", night.elapsed_ms(), min_ms);
        }
    }
}
