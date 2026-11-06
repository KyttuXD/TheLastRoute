//! Reads story packs into one `Pack` and checks it. The core pack is the game's own content; mod packs add to it.
//! Every problem found is collected, so one run reports them all.
//!
//! A pack is a folder with a `pack.json` (`lr.pack/1`, the same manifest as last_route and the Godot mod kit):
//!
//! ```text
//! pack.json          {"schema": "lr.pack/1", "id", "name"?, "version"?, "author"?, "description"?,
//!                     "overrides"?: ["scenes/<id>", "items/<id>", "maps/<id>", "start"],
//!                     "start"?: {"map", "spawn"}, "start_scene"?}   (start is for the core pack or an override)
//! items/*.json       {"schema": "lr.story.items/1", "items": {id: {"name", "description"}}}
//! scenes/*.json      {"schema": "lr.story.scenes/1", "scenes": {id: scene}}
//! maps/*.json        {"schema": "lr.story.map/1", "id"?, "name", "art", "mask"?, "legend": {hex: kind}}
//! maps/<art>.png     the floor picture (any size, a whole multiple of the mask)
//! maps/<mask>.png    the mask: one pixel per tile, coloured by the legend ("mask" defaults to the art)
//! ```
//!
//! Ids a mod adds must start with `<pack id>_`, so they never collide. A mod may replace another pack's id only when
//! it lists `"<kind>/<id>"` in its overrides. The folder name must equal the pack id.

use crate::model::*;
use macroquad::prelude::{Image, ImageFormat};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_PACK: &str = "lr.pack/1";
pub const SCHEMA_ITEMS: &str = "lr.story.items/1";
pub const SCHEMA_SCENES: &str = "lr.story.scenes/1";
pub const SCHEMA_MAP: &str = "lr.story.map/1";
const DEFAULT_LOCKED: &str = "It won't open.";
/// A mask bigger than this is almost certainly a mistake (the grid is walked and drawn every frame).
const MAX_MASK_SIDE: usize = 512;

/// Where to look for packs: the core folder, extra pack folders named directly, and roots whose subfolders are packs.
#[derive(Clone, Debug, Default)]
pub struct Config {
    pub core: PathBuf,
    pub mod_dirs: Vec<PathBuf>,
    pub mod_roots: Vec<PathBuf>,
}

/// One pack's own content, before it is merged.
struct Contribution {
    id: String,
    name: String,
    version: String,
    core: bool,
    dir: PathBuf,
    overrides: Vec<String>,
    start: Option<(String, String)>,
    start_scene: Option<String>,
    items: BTreeMap<String, Item>,
    scenes: BTreeMap<String, Scene>,
    maps: BTreeMap<String, MapDef>,
}

/// The pack folders to read as mods: subfolders of each root that hold a `pack.json`, then the named folders. The core
/// folder is skipped, and a folder reached twice is read once.
pub fn discover(cfg: &Config) -> Vec<PathBuf> {
    let core = canonical(&cfg.core);
    let mut candidates: Vec<PathBuf> = Vec::new();
    for root in &cfg.mod_roots {
        let Ok(read) = fs::read_dir(root) else { continue };
        let mut subs: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.join("pack.json").is_file()).collect();
        subs.sort();
        candidates.extend(subs);
    }
    candidates.extend(cfg.mod_dirs.iter().cloned());
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for dir in candidates {
        let key = canonical(&dir);
        if key == core || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(dir);
    }
    out
}

