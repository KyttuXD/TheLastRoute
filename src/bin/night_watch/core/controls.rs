//! Night Watch controls: one table of actions with default keys (the source of truth for the keyboard, the controls
//! screen and the hint line), rebinding saved in `Save::keys`, and key names. Pure logic; KeyCode is a plain value type.
#![allow(dead_code)]

use super::save::Save;
use macroquad::prelude::KeyCode;
use std::collections::BTreeMap;

pub struct ActionDef {
    pub name: &'static str,
    pub label: &'static str,
    pub key: KeyCode,
    pub alt: Option<KeyCode>,
}

macro_rules! act {
    ($n:expr, $l:expr, $k:ident) => {
        ActionDef { name: $n, label: $l, key: KeyCode::$k, alt: None }
    };
    ($n:expr, $l:expr, $k:ident, $a:ident) => {
        ActionDef { name: $n, label: $l, key: KeyCode::$k, alt: Some(KeyCode::$a) }
    };
}

pub const DOOR_LEFT: &str = "nw_door_left";
pub const DOOR_RIGHT: &str = "nw_door_right";
pub const LIGHT_LEFT: &str = "nw_light_left";
pub const LIGHT_RIGHT: &str = "nw_light_right";
pub const LAMP: &str = "nw_lamp";
pub const MONITOR: &str = "nw_monitor";
pub const CAM_PREV: &str = "nw_cam_prev";
pub const CAM_NEXT: &str = "nw_cam_next";
pub const MEET: &str = "nw_meet";
pub const MUTE: &str = "office_radio";
pub const INTERACT: &str = "interact";
pub const PAUSE: &str = "pause";

/// Every rebindable action in display order: the classic office first, then the shared ones, then each mode's extras.
pub const ACTIONS: &[ActionDef] = &[
    act!("nw_door_left", "LEFT DOOR (QUICK KEY)", Z),
    act!("nw_door_right", "RIGHT DOOR (QUICK KEY)", X),
    act!("nw_light_left", "LEFT DOOR LIGHT (HOLD)", G),
    act!("nw_light_right", "RIGHT DOOR LIGHT (HOLD)", H),
    act!("nw_lamp", "DESK LAMP", Y),
    act!("nw_monitor", "CAMERAS (OPEN / CLOSE)", Space, U),
    act!("nw_cam_prev", "PREVIOUS CAMERA", Left),
    act!("nw_cam_next", "NEXT CAMERA", Right),
    act!("nw_meet", "MEET HER EYES (NIGHT 7)", O),
    act!("nw_cam_1", "CAMERA 1", Key1, Kp1),
    act!("nw_cam_2", "CAMERA 2", Key2, Kp2),
    act!("nw_cam_3", "CAMERA 3", Key3, Kp3),
    act!("nw_cam_4", "CAMERA 4", Key4, Kp4),
    act!("nw_cam_5", "CAMERA 5", Key5, Kp5),
    act!("nw_cam_6", "CAMERA 6", Key6, Kp6),
    act!("nw_cam_7", "CAMERA 7", Key7, Kp7),
    act!("nw_cam_8", "CAMERA 8", Key8, Kp8),
    act!("nw_cam_9", "CAMERA 9 (VENT RUN)", Key9, Kp9),
    act!("interact", "USE (PANELS, MONITOR, CHAIR, LAMP; CLOSES THE MONITOR)", E),
    act!("office_radio", "RADIO (MUTE ZELLIA)", R),
    act!("pause", "PAUSE", P),
    // Masked Hours
    act!("nw_wind", "WIND THE MUSIC BOX (HOLD)", R),
    act!("nw_hall_light", "HALL LIGHT", X),
    // Mixed Shift
    act!("nw_vent_l", "LEFT VENT SEAL", V),
    act!("nw_vent_t", "TOP VENT SEAL", B),
    act!("nw_vent_r", "RIGHT VENT SEAL", N),
    act!("nw_vlight_l", "LEFT VENT LIGHT", G),
    act!("nw_vlight_t", "TOP VENT LIGHT", Y),
    act!("nw_vlight_r", "RIGHT VENT LIGHT", H),
    // Dead Air
    act!("nw_lure", "LURE", L),
    act!("nw_reboot_audio", "REBOOT AUDIO", Q),
    act!("nw_reboot_cams", "REBOOT CAMERAS", W),
    act!("nw_reboot_vent", "REBOOT VENTS", E),
    act!("nw_reboot_heater", "REBOOT HEATER", R),
    act!("nw_reboot_fan", "REBOOT FAN", T),
    // Bedtime
    act!("nw_bed_hold", "HOLD THE CLOSET DOOR (HOLD)", A),
    act!("nw_bed_hall", "HALL LIGHT (HOLD)", D),
    act!("nw_bed_light", "BEDSIDE LIGHT (HOLD)", F),
    // The Facility
    act!("nw_fac_beacon", "BEACON", F),
    act!("nw_fac_hide", "HIDE", H),
    // Open Air
    act!("nw_oa_tent", "TENT", T),
    act!("nw_oa_fetch", "FETCH LOGS", F),
    // The Descent
    act!("nw_hold", "HOLD (HOLD)", Space),
];

