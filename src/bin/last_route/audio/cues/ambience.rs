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

// The synth's own rate (ambience_synth.gd `RATE`, the result is up-sampled to `SAMPLE_RATE`), the highest tone, the DC
// high-pass cut-off and the level follower time are data (`synth` in data/audio/ambience.json); these read them.
fn synth_rate() -> f64 {
    settings().rate
}
fn max_tone_hz() -> f64 {
    settings().max_tone_hz
}
fn dc_cutoff_hz() -> f64 {
    settings().dc_cutoff_hz
}
fn level_seconds() -> f64 {
    settings().level_seconds
}

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
    /// Synth constants (`synth`): the working rate, highest tone, DC cut-off and level-follower time.
    pub rate: f64,
    pub max_tone_hz: f64,
    pub dc_cutoff_hz: f64,
    pub level_seconds: f64,
    /// Beds kept rendered, least recently used evicted (`render.cache_beds`).
    pub cache_beds: usize,
    /// Time a game frame may spend rendering a bed, ms (`render.bed_budget_ms`). A render step works at most
    /// `render.synth_chunk` synth samples of a layer or finishing pass (never fewer than the longest event,
    /// `LONGEST_EVENT_SECONDS`), and the up-sample step `render.upsample_chunk` output frames.
    pub bed_budget_ms: f64,
    pub synth_chunk: usize,
    pub upsample_chunk: usize,
    /// The music-box run render (`render.box_budget_ms`, `render.box_slice_frames`), read by audio/world.rs.
    pub box_budget_ms: f64,
    pub box_slice_frames: usize,
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
    let sub = |obj: &str, k: &str, d: f64| {
        root.get(obj).and_then(|o| o.get(k)).and_then(Value::as_f64).unwrap_or(d)
    };
    let rate = sub("synth", "rate", 22050.0).clamp(4000.0, 48000.0);
    Settings {
        rate,
        max_tone_hz: sub("synth", "max_tone_hz", 1600.0).clamp(100.0, 8000.0),
        dc_cutoff_hz: sub("synth", "dc_cutoff_hz", 30.0).clamp(1.0, 200.0),
        level_seconds: sub("synth", "level_seconds", 0.03).clamp(0.001, 1.0),
        cache_beds: sub("render", "cache_beds", 4.0).clamp(2.0, 64.0) as usize,
        bed_budget_ms: sub("render", "bed_budget_ms", 2.0).clamp(0.0, 50.0),
        synth_chunk: sub("render", "synth_chunk", 65536.0).max((LONGEST_EVENT_SECONDS * rate).ceil()).min(10_000_000.0) as usize,
        upsample_chunk: sub("render", "upsample_chunk", 16384.0).clamp(256.0, 1_000_000.0) as usize,
        box_budget_ms: sub("render", "box_budget_ms", 2.0).clamp(0.0, 50.0),
        box_slice_frames: sub("render", "box_slice_frames", 4096.0).clamp(256.0, 1_000_000.0) as usize,
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
    // an exact bed id (what the mixer passes on each frame) needs no trim or lowercase
    if let Some((id, _)) = s.beds.get_key_value(region) {
        return id.as_str();
    }
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
        self.l.len() as f64 / synth_rate()
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

// A layer is synthesised a chunk at a time (`Job`): the continuous layers (room tone, drone, wind) keep their filter
// state and the next sample; the event layers (details, chirps, thumps, creaks, groans, water, crackle) keep their
// schedule and write one whole event (a note, tick, thud, creak, groan or drip, never longer than
// `LONGEST_EVENT_SECONDS`) at a time. The random numbers are drawn in the order of the original's one-pass loops, so a bed
// has the same samples however its render is cut up.

/// The longest single event a layer writes (a groan, `groans` length 1.1..2.5 s): the synth chunk is never shorter.
pub const LONGEST_EVENT_SECONDS: f64 = 2.5;

/// Room tone (two one-pole low-passed noises with an optional swell), resumable at any sample.
struct Tone {
    a: f64,
    vol: f64,
    swell: f64,
    phase: f64,
    l1: f64,
    l2: f64,
    r1: f64,
    r2: f64,
    i: usize,
}

impl Tone {
    fn new(hz: f64, vol: f64, swell: f64, rng: &mut Rng) -> Tone {
        let hz = hz.min(max_tone_hz());
        let a = 1.0 - (-TAU * hz / synth_rate()).exp();
        let phase = rng.randf() * TAU;
        Tone { a, vol: vol * 6.0, swell, phase, l1: 0.0, l2: 0.0, r1: 0.0, r2: 0.0, i: 0 }
    }

    /// Writes up to `budget` samples; the samples written and whether the layer is done.
    fn run(&mut self, b: &mut Buf, rng: &mut Rng, budget: usize) -> (usize, bool) {
        let (from, to) = (self.i, (self.i + budget).min(b.len()));
        for i in from..to {
            self.l1 += (rng.range(-1.0, 1.0) - self.l1) * self.a;
            self.l2 += (self.l1 - self.l2) * self.a;
            self.r1 += (rng.range(-1.0, 1.0) - self.r1) * self.a;
            self.r2 += (self.r1 - self.r2) * self.a;
            let env = if self.swell > 0.0 { 0.6 + 0.4 * (TAU * i as f64 / (self.swell * synth_rate()) + self.phase).sin() } else { 1.0 };
            b.add(i, self.l2 * self.vol * env, self.r2 * self.vol * env);
        }
        self.i = to;
        (to - from, to >= b.len())
    }
}

/// A resonant noise drone with a sine under it, resumable at any sample.
struct Drone {
    hz: f64,
    vol: f64,
    r: f64,
    c: f64,
    y1: f64,
    y2: f64,
    wobble: f64,
    follow: f64,
    power: f64,
    i: usize,
}

impl Drone {
    fn new(layer: &Value, rng: &mut Rng) -> Drone {
        let hz = num(layer, "hz", 100.0).clamp(30.0, max_tone_hz());
        let vol = num(layer, "vol", 0.008);
        let r = 0.995;
        let c = 2.0 * r * (TAU * hz / synth_rate()).cos();
        let wobble = rng.range(0.07, 0.13);
        let follow = 1.0 - (-1.0 / (level_seconds() * synth_rate())).exp();
        let power = ((1.0 - r) * 4.0f64).powi(2) / 3.0 / ((1.0 - r) * 4.0 * (1.0 - r));
        Drone { hz, vol, r, c, y1: 0.0, y2: 0.0, wobble, follow, power, i: 0 }
    }

    fn run(&mut self, b: &mut Buf, rng: &mut Rng, budget: usize) -> (usize, bool) {
        let (from, to) = (self.i, (self.i + budget).min(b.len()));
        let r = self.r;
        for i in from..to {
            let t = i as f64 / synth_rate();
            let y = rng.range(-1.0, 1.0) * (1.0 - r) * 4.0 + self.c * self.y1 - r * r * self.y2;
            self.y2 = self.y1;
            self.y1 = y;
            self.power += (y * y - self.power) * self.follow;
            let level = y / (self.power + 1e-12).sqrt() * 0.25;
            let breath = 0.85 + 0.15 * (TAU * self.wobble * t).sin();
            let v = (level + 0.18 * (TAU * self.hz * t).sin()) * self.vol * breath;
            b.add(i, v, v * 0.92);
        }
        self.i = to;
        (to - from, to >= b.len())
    }
}

/// A smooth random curve over `frames` samples: a cosine ease between random knots `step` seconds apart. The knots are
/// drawn up front (in the order a one-pass curve draws them), so the curve is read at any sample.
struct Curve {
    span: usize,
    knots: Vec<f64>,
}

impl Curve {
    fn new(frames: usize, step: f64, rng: &mut Rng) -> Curve {
        let span = ((step * synth_rate()) as usize).max(1);
        let mut knots = vec![rng.randf(), rng.randf()];
        for _ in 0..frames.saturating_sub(1) / span {
            knots.push(rng.randf());
        }
        Curve { span, knots }
    }

    fn at(&self, i: usize) -> f64 {
        let (seg, k) = (i / self.span, i % self.span);
        let (a, bb) = (self.knots[seg], self.knots[seg + 1]);
        let p = k as f64 / self.span as f64;
        a + (bb - a) * (0.5 - 0.5 * (PI * p).cos())
    }
}

/// Band-passed noise with gusts, resumable at any sample.
struct Wind {
    hz: f64,
    vol: f64,
    damp: f64,
    curve: Curve,
    st: [[f64; 2]; 2],
    i: usize,
}

impl Wind {
    fn new(b: &Buf, layer: &Value, rng: &mut Rng) -> Wind {
        let hz = num(layer, "hz", 400.0).clamp(60.0, max_tone_hz());
        let vol = num(layer, "vol", 0.01) * 5.0;
        let q = num(layer, "q", 1.2).clamp(0.5, 8.0);
        let gust = num(layer, "gust", 6.0).max(1.0);
        let curve = Curve::new(b.len(), gust * 0.5, rng);
        Wind { hz, vol, damp: 1.0 / q, curve, st: [[0.0; 2]; 2], i: 0 }
    }

    fn run(&mut self, b: &mut Buf, rng: &mut Rng, budget: usize) -> (usize, bool) {
        let (from, to) = (self.i, (self.i + budget).min(b.len()));
        for i in from..to {
            let g = self.curve.at(i);
            let f = 2.0 * (PI * self.hz * (0.8 + 0.4 * g) / synth_rate()).sin();
            let env = (0.35 + 0.65 * g) * self.vol;
            let mut out = [0.0; 2];
            for ch in 0..2 {
                let st = &mut self.st[ch];
                let low = st[0] + f * st[1];
                let high = rng.range(-1.0, 1.0) - low - self.damp * st[1];
                let band = st[1] + f * high;
                *st = [low, band];
                out[ch] = band * env;
            }
            b.add(i, out[0], out[1]);
        }
        self.i = to;
        (to - from, to >= b.len())
    }
}

/// One event of an event layer, its numbers drawn; `run` writes it whole.
enum Event {
    Note { at: f64, hz: f64, length: f64, gain: f64, pan: f64 },
    Tick { at: f64, hz: f64, length: f64, gain: f64, pan: f64 },
    Thud { at: f64, hz: f64, gain: f64, pan: f64 },
    Creak { start: usize, count: usize, hz: f64, glide: f64, rate: f64, gain: f64, pan: f64 },
    Groan { start: usize, count: usize, hz: f64, f_start: f64, f_end: f64, gain: f64, pan: f64 },
    Drip { start: usize, count: usize, f0: f64, decay: f64, gain: f64, pan: f64 },
}

impl Event {
    /// The samples the event writes at most (it stops early at the buffer's end).
    fn frames(&self) -> usize {
        match *self {
            Event::Note { length, .. } => (length * synth_rate()) as usize,
            Event::Tick { length, .. } => ((length + 0.25) * synth_rate()) as usize,
            Event::Thud { .. } => (0.6 * synth_rate()) as usize,
            Event::Creak { count, .. } | Event::Groan { count, .. } | Event::Drip { count, .. } => count,
        }
    }

    fn run(&self, b: &mut Buf, rng: &mut Rng) {
        match *self {
            Event::Note { at, hz, length, gain, pan } => soft_note(b, at, hz, length, gain, pan),
            Event::Tick { at, hz, length, gain, pan } => tick(b, at, hz, length, gain, pan, rng),
            Event::Thud { at, hz, gain, pan } => thud(b, at, hz, gain, pan, rng),
            Event::Creak { start, count, hz, glide, rate, gain, pan } => creak(b, start, count, hz, glide, rate, gain, pan, rng),
            Event::Groan { start, count, hz, f_start, f_end, gain, pan } => groan(b, start, count, hz, f_start, f_end, gain, pan),
            Event::Drip { start, count, f0, decay, gain, pan } => drip(b, start, count, f0, decay, gain, pan),
        }
    }
}

/// The schedule of an event layer: `next` draws the next event's numbers, called only once the previous event is
/// written (an event draws random numbers of its own as it is written).
enum Sched {
    /// `details`: ticks (short) or soft notes (long) every `every` seconds or so.
    Details { hz: f64, vol: f64, every: f64, length: f64, t: f64, end: f64, after: bool },
    /// `chirps`: `n` bursts of 2-4 short notes at random times.
    Chirps { hz: f64, vol: f64, seconds: f64, i: i64, n: i64, k: i64, kn: i64, at: f64, pan: f64 },
    /// `thumps`: thuds every `every` seconds or so, some with a softer echo (`stage` 0 fresh, 1 after a thud that may get
    /// its echo, 2 after the echo or none).
    Thumps { hz: f64, vol: f64, every: f64, t: f64, end: f64, stage: u8, gain: f64, pan: f64 },
    Creaks { hz: f64, vol: f64, every: f64, t: f64, end: f64, after: bool },
    Groans { hz: f64, vol: f64, every: f64, t: f64, end: f64, after: bool },
    /// `water`'s drips (its room tone runs first).
    Drips { hz: f64, vol: f64, low: f64, seconds: f64, i: usize, n: usize },
    /// `crackle`: `n` crackles of one click or a burst of 3-6.
    Crackle { hz: f64, vol: f64, seconds: f64, i: usize, n: usize, c: i64, clicks: i64, at: f64, pan: f64 },
}

impl Sched {
    fn next(&mut self, rng: &mut Rng) -> Option<Event> {
        match self {
            Sched::Details { hz, vol, every, length, t, end, after } => {
                if *after {
                    *t += *every * rng.range(0.6, 1.7);
                }
                if *t >= *end {
                    return None;
                }
                *after = true;
                let pan = rng.range(-0.6, 0.6);
                let gain = *vol * rng.range(0.55, 1.0);
                Some(if *length >= 0.25 {
                    let f = *hz * rng.range(0.995, 1.005);
                    Event::Note { at: *t, hz: f, length: *length * 1.6, gain: gain * 0.8, pan }
                } else {
                    let f = *hz * rng.range(0.9, 1.1);
                    Event::Tick { at: *t, hz: f, length: *length, gain: gain * 2.2, pan }
                })
            }
            Sched::Chirps { hz, vol, seconds, i, n, k, kn, at, pan } => loop {
                if *k < *kn {
                    let f = *hz * rng.range(0.92, 1.12);
                    let e = Event::Note { at: *at + *k as f64 * 0.09, hz: f, length: 0.07, gain: *vol, pan: *pan };
                    *k += 1;
                    return Some(e);
                }
                if *i >= *n {
                    return None;
                }
                *i += 1;
                *at = rng.range(0.0, (*seconds - 0.6).max(0.0));
                *pan = rng.range(-0.8, 0.8);
                *kn = rng.int(2, 4);
                *k = 0;
            },
            Sched::Thumps { hz, vol, every, t, end, stage, gain, pan } => {
                if *stage == 1 {
                    *stage = 2;
                    if rng.randf() < 0.3 {
                        let at = *t + rng.range(0.18, 0.4);
                        let f = *hz * rng.range(0.9, 1.1);
                        return Some(Event::Thud { at, hz: f, gain: *gain * 0.55, pan: *pan });
                    }
                }
                if *stage == 2 {
                    *t += *every * rng.range(0.5, 1.6);
                }
                if *t >= *end {
                    return None;
                }
                *pan = rng.range(-0.7, 0.7);
                *gain = *vol * rng.range(0.5, 1.0);
                let f = *hz * rng.range(0.85, 1.15);
                *stage = 1;
                Some(Event::Thud { at: *t, hz: f, gain: *gain, pan: *pan })
            }
            Sched::Creaks { hz, vol, every, t, end, after } => {
                if *after {
                    *t += *every * rng.range(0.6, 1.6);
                }
                if *t >= *end {
                    return None;
                }
                *after = true;
                let length = rng.range(0.45, 1.3);
                let pan = rng.range(-0.8, 0.8);
                let gain = *vol * rng.range(0.5, 1.0) * 3.0;
                let glide = rng.range(-0.18, 0.18);
                let rate = rng.range(25.0, 70.0);
                let start = (*t * synth_rate()) as usize;
                let count = (length * synth_rate()) as usize;
                Some(Event::Creak { start, count, hz: *hz, glide, rate, gain, pan })
            }
            Sched::Groans { hz, vol, every, t, end, after } => {
                if *after {
                    *t += *every * rng.range(0.6, 1.5);
                }
                if *t >= *end {
                    return None;
                }
                *after = true;
                let length = rng.range(1.1, 2.5);
                let pan = rng.range(-0.6, 0.6);
                let gain = *vol * rng.range(0.6, 1.0);
                let f_start = *hz * rng.range(0.85, 1.15);
                let f_end = f_start * rng.range(0.8, 1.25);
                let start = (*t * synth_rate()) as usize;
                let count = (length * synth_rate()) as usize;
                Some(Event::Groan { start, count, hz: *hz, f_start, f_end, gain, pan })
            }
            Sched::Drips { hz, vol, low, seconds, i, n } => {
                if *i >= *n {
                    return None;
                }
                *i += 1;
                let at = rng.range(0.0, *seconds - 0.06);
                let f0 = rng.range(low.ln(), hz.ln()).exp();
                let decay = rng.range(0.012, 0.04) * (600.0 / f0);
                let gain = *vol * rng.range(0.2, 1.0) * 2.0;
                let pan = rng.range(-0.7, 0.7);
                let start = (at * synth_rate()) as usize;
                let count = ((decay * 5.0 * synth_rate()) as usize).min((0.08 * synth_rate()) as usize);
                Some(Event::Drip { start, count, f0, decay, gain, pan })
            }
            Sched::Crackle { hz, vol, seconds, i, n, c, clicks, at, pan } => loop {
                if *c < *clicks {
                    let when = *at + *c as f64 * rng.range(0.008, 0.03);
                    let f = *hz * rng.range(0.7, 1.3);
                    let g = *vol * rng.randf().powi(2) * 3.0;
                    *c += 1;
                    return Some(Event::Tick { at: when, hz: f, length: 0.02, gain: g, pan: *pan });
                }
                if *i >= *n {
                    return None;
                }
                *i += 1;
                *at = rng.range(0.0, *seconds - 0.3);
                *pan = rng.range(-0.8, 0.8);
                *clicks = if rng.randf() < 0.7 { 1 } else { rng.int(3, 6) };
                *c = 0;
            },
        }
    }
}

/// One layer's synthesis in progress.
enum Job {
    Tone(Tone),
    Drone(Drone),
    Wind(Wind),
    /// An event layer: its room tone first (water), then its events; `pending` is an event drawn but not yet written.
    Events { tone: Option<Tone>, sched: Sched, pending: Option<Event> },
    /// A kind with no synthesis.
    Nothing,
}

impl Job {
    /// The job of one layer (kind plus numbers). Its set-up draws come first, as in a one-pass render.
    fn new(b: &Buf, layer: &Value, rng: &mut Rng) -> Job {
        let events = |sched: Sched| Job::Events { tone: None, sched, pending: None };
        let seconds = b.seconds();
        match layer.get("kind").and_then(Value::as_str).unwrap_or("hum") {
            "noise" => Job::Tone(Tone::new(num(layer, "hz", 200.0), num(layer, "vol", 0.005), num(layer, "swell", 0.0), rng)),
            "hum" => Job::Drone(Drone::new(layer, rng)),
            "wind" => Job::Wind(Wind::new(b, layer, rng)),
            "pulse" => {
                let every = num(layer, "every", 1.5).max(0.3);
                let length = num(layer, "len", 0.1).clamp(0.03, 1.2);
                let hz = num(layer, "hz", 800.0).clamp(60.0, 2400.0);
                let vol = num(layer, "vol", 0.006);
                let t = rng.range(0.0, every);
                events(Sched::Details { hz, vol, every, length, t, end: seconds - length - 0.05, after: false })
            }
            "chirp" => {
                let hz = num(layer, "hz", 2600.0).clamp(200.0, 3200.0);
                let vol = num(layer, "vol", 0.004) * 0.6;
                let n = (num(layer, "count", 3.0) as i64 * 2).max(0);
                events(Sched::Chirps { hz, vol, seconds, i: 0, n, k: 0, kn: 0, at: 0.0, pan: 0.0 })
            }
            "thump" => {
                let hz = num(layer, "hz", 50.0).clamp(25.0, 200.0);
                let vol = num(layer, "vol", 0.02);
                let every = num(layer, "every", 4.0).max(0.8);
                let t = rng.range(0.2, every);
                events(Sched::Thumps { hz, vol, every, t, end: seconds - 0.8, stage: 0, gain: 0.0, pan: 0.0 })
            }
            "creak" => {
                let hz = num(layer, "hz", 420.0).clamp(120.0, max_tone_hz());
                let vol = num(layer, "vol", 0.008);
                let every = num(layer, "every", 3.5).max(1.0);
                let t = rng.range(0.3, every);
                events(Sched::Creaks { hz, vol, every, t, end: seconds - 1.6, after: false })
            }
            "groan" => {
                let hz = num(layer, "hz", 90.0).clamp(40.0, 400.0);
                let vol = num(layer, "vol", 0.008);
                let every = num(layer, "every", 6.0).max(2.0);
                let t = rng.range(0.5, every);
                events(Sched::Groans { hz, vol, every, t, end: seconds - 2.8, after: false })
            }
            "water" => {
                let hz = num(layer, "hz", 1400.0).clamp(300.0, 3000.0);
                let vol = num(layer, "vol", 0.01);
                let density = num(layer, "density", 40.0).clamp(2.0, 200.0);
                let low = num(layer, "low", 300.0).max(20.0);
                let tone = Tone::new((hz * 0.5).min(max_tone_hz()), vol * 0.35, num(layer, "swell", 0.0), rng);
                let n = (seconds * density) as usize;
                Job::Events { tone: Some(tone), sched: Sched::Drips { hz, vol, low, seconds, i: 0, n }, pending: None }
            }
            "crackle" => {
                let hz = num(layer, "hz", 2200.0).clamp(400.0, 3200.0);
                let vol = num(layer, "vol", 0.006);
                let rate = num(layer, "rate", 3.0).clamp(0.2, 40.0);
                let n = (seconds * rate) as usize;
                events(Sched::Crackle { hz, vol, seconds, i: 0, n, c: 0, clicks: 0, at: 0.0, pan: 0.0 })
            }
            _ => Job::Nothing,
        }
    }

    /// Synthesises up to `budget` samples of work: whole events, the first always (it is never longer than the chunk),
    /// a further one only while it fits. Returns the samples worked and whether the layer is done.
    fn run(&mut self, b: &mut Buf, rng: &mut Rng, budget: usize) -> (usize, bool) {
        match self {
            Job::Tone(t) => t.run(b, rng, budget),
            Job::Drone(d) => d.run(b, rng, budget),
            Job::Wind(w) => w.run(b, rng, budget),
            Job::Nothing => (0, true),
            Job::Events { tone, sched, pending } => {
                let mut worked = 0;
                if let Some(t) = tone.as_mut() {
                    let (w, done) = t.run(b, rng, budget);
                    worked += w;
                    if !done {
                        return (worked, false);
                    }
                    *tone = None;
                }
                loop {
                    if pending.is_none() {
                        *pending = sched.next(rng);
                    }
                    let Some(e) = pending.as_ref() else { return (worked, true) };
                    let n = e.frames();
                    if worked > 0 && worked + n > budget {
                        return (worked, false);
                    }
                    e.run(b, rng);
                    *pending = None;
                    worked += n;
                }
            }
        }
    }
}

fn thud(b: &mut Buf, at: f64, hz: f64, gain: f64, pan: f64, rng: &mut Rng) {
    let start = (at * synth_rate()) as usize;
    let (lg, rg) = gains(pan);
    let a = 1.0 - (-TAU * 180.0 / synth_rate()).exp();
    let (mut lp, mut phase) = (0.0, 0.0);
    for k in 0..(0.6 * synth_rate()) as usize {
        let i = start + k;
        if i >= b.len() {
            break;
        }
        let tt = k as f64 / synth_rate();
        let f = hz * (1.0 + 0.6 * (-tt * 18.0).exp());
        phase += f / synth_rate();
        let body = (TAU * phase).sin() * (-tt * 7.0).exp();
        let knock = rng.range(-1.0, 1.0) * (-tt * 40.0).exp() * 0.6;
        lp += (body + knock - lp) * a;
        b.add(i, lp * gain * lg, lp * gain * rg);
    }
}

#[allow(clippy::too_many_arguments)]
fn creak(b: &mut Buf, start: usize, count: usize, hz: f64, glide: f64, rate: f64, gain: f64, pan: f64, rng: &mut Rng) {
    let (lg, rg) = gains(pan);
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
            next_pulse = k as f64 + synth_rate() / (rate * (1.0 + 0.5 * (PI * p).sin()) * rng.range(0.8, 1.2));
        }
        let mut y = 0.0;
        for m in 0..2 {
            let f0 = hz * if m == 1 { 2.31 } else { 1.0 } * (1.0 + glide * p);
            let r = 0.992;
            let c = 2.0 * r * (TAU * f0 / synth_rate()).cos();
            let v = x * (1.0 - r) * 10.0 + c * res[m][0] - r * r * res[m][1];
            res[m] = [v, res[m][0]];
            y += v * if m == 1 { 0.6 } else { 1.0 };
        }
        let env = (PI * p).sin();
        b.add(i, y * gain * env * lg, y * gain * env * rg);
    }
}

