//! The call box's letter-by-letter text: a `Typewriter` reveals one letter at a time and says how many blips (one per
//! revealed letter) the frame owes. Pure logic, no window and no sound; `blip.rs` makes the sound, `calls_view.rs` draws.
//! Speed, punctuation pauses and hold time come from data/calls_ui.json (schema nw.calls_ui/1).

use super::data::{self, calls_ui};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

/// A pause after the characters in `chars`, added to the normal letter gap.
#[derive(Clone, Debug, PartialEq)]
pub struct Pause {
    pub chars: String,
    pub ms: f64,
}

/// How a line is typed and how long it stays up once it is all there.
#[derive(Clone, Debug, PartialEq)]
pub struct TypewriterRules {
    pub chars_per_second: f64,
    /// Extra wait after a space (0: the space appears with the next letter).
    pub space_ms: f64,
    pub pauses: Vec<Pause>,
    pub hold_base_ms: f64,
    pub hold_word_ms: f64,
}

impl TypewriterRules {
    pub fn from_value(v: &Value) -> Self {
        Self {
            chars_per_second: data::num(v, "chars_per_second").max(1.0),
            space_ms: data::num(v, "space_ms").max(0.0),
            pauses: data::list(v, "pauses")
                .iter()
                .map(|p| Pause { chars: data::text(p, "chars").to_string(), ms: data::num(p, "ms").max(0.0) })
                .collect(),
            hold_base_ms: data::num(v, "hold_base_ms").max(0.0),
            hold_word_ms: data::num(v, "hold_word_ms").max(0.0),
        }
    }

    /// The gap before the next character, after `c` was revealed.
    fn gap_after(&self, c: char) -> f64 {
        let letter_ms = 1000.0 / self.chars_per_second;
        if c == ' ' {
            return self.space_ms;
        }
        letter_ms + self.pauses.iter().find(|p| p.chars.contains(c)).map_or(0.0, |p| p.ms)
    }
}

/// True for the characters that get a blip: letters and digits. Spaces and punctuation stay silent.
pub fn blips(c: char) -> bool {
    c.is_alphanumeric()
}

/// What one `advance` revealed. `shown` is the whole revealed text so far.
#[derive(Clone, Debug, PartialEq)]
pub struct TypewriterStep<'a> {
    pub shown: &'a str,
    /// Letters newly revealed that should blip (the same count as `blip_letters`).
    pub blips: u32,
    pub blip_letters: Vec<char>,
    /// Every character is shown.
    pub done: bool,
}

#[derive(Clone, Debug)]
pub struct Typewriter {
    text: String,
    /// Byte offset just past each character, so `shown` slices on character boundaries.
    ends: Vec<usize>,
    chars: Vec<char>,
    rules: TypewriterRules,
    clock: f64,
    next_at: f64,
    last_reveal_at: f64,
    revealed: usize,
    hold_ms: f64,
}

impl Typewriter {
    pub fn new(text: &str, rules: &TypewriterRules) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let ends = text.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
        let words = text.split_whitespace().count() as f64;
        Self {
            text: text.to_string(),
            ends,
            chars,
            hold_ms: rules.hold_base_ms + words * rules.hold_word_ms,
            rules: rules.clone(),
            clock: 0.0,
            next_at: 0.0,
            last_reveal_at: 0.0,
            revealed: 0,
        }
    }

    /// Moves the clock `dt_ms` on and reveals every letter that came due: a slow frame reveals several, and each
    /// revealed letter is counted, so no blip is skipped.
    pub fn advance(&mut self, dt_ms: f64) -> TypewriterStep<'_> {
        self.clock += dt_ms.max(0.0);
        let mut letters = Vec::new();
        while self.revealed < self.chars.len() && self.next_at <= self.clock {
            let c = self.chars[self.revealed];
            self.revealed += 1;
            self.last_reveal_at = self.next_at;
            self.next_at += self.rules.gap_after(c);
            if blips(c) {
                letters.push(c);
            }
        }
        TypewriterStep { shown: self.shown(), blips: letters.len() as u32, blip_letters: letters, done: self.is_done() }
    }

    /// Shows the whole line at once (a key press), without blips. True when it revealed anything.
    pub fn skip(&mut self) -> bool {
        if self.is_done() {
            return false;
        }
        self.revealed = self.chars.len();
        self.last_reveal_at = self.clock;
        true
    }

    /// Ends the hold too, so the line is over. True when it was not already.
    pub fn expire(&mut self) -> bool {
        let was = self.expired();
        self.skip();
        self.last_reveal_at = self.clock - self.hold_ms;
        !was
    }

    pub fn shown(&self) -> &str {
        match self.revealed {
            0 => "",
            n => &self.text[..self.ends[n - 1]],
        }
    }

    pub fn full_text(&self) -> &str {
        &self.text
    }

    #[allow(dead_code)] // read by the tests; the shell will show the hold timer
    pub fn revealed_chars(&self) -> usize {
        self.revealed
    }

    pub fn is_done(&self) -> bool {
        self.revealed >= self.chars.len()
    }

    /// How long the finished line stays up, from its last letter.
    #[allow(dead_code)] // read by the tests; the shell will show the hold timer
    pub fn hold_ms(&self) -> f64 {
        self.hold_ms
    }

    /// All shown and the hold time has run out after the last letter.
    pub fn expired(&self) -> bool {
        self.is_done() && self.clock - self.last_reveal_at >= self.hold_ms
    }
}

