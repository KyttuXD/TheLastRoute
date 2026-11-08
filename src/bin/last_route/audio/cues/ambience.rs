//! Ambient beds: a port of the original's ambience generator (game/audio/ambience_synth.gd, beds from
//! data/ambience/*.json via ambience_table.gd). The original plays its beds only on the infinite route; this port plays one
//! per region (data/audio/ambience.json), crossfaded when the runner changes region (`Mixer`).
//!
//! `ambience(region, seconds)` renders a seamless loop: the layers (room tone, drones, details, chirps, wind, thuds,
//! creaks, groans, water, crackle) at 22.05 kHz, a small Schroeder reverb sized per space, a DC-blocking high-pass, the
//! tail folded into the head so the loop has no seam, loudness normalised to `target_rms_db`, then up-sampled to the
//! engine's 44.1 kHz. The layer maths follow the original line for line; only the random numbers differ (Godot's PCG32
//! is not ported, a splitmix64 seeded with the same `hash(key)` is used), so a bed has the same character and level but
//! not the same samples. Every number (layers, spaces, loop and fade times, volumes) is data.

use std::collections::HashMap;
use std::f64::consts::{PI, TAU};
use std::sync::OnceLock;

use serde_json::Value;

use super::render::{godot_string_hash, SAMPLE_RATE};

/// The synth's own rate (ambience_synth.gd `RATE`); the result is up-sampled to `SAMPLE_RATE`.
const RATE: f64 = 22050.0;
const MAX_TONE_HZ: f64 = 1600.0;
const DC_CUTOFF_HZ: f64 = 30.0;
const LEVEL_SECONDS: f64 = 0.03;

/// One bed: its reverb space, its volume multiplier, and its layers (kind plus numbers, as in the original's files).
pub struct Bed {
    pub space: String,
    pub volume: f32,
    pub layers: Vec<Value>,
}

pub struct Settings {
    pub loop_seconds: f32,
    pub crossfade_seconds: f32,
    pub target_rms_db: f64,
    pub region_fade_seconds: f32,
    pub gain: f32,
    pub duck_db_per_unit: f32,
    pub fallback: String,
    pub spaces: HashMap<String, (f64, f64)>,
    pub aliases: HashMap<String, String>,
    pub regions: Vec<String>,
    pub beds: HashMap<String, Bed>,
}

fn load() -> Settings {
    let text = crate::content::embedded::data_under("audio/ambience.json")
        .next()
        .map(|x| x.1)
        .unwrap_or("{}");
    let root: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let f = |k: &str, d: f64| root.get(k).and_then(Value::as_f64).unwrap_or(d);
    let strings = |k: &str| -> HashMap<String, String> {
        root.get(k)
            .and_then(Value::as_object)
            .map(|o| o.iter().filter_map(|(a, b)| Some((a.clone(), b.as_str()?.to_string()))).collect())
            .unwrap_or_default()
    };
    let mut spaces = HashMap::new();
    if let Some(o) = root.get("spaces").and_then(Value::as_object) {
        for (name, s) in o {
            let wet = s.get("wet").and_then(Value::as_f64).unwrap_or(0.18);
            let fb = s.get("feedback").and_then(Value::as_f64).unwrap_or(0.68);
            spaces.insert(name.clone(), (wet.clamp(0.0, 1.0), fb.clamp(0.0, 0.95)));
        }
    }
    let mut beds = HashMap::new();
    if let Some(o) = root.get("beds").and_then(Value::as_object) {
        for (id, b) in o {
            beds.insert(
                id.clone(),
                Bed {
                    space: b.get("space").and_then(Value::as_str).unwrap_or("medium").to_string(),
                    volume: b.get("volume").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.0, 4.0) as f32,
                    layers: b.get("layers").and_then(Value::as_array).cloned().unwrap_or_default(),
                },
            );
        }
    }
    Settings {
        loop_seconds: f("loop_seconds", 10.0).clamp(2.0, 60.0) as f32,
        crossfade_seconds: f("crossfade_seconds", 1.0).clamp(0.1, 5.0) as f32,
        target_rms_db: f("target_rms_db", -54.0),
        region_fade_seconds: f("region_fade_seconds", 1.6).clamp(0.05, 20.0) as f32,
        gain: f("gain", 0.5).clamp(0.0, 4.0) as f32,
        duck_db_per_unit: f("duck_db_per_unit", -14.0) as f32,
        fallback: root.get("fallback").and_then(Value::as_str).unwrap_or("maze").to_string(),
        spaces,
        aliases: strings("aliases"),
        regions: root
            .get("regions")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        beds,
    }
}

