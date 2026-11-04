//! The Descent, playable: the shell's driver for `modes::descent`. `DescentRun` owns the menu (the five descents and the four
//! Custom Night presets) and the shift in progress. It maps the frame's `Controls` (wire buttons and signal pads by click
//! or keys 1 to 6, pipe pieces by click or arrows and Enter, the door hold on SPACE) onto the mode's actions, plays the
//! events they return (a wrong press, a stage repaired, the end) and saves a win. `draw` paints the car with Elise at
//! the caged window, the service panel for the stage up, the HUD, the call box and the result screen. Everything but
//! `poll_controls` and `draw` is plain state, so the bot tests run without a window.

use macroquad::prelude::*;

use crate::calls_view::CallView;
use crate::data::{self, CallsData};
use crate::modes::descent::{
    DescentAction, DescentEvent, DescentShift, Memory, MemoryPhase, Outcome, Pipes, Puzzle, Side, StageKind, Wires,
};
use crate::save::NightWatchSave;
use crate::sprites::{draw_hunter, Facing};

const MODE_ID: &str = "descent";
/// Signal pads on the panel.
const PADS: usize = 4;
/// The hunter this mode draws: her cut-out from `sprites`.
const HUNTER: &str = "elise";
/// The `ui` block keys the driver reads (timings, counts and the car's motion). A test checks each one is there.
#[cfg(test)]
const UI_KEYS: [&str; 10] = [
    "max_step_ms", "result_lock_ms", "note_ms", "note_max", "flash_ms", "script_step_ms", "bob_period_ms", "bob_px",
    "rail_pitch_px", "rail_speed",
];
/// The pipe grid's steps as (row, col) for N, E, S and W, as in the mode.
const STEP: [(isize, isize); 4] = [(-1, 0), (0, 1), (1, 0), (0, -1)];
/// Every label the driver reads from `labels` in the rules file. A test checks each one is there.
#[cfg(test)]
const LABEL_KEYS: [&str; 36] = [
    "menu_title", "blurb", "menu_hint", "custom_header", "locked", "elise", "watch", "repeat", "hold_label", "hold_button",
    "shut", "riding", "closeness", "stage", "stage_done", "ride_note", "wrong_note", "wrong_count", "muted", "pipe_in",
    "pipe_out", "hint_wires", "hint_memory", "hint_pipes", "hint_hold", "hint_common", "won_title", "won_line",
    "won_custom_line", "caught_title", "caught_line", "next", "back_to_menu", "again", "esc_menu", "save_failed",
];

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);
const PANEL_PLATE: Color = Color::new(0.050, 0.044, 0.040, 0.94);
const PIPE: Color = Color::new(0.55, 0.78, 0.62, 1.0);
const ELISE: Color = Color::new(0.86, 0.82, 0.92, 1.0);
const PAD_COLOURS: [Color; PADS] = [
    Color::new(0.88, 0.38, 0.38, 1.0),
    Color::new(0.88, 0.69, 0.25, 1.0),
    Color::new(0.31, 0.69, 0.88, 1.0),
    Color::new(0.44, 0.82, 0.56, 1.0),
];
const WIRE_COLOURS: [Color; 6] = [
    Color::new(0.88, 0.69, 0.25, 1.0),
    Color::new(0.31, 0.69, 0.88, 1.0),
    Color::new(0.88, 0.38, 0.38, 1.0),
    Color::new(0.44, 0.82, 0.56, 1.0),
    Color::new(0.75, 0.50, 0.88, 1.0),
    Color::new(0.88, 0.57, 0.31, 1.0),
];

/// One line of the mode's text from `labels` in `data/rules/descent.json`.
fn label(key: &str) -> &'static str {
    data::text(data::rules(MODE_ID), &format!("labels.{key}"))
}

/// One timing or count from the `ui` block of the rules file.
fn ui(key: &str) -> f64 {
    data::num(data::rules(MODE_ID), &format!("ui.{key}"))
}

/// A line of the mode's text with its `{name}` placeholders filled in.
fn fill(key: &str, pairs: &[(&str, String)]) -> String {
    pairs.iter().fold(label(key).to_owned(), |text, (name, value)| text.replace(&format!("{{{name}}}"), value))
}

/// The campaign's descent names ("DESCENT 1" ...), from `levels`.
fn campaign_names() -> Vec<&'static str> {
    data::list(data::rules(MODE_ID), "levels").iter().map(|v| v.as_str().unwrap_or("")).collect()
}

/// A Custom Night preset: its id for the save, its name on the menu and the AI level Elise plays at.
#[derive(Clone, Copy, Debug)]
struct Preset {
    id: &'static str,
    name: &'static str,
    elise: i32,
}

fn presets() -> Vec<Preset> {
    data::list(data::rules(MODE_ID), "presets")
        .iter()
        .map(|p| Preset { id: data::text(p, "id"), name: data::text(p, "name"), elise: data::int(p, "elise") as i32 })
        .collect()
}

/// What the menu starts: a campaign descent (1 to 5) or a Custom Night preset (an index into `presets`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Campaign(usize),
    Preset(usize),
}

/// One row of the menu. A row that is not open cannot be started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub tag: String,
    pub choice: Choice,
    pub open: bool,
}

#[derive(Clone, Debug)]
pub struct Menu {
    pub rows: Vec<Row>,
    pub selected: usize,
}

impl Menu {
    /// The rows for the progress in `save`: a descent opens once the one before it is cleared, the presets once Descent 5 is.
    fn build(save: &NightWatchSave) -> Menu {
        let mut rows: Vec<Row> = campaign_names()
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let n = i + 1;
                let open = save.level_open(MODE_ID, n as u32, false);
                Row { label: (*name).to_owned(), tag: if open { String::new() } else { label("locked").to_owned() }, choice: Choice::Campaign(n), open }
            })
            .collect();
        let custom_open = save.custom_unlocked(MODE_ID);
        rows.extend(presets().iter().enumerate().map(|(i, p)| Row {
            label: p.name.to_owned(),
            tag: if custom_open { format!("{} {}", label("elise"), p.elise) } else { label("locked").to_owned() },
            choice: Choice::Preset(i),
            open: custom_open,
        }));
        Menu { rows, selected: 0 }
    }
}

/// What the shell does after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Stay,
    Hub,
}

