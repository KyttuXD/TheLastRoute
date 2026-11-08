//! The Tab inventory grid and the pack's capacity rules (data/items.json). Capacity is additive: the base slots plus the
//! extra slots of every backpack carried. The grid draw reads [`grid_size`]; the hotbar stays the first row.

use serde_json::Value;
use std::sync::OnceLock;

pub const ITEMS_PATH: &str = "items.json";

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub base_slots: usize,
    pub backpacks: Vec<(String, usize)>,
    pub lockpick_seconds: f32,
    pub kit_self_revive: bool,
}

impl Rules {
    pub fn parse(text: &str) -> Result<Rules, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let base_slots = v.get("base_slots").and_then(Value::as_u64).ok_or("base_slots must be a number")? as usize;
        let backpacks = v
            .get("backpacks")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|b| Some((b.get("id")?.as_str()?.to_string(), b.get("extra_slots")?.as_u64()? as usize)))
                    .collect()
            })
            .unwrap_or_default();
        let lockpick_seconds = v
            .get("lockpick_seconds")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && *x >= 0.0)
            .ok_or("lockpick_seconds must be 0 or more")? as f32;
        Ok(Rules {
            base_slots,
            backpacks,
            lockpick_seconds,
            kit_self_revive: v.get("kit_self_revive").and_then(Value::as_bool).unwrap_or(true),
        })
    }

    /// Slots in the pack with the named backpacks worn (additive; an unknown id adds nothing).
    pub fn capacity(&self, worn: &[&str]) -> usize {
        self.base_slots
            + worn
                .iter()
                .map(|w| self.backpacks.iter().find(|(id, _)| id == w).map(|(_, n)| *n).unwrap_or(0))
                .sum::<usize>()
    }
}

pub fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| {
        let text = crate::content::embedded::data_under("")
            .find(|(p, _)| *p == ITEMS_PATH)
            .map(|(_, t)| t)
            .expect("data/items.json is embedded by build.rs");
        Rules::parse(text).expect("data/items.json is valid")
    })
}

/// (columns, rows) of the Tab grid for `slots` cells: the hotbar's row width, as many rows as it takes.
pub fn grid_size(slots: usize, columns: usize) -> (usize, usize) {
    let c = columns.max(1);
    (c, slots.div_ceil(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_is_additive_over_backpacks() {
        let r = rules();
        assert_eq!(r.capacity(&[]), r.base_slots);
        let one = r.capacity(&["small"]);
        let two = r.capacity(&["small", "large"]);
        assert!(one > r.base_slots && two > one);
        assert_eq!(r.capacity(&["nope"]), r.base_slots);
        assert_eq!(r.base_slots, crate::config::HOTBAR_SIZE);
    }

    #[test]
    fn data_changes_capacity_and_the_grid_covers_every_slot() {
        let r = Rules::parse(r#"{"base_slots":3,"backpacks":[{"id":"a","extra_slots":10}],"lockpick_seconds":1}"#).unwrap();
        assert_eq!(r.capacity(&["a", "a"]), 23);
        let (c, rows) = grid_size(23, 5);
        assert!(c * rows >= 23 && c * (rows - 1) < 23);
        assert!(Rules::parse("{}").is_err());
    }
}
