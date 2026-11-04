//! What a co-op host tells the guests about its hunters and the world events around them (Godot `_hunters` frames
//! from `main_3d.gd` `_collect_hunter_report`, and `_event` relays such as `hunter_grab`).
//!
//! The host simulates every hunter; a guest displays what it is sent. This module owns:
//! * the tuning, in `data/net/hunter_events.json` (`lr.hunter_events/1`): the frame rate, the frame size where the
//!   reliable large channel is used, which difficulties send frames, and per event kind its difficulties and its gaps.
//!   Unknown keys and unknown event kinds are errors, so a typo never silently disables a rule.
//! * the [`Tracker`], which turns the host's hunter frames into event objects (state changes, grab start and end) and
//!   applies the event rules. The host calls it once per frame and sends what it returns on the EVENTS channel.
//! * the state names the frames carry (`IDLE`, `WANDER`, `INVESTIGATE`, `CHASE`, `SEARCH`, `PEEK`).
//!
//! Event kinds listed in the data but not yet emitted by the port (`hunter_door`, `hunter_locker`, `hunter_charge`,
//! `hunter_ability`, `sabotage`) are parsed and checked here, so their tuning is ready when their source is ported.

use crate::config::Difficulty;
use crate::hunters::brain::State;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const SCHEMA: &str = "lr.hunter_events/1";

/// The event kinds the data file may tune.
pub const EVENT_KINDS: [&str; 7] = [
    "hunter_state",
    "hunter_grab",
    "hunter_door",
    "hunter_locker",
    "hunter_charge",
    "hunter_ability",
    "sabotage",
];

/// The difficulty ids a rule may name (`Difficulty::id`).
const DIFFICULTY_IDS: [&str; 4] = ["relaxed", "standard", "hard", "nightmare"];

/// One event kind's rule: the difficulties it is sent on, and the least time between two of its events.
#[derive(Clone, Debug, PartialEq)]
pub struct EventRule {
    pub difficulties: Vec<String>,
    /// Send only when the value changes (hunter state); otherwise every occurrence is sent.
    pub on_change_only: bool,
    pub min_gap: Duration,
}

/// The parsed `lr.hunter_events/1` file.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub rate_hz: f32,
    /// A frame whose JSON is longer than this travels on the reliable large channel (`_hunters_large`).
    pub large_frame_over_bytes: usize,
    pub frame_difficulties: Vec<String>,
    pub events: HashMap<String, EventRule>,
}

impl Default for Rules {
    /// The rules when the embedded file is missing or broken: frames at 20 Hz on every difficulty, and the state and
    /// grab events on the three ported difficulties.
    fn default() -> Rules {
        let ported: Vec<String> = ["relaxed", "standard", "hard"].iter().map(|s| s.to_string()).collect();
        let mut events = HashMap::new();
        events.insert(
            "hunter_state".to_string(),
            EventRule { difficulties: ported.clone(), on_change_only: true, min_gap: Duration::ZERO },
        );
        events.insert(
            "hunter_grab".to_string(),
            EventRule { difficulties: ported, on_change_only: false, min_gap: Duration::ZERO },
        );
        Rules {
            rate_hz: 20.0,
            large_frame_over_bytes: 1100,
            frame_difficulties: DIFFICULTY_IDS.iter().map(|s| s.to_string()).collect(),
            events,
        }
    }
}

/// Keys of `o` must be among `allowed`; an unknown key is an error that names it.
fn check_keys(o: &Map<String, Value>, what: &str, allowed: &[&str]) -> Result<(), String> {
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(format!("{what}: unknown key {k}")),
        None => Ok(()),
    }
}

fn object<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("{what} is not an object"))
}

/// A number in `[lo, hi]`, when the key is present.
fn number(o: &Map<String, Value>, key: &str, what: &str, lo: f64, hi: f64) -> Result<Option<f64>, String> {
    match o.get(key) {
        None => Ok(None),
        Some(v) => match v.as_f64() {
            Some(f) if f.is_finite() && f >= lo && f <= hi => Ok(Some(f)),
            _ => Err(format!("{what}.{key} must be a number from {lo} to {hi}")),
        },
    }
}

