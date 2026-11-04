//! Salvage Run on screen: the shell's driver for `modes::salvage_run` (Godot `dayshift`). A week is a day shop (spend
//! the budget on five upgrades) and then the night at the rack. `SalvageRunRun` owns the week, maps the frame's
//! `Controls` (doors A/D pressed, lights Q/E held, rack keys 1 to 4 with Space held, shop keys 1 to 5, Enter) onto the
//! mode's `Input`, plays the events it returns, saves a cleared week and runs Zellia's call for the week. `draw` paints
//! the shop, or the office with the window, the doors, the rack and the HUD. Everything but `poll_controls`, `draw` and
//! `Art` is plain state, so it is tested without a window. All tunables come from `data/rules/salvage_run.json`.

use macroquad::prelude::*;

use crate::calls_view::CallView;
use crate::data::{self, num, text, CallsData};
use crate::framework::CallScript;
use crate::modes::salvage_run::{self as logic, Devices, Event, Hunter, Input, Kind, Options, Outcome, SalvageRun, Upgrade, WindowState};
use crate::save::NightWatchSave;
use crate::sprites::{self, Facing};

const MODE: &str = "salvage_run";
/// The call file's id: its scripts are keyed by the Godot folder name, not by the mode id.
const CALL_ID: &str = "dayshift";
/// The upgrade keys in `Upgrade::ALL` order, as the rules file names them.
const UPGRADE_KEYS: [&str; 5] = ["hinges", "cell", "soles", "gloves", "bench"];

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);
const SHUT_GREY: Color = Color::new(0.30, 0.33, 0.37, 1.0);
const WARM: Color = Color::new(0.80, 0.72, 0.45, 1.0);

/// What the player did this frame. Doors are presses (they toggle); lights, the work key and the mouse hold are held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub door_l: bool,
    pub door_r: bool,
    pub light_l: bool,
    pub light_r: bool,
    /// The rack slot (0 to 3) the player picks: keys 1 to 4, or a click on a slot.
    pub pick_slot: Option<usize>,
    /// Working the picked piece this frame: Space held, or the mouse held on a slot.
    pub work: bool,
    /// The shop upgrade (0 to 4) to buy or drop: keys 1 to 5, or a click on its row.
    pub shop_pick: Option<usize>,
    /// Enter, Space or a click: starts the shift in the shop, moves on from a result.
    pub confirm: bool,
    pub skip_call: bool,
    pub mute_call: bool,
    pub back: bool,
}

/// Where the shell goes after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Stay,
    Hub,
}

/// How a night ended. `Cleared` and `Finished` are a met quota (the last week is `Finished`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    Cleared,
    Finished,
    Short,
    Caught { by: Hunter, dark: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Shop,
    Night,
    Result(Ending),
}

pub struct SalvageRunRun {
    week: u32,
    carry: u32,
    seed: i64,
    persist: bool,
    phase: Phase,
    owned: Vec<Upgrade>,
    selected: usize,
    shut: [bool; 2],
    lights: [bool; 2],
    night: Option<SalvageRun>,
    call: CallScript,
    view: CallView,
    money: u32,
    quota: u32,
    shifts: u32,
    elapsed_ms: f64,
    anim_ms: f64,
    end_ms: f64,
    flash_ms: f64,
    notes: Vec<(String, f64)>,
}

fn ui_num(key: &str) -> f64 {
    num(data::rules(MODE), &format!("ui.{key}"))
}

fn weeks() -> u32 {
    num(data::rules(MODE), "weeks") as u32
}

/// A UI label from `ui.labels` in the rules file.
fn label(key: &str) -> &'static str {
    text(data::rules(MODE), &format!("ui.labels.{key}"))
}

/// A UI label with its `{name}` placeholders filled in.
fn fill(key: &str, pairs: &[(&str, String)]) -> String {
    pairs.iter().fold(label(key).to_owned(), |s, (name, value)| s.replace(&format!("{{{name}}}"), value))
}

fn hunter_key(hunter: Hunter) -> &'static str {
    match hunter {
        Hunter::Ravelle => "ravelle",
        Hunter::Ceryth => "ceryth",
        Hunter::Ysolde => "ysolde",
    }
}

fn hunter_name(hunter: Hunter) -> &'static str {
    text(data::rules(MODE), &format!("hunters.{}.name", hunter_key(hunter)))
}

fn kind_key(kind: Kind) -> &'static str {
    match kind {
        Kind::Scrap => "scrap",
        Kind::Parts => "parts",
        Kind::Prize => "prize",
    }
}

fn kind_label(kind: Kind) -> &'static str {
    text(data::rules(MODE), &format!("rack.kinds.{}.label", kind_key(kind)))
}

fn kind_color(kind: Kind) -> Color {
    match kind {
        Kind::Scrap => Color::new(0.62, 0.62, 0.60, 1.0),
        Kind::Parts => Color::new(0.45, 0.70, 0.95, 1.0),
        Kind::Prize => Color::new(0.95, 0.80, 0.30, 1.0),
    }
}

fn upgrade_blurb(index: usize) -> &'static str {
    text(data::rules(MODE), &format!("upgrades.{}.blurb", UPGRADE_KEYS[index]))
}

