//! Shared Night Watch framework: the 2D game's seeded integer streams and Zellia's call state machine.
//! Ported from `night_watch_mulberry.gd`, `night_watch_stream.gd` and `night_watch_call_script.gd`.

use super::data::{CallsData, CALL_INCOMING_MS, CALL_INCOMING_TEXT};
use super::typewriter::{ui, Typewriter};

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
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Source {
    Seeded,
    Neutral,
}

/// A draw source: seeded, or the neutral stream that answers 0.5, so every
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

/// A call: rings `at` seconds into the night, then types each line letter by letter (see `typewriter.rs`); a finished
/// line stays up for its hold time and the box closes after the last line. Pure: feed it the shift clock and draw
/// `text` (under `speaker_name`) while `open`. Each revealed letter is one `Talk` event and one entry of `blip_letters`.
#[derive(Clone, Debug)]
pub struct CallScript {
    pub open: bool,
    pub done: bool,
    pub muted: bool,
    pub text: String,
    pub line: i32,
    /// The letters that blipped in the last `update`, in order (empty while silent).
    pub blip_letters: Vec<char>,
    lines: Vec<String>,
    speakers: Vec<String>,
    at_ms: f64,
    active: bool,
    silent: bool,
    t: f64,
    speaker: String,
    typed: Option<Typewriter>,
}

impl CallScript {
    pub fn new(lines: Vec<String>, at_seconds: f64) -> Self {
        Self {
            open: false,
            done: lines.is_empty(),
            muted: false,
            text: String::new(),
            line: -1,
            blip_letters: Vec::new(),
            lines,
            speakers: Vec::new(),
            at_ms: at_seconds * 1000.0,
            active: false,
            silent: false,
            t: 0.0,
            speaker: ui().default_speaker.clone(),
            typed: None,
        }
    }

    /// Who says each line (parallel to the lines; an empty or missing id is the default speaker).
    pub fn with_speakers(mut self, speakers: Vec<String>) -> Self {
        self.speakers = speakers;
        self
    }

    pub fn for_level(calls: &CallsData, mode_id: &str, level: i32) -> Self {
        match calls.for_level(mode_id, level) {
            Some(entry) => Self::new(entry.lines.clone(), f64::from(entry.at)).with_speakers(entry.speakers.clone()),
            None => Self::new(Vec::new(), 6.0),
        }
    }

    pub fn update(&mut self, elapsed_ms: f64, dt_ms: f64) -> Vec<CallEvent> {
        let mut events = Vec::new();
        self.blip_letters.clear();
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
        // The frame that ends the ring only has its remainder left to type with.
        let type_ms = if self.t - dt_ms < 0.0 { self.t } else { dt_ms };
        if self.typed.as_ref().map_or(true, Typewriter::expired) && !self.next_line() {
            self.done = true;
            self.open = false;
            events.push(CallEvent::Close);
            return events;
        }
        if let Some(typed) = self.typed.as_mut() {
            let step = typed.advance(type_ms);
            self.text = step.shown.to_string();
            if !self.silent {
                events.extend(std::iter::repeat(CallEvent::Talk).take(step.blips as usize));
                self.blip_letters = step.blip_letters;
            }
        }
        events
    }

    /// Starts the next line's typewriter; false when the lines have run out.
    fn next_line(&mut self) -> bool {
        self.line += 1;
        let Some(text) = self.lines.get(self.line as usize) else { return false };
        self.typed = Some(Typewriter::new(text, &ui().typewriter));
        self.speaker = match self.speakers.get(self.line as usize) {
            Some(id) if !id.is_empty() => id.clone(),
            _ => ui().default_speaker.clone(),
        };
        self.text.clear();
        true
    }

    /// A key press: shows the rest of the line at once, or, when it is all there, moves on to the next line.
    /// True when it did something.
    pub fn skip(&mut self) -> bool {
        if self.done {
            return false;
        }
        let Some(typed) = self.typed.as_mut() else { return false };
        let did = if typed.is_done() { typed.expire() } else { typed.skip() };
        self.text = typed.shown().to_string();
        did
    }

    /// Text without blips: the lines still type and show, but nothing sounds. (`mute` is the other thing: it ends the call.)
    #[cfg(test)]
    pub fn set_silent(&mut self, silent: bool) {
        self.silent = silent;
    }

    /// The speaker's id of the line in progress (the default speaker before the first line).
    pub fn speaker(&self) -> &str {
        &self.speaker
    }

    /// The name for the caller box.
    pub fn speaker_name(&self) -> &str {
        &ui().voice(&self.speaker).name
    }

