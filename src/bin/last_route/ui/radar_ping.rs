//! Porcelle's radar blip on the HUD minimap (Godot `game/hud/radar_ping.gd`).
//! A short, slow ping every few seconds — "there now and then" rather than a constant
//! dot. Cosmetic and local: every client samples from its own clock. With reduced
//! motion the blip is a steady dim dot with a slow, small fade.

use std::f32::consts::{PI, TAU};

/// Seconds between pings (centre) and how far each gap may wander.
pub const INTERVAL: f32 = 8.0;
pub const JITTER: f32 = 2.0;
/// How long one ping lasts, and how far its ring grows (in mark radii).
pub const DURATION: f32 = 0.7;
#[allow(dead_code)] // Godot parity; ring draw uses this when the ping ring is ported.
pub const RING_GROWTH: f32 = 3.0;
/// Brightest the dot and ring get (0..1 alpha).
pub const PEAK_ALPHA: f32 = 0.9;
/// Reduced motion: steady dim with a slow small fade.
pub const REDUCED_ALPHA: f32 = 0.38;
pub const REDUCED_FADE: f32 = 0.10;
pub const REDUCED_PERIOD: f32 = 6.0;

/// One sample of the ping: strength (0 = hidden between pings), progress across the ping, cycle index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub strength: f32,
    pub progress: f32,
    pub index: i32,
}

/// Length of one cycle: INTERVAL ± JITTER, deterministic per index.
pub fn cycle_length(index: i32) -> f32 {
    let noise = ((index as f32 * 12.9898 + 78.233).sin() * 43758.5453).rem_euclid(1.0);
    INTERVAL + JITTER * (noise * 2.0 - 1.0)
}

/// Sample the ping at `time` seconds. With `reduced` the strength is the steady dim
/// level with a slow, small fade instead of a ping (Godot `RadarPing.sample`).
pub fn sample(time: f32, reduced: bool) -> Sample {
    if reduced {
        let wave = 0.5 + 0.5 * (TAU * time / REDUCED_PERIOD).sin();
        return Sample {
            strength: REDUCED_ALPHA - REDUCED_FADE * wave,
            progress: 0.0,
            index: -1,
        };
    }
    // Walk cycles from 0 until time falls inside one (stateless, matches Godot's
    // advancing statics for a monotonically increasing clock).
    let mut cycle_index = 0i32;
    let mut cycle_start = 0.0f32;
    if time < 0.0 {
        return Sample {
            strength: 0.0,
            progress: 1.0,
            index: 0,
        };
    }
    loop {
        let len = cycle_length(cycle_index);
        if time < cycle_start + len {
            break;
        }
        cycle_start += len;
        cycle_index += 1;
        // safety: avoid unbounded if time is huge
        if cycle_index > 1_000_000 {
            break;
        }
    }
    let inside = time - cycle_start;
    if inside >= DURATION {
        return Sample {
            strength: 0.0,
            progress: 1.0,
            index: cycle_index,
        };
    }
    let progress = inside / DURATION;
    Sample {
        strength: PEAK_ALPHA * (progress * PI).sin(),
        progress,
        index: cycle_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_stays_dim() {
        let a = sample(0.0, true).strength;
        let b = sample(1.5, true).strength;
        assert!(a > 0.2 && a < 0.5);
        assert!(b > 0.2 && b < 0.5);
    }

    #[test]
    fn ping_peaks_then_hides() {
        let mid = sample(DURATION * 0.5, false);
        assert!(mid.strength > 0.8);
        let after = sample(DURATION + 0.05, false);
        assert!(after.strength < 0.001);
    }
}
