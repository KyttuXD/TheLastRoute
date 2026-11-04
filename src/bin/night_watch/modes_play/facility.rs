//! The Facility on screen (hub id `story`): the shell's driver for `modes::facility`. `FacilityRun` owns one shift and
//! maps the frame's `Controls` onto the mode's `Input`, `walk_through` and `interact`: A, D and W walk the side and lift
//! doors, C crawls, F holds the beacon, H hides under the desk, E holds the lift's call button, digits and clicks press
//! the station and SPACE turns the crank. `draw` paints the room picture, the hunters as their sprites, the station, the
//! list, the hunters' distance, the gauges and the call box. A signed-off night is saved. The UI timings and every label
//! the screen shows come from the `ui` block of `data/rules/facility.json`. Everything but `poll_controls`, `draw` and
//! `Art` is plain state, so the tests run without a window; `autoplay` plays a shift on its own for the tests and the
//! screenshot run.

use std::collections::VecDeque;

use macroquad::prelude::*;

use crate::calls_view::CallView;
use crate::data;
use crate::data::CallsData;
use crate::framework::CallScript;
use crate::modes::facility::{self, CloseKind, Door, Event, Facility, FacilityOptions, Hunter, Input, LiftPhase, Outcome, Press, Puzzle, Room, TaskKind, MODE_ID};
use crate::save::NightWatchSave;
use crate::sprites::{self, Facing};

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);

/// The rules file this driver reads its `ui` block from (the mode's `rules` id, not the hub id `MODE_ID`).
const RULES_MODE: &str = "facility";

/// A UI timing in milliseconds from the `ui` block, e.g. `ui_ms("note_ms")`. A missing key panics: a content bug.
fn ui_ms(key: &str) -> f64 {
    data::num(data::rules(RULES_MODE), &format!("ui.{key}"))
}

/// A label from the `ui.labels` block of the rules file.
fn label(key: &str) -> &'static str {
    data::text(data::rules(RULES_MODE), &format!("ui.labels.{key}"))
}

/// `template` with each `{key}` placeholder replaced by its value.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    values.iter().fold(template.to_owned(), |text, (key, value)| text.replace(&format!("{{{key}}}"), value))
}

/// The fixed step the scripted screenshot run advances by: one frame at `ui.script_fps`.
fn script_step_ms() -> f64 {
    1000.0 / ui_ms("script_fps")
}

const DOORS: [Door; 3] = [Door::Left, Door::Right, Door::Lift];
const DIGIT_KEYS: [KeyCode; 10] = [
    KeyCode::Key0,
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
/// The silhouette colours, indexed by `Hunter` (Opalie, Porcelle, Quenelle). The shape is the fallback for a missing sprite.
const HUNTER_COLORS: [Color; 3] = [Color::new(0.92, 0.55, 0.70, 1.0), Color::new(0.35, 0.80, 0.78, 1.0), Color::new(0.90, 0.25, 0.25, 1.0)];

/// The room pictures, the Godot story office art (480 by 270). Each room has a back picture and a foreground that
/// overlays it (the desk in Control). Only Control uses `story.png`; the others are `story-<room>.png`.
const BACK_PNG: [&[u8]; 6] = [
    include_bytes!("../assets/offices/story-breaker.png"),
    include_bytes!("../assets/offices/story-gallery.png"),
    include_bytes!("../assets/offices/story.png"),
    include_bytes!("../assets/offices/story-workshop.png"),
    include_bytes!("../assets/offices/story-auditorium.png"),
    include_bytes!("../assets/offices/story-lift.png"),
];
const FRONT_PNG: [&[u8]; 6] = [
    include_bytes!("../assets/offices/story-breaker-fg.png"),
    include_bytes!("../assets/offices/story-gallery-fg.png"),
    include_bytes!("../assets/offices/story-fg.png"),
    include_bytes!("../assets/offices/story-workshop-fg.png"),
    include_bytes!("../assets/offices/story-auditorium-fg.png"),
    include_bytes!("../assets/offices/story-lift-fg.png"),
];

/// The decoded room pictures. Needs the window, so the shell builds it once after the window opens.
pub struct Art {
    back: Vec<Texture2D>,
    front: Vec<Texture2D>,
}

impl Art {
    pub fn load() -> Art {
        Art {
            back: BACK_PNG.iter().map(|bytes| Texture2D::from_file_with_format(bytes, None)).collect(),
            front: FRONT_PNG.iter().map(|bytes| Texture2D::from_file_with_format(bytes, None)).collect(),
        }
    }
}

/// What the player did this frame. Doors and station presses are presses; the crawl, beacon, hide and hold are held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    /// A walk through this door starts this frame (A, D, W, or a click on the doorway or its button).
    pub door: Option<Door>,
    /// One action at the current station (a digit, a numbered button, or the crank).
    pub press: Option<Press>,
    pub crawl: bool,
    pub light: bool,
    pub hide: bool,
    pub work: bool,
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
    /// Start this night (the next one after a sign-off, the same one again after a catch).
    Night(u32),
}

pub struct FacilityRun {
    sim: Facility,
    night: u32,
    persist: bool,
    call: CallScript,
    view: CallView,
    end: Option<Outcome>,
    end_ms: f64,
    flash_ms: f64,
    notes: Vec<(String, f64)>,
    anim_ms: f64,
}

