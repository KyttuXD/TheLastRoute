//! Open Air, playable (Godot `game/night_watch/openair`, mode id `extra`). The driver for the rules in
//! `modes/open_air.rs`: the watch select, the keys and the mouse, the picture of each watch, the two-floor map with the
//! hunters on it, the call box, the endings and the Custom Night chain. Every number comes from the mode's data
//! (`data/rules|cast|maps/open_air.json`, `calls/extra.json`); the rules state is read through `OpenAir::seen`.
//! Pattern: `classic_play.rs`.

use macroquad::prelude::*;

use crate::building_map::{self, MapView};
use crate::calls_view::CallView;
use crate::data::{self, CallsData};
use crate::framework::CallScript;
use crate::modes::open_air::{
    self as rules, Action, Cause, Event, Glimpse, Hunter, Input, Levels, OpenAir, Outcome, SceneSeen, Seen, Side, Watch,
    WeatherKind,
};
use crate::save::NightWatchSave;
use crate::sprites::{self, Facing};

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.78);

const MODE: &str = rules::MODE_ID;
/// The data registry key (`data/*/open_air.json`, `data/maps/open_air.json`); the hub id above keys the saves and the calls.
const DATA: &str = "open_air";
const CAN_KEYS: [KeyCode; 3] = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3];

/// A UI timing from the rules file's `ui` block (ms). A missing key is a content bug and panics, like the other data.
fn ui_ms(key: &str) -> f64 {
    data::num(data::rules(DATA), &format!("ui.{key}"))
}

/// The length of one scripted step (the 60 fps frame time), from `ui.script_fps`.
fn script_step_ms() -> f64 {
    1000.0 / data::num(data::rules(DATA), "ui.script_fps")
}

/// A label from the rules file's `ui.labels` block.
fn label(key: &str) -> &'static str {
    data::text(data::rules(DATA), &format!("ui.labels.{key}"))
}

/// A label with `{name}` placeholders filled from `pairs`.
fn filled(key: &str, pairs: &[(&str, &str)]) -> String {
    pairs.iter().fold(label(key).to_owned(), |text, (name, value)| text.replace(&format!("{{{name}}}"), value))
}

/// Where the shell goes after a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Stay,
    Hub,
}

/// What the player holds or presses for the watch on screen. Torch, bars, relight, gather and tent are held; the
/// lamp, a canister and a log are presses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    TorchL,
    TorchR,
    BarL,
    BarR,
    Relight,
    Gather,
    Tent,
    Lamp,
    Can(u8),
    Log,
}

impl Control {
    fn label(self) -> String {
        match self {
            Control::TorchL => label("ctl_torch_l").to_owned(),
            Control::TorchR => label("ctl_torch_r").to_owned(),
            Control::BarL => label("ctl_bar_l").to_owned(),
            Control::BarR => label("ctl_bar_r").to_owned(),
            Control::Relight => label("ctl_relight").to_owned(),
            Control::Gather => label("ctl_gather").to_owned(),
            Control::Tent => label("ctl_tent").to_owned(),
            Control::Lamp => label("ctl_lamp").to_owned(),
            Control::Can(i) => filled("ctl_can", &[("n", &(i + 1).to_string())]),
            Control::Log => label("ctl_log").to_owned(),
        }
    }

    fn key(self) -> KeyCode {
        match self {
            Control::TorchL => KeyCode::Q,
            Control::TorchR => KeyCode::E,
            Control::BarL => KeyCode::A,
            Control::BarR => KeyCode::D,
            Control::Relight => KeyCode::R,
            Control::Gather => KeyCode::G,
            Control::Tent => KeyCode::T,
            Control::Lamp => KeyCode::H,
            Control::Can(i) => CAN_KEYS[usize::from(i).min(CAN_KEYS.len() - 1)],
            Control::Log => KeyCode::Space,
        }
    }

    fn is_hold(self) -> bool {
        matches!(
            self,
            Control::TorchL | Control::TorchR | Control::BarL | Control::BarR | Control::Relight | Control::Gather | Control::Tent
        )
    }
}

/// The controls a watch uses, in button order. Canisters are the ones the data allows (`bus_stop.max_cans`).
fn controls_for(watch: Watch) -> Vec<Control> {
    match watch {
        Watch::Rooftop => vec![Control::TorchL, Control::TorchR, Control::BarL, Control::BarR, Control::Relight],
        Watch::BusStop => {
            let cans = data::int(data::rules(DATA), "bus_stop.max_cans") as usize;
            let mut out = vec![Control::Lamp];
            out.extend((0..cans.min(CAN_KEYS.len())).map(|i| Control::Can(i as u8)));
            out
        }
        Watch::Campfire => vec![Control::TorchL, Control::TorchR, Control::Gather, Control::Tent, Control::Log],
    }
}

/// One frame of input: what is held, what was pressed, and the menu and call keys.
#[derive(Clone, Debug, Default)]
pub struct Controls {
    pub hold: Vec<Control>,
    pub press: Vec<Control>,
    pub pick: Option<Watch>,
    pub custom: bool,
    pub confirm: bool,
    pub back: bool,
    pub skip_call: bool,
    pub mute_call: bool,
}

fn input_of(c: &Controls) -> Input {
    let held = |k: Control| c.hold.contains(&k);
    Input {
        torch_left: held(Control::TorchL),
        torch_right: held(Control::TorchR),
        bar_left: held(Control::BarL),
        bar_right: held(Control::BarR),
        relight: held(Control::Relight),
        gather: held(Control::Gather),
        tent: held(Control::Tent),
    }
}

