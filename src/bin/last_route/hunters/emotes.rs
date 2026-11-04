//! Hunter emotes and personalities (data/hunters/emotes.json): which short personality moment a hunter plays, when, and
//! with which clip. Port of game/config/hunter_emote_catalog.gd, hunter_emote_personality.gd and hunters/ai/hunter_emoter.gd.
//! Presentation only: the brain keeps simulating, and the caller plays the clip (or face) it is handed back.
//!
//! The emote's index in the catalogue is its wire value (the hunter state `emote`), so the file only ever appends.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

use crate::content::embedded;

/// The embedded catalogue file (relative to the bin data folder).
pub const PATH: &str = "hunters/emotes.json";
pub const SCHEMA: &str = "lr.hunter_emotes/1";

/// One catalogue entry (`HunterEmoteCatalog.ENTRIES`).
#[derive(Clone, Debug, PartialEq)]
pub struct Emote {
    pub id: String,
    pub label: String,
    /// The body clip the rigs should carry.
    pub clip: String,
    /// Shared clips tried when the model lacks `clip`.
    pub fallbacks: Vec<String>,
    /// The BlockyFace expression played with it (face-only when there is no clip).
    pub face: String,
    pub seconds: f32,
    /// Audio radius in metres (0 = silent).
    pub noise: f32,
    /// Whether the clip may play while the hunter walks or runs (otherwise face-only while it moves).
    pub moving: bool,
}

/// When an emote starts (`HunterEmoter`'s groups and chances), as data in `triggers`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    /// Indexes into `Catalogue::emotes`, the candidates for this trigger.
    pub emotes: Vec<usize>,
    /// Probability per event, or per second when `per_second`.
    pub chance: f32,
    pub per_second: bool,
    /// Seconds before this rule may fire again (0 = no rule cooldown).
    pub cooldown: f32,
    /// Seconds the hunter must have been in the watched state (search, idle).
    pub min_seconds: f32,
    /// Metres the hunter must be from the runner (chase).
    pub min_distance: f32,
    /// Flinch-style rules start even inside the global cooldown and do not restart it.
    pub ignores_global_cooldown: bool,
}

/// A personality tag: the hunters that carry it, their emote weights and personal clips.
#[derive(Clone, Debug, PartialEq)]
pub struct Personality {
    pub tag: String,
    /// Hunter ids that carry this tag.
    pub hunters: Vec<String>,
    /// Hunter id prefix that carries this tag instead (Void hunters: "void_").
    pub hunter_prefix: Option<String>,
    /// emote id -> weight (a multiplier on the base chance; 0 = never, missing = 1).
    pub weights: HashMap<String, f32>,
    /// emote id -> personal clip, tried before the emote's own clip.
    pub clips: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Catalogue {
    /// Seconds between any two emotes of one hunter (flinch excepted).
    pub global_cooldown: f32,
    pub emotes: Vec<Emote>,
    /// Void hunters: emote id -> the shared Void clip that stands in for it.
    pub void_fallbacks: HashMap<String, String>,
    pub rules: Vec<Rule>,
    pub personalities: Vec<Personality>,
    /// Difficulty id -> multiplier on every chance (missing = 1).
    pub probability_scale: HashMap<String, f32>,
}

/// What the brain tells a trigger check each tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ctx {
    /// Seconds since the last tick (per-second rules scale their chance by it).
    pub dt: f32,
    /// Seconds the hunter has been in the state a rule watches (search, idle).
    pub time_in_state: f32,
    /// Metres to the runner (chase rule).
    pub distance: f32,
    /// Another emote, a flight or a charge is playing, or a queued emote waits: nothing starts.
    pub busy: bool,
}

/// Per-hunter cooldowns, ticked by the caller.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timers {
    /// Seconds until any non-flinch emote may start.
    pub global: f32,
    rule_cooldowns: HashMap<String, f32>,
}

