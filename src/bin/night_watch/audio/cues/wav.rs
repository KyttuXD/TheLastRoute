//! A RIFF/WAVE file of stereo frames, for `macroquad::audio::load_sound_from_bytes`.

use super::render::SAMPLE_RATE;

/// 16-bit stereo PCM WAV at `SAMPLE_RATE`. Frames from `render`/`synth` re-quantise to exactly the original's samples
/// (`round(x * 32767)`, clamped to the original's +-0.98).
pub fn wav_bytes(frames: &[[f32; 2]]) -> Vec<u8> {
    wav_bytes_at(frames, SAMPLE_RATE)
}

pub fn wav_bytes_at(frames: &[[f32; 2]], rate: u32) -> Vec<u8> {
    let data_len = (frames.len() * 4) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes()); // stereo
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for f in frames {
        for s in f {
            let v = ((*s as f64).clamp(-0.98, 0.98) * 32767.0).round() as i16;
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    b
}
