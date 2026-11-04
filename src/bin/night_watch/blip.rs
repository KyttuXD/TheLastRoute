//! The call box's speech blips: one short procedural burst per revealed letter, rendered to a WAV byte buffer per
//! speaker (a few pitch variants each, picked by the letter so the same letter always sounds the same) and played
//! with macroquad's audio. No sound file ships; with no audio context the player stays silent.

use super::typewriter::{ui, BlipRules, CallsUi, Voice, Wave};
use macroquad::audio::{load_sound_from_bytes, play_sound, PlaySoundParams, Sound};
use std::collections::HashMap;
use std::f64::consts::PI;
use std::future::Future;
use std::task::{Context, Poll, Waker};

/// The variant (0..n) a letter plays: a small deterministic hash, case-blind, so repeats sound alike.
pub fn variant_of(c: char, n: usize) -> usize {
    let k = c.to_lowercase().next().unwrap_or(c) as u32;
    (k.wrapping_mul(2_654_435_761) >> 16) as usize % n.max(1)
}

/// The pitch of variant `v` of `n`: `base` spread evenly across plus or minus `jitter` (a fraction).
pub fn variant_pitch(base: f64, jitter: f64, v: usize, n: usize) -> f64 {
    if n <= 1 {
        return base;
    }
    base * (1.0 + jitter * (2.0 * v as f64 / (n - 1) as f64 - 1.0))
}

/// One blip as 16-bit mono samples: a sine or triangle burst with a fast attack and an exponential-ish decay.
pub fn blip_samples(wave: Wave, hz: f64, ms: f64, attack_ms: f64, volume: f64, sample_rate: u32) -> Vec<i16> {
    let rate = f64::from(sample_rate);
    let n = ((ms / 1000.0) * rate).round().max(1.0) as usize;
    let attack = ((attack_ms / 1000.0) * rate).max(1.0);
    (0..n)
        .map(|i| {
            let t = i as f64 / rate;
            let phase = (hz * t).fract();
            let raw = match wave {
                Wave::Sine => (2.0 * PI * phase).sin(),
                Wave::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
            };
            let rise = (i as f64 / attack).min(1.0);
            let fall = (1.0 - i as f64 / n as f64).powi(2);
            (raw * rise * fall * volume.clamp(0.0, 1.0) * f64::from(i16::MAX)) as i16
        })
        .collect()
}

/// A canonical 44-byte-header PCM WAV (mono, 16-bit) around `samples`.
pub fn wav_bytes(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The WAV of one speaker's blip variant `v`.
pub fn voice_wav(voice: &Voice, rules: &BlipRules, v: usize) -> Vec<u8> {
    let hz = variant_pitch(voice.pitch_hz, rules.pitch_jitter, v, rules.variants);
    let samples = blip_samples(voice.wave, hz, rules.ms, rules.attack_ms, rules.volume * voice.volume, rules.sample_rate);
    wav_bytes(&samples, rules.sample_rate)
}

/// `load_sound_from_bytes` finishes on its first poll on every native target, so a blip loads when first asked for.
fn load(bytes: &[u8]) -> Option<Sound> {
    let mut fut = std::pin::pin!(load_sound_from_bytes(bytes));
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(s)) => Some(s),
        _ => None,
    }
}

/// Plays blips. Each speaker's variants are rendered and loaded once, the first time that speaker blips.
pub struct BlipPlayer {
    banks: HashMap<String, Vec<Option<Sound>>>,
    pub master: f32,
    pub enabled: bool,
}

impl BlipPlayer {
    pub fn new() -> Self {
        Self { banks: HashMap::new(), master: ui().blip.master as f32, enabled: true }
    }

    /// Master volume 0..1; 0 plays nothing. (The settings screen will call this; nothing does yet.)
    #[allow(dead_code)]
    pub fn set_master(&mut self, volume: f32) {
        self.master = volume.clamp(0.0, 1.0);
    }