#[allow(clippy::too_many_arguments)]
fn groan(b: &mut Buf, start: usize, count: usize, hz: f64, f_start: f64, f_end: f64, gain: f64, pan: f64) {
    let (lg, rg) = gains(pan);
    let a = 1.0 - (-TAU * (hz * 4.0).min(900.0) / synth_rate()).exp();
    let (mut lp1, mut lp2, mut phase) = (0.0, 0.0, 0.0f64);
    for k in 0..count {
        let i = start + k;
        if i >= b.len() {
            break;
        }
        let p = k as f64 / count as f64;
        let f = (f_start + (f_end - f_start) * p) * (1.0 + 0.012 * (TAU * 5.5 * k as f64 / synth_rate()).sin());
        phase = (phase + f / synth_rate()) % 1.0;
        lp1 += (phase * 2.0 - 1.0 - lp1) * a;
        lp2 += (lp1 - lp2) * a;
        let env = (PI * p).sin().powf(1.5);
        b.add(i, lp2 * gain * env * lg, lp2 * gain * env * rg);
    }
}

fn drip(b: &mut Buf, start: usize, count: usize, f0: f64, decay: f64, gain: f64, pan: f64) {
    let (lg, rg) = gains(pan);
    let mut phase = 0.0;
    for k in 0..count {
        let i = start + k;
        if i >= b.len() {
            break;
        }
        let tt = k as f64 / synth_rate();
        phase += f0 * (1.0 + tt / decay * 0.25) / synth_rate();
        let v = (TAU * phase).sin() * (-tt / decay).exp() * gain;
        b.add(i, v * lg, v * rg);
    }
}

