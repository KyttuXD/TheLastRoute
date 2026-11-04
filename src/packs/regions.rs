//! The region catalogue (standalone: serde_json and std only, so `lr_server` builds without the game
//! modules): `lr.region/1` files (data/regions/<id>.json). A port of the region contract
//! in game/config/content_registry.gd (REGION_FIELDS, _parse_entry, resolve_with) and of
//! REGION_RULES.md section 1, so both engines read the same files with the same rules.
//!
//! A region file holds only the facts the data owns. The rules that place a region
//! (`route::regions::region_for`, the sewer and Inferno levels, the layout classes) stay in code:
//! a data file names no code path, and the schema has no key for one.
//!
//! Packs: a pack's `regions/<id>.json` adds a new region id (it loads, but no level places it) or
//! replaces a built-in region when the pack's `overrides` lists `"regions/<id>"`. A file with any
//! error is dropped. A pack whose region files have any error adds no regions. Every error names
//! the file and the key.

use super::Pack;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The wall height, in metres, of a region without a `geometry` block (the game's `config::WALL_HEIGHT`).
pub const DEFAULT_WALL_HEIGHT: f32 = 5.0;
/// The default smallest room side, in grid cells (the game's `world::facility::MIN_AUTHORED_ROOM_SIZE`).
pub const DEFAULT_ROOM_MIN: i32 = 10;
/// The largest room side, in grid cells (the game's `world::facility::MAX_AUTHORED_ROOM_SIZE`).
pub const MAX_ROOM_SIZE: i32 = 26;

/// The built-in region files, `(file name, text)`. The game embeds the same files from data/regions/ at
/// build time; a test in `content::regions` keeps the two lists equal.
pub const BUILTIN_FILES: [(&str, &str); 13] = [
    ("regions/apartment.json", include_str!("../../data/regions/apartment.json")),
    ("regions/beach.json", include_str!("../../data/regions/beach.json")),
    ("regions/carnival.json", include_str!("../../data/regions/carnival.json")),
    ("regions/gothic.json", include_str!("../../data/regions/gothic.json")),
    ("regions/greenhouse.json", include_str!("../../data/regions/greenhouse.json")),
    ("regions/hospital.json", include_str!("../../data/regions/hospital.json")),
    ("regions/hotel.json", include_str!("../../data/regions/hotel.json")),
    ("regions/lab.json", include_str!("../../data/regions/lab.json")),
    ("regions/mall.json", include_str!("../../data/regions/mall.json")),
    ("regions/mansion.json", include_str!("../../data/regions/mansion.json")),
    ("regions/maze.json", include_str!("../../data/regions/maze.json")),
    ("regions/prison.json", include_str!("../../data/regions/prison.json")),
    ("regions/sewer.json", include_str!("../../data/regions/sewer.json")),
];

/// The only region schema this build reads. A new shape is `lr.region/2`, added in both engines.
pub const SCHEMA: &str = "lr.region/1";
/// The source name of a built-in region (every other source is the id of the pack that added it).
pub const BUILTIN: &str = "builtin";

/// Every top-level key of `lr.region/1` (Godot REGION_FIELDS). Any other key is an error. `geometry` and
/// `safe_light` are optional: they were added to /1 before the first full release, so /1 stays the schema name.
const KEYS: [&str; 13] = [
    "schema",
    "id",
    "hall_floor",
    "hall_runner",
    "room_default_floor",
    "room_roles",
    "shop_names",
    "room_door",
    "double_room_door",
    "prop_set",
    "geometry",
    "safe_light",
    // the sewer level generator's layout rules (world/route/sewer/core.rs `Rules`); not read here
    "layout",
];
/// The keys of the optional `geometry` block.
const GEOMETRY_KEYS: [&str; 3] = ["wall_height", "room_min", "room_max"];
/// The allowed range of `geometry.wall_height`, in metres.
pub const WALL_HEIGHT_RANGE: (f64, f64) = (2.5, 8.0);
/// The keys of the optional `safe_light` block.
const SAFE_LIGHT_KEYS: [&str; 2] = ["color", "energy"];
/// The allowed range of `safe_light.energy`.
pub const SAFE_LIGHT_ENERGY_RANGE: (f64, f64) = (0.2, 6.0);
/// The safe-zone checkpoint light's colour (linear RGB, 0..1) when the region has no `safe_light` block.
pub const SAFE_LIGHT_COLOR: [f32; 3] = [0.95, 0.78, 0.55];
/// The safe-zone checkpoint light's energy when the region has no `safe_light` block.
pub const SAFE_LIGHT_ENERGY: f32 = 2.0;
/// The keys a file must have. `shop_names`, `room_door` and `double_room_door` are optional.
const REQUIRED: [&str; 7] = [
    "schema",
    "id",
    "hall_floor",
    "hall_runner",
    "room_default_floor",
    "room_roles",
    "prop_set",
];
/// The keys of one `prop_set` item. `solid` defaults to true (prop_tables.rs PropSetEntry).
const PROP_KEYS: [&str; 3] = ["type", "size", "solid"];

/// One `prop_set` item: a catalogue prop type with its authored blocker box (metres) and solidity.
#[derive(Clone, Debug, PartialEq)]
pub struct PropChoice {
    pub prop_type: String,
    pub size: [f32; 3],
    pub solid: bool,
}

/// The optional `geometry` block: the wall height in metres, and the room size range in grid cells (a room's
/// side is drawn from `room_min..=room_max`). A region without the block has the game's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionGeometry {
    pub wall_height: f32,
    pub room_min: i32,
    pub room_max: i32,
}

