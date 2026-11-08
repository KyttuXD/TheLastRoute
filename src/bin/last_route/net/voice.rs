//! Voice chat groundwork: the codec, the packet framing, the jitter buffer and the range / mute rules.
//! There is no capture or playback here (no audio dependency may be added); see docs/VOICE_CHAT.md.
//!
//! * [`AdpcmEncoder`] / [`decode_frame`]: IMA ADPCM, 16-bit mono, 16 kHz, 20 ms frames (320 samples, 163 bytes).
//!   Every frame carries its own predictor and step index, so one lost datagram never breaks the next frame.
//! * [`VoicePacket`]: sequence, speaker peer, frame count, then the frames. Fits one relay message
//!   ([`super::session::MAX_MESSAGE_BYTES`]).
//! * [`JitterBuffer`]: reorders by sequence, drops late and duplicate frames, reports gaps so the player can conceal them.
//! * [`VoiceRules`] / [`VoiceMutes`]: volume by distance (the same floor and radius idea as the nearby chat and TTS) and the
//!   per-player mute set.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Samples per second.
pub const SAMPLE_RATE: u32 = 16_000;
/// One frame is 20 ms.
pub const FRAME_MS: u32 = 20;
/// Samples in one frame (320).
pub const FRAME_SAMPLES: usize = (SAMPLE_RATE * FRAME_MS / 1000) as usize;
/// Bytes of a frame header: predictor (i16 LE) and step index (u8).
pub const FRAME_HEADER_BYTES: usize = 3;
/// Bytes of a whole encoded frame: header plus two samples per byte (163).
pub const FRAME_BYTES: usize = FRAME_HEADER_BYTES + FRAME_SAMPLES / 2;

/// Bytes of the packet header: tag, sequence (u32 LE), speaker peer (u32 LE), frame count (u8).
pub const PACKET_HEADER_BYTES: usize = 10;
/// First byte of every voice packet, so a stray message is rejected.
pub const PACKET_TAG: u8 = 0x56;
/// The most frames one packet carries: what fits one relay message (8 at 163 bytes), capped at 4 so one lost datagram
/// costs at most 80 ms of speech.
pub const MAX_FRAMES_PER_PACKET: usize = {
    let fit = (super::session::MAX_MESSAGE_BYTES - PACKET_HEADER_BYTES) / FRAME_BYTES;
    if fit < 4 {
        fit
    } else {
        4
    }
};

const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107, 118, 130, 143,
    157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552,
    1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487,
    12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];
const INDEX_TABLE: [i32; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

/// Predictor and step index: the whole state of an IMA ADPCM stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdpcmState {
    pub predictor: i16,
    pub index: u8,
}

impl AdpcmState {
    /// Applies one 4-bit code and returns the reconstructed sample (the encoder and decoder share this).
    fn apply(&mut self, code: u8) -> i16 {
        let step = STEP_TABLE[self.index as usize];
        let mut diff = step >> 3;
        if code & 4 != 0 {
            diff += step;
        }
        if code & 2 != 0 {
            diff += step >> 1;
        }
        if code & 1 != 0 {
            diff += step >> 2;
        }
        let mut p = self.predictor as i32;
        p += if code & 8 != 0 { -diff } else { diff };
        self.predictor = p.clamp(-32768, 32767) as i16;
        let idx = (self.index as i32 + INDEX_TABLE[(code & 7) as usize]).clamp(0, 88);
        self.index = idx as u8;
        self.predictor
    }

    fn encode_sample(&mut self, sample: i16) -> u8 {
        let mut diff = sample as i32 - self.predictor as i32;
        let mut code = 0u8;
        if diff < 0 {
            code = 8;
            diff = -diff;
        }
        let mut step = STEP_TABLE[self.index as usize];
        if diff >= step {
            code |= 4;
            diff -= step;
        }
        step >>= 1;
        if diff >= step {
            code |= 2;
            diff -= step;
        }
        step >>= 1;
        if diff >= step {
            code |= 1;
        }
        self.apply(code);
        code
    }
}

/// A streaming encoder: keep one per local microphone.
#[derive(Clone, Debug, Default)]
pub struct AdpcmEncoder {
    state: AdpcmState,
}

impl AdpcmEncoder {
    pub fn new() -> AdpcmEncoder {
        AdpcmEncoder::default()
    }