impl SalvageRunRun {
    /// A fresh campaign at week 1, in the shop. `persist` lets a cleared week write the save file (off for tests).
    /// Opens week `week` (1 on) straight away, as the select screen does. The shop starts with the week's budget.
    pub fn with_week(week: u32, seed: i64, persist: bool) -> Self {
        let mut run = Self::new(seed, persist);
        run.week = week.max(1);
        run
    }

    pub fn new(seed: i64, persist: bool) -> Self {
        let mut view = CallView::new();
        view.player.enabled = persist;
        Self {
            week: 1,
            carry: 0,
            seed,
            persist,
            phase: Phase::Shop,
            owned: Vec::new(),
            selected: 0,
            shut: [false; 2],
            lights: [false; 2],
            night: None,
            call: CallScript::new(Vec::new(), 0.0),
            view,
            money: 0,
            quota: logic::quota_for(1),
            shifts: 0,
            elapsed_ms: 0.0,
            anim_ms: 0.0,
            end_ms: 0.0,
            flash_ms: 0.0,
            notes: Vec::new(),
        }
    }

    pub fn in_shop(&self) -> bool {
        self.phase == Phase::Shop
    }

    pub fn ending(&self) -> Option<Ending> {
        match self.phase {
            Phase::Result(ending) => Some(ending),
            _ => None,
        }
    }

    fn budget(&self) -> u32 {
        logic::budget_for(self.week, self.carry)
    }

    fn note(&mut self, text: impl Into<String>) {
        self.notes.push((text.into(), ui_num("note_ms")));
        if self.notes.len() > 3 {
            self.notes.remove(0);
        }
    }

    /// One click or key in the shop: buys the upgrade, or drops it when it is already in the basket.
    fn shop_pick(&mut self, index: usize) {
        let Some(&upgrade) = Upgrade::ALL.get(index) else { return };
        match logic::shop_toggle(&self.owned, upgrade, self.week, self.carry) {
            Some(next) => self.owned = next,
            None => self.note(label("over_budget")),
        }
    }

    /// Opens the night for the current week with the bought upgrades, and the week's call.
    fn start_night(&mut self, calls: &CallsData) {
        self.shifts += 1;
        let seed = self.seed.wrapping_add(i64::from(self.shifts));
        let night = SalvageRun::new(self.week, seed, Options { upgrades: self.owned.clone(), ..Options::default() });
        self.quota = night.quota();
        self.night = Some(night);
        self.call = CallScript::for_level(calls, CALL_ID, self.week as i32);
        self.phase = Phase::Night;
        self.selected = 0;
        self.shut = [false; 2];
        self.lights = [false; 2];
        self.elapsed_ms = 0.0;
        self.end_ms = 0.0;
        self.notes.clear();
    }

    fn step_shop(&mut self, c: &Controls, calls: &CallsData) -> Flow {
        if c.back {
            return Flow::Hub;
        }
        if let Some(index) = c.shop_pick {
            self.shop_pick(index);
        }
        if c.confirm {
            self.start_night(calls);
        }
        Flow::Stay
    }

    fn step_night(&mut self, dt_ms: f64, c: &Controls) {
        let out = self.night.as_ref().map_or(true, SalvageRun::is_out);
        if out {
            self.shut = [false; 2];
            self.lights = [false; 2];
        } else {
            if c.door_l {
                self.shut[0] = !self.shut[0];
            }
            if c.door_r {
                self.shut[1] = !self.shut[1];
            }
            self.lights = [c.light_l, c.light_r];
        }
        if let Some(slot) = c.pick_slot {
            self.selected = slot;
        }
        self.elapsed_ms += dt_ms;
        self.call.update(self.elapsed_ms, dt_ms);
        self.view.update(&self.call);

        let input = Input {
            work: c.work,
            target: Some(self.selected),
            devices: Devices { door_l: self.shut[0], door_r: self.shut[1], light_l: self.lights[0], light_r: self.lights[1] },
        };
        let Some(night) = self.night.as_mut() else { return };
        let events = night.step(dt_ms, input);
        let outcome = night.outcome();
        let dark = night.is_out();
        let (money, quota) = (night.money(), night.quota());
        self.money = money;
        self.quota = quota;
        for event in events {
            self.play(event);
        }
        match outcome {
            Some(Outcome::Win) if self.week >= weeks() => self.win(Ending::Finished),
            Some(Outcome::Win) => self.win(Ending::Cleared),
            Some(Outcome::Short) => self.end(Ending::Short),
            Some(Outcome::Caught(by)) => self.end(Ending::Caught { by, dark }),
            None => {}
        }
    }

    fn play(&mut self, event: Event) {
        match event {
            Event::Salvage { value, .. } => self.note(fill("note_salvaged", &[("value", value.to_string())])),
            Event::Expire { .. } => self.note(label("note_expire")),
            Event::Close { id, dist: 0 } => self.note(fill("note_at_door", &[("name", hunter_name(id).to_owned())])),
            Event::Repelled { id } => self.note(fill("note_turned_away", &[("name", hunter_name(id).to_owned())])),
            Event::Warned { id } => self.note(fill("note_warned", &[("name", hunter_name(id).to_owned())])),
            Event::Knock => self.note(label("note_knock")),
            Event::Window => self.note(label("note_window")),
            Event::Gone => self.note(label("note_gone")),
            Event::Out => {
                self.note(label("note_out"));
                self.flash_ms = ui_num("flash_ms");
            }
            _ => {}
        }
    }