impl Default for RegionGeometry {
    fn default() -> Self {
        RegionGeometry {
            wall_height: DEFAULT_WALL_HEIGHT,
            room_min: DEFAULT_ROOM_MIN,
            room_max: MAX_ROOM_SIZE,
        }
    }
}

/// The optional `safe_light` block: the colour and energy of the safe zone's checkpoint light in a region.
/// A region without the block has `RegionSafeLight::default()`, the game's current light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionSafeLight {
    pub color: [f32; 3],
    pub energy: f32,
}

impl Default for RegionSafeLight {
    fn default() -> Self {
        RegionSafeLight {
            color: SAFE_LIGHT_COLOR,
            energy: SAFE_LIGHT_ENERGY,
        }
    }
}

/// One region's facts, as `lr.region/1` states them.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionDef {
    pub id: String,
    pub hall_floor: Vec<String>,
    pub hall_runner: bool,
    pub room_default_floor: Vec<String>,
    pub room_roles: Vec<String>,
    /// Mall signs, in file order (room role to sign text).
    pub shop_names: Vec<(String, String)>,
    pub room_door: Option<String>,
    pub double_room_door: Option<String>,
    pub prop_set: Vec<PropChoice>,
    /// Wall height and room sizes, when the file has a `geometry` block. `None` keeps the game-wide sizes.
    pub geometry: Option<RegionGeometry>,
    /// Safe-zone light colour and energy, when the file has a `safe_light` block. `None` keeps the game's light.
    pub safe_light: Option<RegionSafeLight>,
    /// `BUILTIN`, or the id of the pack that added or replaced the region.
    pub source: String,
}

/// The regions of one loaded game, sorted by id.
#[derive(Clone, Debug, Default)]
pub struct RegionTable {
    pub defs: Vec<RegionDef>,
}

fn wrong(file: &str, key: &str, what: &str) -> String {
    format!("{file}: \"{key}\" must be {what}")
}

fn string_list(file: &str, obj: &Map<String, Value>, key: &str) -> Result<Vec<String>, String> {
    let items = obj
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| wrong(file, key, "an array of strings"))?;
    items
        .iter()
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .ok_or_else(|| wrong(file, key, "an array of strings"))
        })
        .collect()
}

fn shop_names(file: &str, obj: &Map<String, Value>) -> Result<Vec<(String, String)>, String> {
    let Some(value) = obj.get("shop_names") else {
        return Ok(vec![]);
    };
    let map = value
        .as_object()
        .ok_or_else(|| wrong(file, "shop_names", "an object of strings"))?;
    map.iter()
        .map(|(role, sign)| {
            sign.as_str()
                .map(|s| (role.clone(), s.to_string()))
                .ok_or_else(|| wrong(file, &format!("shop_names.{role}"), "a string"))
        })
        .collect()
}