/// A run of one watch, or of the Custom Night chain: the rules state, the call and the notes on screen.
struct Night {
    state: OpenAir,
    chain: Vec<Watch>,
    at: usize,
    custom: bool,
    levels: Levels,
    seed: i64,
    available: Vec<Control>,
    held: Vec<Control>,
    call: CallScript,
    view: CallView,
    notes: Vec<(String, f64)>,
    flash_ms: f64,
}

#[derive(Clone, Copy, Debug)]
enum Screen {
    Select,
    Playing,
    Ended(End),
}

#[derive(Clone, Copy, Debug)]
struct End {
    outcome: Outcome,
    custom: bool,
}

/// The Open Air screens: the watch select, a watch being played, and the ending of one.
pub struct OpenAirRun {
    screen: Screen,
    night: Option<Night>,
    persist: bool,
    seed: i64,
    end_ms: f64,
    anim_ms: f64,
}

/// The data's stalker start distance, which sets how far a hunter has to walk to the door, the fire or the bus.
fn start_dist() -> f32 {
    data::int(data::rules(DATA), "stalker.start_dist") as f32
}

fn open_watch(levels: Levels, seed: i64, at: usize, watch: Watch) -> OpenAir {
    OpenAir::new(watch, seed.wrapping_add(at as i64), levels, false)
}

impl OpenAirRun {
    /// The watch select. `persist` lets a won watch write the save file (off for the tests and screenshots).
    pub fn new(seed: i64, persist: bool) -> Self {
        Self { screen: Screen::Select, night: None, persist, seed, end_ms: 0.0, anim_ms: 0.0 }
    }

    /// Starts the chain: one watch, or the Custom Night chain (no calls).
    fn begin(&mut self, calls: &CallsData, chain: Vec<Watch>, custom: bool) {
        let levels = Levels::listed();
        let first = chain[0];
        let mut view = CallView::new();
        view.player.enabled = self.persist;
        self.night = Some(Night {
            state: open_watch(levels, self.seed, 0, first),
            chain,
            at: 0,
            custom,
            levels,
            seed: self.seed,
            available: controls_for(first),
            held: Vec::new(),
            call: rules::call_script(calls, first, custom),
            view,
            notes: Vec::new(),
            flash_ms: 0.0,
        });
        self.screen = Screen::Playing;
        self.end_ms = 0.0;
    }

    /// Advances the run by `dt_ms` with this frame's controls. Returns Hub when the player leaves.
    pub fn update(&mut self, calls: &CallsData, dt_ms: f64, c: &Controls) -> Flow {
        self.anim_ms += dt_ms;
        if c.back {
            return Flow::Hub;
        }
        match self.screen {
            Screen::Select => {
                if let Some(watch) = c.pick {
                    self.begin(calls, vec![watch], false);
                } else if c.custom {
                    self.begin(calls, rules::custom_watches(Levels::listed()), true);
                }
            }
            Screen::Playing => {
                if let Some(outcome) = self.play(calls, dt_ms, c) {
                    self.screen = Screen::Ended(End { outcome, custom: self.custom() });
                    self.end_ms = 0.0;
                }
            }
            Screen::Ended(end) => {
                self.end_ms += dt_ms;
                if c.confirm && self.end_ms >= ui_ms("result_lock_ms") {
                    match (end.outcome, self.night.as_ref().map(|n| (n.chain.clone(), n.custom))) {
                        (Outcome::Caught { .. }, Some((chain, custom))) => self.begin(calls, chain, custom),
                        _ => {
                            self.screen = Screen::Select;
                            self.night = None;
                        }
                    }
                }
            }
        }
        Flow::Stay
    }

    fn custom(&self) -> bool {
        self.night.as_ref().is_some_and(|n| n.custom)
    }

    /// One step of the playing watch. Returns the outcome of the whole run once it is over.
    fn play(&mut self, calls: &CallsData, dt_ms: f64, c: &Controls) -> Option<Outcome> {
        let persist = self.persist;
        let night = self.night.as_mut()?;
        night.flash_ms = (night.flash_ms - dt_ms).max(0.0);
        for note in night.notes.iter_mut() {
            note.1 -= dt_ms;
        }
        night.notes.retain(|note| note.1 > 0.0);
        if c.skip_call {
            night.call.skip();
        }
        if c.mute_call && night.call.mute() {
            note(night, label("call_muted"));
        }
        for control in &c.press {
            match *control {
                Control::Lamp => {
                    night.state.act(Action::Lamp);
                }
                Control::Can(i) => {
                    night.state.act(Action::Can(i));
                }
                Control::Log => {
                    night.state.act(Action::Log);
                }
                _ => {}
            }
        }
        night.held = c.hold.clone();

        let step_ms = dt_ms.min(ui_ms("max_step_ms"));
        let events = night.state.step(step_ms, input_of(c));
        night.call.update(night.state.elapsed_ms(), step_ms);
        night.view.update(&night.call);
        let watch = night.state.watch();
        for event in events {
            if let Some(text) = note_for(event, watch) {
                note(night, &text);
            }
            if let Event::Caught { .. } = event {
                night.flash_ms = ui_ms("flash_ms");
            }
        }
        match night.state.outcome() {
            Outcome::Playing => None,
            Outcome::Won if night.at + 1 < night.chain.len() => {
                let done = watch.label();
                night.at += 1;
                let next = night.chain[night.at];
                note(night, &filled("dawn", &[("done", done), ("next", next.label())]));
                night.state = open_watch(night.levels, night.seed, night.at, next);
                night.available = controls_for(next);
                night.call = rules::call_script(calls, next, night.custom);
                None
            }
            outcome @ Outcome::Won => {
                save_win(night, persist);
                Some(outcome)
            }
            outcome @ Outcome::Caught { .. } => Some(outcome),
        }
    }
}

fn note(night: &mut Night, text: &str) {
    night.notes.push((text.to_owned(), ui_ms("note_ms")));
    if night.notes.len() > 3 {
        night.notes.remove(0);
    }
}