/// The parsed data/audio/ambience.json.
pub fn settings() -> &'static Settings {
    static S: OnceLock<Settings> = OnceLock::new();
    S.get_or_init(load)
}

/// The bed id a region id plays: its own bed, else its alias, else the data's fallback. Case and spaces are ignored.
pub fn bed_for(region: &str) -> &'static str {
    let s = settings();
    let key = region.trim().to_lowercase();
    if let Some((id, _)) = s.beds.get_key_value(key.as_str()) {
        return id.as_str();
    }
    if let Some(target) = s.aliases.get(&key) {
        if let Some((id, _)) = s.beds.get_key_value(target.as_str()) {
            return id.as_str();
        }
    }
    s.beds.get_key_value(s.fallback.as_str()).map(|(id, _)| id.as_str()).unwrap_or("")
}

/// True when `region` has a bed of its own or an alias (not just the fallback).
pub fn has_bed(region: &str) -> bool {
    let s = settings();
    let key = region.trim().to_lowercase();
    s.beds.contains_key(&key) || s.aliases.get(&key).is_some_and(|t| s.beds.contains_key(t))
}

// ---- random numbers ----

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_add(0x9E37_79B9_7F4A_7C15))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// 0..1
    fn randf(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.randf()
    }
    /// a..=b
    fn int(&mut self, a: i64, b: i64) -> i64 {
        a + (self.next() % ((b - a + 1) as u64)) as i64
    }
}

// ---- layers ----

struct Buf {
    l: Vec<f32>,
    r: Vec<f32>,
}

impl Buf {
    fn len(&self) -> usize {
        self.l.len()
    }
    fn seconds(&self) -> f64 {
        self.l.len() as f64 / RATE
    }
    fn add(&mut self, i: usize, l: f64, r: f64) {
        self.l[i] += l as f32;
        self.r[i] += r as f32;
    }
}

fn gains(pan: f64) -> (f64, f64) {
    let a = (pan + 1.0) * PI * 0.25;
    (a.cos(), a.sin())
}

fn num(layer: &Value, key: &str, default: f64) -> f64 {
    layer.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn room_tone(b: &mut Buf, hz: f64, vol: f64, swell: f64, rng: &mut Rng) {
    let hz = hz.min(MAX_TONE_HZ);
    let vol = vol * 6.0;
    let a = 1.0 - (-TAU * hz / RATE).exp();
    let (mut l1, mut l2, mut r1, mut r2) = (0.0, 0.0, 0.0, 0.0);
    let phase = rng.randf() * TAU;
    for i in 0..b.len() {
        l1 += (rng.range(-1.0, 1.0) - l1) * a;
        l2 += (l1 - l2) * a;
        r1 += (rng.range(-1.0, 1.0) - r1) * a;
        r2 += (r1 - r2) * a;
        let env = if swell > 0.0 { 0.6 + 0.4 * (TAU * i as f64 / (swell * RATE) + phase).sin() } else { 1.0 };
        b.add(i, l2 * vol * env, r2 * vol * env);
    }
}

fn drone(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 100.0).clamp(30.0, MAX_TONE_HZ);
    let vol = num(layer, "vol", 0.008);
    let r = 0.995;
    let c = 2.0 * r * (TAU * hz / RATE).cos();
    let (mut y1, mut y2) = (0.0, 0.0);
    let wobble = rng.range(0.07, 0.13);
    let follow = 1.0 - (-1.0 / (LEVEL_SECONDS * RATE)).exp();
    let mut power = ((1.0 - r) * 4.0f64).powi(2) / 3.0 / ((1.0 - r) * 4.0 * (1.0 - r));
    for i in 0..b.len() {
        let t = i as f64 / RATE;
        let y = rng.range(-1.0, 1.0) * (1.0 - r) * 4.0 + c * y1 - r * r * y2;
        y2 = y1;
        y1 = y;
        power += (y * y - power) * follow;
        let level = y / (power + 1e-12).sqrt() * 0.25;
        let breath = 0.85 + 0.15 * (TAU * wobble * t).sin();
        let v = (level + 0.18 * (TAU * hz * t).sin()) * vol * breath;
        b.add(i, v, v * 0.92);
    }
}