impl Timers {
    pub fn tick(&mut self, dt: f32) {
        self.global = (self.global - dt).max(0.0);
        for left in self.rule_cooldowns.values_mut() {
            *left = (*left - dt).max(0.0);
        }
    }

    /// Seconds left on a rule's own cooldown (0 when it may fire).
    pub fn rule_left(&self, rule: &str) -> f32 {
        self.rule_cooldowns.get(rule).copied().unwrap_or(0.0)
    }
}

impl Catalogue {
    /// The embedded data/hunters/emotes.json, read once.
    pub fn get() -> &'static Catalogue {
        static CATALOGUE: OnceLock<Catalogue> = OnceLock::new();
        CATALOGUE.get_or_init(|| {
            let (_, text) = embedded::data_under(PATH).next().expect("data/hunters/emotes.json is embedded");
            let v: Value = serde_json::from_str(text).expect("data/hunters/emotes.json is JSON");
            Catalogue::parse(&v).expect("data/hunters/emotes.json has the emote keys")
        })
    }

    /// Reads a parsed emotes.json. Err names the first key that is missing, has the wrong type or points at an
    /// unknown emote.
    pub fn parse(v: &Value) -> Result<Catalogue, String> {
        if v["schema"].as_str() != Some(SCHEMA) {
            return Err(format!("schema must be \"{SCHEMA}\""));
        }
        let global_cooldown = non_negative(num(v, "global_cooldown")?, "global_cooldown")?;

        let mut emotes: Vec<Emote> = Vec::new();
        for e in items(v, "emotes")? {
            let id = text(e, "id")?;
            if emotes.iter().any(|x| x.id == id) {
                return Err(format!("duplicate emote \"{id}\""));
            }
            emotes.push(Emote {
                label: text(e, "label")?,
                clip: text(e, "clip")?,
                fallbacks: strings(e, "fallbacks")?,
                face: text(e, "face")?,
                seconds: num(e, "seconds")?,
                noise: non_negative(num(e, "noise")?, "noise")?,
                moving: flag(e, "moving")?,
                id,
            });
        }
        let index = |id: &str| -> Result<usize, String> {
            emotes.iter().position(|e| e.id == id).ok_or_else(|| format!("unknown emote \"{id}\""))
        };

        let void_fallbacks = text_map(v, "void_fallbacks")?;
        for id in void_fallbacks.keys() {
            index(id.as_str())?;
        }

        let mut rules: Vec<Rule> = Vec::new();
        for r in items(v, "triggers")? {
            let name = text(r, "name")?;
            let ids = strings(r, "emotes")?;
            if ids.is_empty() {
                return Err(format!("trigger \"{name}\" has no emotes"));
            }
            let mut picks = Vec::with_capacity(ids.len());
            for id in &ids {
                picks.push(index(id.as_str())?);
            }
            rules.push(Rule {
                chance: non_negative(num(r, "chance")?, "chance")?,
                per_second: opt_flag(r, "per_second"),
                cooldown: opt_num(r, "cooldown")?,
                min_seconds: opt_num(r, "min_seconds")?,
                min_distance: opt_num(r, "min_distance")?,
                ignores_global_cooldown: opt_flag(r, "ignores_global_cooldown"),
                emotes: picks,
                name,
            });
        }

        let mut personalities: Vec<Personality> = Vec::new();
        for p in items(v, "personalities")? {
            let weights = num_map(p, "weights")?;
            for (id, w) in &weights {
                index(id.as_str())?;
                non_negative(*w, "weights")?;
            }
            personalities.push(Personality {
                tag: text(p, "tag")?,
                hunters: opt_strings(p, "hunters")?,
                hunter_prefix: p["hunter_prefix"].as_str().map(str::to_string),
                weights,
                clips: text_map(p, "clips")?,
            });
        }

        let probability_scale = num_map(v, "probability_scale")?;
        for s in probability_scale.values() {
            non_negative(*s, "probability_scale")?;
        }

        Ok(Catalogue { global_cooldown, emotes, void_fallbacks, rules, personalities, probability_scale })
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.emotes.iter().position(|e| e.id == id)
    }

    pub fn emote(&self, index: usize) -> Option<&Emote> {
        self.emotes.get(index)
    }

    pub fn rule(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// The personality a hunter carries: a listed id first, then a matching prefix.
    pub fn personality(&self, hunter_id: &str) -> Option<&Personality> {
        self.personalities
            .iter()
            .find(|p| p.hunters.iter().any(|h| h == hunter_id))
            .or_else(|| {
                self.personalities
                    .iter()
                    .find(|p| p.hunter_prefix.as_deref().is_some_and(|pre| hunter_id.starts_with(pre)))
            })
    }

    /// The personality tag of a hunter, None when it carries none (every emote weighs 1).
    pub fn tag(&self, hunter_id: &str) -> Option<&str> {
        self.personality(hunter_id).map(|p| p.tag.as_str())
    }

    /// A hunter's weight for an emote: 1 by default, the personality's value when it sets one.
    pub fn weight(&self, hunter_id: &str, index: usize) -> f32 {
        let Some(e) = self.emote(index) else {
            return 0.0;
        };
        self.personality(hunter_id).and_then(|p| p.weights.get(&e.id)).copied().unwrap_or(1.0)
    }

    /// The personal clip a hunter plays for an emote, if its personality sets one.
    pub fn clip_override(&self, hunter_id: &str, index: usize) -> Option<&str> {
        let e = self.emote(index)?;
        self.personality(hunter_id)?.clips.get(&e.id).map(String::as_str)
    }

    /// Whether the emote's clip may play while the hunter walks or runs (else it is face-only then).
    pub fn plays_while_moving(&self, index: usize) -> bool {
        self.emote(index).is_some_and(|e| e.moving)
    }

    /// The clip to play for this emote on a model: the personal clip, then the emote's own clip, then the Void stand-in
    /// (Void hunters) or the fallbacks. None means face-only. `has_clip` tells whether the model's player has a clip.
    pub fn resolve_clip(
        &self,
        hunter_id: &str,
        index: usize,
        personal: Option<&str>,
        has_clip: impl Fn(&str) -> bool,
    ) -> Option<String> {
        let e = self.emote(index)?;
        let mut candidates: Vec<&str> = Vec::new();
        if let Some(p) = personal.filter(|p| !p.is_empty()) {
            candidates.push(p);
        }
        candidates.push(e.clip.as_str());
        match self.void_fallbacks.get(&e.id) {
            Some(stand_in) if hunter_id.starts_with("void_") => candidates.push(stand_in.as_str()),
            _ => candidates.extend(e.fallbacks.iter().map(String::as_str)),
        }
        candidates.into_iter().find(|c| has_clip(*c)).map(str::to_string)
    }

    /// Weighted choice among a rule's emotes for this hunter. `roll` is in [0, 1). None when the rule is unknown or
    /// every candidate weighs 0.
    pub fn pick(&self, hunter_id: &str, rule_name: &str, roll: f32) -> Option<usize> {
        let rule = self.rule(rule_name)?;
        let weighted: Vec<(usize, f32)> =
            rule.emotes.iter().map(|&i| (i, self.weight(hunter_id, i).max(0.0))).collect();
        let total: f32 = weighted.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            return None;
        }
        let mut left = roll.clamp(0.0, 1.0) * total;
        let mut last = None;
        for (i, w) in weighted {
            if w <= 0.0 {
                continue;
            }
            last = Some(i);
            if left < w {
                return Some(i);
            }
            left -= w;
        }
        last
    }

    /// One trigger check. `gate` and `pick_roll` are rolls in [0, 1). The emote starts when `gate` is under the rule's
    /// chance (times dt for a per-second rule, times the difficulty's scale) and the cooldowns and conditions allow it.
    /// On a start the global and rule cooldowns are set and the chosen emote index is returned.
    pub fn try_trigger(
        &self,
        timers: &mut Timers,
        hunter_id: &str,
        rule_name: &str,
        difficulty: &str,
        ctx: Ctx,
        gate: f32,
        pick_roll: f32,
    ) -> Option<usize> {
        let rule = self.rule(rule_name)?;
        if ctx.busy || timers.rule_left(&rule.name) > 0.0 {
            return None;
        }
        if !rule.ignores_global_cooldown && timers.global > 0.0 {
            return None;
        }
        if ctx.time_in_state < rule.min_seconds || ctx.distance < rule.min_distance {
            return None;
        }
        let scale = self.probability_scale.get(difficulty).copied().unwrap_or(1.0);
        let per_tick = if rule.per_second { ctx.dt.max(0.0) } else { 1.0 };
        if gate >= rule.chance * per_tick * scale {
            return None;
        }
        let index = self.pick(hunter_id, rule_name, pick_roll)?;
        if !rule.ignores_global_cooldown {
            timers.global = self.global_cooldown;
        }
        if rule.cooldown > 0.0 {
            timers.rule_cooldowns.insert(rule.name.clone(), rule.cooldown);
        }
        Some(index)
    }
}

