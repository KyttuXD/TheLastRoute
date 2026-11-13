//! Draw scale per prop type: Godot's PropModelCatalog.HUNTER_SCALE (the person-sized models are
//! drawn at 1.5x, a few at 1.2 to 1.48, and any type not listed at 1.0, as hunter_scale() does).
//! The table is data/render/prop_draw_scale.json, compiled in by build.rs. Only drawing uses it;
//! placement, collision and footprints do not (the generator reads it for seat heights, so it stays Godot's).
//!
//! A second, Rust-only table (data/render/prop_axis_scale.json) stretches a few types per axis on top of it: the
//! couches, the beach chair and the gothic high-back chair, whose converted original models do not match the collider
//! Godot derived from its blocky ones. That stretch is baked into the model when it is loaded ([`model_axis_scale`]).

use crate::content::embedded;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The table's path under data/ (the embedded file's name).
pub const TABLE_PATH: &str = "render/prop_draw_scale.json";

pub struct PropDrawScale {
    scales: HashMap<String, f32>,
    default: f32,
}

impl PropDrawScale {
    /// Parses `{ "<prop type>": <scale>, ..., "default": 1.0 }`. Every value must be a positive
    /// number; a missing "default" is 1.0.
    pub fn parse(text: &str) -> Result<PropDrawScale, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let object = value
            .as_object()
            .ok_or_else(|| "expected a JSON object of prop type to scale".to_string())?;
        let mut table = PropDrawScale {
            scales: HashMap::new(),
            default: 1.0,
        };
        for (key, v) in object {
            let scale = v
                .as_f64()
                .filter(|s| *s > 0.0)
                .ok_or_else(|| format!("{key}: the scale must be a positive number"))?
                as f32;
            if key == "default" {
                table.default = scale;
            } else {
                table.scales.insert(key.clone(), scale);
            }
        }
        Ok(table)
    }

    /// The draw scale of a prop type (PropModelCatalog.hunter_scale): its entry, else the default.
    pub fn for_type(&self, ptype: &str) -> f32 {
        self.scales.get(ptype).copied().unwrap_or(self.default)
    }

    /// How many prop types have their own entry (the default is not counted).
    pub fn len(&self) -> usize {
        self.scales.len()
    }
}

/// The table compiled into the executable. A table that fails to parse is reported on stderr and
/// every prop draws at the default 1.0; the unit test below keeps the embedded file valid.
pub fn table() -> &'static PropDrawScale {
    static TABLE: OnceLock<PropDrawScale> = OnceLock::new();
    TABLE.get_or_init(|| {
        let text = embedded::data_under(TABLE_PATH)
            .next()
            .map(|(_, text)| text)
            .unwrap_or("{}");
        PropDrawScale::parse(text).unwrap_or_else(|e| {
            eprintln!("{TABLE_PATH}: {e}");
            PropDrawScale {
                scales: HashMap::new(),
                default: 1.0,
            }
        })
    })
}

/// The draw scale of a prop type, as Godot's PropModelCatalog.hunter_scale gives it.
pub fn draw_scale(ptype: &str) -> f32 {
    table().for_type(ptype)
}

/// The per-axis table's path under data/ (the embedded file's name).
pub const AXIS_PATH: &str = "render/prop_axis_scale.json";
const AXIS_SCHEMA: &str = "lr.prop_axis_scale/1";

/// Draw-only stretch per axis for the few prop types whose model does not fill its collider
/// (data/render/prop_axis_scale.json). It is baked into the model when the model is loaded, so the draw loop does
/// nothing for it; colliders and placement never read it.
#[derive(Clone, Debug, PartialEq)]
pub struct PropAxisScale {
    types: Vec<(String, [f32; 3])>,
}

impl PropAxisScale {
    /// Parses `{ "schema": "lr.prop_axis_scale/1", "types": { "<prop type>": [x, y, z], ... } }`. Every factor must
    /// be between 0.25 and 4.
    pub fn parse(text: &str) -> Result<PropAxisScale, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("{AXIS_PATH}: {e}"))?;
        if v["schema"].as_str() != Some(AXIS_SCHEMA) {
            return Err(format!("{AXIS_PATH}: 'schema' must be {AXIS_SCHEMA}"));
        }
        let mut types = Vec::new();
        for (ptype, row) in v["types"].as_object().ok_or_else(|| format!("{AXIS_PATH}: 'types' missing"))? {
            let axes: Vec<f32> = row
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect())
                .unwrap_or_default();
            if axes.len() != 3 || axes.iter().any(|a| !a.is_finite() || *a < 0.25 || *a > 4.0) {
                return Err(format!("{AXIS_PATH}: {ptype}: expected [x, y, z], each between 0.25 and 4"));
            }
            types.push((ptype.clone(), [axes[0], axes[1], axes[2]]));
        }
        Ok(PropAxisScale { types })
    }

    /// The stretch of a prop type, `[1, 1, 1]` for a type that is not listed.
    pub fn for_type(&self, ptype: &str) -> [f32; 3] {
        self.types.iter().find(|(t, _)| t == ptype).map_or([1.0; 3], |(_, s)| *s)
    }

    /// The stretch of each model file of the listed types: (asset path, factors), read off `props` (prop type, its
    /// model paths).
    fn by_model<'a>(&self, props: impl Iterator<Item = (&'a str, &'a [String])>) -> HashMap<String, [f32; 3]> {
        let mut out = HashMap::new();
        for (ptype, models) in props {
            let s = self.for_type(ptype);
            if s != [1.0; 3] {
                for m in models {
                    out.insert(m.clone(), s);
                }
            }
        }
        out
    }
}

