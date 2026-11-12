//! The playing side: what the original's Sound autoload does with a rendered cue (sound.gd 725-785). Renders are cached
//! by the original's cache key (`cue|pan|intensity`, pan and intensity snapped to 0.05), so every play of a key is the
//! same bytes, like the original. A play's volume is the player's gain (1 for `_play_named`, the loudness for
//! `_play_scaled`) times the bus: MASTER x its slider (data/audio/buses.json says which bus a cue plays on).
//!
//! Every gain goes through `Engine::bus_gain` (rules::bus_gain underneath). A one-shot takes it when it starts. A loop
//! keeps its base gains (the caller's positional attenuation, the tune's gain) and gets the bus gain applied here, so a
//! bus slider moved while the loop plays (`refresh_loops`, from `set_volumes`) reaches it at once, whatever the game is
//! doing (pause menu, report screen).
//!
//! macroquad's `play_sound` multiplies the samples by `volume` with no clamp, so gains above 1 (hunter steps up to 1.8)
//! play as loud as in the original.

#![allow(dead_code)]
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use macroquad::audio::{
    load_sound_from_bytes, play_sound, set_sound_volume, stop_sound, PlaySoundParams, Sound,
};
use macroquad::experimental::coroutines::start_coroutine;

use super::cues::{
    self,
    rules::{self, Bus, Play, SoundFront},
};

/// Silence after a timed loop's buffer (see `Engine::play_frames`): a stop that lands a frame late is inaudible.
const TIMED_LOOP_TAIL_SECONDS: f32 = 0.5;

/// True on the browser build. There the browser decodes a sound and it is ready some frames after it is handed over, so
/// a cue or loop that is not loaded yet loads in the background (a coroutine) and plays from the next call on.
const WEB: bool = cfg!(target_arch = "wasm32");

/// How long the start waits for the browser build's pre-loads. They go on in the background after that anyway.
const WEB_PRELOAD_WAIT_SECONDS: f64 = 20.0;

/// Set once the first cue that was not pre-loaded is logged (a miss is logged once, not per play).
static MISS_LOGGED: AtomicBool = AtomicBool::new(false);

/// The stereo volumes of a loop: the base gains the caller computed (before the bus), and the volumes the two channel
/// sounds were last set to (the base times the bus gain). Pure, so the rule that a loop follows its bus is testable
/// without an audio device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LoopGains {
    pub base: (f32, f32),
    pub applied: (f32, f32),
}

fn scaled(base: (f32, f32), gain: f32) -> (f32, f32) {
    (base.0 * gain, base.1 * gain)
}

impl LoopGains {
    /// A loop at `base` on a bus whose gain is `bus_gain`.
    pub fn new(base: (f32, f32), bus_gain: f32) -> Self {
        LoopGains { base, applied: scaled(base, bus_gain) }
    }

    /// Moves the loop to `base` at `bus_gain`. Returns the volumes to set on the two channel sounds when they changed,
    /// or `None` when nothing moved.
    pub fn retarget(&mut self, base: (f32, f32), bus_gain: f32) -> Option<(f32, f32)> {
        self.base = base;
        let next = scaled(base, bus_gain);
        if (self.applied.0 - next.0).abs() > 1e-4 || (self.applied.1 - next.1).abs() > 1e-4 {
            self.applied = next;
            return Some(next);
        }
        None
    }
}

pub(crate) struct LoopVoice {
    /// The loop's left channel only and its right channel only, started together, so the stereo gains of a
    /// positional source can be set per channel (macroquad has volume per sound, no pan).
    left: Sound,
    right: Sound,
    bus: Bus,
    gains: LoopGains,
    /// Wall-clock time (`now()`) at which the loop stops by itself; `None` for an open-ended loop.
    until: Option<f64>,
}