/// Actions that follow another one unless they are rebound themselves: (alias, target).
pub const ALIASES: &[(&str, &str)] = &[("nw_hall_light", "nw_door_right")];

macro_rules! key_table {
    ($($k:ident),* $(,)?) => { &[$((KeyCode::$k, stringify!($k))),*] };
}

/// KeyCode <-> stable name (the variant's name; this is what the save stores).
const KEY_TABLE: &[(KeyCode, &str)] = key_table!(
    Space, Apostrophe, Comma, Minus, Period, Slash, Semicolon, Equal, LeftBracket, Backslash, RightBracket, GraveAccent,
    Key0, Key1, Key2, Key3, Key4, Key5, Key6, Key7, Key8, Key9,
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Escape, Enter, Tab, Backspace, Insert, Delete, Right, Left, Down, Up, PageUp, PageDown, Home, End,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Kp0, Kp1, Kp2, Kp3, Kp4, Kp5, Kp6, Kp7, Kp8, Kp9, KpDecimal, KpDivide, KpMultiply, KpSubtract, KpAdd, KpEnter,
    LeftShift, LeftControl, LeftAlt, RightShift, RightControl, RightAlt,
);

pub fn keycode_name(k: KeyCode) -> String {
    KEY_TABLE.iter().find(|(c, _)| *c == k).map(|(_, n)| n.to_string()).unwrap_or_else(|| "Unknown".to_string())
}

pub fn keycode_from_name(s: &str) -> Option<KeyCode> {
    KEY_TABLE.iter().find(|(_, n)| *n == s).map(|(c, _)| *c)
}

/// What a key is called on screen ("1", "Left", "Space").
pub fn key_label(k: KeyCode) -> String {
    let name = keycode_name(k);
    if let Some(rest) = name.strip_prefix("Key") {
        if rest.len() == 1 {
            return rest.to_string();
        }
    }
    match name.as_str() {
        "Escape" => "Esc".to_string(),
        "Apostrophe" => "'".to_string(),
        "Comma" => ",".to_string(),
        "Minus" => "-".to_string(),
        "Period" => ".".to_string(),
        "Slash" => "/".to_string(),
        "Semicolon" => ";".to_string(),
        "Equal" => "=".to_string(),
        "LeftBracket" => "[".to_string(),
        "RightBracket" => "]".to_string(),
        "Backslash" => "\\".to_string(),
        "GraveAccent" => "`".to_string(),
        _ => name,
    }
}

pub fn action_def(name: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|a| a.name == name)
}

/// The camera action for camera n (1-9; 9 is Mixed Shift's vent run).
pub fn camera_action(n: i32) -> &'static str {
    match n {
        1 => "nw_cam_1",
        2 => "nw_cam_2",
        3 => "nw_cam_3",
        4 => "nw_cam_4",
        5 => "nw_cam_5",
        6 => "nw_cam_6",
        7 => "nw_cam_7",
        9 => "nw_cam_9",
        _ => "nw_cam_8",
    }
}

fn defaults() -> BTreeMap<&'static str, Vec<KeyCode>> {
    let mut m = BTreeMap::new();
    for a in ACTIONS {
        let mut v = vec![a.key];
        if let Some(alt) = a.alt {
            v.push(alt);
        }
        m.insert(a.name, v);
    }
    m
}

/// The live key table: action name -> keys.
#[derive(Clone, Debug)]
pub struct Bindings {
    map: BTreeMap<&'static str, Vec<KeyCode>>,
}

