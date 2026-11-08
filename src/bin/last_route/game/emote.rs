//! The local player's emote state: the selection wheel (held key), the emote that is playing, and the rules that end it.
//! The tunables (key, durations, which actions cancel) are in `data/player/emotes.json`
//! (`render/anim/emote_pose.rs`); this file only holds behaviour.
//!
//! Flow: the key goes down -> [`EmoteState::open`]; the pointer or stick moves -> [`EmoteState::set_hover`]; the key
//! comes up -> [`EmoteState::release`] plays the hovered emote (nothing if none is hovered); Esc ->
//! [`EmoteState::cancel_wheel`]. Every game step calls [`EmoteState::update`], which ends an emote that ran out
//! or that moving or sprinting cancels. The sample carries [`EmoteState::wire`] so other players see it.

use crate::render::anim::emote_pose::EmoteSet;
use macroquad::prelude::KeyCode;

/// The emote that is playing.
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    pub id: String,
    /// The game clock when it began.
    pub start: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EmoteState {
    open: bool,
    hover: Option<usize>,
    active: Option<Active>,
    /// Counts the emotes started, so a peer can tell the same emote played twice in a row from one still playing.
    seq: u32,
}

impl EmoteState {
    /// Whether the wheel is showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The slice under the pointer or stick, while the wheel is open.
    pub fn hover(&self) -> Option<usize> {
        if self.open {
            self.hover
        } else {
            None
        }
    }

    pub fn active(&self) -> Option<&Active> {
        self.active.as_ref()
    }

    /// The count of emotes started so far (the wire's start sequence number).
    pub fn seq(&self) -> u32 {
        self.seq
    }

    /// Opens the wheel with nothing picked.
    pub fn open(&mut self) {
        self.open = true;
        self.hover = None;
    }

    /// Sets the slice under the pointer or stick (None for the dead zone). Ignored while the wheel is shut.
    pub fn set_hover(&mut self, hover: Option<usize>) {
        if self.open {
            self.hover = hover;
        }
    }

    /// Esc: shuts the wheel and plays nothing.
    pub fn cancel_wheel(&mut self) {
        self.open = false;
        self.hover = None;
    }

    /// Whether moving or sprinting stops (or refuses) an emote under `set`.
    pub fn blocked(set: &EmoteSet, moving: bool, sprinting: bool) -> bool {
        (set.cancel_on_move && moving) || (set.cancel_on_sprint && sprinting)
    }

    /// The key came up: shuts the wheel and plays the hovered emote. Returns its id when one started. Nothing starts
    /// while the player is moving or sprinting, or when no slice is hovered.
    pub fn release(&mut self, set: &EmoteSet, now: f32, moving: bool, sprinting: bool) -> Option<String> {
        let pick = self.hover();
        self.cancel_wheel();
        let id = set.emotes.get(pick?)?.id.clone();
        if Self::blocked(set, moving, sprinting) || !self.play(set, &id, now) {
            return None;
        }
        Some(id)
    }

    /// Starts emote `id` now, replacing any playing. False for an id `set` does not hold.
    pub fn play(&mut self, set: &EmoteSet, id: &str, now: f32) -> bool {
        if set.find(id).is_none() {
            return false;
        }
        self.seq = self.seq.wrapping_add(1);
        self.active = Some(Active { id: id.to_string(), start: now });
        true
    }

    /// Ends the playing emote.
    pub fn stop(&mut self) {
        self.active = None;
    }

    /// One game step: ends an emote that has run its length or that moving / sprinting cancels. Opening the wheel
    /// while moving is allowed (the pick just will not start).
    pub fn update(&mut self, set: &EmoteSet, now: f32, moving: bool, sprinting: bool) {
        let Some(a) = &self.active else {
            return;
        };
        let over = set.find(&a.id).map_or(true, |e| now - a.start >= e.duration);
        // a clock that went backwards (a new run) ends it too
        if over || now < a.start || Self::blocked(set, moving, sprinting) {
            self.active = None;
        }
    }

    /// What the local sample sends: the playing emote's id and its start sequence number.
    pub fn wire(&self) -> Option<(&str, u32)> {
        self.active.as_ref().map(|a| (a.id.as_str(), self.seq))
    }

    /// The playing emote's id and the seconds since it began, for posing the local body in a third-person view.
    pub fn local_pose(&self, now: f32) -> Option<(&str, f32)> {
        self.active.as_ref().map(|a| (a.id.as_str(), (now - a.start).max(0.0)))
    }
}

