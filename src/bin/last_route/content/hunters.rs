//! Hunter definitions (`lr.hunter/1`, data/hunters/<id>.json) and the roster pools from
//! game/config/hunter_roster.gd.

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct HunterDef {
    pub id: String,
    pub name: String,
    /// 0 easy, 1 intermediate, 2 difficult
    pub tier: usize,
    pub speed: f32,
    pub chase: f32,
    pub prog: f32,
    pub sight: f32,
    pub hearing: f32,
    pub spawn: f32,
    pub chase_style: String,
    pub taser_immune: bool,
    pub melee_immune: bool,
    /// Her own taser slowdown (game/config/hunter_roster.gd taser_slow_factor / taser_slow_seconds, lines 243-248).
    /// None means the difficulty profile's value applies.
    pub taser_slow_factor: Option<f32>,
    pub taser_slow_seconds: Option<f32>,
    pub category: String,
    /// The source: the pack that added or last overrode this hunter, or "" for the embedded
    /// base game.
    pub pack: String,
    /// Model and atlas paths inside the pack (or assets/hunters/ for the base game).
    pub model: Option<String>,
    pub atlas: Option<String>,
    pub portrait: Option<String>,
}

impl HunterDef {
    /// True when a mod pack added or replaced this hunter (the dossier's Custom Hunters tab).
    /// A pack that replaces a built-in id counts as a pack hunter.
    pub fn is_pack(&self) -> bool {
        !self.pack.is_empty()
    }
}

/// IMPLEMENTED_IDS: the hunters the ordinary per-floor draw picks from.
pub const IMPLEMENTED_IDS: [&str; 24] = [
    "dovelle", "quenelle", "elise", "xivvy", "kathy", "ravelle", "mavrine", "umbriel", "wynelle",
    "thessine", "haelith", "ysolde", "opalie", "aki", "porcelle", "javelle", "loxelle", "ceryth",
    "briselle", "ferryn", "glyra", "ilyra", "verrine", "zellia",
];

/// SPECIAL_IDS: always present but dormant, woken by their own rule.
pub const SPECIAL_IDS: [&str; 2] = ["nyxie", "solyth"];

pub fn tier_index(tier: &str) -> usize {
    match tier {
        "intermediate" => 1,
        "difficult" => 2,
        _ => 0,
    }
}

fn f(v: &Value, k: &str, d: f32) -> f32 {
    v.get(k)
        .and_then(|x| x.as_f64())
        .map(|x| x as f32)
        .unwrap_or(d)
}

fn s(v: &Value, k: &str, d: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or(d).to_string()
}

/// An optional float field: absent keeps the base value, present must be a number (validate_data_hunters.gd
/// rejects a non-float taser_slow_factor / taser_slow_seconds).
fn opt_f(v: &Value, k: &str, base: Option<f32>) -> Result<Option<f32>, String> {
    match v.get(k) {
        None => Ok(base),
        Some(x) => x
            .as_f64()
            .map(|n| Some(n as f32))
            .ok_or_else(|| format!("{k} must be a number")),
    }
}