    fn end(&mut self, ending: Ending) {
        if let Ending::Caught { .. } = ending {
            self.flash_ms = ui_num("flash_ms");
        }
        self.phase = Phase::Result(ending);
        self.end_ms = 0.0;
    }

    /// A met quota: saves the week (best effort) and shows the result.
    fn win(&mut self, ending: Ending) {
        if self.persist {
            let mut save = NightWatchSave::load();
            save.record_level_clear(MODE, self.week);
            save.record_best(MODE, self.money);
            save.record_night_best(MODE, self.week, self.money);
            if save.save().is_err() {
                self.note(label("note_save_failed"));
            }
        }
        self.end(ending);
    }

    fn step_result(&mut self, dt_ms: f64, c: &Controls) -> Flow {
        self.end_ms += dt_ms;
        if c.back {
            return Flow::Hub;
        }
        if !(c.confirm && self.end_ms >= ui_num("result_lock_ms")) {
            return Flow::Stay;
        }
        match self.ending() {
            Some(Ending::Cleared) => {
                self.carry = logic::carry_from(self.money, self.quota);
                self.week += 1;
                self.owned.clear();
                self.quota = logic::quota_for(self.week);
                self.phase = Phase::Shop;
                Flow::Stay
            }
            Some(Ending::Finished) => Flow::Hub,
            // a short or a caught week is tried again: same week, same carry, the shop again
            _ => {
                self.phase = Phase::Shop;
                Flow::Stay
            }
        }
    }

    /// Advances the screen by `dt_ms` with the frame's controls. `calls` is read only when a shift starts.
    pub fn update(&mut self, dt_ms: f64, c: &Controls, calls: &CallsData) -> Flow {
        self.anim_ms += dt_ms;
        self.flash_ms = (self.flash_ms - dt_ms).max(0.0);
        for note in self.notes.iter_mut() {
            note.1 -= dt_ms;
        }
        self.notes.retain(|note| note.1 > 0.0);
        match self.phase {
            Phase::Shop => self.step_shop(c, calls),
            Phase::Night => {
                if c.back {
                    return Flow::Hub;
                }
                if c.skip_call {
                    self.call.skip();
                }
                if c.mute_call && self.call.mute() {
                    self.note(label("note_call_muted"));
                }
                self.step_night(dt_ms.min(ui_num("max_step_ms")), c);
                Flow::Stay
            }
            Phase::Result(_) => self.step_result(dt_ms, c),
        }
    }
}

// ---- layout ----------------------------------------------------------------------------------------------------

/// Every clickable or drawn region for a window of `sw` by `sh`. Pure, so the click tests and the paint agree.
struct Layout {
    call: Rect,
    doorway: [Rect; 2],
    window: Rect,
    slots: [Rect; 4],
    door_btn: [Rect; 2],
    light_btn: [Rect; 2],
    cards: [Rect; 5],
    start: Rect,
}

fn layout(sw: f32, sh: f32) -> Layout {
    let btn_w = (sw * 0.11).clamp(110.0, 150.0);
    let btn_y = sh * 0.80;
    let door_x = [sw * 0.02, sw - sw * 0.02 - btn_w];
    let light_x = [door_x[0] + btn_w + 10.0, door_x[1] - btn_w - 10.0];
    let door_btn = [Rect::new(door_x[0], btn_y, btn_w, 44.0), Rect::new(door_x[1], btn_y, btn_w, 44.0)];
    let light_btn = [Rect::new(light_x[0], btn_y, btn_w, 44.0), Rect::new(light_x[1], btn_y, btn_w, 44.0)];
    let call_w = (sw * 0.56).clamp(420.0, 760.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let door_w = sw * 0.18;
    let doorway = [Rect::new(sw * 0.03, sh * 0.22, door_w, sh * 0.52), Rect::new(sw - sw * 0.03 - door_w, sh * 0.22, door_w, sh * 0.52)];
    let window = Rect::new(sw * 0.36, sh * 0.26, sw * 0.28, sh * 0.22);
    let gap = 12.0;
    let slot_w = (sw * 0.40 - 3.0 * gap) / 4.0;
    let slots = std::array::from_fn(|i| Rect::new(sw * 0.30 + i as f32 * (slot_w + gap), sh * 0.60, slot_w, sh * 0.14));
    let card_w = (sw * 0.5).clamp(420.0, 640.0);
    let card_x = (sw - card_w) * 0.5;
    let cards = std::array::from_fn(|i| Rect::new(card_x, sh * 0.27 + i as f32 * 60.0, card_w, 52.0));
    let start = Rect::new((sw - 300.0) * 0.5, sh * 0.27 + 5.0 * 60.0 + 16.0, 300.0, 52.0);
    Layout { call, doorway, window, slots, door_btn, light_btn, cards, start }
}

// ---- input -----------------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse. The meaning of a key depends on the screen: the shop reads 1 to 5 and Enter,
/// the night reads doors, lights, the rack and the call keys, and a result reads Enter.
pub fn poll_controls(run: &SalvageRunRun) -> Controls {
    let l = layout(screen_width(), screen_height());
    let mouse = Vec2::from(mouse_position());
    let click = is_mouse_button_pressed(MouseButton::Left);
    let held = is_mouse_button_down(MouseButton::Left);
    let mut c = Controls {
        door_l: is_key_pressed(KeyCode::A),
        door_r: is_key_pressed(KeyCode::D),
        light_l: is_key_down(KeyCode::Q),
        light_r: is_key_down(KeyCode::E),
        work: is_key_down(KeyCode::Space),
        skip_call: is_key_pressed(KeyCode::Enter),
        mute_call: is_key_pressed(KeyCode::M),
        confirm: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space),
        back: is_key_pressed(KeyCode::Escape),
        ..Controls::default()
    };
    let digits = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3, KeyCode::Key4, KeyCode::Key5];
    for (i, key) in digits.iter().enumerate() {
        if is_key_pressed(*key) {
            c.shop_pick = Some(i);
            c.pick_slot = (i < 4).then_some(i);
        }
    }
    match run.phase {
        Phase::Shop => {
            if click {
                if let Some(i) = l.cards.iter().position(|r| r.contains(mouse)) {
                    c.shop_pick = Some(i);
                }
                c.confirm |= l.start.contains(mouse);
            }
        }
        Phase::Night => {
            c.light_l |= held && l.light_btn[0].contains(mouse);
            c.light_r |= held && l.light_btn[1].contains(mouse);
            if click {
                c.door_l |= l.door_btn[0].contains(mouse);
                c.door_r |= l.door_btn[1].contains(mouse);
            }
            for (i, r) in l.slots.iter().enumerate() {
                if r.contains(mouse) && (click || held) {
                    c.pick_slot = Some(i);
                    c.work |= held;
                }
            }
        }
        Phase::Result(_) => c.confirm |= click,
    }
    c
}

