//! Night Watch's data: the headset call scripts (copied from Godot's data/night_watch/calls, schema nw.calls/1) and
//! the call timings from the 2D core/callbox.js. The call files are embedded so the executable needs no data folder.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

/// "Incoming call..." shows this long before the first line (the ring).
pub const CALL_INCOMING_MS: f64 = 2200.0;
// Letter speed, punctuation pauses, hold time and the speaker voices live in data/calls_ui.json (see `calls_ui`).
pub const CALL_INCOMING_TEXT: &str = "Incoming call…";

/// One night's call: the seconds into the night she rings, and the lines she says. `speakers` runs parallel to `lines`
/// (an empty id is the default speaker); a line is a bare string (Zellia) or `{"speaker": id, "text": line}`.
#[derive(Clone, Debug)]
pub struct CallEntry {
    pub at: u32,
    pub lines: Vec<String>,
    pub speakers: Vec<String>,
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
    let mut lines = Vec::new();
    let mut speakers = Vec::new();
    for l in row["lines"].as_array()? {
        if let Some(text) = l.as_str() {
            lines.push(text.to_string());
            speakers.push(String::new());
        } else if let Some(text) = l["text"].as_str() {
            lines.push(text.to_string());
            speakers.push(l["speaker"].as_str().unwrap_or("").to_string());
        }
    }
    Some((level, CallEntry { at, lines, speakers }))
}

// ---- Mode data registry (see docs/NIGHT_WATCH_DATA.md). Every file is listed here once, so mode agents only edit JSON.
// Dead code is allowed on the accessors until the modes read their tunables from here; the registry test exercises them.

pub const RULES_FILES: [(&str, &str); 9] = [
    ("classic", include_str!("data/rules/classic.json")),
    ("bedtime", include_str!("data/rules/bedtime.json")),
    ("salvage_run", include_str!("data/rules/salvage_run.json")),
    ("descent", include_str!("data/rules/descent.json")),
    ("masked_hours", include_str!("data/rules/masked_hours.json")),
    ("dead_air", include_str!("data/rules/dead_air.json")),
    ("facility", include_str!("data/rules/facility.json")),
    ("open_air", include_str!("data/rules/open_air.json")),
    ("mixed_shift", include_str!("data/rules/mixed_shift.json")),
];
pub const CAST_FILES: [(&str, &str); 7] = [
    ("classic", include_str!("data/cast/classic.json")),
    ("bedtime", include_str!("data/cast/bedtime.json")),
    ("dead_air", include_str!("data/cast/dead_air.json")),
    ("masked_hours", include_str!("data/cast/masked_hours.json")),
    ("facility", include_str!("data/cast/facility.json")),
    ("open_air", include_str!("data/cast/open_air.json")),
    ("mixed_shift", include_str!("data/cast/mixed_shift.json")),
];
const BUILDING_FILES: [(&str, &str); 6] = [
    ("classic", include_str!("data/buildings/classic.json")),
    ("bedtime", include_str!("data/buildings/bedtime.json")),
    ("dead_air", include_str!("data/buildings/dead_air.json")),
    ("masked_hours", include_str!("data/buildings/masked_hours.json")),
    ("facility", include_str!("data/buildings/facility.json")),
    ("mixed_shift", include_str!("data/buildings/mixed_shift.json")),
];

/// The call box's data: typewriter speed and pauses, blip sound and the speakers' voices (schema nw.calls_ui/1).
const CALLS_UI: &str = include_str!("data/calls_ui.json");

/// The parsed call box data. Panics on bad JSON: a broken embedded file is a content bug.
pub fn calls_ui() -> &'static Value {
    static UI: OnceLock<Value> = OnceLock::new();
    UI.get_or_init(|| serde_json::from_str(CALLS_UI).unwrap_or_else(|e| panic!("night_watch data/calls_ui.json does not parse: {e}")))
}