fn non_negative(x: f32, key: &str) -> Result<f32, String> {
    if x >= 0.0 && x.is_finite() {
        Ok(x)
    } else {
        Err(format!("\"{key}\" must be a number >= 0"))
    }
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value, String> {
    match v.get(key) {
        Some(x) if !x.is_null() => Ok(x),
        _ => Err(format!("missing \"{key}\"")),
    }
}

fn num(v: &Value, key: &str) -> Result<f32, String> {
    field(v, key)?.as_f64().map(|x| x as f32).ok_or_else(|| format!("\"{key}\" must be a number"))
}

fn opt_num(v: &Value, key: &str) -> Result<f32, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(0.0),
        Some(_) => num(v, key),
    }
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    field(v, key)?.as_str().map(str::to_string).ok_or_else(|| format!("\"{key}\" must be a string"))
}

fn flag(v: &Value, key: &str) -> Result<bool, String> {
    field(v, key)?.as_bool().ok_or_else(|| format!("\"{key}\" must be true or false"))
}

fn opt_flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn items<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, String> {
    field(v, key)?.as_array().ok_or_else(|| format!("\"{key}\" must be a list"))
}

fn strings(v: &Value, key: &str) -> Result<Vec<String>, String> {
    items(v, key)?
        .iter()
        .map(|x| x.as_str().map(str::to_string).ok_or_else(|| format!("\"{key}\" must hold strings")))
        .collect()
}

