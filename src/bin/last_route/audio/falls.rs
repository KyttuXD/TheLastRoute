//! The waterfall loop: filtered noise rendered at runtime (the original ships no sound files), played positionally
//! from the nearest fall of the sewer window, keyed by the table's `sound_key`. Every number is in the `sound` block
//! of data/render/waterfall.json; this file holds only the behaviour. The scene calls `update` every frame it draws
//! a sewer window (with `None` when no fall is in range, which stops the loop).

use super::cues::positional::Emitter3D;
use super::cues::rules::Bus;
use super::cues::SAMPLE_RATE;
use super::engine::with;
use crate::render::waterfall::{config, SoundCfg, WaterfallCfg};

/// A small deterministic noise source (an LCG), a value in -1..1 per call.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        ((self.0 >> 8) as f32 / (1u32 << 24) as f32) * 2.0 - 1.0
    }
}

/// The loop's frames: one-pole low-passed noise at `gain`, the left and right from different noise, the end
/// cross-faded into the start so the loop has no seam.
pub fn loop_frames(cfg: &SoundCfg) -> Vec<[f32; 2]> {
    let n = ((cfg.loop_seconds * SAMPLE_RATE as f32) as usize).max(2);
    let fade = (n / 4).max(1);
    let chan = |seed: u32| -> Vec<f32> {
        let mut noise = Noise(seed);
        let mut y = 0.0f32;
        (0..n + fade)
            .map(|_| {
                y += cfg.lowpass * (noise.next() - y);
                // the one-pole filter takes the level down by about sqrt(lowpass / 2); bring it back up
                y * (2.0 / cfg.lowpass.max(1e-3)).sqrt() * 0.5
            })
            .collect()
    };
    let (l, r) = (chan(0x1234_5678), chan(0x9e37_79b9));
    let seamless = |x: &[f32], i: usize| -> f32 {
        if i < fade {
            let w = i as f32 / fade as f32;
            x[i] * w + x[n + i] * (1.0 - w)
        } else {
            x[i]
        }
    };
    (0..n)
        .map(|i| [(seamless(&l, i) * cfg.gain).clamp(-1.0, 1.0), (seamless(&r, i) * cfg.gain).clamp(-1.0, 1.0)])
        .collect()
}

/// The stereo gains of the loop for a fall at `pos` heard from `eye` with the camera's basis, before the master bus.
pub fn gains(cfg: &WaterfallCfg, pos: [f32; 3], eye: [f32; 3], right: [f32; 3], up: [f32; 3], forward: [f32; 3]) -> (f32, f32) {
    Emitter3D {
        unit_size: cfg.sound.unit_size_m,
        max_distance: cfg.sound_range_m,
        volume_db: cfg.sound.volume_db,
        max_db: cfg.sound.max_db,
        height: cfg.sound.height_m,
    }
    .gains_from(pos, eye, right, up, forward)
}

/// Keeps the falls loop playing from `pos` (the nearest fall in range), or stops it for `None`. A positional loop on the
/// Master bus: the engine applies Master (and re-applies it when it changes), so the gains here are the positional ones.
pub fn update(pos: Option<[f32; 3]>, eye: [f32; 3], right: [f32; 3], up: [f32; 3], forward: [f32; 3]) {
    let cfg = config();
    with(|e| {
        let base = pos.map(|p| gains(cfg, p, eye, right, up, forward));
        e.set_loop(cfg.sound_key.as_str(), Bus::Master, base, || loop_frames(&cfg.sound));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WaterfallCfg {
        WaterfallCfg::parse(include_str!("../data/render/waterfall.json")).unwrap()
    }

    #[test]
    fn the_loop_has_the_data_length_and_no_seam() {
        let c = cfg();
        let f = loop_frames(&c.sound);
        assert_eq!(f.len(), (c.sound.loop_seconds * SAMPLE_RATE as f32) as usize);
        assert!(f.iter().any(|s| s[0].abs() > 0.01), "it makes sound");
        assert!(f.iter().all(|s| s[0].abs() <= 1.0 && s[1].abs() <= 1.0));
        let step = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs().max((a[1] - b[1]).abs());
        let typical = f.windows(2).map(|w| step(w[0], w[1])).fold(0.0f32, f32::max);
        assert!(step(f[f.len() - 1], f[0]) <= typical * 1.5 + 1e-3, "the wrap is as smooth as the loop itself");
    }

    #[test]
    fn data_changes_the_loop() {
        let mut c = cfg();
        let quiet = loop_frames(&c.sound);
        c.sound.gain *= 0.5;
        c.sound.loop_seconds = 1.0;
        let half = loop_frames(&c.sound);
        assert_eq!(half.len() * 2, quiet.len());
        let peak = |f: &[[f32; 2]]| f.iter().map(|s| s[0].abs()).fold(0.0f32, f32::max);
        assert!(peak(&half) < peak(&quiet));
    }

    #[test]
    fn the_loop_fades_with_distance_and_is_silent_past_its_range() {
        let c = cfg();
        let (right, up, fwd) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]);
        let eye = [0.0, 1.6, 0.0];
        let near = gains(&c, [0.0, 0.0, -3.0], eye, right, up, fwd);
        let far = gains(&c, [0.0, 0.0, -12.0], eye, right, up, fwd);
        let gone = gains(&c, [0.0, 0.0, -(c.sound_range_m + 1.0)], eye, right, up, fwd);
        assert!(near.0 > far.0 && far.0 > 0.0);
        assert_eq!(gone, (0.0, 0.0));
        let to_the_right = gains(&c, [4.0, 0.0, 0.0], eye, right, up, fwd);
        assert!(to_the_right.1 > to_the_right.0, "a fall on the right is louder in the right ear");
    }
}