/// Every hunter cut-out, embedded at build time: (hunter id, front PNG, back PNG). `sprites.rs` decodes them once.
pub const HUNTER_SPRITE_FILES: [(&str, &[u8], &[u8]); 25] = [
    ("aki", include_bytes!("assets/hunters/aki-front-cutout.png"), include_bytes!("assets/hunters/aki-back-cutout.png")),
    ("briselle", include_bytes!("assets/hunters/briselle-front-cutout.png"), include_bytes!("assets/hunters/briselle-back-cutout.png")),
    ("ceryth", include_bytes!("assets/hunters/ceryth-front-cutout.png"), include_bytes!("assets/hunters/ceryth-back-cutout.png")),
    ("dovelle", include_bytes!("assets/hunters/dovelle-front-cutout.png"), include_bytes!("assets/hunters/dovelle-back-cutout.png")),
    ("elise", include_bytes!("assets/hunters/elise-front-cutout.png"), include_bytes!("assets/hunters/elise-back-cutout.png")),
    ("ferryn", include_bytes!("assets/hunters/ferryn-front-cutout.png"), include_bytes!("assets/hunters/ferryn-back-cutout.png")),
    ("glyra", include_bytes!("assets/hunters/glyra-front-cutout.png"), include_bytes!("assets/hunters/glyra-back-cutout.png")),
    ("haelith", include_bytes!("assets/hunters/haelith-front-cutout.png"), include_bytes!("assets/hunters/haelith-back-cutout.png")),
    ("ilyra", include_bytes!("assets/hunters/ilyra-front-cutout.png"), include_bytes!("assets/hunters/ilyra-back-cutout.png")),
    ("javelle", include_bytes!("assets/hunters/javelle-front-cutout.png"), include_bytes!("assets/hunters/javelle-back-cutout.png")),
    ("kathy", include_bytes!("assets/hunters/kathy-front-cutout.png"), include_bytes!("assets/hunters/kathy-back-cutout.png")),
    ("loxelle", include_bytes!("assets/hunters/loxelle-front-cutout.png"), include_bytes!("assets/hunters/loxelle-back-cutout.png")),
    ("mavrine", include_bytes!("assets/hunters/mavrine-front-cutout.png"), include_bytes!("assets/hunters/mavrine-back-cutout.png")),
    ("nyxie", include_bytes!("assets/hunters/nyxie-front-cutout.png"), include_bytes!("assets/hunters/nyxie-back-cutout.png")),
    ("opalie", include_bytes!("assets/hunters/opalie-front-cutout.png"), include_bytes!("assets/hunters/opalie-back-cutout.png")),
    ("porcelle", include_bytes!("assets/hunters/porcelle-front-cutout.png"), include_bytes!("assets/hunters/porcelle-back-cutout.png")),
    ("quenelle", include_bytes!("assets/hunters/quenelle-front-cutout.png"), include_bytes!("assets/hunters/quenelle-back-cutout.png")),
    ("ravelle", include_bytes!("assets/hunters/ravelle-front-cutout.png"), include_bytes!("assets/hunters/ravelle-back-cutout.png")),
    ("solyth", include_bytes!("assets/hunters/solyth-front-cutout.png"), include_bytes!("assets/hunters/solyth-back-cutout.png")),
    ("thessine", include_bytes!("assets/hunters/thessine-front-cutout.png"), include_bytes!("assets/hunters/thessine-back-cutout.png")),
    ("umbriel", include_bytes!("assets/hunters/umbriel-front-cutout.png"), include_bytes!("assets/hunters/umbriel-back-cutout.png")),
    ("verrine", include_bytes!("assets/hunters/verrine-front-cutout.png"), include_bytes!("assets/hunters/verrine-back-cutout.png")),
    ("wynelle", include_bytes!("assets/hunters/wynelle-front-cutout.png"), include_bytes!("assets/hunters/wynelle-back-cutout.png")),
    ("xivvy", include_bytes!("assets/hunters/xivvy-front-cutout.png"), include_bytes!("assets/hunters/xivvy-back-cutout.png")),
    ("ysolde", include_bytes!("assets/hunters/ysolde-front-cutout.png"), include_bytes!("assets/hunters/ysolde-back-cutout.png")),
];

/// Classic's office art from the web build (480 x 270): the back wall and its foreground overlay. `classic_play.rs` decodes them.
pub const CLASSIC_OFFICE_PNG: &[u8] = include_bytes!("assets/offices/classic.png");
pub const CLASSIC_OFFICE_FG_PNG: &[u8] = include_bytes!("assets/offices/classic-fg.png");

/// Classic's camera views, one per room the cameras cover (480 x 270), keyed by room id.
pub const CLASSIC_CAM_FILES: [(&str, &[u8]); 8] = [
    ("backstage", include_bytes!("assets/cams/classic/backstage.png")),
    ("closet", include_bytes!("assets/cams/classic/closet.png")),
    ("corner", include_bytes!("assets/cams/classic/corner.png")),
    ("dining", include_bytes!("assets/cams/classic/dining.png")),
    ("easthall", include_bytes!("assets/cams/classic/easthall.png")),
    ("kitchen", include_bytes!("assets/cams/classic/kitchen.png")),
    ("stage", include_bytes!("assets/cams/classic/stage.png")),
    ("westhall", include_bytes!("assets/cams/classic/westhall.png")),
];

/// The hunter sprite file: each hunter's scale and feet anchor (schema nw.sprites/1). Panics on bad JSON, like the other data.
const SPRITES_JSON: &str = include_str!("data/sprites.json");

/// The parsed hunter sprite file. Panics on bad JSON: a broken embedded file is a content bug.
pub fn sprites_config() -> &'static Value {
    static SPRITES: OnceLock<Value> = OnceLock::new();
    SPRITES.get_or_init(|| serde_json::from_str(SPRITES_JSON).unwrap_or_else(|e| panic!("night_watch data/sprites.json does not parse: {e}")))
}

#[allow(dead_code)]
struct ModeData {
    rules: HashMap<&'static str, Value>,
    cast: HashMap<&'static str, Value>,
    building: HashMap<&'static str, Value>,
}

