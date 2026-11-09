//! The levels under the surface, as data: `data/levels.json` (`schema_version` 1) lists each level's region, layer,
//! bed, look, whether it is a whole level (`whole`), and the deepest level (`bottom`). The shipped table is levels
//! -1 (sewer), -2 (lower sewer) and -3 (the Inferno): the numbers the code used before the table existed, so nothing
//! changes for the shipped game. A pack may add levels below them (`pack.json`, section `levels`, same entry shape),
//! one at a time with no gap.
//!
//! A table is refused when a level is not below the surface, names no known region, is listed twice, leaves a gap
//! (-1, -2, -3, then -5 without -4), is missing one of -1 to -3, is marked `bottom` above the deepest level, or
//! goes past `max_depth`.

use crate::world::route::regions::NAMES;
use serde_json::Value;
use std::sync::{OnceLock, RwLock};

/// The table in force for play: None until a pack registers a level (`register_pack_section`), then the table that
/// registration built. Each registration leaks one small table, so reads hand out `&'static` entries: no lock is held
/// past the read and nothing is allocated per call.
static ACTIVE: RwLock<Option<&'static Table>> = RwLock::new(None);

/// The shipped table, read at compile time (the same file the game reads as data).
const SHIPPED: &str = include_str!("../data/levels.json");
const SCHEMA_VERSION: i64 = 1;
/// The shallowest level: -1 is the first level under the surface.
const FIRST_LEVEL: i32 = -1;
/// The deepest level every table must reach (the Inferno).
const REQUIRED_BOTTOM: i32 = -3;
/// Longest text a layer, bed or look may hold.
const MAX_NAME: usize = 32;
/// Depth cap used when the file does not state `max_depth`.
const DEFAULT_MAX_DEPTH: usize = 64;

/// One level of the table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub level: i32,
    /// A region id from `route::regions::NAMES` (required).
    pub region: String,
    /// The layer's name ("" when none); `layer_name` returns it when set.
    pub layer: String,
    /// The bed the level's floor is drawn on ("" when none). Read by no code yet.
    pub bed: String,
    /// The look the level is textured with ("" when none). Read by no code yet.
    pub look: String,
    /// True on the deepest level of the table.
    pub bottom: bool,
    /// True when the whole level is one region (`region_for` returns `region` for every chunk of it).
    pub whole: bool,
}

/// The levels below the surface, index i holding level -(i + 1), so the table is contiguous from -1 down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    entries: Vec<Entry>,
    max_depth: usize,
}

impl Table {
    /// The entry of `level`, or None when the table does not reach it (the surface and levels below the bottom).
    pub fn entry(&self, level: i32) -> Option<&Entry> {
        if level >= 0 {
            return None;
        }
        let index = usize::try_from(-(level as i64) - 1).ok()?;
        self.entries.get(index)
    }

    /// The deepest level of the table.
    pub fn bottom(&self) -> i32 {
        self.entries.last().map_or(REQUIRED_BOTTOM, |e| e.level)
    }

    /// The layer name of `level`, None when the level has no entry or no layer.
    pub fn layer_name(&self, level: i32) -> Option<&str> {
        self.entry(level).map(|e| e.layer.as_str()).filter(|s| !s.is_empty())
    }

    /// Every entry, from -1 down to the bottom.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
}

/// The table read from `text` (the whole `data/levels.json` file). Refused with the reason when it breaks a rule.
pub fn parse(text: &str) -> Result<Table, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("levels.json: not valid JSON ({e})"))?;
    if v.get("schema_version").and_then(Value::as_i64) != Some(SCHEMA_VERSION) {
        return Err("levels.json: \"schema_version\" must be 1 (LEVELS_SCHEMA)".to_string());
    }
    let max_depth = match v.get("max_depth") {
        None => DEFAULT_MAX_DEPTH,
        Some(m) => m
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n >= REQUIRED_BOTTOM.unsigned_abs() as usize)
            .ok_or_else(|| "levels.json: \"max_depth\" must be a whole number of at least 3 (LEVEL_TOO_DEEP)".to_string())?,
    };
    let list = v
        .get("levels")
        .and_then(Value::as_array)
        .ok_or_else(|| "levels.json: \"levels\" must be a list (LEVELS_SCHEMA)".to_string())?;
    let entries = list.iter().map(read_entry).collect::<Result<Vec<_>, _>>()?;
    chain(entries, max_depth)
}