impl FacilityRun {
    /// Opens Facility night `night`. `persist` lets a signed-off night write the save file (off for tests and scripts).
    pub fn new(calls: &CallsData, night: u32, seed: i64, persist: bool) -> Self {
        let night = night.clamp(1, facility::night_count());
        let mut view = CallView::new();
        view.player.enabled = persist;
        Self {
            sim: Facility::start(FacilityOptions::new(night, seed)),
            night,
            persist,
            call: CallScript::for_level(calls, MODE_ID, night as i32),
            view,
            end: None,
            end_ms: 0.0,
            flash_ms: 0.0,
            notes: Vec::new(),
            anim_ms: 0.0,
        }
    }

    fn note(&mut self, text: impl Into<String>) {
        self.notes.push((text.into(), ui_ms("note_ms")));
        if self.notes.len() > 3 {
            self.notes.remove(0);
        }
    }

    /// The label of the task at `index` on tonight's list.
    fn task_label(&self, index: usize) -> &'static str {
        self.sim.tasks.get(index).map_or("", |task| task.label)
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
            if c.confirm && self.end_ms >= ui_ms("result_lock_ms") {
                return match end {
                    Outcome::Win if self.night < facility::night_count() => Flow::Night(self.night + 1),
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
            self.note(label("muted"));
        }
        self.sim.crawl = c.crawl;
        if let Some(door) = c.door {
            let walking = self.sim.moving.is_some();
            if !self.sim.walk_through(door) && !walking {
                self.note(label("no_door"));
            }
        }
        if let Some(press) = c.press {
            for event in self.sim.interact(press) {
                self.play(event);
            }
        }
        let dt = dt_ms.min(ui_ms("max_step_ms"));
        let events = self.sim.step(dt, Input { work: c.work, light: c.light, hide: c.hide });
        self.call.update(self.sim.elapsed, dt);
        self.view.update(&self.call);
        for event in events {
            self.play(event);
        }
        Flow::Stay
    }

    fn play(&mut self, event: Event) {
        match event {
            Event::Arrive(_) | Event::Station { ok: true, .. } => {}
            Event::Close { hunter, dist, kind } => {
                let key = match kind {
                    CloseKind::Noticed => "noticed",
                    CloseKind::Quiet => "creeps",
                    CloseKind::Away => "coming_back",
                };
                let line = fill(label(key), &[("name", hunter.name()), ("steps", &steps_text(dist))]);
                self.note(line);
            }
            Event::Back { hunter, dist } => {
                let line = fill(label("drifts_back"), &[("name", hunter.name()), ("steps", &steps_text(dist))]);
                self.note(line);
            }
            Event::Pushed(hunter) => self.note(fill(label("pushed"), &[("name", hunter.name())])),
            Event::Alert(hunter) => self.note(fill(label("hears_warning"), &[("name", hunter.name())])),
            Event::Bell(_) => self.note(label("lift_bell")),
            Event::Open(_) => self.note(label("lift_open")),
            Event::Leave(_) => self.note(label("lift_leaves")),
            Event::Station { ok: false, .. } => self.note(label("wrong")),
            Event::Deny => self.note(label("nothing_here")),
            Event::TaskDone(done) => {
                let line = fill(label("done"), &[("label", self.task_label(done.saturating_sub(1)))]);
                self.note(line);
            }
            Event::Caught(hunter) => {
                self.end = Some(Outcome::Caught(hunter));
                self.flash_ms = ui_ms("flash_ms");
            }
            Event::Win => {
                self.end = Some(Outcome::Win);
                self.save_clear();
            }
        }
    }

    /// Opens the Custom Night: the three sliders are Opalie's, Porcelle's and Quenelle's levels. Nothing is saved from it.
    pub fn new_custom(calls: &CallsData, levels: [Option<i32>; 3], seed: i64) -> Self {
        let mut run = Self::new(calls, 1, seed, false);
        let options = FacilityOptions { custom: true, levels, ..FacilityOptions::new(facility::night_count(), seed) };
        run.night = facility::night_count();
        run.sim = Facility::start(options);
        run.call = CallScript::new(Vec::new(), 0.0);
        run
    }

    /// Writes the signed-off night and its score to the progress file (best effort; test and script runs skip it).
    fn save_clear(&mut self) {
        if !self.persist {
            return;
        }
        let mut save = NightWatchSave::load();
        save.record_level_clear(MODE_ID, self.night);
        save.record_best(MODE_ID, self.sim.score());
        save.record_night_best(MODE_ID, self.night, self.sim.score());
        if save.save().is_err() {
            self.note(label("save_failed"));
        }
    }
}

/// "1 step" or "N steps", from the `ui.labels` block.
fn steps_text(n: i32) -> String {
    let n = n.max(0);
    let template = label(if n == 1 { "step_one" } else { "step_many" });
    fill(template, &[("n", &n.to_string())])
}

/// The door that starts a walk from `from` toward `to` along the shortest route, or None when no route exists.
pub fn route_door(from: Room, to: Room) -> Option<Door> {
    if from == to {
        return None;
    }
    let mut queue: VecDeque<(Room, Option<Door>)> = VecDeque::from([(from, None)]);
    let mut seen = vec![from];
    while let Some((room, first)) = queue.pop_front() {
        for door in DOORS {
            let Some(next) = room.exit(door) else { continue };
            let first = first.or(Some(door));
            if next == to {
                return first;
            }
            if !seen.contains(&next) {
                seen.push(next);
                queue.push_back((next, first));
            }
        }
    }
    None
}