    /// Encodes exactly [`FRAME_SAMPLES`] samples into [`FRAME_BYTES`] bytes; `None` for any other length.
    pub fn encode_frame(&mut self, pcm: &[i16]) -> Option<Vec<u8>> {
        if pcm.len() != FRAME_SAMPLES {
            return None;
        }
        let mut out = Vec::with_capacity(FRAME_BYTES);
        out.extend_from_slice(&self.state.predictor.to_le_bytes());
        out.push(self.state.index);
        for pair in pcm.chunks_exact(2) {
            let lo = self.state.encode_sample(pair[0]);
            let hi = self.state.encode_sample(pair[1]);
            out.push(lo | (hi << 4));
        }
        Some(out)
    }
}

/// Decodes one frame (stateless: the header carries the state). `None` for a wrong length or a bad step index.
pub fn decode_frame(bytes: &[u8]) -> Option<Vec<i16>> {
    if bytes.len() != FRAME_BYTES || bytes[2] > 88 {
        return None;
    }
    let mut st = AdpcmState { predictor: i16::from_le_bytes([bytes[0], bytes[1]]), index: bytes[2] };
    let mut out = Vec::with_capacity(FRAME_SAMPLES);
    for b in &bytes[FRAME_HEADER_BYTES..] {
        out.push(st.apply(b & 0x0F));
        out.push(st.apply(b >> 4));
    }
    Some(out)
}

/// One datagram of speech from one speaker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoicePacket {
    /// Counts frames, not packets: the first frame's number (so the jitter buffer orders every frame).
    pub seq: u32,
    pub speaker: u32,
    /// Encoded frames, each [`FRAME_BYTES`] long, 1..=[`MAX_FRAMES_PER_PACKET`].
    pub frames: Vec<Vec<u8>>,
}

impl VoicePacket {
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.frames.is_empty() || self.frames.len() > MAX_FRAMES_PER_PACKET || self.frames.iter().any(|f| f.len() != FRAME_BYTES) {
            return None;
        }
        let mut out = Vec::with_capacity(PACKET_HEADER_BYTES + self.frames.len() * FRAME_BYTES);
        out.push(PACKET_TAG);
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.speaker.to_le_bytes());
        out.push(self.frames.len() as u8);
        for f in &self.frames {
            out.extend_from_slice(f);
        }
        Some(out)
    }

    /// Strict: the tag, the count and the exact length must all match.
    pub fn decode(bytes: &[u8]) -> Option<VoicePacket> {
        if bytes.len() < PACKET_HEADER_BYTES || bytes[0] != PACKET_TAG {
            return None;
        }
        let count = bytes[9] as usize;
        if count == 0 || count > MAX_FRAMES_PER_PACKET || bytes.len() != PACKET_HEADER_BYTES + count * FRAME_BYTES {
            return None;
        }
        let seq = u32::from_le_bytes(bytes[1..5].try_into().ok()?);
        let speaker = u32::from_le_bytes(bytes[5..9].try_into().ok()?);
        let frames = bytes[PACKET_HEADER_BYTES..].chunks_exact(FRAME_BYTES).map(|c| c.to_vec()).collect();
        Some(VoicePacket { seq, speaker, frames })
    }

    /// Frame number of the i-th frame.
    pub fn frame_seq(&self, i: usize) -> u32 {
        self.seq.wrapping_add(i as u32)
    }
}

/// What the player gets for the next 20 ms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Playout {
    /// Decoded samples.
    Frame(Vec<i16>),
    /// This frame never arrived but later ones did: play concealment (repeat or fade) and move on.
    Lost,
    /// Nothing buffered (the speaker stopped, or the buffer is refilling): play silence, do not advance.
    Empty,
}

/// Reorders one speaker's frames. Sequence numbers are compared as plain u32 (50 per second: wraps after about 2.7 years).
#[derive(Debug)]
pub struct JitterBuffer {
    frames: BTreeMap<u32, Vec<i16>>,
    next: Option<u32>,
    /// Frames to collect before playout starts.
    target_depth: usize,
    /// Hard cap; beyond it the oldest frames are skipped.
    max_depth: usize,
    pub late_dropped: u32,
    pub duplicates_dropped: u32,
    pub lost: u32,
    pub overflow_dropped: u32,
}