/// `base` with the levels of a pack's `levels` section added below it. The base's bottom flag moves to the new
/// deepest level. Refused with the reason when a section entry breaks a rule (see the module notes).
pub fn extend(base: &Table, section: &Value) -> Result<Table, String> {
    let list = section
        .as_array()
        .ok_or_else(|| "must be a list of levels (LEVELS_SCHEMA)".to_string())?;
    let mut entries: Vec<Entry> = base.entries.iter().map(|e| Entry { bottom: false, ..e.clone() }).collect();
    let mut added = list.iter().map(read_entry).collect::<Result<Vec<Entry>, String>>()?;
    // A section that says nothing about "bottom" makes its deepest level the new bottom.
    if !list.iter().any(|v| v.get("bottom").is_some()) {
        if let Some(deepest) = added.iter_mut().min_by_key(|e| e.level) {
            deepest.bottom = true;
        }
    }
    entries.append(&mut added);
    chain(entries, base.max_depth)
}

/// Checks a pack's `levels` section against the shipped table. Ok when every entry is valid and adds on to it.
pub fn check_section(section: &Value) -> Result<(), String> {
    extend(shipped(), section).map(|_| ())
}

/// The shipped table: data/levels.json, or the same values built in when that file cannot be read.
pub fn shipped() -> &'static Table {
    static CELL: OnceLock<Table> = OnceLock::new();
    CELL.get_or_init(|| parse(SHIPPED).unwrap_or_else(|_| builtin()))
}

/// The table in force: the shipped one, plus the levels of every pack registered so far.
fn active() -> &'static Table {
    match ACTIVE.read() {
        Ok(g) => (*g).unwrap_or_else(shipped),
        Err(_) => shipped(),
    }
}

/// The entry of `level` in the table in force (the shipped table when no pack adds levels).
pub fn entry(level: i32) -> Option<&'static Entry> {
    active().entry(level)
}

/// The deepest level of the table in force (-3 with the shipped file and no pack levels).
pub fn bottom() -> i32 {
    active().bottom()
}

/// The layer name of `level` in the table in force.
pub fn layer_name(level: i32) -> Option<&'static str> {
    active().layer_name(level)
}

/// Whether the route streams `level`: the surface (0) down to the table's deepest level, inclusive.
pub fn allows(level: i32) -> bool {
    (bottom()..=0).contains(&level)
}

/// Adds a pack's `levels` section (data/levels.json's entry shape) to the table in force, so its levels are played.
/// Refused with the reason, and the table left unchanged, when the section breaks a rule (a level listed twice, a gap
/// below the table, an unknown region, ...). Called once per loaded pack from `content::packs::load_all`.
pub fn register_pack_section(section: &Value) -> Result<(), String> {
    let mut guard = ACTIVE.write().map_err(|_| "levels: the level table is unavailable".to_string())?;
    let base: &'static Table = (*guard).unwrap_or_else(shipped);
    let next = extend(base, section)?;
    let leaked: &'static Table = Box::leak(Box::new(next));
    *guard = Some(leaked);
    Ok(())
}

/// Drops every registered pack level, so the shipped table is in force again (tests).
#[cfg(test)]
pub fn reset_pack_levels() {
    if let Ok(mut guard) = ACTIVE.write() {
        *guard = None;
    }
    crate::content::packs::forget_level_packs();
}

