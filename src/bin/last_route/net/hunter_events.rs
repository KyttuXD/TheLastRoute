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
//! The host's other world events (`hunter_door`, `hunter_locker`, `hunter_charge`, `hunter_ability`) are raised by the
//! game (game/hunting.rs) into an [`Outbox`], which applies the same rules and hands the events to the net session.
//! `sabotage` is parsed and checked here, so its tuning is ready when its source is ported.

use crate::config::Difficulty;
use crate::hunters::brain::State;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use crate::shared::clock::Instant;
use std::time::Duration;

pub const SCHEMA: &str = "lr.hunter_events/1";

/// The event kinds the data file may tune.
pub const EVENT_KINDS: [&str; 10] = [
    "hunter_state",
    "hunter_grab",
    "hunter_door",
    "hunter_locker",
    "hunter_charge",
    "hunter_ability",
    "sabotage",
    "stream_valve",
    "stream_pipe_burst",
    "stream_safe_room",
];

/// The capture timeout when the data file has no `capture` block.
const DEFAULT_CAPTURE_TIMEOUT_S: u64 = 15;

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
    /// How long the host waits for a captured guest's escaped, caught or capture_blocked report before it stops waiting
    /// (`capture.timeout_s`).
    pub capture_timeout: Duration,
    /// The least time between two taser intents from one guest (`guest.taser_min_gap_s`).
    pub guest_taser_gap: Duration,
    /// The most valve turn requests the host keeps waiting for its game (`guest.max_valve_requests`).
    pub max_valve_requests: usize,
    /// How often the app asks whether the chosen skin can be (re)published to the room (`guest.skin_recheck_s`).
    pub skin_recheck: Duration,
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
            capture_timeout: Duration::from_secs(DEFAULT_CAPTURE_TIMEOUT_S),
            guest_taser_gap: Duration::from_millis(400),
            max_valve_requests: 16,
            skin_recheck: Duration::from_secs(2),
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
    check_keys(root, "hunter_events", &["schema", "frame", "events", "capture", "guest"])?;
    if root.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err(format!("hunter_events.schema must be {SCHEMA}"));
    }
    let frame = object(root.get("frame").ok_or("hunter_events.frame is missing")?, "frame")?;
    check_keys(frame, "frame", &["channel", "rate_hz", "large_frame_over_bytes", "difficulties"])?;
    channel_is(frame, "frame", "HUNTERS")?;
    let rate_hz = number(frame, "rate_hz", "frame", 0.5, 60.0)?.unwrap_or(20.0) as f32;
    let large = number(frame, "large_frame_over_bytes", "frame", 64.0, 65_000.0)?.unwrap_or(1100.0) as usize;
    let frame_difficulties = difficulties(frame, "frame")?;

    let capture_timeout = match root.get("capture") {
        None => Duration::from_secs(DEFAULT_CAPTURE_TIMEOUT_S),
        Some(c) => {
            let c = object(c, "capture")?;
            check_keys(c, "capture", &["timeout_s"])?;
            let s = number(c, "timeout_s", "capture", 1.0, 300.0)?.unwrap_or(DEFAULT_CAPTURE_TIMEOUT_S as f64);
            Duration::from_secs_f64(s)
        }
    };

    let (mut guest_taser_gap, mut max_valve_requests) = (Duration::from_millis(400), 16usize);
    let mut skin_recheck = Duration::from_secs(2);
    if let Some(g) = root.get("guest") {
        let g = object(g, "guest")?;
        check_keys(g, "guest", &["taser_min_gap_s", "max_valve_requests", "skin_recheck_s"])?;
        if let Some(s) = number(g, "skin_recheck_s", "guest", 0.1, 60.0)? {
            skin_recheck = Duration::from_secs_f64(s);
        }
        if let Some(s) = number(g, "taser_min_gap_s", "guest", 0.0, 60.0)? {
            guest_taser_gap = Duration::from_secs_f64(s);
        }
        if let Some(n) = number(g, "max_valve_requests", "guest", 1.0, 256.0)? {
            max_valve_requests = n as usize;
        }
    }

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
    Ok(Rules { rate_hz, large_frame_over_bytes: large, frame_difficulties, events, capture_timeout, guest_taser_gap, max_valve_requests, skin_recheck })
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