/// Writes the cleared watch (or the Custom Night score) to the progress file (best effort).
fn save_win(night: &mut Night, persist: bool) {
    if !persist {
        return;
    }
    let mut save = NightWatchSave::load();
    if night.custom {
        save.record_best(MODE, rules::custom_score(night.levels) as u32);
    } else {
        save.record_level_clear(MODE, night.state.watch().level() as u32);
    }
    if save.save().is_err() {
        note(night, label("save_failed"));
    }
}

/// The line the notes show for one event of the watch (None for the events the ending already says).
fn note_for(event: Event, watch: Watch) -> Option<String> {
    Some(match event {
        Event::Close { hunter, .. } => filled("note_close", &[("hunter", hunter.name())]),
        Event::Repelled { hunter, .. } => filled("note_repelled", &[("hunter", hunter.name())]),
        Event::Gust => label("note_gust").to_owned(),
        Event::Relit => label("note_relit").to_owned(),
        Event::Dry => match watch {
            Watch::Rooftop => label("note_dry_rooftop").to_owned(),
            _ => label("note_dry_fire").to_owned(),
        },
        Event::Weather(WeatherKind::Snow) => label("note_snow").to_owned(),
        Event::Weather(WeatherKind::Rain) => label("note_rain").to_owned(),
        Event::WeatherEnd(_) => label("note_weather_end").to_owned(),
        Event::Bus => label("note_bus").to_owned(),
        Event::Can => label("note_can").to_owned(),
        Event::Gathered => label("note_gathered").to_owned(),
        Event::TentIn => label("note_tent_in").to_owned(),
        Event::TentOut => label("note_tent_out").to_owned(),
        Event::Creep => label("note_creep").to_owned(),
        Event::Stolen => label("note_stolen").to_owned(),
        Event::Backoff { .. } => label("note_backoff").to_owned(),
        Event::Caught { .. } | Event::Win => return None,
    })
}

// ---- layout ----------------------------------------------------------------------------------------------------

/// Every drawn and clickable region for a window of `sw` by `sh` with `buttons` control buttons. Pure.
struct Layout {
    call: Rect,
    scene: Rect,
    map: Rect,
    buttons: Vec<Rect>,
    picks: [Rect; 4],
}

fn layout(sw: f32, sh: f32, buttons: usize) -> Layout {
    let pad = 24.0;
    let call_w = (sw * 0.56).clamp(420.0, 760.0);
    let call = Rect::new((sw - call_w) * 0.5, 48.0, call_w, 132.0);
    let top = call.y + call.h + 8.0;
    let bottom = sh * 0.74;
    let scene = Rect::new(pad, top, sw * 0.60 - pad, bottom - top);
    let map_x = scene.x + scene.w + pad;
    let map = Rect::new(map_x, top, sw - map_x - pad, bottom - top);
    let (btn_w, gap) = (150.0, 12.0);
    let n = buttons.max(1) as f32;
    let x0 = (sw - (n * btn_w + (n - 1.0) * gap)) * 0.5;
    let buttons = (0..buttons).map(|i| Rect::new(x0 + i as f32 * (btn_w + gap), sh * 0.80, btn_w, 44.0)).collect();
    let pick_w = 360.0;
    let picks = std::array::from_fn(|i| Rect::new((sw - pick_w) * 0.5, sh * 0.34 + i as f32 * 72.0, pick_w, 56.0));
    Layout { call, scene, map, buttons, picks }
}

fn current_layout(buttons: usize) -> Layout {
    layout(screen_width(), screen_height(), buttons)
}

// ---- input reading ---------------------------------------------------------------------------------------------

/// Reads this frame's keyboard and mouse for the screen the run is on.
pub fn poll_controls(run: &OpenAirRun) -> Controls {
    let mouse = Vec2::from(mouse_position());
    let click = is_mouse_button_pressed(MouseButton::Left);
    let held = is_mouse_button_down(MouseButton::Left);
    let mut c = Controls { back: is_key_pressed(KeyCode::Escape), ..Controls::default() };
    match run.screen {
        Screen::Select => {
            let picks = [KeyCode::Key1, KeyCode::Key2, KeyCode::Key3];
            for (i, key) in picks.iter().enumerate() {
                if is_key_pressed(*key) {
                    c.pick = Some(Watch::from_level(i as i32 + 1));
                }
            }
            c.custom = is_key_pressed(KeyCode::C);
            if click {
                let l = current_layout(0);
                for (i, rect) in l.picks.iter().enumerate() {
                    if rect.contains(mouse) {
                        if i < 3 {
                            c.pick = Some(Watch::from_level(i as i32 + 1));
                        } else {
                            c.custom = true;
                        }
                    }
                }
            }
        }
        Screen::Ended(_) => {
            c.confirm = is_key_pressed(KeyCode::Enter) || click;
        }
        Screen::Playing => {
            if let Some(night) = &run.night {
                let l = current_layout(night.available.len());
                c.skip_call = is_key_pressed(KeyCode::Enter);
                c.mute_call = is_key_pressed(KeyCode::M);
                for (control, rect) in night.available.iter().zip(l.buttons.iter()) {
                    let key = control.key();
                    if control.is_hold() {
                        if is_key_down(key) || (held && rect.contains(mouse)) {
                            c.hold.push(*control);
                        }
                    } else if is_key_pressed(key) || (click && rect.contains(mouse)) {
                        c.press.push(*control);
                    }
                }
            }
        }
    }
    c
}