/// A key from its name in emotes.json ("G", "B", "F5", "TAB" ...). None for a name this does not know.
pub fn key_from_name(name: &str) -> Option<KeyCode> {
    let n = name.trim().to_ascii_uppercase();
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_uppercase() {
            const LETTERS: [KeyCode; 26] = [
                KeyCode::A, KeyCode::B, KeyCode::C, KeyCode::D, KeyCode::E, KeyCode::F, KeyCode::G, KeyCode::H, KeyCode::I,
                KeyCode::J, KeyCode::K, KeyCode::L, KeyCode::M, KeyCode::N, KeyCode::O, KeyCode::P, KeyCode::Q, KeyCode::R,
                KeyCode::S, KeyCode::T, KeyCode::U, KeyCode::V, KeyCode::W, KeyCode::X, KeyCode::Y, KeyCode::Z,
            ];
            return Some(LETTERS[(c - b'A') as usize]);
        }
    }
    match n.as_str() {
        "TAB" => Some(KeyCode::Tab),
        "CAPSLOCK" => Some(KeyCode::CapsLock),
        "F1" => Some(KeyCode::F1),
        "F2" => Some(KeyCode::F2),
        "F3" => Some(KeyCode::F3),
        "F4" => Some(KeyCode::F4),
        "F5" => Some(KeyCode::F5),
        "F6" => Some(KeyCode::F6),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::embedded;
    use crate::render::anim::emote_pose::FILE;

    fn set() -> EmoteSet {
        let (_, src) = embedded::data_under(FILE).next().expect("player/emotes.json is embedded");
        EmoteSet::parse(src).expect("emotes.json parses")
    }

    #[test]
    fn hold_pick_release_plays_the_hovered_emote() {
        let s = set();
        let mut e = EmoteState::default();
        e.set_hover(Some(1));
        assert_eq!(e.hover(), None, "a shut wheel takes no hover");
        e.open();
        assert!(e.is_open());
        e.set_hover(Some(1));
        assert_eq!(e.release(&s, 10.0, false, false).as_deref(), Some(s.emotes[1].id.as_str()));
        assert!(!e.is_open());
        assert_eq!(e.wire(), Some((s.emotes[1].id.as_str(), 1)));
        assert_eq!(e.local_pose(10.5).map(|p| p.1), Some(0.5));
    }

    #[test]
    fn release_with_no_pick_or_esc_plays_nothing() {
        let s = set();
        let mut e = EmoteState::default();
        e.open();
        assert_eq!(e.release(&s, 0.0, false, false), None);
        e.open();
        e.set_hover(Some(0));
        e.cancel_wheel();
        assert_eq!(e.release(&s, 0.0, false, false), None);
        assert_eq!(e.wire(), None);
        e.open();
        e.set_hover(Some(99));
        assert_eq!(e.release(&s, 0.0, false, false), None, "a slice that does not exist plays nothing");
    }

    #[test]
    fn moving_or_sprinting_cancels_by_data() {
        let mut s = set();
        let mut e = EmoteState::default();
        assert!(e.play(&s, "wave", 0.0));
        e.update(&s, 0.5, false, false);
        assert!(e.active().is_some());
        e.update(&s, 0.6, true, false);
        assert!(e.active().is_none());
        // the same step with the rule off in the data changes behaviour
        s.cancel_on_move = false;
        assert!(e.play(&s, "wave", 1.0));
        e.update(&s, 1.1, true, false);
        assert!(e.active().is_some());
        e.update(&s, 1.2, false, true);
        assert!(e.active().is_none());
    }

    #[test]
    fn moving_blocks_a_release_pick() {
        let s = set();
        let mut e = EmoteState::default();
        e.open();
        e.set_hover(Some(0));
        assert_eq!(e.release(&s, 0.0, true, false), None);
        assert_eq!(e.wire(), None);
    }

    #[test]
    fn an_emote_ends_after_its_duration_and_a_replay_bumps_the_sequence() {
        let s = set();
        let mut e = EmoteState::default();
        let len = s.find("no").unwrap().duration;
        assert!(e.play(&s, "no", 5.0));
        e.update(&s, 5.0 + len - 0.01, false, false);
        assert!(e.active().is_some());
        e.update(&s, 5.0 + len + 0.01, false, false);
        assert!(e.active().is_none());
        assert!(e.play(&s, "no", 20.0));
        assert!(e.play(&s, "no", 21.0));
        assert_eq!(e.seq(), 3);
    }

    #[test]
    fn an_unknown_id_does_not_play() {
        let s = set();
        let mut e = EmoteState::default();
        assert!(!e.play(&s, "no-such-emote", 0.0));
        assert_eq!(e.seq(), 0);
        assert_eq!(e.wire(), None);
    }

    #[test]
    fn the_shipped_key_name_maps_to_a_key() {
        assert_eq!(key_from_name(&set().key), Some(KeyCode::B));
        assert_eq!(key_from_name("z"), Some(KeyCode::Z));
        assert_eq!(key_from_name("nonsense"), None);
    }
}
