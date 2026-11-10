//! What the menu hands to a shift: which mode and level (a night, a week, a watch), the seed it plays from (so Retry
//! replays the same shift) and, for a Custom Night, the hunters' sliders. Pure logic.
#![allow(dead_code)]

use super::data::{ModesData, NIGHTS};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Session {
    pub mode_id: String,
    pub night: i32,
    pub seed_value: u32,
    pub custom_levels: BTreeMap<String, i32>,
    pub is_custom: bool,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    /// Classic, night 1, seed 0 (a fresh seed is made when a shift reads 0), not custom.
    pub fn new() -> Session {
        Session { mode_id: "classic".to_string(), night: 1, seed_value: 0, custom_levels: BTreeMap::new(), is_custom: false }
    }

    /// night = max(1, level); classic clamps 1..=7; seed None gives a fresh one; is_custom = sliders given.
    pub fn start_mode(&mut self, mode: &str, level: i32, custom: BTreeMap<String, i32>, seed: Option<u32>) {
        self.mode_id = mode.to_string();
        self.night = level.max(1);
        if mode == "classic" {
            self.night = self.night.clamp(1, NIGHTS);
        }
        self.seed_value = seed.unwrap_or_else(Session::fresh_seed);
        self.is_custom = !custom.is_empty();
        self.custom_levels = custom;
    }

    pub fn level(&self) -> i32 {
        self.night
    }

    /// A seed from the wall clock (the only clock allowed: std SystemTime panics on wasm).
    pub fn fresh_seed() -> u32 {
        let micros = (macroquad::miniquad::date::now() * 1_000_000.0) as u64;
        (micros & 0x7fff_ffff) as u32
    }

    /// The seed to play from: the stored one, or a fresh one when it is still 0.
    pub fn seed_or_fresh(&self) -> u32 {
        if self.seed_value == 0 {
            Session::fresh_seed()
        } else {
            self.seed_value
        }
    }

    /// Same mode, level, sliders and seed.
    pub fn retry(&mut self) {
        let mode = self.mode_id.clone();
        let custom = self.custom_levels.clone();
        let (level, seed) = (self.night, self.seed_value);
        self.start_mode(&mode, level, custom, Some(seed));
    }

    /// Move to the next level (fresh seed) if there is one.
    pub fn next_level(&mut self, modes: &ModesData, classic_nights: i32) -> bool {
        if !self.has_next_with(modes, classic_nights) {
            return false;
        }
        let mode = self.mode_id.clone();
        let level = self.night + 1;
        self.start_mode(&mode, level, BTreeMap::new(), None);
        true
    }

    pub fn has_next(&self, modes: &ModesData) -> bool {
        self.has_next_with(modes, NIGHTS)
    }

    fn has_next_with(&self, modes: &ModesData, classic_nights: i32) -> bool {
        if self.is_custom {
            return false;
        }
        let scored = if self.mode_id == "classic" {
            classic_nights as usize
        } else {
            modes.get(&self.mode_id).map(|m| m.scored).unwrap_or(1)
        };
        (self.night as usize) < scored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_is_classic_night_one() {
        let s = Session::new();
        assert_eq!((s.mode_id.as_str(), s.night, s.seed_value, s.is_custom), ("classic", 1, 0, false));
        assert_eq!(s.level(), 1);
    }

    #[test]
    fn start_mode_clamps_and_flags_custom() {
        let mut s = Session::new();
        s.start_mode("classic", 99, BTreeMap::new(), Some(5));
        assert_eq!(s.night, 7);
        assert_eq!(s.seed_value, 5);
        s.start_mode("classic", -3, BTreeMap::new(), Some(5));
        assert_eq!(s.night, 1);
        s.start_mode("mask", 12, BTreeMap::new(), Some(5));
        assert_eq!(s.night, 12); // only classic is clamped
        let mut c = BTreeMap::new();
        c.insert("dovelle".to_string(), 8);
        s.start_mode("classic", 2, c, Some(9));
        assert!(s.is_custom);
        assert_eq!(s.custom_levels.len(), 1);
        s.start_mode("classic", 2, BTreeMap::new(), Some(9));
        assert!(!s.is_custom);
    }

    #[test]
    fn a_fresh_seed_is_31_bits() {
        let mut s = Session::new();
        s.start_mode("classic", 1, BTreeMap::new(), None);
        assert!(s.seed_value <= 0x7fff_ffff);
        assert!(Session::fresh_seed() <= 0x7fff_ffff);
    }

    #[test]
    fn retry_keeps_the_seed() {
        let mut s = Session::new();
        let mut c = BTreeMap::new();
        c.insert("ravelle".to_string(), 3);
        s.start_mode("classic", 3, c.clone(), Some(777));
        s.retry();
        assert_eq!((s.night, s.seed_value, s.is_custom), (3, 777, true));
        assert_eq!(s.custom_levels, c);
    }

    #[test]
    fn next_level_stops_at_the_last_scored_level() {
        let modes = ModesData::load();
        let mut s = Session::new();
        s.start_mode("classic", 6, BTreeMap::new(), Some(1));
        assert!(s.has_next(&modes));
        assert!(s.next_level(&modes, NIGHTS));
        assert_eq!(s.night, 7);
        assert!(!s.has_next(&modes));
        assert!(!s.next_level(&modes, NIGHTS));
        assert_eq!(s.night, 7);
        s.start_mode("mixed", 4, BTreeMap::new(), Some(1));
        assert!(!s.has_next(&modes)); // Endless is not scored
        s.start_mode("extra", 2, BTreeMap::new(), Some(1));
        assert!(s.has_next(&modes));
    }

    #[test]
    fn custom_nights_have_no_next() {
        let modes = ModesData::load();
        let mut s = Session::new();
        let mut c = BTreeMap::new();
        c.insert("dovelle".to_string(), 8);
        s.start_mode("classic", 1, c, Some(1));
        assert!(!s.has_next(&modes));
    }
}