/// The hunter's room on the two-floor map, from her approach. Her `dist` counts down to the door, the fire or the bus.
fn room_of(g: &Glimpse, watch: Watch) -> &'static str {
    match (watch, g.side, g.dist) {
        (Watch::Rooftop, Some(Side::Left), 1) => "catwalk_w",
        (Watch::Rooftop, Some(Side::Right), 1) => "catwalk_e",
        (Watch::Rooftop, _, d) if d <= 0 => "roof",
        (Watch::Rooftop, _, 2) => "yard",
        (Watch::Rooftop, _, _) => "service_road",
        (Watch::Campfire, None, d) if d >= 2 => "pine_trail",
        (Watch::Campfire, None, 1) => "clearing",
        (Watch::Campfire, None, _) => "log_pile",
        (Watch::Campfire, Some(Side::Left), d) if d >= 2 => "trees_w",
        (Watch::Campfire, Some(Side::Right), d) if d >= 2 => "trees_e",
        (Watch::Campfire, _, _) => "clearing",
        (Watch::BusStop, _, _) => "bus_shelter",
    }
}

fn hunter_id(h: Hunter) -> &'static str {
    match h {
        Hunter::Verrine => "verrine",
        Hunter::Wynelle => "wynelle",
        Hunter::Ysolde => "ysolde",
        Hunter::Quenelle => "quenelle",
    }
}

fn hunter_color(h: Hunter) -> Color {
    match h {
        Hunter::Verrine => Color::new(0.85, 0.45, 0.30, 1.0),
        Hunter::Wynelle => Color::new(0.45, 0.70, 0.95, 1.0),
        Hunter::Ysolde => Color::new(0.80, 0.55, 0.95, 1.0),
        Hunter::Quenelle => Color::new(0.55, 0.90, 0.55, 1.0),
    }
}

/// The controls this picture calls for: the torch on a hunter at her side, the bar on the climber at the door, the
/// lamp and the canisters, and the fire kept fed. Only keys the watch has are ever returned.
fn scripted(seen: &Seen, available: &[Control]) -> Controls {
    let mut c = Controls::default();
    let at_side = |side: Side| seen.hunters.iter().find(|g| g.side == Some(side) && g.dist <= 1).map(|g| g.dist);
    let mut want: Vec<Control> = Vec::new();
    match &seen.scene {
        SceneSeen::Rooftop { lantern, oil, .. } => {
            for side in [Side::Left, Side::Right] {
                if let Some(dist) = at_side(side) {
                    let torch = if side == Side::Left { Control::TorchL } else { Control::TorchR };
                    let bar = if side == Side::Left { Control::BarL } else { Control::BarR };
                    want.push(if dist == 0 { bar } else { torch });
                }
            }
            if !*lantern && *oil >= 4.0 {
                want.push(Control::Relight);
            }
        }
        SceneSeen::BusStop { warm, lamp, cans, .. } => {
            if (*lamp && *warm > 90.0) || (!*lamp && *warm < 45.0) {
                c.press.push(Control::Lamp);
            }
            for &i in cans {
                c.press.push(Control::Can(i));
            }
        }
        SceneSeen::Campfire { fire, logs, hidden, .. } => {
            for side in [Side::Left, Side::Right] {
                if at_side(side).is_some() {
                    want.push(if side == Side::Left { Control::TorchL } else { Control::TorchR });
                }
            }
            if seen.hunters.iter().any(|g| g.hunter == Hunter::Quenelle && g.dist <= 1) {
                want.push(Control::Tent);
            }
            if *fire < 40.0 && *logs > 0 {
                c.press.push(Control::Log);
            }
            if !*hidden && *logs < 3 && want.is_empty() {
                want.push(Control::Gather);
            }
        }
    }
    c.hold = want.into_iter().filter(|k| available.contains(k)).collect();
    c.press.retain(|k| available.contains(k));
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
    text_centered(label, rect.x + rect.w * 0.5, rect.y + rect.h * 0.5 + 7.0, 20.0, if active { AMBER } else { INK });
}

/// A hunter silhouette standing on `base_y`, `h` tall, with glowing eyes.
fn figure(cx: f32, base_y: f32, h: f32, color: Color) {
    let body = Color::new(color.r * 0.35, color.g * 0.35, color.b * 0.35, 1.0);
    let w = h * 0.34;
    draw_rectangle(cx - w * 0.5, base_y - h * 0.62, w, h * 0.62, body);
    draw_rectangle_lines(cx - w * 0.5, base_y - h * 0.62, w, h * 0.62, 2.0, color);
    let r = h * 0.14;
    draw_circle(cx, base_y - h * 0.62 - r, r, body);
    draw_circle_lines(cx, base_y - h * 0.62 - r, r, 2.0, color);
    let eye = Color::new(1.0, 0.95, 0.7, 1.0);
    draw_circle(cx - r * 0.4, base_y - h * 0.62 - r * 1.1, r * 0.14, eye);
    draw_circle(cx + r * 0.4, base_y - h * 0.62 - r * 1.1, r * 0.14, eye);
}

/// The web build's office pictures for the two outdoor watches that have one: the picture behind the scene and the
/// foreground laid over the hunters (the bench and bin at the stop, the logs and stones at the fire). The rooftop has no
/// picture, so it keeps the drawn scene.
struct Art {
    bus_bg: Texture2D,
    bus_fg: Texture2D,
    camp_bg: Texture2D,
    camp_fg: Texture2D,
}

impl Art {
    fn load() -> Art {
        let png = |bytes: &[u8]| Texture2D::from_file_with_format(bytes, Some(ImageFormat::Png));
        Art {
            bus_bg: png(include_bytes!("../assets/offices/openair-busstop.png")),
            bus_fg: png(include_bytes!("../assets/offices/openair-busstop-fg.png")),
            camp_bg: png(include_bytes!("../assets/offices/openair-campfire.png")),
            camp_fg: png(include_bytes!("../assets/offices/openair-campfire-fg.png")),
        }
    }
}

