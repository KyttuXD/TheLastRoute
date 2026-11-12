//! Rebindable controls. The actions, their default keys, pad labels and limits live in `data/controls.json`; the player's
//! changes are the `Bindings` stored in the settings file (only actions that differ from the default are saved).
//!
//! CHANGE replaces an action's keys with one key (a key another action holds is swapped over to this action's old first
//! key), + adds a further key (up to `max_keys_per_action`), RESET drops the action's changes. Input code asks
//! `down` / `pressed` (so a rebind takes effect) and prompts ask `prompt_key` (so "PRESS E" follows the rebind).
//!
//! Gamepad: the pad names in the data are labels and the table an input layer would read; no pad is polled yet because
//! reading one needs a dependency.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Where the controls table lives under data/ (the embedded file's name).
pub const CONTROLS_PATH: &str = "controls.json";

#[derive(Clone, Debug, PartialEq)]
pub struct ActionDef {
    pub id: String,
    pub label: String,
    pub section: String,
    pub keys: Vec<String>,
    pub pad: Vec<String>,
    /// Shown but not rebindable (the hotbar digits, pause, fullscreen).
    pub fixed: bool,
    /// Text shown instead of the keys, for a fixed action with a range (`1-5 / WHEEL`).
    pub shown: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
    pub bob_walk: f32,
    pub bob_sprint: f32,
    pub bob_crouch: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub unfocused_fps: u32,
    pub unfocused_sleep_ms: u64,
    pub min_cap_fps: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub max_keys: usize,
    pub cancel_key: String,
    pub reserved: Vec<String>,
    pub sections: Vec<String>,
    pub actions: Vec<ActionDef>,
    pub pad_enabled: bool,
    pub stick_deadzone: f32,
    pub frame: Frame,
    pub camera: Camera,
}

fn strings(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn num(p: &serde_json::Value, k: &str) -> Result<f64, String> {
    p.get(k).and_then(|x| x.as_f64()).filter(|x| x.is_finite() && *x >= 0.0).ok_or_else(|| format!("{k}: must be a number of at least 0"))
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut actions: Vec<ActionDef> = vec![];
        for a in v["actions"].as_array().ok_or("actions: missing")? {
            let id = a["id"].as_str().ok_or("action without an id")?.to_string();
            let keys = strings(&a["keys"]);
            if keys.is_empty() || keys.iter().any(|k| key_code(k).is_none() && mouse_index(k).is_none()) {
                return Err(format!("{id}: keys must be known key names"));
            }
            if actions.iter().any(|x| x.id == id) {
                return Err(format!("{id}: listed twice"));
            }
            actions.push(ActionDef {
                label: a["label"].as_str().unwrap_or(&id).to_string(),
                section: a["section"].as_str().ok_or_else(|| format!("{id}: section missing"))?.to_string(),
                pad: strings(&a["pad"]),
                fixed: a["fixed"].as_bool().unwrap_or(false),
                shown: a["shown"].as_str().map(String::from),
                keys,
                id,
            });
        }
        let max_keys = num(&v, "max_keys_per_action")? as usize;
        if max_keys == 0 {
            return Err("max_keys_per_action: must be at least 1".into());
        }
        let f = &v["frame"];
        let c = &v["camera"];
        Ok(Rules {
            max_keys,
            cancel_key: v["capture_cancel_key"].as_str().unwrap_or("Escape").to_string(),
            reserved: strings(&v["reserved_keys"]),
            sections: strings(&v["sections"]),
            actions,
            pad_enabled: v["pad"]["enabled"].as_bool().unwrap_or(false),
            stick_deadzone: num(&v["pad"], "stick_deadzone")? as f32,
            frame: Frame {
                unfocused_fps: num(f, "unfocused_fps")? as u32,
                unfocused_sleep_ms: num(f, "unfocused_sleep_ms")? as u64,
                min_cap_fps: num(f, "min_cap_fps")? as u32,
            },
            camera: Camera {
                bob_walk: num(c, "bob_walk")? as f32,
                bob_sprint: num(c, "bob_sprint")? as f32,
                bob_crouch: num(c, "bob_crouch")? as f32,
            },
        })
    }

    pub fn action(&self, id: &str) -> Option<&ActionDef> {
        self.actions.iter().find(|a| a.id == id)
    }
}

