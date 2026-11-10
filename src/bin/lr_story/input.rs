//! The player's key bindings. Each `Action` has one primary key the player can change (in the settings screen); a few
//! extra keys are fixed: the arrow keys also move, Tab also opens the inventory, Enter also interacts. Escape (pause),
//! the digits 1-9 (slots), the mouse buttons and the wheel are fixed and cannot be bound.
//!
//! Bindings are saved by key name (`key_name` / `key_from_name`). Binding an action to a key another action already
//! uses swaps the two keys, so no key ever drives two actions.

use macroquad::prelude::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Sprint,
    Crouch,
    Jump,
    Interact,
    UseItem,
    Inventory,
    Journal,
    View,
}

impl Action {
    pub const ALL: [Action; 12] = [
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Sprint,
        Action::Crouch,
        Action::Jump,
        Action::Interact,
        Action::UseItem,
        Action::Inventory,
        Action::Journal,
        Action::View,
    ];

    /// Stable name used in the settings file.
    pub fn id(self) -> &'static str {
        match self {
            Action::Forward => "forward",
            Action::Back => "back",
            Action::Left => "left",
            Action::Right => "right",
            Action::Sprint => "sprint",
            Action::Crouch => "crouch",
            Action::Jump => "jump",
            Action::Interact => "interact",
            Action::UseItem => "use_item",
            Action::Inventory => "inventory",
            Action::Journal => "journal",
            Action::View => "view",
        }
    }

    /// The label shown in the controls list.
    pub fn label(self) -> &'static str {
        match self {
            Action::Forward => "Move forward",
            Action::Back => "Move back",
            Action::Left => "Strafe left",
            Action::Right => "Strafe right",
            Action::Sprint => "Sprint",
            Action::Crouch => "Crouch",
            Action::Jump => "Jump",
            Action::Interact => "Interact",
            Action::UseItem => "Use item",
            Action::Inventory => "Inventory",
            Action::Journal => "Journal",
            Action::View => "Switch view",
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.id() == id)
    }

    fn index(self) -> usize {
        Action::ALL.iter().position(|a| *a == self).unwrap_or(0)
    }

    pub fn default_key(self) -> KeyCode {
        match self {
            Action::Forward => KeyCode::W,
            Action::Back => KeyCode::S,
            Action::Left => KeyCode::A,
            Action::Right => KeyCode::D,
            Action::Sprint => KeyCode::LeftShift,
            Action::Crouch => KeyCode::LeftControl,
            Action::Jump => KeyCode::Space,
            Action::Interact => KeyCode::E,
            Action::UseItem => KeyCode::F,
            Action::Inventory => KeyCode::I,
            Action::Journal => KeyCode::J,
            Action::View => KeyCode::V,
        }
    }

    /// Fixed extra keys that also trigger this action.
    pub fn extra_keys(self) -> &'static [KeyCode] {
        match self {
            Action::Forward => &[KeyCode::Up],
            Action::Back => &[KeyCode::Down],
            Action::Left => &[KeyCode::Left],
            Action::Right => &[KeyCode::Right],
            Action::Interact => &[KeyCode::Enter],
            Action::Inventory => &[KeyCode::Tab],
            _ => &[],
        }
    }
}

/// Every key the player may bind, with its saved name and its shown label. Digits and Escape are not here.
const KEY_TABLE: &[(&str, &str, KeyCode)] = &[
    ("A", "A", KeyCode::A),
    ("B", "B", KeyCode::B),
    ("C", "C", KeyCode::C),
    ("D", "D", KeyCode::D),
    ("E", "E", KeyCode::E),
    ("F", "F", KeyCode::F),
    ("G", "G", KeyCode::G),
    ("H", "H", KeyCode::H),
    ("I", "I", KeyCode::I),
    ("J", "J", KeyCode::J),
    ("K", "K", KeyCode::K),
    ("L", "L", KeyCode::L),
    ("M", "M", KeyCode::M),
    ("N", "N", KeyCode::N),
    ("O", "O", KeyCode::O),
    ("P", "P", KeyCode::P),
    ("Q", "Q", KeyCode::Q),
    ("R", "R", KeyCode::R),
    ("S", "S", KeyCode::S),
    ("T", "T", KeyCode::T),
    ("U", "U", KeyCode::U),
    ("V", "V", KeyCode::V),
    ("W", "W", KeyCode::W),
    ("X", "X", KeyCode::X),
    ("Y", "Y", KeyCode::Y),
    ("Z", "Z", KeyCode::Z),
    ("Up", "Up", KeyCode::Up),
    ("Down", "Down", KeyCode::Down),
    ("Left", "Left", KeyCode::Left),
    ("Right", "Right", KeyCode::Right),
    ("LeftShift", "Left Shift", KeyCode::LeftShift),
    ("RightShift", "Right Shift", KeyCode::RightShift),
    ("LeftControl", "Left Ctrl", KeyCode::LeftControl),
    ("RightControl", "Right Ctrl", KeyCode::RightControl),
    ("LeftAlt", "Left Alt", KeyCode::LeftAlt),
    ("RightAlt", "Right Alt", KeyCode::RightAlt),
    ("Space", "Space", KeyCode::Space),
    ("Tab", "Tab", KeyCode::Tab),
    ("Enter", "Enter", KeyCode::Enter),
    ("Backspace", "Backspace", KeyCode::Backspace),
    ("F1", "F1", KeyCode::F1),
    ("F2", "F2", KeyCode::F2),
    ("F3", "F3", KeyCode::F3),
    ("F4", "F4", KeyCode::F4),
    ("F5", "F5", KeyCode::F5),
    ("F6", "F6", KeyCode::F6),
    ("F7", "F7", KeyCode::F7),
    ("F8", "F8", KeyCode::F8),
    ("F9", "F9", KeyCode::F9),
    ("F10", "F10", KeyCode::F10),
    ("F11", "F11", KeyCode::F11),
    ("F12", "F12", KeyCode::F12),
    ("Comma", ",", KeyCode::Comma),
    ("Period", ".", KeyCode::Period),
    ("Slash", "/", KeyCode::Slash),
    ("Semicolon", ";", KeyCode::Semicolon),
    ("Apostrophe", "'", KeyCode::Apostrophe),
    ("LeftBracket", "[", KeyCode::LeftBracket),
    ("RightBracket", "]", KeyCode::RightBracket),
    ("Minus", "-", KeyCode::Minus),
    ("Equal", "=", KeyCode::Equal),
];

