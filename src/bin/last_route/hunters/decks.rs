//! Hunters on catwalk decks: the per-hunter rules in `data/hunters/decks.json` (schema `lr.hunters.decks/1`).
//!
//! A deck is a stretch of raised walk surface over level 0 (`Facility::height_at`). A hunter reaches one by the catwalk
//! ladder and may leave it by the same ladder or, chasing a runner on the floor below, by jumping down (the edges are
//! `flight_edges::EdgeKind::CatwalkLadder` and `CatwalkJump`). This file says which hunters may do which. A hunter whose
//! gimmick moves it on the ground only (teleports, lunges) stays off decks until the gimmick is deck-aware.
//!
//! Same shape as `water.json`: a `default` row and a `hunters` map of per-id rows that override single keys. Unknown keys
//! and out-of-range values refuse the file.

use std::collections::HashMap;
use std::sync::OnceLock;

pub const SCHEMA: &str = "lr.hunters.decks/1";
/// The embedded path; content/mod.rs skips it when it reads hunter definitions.
pub const DECKS_PATH: &str = "hunters/decks.json";

/// What a hunter takes when it lands from a jump. Only `None` exists today: a landing never hurts a hunter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallDamage {
    None,
}

/// One hunter's deck rules, resolved (no nulls left).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HunterDeck {
    /// May walk to a catwalk ladder and climb it (up to a deck, and down again).
    pub can_climb: bool,
    /// May jump off a deck to chase a target on the floor below (Chase and Investigate only).
    pub can_jump_down: bool,
    /// Seconds a hunter is stunned on landing (0 = none).
    pub landing_stun_seconds: f32,
    pub fall_damage: FallDamage,
}

impl Default for HunterDeck {
    /// The embedded default row.
    fn default() -> Self {
        DeckRules::get().default
    }
}

#[derive(Clone, Debug)]
pub struct DeckRules {
    pub default: HunterDeck,
    pub hunters: HashMap<String, HunterDeck>,
}

const KEYS: [&str; 4] = ["can_climb", "can_jump_down", "landing_stun_seconds", "fall_damage"];

type Row = serde_json::Map<String, serde_json::Value>;

fn merged(base: &Row, over: Option<&serde_json::Value>, who: &str) -> Result<Row, String> {
    let mut m = base.clone();
    if let Some(o) = over {
        let o = o.as_object().ok_or_else(|| format!("hunter decks: \"{who}\" must be an object"))?;
        for (k, v) in o {
            m.insert(k.clone(), v.clone());
        }
    }
    for k in m.keys() {
        if !KEYS.contains(&k.as_str()) {
            return Err(format!("hunter decks: \"{who}\" has an unknown key \"{k}\""));
        }
    }
    Ok(m)
}

fn resolve(m: &Row, who: &str) -> Result<HunterDeck, String> {
    let flag = |k: &str| {
        m.get(k).and_then(|v| v.as_bool()).ok_or_else(|| format!("hunter decks: \"{who}\".{k} must be true or false"))
    };
    let stun = match m.get("landing_stun_seconds").and_then(|v| v.as_f64()) {
        Some(x) if x.is_finite() && (0.0..=10.0).contains(&x) => x as f32,
        _ => return Err(format!("hunter decks: \"{who}\".landing_stun_seconds must be a number from 0 to 10")),
    };
    let fall_damage = match m.get("fall_damage").and_then(|v| v.as_str()) {
        Some("none") => FallDamage::None,
        _ => return Err(format!("hunter decks: \"{who}\".fall_damage must be \"none\"")),
    };
    Ok(HunterDeck { can_climb: flag("can_climb")?, can_jump_down: flag("can_jump_down")?, landing_stun_seconds: stun, fall_damage })
}