/// A loop the browser build is still loading (its channels are not decoded yet). `token` tells the load that finishes
/// whether it is still the loop that was asked for (a stop, or a stop and a new start, cancels it).
struct PendingLoop {
    token: u64,
    bus: Bus,
    base: (f32, f32),
    /// For a timed loop, how long it plays once it starts. The end is counted from the moment the channels are decoded and
    /// the loop really starts, not from the request, so a slow decode does not cut the stretch short.
    seconds: Option<f64>,
}

pub(crate) struct Engine {
    pub ready: bool,
    cache: HashMap<String, Sound>,
    pub front: SoundFront,
    /// The settings' volume sliders (rules::Volumes); `bus_gain` is the only reader.
    pub vol: rules::Volumes,
    /// (play at, cue): delayed cues (the tool impact, Nyxie's second breath).
    pending: Vec<(f64, Play)>,
    loops: HashMap<&'static str, LoopVoice>,
    /// Cue keys loading in the background (browser build): each is started once.
    inflight: HashSet<String>,
    loops_pending: HashMap<&'static str, PendingLoop>,
    next_token: u64,
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
            vol: rules::Volumes::default(),
            pending: vec![],
            loops: HashMap::new(),
            inflight: HashSet::new(),
            loops_pending: HashMap::new(),
            next_token: 0,
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

/// Loads `frames` as a sound. Native: the load is ready at once. Browser: it resolves once the browser has decoded the
/// samples, a few frames later, so the callers await it.
pub(crate) async fn load_frames(frames: &[[f32; 2]]) -> Option<Sound> {
    let bytes = cues::wav_bytes(frames);
    load_sound_from_bytes(&bytes).await.ok()
}

/// [`load_frames`] for the synchronous native paths. `load_sound_from_bytes` finishes on its first poll on native targets,
/// so a cue can be rendered and loaded the moment the game first asks for it, like the original's `render_cue`. (On the
/// browser it is never ready on the first poll, so the browser build does not take this path.)
pub(crate) fn load_frames_now(frames: &[[f32; 2]]) -> Option<Sound> {
    let bytes = cues::wav_bytes(frames);
    let mut fut = std::pin::pin!(load_sound_from_bytes(&bytes));
    let waker = crate::shared::task::noop_waker();
    let mut cx = Context::from_waker(&waker);
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(s)) => Some(s),
        _ => None,
    }
}

/// The cues the original prewarms (sound.gd 637-646) plus the other common ones, as (cue, pan, intensity).
fn warm_list() -> Vec<(String, f64, f64)> {
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
    warm
}

/// Every cue the game can play on the browser build, as the (cue, pan, intensity) keys it plays them with: each name the
/// cue set knows (Night Watch's `nw_` cues aside), at the variants a step or a cue takes (`default_call`). Loading these
/// up front is what lets `sound_for` answer from the cache on the web.
fn web_cues() -> Vec<(String, f64, f64)> {
    let mut out = vec![];
    for name in cues::cue_names().iter().filter(|n| !n.starts_with("nw_")) {
        for variant in 0..4 {
            out.extend(cues::default_call(name, variant));
        }
    }
    out
}

/// Starts the engine and loads the cues the original prewarms. The browser build also loads every cue the game can play
/// (`web_cues`) and waits for them, up to `WEB_PRELOAD_WAIT_SECONDS`; the rest keep loading in the background.
pub(crate) async fn init() {
    with(|e| e.ready = true);
    if !WEB {
        with(|e| {
            for (cue, pan, intensity) in warm_list() {
                e.sound_for(&cue, pan, intensity);
            }
        });
        return;
    }
    with(|e| {
        for (cue, pan, intensity) in warm_list().into_iter().chain(web_cues()) {
            e.preload(&cue, pan, intensity);
        }
    });
    let deadline = macroquad::time::get_time() + WEB_PRELOAD_WAIT_SECONDS;
    while macroquad::time::get_time() < deadline && with(|e| !e.inflight.is_empty()) {
        macroquad::window::next_frame().await;
    }
}

impl Engine {
    pub fn now(&self) -> f64 {
        macroquad::time::get_time()
    }

    pub fn now_usec(&self) -> i64 {
        (self.now() * 1_000_000.0) as i64
    }

