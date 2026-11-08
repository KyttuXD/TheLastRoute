//! Night Watch audio. Every sound is synthesized by the shared Last Route cue renderer (`cues`, std only); this file
//! caches the rendered buffers as macroquad sounds and plays them. `cue_names` maps game events to cue lists.
#![allow(dead_code)]

// A private copy of last_route/audio/cues (tests and the two game-rules lookups removed): the shared cue renderer.
pub mod cues;
pub mod cue_names;

use crate::core::Cue;
use macroquad::audio::{load_sound_from_bytes, play_sound, set_sound_volume, PlaySoundParams, Sound};
use std::collections::HashMap;
use std::future::Future;
use std::task::{Context, Poll, Waker};

/// Pan and intensity are snapped to 0.05 like the renderer does; the key is built from the snapped integers.
pub fn snap20(v: f32) -> i32 {
    (v * 20.0).round() as i32
}

/// Cache key `name|pan|intensity`, both numbers snapped to 0.05 (pan -1..1, intensity 0..4).
pub fn cache_key(name: &str, pan: f32, intensity: f32) -> String {
    let p = snap20(pan.clamp(-1.0, 1.0));
    let i = snap20(intensity.clamp(0.0, 4.0));
    format!("{name}|{p}|{i}")
}

/// The renderer's name: Night Watch cues are `nw_<name>`.
fn render_name(name: &str) -> String {
    if name.starts_with("nw_") {
        name.to_string()
    } else {
        format!("nw_{name}")
    }
}

pub struct Sounds {
    cache: HashMap<String, Option<Sound>>,
    master: f32,
    pub mute: bool,
    silent: bool,
    playing: Vec<Sound>,
}

impl Sounds {
    pub fn new() -> Sounds {
        Sounds { cache: HashMap::new(), master: 100.0, mute: false, silent: false, playing: Vec::new() }
    }

    /// For tests and `--check`: play does nothing and nothing is rendered.
    pub fn silent() -> Sounds {
        Sounds { cache: HashMap::new(), master: 100.0, mute: false, silent: true, playing: Vec::new() }
    }

    pub fn is_silent(&self) -> bool {
        self.silent
    }

    /// Master volume in percent (0..100).
    pub fn set_master(&mut self, percent: f32) {
        self.master = percent.clamp(0.0, 100.0);
    }

    pub fn master(&self) -> f32 {
        self.master
    }

    pub fn set_mute(&mut self, m: bool) {
        self.mute = m;
    }

    /// Number of distinct rendered buffers cached so far (tests).
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    fn sound_for(&mut self, name: &str, pan: f32, intensity: f32) -> Option<Sound> {
        let key = cache_key(name, pan, intensity);
        if let Some(s) = self.cache.get(&key) {
            return s.clone();
        }
        // Render with the snapped values so the cache key and the bytes agree.
        let p = snap20(pan.clamp(-1.0, 1.0)) as f64 / 20.0;
        let i = snap20(intensity.clamp(0.0, 4.0)) as f64 / 20.0;
        let sound = cues::render(&render_name(name), p, i).and_then(|frames| load_now(&frames));
        self.cache.insert(key, sound.clone());
        sound
    }

    /// Renders lazily and plays. Never panics: an unknown cue, a failed load or missing audio output is silent.
    pub fn play(&mut self, name: &str, pan: f32, intensity: f32) {
        if self.silent || self.mute || self.master <= 0.0 {
            return;
        }
        let volume = self.master / 100.0;
        if let Some(s) = self.sound_for(name, pan, intensity) {
            play_sound(&s, PlaySoundParams { looped: false, volume });
            if self.playing.len() < 64 {
                self.playing.push(s);
            } else {
                self.playing.clear();
            }
        }
    }

    pub fn cue(&mut self, c: &Cue) {
        self.play(c.name, c.pan, c.intensity);
    }

    /// Plays every cue of a list (what the `cue_names` functions return).
    pub fn cues(&mut self, list: &[Cue]) {
        for c in list {
            self.cue(c);
        }
    }

    /// Applies the current master volume to sounds that are still around (best effort).
    pub fn refresh_volume(&mut self) {
        let v = if self.mute { 0.0 } else { self.master / 100.0 };
        for s in &self.playing {
            set_sound_volume(s, v);
        }
    }
}

impl Default for Sounds {
    fn default() -> Self {
        Sounds::new()
    }
}

/// Loads frames as a sound. On native targets the load is ready on its first poll; in a browser it is not, so
/// that path stays silent rather than blocking.
fn load_now(frames: &[[f32; 2]]) -> Option<Sound> {
    let bytes = cues::wav_bytes(frames);
    let mut fut = std::pin::pin!(load_sound_from_bytes(&bytes));
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(s)) => Some(s),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_snaps_to_five_hundredths() {
        assert_eq!(cache_key("step", 0.51, 1.0), cache_key("step", 0.52, 1.02));
        assert_ne!(cache_key("step", 0.5, 1.0), cache_key("step", 0.6, 1.0));
        assert_eq!(cache_key("door", 0.0, 1.0), "door|0|20");
    }

    #[test]
    fn cache_key_clamps() {
        assert_eq!(cache_key("a", 5.0, 9.0), cache_key("a", 1.0, 4.0));
        assert_eq!(cache_key("a", -5.0, -1.0), cache_key("a", -1.0, 0.0));
    }

    #[test]
    fn silent_sounds_never_render() {
        let mut s = Sounds::silent();
        s.play("door", 0.0, 1.0);
        s.cue(&Cue { name: "step", pan: 0.3, intensity: 1.0 });
        assert_eq!(s.cached(), 0);
    }

    #[test]
    fn master_is_clamped() {
        let mut s = Sounds::silent();
        s.set_master(250.0);
        assert_eq!(s.master(), 100.0);
        s.set_master(-3.0);
        assert_eq!(s.master(), 0.0);
    }

    #[test]
    fn render_name_prefixes_once() {
        assert_eq!(render_name("door"), "nw_door");
        assert_eq!(render_name("nw_door"), "nw_door");
    }
}
