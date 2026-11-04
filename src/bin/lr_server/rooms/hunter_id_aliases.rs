//! Port of `backend/hunter_id_aliases.mjs`.
//!
//! Four hunters were renamed to their lore names and their internal ids followed. Ids that outlive a build (room
//! settings from an older client, stored account data) can still be the old ones: normalise them on the way in.
//! Mirrors game/config/hunter_id_aliases.gd, the only other place the old ids live (docs/HUNTER_ID_ALIASES.md).

use crate::util::js_string_of;
use serde_json::Value;

/// Old id -> lore-name id, in the JS object's key order.
pub const HUNTER_ID_ALIASES: [(&str, &str); 4] = [
    ("eirys", "elise"),
    ("jyvra", "javelle"),
    ("quirith", "quenelle"),
    ("zephyra", "zellia"),
];

fn alias_of(id: &str) -> Option<&'static str> {
    HUNTER_ID_ALIASES.iter().find(|(old, _)| *old == id).map(|(_, new)| *new)
}

/// `canonicalHunterId(id)` for a string id: the lore-name id for an old alias, otherwise the id unchanged.
#[cfg(test)]
pub fn canonical_hunter_id(id: &str) -> String {
    alias_of(id).map(str::to_string).unwrap_or_else(|| id.to_string())
}

/// `canonicalHunterId(id)` for an arbitrary JSON value (`None` = undefined). Like `hasOwnProperty`, the value is
/// coerced to a property key, so only a value whose string form is an alias is replaced; anything else is returned
/// unchanged.
pub fn canonical_hunter_id_value(id: Option<&Value>) -> Option<Value> {
    let value = id?;
    match alias_of(&js_string_of(value)) {
        Some(new) => Some(Value::String(new.to_string())),
        None => Some(value.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_hunter_ids_map_to_the_lore_name_ids_everything_else_is_unchanged() {
        assert_eq!(
            HUNTER_ID_ALIASES,
            [("eirys", "elise"), ("jyvra", "javelle"), ("quirith", "quenelle"), ("zephyra", "zellia")]
        );
        for (old_id, new_id) in HUNTER_ID_ALIASES {
            assert_eq!(canonical_hunter_id(old_id), new_id);
            assert_eq!(canonical_hunter_id(new_id), new_id);
        }
        assert_eq!(canonical_hunter_id("kathy"), "kathy");
        assert_eq!(canonical_hunter_id("constructor"), "constructor");
        assert_eq!(canonical_hunter_id_value(None), None);
        assert_eq!(canonical_hunter_id_value(Some(&json!("eirys"))), Some(json!("elise")));
        assert_eq!(canonical_hunter_id_value(Some(&json!(5))), Some(json!(5)));
    }
}