/// Every key the driver reads for one frame. Presses are true for one frame; `hold` is down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    /// -1 or 1 moves the menu's selection.
    pub menu_step: i32,
    /// A menu row clicked.
    pub menu_pick: Option<usize>,
    /// A wire button (0-based) or a signal pad pressed, by click or by key 1 to 6.
    pub slot: Option<usize>,
    /// A pipe piece clicked.
    pub pipe_click: Option<(usize, usize)>,
    /// Arrow keys: (rows, columns) the pipe cursor moves by.
    pub cursor_step: (i32, i32),
    /// ENTER on the piece under the pipe cursor turns it.
    pub turn: bool,
    /// The door hold is down.
    pub hold: bool,
    pub confirm: bool,
    pub back: bool,
    pub skip_call: bool,
    pub mute_call: bool,
}

/// How the shift ended and what it was for, kept for the result screen and the save.
#[derive(Clone, Copy, Debug)]
struct End {
    outcome: Outcome,
    choice: Choice,
    descent: usize,
    score: i32,
}

/// The shift in progress and the choice it was started from.
struct Play {
    shift: DescentShift,
    choice: Choice,
}

pub struct DescentRun {
    menu: Option<Menu>,
    play: Option<Play>,
    save: NightWatchSave,
    persist: bool,
    seed: i64,
    /// The pipe piece the keyboard cursor is on.
    cursor: (usize, usize),
    end: Option<End>,
    end_ms: f64,
    flash_ms: f64,
    anim_ms: f64,
    notes: Vec<(String, f64)>,
    /// Sound events raised since the shell last took them (`take_cues`).
    cues: Vec<&'static str>,
    view: CallView,
}

impl DescentRun {
    /// Opens the menu. `persist` lets a win write the save file (off in tests and the screenshot runs).
    pub fn new(seed: i64, persist: bool) -> Self {
        let save = if persist { NightWatchSave::load() } else { NightWatchSave::default() };
        let menu = Some(Menu::build(&save));
        Self::blank(save, persist, seed, menu)
    }

    /// Starts `choice` straight away (the screenshot runs and the tests).
    pub fn with_choice(calls: &CallsData, choice: Choice, seed: i64, persist: bool) -> Self {
        let mut run = Self::blank(NightWatchSave::default(), persist, seed, None);
        run.begin(calls, choice);
        run
    }

    fn blank(save: NightWatchSave, persist: bool, seed: i64, menu: Option<Menu>) -> Self {
        let mut view = CallView::new();
        view.player.enabled = persist;
        Self {
            menu,
            play: None,
            save,
            persist,
            seed,
            cursor: (0, 0),
            end: None,
            end_ms: 0.0,
            flash_ms: 0.0,
            anim_ms: 0.0,
            notes: Vec::new(),
            cues: Vec::new(),
            view,
        }
    }

    /// Starts a shift for `choice` with a fresh seed each time (so a retry deals new puzzles).
    fn begin(&mut self, calls: &CallsData, choice: Choice) {
        let descents = campaign_names().len();
        let shift = match choice {
            Choice::Campaign(n) => DescentShift::start(n, self.seed, None, calls),
            Choice::Preset(i) => DescentShift::start(descents, self.seed, Some(presets().get(i).map_or(0, |p| p.elise)), calls),
        };
        self.seed = self.seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.menu = None;
        self.end = None;
        self.end_ms = 0.0;
        self.cursor = (0, 0);
        self.notes.clear();
        self.play = Some(Play { shift, choice });
    }

    fn to_menu(&mut self) {
        self.play = None;
        self.end = None;
        self.menu = Some(Menu::build(&self.save));
    }

    pub fn update(&mut self, dt_ms: f64, c: &Controls, calls: &CallsData) -> Flow {
        self.anim_ms += dt_ms;
        self.flash_ms = (self.flash_ms - dt_ms).max(0.0);
        for note in self.notes.iter_mut() {
            note.1 -= dt_ms;
        }
        self.notes.retain(|n| n.1 > 0.0);
        if self.menu.is_some() {
            return self.update_menu(c, calls);
        }
        if c.back {
            self.to_menu();
            return Flow::Stay;
        }
        if self.end.is_some() {
            self.end_ms += dt_ms;
            if c.confirm && self.end_ms >= ui("result_lock_ms") {
                self.after_end(calls);
            }
            return Flow::Stay;
        }
        self.update_shift(dt_ms, c);
        Flow::Stay
    }

    fn update_menu(&mut self, c: &Controls, calls: &CallsData) -> Flow {
        if c.back {
            return Flow::Hub;
        }
        let picked = {
            let Some(menu) = self.menu.as_mut() else { return Flow::Stay };
            let len = menu.rows.len() as i32;
            if c.menu_step != 0 {
                menu.selected = (menu.selected as i32 + c.menu_step).rem_euclid(len) as usize;
            }
            if let Some(row) = c.menu_pick {
                if row < menu.rows.len() {
                    menu.selected = row;
                }
            }
            if c.confirm || c.menu_pick.is_some() {
                menu.rows.get(menu.selected).filter(|row| row.open).map(|row| row.choice)
            } else {
                None
            }
        };
        if let Some(choice) = picked {
            self.begin(calls, choice);
        }
        Flow::Stay
    }

    fn update_shift(&mut self, dt_ms: f64, c: &Controls) {
        let dt = dt_ms.min(ui("max_step_ms"));
        let mut events: Vec<DescentEvent> = Vec::new();
        let mut muted = false;
        if let Some(play) = self.play.as_mut() {
            if c.skip_call {
                play.shift.call.skip();
            }
            muted = c.mute_call && play.shift.call.mute();
            if let Puzzle::Pipes(p) = play.shift.puzzle() {
                let (row, col) = self.cursor;
                let n = p.n as i32;
                self.cursor = (
                    (row as i32 + c.cursor_step.0).clamp(0, n - 1) as usize,
                    (col as i32 + c.cursor_step.1).clamp(0, n - 1) as usize,
                );
            }
            if let Some(at) = c.pipe_click {
                self.cursor = at;
            }
            events.extend(play.shift.step(dt, c.hold));
            play.shift.call_update(dt);
            self.view.update(&play.shift.call);
            let action = match play.shift.puzzle() {
                Puzzle::Wires(w) => c.slot.filter(|&s| s < w.buttons).map(|button| DescentAction::Wire { button }),
                Puzzle::Memory(_) => c.slot.filter(|&s| s < PADS).map(|pad| DescentAction::Pad { pad }),
                Puzzle::Pipes(p) => {
                    let target = c.pipe_click.or(c.turn.then_some(self.cursor));
                    target.filter(|&(row, col)| row < p.n && col < p.n).map(|(row, col)| DescentAction::Pipe { row, col })
                }
                Puzzle::Hold | Puzzle::Finished => None,
            };
            if let Some(action) = action {
                events.extend(play.shift.act(action));
            }
        }
        if muted {
            self.note(label("muted"));
        }
        for event in events {
            self.on_event(event);
        }
    }