/// The controls a player who knows the answers presses this frame: walk to the task's room, then solve its station.
/// The tests and the scripted screenshot run use it; the window never does.
pub fn autoplay(run: &FacilityRun) -> Controls {
    let sim = &run.sim;
    let mut c = Controls::default();
    if sim.moving.is_some() {
        return c;
    }
    let Some(task) = sim.current_task() else { return c };
    if task.room != sim.room {
        c.door = route_door(sim.room, task.room);
        return c;
    }
    match sim.puzzle() {
        Puzzle::Code { code, entered } => c.press = code.get(*entered).map(|&digit| Press::Digit(digit)),
        Puzzle::Dials { target, pos } => c.press = (0..pos.len()).find(|&i| pos[i] != target[i]).map(Press::Index),
        Puzzle::Collect { n, taken } => c.press = (0..*n).find(|i| !taken.contains(i)).map(Press::Index),
        Puzzle::Crank { .. } => c.press = Some(Press::Crank),
        Puzzle::Switches { order, next } => c.press = order.get(*next).map(|&i| Press::Index(i)),
        Puzzle::Hold => c.work = true,
        Puzzle::None => {}
    }
    c
}

/// The press a digit key gives at this station: a digit on the code, or the numbered button of the task (1 to n).
fn press_for_digit(puzzle: &Puzzle, digit: usize) -> Option<Press> {
    match puzzle {
        Puzzle::Code { .. } => Some(Press::Digit(digit)),
        Puzzle::Dials { pos, .. } => index_press(digit, pos.len()),
        Puzzle::Collect { n, .. } => index_press(digit, *n),
        Puzzle::Switches { order, .. } => index_press(digit, order.len()),
        Puzzle::Crank { .. } | Puzzle::Hold | Puzzle::None => None,
    }
}

/// Button or key `key` (1 to `count`) as an index press.
fn index_press(key: usize, count: usize) -> Option<Press> {
    (key >= 1 && key <= count).then(|| Press::Index(key - 1))
}

// ---- layout ----------------------------------------------------------------------------------------------------

/// The held and pressed buttons along the bottom, in `Btn` order (also their index in `Layout::buttons`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    Left,
    Lift,
    Right,
    Crawl,
    Beacon,
    Hold,
    Hide,
}

const BUTTONS: [Btn; 7] = [Btn::Left, Btn::Lift, Btn::Right, Btn::Crawl, Btn::Beacon, Btn::Hold, Btn::Hide];

fn btn_label(button: Btn) -> &'static str {
    match button {
        Btn::Left => label("button_left"),
        Btn::Lift => label("button_lift"),
        Btn::Right => label("button_right"),
        Btn::Crawl => label("button_crawl"),
        Btn::Beacon => label("button_beacon"),
        Btn::Hold => label("button_hold"),
        Btn::Hide => label("button_hide"),
    }
}

/// Every clickable or drawn region for a window of `sw` by `sh`. Pure, so the clicks and the paint agree.
struct Layout {
    buttons: [Rect; 7],
    /// The doorway hit zones, indexed by `Door` (left, right, lift).
    doors: [Rect; 3],
    task: Rect,
    hunters: Rect,
    call: Rect,
    station: Rect,
}

