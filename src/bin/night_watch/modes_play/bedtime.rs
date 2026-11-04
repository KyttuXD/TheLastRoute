//! A Bedtime night on screen: the shell's driver for `modes::bedtime`. `BedtimeRun` owns the night and its call box,
//! maps the frame's `Controls` (hold A for the closet door, hold D for the hall light, hold F for the bed light, TAB
//! for the floor plan) onto the mode's `BedtimeInput`, plays the events it returns and saves a cleared night. `draw`
//! paints the bedroom from the mode's own door and creep state, the buttons, the HUD, the call box and the plan of the
//! house by sound. Everything but `poll_controls` and `draw` is plain state, so it is tested without a window.

use macroquad::prelude::*;

use crate::building_map::{self, BuildingMap, MapView};
use crate::calls_view::CallView;
use crate::data::{num, rules, text, CallsData, CALL_INCOMING_TEXT};
use crate::framework::CallScript;
use crate::modes::bedtime::{self, Bedtime, BedtimeEvent, BedtimeInput, BedtimeOptions, DoorState, Hunter, Light, Outcome};
use crate::save::NightWatchSave;
use crate::sprites::{self, Facing};

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);
const LAMP: Color = Color::new(0.85, 0.72, 0.45, 1.0);

/// The three hunters in the order the mode lists them.
const HUNTERS: [Hunter; 3] = [Hunter::Loxelle, Hunter::Javelle, Hunter::Haelith];

/// The call file id in `calls/bedroom.json`, and the key the save file keeps Bedtime's cleared nights under.
const CALL_ID: &str = "bedroom";
const SAVE_KEY: &str = "bedroom";

/// The office art the web build draws behind the bedroom: the back wall and the foreground furniture (`bedroom.png`,
/// `bedroom-fg.png`). Decoded once per thread on first draw, so the tests (which never draw) do not need a window.
struct OfficeArt {
    back: Texture2D,
    front: Texture2D,
}

thread_local! {
    // Leaked on purpose, like the sprite layer: the textures live for the whole run.
    static OFFICE: &'static OfficeArt = Box::leak(Box::new(OfficeArt {
        back: Texture2D::from_file_with_format(include_bytes!("../assets/offices/bedroom.png"), Some(ImageFormat::Png)),
        front: Texture2D::from_file_with_format(include_bytes!("../assets/offices/bedroom-fg.png"), Some(ImageFormat::Png)),
    }));
}

fn office_art() -> &'static OfficeArt {
    OFFICE.with(|o| *o)
}

/// A number from the mode's `ui` block in data/rules/bedtime.json (timings, in ms).
fn ui_ms(key: &str) -> f64 {
    num(rules("bedtime"), &format!("ui.{key}"))
}

/// A label or line from the mode's `ui` block in data/rules/bedtime.json.
fn ui_text(key: &str) -> &'static str {
    text(rules("bedtime"), &format!("ui.{key}"))
}

/// `template` with `key` (such as "{n}") replaced by `value`.
fn fill(template: &str, key: &str, value: &str) -> String {
    template.replace(key, value)
}

fn hunter_name(who: Hunter) -> &'static str {
    ui_text(match who {
        Hunter::Loxelle => "names.loxelle",
        Hunter::Javelle => "names.javelle",
        Hunter::Haelith => "names.haelith",
    })
}

/// The id the rules file's `sound_rooms` table uses for a hunter.
fn hunter_key(who: Hunter) -> &'static str {
    match who {
        Hunter::Loxelle => "loxelle",
        Hunter::Javelle => "javelle",
        Hunter::Haelith => "haelith",
    }
}

fn hunter_color(who: Hunter) -> Color {
    match who {
        Hunter::Loxelle => Color::new(0.62, 0.50, 0.92, 1.0),
        Hunter::Javelle => Color::new(0.92, 0.55, 0.70, 1.0),
        Hunter::Haelith => Color::new(0.90, 0.25, 0.25, 1.0),
    }
}

/// The room in the house the hunter's sounds come from (`sound_rooms` in `data/rules/bedtime.json`).
fn sound_room(who: Hunter) -> &'static str {
    text(rules("bedtime"), &format!("sound_rooms.{}", hunter_key(who)))
}