thread_local! {
    // Leaked on purpose, like the sprites: the textures live for the whole run. Decoded on the first frame drawn.
    static ART: &'static Art = Box::leak(Box::new(Art::load()));
}

/// The (background, foreground) pictures of `watch`, or None when the watch has no picture.
fn art_of(watch: Watch) -> Option<(&'static Texture2D, &'static Texture2D)> {
    let art: &'static Art = ART.with(|a| *a);
    match watch {
        Watch::BusStop => Some((&art.bus_bg, &art.bus_fg)),
        Watch::Campfire => Some((&art.camp_bg, &art.camp_fg)),
        Watch::Rooftop => None,
    }
}

/// Draws `tex` over the whole of `r`, scaled to cover it and cropped at the edges, so the picture keeps its shape.
fn draw_cover(tex: &Texture2D, r: Rect) {
    let (tw, th) = (tex.width(), tex.height());
    let scale = (r.w / tw).max(r.h / th);
    let (cw, ch) = (r.w / scale, r.h / scale);
    let source = Rect::new((tw - cw) * 0.5, (th - ch) * 0.5, cw, ch);
    draw_texture_ex(
        tex,
        r.x,
        r.y,
        WHITE,
        DrawTextureParams { dest_size: Some(vec2(r.w, r.h)), source: Some(source), ..Default::default() },
    );
}

/// Which cut-out a hunter is drawn with. The web build shows every Open Air hunter from the front (the hunters walk
/// toward the player), so every one is `Front`; `Back` stays for a hunter seen from behind, which no Open Air state needs yet.
fn facing_of(_g: &Glimpse) -> Facing {
    Facing::Front
}

/// Draws a hunter at her spot: her sprite standing on `feet_y`, `h` tall, centred on `x`. The shape `figure` is the
/// fallback when the sprite is missing.
fn draw_hunter_at(g: &Glimpse, x: f32, feet_y: f32, h: f32) {
    let rect = Rect::new(x - h * 0.5, feet_y - h, h, h);
    let id = hunter_id(g.hunter);
    if !sprites::draw_hunter(id, facing_of(g), rect, WHITE) {
        figure(x, feet_y, h, hunter_color(g.hunter));
    }
}