/// How one speaker's blip sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Sine,
    Triangle,
}

#[derive(Clone, Debug)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub wave: Wave,
    pub pitch_hz: f64,
    pub volume: f64,
}

/// The shared blip sound settings.
#[derive(Clone, Debug)]
pub struct BlipRules {
    pub ms: f64,
    pub attack_ms: f64,
    pub sample_rate: u32,
    pub variants: usize,
    pub pitch_jitter: f64,
    pub volume: f64,
    pub master: f64,
}

/// The typed call box data.
#[derive(Clone, Debug)]
pub struct CallsUi {
    pub default_speaker: String,
    pub typewriter: TypewriterRules,
    pub blip: BlipRules,
    pub speakers: HashMap<String, Voice>,
}

impl CallsUi {
    pub fn from_value(v: &Value) -> Self {
        let b = &v["blip"];
        let speakers = v["speakers"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(id, s)| {
                        let wave = if data::text(s, "wave") == "sine" { Wave::Sine } else { Wave::Triangle };
                        let voice = Voice { id: id.clone(), name: data::text(s, "name").to_string(), wave, pitch_hz: data::num(s, "pitch_hz"), volume: data::num(s, "volume") };
                        (id.clone(), voice)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            default_speaker: data::text(v, "default_speaker").to_string(),
            typewriter: TypewriterRules::from_value(&v["typewriter"]),
            blip: BlipRules {
                ms: data::num(b, "ms").clamp(5.0, 200.0),
                attack_ms: data::num(b, "attack_ms").max(0.0),
                sample_rate: data::int(b, "sample_rate").clamp(8000, 48000) as u32,
                variants: data::int(b, "variants").clamp(1, 32) as usize,
                pitch_jitter: data::num(b, "pitch_jitter").clamp(0.0, 0.5),
                volume: data::num(b, "volume").clamp(0.0, 1.0),
                master: data::num(b, "master").clamp(0.0, 1.0),
            },
            speakers,
        }
    }

    /// The voice of `id`; an unknown or empty id gets the default speaker's voice.
    pub fn voice(&self, id: &str) -> &Voice {
        self.speakers
            .get(id)
            .or_else(|| self.speakers.get(&self.default_speaker))
            .unwrap_or_else(|| panic!("night_watch calls_ui: default speaker '{}' has no voice", self.default_speaker))
    }
}

/// The embedded call box data, typed, parsed once.
pub fn ui() -> &'static CallsUi {
    static UI: OnceLock<CallsUi> = OnceLock::new();
    UI.get_or_init(|| CallsUi::from_value(calls_ui()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> TypewriterRules {
        // 10 letters a second, easy to count.
        TypewriterRules {
            chars_per_second: 10.0,
            space_ms: 0.0,
            pauses: vec![Pause { chars: ",".into(), ms: 300.0 }, Pause { chars: ".".into(), ms: 500.0 }],
            hold_base_ms: 1000.0,
            hold_word_ms: 100.0,
        }
    }

    fn run(text: &str, dts: &[f64]) -> (String, u32) {
        let mut t = Typewriter::new(text, &rules());
        let mut total = 0;
        for dt in dts {
            total += t.advance(*dt).blips;
        }
        (t.shown().to_string(), total)
    }

    #[test]
    fn the_first_letter_shows_at_once_then_one_per_gap() {
        let mut t = Typewriter::new("abc", &rules());
        assert_eq!(t.advance(0.0).shown, "a");
        assert_eq!(t.advance(99.0).shown, "a");
        assert_eq!(t.advance(1.0).shown, "ab");
        let s = t.advance(100.0);
        assert_eq!((s.shown, s.blips, s.done), ("abc", 1, true));
    }

    #[test]
    fn the_same_text_reveals_the_same_at_any_frame_rate() {
        let text = "Hello there, Zellia.";
        let fine = vec![5.0; 600];
        let coarse = vec![250.0; 12];
        let one = [3000.0];
        let a = run(text, &fine);
        assert_eq!(a, run(text, &coarse));
        assert_eq!(a, run(text, &one));
        assert_eq!(a.0, text);
    }

    #[test]
    fn a_slow_frame_reveals_several_letters_and_every_blip() {
        let mut t = Typewriter::new("abcdef", &rules());
        let s = t.advance(350.0);
        assert_eq!(s.shown, "abcd");
        assert_eq!(s.blips, 4);
        assert_eq!(s.blip_letters, vec!['a', 'b', 'c', 'd']);
    }

    #[test]
    fn blips_equal_the_revealed_non_space_letters_and_repeats_still_blip() {
        let (shown, n) = run("a book, ok. 42!", &[10_000.0]);
        assert_eq!(shown, "a book, ok. 42!");
        // a b o o k o k 4 2
        assert_eq!(n, 9);
        let mut t = Typewriter::new("ooo", &rules());
        assert_eq!(t.advance(1000.0).blips, 3);
    }

    #[test]
    fn punctuation_pauses_and_never_blips() {
        let mut t = Typewriter::new("a,b", &rules());
        assert_eq!(t.advance(0.0).shown, "a");
        // the comma shows at 100 ms and holds the next letter back an extra 300 ms
        let s = t.advance(100.0);
        assert_eq!((s.shown, s.blips), ("a,", 0));
        assert_eq!(t.advance(399.0).shown, "a,");
        let s = t.advance(1.0);
        assert_eq!((s.shown, s.blips), ("a,b", 1));
        let mut t = Typewriter::new("x.y", &rules());
        t.advance(100.0);
        assert_eq!(t.advance(599.0).shown, "x.");
        assert_eq!(t.advance(1.0).shown, "x.y");
    }

    #[test]
    fn a_space_adds_no_wait_and_no_blip() {
        let mut t = Typewriter::new("a b", &rules());
        let s = t.advance(100.0);
        assert_eq!((s.shown, s.blips), ("a b", 2));
    }

    #[test]
    fn skip_shows_everything_without_blips() {
        let mut t = Typewriter::new("Hello there", &rules());
        t.advance(150.0);
        assert!(t.skip());
        assert_eq!(t.shown(), "Hello there");
        assert!(t.is_done());
        let s = t.advance(10.0);
        assert_eq!(s.blips, 0);
        assert!(!t.skip());
    }

    #[test]
    fn the_line_holds_after_the_last_letter_then_expires() {
        // "ab cd": 4 letters, 2 words -> hold 1000 + 2 * 100 = 1200 ms from the last letter at 300 ms.
        let mut t = Typewriter::new("ab cd", &rules());
        assert_eq!(t.hold_ms(), 1200.0);
        t.advance(300.0);
        assert!(t.is_done());
        assert!(!t.expired());
        t.advance(1199.0);
        assert!(!t.expired());
        t.advance(1.0);
        assert!(t.expired());
    }

    #[test]
    fn expire_ends_a_line_early() {
        let mut t = Typewriter::new("hi", &rules());
        assert!(t.expire());
        assert!(t.expired());
        assert_eq!(t.shown(), "hi");
        assert!(!t.expire());
    }

    #[test]
    fn multibyte_text_slices_on_character_boundaries() {
        let mut t = Typewriter::new("Incoming call…é", &rules());
        t.advance(10_000.0);
        assert_eq!(t.shown(), "Incoming call…é");
        let mut t = Typewriter::new("é…", &rules());
        t.advance(0.0);
        assert_eq!(t.shown(), "é");
    }

    #[test]
    fn empty_text_is_done_and_holds_its_base_time() {
        let mut t = Typewriter::new("", &rules());
        assert!(t.advance(0.0).done);
        assert_eq!(t.shown(), "");
        assert!(!t.expired());
        t.advance(1000.0);
        assert!(t.expired());
    }

    #[test]
    fn the_embedded_ui_data_loads_with_a_voice_for_zellia() {
        let ui = ui();
        assert_eq!(ui.voice("zellia").name, "Zellia");
        assert_eq!(ui.voice("nobody").id, "zellia");
        assert!(ui.typewriter.chars_per_second > 0.0);
        assert!(ui.typewriter.pauses.iter().any(|p| p.chars.contains('.')));
        assert!(ui.speakers.len() >= 2);
        // every shipped call line fits a speaker with a voice
        let calls = data::CallsData::load();
        let entry = calls.for_level("classic", 1).unwrap();
        assert_eq!(entry.speakers.len(), entry.lines.len());
    }
}