/// The scripted player used by the tests and the screenshot run. In the shop it buys hinges and starts the shift. In
/// the night it shuts a door while a visitor stands at it and opens it once she has gone, and it works the first piece
/// on the rack except while Ysolde is at the glass. On a result it confirms.
pub fn autoplay(run: &SalvageRunRun) -> Controls {
    let mut c = Controls::default();
    match run.phase {
        Phase::Shop => {
            if run.owned.is_empty() {
                c.shop_pick = Some(0);
            } else {
                c.confirm = true;
            }
        }
        Phase::Night => {
            let Some(night) = run.night.as_ref() else { return c };
            let out = night.is_out();
            let at_door = [Hunter::Ravelle, Hunter::Ceryth].map(|h| !out && night.distance(h) == 0);
            c.door_l = at_door[0] != run.shut[0] && !out;
            c.door_r = at_door[1] != run.shut[1] && !out;
            let on_rack = run.selected;
            let slot = night.items().iter().find(|it| it.slot == on_rack).or_else(|| night.items().first()).map(|it| it.slot);
            if let Some(slot) = slot {
                c.pick_slot = Some(slot);
                c.work = night.window() != WindowState::Here;
            }
        }
        Phase::Result(_) => c.confirm = true,
    }
    c
}

// ---- drawing ---------------------------------------------------------------------------------------------------

/// The office art, the picture of Salvage Run's night and the shop's backdrop. Load it once the window is open.
pub struct Art {
    back: Texture2D,
    front: Texture2D,
}

impl Art {
    pub fn load() -> Self {
        Self {
            back: Texture2D::from_file_with_format(include_bytes!("../assets/offices/dayshift.png"), Some(ImageFormat::Png)),
            front: Texture2D::from_file_with_format(include_bytes!("../assets/offices/dayshift-fg.png"), Some(ImageFormat::Png)),
        }
    }
}

fn text_centered(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_text(text, cx - w * 0.5, y, size, color);
}

fn button(rect: Rect, label: &str, active: bool) {
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, if active { AMBER } else { DIM });
    text_centered(label, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5 + 7.0, 22.0, if active { AMBER } else { INK });
}

/// A hunter silhouette standing on `base_y`, `h` tall, with glowing eyes (as Classic's). Only the fallback now.
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

fn hunter_color(hunter: Hunter) -> Color {
    match hunter {
        Hunter::Ravelle => Color::new(0.35, 0.80, 0.78, 1.0),
        Hunter::Ceryth => Color::new(0.95, 0.60, 0.40, 1.0),
        Hunter::Ysolde => Color::new(0.62, 0.50, 0.92, 1.0),
    }
}

/// A hunter standing on `base_y`, `h` tall. Drawn from her front cut-out: every hunter in this room faces the player
/// (the web build's dayshift uses the front for all three), so the back cut-out is not needed here. The shape is the
/// fallback for a hunter with no sprite.
fn hunter_figure(hunter: Hunter, cx: f32, base_y: f32, h: f32, alpha: f32) {
    let rect = Rect::new(cx - h * 0.5, base_y - h, h, h);
    if !sprites::draw_hunter(hunter_key(hunter), Facing::Front, rect, Color::new(1.0, 1.0, 1.0, alpha)) {
        figure(cx, base_y, h, hunter_color(hunter), alpha);
    }
}