fn tick(b: &mut Buf, at: f64, hz: f64, length: f64, gain: f64, pan: f64, rng: &mut Rng) {
    let start = (at * synth_rate()) as usize;
    let count = ((length + 0.25) * synth_rate()) as usize;
    let r = (-1.0 / (length * synth_rate() * 0.5)).exp();
    let c = 2.0 * r * (TAU * hz / synth_rate()).cos();
    let (mut y1, mut y2) = (0.0, 0.0);
    let (lg, rg) = gains(pan);
    let burst = (0.004 * synth_rate()) as usize;
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
    let start = (at * synth_rate()) as usize;
    let count = (length * synth_rate()) as usize;
    let (lg, rg) = gains(pan);
    let a = 1.0 - (-TAU * (hz * 2.0).min(max_tone_hz() * 2.0) / synth_rate()).exp();
    let mut lp = 0.0;
    for k in 0..count {
        let i = start + k;
        if i >= b.len() {
            break;
        }
        let p = k as f64 / count as f64;
        let env = (PI * p).sin().powi(2);
        let phase = hz * k as f64 / synth_rate();
        let tri = 4.0 * (phase - (phase + 0.5).floor()).abs() - 1.0;
        lp += (tri - lp) * a;
        b.add(i, lp * gain * env * lg, lp * gain * env * rg);
    }
}