fn layout(sw: f32, sh: f32, tasks: usize) -> Layout {
    let gap = 10.0;
    let btn_w = ((sw - 40.0 - gap * 6.0) / 7.0).clamp(96.0, 150.0);
    let x0 = (sw - (btn_w * 7.0 + gap * 6.0)) * 0.5;
    let buttons = std::array::from_fn(|i| Rect::new(x0 + i as f32 * (btn_w + gap), sh - 150.0, btn_w, 40.0));
    let doors = [
        Rect::new(0.0, sh * 0.50, sw * 0.10, sh * 0.30),
        Rect::new(sw * 0.90, sh * 0.50, sw * 0.10, sh * 0.30),
        Rect::new(sw * 0.44, sh * 0.30, sw * 0.12, sh * 0.36),
    ];
    let call_w = (sw * 0.44).clamp(400.0, 600.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let task = Rect::new(16.0, 56.0, 262.0, 52.0 + tasks as f32 * 36.0);
    let hunters = Rect::new(sw - 270.0, 56.0, 254.0, 130.0);
    let station_w = (sw * 0.46).clamp(420.0, 640.0);
    let station = Rect::new((sw - station_w) * 0.5, sh * 0.30, station_w, sh * 0.36);
    Layout { buttons, doors, task, hunters, call, station }
}

/// The press buttons of a station, each with the press it gives. Empty for a hold (the E button holds the work).
fn station_slots(puzzle: &Puzzle, area: Rect) -> Vec<(Rect, Press)> {
    let top = area.y + 124.0;
    match puzzle {
        Puzzle::Code { .. } => {
            // a keypad, 1 to 9 then 0
            const KEYPAD: [usize; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 0];
            let w = (area.w - 40.0 - 4.0 * 8.0) / 5.0;
            KEYPAD
                .iter()
                .enumerate()
                .map(|(i, &digit)| (Rect::new(area.x + 20.0 + (i % 5) as f32 * (w + 8.0), top + (i / 5) as f32 * 56.0, w, 48.0), Press::Digit(digit)))
                .collect()
        }
        Puzzle::Dials { pos, .. } => row(area, top + 8.0, pos.len(), Press::Index),
        Puzzle::Collect { n, .. } => row(area, top + 8.0, *n, Press::Index),
        Puzzle::Switches { order, .. } => row(area, top + 8.0, order.len(), Press::Index),
        Puzzle::Crank { .. } => vec![(Rect::new(area.x + (area.w - 240.0) * 0.5, top + 16.0, 240.0, 72.0), Press::Crank)],
        Puzzle::None | Puzzle::Hold => Vec::new(),
    }
}

/// `n` buttons in one centred row at height `y`.
fn row(area: Rect, y: f32, n: usize, press: impl Fn(usize) -> Press) -> Vec<(Rect, Press)> {
    let n = n.max(1);
    let w = ((area.w - 40.0 - 8.0 * (n as f32 - 1.0)) / n as f32).min(120.0);
    let total = w * n as f32 + 8.0 * (n as f32 - 1.0);
    let x0 = area.x + (area.w - total) * 0.5;
    (0..n).map(|i| (Rect::new(x0 + i as f32 * (w + 8.0), y, w, 56.0), press(i))).collect()
}

fn press_label(puzzle: &Puzzle, press: Press) -> String {
    match (puzzle, press) {
        (_, Press::Digit(digit)) => digit.to_string(),
        (Puzzle::Dials { pos, .. }, Press::Index(i)) => fill(label("press_dial"), &[("n", &(i + 1).to_string()), ("mark", &(pos[i] + 1).to_string())]),
        (Puzzle::Collect { .. }, Press::Index(i)) => fill(label("press_part"), &[("n", &(i + 1).to_string())]),
        (Puzzle::Switches { .. }, Press::Index(i)) => fill(label("press_breaker"), &[("n", &(i + 1).to_string())]),
        (_, Press::Crank) => label("press_crank").to_owned(),
        (_, Press::Index(i)) => (i + 1).to_string(),
    }
}

/// A part already taken off the bench: its button is greyed out.
fn press_taken(puzzle: &Puzzle, press: Press) -> bool {
    matches!((puzzle, press), (Puzzle::Collect { taken, .. }, Press::Index(i)) if taken.contains(&i))
}

/// The lines on the station's note: the code, the dial marks, the flip order and the counts.
fn station_lines(puzzle: &Puzzle) -> Vec<String> {
    let digits = |values: &[usize], offset: usize| values.iter().map(|v| (v + offset).to_string()).collect::<Vec<_>>().join("  ");
    match puzzle {
        Puzzle::Code { code, entered } => vec![
            fill(label("station_note"), &[("digits", &digits(code, 0))]),
            fill(label("station_entered"), &[("entered", &entered.to_string()), ("total", &code.len().to_string())]),
        ],
        Puzzle::Dials { target, .. } => vec![fill(label("station_target"), &[("digits", &digits(target, 1))]), label("station_dial_hint").to_owned()],
        Puzzle::Collect { n, taken } => vec![
            fill(label("station_parts"), &[("taken", &taken.len().to_string()), ("total", &n.to_string())]),
            label("station_parts_hint").to_owned(),
        ],
        Puzzle::Crank { n, done } => vec![fill(label("station_turns"), &[("done", &done.to_string()), ("total", &n.to_string())]), label("station_crank_hint").to_owned()],
        Puzzle::Switches { order, next } => vec![
            fill(label("station_order"), &[("digits", &digits(order, 1))]),
            match order.get(*next) {
                Some(&breaker) => fill(label("station_next"), &[("n", &(breaker + 1).to_string())]),
                None => label("station_all_flipped").to_owned(),
            },
        ],
        Puzzle::Hold => vec![label("station_hold_hint").to_owned()],
        Puzzle::None => Vec::new(),
    }
}

// ---- input -----------------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse. C, F, H and E (or holding their button) are held; the rest are presses.
pub fn poll_controls(run: &FacilityRun) -> Controls {
    let mut c = Controls {
        crawl: is_key_down(KeyCode::C),
        light: is_key_down(KeyCode::F),
        hide: is_key_down(KeyCode::H),
        work: is_key_down(KeyCode::E),
        skip_call: is_key_pressed(KeyCode::Enter),
        mute_call: is_key_pressed(KeyCode::M),
        confirm: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space),
        back: is_key_pressed(KeyCode::Escape),
        ..Controls::default()
    };
    let click = is_mouse_button_pressed(MouseButton::Left);
    let held = is_mouse_button_down(MouseButton::Left);
    if run.end.is_some() {
        c.confirm |= click;
        return c;
    }

    if is_key_pressed(KeyCode::A) {
        c.door = Some(Door::Left);
    }
    if is_key_pressed(KeyCode::D) {
        c.door = Some(Door::Right);
    }
    if is_key_pressed(KeyCode::W) {
        c.door = Some(Door::Lift);
    }
    for (digit, key) in DIGIT_KEYS.iter().enumerate() {
        if is_key_pressed(*key) {
            c.press = press_for_digit(run.sim.puzzle(), digit);
        }
    }
    if is_key_pressed(KeyCode::Space) && matches!(run.sim.puzzle(), Puzzle::Crank { .. }) {
        c.press = Some(Press::Crank);
    }

    let (sw, sh) = (screen_width(), screen_height());
    let l = layout(sw, sh, run.sim.tasks.len());
    let mouse = Vec2::from(mouse_position());
    let on = |b: Btn| l.buttons[b as usize].contains(mouse);
    if held {
        c.crawl |= on(Btn::Crawl);
        c.light |= on(Btn::Beacon);
        c.hide |= on(Btn::Hide);
        c.work |= on(Btn::Hold);
    }
    if click {
        if let Some((_, press)) = station_slots(run.sim.puzzle(), l.station).into_iter().find(|(r, _)| r.contains(mouse)) {
            c.press = Some(press);
        } else if on(Btn::Left) {
            c.door = Some(Door::Left);
        } else if on(Btn::Right) {
            c.door = Some(Door::Right);
        } else if on(Btn::Lift) {
            c.door = Some(Door::Lift);
        } else if !l.station.contains(mouse) {
            // the doorways themselves (not the station panel that sits in front of the lift door)
            for (i, door) in DOORS.iter().enumerate() {
                if l.doors[i].contains(mouse) {
                    c.door = Some(*door);
                }
            }
        }
    }
    c
}