    /// The sound events raised since the last call. The shell plays each with `nw_sound::event("descent", id, 0.0)`.
    pub fn take_cues(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.cues)
    }

    fn note(&mut self, text: impl Into<String>) {
        self.notes.push((text.into(), ui("note_ms")));
        if self.notes.len() > ui("note_max") as usize {
            self.notes.remove(0);
        }
    }

    fn on_event(&mut self, event: DescentEvent) {
        self.cues.push(match event {
            DescentEvent::Step => "step",
            DescentEvent::Wrong => "wrong",
            DescentEvent::StageCleared { .. } => "stage_cleared",
            DescentEvent::Ride { .. } => "ride",
            DescentEvent::Won => "won",
            DescentEvent::Caught => "caught",
        });
        match event {
            DescentEvent::Step => {}
            DescentEvent::Wrong => {
                self.flash_ms = ui("flash_ms");
                self.note(label("wrong_note"));
            }
            DescentEvent::StageCleared { number } => self.note(fill("stage_done", &[("n", number.to_string())])),
            DescentEvent::Ride { .. } => self.note(label("ride_note")),
            DescentEvent::Won => self.finish(Outcome::Won),
            DescentEvent::Caught => self.finish(Outcome::Caught),
        }
    }

    /// Ends the shift: a win is saved (the descent cleared, or the preset's best and star), a loss only shows the screen.
    fn finish(&mut self, outcome: Outcome) {
        let Some(play) = self.play.as_ref() else { return };
        let end = End { outcome, choice: play.choice, descent: play.shift.descent_number(), score: play.shift.score() };
        self.end = Some(end);
        self.end_ms = 0.0;
        if outcome == Outcome::Caught {
            self.flash_ms = ui("flash_ms");
            return;
        }
        if play.shift.is_custom() {
            self.save.record_best(MODE_ID, end.score.max(0) as u32);
            if let Choice::Preset(i) = end.choice {
                if let Some(preset) = presets().get(i) {
                    self.save.record_star(MODE_ID, preset.id);
                }
            }
        } else if let Choice::Campaign(n) = end.choice {
            self.save.record_level_clear(MODE_ID, n as u32);
            self.save.record_night_best(MODE_ID, n as u32, end.score.max(0) as u32);
        }
        if self.persist && self.save.save().is_err() {
            self.note(label("save_failed"));
        }
    }

    /// ENTER on the result: the next descent after a win, the same run again after a loss, the menu otherwise.
    fn after_end(&mut self, calls: &CallsData) {
        let Some(end) = self.end else { return };
        match (end.outcome, end.choice) {
            (Outcome::Won, Choice::Campaign(n)) if n < campaign_names().len() => self.begin(calls, Choice::Campaign(n + 1)),
            (Outcome::Caught, choice) => self.begin(calls, choice),
            _ => self.to_menu(),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.end.is_some()
    }
}

// ---- layout ----------------------------------------------------------------------------------------------------

/// Every drawn and clickable region for a window of `sw` by `sh`. Pure, so the click tests and the paint agree.
struct Layout {
    call: Rect,
    panel: Rect,
    body: Rect,
    hold_btn: Rect,
    bar: Rect,
    window_left: Rect,
    window_right: Rect,
}