// ---- finishing ----

/// The small Schroeder reverb over one channel (three damped combs, then an all-pass), resumable at any sample: each
/// sample reads only its own dry value, so it is worked in place a chunk at a time.
struct Verb {
    lines: [Vec<f32>; 3],
    idx: [usize; 3],
    damp: [f32; 3],
    aline: Vec<f32>,
    aidx: usize,
    wet: f64,
    feedback: f64,
    i: usize,
}

impl Verb {
    fn new(wet: f64, feedback: f64, offset: usize) -> Verb {
        let line = |d0: usize| vec![0.0f32; (d0 / 2).max(1)];
        Verb {
            lines: [line(1116 + offset), line(1356 + offset), line(1617 + offset)],
            idx: [0; 3],
            damp: [0.0; 3],
            aline: vec![0.0f32; 278 + offset],
            aidx: 0,
            wet,
            feedback,
            i: 0,
        }
    }

    fn run(&mut self, buf: &mut [f32], budget: usize) -> (usize, bool) {
        let (from, to) = (self.i, (self.i + budget).min(buf.len()));
        let ad = self.aline.len();
        for i in from..to {
            let dry = buf[i];
            let mut out = 0.0f32;
            for c in 0..3 {
                let d = self.lines[c].len();
                let y = self.lines[c][self.idx[c]];
                self.damp[c] += (y - self.damp[c]) * 0.45;
                self.lines[c][self.idx[c]] = dry + self.damp[c] * self.feedback as f32;
                self.idx[c] = (self.idx[c] + 1) % d;
                out += y / 3.0;
            }
            let bv = self.aline[self.aidx];
            let y = -out * 0.5 + bv;
            self.aline[self.aidx] = out + bv * 0.5;
            self.aidx = (self.aidx + 1) % ad;
            buf[i] = dry * (1.0 - self.wet as f32 * 0.5) + y * self.wet as f32;
        }
        self.i = to;
        (to - from, to >= buf.len())
    }
}

