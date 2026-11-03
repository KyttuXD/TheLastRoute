//! Shared Night Watch framework: the 2D game's seeded integer streams and Zellia's call state machine.
//! Ported from `night_watch_mulberry.gd`, `night_watch_stream.gd` and `night_watch_call_script.gd`.

use super::data::{CallsData, CALL_INCOMING_MS, CALL_INCOMING_TEXT, CALL_LINE_BASE_MS, CALL_LINE_WORD_MS, CALL_WORD_MS};

const MASK_SEED_SALT: u32 = 0x7a3c_19e5;
const STEP: u32 = 0x6d2b_79f5;

fn mulberry_step(a: &mut u32) -> f64 {
    *a = a.wrapping_add(STEP);
    let mut t = *a;
    t = (t ^ (t >> 15)).wrapping_mul(t | 1);
    t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
    f64::from(t ^ (t >> 14)) / 4_294_967_296.0
}

/// The 2D game's `mulberry32`, bit for bit. `next` is one draw in `[0, 1)`.
#[derive(Clone, Debug)]
pub struct Mulberry {
    a: u32,
}

impl Mulberry {
    pub fn new(seed: i64) -> Self {
        Self { a: seed as u32 }
    }

    pub fn next(&mut self) -> f64 {
        mulberry_step(&mut self.a)
    }

    pub fn state(&self) -> u32 {
        self.a
    }

    pub fn set_state(&mut self, v: u32) {
        self.a = v;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    Seeded,
    Fixed(f64),
    Neutral,
}

/// A draw source: seeded, a fixed answer (the rule tests' stubs), or the neutral stream that answers 0.5, so every
/// jitter is its base value, every chance fails and every pick is the first.
#[derive(Clone, Debug)]
pub struct NightWatchStream {
    a: u32,
    source: Source,
}

impl NightWatchStream {
    pub fn seeded(seed: i64) -> Self {
        Self { a: seed as u32, source: Source::Seeded }
    }

    pub fn fixed(value: f64) -> Self {
        Self { a: 0, source: Source::Fixed(value) }
    }

    pub fn neutral() -> Self {
        Self { a: 0, source: Source::Neutral }
    }

    pub fn is_neutral(&self) -> bool {
        self.source == Source::Neutral
    }

    pub fn has_state(&self) -> bool {
        self.source == Source::Seeded
    }

    pub fn next(&mut self) -> f64 {
        match self.source {
            Source::Seeded => mulberry_step(&mut self.a),
            Source::Fixed(v) => v,
            Source::Neutral => 0.5,
        }
    }

    pub fn state(&self) -> u32 {
        if self.has_state() {
            self.a
        } else {
            0
        }
    }

    pub fn set_state(&mut self, v: u32) {
        self.a = v;
    }

    /// `given` wins, else a stream derived from `rng`'s state (before the night's first draw), else neutral.
    pub fn variety_of(rng: &NightWatchStream, given: Option<NightWatchStream>) -> NightWatchStream {
        if let Some(g) = given {
            return g;
        }
        if rng.has_state() {
            return NightWatchStream::seeded(i64::from(rng.state() ^ MASK_SEED_SALT));
        }
        NightWatchStream::neutral()
    }
}

/// `base` plus or minus `spread` (a fraction, capped at 0.9), one draw.
pub fn jitter(base: f64, spread: f64, stream: &mut NightWatchStream) -> f64 {
    let k = spread.clamp(0.0, 0.9);
    base * (1.0 - k + 2.0 * k * stream.next())
}

/// `ms` plus or minus `spread`, evenly (the 2D default spread is 0.2).
pub fn spread_ms(ms: f64, rng: &mut NightWatchStream, spread: f64) -> f64 {
    ms * (1.0 - spread + 2.0 * spread * rng.next())
}

/// A pacer multiplier built so the wait is what is even.
pub fn tempo(vr: &mut NightWatchStream, spread: f64) -> f64 {
    1.0 / jitter(1.0, spread, vr)
}

pub fn chance(vr: &mut NightWatchStream, p: f64) -> bool {
    let u = vr.next();
    !vr.is_neutral() && u < p
}

pub fn pick_index(vr: &mut NightWatchStream, n: usize) -> usize {
    let u = vr.next();
    if vr.is_neutral() || n == 0 {
        0
    } else {
        (n - 1).min((u * n as f64).floor() as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallEvent {
    Open,
    Ring,
    Talk,
    Close,
}

/// Zellia's call: rings `at` seconds into the night, then types each line word by word; the box closes after the last line.
/// Pure: feed it the shift clock and draw `text` while `open`.
#[derive(Clone, Debug)]
pub struct CallScript {
    pub open: bool,
    pub done: bool,
    pub muted: bool,
    pub text: String,
    pub line: i32,
    lines: Vec<String>,
    at_ms: f64,
    active: bool,
    t: f64,
    dur: f64,
    words: Vec<String>,
    shown: usize,
}

impl CallScript {
    pub fn new(lines: Vec<String>, at_seconds: f64) -> Self {
        Self {
            open: false,
            done: lines.is_empty(),
            muted: false,
            text: String::new(),
            line: -1,
            lines,
            at_ms: at_seconds * 1000.0,
            active: false,
            t: 0.0,
            dur: 0.0,
            words: Vec::new(),
            shown: 0,
        }
    }

    pub fn for_level(calls: &CallsData, mode_id: &str, level: i32) -> Self {
        match calls.for_level(mode_id, level) {
            Some(entry) => Self::new(entry.lines.clone(), f64::from(entry.at)),
            None => Self::new(Vec::new(), 6.0),
        }
    }

    pub fn update(&mut self, elapsed_ms: f64, dt_ms: f64) -> Vec<CallEvent> {
        let mut events = Vec::new();
        if self.done {
            return events;
        }
        if !self.active {
            if elapsed_ms < self.at_ms {
                return events;
            }
            self.active = true;
            self.t = -CALL_INCOMING_MS;
            self.text = CALL_INCOMING_TEXT.to_string();
            self.open = true;
            events.push(CallEvent::Open);
            events.push(CallEvent::Ring);
        }
        self.t += dt_ms;
        if self.t < 0.0 {
            return events;
        }
        if self.line == -1 || self.t >= self.dur {
            self.line += 1;
            self.t = 0.0;
            self.shown = 0;
            if self.line as usize >= self.lines.len() {
                self.done = true;
                self.open = false;
                events.push(CallEvent::Close);
                return events;
            }
            self.words = self.lines[self.line as usize].split(' ').map(str::to_string).collect();
            self.dur = CALL_LINE_BASE_MS + self.words.len() as f64 * CALL_LINE_WORD_MS;
        }
        let want = self.words.len().min((self.t / CALL_WORD_MS).floor() as usize);
        if want != self.shown {
            self.shown = want;
            self.text = self.words[..want].join(" ");
            if want % 2 == 1 {
                events.push(CallEvent::Talk);
            }
        }
        events
    }

    /// Ends the call; true when one was in progress.
    pub fn mute(&mut self) -> bool {
        if self.active && !self.done {
            self.done = true;
            self.open = false;
            self.muted = true;
            return true;
        }
        false
    }
}