impl JitterBuffer {
    pub fn new(target_depth: usize, max_depth: usize) -> JitterBuffer {
        let target = target_depth.max(1);
        JitterBuffer {
            frames: BTreeMap::new(),
            next: None,
            target_depth: target,
            max_depth: max_depth.max(target),
            late_dropped: 0,
            duplicates_dropped: 0,
            lost: 0,
            overflow_dropped: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Adds an encoded frame. Returns false when it was dropped (bad data, too late, or a duplicate).
    pub fn push(&mut self, seq: u32, encoded: &[u8]) -> bool {
        let Some(pcm) = decode_frame(encoded) else { return false };
        if matches!(self.next, Some(n) if seq < n) {
            self.late_dropped += 1;
            return false;
        }
        if self.frames.contains_key(&seq) {
            self.duplicates_dropped += 1;
            return false;
        }
        self.frames.insert(seq, pcm);
        while self.frames.len() > self.max_depth {
            let oldest = *self.frames.keys().next().unwrap();
            self.frames.remove(&oldest);
            self.overflow_dropped += 1;
            if matches!(self.next, Some(n) if n <= oldest) {
                self.next = Some(oldest + 1);
            }
        }
        true
    }

    /// The next 20 ms. Call once per frame period.
    pub fn pop(&mut self) -> Playout {
        let next = match self.next {
            Some(n) => n,
            None => {
                if self.frames.len() < self.target_depth {
                    return Playout::Empty;
                }
                *self.frames.keys().next().unwrap()
            }
        };
        if let Some(pcm) = self.frames.remove(&next) {
            self.next = Some(next + 1);
            return Playout::Frame(pcm);
        }
        if self.frames.is_empty() {
            // The speaker stopped: re-prime on the next burst.
            self.next = None;
            return Playout::Empty;
        }
        self.lost += 1;
        self.next = Some(next + 1);
        Playout::Lost
    }
}

/// Range and volume tuning, from data/voice.json (the Standard defaults if it is missing or broken).
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceRules {
    /// Beyond this many world units a speaker is silent.
    pub radius: f64,
    /// Within this many units the volume is full.
    pub full_radius: f64,
    pub jitter_target_frames: usize,
    pub jitter_max_frames: usize,
}

impl Default for VoiceRules {
    fn default() -> Self {
        VoiceRules { radius: 30.0, full_radius: 6.0, jitter_target_frames: 3, jitter_max_frames: 15 }
    }
}

impl VoiceRules {
    pub fn parse(v: &Value) -> VoiceRules {
        let d = VoiceRules::default();
        let radius = v.get("radius").and_then(Value::as_f64).filter(|r| *r > 0.0).unwrap_or(d.radius);
        let full = v.get("full_radius").and_then(Value::as_f64).filter(|r| *r >= 0.0).unwrap_or(d.full_radius).min(radius);
        VoiceRules {
            radius,
            full_radius: full,
            jitter_target_frames: v.get("jitter_target_frames").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, 20)).unwrap_or(d.jitter_target_frames),
            jitter_max_frames: v.get("jitter_max_frames").and_then(Value::as_u64).map(|n| (n as usize).clamp(2, 100)).unwrap_or(d.jitter_max_frames),
        }
    }

    pub fn load() -> &'static VoiceRules {
        static RULES: std::sync::OnceLock<VoiceRules> = std::sync::OnceLock::new();
        RULES.get_or_init(|| {
            crate::content::embedded::data_under("voice.json")
                .next()
                .and_then(|(_, text)| serde_json::from_str::<Value>(text).ok())
                .map(|v| VoiceRules::parse(&v))
                .unwrap_or_default()
        })
    }

    /// Volume 0..=1 from the distance: full inside `full_radius`, a straight fall to zero at `radius`.
    pub fn volume_at(&self, distance: f64) -> f32 {
        if !distance.is_finite() || distance >= self.radius {
            return 0.0;
        }
        if distance <= self.full_radius {
            return 1.0;
        }
        (1.0 - (distance - self.full_radius) / (self.radius - self.full_radius)) as f32
    }

    /// Volume for a listener: zero on another floor, else by distance (the same floor rule as nearby chat and TTS).
    pub fn volume_for(&self, speaker_world: [f64; 3], speaker_depth: i64, listener_world: [f64; 3], listener_depth: i64) -> f32 {
        if speaker_depth != listener_depth {
            return 0.0;
        }
        let d: f64 = (0..3).map(|i| (speaker_world[i] - listener_world[i]).powi(2)).sum::<f64>().sqrt();
        self.volume_at(d)
    }
}

/// The local player's per-peer mutes.
#[derive(Clone, Debug, Default)]
pub struct VoiceMutes {
    muted: BTreeSet<u32>,
}

impl VoiceMutes {
    pub fn set_muted(&mut self, peer: u32, muted: bool) {
        if muted {
            self.muted.insert(peer);
        } else {
            self.muted.remove(&peer);
        }
    }

    /// Flips and returns the new state.
    pub fn toggle(&mut self, peer: u32) -> bool {
        let now = !self.is_muted(peer);
        self.set_muted(peer, now);
        now
    }