/// The DC-blocking one-pole high-pass over one channel, resumable at any sample.
struct HighPass {
    r: f32,
    x1: f32,
    y1: f32,
    i: usize,
}

impl HighPass {
    fn new() -> HighPass {
        HighPass { r: (-TAU * dc_cutoff_hz() / synth_rate()).exp() as f32, x1: 0.0, y1: 0.0, i: 0 }
    }

    fn run(&mut self, buf: &mut [f32], budget: usize) -> (usize, bool) {
        let (from, to) = (self.i, (self.i + budget).min(buf.len()));
        for s in &mut buf[from..to] {
            let x = *s;
            let y = x - self.x1 + self.r * self.y1;
            self.x1 = x;
            self.y1 = y;
            *s = y;
        }
        self.i = to;
        (to - from, to >= buf.len())
    }
}

/// Folds samples `from..to` of the tail (past `loop_len`) into the head, so the loop has no seam.
fn fold_tail(buf: &mut [f32], loop_len: usize, from: usize, to: usize) {
    let fade = buf.len() - loop_len;
    for k in from..to {
        let p = k as f64 / fade as f64;
        buf[k] = buf[k] * (p * PI * 0.5).sin() as f32 + buf[loop_len + k] * (p * PI * 0.5).cos() as f32;
    }
}