/// The rules compiled into the executable; a table that fails to parse stops the game at first use (a unit test parses it).
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under(CONTROLS_PATH).next().map(|(_, t)| t).unwrap_or("{}");
        Rules::parse(text).unwrap_or_else(|e| panic!("{CONTROLS_PATH}: {e}"))
    })
}

/// Every key name a binding may use, in the order the panel's capture looks them up.
const KEY_NAMES: &[&str] = &[
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
    "Key0", "Key1", "Key2", "Key3", "Key4", "Key5", "Key6", "Key7", "Key8", "Key9", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8",
    "F9", "F10", "F11", "F12", "Space", "Tab", "Enter", "Backspace", "Up", "Down", "Left", "Right", "LeftShift", "RightShift",
    "LeftControl", "RightControl", "LeftAlt", "RightAlt", "Slash", "Comma", "Period", "Semicolon", "Apostrophe", "Minus", "Equal",
    "LeftBracket", "RightBracket", "Backslash", "GraveAccent", "Escape", "Insert", "Delete", "Home", "End", "PageUp", "PageDown",
    "CapsLock",
];

/// The key a saved name stands for (the macroquad variant's name).
pub fn key_code(name: &str) -> Option<macroquad::prelude::KeyCode> {
    use macroquad::prelude::KeyCode as K;
    Some(match name {
        "A" => K::A, "B" => K::B, "C" => K::C, "D" => K::D, "E" => K::E, "F" => K::F, "G" => K::G, "H" => K::H, "I" => K::I,
        "J" => K::J, "K" => K::K, "L" => K::L, "M" => K::M, "N" => K::N, "O" => K::O, "P" => K::P, "Q" => K::Q, "R" => K::R,
        "S" => K::S, "T" => K::T, "U" => K::U, "V" => K::V, "W" => K::W, "X" => K::X, "Y" => K::Y, "Z" => K::Z,
        "Key0" => K::Key0, "Key1" => K::Key1, "Key2" => K::Key2, "Key3" => K::Key3, "Key4" => K::Key4, "Key5" => K::Key5,
        "Key6" => K::Key6, "Key7" => K::Key7, "Key8" => K::Key8, "Key9" => K::Key9,
        "F1" => K::F1, "F2" => K::F2, "F3" => K::F3, "F4" => K::F4, "F5" => K::F5, "F6" => K::F6, "F7" => K::F7, "F8" => K::F8,
        "F9" => K::F9, "F10" => K::F10, "F11" => K::F11, "F12" => K::F12,
        "Space" => K::Space, "Tab" => K::Tab, "Enter" => K::Enter, "Backspace" => K::Backspace,
        "Up" => K::Up, "Down" => K::Down, "Left" => K::Left, "Right" => K::Right,
        "LeftShift" => K::LeftShift, "RightShift" => K::RightShift, "LeftControl" => K::LeftControl,
        "RightControl" => K::RightControl, "LeftAlt" => K::LeftAlt, "RightAlt" => K::RightAlt,
        "Slash" => K::Slash, "Comma" => K::Comma, "Period" => K::Period, "Semicolon" => K::Semicolon,
        "Apostrophe" => K::Apostrophe, "Minus" => K::Minus, "Equal" => K::Equal, "LeftBracket" => K::LeftBracket,
        "RightBracket" => K::RightBracket, "Backslash" => K::Backslash, "GraveAccent" => K::GraveAccent,
        "Escape" => K::Escape, "Insert" => K::Insert, "Delete" => K::Delete, "Home" => K::Home, "End" => K::End,
        "PageUp" => K::PageUp, "PageDown" => K::PageDown, "CapsLock" => K::CapsLock,
        _ => return None,
    })
}