fn layout(sw: f32, sh: f32) -> Layout {
    let call_w = (sw * 0.56).clamp(420.0, 760.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let panel_w = (sw * 0.375).clamp(320.0, 480.0);
    let panel = Rect::new((sw - panel_w) * 0.5, 196.0, panel_w, 340.0);
    let body = Rect::new(panel.x + 16.0, panel.y + 44.0, panel.w - 32.0, panel.h - 84.0);
    let hold_btn = Rect::new((sw - 260.0) * 0.5, panel.bottom() + 10.0, 260.0, 40.0);
    let bar = Rect::new((sw - 520.0) * 0.5, sh - 96.0, 520.0, 18.0);
    let win_w = sw * 0.16;
    let win_h = sh * 0.34;
    let win_y = sh * 0.30;
    Layout {
        call,
        panel,
        body,
        hold_btn,
        bar,
        window_left: Rect::new(sw * 0.05, win_y, win_w, win_h),
        window_right: Rect::new(sw - sw * 0.05 - win_w, win_y, win_w, win_h),
    }
}

/// The wire boxes along the top of the body, one per fuse box.
fn wire_boxes(body: Rect, n: usize) -> Vec<Rect> {
    let count = n.max(1) as f32;
    let gap = 8.0;
    let w = ((body.w - gap * (count - 1.0)) / count).min(64.0);
    let total = w * count + gap * (count - 1.0);
    let x0 = body.x + (body.w - total) * 0.5;
    (0..n).map(|i| Rect::new(x0 + i as f32 * (w + gap), body.y + 6.0, w, 50.0)).collect()
}

/// The wire buttons along the bottom of the body.
fn wire_buttons(body: Rect, n: usize) -> Vec<Rect> {
    let count = n.max(1) as f32;
    let gap = 8.0;
    let w = ((body.w - gap * (count - 1.0)) / count).min(64.0);
    let total = w * count + gap * (count - 1.0);
    let x0 = body.x + (body.w - total) * 0.5;
    (0..n).map(|i| Rect::new(x0 + i as f32 * (w + gap), body.bottom() - 52.0, w, 44.0)).collect()
}

/// The four signal pads in a 2 by 2 block.
fn pad_rects(body: Rect) -> [Rect; PADS] {
    let (pw, ph, gap) = (120.0, 96.0, 24.0);
    let x0 = body.x + (body.w - (2.0 * pw + gap)) * 0.5;
    let y0 = body.y + (body.h - (2.0 * ph + gap)) * 0.5;
    [
        Rect::new(x0, y0, pw, ph),
        Rect::new(x0 + pw + gap, y0, pw, ph),
        Rect::new(x0, y0 + ph + gap, pw, ph),
        Rect::new(x0 + pw + gap, y0 + ph + gap, pw, ph),
    ]
}

/// The pipe grid, centred in the body, row by row.
fn pipe_cells(body: Rect, n: usize) -> Vec<Vec<Rect>> {
    let count = n.max(1) as f32;
    let cell = ((body.h - 8.0) / count).min((body.w - 80.0) / count).floor();
    let total = cell * count;
    let x0 = body.x + (body.w - total) * 0.5;
    let y0 = body.y + (body.h - total) * 0.5;
    (0..n)
        .map(|r| (0..n).map(|c| Rect::new(x0 + c as f32 * cell, y0 + r as f32 * cell, cell, cell)).collect())
        .collect()
}

/// The menu row `i`: the five descents, a gap, then the four presets.
fn menu_row(i: usize, sw: f32, sh: f32) -> Rect {
    let w = 520.0;
    let top = sh * 0.30;
    let y = if i < 5 { top + i as f32 * 48.0 } else { top + 5.0 * 48.0 + 36.0 + (i - 5) as f32 * 48.0 };
    Rect::new((sw - w) * 0.5, y, w, 44.0)
}

// ---- input -----------------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse. Keys 1 to 6 press the wire button or pad of that number; arrows move the pipe
/// cursor and ENTER turns the piece under it; SPACE holds the door.
pub fn poll_controls(run: &DescentRun) -> Controls {
    let (sw, sh) = (screen_width(), screen_height());
    let mouse = Vec2::from(mouse_position());
    let click = is_mouse_button_pressed(MouseButton::Left);
    let held = is_mouse_button_down(MouseButton::Left);
    let up = is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::W);
    let down = is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::S);
    let mut c = Controls {
        menu_step: i32::from(down) - i32::from(up),
        cursor_step: (
            i32::from(is_key_pressed(KeyCode::Down)) - i32::from(is_key_pressed(KeyCode::Up)),
            i32::from(is_key_pressed(KeyCode::Right)) - i32::from(is_key_pressed(KeyCode::Left)),
        ),
        confirm: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space),
        turn: is_key_pressed(KeyCode::Enter),
        hold: is_key_down(KeyCode::Space),
        back: is_key_pressed(KeyCode::Escape),
        skip_call: is_key_pressed(KeyCode::Tab),
        mute_call: is_key_pressed(KeyCode::M),
        ..Controls::default()
    };
    let digits = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5, KeyCode::Key6];
    c.slot = digits.iter().position(|key| is_key_pressed(*key));

    if let Some(menu) = run.menu.as_ref() {
        if click {
            c.menu_pick = (0..menu.rows.len()).find(|&i| menu_row(i, sw, sh).contains(mouse));
        }
        return c;
    }
    if run.end.is_some() {
        c.confirm |= click;
        return c;
    }
    let l = layout(sw, sh);
    if held && l.hold_btn.contains(mouse) {
        c.hold = true;
    }
    if click {
        if let Some(play) = run.play.as_ref() {
            match play.shift.puzzle() {
                Puzzle::Wires(w) => c.slot = wire_buttons(l.body, w.buttons).iter().position(|r| r.contains(mouse)),
                Puzzle::Memory(_) => c.slot = pad_rects(l.body).iter().position(|r| r.contains(mouse)),
                Puzzle::Pipes(p) => {
                    c.pipe_click = pipe_cells(l.body, p.n).iter().enumerate().find_map(|(row, cells)| {
                        cells.iter().position(|cell| cell.contains(mouse)).map(|col| (row, col))
                    });
                }
                Puzzle::Hold | Puzzle::Finished => {}
            }
            if l.hold_btn.contains(mouse) && matches!(play.shift.puzzle(), Puzzle::Hold) {
                c.hold = true;
            }
        }
    }
    c
}

// ---- bot (tests and the scripted screenshot) --------------------------------------------------------------------

/// The route through the pipework a player who knows the answer would take: (row, col, rot) for each piece on the line
/// from the left edge to the right edge.
fn pipe_route(p: &Pipes) -> Option<Vec<(usize, usize, usize)>> {
    let mut seen = vec![vec![false; p.n]; p.n];
    let mut path = Vec::new();
    route_from(p, (0, 0), 3, &mut seen, &mut path).then_some(path)
}

fn route_from(p: &Pipes, at: (usize, usize), enter: usize, seen: &mut [Vec<bool>], path: &mut Vec<(usize, usize, usize)>) -> bool {
    let (row, col) = at;
    if seen[row][col] {
        return false;
    }
    seen[row][col] = true;
    for (rot, pair) in p.tiles[row][col].kind.pairs().iter().enumerate() {
        if !pair.contains(&enter) {
            continue;
        }
        let out = if pair[0] == enter { pair[1] } else { pair[0] };
        path.push((row, col, rot));
        if row == p.n - 1 && col == p.n - 1 && out == 1 {
            return true;
        }
        let next = (row as isize + STEP[out].0, col as isize + STEP[out].1);
        if next.0 >= 0 && next.1 >= 0 && (next.0 as usize) < p.n && (next.1 as usize) < p.n {
            let next = (next.0 as usize, next.1 as usize);
            if route_from(p, next, (out + 2) % 4, seen, path) {
                return true;
            }
        }
        path.pop();
    }
    seen[row][col] = false;
    false
}

/// The piece the route needs turned next, if any is still wrong.
fn pipe_fix(p: &Pipes) -> Option<(usize, usize)> {
    pipe_route(p)?
        .into_iter()
        .find(|&(row, col, rot)| p.tiles[row][col].rot != rot)
        .map(|(row, col, _)| (row, col))
}

/// The controls a player who knows the answer sends this frame: the next wire, the next pad of the signal, the next
/// wrong piece of the pipe route, or the door hold. The tests and the `--solve` screenshot run use it.
pub fn bot_controls(run: &DescentRun) -> Controls {
    let mut c = Controls::default();
    let Some(play) = run.play.as_ref() else { return c };
    let shift = &play.shift;
    if shift.riding() {
        return c;
    }
    match shift.puzzle() {
        Puzzle::Wires(w) => {
            if let Some(&lit) = w.lit.get(w.step) {
                c.slot = Some(w.wire[lit]);
            }
        }
        Puzzle::Memory(m) => {
            if m.phase == MemoryPhase::Input {
                c.slot = m.seq.get(m.step).copied();
            }
        }
        Puzzle::Pipes(p) => c.pipe_click = pipe_fix(p),
        Puzzle::Hold => c.hold = true,
        Puzzle::Finished => {}
    }
    c
}