/// A cheap deterministic hash for the weather.
fn hash(a: u64, b: u64) -> f32 {
    let x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    ((x ^ (x >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9) >> 40) as f32 / (1u64 << 24) as f32
}

/// Where a hunter stands in the scene: far at the edge on her side, closer as `dist` counts down.
fn approach_x(r: Rect, g: &Glimpse) -> f32 {
    let t = 1.0 - (g.dist as f32 / start_dist()).clamp(0.0, 1.0);
    match g.side {
        Some(Side::Left) => r.x + r.w * (0.08 + 0.34 * t),
        Some(Side::Right) => r.x + r.w * (0.92 - 0.34 * t),
        None => r.x + r.w * (0.50 + 0.05 * g.dist as f32),
    }
}

fn draw_scene(night: &Night, anim_ms: f64, r: Rect) {
    let seen = night.state.seen();
    let art = art_of(seen.watch);
    let ground = r.y + r.h * 0.78;
    match art {
        // the web build's picture of the stop or the fire; the scene's own state is drawn over it
        Some((bg, _)) => draw_cover(bg, r),
        None => {
            draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.03, 0.04, 0.08, 1.0));
            draw_rectangle(r.x, ground, r.w, r.h - (ground - r.y), Color::new(0.05, 0.06, 0.05, 1.0));
        }
    }
    draw_text(seen.watch.label(), r.x + 12.0, r.y + 28.0, 24.0, DIM);

    match &seen.scene {
        SceneSeen::Rooftop { lantern, oil, relight } => {
            // the roof and the lantern on its rail
            draw_rectangle(r.x, r.y + r.h * 0.46, r.w, r.h * 0.05, Color::new(0.10, 0.09, 0.08, 1.0));
            let lamp_x = r.x + r.w * 0.5;
            let lit = if *lantern { Color::new(0.95, 0.80, 0.45, 0.9) } else { Color::new(0.25, 0.25, 0.25, 1.0) };
            draw_circle(lamp_x, r.y + r.h * 0.36, 16.0, lit);
            if *lantern {
                draw_circle(lamp_x, r.y + r.h * 0.36, 60.0, Color::new(0.95, 0.80, 0.45, 0.08));
            }
            let relight_pct = relight / data::num(data::rules(DATA), "rooftop.relight_ms") * 100.0;
            let status = if *lantern {
                label("scene_lantern_lit").to_owned()
            } else {
                filled("scene_lantern_out", &[("n", &(relight_pct as i32).to_string())])
            };
            text_centered(&status, lamp_x, r.y + r.h * 0.62, 20.0, if *lantern { AMBER } else { RED_TEXT });
            text_centered(&filled("scene_oil", &[("n", &(oil.ceil() as i32).to_string())]), lamp_x, r.y + r.h * 0.68, 18.0, INK);
            text_centered(label("scene_stair"), r.x + 40.0, ground - 20.0, 18.0, DIM);
            text_centered(label("scene_hatch"), r.x + r.w - 40.0, ground - 20.0, 18.0, DIM);
        }
        SceneSeen::BusStop { warm, lamp, fuel, cans, chill } => {
            let shelter = Rect::new(r.x + r.w * 0.60, ground - r.h * 0.46, r.w * 0.30, r.h * 0.46);
            // the picture has its own shelter and lamp; the drawn box and the lamp glow only stand in without it
            let lamp_at = if art.is_some() { vec2(r.x + r.w * 0.56, r.y + r.h * 0.30) } else { vec2(shelter.x + shelter.w * 0.5, shelter.y + shelter.h * 0.35) };
            if art.is_none() {
                draw_rectangle(shelter.x, shelter.y, shelter.w, shelter.h, Color::new(0.10, 0.10, 0.12, 1.0));
                draw_rectangle_lines(shelter.x, shelter.y, shelter.w, shelter.h, 3.0, DIM);
            }
            if *lamp && *fuel > 0.0 {
                draw_circle(lamp_at.x, lamp_at.y, 46.0, Color::new(0.95, 0.60, 0.25, 0.14));
            }
            if art.is_none() {
                text_centered(label("scene_bus_stop"), shelter.x + shelter.w * 0.5, shelter.y - 10.0, 18.0, DIM);
            }
            for n in 0..cans.len() {
                draw_rectangle(r.x + r.w * 0.55 + n as f32 * 14.0, ground + 6.0, 10.0, 16.0, AMBER);
            }
            let snow = seen.weather.is_some();
            text_centered(
                &filled("scene_warm", &[("warm", &(warm.ceil() as i32).to_string()), ("chill", &(chill.ceil() as i32).to_string())]),
                r.x + r.w * 0.30,
                r.y + r.h * 0.12,
                20.0,
                if *warm < 40.0 { RED_TEXT } else { INK },
            );
            if snow {
                let tick = (anim_ms / 60.0) as u64;
                for i in 0..80u64 {
                    let x = r.x + hash(i, tick) * r.w;
                    let y = r.y + hash(tick, i) * r.h;
                    draw_circle(x, y, 1.6, Color::new(0.9, 0.95, 1.0, 0.6));
                }
            }
        }
        SceneSeen::Campfire { fire, logs, gather, hidden } => {
            let fx = r.x + r.w * 0.5;
            if art.is_none() {
                for tree in [0.06, 0.92] {
                    draw_triangle(
                        Vec2::new(r.x + r.w * tree, ground),
                        Vec2::new(r.x + r.w * (tree + 0.05), r.y + r.h * 0.20),
                        Vec2::new(r.x + r.w * (tree + 0.10), ground),
                        Color::new(0.02, 0.05, 0.03, 1.0),
                    );
                }
            }
            let flick = ((anim_ms / 90.0).sin() * 3.0) as f32;
            let size = 10.0 + (*fire as f32) / 100.0 * 26.0 + flick;
            draw_circle(fx, ground - 4.0, size, Color::new(0.95, 0.45, 0.15, 0.85));
            draw_circle(fx, ground - 6.0, size * 0.5, Color::new(1.0, 0.85, 0.35, 0.9));
            for i in 0..(*logs as usize).min(8) {
                draw_rectangle(fx - 60.0 + i as f32 * 14.0, ground + 4.0, 12.0, 6.0, Color::new(0.35, 0.22, 0.12, 1.0));
            }
            if *hidden {
                draw_rectangle(r.x + r.w * 0.80, ground - 40.0, 70.0, 40.0, Color::new(0.20, 0.30, 0.20, 1.0));
                text_centered(label("scene_tent"), r.x + r.w * 0.80 + 35.0, ground - 14.0, 16.0, INK);
            }
            if *gather > 0.0 {
                text_centered(label("scene_gathering"), fx, ground + 40.0, 18.0, DIM);
            }
            let rain = seen.weather.is_some();
            if rain {
                let tick = (anim_ms / 40.0) as u64;
                for i in 0..70u64 {
                    let x = r.x + hash(i, 3) * r.w;
                    let y = r.y + ((hash(i, tick) * r.h) % r.h);
                    draw_line(x, y, x - 4.0, y + 10.0, 1.0, Color::new(0.6, 0.7, 0.9, 0.35));
                }
            }
        }
    }

    // the torch, lit on the side it holds
    if let Some(side) = seen.torch_side {
        let x = if side == Side::Left { r.x } else { r.x + r.w * 0.72 };
        draw_rectangle(x, r.y, r.w * 0.28, r.h, Color::new(0.95, 0.80, 0.45, 0.07));
    }
    // the picture's foreground goes over the scene and under the hunters, who stand in front of it
    if let Some((_, fg)) = art {
        draw_cover(fg, r);
    }
    for g in &seen.hunters {
        let x = approach_x(r, g);
        draw_hunter_at(g, x, ground + r.h * 0.06, r.h * 0.6);
        text_centered(g.hunter.name(), x, ground + r.h * 0.06 + 22.0, 16.0, INK);
    }
    draw_rectangle_lines(r.x, r.y, r.w, r.h, 3.0, Color::new(0.30, 0.55, 0.45, 1.0));
}

fn markers_of(seen: &Seen) -> Vec<(&'static str, &'static str)> {
    seen.hunters.iter().map(|g| (room_of(g, seen.watch), hunter_id(g.hunter))).collect()
}

fn draw_map(night: &Night, area: Rect) {
    let seen = night.state.seen();
    let markers = markers_of(&seen);
    let map = building_map::load(DATA).expect("the open air building map is embedded");
    let view = MapView { active_camera: None, hunters: &markers, floor: None, legend: true };
    map.draw(area, &view);
}

fn draw_controls(night: &Night, l: &Layout) {
    for (control, rect) in night.available.iter().zip(l.buttons.iter()) {
        let active = night.held.contains(control);
        button(*rect, &control.label(), active);
    }
}