/// The per-axis table compiled into the executable. A table that fails to parse is reported on stderr and nothing is
/// stretched.
pub fn axis_table() -> &'static PropAxisScale {
    static TABLE: OnceLock<PropAxisScale> = OnceLock::new();
    TABLE.get_or_init(|| {
        let text = embedded::data_under(AXIS_PATH).next().map(|(_, text)| text).unwrap_or("");
        PropAxisScale::parse(text).unwrap_or_else(|e| {
            eprintln!("{e}");
            PropAxisScale { types: Vec::new() }
        })
    })
}

/// The stretch a built-in model file is loaded with, by its asset path ("props/beach/beach_chair.bbmodel"): the
/// factors of the prop type that names it in data/props/*.json, `None` for every other model. Built once, on the
/// first model load.
pub fn model_axis_scale(model_path: &str) -> Option<[f32; 3]> {
    static BY_MODEL: OnceLock<HashMap<String, [f32; 3]>> = OnceLock::new();
    BY_MODEL
        .get_or_init(|| {
            let table = axis_table();
            if table.types.is_empty() {
                return HashMap::new();
            }
            let defs: Vec<crate::content::props::PropDef> = embedded::data_under("props/")
                .filter_map(|(_, text)| serde_json::from_str::<serde_json::Value>(text).ok())
                .flat_map(|v| crate::content::props::parse_file(&v))
                .collect();
            table.by_model(defs.iter().map(|d| (d.id.as_str(), d.models.as_slice())))
        })
        .get(model_path)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn the_shipped_axis_table_stretches_the_couches_and_two_chairs_only() {
        let text = embedded::data_under(AXIS_PATH).next().expect("prop_axis_scale.json is embedded").1;
        let t = PropAxisScale::parse(text).expect("prop_axis_scale.json parses");
        assert_eq!(&t, axis_table());
        for ptype in ["beachChair", "gothicHighbackChair", "loungeCouch", "loungeCouchWide"] {
            assert_ne!(t.for_type(ptype), [1.0; 3], "{ptype}");
        }
        assert_eq!(t.for_type("officeChair"), [1.0; 3]);
        assert_eq!(t.types.len(), 4);
        // the uniform table the generator reads for seat heights is untouched by it
        assert!(close(draw_scale("loungeCouch"), 1.5) && close(draw_scale("beachChair"), 1.5));
        // every model file of a listed type is stretched, and no other model
        assert_eq!(model_axis_scale("props/beach/beach_chair.bbmodel"), Some(t.for_type("beachChair")));
        assert_eq!(model_axis_scale("props/maze/lounge_couch_red.bbmodel"), Some(t.for_type("loungeCouch")));
        assert_eq!(model_axis_scale("props/mansion/lounge_couch_navy_wide.bbmodel"), Some(t.for_type("loungeCouchWide")));
        assert_eq!(model_axis_scale("props/maze/arcade_cabinet_default.bbmodel"), None);
    }

    #[test]
    fn a_changed_axis_table_changes_the_stretch_and_bad_rows_are_refused() {
        let t = PropAxisScale::parse(r#"{"schema":"lr.prop_axis_scale/1","types":{"sofa":[2.0,1.0,0.5]}}"#).unwrap();
        assert_eq!(t.for_type("sofa"), [2.0, 1.0, 0.5]);
        let models = vec!["a.bbmodel".to_string(), "b.bbmodel".to_string()];
        let other = vec!["c.bbmodel".to_string()];
        let by = t.by_model(vec![("sofa", models.as_slice()), ("lamp", other.as_slice())].into_iter());
        assert_eq!(by.len(), 2);
        assert_eq!(by["b.bbmodel"], [2.0, 1.0, 0.5]);
        assert!(PropAxisScale::parse("{}").is_err());
        assert!(PropAxisScale::parse(r#"{"schema":"lr.prop_axis_scale/1"}"#).is_err());
        assert!(PropAxisScale::parse(r#"{"schema":"lr.prop_axis_scale/1","types":{"sofa":[1.0,1.0]}}"#).is_err());
        assert!(PropAxisScale::parse(r#"{"schema":"lr.prop_axis_scale/1","types":{"sofa":[1.0,0.0,1.0]}}"#).is_err());
        assert!(PropAxisScale::parse(r#"{"schema":"lr.prop_axis_scale/1","types":{"sofa":[1.0,9.0,1.0]}}"#).is_err());
    }

    #[test]
    fn embedded_table_matches_godot_hunter_scale() {
        let t = table();
        // Godot draws officePrinter at 1.0: it has no HUNTER_SCALE entry, so hunter_scale() gives the default.
        assert!(close(t.for_type("officePrinter"), 1.0));
        // Godot's HUNTER_SCALE has fumeHood at 1.2 and woodenChair at 1.5.
        assert!(close(t.for_type("fumeHood"), 1.2));
        assert!(close(t.for_type("woodenChair"), 1.5));
        // The whole Godot table is ported: 183 entries.
        assert_eq!(t.len(), 183);
    }

    #[test]
    fn unlisted_types_take_the_default() {
        let t = PropDrawScale::parse(r#"{"default": 1.0, "chairA": 1.5}"#).unwrap();
        assert!(close(t.for_type("chairA"), 1.5));
        assert!(close(t.for_type("anything-else"), 1.0));
        let custom = PropDrawScale::parse(r#"{"default": 2.0}"#).unwrap();
        assert!(close(custom.for_type("anything-else"), 2.0));
    }

    #[test]
    fn rejects_bad_tables() {
        assert!(PropDrawScale::parse("[1.5]").is_err());
        assert!(PropDrawScale::parse(r#"{"chairA": 0}"#).is_err());
        assert!(PropDrawScale::parse(r#"{"chairA": "big"}"#).is_err());
        assert!(PropDrawScale::parse("not json").is_err());
    }
}