/// One bed rendered a chunk at a time, so a browser frame never stalls on the whole synth: each layer (`Job`), then the
/// two reverbs, the two high-passes, the tail fold and the loudness sum, at most `render.synth_chunk` synth samples of
/// work a step; then the up-sample in chunks of `render.upsample_chunk` output frames. `finish` runs every step at
/// once, giving the same samples as stepping.
pub struct BedRender {
    bed: Option<&'static Bed>,
    loop_len: usize,
    out_len: usize,
    b: Buf,
    rng: Rng,
    stage: usize,
    /// The layer being synthesised, the reverb or high-pass being run, and the sample reached by the fold or the sum.
    job: Option<Job>,
    verb: Option<Verb>,
    hp: Option<HighPass>,
    cursor: usize,
    sum: f64,
    gain: f32,
    out: Vec<[f32; 2]>,
    /// Steps run so far (tests).
    pub stages_done: usize,
    /// Samples the last step worked (synth samples, or output frames for the up-sample) (tests).
    pub last_work: usize,
}

impl BedRender {
    pub fn new(region: &str, seconds: f32) -> Self {
        let s = settings();
        let id = bed_for(region);
        let out_len = ((seconds.max(0.0) as f64) * SAMPLE_RATE as f64).round().max(1.0) as usize;
        let bed = s.beds.get(id);
        let loop_len = ((seconds.max(0.1) as f64) * synth_rate()) as usize;
        let total = if bed.is_some() { loop_len + (s.crossfade_seconds as f64 * synth_rate()) as usize } else { 0 };
        BedRender {
            bed,
            loop_len,
            out_len,
            b: Buf { l: vec![0.0; total], r: vec![0.0; total] },
            rng: Rng::new(godot_string_hash(id) as u64),
            stage: 0,
            job: None,
            verb: None,
            hp: None,
            cursor: 0,
            sum: 0.0,
            gain: 1.0,
            out: if bed.is_some() { Vec::with_capacity(out_len) } else { vec![[0.0, 0.0]; out_len] },
            stages_done: 0,
            last_work: 0,
        }
    }