    /// The whole line in progress (what `text` is typing towards); empty before the first line.
    pub fn full_text(&self) -> &str {
        self.typed.as_ref().map_or("", Typewriter::full_text)
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

#[cfg(test)]
mod call_tests {
    use super::*;

    fn script(lines: &[&str]) -> CallScript {
        CallScript::new(lines.iter().map(|l| l.to_string()).collect(), 1.0)
    }

    /// Runs the script in `dt` steps until it closes; the talk count and the ms taken.
    fn run_to_end(c: &mut CallScript, dt: f64) -> (u32, f64) {
        let (mut talks, mut now) = (0, 1000.0 - dt);
        while !c.done && now < 600_000.0 {
            now += dt;
            talks += c.update(now, dt).iter().filter(|e| **e == CallEvent::Talk).count() as u32;
        }
        (talks, now)
    }

    #[test]
    fn nothing_happens_before_the_ring_time() {
        let mut c = script(&["Hi."]);
        assert!(c.update(999.0, 16.0).is_empty());
        assert!(!c.open);
    }

    #[test]
    fn it_rings_then_types_letter_by_letter() {
        let mut c = script(&["Hello"]);
        let ev = c.update(1000.0, 16.0);
        assert_eq!(ev, vec![CallEvent::Open, CallEvent::Ring]);
        assert_eq!(c.text, CALL_INCOMING_TEXT);
        // the ring ends 100 ms into this frame: letters come about 29 ms apart, so a few show, never the whole word
        c.update(1016.0, 2284.0);
        assert!(!c.text.is_empty() && c.text.len() < 5, "{}", c.text);
        assert!("Hello".starts_with(&c.text));
    }

    #[test]
    fn talk_events_and_blip_letters_match_the_revealed_letters() {
        let mut c = script(&["Hi, ok. Go"]);
        let (talks, _) = run_to_end(&mut c, 7.0);
        assert_eq!(talks, 6); // H i o k G o
        assert!(c.done && !c.open);
        let mut c = script(&["Hi, ok. Go"]);
        let mut letters = Vec::new();
        let mut now = 1000.0;
        while !c.done {
            now += 50.0;
            let ev = c.update(now, 50.0);
            assert_eq!(ev.iter().filter(|e| **e == CallEvent::Talk).count(), c.blip_letters.len());
            letters.extend(c.blip_letters.clone());
        }
        assert_eq!(letters.iter().collect::<String>(), "HiokGo");
    }

    #[test]
    fn the_blip_count_does_not_depend_on_the_frame_rate() {
        let line = "Zellia, on the headset. Can you hear me all right? Good.";
        let counts: Vec<u32> = [4.0, 16.0, 100.0, 700.0].iter().map(|dt| run_to_end(&mut script(&[line, "Again."]), *dt).0).collect();
        let letters = line.chars().chain("Again.".chars()).filter(|c| c.is_alphanumeric()).count() as u32;
        assert!(counts.iter().all(|n| *n == letters), "{counts:?} vs {letters}");
    }

    #[test]
    fn a_finished_line_holds_before_the_next_one_starts() {
        let mut c = script(&["Ab", "Cd"]);
        let mut now = 1000.0;
        c.update(now, 16.0);
        // the ring, then both letters of the first line
        now += 2500.0;
        c.update(now, 2500.0);
        assert_eq!((c.line, c.text.as_str()), (0, "Ab"));
        // the hold (1200 + 140 for one word) has not run out yet
        now += 500.0;
        c.update(now, 500.0);
        assert_eq!(c.line, 0);
        now += 1500.0;
        c.update(now, 1500.0);
        c.update(now + 16.0, 16.0); // the frame after the hold ran out starts the next line
        assert_eq!(c.line, 1);
    }

    #[test]
    fn skip_completes_the_line_then_moves_on() {
        let mut c = script(&["Hello there", "Next"]);
        c.update(1000.0, 16.0);
        c.update(3300.0, 2300.0);
        assert!(c.skip());
        assert_eq!(c.text, "Hello there");
        let talks = c.update(3316.0, 16.0).iter().filter(|e| **e == CallEvent::Talk).count();
        assert_eq!(talks, 0);
        assert_eq!(c.line, 0);
        assert!(c.skip()); // all there: ends the hold
        c.update(3332.0, 16.0);
        assert_eq!(c.line, 1);
        assert!(!script(&["x"]).skip());
    }

    #[test]
    fn silent_shows_the_text_without_blips() {
        let mut c = script(&["Quiet please"]);
        c.set_silent(true);
        let (talks, _) = run_to_end(&mut c, 20.0);
        assert_eq!(talks, 0);
        assert!(c.blip_letters.is_empty());
        let mut c = script(&["Quiet please"]);
        c.set_silent(true);
        c.update(1000.0, 16.0);
        c.update(4000.0, 3000.0);
        assert_eq!(c.text, "Quiet please");
    }

    #[test]
    fn the_caller_box_names_the_speaker_of_each_line() {
        let mut c = script(&["One", "Two"]).with_speakers(vec![String::new(), "kathy".to_string()]);
        assert_eq!(c.speaker_name(), "Zellia");
        let mut now = 1000.0;
        c.update(now, 16.0);
        now += 3000.0;
        c.update(now, 3000.0);
        assert_eq!(c.speaker(), "zellia");
        assert_eq!(c.full_text(), "One");
        for _ in 0..40 {
            now += 100.0;
            c.update(now, 100.0);
        }
        assert_eq!(c.speaker_name(), "Kathy");
        assert_eq!(c.full_text(), "Two");
    }

    #[test]
    fn mute_still_ends_the_call() {
        let mut c = script(&["Hello"]);
        assert!(!c.mute());
        c.update(1000.0, 16.0);
        assert!(c.mute());
        assert!(c.done && c.muted && !c.open);
    }

    #[test]
    fn a_script_without_lines_is_already_done() {
        let mut c = script(&[]);
        assert!(c.done);
        assert!(c.update(5000.0, 16.0).is_empty());
    }
}