/// The optional `geometry` block. Missing keys keep the defaults; `room_min` is capped at `room_max` when only
/// `room_max` is given, so a small `room_max` alone is valid.
fn geometry(file: &str, obj: &Map<String, Value>) -> Result<Option<RegionGeometry>, String> {
    let Some(value) = obj.get("geometry") else {
        return Ok(None);
    };
    let mut g = RegionGeometry::default();
    let block = value
        .as_object()
        .ok_or_else(|| wrong(file, "geometry", "an object"))?;
    if let Some(key) = block.keys().find(|k| !GEOMETRY_KEYS.contains(&k.as_str())) {
        return Err(format!("{file}: unknown key \"geometry.{key}\""));
    }
    if let Some(value) = block.get("wall_height") {
        let (lo, hi) = WALL_HEIGHT_RANGE;
        let h = value
            .as_f64()
            .filter(|h| (lo..=hi).contains(h))
            .ok_or_else(|| wrong(file, "geometry.wall_height", &format!("a number from {lo} to {hi}")))?;
        g.wall_height = h as f32;
    }
    let (min_lo, max_hi) = (4, MAX_ROOM_SIZE);
    let size = |key: &str| -> Result<Option<i32>, String> {
        match block.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_i64()
                .filter(|n| (min_lo as i64..=max_hi as i64).contains(n))
                .map(|n| Some(n as i32))
                .ok_or_else(|| wrong(file, &format!("geometry.{key}"), &format!("a whole number from {min_lo} to {max_hi}"))),
        }
    };
    let (min, max) = (size("room_min")?, size("room_max")?);
    if let Some(max) = max {
        g.room_max = max;
        g.room_min = g.room_min.min(max);
    }
    if let Some(min) = min {
        g.room_min = min;
    }
    if g.room_min > g.room_max {
        return Err(format!("{file}: \"geometry.room_min\" ({}) is larger than \"geometry.room_max\" ({})", g.room_min, g.room_max));
    }
    Ok(Some(g))
}

/// The optional `safe_light` block. Missing keys keep the defaults. `color` is three numbers from 0 to 1.
fn safe_light(file: &str, obj: &Map<String, Value>) -> Result<Option<RegionSafeLight>, String> {
    let Some(value) = obj.get("safe_light") else {
        return Ok(None);
    };
    let mut s = RegionSafeLight::default();
    let block = value
        .as_object()
        .ok_or_else(|| wrong(file, "safe_light", "an object"))?;
    if let Some(key) = block.keys().find(|k| !SAFE_LIGHT_KEYS.contains(&k.as_str())) {
        return Err(format!("{file}: unknown key \"safe_light.{key}\""));
    }
    if let Some(value) = block.get("color") {
        let numbers = value
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or_else(|| wrong(file, "safe_light.color", "three numbers from 0 to 1"))?;
        for (slot, n) in s.color.iter_mut().zip(numbers) {
            *slot = n
                .as_f64()
                .filter(|c| (0.0..=1.0).contains(c))
                .ok_or_else(|| wrong(file, "safe_light.color", "three numbers from 0 to 1"))? as f32;
        }
    }
    if let Some(value) = block.get("energy") {
        let (lo, hi) = SAFE_LIGHT_ENERGY_RANGE;
        let e = value
            .as_f64()
            .filter(|e| (lo..=hi).contains(e))
            .ok_or_else(|| wrong(file, "safe_light.energy", &format!("a number from {lo} to {hi}")))?;
        s.energy = e as f32;
    }
    Ok(Some(s))
}

fn optional_string(file: &str, obj: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match obj.get(key) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| wrong(file, key, "a string")),
    }
}

fn prop_set(file: &str, obj: &Map<String, Value>) -> Result<Vec<PropChoice>, String> {
    let items = obj
        .get("prop_set")
        .and_then(Value::as_array)
        .ok_or_else(|| wrong(file, "prop_set", "an array of objects"))?;
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at = format!("prop_set[{i}]");
        let item = item
            .as_object()
            .ok_or_else(|| wrong(file, &at, "an object"))?;
        if let Some(key) = item.keys().find(|k| !PROP_KEYS.contains(&k.as_str())) {
            return Err(format!("{file}: \"{at}\" has unknown key \"{key}\""));
        }
        let prop_type = item
            .get("type")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| wrong(file, &format!("{at}.type"), "a non-empty string"))?;
        let numbers = item
            .get("size")
            .and_then(Value::as_array)
            .filter(|a| a.len() == 3)
            .ok_or_else(|| wrong(file, &format!("{at}.size"), "three numbers"))?;
        let mut size = [0.0f32; 3];
        for (slot, n) in size.iter_mut().zip(numbers) {
            let n = n
                .as_f64()
                .ok_or_else(|| wrong(file, &format!("{at}.size"), "three numbers"))?;
            *slot = n as f32;
        }
        let solid = match item.get("solid") {
            None => true,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| wrong(file, &format!("{at}.solid"), "true or false"))?,
        };
        out.push(PropChoice {
            prop_type: prop_type.to_string(),
            size,
            solid,
        });
    }
    Ok(out)
}

