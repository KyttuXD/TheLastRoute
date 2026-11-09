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
//! - `set_volumes(Volumes)`: MASTER, GAMEPLAY, HUNTERS, AMBIENCE, MUSIC and MENU / UI (linear, like settings.gd `linear_to_db(pct / 100)`).
//!
//! `Sounds` and `play` are the old fixed bank, kept only so call sites not migrated yet still build.
#![allow(dead_code)] // ported API kept ahead of its callers (these items were the lib's pub API); prune as the port settles

pub mod ambient;
pub mod creature;
pub mod cues;
pub mod emitters;
mod engine;
pub mod falls;
pub mod hunter_ambient;
pub mod sound;
pub mod tts;
pub mod world;

use macroquad::audio::{play_sound, PlaySoundParams, Sound};

use cues::rules::Bus;
pub use cues::rules::{bus_label, Volumes};

/// The volume sliders as linear gains 0..1 (see `Volumes`): MASTER scales every sound; GAMEPLAY the game's sounds, with
/// HUNTERS, AMBIENCE and MUSIC each under it; MENU / UI the menu blip. Every sound plays on one bus (data/audio/buses.json),
/// so none skips a slider. Loops that already play are re-gained here, so a slider moved in the pause menu applies at once.
pub fn set_volumes(v: Volumes) {
    engine::with(|e| {
        e.vol = Volumes {
            master: v.master.clamp(0.0, 1.0),
            gameplay: v.gameplay.clamp(0.0, 1.0),
            hunters: v.hunters.clamp(0.0, 1.0),
            ambience: v.ambience.clamp(0.0, 1.0),
            music: v.music.clamp(0.0, 1.0),
            ui: v.ui.clamp(0.0, 1.0),
        };
        e.refresh_loops();
    });
}

/// Once per frame, from the app loop, whatever the game is doing (paused, on a report, loading): releases delayed cues
/// that are due and ends timed loops (the music box wind-down). `world::update` also pumps, but it is skipped then.
pub fn tick() {
    engine::with(|e| e.tick());
}

/// Legacy: one volume for everything (treated as Master, SFX and UI at 100 %). Prefer `set_volumes`.
pub fn set_volume(v: f32) {
    set_volumes(Volumes { master: v, ..Volumes::default() });
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

/// One bank sound: the cue at `variant` (the synth form), loaded. Awaited, since the browser decodes it first.
async fn bank(name: &str, variant: u32) -> Option<Sound> {
    engine::load_frames(&cues::synth(name, variant)?).await
}

/// One bank sound: the cue rendered at pan 0 and intensity 1, loaded.
async fn bank_render(name: &str) -> Option<Sound> {
    engine::load_frames(&cues::render(name, 0.0, 1.0)?).await
}

impl Sounds {
    pub async fn new() -> Sounds {
        engine::init().await;
        Sounds {
            step: bank("footstep_walk", 0).await,
            step_alt: bank("footstep_walk", 1).await,
            step_sprint: bank("footstep_sprint", 0).await,
            step_crouch: bank("footstep_crouch", 0).await,
            step_vent: bank("step:crouch:metal", 0).await,
            hunter_step: bank("hunter_footstep", 0).await,
            collect: bank("collect", 0).await,
            battery: bank_render("recharge").await,
            click: bank("light_switch", 0).await,
            taser: bank("taser", 0).await,
            swing: bank("axe_swing", 0).await,
            chop: bank("axe_chop", 0).await,
            tool_break: bank("tool_break", 0).await,
            fail: bank("interaction_fail", 0).await,
            chase: bank("chase_start", 0).await,
            capture: bank("capture", 0).await,
            struggle: bank("struggle", 0).await,
            escape: bank("escape", 0).await,
            lift: bank("ladder_rung", 0).await,
            heartbeat: bank_render("heartbeat").await,
            music_box: bank("music_box", 0).await,
            menu: bank("menu", 0).await,
        }
    }
}

#[allow(dead_code)]
/// Legacy: plays a bank sound at `volume` x Master x GAMEPLAY (the engine's SFX bus gain). Migrated call sites use `sound::*`.
pub fn play(s: &Option<Sound>, volume: f32) {
    let Some(s) = s else { return };
    let volume = engine::with(|e| if e.ready { volume * e.bus_gain(Bus::Sfx) } else { 0.0 });
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
