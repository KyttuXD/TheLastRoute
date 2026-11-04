//! Night Select: the nights of one night type, and its Custom Night (Godot `night_watch_menu.gd`, `_build_custom`).
//! The night count comes from each mode's rules JSON, the cleared, locked and best state from the save, and the Custom
//! Night sliders and their max from `data/select.json`. Every label and number is in that file; this file holds the
//! logic (`Select::handle`) and the draw. `main.rs` starts the night a `SelectAction` names.

use macroquad::prelude::*;
use serde_json::Value;
use std::sync::OnceLock;

use crate::data;
use crate::save::{NightWatchSave, CLASSIC};

const SELECT_JSON: &str = include_str!("data/select.json");

const INK: Color = Color::new(0.910, 0.886, 0.816, 1.0);
const DIM: Color = Color::new(0.608, 0.580, 0.498, 1.0);
const RED_TEXT: Color = Color::new(0.878, 0.314, 0.290, 1.0);
const AMBER: Color = Color::new(0.878, 0.690, 0.290, 1.0);
const PANEL: Color = Color::new(0.024, 0.020, 0.016, 0.72);
const BAR: Color = Color::new(0.300, 0.260, 0.200, 1.0);

/// One Custom Night slider: the hunter's id in the rules and the label on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slider {
    pub id: &'static str,
    pub label: &'static str,
}

/// A mode's Custom Night: its sliders, the night the driver plays with, the night that unlocks it, the top of each
/// slider and the points per level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomSpec {
    pub night: u32,
    pub unlock_night: u32,
    pub max: u32,
    pub points_per_level: u32,
    pub sliders: Vec<Slider>,
}

/// One night type on the select screen. `nights` is the count read from the rules JSON; `names` holds per-night labels
/// when the rules name them (Descent's levels, Mixed Shift's shifts).
#[derive(Clone, Debug)]
pub struct ModeSpec {
    pub id: &'static str,
    pub title: &'static str,
    pub save_key: &'static str,
    pub nights: u32,
    pub night_label: &'static str,
    pub names: Option<Vec<&'static str>>,
    pub custom: Option<CustomSpec>,
}

/// One night on the list: its number, its label, whether it can be started, and what the save holds for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NightRow {
    pub number: u32,
    pub label: String,
    pub open: bool,
    pub cleared: bool,
    pub best: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Nights,
    Custom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectInput {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    /// The C key: jump to the Custom Night row.
    CustomKey,
    Pick(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectAction {
    None,
    Back,
    /// Start night `night` (1-based) of the mode whose hub id is `mode`.
    Start { mode: &'static str, night: u32 },
    /// Start the Custom Night of `mode` with these slider values (hunter id, level), in slider order.
    StartCustom { mode: &'static str, values: Vec<(&'static str, u32)> },
}

pub struct Select {
    spec: &'static ModeSpec,
    screen: Screen,
    row: usize,
    values: Vec<u32>,
    toast: Option<String>,
}

fn doc() -> &'static Value {
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| serde_json::from_str(SELECT_JSON).expect("data/select.json is valid JSON"))
}

/// A label from `data/select.json`. A missing key is a content bug, so this panics with the key's name.
pub fn label(key: &str) -> &'static str {
    doc()["labels"][key].as_str().unwrap_or_else(|| panic!("data/select.json: missing label '{key}'"))
}

/// Replaces each `{name}` in `template` with its value.
fn fill(template: &str, pairs: &[(&str, String)]) -> String {
    pairs.iter().fold(template.to_owned(), |text, (key, value)| text.replace(&format!("{{{key}}}"), value))
}

/// The value at a dotted path; a number in a path indexes an array.
fn at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |cur, key| match cur {
        Value::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
        _ => cur.get(key),
    })
}

/// The spec's strings live as long as the program, so they are leaked once when the data is first read.
fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

fn text_of(value: &Value, key: &str) -> &'static str {
    leak(value[key].as_str().unwrap_or_else(|| panic!("data/select.json: mode is missing '{key}'")))
}

