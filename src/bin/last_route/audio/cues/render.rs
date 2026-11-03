//! The one-shot renderer: an exact port of `Sound._render_components` (game/autoload/sound.gd 1084-1161) plus the cache
//! key / seed it is called with (`_cue_key` 616, `_render_cue_uncached` 621).
//!
//! Exactness notes (they matter: the original pins these bytes with goldens, tests/validate_audio.gd 193):
//! - GDScript floats are f64, so every expression here is f64 in the same order as the source.
//! - The mix buffers are `PackedFloat32Array`: every `left[frame] += x` rounds the running sum to f32. Kept.
//! - Output is 16-bit: `roundi(clampf(x, -.98, .98) * 32767.0)`.
//! - The noise is a 31-bit LCG seeded with Godot's `String.hash()` (djb2 over the cache key), one state shared by every
//!   noise component of the cue in order.

use std::f64::consts::{PI, TAU};

/// The mix rate every cue is rendered at (sound.gd 10, `SAMPLE_RATE := 44100`).
pub const SAMPLE_RATE: u32 = 44100;
const SR: f64 = SAMPLE_RATE as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Tone,
    /// A sine whose pitch glides from 1.15x down to 0.72x of `frequency` over its length.
    Thump,
    /// White noise (the LCG) through a one-pole low-pass.
    Noise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Square,
    Triangle,
    Saw,
}

/// The original's wave names. Anything else is a sine, exactly as `match String(component.get("wave"))` falls through:
/// note "sawtooth" (hunter_snore, hide_search) is NOT "saw" and therefore renders as a sine in the original too.
pub fn wave_named(name: &str) -> Wave {
    match name {
        "square" => Wave::Square,
        "triangle" => Wave::Triangle,
        "saw" => Wave::Saw,
        _ => Wave::Sine,
    }
}

/// One layer of a cue (the original's component Dictionary).
#[derive(Clone, Copy, Debug)]
pub struct Component {
    pub kind: Kind,
    pub frequency: f64,
    pub duration: f64,
    pub volume: f64,
    pub delay: f64,
    pub wave: Wave,
    pub pan: f64,
    /// Noise only: scales the low-pass coefficient (higher = brighter).
    pub rate: f64,
    /// A steady loop layer: no decay envelope (never used by a sound.gd cue; kept for completeness).
    pub flat: bool,
    pub swell: f64,
}

/// `_tone(frequency, duration, volume, delay, wave, pan)` (sound.gd 835).
pub fn tone(
    frequency: f64,
    duration: f64,
    volume: f64,
    delay: f64,
    wave: &str,
    pan: f64,
) -> Component {
    Component {
        kind: Kind::Tone,
        frequency,
        duration,
        volume,
        delay,
        wave: wave_named(wave),
        pan,
        rate: 1.0,
        flat: false,
        swell: 0.0,
    }
}

/// `_thump(frequency, duration, volume, delay, pan)` (sound.gd 841).
pub fn thump(frequency: f64, duration: f64, volume: f64, delay: f64, pan: f64) -> Component {
    Component {
        kind: Kind::Thump,
        ..tone(frequency, duration, volume, delay, "sine", pan)
    }
}

/// `_noise(duration, volume, frequency, delay, pan, rate)` (sound.gd 847). Note the argument order differs from tone.
pub fn noise(
    duration: f64,
    volume: f64,
    frequency: f64,
    delay: f64,
    pan: f64,
    rate: f64,
) -> Component {
    Component {
        kind: Kind::Noise,
        rate,
        ..tone(frequency, duration, volume, delay, "sine", pan)
    }
}

/// Godot's `String.hash()`: djb2 over the UTF-32 code points, as an unsigned 32-bit value.
pub fn godot_string_hash(text: &str) -> u32 {
    let mut hash: u32 = 5381;
    for ch in text.chars() {
        hash = hash.wrapping_mul(33).wrapping_add(ch as u32);
    }
    hash
}

/// Godot's `snappedf(value, step)`: `floor(value / step + 0.5) * step`.
pub fn snapped(value: f64, step: f64) -> f64 {
    if step != 0.0 {
        (value / step + 0.5).floor() * step
    } else {
        value
    }
}

/// The pan and intensity a render really uses: clamped and snapped to 0.05 (sound.gd 622-623).
pub fn clean_params(pan: f64, intensity: f64) -> (f64, f64) {
    (
        snapped(pan.clamp(-1.0, 1.0), 0.05),
        snapped(intensity.clamp(0.0, 4.0), 0.05),
    )
}