/// Parses one `lr.hunter/1` document. `base` supplies defaults when a pack overrides an
/// existing hunter with a partial document; fields left out keep the base value.
pub fn parse(v: &Value, base: Option<&HunterDef>, pack: &str) -> Result<HunterDef, String> {
    if let Some(schema) = v.get("schema").and_then(|x| x.as_str()) {
        if schema != "lr.hunter/1" {
            return Err(format!("unknown schema {schema}"));
        }
    }
    let id = s(v, "id", "");
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!("bad hunter id {id:?}"));
    }
    let d = base.cloned();
    let taser_slow_factor = opt_f(v, "taser_slow_factor", d.as_ref().and_then(|x| x.taser_slow_factor))?;
    let taser_slow_seconds = opt_f(v, "taser_slow_seconds", d.as_ref().and_then(|x| x.taser_slow_seconds))?;
    let b =
        |k: &str, dv: f32, pick: fn(&HunterDef) -> f32| f(v, k, d.as_ref().map(pick).unwrap_or(dv));
    let def = HunterDef {
        name: s(
            v,
            "name",
            d.as_ref().map(|x| x.name.as_str()).unwrap_or(&id),
        ),
        tier: v
            .get("tier")
            .and_then(|x| x.as_str())
            .map(tier_index)
            .unwrap_or(d.as_ref().map(|x| x.tier).unwrap_or(0)),
        speed: b("speed", 0.8, |x| x.speed),
        chase: b("chase", 3.55, |x| x.chase),
        prog: b("prog", 0.18, |x| x.prog),
        sight: b("sight", 8.0, |x| x.sight),
        hearing: b("hearing", 1.0, |x| x.hearing),
        spawn: b("spawn", 12.0, |x| x.spawn),
        chase_style: s(
            v,
            "chase_style",
            d.as_ref().map(|x| x.chase_style.as_str()).unwrap_or("pump"),
        ),
        taser_immune: v
            .get("taser_immune")
            .and_then(|x| x.as_bool())
            .unwrap_or(d.as_ref().map(|x| x.taser_immune).unwrap_or(true)),
        melee_immune: v
            .get("melee_immune")
            .and_then(|x| x.as_bool())
            .unwrap_or(d.as_ref().map(|x| x.melee_immune).unwrap_or(false)),
        taser_slow_factor,
        taser_slow_seconds,
        category: s(
            v,
            "category",
            d.as_ref().map(|x| x.category.as_str()).unwrap_or("az"),
        ),
        pack: pack.to_string(),
        model: v
            .get("model")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .or(d.as_ref().and_then(|x| x.model.clone())),
        atlas: v
            .get("atlas")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .or(d.as_ref().and_then(|x| x.atlas.clone())),
        portrait: v
            .get("portrait")
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .or(d.as_ref().and_then(|x| x.portrait.clone())),
        id,
    };
    for (k, val) in [
        ("speed", def.speed),
        ("chase", def.chase),
        ("sight", def.sight),
    ] {
        if !(val.is_finite() && val > 0.0 && val < 50.0) {
            return Err(format!("{}: {k} out of range", def.id));
        }
    }
    Ok(def)
}

/// The Void incursion knobs of data/hunters/draw_pool.json `incursion` (hunter_roster.gd has_void_incursion).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Incursion {
    pub min_depth: u32,
    pub base_chance: f32,
    pub per_depth: f32,
    pub max_chance: f32,
}

impl Incursion {
    /// Parse the draw pool text; a missing or bad key keeps the Godot value (2, 0.08, 0.025, 0.40).
    pub fn parse(text: &str) -> Incursion {
        let v: Value = serde_json::from_str(text).unwrap_or_default();
        let o = v.get("incursion").cloned().unwrap_or_default();
        let num = |k: &str, d: f64| o.get(k).and_then(|x| x.as_f64()).filter(|n| *n >= 0.0 && n.is_finite()).unwrap_or(d);
        Incursion {
            min_depth: num("min_depth", 2.0) as u32,
            base_chance: num("base_chance", 0.08).min(1.0) as f32,
            per_depth: num("per_depth", 0.025) as f32,
            max_chance: num("max_chance", 0.40).min(1.0) as f32,
        }
    }

    /// The chance a floor at `depth` brings a Void: 0 before min_depth, then rising per floor up to the cap.
    pub fn chance(&self, depth: u32) -> f32 {
        if depth < self.min_depth {
            return 0.0;
        }
        (self.base_chance + (depth - self.min_depth) as f32 * self.per_depth).min(self.max_chance)
    }

    /// True when a roll in [0, 1) from the floor's seeded stream brings a Void.
    pub fn brings_void(&self, depth: u32, roll: f32) -> bool {
        roll < self.chance(depth)
    }
}

