//! Zellia's headset calls: the per-mode call tables (compiled-in JSON) and the call as a state machine, ported line
//! for line from the 2D game's createCall. Pure logic: the HUD feeds it the shift clock and draws `text` while `open`.
#![allow(dead_code)]

use serde_json::Value;

/// "Incoming call..." shows this long before the first line (the ring).
pub const INCOMING_MS: f64 = 2200.0;
/// One more word every 250 ms.
pub const WORD_MS: f64 = 250.0;
/// A line stays up for this plus LINE_WORD_MS per word.
pub const LINE_BASE_MS: f64 = 1800.0;
pub const LINE_WORD_MS: f64 = 380.0;
pub const INCOMING_TEXT: &str = "Incoming call\u{2026}";

#[derive(Clone, Debug)]
pub struct CallLevel {
    /// Extra: the level (night, week, watch) the call belongs to.
    pub level: i32,
    pub at_s: f64,
    pub lines: Vec<String>,
}

/// Parse one calls file (nw.calls/1).
pub fn parse_calls(text: &str) -> Result<Vec<CallLevel>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("calls json: {e}"))?;
    let levels = root.get("levels").and_then(|l| l.as_array()).ok_or_else(|| "calls json: no levels".to_string())?;
    let mut out = Vec::new();
    for l in levels {
        let level = match l.get("level") {
            Some(Value::String(s)) => s.trim().parse::<i32>().map_err(|_| format!("calls json: bad level {s}"))?,
            Some(v) => v.as_f64().map(|f| f as i32).ok_or_else(|| "calls json: bad level".to_string())?,
            None => return Err("calls json: a level has no number".to_string()),
        };
        let lines: Vec<String> = l
            .get("lines")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        out.push(CallLevel { level, at_s: l.get("at").and_then(|a| a.as_f64()).unwrap_or(6.0), lines });
    }
    Ok(out)
}

