//! The wound music box and Glyra's hum: looping, positional, pitch-ridden playback of two rendered cues.
//!
//! Sources: game/world/interactables/music_box_tune.gd (the tune), music_boxes.gd (the clock),
//! game/hunters/view/hunter_view_3d.gd 1546-1570 (Glyra's hum), game/config/game_config.gd 543-544.
//!
//! IMPORTANT for the finite route: MusicBoxTune is only attached when the interactable has a `music_boxes` clock, which
//! only the streamed (infinite) route view sets (streamed_facility_view_3d.gd 4734). The finite floor's FacilityView3D
//! never sets it, so in main_3d winding the box plays the one-shot "music_box" phrase (route_interactable_3d.gd 818)
//! and the only continuous music is Glyra's own hum while her box PLAYS. The tune and wind-down are ported here
//! anyway (requested, and the port may want them); NOTES.md says the same.
//!
//! Godot changes a looping stream's pitch with `pitch_scale` (resampling: slower AND lower). macroquad 0.4 cannot
//! change the pitch of a playing sound, so `LoopPlayer` resamples in software and `winddown` pre-renders the whole
//! off-key stretch as one buffer.

use super::render::SAMPLE_RATE;

/// The last stretch of a wound box, in seconds, where the tune slows (data/music_box.json `winddown_seconds`).
pub fn winddown_seconds() -> f32 {
    crate::game::music_box::rules().winddown_seconds
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Playing,
    WindingDown,
    Stopped,
}

/// MusicBoxes.mood_for (music_boxes.gd 31).
pub fn mood_for(seconds_left: f32) -> Mood {
    if seconds_left <= 0.0 {
        Mood::Stopped
    } else if seconds_left <= winddown_seconds() {
        Mood::WindingDown
    } else {
        Mood::Playing
    }
}

/// The tune's `pitch_scale` (music_box_tune.gd 35-45): 1 while playing; in the wind-down
/// `max(.55, .78 + .2 * left / 15 + .045 * sin(clock * 6.3))` (slower towards the end, with a wobble that pulls it off
/// key); `None` = stopped. `clock` is the seconds the tune node has existed (its own `_clock += delta`).
pub fn pitch_scale(seconds_left: f32, clock: f32) -> Option<f32> {
    match mood_for(seconds_left) {
        Mood::Playing => Some(1.0),
        Mood::WindingDown => {
            let left = seconds_left / winddown_seconds();
            Some((0.78 + 0.2 * left + 0.045 * (clock * 6.3).sin()).max(0.55))
        }
        Mood::Stopped => None,
    }
}

/// The tune loop (`render_cue("music_box_tune")`, looped over its whole length: 16 notes 0.27 s apart, then a breath).
pub fn tune() -> Vec<[f32; 2]> {
    super::render("music_box_tune", 0.0, 1.0).unwrap_or_default()
}

/// Glyra's hum loop (`render_cue("glyra_hum")`, looped), played while her box is PLAYING (GlyraGimmick.humming()).
pub fn glyra_hum() -> Vec<[f32; 2]> {
    super::render("glyra_hum", 0.0, 1.0).unwrap_or_default()
}

/// Positional settings of the two loops (AudioStreamPlayer3D, inverse-distance attenuation, Godot's default max_db 3).
#[derive(Clone, Copy, Debug)]
pub struct Emitter {
    pub unit_size: f32,
    pub max_distance: f32,
    pub volume_db: f32,
    /// Height of the emitter above its node (m).
    pub height: f32,
}

/// MusicBoxTune: heard to 12 m, +6 dB, 0.4 m above the box.
pub const TUNE_EMITTER: Emitter = Emitter {
    unit_size: 5.0,
    max_distance: 12.0,
    volume_db: 6.0,
    height: 0.4,
};
/// GlyraHum: heard to 14 m, +3 dB, 1.5 m up (her head).
pub const HUM_EMITTER: Emitter = Emitter {
    unit_size: 4.0,
    max_distance: 14.0,
    volume_db: 3.0,
    height: 1.5,
};

impl Emitter {
    /// Godot's ATTENUATION_INVERSE_DISTANCE: `linear_to_db(1 / (d / unit_size + eps)) + volume_db`, capped at max_db
    /// (3 dB), silent past max_distance. Returned as a linear gain. (Godot also low-passes distant 3D sources, default
    /// cutoff 5 kHz at -24 dB attenuation; not modelled.)
    pub fn gain(&self, distance: f32) -> f32 {
        if distance > self.max_distance {
            return 0.0;
        }
        let db =
            (20.0 * (1.0 / (distance / self.unit_size + 1e-5)).log10() + self.volume_db).min(3.0);
        10f32.powf(db / 20.0)
    }
}

/// A looping buffer read at a variable rate (linear interpolation between frames, like Godot's WAV playback).
#[derive(Clone, Debug)]
pub struct LoopPlayer {
    pub frames: Vec<[f32; 2]>,
    /// Read position in source frames.
    pub pos: f64,
}

impl LoopPlayer {
    pub fn new(frames: Vec<[f32; 2]>) -> Self {
        LoopPlayer { frames, pos: 0.0 }
    }

    /// Fills `out` (at SAMPLE_RATE) advancing `pitch` source frames per output frame.
    pub fn fill(&mut self, out: &mut [[f32; 2]], pitch: f32) {
        let n = self.frames.len();
        if n == 0 {
            out.iter_mut().for_each(|f| *f = [0.0, 0.0]);
            return;
        }
        for o in out.iter_mut() {
            let i = self.pos.floor() as usize % n;
            let j = (i + 1) % n;
            let frac = (self.pos - self.pos.floor()) as f32;
            let (a, b) = (self.frames[i], self.frames[j]);
            *o = [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac];
            self.pos = (self.pos + pitch as f64) % n as f64;
        }
    }
}

/// The whole wind-down as one buffer: the tune from `start_pos` (source frames into the loop, e.g. where the steady
/// loop was) with the pitch recomputed every output frame from `seconds_left` (counting down from the value given,
/// normally `winddown_seconds()`) and the tune node's `clock`. Ends when the box stops. Per-frame pitch is a smooth
/// stand-in for the original's per-frame (_process) update.
pub fn winddown(start_pos: f64, clock: f32, seconds_left: f32) -> Vec<[f32; 2]> {
    let mut player = LoopPlayer::new(tune());
    player.pos = start_pos;
    let total = (seconds_left.max(0.0) * SAMPLE_RATE as f32) as usize;
    let mut out = vec![[0.0f32; 2]; total];
    let dt = 1.0 / SAMPLE_RATE as f32;
    for (k, frame) in out.iter_mut().enumerate() {
        let t = k as f32 * dt;
        let Some(pitch) = pitch_scale(seconds_left - t, clock + t) else {
            break;
        };
        player.fill(std::slice::from_mut(frame), pitch);
    }
    out
}
