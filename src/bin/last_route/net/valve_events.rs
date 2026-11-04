//! The sewer pipe events on the wire (Godot `_request_valve` / `_accept_valve` / `_apply_valve_event`, and the Axe pipe
//! burst). Transport only: the model is `game::pipes`, and the tuning (limits, queue size) is
//! `data/net/hunter_events.json` plus `data/pipes.json`.
//!
//! * A guest sends a [`ValveRequest`] (`RequestInteraction`, what = `stream_valve_request`) naming the valve, its
//!   effect and gate, and where the valve is. The host checks the guest's own sample, runs `PipeNetwork::turn` with
//!   `by_player = false` (a remote runner's noise is its own machine's), and publishes the result.
//! * The host publishes a turn as a `stream_valve` world event ([`turn_fields`]) and a burst pipe as
//!   `stream_pipe_burst` ([`burst_fields`]). Every machine, the host included, applies them with
//!   `Game::apply_pipe_event`; replays are harmless because `PipeNetwork::apply` sets the same state again.

use crate::game::pipes::{self, TurnEvent};
use serde_json::{json, Map, Value};

/// The `what` of a guest's request to turn a valve.
pub const REQUEST: &str = "stream_valve_request";
/// The `what` of the host's published valve turn.
pub const VALVE: &str = "stream_valve";
/// The `what` of the host's published pipe burst.
pub const BURST: &str = "stream_pipe_burst";

/// A guest's valve turn request as the host holds it until its game answers.
#[derive(Clone, Debug, PartialEq)]
pub struct ValveRequest {
    pub peer: u32,
    pub valve_id: String,
    /// The prop's `valve_effect` ("" is a flood valve) and `gate` keys.
    pub effect: String,
    pub gate: String,
    /// The valve's absolute cell, as the guest read it from its prop.
    pub valve_at: (f32, f32),
}

/// The request a guest sends.
pub fn request_message(valve_id: &str, effect: &str, gate: &str, valve_at: (f32, f32)) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("what".into(), json!(REQUEST));
    o.insert("valve_id".into(), json!(valve_id));
    o.insert("effect".into(), json!(effect));
    o.insert("gate".into(), json!(gate));
    o.insert("x".into(), json!(valve_at.0));
    o.insert("y".into(), json!(valve_at.1));
    o
}

fn text(o: &Map<String, Value>, key: &str, max: usize) -> Option<String> {
    let s = o.get(key).and_then(Value::as_str).unwrap_or_default();
    (s.len() <= max).then(|| s.to_string())
}

fn finite(o: &Map<String, Value>, key: &str) -> Option<f32> {
    o.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32)
}

/// Reads a guest's request. `None` for another kind, an id that is not a valve id, an effect the pipes data does not
/// know, or a position that is not a finite pair; text is bounded by the pipes data `max_id_length`.
pub fn parse_request(peer: u32, o: &Map<String, Value>) -> Option<ValveRequest> {
    if o.get("what").and_then(Value::as_str) != Some(REQUEST) {
        return None;
    }
    let r = pipes::rules();
    let valve_id = text(o, "valve_id", r.valve.max_id_length)?;
    pipes::parse_valve_id(&valve_id, r.valve.max_id_length)?;
    let effect = text(o, "effect", r.valve.max_id_length)?;
    r.effect(&effect)?;
    let gate = text(o, "gate", r.valve.max_id_length)?;
    Some(ValveRequest { peer, valve_id, effect, gate, valve_at: (finite(o, "x")?, finite(o, "y")?) })
}

/// The fields of a `stream_valve` event for `Outbox::emit` (the noise stays with the machine that turned the wheel).
pub fn turn_fields(ev: &TurnEvent) -> Vec<(&'static str, Value)> {
    vec![
        ("valve_id", json!(ev.valve_id)),
        ("effect", json!(ev.effect)),
        ("shut", json!(ev.shut)),
        ("turns", json!(ev.turns)),
        ("gate", json!(ev.gate)),
    ]
}

/// The fields of a `stream_pipe_burst` event: the flow id the Axe opened.
pub fn burst_fields(flow_id: &str) -> Vec<(&'static str, Value)> {
    vec![("flow_id", json!(flow_id))]
}