/// The number of nights a rules path holds: the length of an array, or the number itself.
fn night_count(rules: &str, path: &str) -> u32 {
    match at(data::rules(rules), path) {
        Some(Value::Array(items)) => items.len() as u32,
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0) as u32,
        _ => panic!("rules/{rules}.json has no night count at '{path}'"),
    }
}

/// The per-night names a mode's rules give: a list of strings, or a field of each object in a list.
fn names_of(rules: &str, spec: &Value) -> Option<Vec<&'static str>> {
    let names = spec.get("names")?;
    let list = at(data::rules(rules), names["path"].as_str()?)?.as_array()?;
    let field = names.get("field").and_then(Value::as_str);
    Some(
        list.iter()
            .map(|item| {
                let text = match field {
                    Some(key) => item[key].as_str(),
                    None => item.as_str(),
                };
                leak(text.unwrap_or("?"))
            })
            .collect(),
    )
}

fn parse_custom(spec: &Value) -> Option<CustomSpec> {
    let custom = spec.get("custom")?;
    let sliders = custom["sliders"]
        .as_array()
        .expect("data/select.json: custom has sliders")
        .iter()
        .map(|s| Slider { id: leak(s["id"].as_str().unwrap_or("")), label: leak(s["label"].as_str().unwrap_or("")) })
        .collect();
    Some(CustomSpec {
        night: custom["night"].as_u64().expect("custom has night") as u32,
        unlock_night: custom["unlock_night"].as_u64().expect("custom has unlock_night") as u32,
        max: custom["max"].as_u64().expect("custom has max") as u32,
        points_per_level: custom["points_per_level"].as_u64().expect("custom has points_per_level") as u32,
        sliders,
    })
}

/// Every night type on the select screen, in data order.
pub fn specs() -> &'static [ModeSpec] {
    static SPECS: OnceLock<Vec<ModeSpec>> = OnceLock::new();
    SPECS.get_or_init(|| {
        doc()["modes"]
            .as_array()
            .expect("data/select.json has modes")
            .iter()
            .map(|spec| {
                let rules = spec["rules"].as_str().expect("mode has rules");
                ModeSpec {
                    id: text_of(spec, "id"),
                    title: text_of(spec, "title"),
                    save_key: text_of(spec, "save_key"),
                    nights: night_count(rules, spec["nights"].as_str().expect("mode has nights")),
                    night_label: spec.get("night_label").and_then(Value::as_str).map_or(label("night"), leak),
                    names: names_of(rules, spec),
                    custom: parse_custom(spec),
                }
            })
            .collect()
    })
}

/// The night a mode's Custom Night plays (its `custom.night` in the data); 1 for a mode with no Custom Night.
pub fn custom_night(mode_id: &str) -> u32 {
    spec(mode_id).and_then(|s| s.custom.as_ref()).map_or(1, |custom| custom.night)
}

/// The select entry for hub id `mode_id`; None for an id with no night list.
pub fn spec(mode_id: &str) -> Option<&'static ModeSpec> {
    specs().iter().find(|s| s.id == mode_id)
}

impl ModeSpec {
    /// The label for night `number` (1-based): the rules' name when there is one, else the mode's night label.
    pub fn night_name(&self, number: u32) -> String {
        match self.names.as_ref().and_then(|names| names.get(number as usize - 1)) {
            Some(name) => (*name).to_owned(),
            None => fill(self.night_label, &[("n", number.to_string())]),
        }
    }

    /// Whether night `number` can be started: night 1 always, a night once the one before it is cleared. Classic keeps
    /// its own unlock, the highest night the save allows.
    pub fn night_open(&self, number: u32, save: &NightWatchSave) -> bool {
        if number < 1 || number > self.nights {
            return false;
        }
        if self.save_key == CLASSIC {
            return number <= save.unlocked_night();
        }
        save.level_open(self.save_key, number, false)
    }