    /// The sound for a cue at a pan and intensity, from the cache. A miss on the native build renders and loads it now.
    /// On the browser build a miss starts loading it in the background and returns None: the play is skipped, and the
    /// cue is there from its next call (the start pre-loads every cue the game plays, so this is rare).
    fn sound_for(&mut self, cue: &str, pan: f64, intensity: f64) -> Option<Sound> {
        let key = cues::cache_key(cue, pan, intensity);
        if let Some(s) = self.cache.get(&key) {
            return Some(s.clone());
        }
        if WEB {
            if !MISS_LOGGED.swap(true, Ordering::Relaxed) {
                eprintln!("audio: {key} was not pre-loaded; it loads in the background and plays from its next call");
            }
            self.spawn_cue(key, cue, pan, intensity);
            return None;
        }
        // an unknown name plays the original's fallback beep (sound.gd 1080)
        let frames = cues::render(cue, pan, intensity).unwrap_or_else(cues::fallback_beep);
        let s = load_frames_now(&frames)?;
        self.cache.insert(key, s.clone());
        Some(s)
    }

    /// Browser start: a cue loads ahead of the game asking for it.
    fn preload(&mut self, cue: &str, pan: f64, intensity: f64) {
        let key = cues::cache_key(cue, pan, intensity);
        self.spawn_cue(key, cue, pan, intensity);
    }

    /// Starts loading one cue in the background (browser build): a coroutine renders it, decodes it and puts it in the
    /// cache, and the key leaves `inflight` then. Nothing happens for a key that is cached or already loading.
    fn spawn_cue(&mut self, key: String, cue: &str, pan: f64, intensity: f64) {
        if self.cache.contains_key(&key) || !self.inflight.insert(key.clone()) {
            return;
        }
        let cue = cue.to_string();
        let _ = start_coroutine(async move {
            let frames = cues::render(&cue, pan, intensity).unwrap_or_else(cues::fallback_beep);
            let loaded = load_frames(&frames).await;
            with(|e| {
                e.inflight.remove(&key);
                if let Some(s) = loaded {
                    e.cache.insert(key, s);
                }
            });
        });
    }

    /// The gain of `bus` now: MASTER x its slider (x GAMEPLAY for the buses under it).
    pub fn bus_gain(&self, bus: Bus) -> f32 {
        rules::bus_gain(bus, &self.vol)
    }

    /// The volume a one-shot plays at: its player gain times its bus, read when it starts. A one-shot already playing
    /// keeps that volume (only loops and the timed wind-down follow a slider while they play).
    pub fn play_volume(&self, p: &Play) -> f32 {
        p.gain * self.bus_gain(p.bus)
    }