/// Frame keys added by continuous levels (optional; an old build reads only `position`).
pub const KEY_LEVEL: &str = "level";
pub const KEY_PRESENTATION_Y: &str = "presentation_y";

/// The level a frame entry says its hunter is on, or `default_level` when the key is missing or not an integer.
pub fn frame_level(entry: &Value, default_level: i32) -> i32 {
    entry
        .get(KEY_LEVEL)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(default_level)
}

/// The absolute y a frame entry presents its hunter at; a missing, non-numeric or non-finite value falls back to the
/// floor of `level`.
pub fn frame_y(entry: &Value, level: i32) -> f32 {
    entry
        .get(KEY_PRESENTATION_Y)
        .and_then(Value::as_f64)
        .map(|f| f as f32)
        .filter(|f| f.is_finite())
        .unwrap_or_else(|| crate::game::level_stack::floor_y_of(level))
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

/// The host's world events raised by the game this frame, waiting for the net session. The game queues an event with
/// [`Outbox::emit`], which applies the rule's difficulties and gap; the app takes the queue with [`Outbox::take`] and
/// sends it. A guest never queues, and a solo run queues nothing it is not told to take.
#[derive(Debug, Default)]
pub struct Outbox {
    gate: Tracker,
    queue: Vec<Map<String, Value>>,
    /// Guest: the world objects this runner asked the host to change (`net/world_objects.rs`), until the app sends them.
    object_asks: Vec<(super::world_objects::ObjKey, bool)>,
    /// Guest: the host of this room decides the world objects (it said so in its room message), so a pickup, a door or a
    /// light switch is asked for instead of being changed here alone. False for a host, a solo run and an older host.
    pub host_decides_objects: bool,
}

/// Most events and object asks an [`Outbox`] holds: a run whose loop never takes them cannot grow it without end.
const OUTBOX_MAX: usize = 1024;

impl Outbox {
    /// Queues an event of `kind` with `fields` when the rules send that kind on this difficulty and its gap has passed.
    /// Returns whether it was queued. A kind with no rule is never queued.
    pub fn emit(
        &mut self,
        rules: &Rules,
        diff: Difficulty,
        kind: &'static str,
        fields: Vec<(&str, Value)>,
        now: Instant,
    ) -> bool {
        if !self.gate.allow(rules, diff, kind, now) {
            return false;
        }
        let mut o = Map::new();
        o.insert("what".into(), json!(kind));
        for (k, v) in fields {
            o.insert(k.into(), v);
        }
        self.queue.push(o);
        true
    }

    /// Takes the queued events, oldest first.
    pub fn take(&mut self) -> Vec<Map<String, Value>> {
        std::mem::take(&mut self.queue)
    }

    /// Queues an event that sets lasting world state (a world object taken, a door, a light). No rule gates it: a
    /// state change that is not sent leaves the runners in different worlds.
    pub fn push_state(&mut self, event: Map<String, Value>) {
        if self.queue.len() < OUTBOX_MAX {
            self.queue.push(event);
        }
    }

    /// Guest: queues a request to the host to make world object `key` `on` (taken, open or lit).
    pub fn ask_object(&mut self, key: super::world_objects::ObjKey, on: bool) {
        if self.object_asks.len() < OUTBOX_MAX {
            self.object_asks.push((key, on));
        }
    }

    /// Takes the queued object requests, oldest first.
    pub fn take_object_asks(&mut self) -> Vec<(super::world_objects::ObjKey, bool)> {
        std::mem::take(&mut self.object_asks)
    }
}

/// The schema of a late joiner's snapshot (Godot `_snapshot` from `main_3d.gd` `_build_snapshot`, hunter part only).
pub const SNAPSHOT_SCHEMA: &str = "lr.hunter_snapshot/1";

/// A late joiner's hunter state: the hunters as the host's last frame carried them, and the sabotaged relay count.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub hunters: Vec<Value>,
    pub relays_sabotaged: u32,
}