fn caught_line(who: Hunter) -> &'static str {
    ui_text(match who {
        Hunter::Loxelle => "caught.loxelle",
        Hunter::Javelle => "caught.javelle",
        Hunter::Haelith => "caught.haelith",
    })
}

/// What the player did this frame. The hold is held (it works while A is down); the lights are held too, one at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    /// Hold the closet door shut. No light works while it is held (both hands are on the door).
    pub hold: bool,
    /// Hold the hall light (it wins over the bed light when both are down).
    pub light_hall: bool,
    /// Hold the bed light.
    pub light_bed: bool,
    /// Toggle the floor plan.
    pub plan: bool,
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
    /// Start this Bedtime night (the next one after a win, the same one to retry).
    Night(u32),
}

#[derive(Clone, Copy, Debug)]
struct End {
    outcome: Outcome,
}

pub struct BedtimeRun {
    night: Bedtime,
    number: u32,
    map: &'static BuildingMap,
    persist: bool,
    call: CallScript,
    view: CallView,
    plan_up: bool,
    end: Option<End>,
    end_ms: f64,
    flash_ms: f64,
    notes: Vec<(String, f64)>,
    anim_ms: f64,
}

#[cfg(test)]
/// The first campaign night the save file has not cleared yet; the first night once all are cleared.
pub fn first_open_night(save: &NightWatchSave) -> u32 {
    let cleared = save.cleared_levels(SAVE_KEY);
    (1..=bedtime::night_count()).find(|n| !cleared.contains(n)).unwrap_or(1)
}

impl BedtimeRun {
    /// Opens Bedtime night `night`. `persist` lets a won night write the save file (off for the tests and screenshots).
    pub fn new(calls: &CallsData, night: u32, seed: i64, persist: bool) -> Self {
        let number = night.clamp(1, bedtime::night_count());
        let mut view = CallView::new();
        view.player.enabled = persist;
        Self {
            night: Bedtime::new(BedtimeOptions::campaign(number, seed)),
            number,
            map: building_map::load("bedtime").expect("the bedtime building map is embedded"),
            persist,
            call: CallScript::for_level(calls, CALL_ID, number as i32),
            view,
            plan_up: false,
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

    /// The room a hunter's sounds come from, while she is making them. The plan shows only these.
    fn where_heard(&self, who: Hunter) -> Option<&'static str> {
        let making_sound = match who {
            Hunter::Haelith => self.night.creep_share() > 0.0,
            _ => self.night.door_state(who) != DoorState::Away,
        };
        making_sound.then(|| sound_room(who))
    }

    fn sound_markers(&self) -> Vec<(&'static str, &'static str)> {
        HUNTERS.iter().filter_map(|&who| self.where_heard(who).map(|room| (room, hunter_key(who)))).collect()
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
                return match end.outcome {
                    Outcome::Win if self.number < bedtime::night_count() => Flow::Night(self.number + 1),
                    Outcome::Win => Flow::Hub,
                    Outcome::Caught(_) => Flow::Night(self.number),
                };
            }
            return Flow::Stay;
        }