/// The mode's call table in file order (empty for modes without calls: mixed). A broken file gives an empty table.
pub fn call_levels(mode_id: &str) -> Vec<CallLevel> {
    if !matches!(mode_id, "classic" | "mask" | "lure" | "bedroom" | "story" | "descent" | "dayshift" | "extra") {
        return Vec::new();
    }
    parse_calls(crate::assets::calls_json(mode_id)).unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallEvent {
    Open,
    Ring,
    Talk,
    Close,
}

#[derive(Clone, Debug)]
pub struct CallScript {
    pub open: bool,
    pub done: bool,
    pub text: String,
    pub muted: bool,
    line: i32,
    lines: Vec<String>,
    at_ms: f64,
    active: bool,
    t: f64,
    dur: f64,
    words: Vec<String>,
    shown: usize,
}

impl Default for CallScript {
    fn default() -> Self {
        CallScript::empty()
    }
}

impl CallScript {
    /// No call (done from the start): custom nights and mixed.
    pub fn empty() -> CallScript {
        CallScript::from_lines(Vec::new(), 6.0)
    }

    pub fn from_lines(lines: Vec<String>, at_s: f64) -> CallScript {
        CallScript {
            open: false,
            done: lines.is_empty(),
            text: String::new(),
            muted: false,
            line: -1,
            lines,
            at_ms: at_s * 1000.0,
            active: false,
            t: 0.0,
            dur: 0.0,
            words: Vec::new(),
            shown: 0,
        }
    }

    /// The call for a level of a mode; empty when it is a quiet night.
    pub fn for_level(mode_id: &str, level: i32) -> CallScript {
        match call_levels(mode_id).into_iter().find(|c| c.level == level) {
            Some(c) if !c.lines.is_empty() => CallScript::from_lines(c.lines, c.at_s),
            _ => CallScript::empty(),
        }
    }

    /// Index of the line being typed (-1 before the first).
    pub fn line_index(&self) -> i32 {
        self.line
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
            self.t = -INCOMING_MS;
            self.text = INCOMING_TEXT.to_string();
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
            self.words = self.lines[self.line as usize].split(' ').map(|s| s.to_string()).collect();
            self.dur = LINE_BASE_MS + self.words.len() as f64 * LINE_WORD_MS;
        }
        let want = self.words.len().min((self.t / WORD_MS).floor() as usize);
        if want != self.shown {
            self.shown = want;
            self.text = self.words[..want].join(" ");
            if want % 2 == 1 {
                events.push(CallEvent::Talk);
            }
        }
        events
    }

    /// Mute ends the call; true if there was a call to end.
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
mod tests {
    use super::*;

    fn script() -> CallScript {
        CallScript::from_lines(vec!["a b c".to_string(), "d e".to_string()], 6.0)
    }

    #[test]
    fn nothing_happens_before_the_ring_time() {
        let mut c = script();
        assert!(c.update(5999.0, 100.0).is_empty());
        assert!(!c.open);
    }

    #[test]
    fn the_ring_opens_at_the_call_time_and_text_starts_after_2200() {
        let mut c = script();
        let ev = c.update(6000.0, 0.0);
        assert_eq!(ev, vec![CallEvent::Open, CallEvent::Ring]);
        assert!(c.open);
        assert_eq!(c.text, INCOMING_TEXT);
        assert!(c.update(6100.0, 2199.0).is_empty());
        assert_eq!(c.text, INCOMING_TEXT);
        assert!(c.update(8300.0, 1.0).is_empty()); // line 0 begins, no word yet
    }

    #[test]
    fn words_arrive_every_250_ms_and_talk_on_odd_counts() {
        let mut c = script();
        c.update(6000.0, 2200.0);
        let ev = c.update(8250.0, 250.0);
        assert_eq!((c.text.as_str(), ev), ("a", vec![CallEvent::Talk]));
        let ev = c.update(8500.0, 250.0);
        assert_eq!((c.text.as_str(), ev), ("a b", vec![]));
        let ev = c.update(8750.0, 250.0);
        assert_eq!((c.text.as_str(), ev), ("a b c", vec![CallEvent::Talk]));
    }

    #[test]
    fn a_line_stays_up_1800_plus_380_per_word_then_the_next_starts() {
        let mut c = script();
        c.update(6000.0, 2200.0);
        c.update(0.0, 0.0); // line 0 starts
        c.update(0.0, 2939.0);
        assert_eq!(c.line_index(), 0);
        c.update(0.0, 1.0); // 2940 ms: next line
        assert_eq!(c.line_index(), 1);
        assert!(!c.done);
    }

    #[test]
    fn the_box_closes_after_the_last_line() {
        let mut c = script();
        c.update(6000.0, 2200.0);
        c.update(0.0, 0.0);
        c.update(0.0, 2940.0);
        // line 1 has 2 words: 1800 + 760 = 2560
        let ev = c.update(0.0, 2560.0);
        assert_eq!(ev, vec![CallEvent::Close]);
        assert!(c.done && !c.open);
        assert!(c.update(99999.0, 1000.0).is_empty());
    }

    #[test]
    fn mute_ends_a_running_call_only() {
        let mut c = script();
        assert!(!c.mute());
        c.update(6000.0, 10.0);
        assert!(c.mute());
        assert!(c.muted && c.done && !c.open);
        assert!(!c.mute());
        assert!(c.update(7000.0, 1000.0).is_empty());
    }

    #[test]
    fn an_empty_script_is_done() {
        let mut c = CallScript::empty();
        assert!(c.done);
        assert!(c.update(100000.0, 100.0).is_empty());
        assert!(CallScript::from_lines(Vec::new(), 5.0).done);
    }

    #[test]
    fn the_shipped_tables_load() {
        let classic = call_levels("classic");
        assert_eq!(classic.len(), 7);
        assert_eq!(classic[0].level, 1);
        assert_eq!(classic[0].at_s, 5.0);
        assert!(classic[0].lines[0].starts_with("Zellia"));
        for id in ["mask", "lure", "bedroom", "story", "descent", "dayshift", "extra"] {
            assert!(!call_levels(id).is_empty(), "{id}");
        }
        assert!(call_levels("mixed").is_empty());
        assert!(!CallScript::for_level("classic", 1).done);
        assert!(CallScript::for_level("classic", 99).done);
        assert!(CallScript::for_level("mixed", 1).done);
    }
}