fn details(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 800.0).clamp(60.0, 2400.0);
    let vol = num(layer, "vol", 0.006);
    let every = num(layer, "every", 1.5).max(0.3);
    let length = num(layer, "len", 0.1).clamp(0.03, 1.2);
    let mut t = rng.range(0.0, every);
    let end = b.seconds() - length - 0.05;
    while t < end {
        let pan = rng.range(-0.6, 0.6);
        let gain = vol * rng.range(0.55, 1.0);
        if length >= 0.25 {
            let f = hz * rng.range(0.995, 1.005);
            soft_note(b, t, f, length * 1.6, gain * 0.8, pan);
        } else {
            let f = hz * rng.range(0.9, 1.1);
            tick(b, t, f, length, gain * 2.2, pan, rng);
        }
        t += every * rng.range(0.6, 1.7);
    }
}

fn chirps(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 2600.0).clamp(200.0, 3200.0);
    let vol = num(layer, "vol", 0.004) * 0.6;
    let seconds = b.seconds();
    for _ in 0..(num(layer, "count", 3.0) as i64 * 2) {
        let at = rng.range(0.0, (seconds - 0.6).max(0.0));
        let pan = rng.range(-0.8, 0.8);
        for k in 0..rng.int(2, 4) {
            let f = hz * rng.range(0.92, 1.12);
            soft_note(b, at + k as f64 * 0.09, f, 0.07, vol, pan);
        }
    }
}

fn smooth_curve(frames: usize, step: f64, rng: &mut Rng) -> Vec<f64> {
    let span = ((step * RATE) as usize).max(1);
    let (mut a, mut bb) = (rng.randf(), rng.randf());
    (0..frames)
        .map(|i| {
            let k = i % span;
            if k == 0 && i > 0 {
                a = bb;
                bb = rng.randf();
            }
            let p = k as f64 / span as f64;
            a + (bb - a) * (0.5 - 0.5 * (PI * p).cos())
        })
        .collect()
}

fn wind(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 400.0).clamp(60.0, MAX_TONE_HZ);
    let vol = num(layer, "vol", 0.01) * 5.0;
    let q = num(layer, "q", 1.2).clamp(0.5, 8.0);
    let gust = num(layer, "gust", 6.0).max(1.0);
    let curve = smooth_curve(b.len(), gust * 0.5, rng);
    let damp = 1.0 / q;
    let mut st = [[0.0f64; 2]; 2];
    for i in 0..b.len() {
        let g = curve[i];
        let f = 2.0 * (PI * hz * (0.8 + 0.4 * g) / RATE).sin();
        let env = (0.35 + 0.65 * g) * vol;
        let mut out = [0.0; 2];
        for ch in 0..2 {
            let low = st[ch][0] + f * st[ch][1];
            let high = rng.range(-1.0, 1.0) - low - damp * st[ch][1];
            let band = st[ch][1] + f * high;
            st[ch] = [low, band];
            out[ch] = band * env;
        }
        b.add(i, out[0], out[1]);
    }
}

