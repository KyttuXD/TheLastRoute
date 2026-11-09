//! Which ambience bed plays where: region beds on the finite and streamed routes, and level-layer beds (sewer,
//! Inferno, and the deep fallback) that win over the region. Data: `layer_beds` and `deep_below_level` in
//! data/audio/ambience.json; region beds, aliases and the fallback stay in `cues::ambience`.

use super::cues::ambience::{bed_for, has_bed};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

const TEXT: &str = include_str!("../data/audio/ambience.json");

/// The layer name a level deeper than `deep_below_level` takes when the levels table names it none.
const DEEP_LAYER: &str = "deep";

fn layer_beds() -> &'static HashMap<String, String> {
    static L: OnceLock<HashMap<String, String>> = OnceLock::new();
    L.get_or_init(|| {
        let v: Value = serde_json::from_str(TEXT).unwrap_or(Value::Null);
        v.get("layer_beds")
            .and_then(Value::as_object)
            .map(|o| o.iter().filter_map(|(k, x)| x.as_str().map(|s| (k.to_lowercase(), s.to_string()))).collect())
            .unwrap_or_default()
    })
}

/// The level below which (exclusive) a level with no named layer takes the deep layer. The file's `deep_below_level`
/// when it names one; else the deepest level of the levels table (`levels::bottom`, read each call so a pack's levels
/// count, -3 with the shipped table).
fn deep_below() -> i32 {
    static D: OnceLock<Option<i32>> = OnceLock::new();
    let from_file = *D.get_or_init(|| {
        let v: Value = serde_json::from_str(TEXT).unwrap_or(Value::Null);
        v.get("deep_below_level").and_then(Value::as_i64).map(|n| n as i32)
    });
    from_file.unwrap_or_else(crate::world::levels::bottom)
}

/// The bed id for a level layer name ("sewer", "inferno", "deep"), or None for a layer without its own bed.
pub fn layer_bed(layer: &str) -> Option<&'static str> {
    let id = layer_beds().get(layer.trim().to_lowercase().as_str())?;
    has_bed(id).then(|| bed_for(id))
}

/// The level layer whose bed wins over the region's, for a level of the Infinite route: none on level 0 and above; the
/// levels table's layer name for a level that has one; else the deep layer for a level deeper than `deep_below_level`.
pub fn layer_for_level(level: i32) -> Option<&'static str> {
    if level >= 0 {
        return None;
    }
    crate::world::levels::layer_name(level).or_else(|| (level < deep_below()).then_some(DEEP_LAYER))
}

/// The bed the runner hears: the layer's bed when the level layer has one, else the region's bed (own, alias or
/// fallback). None with neither a layer nor a region (menus). Pass the result to `world::ambience` as the region id.
pub fn bed_id_for(layer: Option<&str>, region: Option<&str>) -> Option<&'static str> {
    if let Some(b) = layer.and_then(layer_bed) {
        return Some(b);
    }
    region.map(bed_for).filter(|b| !b.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_wins_over_region() {
        assert_eq!(bed_id_for(Some("sewer"), Some("lab")), Some("sewer"));
        assert_eq!(bed_id_for(Some("inferno"), Some("lab")), Some("inferno"));
        assert_eq!(bed_id_for(Some("surface"), Some("lab")), Some("lab"));
    }

    #[test]
    fn region_bed_when_no_layer() {
        assert_eq!(bed_id_for(None, Some("prison")), Some("prison"));
        assert_eq!(bed_id_for(None, Some("hotel")), Some("apartments"));
        assert_eq!(bed_id_for(None, None), None);
    }

    #[test]
    fn unknown_region_falls_back() {
        assert!(bed_id_for(None, Some("zzz")).is_some());
    }

    #[test]
    fn the_inferno_bed_is_unchanged_and_a_deeper_layer_gets_the_default_bed() {
        assert_eq!(layer_bed("inferno"), Some("inferno"));
        assert_eq!(layer_bed("deep"), layer_bed("sewer"), "the deep fallback is the sewer-like bed from data");
        assert_eq!(bed_id_for(Some("deep"), Some("lab")), layer_bed("sewer"));
    }

    #[test]
    fn level_zero_and_up_never_take_a_layer_bed() {
        assert_eq!(layer_for_level(0), None);
        assert_eq!(layer_for_level(4), None);
    }
}
