//! PATCHED for Last Route (src/audio/quad-snd, see src/audio/cues/NOTES.md "Output"): the mixer runs at the output
//! device's rate and reads every voice through Godot 4.7's AudioStreamPlaybackResampled (16.16 fixed-point position,
//! Catmull-Rom cubic over a 3-frame history, so the output reads 2 source frames late), which is what the original's
//! game does to its 44.1 kHz cues on a 48 kHz device. Sounds are still loaded as 44.1 kHz stereo.

use crate::{AudioContext, PlaySoundParams};

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc;

/// The rate sounds are stored at (load_samples_from_file converts anything else to it).
pub const SOURCE_RATE: u32 = 44100;
const FP_BITS: u32 = 16;
const FP_LEN: u64 = 1 << FP_BITS;
const FP_MASK: u64 = FP_LEN - 1;

enum AudioMessage {
    AddSound(u32, Vec<f32>),
    Play(u32, u32, bool, f32),
    Stop(u32),
    StopAll(u32),
    SetVolume(u32, f32),
    SetVolumeAll(u32, f32),
    Delete(u32),
}

/// Last Route patch: loaded sounds are held as 16-bit samples, half the memory of f32. The sources are 16-bit
/// files and 16-bit rendered cues, so nothing audible is lost; mixing is still done in f32.
const PCM_SCALE: f32 = 32767.0;

fn to_pcm(samples: &[f32]) -> Arc<[i16]> {
    samples.iter().map(|s| (s.clamp(-1.0, 1.0) * PCM_SCALE).round() as i16).collect::<Vec<i16>>().into()
}

#[derive(Debug)]
pub struct SoundState {
    sound_id: u32,
    play_id: u32,
    /// Godot's `mix_offset`: source frame position in 16.16 fixed point.
    offset: u64,
    data: Arc<[i16]>,
    looped: bool,
    volume: f32,
}

impl SoundState {
    fn frames(&self) -> usize {
        self.data.len() / 2
    }

    /// Source frame `k` (k < 0 is the zeroed history; past the end wraps when looped, else silence).
    fn frame(&self, k: i64) -> (f32, f32) {
        let n = self.frames() as i64;
        if k < 0 || n == 0 {
            return (0.0, 0.0);
        }
        let k = if k >= n {
            if !self.looped {
                return (0.0, 0.0);
            }
            k % n
        } else {
            k
        } as usize;
        (self.data[2 * k] as f32 / PCM_SCALE, self.data[2 * k + 1] as f32 / PCM_SCALE)
    }

    /// One output frame (AudioStreamPlaybackResampled::mix).
    fn next(&mut self, increment: u64) -> (f32, f32) {
        let k = (self.offset >> FP_BITS) as i64;
        let mu = (self.offset & FP_MASK) as f32 / FP_LEN as f32;
        let y0 = self.frame(k - 3);
        let y1 = self.frame(k - 2);
        let y2 = self.frame(k - 1);
        let y3 = self.frame(k);
        let mu2 = mu * mu;
        let cubic = |y0: f32, y1: f32, y2: f32, y3: f32| {
            let a0 = 3.0 * y1 - 3.0 * y2 + y3 - y0;
            let a1 = 2.0 * y0 - 5.0 * y1 + 4.0 * y2 - y3;
            let a2 = y2 - y0;
            let a3 = 2.0 * y1;
            (a0 * mu * mu2 + a1 * mu2 + a2 * mu + a3) / 2.0
        };
        let out = (cubic(y0.0, y1.0, y2.0, y3.0), cubic(y0.1, y1.1, y2.1, y3.1));
        self.offset += increment;
        if self.looped {
            // keep the position bounded; the history reads wrap with it
            let n = self.frames() as u64;
            if n > 0 && (self.offset >> FP_BITS) >= 2 * n + 3 {
                self.offset -= n << FP_BITS;
            }
        }
        out
    }

    fn finished(&self) -> bool {
        !self.looped && (self.offset >> FP_BITS) as usize >= self.frames() + 3
    }
}