/// The saved name of a bindable key, or None for a key that cannot be bound (digits, Escape, ...).
pub fn key_name(key: KeyCode) -> Option<&'static str> {
    KEY_TABLE.iter().find(|(_, _, k)| *k == key).map(|(n, _, _)| *n)
}

pub fn key_from_name(name: &str) -> Option<KeyCode> {
    KEY_TABLE.iter().find(|(n, _, _)| *n == name).map(|(_, _, k)| *k)
}

/// The text shown for a key in the controls list.
pub fn key_label(key: KeyCode) -> &'static str {
    KEY_TABLE.iter().find(|(_, _, k)| *k == key).map(|(_, l, _)| *l).unwrap_or("?")
}

pub fn is_bindable(key: KeyCode) -> bool {
    key_name(key).is_some()
}

/// Every bindable key, for the "press a key" capture to scan.
pub fn bindable_keys() -> impl Iterator<Item = KeyCode> {
    KEY_TABLE.iter().map(|(_, _, k)| *k)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    keys: [KeyCode; 12],
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings { keys: Action::ALL.map(Action::default_key) }
    }
}

impl Bindings {
    pub fn key(&self, a: Action) -> KeyCode {
        self.keys[a.index()]
    }

    /// Binds `a` to `key`. A key that cannot be bound (digits, Escape, ...) is refused with false and nothing changes.
    /// If another action already uses `key`, it takes `a`'s old key (a swap).
    pub fn rebind(&mut self, a: Action, key: KeyCode) -> bool {
        // The fixed extras (arrows, Enter, Tab) always drive their own action, so binding one elsewhere would make one
        // key drive two actions: refused like digits and Escape.
        if !is_bindable(key) || Action::ALL.iter().any(|o| o.extra_keys().contains(&key)) {
            return false;
        }
        let old = self.keys[a.index()];
        if let Some(other) = Action::ALL.iter().copied().find(|o| *o != a && self.keys[o.index()] == key) {
            self.keys[other.index()] = old;
        }
        self.keys[a.index()] = key;
        true
    }

    #[allow(dead_code)]
    pub fn reset(&mut self) {
        *self = Bindings::default();
    }

    /// Held now: the bound key or one of the action's fixed extras.
    pub fn down(&self, a: Action) -> bool {
        is_key_down(self.key(a)) || a.extra_keys().iter().any(|k| is_key_down(*k))
    }

    /// Went down this frame.
    pub fn pressed(&self, a: Action) -> bool {
        is_key_pressed(self.key(a)) || a.extra_keys().iter().any(|k| is_key_pressed(*k))
    }

    /// Action id -> key name, for saving.
    pub fn to_names(&self) -> BTreeMap<String, String> {
        Action::ALL
            .iter()
            .filter_map(|a| key_name(self.key(*a)).map(|n| (a.id().to_string(), n.to_string())))
            .collect()
    }