fn draw_hud(night: &Night, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, 44.0, Color::new(0.0, 0.0, 0.0, 0.6));
    let title = if night.custom {
        filled("custom_title", &[("at", &(night.at + 1).to_string()), ("of", &night.chain.len().to_string())])
    } else {
        label("title").to_owned()
    };
    draw_text(&title, 24.0, 30.0, 26.0, INK);
    let hours = data::int(data::rules(DATA), "hours");
    let seen = night.state.seen();
    text_centered(&filled("hour", &[("hour", &(seen.hour + 1).to_string()), ("hours", &hours.to_string())]), sw * 0.5, 32.0, 28.0, AMBER);
    let meter = match &seen.scene {
        SceneSeen::Rooftop { oil, .. } => {
            filled("meter_oil", &[("n", &((oil / data::num(data::rules(DATA), "meter_max") * 100.0) as i32).to_string())])
        }
        SceneSeen::BusStop { fuel, .. } => filled("meter_fuel", &[("n", &(fuel.ceil() as i32).to_string())]),
        SceneSeen::Campfire { fire, logs, .. } => {
            filled("meter_fire", &[("n", &(fire.ceil() as i32).to_string()), ("logs", &logs.to_string())])
        }
    };
    let w = measure_text(&meter, None, 26, 1.0).width;
    draw_text(&meter, sw - w - 24.0, 30.0, 26.0, INK);
    let torch = filled("torch", &[("n", &(seen.torch.ceil() as i32).to_string())]);
    draw_text(&torch, sw - w - 24.0 - 160.0, 30.0, 22.0, DIM);

    text_centered(label("hint"), sw * 0.5, sh - 14.0, 16.0, DIM);
}

fn draw_select(sw: f32, sh: f32) {
    text_centered(label("title"), sw * 0.5, sh * 0.20, 64.0, AMBER);
    text_centered(label("select_sub"), sw * 0.5, sh * 0.26, 22.0, INK);
    let l = current_layout(0);
    let labels = [label("pick_1"), label("pick_2"), label("pick_3"), label("pick_custom")];
    for (rect, text) in l.picks.iter().zip(labels.iter()) {
        button(*rect, text, false);
    }
    text_centered(label("hub"), sw * 0.5, sh * 0.34 + 4.0 * 72.0 + 10.0, 20.0, DIM);
}

fn draw_result(run: &OpenAirRun, night: &Night, end: End, sw: f32, sh: f32) {
    draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.82));
    let (title, colour, line) = match end.outcome {
        Outcome::Won if end.custom => (
            label("result_custom_won").to_owned(),
            AMBER,
            filled("result_custom_score", &[("n", &rules::custom_score(night.levels).to_string())]),
        ),
        Outcome::Won => (filled("result_won", &[("watch", night.state.watch().label())]), AMBER, label("result_won_line").to_owned()),
        Outcome::Caught { by, cause } => {
            let why = match cause {
                Cause::Edge => filled("result_edge", &[("hunter", by.name())]),
                Cause::Cold => label("result_cold").to_owned(),
                Cause::Fire => label("result_fire").to_owned(),
                Cause::Tent => label("result_tent").to_owned(),
            };
            (label("result_caught").to_owned(), RED_TEXT, why)
        }
        Outcome::Playing => return,
    };
    text_centered(&title, sw * 0.5, sh * 0.40, 60.0, colour);
    text_centered(&line, sw * 0.5, sh * 0.40 + 48.0, 26.0, INK);
    if run.end_ms >= ui_ms("result_lock_ms") {
        let next = match end.outcome {
            Outcome::Won => label("result_next_won"),
            _ => label("result_next_lost"),
        };
        text_centered(next, sw * 0.5, sh * 0.40 + 110.0, 26.0, AMBER);
        text_centered(label("result_hub"), sw * 0.5, sh * 0.40 + 146.0, 22.0, DIM);
    }
}

/// Draws the whole screen for the run's current state.
pub fn draw(run: &OpenAirRun) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let (sw, sh) = (screen_width(), screen_height());
    match (run.screen, run.night.as_ref()) {
        (Screen::Playing, Some(night)) | (Screen::Ended(_), Some(night)) => {
            let l = current_layout(night.available.len());
            draw_scene(night, run.anim_ms, l.scene);
            draw_map(night, l.map);
            draw_controls(night, &l);
            draw_hud(night, sw, sh);
            night.view.draw(&night.call, l.call);
            for (i, (text, _)) in night.notes.iter().enumerate() {
                text_centered(text, sw * 0.5, sh - 40.0 - (night.notes.len() - 1 - i) as f32 * 24.0, 22.0, AMBER);
            }
            if night.flash_ms > 0.0 {
                draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.6, 0.02, 0.02, (night.flash_ms / ui_ms("flash_ms")) as f32 * 0.55));
            }
            if let Screen::Ended(end) = run.screen {
                draw_result(run, night, end, sw, sh);
            }
        }
        _ => draw_select(sw, sh),
    }
}

// ---- scripted runs (the tests and the --open-air-screenshot flag) --------------------------------------------

impl OpenAirRun {
    /// One fixed step of the scripted player.
    fn scripted_step(&mut self, calls: &CallsData) -> Flow {
        let c = match &self.night {
            Some(night) => scripted(&night.state.seen(), &night.available),
            None => Controls::default(),
        };
        self.update(calls, script_step_ms(), &c)
    }

    fn is_over(&self) -> bool {
        matches!(self.screen, Screen::Ended(_))
    }
}

/// A run started on `watch` (or the Custom Night chain when `custom`) and stepped by the scripted player for up to
/// `frames` fixed 60 fps steps or until the run ends. Nothing is saved.
pub fn run_script(calls: &CallsData, seed: i64, watch: Option<Watch>, custom: bool, frames: u32) -> OpenAirRun {
    let mut run = OpenAirRun::new(seed, false);
    if custom {
        run.begin(calls, rules::custom_watches(Levels::listed()), true);
    } else if let Some(watch) = watch {
        run.begin(calls, vec![watch], false);
    }
    for _ in 0..frames {
        if run.is_over() {
            break;
        }
        run.scripted_step(calls);
    }
    run
}

