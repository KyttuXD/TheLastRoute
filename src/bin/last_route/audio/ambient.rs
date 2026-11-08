//! Which ambience bed plays where: region beds on the finite and streamed routes, and level-layer beds (sewer,
//! Inferno) that win over the region. Data: `layer_beds` in data/audio/ambience.json; region beds, aliases and the
//! fallback stay in `cues::ambience`.

use super::cues::ambience::{bed_for, has_bed};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

const TEXT: &str = include_str!("../data/audio/ambience.json");

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

/// The bed id for a level layer name ("sewer", "inferno"), or None for a layer without its own bed.
pub fn layer_bed(layer: &str) -> Option<&'static str> {
    let id = layer_beds().get(layer.trim().to_lowercase().as_str())?;
    has_bed(id).then(|| bed_for(id))
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
}
