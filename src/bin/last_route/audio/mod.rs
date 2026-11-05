//! Sound. The original ships no sound files: every cue is oscillators, pitch-gliding thumps and filtered noise rendered
//! at runtime (game/autoload/sound.gd). `cues` renders the same 44.1 kHz samples (checked against the original's audio
//! goldens AND against Godot 4.7.2's real mixed output: see cues/NOTES.md "Measured against Godot").
//!
//! The game talks to sound the way the original talks to its `Sound` autoload:
//! - `sound::*`: one function per `Sound.<method>()` (collect, recharge, footstep, menu_blip, ...), with the original's
//!   throttles, pan / intensity renders (cached by the original's cache key) and player gains;
//! - `world::update(dt, &listener, &hunters)` once per game frame: main_3d's `_update_local_audio` (heartbeat by danger,
//!   chase_start, Nyxie's wake cue, hunter footsteps on the nearest hunter's real footfall cadence, panned) and Glyra's
//!   positional hum loop;
//! - `set_volumes(master, sfx, ui)`: the Master / SFX / UI buses (linear, like settings.gd `linear_to_db(pct / 100)`).
//!
//! `Sounds` and `play` are the old fixed bank, kept only so call sites not migrated yet still build.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub mod creature;
pub mod cues;
mod engine;
pub mod falls;
pub mod sound;
pub mod tts;
pub mod world;

use macroquad::audio::{play_sound, PlaySoundParams, Sound};

/// The bus volumes as linear gains 0..1: Master (everything), SFX (every cue but the menu blip), UI (the menu blip).
/// Glyra's hum plays on Master only, like the original's AudioStreamPlayer3D.
pub fn set_volumes(master: f32, sfx: f32, ui: f32) {
    engine::with(|e| {
        e.master = master.clamp(0.0, 1.0);
        e.sfx = sfx.clamp(0.0, 1.0);
        e.ui = ui.clamp(0.0, 1.0);
    });
}

/// Legacy: one volume for everything (treated as Master, SFX and UI at 100 %). Prefer `set_volumes`.
pub fn set_volume(v: f32) {
    set_volumes(v, 1.0, 1.0);
}

/// Stops every queued cue and loop (a run ends, the floor changes): Sound.reset_run_effects / stop_all.
#[allow(dead_code)]
pub fn stop_all() {
    engine::with(|e| e.stop_all());
}

/// Legacy fixed bank. `Sounds::new()` also starts the sound engine (renders the common cues ahead of use);
/// `Sounds::default()` stays silent (headless runs, tests).
#[allow(dead_code)]
#[derive(Default)]
pub struct Sounds {
    pub step: Option<Sound>,
    pub step_alt: Option<Sound>,
    pub step_sprint: Option<Sound>,
    pub step_crouch: Option<Sound>,
    pub step_vent: Option<Sound>,
    pub hunter_step: Option<Sound>,
    pub collect: Option<Sound>,
    pub battery: Option<Sound>,
    pub click: Option<Sound>,
    pub taser: Option<Sound>,
    pub swing: Option<Sound>,
    pub chop: Option<Sound>,
    pub tool_break: Option<Sound>,
    pub fail: Option<Sound>,
    pub chase: Option<Sound>,
    pub capture: Option<Sound>,
    pub struggle: Option<Sound>,
    pub escape: Option<Sound>,
    pub lift: Option<Sound>,
    pub heartbeat: Option<Sound>,
    pub music_box: Option<Sound>,
    pub menu: Option<Sound>,
}

impl Sounds {
    pub async fn new() -> Sounds {
        engine::init();
        let c = |name: &str, variant: u32| engine::load_frames(&cues::synth(name, variant)?);
        Sounds {
            step: c("footstep_walk", 0),
            step_alt: c("footstep_walk", 1),
            step_sprint: c("footstep_sprint", 0),
            step_crouch: c("footstep_crouch", 0),
            step_vent: c("step:crouch:metal", 0),
            hunter_step: c("hunter_footstep", 0),
            collect: c("collect", 0),
            battery: cues::render("recharge", 0.0, 1.0).and_then(|f| engine::load_frames(&f)),
            click: c("light_switch", 0),
            taser: c("taser", 0),
            swing: c("axe_swing", 0),
            chop: c("axe_chop", 0),
            tool_break: c("tool_break", 0),
            fail: c("interaction_fail", 0),
            chase: c("chase_start", 0),
            capture: c("capture", 0),
            struggle: c("struggle", 0),
            escape: c("escape", 0),
            lift: c("ladder_rung", 0),
            heartbeat: cues::render("heartbeat", 0.0, 1.0).and_then(|f| engine::load_frames(&f)),
            music_box: c("music_box", 0),
            menu: c("menu", 0),
        }
    }
}

#[allow(dead_code)]
/// Legacy: plays a bank sound at `volume` x Master x SFX. Migrated call sites use `sound::*` instead.
pub fn play(s: &Option<Sound>, volume: f32) {
    let bus = engine::with(|e| e.master * e.sfx);
    let volume = volume * bus;
    if let Some(s) = s {
        if volume > 0.0 {
            play_sound(
                s,
                PlaySoundParams {
                    looped: false,
                    volume,
                },
            );
        }
    }
}