fn hour_label(hour: u32) -> String {
    if hour == 0 {
        label("hour_midnight").to_owned()
    } else {
        fill("hour_format", &[("hour", hour.to_string())])
    }
}

/// The background texture stretched over the whole window.
fn full(texture: &Texture2D, sw: f32, sh: f32) {
    draw_texture_ex(texture, 0.0, 0.0, WHITE, DrawTextureParams { dest_size: Some(vec2(sw, sh)), ..Default::default() });
}

pub fn draw(run: &SalvageRunRun, art: &Art) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    let l = layout(sw, sh);
    match run.phase {
        Phase::Shop => draw_shop(run, art, &l, sw, sh),
        _ => {
            draw_night(run, art, &l, sw, sh);
            draw_controls(run, &l);
            draw_hud(run, sw, sh);
            run.view.draw(&run.call, l.call);
            for (i, (note, _)) in run.notes.iter().enumerate() {
                text_centered(note, sw * 0.5, sh - 44.0 - (run.notes.len() - 1 - i) as f32 * 24.0, 22.0, AMBER);
            }
            if run.flash_ms > 0.0 {
                let a = (run.flash_ms / ui_num("flash_ms")) as f32 * 0.55;
                draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, a));
            }
            if run.night.as_ref().is_some_and(SalvageRun::is_out) && run.ending().is_none() {
                draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.55));
                text_centered(label("power_out"), sw * 0.5, sh * 0.52, 64.0, RED_TEXT);
            }
        }
    }
    if let Some(ending) = run.ending() {
        draw_result(run, ending, sw, sh);
    }
}

fn draw_shop(run: &SalvageRunRun, art: &Art, l: &Layout, sw: f32, sh: f32) {
    full(&art.back, sw, sh);
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.66));
    text_centered(label("shop_title"), sw * 0.5, 78.0, 48.0, INK);
    text_centered(&fill("week_of", &[("week", run.week.to_string()), ("weeks", weeks().to_string())]), sw * 0.5, 108.0, 24.0, AMBER);
    let spent = logic::total_cost(&run.owned);
    text_centered(&fill("budget_line", &[("spent", spent.to_string()), ("budget", run.budget().to_string())]), sw * 0.5, 140.0, 22.0, INK);
    let visitors: Vec<&str> = logic::active_hunters(run.week).into_iter().map(hunter_name).collect();
    text_centered(
        &fill("carried_line", &[("carry", run.carry.to_string()), ("quota", logic::quota_for(run.week).to_string())]),
        sw * 0.5,
        162.0,
        20.0,
        DIM,
    );
    text_centered(&fill("visitors_line", &[("names", visitors.join(", "))]), sw * 0.5, 184.0, 20.0, DIM);
    for (i, r) in l.cards.iter().enumerate() {
        let upgrade = Upgrade::ALL[i];
        let owned = run.owned.contains(&upgrade);
        draw_rectangle(r.x, r.y, r.w, r.h, PANEL);
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, if owned { AMBER } else { DIM });
        draw_text(&format!("{}  {}", i + 1, upgrade.name()), r.x + 14.0, r.y + 22.0, 24.0, if owned { AMBER } else { INK });
        let cost = format!("{}", upgrade.cost());
        let w = measure_text(&cost, None, 24, 1.0).width;
        draw_text(&cost, r.x + r.w - w - 14.0, r.y + 22.0, 24.0, if owned { AMBER } else { INK });
        draw_text(upgrade_blurb(i), r.x + 14.0, r.y + 42.0, 16.0, DIM);
    }
    let fits = logic::can_buy(&run.owned, run.week, run.carry);
    button(l.start, label("start_shift"), fits);
    text_centered(label("shop_hint"), sw * 0.5, sh - 20.0, 18.0, DIM);
}