    /// Every night of the mode with its state from the save.
    pub fn rows(&self, save: &NightWatchSave) -> Vec<NightRow> {
        let cleared = save.cleared_levels(self.save_key);
        (1..=self.nights)
            .map(|number| NightRow {
                number,
                label: self.night_name(number),
                open: self.night_open(number, save),
                cleared: cleared.contains(&number),
                best: save.night_best(self.save_key, number),
            })
            .collect()
    }
}

/// The first night to highlight: the first open one not cleared yet, else the first row.
fn first_focus(spec: &ModeSpec, save: &NightWatchSave) -> usize {
    spec.rows(save).iter().position(|row| row.open && !row.cleared).unwrap_or(0)
}

/// The slider values from the save, clamped to each slider's max; a hunter the save does not hold starts at 0.
fn saved_values(custom: &CustomSpec, mode_key: &str, save: &NightWatchSave) -> Vec<u32> {
    let saved = save.custom_values(mode_key);
    custom
        .sliders
        .iter()
        .map(|slider| saved.iter().find(|(id, _)| id == slider.id).map_or(0, |(_, v)| (*v).min(custom.max)))
        .collect()
}

impl Select {
    /// Opens the select screen for hub id `mode_id`; None for an id with no night list.
    pub fn open(mode_id: &str, save: &NightWatchSave) -> Option<Select> {
        let spec = spec(mode_id)?;
        let values = spec.custom.as_ref().map_or_else(Vec::new, |custom| saved_values(custom, spec.save_key, save));
        Some(Select { spec, screen: Screen::Nights, row: first_focus(spec, save), values, toast: None })
    }