        if c.plan {
            self.plan_up = !self.plan_up;
        }
        if c.skip_call {
            self.call.skip();
        }
        if c.mute_call && self.call.mute() {
            self.note(ui_text("notes.muted"));
        }
        let lit = if c.light_hall {
            Light::Hall
        } else if c.light_bed {
            Light::Bed
        } else {
            Light::Off
        };
        let dt = dt_ms.min(ui_ms("max_step_ms"));
        let events = self.night.step(dt, BedtimeInput { hold: c.hold, light: lit });
        let elapsed = self.night.elapsed;
        self.call.update(elapsed, dt);
        self.view.update(&self.call);
        for event in events {
            self.play(event);
        }
        Flow::Stay
    }

    fn play(&mut self, event: BedtimeEvent) {
        match event {
            BedtimeEvent::Restless => self.note(ui_text("notes.restless")),
            BedtimeEvent::Breath(Hunter::Loxelle) => self.note(ui_text("notes.breath_loxelle")),
            BedtimeEvent::Breath(Hunter::Javelle) => self.note(ui_text("notes.breath_javelle")),
            BedtimeEvent::Near(who) => self.note(fill(ui_text("notes.near"), "{name}", hunter_name(who))),
            BedtimeEvent::Leave(who) => self.note(fill(ui_text("notes.leave"), "{name}", hunter_name(who))),
            BedtimeEvent::Stir { .. } => self.note(ui_text("notes.stir")),
            BedtimeEvent::Breathing(Hunter::Haelith) => self.note(ui_text("notes.exhale")),
            BedtimeEvent::Caught(who) => {
                self.end = Some(End { outcome: Outcome::Caught(who) });
                self.flash_ms = ui_ms("flash_ms");
            }
            BedtimeEvent::Win => {
                self.end = Some(End { outcome: Outcome::Win });
                self.save_clear();
            }
            _ => {}
        }
    }

    /// Opens the Custom Night: the three sliders are the hunters' levels, played on night `night` (the custom battery).
    /// Nothing is saved from it.
    pub fn new_custom(calls: &CallsData, night: u32, sliders: [i32; 3], seed: i64) -> Self {
        let mut run = Self::new(calls, night, seed, false);
        run.night = Bedtime::new(BedtimeOptions::custom(sliders, seed));
        run.call = CallScript::new(Vec::new(), 0.0);
        run
    }

    /// Writes the cleared night to the progress file (best effort; the tests and screenshot runs skip it).
    fn save_clear(&mut self) {
        if !self.persist {
            return;
        }
        let mut save = NightWatchSave::load();
        save.record_level_clear(SAVE_KEY, self.number);
        if save.save().is_err() {
            self.note(ui_text("notes.save_failed"));
        }
    }
}

// ---- layout ----------------------------------------------------------------------------------------------------

/// Every clickable or drawn region, for a window of `sw` by `sh`. Pure, so the click tests and the paint agree.
struct Layout {
    closet_btn: Rect,
    bed_btn: Rect,
    hall_btn: Rect,
    plan_btn: Rect,
    closet: Rect,
    hallway: Rect,
    bed: Rect,
    plan: Rect,
    call: Rect,
}

fn layout(sw: f32, sh: f32) -> Layout {
    let pad = sw * 0.06;
    let btn_w = (sw * 0.16).clamp(140.0, 220.0);
    let btn_y = sh * 0.80;
    let call_w = (sw * 0.56).clamp(420.0, 760.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let plan_btn = Rect::new(sw - 24.0 - 150.0, 52.0, 150.0, 36.0);
    let closet_btn = Rect::new(pad, btn_y, btn_w, 44.0);
    let bed_btn = Rect::new((sw - btn_w) * 0.5, btn_y, btn_w, 44.0);
    let hall_btn = Rect::new(sw - pad - btn_w, btn_y, btn_w, 44.0);
    let closet = Rect::new(pad, sh * 0.30, sw * 0.20, sh * 0.40);
    let hallway = Rect::new(sw - pad - sw * 0.20, sh * 0.30, sw * 0.20, sh * 0.40);
    let bed = Rect::new(sw * 0.36, sh * 0.50, sw * 0.28, sh * 0.20);
    let plan_top = call.bottom() + 10.0;
    let plan = Rect::new(pad, plan_top, sw - 2.0 * pad, btn_y - plan_top - 16.0);
    Layout { closet_btn, bed_btn, hall_btn, plan_btn, closet, hallway, bed, plan, call }
}

impl BedtimeRun {
    fn layout(&self) -> Layout {
        layout(screen_width(), screen_height())
    }
}

// ---- input -----------------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse. A, D, F and the buttons under them are held; TAB and the plan button toggle.
pub fn poll_controls(run: &BedtimeRun) -> Controls {
    let l = run.layout();
    let mouse = Vec2::from(mouse_position());
    let click = is_mouse_button_pressed(MouseButton::Left);
    let held = is_mouse_button_down(MouseButton::Left);
    let mut c = Controls {
        hold: is_key_down(KeyCode::A),
        light_hall: is_key_down(KeyCode::D),
        light_bed: is_key_down(KeyCode::F),
        plan: is_key_pressed(KeyCode::Tab),
        skip_call: is_key_pressed(KeyCode::Enter),
        mute_call: is_key_pressed(KeyCode::M),
        confirm: is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space),
        back: is_key_pressed(KeyCode::Escape),
    };
    if run.end.is_some() {
        c.confirm |= click;
        return c;
    }
    c.hold |= held && l.closet_btn.contains(mouse);
    c.light_hall |= held && l.hall_btn.contains(mouse);
    c.light_bed |= held && l.bed_btn.contains(mouse);
    c.plan |= click && l.plan_btn.contains(mouse);
    c
}