    /// Plays one cue now.
    pub fn play(&mut self, p: &Play) {
        if !self.ready {
            return;
        }
        let volume = self.play_volume(p);
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

    /// Plays `p` after `delay` seconds (a GDScript `await create_timer(delay)`); `tick` releases it.
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

    /// Once per frame, whatever the game is doing (paused, on the report screen, loading): releases the delayed cues
    /// that are due and ends the timed loops that are over.
    pub fn tick(&mut self) {
        if !self.ready {
            return;
        }
        self.pump();
        let now = self.now();
        let ended: Vec<&'static str> = self
            .loops
            .iter()
            .filter(|(_, v)| v.until.is_some_and(|t| now >= t))
            .map(|(n, _)| *n)
            .collect();
        for n in ended {
            self.stop_loop(n);
        }
    }

    /// Plays `frames` once as a looped voice called `name`, on `bus` at the stereo `base` gains, and stops it `seconds`
    /// from now. This is the music box's wind-down: one long pre-rendered stretch that must follow Master while it plays
    /// and stop with `stop_all`. It loops, so the mixer's one-shot pool never cuts it, and it ends in silence, so a stop
    /// that lands a frame late is not heard as the start of the stretch again.
    pub fn play_frames(
        &mut self,
        name: &'static str,
        bus: Bus,
        mut frames: Vec<[f32; 2]>,
        base: (f32, f32),
        seconds: f32,
    ) {
        if !self.ready {
            return;
        }
        self.stop_loop(name);
        let tail = (TIMED_LOOP_TAIL_SECONDS * cues::SAMPLE_RATE as f32) as usize;
        frames.extend(std::iter::repeat([0.0, 0.0]).take(tail));
        self.start_loop(name, bus, base, frames, Some(f64::from(seconds.max(0.0))));
    }

    /// Keeps the looping `name` playing at the stereo `base` gains (before the bus), on `bus`, or stops it (`None`).
    /// `frames` renders the loop the first time it starts. The bus gain is applied here and by `refresh_loops`.
    pub fn set_loop(
        &mut self,
        name: &'static str,
        bus: Bus,
        base: Option<(f32, f32)>,
        frames: impl FnOnce() -> Vec<[f32; 2]>,
    ) {
        if !self.ready {
            return;
        }
        let Some(base) = base else {
            self.stop_loop(name);
            return;
        };
        let gain = self.bus_gain(bus);
        if let Some(v) = self.loops.get_mut(name) {
            v.bus = bus;
            if let Some((l, r)) = v.gains.retarget(base, gain) {
                set_sound_volume(&v.left, l);
                set_sound_volume(&v.right, r);
            }
            return;
        }
        // still loading (browser build): it starts with the latest bus and base, once its channels are decoded
        if let Some(p) = self.loops_pending.get_mut(name) {
            p.bus = bus;
            p.base = base;
            return;
        }
        self.start_loop(name, bus, base, frames(), None);
    }

    /// Re-applies every bus gain to the loops that play (called when a bus slider moves), so a playing loop follows Master
    /// at once and does not wait for its caller's next frame.
    pub fn refresh_loops(&mut self) {
        let vol = self.vol;
        for v in self.loops.values_mut() {
            let gain = rules::bus_gain(v.bus, &vol);
            let base = v.gains.base;
            if let Some((l, r)) = v.gains.retarget(base, gain) {
                set_sound_volume(&v.left, l);
                set_sound_volume(&v.right, r);
            }
        }
    }

    fn start_loop(
        &mut self,
        name: &'static str,
        bus: Bus,
        base: (f32, f32),
        frames: Vec<[f32; 2]>,
        seconds: Option<f64>,
    ) {
        let left: Vec<[f32; 2]> = frames.iter().map(|x| [x[0], 0.0]).collect();
        let right: Vec<[f32; 2]> = frames.iter().map(|x| [0.0, x[1]]).collect();
        drop(frames);
        if WEB {
            // the browser decodes both channels first: they start when they are ready, unless the loop is stopped meanwhile
            self.next_token += 1;
            let token = self.next_token;
            self.loops_pending.insert(name, PendingLoop { token, bus, base, seconds });
            let _ = start_coroutine(async move {
                let l = load_frames(&left).await;
                let r = load_frames(&right).await;
                with(|e| {
                    if !matches!(e.loops_pending.get(name), Some(p) if p.token == token) {
                        return;
                    }
                    let Some(p) = e.loops_pending.remove(name) else { return };
                    let (Some(l), Some(r)) = (l, r) else { return };
                    let until = p.seconds.map(|s| e.now() + s);
                    e.play_loop(name, p.bus, p.base, l, r, until);
                });
            });
            return;
        }
        let (Some(l), Some(r)) = (load_frames_now(&left), load_frames_now(&right)) else {
            return;
        };
        let until = seconds.map(|s| self.now() + s);
        self.play_loop(name, bus, base, l, r, until);
    }

    /// Starts the two loaded channels of a loop together and keeps them.
    fn play_loop(&mut self, name: &'static str, bus: Bus, base: (f32, f32), l: Sound, r: Sound, until: Option<f64>) {
        let gains = LoopGains::new(base, self.bus_gain(bus));
        play_sound(
            &l,
            PlaySoundParams {
                looped: true,
                volume: gains.applied.0,
            },
        );
        play_sound(
            &r,
            PlaySoundParams {
                looped: true,
                volume: gains.applied.1,
            },
        );
        // the voices are kept here: dropping a Sound deletes it in the mixer, which would cut the loop off
        self.loops.insert(
            name,
            LoopVoice {
                left: l,
                right: r,
                bus,
                gains,
                until,
            },
        );
    }

    fn stop_loop(&mut self, name: &'static str) {
        self.loops_pending.remove(name);
        if let Some(v) = self.loops.remove(name) {
            stop_sound(&v.left);
            stop_sound(&v.right);
        }
    }

    /// Stops everything: the queued cues, every one-shot still playing (the mixer stops by sound, so each cached cue's
    /// instances go) and every loop, the timed wind-down included.
    pub fn stop_all(&mut self) {
        self.pending.clear();
        self.loops_pending.clear();
        for s in self.cache.values() {
            stop_sound(s);
        }
        let names: Vec<&'static str> = self.loops.keys().copied().collect();
        for n in names {
            self.stop_loop(n);
        }
        self.stim_crashing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
    }

    #[test]
    fn a_loop_follows_a_master_change_through_its_bus() {
        // the music box tune: base gain on the Music bus, started at master 1
        let mut e = Engine::default();
        let mut g = LoopGains::new((0.8, 0.8), e.bus_gain(Bus::Music));
        assert!(near(g.applied, (0.8, 0.8)));
        e.vol.master = 0.5;
        let base = g.base;
        let gain = e.bus_gain(Bus::Music);
        assert!(near(g.retarget(base, gain).unwrap(), (0.4, 0.4)), "half master halves the loop");
        assert_eq!(g.retarget(base, gain), None, "nothing to set when nothing moved");
        e.vol.master = 0.0;
        assert!(near(g.retarget(base, e.bus_gain(Bus::Music)).unwrap(), (0.0, 0.0)), "master 0 silences it");
    }

    #[test]
    fn a_positional_loop_keeps_its_stereo_balance_when_master_moves() {
        // the hum and the falls: left and right differ, and both scale with master
        let mut e = Engine::default();
        let mut g = LoopGains::new((0.9, 0.3), e.bus_gain(Bus::Ambience));
        e.vol.master = 0.25;
        let base = g.base;
        assert!(near(g.retarget(base, e.bus_gain(Bus::Ambience)).unwrap(), (0.225, 0.075)));
    }

    #[test]
    fn every_bus_follows_master_and_its_own_slider() {
        let mut e = Engine::default();
        e.vol = rules::Volumes { master: 0.5, gameplay: 0.5, hunters: 0.5, ambience: 0.4, music: 0.2, ui: 0.2 };
        assert!((e.bus_gain(Bus::Sfx) - 0.25).abs() < 1e-6);
        assert!((e.bus_gain(Bus::Hunters) - 0.125).abs() < 1e-6);
        assert!((e.bus_gain(Bus::Ambience) - 0.1).abs() < 1e-6);
        assert!((e.bus_gain(Bus::Music) - 0.05).abs() < 1e-6);
        assert!((e.bus_gain(Bus::Ui) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn a_one_shot_cue_scales_with_master_and_its_bus() {
        let mut e = Engine::default();
        let cue = rules::collect();
        assert_eq!(cue.bus, Bus::Sfx);
        e.vol.master = 0.5;
        e.vol.gameplay = 0.8;
        assert!((e.play_volume(&cue) - cue.gain * 0.4).abs() < 1e-6);
        e.vol.master = 0.25;
        assert!((e.play_volume(&cue) - cue.gain * 0.2).abs() < 1e-6, "halving master again halves the cue");
        let blip = SoundFront::new().menu_blip(0).unwrap();
        assert_eq!(blip.bus, Bus::Ui);
        e.vol.ui = 0.5;
        assert!((e.play_volume(&blip) - blip.gain * 0.125).abs() < 1e-6, "the UI cue uses the UI slider, under master");
    }
}