fn thumps(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 50.0).clamp(25.0, 200.0);
    let vol = num(layer, "vol", 0.02);
    let every = num(layer, "every", 4.0).max(0.8);
    let mut t = rng.range(0.2, every);
    let end = b.seconds() - 0.8;
    while t < end {
        let pan = rng.range(-0.7, 0.7);
        let gain = vol * rng.range(0.5, 1.0);
        let f = hz * rng.range(0.85, 1.15);
        thud(b, t, f, gain, pan, rng);
        if rng.randf() < 0.3 {
            let at = t + rng.range(0.18, 0.4);
            let f = hz * rng.range(0.9, 1.1);
            thud(b, at, f, gain * 0.55, pan, rng);
        }
        t += every * rng.range(0.5, 1.6);
    }
}

fn thud(b: &mut Buf, at: f64, hz: f64, gain: f64, pan: f64, rng: &mut Rng) {
    let start = (at * RATE) as usize;
    let (lg, rg) = gains(pan);
    let a = 1.0 - (-TAU * 180.0 / RATE).exp();
    let (mut lp, mut phase) = (0.0, 0.0);
    for k in 0..(0.6 * RATE) as usize {
        let i = start + k;
        if i >= b.len() {
            break;
        }
        let tt = k as f64 / RATE;
        let f = hz * (1.0 + 0.6 * (-tt * 18.0).exp());
        phase += f / RATE;
        let body = (TAU * phase).sin() * (-tt * 7.0).exp();
        let knock = rng.range(-1.0, 1.0) * (-tt * 40.0).exp() * 0.6;
        lp += (body + knock - lp) * a;
        b.add(i, lp * gain * lg, lp * gain * rg);
    }
}

fn creaks(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 420.0).clamp(120.0, MAX_TONE_HZ);
    let vol = num(layer, "vol", 0.008);
    let every = num(layer, "every", 3.5).max(1.0);
    let mut t = rng.range(0.3, every);
    let end = b.seconds() - 1.6;
    while t < end {
        let length = rng.range(0.45, 1.3);
        let pan = rng.range(-0.8, 0.8);
        let (lg, rg) = gains(pan);
        let gain = vol * rng.range(0.5, 1.0) * 3.0;
        let glide = rng.range(-0.18, 0.18);
        let rate = rng.range(25.0, 70.0);
        let start = (t * RATE) as usize;
        let count = (length * RATE) as usize;
        let mut res = [[0.0f64; 2]; 2];
        let mut next_pulse = 0.0;
        for k in 0..count {
            let i = start + k;
            if i >= b.len() {
                break;
            }
            let p = k as f64 / count as f64;
            let mut x = 0.0;
            if k as f64 >= next_pulse {
                x = rng.range(0.5, 1.0);
                next_pulse = k as f64 + RATE / (rate * (1.0 + 0.5 * (PI * p).sin()) * rng.range(0.8, 1.2));
            }
            let mut y = 0.0;
            for m in 0..2 {
                let f0 = hz * if m == 1 { 2.31 } else { 1.0 } * (1.0 + glide * p);
                let r = 0.992;
                let c = 2.0 * r * (TAU * f0 / RATE).cos();
                let v = x * (1.0 - r) * 10.0 + c * res[m][0] - r * r * res[m][1];
                res[m] = [v, res[m][0]];
                y += v * if m == 1 { 0.6 } else { 1.0 };
            }
            let env = (PI * p).sin();
            b.add(i, y * gain * env * lg, y * gain * env * rg);
        }
        t += every * rng.range(0.6, 1.6);
    }
}

fn groans(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 90.0).clamp(40.0, 400.0);
    let vol = num(layer, "vol", 0.008);
    let every = num(layer, "every", 6.0).max(2.0);
    let mut t = rng.range(0.5, every);
    let end = b.seconds() - 2.8;
    while t < end {
        let length = rng.range(1.1, 2.5);
        let pan = rng.range(-0.6, 0.6);
        let (lg, rg) = gains(pan);
        let gain = vol * rng.range(0.6, 1.0);
        let f_start = hz * rng.range(0.85, 1.15);
        let f_end = f_start * rng.range(0.8, 1.25);
        let a = 1.0 - (-TAU * (hz * 4.0).min(900.0) / RATE).exp();
        let (mut lp1, mut lp2, mut phase) = (0.0, 0.0, 0.0f64);
        let start = (t * RATE) as usize;
        let count = (length * RATE) as usize;
        for k in 0..count {
            let i = start + k;
            if i >= b.len() {
                break;
            }
            let p = k as f64 / count as f64;
            let f = (f_start + (f_end - f_start) * p) * (1.0 + 0.012 * (TAU * 5.5 * k as f64 / RATE).sin());
            phase = (phase + f / RATE) % 1.0;
            lp1 += (phase * 2.0 - 1.0 - lp1) * a;
            lp2 += (lp1 - lp2) * a;
            let env = (PI * p).sin().powf(1.5);
            b.add(i, lp2 * gain * env * lg, lp2 * gain * env * rg);
        }
        t += every * rng.range(0.6, 1.5);
    }
}

