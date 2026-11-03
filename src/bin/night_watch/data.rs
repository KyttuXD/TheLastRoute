//! Night Watch's data: the headset call scripts (copied from Godot's data/night_watch/calls, schema nw.calls/1) and
//! the call timings from the 2D core/callbox.js. The call files are embedded so the executable needs no data folder.

use serde_json::Value;
use std::collections::HashMap;

/// "Incoming call..." shows this long before the first line (the ring).
pub const CALL_INCOMING_MS: f64 = 2200.0;
/// One more word every this many milliseconds.
pub const CALL_WORD_MS: f64 = 250.0;
/// A line stays up for this plus CALL_LINE_WORD_MS per word.
pub const CALL_LINE_BASE_MS: f64 = 1800.0;
pub const CALL_LINE_WORD_MS: f64 = 380.0;
pub const CALL_INCOMING_TEXT: &str = "Incoming call…";

/// One night's call: the seconds into the night she rings, and the lines she says.
#[derive(Clone, Debug)]
pub struct CallEntry {
    pub at: u32,
    pub lines: Vec<String>,
}

/// Every mode's call table, keyed by mode id then by level number.
#[derive(Debug, Default)]
pub struct CallsData {
    calls: HashMap<String, HashMap<i32, CallEntry>>,
}

/// The call files, embedded at build time (the same set Godot ships in data/night_watch/calls).
const FILES: [&str; 8] = [
    include_str!("calls/bedroom.json"),
    include_str!("calls/classic.json"),
    include_str!("calls/dayshift.json"),
    include_str!("calls/descent.json"),
    include_str!("calls/extra.json"),
    include_str!("calls/lure.json"),
    include_str!("calls/mask.json"),
    include_str!("calls/story.json"),
];

impl CallsData {
    /// Parses the embedded call files. A file that does not parse is skipped, so one bad file cannot stop the game.
    pub fn load() -> CallsData {
        let mut calls = HashMap::new();
        for text in FILES {
            let Ok(file) = serde_json::from_str::<Value>(text) else { continue };
            let (Some(id), Some(levels)) = (file["id"].as_str(), file["levels"].as_array()) else { continue };
            let table = levels.iter().filter_map(level_entry).collect();
            calls.insert(id.to_string(), table);
        }
        CallsData { calls }
    }

    /// The call for `mode_id` on `level`, if the mode has one.
    pub fn for_level(&self, mode_id: &str, level: i32) -> Option<&CallEntry> {
        self.calls.get(mode_id)?.get(&level)
    }
}

/// One `levels` row of a call file: its level number, the ring time and the lines. None for a malformed row.
fn level_entry(row: &Value) -> Option<(i32, CallEntry)> {
    let level = row["level"].as_str()?.parse::<i32>().ok()?;
    let at = u32::try_from(row["at"].as_u64()?).ok()?;
    let lines = row["lines"].as_array()?.iter().filter_map(|l| l.as_str().map(str::to_string)).collect();
    Some((level, CallEntry { at, lines }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_classic_first_night_call_is_loaded() {
        let calls = CallsData::load();
        let entry = calls.for_level("classic", 1).expect("classic has a level 1 call");
        assert_eq!(entry.at, 5);
        assert!(!entry.lines.is_empty());
    }

    #[test]
    fn a_mode_without_a_call_gives_none() {
        assert!(CallsData::load().for_level("no_such_mode", 1).is_none());
    }
}