// ---- scripted runs (the --descent-shot flag) ---------------------------------------------------------------------

/// What `--descent-shot` does before its picture: the descent (0 for the menu), how many fixed frames to run, and how
/// many stages the bot repairs first (`--solve`).
#[derive(Clone, Debug, Default)]
pub struct ShotPlan {
    pub descent: usize,
    pub frames: u32,
    pub solve: usize,
}

/// The `--descent-shot PATH` request: its output path and plan, or None for a normal start.
pub fn shot_request(args: &[String]) -> Option<(String, ShotPlan)> {
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let number = |flag: &str| value(flag).and_then(|v| v.parse::<usize>().ok());
    let path = value("--descent-shot")?;
    Some((
        path,
        ShotPlan { descent: number("--descent").unwrap_or(1), frames: number("--frames").unwrap_or(300) as u32, solve: number("--solve").unwrap_or(0) },
    ))
}

/// Starts the descent in `plan`, lets the bot repair `plan.solve` stages, then runs the frames. Nothing is saved.
pub fn run_script(calls: &CallsData, seed: i64, plan: &ShotPlan) -> DescentRun {
    // `--descent 0` opens the menu instead of a descent
    let mut run = if plan.descent == 0 {
        DescentRun::new(seed, false)
    } else {
        DescentRun::with_choice(calls, Choice::Campaign(plan.descent), seed, false)
    };
    let mut solved = 0;
    for _ in 0..20_000 {
        if solved >= plan.solve || run.is_finished() {
            break;
        }
        let before = stages_cleared(&run);
        let c = bot_controls(&run);
        run.update(ui("script_step_ms"), &c, calls);
        if stages_cleared(&run) > before {
            solved += 1;
        }
    }
    for _ in 0..plan.frames {
        run.update(ui("script_step_ms"), &Controls::default(), calls);
    }
    run
}

fn stages_cleared(run: &DescentRun) -> usize {
    run.play.as_ref().map_or(0, |play| play.shift.stages_cleared())
}

// ---- drawing ---------------------------------------------------------------------------------------------------

fn text_centered(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_text(text, cx - w * 0.5, y, size, color);
}

fn button(rect: Rect, text: &str, active: bool) {
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, if active { AMBER } else { DIM });
    text_centered(text, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5 + 7.0, 22.0, if active { AMBER } else { INK });
}

/// A figure standing on `base_y`, `h` tall, with glowing eyes (as in Classic's monitor).
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

/// The Descent room from the web build (`offices/descent.png`) and its foreground lip (`descent-fg.png`).
struct OfficeArt {
    back: Texture2D,
    front: Texture2D,
}

thread_local! {
    static OFFICE: OfficeArt = OfficeArt {
        back: Texture2D::from_file_with_format(include_bytes!("../assets/offices/descent.png"), Some(ImageFormat::Png)),
        front: Texture2D::from_file_with_format(include_bytes!("../assets/offices/descent-fg.png"), Some(ImageFormat::Png)),
    };
}

fn full_screen(tex: &Texture2D, sw: f32, sh: f32) {
    draw_texture_ex(tex, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(vec2(sw, sh)), ..Default::default() });
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::new(a.r + (b.r - a.r) * t, a.g + (b.g - a.g) * t, a.b + (b.b - a.b) * t, 1.0)
}

pub fn draw(run: &DescentRun) {
    let (sw, sh) = (screen_width(), screen_height());
    if let Some(menu) = run.menu.as_ref() {
        draw_menu(menu, sw, sh);
        return;
    }
    let Some(play) = run.play.as_ref() else { return };
    let l = layout(sw, sh);
    draw_car(run, play, &l, sw, sh);
    draw_panel(run, play, &l);
    draw_controls(play, &l);
    draw_hud(play, &l, sw, sh);
    run.view.draw(&play.shift.call, l.call);
    for (i, (note, _)) in run.notes.iter().enumerate() {
        text_centered(note, sw * 0.5, l.bar.y - 16.0 - (run.notes.len() - 1 - i) as f32 * 22.0, 20.0, AMBER);
    }
    if run.flash_ms > 0.0 {
        let a = (run.flash_ms / ui("flash_ms")) as f32 * 0.45;
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, a));
    }
    if let Some(end) = run.end {
        draw_result(end, run.end_ms, sw, sh);
    }
}

fn draw_menu(menu: &Menu, sw: f32, sh: f32) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    text_centered(label("menu_title"), sw * 0.5, sh * 0.16, 56.0, AMBER);
    text_centered(label("blurb"), sw * 0.5, sh * 0.16 + 34.0, 20.0, DIM);
    text_centered(label("custom_header"), sw * 0.5, menu_row(5, sw, sh).y - 14.0, 18.0, DIM);
    for (i, row) in menu.rows.iter().enumerate() {
        let r = menu_row(i, sw, sh);
        let active = i == menu.selected;
        draw_rectangle(r.x, r.y, r.w, r.h, PANEL);
        if active && row.open {
            draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, AMBER);
        }
        let ink = if !row.open {
            DIM
        } else if active {
            AMBER
        } else {
            INK
        };
        draw_text(&row.label, r.x + 16.0, r.y + 28.0, 26.0, ink);
        let tag_w = measure_text(&row.tag, None, 18, 1.0).width;
        draw_text(&row.tag, r.x + r.w - tag_w - 16.0, r.y + 27.0, 18.0, DIM);
    }
    text_centered(label("menu_hint"), sw * 0.5, sh - 24.0, 18.0, DIM);
}