    pub fn spec(&self) -> &'static ModeSpec {
        self.spec
    }

    pub fn screen(&self) -> Screen {
        self.screen
    }

    pub fn row(&self) -> usize {
        self.row
    }

    pub fn toast(&self) -> Option<&str> {
        self.toast.as_deref()
    }

    /// The Custom Night slider values, in slider order.
    pub fn values(&self) -> &[u32] {
        &self.values
    }

    /// Sets slider `index` to `value`, clamped to the slider's max.
    pub fn set_slider(&mut self, index: usize, value: u32) {
        let Some(custom) = self.spec.custom.as_ref() else { return };
        if let Some(slot) = self.values.get_mut(index) {
            *slot = value.min(custom.max);
        }
    }

    /// The points the sliders are worth now: each slider clamped to the max, summed, times the points per level.
    pub fn custom_score(&self) -> u32 {
        self.spec.custom.as_ref().map_or(0, |custom| self.values.iter().map(|v| (*v).min(custom.max)).sum::<u32>() * custom.points_per_level)
    }

    /// Rows on the current screen: the nights and the Custom Night row, or the sliders.
    pub fn row_count(&self) -> usize {
        match self.screen {
            Screen::Nights => self.spec.nights as usize + usize::from(self.spec.custom.is_some()),
            Screen::Custom => self.values.len(),
        }
    }

    /// Puts the current slider values into `save` under the mode's key. The caller saves the file.
    pub fn store_values(&self, save: &mut NightWatchSave) {
        if let Some(custom) = self.spec.custom.as_ref() {
            let pairs: Vec<(String, u32)> = custom.sliders.iter().zip(&self.values).map(|(s, v)| (s.id.to_owned(), *v)).collect();
            save.set_custom_values(self.spec.save_key, &pairs);
        }
    }

    fn enter_custom(&mut self, save: &NightWatchSave) {
        let Some(custom) = self.spec.custom.as_ref() else { return };
        if save.cleared_levels(self.spec.save_key).contains(&custom.unlock_night) {
            self.screen = Screen::Custom;
            self.row = 0;
            self.toast = None;
        } else {
            self.toast = Some(fill(label("custom_locked"), &[("n", custom.unlock_night.to_string())]));
        }
    }

    fn start_night(&mut self, number: u32, save: &NightWatchSave) -> SelectAction {
        if self.spec.night_open(number, save) {
            self.toast = None;
            return SelectAction::Start { mode: self.spec.id, night: number };
        }
        let prev = number.saturating_sub(1).max(1);
        self.toast = Some(fill(label("night_locked"), &[("n", number.to_string()), ("prev", prev.to_string())]));
        SelectAction::None
    }

    /// Applies one input and says what to start, if anything.
    pub fn handle(&mut self, input: SelectInput, save: &NightWatchSave) -> SelectAction {
        match self.screen {
            Screen::Nights => self.handle_nights(input, save),
            Screen::Custom => self.handle_custom(input),
        }
    }

    fn handle_nights(&mut self, input: SelectInput, save: &NightWatchSave) -> SelectAction {
        let count = self.row_count().max(1);
        let nights = self.spec.nights as usize;
        let has_custom = self.spec.custom.is_some();
        match input {
            SelectInput::Up => self.row = (self.row + count - 1) % count,
            SelectInput::Down => self.row = (self.row + 1) % count,
            SelectInput::Left | SelectInput::Right => {}
            SelectInput::CustomKey => {
                if has_custom {
                    self.row = nights;
                    self.enter_custom(save);
                }
            }
            SelectInput::Pick(row) => {
                if row < count {
                    self.row = row;
                    return self.handle_nights(SelectInput::Confirm, save);
                }
            }
            SelectInput::Confirm => {
                if self.row < nights {
                    return self.start_night(self.row as u32 + 1, save);
                }
                if has_custom {
                    self.enter_custom(save);
                }
            }
            SelectInput::Back => return SelectAction::Back,
        }
        SelectAction::None
    }

    fn handle_custom(&mut self, input: SelectInput) -> SelectAction {
        let count = self.row_count().max(1);
        match input {
            SelectInput::Up => self.row = (self.row + count - 1) % count,
            SelectInput::Down => self.row = (self.row + 1) % count,
            SelectInput::Left => {
                let value = self.values.get(self.row).copied().unwrap_or(0);
                self.set_slider(self.row, value.saturating_sub(1));
            }
            SelectInput::Right => {
                let value = self.values.get(self.row).copied().unwrap_or(0);
                self.set_slider(self.row, value + 1);
            }
            SelectInput::Pick(row) => {
                if row < count {
                    self.row = row;
                }
            }
            SelectInput::Confirm => {
                let Some(custom) = self.spec.custom.as_ref() else { return SelectAction::None };
                let values = custom.sliders.iter().zip(&self.values).map(|(s, v)| (s.id, *v)).collect();
                return SelectAction::StartCustom { mode: self.spec.id, values };
            }
            SelectInput::Back => {
                self.screen = Screen::Nights;
                self.row = self.spec.nights as usize;
                self.toast = None;
            }
            SelectInput::CustomKey => {}
        }
        SelectAction::None
    }
}

/// A save with some nights cleared and scores on record, for the screenshots and the tests.
pub fn demo_save() -> NightWatchSave {
    let mut save = NightWatchSave::from_json("{}");
    for mode in ["classic", "mask", "lure", "bedroom", "story", "descent", "salvage_run", "mixed_shift"] {
        for night in 1..=2 {
            save.record_level_clear(mode, night);
        }
    }
    save.record_night_best("classic", 1, 340);
    save.record_night_best("classic", 2, 210);
    save.record_night_best("lure", 1, 180);
    save.record_level_clear("classic", 5);
    save.record_level_clear("lure", 5);
    save.record_level_clear("story", 5);
    save.set_custom_values("classic", &[("dovelle".to_owned(), 8), ("nyxie".to_owned(), 0), ("ravelle".to_owned(), 12), ("ilyra".to_owned(), 3), ("aki".to_owned(), 20)]);
    save
}

const ROW_HEIGHT: f32 = 44.0;
const ROW_WIDTH: f32 = 620.0;
const ROWS_TOP: f32 = 118.0;

fn row_rect(row: usize) -> Rect {
    Rect::new((screen_width() - ROW_WIDTH) * 0.5, ROWS_TOP + ROW_HEIGHT * row as f32, ROW_WIDTH, ROW_HEIGHT - 4.0)
}