/// A published valve turn, read back. The chunk comes from the valve id, never from the sender.
pub fn parse_turn(o: &Map<String, Value>) -> Option<TurnEvent> {
    if o.get("what").and_then(Value::as_str) != Some(VALVE) {
        return None;
    }
    let max = pipes::rules().valve.max_id_length;
    let valve_id = text(o, "valve_id", max)?;
    let chunk = pipes::parse_valve_id(&valve_id, max)?;
    let turns = o.get("turns").and_then(Value::as_u64).and_then(|t| u32::try_from(t).ok())?;
    Some(TurnEvent {
        valve_id,
        chunk,
        effect: text(o, "effect", max)?,
        shut: o.get("shut").and_then(Value::as_bool).unwrap_or(false),
        turns,
        gate: text(o, "gate", max)?,
        noise: None,
    })
}

/// A published pipe burst, read back: the flow id (`BRK:x:y:z:n`), bounded like a valve id.
pub fn parse_burst(o: &Map<String, Value>) -> Option<String> {
    if o.get("what").and_then(Value::as_str) != Some(BURST) {
        return None;
    }
    let id = text(o, "flow_id", pipes::rules().valve.max_id_length)?;
    id.starts_with("BRK:").then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALVE_ID: &str = "SEWER:0:-1:0:PROP:7";

    fn with_what(fields: Vec<(&'static str, Value)>, what: &str) -> Map<String, Value> {
        let mut o = Map::new();
        o.insert("what".into(), json!(what));
        for (k, v) in fields {
            o.insert(k.into(), v);
        }
        o
    }

    #[test]
    fn a_request_round_trips_and_carries_the_sender() {
        let m = request_message(VALVE_ID, "flow", "", (4.5, 9.5));
        let r = parse_request(3, &m).expect("a valid request parses");
        assert_eq!((r.peer, r.valve_id.as_str(), r.effect.as_str(), r.valve_at), (3, VALVE_ID, "flow", (4.5, 9.5)));
    }

    #[test]
    fn a_request_with_a_bad_id_effect_or_position_is_dropped() {
        assert!(parse_request(2, &request_message("nope", "flow", "", (0.0, 0.0))).is_none());
        assert!(parse_request(2, &request_message(VALVE_ID, "explode", "", (0.0, 0.0))).is_none());
        let mut m = request_message(VALVE_ID, "flow", "", (0.0, 0.0));
        m.remove("x");
        assert!(parse_request(2, &m).is_none(), "a position with no x is not a position");
        let long = format!("{VALVE_ID}{}", "9".repeat(pipes::rules().valve.max_id_length));
        assert!(parse_request(2, &request_message(&long, "flow", "", (0.0, 0.0))).is_none(), "ids are bounded by data");
        let mut other = request_message(VALVE_ID, "flow", "", (0.0, 0.0));
        other.insert("what".into(), json!("taser"));
        assert!(parse_request(2, &other).is_none());
    }

    #[test]
    fn a_published_turn_reads_back_with_its_chunk_from_the_id() {
        let ev = TurnEvent {
            valve_id: VALVE_ID.into(),
            chunk: (0, -1, 0),
            effect: "flow".into(),
            shut: false,
            turns: 3,
            gate: String::new(),
            noise: Some((9.0, 1.6)),
        };
        let back = parse_turn(&with_what(turn_fields(&ev), VALVE)).expect("parses");
        assert_eq!((back.valve_id.as_str(), back.chunk, back.turns, back.shut), (VALVE_ID, (0, -1, 0), 3, false));
        assert_eq!(back.noise, None, "the noise never travels, so a remote never feeds a danger cue");
        assert!(parse_turn(&with_what(turn_fields(&ev), "other")).is_none());
        let mut bad = with_what(turn_fields(&ev), VALVE);
        bad.insert("turns".into(), json!(-1));
        assert!(parse_turn(&bad).is_none());
    }

    #[test]
    fn a_burst_names_a_flow_id_and_nothing_else() {
        let flow = format!("{}1", pipes::flow_prefix((1, -1, 2)));
        assert_eq!(parse_burst(&with_what(burst_fields(&flow), BURST)), Some(flow));
        assert!(parse_burst(&with_what(burst_fields("junk"), BURST)).is_none());
        assert!(parse_burst(&with_what(burst_fields("BRK:0:0:0:1"), VALVE)).is_none());
    }
}