fn draw_night(run: &SalvageRunRun, art: &Art, l: &Layout, sw: f32, sh: f32) {
    full(&art.back, sw, sh);
    let night = run.night.as_ref();
    // the window at the back: Ysolde knocking, or at the glass
    let w = l.window;
    draw_rectangle(w.x, w.y, w.w, w.h, Color::new(0.10, 0.16, 0.20, 0.85));
    if let Some(night) = night {
        match night.window() {
            WindowState::Away => {}
            WindowState::Knock => {
                draw_rectangle_lines(w.x - 4.0, w.y - 4.0, w.w + 8.0, w.h + 8.0, 4.0, RED_TEXT);
                text_centered(label("knock"), w.x + w.w * 0.5, w.y - 10.0, 26.0, RED_TEXT);
            }
            WindowState::Here => {
                hunter_figure(Hunter::Ysolde, w.x + w.w * 0.5, w.y + w.h * 0.95, w.h * 0.85, 0.9);
                text_centered(label("window_label"), w.x + w.w * 0.5, w.y + w.h + 22.0, 20.0, INK);
            }
        }
    }
    draw_rectangle_lines(w.x, w.y, w.w, w.h, 3.0, Color::new(0.30, 0.45, 0.50, 1.0));

    // the two side doors: Ravelle's on the left (A), Ceryth's on the right (D)
    for side in 0..2 {
        let r = l.doorway[side];
        let hunter = if side == 0 { Hunter::Ravelle } else { Hunter::Ceryth };
        if run.lights[side] {
            draw_rectangle(r.x, r.y, r.w, r.h, Color::new(WARM.r, WARM.g, WARM.b, 0.30));
        }
        if let Some(night) = night {
            let dist = night.distance(hunter);
            let cx = r.x + r.w * 0.5;
            if dist == 0 && !run.shut[side] {
                hunter_figure(hunter, cx, r.y + r.h, r.h * 0.82, 0.95);
            } else if dist > 0 {
                // further off the further she is, walking in from the hall
                let toward = sw * 0.5 + (cx - sw * 0.5) * 0.2;
                let x = toward + (cx - toward) * (1.0 - dist as f32 / 4.0);
                hunter_figure(hunter, x, r.y + r.h * 0.9, r.h * 0.5, 0.35);
            }
            let wait = night.door_wait(hunter) as f32;
            draw_rectangle(r.x, r.bottom() + 6.0, r.w, 6.0, Color::new(0.2, 0.2, 0.2, 1.0));
            draw_rectangle(r.x, r.bottom() + 6.0, r.w * wait, 6.0, RED_TEXT);
        }
        if run.shut[side] {
            draw_rectangle(r.x, r.y, r.w, r.h, SHUT_GREY);
            for i in 1..8 {
                let y = r.y + r.h * i as f32 / 8.0;
                draw_line(r.x, y, r.x + r.w, y, 3.0, Color::new(0.22, 0.24, 0.28, 1.0));
            }
            text_centered(label("shut"), r.x + r.w * 0.5, r.y + r.h * 0.5, 30.0, Color::new(0.85, 0.88, 0.92, 1.0));
        }
        draw_rectangle_lines(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0, 4.0, Color::new(0.22, 0.20, 0.18, 1.0));
        let caption = if side == 0 { label("door_left") } else { label("door_right") };
        text_centered(caption, r.x + r.w * 0.5, r.y - 14.0, 20.0, DIM);
    }

    full(&art.front, sw, sh);
    draw_rack(run, l);
}

fn draw_rack(run: &SalvageRunRun, l: &Layout) {
    let Some(night) = run.night.as_ref() else { return };
    for (i, r) in l.slots.iter().enumerate() {
        draw_rectangle(r.x, r.y, r.w, r.h, PANEL);
        match night.items().iter().find(|it| it.slot == i) {
            Some(item) => {
                let colour = kind_color(item.kind);
                draw_rectangle(r.x + 6.0, r.y + 8.0, r.w - 12.0, r.h - 34.0, Color::new(colour.r, colour.g, colour.b, 0.28));
                draw_text(kind_label(item.kind), r.x + 12.0, r.y + 36.0, 24.0, colour);
                let value = format!("+{}", item.value);
                draw_text(&value, r.x + 12.0, r.y + 58.0, 20.0, INK);
                // the hold: how much is salvaged; the top rule shrinks as the piece slips off
                draw_rectangle(r.x + 6.0, r.bottom() - 14.0, (r.w - 12.0) * item.progress() as f32, 6.0, colour);
                draw_rectangle(r.x, r.y, r.w * (1.0 - item.age() as f32), 3.0, colour);
            }
            None => text_centered(label("empty"), r.x + r.w * 0.5, r.y + r.h * 0.5 + 6.0, 20.0, DIM),
        }
        let edge = if i == run.selected { AMBER } else { DIM };
        draw_rectangle_lines(r.x, r.y, r.w, r.h, 2.0, edge);
        draw_text(&format!("{}", i + 1), r.x + 6.0, r.y + 20.0, 18.0, edge);
    }
}

fn draw_controls(run: &SalvageRunRun, l: &Layout) {
    if run.ending().is_some() {
        return;
    }
    let shut = run.shut;
    let lit = run.lights;
    button(l.door_btn[0], label("btn_door_a"), shut[0]);
    button(l.door_btn[1], label("btn_door_d"), shut[1]);
    button(l.light_btn[0], label("btn_light_q"), lit[0]);
    button(l.light_btn[1], label("btn_light_e"), lit[1]);
}

fn draw_hud(run: &SalvageRunRun, sw: f32, sh: f32) {
    let Some(night) = run.night.as_ref() else { return };
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    draw_text(&fill("hud_week", &[("week", run.week.to_string())]), 24.0, 30.0, 28.0, INK);
    text_centered(&hour_label(night.hour()), sw * 0.5, 32.0, 32.0, AMBER);
    let money = fill("hud_salvage", &[("money", night.money().to_string()), ("quota", night.quota().to_string())]);
    let w = measure_text(&money, None, 28, 1.0).width;
    draw_text(&money, sw - w - 24.0, 30.0, 28.0, INK);

    // power and noise on the left, under the bar
    let power = night.power() as f32;
    let power_colour = if power < 20.0 { RED_TEXT } else { INK };
    draw_text(&fill("hud_power", &[("power", (power.ceil() as i32).to_string())]), 24.0, 66.0, 20.0, power_colour);
    draw_rectangle(24.0, 72.0, 200.0, 6.0, Color::new(0.2, 0.2, 0.2, 1.0));
    draw_rectangle(24.0, 72.0, 200.0 * (power / 100.0).clamp(0.0, 1.0), 6.0, power_colour);
    let noise = night.noise() as f32;
    let loud = num(data::rules(MODE), "noise.loud") as f32;
    let uneasy = num(data::rules(MODE), "noise.uneasy") as f32;
    let noise_max = num(data::rules(MODE), "noise.max") as f32;
    let noise_colour = if noise >= loud {
        RED_TEXT
    } else if noise >= uneasy {
        AMBER
    } else {
        INK
    };
    draw_text(&fill("hud_noise", &[("noise", (noise.ceil() as i32).to_string())]), 24.0, 100.0, 20.0, noise_colour);
    draw_rectangle(24.0, 106.0, 200.0, 6.0, Color::new(0.2, 0.2, 0.2, 1.0));
    draw_rectangle(24.0, 106.0, 200.0 * (noise / noise_max).clamp(0.0, 1.0), 6.0, noise_colour);

    text_centered(label("hud_hint"), sw * 0.5, sh - 14.0, 16.0, DIM);
}