/// The car: the back wall and the shaft beyond it, the rails sliding past while it rides, and Elise at the caged window.
fn draw_car(run: &DescentRun, play: &Play, l: &Layout, sw: f32, sh: f32) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    OFFICE.with(|art| full_screen(&art.back, sw, sh));
    let pitch = ui("rail_pitch_px") as f32;
    let slide = if play.shift.riding() { (run.anim_ms * ui("rail_speed")) as f32 % pitch } else { 0.0 };
    let mut y = -pitch + slide;
    while y < sh * 0.78 {
        draw_line(0.0, y, sw, y, 1.0, Color::new(0.09, 0.09, 0.11, 1.0));
        y += pitch;
    }
    let win = match play.shift.side() {
        Side::Left => l.window_left,
        Side::Right => l.window_right,
    };
    draw_rectangle(win.x, win.y, win.w, win.h, Color::new(0.02, 0.04, 0.06, 1.0));
    let t = (play.shift.threat() / 100.0).clamp(0.0, 1.0) as f32;
    let bob = if play.shift.riding() { 0.0 } else { (run.anim_ms / ui("bob_period_ms")).sin() as f32 * ui("bob_px") as f32 };
    let base_y = win.bottom() - 4.0 + bob;
    let height = win.h * (0.5 + 0.4 * t);
    let alpha = 0.35 + 0.6 * t;
    // Elise faces the glass, so her front cut-out; the figure is only the fallback for a missing sprite.
    let cut_out = Rect::new(win.x, base_y - height, win.w, height);
    if !draw_hunter(HUNTER, Facing::Front, cut_out, Color::new(1.0, 1.0, 1.0, alpha)) {
        figure(win.x + win.w * 0.5, base_y, height, ELISE, alpha);
    }
    let mut x = win.x;
    while x <= win.right() {
        draw_line(x, win.y, x, win.bottom(), 3.0, Color::new(0.2, 0.2, 0.22, 1.0));
        x += 18.0;
    }
    draw_rectangle_lines(win.x, win.y, win.w, win.h, 4.0, Color::new(0.25, 0.24, 0.22, 1.0));
    text_centered(label("elise"), win.x + win.w * 0.5, win.bottom() + 20.0, 20.0, DIM);
    OFFICE.with(|art| full_screen(&art.front, sw, sh));
    if t > 0.0 {
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.5, 0.02, 0.02, t * 0.22));
    }
}

fn draw_panel(run: &DescentRun, play: &Play, l: &Layout) {
    let p = l.panel;
    draw_rectangle(p.x, p.y, p.w, p.h, PANEL_PLATE);
    draw_rectangle_lines(p.x, p.y, p.w, p.h, 3.0, Color::new(0.35, 0.30, 0.22, 1.0));
    let shift = &play.shift;
    text_centered(shift.puzzle().stage().map_or("", StageKind::title), p.x + p.w * 0.5, p.y + 28.0, 24.0, AMBER);
    if shift.riding() {
        let b = l.body;
        draw_rectangle(b.x, b.y, b.w, b.h, Color::new(0.0, 0.0, 0.0, 0.6));
        text_centered(label("riding"), b.x + b.w * 0.5, b.y + b.h * 0.45, 26.0, INK);
        let done = 1.0 - (shift.ride_left_ms() / shift.ride_total_ms().max(1.0)).clamp(0.0, 1.0) as f32;
        draw_rectangle(b.x + 40.0, b.y + b.h * 0.55, b.w - 80.0, 14.0, Color::new(0.2, 0.2, 0.2, 1.0));
        draw_rectangle(b.x + 40.0, b.y + b.h * 0.55, (b.w - 80.0) * done, 14.0, AMBER);
        return;
    }
    match shift.puzzle() {
        Puzzle::Wires(w) => draw_wires(w, l.body),
        Puzzle::Memory(m) => draw_memory(m, l.body),
        Puzzle::Pipes(pipes) => draw_pipes(run, pipes, l.body),
        Puzzle::Hold => draw_hold(shift, l.body),
        Puzzle::Finished => {}
    }
}

fn draw_wires(w: &Wires, body: Rect) {
    let boxes = wire_boxes(body, w.boxes);
    let buttons = wire_buttons(body, w.buttons);
    let active = w.lit.get(w.step).copied();
    for i in 0..w.boxes {
        let col = WIRE_COLOURS[w.colors[i] % WIRE_COLOURS.len()];
        let from = boxes[i];
        let to = buttons[w.wire[i]];
        let alpha = if active == Some(i) { 0.9 } else { 0.22 };
        draw_line(from.x + from.w * 0.5, from.bottom(), to.x + to.w * 0.5, to.y, 3.0, Color::new(col.r, col.g, col.b, alpha));
    }
    for (i, r) in boxes.iter().enumerate() {
        let col = WIRE_COLOURS[w.colors[i] % WIRE_COLOURS.len()];
        let on = active == Some(i);
        draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.04, 0.035, 0.03, 1.0));
        let lamp = if on { col } else { Color::new(0.12, 0.11, 0.10, 1.0) };
        draw_rectangle(r.x + 6.0, r.y + 6.0, r.w - 12.0, r.h - 12.0, lamp);
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, DIM);
    }
    for (j, r) in buttons.iter().enumerate() {
        draw_rectangle(r.x, r.y, r.w, r.h, PANEL);
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, DIM);
        text_centered(&(j + 1).to_string(), r.x + r.w * 0.5, r.y + r.h * 0.5 + 8.0, 24.0, INK);
    }
}

fn draw_memory(m: &Memory, body: Rect) {
    for (i, r) in pad_rects(body).iter().enumerate() {
        let col = PAD_COLOURS[m.hues[i] % PADS];
        let on = m.lit == Some(i);
        let shade = if on { 1.0 } else { 0.22 };
        draw_rectangle(r.x, r.y, r.w, r.h, Color::new(col.r * shade, col.g * shade, col.b * shade, 1.0));
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, if on { INK } else { DIM });
        text_centered(&(i + 1).to_string(), r.x + r.w * 0.5, r.y + r.h * 0.5 + 8.0, 26.0, INK);
    }
    match m.phase {
        MemoryPhase::Show => text_centered(label("watch"), body.x + body.w * 0.5, body.bottom() + 16.0, 18.0, DIM),
        MemoryPhase::Input => {
            text_centered(label("repeat"), body.x + body.w * 0.5, body.bottom() + 16.0, 18.0, DIM);
            let gap = 18.0;
            let x0 = body.x + body.w * 0.5 - (m.seq.len() as f32 - 1.0) * gap * 0.5;
            for k in 0..m.seq.len() {
                let filled = k < m.step;
                let x = x0 + k as f32 * gap;
                draw_circle(x, body.y + 8.0, 5.0, if filled { AMBER } else { DIM });
            }
        }
    }
}