/// Serializes the tests that read or register the process-wide table: a test that asserts on a level the registration
/// test adds (-4) takes this lock first.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The shipped values in code, equal to data/levels.json (a test keeps them equal).
fn builtin() -> Table {
    let entry = |level: i32, layer: &str, bed: &str, bottom: bool| {
        let inferno = level == -3;
        Entry {
            level,
            region: if inferno { "inferno" } else { "sewer" }.to_string(),
            layer: layer.to_string(),
            bed: bed.to_string(),
            look: if inferno { "inferno" } else { "sewer" }.to_string(),
            bottom,
            whole: true,
        }
    };
    Table {
        entries: vec![
            entry(-1, "sewer", "sewer_channel", false),
            entry(-2, "sewer", "sewer_channel", false),
            entry(-3, "inferno", "inferno_floor", true),
        ],
        max_depth: DEFAULT_MAX_DEPTH,
    }
}

fn read_text(obj: &serde_json::Map<String, Value>, key: &str, level: i32) -> Result<String, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => {
            let ok = s.len() <= MAX_NAME && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
            if ok {
                Ok(s.clone())
            } else {
                Err(format!("level {level}: \"{key}\" must be up to {MAX_NAME} lower case letters, digits or _"))
            }
        }
        Some(_) => Err(format!("level {level}: \"{key}\" must be text")),
    }
}

fn read_flag(obj: &serde_json::Map<String, Value>, key: &str, level: i32) -> Result<bool, String> {
    read_flag_or(obj, key, level, false)
}

fn read_flag_or(obj: &serde_json::Map<String, Value>, key: &str, level: i32, default: bool) -> Result<bool, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("level {level}: \"{key}\" must be true or false")),
    }
}

/// One level from its JSON object, with every field checked on its own.
fn read_entry(v: &Value) -> Result<Entry, String> {
    let Some(obj) = v.as_object() else {
        return Err("each level must be an object (LEVELS_SCHEMA)".to_string());
    };
    let Some(n) = obj.get("level").and_then(Value::as_i64) else {
        return Err("a level has no \"level\" number (LEVELS_SCHEMA)".to_string());
    };
    let level = i32::try_from(n).map_err(|_| format!("level {n} is out of range (LEVELS_SCHEMA)"))?;
    if level >= 0 {
        return Err(format!("level {level} is not below the surface (LEVEL_NOT_BELOW_SURFACE)"));
    }
    let region = obj.get("region").and_then(Value::as_str).unwrap_or("");
    if region.is_empty() || !NAMES.iter().any(|n| *n == region) {
        return Err(format!("level {level}: region \"{region}\" is missing or not a known region (LEVEL_REGION_MISSING)"));
    }
    Ok(Entry {
        level,
        region: region.to_string(),
        layer: read_text(obj, "layer", level)?,
        bed: read_text(obj, "bed", level)?,
        look: read_text(obj, "look", level)?,
        bottom: read_flag(obj, "bottom", level)?,
        whole: read_flag_or(obj, "whole", level, true)?,
    })
}

