//! Draw scale per prop type: Godot's PropModelCatalog.HUNTER_SCALE (the person-sized models are
//! drawn at 1.5x, a few at 1.2 to 1.48, and any type not listed at 1.0, as hunter_scale() does).
//! The table is data/render/prop_draw_scale.json, compiled in by build.rs. Only drawing uses it;
//! placement, collision and footprints do not.

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

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
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