/// The difficulty ids a rule lists; all of them when the key is absent. An unknown id is an error.
fn difficulties(o: &Map<String, Value>, what: &str) -> Result<Vec<String>, String> {
    let Some(list) = o.get("difficulties") else {
        return Ok(DIFFICULTY_IDS.iter().map(|s| s.to_string()).collect());
    };
    let list = list.as_array().ok_or_else(|| format!("{what}.difficulties must be a list"))?;
    let mut out: Vec<String> = Vec::new();
    for v in list {
        let id = v
            .as_str()
            .filter(|id| DIFFICULTY_IDS.contains(id))
            .ok_or_else(|| format!("{what}.difficulties has an unknown id"))?;
        if !out.iter().any(|x| x == id) {
            out.push(id.to_string());
        }
    }
    Ok(out)
}

fn channel_is(o: &Map<String, Value>, what: &str, want: &str) -> Result<(), String> {
    match o.get("channel") {
        None => Ok(()),
        Some(v) if v.as_str() == Some(want) => Ok(()),
        Some(_) => Err(format!("{what}.channel must be {want}")),
    }
}

/// Parses `lr.hunter_events/1`. Every error names the key at fault.
pub fn parse(text: &str) -> Result<Rules, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let root = object(&root, "hunter_events")?;
    check_keys(root, "hunter_events", &["schema", "frame", "events"])?;
    if root.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("hunter_events.schema must be {SCHEMA}"));
    }
    let frame = object(root.get("frame").ok_or("hunter_events.frame is missing")?, "frame")?;
    check_keys(frame, "frame", &["channel", "rate_hz", "large_frame_over_bytes", "difficulties"])?;
    channel_is(frame, "frame", "HUNTERS")?;
    let rate_hz = number(frame, "rate_hz", "frame", 0.5, 60.0)?.unwrap_or(20.0) as f32;
    let large = number(frame, "large_frame_over_bytes", "frame", 64.0, 65_000.0)?.unwrap_or(1100.0) as usize;
    let frame_difficulties = difficulties(frame, "frame")?;

    let mut events = HashMap::new();
    let table = object(root.get("events").ok_or("hunter_events.events is missing")?, "events")?;
    for (kind, value) in table {
        if !EVENT_KINDS.contains(&kind.as_str()) {
            return Err(format!("events: unknown event kind {kind}"));
        }
        let what = format!("events.{kind}");
        let o = object(value, &what)?;
        check_keys(o, &what, &["channel", "difficulties", "on_change_only", "min_gap_s", "cooldown_s", "max_per_s"])?;
        channel_is(o, &what, "EVENTS")?;
        let on_change_only = match o.get("on_change_only") {
            None => false,
            Some(v) => v.as_bool().ok_or_else(|| format!("{what}.on_change_only must be true or false"))?,
        };
        // the least gap is the largest of the three gap keys
        let mut gap = 0.0f64;
        if let Some(s) = number(o, "min_gap_s", &what, 0.0, 60.0)? {
            gap = gap.max(s);
        }
        if let Some(s) = number(o, "cooldown_s", &what, 0.0, 60.0)? {
            gap = gap.max(s);
        }
        if let Some(per_s) = number(o, "max_per_s", &what, 0.01, 60.0)? {
            gap = gap.max(1.0 / per_s);
        }
        events.insert(
            kind.clone(),
            EventRule {
                difficulties: difficulties(o, &what)?,
                on_change_only,
                min_gap: Duration::from_secs_f64(gap),
            },
        );
    }
    Ok(Rules { rate_hz, large_frame_over_bytes: large, frame_difficulties, events })
}

