//! Bit-faithful port of the original's procedural sound (game/autoload/sound.gd and friends). The original ships no
//! sound files: every cue is a list of components (tones, pitch-gliding "thumps", low-passed noise) rendered once to a
//! cached 44.1 kHz stereo 16-bit buffer. This module renders the same bytes (checked against the original's own goldens,
//! tests/validate_audio.gd 193, in `tests` below).
//!
//! - `synth(cue, variant)`: a cue as its usual call site plays it (pan 0 / intensity 1 except the per-step variants).
//! - `render(cue, pan, intensity)`: the full `Sound.render_cue(cue, pan, intensity)`, for intensity-driven cues
//!   (heartbeat, struggle, recharge, signal_node, hunter steps...).
//! - `rules`: the front end of sound.gd (throttles, gaits, pan and volume rules, buses) as pure functions that return a
//!   `rules::Play` to render and play. See NOTES.md for every cue's trigger.
//! - `music_box`: the wound music box's looping tune and its slowing, off-key wind-down.
//! - `ambience`: the ambient beds, which never play on the finite route (see ambience.rs).
//!
//! std only; no macroquad here, so it can be tested headless. `wav_bytes` turns frames into a file for
//! `macroquad::audio::load_sound_from_bytes`.
#![allow(dead_code, unused_imports)]

mod ambience;
pub mod music_box;
mod nw;
pub mod positional;
mod render;
pub mod rules;
pub mod stride;
mod surfaces;
mod table;
mod wav;

pub use ambience::ambience;
pub use render::{cache_key, godot_string_hash, Component, Kind, Wave, SAMPLE_RATE};
pub use surfaces::{for_floor as surface_for_floor, SURFACES};
pub use wav::wav_bytes;

use std::sync::OnceLock;

/// The components of a cue as the original builds them (already clean pan / intensity), or `None` for an unknown name.
pub fn components(cue: &str, pan: f64, intensity: f64) -> Option<Vec<Component>> {
    let (p, i) = render::clean_params(pan, intensity);
    table::components(cue, p, i)
}

/// `Sound.render_cue(cue, pan, intensity)` as the original's 16-bit frames (exact bytes). Pan is -1 (left) .. 1 (right),
/// intensity 0..4; both are clamped and snapped to 0.05 like the original, and they also pick the noise seed.
pub fn render_pcm16(cue: &str, pan: f64, intensity: f64) -> Option<Vec<[i16; 2]>> {
    let (p, i) = render::clean_params(pan, intensity);
    let parts = table::components(cue, p, i)?;
    Some(render::render_components(
        &parts,
        render::seed_for(cue, pan, intensity),
        0.0,
    ))
}

/// `Sound.render_cue(cue, pan, intensity)` as float frames (16-bit value / 32767, as Godot's WAV playback reads them).
pub fn render(cue: &str, pan: f64, intensity: f64) -> Option<Vec<[f32; 2]>> {
    render_pcm16(cue, pan, intensity).map(|f| render::to_float(&f))
}

/// A cue the way its usual call site plays it, `variant` standing for the per-play state the original has:
/// - runner steps, `"footstep_<walk|sprint|crouch>"` and `"step:<gait>:<surface>"`: `variant` is the original's step
///   counter. `variant % 4` picks the pitch variant of a surface step (1 + (v - 1.5) * 3.5 %), and odd / even steps pan
///   +0.045 / -0.045 (right foot, left foot), exactly what `Sound.footstep` renders. `"step:<gait>:<surface>:<n>"` is
///   accepted too (n wins over variant for the pitch).
/// - `"step:hunter:<surface>"`: the pitch variant from `variant % 4`, pan 0 (hunter steps are panned at play time, see
///   `rules::hunter_footstep`).
/// - `"struggle"`: intensity 0.5 (the method default), the side from the counter's parity like the original.
/// - `"signal_node"`: intensity 0.7 (the first press of a route).
/// - everything else: pan 0, intensity 1. The original has no other per-play randomness: every play of a cue with the
///   same pan and intensity is the same bytes (the noise is seeded from the cue's key, not per play), so `variant` is
///   ignored for those.
pub fn synth(cue: &str, variant: u32) -> Option<Vec<[f32; 2]>> {
    let (name, pan, intensity) = default_call(cue, variant)?;
    render(&name, pan, intensity)
}

/// The (cue, pan, intensity) `synth` renders, for callers that want to cache by `cache_key`.
pub fn default_call(cue: &str, variant: u32) -> Option<(String, f64, f64)> {
    let side = if variant % 2 == 1 { 1.0 } else { -1.0 };
    if let Some(rest) = cue.strip_prefix("step:") {
        let parts: Vec<&str> = rest.split(':').collect();
        let (gait, surface) = match parts.as_slice() {
            [g, s] | [g, s, _] => (*g, *s),
            _ => return None,
        };
        let index = if parts.len() == 3 {
            parts[2].parse::<u32>().unwrap_or(0)
        } else {
            variant % 4
        };
        let name = format!("step:{}:{}:{}", gait, surface, index);
        let pan = if gait == "hunter" {
            0.0
        } else if index % 2 == 1 {
            0.045
        } else {
            -0.045
        };
        return Some((name, pan, 1.0));
    }
    let known = table::components(cue, 0.0, 1.0).is_some();
    if !known {
        return None;
    }
    Some(match cue {
        "footstep_walk" | "footstep_sprint" | "footstep_crouch" => {
            (cue.to_string(), side * 0.045, 1.0)
        }
        "struggle" => (
            cue.to_string(),
            if variant % 2 == 1 { -0.035 } else { 0.035 },
            0.5,
        ),
        "signal_node" => (cue.to_string(), 0.0, 0.7),
        _ => (cue.to_string(), 0.0, 1.0),
    })
}

/// True for a name `render` / `synth` accept (the original's `knows_cue`, plus the 3-part step names).
pub fn knows_cue(cue: &str) -> bool {
    default_call(cue, 0).is_some()
}

/// The original's beep for an unknown cue name (sound.gd 1080), rendered.
pub fn fallback_beep() -> Vec<[f32; 2]> {
    render::to_float(&render::render_components(&table::fallback(), 1, 0.0))
}

/// Every name `synth` accepts: the plain cues (sound.gd `_cue_components`), the Night Watch cues as `nw_<name>`, and the
/// surface steps as `step:<walk|sprint|crouch|hunter>:<surface>` (append `:<0-3>` for a fixed pitch variant).
pub fn cue_names() -> &'static [&'static str] {
    static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names: Vec<&'static str> = table::NAMES.to_vec();
        for n in nw::NAMES {
            names.push(Box::leak(format!("nw_{}", n).into_boxed_str()));
        }
        for gait in surfaces::GAITS {
            for surface in surfaces::SURFACES {
                names.push(Box::leak(
                    format!("step:{}:{}", gait, surface).into_boxed_str(),
                ));
            }
        }
        names
    })
}

#[cfg(test)]
mod tests;