// ---- drawing ---------------------------------------------------------------------------------------------------

fn text_centered(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_text(text, cx - w * 0.5, y, size, color);
}

fn button(rect: Rect, label: &str, active: bool, enabled: bool) {
    let edge = if !enabled {
        DIM
    } else if active {
        AMBER
    } else {
        INK
    };
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, edge);
    text_centered(label, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5 + 7.0, 20.0, edge);
}

/// The hunter id the sprite file and `data/sprites.json` use.
fn sprite_id(hunter: Hunter) -> &'static str {
    match hunter {
        Hunter::Opalie => "opalie",
        Hunter::Porcelle => "porcelle",
        Hunter::Quenelle => "quenelle",
    }
}

/// A hunter standing on `base_y`, `h` tall, drawn as her cut-out. Each hunter faces the player in these rooms (the web
/// story draws every hunter from the front), so the front sprite is used. The shape is only a fallback for a missing sprite.
fn hunter_figure(hunter: Hunter, cx: f32, base_y: f32, h: f32, color: Color, alpha: f32) {
    let rect = Rect::new(cx - h * 0.5, base_y - h, h, h);
    if sprites::draw_hunter(sprite_id(hunter), Facing::Front, rect, Color::new(1.0, 1.0, 1.0, alpha)) {
        return;
    }
    figure(cx, base_y, h, color, alpha);
}

/// A hunter silhouette standing on `base_y`, `h` tall, with glowing eyes (as in Classic). Fallback only.
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

pub fn draw(run: &FacilityRun, art: &Art) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    let l = layout(sw, sh, run.sim.tasks.len());
    draw_room(run, art, sw, sh);
    draw_doorways(run, &l);
    draw_station(run, &l);
    draw_task_list(run, &l);
    draw_hunter_panel(run, &l);
    draw_buttons(run, &l);
    draw_hud(run, sw);
    draw_overlays(run, sw, sh);
    run.view.draw(&run.call, l.call);
    for (i, (note, _)) in run.notes.iter().enumerate() {
        text_centered(note, sw * 0.5, sh - 36.0 - (run.notes.len() - 1 - i) as f32 * 26.0, 22.0, AMBER);
    }
    if run.flash_ms > 0.0 {
        let a = (run.flash_ms / ui_ms("flash_ms")) as f32 * 0.55;
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, a));
    }
    if let Some(outcome) = run.end {
        draw_result(run, outcome, sw, sh);
    }
}

/// The room picture, the hunters that stand in it, then the foreground (the desk in Control).
fn draw_room(run: &FacilityRun, art: &Art, sw: f32, sh: f32) {
    let idx = run.sim.room as usize;
    let fit = || DrawTextureParams { dest_size: Some(vec2(sw, sh)), ..Default::default() };
    draw_texture_ex(&art.back[idx], 0.0, 0.0, WHITE, fit());
    draw_hunters(run, sw, sh);
    draw_texture_ex(&art.front[idx], 0.0, 0.0, WHITE, fit());
}

/// Opalie in the gallery and Porcelle in the workshop or auditorium stand closer the nearer they are. Quenelle is in the
/// lift doorway while it is open at Control or in the lift.
fn draw_hunters(run: &FacilityRun, sw: f32, sh: f32) {
    let sim = &run.sim;
    let base = sh * 0.74;
    let start = sim.start_distance().max(1) as f32;
    for hunter in Hunter::ALL {
        let color = HUNTER_COLORS[hunter as usize];
        if sim.level(hunter) <= 0 {
            continue;
        }
        match hunter {
            Hunter::Quenelle => {
                let open = sim.lift_phase() == LiftPhase::Open && matches!(sim.room, Room::Control | Room::Lift);
                if open {
                    let alpha = 0.5 + 0.45 * sim.lift_exposure() as f32;
                    hunter_figure(hunter, sw * 0.5, base, sh * 0.62, color, alpha);
                    text_centered(hunter.name(), sw * 0.5, base - sh * 0.66, 20.0, INK);
                }
            }
            _ => {
                let here = matches!((hunter, sim.room), (Hunter::Opalie, Room::Gallery) | (Hunter::Porcelle, Room::Workshop | Room::Auditorium));
                if !here {
                    continue;
                }
                let dist = sim.distance(hunter).unwrap_or(0).max(0) as f32;
                let frac = (dist / start).clamp(0.0, 1.0);
                let side = if hunter == Hunter::Opalie { -1.0 } else { 1.0 };
                let cx = sw * 0.5 + side * sw * 0.30 * frac;
                let h = sh * (0.30 + 0.45 * (1.0 - frac));
                hunter_figure(hunter, cx, base, h, color, 0.95);
                text_centered(hunter.name(), cx, base - h - 8.0, 20.0, INK);
            }
        }
    }
}