/// The embedded rules (data/net/hunter_events.json), read once. The defaults when the file is missing or broken.
pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        crate::content::embedded::data_under("net/hunter_events.json")
            .next()
            .and_then(|(_, text)| parse(text).ok())
            .unwrap_or_default()
    })
}

/// The name a frame carries for a hunter state (the Godot `state` numbers have no meaning on the wire).
pub fn state_name(state: State) -> &'static str {
    match state {
        State::Idle => "IDLE",
        State::Wander => "WANDER",
        State::Investigate => "INVESTIGATE",
        State::Chase => "CHASE",
        State::Search => "SEARCH",
        State::Peek => "PEEK",
    }
}

/// The state a frame names, or None for anything unknown.
pub fn state_from_name(name: &str) -> Option<State> {
    Some(match name {
        "IDLE" => State::Idle,
        "WANDER" => State::Wander,
        "INVESTIGATE" => State::Investigate,
        "CHASE" => State::Chase,
        "SEARCH" => State::Search,
        "PEEK" => State::Peek,
        _ => return None,
    })
}

/// True when the host sends hunter frames on this difficulty.
pub fn frames_on(rules: &Rules, diff: Difficulty) -> bool {
    rules.frame_difficulties.iter().any(|d| d == diff.id())
}

/// True when the event kind is sent on this difficulty. A kind with no rule is not sent.
pub fn event_on(rules: &Rules, kind: &str, diff: Difficulty) -> bool {
    rules.events.get(kind).is_some_and(|r| r.difficulties.iter().any(|d| d == diff.id()))
}

/// The host's event state between frames: the last state of each hunter, the hunter holding the runner, the last
/// time each kind was sent, and when the next frame is due.
#[derive(Debug, Default)]
pub struct Tracker {
    states: Vec<Option<String>>,
    grab: Option<usize>,
    last_sent: HashMap<&'static str, Instant>,
    next_frame: Option<Instant>,
}

impl Tracker {
    /// The event objects this frame produces, from the host's frame. A hunter state event is sent when the state
    /// changed since the last call (the first call only records). A grab is sent when a hunter takes the runner
    /// (`phase: start`) and when it lets go (`phase: end`). Each kind obeys its rule's difficulties and gap.
    pub fn events(
        &mut self,
        rules: &Rules,
        diff: Difficulty,
        frame: &[Value],
        grab: Option<usize>,
        now: Instant,
    ) -> Vec<Map<String, Value>> {
        let mut out = Vec::new();
        let hunter_id = |index: usize| {
            frame
                .iter()
                .find(|e| e["index"].as_u64() == Some(index as u64))
                .and_then(|e| e["id"].as_str())
                .unwrap_or_default()
                .to_string()
        };
        for entry in frame {
            let Some(index) = entry["index"].as_u64().map(|i| i as usize) else { continue };
            let Some(state) = entry["state"].as_str() else { continue };
            if self.states.len() <= index {
                self.states.resize(index + 1, None);
            }
            let previous = self.states[index].replace(state.to_string());
            if previous.as_deref().is_some_and(|p| p != state) && self.allow(rules, diff, "hunter_state", now) {
                out.push(event(
                    "hunter_state",
                    [("index", json!(index)), ("hunter_id", json!(hunter_id(index))), ("state", json!(state))],
                ));
            }
        }
        if grab != self.grab {
            if let Some(i) = grab {
                if self.allow(rules, diff, "hunter_grab", now) {
                    out.push(event(
                        "hunter_grab",
                        [("phase", json!("start")), ("index", json!(i)), ("hunter_id", json!(hunter_id(i)))],
                    ));
                }
            } else if let Some(i) = self.grab {
                if self.allow(rules, diff, "hunter_grab", now) {
                    out.push(event(
                        "hunter_grab",
                        [("phase", json!("end")), ("index", json!(i)), ("hunter_id", json!(hunter_id(i)))],
                    ));
                }
            }
            self.grab = grab;
        }
        out
    }