    /// Output frames up-sampled so far (tests).
    pub fn frames_ready(&self) -> usize {
        self.out.len()
    }

    pub fn is_done(&self) -> bool {
        match self.bed {
            None => true,
            Some(_) => self.out.len() >= self.out_len,
        }
    }

    /// Whether the next step is the up-sample (its work is counted in output frames, `render.upsample_chunk`) (tests).
    pub fn upsampling(&self) -> bool {
        self.bed.is_some_and(|bed| self.stage >= bed.layers.len() + 6)
    }

    /// Runs one step: at most `render.synth_chunk` samples of a layer or a finishing pass, or one up-sample chunk.
    /// Returns true when the bed is finished.
    pub fn step(&mut self) -> bool {
        let Some(bed) = self.bed else { return true };
        if self.is_done() {
            return true;
        }
        self.stages_done += 1;
        let n = bed.layers.len();
        let s = settings();
        let chunk = s.synth_chunk;
        let st = self.stage;
        let (worked, done) = if st < n {
            let (b, rng) = (&mut self.b, &mut self.rng);
            let job = self.job.get_or_insert_with(|| Job::new(b, &bed.layers[st], rng));
            let r = job.run(b, rng, chunk);
            if r.1 {
                self.job = None;
            }
            r
        } else {
            let (wet, fb) = s.spaces.get(&bed.space).or_else(|| s.spaces.get("medium")).copied().unwrap_or((0.18, 0.68));
            match st - n {
                k @ (0 | 1) => {
                    let buf = if k == 0 { &mut self.b.l } else { &mut self.b.r };
                    let v = self.verb.get_or_insert_with(|| Verb::new(wet, fb, if k == 0 { 0 } else { 7 }));
                    let r = v.run(buf, chunk);
                    if r.1 {
                        self.verb = None;
                    }
                    r
                }
                k @ (2 | 3) => {
                    let buf = if k == 2 { &mut self.b.l } else { &mut self.b.r };
                    let hp = self.hp.get_or_insert_with(HighPass::new);
                    let r = hp.run(buf, chunk);
                    if r.1 {
                        self.hp = None;
                    }
                    r
                }
                4 => {
                    // both channels are folded together, so a step works up to a chunk of samples in all
                    let fade = self.b.len() - self.loop_len;
                    let (from, to) = (self.cursor, (self.cursor + (chunk / 2).max(1)).min(fade));
                    fold_tail(&mut self.b.l, self.loop_len, from, to);
                    fold_tail(&mut self.b.r, self.loop_len, from, to);
                    self.cursor = to;
                    (2 * (to - from), to >= fade)
                }
                5 => {
                    let ll = self.loop_len;
                    let (from, to) = (self.cursor, (self.cursor + chunk).min(ll));
                    for i in from..to {
                        self.sum += ((self.b.l[i] + self.b.r[i]) as f64 * 0.5).powi(2);
                    }
                    self.cursor = to;
                    if to >= ll {
                        let rms = (self.sum / ll as f64).sqrt();
                        self.gain = if rms > 1e-9 { (10f64.powf(s.target_rms_db / 20.0) / rms) as f32 } else { 1.0 };
                    }
                    (to - from, to >= ll)
                }
                _ => {
                    // up-sample 22.05 -> 44.1 kHz by linear interpolation, wrapping at the loop point so the seam stays closed
                    let ll = self.loop_len;
                    let at = |v: &[f32], pos: f64| -> f32 {
                        let i = pos.floor() as usize;
                        let f = (pos - i as f64) as f32;
                        v[i % ll] * (1.0 - f) + v[(i + 1) % ll] * f
                    };
                    let from = self.out.len();
                    let to = (from + s.upsample_chunk).min(self.out_len);
                    for k in from..to {
                        let pos = k as f64 * synth_rate() / SAMPLE_RATE as f64;
                        self.out.push([at(&self.b.l, pos) * self.gain, at(&self.b.r, pos) * self.gain]);
                    }
                    self.last_work = to - from;
                    if self.out.len() >= self.out_len {
                        // the working buffers are no longer needed
                        self.b = Buf { l: Vec::new(), r: Vec::new() };
                    }
                    return self.is_done();
                }
            }
        };
        self.last_work = worked;
        if done {
            self.stage += 1;
            self.cursor = 0;
        }
        false
    }

    /// Runs steps until `budget` is spent (always at least one). Returns true when finished.
    pub fn step_for(&mut self, budget: std::time::Duration) -> bool {
        let t0 = crate::shared::clock::Instant::now();
        loop {
            if self.step() {
                return true;
            }
            if t0.elapsed() >= budget {
                return false;
            }
        }
    }