/// The shipped incursion knobs, parsed once.
pub fn incursion() -> Incursion {
    static I: std::sync::OnceLock<Incursion> = std::sync::OnceLock::new();
    *I.get_or_init(|| Incursion::parse(include_str!("../data/hunters/draw_pool.json")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(id: &str, name: &str) -> Value {
        json!({ "schema": "lr.hunter/1", "id": id, "name": name, "speed": 0.9, "chase": 3.0, "sight": 7.0 })
    }

    #[test]
    fn base_game_hunter_is_not_a_pack_hunter() {
        let h = parse(&doc("aki", "Aki"), None, "").unwrap();
        assert_eq!(h.pack, "");
        assert!(!h.is_pack());
    }

    #[test]
    fn new_pack_hunter_is_tagged_with_its_pack() {
        let h = parse(&doc("mod_new", "New"), None, "dossier_test").unwrap();
        assert_eq!(h.pack, "dossier_test");
        assert!(h.is_pack());
    }

    #[test]
    fn pack_override_of_a_built_in_id_counts_as_a_pack_hunter() {
        let base = parse(&doc("aki", "Aki"), None, "").unwrap();
        // a partial override: the name and speed come from the pack, the rest from the base
        let partial = json!({ "schema": "lr.hunter/1", "id": "aki", "speed": 1.1 });
        let h = parse(&partial, Some(&base), "dossier_test").unwrap();
        assert!(h.is_pack());
        assert_eq!(h.pack, "dossier_test");
        assert_eq!(h.id, "aki");
        assert_eq!(
            h.name, "Aki",
            "fields the override leaves out keep the base value"
        );
        assert!((h.speed - 1.1_f32).abs() < 1e-4);
    }

    /// The void runners wear their own bbmodel (texture embedded in the model), not a path the loader would skip.
    #[test]
    fn every_void_hunter_has_an_embedded_bbmodel() {
        let content = crate::content::Content::load();
        let mut n = 0;
        for h in content.hunters.iter().filter(|h| h.category == "void") {
            n += 1;
            let path = h.model.as_deref().unwrap_or_default();
            assert!(path.ends_with(".bbmodel"), "{}: model {path:?}", h.id);
            let text = crate::content::embedded::asset(path).unwrap_or_else(|| panic!("{path} is not embedded"));
            let j: serde_json::Value = serde_json::from_slice(text).unwrap();
            let none = |_: &str| None;
            assert!(crate::render::bbmodel::texture_bytes(&j, &none).is_some(), "{}: no texture", h.id);
        }
        assert_eq!(n, 12);
    }

    /// Regression (2026-11-04): Nyxie and Solyth had no model files, so the renderer drew them with Dovelle's body
    /// and skin. Every built-in hunter must ship its own model and a texture (an atlas file or one embedded in the model).
    #[test]
    fn every_built_in_hunter_ships_its_own_model_and_texture() {
        let content = crate::content::Content::load();
        let mut checked = 0;
        for h in content.hunters.iter().filter(|h| !h.is_pack()) {
            checked += 1;
            let model = h.model.as_deref().unwrap_or_default();
            let bytes = crate::content::embedded::asset(model).unwrap_or_else(|| panic!("{}: model {model:?} is not embedded", h.id));
            let j: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_else(|e| panic!("{}: {model} is not JSON: {e}", h.id));
            let atlas = h.atlas.as_deref().and_then(crate::content::embedded::asset);
            let find = |name: &str| -> Option<Vec<u8>> { crate::content::embedded::asset(&format!("hunters/{name}")).map(|b| b.to_vec()) };
            assert!(atlas.is_some() || crate::render::bbmodel::texture_bytes(&j, &find).is_some(), "{}: no atlas file and no embedded texture", h.id);
        }
        assert!(checked >= 36, "only {checked} built-in hunters checked");
    }

    /// The canonical display names of the 26 A to Z hunters (KT, 2026-11-06: the single-letter names were wrong; the
    /// full names are the canonical ones, from the Godot hunter definitions). The id is the key and never changes.
    const AZ_NAMES: [(&str, &str); 26] = [
        ("aki", "Aki"), ("briselle", "Briselle"), ("ceryth", "Ceryth"), ("dovelle", "Dovelle"), ("elise", "Elise"),
        ("ferryn", "Ferryn"), ("glyra", "Glyra"), ("haelith", "Haelith"), ("ilyra", "Ilyra"), ("javelle", "Javelle"),
        ("kathy", "Kathy"), ("loxelle", "Loxelle"), ("mavrine", "Mavrine"), ("nyxie", "Nyxie"), ("opalie", "Opalie"),
        ("porcelle", "Porcelle"), ("quenelle", "Quenelle"), ("ravelle", "Ravelle"), ("solyth", "Solyth"),
        ("thessine", "Thessine"), ("umbriel", "Umbriel"), ("verrine", "Verrine"), ("wynelle", "Wynelle"),
        ("xivvy", "Xivvy"), ("ysolde", "Ysolde"), ("zellia", "Zellia"),
    ];

    /// Every built-in A to Z hunter shows its full canonical name (at least 3 letters), one hunter per letter.
    #[test]
    fn the_a_to_z_hunters_carry_their_full_canonical_names() {
        let content = crate::content::Content::load();
        let mut seen = std::collections::BTreeSet::new();
        for h in content.hunters.iter().filter(|h| !h.is_pack() && h.category == "az") {
            assert!(h.name.chars().count() >= 3, "{}: name {:?} is shorter than 3 letters", h.id, h.name);
            let want = AZ_NAMES.iter().find(|(id, _)| *id == h.id).unwrap_or_else(|| panic!("{}: not in the canonical list", h.id)).1;
            assert_eq!(h.name, want, "{}", h.id);
            assert!(h.name.to_ascii_lowercase().starts_with(&h.id[..1]), "{}: the name starts with the id's letter", h.id);
            seen.insert(h.id.chars().next().unwrap());
        }
        assert_eq!(seen.len(), 26, "one hunter per letter: {seen:?}");
    }

    #[test]
    fn a_bad_id_is_rejected_whatever_the_source() {
        let bad = json!({ "schema": "lr.hunter/1", "id": "Bad Id" });
        assert!(parse(&bad, None, "dossier_test").is_err());
        assert!(parse(&bad, None, "").is_err());
    }

    /// Void incursions: none on floor 1, 8 percent at depth 2 rising 2.5 points a floor to a 40 percent cap, and the
    /// roll against that chance decides (a seeded stream supplies it).
    #[test]
    fn the_incursion_chance_follows_the_shipped_data() {
        let i = incursion();
        assert_eq!(i.chance(1), 0.0);
        assert!((i.chance(2) - 0.08).abs() < 1e-6);
        assert!(i.chance(6) > i.chance(3));
        assert_eq!(i.chance(500), 0.40);
        assert!(i.brings_void(2, 0.07) && !i.brings_void(2, 0.09));
        assert!(!i.brings_void(1, 0.0), "no void on the first floor");
    }

    /// Over many rolls the draw rate matches the chance, and a data edit moves it.
    #[test]
    fn the_incursion_draw_rate_matches_the_chance_and_follows_edits() {
        let i = incursion();
        let n = 10_000;
        let hits = (0..n).filter(|k| i.brings_void(10, *k as f32 / n as f32)).count() as f32 / n as f32;
        assert!((hits - i.chance(10)).abs() < 0.01, "rate {hits} vs chance {}", i.chance(10));
        let off = Incursion::parse(r#"{"incursion": {"max_chance": 0, "base_chance": 0}}"#);
        assert_eq!(off.chance(50), 0.0);
        assert_eq!(Incursion::parse("nope"), Incursion::parse("{}"));
    }
}