impl Bindings {
    /// Defaults with the saved overrides applied. An override replaces the primary key and drops the alt.
    pub fn from_save(save: &Save) -> Bindings {
        let mut map = defaults();
        for a in ACTIONS {
            if let Some(k) = save.keys.get(a.name).and_then(|n| keycode_from_name(n)) {
                map.insert(a.name, vec![k]);
            }
        }
        for (alias, target) in ALIASES {
            if !save.keys.contains_key(*alias) {
                if let Some(k) = save.keys.get(*target).and_then(|n| keycode_from_name(n)) {
                    map.insert(*alias, vec![k]);
                }
            }
        }
        Bindings { map }
    }

    /// Rebind an action to a key now and for next time. Aliases follow their target.
    pub fn rebind(&mut self, save: &mut Save, action: &str, key: KeyCode) {
        let def = match action_def(action) {
            Some(d) => d,
            None => return,
        };
        self.map.insert(def.name, vec![key]);
        save.keys.insert(def.name.to_string(), keycode_name(key));
        for (alias, target) in ALIASES {
            if *target == def.name && !save.keys.contains_key(*alias) {
                self.map.insert(*alias, vec![key]);
            }
        }
        save.store();
    }

    /// Back to the default keys, and forget the saved overrides.
    pub fn clear(&mut self, save: &mut Save) {
        self.map = defaults();
        save.keys.clear();
        save.store();
    }