/// The exits: the room behind each side door (and the lift in Control) named on its doorway.
fn draw_doorways(run: &FacilityRun, l: &Layout) {
    let room = run.sim.room;
    if let Some(next) = room.exit(Door::Left) {
        let r = l.doors[Door::Left as usize];
        draw_text(&fill(label("exit_left"), &[("room", next.label())]), r.x + 12.0, r.y + 24.0, 18.0, DIM);
    }
    if let Some(next) = room.exit(Door::Right) {
        let r = l.doors[Door::Right as usize];
        let text = fill(label("exit_right"), &[("room", next.label())]);
        let w = measure_text(&text, None, 18, 1.0).width;
        draw_text(&text, r.right() - w - 12.0, r.y + 24.0, 18.0, DIM);
    }
    if room.exit(Door::Lift).is_some() {
        let r = l.doors[Door::Lift as usize];
        text_centered(label("lift_door"), r.x + r.w * 0.5, r.y - 8.0, 18.0, DIM);
    }
}

fn draw_station(run: &FacilityRun, l: &Layout) {
    let sim = &run.sim;
    let Some(task) = sim.current_task() else { return };
    if task.room != sim.room || sim.moving.is_some() {
        return;
    }
    let area = l.station;
    draw_rectangle(area.x, area.y, area.w, area.h, PANEL);
    draw_rectangle_lines(area.x, area.y, area.w, area.h, 2.0, AMBER);
    text_centered(&task.label.to_uppercase(), area.x + area.w * 0.5, area.y + 34.0, 28.0, AMBER);
    for (i, line) in station_lines(sim.puzzle()).iter().enumerate() {
        text_centered(line, area.x + area.w * 0.5, area.y + 62.0 + i as f32 * 24.0, 20.0, INK);
    }
    if let TaskKind::Hold(ms) = task.kind {
        let frac = (sim.hold_ms() / f64::from(ms)).clamp(0.0, 1.0) as f32;
        let bar = Rect::new(area.x + 40.0, area.y + 104.0, area.w - 80.0, 12.0);
        draw_rectangle(bar.x, bar.y, bar.w, bar.h, Color::new(0.2, 0.2, 0.2, 1.0));
        draw_rectangle(bar.x, bar.y, bar.w * frac, bar.h, AMBER);
    }
    for (rect, press) in station_slots(sim.puzzle(), area) {
        let taken = press_taken(sim.puzzle(), press);
        button(rect, &press_label(sim.puzzle(), press), false, !taken);
    }
}

fn draw_task_list(run: &FacilityRun, l: &Layout) {
    let r = l.task;
    draw_rectangle(r.x, r.y, r.w, r.h, PANEL);
    draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, DIM);
    draw_text(label("task_list"), r.x + 12.0, r.y + 26.0, 20.0, AMBER);
    for (i, task) in run.sim.tasks.iter().enumerate() {
        let y = r.y + 44.0 + i as f32 * 36.0;
        let (mark, color) = if i < run.sim.task_idx {
            (label("mark_done"), DIM)
        } else if i == run.sim.task_idx {
            (label("mark_current"), AMBER)
        } else {
            (label("mark_todo"), INK)
        };
        draw_text(&format!("{mark} {}", task.label), r.x + 12.0, y + 14.0, 18.0, color);
        draw_text(task.room.label(), r.x + 40.0, y + 30.0, 14.0, DIM);
    }
}

fn draw_hunter_panel(run: &FacilityRun, l: &Layout) {
    let r = l.hunters;
    let sim = &run.sim;
    draw_rectangle(r.x, r.y, r.w, r.h, PANEL);
    draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, DIM);
    for (i, hunter) in Hunter::ALL.into_iter().enumerate() {
        let y = r.y + 12.0 + i as f32 * 38.0;
        let color = HUNTER_COLORS[hunter as usize];
        draw_text(hunter.name(), r.x + 12.0, y + 18.0, 20.0, color);
        if sim.level(hunter) <= 0 {
            draw_text(label("not_tonight"), r.x + 120.0, y + 18.0, 16.0, DIM);
            continue;
        }
        match sim.distance(hunter) {
            Some(dist) => {
                let start = sim.start_distance().max(0);
                let closeness = (start - dist.max(0)).clamp(0, start);
                for pip in 0..start {
                    let px = r.x + 120.0 + pip as f32 * 16.0;
                    let lit = pip < closeness;
                    draw_rectangle(px, y + 6.0, 11.0, 11.0, if lit { color } else { Color::new(0.2, 0.2, 0.2, 1.0) });
                }
                let (state, state_color) = if dist <= 0 {
                    (label("at_you"), RED_TEXT)
                } else if dist == 1 {
                    (label("close"), AMBER)
                } else {
                    (label("far"), DIM)
                };
                draw_text(state, r.x + 12.0, y + 34.0, 16.0, state_color);
            }
            None => {
                let (state, state_color) = match sim.lift_phase() {
                    LiftPhase::Away => (label("away"), DIM),
                    LiftPhase::Bell => (label("bell"), AMBER),
                    LiftPhase::Open => (label("open"), RED_TEXT),
                };
                draw_text(state, r.x + 120.0, y + 18.0, 16.0, state_color);
                let expose = sim.lift_exposure() as f32;
                draw_rectangle(r.x + 12.0, y + 28.0, r.w - 24.0, 6.0, Color::new(0.2, 0.2, 0.2, 1.0));
                draw_rectangle(r.x + 12.0, y + 28.0, (r.w - 24.0) * expose, 6.0, RED_TEXT);
            }
        }
    }
}

