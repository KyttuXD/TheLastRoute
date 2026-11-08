//! Death lines and the death moment. Ports `game/config/death_lines.gd` (the pick) and `game/hud/death_moment.gd`
//! (typing, guard, auto dismiss). `pick_line` follows `src/bin/last_route/ui/report.rs` (`pick_line_with`) and adds `{name}`.
//! The drawing of the moment lives in `cards.rs`; this file is state only.
#![allow(dead_code)]

use serde_json::Value;
use std::sync::OnceLock;

pub const WORD_SECONDS: f32 = 0.25;
pub const INPUT_GUARD_SECONDS: f32 = 0.4;
pub const AUTO_DISMISS_SECONDS: f32 = 7.0;
pub const SHORT_DISMISS_SECONDS: f32 = 2.5;
pub const FADE_SECONDS: f32 = 0.25;
const DEFAULT_COPY: &str = "A hunter intercepted your signal on depth {depth}.";

/// FNV-1a, 32 bit, over UTF-16 code units like the JS original (the lines are ASCII).
pub fn fnv(text: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for c in text.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn data() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(crate::assets::DEATH_LINES_JSON).unwrap_or(Value::Null))
}

fn line_text(v: &Value) -> Option<(String, bool)> {
    match v {
        Value::String(s) => Some((s.clone(), false)),
        Value::Object(o) => o.get("text").and_then(|t| t.as_str()).map(|t| {
            (t.to_string(), o.get("style").and_then(|s| s.as_str()) == Some("caption"))
        }),
        _ => None,
    }
}

/// DeathLines.pick. Returns (text, caption, plays). `plays` is true only for a hunter or cause line (DeathMoment.plays);
/// the end-card fallback copy and void hunters (voiceless) give plays = false.
/// `cause` is "", "deal", "cold", "darkness" (or "lamp") or any other word (ignored).
pub fn pick_line(hunter_id: &str, cause: &str, seed: &str, depth: i32, name: &str) -> (String, bool, bool) {
    pick_line_with(hunter_id, cause, seed, depth, name, cfg!(debug_assertions))
}

pub fn pick_line_with(
    hunter_id: &str,
    cause: &str,
    seed: &str,
    depth: i32,
    name: &str,
    allow_placeholders: bool,
) -> (String, bool, bool) {
    let d = data();
    let id = hunter_id.to_lowercase().replace('_', "");
    let mute = id.starts_with("void");
    let key = if mute {
        ""
    } else {
        match cause {
            "deal" => "deal",
            "cold" => "cold",
            _ if id == "nyxie" => {
                if cause == "darkness" || cause == "lamp" {
                    "darkness"
                } else {
                    "nyxie"
                }
            }
            _ => "",
        }
    };
    let hunter = if mute { &Value::Null } else { &d["hunters"][id.as_str()] };
    let hunter_caption = hunter["style"].as_str() == Some("caption");
    let mut pool: Vec<(String, bool)> = Vec::new();
    if !key.is_empty() {
        if let Some(list) = d["causes"][key].as_array() {
            pool.extend(list.iter().filter_map(line_text).map(|(t, c)| (t, c || hunter_caption)));
        }
    }
    if let Some(list) = hunter["lines"].as_array() {
        pool.extend(list.iter().filter_map(line_text).map(|(t, c)| (t, c || hunter_caption)));
    }
    if let Some((t, c)) = line_text(&hunter["loreLine"]) {
        pool.push((t, c || hunter_caption));
    }
    pool.retain(|(t, _)| {
        !t.trim().is_empty() && (allow_placeholders || !t.trim().to_uppercase().starts_with("[PLACEHOLDER]"))
    });
    let fill = |t: &str| t.replace("{depth}", &depth.to_string()).replace("{name}", name);
    if !pool.is_empty() {
        let i = fnv(&format!("{seed}|{depth}|{id}|{key}")) as usize % pool.len();
        let (t, c) = &pool[i];
        return (fill(t), *c, true);
    }
    let fb = d["fallback"].as_str().unwrap_or("");
    if fb.trim().is_empty() {
        (fill(DEFAULT_COPY), false, false)
    } else {
        (fill(fb), false, false)
    }
}

/// "ZELLIA" -> "Z E L L I A" (spaced capitals).
pub fn track(s: &str) -> String {
    s.to_uppercase().chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
}

/// The death moment's state while it is on screen.
#[derive(Clone, Debug)]
pub struct DeathMoment {
    pub name: String,
    pub words: Vec<String>,
    pub caption: bool,
    pub shown: usize,
    pub timer: f32,
    pub age: f32,
    pub typing: bool,
    pub full: bool,
    pub auto_after: f32,
    pub calm: bool,
    pub done: bool,
    blips: u32,
}

impl DeathMoment {
    /// `short` = Save::short_death_scenes, `calm` = reduced motion. Caption, calm and short show the whole line at once.
    pub fn new(name: &str, line: &str, caption: bool, short: bool, calm: bool) -> DeathMoment {
        let mut m = DeathMoment {
            name: track(name),
            words: line.split_whitespace().map(str::to_string).collect(),
            caption,
            shown: 0,
            timer: 0.0,
            age: 0.0,
            typing: false,
            full: false,
            auto_after: if short { SHORT_DISMISS_SECONDS } else { AUTO_DISMISS_SECONDS },
            calm,
            done: false,
            blips: 0,
        };
        if caption || calm || short {
            m.show_all();
        } else {
            m.typing = true;
        }
        m
    }