// ---- drawing ---------------------------------------------------------------------------------------------------

fn text_centered(text: &str, cx: f32, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_text(text, cx - w * 0.5, y, size, color);
}

fn button(rect: Rect, label: &str, active: bool) {
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, if active { AMBER } else { DIM });
    text_centered(label, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5 + 7.0, 22.0, if active { AMBER } else { INK });
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

/// A hunter standing on `base_y`, `h` tall, facing the player: her front cut-out from the shared sprite layer (the web
/// build shows both of these hunters from the front). The old shape is kept only as the fallback for a missing sprite.
fn hunter(who: Hunter, cx: f32, base_y: f32, h: f32, alpha: f32) {
    let rect = Rect::new(cx - h * 0.2, base_y - h, h * 0.4, h);
    if !sprites::draw_hunter(hunter_key(who), Facing::Front, rect, Color::new(1.0, 1.0, 1.0, alpha)) {
        figure(cx, base_y, h, hunter_color(who), alpha);
    }
}

/// Draws the office art stretched to cover the window, centred and cropped to keep its shape.
fn draw_office(tex: &Texture2D, tint: Color, sw: f32, sh: f32) {
    let (tw, th) = (tex.width(), tex.height());
    let scale = (sw / tw).max(sh / th);
    let (dw, dh) = (tw * scale, th * scale);
    draw_texture_ex(tex, (sw - dw) * 0.5, (sh - dh) * 0.5, tint, DrawTextureParams { dest_size: Some(vec2(dw, dh)), ..Default::default() });
}

fn meter(rect: Rect, share: f32, color: Color) {
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, Color::new(0.2, 0.2, 0.2, 1.0));
    draw_rectangle(rect.x, rect.y, rect.w * share.clamp(0.0, 1.0), rect.h, color);
}

fn hour_label(hour: u32) -> String {
    if hour == 0 {
        ui_text("labels.midnight").to_owned()
    } else {
        fill(ui_text("labels.hour"), "{h}", &hour.to_string())
    }
}

pub fn draw(run: &BedtimeRun) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    let l = run.layout();
    if run.plan_up {
        draw_plan(run, &l, sw, sh);
    } else {
        draw_room(run, &l, sw, sh);
    }
    draw_controls(run, &l);
    draw_hud(run, &l, sw, sh);
    run.view.draw(&run.call, l.call);
    for (i, (note, _)) in run.notes.iter().enumerate() {
        text_centered(note, sw * 0.5, sh - 18.0 - (run.notes.len() - 1 - i) as f32 * 24.0, 22.0, AMBER);
    }
    if run.flash_ms > 0.0 {
        let a = (run.flash_ms / ui_ms("flash_ms")) as f32 * 0.55;
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, a));
    }
    if let Some(end) = run.end {
        draw_result(run, end, sw, sh);
    }
}