/// The one-shot voice pool (cues/NOTES.md, "One-shots ... from a pool of 16 voices"). Non-looped plays are the pool; loops
/// (the music box tune, Glyra's hum, the falls, the wind-down) are not in it.
pub const ONE_SHOT_VOICES: usize = 16;

pub struct Mixer {
    rx: mpsc::Receiver<AudioMessage>,
    sounds: HashMap<u32, Arc<[i16]>>,
    mixer_state: Vec<SoundState>,
    /// Godot's `mix_increment` for a 44.1 kHz stream at the output rate.
    increment: u64,
}

impl Mixer {
    /// With the one-shot pool full, the oldest one-shot (lowest play id) is cut off so the new one can start.
    fn cut_oldest_one_shot(&mut self) {
        let busy = self.mixer_state.iter().filter(|s| !s.looped).count();
        if busy < ONE_SHOT_VOICES {
            return;
        }
        let oldest = self
            .mixer_state
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.looped)
            .min_by_key(|(_, s)| s.play_id)
            .map(|(i, _)| i);
        if let Some(i) = oldest {
            self.mixer_state.swap_remove(i);
        }
    }
}

pub struct MixerBuilder {
    rx: mpsc::Receiver<AudioMessage>,
}

pub struct MixerControl {
    tx: mpsc::Sender<AudioMessage>,
    sound_id: Cell<u32>,
    play_id: Cell<u32>,
}

pub struct Playback {
    play_id: u32,
}

impl Playback {
    pub fn stop(self, ctx: &AudioContext) {
        ctx.mixer_ctrl.send(AudioMessage::Stop(self.play_id));
    }

    pub fn set_volume(&self, ctx: &AudioContext, volume: f32) {
        ctx.mixer_ctrl.send(AudioMessage::SetVolume(self.play_id, volume));
    }
}

impl MixerControl {
    pub fn load(&self, data: &[u8]) -> u32 {
        let sound_id = self.sound_id.get();

        let samples = load_samples_from_file(data).unwrap();

        self.tx
            .send(crate::mixer::AudioMessage::AddSound(sound_id, samples))
            .unwrap_or_else(|_| println!("Audio thread died"));
        self.sound_id.set(sound_id + 1);

        sound_id
    }

    pub fn play(&self, sound_id: u32, params: PlaySoundParams) -> Playback {
        let play_id = self.play_id.get();

        self.send(AudioMessage::Play(sound_id, play_id, params.looped, params.volume));

        self.play_id.set(play_id + 1);

        Playback { play_id }
    }

    pub fn stop(&self, play_id: u32) {
        self.send(AudioMessage::Stop(play_id));
    }

    pub fn stop_all(&self, sound_id: u32) {
        self.send(AudioMessage::StopAll(sound_id));
    }

    pub fn set_volume_all(&self, sound_id: u32, volume: f32) {
        self.send(AudioMessage::SetVolumeAll(sound_id, volume));
    }

    pub fn delete(&self, sound_id: u32) {
        self.send(AudioMessage::Delete(sound_id));
    }

    fn send(&self, message: AudioMessage) {
        self.tx.send(message).unwrap_or_else(|_| println!("Audio thread died"))
    }
}

impl MixerBuilder {
    /// A mixer whose output runs at 44.1 kHz (the backends other than WASAPI).
    pub fn build(self) -> Mixer {
        self.build_at(SOURCE_RATE)
    }

    /// A mixer whose output runs at `output_rate` Hz.
    pub fn build_at(self, output_rate: u32) -> Mixer {
        Mixer {
            rx: self.rx,
            sounds: HashMap::new(),
            mixer_state: vec![],
            increment: mix_increment(output_rate),
        }
    }
}

/// `uint64_t((stream_rate / target_rate) * FP_LEN)` (audio_stream.cpp, AudioStreamPlaybackResampled::mix).
pub fn mix_increment(output_rate: u32) -> u64 {
    ((SOURCE_RATE as f64 / output_rate.max(1) as f64) * FP_LEN as f64) as u64
}