    pub fn is_muted(&self, peer: u32) -> bool {
        self.muted.contains(&peer)
    }

    /// The gain to play a speaker at: zero when muted, else the range volume.
    pub fn gain(&self, peer: u32, volume_in_range: f32) -> f32 {
        if self.is_muted(peer) {
            0.0
        } else {
            volume_in_range.clamp(0.0, 1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(offset: usize, amp: f64) -> Vec<i16> {
        (0..FRAME_SAMPLES).map(|i| (amp * (((i + offset) as f64) * 0.15).sin()) as i16).collect()
    }

    fn rms(a: &[i16], b: &[i16]) -> f64 {
        (a.iter().zip(b).map(|(x, y)| ((*x as f64) - (*y as f64)).powi(2)).sum::<f64>() / a.len() as f64).sqrt()
    }

    #[test]
    fn frame_constants_fit_the_relay() {
        assert_eq!(FRAME_SAMPLES, 320);
        assert_eq!(FRAME_BYTES, 163);
        assert!(MAX_FRAMES_PER_PACKET >= 1);
        assert!(PACKET_HEADER_BYTES + MAX_FRAMES_PER_PACKET * FRAME_BYTES <= super::super::session::MAX_MESSAGE_BYTES);
    }

    #[test]
    fn adpcm_round_trip_stays_within_error_bounds() {
        let mut enc = AdpcmEncoder::new();
        let mut worst = 0i32;
        for f in 0..20 {
            let pcm = sine(f * FRAME_SAMPLES, 8000.0);
            let bytes = enc.encode_frame(&pcm).unwrap();
            assert_eq!(bytes.len(), FRAME_BYTES);
            let out = decode_frame(&bytes).unwrap();
            assert_eq!(out.len(), FRAME_SAMPLES);
            if f >= 2 {
                assert!(rms(&pcm, &out) < 400.0, "frame {f} rms {}", rms(&pcm, &out));
                for (a, b) in pcm.iter().zip(&out) {
                    worst = worst.max((*a as i32 - *b as i32).abs());
                }
            }
        }
        assert!(worst < 2500, "worst {worst}");
    }

    #[test]
    fn silence_and_extremes_do_not_break_the_codec() {
        let mut enc = AdpcmEncoder::new();
        let quiet = enc.encode_frame(&vec![0i16; FRAME_SAMPLES]).unwrap();
        assert!(decode_frame(&quiet).unwrap().iter().all(|s| s.abs() <= 8));
        let loud: Vec<i16> = (0..FRAME_SAMPLES).map(|i| if i % 2 == 0 { i16::MAX } else { i16::MIN }).collect();
        let out = decode_frame(&enc.encode_frame(&loud).unwrap()).unwrap();
        assert_eq!(out.len(), FRAME_SAMPLES);
        assert!(enc.encode_frame(&[0i16; 10]).is_none());
        assert!(decode_frame(&[0u8; 10]).is_none());
        let mut bad = vec![0u8; FRAME_BYTES];
        bad[2] = 200;
        assert!(decode_frame(&bad).is_none());
    }

    #[test]
    fn a_lost_frame_does_not_corrupt_the_next() {
        let mut enc = AdpcmEncoder::new();
        let frames: Vec<Vec<u8>> = (0..3).map(|f| enc.encode_frame(&sine(f * FRAME_SAMPLES, 6000.0)).unwrap()).collect();
        // Frames 0 and 1 are never decoded: frame 2 still comes out right, because its header carries the state.
        let alone = decode_frame(&frames[2]).unwrap();
        assert!(rms(&alone, &sine(2 * FRAME_SAMPLES, 6000.0)) < 400.0);
    }

    fn packet(seq: u32, n: usize) -> VoicePacket {
        let mut enc = AdpcmEncoder::new();
        VoicePacket { seq, speaker: 7, frames: (0..n).map(|i| enc.encode_frame(&sine(i * 50, 5000.0)).unwrap()).collect() }
    }

    #[test]
    fn packet_round_trip() {
        for n in 1..=MAX_FRAMES_PER_PACKET {
            let p = packet(123_456, n);
            let bytes = p.encode().unwrap();
            assert!(bytes.len() <= super::super::session::MAX_MESSAGE_BYTES);
            assert_eq!(VoicePacket::decode(&bytes), Some(p));
        }
    }

    #[test]
    fn packet_rejects_bad_input() {
        let good = packet(1, 2).encode().unwrap();
        assert!(VoicePacket::decode(&good[..good.len() - 1]).is_none());
        let mut tag = good.clone();
        tag[0] = 0;
        assert!(VoicePacket::decode(&tag).is_none());
        let mut count = good.clone();
        count[9] = 0;
        assert!(VoicePacket::decode(&count).is_none());
        assert!(VoicePacket::decode(&[]).is_none());
        assert!(packet(1, MAX_FRAMES_PER_PACKET + 1).encode().is_none());
        assert!(VoicePacket { seq: 0, speaker: 0, frames: vec![] }.encode().is_none());
        assert_eq!(packet(u32::MAX, 2).frame_seq(1), 0);
    }

    fn frame_of(level: i16) -> Vec<u8> {
        AdpcmEncoder::new().encode_frame(&vec![level; FRAME_SAMPLES]).unwrap()
    }

    #[test]
    fn jitter_buffer_reorders() {
        let mut jb = JitterBuffer::new(3, 10);
        let f: Vec<Vec<u8>> = (0..4).map(|i| frame_of(i * 1000)).collect();
        assert_eq!(jb.pop(), Playout::Empty);
        jb.push(2, &f[2]);
        jb.push(0, &f[0]);
        assert_eq!(jb.pop(), Playout::Empty, "still priming");
        jb.push(1, &f[1]);
        for i in 0..3 {
            assert_eq!(jb.pop(), Playout::Frame(decode_frame(&f[i]).unwrap()), "frame {i}");
        }
        assert_eq!(jb.pop(), Playout::Empty);
    }

    #[test]
    fn jitter_buffer_drops_late_and_duplicates_and_reports_gaps() {
        let mut jb = JitterBuffer::new(2, 10);
        let f = frame_of(500);
        jb.push(0, &f);
        jb.push(2, &f);
        assert!(!jb.push(2, &f));
        assert_eq!(jb.duplicates_dropped, 1);
        assert!(matches!(jb.pop(), Playout::Frame(_)));
        assert_eq!(jb.pop(), Playout::Lost, "frame 1 never came");
        assert!(!jb.push(1, &f), "frame 1 is late now");
        assert_eq!(jb.late_dropped, 1);
        assert!(matches!(jb.pop(), Playout::Frame(_)));
        assert_eq!(jb.lost, 1);
        assert!(!jb.push(5, &[1, 2, 3]), "garbage is refused");
    }

    #[test]
    fn jitter_buffer_caps_its_depth() {
        let mut jb = JitterBuffer::new(2, 4);
        let f = frame_of(100);
        for s in 0..10 {
            jb.push(s, &f);
        }
        assert_eq!(jb.len(), 4);
        assert_eq!(jb.overflow_dropped, 6);
        assert!(matches!(jb.pop(), Playout::Frame(_)));
    }

    #[test]
    fn volume_falls_with_distance_and_floors() {
        let r = VoiceRules::default();
        assert_eq!(r.volume_at(0.0), 1.0);
        assert_eq!(r.volume_at(r.full_radius), 1.0);
        assert_eq!(r.volume_at(r.radius), 0.0);
        assert_eq!(r.volume_at(f64::NAN), 0.0);
        let mid = r.volume_at((r.full_radius + r.radius) / 2.0);
        assert!(mid > 0.4 && mid < 0.6);
        assert!(r.volume_at(10.0) > r.volume_at(20.0));
        assert_eq!(r.volume_for([0.0; 3], 0, [3.0, 0.0, 0.0], 1), 0.0);
        assert_eq!(r.volume_for([0.0; 3], 0, [3.0, 0.0, 0.0], 0), 1.0);
    }

    #[test]
    fn rules_parse_from_data() {
        let r = VoiceRules::parse(&serde_json::json!({"radius": 10.0, "full_radius": 50.0, "jitter_target_frames": 5}));
        assert_eq!(r.radius, 10.0);
        assert_eq!(r.full_radius, 10.0, "full radius never exceeds the radius");
        assert_eq!(r.jitter_target_frames, 5);
        assert_eq!(VoiceRules::parse(&serde_json::json!({})), VoiceRules::default());
        assert_eq!(VoiceRules::parse(&serde_json::json!({"radius": -1})).radius, VoiceRules::default().radius);
    }

    #[test]
    fn mute_silences_one_peer_only() {
        let mut m = VoiceMutes::default();
        assert_eq!(m.gain(1, 0.8), 0.8);
        assert!(m.toggle(1));
        assert_eq!(m.gain(1, 0.8), 0.0);
        assert_eq!(m.gain(2, 0.8), 0.8);
        assert!(!m.toggle(1));
        assert_eq!(m.gain(1, 0.8), 0.8);
    }
}