    pub fn keys(&self, action: &str) -> &[KeyCode] {
        self.map.get(action).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The first key of an action as text ("U", "Left", "1"); "?" if none.
    pub fn key_text(&self, action: &str) -> String {
        match self.keys(action).first() {
            Some(k) => key_label(*k),
            None => "?".to_string(),
        }
    }

    fn cams_default(&self) -> bool {
        (1..=8).all(|n| self.key_text(camera_action(n)) == n.to_string())
    }

    /// The hint line under the HUD. `active` are the actions the current office uses (empty: all of them); the monitor
    /// line shows while the monitor is up.
    pub fn hint_line(&self, monitor_up: bool, active: &[&str]) -> String {
        let on = |a: &str| active.is_empty() || active.contains(&a);
        let t = |a: &str| self.key_text(a);
        let mut parts: Vec<String> = Vec::new();
        if monitor_up {
            if self.cams_default() {
                parts.push(format!("{} / {} or 1-8 camera", t(CAM_PREV), t(CAM_NEXT)));
            } else {
                parts.push(format!("{} / {} camera", t(CAM_PREV), t(CAM_NEXT)));
            }
            if active.contains(&MEET) {
                parts.push(format!("{} meet her eyes", t(MEET)));
            }
            parts.push(format!("{} or {} close", t(MONITOR), t(INTERACT)));
            parts.push(format!("{} mute", t(MUTE)));
            return parts.join(" | ");
        }
        parts.push("WASD move".to_string());
        parts.push(format!("{} use (hold on LIGHT)", t(INTERACT)));
        if on(DOOR_LEFT) {
            parts.push(format!("{} / {} doors", t(DOOR_LEFT), t(DOOR_RIGHT)));
        }
        if on(LIGHT_LEFT) {
            parts.push(format!("{} / {} lights", t(LIGHT_LEFT), t(LIGHT_RIGHT)));
        }
        if on(MONITOR) {
            parts.push(format!("{} cameras", t(MONITOR)));
        }
        if on(LAMP) {
            parts.push(format!("{} lamp", t(LAMP)));
        }
        parts.push(format!("{} mute", t(MUTE)));
        parts.push(format!("{} pause", t(PAUSE)));
        if active.contains(&"admin") {
            parts.push("0 admin".to_string());
        }
        parts.join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save() -> Save {
        let mut s = Save::default();
        s.debug_run = true; // store() does nothing
        s
    }

    #[test]
    fn defaults_follow_the_table() {
        let b = Bindings::from_save(&save());
        assert_eq!(b.key_text("nw_door_left"), "Z");
        assert_eq!(b.key_text("nw_door_right"), "X");
        assert_eq!(b.key_text("nw_monitor"), "Space");
        assert_eq!(b.keys("nw_monitor"), &[KeyCode::Space, KeyCode::U]);
        assert_eq!(b.keys("nw_cam_3"), &[KeyCode::Key3, KeyCode::Kp3]);
        assert_eq!(b.key_text("nw_cam_next"), "Right");
        assert_eq!(b.key_text("interact"), "E");
        assert_eq!(b.key_text("office_radio"), "R");
        assert_eq!(b.key_text("pause"), "P");
        assert_eq!(b.key_text("nw_lure"), "L");
        assert_eq!(b.key_text("nw_hold"), "Space");
        assert_eq!(b.key_text("nope"), "?");
        assert!(b.keys("nope").is_empty());
    }

    #[test]
    fn every_action_name_is_unique_and_every_key_has_a_name() {
        for (i, a) in ACTIONS.iter().enumerate() {
            assert!(ACTIONS.iter().skip(i + 1).all(|b| b.name != a.name), "{}", a.name);
            for k in [Some(a.key), a.alt].into_iter().flatten() {
                assert_ne!(keycode_name(k), "Unknown", "{}", a.name);
            }
        }
        for n in 1..=8 {
            assert!(action_def(camera_action(n)).is_some());
        }
    }

    #[test]
    fn key_names_round_trip() {
        for (k, name) in KEY_TABLE {
            assert_eq!(keycode_from_name(name), Some(*k));
            assert_eq!(&keycode_name(*k), name);
        }
        assert_eq!(keycode_from_name("Banana"), None);
        assert_eq!(key_label(KeyCode::Key7), "7");
        assert_eq!(key_label(KeyCode::Kp7), "Kp7");
        assert_eq!(key_label(KeyCode::Escape), "Esc");
    }

    #[test]
    fn rebind_replaces_the_primary_and_drops_the_alt() {
        let mut s = save();
        let mut b = Bindings::from_save(&s);
        b.rebind(&mut s, "nw_monitor", KeyCode::M);
        assert_eq!(b.keys("nw_monitor"), &[KeyCode::M]);
        assert_eq!(s.keys.get("nw_monitor").map(|x| x.as_str()), Some("M"));
        // a fresh table from the same save agrees
        let again = Bindings::from_save(&s);
        assert_eq!(again.keys("nw_monitor"), &[KeyCode::M]);
        // unknown actions are ignored
        b.rebind(&mut s, "bogus", KeyCode::M);
        assert!(!s.keys.contains_key("bogus"));
    }

    #[test]
    fn the_hall_light_alias_follows_the_right_door() {
        let mut s = save();
        let mut b = Bindings::from_save(&s);
        assert_eq!(b.key_text("nw_hall_light"), "X");
        b.rebind(&mut s, "nw_door_right", KeyCode::C);
        assert_eq!(b.key_text("nw_hall_light"), "C");
        assert_eq!(Bindings::from_save(&s).key_text("nw_hall_light"), "C");
        b.rebind(&mut s, "nw_hall_light", KeyCode::V);
        b.rebind(&mut s, "nw_door_right", KeyCode::B);
        assert_eq!(b.key_text("nw_hall_light"), "V");
    }

    #[test]
    fn clear_restores_the_defaults() {
        let mut s = save();
        let mut b = Bindings::from_save(&s);
        b.rebind(&mut s, "nw_lamp", KeyCode::T);
        b.clear(&mut s);
        assert_eq!(b.key_text("nw_lamp"), "Y");
        assert!(s.keys.is_empty());
    }

    #[test]
    fn hint_line_lists_the_office_controls() {
        let b = Bindings::from_save(&save());
        let h = b.hint_line(false, &[]);
        assert!(h.starts_with("WASD move | E use (hold on LIGHT)"), "{h}");
        assert!(h.contains("Z / X doors"));
        assert!(h.contains("G / H lights"));
        assert!(h.contains("Space cameras"));
        assert!(h.contains("Y lamp"));
        assert!(h.ends_with("R mute | P pause"));
        let only_lamp = b.hint_line(false, &["nw_lamp"]);
        assert!(only_lamp.contains("Y lamp") && !only_lamp.contains("doors"));
        assert!(b.hint_line(false, &["admin"]).ends_with("0 admin"));
    }

    #[test]
    fn hint_line_in_the_monitor() {
        let mut s = save();
        let mut b = Bindings::from_save(&s);
        assert_eq!(b.hint_line(true, &[]), "Left / Right or 1-8 camera | Space or E close | R mute");
        assert!(b.hint_line(true, &["nw_meet"]).contains("O meet her eyes"));
        b.rebind(&mut s, "nw_cam_1", KeyCode::Q);
        assert!(b.hint_line(true, &[]).starts_with("Left / Right camera"));
    }
}
