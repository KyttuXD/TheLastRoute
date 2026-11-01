//! The playing side: what the original's Sound autoload does with a rendered cue (sound.gd 725-785). Renders are cached
//! by the original's cache key (`cue|pan|intensity`, pan and intensity snapped to 0.05), so every play of a key is the
//! same bytes, like the original. A play's volume is the player's gain (1 for `_play_named`, the loudness for
//! `_play_scaled`) times the bus: Master x SFX, or Master x UI for the menu blip.
//!
//! macroquad's `play_sound` multiplies the samples by `volume` with no clamp, so gains above 1 (hunter steps up to 1.8)
//! play as loud as in the original.

#![allow(dead_code)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::task::{Context, Poll, Waker};

use macroquad::audio::{
    load_sound_from_bytes, play_sound, set_sound_volume, stop_sound, PlaySoundParams, Sound,
};

use super::cues::{
    self,
    rules::{Bus, Play, SoundFront},
};

pub(crate) struct LoopVoice {
    /// The loop's left channel only and its right channel only, started together, so the stereo gains of a
    /// positional source can be set per channel (macroquad has volume per sound, no pan).
    left: Sound,
    right: Sound,
    gains: (f32, f32),
}

pub(crate) struct Engine {
    pub ready: bool,
    cache: HashMap<String, Sound>,
    pub front: SoundFront,
    pub master: f32,
    pub sfx: f32,
    pub ui: f32,
    /// (play at, cue): delayed cues (the tool impact, Nyxie's second breath).
    pending: Vec<(f64, Play)>,
    loops: HashMap<&'static str, LoopVoice>,
    /// Stim crash heartbeat countdown (player_controller_3d.gd `_heartbeat_in`), and whether the crash was on.
    pub stim_heartbeat_in: f32,
    pub stim_crashing: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Engine {
            ready: false,
            cache: HashMap::new(),
            front: SoundFront::new(),
            master: 1.0,
            sfx: 1.0,
            ui: 1.0,
            pending: vec![],
            loops: HashMap::new(),
            stim_heartbeat_in: 0.0,
            stim_crashing: false,
        }
    }
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::new(Engine::default());
}

pub(crate) fn with<R>(f: impl FnOnce(&mut Engine) -> R) -> R {
    ENGINE.with(|e| f(&mut e.borrow_mut()))
}

/// `load_sound_from_bytes` finishes on its first poll on every native target (only wasm waits a frame), so a cue can be
/// rendered and loaded the moment the game first asks for it, like the original's `render_cue`.
pub(crate) fn load_frames(frames: &[[f32; 2]]) -> Option<Sound> {
    let bytes = cues::wav_bytes(frames);
    let mut fut = std::pin::pin!(load_sound_from_bytes(&bytes));
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(s)) => Some(s),
        _ => None,
    }
}

/// Starts the engine and renders the cues the original prewarms (sound.gd 637-646) plus the other common ones.
pub(crate) fn init() {
    with(|e| {
        e.ready = true;
        let mut warm: Vec<(String, f64, f64)> = vec![];
        for gait in ["walk", "sprint", "crouch"] {
            warm.push((format!("footstep_{gait}"), 0.045, 1.0));
            warm.push((format!("footstep_{gait}"), -0.045, 1.0));
        }
        for v in 0..4 {
            warm.push((
                format!("step:crouch:metal:{v}"),
                if v % 2 == 1 { 0.045 } else { -0.045 },
                1.0,
            ));
        }
        for i in 1..=20 {
            warm.push(("heartbeat".into(), 0.0, i as f64 * 0.05));
        }
        for k in -4..=4 {
            warm.push(("hunter_footstep".into(), k as f64 * 0.25, 1.0));
        }
        for c in [
            "menu",
            "collect",
            "interaction_fail",
            "chase_start",
            "capture",
            "taser",
            "axe_swing",
            "axe_chop",
            "door_breach",
        ] {
            warm.push((c.into(), 0.0, 1.0));
        }
        for (cue, pan, intensity) in warm {
            e.sound_for(&cue, pan, intensity);
        }
    });
}