/// Draws the scripted run's screen for a few frames (the first ones can land at a different size), saves the PNG.
pub async fn take_screenshot(run: &OpenAirRun, path: &str) {
    for frame in 0..4 {
        draw(run);
        if frame == 3 {
            get_screen_data().export_png(path);
        }
        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUND: u32 = 40_000;

    fn calls() -> CallsData {
        CallsData::load()
    }

    #[test]
    fn the_layout_fits_the_window_and_the_buttons_do_not_overlap() {
        for (sw, sh) in [(1280.0, 720.0), (960.0, 600.0), (1920.0, 1080.0)] {
            let l = layout(sw, sh, 5);
            for r in [l.call, l.scene, l.map] {
                assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.w <= sw && r.y + r.h <= sh, "{r:?} leaves the window {sw}x{sh}");
            }
            assert!(l.scene.x + l.scene.w <= l.map.x, "the scene and the map overlap");
            for pair in l.buttons.windows(2) {
                assert!(pair[0].x + pair[0].w <= pair[1].x, "two buttons overlap");
            }
            for b in &l.buttons {
                assert!(b.y >= l.scene.y + l.scene.h && b.x >= 0.0 && b.x + b.w <= sw);
            }
            for p in &l.picks {
                assert!(p.x >= 0.0 && p.x + p.w <= sw && p.y + p.h <= sh);
            }
        }
    }

    #[test]
    fn every_watch_has_distinct_keys_and_the_bus_has_its_canisters() {
        for watch in Watch::ALL {
            let keys: Vec<KeyCode> = controls_for(watch).iter().map(|c| c.key()).collect();
            for (i, key) in keys.iter().enumerate() {
                assert!(!keys[..i].contains(key), "{watch:?} binds a key twice");
            }
        }
        let bus = controls_for(Watch::BusStop);
        assert!(bus.contains(&Control::Can(0)) && bus.contains(&Control::Can(2)));
    }

    #[test]
    fn every_watch_plays_to_a_win_or_a_loss_within_the_bound() {
        for watch in Watch::ALL {
            let run = run_script(&calls(), 11 + i64::from(watch.level()), Some(watch), false, BOUND);
            assert!(run.is_over(), "{watch:?} did not end within {BOUND} steps");
        }
    }

    #[test]
    fn the_custom_night_chains_its_watches_in_order_to_an_end() {
        let data = calls();
        let chain = rules::custom_watches(Levels::listed());
        let mut run = OpenAirRun::new(5, false);
        run.begin(&data, chain.clone(), true);
        let mut visited = vec![chain[0]];
        for _ in 0..BOUND {
            if run.is_over() {
                break;
            }
            run.scripted_step(&data);
            if let Some(night) = &run.night {
                if night.at + 1 > visited.len() {
                    visited.push(night.state.watch());
                }
            }
        }
        assert!(run.is_over(), "the custom night did not end within {BOUND} steps");
        assert_eq!(chain[..visited.len()], visited[..], "the watches came in the wrong order");
    }

    #[test]
    fn the_rooftop_call_script_plays_its_lines() {
        let data = calls();
        let mut run = OpenAirRun::new(3, false);
        run.begin(&data, vec![Watch::Rooftop], false);
        // the call rings at 6 seconds and the climber needs well over 20 seconds to reach the door, so the first
        // lines are on screen by then while the watch is still running
        for _ in 0..(20_000 / 17) {
            run.scripted_step(&data);
        }
        let night = run.night.as_ref().expect("the rooftop watch is running");
        assert!(night.state.elapsed_ms() >= 6_000.0);
        assert!(night.call.line >= 0, "the call never opened");
        let entry = data.for_level(rules::MODE_ID, 1).expect("the rooftop has a call");
        let typed = night.call.full_text();
        assert!(entry.lines.iter().any(|l| l.as_str() == typed), "the typed line is not from the rooftop call: {typed:?}");
    }

    #[test]
    fn the_scripted_input_only_uses_keys_the_watch_has() {
        let data = calls();
        for watch in Watch::ALL {
            let mut run = OpenAirRun::new(9, false);
            run.begin(&data, vec![watch], false);
            for _ in 0..600 {
                let night = run.night.as_ref().expect("running");
                let c = scripted(&night.state.seen(), &night.available);
                assert!(c.hold.iter().chain(c.press.iter()).all(|k| night.available.contains(k)));
                run.scripted_step(&data);
            }
        }
    }

    #[test]
    fn the_ui_timings_and_labels_come_from_the_data() {
        assert_eq!(ui_ms("max_step_ms"), 100.0);
        assert_eq!(ui_ms("result_lock_ms"), 900.0);
        assert_eq!(ui_ms("note_ms"), 3200.0);
        assert_eq!(ui_ms("flash_ms"), 700.0);
        assert_eq!(script_step_ms(), 1000.0 / 60.0);
        for key in ["title", "select_sub", "hub", "hint", "pick_1", "pick_custom", "result_next_won", "scene_tent"] {
            assert!(!label(key).is_empty(), "ui.labels.{key} is empty");
        }
        assert_eq!(filled("custom_title", &[("at", "2"), ("of", "3")]), "OPEN AIR  CUSTOM NIGHT  2 OF 3");
        assert_eq!(filled("ctl_can", &[("n", "3")]), "3  CANISTER");
    }

    #[test]
    fn every_open_air_hunter_is_embedded_as_a_sprite() {
        // the decode needs a window, so the test checks the embedded list the decoder reads
        for hunter in [Hunter::Verrine, Hunter::Wynelle, Hunter::Ysolde, Hunter::Quenelle] {
            assert!(data::HUNTER_SPRITE_FILES.iter().any(|(id, _, _)| *id == hunter_id(hunter)), "{hunter:?} has no sprite");
        }
    }
}