/// 0 left, 1 right, 2 middle for `MouseLeft` / `MouseRight` / `MouseMiddle`.
pub fn mouse_index(name: &str) -> Option<usize> {
    ["MouseLeft", "MouseRight", "MouseMiddle"].iter().position(|n| *n == name)
}

fn mouse_button(i: usize) -> macroquad::prelude::MouseButton {
    use macroquad::prelude::MouseButton::*;
    [Left, Right, Middle][i]
}

/// A key name as a prompt or the panel shows it, in capitals.
pub fn display_name(name: &str) -> String {
    if let Some(i) = mouse_index(name) {
        return format!("MOUSE {}", i + 1);
    }
    match name {
        "LeftShift" | "RightShift" => "SHIFT".into(),
        "LeftControl" | "RightControl" => "CTRL".into(),
        "LeftAlt" | "RightAlt" => "ALT".into(),
        "Slash" => "/".into(),
        "GraveAccent" => "`".into(),
        "Comma" => ",".into(),
        "Period" => ".".into(),
        n if n.starts_with("Key") && n.len() == 4 => n[3..].to_string(),
        n => n.to_uppercase(),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BindError {
    UnknownAction,
    Fixed,
    UnknownKey,
    Reserved,
    TooMany,
    /// The key belongs to a fixed action, or taking it would leave another action with no key.
    Conflict(String),
}

/// What a capture step did.
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureResult {
    Waiting,
    Cancelled,
    Bound,
    Refused(BindError),
}

/// The player's changes: action id to its keys, only where they differ from the default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bindings {
    over: BTreeMap<String, Vec<String>>,
}

impl Bindings {
    /// The action's keys as the player has them, borrowed (the per-frame input checks call this, so no copy).
    fn keys_ref<'a>(&'a self, r: &'a Rules, id: &str) -> &'a [String] {
        match self.over.get(id) {
            Some(k) => k.as_slice(),
            None => r.action(id).map_or(&[][..], |a| a.keys.as_slice()),
        }
    }

    pub fn keys_in(&self, r: &Rules, id: &str) -> Vec<String> {
        self.keys_ref(r, id).to_vec()
    }

    pub fn keys(&self, id: &str) -> Vec<String> {
        self.keys_in(rules(), id)
    }

    fn store(&mut self, r: &Rules, id: &str, keys: Vec<String>) {
        if r.action(id).is_some_and(|a| a.keys == keys) {
            self.over.remove(id);
        } else {
            self.over.insert(id.to_string(), keys);
        }
    }

    fn check(r: &Rules, id: &str, key: &str) -> Result<(), BindError> {
        let a = r.action(id).ok_or(BindError::UnknownAction)?;
        if a.fixed {
            return Err(BindError::Fixed);
        }
        if key_code(key).is_none() && mouse_index(key).is_none() {
            return Err(BindError::UnknownKey);
        }
        if r.reserved.iter().any(|k| k == key) {
            return Err(BindError::Reserved);
        }
        Ok(())
    }

    /// The other action that holds `key`, if any.
    fn holder(&self, r: &Rules, id: &str, key: &str) -> Option<String> {
        r.actions.iter().find(|a| a.id != id && self.keys_ref(r, &a.id).iter().any(|k| k == key)).map(|a| a.id.clone())
    }

    /// CHANGE: the action gets only `key`; an action that held `key` takes this action's old first key in its place.
    pub fn set_in(&mut self, r: &Rules, id: &str, key: &str) -> Result<(), BindError> {
        Self::check(r, id, key)?;
        let old = self.keys_in(r, id);
        if let Some(other) = self.holder(r, id, key) {
            if r.action(&other).is_some_and(|a| a.fixed) {
                return Err(BindError::Conflict(other));
            }
            let mut theirs = self.keys_in(r, &other);
            theirs.retain(|k| k != key);
            if let Some(give) = old.first().filter(|k| !theirs.contains(k)) {
                theirs.push(give.clone());
            }
            if theirs.is_empty() {
                return Err(BindError::Conflict(other));
            }
            self.store(r, &other, theirs);
        }
        self.store(r, id, vec![key.to_string()]);
        Ok(())
    }

    /// +: one more key for the action. A key another action holds moves over unless that leaves it with none.
    pub fn add_in(&mut self, r: &Rules, id: &str, key: &str) -> Result<(), BindError> {
        Self::check(r, id, key)?;
        let mut mine = self.keys_in(r, id);
        if mine.iter().any(|k| k == key) {
            return Ok(());
        }
        if mine.len() >= r.max_keys {
            return Err(BindError::TooMany);
        }
        if let Some(other) = self.holder(r, id, key) {
            let mut theirs = self.keys_in(r, &other);
            theirs.retain(|k| k != key);
            if theirs.is_empty() || r.action(&other).is_some_and(|a| a.fixed) {
                return Err(BindError::Conflict(other));
            }
            self.store(r, &other, theirs);
        }
        mine.push(key.to_string());
        self.store(r, id, mine);
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn set(&mut self, id: &str, key: &str) -> Result<(), BindError> {
        self.set_in(rules(), id, key)
    }

    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn add(&mut self, id: &str, key: &str) -> Result<(), BindError> {
        self.add_in(rules(), id, key)
    }

    /// RESET: the action takes its default keys back. An action that holds one of them now gives it up; one left with
    /// no key takes this action's old first key (as CHANGE swaps), or goes back to its own defaults when that key is
    /// one of the defaults being taken back. No key ends up on two actions.
    pub fn reset_in(&mut self, r: &Rules, id: &str) {
        let old_first = self.keys_ref(r, id).first().cloned();
        if self.over.remove(id).is_none() {
            return;
        }
        let Some(a) = r.action(id) else { return };
        for key in &a.keys {
            let Some(other) = self.holder(r, id, key) else { continue };
            if r.action(&other).is_some_and(|o| o.fixed) {
                continue; // a fixed action keeps its defaults, and no two actions share a default key
            }
            let mut theirs = self.keys_in(r, &other);
            theirs.retain(|k| k != key);
            if theirs.is_empty() {
                if let Some(give) = old_first.as_ref().filter(|k| !a.keys.contains(k)) {
                    theirs.push(give.clone());
                }
            }
            if theirs.is_empty() {
                self.reset_in(r, &other);
            } else {
                self.store(r, &other, theirs);
            }
        }
    }

    pub fn reset(&mut self, id: &str) {
        self.reset_in(rules(), id);
    }

    #[cfg_attr(not(test), allow(dead_code))] // not wired into the game yet; its tests use it
    pub fn reset_all(&mut self) {
        self.over.clear();
    }

    pub fn is_default(&self, id: &str) -> bool {
        !self.over.contains_key(id)
    }

    /// The saved form: `{ action: [keys] }` for the changed actions only.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.over).unwrap_or_default()
    }

    /// Reads the saved form. Unknown and fixed actions, unknown or reserved keys are dropped. The saved actions are read
    /// together (the rest at their defaults), then any saved action that shares a key with another action goes back to
    /// its default, repeated until none does, so a saved swap loads as it was and a hand-edited file cannot leave a key
    /// doing two things.
    pub fn from_json_in(r: &Rules, v: &serde_json::Value) -> Bindings {
        let mut b = Bindings::default();
        let Some(map) = v.as_object() else { return b };
        for a in r.actions.iter().filter(|a| !a.fixed) {
            let Some(list) = map.get(&a.id) else { continue };
            let mut keys: Vec<String> = vec![];
            for k in strings(list) {
                if (key_code(&k).is_some() || mouse_index(&k).is_some()) && !r.reserved.contains(&k) && !keys.contains(&k) {
                    keys.push(k);
                }
            }
            keys.truncate(r.max_keys);
            if !keys.is_empty() {
                b.store(r, &a.id, keys);
            }
        }
        loop {
            let clashing: Vec<String> = b
                .over
                .iter()
                .filter(|(id, keys)| keys.iter().any(|k| b.holder(r, id, k).is_some()))
                .map(|(id, _)| id.clone())
                .collect();
            if clashing.is_empty() {
                return b;
            }
            for id in clashing {
                b.over.remove(&id);
            }
        }
    }

    pub fn from_json(v: &serde_json::Value) -> Bindings {
        Self::from_json_in(rules(), v)
    }

    pub fn down(&self, id: &str) -> bool {
        use macroquad::prelude::{is_key_down, is_mouse_button_down};
        self.keys_ref(rules(), id).iter().any(|k| key_code(k).is_some_and(is_key_down) || mouse_index(k).is_some_and(|i| is_mouse_button_down(mouse_button(i))))
    }

    pub fn pressed(&self, id: &str) -> bool {
        use macroquad::prelude::{is_key_pressed, is_mouse_button_pressed};
        self.keys_ref(rules(), id).iter().any(|k| key_code(k).is_some_and(is_key_pressed) || mouse_index(k).is_some_and(|i| is_mouse_button_pressed(mouse_button(i))))
    }

    /// The text a prompt shows for the action (`PRESS [E]`): the first key as the player has it bound.
    pub fn prompt_key_in(&self, r: &Rules, id: &str) -> String {
        match r.action(id) {
            Some(ActionDef { shown: Some(s), .. }) => s.clone(),
            _ => self.keys_in(r, id).first().map(|k| display_name(k)).unwrap_or_default(),
        }
    }

    pub fn prompt_key(&self, id: &str) -> String {
        self.prompt_key_in(rules(), id)
    }

    /// The panel's key column: every key, then the pad names, as `E / MOUSE 2 / PAD X`.
    pub fn label_in(&self, r: &Rules, id: &str) -> String {
        let Some(a) = r.action(id) else { return String::new() };
        if let Some(s) = &a.shown {
            return s.clone();
        }
        let mut parts: Vec<String> = self.keys_in(r, id).iter().map(|k| display_name(k)).collect();
        for p in &a.pad {
            if !parts.contains(p) {
                parts.push(p.clone());
            }
        }
        parts.join(" / ")
    }

    pub fn label(&self, id: &str) -> String {
        self.label_in(rules(), id)
    }
}

/// An in-progress CHANGE or +: which action is waiting for a key. Kept here, not in the menu state, so the panel and the
/// input code share it.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    pub action: String,
    pub add: bool,
}