/// The bedroom: the closet on the left, the hall doorway on the right, the bed in the middle. Dark unless a light is on.
fn draw_room(run: &BedtimeRun, l: &Layout, sw: f32, sh: f32) {
    let night = &run.night;
    let lit_any = night.light != Light::Off;
    // the office art, dark unless a light is on: the dark is in the tint, so the frame stays solid under the HUD and the call box
    let art = office_art();
    let shade = if lit_any { 1.0 } else { 0.42 };
    let tint = Color::new(shade, shade * 0.95, shade * 1.08, 1.0);
    draw_office(&art.back, tint, sw, sh);

    // the closet: Loxelle's breath shows as wisps at the crack and the door rattles when she is at it
    let loxelle = night.door_state(Hunter::Loxelle);
    let shake = if loxelle == DoorState::Near { (run.anim_ms as f32 / 25.0).sin() * 4.0 } else { 0.0 };
    let c = Rect::new(l.closet.x + shake, l.closet.y, l.closet.w, l.closet.h);
    if night.hold {
        draw_rectangle(c.x, c.y, c.w, c.h, Color::new(0.30, 0.33, 0.37, 1.0));
        for i in 1..8 {
            let y = c.y + c.h * i as f32 / 8.0;
            draw_line(c.x, y, c.x + c.w, y, 3.0, Color::new(0.22, 0.24, 0.28, 1.0));
        }
        text_centered(ui_text("labels.shut"), c.x + c.w * 0.5, c.y + c.h * 0.5, 30.0, Color::new(0.85, 0.88, 0.92, 1.0));
    } else {
        draw_rectangle(c.x, c.y, c.w, c.h, Color::new(0.01, 0.01, 0.015, 1.0));
        let breath = match loxelle {
            DoorState::Away => 0.0,
            DoorState::Breath => 0.18,
            DoorState::Near => 0.45,
        };
        if breath > 0.0 {
            let pulse = 0.5 + 0.5 * (run.anim_ms as f32 / 400.0).sin();
            for i in 0..3 {
                let x = c.x + c.w * (0.3 + 0.2 * i as f32);
                draw_circle(x, c.bottom() - c.h * 0.2, c.w * (0.10 + 0.04 * pulse), Color::new(0.7, 0.75, 0.9, breath * 0.5));
            }
            draw_rectangle_lines(c.x, c.y, c.w, c.h, 3.0, Color::new(0.6, 0.5, 0.9, breath + 0.2));
        }
    }
    draw_rectangle_lines(c.x - 4.0, c.y - 4.0, c.w + 8.0, c.h + 8.0, 4.0, Color::new(0.22, 0.20, 0.18, 1.0));
    text_centered(ui_text("labels.closet"), c.x + c.w * 0.5, c.y - 14.0, 20.0, DIM);
    if loxelle != DoorState::Away {
        meter(Rect::new(c.x, c.bottom() + 10.0, c.w, 8.0), night.answer_share(Hunter::Loxelle) as f32, hunter_color(Hunter::Loxelle));
    }

    // the hall doorway: the lamp washes out past it, and Javelle waits in it
    let h = l.hallway;
    draw_rectangle(h.x, h.y, h.w, h.h, Color::new(0.01, 0.01, 0.015, 1.0));
    let hall_lit = night.light == Light::Hall;
    if hall_lit {
        draw_rectangle(h.x, h.y, h.w, h.h, Color::new(LAMP.r, LAMP.g, LAMP.b, 0.30));
        draw_rectangle(h.x - h.w * 0.6, h.bottom(), h.w * 1.6, sh * 0.72 - h.bottom(), Color::new(LAMP.r, LAMP.g, LAMP.b, 0.10));
    }
    let javelle = night.door_state(Hunter::Javelle);
    if javelle != DoorState::Away {
        let alpha = if hall_lit { 0.95 } else { 0.20 };
        hunter(Hunter::Javelle, h.x + h.w * 0.5, h.bottom(), h.h * 0.8, alpha);
        if hall_lit {
            text_centered(hunter_name(Hunter::Javelle), h.x + h.w * 0.5, h.y + h.h * 0.12, 18.0, INK);
        }
        meter(Rect::new(h.x, h.bottom() + 10.0, h.w, 8.0), night.answer_share(Hunter::Javelle) as f32, hunter_color(Hunter::Javelle));
    }
    draw_rectangle_lines(h.x - 4.0, h.y - 4.0, h.w + 8.0, h.h + 8.0, 4.0, Color::new(0.22, 0.20, 0.18, 1.0));
    text_centered(ui_text("labels.hall"), h.x + h.w * 0.5, h.y - 14.0, 20.0, DIM);

    // the bed, Haelith at its foot while she creeps (she shows fully only under the bed light)
    let b = l.bed;
    draw_rectangle(b.x - 10.0, b.y - 30.0, 14.0, b.h + 30.0, Color::new(0.16, 0.10, 0.07, 1.0));
    draw_rectangle(b.x, b.y, b.w, b.h, Color::new(0.14, 0.12, 0.16, 1.0));
    draw_rectangle(b.x + 16.0, b.y + 10.0, b.w * 0.22, b.h * 0.35, Color::new(0.24, 0.23, 0.27, 1.0));
    let creep = night.creep_share() as f32;
    let bed_lit = night.light == Light::Bed;
    if bed_lit {
        draw_circle(b.x + b.w * 0.5, b.y + b.h * 0.3, sw * 0.22, Color::new(LAMP.r, LAMP.g, LAMP.b, 0.10));
        draw_circle(b.x + b.w * 0.5, b.y + b.h * 0.3, sw * 0.12, Color::new(LAMP.r, LAMP.g, LAMP.b, 0.10));
    }
    if creep > 0.0 || bed_lit {
        let x = b.right() - 16.0 + (1.0 - creep) * sw * 0.12;
        let alpha = if bed_lit { 0.95 } else { 0.10 + 0.6 * creep };
        hunter(Hunter::Haelith, x, b.bottom() + 6.0, b.h * 1.3, alpha);
    }
    if creep > 0.0 {
        meter(Rect::new(b.x, b.bottom() + 16.0, b.w, 8.0), creep, hunter_color(Hunter::Haelith));
    }
    // the office's foreground furniture sits in front of the hunters, as in the web build
    draw_office(&art.front, tint, sw, sh);
}