impl Mixer {
    pub fn new() -> (MixerBuilder, MixerControl) {
        let (tx, rx) = mpsc::channel();

        (
            MixerBuilder { rx },
            MixerControl { tx, sound_id: Cell::new(0), play_id: Cell::new(0) },
        )
    }

    pub fn fill_audio_buffer(&mut self, buffer: &mut [f32], _frames: usize) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                AudioMessage::AddSound(id, data) => {
                    self.sounds.insert(id, to_pcm(&data));
                }
                AudioMessage::Play(sound_id, play_id, looped, volume) => {
                    if let Some(data) = self.sounds.get(&sound_id).cloned() {
                        if !looped {
                            self.cut_oldest_one_shot();
                        }
                        self.mixer_state.push(SoundState { sound_id, play_id, offset: 0, data, looped, volume });
                    }
                }
                AudioMessage::Stop(play_id) => {
                    if let Some(i) = self.mixer_state.iter().position(|s| s.play_id == play_id) {
                        self.mixer_state.swap_remove(i);
                    }
                }
                AudioMessage::StopAll(sound_id) => {
                    for i in (0..self.mixer_state.len()).rev() {
                        if self.mixer_state[i].sound_id == sound_id {
                            self.mixer_state.swap_remove(i);
                        }
                    }
                }
                AudioMessage::SetVolume(play_id, volume) => {
                    if let Some(sound) = self.mixer_state.iter_mut().find(|s| s.play_id == play_id) {
                        sound.volume = volume;
                    }
                }
                AudioMessage::SetVolumeAll(sound_id, volume) => {
                    for sound in self.mixer_state.iter_mut().filter(|s| s.sound_id == sound_id) {
                        sound.volume = volume;
                    }
                }
                AudioMessage::Delete(sound_id) => {
                    for i in (0..self.mixer_state.len()).rev() {
                        if self.mixer_state[i].sound_id == sound_id {
                            self.mixer_state.swap_remove(i);
                        }
                    }
                    self.sounds.remove(&sound_id);
                }
            }
        }

        buffer.fill(0.0);
        let increment = self.increment;
        let mut i = 0;
        while let Some(sound) = self.mixer_state.get_mut(i) {
            let volume = sound.volume;
            for frame in buffer.chunks_exact_mut(2) {
                if sound.finished() {
                    break;
                }
                let (l, r) = sound.next(increment);
                frame[0] += l * volume;
                frame[1] += r * volume;
            }
            if sound.finished() {
                self.mixer_state.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}

/// PCM WAV samples as interleaved f32 with the channel count and sample rate.
/// Last Route patch: this replaces the audrey decoder (and with it lewton, ogg, hound and dasp). Every sound the
/// games load is a 16-bit WAV they render themselves; Ogg Vorbis is no longer decoded.
fn read_wav(bytes: &[u8]) -> Option<(Vec<f32>, usize, u32)> {
    use std::convert::TryInto;
    let u16_at = |at: usize| Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut format = None;
    while at + 8 <= bytes.len() {
        let id = bytes.get(at..at + 4)?;
        let len = u32_at(at + 4)? as usize;
        let body = at + 8;
        if id == b"fmt " {
            // tag 1 is integer PCM, 3 is float; 0xFFFE (extensible) carries the real tag at offset 24
            let mut tag = u16_at(body)?;
            if tag == 0xFFFE {
                tag = u16_at(body + 24)?;
            }
            format = Some((tag, u16_at(body + 2)? as usize, u32_at(body + 4)?, u16_at(body + 14)?));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format?;
            let data = bytes.get(body..body.checked_add(len)?.min(bytes.len()))?;
            let samples: Vec<f32> = match (tag, bits) {
                (1, 8) => data.iter().map(|b| (*b as f32 - 128.0) / 128.0).collect(),
                (1, 16) => data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect(),
                (1, 24) => data.chunks_exact(3).map(|b| (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0).collect(),
                (1, 32) => data.chunks_exact(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0).collect(),
                (3, 32) => data.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
                _ => return None,
            };
            return Some((samples, channels, rate));
        }
        at = body.checked_add(len)?.checked_add(len & 1)?;
    }
    None
}

/// Parse a PCM WAV file and get resampled to 44100, 2 channel data
pub fn load_samples_from_file(bytes: &[u8]) -> Result<Vec<f32>, ()> {
    let (samples, channels_count, sample_rate) = read_wav(bytes).ok_or(())?;
    if channels_count != 1 && channels_count != 2 {
        return Err(());
    }
    let mut frames: Vec<f32> = Vec::with_capacity(samples.len() * 2 / channels_count);
    if channels_count == 1 {
        frames.extend(samples.into_iter().flat_map(|sample| [sample, sample]));
    } else {
        frames.extend(samples);
    }
    frames.truncate(frames.len() - frames.len() % 2);


    // stupid nearest-neighbor resampler (only for files not at 44.1 kHz; the game's cues all are)
    if sample_rate != SOURCE_RATE {
        let mut new_length = ((SOURCE_RATE as f32 / sample_rate as f32) * frames.len() as f32) as usize;
        new_length -= new_length % 2;
        let mut resampled = vec![0.0; new_length];
        for (n, sample) in resampled.chunks_exact_mut(2).enumerate() {
            let ix = 2 * ((n as f32 / new_length as f32) * frames.len() as f32) as usize;
            sample[0] = frames[ix];
            sample[1] = frames[ix + 1];
        }
        return Ok(resampled);
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seventeenth_one_shot_cuts_the_oldest_and_loops_are_outside_the_pool() {
        let (builder, control) = Mixer::new();
        let mut mixer = builder.build();
        control.tx.send(AudioMessage::AddSound(0, vec![0.1; 4000])).unwrap();
        control.tx.send(AudioMessage::Play(0, 0, true, 1.0)).unwrap(); // a loop: not a pool voice
        for play_id in 1..=17u32 {
            control.tx.send(AudioMessage::Play(0, play_id, false, 1.0)).unwrap();
        }
        let mut buf = vec![0.0f32; 64];
        mixer.fill_audio_buffer(&mut buf, 32);
        let ids: Vec<u32> = mixer.mixer_state.iter().map(|s| s.play_id).collect();
        assert_eq!(ids.len(), 17, "one loop and the 16 newest one-shots");
        assert!(!ids.contains(&1), "the oldest one-shot is the one cut");
        assert!(ids.contains(&0) && ids.contains(&17), "the loop and the newest one-shot both play");
    }

    #[test]
    fn unity_rate_is_exact_two_frames_late() {
        let data: Vec<f32> = (0..200).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut s = SoundState { sound_id: 0, play_id: 0, offset: 0, data: to_pcm(&data), looped: false, volume: 1.0 };
        let inc = mix_increment(44100);
        assert_eq!(inc, FP_LEN);
        let out: Vec<(f32, f32)> = (0..102).map(|_| s.next(inc)).collect();
        assert_eq!(out[0], (0.0, 0.0));
        for k in 2..102 {
            // exact to the 16-bit step the samples are stored at
            assert!((out[k].0 - data[2 * (k - 2)]).abs() < 1.0 / PCM_SCALE);
        }
    }

    #[test]
    fn a_16_bit_wav_reads_back_and_other_data_is_refused() {
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 2] {
            wav.extend_from_slice(&v.to_le_bytes());
        }
        wav.extend_from_slice(&44100u32.to_le_bytes());
        wav.extend_from_slice(&(44100u32 * 4).to_le_bytes());
        for v in [4u16, 16] {
            wav.extend_from_slice(&v.to_le_bytes());
        }
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&8u32.to_le_bytes());
        for v in [16384i16, -16384, 32767, -32768] {
            wav.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(load_samples_from_file(&wav), Ok(vec![0.5, -0.5, 32767.0 / 32768.0, -1.0]));
        assert_eq!(load_samples_from_file(b"OggS not a wav"), Err(()));
        assert_eq!(load_samples_from_file(&wav[..30]), Err(()));
    }
}