/// The cache key of a cue, `"%s|%.2f|%.2f" % [cue, pan, intensity]` (sound.gd 616). Two plays with the same key are the
/// same bytes, so a port can cache loaded sounds by this string.
pub fn cache_key(cue: &str, pan: f64, intensity: f64) -> String {
    let (p, i) = clean_params(pan, intensity);
    format!("{}|{:.2}|{:.2}", cue, p, i)
}

/// The noise seed of a cue: `abs(key.hash())` (sound.gd 625).
pub fn seed_for(cue: &str, pan: f64, intensity: f64) -> i64 {
    godot_string_hash(&cache_key(cue, pan, intensity)) as i64
}

/// `_render_components(components, seed_value, fixed_seconds)` as 16-bit stereo frames, the exact bytes of the
/// original's AudioStreamWAV data.
pub fn render_components(
    components: &[Component],
    seed_value: i64,
    fixed_seconds: f64,
) -> Vec<[i16; 2]> {
    let mut total: f64 = 0.05;
    for c in components {
        total = total.max(c.delay + c.duration + 0.025);
    }
    if fixed_seconds > 0.0 {
        total = fixed_seconds;
    }
    let frame_count = ((total * SR).ceil() as i64).max(1) as usize;
    let mut left = vec![0.0f32; frame_count];
    let mut right = vec![0.0f32; frame_count];
    let mut noise_state: i64 = seed_value.max(1);
    for c in components {
        let start = (c.delay * SR) as i64;
        let count = ((c.duration * SR) as i64).max(1);
        let frequency = c.frequency.max(20.0);
        let volume = c.volume;
        let pan = c.pan.clamp(-1.0, 1.0);
        let left_gain = ((pan + 1.0) * PI * 0.25).cos();
        let right_gain = ((pan + 1.0) * PI * 0.25).sin();
        let mut filtered: f64 = 0.0;
        let alpha = (frequency * c.rate / SR * 5.0).clamp(0.015, 0.55);
        let span = (count as f64 - 1.0).max(1.0);
        for local_index in 0..count {
            let frame = start + local_index;
            if frame >= frame_count as i64 {
                break;
            }
            let t = local_index as f64 / SR;
            let progress = local_index as f64 / span;
            let attack = (t / 0.009).min(1.0);
            let mut envelope = attack * (1.0 - progress).powf(2.25);
            if c.flat {
                envelope = 1.0;
                if c.swell > 0.0 {
                    envelope = 0.55 + 0.45 * (TAU * t / c.swell).sin();
                }
            }
            let sample = match c.kind {
                Kind::Noise => {
                    noise_state =
                        (noise_state.wrapping_mul(1103515245).wrapping_add(12345)) & 0x7fff_ffff;
                    let white = noise_state as f64 / 1073741823.5 - 1.0;
                    filtered += (white - filtered) * alpha;
                    filtered
                }
                Kind::Tone | Kind::Thump => {
                    let mut phase_frequency = frequency;
                    if c.kind == Kind::Thump {
                        // lerpf(a, b, w) = a + (b - a) * w
                        let a = frequency * 1.15;
                        let b = frequency * 0.72;
                        phase_frequency = a + (b - a) * progress;
                    }
                    let phase = TAU * phase_frequency * t;
                    match c.wave {
                        Wave::Square => {
                            if phase.sin() >= 0.0 {
                                1.0
                            } else {
                                -1.0
                            }
                        }
                        Wave::Triangle => phase.sin().asin() * 2.0 / PI,
                        Wave::Saw => 2.0 * ((phase / TAU) % 1.0) - 1.0,
                        Wave::Sine => phase.sin(),
                    }
                }
            };
            let value = sample * volume * envelope;
            let f = frame as usize;
            left[f] = (left[f] as f64 + value * left_gain) as f32;
            right[f] = (right[f] as f64 + value * right_gain) as f32;
        }
    }
    left.iter()
        .zip(right.iter())
        .map(|(l, r)| [quantise(*l), quantise(*r)])
        .collect()
}

fn quantise(x: f32) -> i16 {
    ((x as f64).clamp(-0.98, 0.98) * 32767.0).round() as i16
}

/// 16-bit frames to floats the way Godot's WAV playback reads them (`sample / 32767.0`).
pub fn to_float(frames: &[[i16; 2]]) -> Vec<[f32; 2]> {
    frames
        .iter()
        .map(|f| [f[0] as f32 / 32767.0, f[1] as f32 / 32767.0])
        .collect()
}