/// The house by sound: the building map with a marker in each room a hunter's sound comes from.
fn draw_plan(run: &BedtimeRun, l: &Layout, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.015, 0.02, 0.03, 1.0));
    let markers = run.sound_markers();
    let view = MapView { active_camera: None, hunters: &markers, floor: None, legend: true };
    run.map.draw(l.plan, &view);
}

fn draw_controls(run: &BedtimeRun, l: &Layout) {
    if run.end.is_some() {
        return;
    }
    button(l.closet_btn, ui_text("labels.closet_btn"), run.night.hold);
    button(l.bed_btn, ui_text("labels.bed_btn"), run.night.light == Light::Bed);
    button(l.hall_btn, ui_text("labels.hall_btn"), run.night.light == Light::Hall);
}

fn draw_hud(run: &BedtimeRun, l: &Layout, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    draw_text(&fill(ui_text("labels.night"), "{n}", &run.number.to_string()), 24.0, 30.0, 28.0, INK);
    text_centered(&hour_label(run.night.hour()), sw * 0.5, 32.0, 32.0, AMBER);
    let share = run.night.battery_share() as f32;
    let pct = fill(ui_text("labels.battery"), "{pct}", &((share * 100.0).ceil() as i32).to_string());
    let w = measure_text(&pct, None, 28, 1.0).width;
    let colour = if share < 0.2 { RED_TEXT } else { INK };
    draw_text(&pct, sw - w - 24.0, 30.0, 28.0, colour);
    let bar = Rect::new(sw - 24.0 - 200.0, 36.0, 200.0, 6.0);
    meter(bar, share, colour);
    button(l.plan_btn, ui_text(if run.plan_up { "labels.room_btn" } else { "labels.plan_btn" }), run.plan_up);
    if run.night.restless() {
        draw_text(ui_text("labels.restless"), 24.0, 64.0, 20.0, RED_TEXT);
    }
    text_centered(ui_text("labels.hint"), sw * 0.5, sh - 2.0 - run.notes.len() as f32 * 24.0 - 6.0, 16.0, DIM);
}