fn draw_buttons(run: &FacilityRun, l: &Layout) {
    let sim = &run.sim;
    let station_hold = matches!(sim.current_task(), Some(task) if task.room == sim.room && matches!(task.kind, TaskKind::Hold(_)));
    for button_kind in BUTTONS {
        let rect = l.buttons[button_kind as usize];
        let (active, enabled) = match button_kind {
            Btn::Left => (false, sim.room.exit(Door::Left).is_some()),
            Btn::Right => (false, sim.room.exit(Door::Right).is_some()),
            Btn::Lift => (false, sim.room.exit(Door::Lift).is_some()),
            Btn::Crawl => (sim.crawl, true),
            Btn::Beacon => (sim.light, true),
            Btn::Hold => (sim.working, station_hold),
            Btn::Hide => (sim.hidden, sim.room == Room::Control),
        };
        button(rect, btn_label(button_kind), active, enabled);
    }
}

fn draw_hud(run: &FacilityRun, sw: f32) {
    let sim = &run.sim;
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    draw_text(&fill(label("hud_night"), &[("n", &run.night.to_string())]), 24.0, 30.0, 28.0, INK);
    draw_text(sim.room.label(), 170.0, 30.0, 22.0, DIM);
    let total = sim.tasks.len();
    let task_text = fill(label("hud_task"), &[("task", &(sim.task_idx + 1).min(total).to_string()), ("total", &total.to_string())]);
    text_centered(&task_text, sw * 0.5, 31.0, 26.0, AMBER);

    let gauge = |y: f32, text: &str, frac: f32, mark: Option<f32>, color: Color| {
        draw_text(text, sw - 430.0, y + 9.0, 16.0, DIM);
        let bar = Rect::new(sw - 340.0, y, 240.0, 9.0);
        draw_rectangle(bar.x, bar.y, bar.w, bar.h, Color::new(0.2, 0.2, 0.2, 1.0));
        draw_rectangle(bar.x, bar.y, bar.w * frac.clamp(0.0, 1.0), bar.h, color);
        if let Some(at) = mark {
            draw_line(bar.x + bar.w * at, bar.y - 3.0, bar.x + bar.w * at, bar.y + bar.h + 3.0, 2.0, RED_TEXT);
        }
    };
    let noise_max = facility::noise_max() as f32;
    let hear = (sim.hear() as f32 / noise_max).clamp(0.0, 1.0);
    let noise = sim.noise as f32 / noise_max;
    gauge(6.0, label("noise"), noise, Some(hear), if noise > hear { RED_TEXT } else { INK });
    let charge_max = facility::charge_max() as f32;
    gauge(24.0, label("beacon"), sim.charge as f32 / charge_max, None, AMBER);
}

/// A line of text on a dark strip, so it reads over the room picture.
fn banner(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_rectangle(cx - w * 0.5 - 16.0, y - size - 4.0, w + 32.0, size + 18.0, PANEL);
    text_centered(text, cx, y, size, color);
}

/// Walking, hiding, the lift bell and the lift's open doors, drawn over the room.
fn draw_overlays(run: &FacilityRun, sw: f32, sh: f32) {
    let sim = &run.sim;
    if let Some(walk) = sim.moving {
        let verb = if sim.crawl { label("crawling") } else { label("walking") };
        banner(&fill(label("walk_to"), &[("verb", verb), ("room", walk.to.label())]), sw * 0.5, sh * 0.52, 30.0, INK);
    } else if sim.hidden {
        banner(label("under_desk"), sw * 0.5, sh * 0.52, 30.0, DIM);
    }
    if sim.room == Room::Control || sim.room == Room::Lift {
        match sim.lift_phase() {
            LiftPhase::Bell => banner(label("bell_rings"), sw * 0.5, sh * 0.28, 26.0, AMBER),
            LiftPhase::Open => {
                draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, 0.45 * sim.lift_exposure() as f32));
                banner(label("quenelle_at_lift"), sw * 0.5, sh * 0.28, 26.0, RED_TEXT);
            }
            LiftPhase::Away => {}
        }
    }
}