/// Reads this frame's keyboard and mouse into at most one select input.
pub fn poll_input(select: &Select) -> Option<SelectInput> {
    if is_key_pressed(KeyCode::Escape) {
        return Some(SelectInput::Back);
    }
    if is_key_pressed(KeyCode::Up) || is_key_pressed(KeyCode::W) {
        return Some(SelectInput::Up);
    }
    if is_key_pressed(KeyCode::Down) || is_key_pressed(KeyCode::S) {
        return Some(SelectInput::Down);
    }
    if is_key_pressed(KeyCode::Left) || is_key_pressed(KeyCode::A) {
        return Some(SelectInput::Left);
    }
    if is_key_pressed(KeyCode::Right) || is_key_pressed(KeyCode::D) {
        return Some(SelectInput::Right);
    }
    if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) {
        return Some(SelectInput::Confirm);
    }
    if select.screen() == Screen::Nights && is_key_pressed(KeyCode::C) {
        return Some(SelectInput::CustomKey);
    }
    let (mx, my) = mouse_position();
    let hovered = (0..select.row_count()).find(|&row| row_rect(row).contains(vec2(mx, my)));
    if is_mouse_button_pressed(MouseButton::Left) {
        return hovered.map(SelectInput::Pick);
    }
    None
}

fn centered(text: &str, y: f32, size: f32, color: Color) {
    let w = measure_text(text, None, size as u16, 1.0).width;
    draw_text(text, (screen_width() - w) * 0.5, y, size, color);
}

/// Draws the select screen: the nights of one mode, or its Custom Night sliders.
pub fn draw(select: &Select, save: &NightWatchSave) {
    clear_background(Color::new(0.02, 0.02, 0.03, 1.0));
    let spec = select.spec();
    centered(spec.title, 58.0, 40.0, RED_TEXT);
    let stars = fill(label("stars"), &[("n", save.stars(spec.save_key).len().to_string())]);
    centered(&stars, 84.0, 20.0, DIM);

    match select.screen() {
        Screen::Nights => draw_nights(select, save),
        Screen::Custom => draw_custom(select),
    }

    if let Some(toast) = select.toast() {
        centered(toast, screen_height() - 60.0, 22.0, AMBER);
    }
    let hint = match select.screen() {
        Screen::Nights if spec.custom.is_some() => label("hint_nights"),
        Screen::Nights => label("hint_nights_plain"),
        Screen::Custom => label("hint_custom"),
    };
    centered(hint, screen_height() - 20.0, 18.0, DIM);
}

fn draw_row_frame(rect: Rect, active: bool) {
    draw_rectangle(rect.x, rect.y, rect.w, rect.h, PANEL);
    if active {
        draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, AMBER);
    }
}

fn draw_nights(select: &Select, save: &NightWatchSave) {
    let spec = select.spec();
    for row in spec.rows(save) {
        let index = row.number as usize - 1;
        let rect = row_rect(index);
        let active = index == select.row();
        draw_row_frame(rect, active);
        let color = if !row.open {
            DIM
        } else if active {
            AMBER
        } else {
            INK
        };
        draw_text(&row.label, rect.x + 16.0, rect.y + 28.0, 26.0, color);
        let status = if !row.open {
            label("locked").to_owned()
        } else if row.cleared {
            label("cleared").to_owned()
        } else {
            String::new()
        };
        let best = match row.best {
            Some(score) => fill(label("best"), &[("score", score.to_string())]),
            None if row.open => label("not_played").to_owned(),
            None => String::new(),
        };
        let tail = match (status.is_empty(), best.is_empty()) {
            (true, _) => best,
            (false, true) => status,
            (false, false) => format!("{status}   {best}"),
        };
        let w = measure_text(&tail, None, 18, 1.0).width;
        draw_text(&tail, rect.x + rect.w - w - 16.0, rect.y + 27.0, 18.0, DIM);
    }

    if let Some(custom) = spec.custom.as_ref() {
        let index = spec.nights as usize;
        let rect = row_rect(index);
        let active = select.row() == index;
        let open = save.cleared_levels(spec.save_key).contains(&custom.unlock_night);
        draw_row_frame(rect, active);
        let color = if !open {
            DIM
        } else if active {
            AMBER
        } else {
            INK
        };
        draw_text(label("custom_row"), rect.x + 16.0, rect.y + 28.0, 26.0, color);
        if !open {
            let tail = label("locked");
            let w = measure_text(tail, None, 18, 1.0).width;
            draw_text(tail, rect.x + rect.w - w - 16.0, rect.y + 27.0, 18.0, DIM);
        }
    }
}

