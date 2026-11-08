//! Looping positional emitters at water and lava features (waterfall, flood, lava). Pure maths: the caller (app loop)
//! lists the emitter cells near the runner and plays the bed `bed_of` names at the gains `stereo` returns.
//! Tuning: `data/audio/emitters.json`. No sound asset ships for these; each kind loops an existing ambience bed.

use super::cues::positional::Emitter3D;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

const TEXT: &str = include_str!("../data/audio/emitters.json");

pub struct Kind {
    pub bed: String,
    pub emitter: Emitter3D,
    pub gain: f32,
}

pub struct Config {
    pub cell_metres: f32,
    pub loudest_count: usize,
    pub kinds: HashMap<String, Kind>,
    pub features: HashMap<String, String>,
}

fn f(v: &Value, k: &str) -> f32 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

fn parse(src: &str) -> Config {
    let v: Value = serde_json::from_str(src).expect("emitters.json");
    let mut kinds = HashMap::new();
    for (name, k) in v["kinds"].as_object().expect("emitters.json kinds") {
        kinds.insert(
            name.clone(),
            Kind {
                bed: k["bed"].as_str().unwrap_or("").to_string(),
                emitter: Emitter3D {
                    unit_size: f(k, "unit_size"),
                    max_distance: f(k, "max_distance"),
                    volume_db: f(k, "volume_db"),
                    max_db: f(k, "max_db"),
                    height: f(k, "height"),
                },
                gain: f(k, "gain"),
            },
        );
    }
    let features = v["features"]
        .as_object()
        .map(|o| o.iter().filter_map(|(k, x)| x.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();
    Config {
        cell_metres: f(&v, "cell_metres").max(0.01),
        loudest_count: v["loudest_count"].as_u64().unwrap_or(2) as usize,
        kinds,
        features,
    }
}

pub fn config() -> &'static Config {
    static C: OnceLock<Config> = OnceLock::new();
    C.get_or_init(|| parse(TEXT))
}

/// The emitter kind a feature id plays ("lava_cell" -> "lava"), or None.
pub fn kind_of_feature(feature: &str) -> Option<&'static str> {
    let c = config();
    let k = c.features.get(feature)?;
    c.kinds.get_key_value(k.as_str()).map(|(n, _)| n.as_str())
}

/// The ambience bed an emitter kind loops.
pub fn bed_of(kind: &str) -> Option<&'static str> {
    config().kinds.get(kind).map(|k| k.bed.as_str()).filter(|b| !b.is_empty())
}

/// Linear distance gain of an emitter of `kind` at `cell` heard from `listener` (both in cells).
pub fn gain_at(kind: &str, cell: (f32, f32), listener: (f32, f32)) -> f32 {
    let c = config();
    let Some(k) = c.kinds.get(kind) else { return 0.0 };
    let d = ((cell.0 - listener.0).powi(2) + (cell.1 - listener.1).powi(2)).sqrt() * c.cell_metres;
    k.emitter.distance_gain(d) * k.gain
}

/// Left / right gains for an emitter cell heard from `listener`; `right` and `forward` are the listener's unit
/// vectors in cell space.
pub fn stereo(kind: &str, cell: (f32, f32), listener: (f32, f32), right: (f32, f32), forward: (f32, f32)) -> (f32, f32) {
    let c = config();
    let Some(k) = c.kinds.get(kind) else { return (0.0, 0.0) };
    let dx = (cell.0 - listener.0) * c.cell_metres;
    let dy = (cell.1 - listener.1) * c.cell_metres;
    let local_x = dx * right.0 + dy * right.1;
    let local_z = -(dx * forward.0 + dy * forward.1);
    let (l, r) = k.emitter.stereo_gains([local_x, 0.0, local_z]);
    (l * k.gain, r * k.gain)
}

/// The `loudest_count` loudest emitter cells of `kind` for the runner at `listener`: (cell, gain), loudest first,
/// silent ones dropped. Lava cells come from task 14's lava cell lookup (hook).
pub fn loudest(kind: &str, cells: &[(f32, f32)], listener: (f32, f32)) -> Vec<((f32, f32), f32)> {
    let mut v: Vec<((f32, f32), f32)> =
        cells.iter().map(|c| (*c, gain_at(kind, *c, listener))).filter(|(_, g)| *g > 0.0).collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(config().loudest_count);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_falls_with_distance_and_ends_at_max() {
        let near = gain_at("lava", (0.0, 0.0), (1.0, 0.0));
        let far = gain_at("lava", (0.0, 0.0), (5.0, 0.0));
        assert!(near > far && far > 0.0);
        assert_eq!(gain_at("lava", (0.0, 0.0), (100.0, 0.0)), 0.0);
        assert_eq!(gain_at("nope", (0.0, 0.0), (1.0, 0.0)), 0.0);
    }

    #[test]
    fn features_map_to_kinds_with_existing_beds() {
        assert_eq!(kind_of_feature("lava_cell"), Some("lava"));
        assert_eq!(kind_of_feature("rock"), None);
        for k in config().kinds.keys() {
            let bed = bed_of(k).unwrap();
            assert!(super::super::cues::ambience::settings().beds.contains_key(bed), "{k} bed {bed}");
        }
    }

    #[test]
    fn loudest_keeps_the_nearest_few() {
        let cells = [(10.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0)];
        let v = loudest("waterfall", &cells, (0.0, 0.0));
        assert_eq!(v.len(), config().loudest_count);
        assert_eq!(v[0].0, (1.0, 0.0));
        assert!(v[0].1 >= v[1].1);
    }

    #[test]
    fn a_source_on_the_right_is_louder_in_the_right_ear() {
        let (l, r) = stereo("flood", (3.0, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, -1.0));
        assert!(r > l);
    }
}