    fn show_all(&mut self) {
        self.typing = false;
        self.full = true;
        self.timer = 0.0;
        self.shown = self.words.len();
    }

    pub fn shown_line(&self) -> String {
        self.words[..self.shown.min(self.words.len())].join(" ")
    }

    /// Fade-in alpha (1.0 at once with reduced motion).
    pub fn fade(&self) -> f32 {
        if self.calm {
            1.0
        } else {
            (self.age / FADE_SECONDS).clamp(0.0, 1.0)
        }
    }

    /// Menu blips owed for newly typed odd words (the caller plays them); resets the count.
    pub fn take_blips(&mut self) -> u32 {
        std::mem::take(&mut self.blips)
    }

    /// One frame. `pressed` = any key, click or tap this frame; `escape` = Escape was pressed.
    pub fn update(&mut self, dt: f32, pressed: bool, escape: bool) {
        if self.done {
            return;
        }
        self.age += dt;
        self.timer += dt;
        if self.typing {
            let due = self.words.len().min((self.timer / WORD_SECONDS) as usize + 1);
            if due != self.shown {
                self.shown = due;
                if !self.caption && self.shown % 2 == 1 {
                    self.blips += 1;
                }
            }
            if self.shown >= self.words.len() {
                self.typing = false;
                self.full = true;
                self.timer = 0.0;
            }
        } else if self.full && self.timer >= self.auto_after {
            self.done = true;
            return;
        }
        if pressed && self.age >= INPUT_GUARD_SECONDS {
            if escape {
                self.done = true;
            } else if self.typing {
                self.show_all();
            } else {
                self.done = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pick_is_stable_and_fills_the_name() {
        let a = pick_line("dovelle", "", "nightwatch:1", 1, "Dovelle");
        let b = pick_line("dovelle", "", "nightwatch:1", 1, "Dovelle");
        assert_eq!(a, b);
        assert!(a.2 && !a.0.is_empty());
        assert!(!a.0.contains("{name}") && !a.0.contains("{depth}"));
    }

    #[test]
    fn kathy_and_void_hunters_never_play_a_line() {
        for seed in 0..20 {
            let s = format!("nightwatch:{seed}");
            assert!(!pick_line("kathy", "", &s, 7, "Kathy").2);
            assert!(!pick_line("voidred", "", &s, 1, "Red").2);
            assert!(!pick_line("void_red", "", &s, 1, "Red").2);
        }
        let fb = pick_line("kathy", "", "x", 3, "Kathy");
        assert!(!fb.0.is_empty() && !fb.1);
    }

    #[test]
    fn nyxie_in_the_dark_can_say_a_caption_line() {
        let mut caption = false;
        for seed in 0..80 {
            let (t, c, plays) = pick_line("nyxie", "darkness", &format!("nightwatch:{seed}"), 1, "Nyxie");
            assert!(plays && !t.is_empty());
            caption |= c;
        }
        assert!(caption);
    }

    #[test]
    fn placeholders_follow_the_flag() {
        let (t, _, _) = pick_line_with("aki", "", "ROOM-X", 3, "Aki", false);
        assert!(!t.to_uppercase().starts_with("[PLACEHOLDER]"));
    }

    #[test]
    fn typing_shows_one_word_per_quarter_second() {
        let mut m = DeathMoment::new("Dovelle", "one two three four", false, false, false);
        m.update(0.0, false, false);
        assert_eq!(m.shown, 1);
        m.update(0.25, false, false);
        assert_eq!(m.shown, 2);
        m.update(0.5, false, false);
        assert_eq!(m.shown, 4);
        assert!(m.full && !m.typing);
        assert_eq!(m.shown_line(), "one two three four");
    }

    #[test]
    fn input_is_ignored_for_the_first_400_ms() {
        let mut m = DeathMoment::new("Aki", "a b c d e f g", false, false, false);
        m.update(0.1, true, true);
        assert!(!m.done);
        m.update(0.31, true, false);
        assert_eq!(m.shown, 7, "a press finishes the typing");
        assert!(!m.done);
        m.update(0.01, true, false);
        assert!(m.done, "a second press dismisses");
    }

    #[test]
    fn escape_dismisses_at_once_after_the_guard() {
        let mut m = DeathMoment::new("Aki", "a b c d e f g", false, false, false);
        m.update(0.5, true, true);
        assert!(m.done);
    }

    #[test]
    fn auto_dismiss_is_seven_seconds_or_two_and_a_half_when_short() {
        let mut m = DeathMoment::new("Aki", "hello there", false, false, false);
        m.update(1.0, false, false);
        assert!(m.full && !m.done);
        m.update(6.9, false, false);
        assert!(!m.done);
        m.update(0.2, false, false);
        assert!(m.done);
        let mut s = DeathMoment::new("Aki", "hello there", false, true, false);
        assert!(s.full);
        s.update(2.6, false, false);
        assert!(s.done);
    }

    #[test]
    fn caption_lines_show_whole_and_do_not_blip() {
        let mut m = DeathMoment::new("Nyxie", "dark halls keep things", true, false, false);
        assert_eq!(m.shown, 4);
        m.update(0.1, false, false);
        assert_eq!(m.take_blips(), 0);
    }

    #[test]
    fn track_spaces_the_capitals() {
        assert_eq!(track("Zellia"), "Z E L L I A");
    }
}