thread_local!(static CAPTURE: std::cell::RefCell<Option<Capture>> = const { std::cell::RefCell::new(None) });

pub fn begin_capture(action: &str, add: bool) {
    CAPTURE.with(|c| *c.borrow_mut() = Some(Capture { action: action.to_string(), add }));
}

pub fn capture() -> Option<Capture> {
    CAPTURE.with(|c| c.borrow().clone())
}

pub fn cancel_capture() {
    CAPTURE.with(|c| *c.borrow_mut() = None);
}

/// Gives a pressed key to the waiting capture (pure; `poll_capture` reads the keyboard). The cancel key ends it with no
/// change.
pub fn apply_capture(r: &Rules, b: &mut Bindings, key: &str) -> CaptureResult {
    let Some(cap) = capture() else { return CaptureResult::Waiting };
    if key == r.cancel_key {
        cancel_capture();
        return CaptureResult::Cancelled;
    }
    let res = if cap.add { b.add_in(r, &cap.action, key) } else { b.set_in(r, &cap.action, key) };
    cancel_capture();
    match res {
        Ok(()) => CaptureResult::Bound,
        Err(e) => CaptureResult::Refused(e),
    }
}

/// Reads this frame's key (or the right / middle mouse button; the left button is the click that began the capture) and
/// hands it to the capture. Call once a frame while the panel is open.
pub fn poll_capture(b: &mut Bindings) -> CaptureResult {
    use macroquad::prelude::*;
    if capture().is_none() {
        return CaptureResult::Waiting;
    }
    let r = rules();
    if let Some(k) = get_last_key_pressed() {
        if let Some(name) = KEY_NAMES.iter().find(|n| key_code(n) == Some(k)) {
            return apply_capture(r, b, name);
        }
    }
    for name in ["MouseRight", "MouseMiddle"] {
        if mouse_index(name).is_some_and(|i| is_mouse_button_pressed(mouse_button(i))) {
            return apply_capture(r, b, name);
        }
    }
    CaptureResult::Waiting
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> &'static Rules {
        rules()
    }

    #[test]
    fn the_data_parses_and_holds_the_godot_defaults() {
        let b = Bindings::default();
        assert_eq!(b.keys("inventory"), ["Tab"]);
        assert_eq!(b.keys("chat")[0], "T");
        assert_eq!(b.keys("player_list"), ["O"]);
        assert!(r().action("inventory").unwrap().pad.contains(&"PAD Y".to_string()));
        assert!(r().action("jump").unwrap().pad.contains(&"PAD A".to_string()));
        assert!(b.label("interact").contains("MOUSE 2") && b.label("interact").contains("PAD X"));
        // no two actions share a default key, and every action sits in a listed section
        let mut all: Vec<&String> = r().actions.iter().flat_map(|a| a.keys.iter()).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "a default key is held by two actions");
        assert!(r().actions.iter().all(|a| r().sections.contains(&a.section)));
        assert!(KEY_NAMES.iter().all(|n| key_code(n).is_some()));
    }

    #[test]
    fn bindings_round_trip_and_reset() {
        let mut b = Bindings::default();
        b.set("flashlight", "H").unwrap();
        b.add("flashlight", "V").unwrap();
        assert_eq!(b.keys("flashlight"), ["H", "V"]);
        assert_eq!(Bindings::from_json(&b.to_json()), b);
        assert_eq!(Bindings::default().to_json(), serde_json::json!({}));
        b.reset("flashlight");
        assert_eq!(b.keys("flashlight"), ["F"]);
        assert!(b.is_default("flashlight") && b.to_json() == serde_json::json!({}));
        // setting an action back to its default key leaves nothing saved
        b.set("flashlight", "N").unwrap();
        b.set("flashlight", "F").unwrap();
        assert!(b.is_default("flashlight"));
    }

    #[test]
    fn a_taken_key_swaps_with_the_action_that_held_it() {
        let mut b = Bindings::default();
        b.set("flashlight", "G").unwrap(); // G is the taser's
        assert_eq!(b.keys("flashlight"), ["G"]);
        assert_eq!(b.keys("taser"), ["F"], "the taser takes the flashlight's old key");
        b.reset_all();
        assert_eq!(b.keys("taser"), ["G"]);
    }

    #[test]
    fn refusals_leave_the_bindings_alone() {
        let mut b = Bindings::default();
        assert_eq!(b.set("pause", "P"), Err(BindError::Fixed));
        assert_eq!(b.set("jump", "Escape"), Err(BindError::Reserved));
        assert_eq!(b.set("jump", "Banana"), Err(BindError::UnknownKey));
        assert_eq!(b.set("nope", "P"), Err(BindError::UnknownAction));
        b.add("flashlight", "H").unwrap();
        b.add("flashlight", "V").unwrap();
        assert_eq!(b.add("flashlight", "B"), Err(BindError::TooMany));
        assert_eq!(b.add("jump", "Tab"), Err(BindError::Conflict("inventory".into())), "+ never strips an action of its only key");
        assert_eq!(b.keys("inventory"), ["Tab"]);
    }

    #[test]
    fn a_hand_edited_file_cannot_strand_an_action() {
        let v = serde_json::json!({ "jump": [], "crouch": ["Nope"], "pause": ["P"], "taser": ["Escape"], "bogus": ["A"] });
        assert_eq!(Bindings::from_json(&v), Bindings::default());
        assert_eq!(Bindings::from_json(&serde_json::json!(5)), Bindings::default());
    }

    #[test]
    fn the_limit_is_data() {
        let text = include_str!("../data/controls.json").replace("\"max_keys_per_action\": 3", "\"max_keys_per_action\": 1");
        let tight = Rules::parse(&text).unwrap();
        let mut b = Bindings::default();
        assert_eq!(b.add_in(&tight, "flashlight", "H"), Err(BindError::TooMany));
    }

    #[test]
    fn the_prompt_label_follows_the_rebind() {
        let mut b = Bindings::default();
        assert_eq!(b.prompt_key("interact"), "E");
        b.set("interact", "R").unwrap();
        assert_eq!(b.prompt_key("interact"), "R");
        assert_eq!(b.prompt_key("inventory"), "TAB");
        assert_eq!(b.prompt_key("hotbar"), "1-5 / WHEEL");
        assert_eq!(display_name("MouseRight"), "MOUSE 2");
        assert_eq!(display_name("Key3"), "3");
        assert_eq!(display_name("LeftShift"), "SHIFT");
    }

    #[test]
    fn capture_binds_cancels_and_reports_refusals() {
        let mut b = Bindings::default();
        cancel_capture();
        assert_eq!(apply_capture(r(), &mut b, "H"), CaptureResult::Waiting, "no capture, nothing happens");
        begin_capture("flashlight", false);
        assert_eq!(apply_capture(r(), &mut b, "Escape"), CaptureResult::Cancelled);
        assert!(capture().is_none() && b.is_default("flashlight"));
        begin_capture("flashlight", false);
        assert_eq!(apply_capture(r(), &mut b, "H"), CaptureResult::Bound);
        begin_capture("flashlight", true);
        assert_eq!(apply_capture(r(), &mut b, "V"), CaptureResult::Bound);
        assert_eq!(b.keys("flashlight"), ["H", "V"]);
        begin_capture("jump", false);
        assert_eq!(apply_capture(r(), &mut b, "F11"), CaptureResult::Refused(BindError::Reserved));
    }

    /// Every key held by at most one action.
    fn no_shared_key(b: &Bindings) {
        let mut all: Vec<String> = r().actions.iter().flat_map(|a| b.keys(&a.id)).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "a key is on two actions: {:?}", b.to_json());
    }

    #[test]
    fn reset_never_leaves_a_key_on_two_actions() {
        let mut b = Bindings::default();
        b.set("flashlight", "G").unwrap(); // the taser's G swaps over, the taser takes F
        b.reset("taser");
        assert_eq!(b.keys("taser"), ["G"]);
        assert_eq!(b.keys("flashlight"), ["F"], "the flashlight takes the taser's old key back");
        no_shared_key(&b);
        no_shared_key(&Bindings::from_json(&b.to_json()));
        // the holder keeps its other keys when it has some
        let mut b = Bindings::default();
        b.set("flashlight", "G").unwrap();
        b.add("flashlight", "H").unwrap();
        b.reset("taser");
        assert_eq!(b.keys("taser"), ["G"]);
        assert_eq!(b.keys("flashlight"), ["H"]);
        no_shared_key(&b);
        assert_eq!(Bindings::from_json(&b.to_json()), b);
    }

    #[test]
    fn a_saved_swap_loads_unchanged_and_a_clash_is_dropped() {
        let swap = serde_json::json!({ "flashlight": ["G"], "taser": ["F"] });
        let b = Bindings::from_json(&swap);
        assert_eq!(b.keys("flashlight"), ["G"]);
        assert_eq!(b.keys("taser"), ["F"]);
        assert_eq!(b.to_json(), swap);
        let clash = Bindings::from_json(&serde_json::json!({ "flashlight": ["G"] }));
        assert_eq!(clash, Bindings::default(), "the taser still holds G by default");
        no_shared_key(&clash);
    }

    #[test]
    fn the_camera_numbers_are_data() {
        
        assert!(r().camera.bob_sprint > r().camera.bob_walk && r().camera.bob_walk > r().camera.bob_crouch);
    }
}