    pub fn finish(mut self) -> Vec<[f32; 2]> {
        while !self.step() {}
        self.out
    }
}

/// One bed as a seamless loop of `seconds` at the engine's rate, or silence for a name with no bed at all.
pub fn ambience(region: &str, seconds: f32) -> Vec<[f32; 2]> {
    BedRender::new(region, seconds).finish()
}

/// Rendered beds, kept least-recently-used (`render.cache_beds`), rendered a time-budgeted slice a frame, and the bed
/// the caller should play: the previous one until the wanted one is ready (no stall, no silence on a region change).
#[derive(Default)]
pub struct BedStreamer {
    /// Oldest first, most recently used last.
    cache: Vec<(&'static str, Vec<[f32; 2]>)>,
    pending: Option<(&'static str, BedRender)>,
    shown: Option<&'static str>,
}

impl BedStreamer {
    fn touch(&mut self, id: &'static str) -> bool {
        match self.cache.iter().position(|(k, _)| *k == id) {
            Some(i) => {
                let e = self.cache.remove(i);
                self.cache.push(e);
                true
            }
            None => false,
        }
    }

    pub fn is_cached(&self, id: &str) -> bool {
        self.cache.iter().any(|(k, _)| *k == id)
    }

    /// Cached bed ids, least recently used first.
    pub fn cached_ids(&self) -> Vec<&'static str> {
        self.cache.iter().map(|(k, _)| *k).collect()
    }

    /// Stores a finished bed, evicting the least recently used beyond the data's cache size.
    pub fn insert(&mut self, id: &'static str, frames: Vec<[f32; 2]>) {
        self.cache.retain(|(k, _)| *k != id);
        self.cache.push((id, frames));
        let cap = settings().cache_beds;
        while self.cache.len() > cap {
            self.cache.remove(0);
        }
    }

    /// The frames of a cached bed (marks it recently used).
    pub fn frames(&mut self, id: &'static str) -> Option<Vec<[f32; 2]>> {
        if self.touch(id) {
            self.cache.last().map(|(_, f)| f.clone())
        } else {
            None
        }
    }

    /// Renders `want` into the cache for up to `budget` this call (the loading screen can warm the spawn region with it).
    /// True when it is cached.
    pub fn warm(&mut self, want: &'static str, budget: std::time::Duration) -> bool {
        if self.touch(want) {
            return true;
        }
        if self.pending.as_ref().map(|(k, _)| *k) != Some(want) {
            self.pending = Some((want, BedRender::new(want, settings().loop_seconds)));
        }
        let done = self.pending.as_mut().map(|(_, r)| r.step_for(budget)).unwrap_or(false);
        if done {
            if let Some((id, r)) = self.pending.take() {
                self.insert(id, r.finish());
            }
        }
        done
    }

    /// One game frame: `want` is the bed id the region plays (`None`: silence). Returns the bed to play, which stays the
    /// previous one while the wanted bed renders.
    pub fn update(&mut self, want: Option<&'static str>, budget: std::time::Duration) -> Option<&'static str> {
        match want {
            None => {
                self.pending = None;
                self.shown = None;
            }
            Some(id) => {
                if self.warm(id, budget) {
                    self.shown = Some(id);
                } else if let Some(prev) = self.shown {
                    self.touch(prev); // the bed still playing must not be evicted
                }
            }
        }
        self.shown
    }

    pub fn shown(&self) -> Option<&'static str> {
        self.shown
    }
}

/// The per-frame render budget (`render.bed_budget_ms`).
pub fn bed_budget() -> std::time::Duration {
    std::time::Duration::from_secs_f64(settings().bed_budget_ms / 1000.0)
}

// ---- crossfade between regions ----

/// Two ambience slots and the rules that crossfade them when the region changes. Pure: the caller starts and stops the
/// loops the output names. Fade-in and fade-out are equal-power (`sqrt` of the slot's level) so the sum stays even.
#[derive(Clone, Debug, Default)]
pub struct Mixer {
    slots: [Slot; 2],
    active: usize,
    /// The region text the bed was last looked up for, and that bed (`wanted_bed`): a frame in the same region does
    /// not lowercase or look the region up again.
    region_raw: Option<String>,
    bed: &'static str,
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

    /// The bed id `region` plays (`None` for no region, or a region with no bed). The lookup (`bed_for`) runs only when
    /// the region text changes, so a frame in the same region costs a string compare, not a lowercase.
    pub fn wanted_bed(&mut self, region: Option<&str>) -> Option<&'static str> {
        let r = region?;
        if self.region_raw.as_deref() != Some(r) {
            self.region_raw = Some(r.to_string());
            self.bed = bed_for(r);
        }
        Some(self.bed).filter(|id| !id.is_empty())
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


#[cfg(test)]
mod tests {
    use super::*;

    /// The region's bed is looked up when the region text changes; the same text again answers from the cache.
    #[test]
    fn the_bed_is_looked_up_once_per_region_change() {
        let mut m = Mixer::default();
        assert_eq!(m.wanted_bed(None), None);
        assert_eq!(m.wanted_bed(Some("Maze")), Some("maze"), "the lookup ignores case");
        // the cached bed answers the same region (a sentinel proves no lookup ran)
        m.bed = "sentinel";
        assert_eq!(m.wanted_bed(Some("Maze")), Some("sentinel"), "a frame in the same region is a cache hit");
        // a new region is looked up again
        assert_eq!(m.wanted_bed(Some("ice")), Some(bed_for("ice")));
        assert_eq!(m.wanted_bed(None), None, "no region is silence");
    }
}