fn draw_result(run: &SalvageRunRun, ending: Ending, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let (money, quota) = (run.money.to_string(), run.quota.to_string());
    let (title, colour, line, next) = match ending {
        Ending::Cleared => (
            fill("cleared_title", &[("week", run.week.to_string())]),
            AMBER,
            fill("cleared_line", &[("money", money.clone()), ("quota", quota.clone())]),
            label("cleared_next"),
        ),
        Ending::Finished => (
            label("finished_title").to_owned(),
            AMBER,
            fill("finished_line", &[("weeks", weeks().to_string()), ("money", money.clone())]),
            label("finished_next"),
        ),
        Ending::Short => (
            label("short_title").to_owned(),
            RED_TEXT,
            fill("short_line", &[("money", money.clone()), ("quota", quota.clone())]),
            label("try_next"),
        ),
        Ending::Caught { by, dark: true } => (
            label("dark_title").to_owned(),
            RED_TEXT,
            fill("dark_line", &[("name", hunter_name(by).to_owned())]),
            label("try_next"),
        ),
        Ending::Caught { by, dark: false } => (
            label("caught_title").to_owned(),
            RED_TEXT,
            fill("caught_line", &[("name", hunter_name(by).to_owned())]),
            label("try_next"),
        ),
    };
    text_centered(&title, sw * 0.5, sh * 0.40, 64.0, colour);
    text_centered(&line, sw * 0.5, sh * 0.40 + 48.0, 28.0, INK);
    if run.end_ms >= ui_num("result_lock_ms") {
        text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
        text_centered(label("hub_hint"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
    }
}

// ---- scripted runs (the --salvage --screenshot flags) ---------------------------------------------------------

/// What a `--salvage --screenshot` run does before the picture: the week, how many fixed frames of the autoplayer to
/// run, and whether to stop in the shop.
#[derive(Clone, Debug, Default)]
pub struct ShotPlan {
    pub week: u32,
    pub frames: u32,
    pub shop: bool,
}

/// Parses `--mode dayshift --screenshot PATH [--week N] [--frames N] [--shop]`. None for any other run.
pub fn shot_request(args: &[String]) -> Option<(String, ShotPlan)> {
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    if !matches!(value("--mode").as_deref(), Some("dayshift" | "salvage_run")) {
        return None;
    }
    let path = value("--screenshot")?;
    let plan = ShotPlan {
        week: value("--week").and_then(|v| v.parse().ok()).unwrap_or(1),
        frames: value("--frames").and_then(|v| v.parse().ok()).unwrap_or(600),
        shop: args.iter().any(|a| a == "--shop"),
    };
    Some((path, plan))
}

/// The fixed step the screenshot run advances by.
pub fn shot_step_ms() -> f64 {
    ui_num("shot_step_ms")
}

/// Starts the shift for the plan's week and runs the autoplayer for its frames at a fixed step. Nothing is saved.
pub fn run_script(calls: &CallsData, seed: i64, plan: &ShotPlan) -> SalvageRunRun {
    let mut run = SalvageRunRun::new(seed, false);
    run.week = plan.week.clamp(1, weeks());
    run.quota = logic::quota_for(run.week);
    if plan.shop {
        return run;
    }
    while run.in_shop() {
        let c = autoplay(&run);
        run.update(shot_step_ms(), &c, calls);
    }
    let mut frame = 0;
    while frame < plan.frames && run.ending().is_none() {
        let c = autoplay(&run);
        run.update(shot_step_ms(), &c, calls);
        frame += 1;
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: f64 = 16.0;

    fn calls() -> CallsData {
        CallsData::load()
    }

    #[test]
    fn a_scripted_campaign_reaches_an_ending_without_panic() {
        let calls = calls();
        let mut run = SalvageRunRun::new(7, false);
        let mut saw_night = false;
        for _ in 0..400_000 {
            let c = autoplay(&run);
            run.update(STEP, &c, &calls);
            saw_night |= !run.in_shop();
            if run.ending().is_some() {
                break;
            }
        }
        assert!(saw_night, "the shift started");
        assert!(run.ending().is_some(), "a night ends within the bound (week {})", run.week);
    }

    #[test]
    fn the_shop_keeps_the_basket_within_the_budget() {
        let calls = calls();
        let mut run = SalvageRunRun::new(3, false);
        run.update(STEP, &Controls { shop_pick: Some(3), ..Controls::default() }, &calls);
        run.update(STEP, &Controls { shop_pick: Some(2), ..Controls::default() }, &calls);
        assert_eq!(run.owned, vec![Upgrade::Gloves, Upgrade::Soles], "45 + 35 fits the 90 budget");
        run.update(STEP, &Controls { shop_pick: Some(0), ..Controls::default() }, &calls);
        assert_eq!(run.owned.len(), 2, "30 more would be 110, over the budget, so it is refused");
        run.update(STEP, &Controls { shop_pick: Some(3), ..Controls::default() }, &calls);
        assert_eq!(run.owned, vec![Upgrade::Soles], "a second click drops it again");
    }

    #[test]
    fn the_call_for_the_week_plays_to_its_end() {
        let calls = calls();
        let mut run = SalvageRunRun::new(11, false);
        run.update(STEP, &Controls { confirm: true, ..Controls::default() }, &calls);
        assert!(!run.in_shop(), "confirm starts the shift");
        let mut opened = false;
        for frame in 0..4_000 {
            let mut c = autoplay(&run);
            c.skip_call = frame % 10 == 0;
            run.update(STEP, &c, &calls);
            opened |= run.call.open;
            if run.call.done || run.ending().is_some() {
                break;
            }
        }
        assert!(opened, "the call rang and opened");
        assert!(run.call.done, "the call ran out of lines");
        assert!(run.call.line >= 2, "week 1 has three lines, the last one at index 2 (got {})", run.call.line);
    }

    #[test]
    fn a_cleared_week_carries_its_surplus_into_the_next() {
        let calls = calls();
        let mut run = SalvageRunRun::new(5, false);
        run.phase = Phase::Result(Ending::Cleared);
        run.money = 200;
        run.quota = logic::quota_for(1);
        run.update(1000.0, &Controls { confirm: true, ..Controls::default() }, &calls);
        assert_eq!(run.week, 2);
        assert_eq!(run.carry, logic::carry_from(200, 140));
        assert_eq!(run.carry, 40, "the surplus of 60 is capped at 40");
        assert!(run.in_shop());
        assert_eq!(run.budget(), 100 + 40);
    }

    #[test]
    fn a_short_week_is_tried_again_with_the_same_carry() {
        let calls = calls();
        let mut run = SalvageRunRun::new(5, false);
        run.week = 3;
        run.carry = 12;
        run.phase = Phase::Result(Ending::Short);
        run.update(1000.0, &Controls { confirm: true, ..Controls::default() }, &calls);
        assert_eq!((run.week, run.carry), (3, 12));
        assert!(run.in_shop());
    }

    #[test]
    fn a_held_piece_pays_and_the_rack_shows_it() {
        let calls = calls();
        let mut run = SalvageRunRun::new(9, false);
        run.update(STEP, &Controls { confirm: true, ..Controls::default() }, &calls);
        run.update(STEP, &Controls { pick_slot: Some(1), work: true, ..Controls::default() }, &calls);
        assert_eq!(run.selected, 1);
        assert!(run.night.as_ref().is_some_and(|n| n.items().iter().all(|it| it.progress() < 1.0)));
    }

    #[test]
    fn the_layout_fits_the_window_and_the_controls_do_not_overlap() {
        let l = layout(1280.0, 720.0);
        let mut all: Vec<Rect> = vec![l.door_btn[0], l.door_btn[1], l.light_btn[0], l.light_btn[1]];
        all.extend(l.slots);
        for (i, a) in all.iter().enumerate() {
            assert!(a.x >= 0.0 && a.right() <= 1280.0 && a.bottom() <= 720.0, "{a:?}");
            for b in &all[i + 1..] {
                assert!(a.intersect(*b).is_none(), "{a:?} overlaps {b:?}");
            }
        }
        for room in [l.doorway[0], l.doorway[1], l.window, l.slots[0], l.slots[3]] {
            assert!(l.call.intersect(room).is_none(), "the call box clears {room:?}");
        }
        assert!(l.start.bottom() <= 720.0 - 30.0 && l.cards.iter().all(|r| r.right() <= 1280.0));
    }

    #[test]
    fn the_rules_drive_the_text_and_the_timings() {
        assert_eq!(hunter_name(Hunter::Ysolde), "Ysolde");
        assert_eq!(kind_label(Kind::Prize), "Prize");
        assert_eq!(upgrade_blurb(0), text(data::rules(MODE), "upgrades.hinges.blurb"));
        assert!(ui_num("note_ms") > 0.0 && ui_num("result_lock_ms") > 0.0);
        assert_eq!(weeks(), 5);
        assert_eq!(hour_label(0), "12 AM");
        assert_eq!(hour_label(3), "3 AM");
        assert_eq!(fill("note_salvaged", &[("value", "25".to_owned())]), "Salvaged: +25.");
        assert_eq!(fill("cleared_line", &[("money", "210".to_owned()), ("quota", "140".to_owned())]), "210 salvaged against a quota of 140.");
        for key in ["shop_title", "start_shift", "knock", "hud_hint", "cleared_next", "hub_hint", "caught_title", "note_out"] {
            assert!(!label(key).is_empty(), "ui.labels.{key} is empty");
        }
    }
}
