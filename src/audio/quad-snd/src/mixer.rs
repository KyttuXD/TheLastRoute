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

#[derive(Debug)]
pub struct SoundState {
    sound_id: u32,
    play_id: u32,
    /// Godot's `mix_offset`: source frame position in 16.16 fixed point.
    offset: u64,
    data: Arc<[f32]>,
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
        (self.data[2 * k], self.data[2 * k + 1])
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

pub struct Mixer {
    rx: mpsc::Receiver<AudioMessage>,
    sounds: HashMap<u32, Arc<[f32]>>,
    mixer_state: Vec<SoundState>,
    /// Godot's `mix_increment` for a 44.1 kHz stream at the output rate.
    increment: u64,
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
                    self.sounds.insert(id, data.into());
                }
                AudioMessage::Play(sound_id, play_id, looped, volume) => {
                    if let Some(data) = self.sounds.get(&sound_id) {
                        self.mixer_state.push(SoundState { sound_id, play_id, offset: 0, data: data.clone(), looped, volume });
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

/// Parse ogg/wav/etc and get resampled to 44100, 2 channel data
pub fn load_samples_from_file(bytes: &[u8]) -> Result<Vec<f32>, ()> {
    let mut audio_stream = {
        let file = std::io::Cursor::new(bytes);
        audrey::Reader::new(file).unwrap()
    };

    let description = audio_stream.description();
    let channels_count = description.channel_count();
    assert!(channels_count == 1 || channels_count == 2);

    let mut frames: Vec<f32> = Vec::with_capacity(4096);
    let mut samples_iterator = audio_stream.samples::<f32>().map(std::result::Result::unwrap);

    if channels_count == 1 {
        frames.extend(samples_iterator.flat_map(|sample| [sample, sample]));
    } else if channels_count == 2 {
        frames.extend(samples_iterator);
    }

    let sample_rate = description.sample_rate();

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
    fn unity_rate_is_exact_two_frames_late() {
        let data: Arc<[f32]> = (0..200).map(|i| (i as f32 * 0.37).sin()).collect::<Vec<_>>().into();
        let mut s = SoundState { sound_id: 0, play_id: 0, offset: 0, data: data.clone(), looped: false, volume: 1.0 };
        let inc = mix_increment(44100);
        assert_eq!(inc, FP_LEN);
        let out: Vec<(f32, f32)> = (0..102).map(|_| s.next(inc)).collect();
        assert_eq!(out[0], (0.0, 0.0));
        for k in 2..102 {
            assert!((out[k].0 - data[2 * (k - 2)]).abs() < 1e-6);
        }
    }
}