#[allow(dead_code)]
fn parse_all(kind: &str, files: &[(&'static str, &str)]) -> HashMap<&'static str, Value> {
    files
        .iter()
        .map(|(mode, text)| {
            let value: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("night_watch data {kind}/{mode}.json does not parse: {e}"));
            (*mode, value)
        })
        .collect()
}

/// Every registered file, parsed once on first use.
#[allow(dead_code)]
fn mode_data() -> &'static ModeData {
    static DATA: OnceLock<ModeData> = OnceLock::new();
    DATA.get_or_init(|| ModeData {
        rules: parse_all("rules", &RULES_FILES),
        cast: parse_all("cast", &CAST_FILES),
        building: parse_all("buildings", &BUILDING_FILES),
    })
}

/// The rules file of `mode_id`. Panics when the mode has none: a missing file is a content bug.
#[allow(dead_code)]
pub fn rules(mode_id: &str) -> &'static Value {
    mode_data().rules.get(mode_id).unwrap_or_else(|| panic!("night_watch: no rules file for mode '{mode_id}'"))
}

/// The cast file of `mode_id` (hunters, phantoms, NPCs). Panics when the mode has none.
#[allow(dead_code)]
pub fn cast(mode_id: &str) -> &'static Value {
    mode_data().cast.get(mode_id).unwrap_or_else(|| panic!("night_watch: no cast file for mode '{mode_id}'"))
}

/// The building file of `mode_id` (rooms, graph, cameras). Panics when the mode has none.
#[allow(dead_code)]
pub fn building(mode_id: &str) -> &'static Value {
    mode_data().building.get(mode_id).unwrap_or_else(|| panic!("night_watch: no building file for mode '{mode_id}'"))
}

/// Follows a dotted path ("night_ms", "doors.grace_ms.2") through objects and array indexes.
#[allow(dead_code)]
fn dig<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |cur, key| match key.parse::<usize>() {
        Ok(i) if cur.is_array() => cur.get(i),
        _ => cur.get(key),
    })
}

#[allow(dead_code)]
fn mode_of(v: &Value) -> &str {
    v.get("mode").and_then(Value::as_str).unwrap_or("?")
}

/// The value at `path`, or a panic naming the mode and key (a missing key is a content bug, not a runtime condition).
#[allow(dead_code)]
fn need<'a>(v: &'a Value, path: &str) -> &'a Value {
    dig(v, path).unwrap_or_else(|| panic!("night_watch data for mode '{}' is missing required key '{path}'", mode_of(v)))
}

/// A required number at `path`.
#[allow(dead_code)]
pub fn num(v: &Value, path: &str) -> f64 {
    need(v, path).as_f64().unwrap_or_else(|| panic!("night_watch data for mode '{}': key '{path}' is not a number", mode_of(v)))
}

/// A required integer at `path`.
#[allow(dead_code)]
pub fn int(v: &Value, path: &str) -> i64 {
    need(v, path).as_i64().unwrap_or_else(|| panic!("night_watch data for mode '{}': key '{path}' is not an integer", mode_of(v)))
}

/// A required string at `path`.
#[allow(dead_code)]
pub fn text<'a>(v: &'a Value, path: &str) -> &'a str {
    need(v, path).as_str().unwrap_or_else(|| panic!("night_watch data for mode '{}': key '{path}' is not a string", mode_of(v)))
}

/// A required array at `path`.
#[allow(dead_code)]
pub fn list<'a>(v: &'a Value, path: &str) -> &'a [Value] {
    need(v, path).as_array().map(Vec::as_slice).unwrap_or_else(|| panic!("night_watch data for mode '{}': key '{path}' is not a list", mode_of(v)))
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
    fn every_registered_data_file_parses_and_names_its_mode() {
        for (kind, files) in [("rules", &RULES_FILES[..]), ("cast", &CAST_FILES[..]), ("buildings", &BUILDING_FILES[..])] {
            for (mode, body) in files {
                let v: Value = serde_json::from_str(body).unwrap_or_else(|e| panic!("{kind}/{mode}.json: {e}"));
                assert_eq!(v["mode"], *mode, "{kind}/{mode}.json carries the wrong mode id");
                assert!(int(&v, "version") >= 1);
            }
        }
        for (mode, _) in RULES_FILES {
            assert_eq!(text(rules(mode), "mode"), mode);
        }
        assert_eq!(text(cast("classic"), "mode"), "classic");
        assert_eq!(text(building("classic"), "mode"), "classic");
    }

    #[test]
    fn typed_helpers_follow_dotted_paths() {
        let v = serde_json::json!({"mode": "t", "a": {"b": [1.5, 2]}, "s": "x", "l": [1]});
        assert_eq!(num(&v, "a.b.0"), 1.5);
        assert_eq!(int(&v, "a.b.1"), 2);
        assert_eq!(text(&v, "s"), "x");
        assert_eq!(list(&v, "l").len(), 1);
    }

    #[test]
    #[should_panic(expected = "mode 't' is missing required key 'nope'")]
    fn a_missing_key_panics_naming_mode_and_key() {
        num(&serde_json::json!({"mode": "t"}), "nope");
    }

    #[test]
    fn a_mode_without_a_call_gives_none() {
        assert!(CallsData::load().for_level("no_such_mode", 1).is_none());
    }
}