fn draw_custom(select: &Select) {
    let Some(custom) = select.spec().custom.as_ref() else { return };
    let max = custom.max;
    for (index, slider) in custom.sliders.iter().enumerate() {
        let rect = row_rect(index);
        let active = index == select.row();
        draw_row_frame(rect, active);
        let color = if active { AMBER } else { INK };
        draw_text(slider.label, rect.x + 16.0, rect.y + 28.0, 24.0, color);
        let bar_x = rect.x + 200.0;
        let bar_w = rect.w - 200.0 - 80.0;
        let value = select.values().get(index).copied().unwrap_or(0).min(max);
        draw_rectangle(bar_x, rect.y + 14.0, bar_w, 14.0, BAR);
        draw_rectangle(bar_x, rect.y + 14.0, bar_w * value as f32 / max.max(1) as f32, 14.0, AMBER);
        draw_text(&value.to_string(), rect.x + rect.w - 56.0, rect.y + 28.0, 24.0, color);
    }
    let below = row_rect(custom.sliders.len()).y;
    centered(&fill(label("score"), &[("score", select.custom_score().to_string())]), below + 30.0, 22.0, INK);
    centered(&fill(label("max_each"), &[("max", max.to_string())]), below + 56.0, 18.0, DIM);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::{bedtime, dead_air, facility};
    use std::collections::HashMap;

    fn fresh() -> NightWatchSave {
        NightWatchSave::from_json("{}")
    }

    #[test]
    fn every_mode_lists_the_nights_its_rules_hold() {
        let count = |id: &str| spec(id).unwrap_or_else(|| panic!("no spec {id}")).nights;
        assert_eq!(count("classic"), 7);
        assert_eq!(count("bedroom"), 6);
        assert_eq!(count("lure"), 6);
        assert_eq!(count("mask"), 6);
        assert_eq!(count("story"), 6);
        assert_eq!(count("descent"), 5);
        assert_eq!(count("dayshift"), 5);
        assert_eq!(count("mixed"), 5);
        assert_eq!(spec("mixed").unwrap().names.as_ref().unwrap()[4], "ENDLESS");
        assert_eq!(spec("descent").unwrap().names.as_ref().unwrap()[0], "DESCENT 1");
    }

    #[test]
    fn a_locked_night_does_not_start() {
        let save = fresh();
        let mut select = Select::open("classic", &save).expect("classic has a list");
        select.handle(SelectInput::Pick(2), &save);
        assert_eq!(select.handle(SelectInput::Confirm, &save), SelectAction::None);
        assert!(select.toast().is_some());
    }

    #[test]
    fn selecting_a_night_starts_its_mode_and_index() {
        let mut save = fresh();
        save.record_level_clear("classic", 1);
        save.record_level_clear("classic", 2);
        let mut select = Select::open("classic", &save).unwrap();
        select.handle(SelectInput::Pick(2), &save);
        assert_eq!(select.handle(SelectInput::Confirm, &save), SelectAction::Start { mode: "classic", night: 3 });
        let mut descent = Select::open("descent", &fresh()).unwrap();
        assert_eq!(descent.handle(SelectInput::Confirm, &fresh()), SelectAction::Start { mode: "descent", night: 1 });
    }

    #[test]
    fn clearing_a_night_opens_the_next() {
        let mut save = fresh();
        let before = spec("lure").unwrap().rows(&save);
        assert!(before[0].open && !before[1].open);
        save.record_level_clear("lure", 1);
        let after = spec("lure").unwrap().rows(&save);
        assert!(after[0].cleared && after[1].open);
    }

    #[test]
    fn custom_night_stays_locked_until_its_unlock_night_is_cleared() {
        let save = fresh();
        let mut select = Select::open("bedroom", &save).unwrap();
        select.handle(SelectInput::CustomKey, &save);
        assert_eq!(select.screen(), Screen::Nights);
        assert!(select.toast().is_some());
        let mut cleared = fresh();
        cleared.record_level_clear("bedroom", 5);
        select.handle(SelectInput::CustomKey, &cleared);
        assert_eq!(select.screen(), Screen::Custom);
    }

    #[test]
    fn custom_sliders_clamp_to_the_max_in_data() {
        let save = fresh();
        let mut select = Select::open("bedroom", &save).unwrap();
        let max = spec("bedroom").unwrap().custom.as_ref().unwrap().max;
        assert_eq!(max, data::int(data::rules("bedtime"), "max_ai") as u32);
        select.set_slider(0, 99);
        assert_eq!(select.values()[0], max);
        let mut custom_save = fresh();
        custom_save.record_level_clear("bedroom", 5);
        select.handle(SelectInput::CustomKey, &custom_save);
        for _ in 0..(max + 5) {
            select.handle(SelectInput::Right, &custom_save);
        }
        assert_eq!(select.values()[0], max);
        for _ in 0..(max + 5) {
            select.handle(SelectInput::Left, &custom_save);
        }
        assert_eq!(select.values()[0], 0);
    }

    #[test]
    fn custom_night_max_matches_each_mode_rules() {
        let lure = spec("lure").unwrap().custom.as_ref().unwrap().max;
        assert_eq!(lure, data::int(data::rules("dead_air"), "max_slider") as u32);
        let story = spec("story").unwrap().custom.as_ref().unwrap().max;
        assert_eq!(story, data::int(data::rules("facility"), "custom.max_slider") as u32);
    }

    #[test]
    fn custom_score_matches_the_mode_score_rules() {
        let save = fresh();
        let mut bed = Select::open("bedroom", &save).unwrap();
        for (i, v) in [25, 4, 7].into_iter().enumerate() {
            bed.set_slider(i, v);
        }
        assert_eq!(bed.custom_score() as i32, bedtime::custom_score([20, 4, 7]));

        let mut story = Select::open("story", &save).unwrap();
        for (i, v) in [20, 5, 100].into_iter().enumerate() {
            story.set_slider(i, v);
        }
        assert_eq!(story.custom_score(), facility::custom_score([20, 5, 100]));

        let mut lure = Select::open("lure", &save).unwrap();
        for i in 0..6 {
            lure.set_slider(i, 30);
        }
        let levels: HashMap<dead_air::Hunter, i32> = lure
            .spec()
            .custom
            .as_ref()
            .unwrap()
            .sliders
            .iter()
            .zip(lure.values())
            .map(|(s, v)| (dead_air::Hunter::from_id(s.id), *v as i32))
            .collect();
        assert_eq!(lure.custom_score(), dead_air::custom_score(&levels));
    }

    #[test]
    fn custom_start_passes_the_slider_values_in_order() {
        let mut save = fresh();
        save.record_level_clear("bedroom", 5);
        let mut select = Select::open("bedroom", &save).unwrap();
        select.handle(SelectInput::CustomKey, &save);
        select.set_slider(0, 3);
        select.set_slider(1, 9);
        select.set_slider(2, 1);
        assert_eq!(
            select.handle(SelectInput::Confirm, &save),
            SelectAction::StartCustom { mode: "bedroom", values: vec![("loxelle", 3), ("javelle", 9), ("haelith", 1)] }
        );
    }
}
