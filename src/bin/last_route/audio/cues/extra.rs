//! Data-defined one-shot cues and game events (data/audio/cues.json). The Godot project has no dedicated sound for a
//! locker, a relay pickup, the music box wind or jam, a vent hatch, or a hunter sniffing or knocking, so the port builds
//! them from the same tone / thump / noise primitives as sound.gd. Every number is data. A cue may name an embedded
//! `file` under assets/audio/ instead (none ship: the Godot project has no sound files to copy).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use super::render::{noise, thump, tone, Component};
use super::rules::{bus_for_cue, Play};

struct Part {
    kind: String,
    hz: f64,
    dur: f64,
    vol: f64,
    delay: f64,
    wave: String,
    rate: f64,
}

/// How a game event plays a cue.
#[derive(Clone, Debug)]
pub struct Event {
    pub cue: String,
    pub intensity: f64,
    pub gain: f32,
    /// Played at the caller's volume (`gain = volume / 0.05`, 0.1..2) like a hunter step.
    pub scaled: bool,
}

struct Data {
    cues: HashMap<String, Vec<Part>>,
    files: HashMap<String, String>,
    events: HashMap<String, Event>,
}

fn num(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn parse(text: &str) -> Data {
    let root: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let mut data = Data { cues: HashMap::new(), files: HashMap::new(), events: HashMap::new() };
    if let Some(cues) = root.get("cues").and_then(Value::as_object) {
        for (name, parts) in cues {
            let list: Vec<Part> = parts
                .as_array()
                .map(|a| a.iter().map(part).collect())
                .unwrap_or_default();
            data.cues.insert(name.clone(), list);
        }
    }
    if let Some(files) = root.get("files").and_then(Value::as_object) {
        for (name, f) in files {
            if let Some(f) = f.as_str() {
                data.files.insert(name.clone(), f.to_string());
            }
        }
    }
    if let Some(events) = root.get("events").and_then(Value::as_object) {
        for (name, e) in events {
            let cue = e.get("cue").and_then(Value::as_str).unwrap_or("").to_string();
            data.events.insert(
                name.clone(),
                Event {
                    cue,
                    intensity: num(e, "intensity", 1.0).clamp(0.0, 4.0),
                    gain: num(e, "gain", 1.0).clamp(0.0, 4.0) as f32,
                    scaled: e.get("scaled").and_then(Value::as_bool).unwrap_or(false),
                },
            );
        }
    }
    data
}

fn part(v: &Value) -> Part {
    Part {
        kind: v.get("kind").and_then(Value::as_str).unwrap_or("tone").to_string(),
        hz: num(v, "hz", 440.0),
        dur: num(v, "dur", 0.1).clamp(0.01, 3.0),
        vol: num(v, "vol", 0.05),
        delay: num(v, "delay", 0.0).clamp(0.0, 3.0),
        wave: v.get("wave").and_then(Value::as_str).unwrap_or("sine").to_string(),
        rate: num(v, "rate", 1.0),
    }
}

fn data() -> &'static Data {
    static DATA: OnceLock<Data> = OnceLock::new();
    DATA.get_or_init(|| {
        let text = crate::content::embedded::data_under("audio/cues.json")
            .next()
            .map(|x| x.1)
            .unwrap_or("{}");
        parse(text)
    })
}

/// The data-defined cue names, sorted.
pub fn names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = data().cues.keys().map(String::as_str).collect();
    v.sort_unstable();
    v
}

/// Every game event name, sorted.
pub fn event_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = data().events.keys().map(String::as_str).collect();
    v.sort_unstable();
    v
}

/// The embedded asset a cue names, when it names one.
pub fn file_of(cue: &str) -> Option<&'static str> {
    data().files.get(cue).map(String::as_str)
}

/// A data cue's components at `pan` and `intensity` (which scales every volume), or `None` for an unknown name.
pub fn components(cue: &str, pan: f64, intensity: f64) -> Option<Vec<Component>> {
    let parts = data().cues.get(cue)?;
    Some(
        parts
            .iter()
            .map(|p| {
                let vol = p.vol * intensity;
                match p.kind.as_str() {
                    "thump" => thump(p.hz, p.dur, vol, p.delay, pan),
                    "noise" => noise(p.dur, vol, p.hz, p.delay, pan, p.rate),
                    _ => tone(p.hz, p.dur, vol, p.delay, &p.wave, pan),
                }
            })
            .collect(),
    )
}

/// The `Play` of a game event, panned `pan` (-1..1). `volume` is the caller's loudness for a scaled event (the 0.05
/// of a hunter step is gain 1); a plain event ignores it. `None` for an unknown event.
pub fn event(name: &str, pan: f64, volume: f64) -> Option<Play> {
    let e = data().events.get(name)?;
    if e.scaled {
        return Some(Play {
            cue: e.cue.clone(),
            pan: super::rules::snap_scaled_pan(pan),
            intensity: 1.0,
            gain: (volume / 0.05).clamp(0.1, 2.0) as f32 * e.gain,
            bus: bus_for_cue(&e.cue),
        });
    }
    Some(Play { cue: e.cue.clone(), pan: pan.clamp(-1.0, 1.0), intensity: e.intensity, gain: e.gain, bus: bus_for_cue(&e.cue) })
}

/// The cue an event plays, for a test or a hook that wants to pre-load it.
pub fn event_cue(name: &str) -> Option<&'static str> {
    data().events.get(name).map(|e| e.cue.as_str())
}