/// Parses one `lr.region/1` file. `file` names the file in every error (for a pack, "<pack>/regions/<name>").
/// The returned definition has an empty `source`; the caller sets it.
pub fn parse_file(file: &str, text: &str) -> Result<RegionDef, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| format!("{file}: the file must hold a JSON object"))?;
    match obj.get("schema") {
        None => return Err(format!("{file}: missing \"schema\" (expected \"{SCHEMA}\")")),
        Some(found) if found.as_str() != Some(SCHEMA) => {
            return Err(format!("{file}: \"schema\" is {found}, expected \"{SCHEMA}\""))
        }
        Some(_) => {}
    }
    if let Some(key) = obj.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("{file}: unknown key \"{key}\""));
    }
    if let Some(key) = REQUIRED.iter().find(|k| !obj.contains_key(**k)) {
        return Err(format!("{file}: missing required key \"{key}\""));
    }
    let id = obj
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .ok_or_else(|| wrong(file, "id", "a non-empty string of letters, digits and _"))?;
    let hall_runner = obj
        .get("hall_runner")
        .and_then(Value::as_bool)
        .ok_or_else(|| wrong(file, "hall_runner", "true or false"))?;
    Ok(RegionDef {
        id: id.to_string(),
        hall_floor: string_list(file, obj, "hall_floor")?,
        hall_runner,
        room_default_floor: string_list(file, obj, "room_default_floor")?,
        room_roles: string_list(file, obj, "room_roles")?,
        shop_names: shop_names(file, obj)?,
        room_door: optional_string(file, obj, "room_door")?,
        double_room_door: optional_string(file, obj, "double_room_door")?,
        prop_set: prop_set(file, obj)?,
        geometry: geometry(file, obj)?,
        safe_light: safe_light(file, obj)?,
        source: String::new(),
    })
}

impl RegionTable {
    /// The definition of a region id, if the table has one.
    pub fn def(&self, id: &str) -> Option<&RegionDef> {
        self.defs.iter().find(|d| d.id == id)
    }

    /// Builds the built-in table from `(file, text)` pairs. A bad file is dropped and reported in
    /// `warnings`; a second file with an id already loaded is dropped too.
    pub fn from_builtin_files<'a>(
        files: impl IntoIterator<Item = (&'a str, &'a str)>,
        warnings: &mut Vec<String>,
    ) -> RegionTable {
        let mut table = RegionTable::default();
        for (file, text) in files {
            match parse_file(file, text) {
                Ok(mut def) => {
                    if table.def(&def.id).is_some() {
                        warnings.push(format!(
                            "{file}: region {} is already defined by another built-in file",
                            def.id
                        ));
                        continue;
                    }
                    def.source = BUILTIN.to_string();
                    table.defs.push(def);
                }
                Err(e) => warnings.push(e),
            }
        }
        table.defs.sort_by(|a, b| a.id.cmp(&b.id));
        table
    }

    /// The built-in regions: every file under data/regions/ (`BUILTIN_FILES`).
    pub fn builtin(warnings: &mut Vec<String>) -> RegionTable {
        Self::from_builtin_files(BUILTIN_FILES, warnings)
    }

    /// Adds one pack's region files, `(file name, text)` in file order, under REGION_RULES 1.3.
    /// `overrides` is the pack's `overrides` list. Any error (a bad file, a file defined twice, a
    /// replacement that is not listed, a listed override of a region the game does not have, or a
    /// region another pack already added) rejects every region file of this pack: the table is left
    /// unchanged and each error is reported.
    pub fn add_pack(
        &mut self,
        pack_id: &str,
        overrides: &[String],
        files: &[(String, String)],
        warnings: &mut Vec<String>,
    ) {
        let mut errors: Vec<String> = vec![];
        let mut parsed: Vec<RegionDef> = vec![];
        for (name, text) in files {
            let file = format!("{pack_id}/regions/{name}");
            match parse_file(&file, text) {
                Ok(def) if parsed.iter().any(|p| p.id == def.id) => errors.push(format!(
                    "{file}: region {} is defined twice in this pack",
                    def.id
                )),
                Ok(def) => parsed.push(def),
                Err(e) => errors.push(e),
            }
        }
        let listed = |id: &str| overrides.iter().any(|o| o.strip_prefix("regions/") == Some(id));
        for id in overrides.iter().filter_map(|o| o.strip_prefix("regions/")) {
            if self.def(id).is_none() {
                errors.push(format!(
                    "{pack_id}: override \"regions/{id}\" names a region the game does not have"
                ));
            }
        }
        for def in &parsed {
            match self.def(&def.id) {
                Some(existing) if existing.source != BUILTIN => errors.push(format!(
                    "{pack_id}: region {} is already defined by {}",
                    def.id, existing.source
                )),
                Some(_) if !listed(&def.id) => errors.push(format!(
                    "{pack_id}: region {} already exists; list \"regions/{}\" in \"overrides\" to replace it",
                    def.id, def.id
                )),
                _ => {}
            }
        }
        if !errors.is_empty() {
            warnings.extend(errors);
            warnings.push(format!(
                "{pack_id}: region files rejected; this pack adds or replaces no regions"
            ));
            return;
        }
        for mut def in parsed {
            def.source = pack_id.to_string();
            match self.defs.iter().position(|d| d.id == def.id) {
                Some(i) => self.defs[i] = def,
                None => {
                    warnings.push(format!(
                        "{pack_id}: region {} is loaded, but no level places it (only built-in regions are placed)",
                        def.id
                    ));
                    self.defs.push(def);
                }
            }
        }
        self.defs.sort_by(|a, b| a.id.cmp(&b.id));
    }
}