impl Engine {
    pub fn now(&self) -> f64 {
        macroquad::time::get_time()
    }

    pub fn now_usec(&self) -> i64 {
        (self.now() * 1_000_000.0) as i64
    }

    fn sound_for(&mut self, cue: &str, pan: f64, intensity: f64) -> Option<Sound> {
        let key = cues::cache_key(cue, pan, intensity);
        if let Some(s) = self.cache.get(&key) {
            return Some(s.clone());
        }
        // an unknown name plays the original's fallback beep (sound.gd 1080)
        let frames = cues::render(cue, pan, intensity).unwrap_or_else(cues::fallback_beep);
        let s = load_frames(&frames)?;
        self.cache.insert(key, s.clone());
        Some(s)
    }

    pub fn bus_gain(&self, bus: Bus) -> f32 {
        self.master * if bus == Bus::Ui { self.ui } else { self.sfx }
    }

    /// Plays one cue now.
    pub fn play(&mut self, p: &Play) {
        if !self.ready {
            return;
        }
        let volume = p.gain * self.bus_gain(p.bus);
        if volume <= 0.0 {
            return;
        }
        if let Some(s) = self.sound_for(&p.cue, p.pan, p.intensity) {
            play_sound(
                &s,
                PlaySoundParams {
                    looped: false,
                    volume,
                },
            );
        }
    }

    /// Plays `p` after `delay` seconds (a GDScript `await create_timer(delay)`); `pump` releases it.
    pub fn schedule(&mut self, delay: f64, p: Play) {
        if self.ready {
            let at = self.now() + delay;
            self.pending.push((at, p));
        }
    }

    pub fn pump(&mut self) {
        if !self.ready || self.pending.is_empty() {
            return;
        }
        let now = self.now();
        let due: Vec<Play> = {
            let (due, keep): (Vec<_>, Vec<_>) =
                self.pending.drain(..).partition(|(at, _)| *at <= now);
            self.pending = keep;
            due.into_iter().map(|(_, p)| p).collect()
        };
        for p in due {
            self.play(&p);
        }
    }

    /// Keeps the looping `name` playing at stereo gains `gains` (already including the bus), or stops it (`None`).
    /// `frames` renders the loop the first time.
    pub fn set_loop(
        &mut self,
        name: &'static str,
        gains: Option<(f32, f32)>,
        frames: impl FnOnce() -> Vec<[f32; 2]>,
    ) {
        if !self.ready {
            return;
        }
        match gains {
            None => {
                if let Some(v) = self.loops.remove(name) {
                    stop_sound(&v.left);
                    stop_sound(&v.right);
                }
            }
            Some(g) => {
                if let Some(v) = self.loops.get_mut(name) {
                    if (v.gains.0 - g.0).abs() > 1e-4 || (v.gains.1 - g.1).abs() > 1e-4 {
                        set_sound_volume(&v.left, g.0);
                        set_sound_volume(&v.right, g.1);
                        v.gains = g;
                    }
                    return;
                }
                let f = frames();
                let left: Vec<[f32; 2]> = f.iter().map(|x| [x[0], 0.0]).collect();
                let right: Vec<[f32; 2]> = f.iter().map(|x| [0.0, x[1]]).collect();
                let (Some(l), Some(r)) = (load_frames(&left), load_frames(&right)) else {
                    return;
                };
                play_sound(
                    &l,
                    PlaySoundParams {
                        looped: true,
                        volume: g.0,
                    },
                );
                play_sound(
                    &r,
                    PlaySoundParams {
                        looped: true,
                        volume: g.1,
                    },
                );
                self.loops.insert(
                    name,
                    LoopVoice {
                        left: l,
                        right: r,
                        gains: g,
                    },
                );
            }
        }
    }

    pub fn stop_all(&mut self) {
        self.pending.clear();
        let names: Vec<&'static str> = self.loops.keys().copied().collect();
        for n in names {
            self.set_loop(n, None, Vec::new);
        }
        self.stim_crashing = false;
    }
}