/// The snapshot object the host sends: `schema`, `hunters` (the frame's objects) and `relays_sabotaged`.
pub fn snapshot_state(hunters: &[Value], relays_sabotaged: u32) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("schema".into(), json!(SNAPSHOT_SCHEMA));
    o.insert("hunters".into(), json!(hunters));
    o.insert("relays_sabotaged".into(), json!(relays_sabotaged));
    o
}

/// Reads a snapshot object. Without this schema it is not a snapshot (None). Missing fields read as none, hunter entries
/// that are not objects are dropped, and a relay count that is negative reads as 0 and one too large as u32::MAX.
pub fn parse_snapshot(o: &Map<String, Value>) -> Option<Snapshot> {
    if o.get("schema").and_then(Value::as_str) != Some(SNAPSHOT_SCHEMA) {
        return None;
    }
    let hunters = o
        .get("hunters")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter(|h| h.is_object()).cloned().collect())
        .unwrap_or_default();
    let relays_sabotaged = o
        .get("relays_sabotaged")
        .and_then(Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX));
    Some(Snapshot { hunters, relays_sabotaged })
}

/// The key of a world event that sets lasting world state (a valve's turn count, a chunk's shut-off, a burst pipe, a
/// safe room's status): the events a runner who was not there to hear them must still be told. None for an event that is
/// only a moment (a hunter's state change, a grab, a door, a charge). Two events with one key say the same thing about
/// the same object, so only the later one matters.
pub fn state_key(event: &Map<String, Value>) -> Option<String> {
    let text = |k: &str| event.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    match event.get("what").and_then(Value::as_str)? {
        // a shut-off and a turn of one valve do different things (PipeNetwork::apply): each keeps its own latest event
        super::valve_events::VALVE => {
            let shut = event.get("shut").and_then(Value::as_bool).unwrap_or(false);
            text("valve_id").map(|id| format!("v{}|{id}", u8::from(shut)))
        }
        super::valve_events::BURST => text("flow_id").map(|id| format!("b|{id}")),
        crate::shared::safe_room::EVENT => text("room_id").map(|id| format!("s|{id}")),
        _ => None,
    }
}

/// Most state events a [`StateLog`] keeps (a memory bound, about 250 KB of small objects at the very most; a route has
/// far fewer valves, pipes and safe rooms in reach). The oldest is dropped past it.
pub const STATE_LOG_MAX: usize = 1024;

/// Host: the lasting world state of the running route as the latest event per [`state_key`], in the order each was last
/// sent. A runner that joins (or joins again) during the route is sent these after its snapshot, so its valves, burst
/// pipes and safe rooms match the host's; replaying them is harmless (`valve_events`). Events with no key are not kept.
#[derive(Debug, Default)]
pub struct StateLog {
    entries: Vec<(String, Map<String, Value>, Instant)>,
}

/// The key a replayed event carries for how long ago it happened, in seconds ([`StateLog::replay`]). Optional: an older
/// guest ignores it and starts the flood when the replay arrives.
pub const KEY_AGE: &str = "age";
/// A replayed event younger than this carries no age: the difference is not seen.
const AGE_WORTH_SENDING_S: f64 = 0.5;

/// The age a replayed event says it has, in seconds; 0 without the key, for a bad value, and never more than a day.
pub fn event_age(event: &Map<String, Value>) -> f32 {
    event.get(KEY_AGE).and_then(Value::as_f64).filter(|a| a.is_finite() && *a > 0.0).map_or(0.0, |a| a.min(86_400.0) as f32)
}

impl StateLog {
    /// Keeps `event` when it sets lasting state: it replaces the earlier event of its key and moves to the end, so a
    /// replay runs in the order things last happened. Returns whether it was kept.
    pub fn record(&mut self, event: &Map<String, Value>) -> bool {
        let Some(key) = state_key(event) else { return false };
        if let Some(at) = self.entries.iter().position(|(k, _, _)| *k == key) {
            self.entries.remove(at);
        } else if self.entries.len() >= STATE_LOG_MAX {
            self.entries.remove(0);
        }
        self.entries.push((key, event.clone(), Instant::now()));
        true
    }