fn water(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 1400.0).clamp(300.0, 3000.0);
    let vol = num(layer, "vol", 0.01);
    let density = num(layer, "density", 40.0).clamp(2.0, 200.0);
    let low = num(layer, "low", 300.0).max(20.0);
    room_tone(b, (hz * 0.5).min(MAX_TONE_HZ), vol * 0.35, num(layer, "swell", 0.0), rng);
    let seconds = b.seconds();
    for _ in 0..(seconds * density) as usize {
        let at = rng.range(0.0, seconds - 0.06);
        let f0 = rng.range(low.ln(), hz.ln()).exp();
        let decay = rng.range(0.012, 0.04) * (600.0 / f0);
        let gain = vol * rng.range(0.2, 1.0) * 2.0;
        let pan = rng.range(-0.7, 0.7);
        let (lg, rg) = gains(pan);
        let start = (at * RATE) as usize;
        let count = ((decay * 5.0 * RATE) as usize).min((0.08 * RATE) as usize);
        let mut phase = 0.0;
        for k in 0..count {
            let i = start + k;
            if i >= b.len() {
                break;
            }
            let tt = k as f64 / RATE;
            phase += f0 * (1.0 + tt / decay * 0.25) / RATE;
            let v = (TAU * phase).sin() * (-tt / decay).exp() * gain;
            b.add(i, v * lg, v * rg);
        }
    }
}

fn crackle(b: &mut Buf, layer: &Value, rng: &mut Rng) {
    let hz = num(layer, "hz", 2200.0).clamp(400.0, 3200.0);
    let vol = num(layer, "vol", 0.006);
    let rate = num(layer, "rate", 3.0).clamp(0.2, 40.0);
    let seconds = b.seconds();
    for _ in 0..(seconds * rate) as usize {
        let at = rng.range(0.0, seconds - 0.3);
        let pan = rng.range(-0.8, 0.8);
        let clicks = if rng.randf() < 0.7 { 1 } else { rng.int(3, 6) };
        for c in 0..clicks {
            let when = at + c as f64 * rng.range(0.008, 0.03);
            let f = hz * rng.range(0.7, 1.3);
            let g = vol * rng.randf().powi(2) * 3.0;
            tick(b, when, f, 0.02, g, pan, rng);
        }
    }
}

fn tick(b: &mut Buf, at: f64, hz: f64, length: f64, gain: f64, pan: f64, rng: &mut Rng) {
    let start = (at * RATE) as usize;
    let count = ((length + 0.25) * RATE) as usize;
    let r = (-1.0 / (length * RATE * 0.5)).exp();
    let c = 2.0 * r * (TAU * hz / RATE).cos();
    let (mut y1, mut y2) = (0.0, 0.0);
    let (lg, rg) = gains(pan);
    let burst = (0.004 * RATE) as usize;
    for k in 0..count {
        let i = start + k;
        if i >= b.len() {
            break;
        }
        let x = rng.range(-1.0, 1.0) * if k < burst { 1.0 } else { 0.0 };
        let y = x * (1.0 - r) * 8.0 + c * y1 - r * r * y2;
        y2 = y1;
        y1 = y;
        b.add(i, y * gain * lg, y * gain * rg);
    }
}