fn canonical(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Reads and merges the core pack and every mod pack, then checks the whole. Mods merge in id order after the core.
pub fn load(cfg: &Config) -> Result<Pack, Vec<String>> {
    let mut errs = Vec::new();
    let Some(core) = read_contribution(&cfg.core, true, &mut errs) else {
        return Err(errs);
    };
    let mut mods: Vec<Contribution> = discover(cfg)
        .iter()
        .filter_map(|dir| read_contribution(dir, false, &mut errs))
        .collect();
    mods.sort_by(|a, b| a.id.cmp(&b.id));

    let (Some(start_map), Some(start_spawn)) = (core.start.clone().map(|s| s.0), core.start.clone().map(|s| s.1)) else {
        return Err(vec![format!("{}: the core pack needs \"start\": {{\"map\", \"spawn\"}}", core.dir.display())]);
    };
    let title = core.name.clone();
    let mut pack = Pack {
        title,
        start_map,
        start_spawn,
        start_scene: core.start_scene.clone(),
        items: BTreeMap::new(),
        scenes: BTreeMap::new(),
        maps: BTreeMap::new(),
        packs: Vec::new(),
    };
    let mut all = vec![core];
    all.extend(mods);
    for c in &all {
        merge(&mut pack, c, &mut errs);
    }
    errs.extend(validate(&pack));
    if errs.is_empty() {
        Ok(pack)
    } else {
        Err(errs)
    }
}

/// Adds one pack's content to the merged pack, applying the override rules and recording it in `packs`.
fn merge(pack: &mut Pack, c: &Contribution, errs: &mut Vec<String>) {
    put(&mut pack.items, "items", &c.items, c, errs);
    put(&mut pack.scenes, "scenes", &c.scenes, c, errs);
    put(&mut pack.maps, "maps", &c.maps, c, errs);
    if let Some((map, spawn)) = &c.start {
        if c.core || c.overrides.iter().any(|o| o == "start") {
            pack.start_map = map.clone();
            pack.start_spawn = spawn.clone();
            if c.start_scene.is_some() {
                pack.start_scene = c.start_scene.clone();
            }
        } else {
            errs.push(format!("pack '{}' sets \"start\"; only the core pack, or a pack listing \"start\" in its overrides, may", c.id));
        }
    }
    pack.packs.push(PackInfo {
        id: c.id.clone(),
        name: c.name.clone(),
        version: c.version.clone(),
        core: c.core,
        dir: c.dir.clone(),
        scenes: c.scenes.len(),
        items: c.items.len(),
        maps: c.maps.len(),
    });
}

fn put<T: Clone>(into: &mut BTreeMap<String, T>, kind: &str, from: &BTreeMap<String, T>, c: &Contribution, errs: &mut Vec<String>) {
    for (id, value) in from {
        let slot = format!("{kind}/{id}");
        if into.contains_key(id) {
            if c.overrides.contains(&slot) {
                into.insert(id.clone(), value.clone());
            } else {
                errs.push(format!(
                    "{kind} '{id}' from pack '{}' is already defined by an earlier pack; list \"{slot}\" in its overrides to replace it",
                    c.id
                ));
            }
        } else {
            into.insert(id.clone(), value.clone());
        }
    }
    for o in c.overrides.iter().filter(|o| o.starts_with(&format!("{kind}/"))) {
        let id = &o[kind.len() + 1..];
        if !into.contains_key(id) && !from.contains_key(id) {
            errs.push(format!("pack '{}' overrides \"{o}\", which no pack defines", c.id));
        }
    }
}

/// Reads one pack folder. Errors go into `errs` and the pack is dropped (None) when its manifest cannot be used.
fn read_contribution(dir: &Path, core: bool, errs: &mut Vec<String>) -> Option<Contribution> {
    let manifest_path = dir.join("pack.json");
    let root = match read_json(&manifest_path, SCHEMA_PACK) {
        Ok(v) => v,
        Err(e) => {
            errs.push(e);
            return None;
        }
    };
    let folder = dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let id = match str_field(&root, "id") {
        Some(id) if id == folder && crate::lore::is_pack_id(&id) => id,
        Some(id) => {
            errs.push(format!("{}: pack id '{id}' must be a valid id and match the folder name '{folder}'", manifest_path.display()));
            return None;
        }
        None => {
            errs.push(format!("{}: missing \"id\"", manifest_path.display()));
            return None;
        }
    };
    let ctx = format!("pack '{id}'");
    let name = str_field(&root, "name").unwrap_or_else(|| id.clone());
    let version = str_field(&root, "version").unwrap_or_else(|| "0.0.0".to_string());
    let overrides: Vec<String> = root["overrides"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    for o in &overrides {
        let known = o == "start" || ["scenes/", "items/", "maps/"].iter().any(|k| o.starts_with(k) && o.len() > k.len());
        if !known {
            errs.push(format!("{ctx}: override \"{o}\" must be \"scenes/<id>\", \"items/<id>\", \"maps/<id>\" or \"start\""));
        }
    }
    let start = match (root["start"]["map"].as_str(), root["start"]["spawn"].as_str()) {
        (Some(m), Some(s)) => Some((m.to_string(), s.to_string())),
        _ => None,
    };
    if core && start.is_none() {
        errs.push(format!("{ctx}: the core pack needs \"start\": {{\"map\", \"spawn\"}}"));
    }
    let start_scene = str_field(&root, "start_scene");

    // A mod's ids must carry its prefix unless the id is an override of another pack's id.
    let prefix = format!("{id}_");
    let check_id = |kind: &str, key: &str, errs: &mut Vec<String>| -> bool {
        if core || key.starts_with(&prefix) || overrides.contains(&format!("{kind}/{key}")) {
            true
        } else {
            errs.push(format!("{ctx}: {kind} id '{key}' must start with '{prefix}' (or be listed as an override)"));
            false
        }
    };

    let mut items = BTreeMap::new();
    for file in json_files(&dir.join("items")) {
        match read_json(&file, SCHEMA_ITEMS) {
            Ok(v) => {
                for (key, def) in obj_entries(&v["items"]) {
                    if !check_id("items", &key, errs) {
                        continue;
                    }
                    let name = str_field(def, "name").unwrap_or_else(|| key.clone());
                    let description = str_field(def, "description").unwrap_or_default();
                    insert_unique(&mut items, key, Item { name, description }, "item", errs);
                }
            }
            Err(e) => errs.push(e),
        }
    }

    let mut scenes = BTreeMap::new();
    for file in json_files(&dir.join("scenes")) {
        match read_json(&file, SCHEMA_SCENES) {
            Ok(v) => {
                for (key, def) in obj_entries(&v["scenes"]) {
                    if !check_id("scenes", &key, errs) {
                        continue;
                    }
                    match parse_scene(def) {
                        Ok(scene) => insert_unique(&mut scenes, key, scene, "scene", errs),
                        Err(e) => errs.push(format!("scene '{key}': {e}")),
                    }
                }
            }
            Err(e) => errs.push(e),
        }
    }

    let mut maps = BTreeMap::new();
    for file in json_files(&dir.join("maps")) {
        if let Some(map) = load_map(&file, dir, errs) {
            if check_id("maps", &map.id, errs) {
                insert_unique(&mut maps, map.id.clone(), map, "map", errs);
            }
        }
    }

    Some(Contribution {
        id,
        name,
        version,
        core,
        dir: dir.to_path_buf(),
        overrides,
        start,
        start_scene,
        items,
        scenes,
        maps,
    })
}

/// Every cross-reference in the merged pack: scene and map targets, item ids and spawn names. Pure, so tests call it.
pub fn validate(pack: &Pack) -> Vec<String> {
    let mut errs = Vec::new();
    let items = |id: &str, ctx: &str, errs: &mut Vec<String>| {
        if !pack.items.contains_key(id) {
            errs.push(format!("{ctx}: unknown item '{id}'"));
        }
    };
    let scene_ref = |id: &str, ctx: &str, errs: &mut Vec<String>| {
        if !pack.scenes.contains_key(id) {
            errs.push(format!("{ctx}: unknown scene '{id}'"));
        }
    };
    let spawn_ref = |map: &str, spawn: &str, ctx: &str, errs: &mut Vec<String>| match pack.maps.get(map) {
        None => errs.push(format!("{ctx}: unknown map '{map}'")),
        Some(m) if !m.spawns.contains_key(spawn) => errs.push(format!("{ctx}: map '{map}' has no spawn '{spawn}'")),
        Some(_) => {}
    };
    let conds = |c: &Cond, ctx: &str, errs: &mut Vec<String>| {
        let mut refs = Vec::new();
        c.item_refs(&mut refs);
        for r in refs {
            items(&r, ctx, errs);
        }
    };
    let ops = |list: &[Op], ctx: &str, errs: &mut Vec<String>| {
        for op in list {
            match op {
                Op::Give(i) | Op::Take(i) => items(i, ctx, errs),
                Op::Exit { map, spawn } => spawn_ref(map, spawn, ctx, errs),
                Op::SetFlag { .. } | Op::Toast(_) => {}
            }
        }
    };

    match pack.maps.get(&pack.start_map) {
        None => errs.push(format!("start map '{}' does not exist", pack.start_map)),
        Some(m) if !m.spawns.contains_key(&pack.start_spawn) => {
            errs.push(format!("start map '{}' has no spawn '{}'", pack.start_map, pack.start_spawn))
        }
        Some(_) => {}
    }
    if let Some(s) = &pack.start_scene {
        scene_ref(s, "start_scene", &mut errs);
    }

    for (id, scene) in &pack.scenes {
        let ctx = format!("scene '{id}'");
        if let Some(n) = &scene.next {
            scene_ref(n, &ctx, &mut errs);
        }
        ops(&scene.ops, &ctx, &mut errs);
        for (i, c) in scene.choices.iter().enumerate() {
            let cctx = format!("{ctx} choice {}", i + 1);
            if let Some(cond) = &c.cond {
                conds(cond, &cctx, &mut errs);
            }
            if let Some(n) = &c.next {
                scene_ref(n, &cctx, &mut errs);
            }
            ops(&c.ops, &cctx, &mut errs);
        }
    }

    for (id, map) in &pack.maps {
        for kind in map.legend.values() {
            let ctx = format!("map '{id}'");
            match kind {
                Kind::Door { open_if, .. } => conds(open_if, &ctx, &mut errs),
                Kind::Item { item, .. } => items(item, &ctx, &mut errs),
                Kind::Npc { scene, .. } | Kind::Trigger { scene, .. } => scene_ref(scene, &ctx, &mut errs),
                Kind::Exit { map: m, spawn } => spawn_ref(m, spawn, &ctx, &mut errs),
                Kind::Floor | Kind::Wall { .. } | Kind::Spawn { .. } => {}
            }
        }
    }
    errs
}

fn load_map(file: &Path, dir: &Path, errs: &mut Vec<String>) -> Option<MapDef> {
    let name_of_file = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let v = match read_json(file, SCHEMA_MAP) {
        Ok(v) => v,
        Err(e) => {
            errs.push(e);
            return None;
        }
    };
    let id = str_field(&v, "id").unwrap_or(name_of_file);
    let ctx = format!("map '{id}'");
    let name = str_field(&v, "name").unwrap_or_else(|| id.clone());
    let Some(art_name) = str_field(&v, "art") else {
        errs.push(format!("{ctx}: needs \"art\" (a PNG file name)"));
        return None;
    };
    let mask_name = str_field(&v, "mask").unwrap_or_else(|| art_name.clone());
    let art_path = dir.join("maps").join(&art_name);
    let mask_path = dir.join("maps").join(&mask_name);

    let mut legend = BTreeMap::new();
    for (hex, def) in obj_entries(&v["legend"]) {
        let Some(key) = normalise_colour(&hex) else {
            errs.push(format!("{ctx}: legend key '{hex}' is not a 6-digit hex colour or \"clear\""));
            continue;
        };
        match parse_kind(def) {
            Ok(kind) => {
                legend.insert(key, kind);
            }
            Err(e) => errs.push(format!("{ctx}: legend '{hex}': {e}")),
        }
    }

    let art = match read_png(&art_path) {
        Ok(img) => img,
        Err(e) => {
            errs.push(format!("{ctx}: {e}"));
            return None;
        }
    };
    let mask = if mask_path == art_path {
        None
    } else {
        match read_png(&mask_path) {
            Ok(img) => Some(img),
            Err(e) => {
                errs.push(format!("{ctx}: {e}"));
                return None;
            }
        }
    };
    let mask = mask.as_ref().unwrap_or(&art);
    let (mw, mh) = (mask.width as usize, mask.height as usize);
    let (aw, ah) = (art.width as usize, art.height as usize);
    if mw == 0 || mh == 0 || mw > MAX_MASK_SIDE || mh > MAX_MASK_SIDE {
        errs.push(format!("{ctx}: mask is {mw}x{mh}; each side must be 1-{MAX_MASK_SIDE} pixels"));
        return None;
    }
    if aw % mw != 0 || ah % mh != 0 {
        errs.push(format!("{ctx}: art is {aw}x{ah}, which is not a whole multiple of the {mw}x{mh} mask"));
    }

    let cells = decode_cells(mask.get_image_data(), &legend, &ctx, errs);
    let mut spawns = BTreeMap::new();
    for (i, kind) in cells.iter().enumerate() {
        if let Kind::Spawn { name } = kind {
            let at = ((i % mw) as i32, (i / mw) as i32);
            if spawns.insert(name.clone(), at).is_some() {
                errs.push(format!("{ctx}: spawn '{name}' is placed more than once"));
            }
        }
    }
    Some(MapDef {
        id,
        name,
        art: art_path,
        legend,
        grid: Grid { w: mw, h: mh, cells },
        spawns,
    })
}

/// Maps each pixel through the legend. A colour with no legend entry is a wall, and every such colour is reported.
fn decode_cells(px: &[[u8; 4]], legend: &BTreeMap<String, Kind>, ctx: &str, errs: &mut Vec<String>) -> Vec<Kind> {
    let mut unknown: BTreeMap<String, usize> = BTreeMap::new();
    let cells = px
        .iter()
        .map(|p| {
            let key = colour_key(p);
            match legend.get(&key) {
                Some(kind) => kind.clone(),
                None => {
                    *unknown.entry(key).or_insert(0) += 1;
                    Kind::Wall {
                        height: 3.0,
                        color: WALL_COLOUR,
                    }
                }
            }
        })
        .collect();
    for (key, n) in unknown {
        let shown = if key == "clear" { "transparent".to_string() } else { format!("#{key}") };
        errs.push(format!("{ctx}: mask colour {shown} is on {n} pixel(s) but the legend has no entry for it"));
    }
    cells
}

/// The legend key of a mask pixel: `rrggbb` in lowercase, or `clear` when the pixel is (nearly) transparent.
pub fn colour_key(p: &[u8; 4]) -> String {
    if p[3] < 128 {
        "clear".to_string()
    } else {
        format!("{:02x}{:02x}{:02x}", p[0], p[1], p[2])
    }
}

fn normalise_colour(raw: &str) -> Option<String> {
    if raw == "clear" {
        return Some(raw.to_string());
    }
    let hex = raw.strip_prefix('#').unwrap_or(raw).to_ascii_lowercase();
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

/// A box height in metres: the optional "height" field, default 3.0, must be above 0 and at most 16.
fn parse_height(v: &Value) -> Result<f32, String> {
    let h = v["height"].as_f64().unwrap_or(3.0);
    if h > 0.0 && h <= 16.0 {
        Ok(h as f32)
    } else {
        Err(format!("\"height\" must be above 0 and at most 16 metres, got {h}"))
    }
}

/// The optional "color" field as RGB, or the default.
fn parse_rgb(v: &Value, default: Rgb) -> Result<Rgb, String> {
    let Some(raw) = str_field(v, "color") else { return Ok(default) };
    let key = normalise_colour(&raw).filter(|k| k != "clear").ok_or_else(|| format!("bad \"color\" '{raw}'"))?;
    let n = u32::from_str_radix(&key, 16).map_err(|e| e.to_string())?;
    Ok([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

const WALL_COLOUR: Rgb = [0x5a, 0x63, 0x78];
const DOOR_COLOUR: Rgb = [0xb0, 0x70, 0x30];
const ITEM_COLOUR: Rgb = [0xff, 0xd8, 0x4a];
const NPC_COLOUR: Rgb = [0x6f, 0xbf, 0x8a];

fn parse_kind(v: &Value) -> Result<Kind, String> {
    Ok(match req_str(v, "kind")?.as_str() {
        "floor" => Kind::Floor,
        "wall" => Kind::Wall {
            height: parse_height(v)?,
            color: parse_rgb(v, WALL_COLOUR)?,
        },
        "spawn" => Kind::Spawn { name: req_str(v, "name")? },
        "door" => {
            let cond = v.get("if").ok_or("a door needs an \"if\" condition")?;
            Kind::Door {
                open_if: parse_cond(cond)?,
                locked: str_field(v, "locked").unwrap_or_else(|| DEFAULT_LOCKED.to_string()),
                height: parse_height(v)?,
                color: parse_rgb(v, DOOR_COLOUR)?,
            }
        }
        "item" => Kind::Item {
            item: req_str(v, "item")?,
            color: parse_rgb(v, ITEM_COLOUR)?,
        },
        "npc" => Kind::Npc {
            name: req_str(v, "name")?,
            scene: req_str(v, "scene")?,
            color: parse_rgb(v, NPC_COLOUR)?,
        },
        "trigger" => Kind::Trigger {
            scene: req_str(v, "scene")?,
            once: v["once"].as_bool().unwrap_or(true),
        },
        "exit" => Kind::Exit {
            map: req_str(v, "map")?,
            spawn: req_str(v, "spawn")?,
        },
        other => return Err(format!("unknown kind '{other}'")),
    })
}

pub fn parse_cond(v: &Value) -> Result<Cond, String> {
    if let Some(f) = str_field(v, "flag") {
        return Ok(Cond::Flag(f));
    }
    if let Some(f) = str_field(v, "not_flag") {
        return Ok(Cond::NotFlag(f));
    }
    if let Some(i) = str_field(v, "has") {
        return Ok(Cond::Has(i));
    }
    if let Some(inner) = v.get("not") {
        return Ok(Cond::Not(Box::new(parse_cond(inner)?)));
    }
    if let Some(list) = v.get("all") {
        return Ok(Cond::All(cond_list(list)?));
    }
    if let Some(list) = v.get("any") {
        return Ok(Cond::Any(cond_list(list)?));
    }
    Err(format!("unknown condition {v}"))
}

fn cond_list(v: &Value) -> Result<Vec<Cond>, String> {
    v.as_array()
        .ok_or("\"all\" and \"any\" take a list")?
        .iter()
        .map(parse_cond)
        .collect()
}

fn parse_op(v: &Value) -> Result<Op, String> {
    Ok(match req_str(v, "op")?.as_str() {
        "set_flag" => Op::SetFlag {
            flag: req_str(v, "flag")?,
            value: v["value"].as_bool().unwrap_or(true),
        },
        "give" => Op::Give(req_str(v, "item")?),
        "take" => Op::Take(req_str(v, "item")?),
        "exit" => Op::Exit {
            map: req_str(v, "map")?,
            spawn: req_str(v, "spawn")?,
        },
        "toast" => Op::Toast(req_str(v, "text")?),
        other => return Err(format!("unknown op '{other}'")),
    })
}

fn parse_ops(v: &Value) -> Result<Vec<Op>, String> {
    match v {
        Value::Null => Ok(Vec::new()),
        Value::Array(list) => list.iter().map(parse_op).collect(),
        _ => Err("\"do\" takes a list of ops".to_string()),
    }
}

fn parse_scene(v: &Value) -> Result<Scene, String> {
    let mut lines = Vec::new();
    if let Some(list) = v["lines"].as_array() {
        for l in list {
            lines.push(match l {
                Value::String(text) => Line {
                    speaker: String::new(),
                    text: text.clone(),
                },
                _ => Line {
                    speaker: str_field(l, "speaker").unwrap_or_default(),
                    text: req_str(l, "text")?,
                },
            });
        }
    }
    let mut choices = Vec::new();
    if let Some(list) = v["choices"].as_array() {
        for c in list {
            choices.push(Choice {
                text: req_str(c, "text")?,
                cond: match c.get("if") {
                    Some(cond) => Some(parse_cond(cond)?),
                    None => None,
                },
                ops: parse_ops(&c["do"])?,
                next: str_field(c, "next"),
            });
        }
    }
    Ok(Scene {
        once: v["once"].as_bool().unwrap_or(false),
        lines,
        choices,
        ops: parse_ops(&v["do"])?,
        next: str_field(v, "next"),
    })
}

fn read_json(path: &Path, schema: &str) -> Result<Value, String> {
    let name = path.display();
    let text = fs::read_to_string(path).map_err(|e| format!("{name}: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
    match str_field(&v, "schema") {
        Some(s) if s == schema => Ok(v),
        Some(s) => Err(format!("{name}: schema is '{s}', expected '{schema}'")),
        None => Err(format!("{name}: missing \"schema\": \"{schema}\"")),
    }
}

fn read_png(path: &Path) -> Result<Image, String> {
    let bytes = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Image::from_file_with_format(&bytes, Some(ImageFormat::Png)).map_err(|e| format!("{}: not a PNG ({e})", path.display()))
}

/// Sorted `*.json` files in a folder. A missing folder is empty, not an error.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |x| x == "json"))
        .collect();
    files.sort();
    files
}

fn obj_entries(v: &Value) -> Vec<(String, &Value)> {
    v.as_object()
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v)).collect())
        .unwrap_or_default()
}

fn insert_unique<T>(map: &mut BTreeMap<String, T>, id: String, value: T, what: &str, errs: &mut Vec<String>) {
    if map.contains_key(&id) {
        errs.push(format!("{what} '{id}' is defined more than once"));
    } else {
        map.insert(id, value);
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn req_str(v: &Value, key: &str) -> Result<String, String> {
    str_field(v, key).ok_or_else(|| format!("missing \"{key}\""))
}