fn opt_strings(v: &Value, key: &str) -> Result<Vec<String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(_) => strings(v, key),
    }
}

fn num_map(v: &Value, key: &str) -> Result<HashMap<String, f32>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(HashMap::new()),
        Some(x) => x
            .as_object()
            .ok_or_else(|| format!("\"{key}\" must be an object"))?
            .iter()
            .map(|(k, n)| {
                n.as_f64().map(|n| (k.clone(), n as f32)).ok_or_else(|| format!("\"{key}.{k}\" must be a number"))
            })
            .collect(),
    }
}

fn text_map(v: &Value, key: &str) -> Result<HashMap<String, String>, String> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(HashMap::new()),
        Some(x) => x
            .as_object()
            .ok_or_else(|| format!("\"{key}\" must be an object"))?
            .iter()
            .map(|(k, s)| s.as_str().map(|s| (k.clone(), s.to_string())).ok_or_else(|| format!("\"{key}.{k}\" must be a string")))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const IDS: [&str; 14] = [
        "taunt", "laugh", "point", "beckon", "shrug", "look_around", "smug", "dance", "flinch", "stretch", "yawn", "hum",
        "wave", "victory",
    ];

    /// The embedded file with one edit applied, parsed again.
    fn edited(edit: impl FnOnce(&mut Value)) -> Result<Catalogue, String> {
        let (_, text) = embedded::data_under(PATH).next().expect("embedded");
        let mut v: Value = serde_json::from_str(text).expect("json");
        edit(&mut v);
        Catalogue::parse(&v)
    }

    fn ctx() -> Ctx {
        Ctx { dt: 0.0, time_in_state: 0.0, distance: 0.0, busy: false }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn catalogue_order_is_the_wire_order() {
        let c = Catalogue::get();
        assert_eq!(c.emotes.len(), IDS.len());
        for (i, id) in IDS.iter().enumerate() {
            assert_eq!(c.index_of(id), Some(i), "{id} keeps its wire index");
        }
        assert_eq!(c.emote(0).unwrap().clip, "emote_taunt");
        assert_eq!(c.emote(13).unwrap().label, "Victory");
    }

    #[test]
    fn standard_timings_match_godot() {
        let c = Catalogue::get();
        assert!(near(c.global_cooldown, 8.0));
        let spot = c.rule("spot").unwrap();
        assert!(near(spot.chance, 0.30) && !spot.per_second);
        assert!(near(c.rule("lose").unwrap().chance, 0.50));
        let search = c.rule("search").unwrap();
        assert!(near(search.chance, 0.35) && near(search.min_seconds, 1.2));
        let chase = c.rule("chase").unwrap();
        assert!(chase.per_second && near(chase.chance, 0.035));
        assert!(near(chase.cooldown, 14.0) && near(chase.min_distance, 3.5));
        let idle = c.rule("idle").unwrap();
        assert!(idle.per_second && near(idle.chance, 0.035) && near(idle.cooldown, 20.0) && near(idle.min_seconds, 2.0));
        assert!(near(c.rule("catch").unwrap().chance, 0.85));
        assert!(near(c.rule("wipe").unwrap().chance, 1.0));
        let stun = c.rule("stun").unwrap();
        assert!(stun.ignores_global_cooldown && near(stun.chance, 1.0));
        assert_eq!(c.probability_scale.get("standard").copied(), Some(1.0));
    }

    #[test]
    fn hunter_weights_match_godot_tables() {
        let c = Catalogue::get();
        let taunt = c.index_of("taunt").unwrap();
        let wave = c.index_of("wave").unwrap();
        let dance = c.index_of("dance").unwrap();
        assert!(near(c.weight("kathy", taunt), 0.0));
        assert!(near(c.weight("kathy", wave), 1.5));
        assert!(near(c.weight("glyra", dance), 2.0));
        assert!(near(c.weight("nobody", taunt), 1.0), "an unlisted hunter weighs 1");
        let laugh = c.index_of("laugh").unwrap();
        let smug = c.index_of("smug").unwrap();
        assert!(near(c.weight("void_red", laugh), 0.0));
        assert!(near(c.weight("void_red", smug), 0.5));
        assert!(near(c.weight("void_red", taunt), 1.0), "a Void hunter not listed in the table weighs 1");
        assert_eq!(c.tag("kathy"), Some("protective"));
        assert_eq!(c.tag("void_red"), Some("void"));
        assert_eq!(c.tag("nobody"), None);
    }

    #[test]
    fn personal_clip_and_void_stand_ins() {
        let c = Catalogue::get();
        let stretch = c.index_of("stretch").unwrap();
        assert_eq!(c.clip_override("aki", stretch), Some("emote_shy"));
        assert_eq!(c.clip_override("kathy", stretch), None);
        let taunt = c.index_of("taunt").unwrap();
        // Void: the own clip first, then the shared Void stand-in; never the fallbacks.
        let only_loom = |name: &str| name == "emote_loom";
        assert_eq!(c.resolve_clip("void_red", taunt, None, only_loom).as_deref(), Some("emote_loom"));
        assert_eq!(c.resolve_clip("void_red", taunt, None, |_| true).as_deref(), Some("emote_taunt"));
        assert_eq!(c.resolve_clip("kathy", taunt, None, only_loom), None, "non-Void hunters have no stand-in");
        // personal clip wins while present, then the own clip
        assert_eq!(c.resolve_clip("aki", stretch, Some("emote_shy"), |_| true).as_deref(), Some("emote_shy"));
        assert_eq!(c.resolve_clip("aki", stretch, Some("emote_shy"), |n| n == "emote_stretch").as_deref(), Some("emote_stretch"));
        // face-only when nothing matches
        assert_eq!(c.resolve_clip("kathy", stretch, None, |_| false), None);
        // fallbacks for a non-Void hunter
        let laugh = c.index_of("laugh").unwrap();
        assert_eq!(c.resolve_clip("kathy", laugh, None, |n| n == "emote_cheer").as_deref(), Some("emote_cheer"));
    }

    #[test]
    fn moving_gate_comes_from_data() {
        let c = Catalogue::get();
        assert!(c.plays_while_moving(c.index_of("taunt").unwrap()));
        assert!(!c.plays_while_moving(c.index_of("laugh").unwrap()));
        assert!(!c.plays_while_moving(99));
    }

    #[test]
    fn pick_is_weighted_and_skips_zero_weights() {
        let c = Catalogue::get();
        let laugh = c.index_of("laugh").unwrap();
        // kathy: taunt 0 in spot, so every roll lands on laugh
        for roll in [0.0, 0.5, 0.999] {
            assert_eq!(c.pick("kathy", "spot", roll), Some(laugh));
        }
        // catch for kathy: smug 0, dance 0, laugh 0.3, so laugh is the only candidate
        assert_eq!(c.pick("kathy", "catch", 0.0), Some(laugh));
        assert_eq!(c.pick("kathy", "catch", 0.99), Some(laugh));
        // catch for a hunter with no table: all three weigh 1, so the roll splits them
        let smug = c.index_of("smug").unwrap();
        let dance = c.index_of("dance").unwrap();
        assert_eq!(c.pick("nobody", "catch", 0.0), Some(smug));
        assert_eq!(c.pick("nobody", "catch", 0.5), Some(dance));
        assert_eq!(c.pick("nobody", "catch", 0.99), Some(laugh));
        assert_eq!(c.pick("kathy", "no_such_rule", 0.0), None);
    }

    #[test]
    fn edited_weight_changes_the_pick() {
        let before = Catalogue::get().clone();
        let c = edited(|v| v["personalities"][0]["weights"]["taunt"] = json!(5.0)).unwrap();
        let taunt = c.index_of("taunt").unwrap();
        assert!(near(c.weight("kathy", taunt), 5.0));
        assert_eq!(c.pick("kathy", "spot", 0.0), Some(taunt), "a higher weight wins the first roll");
        assert_eq!(before.pick("kathy", "spot", 0.0), Some(before.index_of("laugh").unwrap()));
    }

    #[test]
    fn try_trigger_respects_chance_and_cooldowns() {
        let c = Catalogue::get();
        let laugh = c.index_of("laugh").unwrap();
        let mut t = Timers::default();
        assert_eq!(c.try_trigger(&mut t, "kathy", "spot", "standard", ctx(), 0.31, 0.0), None, "over the 0.30 chance");
        assert_eq!(c.try_trigger(&mut t, "kathy", "spot", "standard", ctx(), 0.29, 0.0), Some(laugh));
        assert!(near(t.global, 8.0), "the global cooldown starts");
        assert_eq!(c.try_trigger(&mut t, "kathy", "spot", "standard", ctx(), 0.0, 0.0), None, "inside the global cooldown");
        t.tick(8.0);
        assert_eq!(c.try_trigger(&mut t, "kathy", "spot", "standard", ctx(), 0.0, 0.0), Some(laugh));
    }

    #[test]
    fn stun_flinch_ignores_the_global_cooldown() {
        let c = Catalogue::get();
        let flinch = c.index_of("flinch").unwrap();
        let mut t = Timers { global: 5.0, ..Timers::default() };
        assert_eq!(c.try_trigger(&mut t, "ferryn", "stun", "standard", ctx(), 0.99, 0.0), Some(flinch));
        assert!(near(t.global, 5.0), "a flinch does not restart the cooldown");
        assert_eq!(c.try_trigger(&mut t, "kathy", "spot", "standard", ctx(), 0.0, 0.0), None);
    }

    #[test]
    fn busy_and_conditions_block_a_start() {
        let c = Catalogue::get();
        let mut t = Timers::default();
        let busy = Ctx { busy: true, ..ctx() };
        assert_eq!(c.try_trigger(&mut t, "kathy", "spot", "standard", busy, 0.0, 0.0), None);
        let near_runner = Ctx { dt: 1.0, distance: 2.0, ..ctx() };
        assert_eq!(c.try_trigger(&mut t, "kathy", "chase", "standard", near_runner, 0.0, 0.0), None, "under 3.5 m");
        let far = Ctx { dt: 1.0, distance: 4.0, ..ctx() };
        assert_eq!(c.try_trigger(&mut t, "kathy", "chase", "standard", far, 0.01, 0.0), Some(c.index_of("point").unwrap()));
        assert!(near(t.rule_left("chase"), 14.0));
        let searching = Ctx { time_in_state: 1.0, ..ctx() };
        assert_eq!(c.try_trigger(&mut t, "kathy", "search", "standard", searching, 0.0, 0.0), None, "under 1.2 s");
    }

    #[test]
    fn per_second_rules_scale_with_dt() {
        let c = Catalogue::get();
        let mut t = Timers::default();
        let still = Ctx { dt: 0.0, time_in_state: 3.0, ..ctx() };
        assert_eq!(c.try_trigger(&mut t, "kathy", "idle", "standard", still, 0.0, 0.0), None, "no time passes, no chance");
        let ticking = Ctx { dt: 1.0, time_in_state: 3.0, ..ctx() };
        assert!(c.try_trigger(&mut t, "kathy", "idle", "standard", ticking, 0.01, 0.0).is_some());
        assert!(near(t.rule_left("idle"), 20.0));
    }

    #[test]
    fn edited_chance_and_difficulty_scale_change_behaviour() {
        let off = edited(|v| v["triggers"][0]["chance"] = json!(0.0)).unwrap();
        let mut t = Timers::default();
        assert_eq!(off.try_trigger(&mut t, "kathy", "spot", "standard", ctx(), 0.0, 0.0), None);
        let muted = edited(|v| v["probability_scale"]["hard"] = json!(0.0)).unwrap();
        assert_eq!(muted.try_trigger(&mut t, "kathy", "spot", "hard", ctx(), 0.0, 0.0), None);
        assert!(muted.try_trigger(&mut t, "kathy", "spot", "standard", ctx(), 0.0, 0.0).is_some());
    }

    #[test]
    fn timers_tick_down_to_zero() {
        let mut t = Timers { global: 1.0, ..Timers::default() };
        t.tick(3.0);
        assert!(near(t.global, 0.0));
        assert!(near(t.rule_left("spot"), 0.0));
    }

    #[test]
    fn bad_data_is_refused() {
        assert!(edited(|v| v["schema"] = json!("lr.hunter_emotes/9")).is_err());
        assert!(edited(|v| v["triggers"][0]["emotes"] = json!(["nope"])).is_err(), "unknown emote");
        assert!(edited(|v| v["emotes"][1]["id"] = json!("taunt")).is_err(), "duplicate id");
        assert!(edited(|v| v["personalities"][0]["weights"]["taunt"] = json!(-1.0)).is_err(), "negative weight");
        assert!(edited(|v| v["global_cooldown"] = json!("eight")).is_err(), "wrong type");
    }
}