fn draw_result(run: &BedtimeRun, end: End, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let (title, colour, line) = match end.outcome {
        Outcome::Win => (fill(ui_text("labels.cleared"), "{n}", &run.number.to_string()), AMBER, ui_text("labels.won_line")),
        Outcome::Caught(who) => (ui_text("labels.caught").to_owned(), RED_TEXT, caught_line(who)),
    };
    text_centered(&title, sw * 0.5, sh * 0.40, 64.0, colour);
    text_centered(line, sw * 0.5, sh * 0.40 + 48.0, 28.0, INK);
    if let Some(score) = run.night.score() {
        let name = bedtime::rating_of(score, bedtime::custom_max_score());
        text_centered(&name, sw * 0.5, sh * 0.40 + 84.0, 26.0, AMBER);
    }
    let next = match end.outcome {
        Outcome::Win if run.number < bedtime::night_count() => ui_text("labels.next_night"),
        Outcome::Win => ui_text("labels.hub_night"),
        Outcome::Caught(_) => ui_text("labels.retry"),
    };
    if run.end_ms >= ui_ms("result_lock_ms") {
        text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
        text_centered(ui_text("labels.esc_hub"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
    }
}

// ---- scripted runs (the --screenshot flag) --------------------------------------------------------------------

/// What a `--screenshot --mode bedtime` run does before the picture: the night, how many fixed frames to run, the
/// held controls, and optionally to keep going until the first call line has this many letters showing.
#[derive(Clone, Debug, Default)]
pub struct ShotPlan {
    pub night: u32,
    pub frames: u32,
    pub hold: bool,
    pub light_hall: bool,
    pub light_bed: bool,
    pub plan: bool,
    pub call_chars: Option<usize>,
}

/// Steps a fresh night by the plan at a fixed 60 fps and returns it ready to draw. Nothing is saved.
pub fn run_script(calls: &CallsData, seed: i64, plan: &ShotPlan) -> BedtimeRun {
    let step_ms = ui_ms("script_step_ms");
    let mut run = BedtimeRun::new(calls, plan.night.max(1), seed, false);
    let held = Controls { hold: plan.hold, light_hall: plan.light_hall, light_bed: plan.light_bed, ..Controls::default() };
    let mut frame = 0u32;
    while frame < 200_000 {
        let reveal_done = plan.call_chars.map_or(true, |k| {
            let call = &run.call;
            call.open && call.line == 0 && call.text != CALL_INCOMING_TEXT && call.text.chars().count() >= k
        });
        if (frame >= plan.frames && reveal_done) || run.end.is_some() {
            break;
        }
        let controls = Controls { plan: plan.plan && frame == 0, ..held };
        run.update(step_ms, &controls);
        frame += 1;
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(night: u32) -> BedtimeRun {
        BedtimeRun::new(&CallsData::load(), night, 7, false)
    }

    /// A player who answers each hunter as she comes: the closet while Loxelle is out, the hall light for Javelle, the
    /// bed light for Haelith once she creeps.
    fn answering(run: &BedtimeRun) -> Controls {
        let n = &run.night;
        let hold = n.door_state(Hunter::Loxelle) != DoorState::Away;
        let hall = !hold && n.door_state(Hunter::Javelle) != DoorState::Away;
        let bed = !hold && !hall && n.creep_share() > 0.0;
        Controls { hold, light_hall: hall, light_bed: bed, ..Controls::default() }
    }

    /// A player who does nothing at all.
    fn idle(_: &BedtimeRun) -> Controls {
        Controls::default()
    }

    /// Plays one night to its end with `policy`, at 60 fps steps, and returns the ending (None if it never ends).
    fn play(night: u32, seed: i64, policy: fn(&BedtimeRun) -> Controls) -> (Option<End>, u32) {
        let mut r = BedtimeRun::new(&CallsData::load(), night, seed, false);
        for step in 0..60_000 {
            let c = policy(&r);
            r.update(16.0, &c);
            if let Some(end) = r.end {
                return (Some(end), step);
            }
        }
        (None, 60_000)
    }

    #[test]
    fn the_layout_fits_the_window_and_the_buttons_do_not_overlap() {
        for (sw, sh) in [(1280.0_f32, 720.0_f32), (800.0, 600.0)] {
            let l = layout(sw, sh);
            let all = [l.closet_btn, l.bed_btn, l.hall_btn, l.plan_btn];
            for (i, a) in all.iter().enumerate() {
                assert!(a.x >= 0.0 && a.right() <= sw && a.bottom() <= sh, "{a:?} in {sw}x{sh}");
                for b in &all[i + 1..] {
                    assert!(a.intersect(*b).is_none(), "{a:?} overlaps {b:?}");
                }
            }
            assert!(l.call.y >= 44.0 && l.plan_btn.y >= 44.0, "the HUD bar stays clear");
            assert!(l.plan_btn.intersect(l.call).is_none(), "the plan button clears the call box");
            assert!(l.plan.y > l.call.bottom() && l.plan.bottom() < l.closet_btn.y, "the plan sits between the call box and the buttons");
            assert!(l.closet.y >= l.call.bottom() && l.hallway.y >= l.call.bottom(), "the doors start under the call box");
        }
    }

    #[test]
    fn the_closet_hold_answers_loxelle_and_no_light_works_while_it_is_held() {
        let mut r = run(1);
        r.update(16.0, &Controls { hold: true, light_hall: true, ..Controls::default() });
        assert!(r.night.hold);
        assert_eq!(r.night.light, Light::Off, "both hands are on the door");
    }

    #[test]
    fn the_hall_light_wins_over_the_bed_light_when_both_are_held() {
        let mut r = run(1);
        r.update(16.0, &Controls { light_hall: true, light_bed: true, ..Controls::default() });
        assert_eq!(r.night.light, Light::Hall);
    }

    #[test]
    fn tab_toggles_the_plan_and_the_plan_marks_only_sounds() {
        let mut r = run(1);
        assert!(r.sound_markers().is_empty(), "nobody is making a sound at the start");
        r.update(16.0, &Controls { plan: true, ..Controls::default() });
        assert!(r.plan_up);
        r.update(16.0, &Controls { plan: true, ..Controls::default() });
        assert!(!r.plan_up);
    }

    #[test]
    fn every_night_ends_in_a_win_or_a_loss_within_the_step_bound() {
        for night in 1..=bedtime::night_count() {
            for policy in [answering as fn(&BedtimeRun) -> Controls, idle] {
                let (end, steps) = play(night, 11, policy);
                assert!(end.is_some(), "night {night} never ended");
                assert!(steps < 60_000, "night {night} took {steps} steps");
            }
        }
    }

    #[test]
    fn the_answering_player_clears_the_first_night() {
        let (end, _) = play(1, 11, answering);
        assert!(matches!(end, Some(End { outcome: Outcome::Win })), "{end:?}");
    }

    #[test]
    fn haelith_exhale_is_noted_at_the_foot_of_the_bed() {
        let mut r = run(1);
        r.play(BedtimeEvent::Breathing(Hunter::Haelith));
        assert!(r.notes.iter().any(|(text, _)| text == ui_text("notes.exhale")), "{:?}", r.notes);
    }

    #[test]
    fn doing_nothing_loses_the_night() {
        let (end, _) = play(1, 11, idle);
        assert!(matches!(end, Some(End { outcome: Outcome::Caught(_) })), "{end:?}");
    }

    #[test]
    fn the_call_script_rings_then_types_its_lines() {
        let mut r = run(1);
        let mut opened_at = None;
        let mut elapsed = 0.0;
        for _ in 0..(60_000 / 16) {
            r.update(16.0, &answering(&r));
            elapsed += 16.0;
            if opened_at.is_none() && r.call.open {
                opened_at = Some(elapsed);
            }
            if r.call.done || r.end.is_some() {
                break;
            }
        }
        let at = opened_at.expect("the call rings");
        assert!(at >= 6000.0, "night 1 rings at 6 seconds, opened at {at} ms");
        assert!(r.call.done, "all of night 1's call lines were typed");
        assert!(r.call.text.chars().count() > 0);
    }

    #[test]
    fn the_first_open_night_is_the_first_one_not_yet_cleared() {
        let save = NightWatchSave::from_json("{}");
        assert_eq!(first_open_night(&save), 1);
    }

    #[test]
    fn the_ui_block_carries_every_timing_and_label_the_driver_shows() {
        assert_eq!(ui_ms("max_step_ms"), 100.0);
        assert_eq!(ui_ms("result_lock_ms"), 900.0);
        assert_eq!(ui_ms("note_ms"), 3200.0);
        assert_eq!(ui_ms("flash_ms"), 700.0);
        assert_eq!(ui_ms("script_step_ms"), 1000.0 / 60.0);
        assert_eq!(hunter_name(Hunter::Javelle), "JAVELLE");
        assert_eq!(caught_line(Hunter::Haelith), "Haelith reached the bed while the room was dark.");
        assert_eq!(hour_label(0), "12 AM");
        assert_eq!(hour_label(3), "3 AM");
        assert_eq!(fill(ui_text("labels.night"), "{n}", "4"), "NIGHT 4");
        assert_eq!(fill(ui_text("labels.cleared"), "{n}", "2"), "NIGHT 2 CLEARED");
        assert_eq!(fill(ui_text("notes.near"), "{name}", "LOXELLE"), "LOXELLE is at the door!");
        assert!(ui_text("labels.hint").ends_with("ESC HUB"));
    }

    #[test]
    fn both_hunters_on_the_bed_and_in_the_hall_have_a_sprite_to_draw() {
        for who in [Hunter::Javelle, Hunter::Haelith] {
            assert!(crate::data::HUNTER_SPRITE_FILES.iter().any(|(id, _, _)| *id == hunter_key(who)), "{} has no sprite", hunter_key(who));
        }
    }
}