    /// Bindings from saved names. Unknown actions, unknown keys and keys already taken are ignored (the default
    /// stays where it is free), so the result never has one key on two actions.
    pub fn from_names<'a>(names: impl IntoIterator<Item = (&'a str, &'a str)>) -> Bindings {
        let mut keys: [Option<KeyCode>; 12] = [None; 12];
        for (id, key) in names {
            if let (Some(a), Some(k)) = (Action::from_id(id), key_from_name(key)) {
                if keys[a.index()].is_none() && !keys.contains(&Some(k)) {
                    keys[a.index()] = Some(k);
                }
            }
        }
        // Actions left without a key get their default if it is free, else the first free bindable key.
        for a in Action::ALL {
            if keys[a.index()].is_none() {
                let d = a.default_key();
                let pick = if !keys.contains(&Some(d)) {
                    d
                } else {
                    bindable_keys().find(|k| !keys.contains(&Some(*k))).unwrap_or(d)
                };
                keys[a.index()] = Some(pick);
            }
        }
        let mut b = Bindings::default();
        for a in Action::ALL {
            b.keys[a.index()] = keys[a.index()].unwrap_or_else(|| a.default_key());
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_current_controls() {
        let b = Bindings::default();
        assert_eq!(b.key(Action::Forward), KeyCode::W);
        assert_eq!(b.key(Action::Back), KeyCode::S);
        assert_eq!(b.key(Action::Left), KeyCode::A);
        assert_eq!(b.key(Action::Right), KeyCode::D);
        assert_eq!(b.key(Action::Sprint), KeyCode::LeftShift);
        assert_eq!(b.key(Action::Crouch), KeyCode::LeftControl);
        assert_eq!(b.key(Action::Jump), KeyCode::Space);
        assert_eq!(b.key(Action::Interact), KeyCode::E);
        assert_eq!(b.key(Action::UseItem), KeyCode::F);
        assert_eq!(b.key(Action::Inventory), KeyCode::I);
        assert_eq!(b.key(Action::Journal), KeyCode::J);
        assert_eq!(b.key(Action::View), KeyCode::V);
        assert!(Action::Forward.extra_keys().contains(&KeyCode::Up));
        assert!(Action::Inventory.extra_keys().contains(&KeyCode::Tab));
        assert!(Action::Interact.extra_keys().contains(&KeyCode::Enter));
    }

    #[test]
    fn every_bindable_key_round_trips_by_name() {
        for key in bindable_keys() {
            let name = key_name(key).expect("named");
            assert_eq!(key_from_name(name), Some(key), "{name}");
            assert_ne!(key_label(key), "?");
        }
        assert_eq!(key_from_name("Nope"), None);
    }

    #[test]
    fn key_names_are_unique() {
        let mut names: Vec<_> = KEY_TABLE.iter().map(|(n, _, _)| *n).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), KEY_TABLE.len());
    }

    #[test]
    fn action_ids_round_trip() {
        for a in Action::ALL {
            assert_eq!(Action::from_id(a.id()), Some(a));
            assert!(!a.label().is_empty());
        }
    }

    #[test]
    fn rebinding_to_a_used_key_swaps() {
        let mut b = Bindings::default();
        assert!(b.rebind(Action::Forward, KeyCode::S));
        assert_eq!(b.key(Action::Forward), KeyCode::S);
        assert_eq!(b.key(Action::Back), KeyCode::W);
        // A free key just moves.
        assert!(b.rebind(Action::Jump, KeyCode::X));
        assert_eq!(b.key(Action::Jump), KeyCode::X);
        // Rebinding to its own key changes nothing.
        let before = b.clone();
        assert!(b.rebind(Action::Jump, KeyCode::X));
        assert_eq!(b, before);
    }

    #[test]
    fn digits_and_escape_are_refused() {
        let mut b = Bindings::default();
        for k in [KeyCode::Key1, KeyCode::Key9, KeyCode::Key0, KeyCode::Escape] {
            assert!(!is_bindable(k));
            assert!(!b.rebind(Action::Jump, k));
        }
        assert_eq!(b, Bindings::default());
    }

    #[test]
    fn names_round_trip_and_junk_is_ignored() {
        let mut b = Bindings::default();
        b.rebind(Action::Journal, KeyCode::K);
        b.rebind(Action::Sprint, KeyCode::RightShift);
        let saved = b.to_names();
        let back = Bindings::from_names(saved.iter().map(|(a, k)| (a.as_str(), k.as_str())));
        assert_eq!(back, b);

        let junk = Bindings::from_names([("bogus", "W"), ("jump", "Nope"), ("journal", "Key1")]);
        assert_eq!(junk, Bindings::default());
    }

    #[test]
    fn a_saved_file_with_a_duplicate_key_never_binds_it_twice() {
        let b = Bindings::from_names([("forward", "S"), ("back", "S")]);
        let mut seen = Vec::new();
        for a in Action::ALL {
            assert!(!seen.contains(&b.key(a)), "{:?} duplicated", b.key(a));
            seen.push(b.key(a));
        }
        assert_eq!(b.key(Action::Forward), KeyCode::S);
    }

    /// The fixed extra keys (arrows, Enter, Tab) cannot be bound to another action, so no key drives two actions.
    #[test]
    fn the_fixed_extra_keys_cannot_be_rebound() {
        let mut b = Bindings::default();
        for key in [KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right, KeyCode::Enter, KeyCode::Tab] {
            assert!(!b.rebind(Action::Back, key), "{key:?} is fixed");
            assert!(!b.rebind(Action::Journal, key), "{key:?} is fixed");
        }
        assert_eq!(b, Bindings::default(), "nothing changed");
    }
}