fn draw_pipes(run: &DescentRun, p: &Pipes, body: Rect) {
    let cells = pipe_cells(body, p.n);
    for (r, row) in cells.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            draw_rectangle(cell.x, cell.y, cell.w, cell.h, Color::new(0.03, 0.03, 0.035, 1.0));
            draw_rectangle_lines(cell.x, cell.y, cell.w, cell.h, 1.0, Color::new(0.16, 0.15, 0.14, 1.0));
            let thick = (cell.w * 0.14).max(4.0);
            let (cx, cy) = (cell.x + cell.w * 0.5, cell.y + cell.h * 0.5);
            let mids = [(cx, cell.y), (cell.right(), cy), (cx, cell.bottom()), (cell.x, cy)];
            for side in p.tiles[r][c].opening() {
                let (mx, my) = mids[side];
                draw_line(cx, cy, mx, my, thick, PIPE);
            }
            draw_circle(cx, cy, thick * 0.5, PIPE);
        }
    }
    let first = cells[0][0];
    let last = cells[p.n - 1][p.n - 1];
    text_centered(label("pipe_in"), first.x - 16.0, first.y + first.h * 0.5 + 6.0, 18.0, DIM);
    text_centered(label("pipe_out"), last.right() + 18.0, last.y + last.h * 0.5 + 6.0, 18.0, DIM);
    let (row, col) = run.cursor;
    if let Some(cell) = cells.get(row).and_then(|r| r.get(col)) {
        draw_rectangle_lines(cell.x, cell.y, cell.w, cell.h, 3.0, AMBER);
    }
}

fn draw_hold(shift: &DescentShift, body: Rect) {
    let frac = (shift.held_ms() / shift.hold_target_ms().max(1.0)).clamp(0.0, 1.0) as f32;
    let door = Rect::new(body.x + body.w * 0.5 - 80.0, body.y + 24.0, 160.0, body.h - 64.0);
    draw_rectangle(door.x, door.y, door.w, door.h, Color::new(0.10, 0.11, 0.13, 1.0));
    draw_rectangle(door.x, door.y, door.w, door.h * frac, Color::new(0.30, 0.33, 0.37, 1.0));
    for i in 1..8 {
        let y = door.y + door.h * i as f32 / 8.0;
        if y <= door.y + door.h * frac {
            draw_line(door.x, y, door.right(), y, 3.0, Color::new(0.22, 0.24, 0.28, 1.0));
        }
    }
    draw_rectangle_lines(door.x, door.y, door.w, door.h, 3.0, DIM);
    text_centered(label("shut"), door.x + door.w * 0.5, door.y + door.h * 0.5 + 8.0, 26.0, INK);
    text_centered(label("hold_label"), body.x + body.w * 0.5, body.bottom() + 16.0, 18.0, DIM);
}

fn draw_controls(play: &Play, l: &Layout) {
    if matches!(play.shift.puzzle(), Puzzle::Hold) {
        button(l.hold_btn, label("hold_button"), play.shift.held_ms() > 0.0);
    }
}

fn draw_hud(play: &Play, l: &Layout, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    let title = match play.choice {
        Choice::Campaign(n) => campaign_names().get(n - 1).copied().unwrap_or("").to_owned(),
        Choice::Preset(i) => format!("{}  {}", label("custom_header"), presets().get(i).map_or("", |p| p.name)),
    };
    draw_text(&title, 24.0, 30.0, 26.0, INK);
    let shift = &play.shift;
    let total = shift.stage_count();
    let now = (shift.stages_cleared() + 1).min(total);
    text_centered(&fill("stage", &[("n", now.to_string()), ("total", total.to_string())]), sw * 0.5, 31.0, 22.0, AMBER);
    let right = format!(
        "{} {}   {}",
        label("elise"),
        shift.level(),
        fill("wrong_count", &[("n", shift.wrong_attempts().to_string())])
    );
    let w = measure_text(&right, None, 22, 1.0).width;
    draw_text(&right, sw - w - 24.0, 30.0, 22.0, INK);

    let bar = l.bar;
    draw_rectangle(bar.x, bar.y, bar.w, bar.h, Color::new(0.2, 0.2, 0.2, 1.0));
    let t = (shift.threat() / 100.0).clamp(0.0, 1.0) as f32;
    draw_rectangle(bar.x, bar.y, bar.w * t, bar.h, mix(AMBER, RED_TEXT, t));
    text_centered(&format!("{}  {}%", label("closeness"), (t * 100.0) as i32), bar.x + bar.w * 0.5, bar.y + 13.0, 16.0, INK);

    let hint = match shift.puzzle() {
        Puzzle::Wires(_) => label("hint_wires"),
        Puzzle::Memory(_) => label("hint_memory"),
        Puzzle::Pipes(_) => label("hint_pipes"),
        Puzzle::Hold => label("hint_hold"),
        Puzzle::Finished => "",
    };
    text_centered(hint, sw * 0.5, sh - 46.0, 18.0, AMBER);
    text_centered(label("hint_common"), sw * 0.5, sh - 16.0, 16.0, DIM);
}