    /// The kept events, oldest first.
    pub fn events(&self) -> impl Iterator<Item = &Map<String, Value>> {
        self.entries.iter().map(|(_, e, _)| e)
    }

    /// The kept events as a late joiner is sent them at `now`, oldest first: a valve turn or a burst pipe that happened a
    /// while ago says how long ago ([`KEY_AGE`]), so the joiner's water stands where the host's does and does not start
    /// filling from empty. Any other event, and one that only just happened, is sent as it was.
    pub fn replay(&self, now: Instant) -> Vec<Map<String, Value>> {
        self.entries
            .iter()
            .map(|(_, event, at)| {
                let mut event = event.clone();
                let age = now.saturating_duration_since(*at).as_secs_f64();
                let timed = matches!(
                    event.get("what").and_then(Value::as_str),
                    Some(super::valve_events::VALVE | super::valve_events::BURST)
                );
                if timed && age >= AGE_WORTH_SENDING_S {
                    event.insert(KEY_AGE.into(), json!((age * 10.0).round() / 10.0));
                }
                event
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A new route (or none): nothing of the old one is replayed.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Adds the sending window's origin (the absolute cell of its (0, 0)) to every frame entry that has none, so a guest in
/// another streamed window moves the hunter's cells into its own (`Game::apply_hunter_frame` reads `origin`; a guest
/// too old to know the key ignores it). An entry that already says its origin keeps it.
pub fn stamp_origin(frame: &mut [Value], origin: (i32, i32)) {
    for entry in frame.iter_mut() {
        if let Some(o) = entry.as_object_mut() {
            if !o.contains_key("origin") {
                o.insert("origin".into(), json!([origin.0, origin.1]));
            }
        }
    }
}

/// Splits a hunter frame into parts whose JSON arrays are each at most `max_bytes` long, in order. A guest applies each
/// entry on its own (by hunter id), so a frame sent as several messages shows the same hunters as one. An entry that is
/// longer than `max_bytes` by itself is a part of its own. An empty frame gives no parts.
pub fn split_frame(frame: Vec<Value>, max_bytes: usize) -> Vec<Vec<Value>> {
    let mut parts: Vec<Vec<Value>> = Vec::new();
    let mut used = 0usize;
    for entry in frame {
        // the entry, its comma, and the array's two brackets counted once per part
        let size = serde_json::to_vec(&entry).map_or(0, |b| b.len()) + 1;
        match parts.last_mut() {
            Some(part) if used + size <= max_bytes => {
                used += size;
                part.push(entry);
            }
            _ => {
                used = size + 2;
                parts.push(vec![entry]);
            }
        }
    }
    parts
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
    let action = event.get("action").and_then(Value::as_str);
    match (event.get("what")?.as_str()?, event.get("phase").and_then(Value::as_str)) {
        ("hunter_grab", Some("start")) => Some(format!("{id} HAS THE HOST")),
        ("hunter_grab", Some("end")) => Some(format!("{id} LET THE HOST GO")),
        ("hunter_door", _) if action == Some("close") => Some(format!("{id} SHUT THE DOOR")),
        ("hunter_door", _) => Some(format!("{id} PULLED THE DOOR OPEN")),
        // Godot prints nothing for a locker check (the furniture rattles and the breath plays): no callout
        ("hunter_locker", _) => None,
        ("hunter_charge", _) => Some(format!("{id} CHARGES")),
        ("hunter_ability", _) => Some(format!("{id} POUNCES")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunter_frame_level_and_y_default_when_absent() {
        let old = json!({"index": 0, "id": "a", "position": [1.0, 2.0]});
        assert_eq!(frame_level(&old, 2), 2);
        assert_eq!(frame_y(&old, -1), -5.0, "the floor of the level");
        let new = json!({"level": -1, "presentation_y": -3.5});
        assert_eq!((frame_level(&new, 0), frame_y(&new, -1)), (-1, -3.5));
        for bad in [json!("x"), json!(null), json!(1e39)] {
            let e = json!({"level": bad.clone(), "presentation_y": bad});
            assert_eq!(frame_level(&e, 3), 3);
            assert_eq!(frame_y(&e, 1), 5.0);
        }
    }

    #[test]
    fn snapshot_tolerates_frame_entries_with_the_new_keys() {
        let hunters = vec![json!({"index": 0, "id": "a", "level": -1, "presentation_y": -2.0})];
        let snap = parse_snapshot(&snapshot_state(&hunters, 1)).expect("snapshot");
        assert_eq!(snap.hunters, hunters);
    }

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
    fn the_guest_and_capture_numbers_come_from_the_data_file() {
        let text = include_str!("../data/net/hunter_events.json");
        let r = parse(text).unwrap();
        assert_eq!(r.capture_timeout, Duration::from_secs(15));
        assert_eq!(r.guest_taser_gap, Duration::from_millis(400));
        assert_eq!(r.max_valve_requests, 16);
        assert_eq!(r.skin_recheck, Duration::from_secs(2));
        let tuned = text.replace(r#""timeout_s": 15"#, r#""timeout_s": 4"#).replace(r#""max_valve_requests": 16"#, r#""max_valve_requests": 2"#);
        let t = parse(&tuned).unwrap();
        assert_eq!((t.capture_timeout, t.max_valve_requests), (Duration::from_secs(4), 2), "a changed value changes the rule");
        assert!(parse(&text.replace(r#""timeout_s": 15"#, r#""timeout_s": 0"#)).is_err(), "a timeout of nothing is refused");
        assert!(parse(&text.replace("taser_min_gap_s", "taser_gap")).unwrap_err().contains("taser_gap"));
    }

    #[test]
    fn the_pipe_events_are_sent_on_every_difficulty() {
        let r = parse(include_str!("../data/net/hunter_events.json")).unwrap();
        for diff in [Difficulty::Relaxed, Difficulty::Standard, Difficulty::Hard, Difficulty::Nightmare] {
            assert!(event_on(&r, "stream_valve", diff) && event_on(&r, "stream_pipe_burst", diff), "{diff:?}");
        }
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
    fn a_snapshot_carries_the_hunters_and_the_sabotaged_relays_and_reads_back() {
        let hunters = frame(&[("aki", "CHASE"), ("bo", "IDLE")]);
        let state = snapshot_state(&hunters, 3);
        assert_eq!(state.get("schema").and_then(Value::as_str), Some(SNAPSHOT_SCHEMA));
        assert_eq!(parse_snapshot(&state), Some(Snapshot { hunters, relays_sabotaged: 3 }));
    }

    #[test]
    fn a_snapshot_without_its_schema_is_ignored_and_missing_fields_read_as_none() {
        assert_eq!(parse_snapshot(&Map::new()), None, "no schema: not a snapshot");
        let mut other = snapshot_state(&[], 0);
        other.insert("schema".into(), json!("lr.hunter_snapshot/0"));
        assert_eq!(parse_snapshot(&other), None, "another schema is not read");
        let mut bare = Map::new();
        bare.insert("schema".into(), json!(SNAPSHOT_SCHEMA));
        assert_eq!(parse_snapshot(&bare), Some(Snapshot::default()));
    }

    #[test]
    fn a_snapshot_drops_hunters_that_are_not_objects_and_clamps_the_relay_count() {
        let mut state = snapshot_state(&[], 0);
        state.insert("hunters".into(), json!([{"id": "aki"}, 7, "bo", null]));
        state.insert("relays_sabotaged".into(), json!(-4));
        let read = parse_snapshot(&state).unwrap();
        assert_eq!(read.hunters, vec![json!({"id": "aki"})]);
        assert_eq!(read.relays_sabotaged, 0, "a negative count reads as none");
        state.insert("relays_sabotaged".into(), json!(u64::MAX));
        assert_eq!(parse_snapshot(&state).unwrap().relays_sabotaged, u32::MAX);
    }

    #[test]
    fn state_names_round_trip() {
        for s in [State::Idle, State::Wander, State::Investigate, State::Chase, State::Search, State::Peek] {
            assert_eq!(state_from_name(state_name(s)), Some(s));
        }
        assert_eq!(state_from_name("NOPE"), None);
    }

    #[test]
    fn the_standard_host_events_match_the_ported_difficulties_and_gaps() {
        let r = parse(include_str!("../data/net/hunter_events.json")).unwrap();
        assert!(event_on(&r, "hunter_door", Difficulty::Standard));
        assert!(event_on(&r, "hunter_locker", Difficulty::Relaxed));
        assert!(event_on(&r, "hunter_charge", Difficulty::Hard));
        assert!(!event_on(&r, "hunter_charge", Difficulty::Relaxed), "charge is standard and hard only");
        assert!(!event_on(&r, "hunter_ability", Difficulty::Hard), "ability is nightmare only");
        assert!(event_on(&r, "hunter_ability", Difficulty::Nightmare));
        assert_eq!(r.events["hunter_door"].min_gap, Duration::from_millis(250), "max_per_s 4");
        assert_eq!(r.events["hunter_locker"].min_gap, Duration::from_secs(1), "cooldown_s 1.0");
        assert_eq!(r.events["hunter_charge"].min_gap, Duration::ZERO);
    }

    #[test]
    fn the_outbox_queues_a_kind_only_on_its_difficulties_and_gaps() {
        let r = parse(include_str!("../data/net/hunter_events.json")).unwrap();
        let t0 = Instant::now();
        let mut out = Outbox::default();
        assert!(!out.emit(&r, Difficulty::Nightmare, "hunter_door", vec![], t0), "door is not on nightmare");
        assert!(out.emit(&r, Difficulty::Standard, "hunter_door", vec![("index", json!(0))], t0));
        assert!(
            !out.emit(&r, Difficulty::Standard, "hunter_door", vec![], t0 + Duration::from_millis(100)),
            "max_per_s 4 holds a second door event back for 0.25 s"
        );
        assert!(out.emit(&r, Difficulty::Standard, "hunter_door", vec![], t0 + Duration::from_millis(250)));
        let sent = out.take();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["what"], json!("hunter_door"));
        assert_eq!(sent[0]["index"], json!(0));
        assert!(out.take().is_empty(), "take empties the queue");
    }

    #[test]
    fn a_kind_with_no_rule_is_never_queued() {
        let r = parse(r#"{"schema":"lr.hunter_events/1","frame":{},"events":{}}"#).unwrap();
        let mut out = Outbox::default();
        assert!(!out.emit(&r, Difficulty::Standard, "hunter_charge", vec![], Instant::now()));
        assert!(out.take().is_empty());
    }

    #[test]
    fn changing_the_data_changes_which_host_events_are_queued() {
        let text = include_str!("../data/net/hunter_events.json");
        let hard_charge = text.replace(r#""difficulties": ["standard", "hard"]"#, r#""difficulties": ["hard"]"#);
        assert_ne!(hard_charge, text, "the replacement must apply to the embedded file");
        let r = parse(&hard_charge).unwrap();
        let t = Instant::now();
        let mut out = Outbox::default();
        assert!(!out.emit(&r, Difficulty::Standard, "hunter_charge", vec![], t));
        assert!(out.emit(&r, Difficulty::Hard, "hunter_charge", vec![], t));

        let slow_door = text.replace(r#""max_per_s": 4"#, r#""max_per_s": 1"#);
        assert_ne!(slow_door, text);
        let r = parse(&slow_door).unwrap();
        let mut out = Outbox::default();
        assert!(out.emit(&r, Difficulty::Standard, "hunter_door", vec![], t));
        assert!(!out.emit(&r, Difficulty::Standard, "hunter_door", vec![], t + Duration::from_millis(500)));
        assert!(out.emit(&r, Difficulty::Standard, "hunter_door", vec![], t + Duration::from_secs(1)));
    }

    #[test]
    fn each_host_event_has_a_guest_notice_that_names_the_hunter() {
        let door = json!({"what": "hunter_door", "hunter_id": "aki", "action": "open"});
        assert_eq!(notice(door.as_object().unwrap()).as_deref(), Some("AKI PULLED THE DOOR OPEN"));
        let shut = json!({"what": "hunter_door", "hunter_id": "aki", "action": "close"});
        assert_eq!(notice(shut.as_object().unwrap()).as_deref(), Some("AKI SHUT THE DOOR"));
        let locker = json!({"what": "hunter_locker", "hunter_id": "aki", "spot": 2});
        assert_eq!(notice(locker.as_object().unwrap()), None, "a locker check has no callout (Godot prints none)");
        let charge = json!({"what": "hunter_charge", "hunter_id": "loxelle"});
        assert_eq!(notice(charge.as_object().unwrap()).as_deref(), Some("LOXELLE CHARGES"));
        let pounce = json!({"what": "hunter_ability", "hunter_id": "nyxie", "ability": "pounce"});
        assert_eq!(notice(pounce.as_object().unwrap()).as_deref(), Some("NYXIE POUNCES"));
        let state = json!({"what": "hunter_state", "hunter_id": "aki", "state": "CHASE"});
        assert_eq!(notice(state.as_object().unwrap()), None, "state arrives in the frames, not as a notice");
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn only_events_that_set_lasting_state_have_a_state_key() {
        let turn = obj(json!({"what": "stream_valve", "valve_id": "SEWER:0:-1:0:PROP:3", "effect": "flow", "shut": false, "turns": 1, "gate": ""}));
        let shut = obj(json!({"what": "stream_valve", "valve_id": "SEWER:0:-1:0:PROP:3", "effect": "", "shut": true, "turns": 2, "gate": ""}));
        assert!(state_key(&turn).is_some());
        assert_ne!(state_key(&turn), state_key(&shut), "a shut-off and a turn of one valve are kept apart");
        assert!(state_key(&obj(json!({"what": "stream_pipe_burst", "flow_id": "BRK:0:-1:0:9"}))).is_some());
        assert!(state_key(&obj(json!({"what": "stream_safe_room", "room_id": "R:1:SAFE", "integrity": 2, "breached": false}))).is_some());
        for moment in ["hunter_state", "hunter_grab", "hunter_door", "hunter_locker", "hunter_charge", "hunter_ability"] {
            assert_eq!(state_key(&obj(json!({"what": moment, "hunter_id": "aki"}))), None, "{moment}");
        }
        assert_eq!(state_key(&obj(json!({"what": "stream_valve"}))), None, "no valve id, nothing to key");
        assert_eq!(state_key(&Map::new()), None);
    }

    #[test]
    fn the_state_log_keeps_the_latest_event_of_each_object_in_the_order_they_last_happened() {
        let mut log = StateLog::default();
        let turn = |valve: &str, turns: u32| obj(json!({"what": "stream_valve", "valve_id": valve, "effect": "flow", "shut": false, "turns": turns, "gate": ""}));
        let burst = obj(json!({"what": "stream_pipe_burst", "flow_id": "BRK:0:-1:0:9"}));
        assert!(log.record(&turn("SEWER:0:-1:0:PROP:3", 1)));
        assert!(log.record(&burst));
        assert!(!log.record(&obj(json!({"what": "hunter_door", "hunter_id": "aki"}))), "a moment is not kept");
        assert!(log.record(&turn("SEWER:0:-1:0:PROP:3", 2)), "the same valve again");
        let kept: Vec<&Map<String, Value>> = log.events().collect();
        assert_eq!(kept.len(), 2, "one event per object");
        assert_eq!(kept[0], &burst, "the burst now comes first");
        assert_eq!(kept[1]["turns"], json!(2), "and the valve's latest turn last");
        log.clear();
        assert!(log.is_empty());
    }

    #[test]
    fn the_state_log_is_bounded() {
        let mut log = StateLog::default();
        for i in 0..STATE_LOG_MAX + 10 {
            log.record(&obj(json!({"what": "stream_pipe_burst", "flow_id": format!("BRK:0:-1:0:{i}")})));
        }
        assert_eq!(log.len(), STATE_LOG_MAX);
        assert_eq!(log.events().next().unwrap()["flow_id"], json!("BRK:0:-1:0:10"), "the oldest went first");
    }

    #[test]
    fn a_replayed_flood_says_how_long_ago_it_started_and_an_older_guest_reads_it_unchanged() {
        let mut log = StateLog::default();
        let burst = obj(json!({"what": "stream_pipe_burst", "flow_id": "BRK:0:-1:0:9"}));
        let safe = obj(json!({"what": "stream_safe_room", "room_id": "R:1:SAFE", "integrity": 0, "breached": true}));
        log.record(&burst);
        log.record(&safe);
        let now = Instant::now();
        assert_eq!(log.replay(now), vec![burst.clone(), safe.clone()], "what only just happened is sent as it was");
        let later = log.replay(now + Duration::from_secs(30));
        let age = event_age(&later[0]);
        assert!((29.5..=31.0).contains(&age), "the burst is half a minute old: {age}");
        assert_eq!(later[1], safe, "a safe room's status has no clock");
        // an older guest reads only the flow id: the extra key changes nothing for it
        assert_eq!(crate::net::valve_events::parse_burst(&later[0]), Some("BRK:0:-1:0:9".to_string()));
        assert_eq!(event_age(&burst), 0.0, "no key reads as now");
        let mut odd = burst.clone();
        odd.insert(KEY_AGE.into(), json!(-5));
        assert_eq!(event_age(&odd), 0.0, "an age in the future is not believed");
        odd.insert(KEY_AGE.into(), json!("old"));
        assert_eq!(event_age(&odd), 0.0);
    }

    #[test]
    fn the_outbox_keeps_a_state_event_whatever_the_rules_say_and_hands_object_asks_over_once() {
        let mut out = Outbox::default();
        assert!(!out.host_decides_objects, "a host or a solo run decides its own objects");
        let key = crate::net::world_objects::ObjKey { kind: crate::net::world_objects::Kind::Door, level: 0, x: 3, y: 4 };
        out.push_state(crate::net::world_objects::event(crate::net::world_objects::Op { key, on: true, by: 0 }));
        out.ask_object(key, false);
        assert_eq!(out.take().len(), 1);
        assert_eq!(out.take_object_asks(), vec![(key, false)]);
        assert!(out.take().is_empty() && out.take_object_asks().is_empty());
        for _ in 0..OUTBOX_MAX + 5 {
            out.ask_object(key, true);
        }
        assert_eq!(out.take_object_asks().len(), OUTBOX_MAX, "a loop that never takes them cannot grow the queue for ever");
    }

    #[test]
    fn a_frame_entry_without_an_origin_is_given_the_senders_and_one_with_an_origin_keeps_it() {
        let mut frame = vec![json!({"index": 0, "id": "aki", "position": [1.0, 2.0]}), json!({"index": 1, "id": "bo", "origin": [5, 6]})];
        stamp_origin(&mut frame, (-96, 64));
        assert_eq!(frame[0]["origin"], json!([-96, 64]));
        assert_eq!(frame[1]["origin"], json!([5, 6]));
        assert_eq!(frame[0]["position"], json!([1.0, 2.0]), "the cells themselves are untouched");
    }

    #[test]
    fn a_frame_too_long_for_one_message_is_split_into_parts_that_each_fit_and_lose_no_hunter() {
        let frame: Vec<Value> = (0..40).map(|i| json!({"index": i, "id": format!("hunter_{i}"), "position": [1.5, 2.5], "state": "WANDER"})).collect();
        let whole = serde_json::to_vec(&frame).unwrap().len();
        let max = 400;
        assert!(whole > max);
        let parts = split_frame(frame.clone(), max);
        assert!(parts.len() > 1);
        for part in &parts {
            assert!(serde_json::to_vec(part).unwrap().len() <= max, "each part fits");
        }
        let joined: Vec<Value> = parts.into_iter().flatten().collect();
        assert_eq!(joined, frame, "every hunter, in order");
        assert_eq!(split_frame(frame.clone(), usize::MAX).len(), 1, "a frame that fits stays whole");
        assert!(split_frame(Vec::new(), max).is_empty());
        assert_eq!(split_frame(frame[..2].to_vec(), 1).len(), 2, "an entry too long by itself is a part of its own");
    }
}