fn draw_result(run: &FacilityRun, outcome: Outcome, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let (title, colour, line) = match outcome {
        Outcome::Win => (
            fill(label("result_signed"), &[("n", &run.night.to_string())]),
            AMBER,
            fill(label("result_points"), &[("score", &run.sim.score().to_string())]),
        ),
        Outcome::Caught(hunter) => (label("result_caught").to_owned(), RED_TEXT, fill(label("result_reached"), &[("name", hunter.name())])),
    };
    text_centered(&title, sw * 0.5, sh * 0.40, 64.0, colour);
    text_centered(&line, sw * 0.5, sh * 0.40 + 48.0, 28.0, INK);
    let next = match outcome {
        Outcome::Win if run.night < facility::night_count() => label("result_next"),
        Outcome::Win => label("result_hub"),
        Outcome::Caught(_) => label("result_retry"),
    };
    if run.end_ms >= ui_ms("result_lock_ms") {
        text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
        text_centered(label("result_esc"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
    }
}

// ---- scripted runs (the --screenshot --facility flags) ----------------------------------------------------------

/// Plays `frames` fixed steps of `autoplay` (god mode on, so the picture is never cut short by a catch) and returns the
/// run ready to draw. Nothing is saved or played.
pub fn run_script(calls: &CallsData, seed: i64, night: u32, frames: u32) -> FacilityRun {
    let mut run = FacilityRun::new(calls, night, seed, false);
    run.sim.god = true;
    for _ in 0..frames {
        let controls = autoplay(&run);
        run.update(script_step_ms(), &controls);
        if run.end.is_some() {
            break;
        }
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME_MS: f64 = 1000.0 / 60.0;

    fn calls() -> CallsData {
        CallsData::load()
    }

    /// Runs the bot until the shift ends or `limit` frames pass. None when it ran out of frames first.
    fn play_out(run: &mut FacilityRun, limit: u32) -> Option<Outcome> {
        for _ in 0..limit {
            let controls = autoplay(run);
            run.update(FRAME_MS, &controls);
            if let Some(end) = run.end {
                return Some(end);
            }
        }
        None
    }

    #[test]
    fn the_bot_signs_off_every_night_with_god_mode_on() {
        let calls = calls();
        for night in 1..=facility::night_count() {
            let mut run = FacilityRun::new(&calls, night, 20_261_103 + i64::from(night), false);
            run.sim.god = true;
            assert_eq!(play_out(&mut run, 200_000), Some(Outcome::Win), "night {night}");
            assert_eq!(run.sim.task_idx, run.sim.tasks.len(), "night {night}: every task is done");
        }
    }

    #[test]
    fn without_god_mode_a_shift_ends_in_a_win_or_a_catch_within_bounds() {
        let calls = calls();
        for seed in 1..=3 {
            for night in [1, 3, 6] {
                let mut run = FacilityRun::new(&calls, night, seed, false);
                assert!(play_out(&mut run, 200_000).is_some(), "night {night} seed {seed} did not end");
            }
        }
    }

    #[test]
    fn the_call_script_rings_types_out_its_first_line() {
        let calls = calls();
        let mut run = FacilityRun::new(&calls, 1, 7, false);
        let first = calls.for_level(MODE_ID, 1).expect("night 1 has a call").lines[0].clone();
        let mut opened = false;
        for _ in 0..5_000 {
            run.update(FRAME_MS, &Controls::default());
            opened |= run.call.open;
            if run.call.line == 0 && run.call.text == first {
                break;
            }
        }
        assert!(opened, "the call rang");
        assert_eq!(run.call.text, first, "the first line typed out in full");
    }

    #[test]
    fn a_call_is_on_every_night_and_the_script_knows_them_all() {
        let calls = calls();
        for night in 1..=facility::night_count() {
            assert!(calls.for_level(MODE_ID, night as i32).is_some(), "night {night} has a call");
        }
    }

    #[test]
    fn doors_lead_toward_the_task_room() {
        assert_eq!(route_door(Room::Control, Room::Breaker), Some(Door::Left));
        assert_eq!(route_door(Room::Control, Room::Lift), Some(Door::Lift));
        assert_eq!(route_door(Room::Breaker, Room::Auditorium), Some(Door::Right));
        assert_eq!(route_door(Room::Lift, Room::Breaker), Some(Door::Lift));
        assert_eq!(route_door(Room::Gallery, Room::Gallery), None);
    }

    #[test]
    fn digit_keys_press_the_station_they_name() {
        let dials = Puzzle::Dials { target: vec![0, 1, 2], pos: vec![0, 0, 0] };
        assert_eq!(press_for_digit(&dials, 1), Some(Press::Index(0)));
        assert_eq!(press_for_digit(&dials, 3), Some(Press::Index(2)));
        assert_eq!(press_for_digit(&dials, 4), None, "there are three dials");
        assert_eq!(press_for_digit(&dials, 0), None);
        let code = Puzzle::Code { code: vec![4, 2, 9], entered: 0 };
        assert_eq!(press_for_digit(&code, 0), Some(Press::Digit(0)));
        assert_eq!(press_for_digit(&Puzzle::Hold, 1), None);
    }

    #[test]
    fn the_keypad_shows_one_to_nine_then_zero() {
        let area = Rect::new(0.0, 0.0, 480.0, 260.0);
        let slots = station_slots(&Puzzle::Code { code: vec![1], entered: 0 }, area);
        let digits: Vec<Press> = slots.iter().map(|(_, press)| *press).collect();
        assert_eq!(digits.len(), 10);
        assert_eq!(digits[0], Press::Digit(1));
        assert_eq!(digits[8], Press::Digit(9));
        assert_eq!(digits[9], Press::Digit(0));
    }

    #[test]
    fn every_facility_hunter_has_a_front_sprite_embedded() {
        for hunter in Hunter::ALL {
            assert!(data::HUNTER_SPRITE_FILES.iter().any(|(id, _, _)| *id == sprite_id(hunter)), "{} has no sprite", hunter.name());
        }
    }

    #[test]
    fn every_ui_label_is_filled_in_the_rules_file() {
        let labels = data::rules(RULES_MODE)["ui"]["labels"].as_object().expect("ui.labels is an object");
        assert!(labels.len() >= 60, "the ui block carries the screen's labels");
        for key in labels.keys() {
            assert!(!label(key).is_empty(), "ui.labels.{key} is empty");
        }
    }

    #[test]
    fn steps_and_placeholders_read_from_the_labels() {
        assert_eq!(steps_text(1), "1 step");
        assert_eq!(steps_text(3), "3 steps");
        assert_eq!(fill("{name} is {verb}.", &[("name", "Opalie"), ("verb", "near")]), "Opalie is near.");
    }
}