impl DeckRules {
    pub fn parse(text: &str) -> Result<DeckRules, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("hunter decks: {e}"))?;
        if v["schema"].as_str() != Some(SCHEMA) {
            return Err(format!("hunter decks: schema must be \"{SCHEMA}\""));
        }
        let base = v["default"].as_object().ok_or("hunter decks: the \"default\" row is missing")?;
        let default_map = merged(base, None, "default")?;
        let default = resolve(&default_map, "default")?;
        let mut hunters = HashMap::new();
        if let Some(h) = v.get("hunters") {
            let h = h.as_object().ok_or("hunter decks: \"hunters\" must be an object")?;
            for (id, row) in h {
                let m = merged(&default_map, Some(row), id)?;
                hunters.insert(id.clone(), resolve(&m, id)?);
            }
        }
        Ok(DeckRules { default, hunters })
    }

    pub fn for_hunter(&self, id: &str) -> HunterDeck {
        self.hunters.get(id).copied().unwrap_or(self.default)
    }

    pub fn get() -> &'static DeckRules {
        static RULES: OnceLock<DeckRules> = OnceLock::new();
        RULES.get_or_init(|| {
            DeckRules::parse(include_str!("../data/hunters/decks.json")).unwrap_or_else(|e| panic!("data/hunters/decks.json: {e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(edit: impl FnOnce(&mut serde_json::Value)) -> Result<DeckRules, String> {
        let mut v: serde_json::Value = serde_json::from_str(include_str!("../data/hunters/decks.json")).unwrap();
        edit(&mut v);
        DeckRules::parse(&v.to_string())
    }

    #[test]
    fn the_embedded_table_loads_with_the_owner_defaults() {
        let r = DeckRules::get();
        assert!(r.default.can_climb && r.default.can_jump_down, "hunters climb and jump down by default");
        assert_eq!(r.default.landing_stun_seconds, 0.0);
        assert_eq!(r.default.fall_damage, FallDamage::None);
        assert_eq!(r.for_hunter("no-such-hunter"), r.default);
    }

    #[test]
    fn ground_only_gimmick_hunters_stay_off_decks() {
        let r = DeckRules::get();
        for id in ["nyxie", "solyth", "ysolde", "void_blue", "void_green"] {
            let d = r.for_hunter(id);
            assert!(!d.can_climb && !d.can_jump_down, "{id} has ground-only movement");
        }
        assert!(r.for_hunter("aki").can_climb);
    }

    #[test]
    fn every_hunter_row_names_a_real_hunter() {
        let content = crate::content::Content::load();
        for id in DeckRules::get().hunters.keys() {
            assert!(content.hunters.iter().any(|h| &h.id == id), "decks.json names an unknown hunter `{id}`");
        }
    }

    #[test]
    fn a_hunter_row_overrides_single_keys() {
        let t = table(|v| v["hunters"]["aki"] = serde_json::json!({"can_jump_down": false, "landing_stun_seconds": 0.5})).unwrap();
        let aki = t.for_hunter("aki");
        assert!(aki.can_climb && !aki.can_jump_down);
        assert_eq!(aki.landing_stun_seconds, 0.5);
        assert_eq!(t.for_hunter("elise"), t.default);
    }

    #[test]
    fn a_bad_table_is_refused_with_a_clear_message() {
        assert!(table(|v| v["schema"] = serde_json::json!("x")).unwrap_err().contains("schema"));
        assert!(table(|v| v["default"]["fly"] = serde_json::json!(true)).unwrap_err().contains("fly"));
        assert!(table(|v| v["hunters"]["aki"] = serde_json::json!({"fly": true})).unwrap_err().contains("fly"));
        assert!(table(|v| v["default"]["landing_stun_seconds"] = serde_json::json!(-1.0)).unwrap_err().contains("landing_stun_seconds"));
        assert!(table(|v| v["default"]["landing_stun_seconds"] = serde_json::json!(99.0)).unwrap_err().contains("landing_stun_seconds"));
        assert!(table(|v| v["default"]["can_climb"] = serde_json::json!("yes")).unwrap_err().contains("can_climb"));
        assert!(table(|v| v["default"]["fall_damage"] = serde_json::json!("lots")).unwrap_err().contains("fall_damage"));
        assert!(table(|v| {
            v.as_object_mut().unwrap().remove("default");
        })
        .unwrap_err()
        .contains("default"));
        assert!(DeckRules::parse("not json").is_err());
    }
}