    /// One blip of `speaker` for the letter `c`. Silent when disabled, at zero volume or without an audio context.
    pub fn play(&mut self, speaker: &str, c: char) {
        if !self.enabled || self.master <= 0.0 {
            return;
        }
        let data: &CallsUi = ui();
        let voice = data.voice(speaker);
        let bank = self.banks.entry(voice.id.clone()).or_insert_with(|| {
            (0..data.blip.variants).map(|v| load(&voice_wav(voice, &data.blip, v))).collect()
        });
        if let Some(Some(sound)) = bank.get(variant_of(c, bank.len())) {
            play_sound(sound, PlaySoundParams { looped: false, volume: self.master });
        }
    }

    /// Plays one blip per letter, in order.
    pub fn play_all(&mut self, speaker: &str, letters: &[char]) {
        for c in letters {
            self.play(speaker, *c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blip_is_20_to_40_ms_of_samples() {
        let r = &ui().blip;
        let s = blip_samples(Wave::Sine, 500.0, r.ms, r.attack_ms, 1.0, r.sample_rate);
        let ms = s.len() as f64 * 1000.0 / f64::from(r.sample_rate);
        assert!((20.0..=40.0).contains(&ms), "{ms} ms");
        assert!(s.iter().any(|v| v.abs() > 1000));
    }

    #[test]
    fn the_envelope_starts_quiet_and_dies_away() {
        let s = blip_samples(Wave::Triangle, 400.0, 30.0, 3.0, 1.0, 22050);
        assert_eq!(s[0], 0);
        let peak = s.iter().map(|v| i32::from(*v).abs()).max().unwrap();
        assert!(peak > 5000);
        let tail = s[s.len() - 4..].iter().map(|v| i32::from(*v).abs()).max().unwrap();
        assert!(tail < peak / 10);
    }

    #[test]
    fn volume_scales_the_peak_and_zero_is_silent() {
        let loud = blip_samples(Wave::Sine, 440.0, 30.0, 2.0, 1.0, 22050);
        let soft = blip_samples(Wave::Sine, 440.0, 30.0, 2.0, 0.25, 22050);
        let peak = |s: &[i16]| s.iter().map(|v| i32::from(*v).abs()).max().unwrap();
        assert!(peak(&soft) < peak(&loud) / 2);
        assert!(blip_samples(Wave::Sine, 440.0, 30.0, 2.0, 0.0, 22050).iter().all(|v| *v == 0));
    }

    #[test]
    fn the_wav_has_a_valid_header_and_length() {
        let samples = vec![1i16, -2, 3, -4];
        let w = wav_bytes(&samples, 22050);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 22050);
        assert_eq!(&w[36..40], b"data");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 8);
        assert_eq!(w.len(), 52);
        assert_eq!(i16::from_le_bytes([w[46], w[47]]), -2);
    }

    #[test]
    fn a_letter_always_plays_the_same_variant_in_range() {
        for c in 'a'..='z' {
            let v = variant_of(c, 6);
            assert!(v < 6);
            assert_eq!(v, variant_of(c, 6));
            assert_eq!(v, variant_of(c.to_ascii_uppercase(), 6));
        }
        let used: std::collections::HashSet<usize> = ('a'..='z').map(|c| variant_of(c, 6)).collect();
        assert!(used.len() >= 3, "letters should spread over the variants");
        assert_eq!(variant_of('x', 0), 0);
    }

    #[test]
    fn variant_pitch_spreads_around_the_base_within_the_jitter() {
        assert_eq!(variant_pitch(500.0, 0.1, 0, 1), 500.0);
        let lo = variant_pitch(500.0, 0.1, 0, 5);
        let hi = variant_pitch(500.0, 0.1, 4, 5);
        assert!((lo - 450.0).abs() < 1e-9 && (hi - 550.0).abs() < 1e-9);
        assert_eq!(variant_pitch(500.0, 0.1, 2, 5), 500.0);
    }

    #[test]
    fn each_speaker_has_its_own_pitch() {
        let u = ui();
        let z = voice_wav(u.voice("zellia"), &u.blip, 0);
        let k = voice_wav(u.voice("kathy"), &u.blip, 0);
        assert_ne!(z, k);
        assert_eq!(z, voice_wav(u.voice("zellia"), &u.blip, 0));
    }
}