    /// True when a frame is due now at the rule's rate. Records the next due time when it is.
    pub fn frame_due(&mut self, rules: &Rules, now: Instant) -> bool {
        match self.next_frame {
            Some(next) if now < next => false,
            _ => {
                self.next_frame = Some(now + Duration::from_secs_f64(1.0 / rules.rate_hz as f64));
                true
            }
        }
    }

    /// Checks that the kind is on for this difficulty and its gap has passed, and records the send when it is.
    fn allow(&mut self, rules: &Rules, diff: Difficulty, kind: &'static str, now: Instant) -> bool {
        let Some(rule) = rules.events.get(kind) else { return false };
        if !rule.difficulties.iter().any(|d| d == diff.id()) {
            return false;
        }
        if let Some(last) = self.last_sent.get(kind) {
            if now.duration_since(*last) < rule.min_gap {
                return false;
            }
        }
        self.last_sent.insert(kind, now);
        true
    }
}

fn event<const N: usize>(what: &str, fields: [(&str, Value); N]) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("what".into(), json!(what));
    for (k, v) in fields {
        o.insert(k.into(), v);
    }
    o
}

/// The notice a guest shows for a world event, or None when the event needs no notice (its state arrives in the frames).
pub fn notice(event: &Map<String, Value>) -> Option<String> {
    let id = event.get("hunter_id").and_then(Value::as_str).unwrap_or("HUNTER").to_uppercase();
    match (event.get("what")?.as_str()?, event.get("phase").and_then(Value::as_str)) {
        ("hunter_grab", Some("start")) => Some(format!("{id} HAS THE HOST")),
        ("hunter_grab", Some("end")) => Some(format!("{id} LET THE HOST GO")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(states: &[(&str, &str)]) -> Vec<Value> {
        states
            .iter()
            .enumerate()
            .map(|(i, (id, state))| json!({"index": i, "id": id, "state": state}))
            .collect()
    }

    #[test]
    fn the_embedded_file_parses_with_the_planned_rules() {
        let text = include_str!("../data/net/hunter_events.json");
        let r = parse(text).expect("data/net/hunter_events.json parses");
        assert_eq!(r.rate_hz, 20.0);
        assert_eq!(r.large_frame_over_bytes, 1100);
        assert!(frames_on(&r, Difficulty::Standard) && frames_on(&r, Difficulty::Nightmare));
        assert_eq!(r.events.len(), EVENT_KINDS.len());
        // Nightmare has no port yet: its ability rule is the only one that names it
        assert!(event_on(&r, "hunter_ability", Difficulty::Nightmare));
        assert!(!event_on(&r, "hunter_state", Difficulty::Nightmare));
        assert!(event_on(&r, "hunter_grab", Difficulty::Hard));
        assert_eq!(rules(), &parse(text).unwrap());
    }

    #[test]
    fn unknown_keys_and_kinds_are_errors_that_name_them() {
        let bad_top = r#"{"schema":"lr.hunter_events/1","frame":{},"events":{},"extra":1}"#;
        assert!(parse(bad_top).unwrap_err().contains("extra"));
        let bad_rule = r#"{"schema":"lr.hunter_events/1","frame":{},"events":{"hunter_grab":{"cooldwn_s":1}}}"#;
        assert!(parse(bad_rule).unwrap_err().contains("cooldwn_s"));
        let bad_kind = r#"{"schema":"lr.hunter_events/1","frame":{},"events":{"hunter_yell":{}}}"#;
        assert!(parse(bad_kind).unwrap_err().contains("hunter_yell"));
        let bad_diff = r#"{"schema":"lr.hunter_events/1","frame":{"difficulties":["easy"]},"events":{}}"#;
        assert!(parse(bad_diff).unwrap_err().contains("unknown id"));
        let no_schema = r#"{"frame":{},"events":{}}"#;
        assert!(parse(no_schema).is_err());
    }

    #[test]
    fn a_state_change_is_sent_once_and_the_first_frame_only_records() {
        let r = Rules::default();
        let now = Instant::now();
        let mut t = Tracker::default();
        assert!(t.events(&r, Difficulty::Standard, &frame(&[("aki", "IDLE")]), None, now).is_empty());
        assert!(t.events(&r, Difficulty::Standard, &frame(&[("aki", "IDLE")]), None, now).is_empty());
        let out = t.events(&r, Difficulty::Standard, &frame(&[("aki", "CHASE")]), None, now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["what"], json!("hunter_state"));
        assert_eq!(out[0]["hunter_id"], json!("aki"));
        assert_eq!(out[0]["state"], json!("CHASE"));
    }

    #[test]
    fn a_grab_is_sent_at_its_start_and_its_end() {
        let r = Rules::default();
        let now = Instant::now();
        let mut t = Tracker::default();
        let f = frame(&[("aki", "CHASE")]);
        t.events(&r, Difficulty::Hard, &f, None, now);
        let start = t.events(&r, Difficulty::Hard, &f, Some(0), now);
        assert_eq!(start.len(), 1);
        assert_eq!(start[0]["phase"], json!("start"));
        let end = t.events(&r, Difficulty::Hard, &f, None, now);
        assert_eq!(end[0]["phase"], json!("end"));
        assert_eq!(notice(&start[0]).as_deref(), Some("AKI HAS THE HOST"));
    }

    #[test]
    fn the_data_decides_which_difficulties_and_gaps_send() {
        let now = Instant::now();
        let f = frame(&[("aki", "IDLE")]);
        let moved = frame(&[("aki", "CHASE")]);
        // a rule that names only hard sends nothing on standard
        let hard_only = parse(
            r#"{"schema":"lr.hunter_events/1","frame":{},"events":{"hunter_state":{"difficulties":["hard"],"on_change_only":true}}}"#,
        )
        .unwrap();
        let mut t = Tracker::default();
        t.events(&hard_only, Difficulty::Standard, &f, None, now);
        assert!(t.events(&hard_only, Difficulty::Standard, &moved, None, now).is_empty());
        let mut t = Tracker::default();
        t.events(&hard_only, Difficulty::Hard, &f, None, now);
        assert_eq!(t.events(&hard_only, Difficulty::Hard, &moved, None, now).len(), 1);
        // a cooldown of one second: a second change inside it is held back
        let gapped =
            parse(r#"{"schema":"lr.hunter_events/1","frame":{},"events":{"hunter_state":{"cooldown_s":1.0}}}"#).unwrap();
        let mut t = Tracker::default();
        t.events(&gapped, Difficulty::Standard, &f, None, now);
        assert_eq!(t.events(&gapped, Difficulty::Standard, &moved, None, now).len(), 1);
        let back = frame(&[("aki", "IDLE")]);
        assert!(t.events(&gapped, Difficulty::Standard, &back, None, now).is_empty());
        // the held-back change was recorded, so the next change after the gap is the one sent
        assert_eq!(t.events(&gapped, Difficulty::Standard, &moved, None, now + Duration::from_secs(2)).len(), 1);
    }

    #[test]
    fn frames_follow_the_rate_and_the_difficulty_list() {
        let r = parse(r#"{"schema":"lr.hunter_events/1","frame":{"rate_hz":10,"difficulties":["hard"]},"events":{}}"#)
            .unwrap();
        assert!(!frames_on(&r, Difficulty::Standard));
        assert!(frames_on(&r, Difficulty::Hard));
        let mut t = Tracker::default();
        let now = Instant::now();
        assert!(t.frame_due(&r, now));
        assert!(!t.frame_due(&r, now + Duration::from_millis(50)));
        assert!(t.frame_due(&r, now + Duration::from_millis(100)));
    }

    #[test]
    fn state_names_round_trip() {
        for s in [State::Idle, State::Wander, State::Investigate, State::Chase, State::Search, State::Peek] {
            assert_eq!(state_from_name(state_name(s)), Some(s));
        }
        assert_eq!(state_from_name("NOPE"), None);
    }
}