/// The `overrides` list of a pack's pack.json (empty when it has none).
fn pack_overrides(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("pack.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|m| {
            m["overrides"].as_array().map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// The `regions/*.json` files of a loaded pack, sorted by name. An unreadable file is reported and skipped.
fn pack_region_files(pack: &Pack, warnings: &mut Vec<String>) -> Vec<(String, String)> {
    let dir = pack.dir.join("regions");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
                .collect()
        })
        .unwrap_or_default();
    paths.sort();
    let mut out = vec![];
    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        match std::fs::read_to_string(&path) {
            Ok(text) => out.push((name, text)),
            Err(e) => warnings.push(format!("{}/regions/{name}: {e}", pack.id)),
        }
    }
    out
}

/// The region table of a loaded game: the built-in files, then each loaded pack's `regions/` in pack order.
pub fn load(packs: &[Pack], warnings: &mut Vec<String>) -> RegionTable {
    let mut table = RegionTable::builtin(warnings);
    for pack in packs {
        let overrides = pack_overrides(&pack.dir);
        let files = pack_region_files(pack, warnings);
        if files.is_empty() && !overrides.iter().any(|o| o.starts_with("regions/")) {
            continue;
        }
        table.add_pack(&pack.id, &overrides, &files, warnings);
    }
    table
}

static BUILT_IN: OnceLock<RegionTable> = OnceLock::new();
static ACTIVE: OnceLock<RegionTable> = OnceLock::new();

/// Makes `table` the table the game reads. `Content::load` calls it with the built-in files and the
/// loaded packs. The first call wins; later calls are ignored.
pub fn install(table: RegionTable) {
    let _ = ACTIVE.set(table);
}