fn soft_note(b: &mut Buf, at: f64, hz: f64, length: f64, gain: f64, pan: f64) {
    let start = (at * RATE) as usize;
    let count = (length * RATE) as usize;
    let (lg, rg) = gains(pan);
    let a = 1.0 - (-TAU * (hz * 2.0).min(MAX_TONE_HZ * 2.0) / RATE).exp();
    let mut lp = 0.0;
    for k in 0..count {
        let i = start + k;
        if i >= b.len() {
            break;
        }
        let p = k as f64 / count as f64;
        let env = (PI * p).sin().powi(2);
        let phase = hz * k as f64 / RATE;
        let tri = 4.0 * (phase - (phase + 0.5).floor()).abs() - 1.0;
        lp += (tri - lp) * a;
        b.add(i, lp * gain * env * lg, lp * gain * env * rg);
    }
}

// ---- finishing ----

fn reverb(buf: &mut [f32], wet: f64, feedback: f64, offset: usize) {
    let dry = buf.to_vec();
    let mut out = vec![0.0f32; buf.len()];
    for d0 in [1116 + offset, 1356 + offset, 1617 + offset] {
        let d = (d0 / 2).max(1);
        let mut line = vec![0.0f32; d];
        let (mut idx, mut damp) = (0, 0.0f32);
        for i in 0..buf.len() {
            let y = line[idx];
            damp += (y - damp) * 0.45;
            line[idx] = dry[i] + damp * feedback as f32;
            idx = (idx + 1) % d;
            out[i] += y / 3.0;
        }
    }
    let ad = 278 + offset;
    let mut aline = vec![0.0f32; ad];
    let mut aidx = 0;
    for i in 0..buf.len() {
        let bv = aline[aidx];
        let y = -out[i] * 0.5 + bv;
        aline[aidx] = out[i] + bv * 0.5;
        aidx = (aidx + 1) % ad;
        buf[i] = dry[i] * (1.0 - wet as f32 * 0.5) + y * wet as f32;
    }
}

fn high_pass(buf: &mut [f32]) {
    let r = (-TAU * DC_CUTOFF_HZ / RATE).exp() as f32;
    let (mut x1, mut y1) = (0.0f32, 0.0f32);
    for s in buf.iter_mut() {
        let x = *s;
        let y = x - x1 + r * y1;
        x1 = x;
        y1 = y;
        *s = y;
    }
}

fn fold_tail(buf: &mut [f32], loop_len: usize) {
    let fade = buf.len() - loop_len;
    for k in 0..fade {
        let p = k as f64 / fade as f64;
        buf[k] = buf[k] * (p * PI * 0.5).sin() as f32 + buf[loop_len + k] * (p * PI * 0.5).cos() as f32;
    }
}