/// Sorts the entries from -1 down and checks them as one chain.
fn chain(mut entries: Vec<Entry>, max_depth: usize) -> Result<Table, String> {
    entries.sort_by(|a, b| b.level.cmp(&a.level));
    if let Some(pair) = entries.windows(2).find(|p| p[0].level == p[1].level) {
        return Err(format!("level {} is listed twice (LEVEL_DUPLICATE)", pair[0].level));
    }
    for (i, e) in entries.iter().enumerate() {
        let want = FIRST_LEVEL - i as i32;
        if e.level != want {
            return Err(format!("level {} has a gap: level {want} is missing (LEVEL_GAP)", e.level));
        }
    }
    if entries.len() < REQUIRED_BOTTOM.unsigned_abs() as usize {
        return Err(format!("levels -1 to {REQUIRED_BOTTOM} are required (LEVEL_MISSING)"));
    }
    if entries.len() > max_depth {
        return Err(format!("the table holds {} levels; at most {max_depth} (LEVEL_TOO_DEEP)", entries.len()));
    }
    let last = entries.len() - 1;
    if let Some((_, e)) = entries.iter().enumerate().find(|(i, e)| e.bottom && *i != last) {
        return Err(format!("level {} is marked bottom but is not the deepest level (LEVEL_BOTTOM)", e.level));
    }
    Ok(Table { entries, max_depth })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::infinite::{level_allowed, INFERNO_LEVEL};
    use crate::world::route::regions::{godot_region_for, region_for};
    use crate::world::route::sewer::core as sewer;
    use serde_json::json;
    use std::sync::MutexGuard;

    /// Holds the test lock and starts from the shipped table; the registry is cleared again when this drops.
    struct Fresh(MutexGuard<'static, ()>);

    fn fresh() -> Fresh {
        let guard = super::TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset_pack_levels();
        Fresh(guard)
    }

    impl Drop for Fresh {
        fn drop(&mut self) {
            reset_pack_levels();
        }
    }

    #[test]
    fn the_shipped_file_parses_and_equals_the_built_in_defaults() {
        let parsed = parse(SHIPPED).expect("data/levels.json is a valid table");
        assert_eq!(parsed, builtin());
        assert_eq!(parsed, *shipped());
    }

    #[test]
    fn the_shipped_table_is_today_s_sewer_and_inferno() {
        let _fresh = fresh();
        let t = shipped();
        let e1 = t.entry(-1).expect("level -1");
        assert_eq!((e1.region.as_str(), e1.layer.as_str()), ("sewer", "sewer"));
        let e2 = t.entry(-2).expect("level -2");
        assert_eq!((e2.region.as_str(), e2.layer.as_str()), ("sewer", "sewer"));
        let e3 = t.entry(-3).expect("level -3");
        assert_eq!((e3.region.as_str(), e3.layer.as_str()), ("inferno", "inferno"));
        assert!(e3.bottom && !e1.bottom && !e2.bottom);
        assert_eq!(t.bottom(), -3);
        assert_eq!(bottom(), INFERNO_LEVEL);
        assert_eq!(bottom(), sewer::INFERNO_LEVEL);
        assert_eq!((sewer::LEVEL, sewer::LOWER_LEVEL), (-1, -2));
        assert!(t.entry(0).is_none() && t.entry(-4).is_none() && t.entry(1).is_none());
        assert_eq!(layer_name(-3), Some("inferno"));
        assert_eq!(layer_name(-1), Some("sewer"));
        assert_eq!(layer_name(0), None);
        assert_eq!(layer_name(-4), None);
    }

    #[test]
    fn with_no_pack_loaded_the_table_in_force_is_the_shipped_one() {
        let _fresh = fresh();
        assert_eq!(*active(), *shipped());
        assert_eq!((bottom(), allows(-3), allows(-4), allows(0), allows(1)), (-3, true, false, true, false));
    }

    #[test]
    fn a_registered_pack_level_is_played_and_a_refused_one_changes_nothing() {
        let _fresh = fresh();
        let shipped_copy = shipped().clone();
        let section = json!([{ "level": -4, "region": "prison", "layer": "prison" }]);
        register_pack_section(&section).expect("-4 follows -3");
        assert_eq!(entry(-4).map(|e| (e.region.as_str(), e.bottom)), Some(("prison", true)));
        assert_eq!(bottom(), -4);
        assert!(allows(-4) && level_allowed(-4), "the route streams the new level");
        assert_eq!(layer_name(-4), Some("prison"));
        assert_eq!(entry(-3).map(|e| e.bottom), Some(false), "the old bottom flag moves to -4");
        let after_first = active().clone();

        let twice = register_pack_section(&section).unwrap_err();
        assert!(twice.contains("LEVEL_DUPLICATE"), "{twice}");
        assert_eq!(*active(), after_first, "a duplicate leaves the table unchanged");

        let gap = register_pack_section(&json!([{ "level": -6, "region": "prison" }])).unwrap_err();
        assert!(gap.contains("LEVEL_GAP"), "{gap}");
        assert_eq!(*active(), after_first, "a gap leaves the table unchanged");

        reset_pack_levels();
        assert_eq!(*active(), shipped_copy, "without the pack the shipped table is back");
        assert_eq!(entry(-4).map(|e| e.level), None);
        assert_eq!(layer_name(-4), None);
    }

    #[test]
    fn the_shipped_levels_keep_today_s_region_and_streaming_behaviour() {
        let _fresh = fresh();
        assert!(level_allowed(-1) && level_allowed(-2) && level_allowed(INFERNO_LEVEL));
        assert!(!level_allowed(INFERNO_LEVEL - 1));
        for level in [-1, -2, -3] {
            for seed in 1..40 {
                let world = format!("ROOM-X-{seed:08}");
                for (cx, cz) in [(0, 0), (seed, -seed), (9, 9)] {
                    assert_eq!(region_for(&world, cx, cz, level), godot_region_for(&world, cx, cz, level));
                }
            }
        }
    }

    #[test]
    fn a_pack_level_below_the_inferno_validates_and_is_returned() {
        let _fresh = fresh();
        let extra = json!([{ "level": -4, "region": "prison", "layer": "prison" }]);
        assert!(check_section(&extra).is_ok());
        let t = extend(shipped(), &extra).expect("-4 follows -3");
        let e4 = t.entry(-4).expect("the pack's level -4 is returned");
        assert_eq!((e4.region.as_str(), e4.layer.as_str()), ("prison", "prison"));
        assert_eq!(t.bottom(), -4);
        assert!(!t.entry(-3).unwrap().bottom, "the old bottom flag moves to the new deepest level");
        assert_eq!(t.layer_name(-4), Some("prison"));
        assert_eq!(bottom(), -3, "the shipped table is untouched");
    }

    #[test]
    fn a_gap_below_the_inferno_is_refused() {
        let gap = json!([{ "level": -5, "region": "prison" }]);
        let why = extend(shipped(), &gap).unwrap_err();
        assert!(why.contains("LEVEL_GAP") && why.contains("-4"), "{why}");
    }

    #[test]
    fn a_duplicate_level_is_refused() {
        let dup = json!([{ "level": -3, "region": "prison" }]);
        let why = extend(shipped(), &dup).unwrap_err();
        assert!(why.contains("LEVEL_DUPLICATE"), "{why}");
    }

    #[test]
    fn a_level_without_a_known_region_is_refused() {
        let none = json!([{ "level": -4 }]);
        assert!(extend(shipped(), &none).unwrap_err().contains("LEVEL_REGION_MISSING"));
        let unknown = json!([{ "level": -4, "region": "dungeon" }]);
        assert!(extend(shipped(), &unknown).unwrap_err().contains("LEVEL_REGION_MISSING"));
    }

    #[test]
    fn a_level_at_or_above_the_surface_is_refused() {
        let surface = json!([{ "level": 0, "region": "maze" }]);
        assert!(extend(shipped(), &surface).unwrap_err().contains("LEVEL_NOT_BELOW_SURFACE"));
    }

    #[test]
    fn bottom_must_be_the_deepest_level_and_the_depth_is_capped() {
        let marked_early = json!([
            { "level": -4, "region": "prison", "bottom": true },
            { "level": -5, "region": "prison" }
        ]);
        assert!(extend(shipped(), &marked_early).unwrap_err().contains("LEVEL_BOTTOM"));
        let shallow = parse(&SHIPPED.replace("\"max_depth\": 64", "\"max_depth\": 3")).expect("a 3-level cap parses");
        let deeper = json!([{ "level": -4, "region": "prison" }]);
        assert!(extend(&shallow, &deeper).unwrap_err().contains("LEVEL_TOO_DEEP"));
    }

    #[test]
    fn the_file_form_needs_the_schema_and_the_three_required_levels() {
        assert!(parse(r#"{"levels":[]}"#).unwrap_err().contains("LEVELS_SCHEMA"));
        let short = r#"{"schema_version":1,"levels":[
            {"level":-1,"region":"sewer"},{"level":-2,"region":"sewer"}]}"#;
        assert!(parse(short).unwrap_err().contains("LEVEL_MISSING"));
    }
}