fn draw_result(end: End, end_ms: f64, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let (title, colour, line) = match end.outcome {
        Outcome::Won => {
            let line = match end.choice {
                Choice::Campaign(_) => label("won_line").to_owned(),
                Choice::Preset(_) => fill("won_custom_line", &[("points", end.score.to_string())]),
            };
            (fill("won_title", &[("n", end.descent.to_string())]), AMBER, line)
        }
        Outcome::Caught => (label("caught_title").to_owned(), RED_TEXT, label("caught_line").to_owned()),
        Outcome::Playing => return,
    };
    text_centered(&title, sw * 0.5, sh * 0.40, 60.0, colour);
    text_centered(&line, sw * 0.5, sh * 0.40 + 48.0, 26.0, INK);
    if end_ms < ui("result_lock_ms") {
        return;
    }
    let next = match (end.outcome, end.choice) {
        (Outcome::Won, Choice::Campaign(n)) if n < campaign_names().len() => label("next"),
        (Outcome::Won, _) => label("back_to_menu"),
        _ => label("again"),
    };
    text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
    text_centered(label("esc_menu"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
}

// ---- tests -----------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const STEP_MS: f64 = 16.0;
    const MAX_STEPS: usize = 40_000;

    /// Plays the bot until the shift has an end, and returns the step count. Fails if the bound is hit.
    fn play_out(run: &mut DescentRun, calls: &CallsData) -> usize {
        for step in 0..MAX_STEPS {
            if run.is_finished() {
                return step;
            }
            let c = bot_controls(run);
            run.update(STEP_MS, &c, calls);
        }
        panic!("the shift did not reach a win or a loss within {MAX_STEPS} steps");
    }

    #[test]
    fn the_bot_reaches_a_win_on_every_campaign_descent() {
        let calls = CallsData::load();
        for n in 1..=5 {
            let mut run = DescentRun::with_choice(&calls, Choice::Campaign(n), 11 + n as i64, false);
            play_out(&mut run, &calls);
            assert!(matches!(run.end, Some(End { outcome: Outcome::Won, .. })), "descent {n}: {:?}", run.end);
        }
    }

    #[test]
    fn every_custom_night_preset_reaches_an_end_without_a_panic() {
        let calls = CallsData::load();
        assert_eq!(presets().len(), 4);
        for i in 0..presets().len() {
            let mut run = DescentRun::with_choice(&calls, Choice::Preset(i), 40 + i as i64, false);
            play_out(&mut run, &calls);
            assert!(run.end.is_some(), "preset {i} has no end");
        }
    }

    #[test]
    fn a_wrong_button_is_a_wrong_attempt_with_a_note() {
        let calls = CallsData::load();
        let mut run = DescentRun::with_choice(&calls, Choice::Campaign(1), 5, false);
        let wrong = match run.play.as_ref().map(|p| p.shift.puzzle()) {
            Some(Puzzle::Wires(w)) => (0..w.buttons).find(|&b| b != w.wire[w.lit[w.step]]).expect("a wrong button"),
            _ => panic!("descent 1 opens on the fuse boxes"),
        };
        // the car rides for a while before the first panel can be worked
        for _ in 0..200 {
            run.update(STEP_MS, &Controls::default(), &calls);
        }
        assert!(!run.play.as_ref().is_some_and(|p| p.shift.riding()), "the car has stopped");
        run.update(STEP_MS, &Controls { slot: Some(wrong), ..Controls::default() }, &calls);
        let play = run.play.as_ref().expect("still playing");
        assert_eq!(play.shift.wrong_attempts(), 1);
        assert_eq!(run.notes.last().map(|n| n.0.as_str()), Some(label("wrong_note")));
    }

    #[test]
    fn the_call_plays_on_every_campaign_descent() {
        let calls = CallsData::load();
        for n in 1..=5 {
            let mut run = DescentRun::with_choice(&calls, Choice::Campaign(n), 77, false);
            let mut opened = false;
            let mut done = false;
            for _ in 0..MAX_STEPS {
                let call = &run.play.as_ref().expect("playing").shift.call;
                opened |= call.open;
                if call.done {
                    done = true;
                    break;
                }
                run.update(STEP_MS, &Controls::default(), &calls);
            }
            assert!(opened && done, "descent {n}: the call should open and finish");
        }
    }

    #[test]
    fn the_menu_opens_a_descent_only_after_the_one_before_is_cleared() {
        let calls = CallsData::load();
        let mut run = DescentRun::new(5, false);
        let open = |run: &DescentRun| run.menu.as_ref().expect("menu").rows.iter().map(|r| r.open).collect::<Vec<bool>>();
        assert_eq!(open(&run), vec![true, false, false, false, false, false, false, false, false]);
        run.update(STEP_MS, &Controls { menu_pick: Some(1), ..Controls::default() }, &calls);
        assert!(run.menu.is_some(), "a locked descent does not start");
        run.save.record_level_clear(MODE_ID, 1);
        run.to_menu();
        assert_eq!(open(&run)[..2], [true, true]);
        run.save.record_level_clear(MODE_ID, 5);
        run.to_menu();
        assert!(open(&run)[5..].iter().all(|&o| o), "the presets open once Descent 5 is cleared");
    }

    #[test]
    fn a_win_is_saved_to_the_save_file_model() {
        let calls = CallsData::load();
        let mut run = DescentRun::with_choice(&calls, Choice::Campaign(1), 9, false);
        play_out(&mut run, &calls);
        assert!(run.save.cleared_levels(MODE_ID).contains(&1));
        assert!(run.save.level_open(MODE_ID, 2, false));
    }

    #[test]
    fn the_labels_and_presets_are_in_the_rules_file() {
        for key in LABEL_KEYS {
            assert!(!label(key).is_empty(), "label {key}");
        }
        assert_eq!(presets().iter().map(|p| p.elise).collect::<Vec<_>>(), vec![5, 10, 15, 20]);
        assert_eq!(campaign_names().len(), 5);
        assert_eq!(StageKind::Pipes.title(), "PIPEWORK");
    }

    #[test]
    fn the_ui_timings_are_in_the_rules_file() {
        for key in UI_KEYS {
            assert!(ui(key) > 0.0, "ui.{key} must be a positive number");
        }
        assert_eq!(ui("script_step_ms"), 1000.0 / 60.0);
    }

    #[test]
    fn the_pipe_route_the_bot_follows_lines_up_with_the_grid() {
        let calls = CallsData::load();
        let mut run = DescentRun::with_choice(&calls, Choice::Campaign(3), 21, false);
        for _ in 0..MAX_STEPS {
            let Some(Puzzle::Pipes(p)) = run.play.as_ref().map(|play| play.shift.puzzle()) else {
                run.update(STEP_MS, &bot_controls(&run), &calls);
                continue;
            };
            let route = pipe_route(p).expect("every grid has a route");
            assert_eq!(route[0].0, 0);
            assert_eq!(route.last().map(|t| (t.0, t.1)), Some((p.n - 1, p.n - 1)));
            return;
        }
        panic!("descent 3 never reached the pipework");
    }

    #[test]
    fn the_shot_request_reads_its_flags() {
        let args: Vec<String> = ["night_watch", "--descent-shot", "out.png", "--descent", "4", "--solve", "2"].iter().map(|s| s.to_string()).collect();
        let (path, plan) = shot_request(&args).expect("a shot");
        assert_eq!(path, "out.png");
        assert_eq!((plan.descent, plan.frames, plan.solve), (4, 300, 2));
        assert!(shot_request(&["night_watch".to_string()]).is_none());
    }
}