/// One bed as a seamless loop of `seconds` at the engine's rate, or silence for a name with no bed at all.
pub fn ambience(region: &str, seconds: f32) -> Vec<[f32; 2]> {
    let out_len = ((seconds.max(0.0) as f64) * SAMPLE_RATE as f64).round().max(1.0) as usize;
    let s = settings();
    let id = bed_for(region);
    let Some(bed) = s.beds.get(id) else {
        return vec![[0.0, 0.0]; out_len];
    };
    let loop_len = ((seconds.max(0.1) as f64) * RATE) as usize;
    let total = loop_len + (s.crossfade_seconds as f64 * RATE) as usize;
    let mut b = Buf { l: vec![0.0; total], r: vec![0.0; total] };
    let mut rng = Rng::new(godot_string_hash(id) as u64);
    for layer in &bed.layers {
        match layer.get("kind").and_then(Value::as_str).unwrap_or("hum") {
            "noise" => room_tone(&mut b, num(layer, "hz", 200.0), num(layer, "vol", 0.005), num(layer, "swell", 0.0), &mut rng),
            "hum" => drone(&mut b, layer, &mut rng),
            "pulse" => details(&mut b, layer, &mut rng),
            "chirp" => chirps(&mut b, layer, &mut rng),
            "wind" => wind(&mut b, layer, &mut rng),
            "thump" => thumps(&mut b, layer, &mut rng),
            "creak" => creaks(&mut b, layer, &mut rng),
            "groan" => groans(&mut b, layer, &mut rng),
            "water" => water(&mut b, layer, &mut rng),
            "crackle" => crackle(&mut b, layer, &mut rng),
            _ => {}
        }
    }
    let (wet, fb) = s.spaces.get(&bed.space).or_else(|| s.spaces.get("medium")).copied().unwrap_or((0.18, 0.68));
    reverb(&mut b.l, wet, fb, 0);
    reverb(&mut b.r, wet, fb, 7);
    high_pass(&mut b.l);
    high_pass(&mut b.r);
    fold_tail(&mut b.l, loop_len);
    fold_tail(&mut b.r, loop_len);
    let sum: f64 = (0..loop_len).map(|i| ((b.l[i] + b.r[i]) as f64 * 0.5).powi(2)).sum();
    let rms = (sum / loop_len as f64).sqrt();
    let gain = if rms > 1e-9 { (10f64.powf(s.target_rms_db / 20.0) / rms) as f32 } else { 1.0 };
    // up-sample 22.05 -> 44.1 kHz by linear interpolation, wrapping at the loop point so the seam stays closed
    let at = |v: &[f32], pos: f64| -> f32 {
        let i = pos.floor() as usize;
        let f = (pos - i as f64) as f32;
        v[i % loop_len] * (1.0 - f) + v[(i + 1) % loop_len] * f
    };
    (0..out_len)
        .map(|n| {
            let pos = n as f64 * RATE / SAMPLE_RATE as f64;
            [at(&b.l, pos) * gain, at(&b.r, pos) * gain]
        })
        .collect()
}

// ---- crossfade between regions ----

/// Two ambience slots and the rules that crossfade them when the region changes. Pure: the caller starts and stops the
/// loops the output names. Fade-in and fade-out are equal-power (`sqrt` of the slot's level) so the sum stays even.
#[derive(Clone, Debug, Default)]
pub struct Mixer {
    slots: [Slot; 2],
    active: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct Slot {
    bed: Option<&'static str>,
    level: f32,
    target: f32,
}

/// What one slot should do this frame: play `bed` at `gain` (a stereo base gain before the bus), or stop (`bed` None).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotOut {
    pub bed: Option<&'static str>,
    pub gain: f32,
}

impl Mixer {
    /// The bed the mixer is fading in (or holding), if any.
    pub fn current(&self) -> Option<&'static str> {
        let s = &self.slots[self.active];
        if s.target > 0.0 { s.bed } else { None }
    }

    /// Advances `dt` seconds. `region` is where the runner is (None: silence), `duck` 0..1 turns the beds down (the
    /// original's `set_ambience_duck`).
    pub fn step(&mut self, dt: f32, region: Option<&str>) -> [SlotOut; 2] {
        self.step_ducked(dt, region, 0.0)
    }

    pub fn step_ducked(&mut self, dt: f32, region: Option<&str>, duck: f32) -> [SlotOut; 2] {
        let s = settings();
        let want = region.map(bed_for).filter(|id| !id.is_empty());
        if want != self.current() {
            self.slots[self.active].target = 0.0;
            if want.is_some() {
                self.active = 1 - self.active;
                self.slots[self.active] = Slot { bed: want, level: 0.0, target: 1.0 };
            }
        }
        let step = dt.max(0.0) / s.region_fade_seconds;
        let duck_gain = 10f32.powf(s.duck_db_per_unit * duck.clamp(0.0, 1.0) / 20.0);
        let mut out = [SlotOut { bed: None, gain: 0.0 }; 2];
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.level < slot.target {
                slot.level = (slot.level + step).min(slot.target);
            } else {
                slot.level = (slot.level - step).max(slot.target);
            }
            if slot.target == 0.0 && slot.level <= 0.0 {
                slot.bed = None;
            }
            if let Some(id) = slot.bed {
                let volume = s.beds.get(id).map(|b| b.volume).unwrap_or(1.0);
                out[i] = SlotOut { bed: Some(id), gain: slot.level.sqrt() * volume * s.gain * duck_gain };
            }
        }
        out
    }
}