/// The table the game reads: the installed one, or the built-in files when nothing is installed yet.
pub fn table() -> &'static RegionTable {
    if let Some(active) = ACTIVE.get() {
        return active;
    }
    BUILT_IN.get_or_init(|| RegionTable::builtin(&mut Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSPITAL: &str = include_str!("../../data/regions/hospital.json");
    const MALL: &str = include_str!("../../data/regions/mall.json");

    /// The smallest valid file: one prop item, no optional keys.
    const MIN: &str = r#"{"schema":"lr.region/1","id":"t","hall_floor":["a"],"hall_runner":false,"room_default_floor":["a"],"room_roles":["r"],"prop_set":[{"type":"crate","size":[1,1,1]}]}"#;

    /// A replacement mall: same id, different facts.
    const ALT_MALL: &str = r#"{"schema":"lr.region/1","id":"mall","hall_floor":["terrazzo0"],"hall_runner":true,"room_default_floor":["terrazzo0"],"room_roles":["cinema"],"prop_set":[]}"#;

    fn builtin() -> RegionTable {
        RegionTable::builtin(&mut Vec::new())
    }

    fn fails(text: &str) -> String {
        parse_file("x.json", text).expect_err("the file should be rejected")
    }

    #[test]
    fn builtin_files_load_without_warnings() {
        let mut warnings = vec![];
        let t = RegionTable::builtin(&mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        // the built-in set grows as regions move to data, so check the known ones are there, not the whole list
        let ids: Vec<&str> = t.defs.iter().map(|d| d.id.as_str()).collect();
        for id in ["hospital", "mall"] {
            assert!(ids.contains(&id), "built-in region {id} is missing from {ids:?}");
        }
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "duplicate region ids in {ids:?}");
        assert!(t.defs.iter().all(|d| d.source == BUILTIN));
    }

    #[test]
    fn built_in_facts_are_read_from_the_files() {
        let t = builtin();
        let mall = t.def("mall").unwrap();
        assert_eq!(mall.room_door.as_deref(), Some("normalRoomDoor"));
        assert_eq!(mall.double_room_door.as_deref(), Some("normalDoubleRoomDoor"));
        assert_eq!(mall.shop_names.len(), 14);
        assert_eq!(mall.prop_set.iter().filter(|p| !p.solid).count(), 1);
        let hospital = t.def("hospital").unwrap();
        assert_eq!(hospital.room_door, None);
        assert_eq!(hospital.prop_set.len(), 7);
        assert_eq!(hospital.prop_set.iter().filter(|p| !p.solid).count(), 2);
        // the two shipped files parse the same as the copies in the test sources
        assert!(parse_file("hospital.json", HOSPITAL).is_ok());
        assert!(parse_file("mall.json", MALL).is_ok());
    }

    #[test]
    fn a_minimal_file_parses_with_defaults() {
        let d = parse_file("t.json", MIN).unwrap();
        assert_eq!(d.id, "t");
        assert_eq!(d.prop_set, vec![PropChoice { prop_type: "crate".into(), size: [1.0, 1.0, 1.0], solid: true }]);
        assert!(d.shop_names.is_empty());
        assert_eq!(d.room_door, None);
    }

    #[test]
    fn a_file_without_geometry_keeps_the_defaults() {
        let d = parse_file("t.json", MIN).unwrap();
        assert_eq!(d.geometry, None);
    }

    #[test]
    fn a_geometry_block_sets_wall_height_and_room_sizes() {
        let text = MIN.replacen("\"id\":\"t\"", "\"id\":\"t\",\"geometry\":{\"wall_height\":6.5,\"room_min\":8,\"room_max\":14}", 1);
        let g = parse_file("t.json", &text).unwrap().geometry;
        assert_eq!(g, Some(RegionGeometry { wall_height: 6.5, room_min: 8, room_max: 14 }));
    }

    #[test]
    fn a_room_max_alone_caps_the_default_minimum() {
        let text = MIN.replacen("\"id\":\"t\"", "\"id\":\"t\",\"geometry\":{\"room_max\":6}", 1);
        let g = parse_file("t.json", &text).unwrap().geometry.unwrap();
        assert_eq!((g.room_min, g.room_max), (6, 6));
        assert_eq!(g.wall_height, RegionGeometry::default().wall_height);
    }

    #[test]
    fn a_wall_height_outside_the_range_names_the_key() {
        let text = MIN.replacen("\"id\":\"t\"", "\"id\":\"t\",\"geometry\":{\"wall_height\":12}", 1);
        let e = fails(&text);
        assert!(e.contains("geometry.wall_height"), "{e}");
    }

    #[test]
    fn an_unknown_geometry_key_fails() {
        let text = MIN.replacen("\"id\":\"t\"", "\"id\":\"t\",\"geometry\":{\"hall_width\":3}", 1);
        let e = fails(&text);
        assert!(e.contains("unknown key \"geometry.hall_width\""), "{e}");
    }

    #[test]
    fn room_min_above_room_max_fails() {
        let text = MIN.replacen("\"id\":\"t\"", "\"id\":\"t\",\"geometry\":{\"room_min\":20,\"room_max\":12}", 1);
        let e = fails(&text);
        assert!(e.contains("room_min") && e.contains("room_max"), "{e}");
    }

    fn with_safe_light(block: &str) -> String {
        MIN.replacen("\"id\":\"t\"", &format!("\"id\":\"t\",\"safe_light\":{block}"), 1)
    }

    #[test]
    fn a_file_without_safe_light_has_none_and_the_defaults_match_the_game_light() {
        let d = parse_file("t.json", MIN).unwrap();
        assert_eq!(d.safe_light, None);
        let s = RegionSafeLight::default();
        assert_eq!(s.color, [0.95, 0.78, 0.55]);
        assert!((s.energy - 2.0_f32).abs() < 1e-4);
        assert_eq!(s, RegionSafeLight { color: SAFE_LIGHT_COLOR, energy: SAFE_LIGHT_ENERGY });
    }

    #[test]
    fn a_safe_light_block_sets_colour_and_energy() {
        let d = parse_file("t.json", &with_safe_light(r#"{"color":[0.2,0.5,1],"energy":4.5}"#)).unwrap();
        assert_eq!(d.safe_light, Some(RegionSafeLight { color: [0.2, 0.5, 1.0], energy: 4.5 }));
    }

    #[test]
    fn a_safe_light_block_with_missing_keys_keeps_the_defaults() {
        let only_energy = parse_file("t.json", &with_safe_light(r#"{"energy":0.2}"#)).unwrap();
        assert_eq!(only_energy.safe_light, Some(RegionSafeLight { color: SAFE_LIGHT_COLOR, energy: 0.2 }));
        let empty = parse_file("t.json", &with_safe_light("{}")).unwrap();
        assert_eq!(empty.safe_light, Some(RegionSafeLight::default()));
    }

    #[test]
    fn a_safe_light_energy_outside_the_range_names_the_key() {
        for bad in ["0.1", "6.5", "\"2\""] {
            let e = fails(&with_safe_light(&format!(r#"{{"energy":{bad}}}"#)));
            assert!(e.contains("safe_light.energy") && e.contains("0.2 to 6"), "{e}");
        }
    }

    #[test]
    fn a_safe_light_colour_outside_zero_to_one_names_the_key() {
        for bad in ["[1.2,0,0]", "[0,-0.1,0]", "[0,0]", "[0,0,\"x\"]", "\"red\""] {
            let e = fails(&with_safe_light(&format!(r#"{{"color":{bad}}}"#)));
            assert!(e.contains("safe_light.color") && e.contains("three numbers from 0 to 1"), "{e}");
        }
    }

    #[test]
    fn an_unknown_safe_light_key_fails() {
        let e = fails(&with_safe_light(r#"{"flicker":true}"#));
        assert!(e.contains("unknown key \"safe_light.flicker\""), "{e}");
    }

    #[test]
    fn a_safe_light_that_is_not_an_object_fails() {
        let e = fails(&with_safe_light("[1]"));
        assert!(e.contains("\"safe_light\" must be an object"), "{e}");
    }

    #[test]
    fn an_unknown_key_names_the_file_and_the_key() {
        let text = MIN.replacen("\"id\":\"t\"", "\"id\":\"t\",\"colour\":\"red\"", 1);
        let e = fails(&text);
        assert!(e.contains("x.json"), "{e}");
        assert!(e.contains("unknown key \"colour\""), "{e}");
    }

    #[test]
    fn a_missing_schema_fails() {
        let text = MIN.replacen("\"schema\":\"lr.region/1\",", "", 1);
        let e = fails(&text);
        assert!(e.contains("x.json") && e.contains("missing \"schema\""), "{e}");
    }

    #[test]
    fn a_wrong_schema_fails() {
        let e = fails(&MIN.replacen("lr.region/1", "lr.region/2", 1));
        assert!(e.contains("\"schema\" is \"lr.region/2\""), "{e}");
    }

    #[test]
    fn a_missing_required_key_fails() {
        let text = MIN.replacen("\"room_roles\":[\"r\"],", "", 1);
        let e = fails(&text);
        assert!(e.contains("missing required key \"room_roles\""), "{e}");
    }

    #[test]
    fn a_wrong_value_type_fails_and_names_the_key() {
        let cases = [
            (MIN.replacen("\"hall_runner\":false", "\"hall_runner\":\"no\"", 1), "\"hall_runner\" must be true or false"),
            (MIN.replacen("\"room_roles\":[\"r\"]", "\"room_roles\":[1]", 1), "\"room_roles\" must be an array of strings"),
            (MIN.replacen("\"id\":\"t\"", "\"id\":\"bad id\"", 1), "\"id\" must be"),
            (MIN.replacen("\"size\":[1,1,1]", "\"size\":[1,1]", 1), "prop_set[0].size"),
            (MIN.replacen("\"size\":[1,1,1]", "\"size\":[1,1,\"x\"]", 1), "prop_set[0].size"),
            (MIN.replacen("\"type\":\"crate\"", "\"type\":\"crate\",\"solid\":\"yes\"", 1), "prop_set[0].solid"),
            (MIN.replacen("\"type\":\"crate\"", "\"type\":\"crate\",\"tint\":1", 1), "unknown key \"tint\""),
            (MIN.replacen("\"hall_floor\":[\"a\"]", "\"hall_floor\":\"a\"", 1), "\"hall_floor\" must be"),
            (MIN.replacen("\"id\":\"t\"", "\"id\":\"t\",\"room_door\":3", 1), "\"room_door\" must be a string"),
            (MIN.replacen("\"id\":\"t\"", "\"id\":\"t\",\"shop_names\":[]", 1), "\"shop_names\" must be"),
        ];
        for (text, want) in cases {
            let e = fails(&text);
            assert!(e.contains("x.json"), "{e}");
            assert!(e.contains(want), "wanted {want:?} in {e:?}");
        }
    }

    #[test]
    fn a_file_that_is_not_json_or_not_an_object_fails() {
        assert!(fails("{").contains("x.json"));
        assert!(fails("[1]").contains("must hold a JSON object"));
    }

    #[test]
    fn duplicate_built_in_ids_are_dropped_and_reported() {
        let mut warnings = vec![];
        let t = RegionTable::from_builtin_files([("a.json", MIN), ("b.json", MIN)], &mut warnings);
        assert_eq!(t.defs.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("b.json") && warnings[0].contains("already defined"), "{warnings:?}");
    }

    #[test]
    fn a_pack_override_replaces_one_region_and_no_other() {
        let before = builtin();
        let mut t = builtin();
        let mut warnings = vec![];
        t.add_pack("alt", &["regions/mall".to_string()], &[("mall.json".into(), ALT_MALL.into())], &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        let mall = t.def("mall").unwrap();
        assert_eq!(mall.room_roles, ["cinema"]);
        assert!(mall.hall_runner);
        assert_eq!(mall.source, "alt");
        assert_eq!(t.def("hospital"), before.def("hospital"));
        assert_eq!(t.defs.len(), before.defs.len());
    }

    #[test]
    fn a_replacement_that_is_not_listed_is_rejected() {
        let before = builtin();
        let mut t = builtin();
        let mut warnings = vec![];
        t.add_pack("alt", &[], &[("mall.json".into(), ALT_MALL.into())], &mut warnings);
        assert!(warnings.iter().any(|w| w.contains("already exists") && w.contains("regions/mall")), "{warnings:?}");
        assert_eq!(t.defs, before.defs);
    }

    #[test]
    fn an_override_of_an_unknown_region_rejects_the_pack() {
        let before = builtin();
        let mut t = builtin();
        let mut warnings = vec![];
        let new_id = MIN.replacen("\"id\":\"t\"", "\"id\":\"dungeon\"", 1);
        t.add_pack("alt", &["regions/nowhere".to_string()], &[("dungeon.json".into(), new_id)], &mut warnings);
        assert!(warnings.iter().any(|w| w.contains("regions/nowhere")), "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("adds or replaces no regions")), "{warnings:?}");
        assert_eq!(t.defs, before.defs);
    }

    #[test]
    fn one_bad_file_rejects_every_region_file_of_its_pack() {
        let before = builtin();
        let mut t = builtin();
        let mut warnings = vec![];
        let dungeon = MIN.replacen("\"id\":\"t\"", "\"id\":\"dungeon\"", 1);
        let bad = MIN.replacen("\"id\":\"t\"", "\"id\":\"cave\",\"colour\":1", 1);
        t.add_pack(
            "alt",
            &[],
            &[("bad.json".into(), bad), ("dungeon.json".into(), dungeon)],
            &mut warnings,
        );
        assert!(warnings.iter().any(|w| w.contains("alt/regions/bad.json") && w.contains("colour")), "{warnings:?}");
        assert!(t.def("dungeon").is_none());
        assert_eq!(t.defs, before.defs);
    }

    #[test]
    fn a_second_pack_cannot_redefine_a_pack_region() {
        let mut t = builtin();
        let mut warnings = vec![];
        let dungeon = MIN.replacen("\"id\":\"t\"", "\"id\":\"dungeon\"", 1);
        t.add_pack("first", &[], &[("dungeon.json".into(), dungeon.clone())], &mut warnings);
        assert!(warnings.iter().any(|w| w.contains("loaded, but no level places it")), "{warnings:?}");
        let mut again = vec![];
        t.add_pack("second", &["regions/dungeon".to_string()], &[("dungeon.json".into(), dungeon)], &mut again);
        assert!(again.iter().any(|w| w.contains("already defined by first")), "{again:?}");
        assert_eq!(t.def("dungeon").unwrap().source, "first");
    }

    #[test]
    fn a_new_id_pack_region_loads_with_a_warning() {
        let mut t = builtin();
        let mut warnings = vec![];
        let dungeon = MIN.replacen("\"id\":\"t\"", "\"id\":\"dungeon\"", 1);
        t.add_pack("alt", &[], &[("dungeon.json".into(), dungeon)], &mut warnings);
        assert!(t.def("dungeon").is_some());
        assert!(warnings.iter().any(|w| w.contains("dungeon") && w.contains("no level places it")), "{warnings:?}");
    }

    #[test]
    fn load_reads_a_pack_folder_from_disk() {
        let root = std::env::temp_dir().join(format!("lr-regions-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("alt");
        std::fs::create_dir_all(dir.join("regions")).unwrap();
        std::fs::write(
            dir.join("pack.json"),
            r#"{"schema":"lr.pack/1","id":"alt","overrides":["regions/mall"]}"#,
        )
        .unwrap();
        std::fs::write(dir.join("regions").join("mall.json"), ALT_MALL).unwrap();
        let packs = vec![Pack {
            id: "alt".into(),
            name: "alt".into(),
            version: String::new(),
            author: String::new(),
            description: String::new(),
            dir: dir.clone(),
            provides: vec![],
        }];
        let mut warnings = vec![];
        let t = load(&packs, &mut warnings);
        let _ = std::fs::remove_dir_all(&root);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(t.def("mall").unwrap().room_roles, ["cinema"]);
        assert!(t.def("hospital").is_some());
    }
}
