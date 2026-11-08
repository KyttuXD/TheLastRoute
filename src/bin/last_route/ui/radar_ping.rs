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

/// Reduced motion: the ring stays put at the mark's size, and is this fraction as bright as the dot.
pub const REDUCED_RING_SHARE: f32 = 0.5;

/// The ping's ring around the dot: its radius, opacity and line width (Godot `facility_map.gd` `_draw_ghost_ping`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub radius: f32,
    pub alpha: f32,
    pub thickness: f32,
}

/// The ring for a sample, or None when nothing shows. `radius` is the dot's radius and `cell` one map cell, both in
/// the units the ring is drawn in. A ping grows the ring from the dot's size to [`RING_GROWTH`] times it while it
/// fades out; with reduced motion it does not grow and is dimmer.
pub fn ring(s: &Sample, radius: f32, cell: f32, reduced: bool) -> Option<Ring> {
    if s.strength <= 0.001 {
        return None;
    }
    let thickness = (cell * 0.10).max(1.0);
    if reduced {
        return Some(Ring { radius, alpha: s.strength * REDUCED_RING_SHARE, thickness });
    }
    let p = s.progress.clamp(0.0, 1.0);
    Some(Ring { radius: radius * (1.0 + (RING_GROWTH - 1.0) * p), alpha: s.strength * (1.0 - p), thickness })
}

/// Draws the ring at `at` in `color` (its own alpha is replaced by the ring's).
pub fn draw_ring(at: (f32, f32), ring: &Ring, mut color: macroquad::color::Color) {
    color.a = ring.alpha.clamp(0.0, 1.0);
    macroquad::shapes::draw_circle_lines(at.0, at.1, ring.radius, ring.thickness, color);
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

    #[test]
    fn the_ring_grows_and_fades_during_a_ping() {
        let early = ring(&sample(DURATION * 0.25, false), 2.0, 10.0, false).unwrap();
        let late = ring(&sample(DURATION * 0.75, false), 2.0, 10.0, false).unwrap();
        assert!(early.radius >= 2.0 && late.radius > early.radius);
        assert!(late.radius <= 2.0 * RING_GROWTH);
        assert!(late.alpha < early.alpha, "it fades as it grows");
        assert!(early.thickness >= 1.0);
    }

    #[test]
    fn no_ring_between_pings() {
        let gap = sample(DURATION + 0.5, false);
        assert!(ring(&gap, 2.0, 10.0, false).is_none());
    }

    #[test]
    fn reduced_motion_ring_is_dim_and_does_not_grow() {
        let a = ring(&sample(0.0, true), 2.0, 10.0, true).unwrap();
        let b = ring(&sample(3.0, true), 2.0, 10.0, true).unwrap();
        assert_eq!((a.radius, b.radius), (2.0, 2.0));
        let normal_peak = ring(&sample(DURATION * 0.2, false), 2.0, 10.0, false).unwrap();
        assert!(a.alpha < normal_peak.alpha && a.alpha < REDUCED_ALPHA);
    }
}
