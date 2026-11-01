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
    pub category: String,
    /// The pack that added or last overrode this hunter ("" for the base game).
    pub pack: String,
    /// Model and atlas paths inside the pack (or assets/hunters/ for the base game).
    pub model: Option<String>,
    pub atlas: Option<String>,
    pub portrait: Option<String>,
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
